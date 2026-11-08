"""Isolated trusted-proxy and Telegram-slot/lease regressions, using two app processes."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import uuid

ROOT=Path(__file__).resolve().parents[1]
project='relayix-test-084-'+uuid.uuid4().hex[:10]
image=os.environ.get('RELAYIX_TEST_IMAGE','relayix:0.8.8')

def run(args,**kwargs):
    result=subprocess.run(args,capture_output=True,text=True,timeout=100,**kwargs)
    if result.returncode:raise AssertionError('Regression command failed; private output omitted: '+args[0])
    return result.stdout.strip()
def wait(predicate,timeout=40):
    deadline=time.monotonic()+timeout
    while time.monotonic()<deadline:
        result=predicate()
        if result:return result
        time.sleep(.2)
    raise AssertionError('0.8.4 regression condition timed out')

with tempfile.TemporaryDirectory(prefix=project) as folder:
    runtime=Path(folder)
    run(['python3',str(ROOT/'tests/generate_runtime.py'),folder])
    state=runtime/'telegram-state';state.mkdir();state.chmod(0o777)
    env={**os.environ,'RELAYIX_TEST_IMAGE':image,'RELAYIX_TEST_RUNTIME_DIR':folder}
    compose=['docker','compose','--env-file',str(runtime/'compose.env'),'-p',project,'-f',str(ROOT/'tests/compose.yaml'),'-f',str(ROOT/'tests/compose_084.yaml')]
    def dc(*args):return run([*compose,*args],env=env)
    def sql(query):return dc('exec','-T','db','psql','-U','relayix','-d','relayix','-Atc',query)
    def app(code,data=None):return json.loads(run([*compose,'exec','-T','app','python3','-c',code],env=env,input=json.dumps(data) if data is not None else None))
    def started():
        p=state/'started.jsonl'
        return [json.loads(line) for line in p.read_text().splitlines()] if p.exists() else []
    try:
        dc('up','-d','--wait','--wait-timeout','60','db','mock','app')
        # Run in the app network namespace; credentials remain in its environment.
        proxy_test=(ROOT/'tests/proxy_084.py').read_text()
        print(dc('exec','-T','app','python3','-c',proxy_test),flush=True)
        print(dc('exec','-T','app','python3','-c',(ROOT/'tests/proxy_085.py').read_text()),flush=True)
        # Direct callers must not get forwarded-header trust just because another peer is trusted.
        probe="""import json,urllib.request,urllib.error
codes=[]
for index in range(61):
 r=urllib.request.Request('http://app:8080/api/auth/login',data=b'{}',headers={'Content-Type':'application/json','X-Forwarded-For':'203.0.113.'+str(index)})
 try:p=urllib.request.urlopen(r)
 except urllib.error.HTTPError as e:p=e
 codes.append(p.status);p.close()
assert codes[-1]==429 and 429 in codes
print('PASS: untrusted direct peers cannot spoof their quota identity')
"""
        print(dc('exec','-T','mock','python3','-c',probe),flush=True)
        assert 'Ignoring X-Forwarded-For from an untrusted peer' in dc('logs','--no-color','app')
        fixture=app("""import json,os,urllib.request,uuid
