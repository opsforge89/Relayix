"""Fail-fast startup messages omit private connection values."""
import os
import secrets
import subprocess

image=os.environ.get('RELAYIX_TEST_IMAGE','relayix:0.8.3')
private=secrets.token_urlsafe(28)
base={'DATABASE_MODE':'external','POSTGRES_HOST':'127.0.0.1','POSTGRES_PORT':'1','POSTGRES_PASSWORD':private}
for override,expected in [({'OUTBOUND_ALLOWLIST':'localhost'},'OUTBOUND_ALLOWLIST'),({'DELIVERY_WORKERS':'0'},'DELIVERY_WORKERS'),({'EVENT_RETENTION_DAYS':'-1'},'EVENT_RETENTION_DAYS'),({'PORT':'bad'},'PORT'),({'COOKIE_SECURE':'yes'},'COOKIE_SECURE'),({'RETRY_BASE_SECONDS':'-1'},'RETRY_BASE_SECONDS'),({'TELEGRAM_API_ID':'123'},'TELEGRAM_API_HASH'),({'TELEGRAM_API_HASH':secrets.token_hex(16)},'TELEGRAM_API_ID'),({'POSTGRES_SSLROOTCERT':'/missing/ca.pem'},'POSTGRES_SSLROOTCERT'),({},'PostgreSQL connection failed')]:
    env={**os.environ,**base,**override}
    keys=list(base)+[k for k in override if k not in base]
    args=['docker','run','--rm','--network','none']
    for key in keys:
        args.extend(['--env',key])
    result=subprocess.run(args+[image],env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,timeout=25)
    assert result.returncode!=0 and expected in result.stdout,(expected,result.stdout)
    assert private not in result.stdout
print('PASS: actionable startup validation and connection failures without credentials')
