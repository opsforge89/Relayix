"""TTL never revives stale work; audit records successful actions without secrets."""
import json
import os
import subprocess
import time
import urllib.error
import urllib.request
import uuid
from runtime import credentials

BASE = os.environ['RELAYIX_TEST_BASE_URL']
suffix = uuid.uuid4().hex
channels = []
routes = []
users = []


def request(path, data=None, method=None, headers=None):
    h = {'Authorization': 'Bearer ' + credentials['admin_token'], 'Content-Type': 'application/json', 'X-Relay-Client': 'web'}
    h.update(headers or {})
    req = urllib.request.Request(BASE + path, data=json.dumps(data).encode() if data is not None else None, method=method, headers=h)
    try:
        response = urllib.request.urlopen(req, timeout=15)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, json.load(response), response.headers


def ok(*args, **kwargs):
    code, value, _ = request(*args, **kwargs)
    assert 200 <= code < 300, (code, value)
    return value


def sql(statement):
    return subprocess.check_output(['psql', '-XAt', '-v', 'ON_ERROR_STOP=1', '-c', statement], text=True).strip()


def wait(predicate):
    deadline = time.monotonic() + 35
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(.15)
    raise AssertionError('TTL transition timed out')


def route(cid, settings):
    value = ok('/api/routes', {'name': 'TTL ' + suffix, 'channel_ids': [cid], 'delivery_settings': settings})
    routes.append(value['id'])
    return value


def send(r, headers=None, body=None):
    return ok(r['endpoint'], body or {'message': 'TTL regression'}, headers={'X-Relay-Key': r['token'], **(headers or {})})


