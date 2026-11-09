# GitHub setup and delivery

The checkout is prepared for a repository named `relayix`. The workflows derive the registry name from `github.repository`; the workflow has no hardcoded owner or registry password. Compose and documentation use the current public package namespace.

## First publication

The local origin is connected to https://github.com/opsforge89/Relayix.git and based on its existing LICENSE commit. Once the checks and review are complete, publish from this directory:

```bash
git remote -v  # origin: https://github.com/opsforge89/Relayix.git
git add .
git diff --cached --stat
git commit -m "Prepare Relayix 0.9.0 Beta"
git push -u origin main
```

Private configuration, local backups, dumps, image exports and generated outputs are ignored. Review staged files before the first commit. Relayix original source is Apache-2.0; preserve LICENSE, NOTICE and THIRD_PARTY_NOTICES.md, together with the dependency licenses generated into the runtime image, when distributing. Review the third-party inventory and run the secret checks before publication.

In repository settings enable Actions and Packages. Protect `main` against force pushes/deletion. Do not require a PR `verify` status: verification runs only before release publication, and `publish` requires its successful completion. Enable private vulnerability reporting and secret scanning if available. Dependabot is configured for Rust, npm and Actions.

## Continuous integration

`PR checks` runs Rust formatting, Clippy, `cargo test --locked`, ESLint and `npm test` directly on GitHub-hosted runners, with pinned toolchains and dependency caches. It does not use Docker, integration environments or package publishing credentials. Changes limited to `docs/**` and root `*.md` files skip this workflow. Do not make path-filtered PR checks required without a documentation-only fallback, since GitHub otherwise leaves their required statuses pending.

A non-documentation push to `main` starts `Release Docker image`, which calls the full `Verify` workflow once. The release trigger uses the same documentation filters; a deliberate manual dispatch can still run a release. The full suite includes source/history secret scans, deployment tests, Rust/frontend checks, dependency audit, runtime image scanning, API, queue, SMTP and Chromium regressions. No local tests/builds are part of this process. Artifacts remain available for seven days.

## Continuous delivery

Prepare a release PR with the Cargo version, synchronized manifests, changelog and documentation, then merge it to `main`:

