# Changelog

## 0.12.0 Beta — 2026-11-11

- Add offline master-key rotation with SECRETS_KEY_OLD, authenticated key identifiers, atomic channel/TOTP re-encryption, stale-writer rejection and recoverable private configuration updates.
- Reset the dedicated public schema transactionally during full restore so newer objects cannot block migrations after restoring an older snapshot.
- Use PostgreSQL-supported timeout units for offline key rotation.
- Upgrade legacy secret envelopes automatically with the existing key; reject wrong keys without rewriting data.
- Audit account changes, password changes and session sign-in/out; bound failed known-account sign-in records to the throttling window without logging submitted credentials or unknown usernames.
- Document independent backup password storage, temporary-disk sizing/TMPDIR configuration and the default 1 GiB attachment budget with retention behavior.
- Capture current README interface screenshots from the verified candidate in Chromium.

## 0.11.0 Beta — 2026-11-11

- Expire undelivered notifications using route TTL and X-Relay-TTL, including paused, scheduled, grouped and fallback work; retain history and prevent retry revival.
- Record successful channel, route, key and queue controls atomically with actor identity; add an administrator audit-log view with 180-day retention.
- Add optional SECRETS_KEY AES-256-GCM encryption for channel credentials, webhook body templates and TOTP seeds, atomic migration and fail-closed key validation.
- Restore the public schema search path before transactional session/lease recovery after pg_restore.
- Add full password-encrypted database backups with attachments and credentials, Argon2id key derivation, authenticated streaming, verification before restore, atomic PostgreSQL restore and recovery of private application settings.

## 0.10.0 Beta — 2026-11-10

- Accept bounded multipart notifications and standalone files on authenticated route file endpoints without reserving new JSON payload fields.
- Persist original file bytes in PostgreSQL alongside incoming events, with atomic storage accounting, private authenticated downloads and existing event retention.
- Forward native files through Mattermost bot/API channel and direct-message delivery, plus Telegram Bot API, HTTP proxies, MTProto and WEB transports.
- Preserve files across pauses, schedules, retries and fallback. Retain acknowledged upload/send steps and separate grouping windows when file contents or metadata differ.
- Show attachments in incoming history and document supported modes, queue behavior and upload/storage limits.

## 0.9.2 Beta — 2026-11-09

- Restrict cancelling a channel's queued notifications and alerts to administrators; members can still pause and resume with sending.
- Expire cancelled alert deliveries under the same retention policy and bounded sweep as sent/failed alerts, while preserving recent and pending work.
- Show original message counts, including every grouped event and queued alert, before backlog cancellation and report grouped message totals on completion.

## 0.9.1 Beta — 2026-11-09

- Use explicit database result columns so additive schema changes do not invalidate cached SELECT/RETURNING result shapes.

- Move explicit webhook schedules to X-Relay-Delay/X-Relay-Deliver-At headers, preserving arbitrary JSON source fields and schedule-aware idempotency.
- Explain that every non-urgent deduplicated event, including a unique message, waits until its fixed grouping window closes.
- Add an explicit resume choice to send or cancel the complete queued notification/alert backlog, retaining incoming events and attempt history.
- Offer opt-in TOTP to member accounts and isolate security-settings throttling per session so stolen sessions cannot lock the account's sign-in factor.
- Document the full API authority of ADMIN_TOKEN and the unencrypted storage of authenticator/channel secrets.

## 0.9.0 Beta — 2026-11-09

- Add manual channel pause/resume with optional expiry, retaining queued notifications and alerts. Resume also resets automatic cooldowns.
- Add route quiet hours with IANA timezones and urgent-priority bypass, plus RFC 3339 deliver_at or bounded delay webhook scheduling.
- Coalesce identical notifications into one ×N delivery and collect low-priority events into hourly or daily digests, retaining every incoming event and durable grouping across replicas.
- Add editable curl, standard-library Python and Bash examples to route integration guides.
- Add opt-in administrator TOTP, password-confirmed enrollment, one-use authenticator/recovery codes, other-session revocation and API-token recovery.

