"""Cross-account authorization, independent grants and revocation through every delivery path."""
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
users, channels, routes = [], [], []


def request(path, data=None, method=None, session=None, headers=None):
    h = {'Authorization': 'Bearer ' + credentials['admin_token'], 'X-Relay-Client': 'web', 'Content-Type': 'application/json'}
    if session:
        h.update({'Authorization': '', 'Cookie': session})
    h.update(headers or {})
    req = urllib.request.Request(BASE + path, data=json.dumps(data).encode() if data is not None else None, method=method, headers=h)
    try:
        response = urllib.request.urlopen(req, timeout=15)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, json.load(response), response.headers


def ok(*args, **kwargs):
    status, value, _ = request(*args, **kwargs)
    assert 200 <= status < 300, 'Access API failed: ' + args[0] + ' status=' + str(status)
    return value


def account(label):
    name = 'acl-' + label + '-' + suffix
    uid = ok('/api/users', {'username': name, 'display_name': label, 'role': 'member', 'password': credentials['operator_password']})['id']
    users.append(uid)
    status, _, headers = request('/api/auth/login', {'username': name, 'password': credentials['operator_password']}, headers={'Authorization': ''})
    assert status == 200
    return uid, headers['Set-Cookie'].split(';')[0]


def sql(query):
    return subprocess.check_output(['psql', '--no-psqlrc', '-v', 'ON_ERROR_STOP=1', '-Atc', query], text=True).strip()


def wait(fn):
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        if fn():
            return
        time.sleep(.2)
    raise AssertionError('Access revocation did not settle')


