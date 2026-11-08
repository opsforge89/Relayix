"""Exercise documented startup and database tools in isolated Compose projects."""
from pathlib import Path
from urllib.request import build_opener,ProxyHandler,Request
import json,os,secrets,socket,subprocess,sys,tempfile,time
ROOT=Path(__file__).resolve().parents[1]
IMAGE=os.environ.get('RELAYIX_TEST_IMAGE','relayix:0.8.5')
PG_IMAGE='postgres:17.11-trixie'

def free_port():
 with socket.socket() as sock:
  sock.bind(('127.0.0.1',0));return str(sock.getsockname()[1])

def exercise(mode,folder):
 project='relayix-test-database-'+mode+'-'+secrets.token_hex(4)
 assert project.startswith('relayix-test-')
 config_path=folder/(mode+'.json')
 subprocess.run([sys.executable,str(ROOT/'scripts/write_config.py'),str(config_path),'--database','local' if mode=='local' else 'external','--generate'],capture_output=True,check=True)
 config=json.loads(config_path.read_text())
 config.update(COMPOSE_PROJECT_NAME=project,BIND_ADDRESS='127.0.0.1',PORT=free_port())
 config['POSTGRES_PASSWORD']=secrets.token_hex(24)+'@:#%/\'"$'
 config['POSTGRES_USER']='relayix';config['POSTGRES_DB']='relayix'
 network=project+'-network';server=project+'-postgres'
 override=folder/(mode+'-network.yaml')
 external_created=False;network_created=False
 if mode!='local':
  config.update(POSTGRES_HOST='external-postgres',POSTGRES_PORT='5432',POSTGRES_SSLMODE='disable')
 if mode=='external-tls':
  certs=folder/'certs';certs.mkdir()
  def openssl(arguments):subprocess.run(['openssl',*arguments],cwd=certs,capture_output=True,check=True)
  openssl(['req','-x509','-newkey','rsa:2048','-nodes','-keyout','ca.key','-out','ca.crt','-days','1','-subj','/CN=Relayix test CA'])
  openssl(['req','-newkey','rsa:2048','-nodes','-keyout','server.key','-out','server.csr','-subj','/CN=external-postgres'])
  (certs/'extensions.txt').write_text('basicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:external-postgres\n')
  openssl(['x509','-req','-in','server.csr','-CA','ca.crt','-CAkey','ca.key','-CAcreateserial','-out','server.crt','-days','1','-extfile','extensions.txt'])
  (certs/'ca.crt').chmod(0o644)
  config.update(POSTGRES_SSLMODE='verify-full',POSTGRES_SSLROOTCERT=str(certs/'ca.crt'))
 config_path.write_text(json.dumps(config));config_path.chmod(0o600)
 env={k:v for k,v in os.environ.items() if k not in ['RELAYIX_IMAGE_OVERRIDE','RELAYIX_COMPOSE_PROJECT','RELAYIX_CONFIG_FILE']}
 env['RELAYIX_CONFIG_FILE']=str(config_path)
 env['RELAYIX_IMAGE_OVERRIDE']=IMAGE
 args=[str(ROOT/'scripts/compose.sh')]
 if mode!='local':
  override.write_text('networks:\n  default:\n    ipam: !reset null\n    external: true\n    name: '+network+'\n')
  args+=['-f',str(override)]
 def compose(tail,**kwargs):return subprocess.run(args+tail,cwd=ROOT,env=env,capture_output=True,check=True,**kwargs)
 try:
  if mode!='local':
   subprocess.run(['docker','network','create',network],capture_output=True,check=True);network_created=True
   pg_env={**os.environ,**{key:config[key] for key in ['POSTGRES_USER','POSTGRES_DB','POSTGRES_PASSWORD']}}
   command=['docker','run','-d','--name',server,'--network',network,'--network-alias','external-postgres','-e','POSTGRES_USER','-e','POSTGRES_DB','-e','POSTGRES_PASSWORD']
   if mode=='external-tls':
    command+=['-v',str(certs)+':/tls-source:ro','--entrypoint','sh',PG_IMAGE,'-c','mkdir -p /tmp/db-tls; cp /tls-source/server.key /tls-source/server.crt /tmp/db-tls/; chown -R postgres:postgres /tmp/db-tls; chmod 600 /tmp/db-tls/server.key; exec docker-entrypoint.sh postgres -c ssl=on -c ssl_cert_file=/tmp/db-tls/server.crt -c ssl_key_file=/tmp/db-tls/server.key']
   else:command+=[PG_IMAGE]
   subprocess.run(command,env=pg_env,capture_output=True,check=True);external_created=True
   for attempt in range(60):
    ready=subprocess.run(['docker','exec',server,'pg_isready','-U','relayix','-d','relayix'],capture_output=True)
    if ready.returncode==0:break
    time.sleep(.5)
   else:raise AssertionError('External PostgreSQL did not start')
  compose(['up','-d'])
  opener=build_opener(ProxyHandler({}));base='http://127.0.0.1:'+config['PORT']
  for attempt in range(60):
   try:
    with opener.open(base+'/healthz',timeout=2) as response:health=json.load(response)
    if health.get('status')=='ok':break
   except Exception:pass
   time.sleep(.5)
  else:raise AssertionError(mode+' application did not start')
  with opener.open(Request(base+'/api/channels',headers={'Authorization':'Bearer '+config['ADMIN_TOKEN']}),timeout=5) as response:assert json.load(response)==[]
  raw=compose(['ps','--format','json']).stdout.decode()
  rows=json.loads(raw) if raw.lstrip().startswith('[') else [json.loads(line) for line in raw.splitlines() if line.strip()]
  # Compose may emit one JSON object per line rather than an array.
  services={row['Service'] for row in rows}
  assert services==({'app','db'} if mode=='local' else {'app'}),services
  query=subprocess.run([sys.executable,str(ROOT/'scripts/database.py'),'psql','-X','-Atc','SELECT current_database() || chr(58) || current_user;'],cwd=ROOT,env=env,capture_output=True,check=True)
  assert query.stdout.strip()==b'relayix:relayix'
  dump=subprocess.run([sys.executable,str(ROOT/'scripts/database.py'),'pg_dump','-Fc'],cwd=ROOT,env=env,capture_output=True,check=True)
  assert dump.stdout.startswith(b'PGDMP')
  print('PASS: '+mode+' startup, authenticated API, expected services, special-character DB password, SQL client and pg_dump',flush=True)
 finally:
  subprocess.run(args+['down','--volumes','--remove-orphans'],cwd=ROOT,env=env,capture_output=True)
  if external_created:subprocess.run(['docker','rm','-f',server],capture_output=True)
  if network_created:subprocess.run(['docker','network','rm',network],capture_output=True)

with tempfile.TemporaryDirectory(prefix='relayix-database-modes-') as temporary:
 for mode in ['local','external','external-tls']:exercise(mode,Path(temporary))
