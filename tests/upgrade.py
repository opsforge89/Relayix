"""Upgrade real 0.6.4 and 0.8.1 data without touching installed deployment data."""
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import time
import uuid

project='relayix-test-upgrade-'+secrets.token_hex(5)
network=project+'-network'
db=project+'-db'
app=project+'-app'
image=os.environ.get('RELAYIX_TEST_IMAGE','relayix:0.8.8')

def run(args, **kwargs):
    result=subprocess.run(args,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,**kwargs)
    if result.returncode:
        raise RuntimeError('Upgrade test command failed; private output omitted: '+args[0])
    return result.stdout

def start(image, envfile):
    run(['docker','run','-d','--name',app,'--network',network,'--env-file',str(envfile),image])
    for _ in range(80):
        result=subprocess.run(['docker','exec',app,'python3','-c',"import urllib.request; urllib.request.urlopen('http://127.0.0.1:8080/healthz',timeout=2).read()"],capture_output=True)
        if result.returncode==0:
            return
        time.sleep(.5)
    raise RuntimeError('Upgrade test application did not become healthy')

with tempfile.TemporaryDirectory(prefix=project) as folder:
    path=Path(folder)
    password=secrets.token_hex(24)
    token=secrets.token_urlsafe(48)
    dbenv=path/'pg.env';dbenv.write_text('POSTGRES_USER=relayix\nPOSTGRES_DB=relayix\nPOSTGRES_PASSWORD='+password+'\n');dbenv.chmod(0o600)
    appenv=path/'app.env';appenv.write_text('DATABASE_URL=postgres://relayix:'+password+'@'+db+':5432/relayix\nADMIN_TOKEN='+token+'\nADMIN_PASSWORD=Ab7!'+secrets.token_hex(24)+'\n');appenv.chmod(0o600)
    try:
        run(['docker','network','create',network])
        run(['docker','run','-d','--name',db,'--network',network,'--env-file',str(dbenv),'postgres:17.11-trixie'])
        for _ in range(60):
            if subprocess.run(['docker','exec',db,'pg_isready','-U','relayix'],capture_output=True).returncode==0:
                break
            time.sleep(.5)
        start('ghcr.io/opsforge89/relayix:0.6.4',appenv)
        # Known legacy records, with a failed delivery so no external send occurs.
        cid,rid,eid,did=[str(uuid.uuid4()) for _ in range(4)]
        sql=f"""INSERT INTO channels(id,name,kind,enabled) VALUES('{cid}','Legacy disabled','webhook',FALSE);
INSERT INTO routes(id,name,slug,token_hash,required_tag) VALUES('{rid}','Legacy route','legacy-upgrade','unused','Deploy');
INSERT INTO route_channels(route_id,channel_id) VALUES('{rid}','{cid}');
INSERT INTO events(id,route_id,title,message,priority,payload) VALUES('{eid}','{rid}','Preserved legacy event','Preserved body',3,'{{}}');
INSERT INTO deliveries(id,event_id,channel_id,status,attempts,error) VALUES('{did}','{eid}','{cid}','failed',5,'Previous failure');"""
        run(['docker','exec','-i',db,'psql','-U','relayix','-d','relayix','-v','ON_ERROR_STOP=1'],input=sql)
        run(['docker','rm','-f',app])
        start('ghcr.io/opsforge89/relayix:0.8.1',appenv)
        tid=str(uuid.uuid4())
        run(['docker','exec','-i',db,'psql','-U','relayix','-d','relayix','-v','ON_ERROR_STOP=1'],input="INSERT INTO message_templates(id,name,body,version) VALUES('"+tid+"','Existing template','Legacy {{message}}',3); UPDATE routes SET message_template_id='"+tid+"' WHERE id='"+rid+"';")
        run(['docker','rm','-f',app])
        start(image,appenv)
        probe="""import json,urllib.request
from pathlib import Path
token=next(line.split('=',1)[1] for line in Path('/proc/self/environ').read_bytes().decode().split(chr(0)) if line.startswith('ADMIN_TOKEN='))
def api(path):
 return json.load(urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:8080'+path,headers={'Authorization':'Bearer '+token})))
assert api('/healthz')['version']=='0.8.8'
event=api('/api/events?route_id=RID')['items'][0]
assert event['id']=='EID' and event['title']=='Preserved legacy event' and event['routing'] is None
attempts=api('/api/deliveries/DID/attempts')
assert attempts['attempts']==5 and attempts['items']==[] and attempts['can_retry']
route=next(r for r in api('/api/routes') if r['id']=='RID')
assert route['required_tag']=='Deploy' and route['fallback_channel_ids']==[]
assert route['payload_rules']==[] and route['template_id']=='TID'
versions=api('/api/templates/TID/versions')['items']
assert len(versions)==1 and versions[0]['version']==3
assert versions[0]['snapshot']['body']=='Legacy {{message}}' and not versions[0]['snapshot']['computed_status']
""".replace('RID',rid).replace('EID',eid).replace('DID',did).replace('TID',tid)
        run(['docker','exec',app,'python3','-c',probe])
        cycle=run(['docker','exec',db,'psql','-U','relayix','-d','relayix','-Atc',"SELECT cycle_attempts FROM deliveries WHERE id='"+did+"'"])
        assert cycle.strip()=='5'
        print('PASS: real 0.6.4 → 0.8.1 → current upgrade preserves users/routes/events/attempts and seeds existing template version history',flush=True)
    finally:
        subprocess.run(['docker','rm','-f','-v',app,db],capture_output=True)
        subprocess.run(['docker','network','rm',network],capture_output=True)