- Cap provider-supplied retry delays at 24 hours across HTTP headers/dates, Matrix JSON and Telegram transports.
- Clear channel cooldowns and release queued retry deadlines atomically after a successful channel save, preserving attempt history.
- Count channel failures toward automatic pauses; retain individual message rejection errors without pausing healthy traffic or losing alert error details.
- Audit the active Rust dependency graph against RustSec with cargo-deny and forbid unused SQLite/MySQL drivers, while retaining the full pre-publication verification gate.

## 0.8.8 Beta — 2026-11-08

- Preview per-channel outgoing messages in channel forms and route tests, including formatting, actual size budgets, truncation markers and fallback recipients. Previews send no network requests or events; custom webhook bodies remain administrator-only.
- Honor HTTP Retry-After seconds/dates, Telegram flood delays and Matrix retry_after_ms. Stop permanent HTTP/SMTP/Telegram errors immediately; retry temporary failures with exponential backoff and expose failure categories and the next attempt time.
- Pause a channel after three consecutive identical failures for at least 60 seconds, or immediately for a provider retry delay. Share cooldowns and a single recovery probe across notification/alert queues and replicas; retain backlog and resume after recovery.
- Require explicit secrets after destination, proxy or channel-type changes in channel updates and draft diagnostics, preventing masked-credential retargeting for every integration.
- Bound Rocket.Chat delivery text to 5,000 UTF-16 units, including the title and truncation marker, while preserving complete history.
- Show incoming and integration-specific limits in channel forms and document all 13 integrations in a capabilities and limits table.

## 0.8.7 Beta — 2026-11-08

- Restrict webhook body-template visibility and channel/draft management to administrators, preventing masked-key retargeting by members.
- Validate channel names on create/update/import and identify invalid legacy records in export errors.
- Bound Pushover/ntfy body/title fields with character/UTF-8-byte policies and Google Chat's 32,000-byte JSON budget; retain complete event history.

- Add a shared Telegram-proxy worker budget across channels/queues/replicas (default 2) and 20 ms retries for claim-lock contention while preserving idle backoff.
- Add Rust/frontend unit tests to Docker-free PR checks and keep unverified OCI candidates off GHCR.
- Truncate oversized Telegram/Discord deliveries with a visible marker and Unicode/markup-safe bounds while preserving full history.
- Add JSON webhook body templates, PostgreSQL-coordinated per-route quotas, and administrator-only JSON configuration export/import as disabled credential-free drafts, with corresponding UI controls.

- Add Docker-free PR checks for Rust formatting, Clippy and ESLint; skip documentation-only PR and main changes using `docs/**` and root `*.md` filters.
- Build production/verification targets together once, test the local OCI archive runtime, and upload the passed artifact only after success. Promote its preserved digest to version/beta without rebuilding; failed candidates never reach GHCR.
- Warn when an untrusted peer supplies X-Forwarded-For, including stale gateway configurations. Bound warnings to one per minute per process without logging header contents.
- Add `DELIVERY_CHANNEL_CONCURRENCY` (default 2) across notification/alert queues and app replicas. Serialize channel claims in PostgreSQL, recount capacity after acquiring the channel lock, and index active/queued work so one slow channel cannot occupy every default worker.
- Clarify the distributed-login throttle's limits and the independent ADMIN_TOKEN recovery API; add regression coverage for healthy-channel progress under a held Telegram backlog.

## 0.8.6 Beta — 2026-11-08

- Run the complete verification suite once in the release workflow, before building/publishing the production image. Trigger the release pipeline only on main pushes (or a manual retry), removing PR/tag and standalone verification triggers; create the version tag after tests pass.
- Split Docker verification stages from production stages: the verification runtime enforces Rust/frontend checks, while the production build does not rerun them. API/SMTP/browser tests and security scans must pass before publication.
- Include the proxy, quota, login and Telegram fixes listed under 0.8.5. Its tag was created, but the duplicate publication workflow was cancelled before image/release publication in favor of this version.

