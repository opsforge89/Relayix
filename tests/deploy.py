"""Exercise deployment/rollback with fake Docker; never touches the real service."""
from pathlib import Path
import os
import json,secrets
import shutil
import subprocess
import sys
import tempfile

SOURCE = Path(__file__).resolve().parents[1] / 'scripts' / 'deploy.sh'
PASSWORD=secrets.token_hex(32)
OLD=json.dumps({'COMPOSE_PROJECT_NAME':'relayix','POSTGRES_PASSWORD':PASSWORD})
OLD_COMPOSE='services:\n  app:\n    image: ${RELAYIX_IMAGE_OVERRIDE:-ghcr.io/example/relayix:0.6.1}\n'
with tempfile.TemporaryDirectory(prefix='relayix-deploy-test-') as directory:
    workspace=Path(directory)
    root=workspace/'repo';root.mkdir()
    (root / 'scripts').mkdir()
    compose_file=root/'compose.yaml'
    compose_file.write_text(OLD_COMPOSE)
    shutil.copy2(SOURCE, root / 'scripts' / 'deploy.sh')
    for name in ['compose.sh','compose.py','deployment.py','database.py']:shutil.copy2(SOURCE.parent/name,root/'scripts'/name)
    private=workspace/'private-config';private.mkdir();config=private/'deployment.json'
    binary = root / 'bin'
    binary.mkdir()
    docker = binary / 'docker'
    docker.write_text('''#!/usr/bin/env python3
import os, sys
from pathlib import Path
args = sys.argv[1:]
with Path('docker.log').open('a') as out:
    out.write(' '.join(args) + '\\n')
    if os.environ.get('RELAYIX_IMAGE_OVERRIDE'):out.write('explicit-image-override\\n')
if 'ps' in args:print('fixture-app')
if 'inspect' in args:
    if '{{.Config.Image}}' in args:print('ghcr.io/example/relayix:0.6.1')
    else:print('{\"fixture-network\":{}}')
if 'pg_dump' in args: print('fixture database dump')
if 'pull' in args and os.environ.get('FAIL_PULL'): sys.exit(1)
''')
    # Intercept only the health checker; execute the actual environment update code.
    python = binary / 'python3'
    python.write_text(f'''#!{sys.executable}
import os, subprocess, sys
code = sys.stdin.read() if sys.argv[1:] == ['-'] else None
if code is not None and 'urlopen' in code:
    sys.exit(1 if os.environ.get('FAIL_HEALTH') else 0)
if code is not None:
    sys.exit(subprocess.run([{sys.executable!r}, '-'], input=code, text=True).returncode)
os.execv({sys.executable!r}, [{sys.executable!r}] + sys.argv[1:])
''')
    docker.chmod(0o755)
    python.chmod(0o755)
    env = {**os.environ, 'PATH': str(binary) + os.pathsep + os.environ['PATH'],
           'RELAYIX_IMAGE_OVERRIDE':'ghcr.io/example/relayix:0.6.0',
           'RELAYIX_DEPLOY_IMAGE': 'ghcr.io/example/relayix:0.8.2', 'RELAYIX_EXPECTED_VERSION': '0.8.2', 'RELAYIX_BACKUP_DIR': str(root / 'private-backups'), 'RELAYIX_CONFIG_FILE':str(config)}
    def run(extra=None):
        return subprocess.run(['bash', str(root / 'scripts' / 'deploy.sh')], cwd=root,
                              env={**env, **(extra or {})}, capture_output=True, text=True)
    (config).write_text(OLD)
    result = run()
    assert result.returncode == 0, result.stderr
    assert 'image: ${RELAYIX_IMAGE_OVERRIDE:-ghcr.io/example/relayix:0.8.2}' in compose_file.read_text()
    assert 'RELAYIX_IMAGE' not in json.loads(config.read_text())
    assert json.loads(config.read_text())['POSTGRES_PASSWORD']==PASSWORD
    assert (root/'docker.log').read_text().count('explicit-image-override')==1
    assert list((root / 'private-backups').glob('db-before-deploy-*.dump'))
    assert PASSWORD not in result.stdout + result.stderr
    (config).write_text(OLD)
    compose_file.write_text(OLD_COMPOSE)
    result = run({'FAIL_PULL': '1'})
    assert result.returncode != 0 and (config).read_text() == OLD
    assert compose_file.read_text()==OLD_COMPOSE
    (root / 'docker.log').write_text('')
    compose_file.write_text(OLD_COMPOSE)
    # A source checkout may already select the new release while the old image is running.
    compose_file.write_text(OLD_COMPOSE.replace('0.6.1', '0.8.2'))
    result = run({'FAIL_HEALTH': '1'})
    assert compose_file.read_text()==OLD_COMPOSE
    assert result.returncode != 0 and (config).read_text() == OLD
    assert (root / 'docker.log').read_text().count('up -d app') == 2
    remote={**json.loads(OLD),'DATABASE_MODE':'external','POSTGRES_HOST':'postgres.example.org'}
    config.write_text(json.dumps(remote))
    result=run()
    assert result.returncode==0,result.stderr
    assert 'pg_dump -Fc' in (root/'docker.log').read_text()
    assert 'exec -T db' not in (root/'docker.log').read_text()
    candidate=private/'candidate.json'
    candidate.write_text(json.dumps({**remote,'POSTGRES_HOST':'different.example.org'}))
    config.write_text(json.dumps(remote))
    result=run({'RELAYIX_DEPLOY_CONFIG_CANDIDATE':str(candidate)})
    assert result.returncode!=0 and json.loads(config.read_text())==remote
    assert 'Database destination differs' in result.stderr
    candidate.write_text(json.dumps(remote))
    result=run({'RELAYIX_DEPLOY_CONFIG_CANDIDATE':str(candidate)})
    assert result.returncode==0,result.stderr
    result = run({'RELAYIX_EXPECTED_VERSION': 'bad;touch should-not-exist'})
    assert result.returncode != 0 and not (root / 'should-not-exist').exists()
print('PASS: deployment backup, image persistence, pull failure, health rollback and input validation with fake Docker')
