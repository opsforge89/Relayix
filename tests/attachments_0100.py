"""Durable native file forwarding, atomic limits, private downloads and immutable retry data."""
import hashlib
import json
import os
import subprocess
import time
import urllib.error
import urllib.request
import uuid
from runtime import credentials

BASE=os.environ['RELAYIX_TEST_BASE_URL']
MIME='application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'
channels=[];routes=[]
suffix=uuid.uuid4().hex
blob=b'PK\x03\x04\x00\xff\x80'+suffix.encode()+b'xlsx-report'
def request(path,data=None,method=None,headers=None):
    h={'Authorization':'Bearer '+credentials['admin_token'],'Content-Type':'application/json','X-Relay-Client':'web'};h.update(headers or {})
    req=urllib.request.Request(BASE+path,data=json.dumps(data).encode() if data is not None else None,method=method,headers=h)
    try:r=urllib.request.urlopen(req,timeout=20)
    except urllib.error.HTTPError as e:r=e
    with r:return r.status,json.load(r)
def ok(path,data=None,method=None,headers=None):
    status,value=request(path,data,method,headers);assert 200<=status<300,(path,status,value);return value
def sql(text):return subprocess.check_output(['psql','--no-psqlrc','-v','ON_ERROR_STOP=1','-Atc',text],text=True).strip()
def wait(fn):
    end=time.monotonic()+35
    while time.monotonic()<end:
        result=fn()
        if result:return result
        time.sleep(.1)
    raise AssertionError('Attachment delivery timed out')
def upload(r,payload=None,files=None,headers=None,expected=200):
    boundary='relayix'+uuid.uuid4().hex
    parts=[]
    if payload is not None:parts.append(b'--'+boundary.encode()+b'\r\nContent-Disposition: form-data; name="payload"\r\n\r\n'+json.dumps(payload).encode()+b'\r\n')
    for name,data in (files if files is not None else [('report.xlsx',blob)]):
        parts.append(b'--'+boundary.encode()+b'\r\nContent-Disposition: form-data; name="files"; filename="'+name.encode()+b'"\r\nContent-Type: '+MIME.encode()+b'\r\n\r\n'+data+b'\r\n')
    body=b''.join(parts)+b'--'+boundary.encode()+b'--\r\n'
    h={'Content-Type':'multipart/form-data; boundary='+boundary,'X-Relay-Key':r['token'],**(headers or {})}
    req=urllib.request.Request(BASE+r['endpoint']+'/files',data=body,headers=h)
    try:response=urllib.request.urlopen(req,timeout=30)
    except urllib.error.HTTPError as e:response=e
    with response:
        value=json.load(response);assert response.status==expected,(response.status,value)
        return value
def channel(target):
    cfg={'url':'http://mock:8000','mode':'bot','bot_token':credentials['mattermost_token'],'target':'channel','channel_id':target}
    cid=ok('/api/channels',{'name':'File '+target,'kind':'mattermost','config':cfg})['id'];channels.append(cid);return cid
def route(cid,settings=None,fallback=None):
    r=ok('/api/routes',{'name':'Files '+suffix,'channel_ids':[cid],'delivery_settings':settings or {},'fallback_channel_ids':fallback or []});routes.append(r['id']);return r
def rows(r):return ok('/api/deliveries?route_id='+r['id'])['items']
def received():
    with urllib.request.urlopen('http://mock:8000/received',timeout=5) as r:return json.load(r)