## 0.8.5 Beta — 2026-11-08

- Fix the standalone Compose bridge subnet/gateway, trust only that gateway by default, permit explicit network/proxy overrides, and warn when the application has no trusted proxies. Existing installations must recreate the bridge without removing volumes and remove stale/empty proxy assignments to adopt the default.
- Reject all-address trusted-proxy networks, including IPv4-mapped equivalents.
- Start Telegram MTProto/WEB transport deadlines after acquiring a subprocess slot; keep renewing delivery/alert leases during capacity waits.
- Make the per-client webhook quota configurable with `WEBHOOK_RATE_LIMIT_PER_MINUTE` (default 600).
- Add a PostgreSQL-coordinated soft account throttle across client IPs: after 30 failures in 15 minutes, allow one verification every two seconds without extending the gate for rejected requests. Successful login clears the account counter. Add one compatible migration column.

## 0.8.4 Beta — 2026-11-08

- Add explicit CIDR-based `TRUSTED_PROXIES` and select client addresses from trusted X-Forwarded-For chains for both request quotas and failed-login windows. Ignore forwarded headers from untrusted peers.
- Aggregate IPv6 request and login-failure identities by /64, including canonical handling of IPv4-mapped IPv6 addresses.
- Renew active delivery leases while waiting for Telegram capacity or network I/O; cancel work if ownership is lost or renewal fails. Preserve the existing 60-second send deadline and attempt-number fencing.

## 0.8.3 Beta — 2026-11-08

- Block private and special-use outbound addresses, check DNS answers at connection time, disable redirects/environment proxies and add an explicit IP/CIDR `OUTBOUND_ALLOWLIST` for internal services. Pin SMTP and Telegram proxy addresses, retaining TLS hostnames.
- Rate-limit login and webhook requests by peer IP, isolate failed-login windows by IP and username, and add CSP, frame denial, MIME sniffing protection and a referrer policy.
- Bind the Compose host port to localhost by default and include a Caddy HTTPS example.
- Run configurable parallel delivery workers, cap Telegram subprocesses, drain active deliveries on shutdown, check PostgreSQL readiness and include an image healthcheck.
- Add configurable completed-history retention (30 days by default; 0 disables deletion), bounded maintenance, trigram search indexes and transactionally maintained event tags.
- Format Rust, require Clippy and frontend ESLint in builds, audit dependencies and scan runtime images in CI. Split API modules from startup and parse adapter settings through a typed enum while preserving stored JSON.
- Remove the package installer and its bundled vulnerable dependencies from the runtime image after installing locked transport packages.
- Generate dependency license bundles at image build time and derive frontend/release manifests from the Cargo version.
- Correct 0.8.2 preview test fixtures to include required template title fields.

## 0.8.2 Beta — 2026-11-08

- Preview and retry up to 500 final-failure deliveries matching current filters. Confirmation is account-owned, expires after 15 minutes and is idempotent; changed delivery states are skipped.
- Preserve template versions, compare saved layouts and restore older content as a new version without changing event snapshots.
- Add optional route payload validation for required fields, JSON types, numeric limits and safe HTTP/HTTPS URLs, with structured field errors before queueing.
- Clone routes and templates into editable drafts; cloned routes receive fresh endpoints and keys on save.
- Configure personal delivery failure subscriptions and recipients in My profile, with final-failure, first-error and threshold triggers, cooldown, test sends and isolated delivery history.
- Compute template status from configurable passed/failed counter paths: failures take precedence; invalid or missing counters cannot imply success.

## 0.8.1 Beta — 2026-11-07

- Reorder table columns with drag-and-drop or arrow buttons and save column order to the user account.
- Use the shared Overview/Technical event drawer for delivery details.
- Unify View all buttons, place Columns and View all in one toolbar, and share channel actions between Dashboard and Channels.
- Group template fields by category with descriptions, examples, search, card/list layouts and insertion at the cursor. All combines suggested fields with detected JSON fields without duplicates.
- Add editable backup report and scheduled job examples, for five templates in total. Keep monitoring and personal delivery alerts deferred.

