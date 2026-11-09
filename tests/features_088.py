"""Presentation previews, credential retargeting, provider backoff and shared cooldowns."""
import json
import os
import subprocess
import time
import urllib.error
import urllib.request
import uuid
from runtime import credentials

BASE=os.environ['RELAYIX_TEST_BASE_URL']
def request(path,data=None,method=None,headers=None):
    h={'Authorization':'Bearer '+credentials['admin_token'],'Content-Type':'application/json','X-Relay-Client':'web'}
    h.update(headers or {})
    req=urllib.request.Request(BASE+path,data=json.dumps(data).encode() if data is not None else None,method=method,headers=h)
    try:r=urllib.request.urlopen(req,timeout=15)
    except urllib.error.HTTPError as e:r=e
    with r:return r.status,json.load(r),r.headers

def ok(path,data=None,method=None,headers=None):
    code,body,_=request(path,data,method,headers)
    assert 200<=code<300,(path,code,body)
    return body

def sql(statement):return subprocess.check_output(['psql','--no-psqlrc','-v','ON_ERROR_STOP=1','-Atc',statement],text=True).strip()
def wait(predicate,label='Delivery regression timed out'):
    deadline=time.monotonic()+30
    while time.monotonic()<deadline:
        value=predicate()
        if value:return value
        time.sleep(.1)
    raise AssertionError(label)

def received(path):
    with urllib.request.urlopen('http://mock:8000/received',timeout=5) as r:
        return [item for item in json.load(r) if item.get('path')==path]

channels=[];routes=[];policies=[];members=[]
suffix=uuid.uuid4().hex

def channel(kind,config):
    result=ok('/api/channels',{'name':'088 '+kind,'kind':kind,'config':config})['id'];channels.append(result);return result

def route(ids,fallback=None):
    value=ok('/api/routes',{'name':'088 route','channel_ids':ids,'fallback_channel_ids':fallback or []});routes.append(value['id']);return value

def send(r,message='Delivery regression'):
    return ok(r['endpoint'],{'message':message},headers={'X-Relay-Key':r['token']})

def rows(r):return ok('/api/deliveries?route_id='+r['id']+'&page_size=100')['items']

def health(cid):return next(c['health'] for c in ok('/api/channels') if c['id']==cid)

