"""Isolated contract tests: never sends to Telegram/Pushover or other real services."""
import json
import os
from runtime import credentials,version as EXPECTED_VERSION
import time
import subprocess
from email import policy
from email.parser import Parser
from http.cookiejar import CookieJar
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode
from urllib.request import Request, urlopen, build_opener, HTTPCookieProcessor

BASE = os.environ['RELAYIX_TEST_BASE_URL']
TOKEN = credentials['admin_token']
ADMIN_PASSWORD = credentials['admin_password']


class Client:
    def __init__(self, token=None):
        self.token = token
        self.cookies = CookieJar()
        self.opener = build_opener(HTTPCookieProcessor(self.cookies))
        self.last_headers = None

    def request(self, path, data=None, method=None, headers=None):
        h = {'Content-Type': 'application/json', 'X-Relay-Client': 'web'}
        if self.token:
            h['Authorization'] = f'Bearer {self.token}'
        h.update(headers or {})
        request = Request(BASE + path, data=json.dumps(data).encode() if data is not None else None, method=method, headers=h)
        try:
            with self.opener.open(request, timeout=35) as response:
                self.last_headers = response.headers
                return response.status, json.load(response)
        except HTTPError as error:
            self.last_headers = error.headers
            return error.code, json.load(error)

    def ok(self, path, data=None, method=None, headers=None):
        status, result = self.request(path, data, method, headers)
        assert status == 200, (path, status, result)
        return result

    def login(self, username, password):
        return self.ok('/api/auth/login', {'username': username, 'password': password}, 'POST')


def wait_for(fn, message, seconds=90):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = fn()
        if result:
            return result
        time.sleep(.5)
    raise AssertionError(message)