try:
    alice, a = account('alice')
    bob, b = account('bob')
    spec = {'name': 'Private ' + suffix, 'kind': 'webhook', 'config': {'url': 'http://mock:8000/acl-' + suffix}}
    cid = ok('/api/channels', spec, session=a)['id']; channels.append(cid)
    private = next(c for c in ok('/api/channels') if c['id'] == cid)
    assert private['permissions']['owner_user_id'] == alice
    assert private['permissions']['manage']
    assert not any(c['id'] == cid for c in ok('/api/channels', session=b))
    assert ok('/api/channels/' + cid + '/access', session=a)['owner_user_id'] == alice
    route = ok('/api/routes', {'name': 'Private route ' + suffix, 'channel_ids': [cid]}, session=a); routes.append(route['id'])
    rid = route['id']
    ok('/api/channels/' + cid + '/pause', {}, session=a)
    event = ok(route['endpoint'], {'message': 'Private payload', 'tags': ['acl-secret-' + suffix]}, headers={'X-Relay-Key': route['token']})['event_id']
    delivery = ok('/api/deliveries?event_id=' + event)['items'][0]['id']
    for path, data, method in [
        ('/api/channels/' + cid, spec, 'PUT'),
        ('/api/channels/' + cid, None, 'DELETE'),
        ('/api/channels/' + cid + '/test', {}, 'POST'),
        ('/api/channels/' + cid + '/diagnose', {}, 'POST'),
        ('/api/channels/diagnose', {'id': cid, 'kind': 'webhook', 'config': {'url': '********'}}, 'POST'),
        ('/api/channels/' + cid + '/pause', {}, 'POST'),
        ('/api/channels/' + cid + '/resume', {}, 'POST'),
        ('/api/channels/' + cid + '/access', None, 'GET'),
        ('/api/routes', {'name': 'Steal', 'channel_ids': [cid]}, 'POST'),
        ('/api/routes', {'name': 'Steal fallback', 'channel_ids': [], 'fallback_channel_ids': [cid]}, 'POST'),
        ('/api/routes/' + rid + '/rotate', {}, 'POST'),
        ('/api/routes/' + rid + '/test', {'message': 'Steal'}, 'POST'),
        ('/api/routes/' + rid + '/preview', {'message': 'Steal'}, 'POST'),
        ('/api/routes/' + rid, {'name': 'Steal route', 'channel_ids': []}, 'PUT'),
        ('/api/routes/' + rid, None, 'DELETE'),
        ('/api/events/' + event, None, 'GET'),
        ('/api/events/' + event + '/attachments/' + str(uuid.uuid4()), None, 'GET'),
        ('/api/deliveries/' + delivery + '/attempts', None, 'GET'),
        ('/api/deliveries/' + delivery + '/retry', {}, 'POST'),
    ]:
        assert request(path, data, method, session=b)[0] == 403, path
    assert ok('/api/events?route_id=' + rid, session=b)['total'] == 0
    assert ok('/api/deliveries?event_id=' + event, session=b)['total'] == 0
    assert ok('/api/deliveries?event_id=' + event + '&group_events=true', session=b)['total'] == 0
    assert not any(r['id'] == rid for r in ok('/api/routes', session=b))
    assert 'acl-secret-' + suffix not in ok('/api/tags', session=b)
    assert ok('/api/stats', session=b)['total'] == 0
    assert ok('/api/deliveries/retry-batch/preview', {'filters': {'event_id': event}}, session=b)['count'] == 0

    def grant(view=False, edit=False, send=False):
        return {'owner_user_id': alice, 'grants': [{'user_id': bob, 'can_view': view, 'can_edit': edit, 'can_send': send}]}

    ok('/api/channels/' + cid + '/access', grant(view=True), 'PUT', session=a)
    assert ok('/api/events/' + event, session=b)['message'] == 'Private payload'
    assert next(c for c in ok('/api/channels', session=b) if c['id'] == cid)['config']['url'] == '********'
    assert request('/api/channels/' + cid + '/test', {}, session=b)[0] == 403
    assert request('/api/deliveries/' + delivery + '/retry', {}, session=b)[0] == 403
    assert request('/api/channels/' + cid, spec, 'PUT', session=b)[0] == 403
    other = ok('/api/channels', {**spec, 'name': 'Unshared destination'}, session=a)['id']; channels.append(other)
    mixed = ok('/api/routes', {'name': 'Mixed visibility', 'channel_ids': [cid], 'fallback_channel_ids': [other]}); routes.append(mixed['id'])
    mixed_event = ok(mixed['endpoint'], {'message': 'Private fallback details'}, headers={'X-Relay-Key': mixed['token']})['event_id']
    assert request('/api/events/' + mixed_event, session=b)[0] == 403
    assert ok('/api/deliveries?event_id=' + mixed_event + '&group_events=true', session=b)['total'] == 0
    assert not any(r['id'] == mixed['id'] for r in ok('/api/routes', session=b))
    assert request('/api/channels/' + cid + '/access', grant(send=True), 'PUT', session=b)[0] == 403
    assert request('/api/channels/' + cid + '/access', {'owner_user_id': bob, 'grants': []}, 'PUT', session=a)[0] == 403

    ok('/api/channels/' + cid + '/access', grant(edit=True), 'PUT', session=a)
    ok('/api/channels/' + cid, spec, 'PUT', session=b)
    ok('/api/channels/' + cid + '/pause', {}, session=b)
    assert request('/api/events/' + event, session=b)[0] == 403
    assert request('/api/channels/' + cid + '/test', {}, session=b)[0] == 403
    assert request('/api/channels/' + cid, method='DELETE', session=b)[0] == 403

    ok('/api/channels/' + cid + '/access', grant(send=True), 'PUT', session=a)
    assert next(c for c in ok('/api/channels', session=b) if c['id'] == cid)['config'] == {}
    own_route = ok('/api/routes', {'name': 'Shared sender', 'channel_ids': [cid]}, session=b); routes.append(own_route['id'])
    assert request('/api/routes/' + rid + '/rotate', {}, session=b)[0] == 403
    policy = {'enabled': True, 'destination_ids': [cid], 'watch_route_ids': [], 'trigger_mode': 'final', 'failure_threshold': 1, 'window_minutes': 15, 'cooldown_minutes': 1}
    ok('/api/auth/delivery-alerts', policy, 'PUT', session=b)
    assert ok('/api/auth/delivery-alerts/test', {}, session=b)['queued'] == 1
    own_event = ok(own_route['endpoint'], {'message': 'Queued before revocation'}, headers={'X-Relay-Key': own_route['token']})['event_id']
    assert request('/api/events/' + event, session=b)[0] == 403
    assert request('/api/channels/' + cid + '/pause', {}, session=b)[0] == 403
    # Grant read access temporarily to prepare a bulk retry; revoke before confirmation.
    ok('/api/channels/' + cid + '/access', grant(view=True, send=True), 'PUT', session=a)
    sql(f"UPDATE deliveries SET status='failed',error='Fixture failure' WHERE event_id='{own_event}'")
    batch = ok('/api/deliveries/retry-batch/preview', {'filters': {'event_id': own_event}}, session=b)
    assert batch['count'] == 1
    sql(f"UPDATE deliveries SET status='queued',error=NULL WHERE event_id='{own_event}'")
    ok('/api/channels/' + cid + '/access', {'owner_user_id': alice, 'grants': []}, 'PUT', session=a)
    assert request('/api/deliveries/retry-batch/' + batch['batch_id'], {}, session=b)[0] == 403
    wait(lambda: sql(f"SELECT status FROM deliveries WHERE event_id='{own_event}'") == 'cancelled')
    wait(lambda: sql(f"SELECT COUNT(*) FROM alert_deliveries WHERE owner_user_id='{bob}' AND status='cancelled'") == '1')
    assert request('/api/auth/delivery-alerts/test', {}, session=b)[0] == 403
    ok('/api/auth/delivery-alerts', {**policy, 'enabled': False}, 'PUT', session=b)
    assert ok(own_route['endpoint'], {'message': 'After revocation'}, headers={'X-Relay-Key': own_route['token']})['queued'] == 0
    # Administrators retain universal control and can transfer ownership.
    ok('/api/channels/' + cid + '/access', {'owner_user_id': bob, 'grants': []}, 'PUT')
    assert not any(c['id'] == cid for c in ok('/api/channels', session=a))
    assert ok('/api/channels/' + cid + '/access', session=b)['owner_user_id'] == bob
    ok('/api/users/' + bob, method='DELETE'); users.remove(bob)
    assert next(c for c in ok('/api/channels') if c['id'] == cid)['permissions']['owner_user_id'] == bob
    assert sql(f"SELECT relay_route_send('{own_route['id']}','{cid}')") == 'f'
    assert any(x['action'] == 'channel.access' for x in ok('/api/audit-log?target_id=' + cid)['items'])
finally:
    for rid in routes: request('/api/routes/' + rid, method='DELETE')
    for cid in channels: request('/api/channels/' + cid, method='DELETE')
    for uid in users: request('/api/users/' + uid, method='DELETE')
print('PASS: personal ownership, independent channel grants, private history, confused-deputy protection and queued revocation')
