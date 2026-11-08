"""Real DB-outage readiness and SIGTERM draining in the isolated Compose project."""
import json
import os
from pathlib import Path
import subprocess
import time

project=os.environ['RELAYIX_TEST_PROJECT']
assert project.startswith('relayix-test-')
compose=['docker','compose','--env-file',str(Path(os.environ['RELAYIX_TEST_RUNTIME_DIR'])/'compose.env'),'-p',project,'-f','tests/compose.yaml']
def run(*args):
    result=subprocess.run([*compose,*args],capture_output=True,text=True,timeout=90)
    assert result.returncode==0,'Isolated runtime check command failed; private output omitted'
    return result.stdout.strip()
def sql(query):return run('exec','-T','db','psql','-U','relayix','-d','relayix','-Atc',query)
def health():
    return json.loads(run('exec','-T','app','python3','-c',"import urllib.request,urllib.error,json\ntry:\n r=urllib.request.urlopen('http://127.0.0.1:8080/healthz',timeout=4)\nexcept urllib.error.HTTPError as e:r=e\nprint(json.dumps({'code':r.status,'body':json.load(r)}))"))
if health()['body']['version']=='0.8.2':
    print('SKIP: runtime hardening checks apply to 0.8.3')
    raise SystemExit(0)
# Verify the current built artifact even when its local tag was refreshed during a longer run.
run('up','-d','--no-deps','--wait','--wait-timeout','60','app')
# The isolated DB uses tmpfs: pause preserves its contents while making queries unavailable.
try:
    run('pause','db')
    assert health()['code']==503
finally:
    run('unpause','db')
for _ in range(40):
    if health()['code']==200:break
    time.sleep(.5)
else:raise AssertionError('Database readiness did not recover')
fixture=json.loads(run('exec','-T','app','python3','-c',"""import os,json,urllib.request
def api(path,data):
 r=urllib.request.Request('http://127.0.0.1:8080'+path,data=json.dumps(data).encode(),headers={'Content-Type':'application/json','Authorization':'Bearer '+os.environ['ADMIN_TOKEN']})
 return json.load(urllib.request.urlopen(r))
c=api('/api/channels',{'name':'Shutdown drain','kind':'webhook','config':{'url':'http://mock:8000/slow'}})['id']
r=api('/api/routes',{'name':'Shutdown drain','channel_ids':[c]})
rq=urllib.request.Request('http://127.0.0.1:8080/h/'+r['slug'],data=b'{"message":"Drain this delivery"}',headers={'Content-Type':'application/json','X-Relay-Key':r['token']})
e=json.load(urllib.request.urlopen(rq))['event_id']
print(json.dumps({'channel':c,'route':r['id'],'event':e}))
"""))
try:
    for _ in range(40):
        if sql(f"SELECT status FROM deliveries WHERE event_id='{fixture['event']}'")=='processing':break
        time.sleep(.1)
    else:raise AssertionError('Delivery was never claimed')
    run('stop','-t','75','app')
    container=run('ps','-aq','app')
    code=subprocess.check_output(['docker','inspect',container,'--format','{{.State.ExitCode}}'],text=True).strip()
    assert code=='0',code
    assert sql(f"SELECT status||':'||attempts FROM deliveries WHERE event_id='{fixture['event']}'")=='sent:1'
finally:
    run('start','app')
    sql(f"DELETE FROM routes WHERE id='{fixture['route']}';DELETE FROM channels WHERE id='{fixture['channel']}'")
print('PASS: readiness returns 503 on DB outage, recovers, and SIGTERM drains the active delivery without lease expiry')