def api(path,data,headers=None):
 h={'Content-Type':'application/json','Authorization':'Bearer '+os.environ['ADMIN_TOKEN']};h.update(headers or {})
 return json.load(urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:8080'+path,data=json.dumps(data).encode(),headers=h)))
c=api('/api/channels',{'name':'Held Telegram fixture','kind':'telegram','config':{'bot_token':'123:'+uuid.uuid4().hex,'chat_id':'-100123','api_id':'12345','api_hash':uuid.uuid4().hex,'proxy_type':'mtproto','proxy_host':'mock','proxy_port':'443','proxy_secret':uuid.uuid4().hex}})['id']
r=api('/api/routes',{'name':'Held Telegram lease fixture','channel_ids':[c]})
events=[api(r['endpoint'],{'message':'Lease fixture'},{'X-Relay-Key':r['token']})['event_id'] for _ in range(12)]
print(json.dumps({'channel':c,'route':r['id'],'events':events,'endpoint':r['endpoint'],'token':r['token']}))
""")
        route=fixture['route']
        wait(lambda:sql(f"SELECT COUNT(*) FROM deliveries d JOIN events e ON e.id=d.event_id WHERE e.route_id='{route}' AND d.status='processing'")=='12')
        wait(lambda:len(started())==4)
        claimed=set(sql(f"SELECT d.id FROM deliveries d JOIN events e ON e.id=d.event_id WHERE e.route_id='{route}'").splitlines())
        waiting=next(iter(claimed-{row['id'] for row in started()}))
        sql(f"UPDATE deliveries SET locked_at=NOW()-INTERVAL '90 seconds' WHERE id='{waiting}'")
        aged=time.monotonic()
        dc('up','-d','replica')
        ready_probe="import urllib.request,time\nfor _ in range(60):\n try:urllib.request.urlopen('http://127.0.0.1:8080/healthz',timeout=2);break\n except Exception:time.sleep(.2)\nelse:raise AssertionError('Replica not ready')"
        dc('exec','-T','replica','python3','-c',ready_probe)
        burst_probe="""import urllib.request,urllib.error
codes=[]
for _ in range(1201):
 r=urllib.request.Request('http://127.0.0.1:8080/h/missing-burst-fixture',data=b'{}',headers={'Content-Type':'application/json'})
 try:p=urllib.request.urlopen(r,timeout=5)
 except urllib.error.HTTPError as e:p=e
 codes.append(p.status);p.close()
assert 429 not in codes[:1200] and codes[1200]==429
print('PASS: configured webhook quota admits 1200 requests and rejects the next')
"""
        print(dc('exec','-T','replica','python3','-c',burst_probe),flush=True)
        # After 30 seconds an unrenewed row would be reclaimable by this other process.
        while time.monotonic()-aged<35:
            assert sql(f"SELECT attempts FROM deliveries WHERE id='{waiting}'")=='1'
            assert len(started())==4
            time.sleep(.5)
        assert float(sql(f"SELECT EXTRACT(EPOCH FROM NOW()-locked_at) FROM deliveries WHERE id='{waiting}'"))<25
        # Free each of the first four slots separately. The final four deliveries
        # must wait beyond the old 60-second total timeout without losing attempts.
        first_wave=started()
        for row in first_wave:(state/(row['id']+'.release')).touch()
        wait(lambda:len(started())==8)
        while time.monotonic()-aged<66:
            assert len(started())==8
            assert sql(f"SELECT COUNT(*) FROM deliveries d JOIN events e ON e.id=d.event_id WHERE e.route_id='{route}' AND d.attempts<>1")=='0'
            time.sleep(.5)
        assert sql(f"SELECT COUNT(*) FROM deliveries d JOIN events e ON e.id=d.event_id WHERE e.route_id='{route}' AND d.status='processing'")=='8'
        (state/'release').touch()
        wait(lambda:sql(f"SELECT COUNT(*) FROM deliveries d JOIN events e ON e.id=d.event_id WHERE e.route_id='{route}' AND d.status='sent' AND d.attempts=1")=='12')
        rows=started();assert len(rows)==12 and len({r['id'] for r in rows})==12
        print('PASS: twelve workers/four Telegram slots renew leases, prevent reclaiming, and wait over 60 seconds without consuming a retry',flush=True)
        # Simulate a new owner/attempt. The old owner must cancel its child and cannot commit a result.
        dc('stop','replica')
        (state/'release').unlink()
        event=app("""import json,sys,urllib.request
fixture=json.load(sys.stdin)
r=urllib.request.Request('http://127.0.0.1:8080'+fixture['endpoint'],data=b'{"message":"Ownership fence"}',headers={'Content-Type':'application/json','X-Relay-Key':fixture['token']})
print(json.dumps(json.load(urllib.request.urlopen(r))))
""",fixture)['event_id']
        target=wait(lambda:sql(f"SELECT id FROM deliveries WHERE event_id='{event}' AND status='processing'"))
        wait(lambda:any(row['id']==target for row in started()))
        sql(f"UPDATE deliveries SET attempts=2,locked_at=NOW() WHERE id='{target}'")
        heartbeat=state/(target+'.heartbeat')
        wait(lambda:heartbeat.exists())
        wait(lambda:time.time()-heartbeat.stat().st_mtime>2,30)
        assert not (state/(target+'.finished')).exists()
        assert sql(f"SELECT status||':'||attempts FROM deliveries WHERE id='{target}'")=='processing:2'
        print('PASS: losing an attempt lease cancels its Telegram subprocess and fences its completion',flush=True)
        # Return to the production default and use two real app processes. Backlog
        # on one held Telegram channel must leave capacity for a later healthy channel.
        sql(f"UPDATE deliveries SET status='failed',locked_at=NULL WHERE id='{target}'")
        env['RELAYIX_TEST_CHANNEL_CONCURRENCY']='2'
        env['RELAYIX_TEST_WORKERS']='4'
        dc('up','-d','--force-recreate','--wait','--wait-timeout','60','app')
        dc('up','-d','--force-recreate','replica')
        dc('exec','-T','replica','python3','-c',ready_probe)
        baseline=len(started())
        fair=app("""import json,os,sys,urllib.request
fixture=json.load(sys.stdin)
def api(path,data,headers=None):
 h={'Content-Type':'application/json','Authorization':'Bearer '+os.environ['ADMIN_TOKEN']};h.update(headers or {})
 return json.load(urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:8080'+path,data=json.dumps(data).encode(),headers=h)))
events=[api(fixture['endpoint'],{'message':'Held channel backlog'},{'X-Relay-Key':fixture['token']})['event_id'] for _ in range(8)]
c=api('/api/channels',{'name':'Healthy fairness fixture','kind':'webhook','config':{'url':'http://mock:8000/fairness087'}})['id']
r=api('/api/routes',{'name':'Healthy fairness route','channel_ids':[c]})
healthy=api(r['endpoint'],{'message':'Healthy channel must pass backlog'},{'X-Relay-Key':r['token']})['event_id']
print(json.dumps({'events':events,'healthy':healthy}))
""",fixture)
        alert_ids=[str(uuid.uuid4()) for _ in range(4)]
        for alert in alert_ids:
            sql(f"INSERT INTO alert_deliveries(id,channel_id,title,message) VALUES('{alert}','{fixture['channel']}','Capacity fixture','Shared alert capacity')")
        active=f"SELECT (SELECT COUNT(*) FROM deliveries WHERE channel_id='{fixture['channel']}' AND status='processing')+(SELECT COUNT(*) FROM alert_deliveries WHERE channel_id='{fixture['channel']}' AND status='processing')"
        wait(lambda:sql(active)=='2')
        wait(lambda:len(started())-baseline==2)
        wait(lambda:sql(f"SELECT status FROM deliveries WHERE event_id='{fair['healthy']}'")=='sent',10)
        deadline=time.monotonic()+5
        while time.monotonic()<deadline:
            assert int(sql(active))<=2
            assert len(started())-baseline==2
            time.sleep(.2)
        (state/'release').touch()
        event_list=','.join("'"+event+"'" for event in fair['events'])
        alert_list=','.join("'"+alert+"'" for alert in alert_ids)
        wait(lambda:sql(f"SELECT COUNT(*) FROM deliveries WHERE event_id IN ({event_list}) AND status='sent' AND attempts=1")=='8')
        wait(lambda:sql(f"SELECT COUNT(*) FROM alert_deliveries WHERE id IN ({alert_list}) AND status='sent' AND attempts=1")=='4')
        assert len(started())-baseline==12
        print('PASS: shared two-attempt channel cap across queues/replicas preserves healthy-channel progress and drains without retries',flush=True)
        # Two or more independent held proxies share a separate global budget.
        env['RELAYIX_TEST_PROXY_CONCURRENCY']='2'
        dc('up','-d','--force-recreate','--wait','--wait-timeout','60','app')
        dc('up','-d','--force-recreate','replica')
        dc('exec','-T','replica','python3','-c',ready_probe)
        (state/'release').unlink()
        baseline=len(started())
        proxy_cfg=json.loads(sql(f"SELECT config FROM channels WHERE id='{fixture['channel']}'"))
        pool_fixture=app("""import json,os,sys,urllib.request
cfg=json.load(sys.stdin)
def api(path,data,headers=None):
 h={'Content-Type':'application/json','Authorization':'Bearer '+os.environ['ADMIN_TOKEN']};h.update(headers or {})
 return json.load(urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:8080'+path,data=json.dumps(data).encode(),headers=h)))
channels=[];routes=[];events=[]
for index in range(3):
 c=api('/api/channels',{'name':'Proxy pool '+str(index),'kind':'telegram','config':cfg})['id'];channels.append(c)
 r=api('/api/routes',{'name':'Proxy budget '+str(index),'channel_ids':[c]});routes.append(r)
 events.append(api(r['endpoint'],{'message':'Hold independent proxy'},{'X-Relay-Key':r['token']})['event_id'])
c=api('/api/channels',{'name':'Proxy pool healthy','kind':'webhook','config':{'url':'http://mock:8000/proxy-pool087'}})['id']
r=api('/api/routes',{'name':'Proxy pool healthy','channel_ids':[c]})
healthy=api(r['endpoint'],{'message':'Bypass held proxies'},{'X-Relay-Key':r['token']})['event_id']
quota=api('/api/routes',{'name':'Replica quota','channel_ids':[],'rate_limit_per_minute':1})
print(json.dumps({'channels':channels,'events':events,'healthy':healthy,'quota':quota}))
""",proxy_cfg)
        channel_list=','.join("'"+channel+"'" for channel in pool_fixture['channels'])
        global_active=f"SELECT (SELECT COUNT(*) FROM deliveries WHERE channel_id IN ({channel_list}) AND status='processing')+(SELECT COUNT(*) FROM alert_deliveries WHERE channel_id IN ({channel_list}) AND status='processing')"
        shared_alert=str(uuid.uuid4())
        sql(f"INSERT INTO alert_deliveries(id,channel_id,title,message) VALUES('{shared_alert}','{pool_fixture['channels'][2]}','Proxy budget alert','Share the same budget')")
        wait(lambda:sql(global_active)=='2')
        wait(lambda:len(started())-baseline==2)
        wait(lambda:sql(f"SELECT status FROM deliveries WHERE event_id='{pool_fixture['healthy']}'")=='sent',10)
        until=time.monotonic()+5
        while time.monotonic()<until:
            assert sql(global_active)=='2'
            assert len(started())-baseline==2
            time.sleep(.2)
        (state/'release').touch()
        events=','.join("'"+event+"'" for event in pool_fixture['events'])
        wait(lambda:sql(f"SELECT COUNT(*) FROM deliveries WHERE event_id IN ({events}) AND status='sent' AND attempts=1")=='3')
        wait(lambda:sql(f"SELECT status FROM alert_deliveries WHERE id='{shared_alert}'")=='sent')
        quota_probe="""import json,sys,urllib.request,urllib.error
r=json.load(sys.stdin)
req=urllib.request.Request('http://127.0.0.1:8080'+r['endpoint'],data=b'{"message":"Replica quota"}',headers={'Content-Type':'application/json','X-Relay-Key':r['token']})
try:p=urllib.request.urlopen(req)
except urllib.error.HTTPError as e:p=e
print(json.dumps(p.status));p.close()
"""
        import concurrent.futures
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
            statuses=list(executor.map(lambda service:json.loads(run([*compose,'exec','-T',service,'python3','-c',quota_probe],env=env,input=json.dumps(pool_fixture['quota']))),['app','replica']))
        assert sorted(statuses)==[200,429],statuses
        long_event=app("""import json,sys,urllib.request
r=json.load(sys.stdin)
req=urllib.request.Request('http://127.0.0.1:8080'+r['endpoint'],data=json.dumps({'message':'&😀'*2000}).encode(),headers={'Content-Type':'application/json','X-Relay-Key':r['token']})
print(json.dumps(json.load(urllib.request.urlopen(req))))
""",fixture)['event_id']
        long_id=wait(lambda:sql(f"SELECT id FROM deliveries WHERE event_id='{long_event}' AND status='sent'"))
        sent=next(item for item in started() if item['id']==long_id)
        assert len(sent['text'].encode('utf-16-le'))//2<=4096 and sent['text'].endswith('[Message truncated]')
        assert sent['parse_mode']=='HTML'
        print('PASS: global proxy capacity across channels/queues/replicas, independent healthy workers, cross-replica route quotas and bounded Telegram HTML',flush=True)


    except BaseException:
        try:
            artifacts=Path(os.environ.get('RELAYIX_TEST_ARTIFACTS','/tmp/relayix-084-regression-artifacts'))/project
            artifacts.mkdir(parents=True,exist_ok=True)
            (artifacts/'compose.log').write_text(dc('logs','--no-color'))
            run(['python3',str(ROOT/'tests/redact_artifacts.py'),str(artifacts),str(runtime/'credentials.json')])
        except Exception:
            pass
        raise
    finally:
        dc('down','--volumes','--remove-orphans')
