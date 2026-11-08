"""Template snapshots, safe previews, exact retry content and per-channel diagnosis."""
import json
import subprocess
import time
import uuid
from urllib.request import urlopen
from runtime import credentials


def verify(api, anonymous):
    channels, templates = [], []
    route = None
    def wait(fn):
        for _ in range(120):
            value=fn()
            if value:return value
            time.sleep(.3)
        raise AssertionError('Template delivery did not finish')
    def details(event):return api.ok('/api/events/'+event['event_id'])
    def received():
        with urlopen('http://mock:8000/received') as response:return json.load(response)
    try:
        spec={'name':'Contract test results','title':'{{test_type}} {{suite}}','body':'Branch: **{{branch}}**\n\nStatus: {{status}}\nFailed: {{failed}}\nPassed: {{passed}}\nSkipped: {{skipped}}','link_label':'Runner log','link_url':'{{log_url}}','link_failure_only':True}
        assert anonymous.request('/api/templates',spec,'POST')[0]==401
        assert anonymous.request('/api/templates/preview',{**spec,'payload':{}},'POST')[0]==401
        for body in ('{{#if status}}','{{bad','{{foo..bar}}'):
            assert api.request('/api/templates',{**spec,'body':body},'POST')[0]==400
        t=api.ok('/api/templates',spec,'POST');templates.append(t['id'])
        assert t['version']==1
        cfg={'mode':'bot','url':'http://mock:8000','bot_token':credentials['mattermost_token'],'target':'channel','channel_id':'mmchannel'}
        primary=api.ok('/api/channels',{'name':'Template primary','kind':'webhook','config':{'url':'http://mock:8000/fail'}},'POST')['id'];channels.append(primary)
        fallback=api.ok('/api/channels',{'name':'Template fallback','kind':'mattermost','config':cfg},'POST')['id'];channels.append(fallback)
        settings={'name':'Template contract','channel_ids':[primary],'fallback_channel_ids':[fallback],'template_id':t['id'],'icon':'flask-conical'}
        assert api.request('/api/routes',{**settings,'icon':'<script>'},'POST')[0]==400
        route=api.ok('/api/routes',settings,'POST')
        stored=next(r for r in api.ok('/api/routes') if r['id']==route['id'])
        assert stored['icon']=='flask-conical' and stored['template_id']==t['id']
        payload={'test_type':'UnitTests','suite':'ExampleSuite','branch':'feature/example','status':'❌','failed':2,'passed':40,'skipped':0,'log_url':'https://gitlab.example.org/acme/demo/-/jobs/12345'}
        before=api.ok('/api/events')['total']
        preview=api.ok('/api/templates/preview',{**spec,'payload':payload},'POST')
        assert preview['sent'] is False and preview['result']['link_added'] and 'Skipped: 0' in preview['message']
        assert api.ok('/api/events')['total']==before
        assert api.ok('/api/routes/'+route['id']+'/preview',payload,'POST')['rendered']['message']==preview['message']
        idem=str(uuid.uuid4());headers={'X-Relay-Key':route['token'],'Idempotency-Key':idem}
        event=api.ok(route['endpoint'],payload,'POST',headers)
        data=details(event)
        assert anonymous.request('/api/events/'+event['event_id'])[0]==401
        assert data['original_payload']==payload and data['template']['version']==1 and data['template_result']['applied']
        assert data['message']==preview['message'] and data['has_title'] and data['title']=='UnitTests ExampleSuite'
        changed=api.ok('/api/templates/'+t['id'],{**spec,'body':'CHANGED {{message}}'},'PUT')
        assert changed['version']==2
        duplicate=api.ok(route['endpoint'],payload,'POST',headers)
        assert duplicate['duplicate'] and duplicate['event_id']==event['event_id']
        assert details(event)['message']==preview['message']
        # Execute real failure attempts; accelerate this event's retry scheduling only.
        def finished():
            subprocess.run(['psql','--no-psqlrc','-v','ON_ERROR_STOP=1','-c',"UPDATE channel_health SET paused_until=NOW()-INTERVAL '1 second' WHERE channel_id='"+primary+"' AND paused_until IS NOT NULL"],check=True,stdout=subprocess.DEVNULL)
            subprocess.run(['psql','--no-psqlrc','-v','ON_ERROR_STOP=1','-c',"UPDATE deliveries SET next_attempt_at=NOW() WHERE event_id='"+event['event_id']+"' AND status='queued'"],check=True,stdout=subprocess.DEVNULL)
            rows=details(event)['recipients']
            return rows if len(rows)==2 and all(r['status'] in ('sent','failed') for r in rows) else None
        done=wait(finished)
        assert next(r for r in done if r['channel_id']==primary)['attempts']==5
        assert next(r for r in done if r['channel_id']==fallback)['status']=='sent'
        assert any(r.get('path')=='/api/v4/posts' and 'Runner log' in json.loads(r['body']).get('message','') and 'CHANGED' not in r['body'] for r in received())
        failed=next(r for r in done if r['channel_id']==primary)
        api.ok('/api/deliveries/'+failed['id']+'/retry',{},'POST');wait(finished)
        assert details(event)['rendered_payload']['message']==preview['message']
        api.ok('/api/routes/'+route['id'],{**settings,'channel_ids':[],'fallback_channel_ids':[]},'PUT')
        missing=api.ok(route['endpoint'],{'message':'Original **body**'},'POST',{'X-Relay-Key':route['token']})
        assert details(missing)['template_result']['reason']=='missing_fields' and details(missing)['message']=='Original **body**'
        assert api.request(route['endpoint'],{},'POST',{'X-Relay-Key':route['token']})[0]==400
        # Restore the user-created layout; success omits optional job URL and preserves zero counts.
        api.ok('/api/templates/'+t['id'],spec,'PUT')
        for suite in ['ExampleSuite','Example.Worker','Example.Documents']:
            sample={**payload,'suite':suite,'status':'✅','failed':0,'skipped':0}
            sample.pop('log_url')
            result=api.ok('/api/templates/preview',{**spec,'payload':sample},'POST')
            assert result['result']['applied'] and not result['result']['link_added'] and 'Failed: 0' in result['message']
        fall=api.ok(route['endpoint'],{'message':'Fallback original'},'POST',{'X-Relay-Key':route['token']})
        assert details(fall)['template_result']['reason']=='missing_fields' and details(fall)['message']=='Fallback original'
        assert api.request(route['endpoint'],{'suite':'Missing fields'},'POST',{'X-Relay-Key':route['token']})[0]==400
        grouped=api.ok('/api/deliveries?group_events=true&event_id='+event['event_id'])['items'][0]
        assert grouped['route_icon']=='flask-conical' and grouped['title']=='UnitTests ExampleSuite' and grouped['has_title']
        api.ok('/api/templates/'+t['id'],method='DELETE');templates.remove(t['id'])
        assert next(r for r in api.ok('/api/routes') if r['id']==route['id'])['template_id'] is None
        assert details(event)['template']['version']==1
        # Diagnostics never submit SMTP DATA or webhook messages; all checks use local servers.
        configs={kind:{'url':'http://mock:8000/'+kind} for kind in ['webhook','slack','teams','google_chat','rocket_chat','mattermost','discord']}
        configs['discord']['url']='http://mock:8000/discord-diagnostic'
        configs.update({'mattermost':cfg,'matrix':{'url':'http://mock:8000','access_token':'fictional','room_id':'!test:local'},'smtp':{'host':'mock','port':1025,'from':'sender@example.org','to':'recipient@example.org','tls':'none'},'ntfy':{'url':'http://mock:8000','topic':'test'},'gotify':{'url':'http://mock:8000','token':'fictional'}})
        before=len([r for r in received() if r.get('method') in ('POST','PUT','SMTP')])
        for kind,config in configs.items():
            result=api.ok('/api/channels/diagnose',{'kind':kind,'config':config},'POST')
            assert result['sent'] is False and result['ok'], (kind,result)
            assert not any(s['status']=='failed' for s in result['steps'])
            assert all(s['status'] in ('passed','skipped') for s in result['steps'])
        # Fixed public endpoints must not be contacted in the isolated suite.
        for kind in ('telegram','pushover'):
            result=api.ok('/api/channels/diagnose',{'kind':kind,'config':{}},'POST')
            assert not result['ok'] and result['steps'][0]['name']=='configuration'
        assert len([r for r in received() if r.get('method') in ('POST','PUT','SMTP')])==before
        print('PASS: editable-template CRUD, zero counters, dotted/literal substitution, preview without send, job links, snapshots across edits/retries/fallback/deletion, optional route icons and individual read-only diagnostics',flush=True)
    finally:
        if route:api.ok('/api/routes/'+route['id'],method='DELETE')
        for tid in templates:api.ok('/api/templates/'+tid,method='DELETE')
        for cid in channels:api.ok('/api/channels/'+cid,method='DELETE')
