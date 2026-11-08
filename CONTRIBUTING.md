# Contributing to Relayix

Use a feature branch and describe the problem, final behavior and relevant validation in the pull request. Keep the English UI consistent across light/dark themes and mobile widths.

The release workflow runs all tests on GitHub before building/publishing the production image; PRs run Docker-free formatting/Clippy/ESLint checks; main runs the full suite once. Documentation-only changes skip both workflows. For optional developer reproduction, `python3 tests/deploy.py` verifies backup/rollback using fake Docker and `scripts/check.sh` uses Docker and Docker Compose v2. It builds the locked Rust/React application and runs API, queue, SMTP and Chromium checks against temporary databases and local mock providers. It never uses production credentials or sends to real channels. Screenshots and failure logs go to `/tmp/relayix-test-artifacts`.

Database migrations must be additive. Never edit applied migrations, rotate existing route keys or rename existing database volumes as part of a branding change. Preserve existing mailbox and provider configuration formats.

Commit Cargo.lock and web/package-lock.json. Keep private deployment JSON, dotenv files, temporary test credentials, backups, database dumps, credentials, build outputs and browser screenshots out of source control. See [GitHub setup](docs/GITHUB.md) for CI and releases.

Set the release version in `Cargo.toml`, run `python3 scripts/sync_release.py`, and add the changelog entry. The frontend embeds the Cargo version at build time; published OCI version labels are set by the release workflow. `python3 scripts/check_release.py` rejects stale generated manifests.

The Docker `verified` target used in the pre-publication job enforces `cargo fmt --check`, Clippy with warnings denied, Rust tests, frontend ESLint and message-safety unit tests. Bake builds the production candidate and verification target together. Runtime tests use the pinned candidate digest; publishing only promotes that digest after the full suite succeeds, without a rebuild. CI also audits Cargo dependencies and scans the runtime image for fixable high/critical vulnerabilities. Dependency license texts and inventories are generated from locked build dependencies and installed Python distributions during the image build, under `/usr/share/doc/relayix/licenses`; keep vendored upstream notices beside their sources.
