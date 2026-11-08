# Relayix Agent Instructions

## Language

Write all repository documentation and code in English only. This includes code comments, identifiers, developer-facing strings, UI/API messages, commit messages, pull request descriptions, and release notes. Maintain this file and AGENTS.md in English as well.

## Local Work

- Edit files and prepare Git changes locally.
- Run all tests and validation only in GitHub Actions. Do not run local tests, application or image builds, dependency audits, security scanners, or integration environments.
- Do not add post-publication checks or download published images unless the user explicitly requests them.

## Release Workflow

Follow this sequence:

1. Prepare a PR with the code, Cargo version, synchronized release manifests, changelog, and documentation.
2. Merge the PR into `main` when the user has authorized publication.
3. PRs run only lightweight Rust formatting, Clippy and ESLint checks on GitHub, without Docker or integration tests.
4. A push to `main` starts the full suite once. Changes limited to `docs/**` and root `*.md` files skip PR/release workflows.
5. Build the production candidate once under `sha-<commit>` and the verification target in one Bake invocation, sharing the same runtime graph. Candidate tags are unverified staging artifacts, never releases.
6. Pin the production digest and run all runtime/integration tests against that exact candidate. The verification target enforces Rust/frontend checks; validate that its runtime layers match the candidate.
7. Only after every check succeeds, create `v<version>` on the tested commit and use `imagetools create` to promote the pinned candidate digest to the version and `:beta`, without rebuilding.
8. Publish the GitHub Beta release, release notes, `compose.yaml`, and `relayix.env.example`. Documentation must belong to the same tested commit.

Tag pushes must not trigger another workflow. The full `check.yml` workflow is called only by `release.yml`, with `publish` depending on successful `verify`. PR checks have no package publishing permissions. Never promote the mutable staging tag directly: use the verified digest output. Never publish the verification target.

If any check fails, stop publication. Never bypass the verification dependency. Never move an existing tag to another commit: increment the version instead. Retry failed runs in GitHub Actions against the original commit, without starting parallel duplicate runs.

The full verification status is not required before merging: it runs after the push to `main` and before publication. Preserve other branch protection settings. Path-filtered PR checks must not be made required unless documentation-only PRs can still merge.

## Data and Secrets

- Preserve existing passwords, route tokens, volume names, and private deployment settings unless the task explicitly requires changing them.
- Never delete database volumes when upgrading or recreating networks. Use `down` without `--volumes` to preserve data.
- Do not edit applied SQL migrations. Add a new migration for schema changes.
- Keep private settings, secrets, database dumps, backups, and generated test credentials outside Git. Never expose secrets in commands, messages, pull requests, or logs.
