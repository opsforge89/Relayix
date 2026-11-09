"""Additive schema changes preserve result shapes on existing application connections."""
from concurrent.futures import ThreadPoolExecutor
import json
import os
import subprocess
import urllib.request
import uuid
from runtime import credentials

base = os.environ['RELAYIX_TEST_BASE_URL']
column = 'cached_plan_probe_' + uuid.uuid4().hex
tables = ['channels', 'message_templates', 'alert_policies']
paths = ['/api/channels', '/api/templates', '/api/alert-policies']

def read(path):
    request = urllib.request.Request(base + path, headers={
        'Authorization': 'Bearer ' + credentials['admin_token'],
        'X-Relay-Client': 'web',
    })
    with urllib.request.urlopen(request, timeout=15) as response:
        assert response.status == 200
        assert isinstance(json.load(response), list)

def sql(statement):
    subprocess.run(['psql', '--no-psqlrc', '-v', 'ON_ERROR_STOP=1', '-c', statement],
                   check=True, stdout=subprocess.DEVNULL)

def exercise():
    # Fill the application's existing pool and prepared-statement caches.
    with ThreadPoolExecutor(max_workers=10) as pool:
        list(pool.map(read, paths * 20))

added = False
try:
    exercise()
    sql('BEGIN; ' + ' '.join('ALTER TABLE ' + table + ' ADD COLUMN ' + column + ' TEXT;'
                            for table in tables) + ' COMMIT;')
    added = True
    exercise()
    print('PASS: additive DDL preserves cached channel/template/alert result shapes on live application connections', flush=True)
finally:
    if added:
        sql('BEGIN; ' + ' '.join('ALTER TABLE ' + table + ' DROP COLUMN ' + column + ';'
                                for table in tables) + ' COMMIT;')
