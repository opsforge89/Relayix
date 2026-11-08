"""Exercise downloaded Compose bundles against the published image without host helpers."""
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]

def run(*args, **kwargs):
    result = subprocess.run(args, capture_output=True, text=True, **kwargs)
    if result.returncode:
        raise RuntimeError('Command failed: ' + args[0] + ' (credential values omitted)')
    return result.stdout

for mode in (sys.argv[1:] or ('local', 'external', 'external-tls')):
    assert mode in ('local', 'external', 'external-tls')
    project = 'relayix-test-quickstart-' + secrets.token_hex(5)
    database_container = project + '-external-db'
    password = secrets.token_hex(24) + '@:#%/$'
    admin_password = 'Ab7!' + secrets.token_hex(24)
    with tempfile.TemporaryDirectory(prefix=project) as directory:
        path = Path(directory)
        shutil.copyfile(ROOT / 'compose.yaml', path / 'compose.yaml')
        if os.environ.get('RELAYIX_TEST_IMAGE'):
            bundle = (path / 'compose.yaml').read_text().splitlines()
            index = next(i for i, line in enumerate(bundle) if line.startswith('    image: '))
            bundle[index] = '    image: ' + os.environ['RELAYIX_TEST_IMAGE']
            (path / 'compose.yaml').write_text('\n'.join(bundle) + '\n')
        current_compose = (path / 'compose.yaml').read_text()
        if mode == 'local':
            # Simulate a pre-upgrade installation with automatic container/network names.
            legacy = '\n'.join(line for line in current_compose.splitlines()
                               if not line.strip().startswith('container_name:')) + '\n'
            legacy = legacy.split('\nnetworks:\n', 1)[0] + '\n'
            (path / 'compose.yaml').write_text(legacy)
        # Exercise the shipped template, including its automatic profile selection.
        settings = (ROOT / 'relayix.env.example').read_text()
        settings = settings.replace("POSTGRES_PASSWORD=''", "POSTGRES_PASSWORD='" + password + "'")
        settings = settings.replace("ADMIN_PASSWORD=''", "ADMIN_PASSWORD='" + admin_password + "'")
        settings = settings.replace('BIND_ADDRESS=0.0.0.0', 'BIND_ADDRESS=127.0.0.1').replace('PORT=8080', 'PORT=0')
        if mode != 'local':
            settings = settings.replace('DATABASE_MODE=local', 'DATABASE_MODE=external')
            settings = settings.replace('POSTGRES_HOST=\n', 'POSTGRES_HOST=external-db\n')
            settings = settings.replace('POSTGRES_SSLMODE=verify-full', 'POSTGRES_SSLMODE=disable')
        if mode == 'external-tls':
            certs = path / 'certs'; certs.mkdir()
            def openssl(*args):
                run('openssl', *args, cwd=certs)
            openssl('req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', 'ca.key', '-out', 'ca.crt',
                    '-days', '1', '-subj', '/CN=Relayix quickstart CA')
            openssl('req', '-newkey', 'rsa:2048', '-nodes', '-keyout', 'server.key', '-out', 'server.csr',
                    '-subj', '/CN=external-db')
            (certs / 'extensions.txt').write_text('subjectAltName=DNS:external-db\nextendedKeyUsage=serverAuth\n')
            openssl('x509', '-req', '-in', 'server.csr', '-CA', 'ca.crt', '-CAkey', 'ca.key', '-CAcreateserial',
                    '-out', 'server.crt', '-days', '1', '-extfile', 'extensions.txt')
            (certs / 'ca.crt').chmod(0o644)
            settings = settings.replace('POSTGRES_SSLMODE=disable', 'POSTGRES_SSLMODE=verify-full')
            settings = settings.replace('POSTGRES_CA_PATH=\n', "POSTGRES_CA_PATH='" + str(certs / 'ca.crt') + "'\n")
        (path / '.env').write_text(settings)
        (path / '.env').chmod(0o600)
        # Ignore the host's unrelated deployment variables.
        clean_env = {k: v for k, v in os.environ.items()
                     if k not in ('COMPOSE_FILE', 'COMPOSE_PROJECT_NAME', 'COMPOSE_ENV_FILES', 'COMPOSE_PROFILES', 'DATABASE_MODE')
                     and not k.startswith(('POSTGRES_', 'ADMIN_', 'TELEGRAM_', 'RELAYIX_'))
                     and k not in ('BIND_ADDRESS', 'PORT', 'COOKIE_SECURE', 'ALLOW_INSECURE_HTTP', 'RUST_LOG')}
        compose = ['docker', 'compose', '--project-directory', str(path), '-p', project]
        def dc(*args):
            return run(*compose, *args, cwd=path, env=clean_env)
        try:
            model = json.loads(dc('config', '--format', 'json'))
            assert set(model['services']) == ({'app', 'db'} if mode == 'local' else {'app'})
            assert model['services']['app']['environment']['POSTGRES_PASSWORD'].replace('$$', '$') == password
            if mode != 'local':
                dc('create', 'app')
                network = run('docker', 'network', 'ls', '--filter', 'label=com.docker.compose.project=' + project,
                              '--format', '{{.Name}}').strip()
                assert network and '\n' not in network
                # This independent container represents a pre-existing database.
                (path / 'pg.env').write_text('POSTGRES_USER=relayix\nPOSTGRES_DB=relayix\nPOSTGRES_PASSWORD=' + password + '\n')
                command = ['docker', 'run', '-d', '--name', database_container, '--network', network,
                           '--network-alias', 'external-db', '--env-file', str(path / 'pg.env')]
                if mode == 'external-tls':
                    command += ['-v', str(certs) + ':/tls-source:ro', '--entrypoint', 'sh',
                                'postgres:17.11-trixie', '-c',
                                'mkdir -p /tmp/db-tls; cp /tls-source/server.key /tls-source/server.crt /tmp/db-tls/; '
                                'chown -R postgres:postgres /tmp/db-tls; chmod 600 /tmp/db-tls/server.key; '
                                'exec docker-entrypoint.sh postgres -c ssl=on '
                                '-c ssl_cert_file=/tmp/db-tls/server.crt -c ssl_key_file=/tmp/db-tls/server.key']
                    mount = model['services']['app']['volumes'][0]
                    assert mount['read_only'] and mount['source'] == str(certs / 'ca.crt')
                else:
                    command += ['postgres:17.11-trixie']
                run(*command)
                for attempt in range(60):
                    ready = subprocess.run(['docker', 'exec', database_container, 'pg_isready', '-U', 'relayix'], capture_output=True)
                    if ready.returncode == 0:
                        break
                    time.sleep(1)
                else:
                    raise RuntimeError('External test database did not become ready')
            dc('up', '-d', '--wait', '--wait-timeout', '90')
            services = set(dc('ps', '--services').split())
            assert services == ({'app', 'db'} if mode == 'local' else {'app'}), services
            container = dc('ps', '-q', 'app').strip()
            runtime_env = json.loads(run('docker', 'inspect', container, '--format', '{{json .Config.Env}}'))
            assert 'POSTGRES_PASSWORD=' + password in runtime_env
            address = dc('port', 'app', '8080').strip()
            url = 'http://' + address
            for attempt in range(90):
                try:
                    with urllib.request.urlopen(url + '/healthz', timeout=2) as response:
                        health = json.load(response)
                    assert health['status'] == 'ok'
                    break
                except (urllib.error.URLError, TimeoutError, ConnectionError):
                    time.sleep(1)
            else:
                raise RuntimeError('Quick-start application did not become healthy')
            if mode == 'external-tls':
                # Verify that the application's actual database sessions use TLS.
                result = run('docker', 'exec', database_container, 'psql', '-U', 'relayix', '-d', 'relayix', '-Atc',
                    "SELECT count(*) || ':' || bool_and(s.ssl)::text FROM pg_stat_ssl s "
                    "JOIN pg_stat_activity a USING(pid) WHERE a.datname = 'relayix' AND s.pid <> pg_backend_pid();")
                count, encrypted = result.strip().split(':')
                assert int(count) > 0 and encrypted == 'true'
            request = urllib.request.Request(url + '/api/auth/login',
                data=json.dumps({'username': 'admin', 'password': admin_password}).encode(),
                headers={'Content-Type': 'application/json', 'X-Relay-Client': 'web'})
            with urllib.request.urlopen(request, timeout=10) as response:
                assert response.status == 200
            if mode == 'local':
                database = dc('ps', '-q', 'db').strip()
                run('docker', 'exec', database, 'psql', '-U', 'relayix', '-d', 'relayix', '-c',
                    "CREATE TABLE compose_upgrade_probe (value text); INSERT INTO compose_upgrade_probe VALUES ('preserved');")
                (path / 'compose.yaml').write_text(current_compose)
                dc('up', '-d', '--wait', '--wait-timeout', '90')
                database = dc('ps', '-q', 'db').strip()
                assert run('docker', 'exec', database, 'psql', '-U', 'relayix', '-d', 'relayix', '-Atc',
                           'SELECT value FROM compose_upgrade_probe;').strip() == 'preserved'
                for service in ('app', 'db'):
                    identifier = dc('ps', '-q', service).strip()
                    assert run('docker', 'inspect', identifier, '--format', '{{.Name}}').strip() == '/' + project + '-' + service
                    networks = json.loads(run('docker', 'inspect', identifier, '--format', '{{json .NetworkSettings.Networks}}'))
                    assert set(networks) == {project + '-network'}
            if mode == 'local':
                # Recreate the bridge without deleting volumes; gateway and trust stay stable.
                dc('down')
                dc('up', '-d', '--wait', '--wait-timeout', '90')
                container = dc('ps', '-q', 'app').strip()
                attached = json.loads(run('docker', 'inspect', container, '--format', '{{json .NetworkSettings.Networks}}'))
                assert attached[project+'-network']['Gateway']=='172.30.84.1'
                runtime_env = json.loads(run('docker', 'inspect', container, '--format', '{{json .Config.Env}}'))
                assert 'TRUSTED_PROXIES=172.30.84.1' in runtime_env
                database = dc('ps', '-q', 'db').strip()
                assert run('docker', 'exec', database, 'psql', '-U', 'relayix', '-d', 'relayix', '-Atc',
                           'SELECT value FROM compose_upgrade_probe;').strip() == 'preserved'
            # Recreating the app must preserve data and the existing login.
            dc('up', '-d', '--no-deps', '--force-recreate', 'app')
            for attempt in range(60):
                try:
                    # An ephemeral host port can change when recreating the app.
                    address = dc('port', 'app', '8080').strip()
                    with urllib.request.urlopen('http://' + address + '/healthz', timeout=2) as response:
                        assert json.load(response)['status'] == 'ok'
                    break
                except (urllib.error.URLError, TimeoutError, ConnectionError):
                    time.sleep(1)
            else:
                raise RuntimeError('Recreated application did not become healthy')
            print('PASS: one Compose file, ' + mode + ', special-character database password, bootstrap login and app recreation', flush=True)
        finally:
            # Remove the independent DB before deleting the Compose network.
            subprocess.run(['docker', 'rm', '-f', '-v', database_container], capture_output=True)
            subprocess.run([*compose, 'down', '--volumes', '--remove-orphans'], cwd=path, env=clean_env, capture_output=True)
