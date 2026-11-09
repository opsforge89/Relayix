"""Durable pauses, explicit schedules, quiet hours, grouped notifications and opt-in TOTP."""
import base64
from concurrent.futures import ThreadPoolExecutor
import datetime
import hashlib
import hmac
import json
import os
import struct
import subprocess
import time
import urllib.error
import urllib.request
import uuid
from runtime import credentials

BASE=os.environ['RELAYIX_TEST_BASE_URL']
def request(path,data=None,method=None,headers=None):
    h={'Authorization':'Bearer '+credentials['admin_token'],'Content-Type':'application/json','X-Relay-Client':'web'};h.update(headers or {})
    req=urllib.request.Request(BASE+path,data=json.dumps(data).encode() if data is not None else None,method=method,headers=h)
    try:r=urllib.request.urlopen(req,timeout=15)
    except urllib.error.HTTPError as e:r=e
    with r:return r.status,json.load(r),r.headers

def ok(path,data=None,method=None,headers=None):
    status,value,_=request(path,data,method,headers);assert 200<=status<300,(path,status,value);return value

def sql(statement):return subprocess.check_output(['psql','--no-psqlrc','-v','ON_ERROR_STOP=1','-Atc',statement],text=True).strip()
def wait(predicate,label='0.9.0 regression timed out'):
    deadline=time.monotonic()+35
    while time.monotonic()<deadline:
        value=predicate()
        if value:return value
        time.sleep(.1)
    raise AssertionError(label)

def received(path):
    with urllib.request.urlopen('http://mock:8000/received',timeout=5) as r:return [json.loads(item['body']) for item in json.load(r) if item.get('path')==path]

def otp(secret,offset=0):
    raw=base64.b32decode(secret+'='*((8-len(secret)%8)%8));step=int(time.time())//30+offset
    digest=hmac.new(raw,struct.pack('>Q',step),hashlib.sha1).digest();pos=digest[-1]&15
    return str((struct.unpack('>I',digest[pos:pos+4])[0]&0x7fffffff)%1000000).zfill(6)

def session(username,password,code=''):
    status,value,h=request('/api/auth/login',{'username':username,'password':password,'code':code},headers={'Authorization':''})
    return status,value,{'Authorization':'','Cookie':h.get('Set-Cookie','').split(';',1)[0]}

channels=[];routes=[];policies=[];users=[]
suffix=uuid.uuid4().hex

def route(channel,settings=None,quota=0):
    value=ok('/api/routes',{'name':'090 route','channel_ids':[channel],'delivery_settings':settings or {},'rate_limit_per_minute':quota});routes.append(value['id']);return value

def send(r,body):return ok(r['endpoint'],body,headers={'X-Relay-Key':r['token']})
def rows(r):return ok('/api/deliveries?route_id='+r['id']+'&page_size=100')['items']

