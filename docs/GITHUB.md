# GitHub setup and delivery

The checkout is prepared for a repository named `relayix`. The workflows derive the registry name from `github.repository`; the workflow has no hardcoded owner or registry password. Compose and documentation use the current public package namespace.

## First publication

The local origin is connected to https://github.com/opsforge89/Relayix.git and based on its existing LICENSE commit. Once the checks and review are complete, publish from this directory:

```bash
git remote -v  # origin: https://github.com/opsforge89/Relayix.git
git add .
git diff --cached --stat
git commit -m "Prepare Relayix 0.8.3 Beta"
git push -u origin main
```

Private configuration, local backups, dumps, image exports and generated outputs are ignored. Review staged files before the first commit. Relayix original source is Apache-2.0; preserve LICENSE, NOTICE and THIRD_PARTY_NOTICES.md, together with the dependency licenses generated into the runtime image, when distributing. Review the third-party inventory and run the secret checks before publication.

In repository settings enable Actions and Packages. Protect `main` and require the `verify` job before merging. Enable private vulnerability reporting and secret scanning if available. Dependabot is configured for Rust, npm and Actions.

## Continuous integration

`Verify` runs on pull requests, pushes to `main` and manual dispatch. Updates limited to `README.md` and `docs/` keep the required `verify` check but skip image builds and all application tests, running only the lightweight private-file scan. Changes to application files, release-tag verification and manual verification retain the full build/test flow. It builds the Docker image with Rust formatting, Clippy and `cargo test --locked --release`, checks the React app with ESLint and unit tests, audits Cargo dependencies, scans the runtime image for fixable HIGH/CRITICAL vulnerabilities, and runs `scripts/check.sh --skip-app-build`. Tests use an isolated PostgreSQL, local SMTP/HTTP mocks and Chromium. Test resources are cleaned up even on failure. Screenshots and failure logs are uploaded for seven days. Fork PRs run without registry publishing credentials.

## Continuous delivery

When a tested version is ready:

```bash
git tag v0.8.3
git push origin v0.8.3
```

`Release Docker image` runs the complete CI again, checks that the tag agrees with both Cargo and npm versions, and publishes `ghcr.io/opsforge89/relayix:0.8.3` and `:beta`. It uses the repository-scoped `GITHUB_TOKEN` with `packages:write`; no personal token is required for publication. It includes source/revision labels, build provenance and an SBOM. Release tags also publish the single standalone Compose file and blank environment template as GitHub Release downloads. Tags are beta releases; the pipeline deliberately does not label them `latest`.

`Release Docker image` can also be dispatched manually with an existing numeric version, such as `0.8.3`. Both verification and publishing check out that exact version tag, and the image revision label records the tagged commit. This supports retrying first-publication tag events without moving a release tag.

Container package visibility is controlled in GitHub Packages. For anonymous pulls set the package public. For private pulls authenticate on the deployment host with a token allowed to read that package; do not commit the credential.

## Deployment configuration and GitHub Secrets

Use the `production` GitHub environment. The deployment workflow runs only on the default branch; existing environment protection settings are preserved. Keep Compose project `relayix` and the data volume selected by `RELAYIX_DB_VOLUME` for an existing installation.

Store these **Secrets** in the production environment:

| Name | Existing private configuration field |
| --- | --- |
| `RELAYIX_POSTGRES_PASSWORD` | `POSTGRES_PASSWORD` |
| `RELAYIX_ADMIN_TOKEN` | `ADMIN_TOKEN` |
| `RELAYIX_ADMIN_PASSWORD` | `ADMIN_PASSWORD` (bootstrap credential) |

Use the current values. Do not generate a new database password for an existing database. With an authenticated GitHub CLI, `python3 scripts/sync_github_secrets.py` creates the environment if needed and synchronizes Secrets and Variables directly from the external private configuration over stdin, without placing them in shell history or printing them. It preserves existing environment protection settings. `--dry-run` lists setting names without contacting GitHub or showing values. Optional Telegram API credentials are also uploaded as Secrets when configured. This command needs access to manage environment Secrets and Variables; source preparation alone does not create them remotely.

