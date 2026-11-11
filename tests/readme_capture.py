"""Capture real candidate UI on an isolated, fictional demonstration database."""
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def browser_capture():
    from playwright.sync_api import sync_playwright, expect
    base = os.environ['RELAYIX_TEST_BASE_URL']
    output = Path('/artifacts')
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch()
        page = browser.new_page(viewport={'width': 1680, 'height': 1100}, locale='en-US', timezone_id='Europe/Moscow')
        page.set_default_timeout(15000)
        errors = []
        page.on('pageerror', lambda error: errors.append(str(error)))
        api = playwright.request.new_context(base_url=base, extra_http_headers={'Authorization': 'Bearer ' + os.environ['RELAYIX_SCREENSHOT_TOKEN'], 'X-Relay-Client': 'web'})

        def ok(response):
            assert response.ok, 'README fixture request failed; private output omitted'
            return response.json()

        print('README capture: seed', flush=True)
        kinds = [
            ('Production · Mattermost', 'mattermost', {'url': 'http://notifications.example.test:8000', 'mode': 'bot', 'bot_token': secrets.token_urlsafe(32), 'target': 'channel', 'channel_id': 'productionalerts'}),
            ('Operations · Webhook', 'webhook', {'url': 'http://notifications.example.test:8000/operations'}),
            ('Release reports · Telegram', 'telegram', {'bot_token': '123456789:' + secrets.token_urlsafe(32), 'chat_id': '-1001234567890'}),
        ]
        channels = [ok(api.post('/api/channels', data={'name': name, 'kind': kind, 'config': config}))['id'] for name, kind, config in kinds]
        ok(api.post('/api/channels/' + channels[2] + '/pause', data={}))
        production = ok(api.post('/api/routes', data={'name': 'Production deployments', 'slug': 'production-deployments', 'channel_ids': channels[:2], 'icon': 'rocket', 'required_tag': 'production', 'delivery_settings': {'ttl_seconds': 3600}}))
        controls = ok(api.post('/api/routes', data={'name': 'Night-time operations', 'slug': 'night-time-operations', 'channel_ids': channels[:2], 'icon': 'activity', 'delivery_settings': {'ttl_seconds': 86400, 'quiet_enabled': True, 'timezone': 'Europe/Moscow', 'quiet_start': '22:00', 'quiet_end': '08:00', 'dedup_seconds': 60, 'urgent_priority': 4}}))
        reports = ok(api.post('/api/routes', data={'name': 'Release reports and files', 'slug': 'release-reports', 'channel_ids': [channels[2]], 'icon': 'package', 'delivery_settings': {'ttl_seconds': 604800}}))
        for title, message, tags in [
            ('API deployment completed', 'Version 2.14.0 is live. Health checks passed in all three regions.', ['production', 'deployment']),
            ('Database backup verified', 'Nightly backup completed and the restore check passed.', ['backup']),
            ('Test suite passed', '428 passed · 0 failed. Ready for the next release.', ['ci', 'tests']),
            ('Worker queue recovered', 'The delayed queue has drained. All consumers are healthy.', ['operations']),
            ('Frontend deployed', 'The new dashboard is available to the team.', ['production', 'frontend']),
        ]:
            ok(api.post(production['endpoint'], data={'title': title, 'message': message, 'tags': tags}, headers={'X-Relay-Key': production['token']}))
        ok(api.post(controls['endpoint'], data={'title': 'Nightly maintenance', 'message': 'Routine tasks will be delivered after quiet hours.', 'priority': 2}, headers={'X-Relay-Key': controls['token']}))
        ok(api.post(reports['endpoint'], data={'title': 'Weekly delivery report', 'message': 'The Telegram destination is paused until the team returns.', 'tags': ['reports']}, headers={'X-Relay-Key': reports['token']}))
        ok(api.post('/api/users', data={'username': 'release.manager', 'display_name': 'Release manager', 'role': 'member', 'password': 'A7!b' + secrets.token_hex(24)}))
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if ok(api.get('/api/stats'))['sent'] >= 10:
                break
            time.sleep(.2)
        else:
            raise AssertionError('README demonstration deliveries did not complete')
        print('README capture: login', flush=True)
        page.goto(base)
        page.get_by_label('Username', exact=True).fill('admin')
        page.get_by_label('Password', exact=True).fill(os.environ['RELAYIX_SCREENSHOT_PASSWORD'])
        page.get_by_role('button', name='Sign in', exact=True).click()
        expect(page.get_by_role('heading', name='Dashboard', exact=True)).to_be_visible()
        print('README capture: dashboard', flush=True)
        expect(page.get_by_role('button', name='Production · Mattermost', exact=True).first).to_be_visible()
        expect(page.get_by_role('button', name='Production deployments', exact=True).first).to_be_visible()
        page.get_by_role('button', name='Dark', exact=True).click()
        page.evaluate('document.fonts.ready')
        page.screenshot(path=str(output / 'readme-dashboard-dark.png'), full_page=True, animations='disabled')
        page.get_by_role('button', name='Light', exact=True).click()

        def nav(name):
            page.locator('nav').get_by_role('button', name=name, exact=True).click()

        print('README capture: channels', flush=True)
        nav('Channels')
        expect(page.locator('.channel-card')).to_have_count(3)
        page.screenshot(path=str(output / 'readme-channels-light.png'), full_page=True, animations='disabled')
        print('README capture: templates', flush=True)
        nav('Templates')
        page.get_by_role('button', name='Use Test results example', exact=True).click()
        editor = page.get_by_role('dialog', name='Create template', exact=True)
        editor.get_by_role('button', name='Preview', exact=True).click()
        expect(editor.get_by_role('region', name='Template preview', exact=True)).to_contain_text('Skipped: 0')
        page.screenshot(path=str(output / 'readme-template-editor-light.png'), full_page=True, animations='disabled')
        editor.get_by_role('button', name='Cancel', exact=True).click()
        print('README capture: routes', flush=True)
        nav('Routes')
        page.get_by_role('button', name='Edit route Night-time operations', exact=True).click()
        dialog = page.get_by_role('dialog', name='Edit route', exact=True)
        dialog.get_by_label('Message TTL (seconds)', exact=True).scroll_into_view_if_needed()
        expect(dialog.get_by_label('Message TTL (seconds)', exact=True)).to_have_value('86400')
        page.screenshot(path=str(output / 'readme-route-controls-light.png'), full_page=True, animations='disabled')
        dialog.get_by_role('button', name='Cancel', exact=True).click()
        print('README capture: audit', flush=True)
        nav('Audit log')
        expect(page.get_by_role('table', name='Audit log')).to_contain_text('user.create')
        page.screenshot(path=str(output / 'readme-audit-light.png'), full_page=True, animations='disabled')
        health = ok(api.get('/healthz'))
        (output / 'readme-capture.json').write_text(json.dumps({'version': health['version'], 'source': 'verified candidate runtime', 'fictional_data': True, 'viewport': [1680, 1100]}))
        assert errors == [], errors
        browser.close()
    print('PASS: actual README dashboard/channels/template/TTL/audit screenshots with isolated fictional data')