try:
    path='/controls090-'+suffix
    channel=ok('/api/channels',{'name':'090 controls','kind':'webhook','config':{'url':'http://mock:8000'+path}})['id'];channels.append(channel)
    r=route(channel)
    assert request('/api/channels/'+channel+'/pause',{},headers={'Authorization':''})[0]==401
    ok('/api/channels/'+channel+'/pause',{})
    first=send(r,{'message':'Retained during vacation'})
    policy=ok('/api/alert-policies',{'name':'090 paused alert','enabled':True,'watch_channel_ids':[channel],'destination_ids':[channel],'failure_threshold':1,'min_attempts':1,'window_minutes':15,'cooldown_minutes':1,'title':'Retained alert','body':'Wait for resume'})['id'];policies.append(policy)
    ok('/api/alert-policies/'+policy+'/test',{},'POST')
    time.sleep(.8)
    assert rows(r)[0]['attempts']==0 and received(path)==[]
    saved=next(c for c in ok('/api/channels') if c['id']==channel)
    ok('/api/channels/'+channel,saved,'PUT')
    assert next(c for c in ok('/api/channels') if c['id']==channel)['manual_paused']
    ok('/api/channels/'+channel+'/resume',{},'POST')
    wait(lambda:rows(r)[0]['status']=='sent')
    wait(lambda:sql(f"SELECT status FROM alert_deliveries WHERE policy_id='{policy}'")=='sent')
    future=(datetime.datetime.now(datetime.timezone.utc)+datetime.timedelta(seconds=4)).isoformat()
    ok('/api/channels/'+channel+'/pause',{'until':future});send(r,{'message':'Timed pause'})
    time.sleep(.3);assert not any(item.get('message')=='Timed pause' for item in received(path))
    wait(lambda:any(item.get('message')=='Timed pause' for item in received(path)))
    send(r,{'message':'Scheduled delivery','delay':2})
    time.sleep(.3);assert not any(item.get('message')=='Scheduled delivery' for item in received(path))
    wait(lambda:any(item.get('message')=='Scheduled delivery' for item in received(path)))
    for payload in [{'delay':-1},{'delay':2592001},{'deliver_at':'invalid'},{'delay':1,'deliver_at':future}]:
        assert request(r['endpoint'],{'message':'Invalid schedule',**payload},headers={'X-Relay-Key':r['token']})[0]==400
    later=(datetime.datetime.now(datetime.timezone.utc)+datetime.timedelta(hours=1)).isoformat()
    scheduled=send(r,{'message':'Resume preserves explicit date','deliver_at':later})
    ok('/api/channels/'+channel+'/pause',{});ok('/api/channels/'+channel+'/resume',{},'POST')
    time.sleep(.3)
    delayed=next(d for d in rows(r) if d['event_id']==scheduled['event_id']);assert delayed['attempts']==0
    assert sql(f"SELECT not_before>NOW()+INTERVAL '50 minutes' FROM deliveries WHERE id='{delayed['id']}'")=='t'

    now=datetime.datetime.now(datetime.timezone.utc)
    quiet={'quiet_enabled':True,'timezone':'UTC','quiet_start':(now-datetime.timedelta(minutes=1)).strftime('%H:%M'),'quiet_end':(now+datetime.timedelta(minutes=2)).strftime('%H:%M'),'urgent_priority':4}
    quiet_route=route(channel,quiet)
    low=send(quiet_route,{'message':'Quiet low priority','priority':1});urgent=send(quiet_route,{'message':'Urgent bypass','priority':5})
    wait(lambda:any(d['event_id']==urgent['event_id'] and d['status']=='sent' for d in rows(quiet_route)))
    assert next(d for d in rows(quiet_route) if d['event_id']==low['event_id'])['attempts']==0
    preview=ok('/api/routes/'+quiet_route['id']+'/preview',{'message':'Quiet preview','priority':1})
    assert preview['routing']['delivery_policy']['quiet_hours'] and preview['routing']['delivery_policy']['timezone']=='UTC'

    if time.time()%300>295:time.sleep(301-time.time()%300)
    dedup=route(channel,{'dedup_seconds':300},quota=1)
    leader=send(dedup,{'message':'Repeated script notification'})
    with ThreadPoolExecutor(max_workers=4) as pool:duplicates=list(pool.map(lambda _:send(dedup,{'message':'Repeated script notification'}),range(4)))
    assert all(item['grouped'] and item['queued']==0 for item in duplicates)
    assert len({item['group_leader_id'] for item in duplicates})==1
    detail=ok('/api/events/'+leader['event_id']);assert detail['delivery_group']['occurrences']==5
    assert ok('/api/events?route_id='+dedup['id'])['total']==5 and len(rows(dedup))==1
    sql(f"UPDATE deliveries SET not_before=NOW(),next_attempt_at=NOW() WHERE event_id='{leader['event_id']}'")
    wait(lambda:rows(dedup)[0]['status']=='sent')
    assert len([p for p in received(path) if 'Repeated script notification' in p.get('message','')])==1
    assert any('×5' in p.get('message','') for p in received(path))
    assert ok('/api/events/'+leader['event_id'])['message']=='Repeated script notification'

    digest=route(channel,{'digest_seconds':3600,'digest_below_priority':3,'urgent_priority':4})
    first=send(digest,{'title':'First task','message':'First summary','priority':1})
    second=send(digest,{'title':'Second task','message':'Second summary','priority':2})
    assert second['grouped'] and second['group_leader_id']==first['event_id']
    urgent=send(digest,{'message':'Digest urgent bypass','priority':5})
    wait(lambda:any(d['event_id']==urgent['event_id'] and d['status']=='sent' for d in rows(digest)))
    sql(f"UPDATE deliveries SET not_before=NOW(),next_attempt_at=NOW() WHERE event_id='{first['event_id']}'")
    wait(lambda:all(d['status']=='sent' for d in rows(digest)))
    assert any(p.get('title')=='Digest · 2 notifications' and 'First summary' in p['message'] and 'Second summary' in p['message'] for p in received(path))
    exported=ok('/api/config/export');assert next(item for item in exported['routes'] if item['key']==digest['id'])['delivery_settings']['digest_seconds']==3600

    username='totp-'+suffix
    user=ok('/api/users',{'username':username,'display_name':'TOTP regression','role':'admin','password':credentials['operator_password']})['id'];users.append(user)
    code,_,account=session(username,credentials['operator_password']);assert code==200
    code,_,old_session=session(username,credentials['operator_password']);assert code==200
    assert ok('/api/auth/totp',headers=account)['enabled'] is False
    prepared=ok('/api/auth/totp/setup',{'password':credentials['operator_password']},headers=account)
    assert 'otpauth://totp/' in prepared['uri'] and prepared['secret']
    assert not ok('/api/auth/totp',headers=account)['enabled']
    enabled=ok('/api/auth/totp/enable',{'code':otp(prepared['secret'])},headers=account)
    backups=enabled['recovery_codes'];assert len(backups)==10 and len(set(backups))==10
    assert request('/api/auth/me',headers=old_session)[0]==401
    assert ok('/api/auth/totp',headers=account)['enabled']
    old_generation=sql(f"SELECT generation FROM user_totp WHERE user_id='{user}'")
    late=uuid.uuid4().hex
    sql(f"INSERT INTO sessions(token_hash,user_id,expires_at) VALUES('{hashlib.sha256(late.encode()).hexdigest()}','{user}',NOW()+INTERVAL '1 day')")
    assert request('/api/auth/me',headers={'Authorization':'','Cookie':'relayix-session='+late})[0]==401
    assert session(username,credentials['operator_password'])[0]==401
    assert session(username,credentials['operator_password'],otp(prepared['secret']))[0]==401
    current_code=otp(prepared['secret'],1)
    assert session(username,credentials['operator_password'],current_code)[0]==200
    assert session(username,credentials['operator_password'],current_code)[0]==401
    assert session(username,credentials['operator_password'],backups[0])[0]==200
    assert session(username,credentials['operator_password'],backups[0])[0]==401
    ok('/api/auth/totp/disable',{'password':credentials['operator_password'],'code':backups[1]},headers=account)
    assert session(username,credentials['operator_password'])[0]==200
    prepared=ok('/api/auth/totp/setup',{'password':credentials['operator_password']},headers=account)
    enabled=ok('/api/auth/totp/enable',{'code':otp(prepared['secret'])},headers=account)
    late=uuid.uuid4().hex
    sql(f"INSERT INTO sessions(token_hash,user_id,expires_at,mfa_generation) VALUES('{hashlib.sha256(late.encode()).hexdigest()}','{user}',NOW()+INTERVAL '1 day','{old_generation}')")
    assert request('/api/auth/me',headers={'Authorization':'','Cookie':'relayix-session='+late})[0]==401
    for _ in range(5):assert session(username,credentials['operator_password'],'invalid')[0]==401
    assert sql(f"SELECT locked_until>NOW() FROM user_totp WHERE user_id='{user}'")=='t'
    assert session(username,credentials['operator_password'],enabled['recovery_codes'][0])[0]==401
    assert request('/api/users/'+user+'/totp/reset',{},'POST',account)[0]==403
    ok('/api/users/'+user+'/totp/reset',{},'POST')
    assert request('/api/auth/me',headers=account)[0]==401
    assert session(username,credentials['operator_password'])[0]==200
    print('PASS: durable manual/timed pauses, schedules, urgent quiet-hour bypass, atomic deduplication, digests, immutable history and opt-in replay-safe TOTP with recovery',flush=True)
finally:
    for policy in policies:request('/api/alert-policies/'+policy,method='DELETE')
    for route_id in routes:request('/api/routes/'+route_id,method='DELETE')
    for channel_id in channels:request('/api/channels/'+channel_id,method='DELETE')
    for user in users:request('/api/users/'+user,method='DELETE')
