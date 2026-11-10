#!/usr/bin/env bash
# Run on the deployment host; GitHub CI never invokes this script.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${RELAYIX_DEPLOY_IMAGE:?Set an explicit image reference}"
: "${RELAYIX_EXPECTED_VERSION:?Set the version expected from healthz}"
[[ "$RELAYIX_EXPECTED_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
export COMPOSE_PROJECT_NAME="${RELAYIX_COMPOSE_PROJECT:-relayix}"
export RELAYIX_HEALTH_URL="${RELAYIX_HEALTH_URL:-http://127.0.0.1:8080/healthz}"
# Image references and versions are arguments, never shell code.
[[ "$RELAYIX_DEPLOY_IMAGE" =~ ^[a-zA-Z0-9][a-zA-Z0-9._:/@-]+$ ]]
export RELAYIX_CONFIG_FILE="${RELAYIX_CONFIG_FILE:-$PWD/../private/relayix-config/deployment.json}"
test -f "$RELAYIX_CONFIG_FILE"
compose=("$PWD/scripts/compose.sh")
unset RELAYIX_IMAGE RELAYIX_IMAGE_OVERRIDE
umask 077
backup_root="${RELAYIX_BACKUP_DIR:-../private/relayix-backups}"
mkdir -p "$backup_root"
chmod 700 "$backup_root"
stamp="$(date -u +%Y%m%d-%H%M%S)-$$"
backup_env="$backup_root/config-before-deploy-$stamp.json"
backup_compose="$backup_root/compose-before-deploy-$stamp.yaml"
cp "$RELAYIX_CONFIG_FILE" "$backup_env"
cp compose.yaml "$backup_compose"
# The checked-out Compose tag can already be newer than the running app.
# Record the running image so a failed release restores the actual previous version.
active_container=$("${compose[@]}" ps -a -q app)
active_image=$(docker inspect "$active_container" --format '{{.Config.Image}}')
[[ "$active_image" =~ ^[a-zA-Z0-9][a-zA-Z0-9._:/@-]+$ ]]
RELAYIX_ACTIVE_IMAGE="$active_image" RELAYIX_BACKUP_COMPOSE="$backup_compose" python3 - <<'BACKUP'
import os, re
from pathlib import Path
path=Path(os.environ['RELAYIX_BACKUP_COMPOSE'])
text,count=re.subn(r'(?m)^(    image:) .+$',lambda match:match[1]+' ${RELAYIX_IMAGE_OVERRIDE:-'+os.environ['RELAYIX_ACTIVE_IMAGE']+'}',path.read_text(),count=1)
if count != 1:raise SystemExit('App image declaration was not found for rollback')
path.write_text(text)
BACKUP
python3 scripts/database.py pg_dump -Fc > "$backup_root/db-before-deploy-$stamp.dump"
# Pull before changing the running service or its persistent configuration.
RELAYIX_IMAGE_OVERRIDE="$RELAYIX_DEPLOY_IMAGE" "${compose[@]}" pull app
export RELAYIX_DEPLOY_BACKUP_ENV="$backup_env"
export RELAYIX_DEPLOY_BACKUP_COMPOSE="$backup_compose"
rollback() {
  status=$?
  trap - ERR
  cp "$RELAYIX_DEPLOY_BACKUP_ENV" "$RELAYIX_CONFIG_FILE"
  cp "$RELAYIX_DEPLOY_BACKUP_COMPOSE" compose.yaml
  chmod 600 "$RELAYIX_CONFIG_FILE"
  "${compose[@]}" up -d app || true
  echo 'Deployment failed; previous environment restored and previous app restart requested.' >&2
  exit "$status"
}
trap rollback ERR
python3 - <<'PY'
import json, os, re
from pathlib import Path
path=Path(os.environ['RELAYIX_CONFIG_FILE'])
config=json.loads(path.read_text())
candidate=os.environ.get('RELAYIX_DEPLOY_CONFIG_CANDIDATE')
if candidate:
    incoming=json.loads(Path(candidate).read_text())
    if incoming.get('POSTGRES_PASSWORD')!=config.get('POSTGRES_PASSWORD'):
        raise SystemExit('GitHub database secret differs from the active database password; refusing deployment')
    from scripts.deployment import database_settings
    current_db=database_settings(config);candidate_db=database_settings(incoming)
    if any(current_db[key]!=candidate_db[key] for key in ['mode','host','port','user','database']):
        raise SystemExit('Database destination differs from the active deployment; transfer data before switching databases')
    if config.get('SECRETS_KEY') and incoming.get('SECRETS_KEY') != config['SECRETS_KEY']:
        raise SystemExit('Secret storage key differs from active deployment; refusing replacement or removal')
    config=incoming
config.pop('RELAYIX_IMAGE',None)
compose_path=Path('compose.yaml')
text=compose_path.read_text()
text,count=re.subn(r'(?m)^(    image:) .+$',lambda match:match[1]+' ${RELAYIX_IMAGE_OVERRIDE:-'+os.environ['RELAYIX_DEPLOY_IMAGE']+'}',text,count=1)
if count!=1:raise SystemExit('App image declaration was not found in compose.yaml')
compose_path.write_text(text)
config['COMPOSE_PROJECT_NAME']=os.environ['COMPOSE_PROJECT_NAME']
path.write_text(json.dumps(config,indent=2)+'\n');path.chmod(0o600)
PY
"${compose[@]}" up -d app
python3 - <<'PY'
import json, os, time
from urllib.request import urlopen
for attempt in range(30):
    try:
        with urlopen(os.environ['RELAYIX_HEALTH_URL'], timeout=3) as response:
            health = json.load(response)
        if health.get('status') == 'ok' and health.get('version') == os.environ['RELAYIX_EXPECTED_VERSION']:
            print('Deployment healthy: Relayix ' + health['version'])
            break
    except Exception:
        pass
    if attempt == 29:
        raise SystemExit('Expected healthy application version did not start')
    time.sleep(1)
PY
trap - ERR
