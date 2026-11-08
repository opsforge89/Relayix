#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export RELAYIX_TEST_PROJECT="${RELAYIX_TEST_PROJECT:-relayix-test-${GITHUB_RUN_ID:-local}-$$}"
[[ "$RELAYIX_TEST_PROJECT" == relayix-test-* ]] || { echo 'Test project must start with relayix-test-' >&2; exit 1; }
export RELAYIX_TEST_IMAGE="${RELAYIX_TEST_IMAGE:-relayix:0.8.7}"
export RELAYIX_TEST_CHECKS_IMAGE="${RELAYIX_TEST_CHECKS_IMAGE:-relayix-checks:0.8.7}"
export RELAYIX_TEST_BROWSER_IMAGE="${RELAYIX_TEST_BROWSER_IMAGE:-relayix-browser:0.8.7}"
export RELAYIX_TEST_ARTIFACTS="${RELAYIX_TEST_ARTIFACTS:-/tmp/relayix-test-artifacts}"
mkdir -p "$RELAYIX_TEST_ARTIFACTS"
python3 scripts/check_release.py
python3 tests/runtime_security.py
python3 tests/compose_modes.py
export RELAYIX_TEST_RUNTIME_DIR
RELAYIX_TEST_RUNTIME_DIR=$(mktemp -d /tmp/relayix-check-XXXXXXXX)
python3 tests/generate_runtime.py "$RELAYIX_TEST_RUNTIME_DIR"
compose=(docker compose --env-file "$RELAYIX_TEST_RUNTIME_DIR/compose.env" -p "$RELAYIX_TEST_PROJECT" -f tests/compose.yaml)
cleanup() {
  status=$?
  if (( status != 0 )); then "${compose[@]}" logs --no-color > "$RELAYIX_TEST_ARTIFACTS/compose.log" 2>&1 || true; fi
  "${compose[@]}" down --volumes --remove-orphans || true
  python3 tests/redact_artifacts.py "$RELAYIX_TEST_ARTIFACTS" "$RELAYIX_TEST_RUNTIME_DIR/credentials.json" || true
  rm -rf -- "$RELAYIX_TEST_RUNTIME_DIR"
  exit "$status"
}
trap cleanup EXIT
if [[ "${1:-}" != '--skip-app-build' ]]; then docker build --target verified -t "$RELAYIX_TEST_IMAGE" .; fi
python3 tests/startup.py
python3 tests/upgrade.py
python3 tests/compose_quickstart.py
python3 tests/compose_databases.py
docker run --rm -v "$PWD/tests:/tests:ro" --entrypoint python3 "$RELAYIX_TEST_IMAGE" /tests/telegram_transport.py
docker run --rm -v "$PWD/tests:/tests:ro" --entrypoint python3 "$RELAYIX_TEST_IMAGE" /tests/faketls.py
docker build -f tests/checks.Dockerfile -t "$RELAYIX_TEST_CHECKS_IMAGE" tests
docker build -f tests/browser.Dockerfile -t "$RELAYIX_TEST_BROWSER_IMAGE" tests
"${compose[@]}" up -d db mock app
"${compose[@]}" run --rm checks
"${compose[@]}" run --rm browser

"${compose[@]}" run --rm browser python /tests/release_082_browser.py
"${compose[@]}" run --rm browser python /tests/features_087_browser.py
"${compose[@]}" run --rm checks python /tests/features_087.py
"${compose[@]}" run --rm checks python /tests/release_083.py
python3 tests/runtime_083.py

python3 tests/release_084.py
