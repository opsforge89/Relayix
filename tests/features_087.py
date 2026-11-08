"""Route quotas, JSON webhook bodies, length limits and safe configuration transfer."""
import concurrent.futures
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
def wait(predicate):
    deadline=time.monotonic()+25
    while time.monotonic()<deadline:
        value=predicate()
        if value:return value
        time.sleep(.1)
    raise AssertionError('Feature regression timed out')
def raw_received(path):
    with urllib.request.urlopen('http://mock:8000/received',timeout=5) as r:
        return [item['body'] for item in json.load(r) if item['path']==path]
def received(path):return [json.loads(body) for body in raw_received(path)]

channels=[];routes=[];members=[]
try:
    suffix=uuid.uuid4().hex
    path='/feature087-'+suffix
    hidden='private-template-'+uuid.uuid4().hex
    channel=ok('/api/channels',{'name':'087 custom JSON','kind':'webhook','config':{'url':'http://mock:8000'+path,'body_template':{'text':'{{message}}','priority':'{{priority}}','static':hidden}}})['id'];channels.append(channel)
    route=ok('/api/routes',{'name':'087 quota','channel_ids':[channel],'rate_limit_per_minute':2});routes.append(route['id'])
    headers={'X-Relay-Key':route['token'],'Idempotency-Key':'stable-'+suffix}
    message='Quote: " and newline:\n {{source}}'
    first=ok(route['endpoint'],{'message':message,'priority':5},headers=headers)
    duplicate=ok(route['endpoint'],{'message':message,'priority':5},headers=headers)
    assert duplicate['duplicate'] and first['event_id']==duplicate['event_id']
    ok(route['endpoint'],{'message':'second'},headers={'X-Relay-Key':route['token']})
    code,body,h=request(route['endpoint'],{'message':'blocked'},headers={'X-Relay-Key':route['token']})
    assert code==429 and h['Retry-After']=='60' and 'Route rate' in body['error']
    assert sql(f"SELECT requests FROM route_rate_windows WHERE route_id='{route['id']}'")=='2'
    ok('/api/routes/'+route['id']+'/preview',{'message':'preview'})
    assert sql(f"SELECT requests FROM route_rate_windows WHERE route_id='{route['id']}'")=='2'
    wait(lambda:len(received(path))>=2)
    assert {'text':message,'priority':5,'static':hidden} in received(path)
    sql(f"UPDATE route_rate_windows SET window_started_at=NOW()-INTERVAL '61 seconds' WHERE route_id='{route['id']}'")
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        statuses=list(pool.map(lambda n:request(route['endpoint'],{'message':'parallel '+str(n)},headers={'X-Relay-Key':route['token']})[0],range(8)))
    assert statuses.count(200)==2 and statuses.count(429)==6,statuses
    assert sql(f"SELECT COUNT(*) FROM events WHERE route_id='{route['id']}'")=='4'
    bad=request('/api/channels',{'name':'Invalid template','kind':'webhook','config':{'url':'http://mock:8000/bad-template','body_template':{'text':'{{unknown}}'}}})
    assert bad[0]==400

    discord_path='/limit-discord087-'+suffix
    discord=ok('/api/channels',{'name':'087 bounded Discord','kind':'discord','config':{'url':'http://mock:8000'+discord_path}})['id'];channels.append(discord)
    discord_route=ok('/api/routes',{'name':'087 long message','channel_ids':[discord]});routes.append(discord_route['id'])
    original='😀'*2400
    event=ok(discord_route['endpoint'],{'message':original},headers={'X-Relay-Key':discord_route['token']})
    content=wait(lambda:received(discord_path))[0]['content']
    assert len(content.encode('utf-16-le'))//2<=2000 and content.endswith('[Message truncated]')
    assert sql(f"SELECT message FROM events WHERE id='{event['event_id']}'")==original

    ntfy_topic='limit-ntfy087-'+suffix
    ntfy=ok('/api/channels',{'name':'087 bounded ntfy','kind':'ntfy','config':{'url':'http://mock:8000','topic':ntfy_topic}})['id'];channels.append(ntfy)
    ntfy_route=ok('/api/routes',{'name':'087 ntfy body','channel_ids':[ntfy]});routes.append(ntfy_route['id'])
    ok(ntfy_route['endpoint'],{'message':original},headers={'X-Relay-Key':ntfy_route['token']})
    ntfy_body=wait(lambda:raw_received('/'+ntfy_topic))[0]
    assert len(ntfy_body.encode())<=4096 and ntfy_body.endswith('[Message truncated]')
    google_path='/limit-google087-'+suffix
    google=ok('/api/channels',{'name':'087 bounded Google Chat','kind':'google_chat','config':{'url':'http://mock:8000'+google_path}})['id'];channels.append(google)
    google_route=ok('/api/routes',{'name':'087 Google body','channel_ids':[google]});routes.append(google_route['id'])
    ok(google_route['endpoint'],{'message':chr(0)*9000},headers={'X-Relay-Key':google_route['token']})
    google_raw=wait(lambda:raw_received(google_path))[0]
    assert len(google_raw.encode())<=32000 and json.loads(google_raw)['text'].endswith('[Message truncated]')

    before_channels={c['id'] for c in ok('/api/channels')};before_routes={r['id'] for r in ok('/api/routes')}
    bundle=ok('/api/config/export')
    member_name='transfer-'+suffix
    member=ok('/api/users',{'username':member_name,'display_name':'Transfer access regression','role':'member','password':credentials['operator_password']})['id'];members.append(member)
    code,_,h=request('/api/auth/login',{'username':member_name,'password':credentials['operator_password']},headers={'Authorization':''})
    assert code==200
    member_headers={'Authorization':'','Cookie':h['Set-Cookie'].split(';',1)[0]}
    assert request('/api/config/export',headers=member_headers)[0]==403
    assert request('/api/config/import',bundle,headers=member_headers)[0]==403
    member_channels=ok('/api/channels',headers=member_headers)
    protected=next(c for c in member_channels if c['id']==channel)
    assert protected['config']['body_template']=='********' and hidden not in json.dumps(member_channels)
    override={'name':'087 custom JSON','kind':'webhook','config':{'url':'http://mock:8000/credential-capture'}}
    assert request('/api/channels/'+channel,override,method='PUT',headers=member_headers)[0]==403
    assert request('/api/channels/'+channel,method='DELETE',headers=member_headers)[0]==403
    assert request('/api/channels/diagnose',{'id':channel,'kind':'webhook','config':override['config']},headers=member_headers)[0]==403
    assert request('/api/channels',{'name':'Member template','kind':'webhook','config':{'url':'http://mock:8000/blocked-template','body_template':{'text':'{{message}}'}}},headers=member_headers)[0]==403
    # Members retain ordinary channel editing; administrators retain a secret template on masked saves.
    ok('/api/channels/'+discord,{'name':'  087 bounded Discord  ','kind':'discord','config':{'url':'********'}},method='PUT',headers=member_headers)
    ok('/api/channels/'+channel,{'name':'087 custom JSON','kind':'webhook','config':{'url':'********','body_template':'********'}},method='PUT')
    assert next(c for c in ok('/api/channels') if c['id']==channel)['config']['body_template']['static']==hidden
    for name in ['', '   ', 'x'*101, 'line'+chr(10)+'break']:
        assert request('/api/channels',{'name':name,'kind':'webhook','config':{'url':'http://mock:8000/name-check'}})[0]==400
    assert request('/api/channels/'+discord,{'name':'x'*101,'kind':'discord','config':{'url':'********'}},method='PUT')[0]==400
    unicode_channel=ok('/api/channels',{'name':'😀'*100,'kind':'webhook','enabled':False,'config':{'url':'http://mock:8000/unicode-name'}})['id']
    channels.append(unicode_channel)
    assert next(c for c in ok('/api/config/export')['channels'] if c['key']==unicode_channel)['name']=='😀'*100
    ok('/api/channels/'+unicode_channel,method='DELETE');channels.remove(unicode_channel)
    # Simulate records created by old versions. Export identifies the exact bad record.
    try:
        for invalid in ['', 'x'*101]:
            sql(f"UPDATE channels SET name='{invalid}' WHERE id='{discord}'")
            code,error,_=request('/api/config/export')
            assert code==400 and discord in error['error'] and 'Channel name' in error['error']
    finally:
        sql(f"UPDATE channels SET name='087 bounded Discord' WHERE id='{discord}'")

    encoded=json.dumps(bundle)
    assert hidden not in encoded and route['token'] not in encoded and credentials['admin_token'] not in encoded
    for c in bundle['channels']:
        assert not any(k in c['config'] for k in ['url','bot_token','token','access_token','password','proxy_secret','api_hash','body_template'])
    assert not any('slug' in r or 'token_hash' in r or 'token' in r for r in bundle['routes'])
    bad_bundle=json.loads(encoded);bad_bundle['channels'][0]['config']['token']=uuid.uuid4().hex
    assert request('/api/config/import',bad_bundle)[0]==400
    assert {c['id'] for c in ok('/api/channels')}==before_channels
    result=ok('/api/config/import',bundle)
    assert result['disabled']
    imported_channels=[c for c in ok('/api/channels') if c['id'] not in before_channels]
    imported_routes=[r for r in ok('/api/routes') if r['id'] not in before_routes]
    channels.extend(c['id'] for c in imported_channels);routes.extend(r['id'] for r in imported_routes)
    assert len(imported_channels)==len(bundle['channels']) and all(not c['enabled'] for c in imported_channels)
    assert len(imported_routes)==len(bundle['routes']) and all(not r['enabled'] for r in imported_routes)
    assert all(set(r['channels']+r['fallback_channel_ids'])<={c['id'] for c in imported_channels} for r in imported_routes)
    assert any(r['rate_limit_per_minute']==2 for r in imported_routes if r['name']=='087 quota')
    print('PASS: atomic route quotas/idempotency, typed JSON bodies, bounded Unicode messages, credential-free export and disabled atomic imports',flush=True)
finally:
    for route in routes:request('/api/routes/'+route,method='DELETE')
    for channel in channels:request('/api/channels/'+channel,method='DELETE')
    for member in members:request('/api/users/'+member,method='DELETE')