## 0.8.0 Beta — 2026-11-07

- Add user-created message templates with optional route assignment, fake-data examples, read-only preview, literal variables, failure-only links and immutable per-event rendering history.
- Add configurable route icons and a compact Channels/Routes dashboard. Preserve the current delivery table without a Result column and scope error styling to recipient chips.
- Add searchable primary/fallback recipient selection, service filters, selected-only review and bulk selection for large channel lists.
- Replace expanding incoming-event cells with an Overview/Technical side panel.
- Provide individual read-only diagnostics for every integration, with provider-specific checks and explicit unverified stages.

## 0.7.4 Beta — 2026-11-06

- Show one event per row in Dashboard and Deliveries, with named service recipients and individual sent/failed/retrying/pending indicators. Keep attempts and errors in delivery details.
- Filter channel and status against the same recipient, highlight matches and retain the other recipients for context. Sort and paginate complete events on the server; keep the existing per-delivery API available.
- Render bold text and safe HTTP/HTTPS links in the full message viewer while preserving the original text for copying.
- Dismiss every Columns menu with an outside click/tap or Escape; keep selections open while editing and migrate existing table preferences.

## 0.7.3 Beta — 2026-11-06

- Automatically format Telegram and Mattermost messages, including optional headings, bold text and links. Remove Message content and Text formatting fields; legacy settings no longer suppress titles or disable formatting.
- Keep titles and tags optional, preserve original event text and tag metadata, and retain full message viewing for untitled events.
- Shorten the README to features, Compose setup, a webhook example and updates; move operational details and API documentation into the linked reference.

## 0.7.2 Beta — 2026-11-06

- Deliver the message without an automatic heading when the original title is omitted, empty or whitespace-only. History labels never become delivery headings; title-only events are not duplicated.
- Add independent Message content and Text formatting settings for Telegram and Mattermost, including Message only, portable **bold** text and Markdown-style HTTP/HTTPS links. Preserve existing channel settings and original-text behavior by default.
- Show View message for untitled rows in recent deliveries and incoming history; keep long bodies in a separate viewer.
- Render portable formatting as escaped Telegram HTML for Bot API and UTF-16-aware MTProto entities, and escaped Mattermost Markdown for webhook, channel and direct-message delivery. Preserve message spacing and line breaks.

## 0.7.1 Beta — 2026-11-06

- Update GitHub/GHCR links and deployment defaults to opsforge89 after the account rename.
- Replace the plain README title and technical introduction with the existing Relayix brand header, preserving its proportions in light and dark themes.

- Keep attempt counts in Attempts and move the Details action into its own column.
- Add account-persisted column visibility and click-to-sort headers to every table. Event/delivery/alert sorting runs on the server before result limits and pagination; default to newest first.
- Run Telegram diagnostics immediately from the channel or edit form and display compact inline stages with green/red icons; remove the separate dialog and redundant permissions warning. Known permission failures are reported as chat-access failures.
- Highlight fallback recipient settings in the route editor and explicitly describe the five-attempt trigger.

## 0.7.0 Beta — 2026-11-05

- Add searchable, paginated incoming webhook history, including accepted events with no queued deliveries and their original routing decisions.
- Preview route decisions without storing events or sending messages.
- Preserve delivery attempt counts and detailed attempt history across manual retries; show attempt errors, timings and fallback status.
- Configure explicit fallback recipients per route. After a primary delivery exhausts five attempts, queue each fallback once per event without recursive fallback.
- Support route-scoped Idempotency-Key headers with concurrent duplicate suppression and conflict detection for changed payloads.
- Add read-only Telegram diagnostics for proxy transport, credentials, destination access and permission checks, without sending messages or exposing credentials.
- Validate startup settings and report actionable database authentication, TLS, network and configuration errors without connection credentials.

## 0.6.4 Beta — 2026-11-05

- Distinguish plain MTProxy secrets from dd/ee markers by their full length and preserve raw keys beginning with those bytes; cover each prefix deterministically.