try:
    channel = ok('/api/channels', {'name': 'TTL controls', 'kind': 'webhook', 'config': {'url': 'http://mock:8000/ttl-' + suffix}})['id']
    channels.append(channel)
    user = ok('/api/users', {'username': 'ttl-' + suffix, 'display_name': 'TTL member', 'role': 'member', 'password': credentials['operator_password']})['id']
    users.append(user)
    code, _, headers = request('/api/auth/login', {'username': 'ttl-' + suffix, 'password': credentials['operator_password']}, headers={'Authorization': ''})
    assert code == 200
    member = {'Authorization': '', 'Cookie': headers.get('Set-Cookie').split(';')[0]}
    assert request('/api/audit-log', headers=member)[0] == 403
    assert request('/api/security/storage', headers=member)[0] == 403
    ok('/api/channels/' + channel + '/pause', {}, headers=member)
    r = route(channel, {'ttl_seconds': 2})
    first = send(r, {'X-Relay-TTL': '120', 'Idempotency-Key': suffix})
    detail = ok('/api/events/' + first['event_id'])
    deadline = detail['expires_at']
    assert deadline and detail['routing']['delivery_policy']['expires_at'] == deadline
    assert send(r, {'X-Relay-TTL': '120', 'Idempotency-Key': suffix})['duplicate']
    assert ok('/api/events/' + first['event_id'])['expires_at'] == deadline
    wait(lambda: sql("SELECT status||':'||attempts FROM deliveries WHERE event_id='" + first['event_id'] + "'") == 'expired:0')
    row = ok('/api/deliveries?event_id=' + first['event_id'])['items'][0]
    assert request('/api/deliveries/' + row['id'] + '/retry', {}, 'POST')[0] == 409
    assert ok('/api/deliveries?status=expired&route_id=' + r['id'])['total'] == 1
    for value in ['0', '-1', '1.5', '5m', '2592001']:
        assert request(r['endpoint'], {'message': 'Invalid TTL'}, headers={'X-Relay-Key': r['token'], 'X-Relay-TTL': value})[0] == 400
    grouped = route(channel, {'ttl_seconds': 1, 'dedup_seconds': 300})
    group1 = send(grouped)
    group2 = send(grouped)
    assert group1['event_id'] != group2['event_id']
    wait(lambda: sql("SELECT status FROM deliveries WHERE event_id='" + group1['event_id'] + "'") == 'expired')
    assert sql("SELECT COUNT(*) FROM events WHERE route_id='" + grouped['id'] + "'") == '2'
    delayed = route(channel, {})
    late = send(delayed, {'X-Relay-Delay': '3600', 'X-Relay-TTL': '1'})
    wait(lambda: sql("SELECT status FROM deliveries WHERE event_id='" + late['event_id'] + "'") == 'expired')
    # No expired failure can be revived through a preview prepared before its deadline.
    future = send(delayed, {'X-Relay-TTL': '30'})
    sql("UPDATE deliveries SET status='failed' WHERE event_id='" + future['event_id'] + "'")
    batch = ok('/api/deliveries/retry-batch/preview', {'filters': {'event_id': future['event_id']}})
    sql("UPDATE deliveries SET expires_at=NOW()-INTERVAL '1 second' WHERE event_id='" + future['event_id'] + "'")
    result = ok('/api/deliveries/retry-batch/' + batch['batch_id'], {}, 'POST')
    assert result['queued'] == 0 and result['skipped'] == 1
    # File-only messages keep their original attachment even after scheduled expiry.
    file_channel = ok('/api/channels', {'name': 'TTL file destination', 'kind': 'mattermost', 'config': {'url': 'http://mock:8000', 'mode': 'bot', 'bot_token': credentials['mattermost_token'], 'target': 'channel', 'channel_id': 'ttlfiles' + suffix[:10]}})['id']
    channels.append(file_channel)
    file_route = route(file_channel, {})
    boundary = 'ttl' + suffix
    file_body = b'PK\x03\x04\x00\xfffile'
    multipart = b'--' + boundary.encode() + b'\r\nContent-Disposition: form-data; name="files"; filename="expired.xlsx"\r\nContent-Type: application/octet-stream\r\n\r\n' + file_body + b'\r\n--' + boundary.encode() + b'--\r\n'
    req = urllib.request.Request(BASE + file_route['endpoint'] + '/files', data=multipart, headers={'X-Relay-Key': file_route['token'], 'X-Relay-TTL': '1', 'X-Relay-Delay': '3600', 'Content-Type': 'multipart/form-data; boundary=' + boundary})
    with urllib.request.urlopen(req, timeout=15) as response:
        file_event = json.load(response)
    wait(lambda: sql("SELECT status||':'||attempts FROM deliveries WHERE event_id='" + file_event['event_id'] + "'") == 'expired:0')
    assert ok('/api/events/' + file_event['event_id'])['attachments'][0]['sha256'] == __import__('hashlib').sha256(file_body).hexdigest()
    ok('/api/channels/' + channel + '/resume', {}, 'POST', headers=member)
    healthy = send(delayed, body={'message': 'Healthy traffic', 'ttl': 'source-specific 5m'})
    wait(lambda: sql("SELECT status FROM deliveries WHERE event_id='" + healthy['event_id'] + "'") == 'sent')
    token = ok('/api/routes/' + r['id'] + '/rotate', {}, 'POST')
    actions = ok('/api/audit-log?target_id=' + channel)['items']
    assert any(row['action'] == 'channel.pause' and row['actor_id'] == user for row in actions)
    assert any(row['action'] == 'channel.resume' and row['actor_id'] == user for row in actions)
    journal = json.dumps(ok('/api/audit-log'))
    assert credentials['admin_token'] not in journal and credentials['operator_password'] not in journal and r['token'] not in journal
    assert token['token'] not in journal
    print('PASS: paused/grouped/scheduled TTL, terminal retries, route-bound deadlines and private actor audit')
finally:
    for rid in routes:
        request('/api/routes/' + rid, method='DELETE')
    for cid in channels:
        request('/api/channels/' + cid, method='DELETE')
    for uid in users:
        request('/api/users/' + uid, method='DELETE')
