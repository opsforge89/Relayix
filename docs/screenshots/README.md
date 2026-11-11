# Interface screenshots

These PNGs were captured from the verified Relayix **0.12.0 Beta** candidate, commit [`6ba970ca7cc8aed5fd953e53c38356ab6582dfdd`](https://github.com/opsforge89/Relayix/commit/6ba970ca7cc8aed5fd953e53c38356ab6582dfdd), during [the complete candidate verification](https://github.com/opsforge89/Relayix/actions/runs/42980899227) on 2026-11-11.

`tests/readme_capture.py` creates an isolated temporary deployment, seeds fictional destinations, routes, notifications and an account, then captures the actual Chromium interface. It runs only in GitHub Actions as part of the single pre-publication verification pipeline. The browser viewport is 1680 × 1100 CSS pixels; full-page screenshots can be taller. The dashboard uses the dark theme; other views use the light theme.

The five images cover the dashboard, channels/integrations, template editor with rendered preview, route TTL/quiet hours and administrator audit. They were visually reviewed and copied byte-for-byte from the `verification-artifacts` archive. Credentials, route keys and authenticator secrets are not displayed. No image editing or synthetic UI rendering was used.

To refresh them, take the matching `readme-*.png` files from a successful candidate verification artifact, review the images, replace these `relayix-*.png` files and update the README captions and provenance in a documentation-only PR. Do not rebuild, pull published application images or run post-publication integration checks just to update screenshots.
