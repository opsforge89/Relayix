"""Operational routing contracts against isolated PostgreSQL and local receivers."""
from concurrent.futures import ThreadPoolExecutor
import json
import subprocess
import time
import uuid
from urllib.parse import urlencode


def verify(api, anonymous):
    channels, route = [], None
    def wait(fn, label, seconds=100):
        deadline=time.monotonic()+seconds
        while time.monotonic()<deadline:
            value=fn()
            if value:
                return value
            time.sleep(.5)
        raise AssertionError(label)
    try:
        for name, path in [('primary-a','fail'),('primary-b','fail'),('fallback','fallback-contract'),('fallback-failed','fail')]:
            channels.append(api.ok('/api/channels', {'name':'Operations '+name,'kind':'webhook','enabled':True,'config':{'url':'http://mock:8000/'+path}},'POST')['id'])
        settings={'name':'Operations contract','channel_ids':channels[:2],'fallback_channel_ids':channels[2:],'min_priority':1,'required_tag':'Deploy','enabled':True}
        route=api.ok('/api/routes',settings,'POST')
        base='/api/routes/'+route['id']
        rows=lambda:api.ok('/api/deliveries?route_id='+route['id']+'&page_size=100')['items']
        history=lambda:api.ok('/api/events?route_id='+route['id'])
        headers={'X-Relay-Key':route['token']}
        assert anonymous.request('/api/events')[0]==401
        assert anonymous.request(base+'/preview',{},'POST')[0]==401
        before=history()['total']
        for _ in range(2):
            preview=api.ok(base+'/preview',{'title':'Preview only'},'POST')
            assert preview['preview'] and preview['queued']==0 and preview['would_queue']==2 and 'event_id' not in preview
            assert all(not r['queued'] and r['would_queue'] for r in preview['routing']['recipients'])
            assert len(preview['routing']['fallback_recipients'])==2
        assert history()['total']==before and rows()==[]
        idem=str(uuid.uuid4())
        def accept(_):
            return type(api)().ok(route['endpoint'],{'title':'Concurrent operation','message':'Must be delivered once'},'POST',{**headers,'Idempotency-Key':idem})
        with ThreadPoolExecutor(max_workers=6) as pool:
            results=list(pool.map(accept,range(6)))
        event_id=results[0]['event_id']
        assert len({r['event_id'] for r in results})==1 and sum(not r.get('duplicate',False) for r in results)==1
        assert all(r['queued']==2 for r in results) and history()['total']==before+1
        assert api.request(route['endpoint'],{'title':'Changed'},'POST',{**headers,'Idempotency-Key':idem})[0]==409
        assert api.request(route['endpoint'],{},'POST',{**headers,'Idempotency-Key':''})[0]==400
        # Advance only this test's retry schedule, while still executing every actual attempt.
        def finished():
            subprocess.run(['psql','--no-psqlrc','-v','ON_ERROR_STOP=1','-c',"UPDATE channel_health SET paused_until=NOW()-INTERVAL '1 second' WHERE channel_id IN (SELECT channel_id FROM deliveries WHERE event_id='"+event_id+"') AND paused_until IS NOT NULL"],check=True,stdout=subprocess.DEVNULL)
            subprocess.run(['psql','--no-psqlrc','-v','ON_ERROR_STOP=1','-c',"UPDATE deliveries SET next_attempt_at=NOW() WHERE event_id='"+event_id+"' AND status='queued'"],check=True,stdout=subprocess.DEVNULL)
            data=rows()
            return data if len(data)==4 and all(d['status'] in ('sent','failed') for d in data) else False
        completed=wait(finished,'Primary/fallback attempts did not complete')
        assert sum(d['is_fallback'] for d in completed)==2
        assert len([d for d in completed if d['channel_id']==channels[2] and d['status']=='sent'])==1
        assert len([d for d in completed if d['status']=='failed' and d['attempts']==5])==3
        primary=next(d for d in completed if not d['is_fallback'])
        attempts_path='/api/deliveries/'+primary['id']+'/attempts'
        assert anonymous.request(attempts_path)[0]==401
        attempts=api.ok(attempts_path)
        assert attempts['attempts']==5 and len(attempts['items'])==5 and attempts['can_retry']
        assert all(a['finished_at'] and a['error'] and a['status']=='failed' for a in attempts['items'])
        assert 'http://mock' not in json.dumps(attempts)
        api.ok('/api/deliveries/'+primary['id']+'/retry',{},'POST')
        def retried():
            finished()
            data=api.ok(attempts_path)
            return data if data['status']=='failed' and data['attempts']==10 else False
        data=wait(retried,'Retry did not preserve previous attempts')
        assert len(data['items'])==10 and len(rows())==4
        assert api.ok('/api/deliveries?'+urlencode({'event_id':event_id}))['total']==4
        settings['min_priority']=5
        api.ok(base,settings,'PUT')
        filtered=api.ok(route['endpoint'],{'title':'No queue'},'POST',headers)
        assert filtered['queued']==0
        hist=api.ok('/api/events?'+urlencode({'route_id':route['id'],'reason':'priority_below_minimum','search':'No queue'}))
        assert hist['total']==1 and hist['items'][0]['id']==filtered['event_id'] and hist['items'][0]['deliveries']==0
        assert hist['items'][0]['routing']['route_tag']=='Deploy'
        settings['enabled']=False
        api.ok(base,settings,'PUT')
        preview=api.ok(base+'/preview',{},'POST')
        assert preview['would_queue']==0 and preview['routing']['reason']=='route_disabled'
        assert history()['total']==before+2
        assert api.request(base,{**settings,'fallback_channel_ids':[channels[0]]},'PUT')[0]==400
        assert anonymous.request('/api/channels/diagnose',{'kind':'telegram','config':{}},'POST')[0]==401
        missing=api.ok('/api/channels/diagnose',{'kind':'telegram','config':{}},'POST')
        assert not missing['ok'] and missing['steps'][0]['name']=='configuration' and missing['sent'] is False
        # HTTP proxy rejects CONNECT locally: diagnostics must not try a direct Telegram connection.
        config={'bot_token':str(uuid.uuid4()),'chat_id':'-123','proxy_type':'http','proxy_host':'proxy','proxy_port':3128}
        telegram=api.ok('/api/channels',{'name':'Operations diagnostics','kind':'telegram','config':config},'POST')['id']
        channels.append(telegram)
        endpoint='/api/channels/'+telegram+'/diagnose'
        assert anonymous.request(endpoint,{},'POST')[0]==401
        diagnostic=api.ok(endpoint,{},'POST')
        assert not diagnostic['ok'] and diagnostic['sent'] is False
        assert diagnostic['steps'][0]['status']=='passed' and diagnostic['steps'][1]['name']=='transport' and diagnostic['steps'][1]['status']=='failed'
        assert config['bot_token'] not in json.dumps(diagnostic)
        print('PASS: read-only preview, incoming history, concurrent idempotency/conflicts, fallback once without recursion, retained retry attempts, exact event filter and safe Telegram diagnostics',flush=True)
    finally:
        if route:
            api.ok('/api/routes/'+route['id'],method='DELETE')
        for cid in channels:
            api.ok('/api/channels/'+cid,method='DELETE')
