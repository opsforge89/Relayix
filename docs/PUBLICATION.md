# Publication boundary

`migrations/` belongs in Git: it contains versioned SQL schema, not a database dump. Applied migrations must not be edited. They are required when installing and upgrading Relayix.

Private backups for this installation are outside the checkout, at `../private/relayix-backups/`, with directory mode 700 and file mode 600. `scripts/deploy.sh` uses that external directory by default; `RELAYIX_BACKUP_DIR` can override it. The old `backups/` path is still ignored as a safeguard. No dotenv file is used in the checkout. Private settings are in `../private/relayix-config/deployment.json`; `config/deployment.example.json` contains nonsecret defaults only. The publication guard rejects any indexed dotenv file.

Before every initial publication:

```bash
git add .
python3 scripts/check_secrets.py
mkdir -p /tmp/relayix-secret-scan
git checkout-index --all --force --prefix=/tmp/relayix-secret-scan/
docker run --rm -v /tmp/relayix-secret-scan:/scan:ro ghcr.io/gitleaks/gitleaks:v8.30.1 dir /scan --config=/scan/.gitleaks.toml --redact=100 --no-banner
docker run --rm -v "$PWD:/repo:ro" ghcr.io/gitleaks/gitleaks:v8.30.1 git /repo --config=/repo/.gitleaks.toml --redact=100 --no-banner
git diff --cached --stat
```

Use a fresh empty temporary export directory each time. The scanner checks indexed paths, private-key markers and exact matches against locally available deployment secrets; Gitleaks adds provider-pattern and history checks. Never attach raw scanner output containing unredacted values. The only custom Gitleaks exception is the exact public placeholder `YOUR_ROUTE_KEY`; test folders and source files are not broadly exempted. CI runs these checks before building/publishing.

A clean scan reduces publication risk; it does not prove that no unknown secret exists. Check GitHub's secret scanning after publication as another independent signal. Do not commit live credentials to tests, screenshots, issue reports, Docker build arguments or workflows. Database credentials and channel secrets are intentionally stored in the private database and may exist in private backups; they are not publication inputs.

Relayix source uses Apache-2.0. Keep LICENSE, NOTICE and THIRD_PARTY_NOTICES.md with distributions, together with the dependency license bundle generated into `/usr/share/doc/relayix/licenses` during the image build. Third-party software retains its own license; service trademarks are not granted by Apache-2.0. See the inventories for dependencies and retained upstream license texts.

Test credentials are generated per run in a private temporary directory, shared with the mock/check containers, masked in GitHub Actions logs and removed after the stack shuts down. CI needs no stored admin/database/provider passwords. `tests/smoke.py` and `tests/browser.py` read their base URL from the test environment.