- Keep webhook priority and tags optional (default priority 3 and empty tags). Route tags are UI labels only and never gate delivery; history filters match route labels or message tags. Preserve the legacy required_tag API field as UI metadata.
- Return routing diagnostics with queue counts, matched filters and per-recipient exclusion reasons, and correlate routing decisions in service logs without exposing credentials or message bodies.
- Add Test webhook to Routes, with priority, optional comma-separated tags, JSON copying, real event submission and a link to delivery results.

- Focus the first editable field when opening a dialog and restore focus without scrolling when closing it. Use an explicit input line height for consistent caret alignment.
- Show page-level feedback in a fixed notification stack so manual refresh, failures and notification dismissal keep the page in place. Stabilize scrollbar space and the refresh timestamp width.
- Cover field focus and keyboard input at 150% scale, and verify page/header/scroll geometry during manual and automatic refresh.

## 0.6.3 Beta — 2026-11-05

- Use one root `compose.yaml` with only the application and optional PostgreSQL services. Select local/external PostgreSQL and TLS in `.env`.
- Read database settings natively in the application; remove inline Python, mode-specific overlays and the database-tools service.
- Name the containers `relayix-app` and `relayix-db` and their shared network `relayix-network`; preserve existing named database volumes.
- Align the README, setup guide and downloadable release bundle with the single Compose configuration.

- Preserve every byte of FakeTLS keys beginning with `dd` or `ee` when passing them to Telethon; cover both prefixes with deterministic wire tests.

## 0.6.2 Beta — 2026-11-05

- Distribute standalone Compose files and a blank environment template with the release: download, set two passwords and run `docker compose up -d`; no source build or host Python is needed.

- Use Debian Trixie throughout application, build and test images with Rust 1.99.1, Node.js 26.13.0 LTS, Python 3.15.2 and PostgreSQL 17.11.
- Update stable Rust and npm dependencies, regenerate Python locks for Python 3.15, and refresh dependency licenses and Playwright.
- Support bundled, external and host PostgreSQL through one private settings file, including custom CA certificates and database backup tools.
- Keep the application image in `compose.app.yaml`; update by changing its tag and recreating only the app. Preserve database settings and data volumes.
- Provide English-only setup and release documentation, presentation screenshots and tested local/external/TLS startup examples.
- Compact channel rows and use the official Mattermost logomark with its source notice.
- Configure Telegram application credentials once per server and resolve bot destinations without manually entered access hashes.
- Complete Relayix naming across Compose, database defaults, browser storage and session cookies while preserving applied migrations.

## 0.6.1 Beta — 2026-11-04

- Add real ee FakeTLS for MTProto with validated SNI, authenticated ClientHello/server HMAC, TLS record framing and explicit rejection without downgrade. WEB remains plain/dd.
- Remove deployment .env and .env.example from the checkout. Load an external owner-only JSON through a Compose wrapper; disable automatic dotenv discovery.
- Render deployment candidates from GitHub production Secrets and Variables, reject database-password mismatches, and retain backup/rollback behavior.
- Generate shared test admin/database/provider/proxy credentials for every run in a temporary external directory, mask them in GitHub logs, and remove them on cleanup. Remove fixed credentials from Python/Rust tests.

## 0.6.0 Beta — 2026-11-04

- Add per-channel authenticated HTTP proxies for HTTP integrations and SMTP CONNECT; preserve TLS verification and refuse silent direct fallback.
- Add Telegram MTProto (plain/dd) and WEB-proxy transport with in-memory bot sessions, per-attempt deadlines, API credentials, topic mapping and safe errors. Explicitly reject unsupported FakeTLS ee secrets and WEB base paths.
- Include destination service identity/icons in dashboard and delivery history.
- Move existing private backups outside the source checkout and keep schema migrations tracked.
- Add Apache-2.0 license metadata, NOTICE, collected dependency licenses and publication documentation.
- Add exact local-credential checks, Gitleaks source/history gates in CI and preserve a narrow public-placeholder exception.

## 0.5.1 Beta — 2026-11-04

