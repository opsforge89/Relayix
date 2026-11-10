#!/usr/bin/env python3
"""Sync deployment Secrets and Variables without exposing credential values."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
from deployment import ROOT, read_config, database_settings

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--repo', default='opsforge89/Relayix')
parser.add_argument('--environment', default='production')
parser.add_argument('--dry-run', action='store_true', help='List setting names without contacting GitHub')
args = parser.parse_args()
config = read_config()
db = database_settings(config)
secrets = {}
for key in ('POSTGRES_PASSWORD', 'ADMIN_TOKEN', 'ADMIN_PASSWORD'):
    if not config.get(key):
        raise SystemExit('Required private deployment value missing: ' + key)
    secrets['RELAYIX_' + key] = config[key]
for key in ('TELEGRAM_API_ID', 'TELEGRAM_API_HASH', 'SECRETS_KEY'):
    if config.get(key):
        secrets['RELAYIX_' + key] = config[key]

defaults = json.loads((ROOT / 'config/deployment.example.json').read_text())
variables = {}
for key in ('DATABASE_MODE', 'ADMIN_USERNAME', 'BIND_ADDRESS', 'PORT', 'COOKIE_SECURE',
            'POSTGRES_IMAGE', 'POSTGRES_CLIENT_IMAGE', 'ALLOW_INSECURE_HTTP', 'RUST_LOG',
            'TRUSTED_PROXIES', 'RELAYIX_NETWORK_SUBNET', 'RELAYIX_NETWORK_GATEWAY', 'WEBHOOK_RATE_LIMIT_PER_MINUTE', 'OUTBOUND_ALLOWLIST', 'DELIVERY_PROXY_CONCURRENCY', 'DELIVERY_CHANNEL_CONCURRENCY', 'DELIVERY_WORKERS', 'EVENT_RETENTION_DAYS'):
    variables[key if key.startswith('RELAYIX_') else 'RELAYIX_' + key] = config.get(key, defaults.get(key, ''))
for key, field in (('POSTGRES_HOST', 'host'), ('POSTGRES_PORT', 'port'),
                   ('POSTGRES_USER', 'user'), ('POSTGRES_DB', 'database'),
                   ('POSTGRES_SSLMODE', 'sslmode')):
    variables['RELAYIX_' + key] = db[field]
if config.get('POSTGRES_SSLROOTCERT'):
    variables['RELAYIX_POSTGRES_SSLROOTCERT'] = config['POSTGRES_SSLROOTCERT']
project = config.get('COMPOSE_PROJECT_NAME', 'relayix')
host = config.get('BIND_ADDRESS', defaults['BIND_ADDRESS'])
if host in ('0.0.0.0', '::'):
    host = '127.0.0.1'
if ':' in host and not host.startswith('['):
    host = '[' + host + ']'
variables.update({
    'RELAYIX_DEPLOY_DIR': str(ROOT),
    'RELAYIX_CONFIG_FILE': str(Path(os.environ.get('RELAYIX_CONFIG_FILE', ROOT.parent / 'private/relayix-config/deployment.json')).resolve()),
    'RELAYIX_COMPOSE_PROJECT': project,
    'RELAYIX_DB_VOLUME': config.get('RELAYIX_DB_VOLUME', project + '_db_data'),
    'RELAYIX_HEALTH_URL': 'http://' + host + ':' + variables['RELAYIX_PORT'] + '/healthz',
})
if args.dry_run:
    for kind, settings in (('secret', secrets), ('variable', variables)):
        for name in sorted(settings):
            print('Planned ' + args.environment + ' ' + kind + ' ' + name)
    raise SystemExit(0)
if not shutil.which('gh'):
    raise SystemExit('An authenticated GitHub CLI is required; no settings were uploaded')
if subprocess.run(['gh', 'auth', 'status'], capture_output=True).returncode:
    raise SystemExit('GitHub authentication is required; no settings were uploaded')
endpoint = 'repos/' + args.repo + '/environments/' + args.environment
result = subprocess.run(['gh', 'api', endpoint], capture_output=True)
if result.returncode:
    if b'404' not in result.stderr:
        raise SystemExit('Cannot inspect deployment environment; no settings were uploaded')
    if subprocess.run(['gh', 'api', '--method', 'PUT', endpoint], capture_output=True).returncode:
        raise SystemExit('Cannot create deployment environment; no settings were uploaded')
# Existing environment protections are preserved. Bodies go through stdin only.
for kind, settings in (('secret', secrets), ('variable', variables)):
    for name, value in settings.items():
        result = subprocess.run(['gh', kind, 'set', name, '--repo', args.repo, '--env', args.environment],
                                input=value, text=True, capture_output=True)
        if result.returncode:
            raise SystemExit('GitHub rejected ' + kind + ' update: ' + name + '; values omitted')
        print('Configured ' + args.environment + ' ' + kind + ' ' + name)
