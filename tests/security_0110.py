"""Real candidate migration, fail-closed master keys and complete encrypted backup recovery."""
import base64
import hashlib
import hmac
import json
import os
from pathlib import Path
import secrets
import socket
import struct
import subprocess
import sys
import tempfile
import time
from urllib.error import HTTPError
from urllib.request import Request, build_opener, ProxyHandler

ROOT = Path(__file__).resolve().parents[1]
IMAGE = os.environ['RELAYIX_TEST_IMAGE']
PROJECT = 'relayix-test-security-' + secrets.token_hex(4)
assert PROJECT.startswith('relayix-test-')


def otp(secret):
    raw = base64.b32decode(secret + '=' * ((8 - len(secret) % 8) % 8))
    digest = hmac.new(raw, struct.pack('>Q', int(time.time()) // 30), hashlib.sha1).digest()
    offset = digest[-1] & 15
    return str((struct.unpack('>I', digest[offset:offset + 4])[0] & 0x7fffffff) % 1000000).zfill(6)


with tempfile.TemporaryDirectory(prefix='relayix-security-') as folder:
    folder = Path(folder)
    config_path = folder / 'deployment.json'
    subprocess.run([sys.executable, str(ROOT / 'scripts/write_config.py'), str(config_path), '--database', 'local', '--generate'], capture_output=True, check=True)
    config = json.loads(config_path.read_text())
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = str(sock.getsockname()[1])
    config.update(COMPOSE_PROJECT_NAME=PROJECT, BIND_ADDRESS='127.0.0.1', PORT=port,
                  RELAYIX_NETWORK_SUBNET='172.29.113.0/24', RELAYIX_NETWORK_GATEWAY='172.29.113.1')
    config_path.write_text(json.dumps(config))
    config_path.chmod(0o600)
    env = {**os.environ, 'RELAYIX_CONFIG_FILE': str(config_path), 'RELAYIX_IMAGE_OVERRIDE': IMAGE}
    compose = [str(ROOT / 'scripts/compose.sh')]
    base = 'http://127.0.0.1:' + port
    opener = build_opener(ProxyHandler({}))

    def run(args, expected=0, **kwargs):
        result = subprocess.run(args, env=env, capture_output=True, timeout=120, **kwargs)
        assert result.returncode == expected, 'Isolated security operation failed; private output omitted'
        return result.stdout

    def sql(statement):
        return run(compose + ['exec', '-T', 'db', 'psql', '-U', 'relayix', '-d', 'relayix', '-XAt', '-v', 'ON_ERROR_STOP=1', '-c', statement]).decode().strip()

    def api(path, data=None, method=None, headers=None):
        h = {'Authorization': 'Bearer ' + config['ADMIN_TOKEN'], 'Content-Type': 'application/json', 'X-Relay-Client': 'web'}
        h.update(headers or {})
        req = Request(base + path, data=json.dumps(data).encode() if data is not None else None, method=method, headers=h)
        try:
            response = opener.open(req, timeout=15)
        except HTTPError as error:
            response = error
        with response:
            return response.status, json.load(response), response.headers

    def ok(*args, **kwargs):
        code, value, _ = api(*args, **kwargs)
        assert 200 <= code < 300, 'Security fixture API failed; private output omitted'
        return value

    def healthy():
        for _ in range(100):
            try:
                with opener.open(base + '/healthz', timeout=2) as response:
                    if response.status == 200:
                        return
            except Exception:
                pass
            time.sleep(.3)
        raise AssertionError('Encrypted application did not become healthy')

    def write_config():
        config_path.write_text(json.dumps(config))
        config_path.chmod(0o600)

    def fail_start(key):
        container = run(compose + ['ps', '-aq', 'app']).decode().strip()
        info = json.loads(run(['docker', 'inspect', container]))[0]
        settings = dict(item.split('=', 1) for item in info['Config']['Env'])
        settings['SECRETS_KEY'] = key
        args = ['docker', 'run', '--rm', '--network', PROJECT + '-network']
        for name in settings:
            args += ['-e', name]
        result = subprocess.run(args + [IMAGE], env={**env, **settings}, capture_output=True, timeout=30)
        assert result.returncode != 0 and b'key' in (result.stdout + result.stderr).lower(), 'Wrong/missing key did not fail closed'
        assert settings['ADMIN_TOKEN'].encode() not in result.stdout + result.stderr

    try:
        run(compose + ['up', '-d'])
        healthy()
        token = str(secrets.randbelow(10**9)) + ':' + secrets.token_urlsafe(32)
        channel = ok('/api/channels', {'name': 'Encrypted file destination', 'kind': 'telegram', 'config': {'bot_token': token, 'chat_id': '123456789'}})['id']
        ok('/api/channels/' + channel + '/pause', {})
        route = ok('/api/routes', {'name': 'Encrypted recovery route', 'channel_ids': [channel], 'delivery_settings': {'ttl_seconds': 3600}})
        code, _, response_headers = api('/api/auth/login', {'username': config['ADMIN_USERNAME'], 'password': config['ADMIN_PASSWORD']}, headers={'Authorization': ''})
        assert code == 200
        session = {'Authorization': '', 'Cookie': response_headers.get('Set-Cookie').split(';')[0]}
        enrollment = ok('/api/auth/totp/setup', {'password': config['ADMIN_PASSWORD']}, headers=session)
        user = sql("SELECT id FROM users WHERE role='admin' ORDER BY created_at LIMIT 1")
        assert sql("SELECT config->>'bot_token' FROM channels WHERE id='" + channel + "'") == token
        assert sql("SELECT pending_secret FROM user_totp WHERE user_id='" + user + "'") == enrollment['secret']
        # Enabling encryption migrates existing credentials without changing identities or tokens.
        run(compose + ['stop', 'app'])
        master = base64.b64encode(secrets.token_bytes(32)).decode()
        config['SECRETS_KEY'] = master
        write_config()
        run(compose + ['up', '-d', '--no-deps', 'app'])
        healthy()
        raw = sql("SELECT config::text FROM channels WHERE id='" + channel + "'")
        assert token not in raw and 'v1:' in raw
        assert sql("SELECT secrets_encrypted FROM channels WHERE id='" + channel + "'") == 't'
        assert enrollment['secret'] not in sql("SELECT pending_secret FROM user_totp WHERE user_id='" + user + "'")
        assert ok('/api/security/storage')['enabled']
        enabled = ok('/api/auth/totp/enable', {'code': otp(enrollment['secret'])}, headers=session)
        assert enrollment['secret'] not in sql("SELECT secret FROM user_totp WHERE user_id='" + user + "'")
        current = ok('/api/channels')[0]
        assert current['config']['bot_token'] == '********'
        current['name'] = 'Encrypted masked edit'
        ok('/api/channels/' + channel, current, 'PUT')
        raw = sql("SELECT config::text FROM channels WHERE id='" + channel + "'")
        fingerprint = sql("SELECT md5(config::text) FROM channels WHERE id='" + channel + "'")
        fail_start('')
        fail_start(base64.b64encode(secrets.token_bytes(32)).decode())
        assert sql("SELECT md5(config::text) FROM channels WHERE id='" + channel + "'") == fingerprint
        # A stale replica cannot overwrite an encrypted row with plaintext.
        denied = subprocess.run(compose + ['exec', '-T', 'db', 'psql', '-U', 'relayix', '-d', 'relayix', '-v', 'ON_ERROR_STOP=1', '-c', "UPDATE channels SET secrets_encrypted=false WHERE id='" + channel + "'"], env=env, capture_output=True)
        assert denied.returncode != 0
        body = b'PK\x03\x04\x00\xff\x80' + secrets.token_bytes(32)
        boundary = 'relayix' + secrets.token_hex(10)
        multipart = b'--' + boundary.encode() + b'\r\nContent-Disposition: form-data; name="files"; filename="report.xlsx"\r\nContent-Type: application/octet-stream\r\n\r\n' + body + b'\r\n--' + boundary.encode() + b'--\r\n'
        req = Request(base + route['endpoint'] + '/files', data=multipart, headers={'X-Relay-Key': route['token'], 'Content-Type': 'multipart/form-data; boundary=' + boundary})
        with opener.open(req) as response:
            event = json.load(response)
        password = folder / 'password'
        password.write_text(secrets.token_urlsafe(32))
        password.chmod(0o600)
        archive = folder / 'full.rxb'
        backup = [sys.executable, str(ROOT / 'scripts/backup.py')]
        run(backup + ['export', str(archive), '--password-file', str(password)])
        assert archive.stat().st_mode & 0o077 == 0
        data = archive.read_bytes()
        assert master.encode() not in data and token.encode() not in data and enrollment['secret'].encode() not in data and body not in data
        wrong = folder / 'wrong'
        wrong.write_text(secrets.token_urlsafe(32))
        wrong.chmod(0o600)
        run(backup + ['restore', str(archive), '--replace', '--password-file', str(wrong)], expected=1)
        assert sql("SELECT md5(config::text) FROM channels WHERE id='" + channel + "'") == fingerprint
        truncated = folder / 'truncated.rxb'
        truncated.write_bytes(data[:-1])
        run(backup + ['restore', str(truncated), '--replace', '--password-file', str(password)], expected=1)
        healthy()
        assert sql("SELECT COUNT(*) FROM event_attachments WHERE event_id='" + event['event_id'] + "'") == '1'
        # Simulate losing the live master key and data; the password-encrypted backup recovers both.
        run(compose + ['stop', 'app'])
        sql("DELETE FROM routes WHERE id='" + route['id'] + "';DELETE FROM channels WHERE id='" + channel + "'")
        config['SECRETS_KEY'] = base64.b64encode(secrets.token_bytes(32)).decode()
        write_config()
        run(backup + ['restore', str(archive), '--replace', '--password-file', str(password)])
        config = json.loads(config_path.read_text())
        assert config['SECRETS_KEY'] == master
        healthy()
        assert sql('SELECT COUNT(*) FROM sessions') == '0'
        assert sql("SELECT config::text FROM channels WHERE id='" + channel + "'") == raw
        assert sql("SELECT status||':'||attempts FROM deliveries WHERE event_id='" + event['event_id'] + "'") == 'queued:0'
        detail = ok('/api/events/' + event['event_id'])
        req = Request(base + '/api/events/' + event['event_id'] + '/attachments/' + detail['attachments'][0]['id'], headers={'Authorization': 'Bearer ' + config['ADMIN_TOKEN']})
        with opener.open(req) as response:
            assert response.read() == body
        assert api('/api/auth/login', {'username': config['ADMIN_USERNAME'], 'password': config['ADMIN_PASSWORD']}, headers={'Authorization': ''})[0] != 200
        assert api('/api/auth/login', {'username': config['ADMIN_USERNAME'], 'password': config['ADMIN_PASSWORD'], 'code': enabled['recovery_codes'][0]}, headers={'Authorization': ''})[0] == 200
        assert ok(route['endpoint'], {'message': 'Restored route key'}, headers={'X-Relay-Key': route['token']})['event_id']
        actions = json.dumps(ok('/api/audit-log'))
        assert 'backup.restore' in actions and token not in actions and master not in actions
        print('PASS: atomic credential/TOTP migration, missing/wrong key preservation, encrypted full backup, rejected corruption and exact file/route/TOTP recovery')
    finally:
        subprocess.run(compose + ['down', '--volumes', '--remove-orphans'], env=env, capture_output=True, timeout=100)