- Add dismiss buttons to all success/error banners and a per-account auto-close timeout in My profile (8 seconds by default, 0 for manual dismissal); pause timers on hover/focus.
- Add a separate SMTP sender name with Unicode support, legacy From mailbox compatibility and control-character validation.
- Explain route priority levels, threshold behavior and the default message priority in route forms.
- Remove the sidebar description and login description/badges; keep sign-in compact.
- Rename the project directory to relayix while preserving the existing Compose project and PostgreSQL volume.
- Prepare GitHub CI with locked dependencies, Rust/API/SMTP/Chromium checks, test artifacts, and gated GHCR release delivery. Add contribution/security documentation and Dependabot.

## 0.5.0 Beta — 2026-11-04

- Rename the application to Relayix with a violet routing mark, new login branding, calmer light/dark surfaces and responsive route cards.
- Add seven account-persisted dashboard periods from five minutes to thirty days; apply the period to metrics, recent deliveries and linked history filters.
- Consolidate endpoints, key rotation, recipient details and a copyable tagged request example in Routes; remove the API Tokens page.
- Label route tags simply as Tag and show recipients in a dedicated block with service icons and delivery methods.
- Support Mattermost bot / personal access tokens for channel posts and direct messages, preserving webhook compatibility and masked credentials. Use the official Mattermost logomark.
- Preserve existing PostgreSQL volume, users, sessions, routes, webhook addresses, credentials and keys on upgrade.

## 0.4.1 Beta — 2026-11-04

- Store theme, refresh interval, delivery/tag filters, route tag filter, search and page size in account preferences; keep account and API-token preferences isolated.
- Add configurable 5–3600 second monitoring refresh and an off setting. Show save failures and support retrying them.
- Generate route webhook addresses automatically and preserve existing addresses when renaming routes.
- Show full selectable endpoints and display key/endpoint clipboard feedback inside the key dialog.
- Display actual delivery tags and route required-tag badges; add exact tag filters and suggestions.
- Add Beta branding on the login screen, application header and browser title.
- Remove the two dashboard callout cards.
- Remove image/source bundles, generated reports and screenshots from the source tree; keep build/test outputs outside the repository and retain Docker images in Docker.

## 0.4.0 — 2026-11-04

- Ship a versioned Docker image and image-only Compose deployment; provide native Docker image export for transfer.
- Add username/password authentication, Argon2id hashes, expiring HttpOnly sessions, login throttling, user management and administrator/member roles.
- Replace the avatar placeholder with an account menu, personal profile, password changes and session revocation.
- Make Refresh show progress, update time, success/errors and fetch uncached data. Update independent resources even if one request fails.
- Add dashboard failed/retrying counters and links to filtered delivery history.
- Add configurable failure rules with watched routes/channels, recipients, thresholds, minimum attempts, time windows, cooldown and editable warning text.
- Deliver failure warnings through a separate persistent queue with history/retries and protection against recursive warnings.
- Support optional Telegram topic IDs.
- Filter and paginate deliveries on the server by status, channel, route, dates and text; show attempts and expandable errors.
- Disable Templates UI, API and message expansion while retaining historical database rows.
- Preserve existing channel credentials, routes, webhook keys and PostgreSQL volume with an additive migration.

## 0.3.0 — 2026-11-04

- Add local SVG service icons to channel cards, integration selection and route recipients.
- Group all 13 integrations into Chat & Messaging, Push Notifications, Email and Other.
- Add Matrix delivery with bot access tokens, safe room path encoding and stable transaction IDs on retries. Refuse encrypted rooms.
- Add Email (SMTP) with STARTTLS, implicit TLS, optional authentication, sender and recipient configuration.
- Mask new credentials and Pushover user keys; preserve unchanged secrets when editing channels.
- Show server validation errors inside channel forms and expose channel deletion on cards.
- Include application version in `/healthz`, update upgrade instructions, and add Rust, HTTP/SMTP queue and browser checks.
- Keep existing database migrations, route URLs, keys and channel settings compatible with v0.1/v0.2.