Use **Variables** for nonsecret configuration:

- `RELAYIX_DEPLOY_DIR`: absolute source/deployment directory.
- `RELAYIX_CONFIG_FILE`: optional absolute private JSON path (default `../private/relayix-config/deployment.json` relative to the checkout).
- `RELAYIX_COMPOSE_PROJECT`: `relayix` for this existing installation.
- `RELAYIX_HEALTH_URL`: `http://192.168.39.110:8080/healthz` for this installation.
- `RELAYIX_ADMIN_USERNAME`, `RELAYIX_BIND_ADDRESS`, `RELAYIX_PORT`, `RELAYIX_COOKIE_SECURE`, `RELAYIX_ALLOW_INSECURE_HTTP` and `RELAYIX_RUST_LOG` configure the app.
- `RELAYIX_DATABASE_MODE`, `RELAYIX_POSTGRES_HOST`, `RELAYIX_POSTGRES_PORT`, `RELAYIX_POSTGRES_USER`, `RELAYIX_POSTGRES_DB`, `RELAYIX_POSTGRES_SSLMODE` and optional `RELAYIX_POSTGRES_SSLROOTCERT` configure database access.
- `RELAYIX_POSTGRES_IMAGE`, `RELAYIX_POSTGRES_CLIENT_IMAGE` and `RELAYIX_DB_VOLUME` preserve database image and storage selection.

The synchronization command sets these Variables from the current private configuration. Passwords and Telegram API credentials are always Secrets.

`Deploy release` is a manual workflow on the default branch, targeting a dedicated Linux x64 runner labelled `relayix-deploy`. That trusted runner needs Docker Compose v2, Bash and Python 3 and access to the deployment host. Pull requests and CI run on GitHub-hosted runners, not that server.

The workflow renders a candidate JSON from Secrets/Variables in `RUNNER_TEMP`, then backs up the active private configuration/database, pulls the image, checks that the candidate database password agrees with the existing one, updates the image in `compose.yaml`, replaces only the app and verifies health. On failure it restores the previous private settings and Compose image. It removes the temporary candidate and registry login afterwards. Active private configuration remains outside the checkout for service restarts and backup operations.

For manual operation, use `scripts/compose.sh`; it loads the private JSON into the Docker process environment and disables dotenv discovery. There is no production `.env` in the repository. Deployment credentials must not be put into repository Variables, workflow YAML, image build arguments, test fixtures or CI artifacts.

After connection/publication, run CI once in GitHub to validate account/runner settings. See [publication checks](PUBLICATION.md). Source preparation and local tests do not publish files or configure remote Secrets automatically.

References: [GitHub Actions secrets](https://docs.github.com/en/actions/security-for-github-actions/security-guides/using-secrets-in-github-actions), [GitHub container publishing](https://docs.github.com/en/actions/tutorials/publish-packages/publish-docker-images).

## Bundled or external PostgreSQL

The standard Docker-only startup guide is [DEPLOYMENT.md](DEPLOYMENT.md). This automated deployment workflow uses [private JSON settings](DEPLOYMENT_ADVANCED.md); all of its database fields live in that file. `DATABASE_MODE=local` selects the bundled database; `external` uses `POSTGRES_HOST`, `POSTGRES_PORT`, `POSTGRES_USER`, `POSTGRES_DB`, `POSTGRES_PASSWORD` and TLS settings without creating a database service or volume. The workflow retains those fields from the active base configuration. `RELAYIX_POSTGRES_PASSWORD` must match the existing database password in either mode.

The deployment script backs up either database through an explicitly requested ephemeral PostgreSQL client before replacing the app. The client is not a Compose service. Select `POSTGRES_CLIENT_IMAGE` for the external server's major version if needed. A candidate cannot silently change database mode/host/port/user/name; perform an explicit backup/restore transfer before changing the active destination. Optional `POSTGRES_SSLROOTCERT` paths refer to files already present on the deployment host, not on the GitHub-hosted build runner.