start_usage=int(sql('SELECT bytes_used FROM attachment_storage'))
try:
    cid=channel('retryfiles'+suffix[:10]);r=route(cid)
    ok('/api/channels/'+cid+'/pause',{})
    key=uuid.uuid4().hex
    accepted=upload(r,{'title':'Development report','message':'Full text before the XLSX file'},headers={'Idempotency-Key':key})
    attachment=accepted['attachments'][0]
    assert attachment['name']=='report.xlsx' and attachment['sha256']==hashlib.sha256(blob).hexdigest()
    assert rows(r)[0]['attempts']==0
    usage=int(sql('SELECT bytes_used FROM attachment_storage'));assert usage==start_usage+4096
    assert upload(r,{'title':'Development report','message':'Full text before the XLSX file'},headers={'Idempotency-Key':key})['duplicate']
    assert int(sql('SELECT bytes_used FROM attachment_storage'))==usage
    upload(r,{'title':'Development report','message':'Full text before the XLSX file'},files=[('report.xlsx',blob+b'changed')],headers={'Idempotency-Key':key},expected=409)
    detail=ok('/api/events/'+accepted['event_id']);assert detail['attachments'][0]==attachment
    download='/api/events/'+accepted['event_id']+'/attachments/'+attachment['id']
    req=urllib.request.Request(BASE+download,headers={'Authorization':'Bearer '+credentials['admin_token']})
    with urllib.request.urlopen(req,timeout=10) as response:
        assert response.read()==blob and response.headers['Content-Type']=='application/octet-stream'
        assert response.headers['Content-Disposition'].startswith('attachment;') and response.headers['X-Content-Type-Options']=='nosniff'
    try:urllib.request.urlopen(BASE+download);raise AssertionError('Anonymous file download accepted')
    except urllib.error.HTTPError as e:assert e.code==401
    ok('/api/channels/'+cid+'/resume',{},'POST')
    wait(lambda:rows(r)[0]['status']=='sent')
    uploads=[item for item in received() if item.get('path')=='/api/v4/files' and item.get('channel_id')=='retryfiles'+suffix[:10]]
    posts=[json.loads(item['body']) for item in received() if item.get('path')=='/api/v4/posts' and json.loads(item['body']).get('channel_id')=='retryfiles'+suffix[:10]]
    assert len(uploads)==1 and len(posts)==2 and all(p['file_ids']==[hashlib.sha256(blob).hexdigest()[:26]] for p in posts)
    assert uploads[0]['files'][0]['sha256']==hashlib.sha256(blob).hexdigest()
    # Standalone file requests and scheduled files retain their bytes until eligible.
    standalone=upload(r)
    wait(lambda:any(d['event_id']==standalone['event_id'] and d['status']=='sent' for d in rows(r)))
    delayed=upload(r,{'message':'Future file'},headers={'X-Relay-Delay':'3600'})
    assert next(d for d in rows(r) if d['event_id']==delayed['event_id'])['attempts']==0
    # Unknown source JSON fields never imply file downloads.
    plain=ok(r['endpoint'],{'message':'Ordinary source data','attachments':[{'url':'http://127.0.0.1/private'}]},headers={'X-Relay-Key':r['token']})
    assert ok('/api/events/'+plain['event_id'])['attachments']==[]
    # Bounds and unsupported recipients reject atomically before creating an event.
    before=ok('/api/events?route_id='+r['id'])['total']
    upload(r,files=[('../report.xlsx',blob)],expected=400)
    upload(r,files=[('report.xlsx',blob)]*6,expected=400)
    upload(r,files=[('large.xlsx',b'x'*(10*1024*1024+1))],expected=413)
    assert ok('/api/events?route_id='+r['id'])['total']==before
    limit=int(sql('SELECT limit_bytes FROM attachment_storage'))
    usage=int(sql('SELECT bytes_used FROM attachment_storage'))
    try:
        sql('UPDATE attachment_storage SET limit_bytes='+str(usage))
        upload(r,expected=507)
        assert ok('/api/events?route_id='+r['id'])['total']==before
        assert int(sql('SELECT bytes_used FROM attachment_storage'))==usage
    finally:sql('UPDATE attachment_storage SET limit_bytes='+str(limit))
    unsupported=ok('/api/channels',{'name':'Text-only files','kind':'mattermost','config':{'url':'http://mock:8000/plain-files'}})['id'];channels.append(unsupported)
    unsupported_route=route(unsupported);upload(unsupported_route,expected=400)
    assert ok('/api/events?route_id='+unsupported_route['id'])['total']==0
    # A fallback receives exactly the same immutable file after a permanent primary failure.
    fallback=channel('fallbackfiles'+suffix[:10]);primary=channel('forbidden');fallback_route=route(primary,fallback=[fallback])
    forwarded=upload(fallback_route,{'message':'Fallback document'})
    wait(lambda:len(rows(fallback_route))==2 and any(d['channel_id']==fallback and d['status']=='sent' for d in rows(fallback_route)))
    assert ok('/api/events/'+forwarded['event_id'])['attachments'][0]['sha256']==hashlib.sha256(blob).hexdigest()
    # Grouping only combines identical file sets; distinct bytes never disappear into a digest.
    ok('/api/channels/'+cid+'/pause',{})
    grouped_route=route(cid,{'dedup_seconds':300})
    if time.time()%300>295:time.sleep(301-time.time()%300)
    first=upload(grouped_route,{'message':'Same file'})
    second=upload(grouped_route,{'message':'Same file'});assert second['grouped']
    upload(grouped_route,{'message':'Same file'},files=[('report.xlsx',blob+b'other')])
    assert len(rows(grouped_route))==2
    assert ok('/api/events/'+first['event_id'])['delivery_group']['occurrences']==2
    print('PASS: native file uploads preserve binary bytes, private downloads, standalone/scheduled files, acknowledged upload reuse, fallback, bounded storage and file-aware grouping',flush=True)
finally:
    for rid in routes:request('/api/routes/'+rid,method='DELETE')
    for cid in channels:request('/api/channels/'+cid,method='DELETE')
assert int(sql('SELECT bytes_used FROM attachment_storage'))==start_usage
print('PASS: deleting retained events releases attachment storage atomically',flush=True)
