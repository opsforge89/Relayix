"""Exercise outbound boundaries, headers, concurrency and bounded history cleanup."""
import os
import subprocess
import time
import uuid
import urllib.error
import urllib.request
BASE_URL=os.environ['RELAYIX_TEST_BASE_URL']


def verify(api):
    def sql(statement):
        return subprocess.check_output(['psql','--no-psqlrc','-v','ON_ERROR_STOP=1','-Atc',statement],text=True).strip()
    def wait(predicate, timeout=15):
        deadline=time.monotonic()+timeout
        while time.monotonic()<deadline:
            result=predicate()
            if result:return result
            time.sleep(.2)
        raise AssertionError('0.8.3 condition did not complete')
    for path in ['/', '/healthz', '/api/auth/me', '/missing.js']:
        try: response=urllib.request.urlopen(BASE_URL+path)
        except urllib.error.HTTPError as error: response=error
        assert response.headers['X-Content-Type-Options']=='nosniff'
        assert response.headers['X-Frame-Options']=='DENY'
        assert "frame-ancestors 'none'" in response.headers['Content-Security-Policy']
        response.close()
    for url in ['http://127.0.0.1:8080/healthz','http://localhost:8080/healthz','http://169.254.169.254/latest/meta-data/','http://[::1]:8080','http://[::ffff:127.0.0.1]:8080']:
        code,body=api.request('/api/channels',{'name':'Blocked','kind':'webhook','config':{'url':url}},'POST')
        assert code==400 and 'blocked' in body['error'].lower(),(code,body)
    channels=[];route=None
    try:
        # A pre-existing stored address must also be checked immediately before sending.
        legacy=str(uuid.uuid4());channels.append(legacy)
        sql(f"INSERT INTO channels(id,name,kind,config) VALUES('{legacy}','Legacy private endpoint','webhook','{{\"url\":\"http://127.0.0.1:8080/healthz\"}}')")
        code,body=api.request('/api/channels/'+legacy+'/test',{},'POST')
        assert code==502 and 'blocked' in body['error'].lower()
        for path in ['slow','release083-fast']:
            channels.append(api.ok('/api/channels',{'name':path,'kind':'webhook','config':{'url':'http://mock:8000/'+path}},'POST')['id'])
        route=api.ok('/api/routes',{'name':'083 concurrent','channel_ids':channels[1:]},'POST')
        accepted=api.ok('/h/'+route['slug'],{'message':'Concurrent work'},'POST',{'X-Relay-Key':route['token']})
        event=accepted['event_id']
        wait(lambda:sql(f"SELECT COUNT(*) FROM deliveries WHERE event_id='{event}' AND status='processing'")!='0')
        wait(lambda:sql(f"SELECT status FROM deliveries WHERE event_id='{event}' AND channel_id='{channels[2]}'")=='sent',4)
        assert sql(f"SELECT status FROM deliveries WHERE event_id='{event}' AND channel_id='{channels[1]}'")=='processing'
        wait(lambda:sql(f"SELECT status FROM deliveries WHERE event_id='{event}' AND channel_id='{channels[1]}'")=='sent')
        old,pending=str(uuid.uuid4()),str(uuid.uuid4());tag='retention-'+uuid.uuid4().hex
        sql(f"""INSERT INTO events(id,route_id,title,message,priority,payload,tags,created_at) VALUES
('{old}','{route['id']}','old','old',3,'{{}}','[\"{tag}\"]',NOW()-INTERVAL '90 days'),
('{pending}','{route['id']}','pending','pending',3,'{{}}','[\"{tag}\"]',NOW()-INTERVAL '90 days');
INSERT INTO deliveries(id,event_id,channel_id,status,next_attempt_at) VALUES('{uuid.uuid4()}','{pending}','{channels[2]}','queued',NOW()+INTERVAL '1 day');""")
        # Terminal alert states expire; recent and pending work survive the same sweep.
        alert_ids={name:str(uuid.uuid4()) for name in ['sent','failed','cancelled','recent_cancelled','queued','processing']}
        sql(f"UPDATE channels SET manual_paused=true WHERE id='{channels[2]}'")
        for name,aid in alert_ids.items():
            state='cancelled' if name=='recent_cancelled' else name
            age='NOW()' if name=='recent_cancelled' else "NOW()-INTERVAL '90 days'"
            sql(f"INSERT INTO alert_deliveries(id,channel_id,title,message,status,created_at,next_attempt_at,locked_at) VALUES('{aid}','{channels[2]}','Retention probe','Retain pending work','{state}',{age},NOW()+INTERVAL '1 day',NOW())")
        assert sql(f"SELECT uses FROM event_tags WHERE tag='{tag}'")=='2'
        wait(lambda:sql(f"SELECT COUNT(*) FROM events WHERE id='{old}'")=='0',70)
        assert sql(f"SELECT COUNT(*) FROM events WHERE id='{pending}'")=='1'
        assert sql(f"SELECT uses FROM event_tags WHERE tag='{tag}'")=='1'
        assert tag in api.ok('/api/tags')
        wait(lambda:all(sql(f"SELECT COUNT(*) FROM alert_deliveries WHERE id='{alert_ids[name]}'")=='0' for name in ['sent','failed','cancelled']),70)
        for name in ['recent_cancelled','queued','processing']:
            assert sql(f"SELECT status FROM alert_deliveries WHERE id='{alert_ids[name]}'")==('cancelled' if name=='recent_cancelled' else name)

    finally:
        if route:api.ok('/api/routes/'+route['id'],method='DELETE')
        for channel in channels:api.ok('/api/channels/'+channel,method='DELETE')
    print('PASS: SSRF on create/send, security headers, independent workers, retention/pending preservation and indexed tags',flush=True)


def verify_rates():
    # Spoofed forwarded headers must never change the quota key.
    for path,count in [('/api/auth/login',61),('/h/nonexistent-rate-test',601)]:
        statuses=[]
        for number in range(count):
            request=urllib.request.Request(BASE_URL+path,data=b'{"message":"test"}',headers={'Content-Type':'application/json','X-Forwarded-For':f'192.0.2.{number%255}'})
            try:reply=urllib.request.urlopen(request)
            except urllib.error.HTTPError as error:reply=error
            statuses.append(reply.status)
            if reply.status==429:assert reply.headers['Retry-After']=='60'
            reply.close()
        assert 429 in statuses and statuses[-1]==429
    print('PASS: peer-IP login/webhook quotas, Retry-After and rejection of spoofed forwarded headers',flush=True)


if __name__=='__main__':
    from runtime import version
    if version!='0.8.2':verify_rates()