try:
    assert request('/api/channels/preview',{'kind':'discord'},headers={'Authorization':''})[0]==401
    before=sql('SELECT COUNT(*) FROM events')
    for kind,maximum in [('telegram',4096),('discord',2000),('rocket_chat',5000)]:
        preview=ok('/api/channels/preview',{'kind':kind,'title':'Title','message':'😀'*2500})
        assert preview['truncated'] and preview['limits'][0]['used']<=maximum
        assert preview['text'].endswith('[Message truncated]')
    assert sql('SELECT COUNT(*) FROM events')==before
    hidden=uuid.uuid4().hex
    secret_channel=channel('webhook',{'url':'http://mock:8000/preview088-'+suffix,'body_template':{'key':hidden,'text':'{{message}}'}})
    discord=channel('discord',{'url':'http://mock:8000/preview-discord088-'+suffix})
    preview_route=route([secret_channel,discord])
    preview=ok('/api/routes/'+preview_route['id']+'/preview',{'message':'a'*9000})
    assert len(preview['channel_previews'])==2
    assert next(c for c in preview['channel_previews'] if c['id']==discord)['presentation']['truncated']
    assert rows(preview_route)==[] and received('/preview088-'+suffix)==[]
    user='preview-'+suffix
    members.append(ok('/api/users',{'username':user,'display_name':'Preview regression','role':'member','password':credentials['operator_password']})['id'])
    code,_,headers=request('/api/auth/login',{'username':user,'password':credentials['operator_password']},headers={'Authorization':''})
    assert code==200
    member_headers={'Authorization':'','Cookie':headers['Set-Cookie'].split(';',1)[0]}
    preview=ok('/api/routes/'+preview_route['id']+'/preview',{'message':'a'*9000},headers=member_headers)
    assert hidden not in json.dumps(preview)
    assert next(c for c in preview['channel_previews'] if c['id']==secret_channel)['presentation']['restricted']
    for kind,config,field,replacement in [
        ('gotify',{'url':'http://mock:8000','token':hidden},'url','http://mock:8000/capture088'),
        ('matrix',{'url':'http://mock:8000','room_id':'!test:local','access_token':hidden},'url','http://mock:8000/capture088'),
        ('smtp',{'host':'mock','port':1025,'tls':'none','username':'example','password':hidden,'from':'test@example.org','to':'ops@example.org'},'host','proxy'),
    ]:
        cid=channel(kind,config)
        masked=next(c['config'] for c in ok('/api/channels',headers=member_headers) if c['id']==cid)
        ok('/api/channels/'+cid,{'name':'088 unchanged','kind':kind,'config':masked},'PUT',member_headers)
        masked[field]=replacement
        assert request('/api/channels/'+cid,{'name':'088 retarget','kind':kind,'config':masked},'PUT',member_headers)[0]==400
        assert request('/api/channels/diagnose',{'id':cid,'kind':kind,'config':masked},headers=member_headers)[0]==400
    assert received('/capture088')==[]
    good=channel('webhook',{'url':'http://mock:8000/resilience088/'+suffix+'/good'})
    auth=channel('webhook',{'url':'http://mock:8000/resilience088/'+suffix+'/auth'})
    auth_route=route([auth],[good]);send(auth_route)
    completed=wait(lambda: (data if len(data)==2 and all(d['status'] in ('sent','failed') for d in data) else None) if (data:=rows(auth_route)) else None)
    failure=next(d for d in completed if d['channel_id']==auth)
    assert failure['status']=='failed' and failure['attempts']==1
    detail=ok('/api/deliveries/'+failure['id']+'/attempts')
    assert detail['error_category']=='authentication' and 'credential echo' not in json.dumps(detail)
    assert next(d for d in completed if d['channel_id']==good)['status']=='sent'

    rate_path='/resilience088/'+suffix+'/rate'
    rate=channel('webhook',{'url':'http://mock:8000'+rate_path});rate_route=route([rate]);send(rate_route)
    rate_delivery=wait(lambda: next((d for d in rows(rate_route) if d['error']),None))
    detail=ok('/api/deliveries/'+rate_delivery['id']+'/attempts')
    assert detail['error_category']=='rate_limit' and detail['next_attempt_at'] and health(rate)['state']=='paused'
    assert int(sql(f"SELECT EXTRACT(EPOCH FROM next_attempt_at-NOW())::int FROM deliveries WHERE id='{rate_delivery['id']}'"))>=110
    send(rate_route,'A second message waits for the same provider deadline')
    ok('/api/deliveries/'+rate_delivery['id']+'/retry',{},'POST')
    time.sleep(1)
    assert len(received(rate_path))==1 and len(rows(rate_route))==2

    # Rejections of individual messages preserve errors without poisoning channel health.
    message_channel=channel('webhook',{'url':'http://mock:8000/resilience088/'+suffix+'/message'})
    message_route=route([message_channel])
    for index in range(3):
        event=send(message_route,'bad')['event_id']
        failure=wait(lambda:next((d for d in rows(message_route) if d['event_id']==event and d['status']=='failed'),None))
        assert failure['attempts']==1 and failure['error']
        assert health(message_channel)['state']=='healthy' and health(message_channel)['consecutive_failures']==0
    send(message_route,'A valid message still delivers')
    wait(lambda:any(d['status']=='sent' for d in rows(message_route)))
    # A message rejection from an expired recovery probe releases retained healthy traffic.
    sql(f"INSERT INTO channel_health(channel_id,consecutive_failures,paused_until) VALUES('{message_channel}',3,NOW()-INTERVAL '1 second')")
    event=send(message_route,'bad')['event_id']
    wait(lambda:any(d['event_id']==event and d['status']=='failed' for d in rows(message_route)))
    assert health(message_channel)['state']=='healthy'
    # The test endpoint generates its own body rather than using the policy template.
    rejected_channel=channel('webhook',{'url':'http://mock:8000/resilience088/'+suffix+'/reject_message'})
    message_policy=ok('/api/alert-policies',{'name':'Message rejection history','enabled':True,'watch_channel_ids':[good],'destination_ids':[rejected_channel],'failure_threshold':1,'min_attempts':1,'window_minutes':15,'cooldown_minutes':1,'title':'Rejected alert','body':'bad'})['id'];policies.append(message_policy)
    ok('/api/alert-policies/'+message_policy+'/test',{},'POST')
    rejected_alert=wait(lambda:next((a for a in ok('/api/alert-deliveries') if a['policy_name']=='Message rejection history' and a['status']=='failed'),None))
    assert rejected_alert['error'] and health(rejected_channel)['consecutive_failures']==0

    # Remote retry delays have a one-day ceiling, and repaired settings resume both queues.
    huge=channel('webhook',{'url':'http://mock:8000/resilience088/'+suffix+'/huge_rate'});huge_route=route([huge]);send(huge_route)
    huge_delivery=wait(lambda:next((d for d in rows(huge_route) if d['error']),None))
    remaining=int(sql(f"SELECT EXTRACT(EPOCH FROM next_attempt_at-NOW())::int FROM deliveries WHERE id='{huge_delivery['id']}'"))
    assert 86380<=remaining<=86400
    assert request('/api/channels/'+huge,{'name':'','kind':'webhook','config':{'url':'http://mock:8000/repaired'}},'PUT')[0]==400
    assert health(huge)['state']=='paused'
    resume_policy=ok('/api/alert-policies',{'name':'Resume queued alerts','enabled':True,'watch_channel_ids':[good],'destination_ids':[huge],'failure_threshold':1,'min_attempts':1,'window_minutes':15,'cooldown_minutes':1,'title':'Resume alert','body':'Retained warning'})['id'];policies.append(resume_policy)
    ok('/api/alert-policies/'+resume_policy+'/test',{},'POST')
    sql(f"UPDATE alert_deliveries SET error='Provider rate limit',next_attempt_at=NOW()+INTERVAL '1 day' WHERE policy_id='{resume_policy}'")
    ok('/api/channels/'+huge,{'name':'Repaired channel','kind':'webhook','config':{'url':'http://mock:8000/resilience088/'+suffix+'/repaired'}},'PUT')
    wait(lambda:any(d['status']=='sent' for d in rows(huge_route)))
    wait(lambda:any(a['policy_name']=='Resume queued alerts' and a['status']=='sent' for a in ok('/api/alert-deliveries')))
    assert health(huge)['state']=='healthy'

    flaky_path='/resilience088/'+suffix+'/flaky'
    flaky=channel('webhook',{'url':'http://mock:8000'+flaky_path});flaky_route=route([flaky]);send(flaky_route)
    def paused():
        sql(f"UPDATE deliveries SET next_attempt_at=NOW() WHERE channel_id='{flaky}' AND status='queued'")
        return health(flaky)['state']=='paused'
    wait(paused)
    assert health(flaky)['consecutive_failures']==3
    count=len(received(flaky_path));send(flaky_route,'Backlog survives the pause')
    policies.append(ok('/api/alert-policies',{'name':'088 shared queue','enabled':True,'watch_channel_ids':[good],'watch_route_ids':[],'destination_ids':[flaky],'failure_threshold':1,'min_attempts':1,'window_minutes':15,'cooldown_minutes':1,'title':'Probe alert','body':'Shared queue probe'})['id'])
    ok('/api/alert-policies/'+policies[-1]+'/test',{},'POST')
    time.sleep(1)
    assert len(received(flaky_path))==count
    assert sql(f"SELECT COALESCE(MAX(attempts),0) FROM alert_deliveries WHERE policy_id='{policies[-1]}'")=='0'
    healthy_route=route([good]);send(healthy_route)
    wait(lambda:any(d['status']=='sent' for d in rows(healthy_route)))
    with urllib.request.urlopen('http://mock:8000/control088/'+suffix,timeout=5) as response:assert response.status==200
    sql(f"UPDATE channel_health SET paused_until=NOW()-INTERVAL '1 second' WHERE channel_id='{flaky}'")
    wait(lambda:sql(f"SELECT (SELECT COUNT(*) FROM deliveries WHERE channel_id='{flaky}' AND status='processing')+(SELECT COUNT(*) FROM alert_deliveries WHERE channel_id='{flaky}' AND status='processing')")=='1')
    # Until that successful probe finishes, both queues share a single active attempt.
    time.sleep(.3)
    assert sql(f"SELECT (SELECT COUNT(*) FROM deliveries WHERE channel_id='{flaky}' AND status='processing')+(SELECT COUNT(*) FROM alert_deliveries WHERE channel_id='{flaky}' AND status='processing')")=='1'
    wait(lambda:health(flaky)['state']=='healthy')
    wait(lambda:all(d['status']=='sent' for d in rows(flaky_route)))
    wait(lambda:sql(f"SELECT COUNT(*) FROM alert_deliveries WHERE policy_id='{policies[-1]}' AND status='sent'")=='1')
    assert health(flaky)['consecutive_failures']==0
    print('PASS: channel previews, safe credentials, permanent errors, Retry-After, cooldown, single shared probe and recovery',flush=True)
finally:
    for policy in policies:request('/api/alert-policies/'+policy,method='DELETE')
    for r in routes:request('/api/routes/'+r,method='DELETE')
    for c in channels:request('/api/channels/'+c,method='DELETE')
    for member in members:request('/api/users/'+member,method='DELETE')