1. Bake builds a local production OCI archive and the verification target together, sharing the exact runtime graph. The archive includes provenance/SBOM. Neither image is pushed to GHCR before tests; build caches use GitHub Actions storage.
2. CI pins the production digest, checks shared runtime layers/labels, and runs runtime/integration tests against the exact archive runtime. Rust/frontend checks are enforced by the verification target. Only after all checks pass is the archive uploaded as a one-day workflow artifact for the publish job.
3. After every check succeeds, publishing creates the annotated version tag on that commit. Skopeo uploads the passed archive under `sha-<commit>` with all attestations and `--preserve-digests`; a digest mismatch stops promotion. [`docker buildx imagetools create`](https://docs.docker.com/reference/cli/docker/buildx/imagetools/create/) assigns the version and `:beta` to that digest without rebuilding.
4. GitHub publishes the Beta release, standalone Compose/environment files, and documentation at that same tagged commit.

Failed candidates remain only on the ephemeral verification runner and are never pushed to GHCR. An interrupted successful publish may leave a passed `sha-<commit>` staging tag, but no untested image. GitHub cache/build-record artifacts are not releases. Promotion consumes the digest output, never a mutable staging tag. Version and `beta` tags are never updated after a failed verification.

Tag pushes do not start another workflow. Existing tags on a different commit are never moved: advance the Cargo version for a new release. Retry a failed workflow at its original commit with GitHub's rerun action. A manual dispatch on `main` deliberately bypasses the documentation path filter but retains all checks.

Publishing uses repository-scoped `GITHUB_TOKEN` permissions: verification has only `contents:read`; publishing needs `packages:write`/`contents:write` to promote/create the release. PR jobs have only `contents:read`. No personal token is needed for publication. Images are Beta and are not tagged `latest`.

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
- `RELAYIX_NETWORK_SUBNET` and `RELAYIX_NETWORK_GATEWAY` select the stable Compose bridge; unset proxy trust follows its gateway. `RELAYIX_WEBHOOK_RATE_LIMIT_PER_MINUTE` changes the webhook quota (default 600).
- `RELAYIX_TRUSTED_PROXIES`, `RELAYIX_OUTBOUND_ALLOWLIST`, `RELAYIX_DELIVERY_PROXY_CONCURRENCY`, `RELAYIX_DELIVERY_CHANNEL_CONCURRENCY`, `RELAYIX_DELIVERY_WORKERS` and `RELAYIX_EVENT_RETENTION_DAYS` configure forwarded client identity, outbound access, delivery concurrency and retention.
- `RELAYIX_DATABASE_MODE`, `RELAYIX_POSTGRES_HOST`, `RELAYIX_POSTGRES_PORT`, `RELAYIX_POSTGRES_USER`, `RELAYIX_POSTGRES_DB`, `RELAYIX_POSTGRES_SSLMODE` and optional `RELAYIX_POSTGRES_SSLROOTCERT` configure database access.
- `RELAYIX_POSTGRES_IMAGE`, `RELAYIX_POSTGRES_CLIENT_IMAGE` and `RELAYIX_DB_VOLUME` preserve database image and storage selection.

The synchronization command sets these Variables from the current private configuration. Passwords and Telegram API credentials are always Secrets.

`Deploy release` is a manual workflow on the default branch, targeting a dedicated Linux x64 runner labelled `relayix-deploy`. That trusted runner needs Docker Compose v2, Bash and Python 3 and access to the deployment host. Pull requests and CI run on GitHub-hosted runners, not that server.

The workflow renders a candidate JSON from Secrets/Variables in `RUNNER_TEMP`, then backs up the active private configuration/database, pulls the image, checks that the candidate database password agrees with the existing one, updates the image in `compose.yaml`, replaces only the app and verifies health. On failure it restores the previous private settings and Compose image. It removes the temporary candidate and registry login afterwards. Active private configuration remains outside the checkout for service restarts and backup operations.

For manual operation, use `scripts/compose.sh`; it loads the private JSON into the Docker process environment and disables dotenv discovery. There is no production `.env` in the repository. Deployment credentials must not be put into repository Variables, workflow YAML, image build arguments, test fixtures or CI artifacts.

The first release workflow validates the account/runner settings before publishing. See [publication checks](PUBLICATION.md). Source preparation and local tests do not publish files or configure remote Secrets automatically.

References: [GitHub Actions secrets](https://docs.github.com/en/actions/security-for-github-actions/security-guides/using-secrets-in-github-actions), [GitHub container publishing](https://docs.github.com/en/actions/tutorials/publish-packages/publish-docker-images).

## Bundled or external PostgreSQL

The standard Docker-only startup guide is [DEPLOYMENT.md](DEPLOYMENT.md). This automated deployment workflow uses [private JSON settings](DEPLOYMENT_ADVANCED.md); all of its database fields live in that file. `DATABASE_MODE=local` selects the bundled database; `external` uses `POSTGRES_HOST`, `POSTGRES_PORT`, `POSTGRES_USER`, `POSTGRES_DB`, `POSTGRES_PASSWORD` and TLS settings without creating a database service or volume. The workflow retains those fields from the active base configuration. `RELAYIX_POSTGRES_PASSWORD` must match the existing database password in either mode.

The deployment script backs up either database through an explicitly requested ephemeral PostgreSQL client before replacing the app. The client is not a Compose service. Select `POSTGRES_CLIENT_IMAGE` for the external server's major version if needed. A candidate cannot silently change database mode/host/port/user/name; perform an explicit backup/restore transfer before changing the active destination. Optional `POSTGRES_SSLROOTCERT` paths refer to files already present on the deployment host, not on the GitHub-hosted build runner.