def main():
    api = Client(TOKEN)
    for _ in range(90):
        try:
            if api.ok('/healthz')['version'] == EXPECTED_VERSION:
                break
        except (URLError, AssertionError):
            time.sleep(1)
    else:
        raise AssertionError('Application did not start')
    assert Client().request('/api/channels')[0] == 401
    assert api.request('/api/channels', headers={'Authorization': 'Bearer invalid'})[0] == 401
    assert api.ok('/api/templates') == []
    assert Client().request('/api/templates')[0] == 401
    assert api.last_headers['Cache-Control'] == 'no-store'
    admin = Client()
    assert admin.request('/api/auth/login', {'username': 'admin', 'password': 'wrong'}, 'POST')[0] == 401
    admin_user = admin.login('admin', ADMIN_PASSWORD)
    assert admin_user['role'] == 'admin' and admin_user['auth_method'] == 'password'
    assert 'HttpOnly' in admin.last_headers['Set-Cookie'] and 'SameSite=Strict' in admin.last_headers['Set-Cookie']
    assert admin.request('/api/channels', headers={'X-Relay-Client': ''})[0] == 401
    assert admin.ok('/api/auth/me')['id'] == admin_user['id']
    assert 'password_hash' not in json.dumps(admin.ok('/api/users'))
    assert api.request('/api/users/' + admin_user['id'], method='DELETE')[0] == 400
    user_input = {'username': 'operator', 'display_name': 'Test operator', 'role': 'member', 'enabled': True, 'password': 'weak'}
    assert admin.request('/api/users', user_input, 'POST')[0] == 400
    user_input['password'] = credentials['operator_password']
    operator_id = admin.ok('/api/users', user_input, 'POST')['id']
    assert admin.request('/api/users', user_input, 'POST')[0] == 409
    operator = Client()
    operator.login('operator', user_input['password'])
    operator.ok('/api/channels')
    assert operator.request('/api/users')[0] == 403
    assert operator.request('/api/alert-policies')[0] == 403
    other_session = Client()
    other_session.login('operator', user_input['password'])
    profile = {'display_name': 'Operator renamed', 'current_password': user_input['password'], 'new_password': credentials['operator_new_password']}
    assert operator.request('/api/auth/profile', {**profile, 'new_password': 'weak'}, 'PUT')[0] == 400
    assert operator.request('/api/auth/profile', {**profile, 'current_password': 'wrong'}, 'PUT')[0] == 400
    operator.ok('/api/auth/profile', profile, 'PUT')
    assert operator.ok('/api/auth/me')['display_name'] == 'Operator renamed'
    assert other_session.request('/api/auth/me')[0] == 401
    assert Client().request('/api/auth/login', {'username': 'operator', 'password': user_input['password']}, 'POST')[0] == 401
    operator.ok('/api/auth/logout', {}, 'POST')
    assert operator.request('/api/auth/me')[0] == 401
    operator.login('operator', profile['new_password'])
    admin.ok('/api/users/' + operator_id, {**user_input, 'password': None, 'enabled': False}, 'PUT')
    assert operator.request('/api/auth/me')[0] == 401
    assert Client().request('/api/auth/login', {'username': 'operator', 'password': profile['new_password']}, 'POST')[0] == 401
    admin.ok('/api/users/' + operator_id, {**user_input, 'password': None}, 'PUT')
    limited = Client()
    for _ in range(10):
        assert limited.request('/api/auth/login', {'username': 'unknown-user', 'password': 'wrong'}, 'POST')[0] == 401
    assert limited.request('/api/auth/login', {'username': 'unknown-user', 'password': 'wrong'}, 'POST')[0] == 429
    print('PASS: login/password, strength checks, admin/member permissions, profiles, session revocation, disabled users, login throttling', flush=True)

    defaults = admin.ok('/api/auth/me')['preferences']
    assert defaults['notification_timeout_seconds'] == 8
    assert defaults['theme'] == 'light' and defaults['refresh_interval_seconds'] == 15
    saved = {**defaults, 'notification_timeout_seconds': 12, 'theme': 'dark', 'refresh_interval_seconds': 5, 'page_size': 10, 'route_tag_filter': 'deploy', 'list_search': 'production', 'delivery_filters': {**defaults['delivery_filters'], 'tag': 'deploy', 'status': 'failed'}}
    admin.ok('/api/auth/preferences', saved, 'PUT')
    second_admin = Client()
    second_admin.login('admin', ADMIN_PASSWORD)
    assert second_admin.ok('/api/auth/me')['preferences'] == saved
    operator.login('operator', profile['new_password'])
    assert operator.ok('/api/auth/me')['preferences'] == defaults
    assert api.ok('/api/auth/me')['preferences'] == defaults
    api.ok('/api/auth/preferences', {**defaults, 'theme': 'dark', 'refresh_interval_seconds': 0}, 'PUT')
    assert admin.ok('/api/auth/me')['preferences'] == saved
    for invalid in [{'theme': 'invalid'}, {'refresh_interval_seconds': 1}, {'refresh_interval_seconds': 3601}, {'page_size': 999}, {'notification_timeout_seconds': -1}, {'notification_timeout_seconds': 301}]:
        assert admin.request('/api/auth/preferences', {**defaults, **invalid}, 'PUT')[0] == 400
    admin.ok('/api/auth/preferences', defaults, 'PUT')
    api.ok('/api/auth/preferences', defaults, 'PUT')
    print('PASS: account preferences persist across sessions, remain isolated between users/token identity, and validate interval/page limits', flush=True)


    configs = {
        'matrix': {'url': 'http://mock:8000', 'room_id': '!room:example.org', 'access_token': credentials['matrix_token']},
        'smtp': {'host': 'mock', 'port': '1025', 'tls': 'none', 'from': 'alerts@example.org', 'from_name': 'Relayix Alerts \u2709', 'to': 'ops@example.org'},
        'ntfy': {'url': 'http://mock:8000', 'topic': 'alerts'},
        'gotify': {'url': 'http://mock:8000', 'token': credentials['gotify_token']},
    }
    for kind in ['mattermost', 'rocket_chat', 'slack', 'discord', 'teams', 'google_chat', 'webhook']:
        configs[kind] = {'url': f'http://mock:8000/{kind}'}
    ids = {}
    for kind, config in configs.items():
        cid = api.ok('/api/channels', {'name': kind, 'kind': kind, 'config': config, 'enabled': True}, 'POST')['id']
        ids[kind] = cid
        api.ok(f'/api/channels/{cid}/test', {}, 'POST')
    for channel in api.ok('/api/channels'):
        for field in ['url', 'access_token', 'token', 'password']:
            if field in channel['config']:
                assert channel['config'][field] == '********'
        api.ok('/api/channels/' + channel['id'], channel, 'PUT')
        api.ok('/api/channels/' + channel['id'] + '/test', {}, 'POST')
    auth_id = api.ok('/api/channels', {'name': 'Authenticated SMTP', 'kind': 'smtp', 'config': {**configs['smtp'], 'username': credentials['smtp_username'], 'password': credentials['smtp_password']}}, 'POST')['id']
    masked = next(c for c in api.ok('/api/channels') if c['id'] == auth_id)
    assert masked['config']['password'] == '********'
    api.ok('/api/channels/' + auth_id, masked, 'PUT')
    api.ok('/api/channels/' + auth_id + '/test', {}, 'POST')
    assert api.request('/api/channels', {'name': 'Bad topic', 'kind': 'telegram', 'config': {'bot_token': credentials['telegram_token'], 'chat_id': '-100123', 'message_thread_id': 'bad'}}, 'POST')[0] == 400
    telegram = api.ok('/api/channels', {'name': 'Telegram topic', 'kind': 'telegram', 'config': {'bot_token': credentials['telegram_token'], 'chat_id': '-100123', 'message_thread_id': '42'}}, 'POST')['id']
    saved_telegram = next(c for c in api.ok('/api/channels') if c['id'] == telegram)
    assert saved_telegram['config']['message_thread_id'] == '42'
    encrypted = api.ok('/api/channels', {'name': 'Encrypted', 'kind': 'matrix', 'config': {**configs['matrix'], 'room_id': '!encrypted:example.org'}}, 'POST')['id']
    status, result = api.request(f'/api/channels/{encrypted}/test', {}, 'POST')
    assert status == 502 and 'Encrypted Matrix rooms' in result['error']
    print('PASS: 11 local adapters, masked edits, SMTP authentication, Matrix protection and Telegram topic configuration', flush=True)

    route = api.ok('/api/routes', {'name': 'Smoke', 'slug': 'smoke', 'channel_ids': list(ids.values())}, 'POST')
    event = api.ok('/h/smoke', {'title': 'Ready', 'message': 'Deployment successful', 'priority': 3, 'tags': ['deploy', 'ops']}, 'POST', {'X-Relay-Key': route['token']})
    assert event['queued'] == len(ids)
    def all_sent():
        rows = api.ok('/api/deliveries')['items']
        rows = [d for d in rows if d['event_id'] == event['event_id']]
        return rows if len(rows) == len(ids) and all(d['status'] == 'sent' for d in rows) else False
    deliveries = wait_for(all_sent, 'Queue did not deliver to all local adapters')
    assert api.ok('/api/deliveries?' + urlencode({'status': 'sent', 'channel_id': ids['smtp'], 'route_id': route['id'], 'search': 'Ready'}))['total'] == 1
    assert api.ok('/api/deliveries?from=2099-01-01T00:00:00Z')['total'] == 0
    assert api.request('/api/deliveries?from=2099-01-01T00:00:00Z&to=2000-01-01T00:00:00Z')[0] == 400
    assert api.request('/api/deliveries?status=unknown')[0] == 400
    first = api.ok('/api/deliveries?page_size=3&page=1')
    second = api.ok('/api/deliveries?page_size=3&page=2')
    assert first['total'] == len(ids) and len(first['items']) == len(second['items']) == 3
    assert not ({d['id'] for d in first['items']} & {d['id'] for d in second['items']})
    assert api.request('/api/deliveries/' + deliveries[0]['id'] + '/retry', {}, 'POST')[0] == 409
    print('PASS: shared routes/queue, server-side status/channel/route/date/search filters and pagination; optional templates authenticated', flush=True)

    assert all(d['tags'] == ['deploy', 'ops'] for d in deliveries)
    assert api.ok('/api/deliveries?tag=deploy')['total'] == len(ids)
    assert api.ok('/api/deliveries?tag=dep')['total'] == 0
    assert api.ok('/api/deliveries?tag=unknown')['total'] == 0
    assert api.request('/api/deliveries?' + urlencode({'tag': 'x' * 65}))[0] == 400
    auto_input = {'name': 'Production notifications', 'channel_ids': [], 'required_tag': 'route-tag'}
    auto = api.ok('/api/routes', auto_input, 'POST')
    other_auto = api.ok('/api/routes', auto_input, 'POST')
    assert auto['slug'] != other_auto['slug'] and auto['endpoint'] == '/h/' + auto['slug']
    api.ok('/api/routes/' + auto['id'], {**auto_input, 'name': 'Renamed notifications'}, 'PUT')
    existing = next(r for r in api.ok('/api/routes') if r['id'] == auto['id'])
    assert existing['slug'] == auto['slug']
    assert api.request('/api/routes/' + auto['id'], {**auto_input, 'slug': 'change-endpoint'}, 'PUT')[0] == 400
    rotated = api.ok('/api/routes/' + auto['id'] + '/rotate', {}, 'POST')
    assert rotated['endpoint'] == auto['endpoint'] and rotated['token'] != auto['token']
    assert {'deploy', 'ops', 'route-tag'} <= set(api.ok('/api/tags'))
    print('PASS: complete delivery tags, exact tag filtering, suggested tags, automatic unique endpoints and stable endpoints on rename/key rotation', flush=True)


    bad = api.ok('/api/channels', {'name': 'Broken webhook', 'kind': 'webhook', 'config': {'url': 'http://mock:8000/fail'}}, 'POST')['id']
    receiver = api.ok('/api/channels', {'name': 'Alert receiver', 'kind': 'webhook', 'config': {'url': 'http://mock:8000/alert-receiver'}}, 'POST')['id']
    failing_route = api.ok('/api/routes', {'name': 'Failing route', 'slug': 'failures', 'channel_ids': [bad]}, 'POST')
    rule_input = {'name': 'Two failures', 'enabled': True, 'watch_channel_ids': [bad], 'watch_route_ids': [failing_route['id']], 'destination_ids': [receiver, ids['smtp']], 'failure_threshold': 2, 'min_attempts': 1, 'window_minutes': 15, 'cooldown_minutes': 1, 'title': 'Delivery warning: {channel}', 'body': '{failures} failures via {route}: {error}'}
    policy_id = admin.ok('/api/alert-policies', rule_input, 'POST')['id']
    ignored = {**rule_input, 'name': 'Other route only', 'watch_route_ids': [route['id']], 'failure_threshold': 1}
    admin.ok('/api/alert-policies', ignored, 'POST')
    for i in range(2):
        api.ok('/h/failures', {'title': f'Failure {i}', 'message': 'Mock failure'}, 'POST', {'X-Relay-Key': failing_route['token']})
    def alerts_sent():
        rows = [r for r in admin.ok('/api/alert-deliveries') if r['policy_name'] == 'Two failures']
        return rows if len(rows) == 2 and all(r['status'] == 'sent' for r in rows) else False
    wait_for(alerts_sent, 'Failure alerts were not delivered', 30)
    assert not any(a['policy_name'] == 'Other route only' for a in admin.ok('/api/alert-deliveries'))
    assert api.ok('/api/deliveries?status=retrying')['total'] == 2
    assert api.ok('/api/stats')['retrying'] == 2
    api.ok('/h/failures', {'title': 'Cooldown failure', 'message': 'No repeated warning yet'}, 'POST', {'X-Relay-Key': failing_route['token']})
    time.sleep(6)
    assert len([a for a in admin.ok('/api/alert-deliveries') if a['policy_name'] == 'Two failures']) == 2
    result = admin.ok('/api/alert-policies/' + policy_id + '/test', {}, 'POST')
    assert result['queued'] == 2
    wait_for(lambda: len([a for a in admin.ok('/api/alert-deliveries') if a['policy_name'] == 'Two failures' and a['status'] == 'sent']) == 4, 'Manual alert test did not deliver')
    isolated = {**rule_input, 'name': 'Alert failure isolation', 'destination_ids': [bad], 'failure_threshold': 1}
    admin.ok('/api/alert-policies', isolated, 'POST')
    wait_for(lambda: any(a['policy_name'] == 'Alert failure isolation' for a in admin.ok('/api/alert-deliveries')), 'Alert failure not queued')
    time.sleep(6)
    assert len([a for a in admin.ok('/api/alert-deliveries') if a['policy_name'] == 'Alert failure isolation']) == 1
    # Exercise the full attempt budget without waiting through this test channel's cooldowns.
    def complete_failures():
        subprocess.run(['psql','--no-psqlrc','-v','ON_ERROR_STOP=1','-c',"UPDATE channel_health SET paused_until=NOW()-INTERVAL '1 second' WHERE channel_id='"+bad+"' AND paused_until IS NOT NULL"],check=True,stdout=subprocess.DEVNULL)
        return api.ok('/api/stats')['failed'] == 3
    wait_for(complete_failures, 'Failed delivery metrics did not update', 90)
    metrics = api.ok('/api/stats')
    assert metrics['failed_30_days'] == 3
    failed = api.ok('/api/deliveries?status=failed')
    assert failed['total'] == 3 and all(d['attempts'] == 5 for d in failed['items'])
    api.ok('/api/deliveries/' + failed['items'][0]['id'] + '/retry', {}, 'POST')
    print('PASS: failure thresholds, channel/route scopes, recipients, alert text, cooldown, alert retries/isolation, failed metrics and manual delivery retry', flush=True)
    with urlopen('http://mock:8000/received') as response:
        received = json.load(response)
    matrix = [r for r in received if r['method'] == 'PUT']
    assert all(r['authorization'] == 'Bearer ' + credentials['matrix_token'] for r in matrix)
    matrix_delivery = next(d for d in deliveries if d['channel_name'] == 'matrix')
    assert any(r['path'].endswith('/' + matrix_delivery['id']) and json.loads(r['body'])['body'] == 'Ready\n\nDeployment successful' for r in matrix)
    warnings = [json.loads(r['body']) for r in received if r.get('path') == '/alert-receiver']
    assert any('2 failures via Failing route' in w['message'] for w in warnings)
    mails = [Parser(policy=policy.default).parsestr(r['body']) for r in received if r['method'] == 'SMTP']
    assert any(m['From'].addresses[0].display_name == 'Relayix Alerts \u2709' and m['From'].addresses[0].addr_spec == 'alerts@example.org' for m in mails)
    assert any(m['Subject'] == 'Ready' and 'Deployment successful' in m.get_content() for m in mails)
    assert any(m['Subject'] == 'Delivery warning: Broken webhook' for m in mails)
    # Exercise both bot/API destinations without contacting a real Mattermost installation.
    bot_cfg = {'mode': 'bot', 'url': 'http://mock:8000', 'bot_token': credentials['mattermost_token'], 'target': 'channel', 'channel_id': 'mmchannel'}
    mm_ids = []
    for name, config in [('MM bot channel', bot_cfg), ('MM direct username', {**bot_cfg, 'target': 'direct', 'recipient': '@ops-bot'}), ('MM direct ID', {**bot_cfg, 'url': 'http://mock:8000/api/v4/', 'target': 'direct', 'recipient': 'r' * 26})]:
        cid = api.ok('/api/channels', {'name': name, 'kind': 'mattermost', 'config': config}, 'POST')['id']
        mm_ids.append(cid)
        api.ok('/api/channels/' + cid + '/test', {}, 'POST')
        masked = next(c for c in api.ok('/api/channels') if c['id'] == cid)
        assert masked['config']['bot_token'] == masked['config']['url'] == '********'
        api.ok('/api/channels/' + cid, masked, 'PUT')
        api.ok('/api/channels/' + cid + '/test', {}, 'POST')
    for config in [{**bot_cfg, 'bot_token': ''}, {**bot_cfg, 'channel_id': ''}, {**bot_cfg, 'target': 'direct', 'recipient': ''}, {**bot_cfg, 'mode': 'invalid'}]:
        assert api.request('/api/channels', {'name': 'Invalid MM', 'kind': 'mattermost', 'config': config}, 'POST')[0] == 400
    for name, config in [('No permission', {**bot_cfg, 'channel_id': 'forbidden'}), ('Bad token', {**bot_cfg, 'bot_token': credentials['failure_token']}), ('Malformed post', {**bot_cfg, 'channel_id': 'malformed'})]:
        cid = api.ok('/api/channels', {'name': name, 'kind': 'mattermost', 'config': config}, 'POST')['id']
        status, error = api.request('/api/channels/' + cid + '/test', {}, 'POST')
        assert status == 502 and credentials['failure_token'] not in error['error'] and 'http://' not in error['error']
    mm_route = api.ok('/api/routes', {'name': 'Mattermost bot queue', 'channel_ids': mm_ids}, 'POST')
    mm_event = api.ok(mm_route['endpoint'], {'title': 'Bot queue', 'message': 'Mattermost API queued delivery'}, 'POST', {'X-Relay-Key': mm_route['token']})
    def mm_sent():
        rows = [r for r in api.ok('/api/deliveries')['items'] if r['event_id'] == mm_event['event_id']]
        return len(rows) == 3 and all(r['status'] == 'sent' for r in rows)
    wait_for(mm_sent, 'Mattermost bot queue did not deliver')
    with urlopen('http://mock:8000/received') as response:
        mm = [r for r in json.load(response) if r.get('path', '').startswith('/api/v4/')]
    assert any(r['path'] == '/api/v4/channels/direct' and json.loads(r['body']) == ['s' * 26, 'r' * 26] for r in mm)
    posts = [r for r in mm if r['path'] == '/api/v4/posts' and r['authorization'] == 'Bearer ' + credentials['mattermost_token']]
    assert any(json.loads(r['body'])['channel_id'] == 'mmchannel' for r in posts)
    assert any(json.loads(r['body'])['channel_id'] == 'd' * 26 for r in posts)
    assert any(json.loads(r['body'])['message'] == '**Bot queue**\n\nMattermost API queued delivery' for r in posts)
    print('PASS: Mattermost webhook compatibility, bot channel and direct-to-user/bot delivery, token masking, permissions/errors and queued bot sends', flush=True)

    # Authenticated proxy forwarding, SMTP CONNECT, redaction and failure without direct fallback.
    proxy = {'proxy_type': 'http', 'proxy_host': 'proxy', 'proxy_port': '3128', 'proxy_username': credentials['proxy_username'], 'proxy_password': credentials['proxy_password']}
    for kind, config in [('webhook', {'url': 'http://mock:8000/proxy-test'}), ('smtp', configs['smtp']), ('mattermost', bot_cfg)]:
        cid = api.ok('/api/channels', {'name': 'Proxy ' + kind, 'kind': kind, 'config': {**config, **proxy}}, 'POST')['id']
        api.ok('/api/channels/' + cid + '/test', {}, 'POST')
        masked = next(c for c in api.ok('/api/channels') if c['id'] == cid)
        assert masked['config']['proxy_username'] == masked['config']['proxy_password'] == '********'
        api.ok('/api/channels/' + cid, masked, 'PUT')
        api.ok('/api/channels/' + cid + '/test', {}, 'POST')
    bad_proxy = api.ok('/api/channels', {'name': 'Rejected proxy', 'kind': 'webhook', 'config': {'url': 'http://mock:8000/must-not-bypass', **proxy, 'proxy_password': credentials['failure_token']}}, 'POST')['id']
    status, error = api.request('/api/channels/' + bad_proxy + '/test', {}, 'POST')
    assert status == 502 and credentials['failure_token'] not in error['error'] and credentials['proxy_password'] not in str(api.ok('/api/channels'))
    with urlopen('http://mock:8000/received') as response: traffic = json.load(response)
    assert any(r.get('path') == '/proxy-test' and r.get('test_proxy') == 'used' for r in traffic)
    assert not any(r.get('path') == '/must-not-bypass' for r in traffic)
    assert all('channel_kind' in d for d in api.ok('/api/deliveries')['items'])
    print('PASS: authenticated HTTP proxy, SMTP CONNECT, Mattermost API through proxy, masked edits and no direct fallback; delivery provider identity', flush=True)

    # Fixtures span 2 minutes to 40 days: independently verify that the period changes the data.
    subprocess.run(['psql', '--no-psqlrc', '-v', 'ON_ERROR_STOP=1', '-f', '/tests/seed_periods.sql'], check=True, stdout=subprocess.DEVNULL)
    windows = [('5m', 300, 4), ('15m', 900, 5), ('30m', 1800, 6), ('1h', 3600, 7), ('24h', 86400, 8), ('7d', 604800, 9), ('30d', 2592000, 10)]
    for period, seconds, fixture_count in windows:
        metrics = api.ok('/api/stats?period=' + period)
        sql = f"SELECT JSON_BUILD_OBJECT('total',COUNT(*),'sent',COUNT(*) FILTER(WHERE status='sent')) FROM deliveries WHERE created_at>=NOW()-INTERVAL '{seconds} seconds' AND created_at<=NOW();"
        expected = json.loads(subprocess.check_output(['psql', '--no-psqlrc', '-At', '-c', sql], text=True))
        assert metrics['period'] == period and metrics['total'] == expected['total'] and metrics['sent'] == expected['sent'], (period, metrics, expected)
        assert metrics['sent'] + metrics['failed'] + metrics['pending'] == metrics['total'] and metrics['retrying'] <= metrics['pending']
        filtered = api.ok('/api/deliveries?' + urlencode({'period': period, 'tag': 'period-fixture'}))
        assert filtered['total'] == fixture_count, (period, filtered['total'], fixture_count)
    assert api.request('/api/stats?period=invalid')[0] == 400
    assert api.request('/api/deliveries?period=invalid')[0] == 400
    preferences = admin.ok('/api/auth/me')['preferences']
    admin.ok('/api/auth/preferences', {**preferences, 'dashboard_period': '15m', 'api_token_tag_filter': 'deploy'}, 'PUT')
    assert second_admin.ok('/api/auth/me')['preferences']['dashboard_period'] == '15m'
    assert second_admin.ok('/api/auth/me')['preferences']['api_token_tag_filter'] == 'deploy'
    assert admin.request('/api/auth/preferences', {**preferences, 'dashboard_period': 'invalid'}, 'PUT')[0] == 400
    admin.ok('/api/auth/preferences', preferences, 'PUT')
    print('PASS: all seven dashboard periods against historical database rows, matching history windows, period validation and account persistence', flush=True)
    from routing import verify
    verify(api, Client())
    from operations import verify as verify_operations
    verify_operations(api, Client())
    from tables import verify as verify_tables
    verify_tables(api, Client())
    from message_content import verify as verify_content
    verify_content(api, Client())
    from delivery_groups import verify as verify_groups
    verify_groups(api, Client())
    from templates_contract import verify as verify_templates
    verify_templates(api, Client())
    from release_082 import verify as verify_release_082
    verify_release_082(api, Client())
    if api.ok('/healthz')['version']!='0.8.2':
        from release_083 import verify as verify_release_083
        verify_release_083(api)
    print('All smoke tests passed.', flush=True)


if __name__ == '__main__':
    main()
