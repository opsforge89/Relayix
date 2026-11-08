"""Runs in app: distributed username backpressure, short recovery and custom webhook bursts."""
import concurrent.futures
import json
import os
import time
import urllib.error
import urllib.request

BASE='http://127.0.0.1:8080'
def request(ip, password='wrong', path='/api/auth/login', token=None, username='  ACCOUNT-THROTTLE  '):
    headers={'Content-Type':'application/json','X-Relay-Client':'web','X-Forwarded-For':ip}
    if token:headers['Authorization']='Bearer '+token
    body={'username':username,'password':password} if path.endswith('login') else {}
    if path=='/api/users':body={'username':'account-throttle','display_name':'Throttle regression','role':'member','password':os.environ['ADMIN_PASSWORD']}
    req=urllib.request.Request(BASE+path,data=json.dumps(body).encode(),headers=headers)
    try:response=urllib.request.urlopen(req,timeout=15)
    except urllib.error.HTTPError as error:response=error
    code=response.status;response.close();return code

assert request('198.51.100.1',path='/api/users',token=os.environ['ADMIN_TOKEN'])==200
for index in range(30):assert request('198.51.100.'+str(index+10))==401
# One verifier is admitted globally, competing IPs see only a short gate.
with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
    codes=list(pool.map(lambda index:request('203.0.113.'+str(index+10)),range(8)))
assert codes.count(401)==1 and codes.count(429)==7,codes
admin=urllib.request.Request(BASE+'/api/auth/login',data=json.dumps({'username':'admin','password':os.environ['ADMIN_PASSWORD']}).encode(),headers={'Content-Type':'application/json','X-Relay-Client':'web','X-Forwarded-For':'203.0.113.60'})
with urllib.request.urlopen(admin,timeout=15) as response:assert response.status==200
# Rejections do not push the gate out: a valid password succeeds in the next slot.
time.sleep(2.1)
assert request('203.0.113.50',os.environ['ADMIN_PASSWORD'])==200
assert request('203.0.113.51')==401
# A fresh distributed burst cannot pre-admit unlimited hash work while waiting
# for the four password slots. Allow bounded in-flight overshoot at the threshold.
with concurrent.futures.ThreadPoolExecutor(max_workers=40) as pool:
    codes=list(pool.map(lambda index:request('198.51.100.'+str(index+100),username='burst-throttle'),range(40)))
assert set(codes)<={401,429} and codes.count(401)<=34 and codes.count(429)>=6,codes
# Only the replica has an increased quota: default /64 regression remains unchanged.
# The harness separately probes the replica from its own namespace.
print('PASS: distributed account failures are throttled atomically, recover after two seconds and reset on valid login',flush=True)
