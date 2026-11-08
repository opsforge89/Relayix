"""Runs inside app: distinct users behind one trusted TCP peer, including hostile chains."""
import json
import os
import urllib.request
import urllib.error

BASE='http://127.0.0.1:8080'
def request(ip,password='wrong',extra=None,path='/api/auth/login'):
    headers={'Content-Type':'application/json','X-Relay-Client':'web','X-Forwarded-For':ip}
    headers.update(extra or {})
    body={'username':'admin','password':password} if path.endswith('login') else {'message':'rate test'}
    req=urllib.request.Request(BASE+path,data=json.dumps(body).encode(),headers=headers)
    try:response=urllib.request.urlopen(req,timeout=10)
    except urllib.error.HTTPError as error:response=error
    code=response.status;response.close();return code

for _ in range(10):assert request('192.0.2.11')==401
assert request('192.0.2.11',os.environ['ADMIN_PASSWORD'])==429
assert request('192.0.2.12',os.environ['ADMIN_PASSWORD'])==200
assert request('192.0.2.12,192.0.2.11',os.environ['ADMIN_PASSWORD'])==429
assert request('192.0.2.13',os.environ['ADMIN_PASSWORD'],{'X-Real-IP':'192.0.2.11','Forwarded':'for=192.0.2.11'})==200
assert request('garbage')==400
for _ in range(60):assert request('192.0.2.14') in (401,429)
assert request('192.0.2.14',os.environ['ADMIN_PASSWORD'])==429
assert request('192.0.2.15',os.environ['ADMIN_PASSWORD'])==200
# Both the failure window and request quota use /64, not individual interface IDs.
for suffix in range(1,11):assert request('2001:db8:1:1::'+format(suffix,'x'))==401
assert request('2001:db8:1:1::ffff',os.environ['ADMIN_PASSWORD'])==429
assert request('2001:db8:1:2::1',os.environ['ADMIN_PASSWORD'])==200
for suffix in range(1,61):assert request('2001:db8:2:1::'+format(suffix,'x')) in (401,429)
assert request('2001:db8:2:1::ffff',os.environ['ADMIN_PASSWORD'])==429
assert request('2001:db8:2:2::1',os.environ['ADMIN_PASSWORD'])==200
for suffix in range(1,601):assert request('2001:db8:3:1::'+format(suffix,'x'),path='/h/missing-proxy-fixture')!=429
assert request('2001:db8:3:1::ffff',path='/h/missing-proxy-fixture')==429
assert request('2001:db8:3:2::1',path='/h/missing-proxy-fixture')!=429
for suffix in range(60):assert request('192.0.2.16',extra={'X-Real-IP':f'203.0.113.{suffix}'}) in (401,429)
assert request('192.0.2.16',os.environ['ADMIN_PASSWORD'])==429
print('PASS: trusted-proxy login isolation, first-untrusted-hop selection and shared IPv6 /64 quotas/lockouts',flush=True)
