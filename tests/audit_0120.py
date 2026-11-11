"""Account/session audit identities, privacy and bounded failed sign-in records."""
import json
import os
import urllib.error
import urllib.request
import uuid
from runtime import credentials

BASE = os.environ['RELAYIX_TEST_BASE_URL']
name = 'audit-' + uuid.uuid4().hex


def request(path, data=None, method=None, headers=None):
    h = {'Authorization': 'Bearer ' + credentials['admin_token'], 'X-Relay-Client': 'web', 'Content-Type': 'application/json'}
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
    assert 200 <= status < 300, 'Audit regression API failed; private output omitted'
    return value


user = ok('/api/users', {'username': name, 'display_name': 'Audit account', 'role': 'member', 'password': credentials['operator_password']})['id']
try:
    status, _, h = request('/api/auth/login', {'username': name, 'password': credentials['operator_password']}, headers={'Authorization': ''})
    assert status == 200
    session = {'Authorization': '', 'Cookie': h['Set-Cookie'].split(';')[0]}
    ok('/api/auth/profile', {'display_name': 'Updated audit account', 'current_password': credentials['operator_password'], 'new_password': credentials['operator_new_password']}, 'PUT', headers=session)
    ok('/api/auth/logout', {}, 'POST', headers=session)
    assert request('/api/auth/me', headers=session)[0] == 401
    for _ in range(3):
        assert request('/api/auth/login', {'username': name, 'password': credentials['operator_password']}, headers={'Authorization': ''})[0] == 401
    actions = ok('/api/audit-log?target_id=' + user)['items']
    assert sum(item['action'] == 'auth.login_failed' for item in actions) == 1
    assert any(item['action'] == 'auth.login' and item['actor_id'] == user for item in actions)
    assert any(item['action'] == 'auth.logout' and item['actor_id'] == user for item in actions)
    assert any(item['action'] == 'user.password_change' and item['actor_id'] == user for item in actions)
    assert any(item['action'] == 'user.profile' for item in actions)
    assert any(item['action'] == 'user.create' and item['auth_method'] == 'token' for item in actions)
    ok('/api/users/' + user, {'username': name, 'display_name': 'Admin edit', 'role': 'admin', 'enabled': False}, 'PUT')
    assert any(item['action'] == 'user.update' for item in ok('/api/audit-log?target_id=' + user)['items'])
    serialized = json.dumps(actions)
    assert credentials['operator_password'] not in serialized and credentials['operator_new_password'] not in serialized and session['Cookie'] not in serialized
finally:
    ok('/api/users/' + user, method='DELETE')
assert any(item['action'] == 'user.delete' for item in ok('/api/audit-log?target_id=' + user)['items'])
print('PASS: account/password/session audit, deleted-target retention, secret exclusion and bounded failed sign-ins')
