# Relayix Agent Instructions

Maintain this file in English.

## Local Work

- Edit files and prepare Git changes locally.
- Run all tests and validation only in GitHub Actions. Do not run local tests, application or image builds, dependency audits, security scanners, or integration environments.
- Do not add post-publication checks or download published images unless the user explicitly requests them.

## Release Workflow

Follow this sequence:

1. Prepare a PR with the code, Cargo version, synchronized release manifests, changelog, and documentation.
2. Merge the PR into `main` when the user has authorized publication.
3. The push to `main` starts one GitHub workflow that runs the complete test and scan suite once.
4. Only after all checks succeed, create `v<version>` on the exact commit that was tested.
5. Build the production image without rerunning tests, then publish the versioned image and `:beta`.
6. Publish the GitHub Beta release, release notes, `compose.yaml`, and `relayix.env.example`. Documentation must belong to the same tested commit.

PR pushes and tag pushes must not start additional verification workflows. `check.yml` is called only by `release.yml`. Publishing must depend on a successful `verify` job through `needs: verify`.

Integration tests require an isolated `verified` image; never publish that image. The `production` Docker target must not depend on the Rust or frontend check stages. Test stages run before the production image build, not during it.

If any check fails, stop publication. Never bypass the verification dependency. Never move an existing tag to another commit: increment the version instead. Retry failed runs in GitHub Actions against the original commit, without starting parallel duplicate runs.

Do not require a verification status before merging a PR: tests run after the push to `main` and before publication. Preserve the other branch protection settings.

## Data and Secrets

- Preserve existing passwords, route tokens, volume names, and private deployment settings unless the task explicitly requires changing them.
- Never delete database volumes when upgrading or recreating networks. Use `down` without `--volumes` to preserve data.
- Do not edit applied SQL migrations. Add a new migration for schema changes.
- Keep private settings, secrets, database dumps, backups, and generated test credentials outside Git. Never expose secrets in commands, messages, pull requests, or logs.
