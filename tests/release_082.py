"""0.8.2 contracts: validation, immutable versions, guarded batches and personal alert isolation."""
import copy
from concurrent.futures import ThreadPoolExecutor
import json
import subprocess
import time
import uuid
from runtime import credentials


def verify(api, anonymous):
    from smoke import Client, TOKEN
    channels, routes, templates, users = [], [], [], []
    def sql(command):
        return subprocess.check_output(['psql','--no-psqlrc','-v','ON_ERROR_STOP=1','-Atc',command],text=True).strip()
    def wait(fn, message='0.8.2 fixture did not finish'):
        deadline=time.monotonic()+35
        while time.monotonic()<deadline:
            value=fn()
            if value:return value
            time.sleep(.25)
        raise AssertionError(message)
    def personal(client):return client.ok('/api/auth/delivery-alerts')['policy']
    try:
        for username in ('release082-one','release082-two'):
            uid=api.ok('/api/users',{'username':username,'display_name':username,'role':'member','enabled':True,'password':credentials['browser_password']},'POST')['id'];users.append(uid)
        one,two=Client(),Client()
        one.login('release082-one',credentials['browser_password']);two.login('release082-two',credentials['browser_password'])
        assert anonymous.request('/api/auth/delivery-alerts')[0]==401
        assert api.request('/api/auth/delivery-alerts')[0]==403
        assert personal(one) is None and personal(two) is None
        for name,path in [('Failed receiver','fail'),('Successful receiver','release082'),('Independent alert recipient','release082-alert')]:
            channels.append(api.ok('/api/channels',{'name':'082 '+name,'kind':'webhook','enabled':True,'config':{'url':'http://mock:8000/'+path}},'POST')['id'])
        for cid in channels:
            api.ok('/api/channels/'+cid+'/access',{'owner_user_id':None,'grants':[{'user_id':uid,'can_view':True,'can_edit':False,'can_send':True} for uid in users]},'PUT')
        spec={'name':'082 computed tests','title':'{{suite}}','body':'{{status}} · Passed: {{passed}} · Failed: {{failed}}','computed_status':True,'passed_field':'passed','failed_field':'failed','link_url':''}
        template=api.ok('/api/templates',spec,'POST');templates.append(template['id'])
        for passed,failed,status in [(40,2,'❌'),(40,0,'✅'),(0,0,'⚠️'),(0,2,'❌')]:
            preview=api.ok('/api/templates/preview',{**spec,'payload':{'suite':'UnitTests','passed':passed,'failed':failed,'status':'untrusted'}},'POST')
            assert preview['result']['computed_status']==status and preview['message'].startswith(status)
        missing=api.ok('/api/templates/preview',{**spec,'body':'{{status}}','payload':{'suite':'UnitTests','passed':40}},'POST')
        assert missing['message']=='⚠️' and missing['result']['warnings']
        nested=api.ok('/api/templates/preview',{**spec,'body':'{{status}}','passed_field':'counts.passed','failed_field':'counts.failed','payload':{'suite':'Nested','counts':{'passed':10,'failed':1}}},'POST')
        assert nested['message']=='❌'
        linked=api.ok('/api/templates/preview',{**spec,'body':'{{status}}','passed_field':'counts.passed','failed_field':'counts.failed','link_url':'https://example.org/job','link_failure_only':True,'payload':{'suite':'Nested','counts':{'passed':10,'failed':1}}},'POST')
        assert linked['result']['link_added']
        legacy=api.ok('/api/templates/preview',{**spec,'body':'{{status}}','computed_status':False,'payload':{'suite':'Legacy','status':'custom','passed':40,'failed':2}},'POST')
        assert legacy['message']=='custom'
        assert api.request('/api/templates',{**spec,'passed_field':'status'},'POST')[0]==400
        rules=[{'field':'suite','kind':'string','required':True},{'field':'passed','kind':'integer','required':True,'min':0},{'field':'failed','kind':'integer','required':True,'min':0},{'field':'build.url','kind':'url','required':False}]
        route_spec={'name':'082 validated route','channel_ids':channels[:2],'template_id':template['id'],'payload_rules':rules}
        route=api.ok('/api/routes',route_spec,'POST');routes.append(route['id'])
        headers={'X-Relay-Key':route['token']}
        total=api.ok('/api/events?route_id='+route['id'])['total']
        status,error=anonymous.request(route['endpoint'],{'passed':40,'failed':'two'},'POST',headers)
        assert status==400 and set(error['fields'])=={'suite','failed'}
        assert api.ok('/api/events?route_id='+route['id'])['total']==total
        assert api.request('/api/routes/'+route['id']+'/preview',{'suite':'Unit','passed':40,'failed':-1},'POST')[0]==400
        assert api.request('/api/routes',{**route_spec,'payload_rules':[{'field':'suite','kind':'javascript','required':True}]},'POST')[0]==400
        assert api.request('/api/routes',{**route_spec,'payload_rules':[rules[0],rules[0]]},'POST')[0]==400
        clone=api.ok('/api/routes',{**route_spec,'name':'082 validated route copy'},'POST');routes.append(clone['id'])
        assert clone['id']!=route['id'] and clone['endpoint']!=route['endpoint'] and clone['token']!=route['token']
        assert next(r for r in api.ok('/api/routes') if r['id']==clone['id'])['payload_rules']==next(r for r in api.ok('/api/routes') if r['id']==route['id'])['payload_rules']
        settings={'enabled':True,'destination_ids':[channels[2]],'watch_route_ids':[route['id']],'trigger_mode':'final','failure_threshold':3,'window_minutes':15,'cooldown_minutes':15}
        a=one.ok('/api/auth/delivery-alerts',settings,'PUT')['policy']
        b=two.ok('/api/auth/delivery-alerts',{**settings,'watch_route_ids':[clone['id']],'owner_user_id':users[0]},'PUT')['policy']
        assert a['id']!=b['id'] and a['owner_user_id']==users[0] and b['owner_user_id']==users[1]
        assert a['final_only'] and a['failure_threshold']==1
        assert not {a['id'],b['id']}&{p['id'] for p in api.ok('/api/alert-policies')}
        shared={'name':'Do not change personal rules','enabled':True,'destination_ids':[channels[2]],'failure_threshold':1,'min_attempts':1,'window_minutes':15,'cooldown_minutes':15,'title':'Test','body':'Test'}
        assert api.request('/api/alert-policies/'+a['id'],shared,'PUT')[0]==404
        assert api.request('/api/alert-policies/'+a['id']+'/test',{},'POST')[0]==404
        assert one.ok('/api/auth/delivery-alerts/test',{},'POST')['queued']==1
        wait(lambda:any(row['status']=='sent' for row in one.ok('/api/auth/delivery-alerts/history')))
        assert two.ok('/api/auth/delivery-alerts/history')==[]
        event=anonymous.ok(route['endpoint'],{'suite':'UnitTests','passed':40,'failed':2},'POST',{**headers,'Idempotency-Key':'release082-immutable'})
        event_id=event['event_id']
        api.ok('/api/routes/'+route['id'],{**route_spec,'payload_rules':rules+[{'field':'new_field','kind':'string','required':True}]},'PUT')
        replay=anonymous.ok(route['endpoint'],{'suite':'UnitTests','passed':40,'failed':2},'POST',{**headers,'Idempotency-Key':'release082-immutable'})
        assert replay['duplicate'] and replay['event_id']==event_id
        api.ok('/api/routes/'+route['id'],route_spec,'PUT')
        wait(lambda:sql("SELECT COALESCE(error,'') FROM deliveries WHERE event_id='"+event_id+"' AND channel_id='"+channels[0]+"'"))
        sql("UPDATE deliveries SET next_attempt_at=NOW()+INTERVAL '1 hour' WHERE event_id='"+event_id+"' AND channel_id='"+channels[0]+"'")
        time.sleep(6)
        assert all('test' in row['title'].lower() for row in one.ok('/api/auth/delivery-alerts/history'))
        sql("UPDATE deliveries SET status='failed',cycle_attempts=5,last_error_at=NOW() WHERE event_id='"+event_id+"' AND channel_id='"+channels[0]+"'")
        wait(lambda:any(row['title']=='Relayix: delivery failure' for row in one.ok('/api/auth/delivery-alerts/history')))
        assert two.ok('/api/auth/delivery-alerts/history')==[]
        own_ids={row['id'] for row in one.ok('/api/auth/delivery-alerts/history')}
        assert not own_ids&{row['id'] for row in api.ok('/api/alert-deliveries')}
        detail=api.ok('/api/events/'+event_id)
        assert detail['template']['version']==1 and detail['message'].startswith('❌')
        assert 'status' not in detail['original_payload']
        second=api.ok('/api/templates/'+template['id'],{**spec,'body':'Changed {{status}}'},'PUT');assert second['version']==2
        history=api.ok('/api/templates/'+template['id']+'/versions')['items'];assert [v['version'] for v in history]==[2,1]
        restored=api.ok('/api/templates/'+template['id']+'/versions/1/restore',{},'POST');assert restored['version']==3 and restored['body']==spec['body']
        assert api.ok('/api/events/'+event_id)['message']==detail['message']
        assert api.request('/api/templates/'+template['id']+'/versions/999/restore',{},'POST')[0]==404
        assert anonymous.request('/api/templates/'+template['id']+'/versions')[0]==401
        def update_version(n):return Client(TOKEN).ok('/api/templates/'+template['id'],{**spec,'body':'Concurrent '+str(n)+' {{status}}'},'PUT')['version']
        with ThreadPoolExecutor(max_workers=2) as pool:assert sorted(pool.map(update_version,[1,2]))==[4,5]
        assert [v['version'] for v in api.ok('/api/templates/'+template['id']+'/versions')['items']]==[5,4,3,2,1]
        copied=api.ok('/api/templates',{**spec,'name':'082 template copy'},'POST');templates.append(copied['id'])
        assert copied['version']==1 and copied['id']!=template['id'] and copied['computed_status']
        # Hold the good recipient in a final-failure state to verify the confirmation state guard.
        wait(lambda:any(r['status']=='sent' for r in api.ok('/api/events/'+event_id)['recipients']))
        sql("UPDATE deliveries SET status='failed',error='Fixture failure' WHERE event_id='"+event_id+"' AND channel_id='"+channels[1]+"'")
        batch=one.ok('/api/deliveries/retry-batch/preview',{'filters':{'route_id':route['id'],'group_events':True}},'POST')
        assert batch['count']==batch['total']==2
        assert two.request('/api/deliveries/retry-batch/'+batch['batch_id'],{},'POST')[0]==404
        assert anonymous.request('/api/deliveries/retry-batch/'+batch['batch_id'],{},'POST')[0]==401
        assert one.request('/api/deliveries/retry-batch/preview',{'filters':{'status':'sent'}},'POST')[0]==400
        sql("UPDATE deliveries SET status='sent',error=NULL WHERE event_id='"+event_id+"' AND channel_id='"+channels[1]+"'")
        done=one.ok('/api/deliveries/retry-batch/'+batch['batch_id'],{},'POST')
        assert done['queued']==1 and done['skipped']==1 and not done['duplicate']
        duplicate=one.ok('/api/deliveries/retry-batch/'+batch['batch_id'],{},'POST')
        assert duplicate['duplicate'] and duplicate['queued']==1
        assert int(sql("SELECT retry_generation FROM deliveries WHERE event_id='"+event_id+"' AND channel_id='"+channels[0]+"'"))==1
        assert api.ok('/api/events/'+event_id)['message']==detail['message']
        sql("UPDATE deliveries SET status='failed',error='New cycle failure',cycle_attempts=5,last_error_at=NOW() WHERE event_id='"+event_id+"' AND channel_id='"+channels[0]+"'")
        expired=one.ok('/api/deliveries/retry-batch/preview',{'filters':{'event_id':event_id}},'POST')
        sql("UPDATE delivery_retry_batches SET expires_at=NOW()-INTERVAL '1 minute' WHERE id='"+expired['batch_id']+"'")
        assert one.request('/api/deliveries/retry-batch/'+expired['batch_id'],{},'POST')[0]==409
        # Cooldown prevents a new cycle warning until the configured interval elapses.
        time.sleep(6)
        assert len([row for row in one.ok('/api/auth/delivery-alerts/history') if row['title']=='Relayix: delivery failure'])==1
        sql("UPDATE alert_policies SET last_triggered_at=NOW()-INTERVAL '16 minutes' WHERE id='"+a['id']+"'")
        wait(lambda:len([row for row in one.ok('/api/auth/delivery-alerts/history') if row['title']=='Relayix: delivery failure'])==2)
        old_alert=one.ok('/api/auth/delivery-alerts/history')[-1]['id']
        sql("UPDATE alert_deliveries SET status='failed',error='Temporary fixture failure' WHERE id='"+old_alert+"'")
        assert two.request('/api/auth/delivery-alerts/history/'+old_alert+'/retry',{},'POST')[0]==404
        one.ok('/api/auth/delivery-alerts/history/'+old_alert+'/retry',{},'POST')
        # Verify early-warning and threshold modes independently of final failures.
        two.ok('/api/auth/delivery-alerts',{**settings,'watch_route_ids':[clone['id']],'trigger_mode':'first_error'},'PUT')
        def held_failure():
            # Alert subscription tests advance only their failing channel's cooldown.
            sql("UPDATE channel_health SET paused_until=NOW()-INTERVAL '1 second' WHERE channel_id='"+channels[0]+"' AND paused_until IS NOT NULL")
            event=anonymous.ok(clone['endpoint'],{'suite':'Retrying','passed':0,'failed':1},'POST',{'X-Relay-Key':clone['token']})['event_id']
            wait(lambda:sql("SELECT COALESCE(error,'') FROM deliveries WHERE event_id='"+event+"' AND channel_id='"+channels[0]+"'"))
            sql("UPDATE deliveries SET next_attempt_at=NOW()+INTERVAL '1 hour' WHERE event_id='"+event+"' AND channel_id='"+channels[0]+"'")
            return event
        early=held_failure()
        wait(lambda:len(two.ok('/api/auth/delivery-alerts/history'))==1)
        assert sql("SELECT status FROM deliveries WHERE event_id='"+early+"' AND channel_id='"+channels[0]+"'")=='queued'
        two.ok('/api/auth/delivery-alerts',{**settings,'watch_route_ids':[clone['id']],'trigger_mode':'threshold','failure_threshold':2},'PUT')
        sql("UPDATE alert_policies SET last_triggered_at=NOW()-INTERVAL '16 minutes' WHERE id='"+b['id']+"'")
        held_failure();held_failure()
        wait(lambda:len(two.ok('/api/auth/delivery-alerts/history'))==2)
        api.ok('/api/users/'+users[1],{'username':'release082-two','display_name':'release082-two','role':'member','enabled':False},'PUT')
        held_failure()
        blocked=str(uuid.uuid4())
        sql("INSERT INTO alert_deliveries(id,policy_id,channel_id,owner_user_id,title,message) VALUES('"+blocked+"','"+b['id']+"','"+channels[2]+"','"+users[1]+"','Disabled account fixture','Test warning')")
        wait(lambda:sql("SELECT status FROM alert_deliveries WHERE id='"+blocked+"'")=='cancelled')
        assert 'permission revoked' in sql("SELECT error FROM alert_deliveries WHERE id='"+blocked+"'").lower()
        one.ok('/api/auth/delivery-alerts',{**settings,'enabled':False,'destination_ids':[str(uuid.uuid4())]},'PUT')
        assert not personal(one)['enabled']
        assert one.request('/api/auth/delivery-alerts/test',{},'POST')[0]==400
        print('PASS: 0.8.2 computed failure precedence/warnings, route schema rejection, clone isolation, version restore/concurrency, batch ownership/expiry/idempotency/state guards, personal alert scope/cooldown/retry cycles and separate history',flush=True)
    finally:
        for uid in users:api.ok('/api/users/'+uid,method='DELETE')
        for rid in routes:api.ok('/api/routes/'+rid,method='DELETE')
        for tid in templates:api.ok('/api/templates/'+tid,method='DELETE')
        for cid in channels:api.ok('/api/channels/'+cid,method='DELETE')


if __name__=='__main__':
    from smoke import Client,TOKEN,wait_for
    api=Client(TOKEN)
    wait_for(lambda:api.ok('/healthz')['status']=='ok','Application readiness')
    verify(api,Client())