def host_capture():
    project = 'relayix-test-readme-' + secrets.token_hex(4)
    mock = project + '-mock'
    artifacts = Path(os.environ['RELAYIX_TEST_ARTIFACTS']).resolve()
    with tempfile.TemporaryDirectory(prefix='relayix-readme-') as directory:
        config_path = Path(directory) / 'deployment.json'
        subprocess.run([sys.executable, str(ROOT / 'scripts/write_config.py'), str(config_path), '--database', 'local', '--generate'], capture_output=True, check=True)
        config = json.loads(config_path.read_text())
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = str(sock.getsockname()[1])
        config.update(COMPOSE_PROJECT_NAME=project, PORT=port, BIND_ADDRESS='127.0.0.1', ALLOW_INSECURE_HTTP='true', OUTBOUND_ALLOWLIST='172.29.114.0/24', RELAYIX_NETWORK_SUBNET='172.29.114.0/24', RELAYIX_NETWORK_GATEWAY='172.29.114.1', SECRETS_KEY=secrets.token_hex(32))
        config_path.write_text(json.dumps(config))
        config_path.chmod(0o600)
        env = {**os.environ, 'RELAYIX_CONFIG_FILE': str(config_path), 'RELAYIX_IMAGE_OVERRIDE': os.environ['RELAYIX_TEST_IMAGE']}
        compose = [str(ROOT / 'scripts/compose.sh')]

        def run(args, **kwargs):
            result = subprocess.run(args, env=env, capture_output=True, timeout=180, **kwargs)
            assert result.returncode == 0, 'README capture command failed; private output omitted'
            return result.stdout

        try:
            run(compose + ['up', '-d'])
            server = """from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
class Handler(BaseHTTPRequestHandler):
 def log_message(self,*args): pass
 def do_POST(self):
  self.rfile.read(int(self.headers.get('Content-Length','0')))
  self.send_response(200)
  self.send_header('Content-Type','application/json')
  self.end_headers()
  self.wfile.write(b'{"id":"demonotification"}')
ThreadingHTTPServer(('0.0.0.0',8000),Handler).serve_forever()
"""
            run(['docker', 'run', '-d', '--name', mock, '--network', project + '-network', '--network-alias', 'notifications.example.test', 'python:3.15.2-slim-trixie', 'python', '-c', server])
            for _ in range(80):
                ready = subprocess.run(compose + ['exec', '-T', 'app', 'python', '-c', "import urllib.request;urllib.request.urlopen('http://127.0.0.1:8080/healthz',timeout=2)"], env=env, capture_output=True)
                if ready.returncode == 0:
                    break
                time.sleep(.3)
            else:
                raise AssertionError('README candidate did not start')
            env.update(RELAYIX_TEST_BASE_URL='http://' + project + '-app:8080', RELAYIX_SCREENSHOT_TOKEN=config['ADMIN_TOKEN'], RELAYIX_SCREENSHOT_PASSWORD=config['ADMIN_PASSWORD'])
            command = ['docker', 'run', '--rm', '--network', project + '-network', '-v', str(ROOT / 'tests') + ':/tests:ro', '-v', str(artifacts) + ':/artifacts']
            for name in ('RELAYIX_TEST_BASE_URL', 'RELAYIX_SCREENSHOT_TOKEN', 'RELAYIX_SCREENSHOT_PASSWORD'):
                command += ['-e', name]
            result = subprocess.run(command + [os.environ['RELAYIX_TEST_BROWSER_IMAGE'], 'python', '/tests/readme_capture.py', '--browser'], env=env, capture_output=True, timeout=180)
            if result.returncode:
                for line in result.stdout.decode(errors='replace').splitlines():
                    if line.startswith('README capture:'):
                        print(line)
                # Browser assertions omit request bodies; never publish the protected environment.
                raise AssertionError('README Chromium capture failed; private browser output omitted')
            print('PASS: current README screenshots captured in isolated Chromium from the exact candidate')
        finally:
            subprocess.run(['docker', 'rm', '-f', mock], capture_output=True)
            subprocess.run(compose + ['down', '--volumes', '--remove-orphans'], env=env, capture_output=True, timeout=100)


if __name__ == '__main__':
    browser_capture() if '--browser' in sys.argv else host_capture()
