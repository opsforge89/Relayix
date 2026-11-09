# User and API reference

[Quick start](../README.md) · [Database setup](DEPLOYMENT.md)

## Incoming events and delivery operations

**Incoming webhooks** shows accepted events even when `queued` is zero. Filter by route, routing result or title/message text and inspect the original routing report. Older events remain visible with a legacy diagnostic label. Invalid requests and failed authentication are not stored as events.

In **Routes → Test webhook**, **Preview routing** shows which recipients would receive a payload without storing an event or sending messages. **Send test webhook** creates real deliveries. Optional priority defaults to 3; tags remain UI metadata only.

Click **Details** in its own column in **Deliveries** to inspect errors and start/finish times. **Retry now** starts another cycle of up to five attempts while preserving total attempts and history. A recovered worker lease is marked interrupted because the previous send outcome may be unknown. Older attempts made before this release have counts but no detailed records.

Select **Fallback recipients** when editing a route. After any primary delivery exhausts five attempts, Relayix queues the original message to each enabled fallback recipient. Fallback targets are captured when the event is accepted; each recipient is queued once per event, even if several primary recipients fail or a primary is manually retried. Fallback deliveries have their own five attempts and cannot trigger another fallback. Primary and fallback recipients must be different. Disabling a channel prevents pending sends and fallback dispatch to it. A delivery already in progress can still complete.

For safe webhook retries, send `Idempotency-Key: YOUR_UNIQUE_EVENT_KEY`. The key is scoped to the route and retained with the event. Repeating a key with the same JSON payload returns the original event ID, original queue count and `duplicate: true`, without new deliveries. Reusing a key with a different payload returns `409 Conflict`. Concurrent retries are serialized. JSON object key order does not matter; changes to values or optional fields count as a different payload. Keys contain 1–256 characters and are stored as hashes. Deleting the event or its route removes its deduplication record. Delivery retries remain at least once; an external receiver may receive a duplicate after a crash or an ambiguous network failure.

Click **Diagnose** on any channel to check only that integration, without opening a separate dialog or sending a notification. The channel form also checks draft settings. Results appear inline: green confirms a stage, red reports failure, and gray means a check does not apply or needs an actual test send. Checks respect the configured proxy without direct fallback. Telegram checks bot/chat access; Mattermost and Matrix use read-only account/destination APIs; Discord reads webhook metadata; Pushover validates application/user keys; ntfy and Gotify check service health; SMTP connects/authenticates and issues NOOP. Webhook-only services check connectivity/TLS and leave credential/posting checks explicitly unverified. Use **Test** for actual delivery and topic/write permissions. MTProto/WEB require Telegram application credentials on the server; HTTP Bot API does not.

Startup errors name invalid settings and distinguish database authentication, missing databases, TLS and connection problems without printing credentials. Inspect them with `docker compose logs app`.

## Grouped delivery history

Dashboard and Deliveries show one row per event. Each recipient chip displays the configured channel name, service icon/name and a status: sent, failed, retrying, queued or sending. Click a chip to inspect that delivery; Details summarizes all recipients and their attempt counts. Fallback recipients are labeled separately.

Channel and status filters must match the same delivery. For example, Telegram + Failed selects events where that Telegram channel failed. All recipients remain visible; matching recipients are highlighted and others are muted. Search also matches errors and channel names on that same delivery. Date/period filters use event creation time. Counts and pagination refer to complete events, sorted before pagination; dashboard metrics still count individual deliveries.

`GET /api/deliveries?group_events=true` returns event rows with a `recipients` array and `matches_filter` flags. Without this parameter the existing per-delivery API remains available. The grouped sort columns are `created_at`, `title`, `tags`, `route` and `recipients` (case-insensitive channel-name order). Original message bodies are stored and copied without alteration; the viewer renders the shared bold/link syntax as safe React elements, with raw HTML kept literal.

## Table controls

Every table has a **Columns** menu to show or hide individual columns. Click a text, numeric or time header to sort; click it again to reverse the order. Time-based tables start with newest records first. Sorting of incoming events, deliveries and alert history runs on the server before pagination or result limits. User rows are sorted across the complete loaded user list. Action-only columns do not sort.

Column visibility and sorting are saved per table to your account and follow you across sessions/devices. **Reset table** restores all columns and newest-first ordering. At least one column stays visible. Attempt counts, errors and retry actions are available in **Details**; there is no Attempts column.

To configure fallback delivery, open **Routes → Edit → Fallback delivery → Fallback recipients (optional)**. Select recipients and save. They receive the original message after a primary recipient exhausts five attempts. A route card shows configured fallback recipients separately.

## Screenshots

The screenshots use demonstration data and show the current interface.

### Dashboard · dark theme

![Relayix dashboard with delivery metrics, channels and recent notifications](screenshots/relayix-dashboard-dark.png)

### Channels and integrations · light theme

![Relayix compact channel list and available integrations](screenshots/relayix-channels-light.png)

## Runtime and dependencies

Application, build and test images use Debian 13 Trixie. The verified toolchain is Rust 1.99.1, Node.js 26.13.0 LTS and Python 3.15.2; PostgreSQL uses the maintained 17.11 release on Trixie. Debian packages are upgraded from stable repositories during application/test image builds. Cargo, npm and Python dependencies are locked, and Python packages retain mandatory artifact hashes.

The November 2026 refresh includes React 19.3.1, Vite 8.3.6, Lucide 1.69.0, SQLx 0.9.0, Reqwest 0.13.5, Argon2 0.6.0 and rand 0.10.3. Keep the existing database image and data volume when updating the app. `RELAYIX_DB_VOLUME` selects an existing named data volume; keep that setting in the private deployment JSON for installations managed by the deployment scripts.

## Accounts and profiles

- Username/password login with Argon2id password hashes and salted passwords.
- Passwords must contain 12–128 characters, uppercase/lowercase letters, a number and a symbol. Usernames and common sequences (`password`, `qwerty`, `123456`, `letmein`) are rejected in passwords. The UI shows requirements; the API enforces them too.
- Administrators create/edit/delete users, set roles, disable accounts and reset passwords. The last active administrator and your own administrator account are protected from removal/demotion.
- Members can manage shared channels, routes and deliveries. This is one shared notification hub, not isolated workspaces per user.
- The account button opens a menu with the real user's name, profile, settings and sign out. Profile supports display name and password changes.
- Password changes revoke other sessions. Administrative resets and disabling a user revoke all their sessions.
- Sessions expire after seven days. Session tokens are hashed in PostgreSQL. Browser cookies are HttpOnly and SameSite=Strict. Cookie-authenticated API requests require `X-Relay-Client: web`; no cross-origin API access is enabled.
- `COOKIE_SECURE=true` enables Secure cookies behind HTTPS. Leave false for direct HTTP access on the configured LAN IP.
- The optional administrator API token remains available for automation and token login. It is not an individual user's profile.
- Login attempts are limited to ten failures per username in fifteen minutes.

## Personal settings

In **My profile → Interface notifications**, set auto-close to 1–300 seconds (default 8), or 0 for manual dismissal. Success and error banners always have a close button, including inside dialogs. Timers pause on hover or keyboard focus. This setting belongs to the account and follows it across devices.

Settings belong to the authenticated account and are stored in PostgreSQL. Theme, automatic refresh interval, saved delivery filters (including tags), route tag filter, list search and delivery page size follow the user between browsers and devices. Another account has its own preferences. The administrator API-token identity has separate preferences.

Use **Settings → Notification monitoring** to set a custom refresh interval from 5 to 3600 seconds; `0` disables automatic refresh. Manual Refresh remains available. Choose 10, 25, 50 or 100 deliveries per page. Theme and applied filters save automatically; monitoring settings use **Save settings**. Saving errors are shown and can be retried. Pending changes are flushed before sign out.

## Dashboard and Refresh

Choose 5, 15 or 30 minutes, 1 hour, 1 day, 7 days or 30 days. The selected window applies to every delivery metric and recent delivery history, and follows failed/retrying links into Deliveries. The period is saved to your account. Routes and Channels show current configuration counts.

The dashboard displays final failed deliveries, deliveries being retried, pending deliveries, total deliveries and success rate for the selected period. Clicking failed/retrying counts opens the corresponding delivery filter.

Refresh makes uncached requests, shows progress, updates the last refresh time and reports success or errors. Successful resources can update even if another endpoint fails. Background refresh follows the saved account interval (15 seconds by default) while the page is visible.

## Channels and routing

All channels support an authenticated **HTTP proxy** with an IP/DNS host, port, login and password. HTTPS and SMTP use CONNECT; proxy failure never silently falls back to direct delivery. Telegram also offers **MTProto** and native **WEB-proxy** modes using application credentials configured once on the server (`TELEGRAM_API_ID` / `TELEGRAM_API_HASH` in `.env`). The proxy provider supplies only host, port and secret; MTProto authorization separately requires these application credentials. Bot chat access hashes are handled automatically. MTProto supports plain/dd and ee FakeTLS secrets with an SNI domain; WEB supports plain/dd. WEB base paths are explicitly rejected. See [proxy configuration and verification limits](PROXIES.md).

Dashboard and delivery history show the destination service icon beside the channel name.

SMTP offers separate **Sender name** and **From address** fields. For example, `Relayix Alerts` and `alerts@example.org` produce `Relayix Alerts <alerts@example.org>`. Existing `Name <address>` configurations continue to work. A separate nonempty sender name takes precedence over the name embedded in From address. The SMTP account must be allowed to send from the configured address.

Route **Minimum priority** filters the notification's JSON `priority`, from 1 (lowest) to 5 (highest). A minimum of 4 accepts priorities 4 and 5. Omitted notification priority defaults to 3. This threshold filters delivery; it does not change queue ordering.

Routes is the single place to manage permanent webhook endpoints, recipients, tags and route keys. Each card displays its endpoint and recipient names, integration icons and delivery methods. Use **Copy**, **Integration guide**, **Edit** or **Rotate key** on the card. Keys are displayed once after creation or rotation and stored as hashes; replace `YOUR_ROUTE_KEY` in the integration example. Rotation invalidates the previous key. The former API Tokens page has been removed.

Mattermost supports both existing incoming webhooks and **Bot / REST API**. Enter the server URL and bot or personal access token, then choose a channel ID or a direct-message recipient (`@username` or user ID). Add the sender to the destination channel. Delivery uses the [official Mattermost v4 API](https://docs.mattermost.com/api/reference/create-post). Credentials remain masked when editing a channel.

13 integrations with local SVG icons: Telegram, Matrix, Mattermost, Rocket.Chat, Slack, Discord, Microsoft Teams (compatible webhook), Google Chat, ntfy, Pushover, Gotify, Email (SMTP) and Webhook.

Configure a channel, use Test, and assign it to a route. Enter a route name and select recipients; a unique permanent webhook address is generated automatically. Existing `/h/{slug}` addresses and hashed `X-Relay-Key` values are preserved. Renaming a route never changes its address. Routes shows the full, selectable endpoint without ellipsis; key-copy feedback is displayed inside the key dialog. Changing recipients does not require changing the sending integration.

### Integration capabilities and message limits

All incoming notifications share the same input limits: **256 UTF-8 bytes for the title** and **10,000 UTF-8 bytes for the message**. Larger inputs are rejected before delivery. These limits also apply to integrations without an additional delivery cap below. UTF-8 bytes differ from characters: Cyrillic letters usually use two bytes, and many emoji use four.

The table describes the adapters and delivery limits enforced by Relayix. Destination servers can impose additional size, permission and rate limits, including instance-specific settings.

| Integration | Delivery and required settings | Relayix delivery limit | Capabilities and restrictions |
| --- | --- | --- | --- |
| Telegram | Bot token and chat ID; optional topic ID | 4,096 UTF-16 units for the combined rendered title and message; shortened when necessary | Bold and HTTP/HTTPS links; Bot API, MTProto or WEB-proxy transport. Emoji can use two units; rendered markup also consumes the allowance. |
| Matrix | Homeserver URL, access token and room ID | Accepted content is sent without additional truncation; server limits apply | Plain-text room messages. Invite the bot to an unencrypted room; room aliases and encrypted rooms are unsupported. Retries reuse the transaction ID. |
| Mattermost | Incoming webhook URL, or server URL and bot/personal token with a channel ID or direct-message recipient | Accepted content is sent without additional truncation; server limits apply | Bold and HTTP/HTTPS links. Bot API supports channels and direct messages; the sender needs access to the destination. |
| Rocket.Chat | Incoming webhook URL | 5,000 UTF-16 units for the combined title and message; shortened when necessary | Sends a text webhook payload. Emoji can use two units. Instance-specific limits can be lower. |
| Slack | Incoming webhook URL | Accepted content is sent without additional truncation; service limits apply | Sends the combined title and message in the webhook's text field. |
| Discord | Incoming webhook URL | 2,000 UTF-16 units for the combined title and message; shortened when necessary | Sends webhook message content. Emoji can use two units. |
| Microsoft Teams | Compatible webhook URL | Accepted content is sent without additional truncation; endpoint limits apply | Requires an endpoint accepting a JSON text payload; compatibility depends on the configured webhook or workflow. |
| Google Chat | Space webhook URL | 32,000 UTF-8 bytes for the serialized JSON payload; shortened when necessary | Title and message share the budget with JSON syntax and escaping, so quotes, backslashes and control characters can use extra space. |
| ntfy | Server URL and topic | Title: 1,024 UTF-8 bytes; message: 4,096 UTF-8 bytes; shortened separately | Title is sent as an HTTP header; body and title have separate budgets. Topic names allow ASCII letters, digits, hyphens and underscores. |
| Pushover | Application API token and user/group key | Title: 250 Unicode characters; message: 1,024 Unicode characters; shortened separately | Title and message have separate budgets. Characters are counted as Unicode code points, rather than UTF-8 bytes. |
| Gotify | Server URL and application token | Accepted content is sent without additional truncation; server limits apply | Sends separate title, message and priority fields. |
| Email (SMTP) | SMTP host, port, TLS mode, sender and recipient; paired username/password when authentication is required | Accepted content is sent without additional truncation; mail server limits apply | Plain-text email; one recipient mailbox per channel. STARTTLS or TLS; unencrypted SMTP requires explicit server configuration. |
| Webhook | Destination URL; optional administrator-managed JSON body template | Accepted content is sent without additional truncation; endpoint limits apply | Default JSON payload or a custom JSON object template. Templates have a separate 16 KiB configuration limit and are managed only by administrators. |

For combined text limits, the title, separating line breaks and message share one allowance. Relayix shortens oversized deliveries with a visible `[Message truncated]` marker; separately limited titles use `[truncated]`. Markers count toward the limits. Complete incoming content remains in history, and truncation preserves valid Unicode and Telegram markup. Changing a destination's own limit does not change Relayix's configured delivery cap.

### Automatic message formatting

The channel creation and editing form shows the incoming title/message size limits and the selected integration's delivery limits, including how oversized deliveries are shortened. Incoming notifications accept at most 256 UTF-8 bytes for the title and 10,000 UTF-8 bytes for the message; larger inputs are rejected.

Send a message with or without a title and tags. Telegram and Mattermost automatically render paired `**bold**` spans and `[label](https://url)` links in the common message. Titles are included only when supplied and nonblank; no fallback heading is added. Title-only events are delivered once. There are no content or formatting settings to configure, and legacy `message_mode` / `text_format` values are ignored.

Relayix escapes user text and uses Telegram HTML for Bot API, Unicode-correct entities for MTProto/WEB, and Mattermost Markdown for webhook, channel and direct-message delivery. HTTP/HTTPS links are supported; unsafe schemes and credential-bearing links stay literal. Other text is literal. This common format supports bold and links rather than a full Markdown syntax. Message spacing and line breaks are preserved.

Tags remain UI metadata and do not filter or decorate outgoing messages. Omitted priority defaults to 3. Untitled rows show **View message** to open and copy the complete original body. Original event text remains stored in history. Rocket.Chat deliveries are capped at 5,000 UTF-16 units including the title and a visible truncation marker when needed.

```json
{"title":"Deployment complete","message":"**Tests passed**\n[TASK-123](https://tickets.example.org/TASK-123)","tags":["deploy"]}
```

The `title` and `tags` fields can both be omitted.

### Telegram topics

Telegram channels support bot token, chat ID and optional **Topic ID**. The topic ID is sent as the numeric [`message_thread_id`](https://core.telegram.org/bots/api#sendmessage). Leave it empty for the chat's default destination. Example config:

```json
{"bot_token":"123:bot-token","chat_id":"-1001234567890","message_thread_id":"42"}
```

### Matrix and SMTP

Matrix requires a homeserver URL, bot access token and a room ID such as `!room:example.org`; invite the bot to the room. Room aliases and encrypted rooms are unsupported. Matrix deliveries retain their transaction ID across retries.

SMTP supports STARTTLS (usually port 587), implicit TLS (usually 465), optional paired username/password, sender and one recipient mailbox per channel. Add multiple channels for multiple mailboxes. Unencrypted SMTP requires explicitly setting `ALLOW_INSECURE_HTTP=true`, which also permits LAN HTTP webhook destinations.

Secret fields are masked in channel responses, and unchanged masked values are preserved when editing. Changing the channel type, URL, SMTP host/port/TLS mode or any `proxy_*` setting or WEB-proxy address requires re-entering all masked secrets; updates and draft diagnostics reject masked values in that case. Channel credentials remain stored in PostgreSQL; restrict volume and backup access.

### Tags

A route's **Tag (UI only)** groups routes and their delivery history in the interface. It never limits webhook delivery. Message `tags` are also optional UI metadata. Delivery tag filters match either the route's label or the message's tags; matching is exact. Route labels and message tags are displayed together, while the original message tags remain unchanged.

The legacy route API/database field `required_tag` is retained for compatibility and now stores this UI label only. Sending decisions use route enabled state, minimum priority and enabled recipient bindings.

### Send an event

```bash
curl --fail-with-body -X POST http://192.168.39.110:8080/h/gitlab-deploy \
  -H "X-Relay-Key: $RELAYIX_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"title":"Deployment successful","message":"auth deployed to production","priority":3,"tags":["deploy"]}'
```

This minimal body uses the default priority of 3 and omits message tags:

```json
{"title":"GitLab CI Tests","message":"Tests completed"}
```

Both `priority` and `tags` are optional. An omitted priority defaults to **3**; omitted or null tags become an empty array. Route tags and message tags do not affect delivery; only the configured minimum priority and enabled route/recipients apply.

Every accepted webhook returns `event_id`, `queued` and `routing`. Routing diagnostics identify the route, configured filters, linked/active recipients and why each recipient was queued or excluded. A zero queue count reports `priority_below_minimum`, `no_destinations` or `no_active_destinations`. An authenticated request to a disabled route returns HTTP 403 with `route_disabled`. Service logs correlate decisions by event/route/channel ID without logging credentials or message bodies.

Use **Routes → Test webhook** to copy an example JSON or send a real event through the same routing/queue code. Priority and comma-separated tags may be left empty. The result explains priority/recipient decisions and links to Deliveries for the actual send outcome. Copying JSON does not send an event.

A successful response confirms queuing, not delivery. Check Deliveries for the result. The PostgreSQL worker uses row locking and stale-worker recovery; delivery is at least once. Temporary errors receive up to five attempts per retry cycle with exponential backoff. Permanent errors finish immediately and can activate configured fallback recipients.

### Message previews

In a channel's creation or editing form, open **Preview a message**, enter a title/body, and choose **Preview message**. In a route's **Test webhook** dialog, choose **Preview routing and messages** to apply the route template and inspect every primary and fallback recipient. Each preview shows the actual rendered output, field sizes, delivery budgets and visible truncation markers. Telegram bold/links and Mattermost formatting are displayed safely; **Exact outgoing text** shows the adapter text. Custom webhook JSON previews are available only to administrators because template values can contain credentials.

Previewing stores no event, consumes no route quota and contacts no destination service. Preview results disappear when their input changes. A preview checks presentation; **Diagnose** checks connectivity and **Test** makes a real send.

### Retries and channel recovery

HTTP 401/403 failures are reported as authentication/permission errors; other permanent request failures (such as 400, 404 or 413) also stop the delivery immediately. HTTP 408, 409, 425, 429 and 5xx responses, timeouts and connection failures use automatic retries. SMTP permanent responses and Telegram credential/destination errors stop immediately. Temporary SMTP and Telegram transport errors retry. Fallback recipients are queued after a permanent primary failure or after five unsuccessful transient attempts.

Relayix waits for the greater of exponential backoff and the provider's retry delay. It accepts HTTP `Retry-After` seconds or dates, Telegram Bot API `parameters.retry_after`, Telegram MTProto/WEB flood-wait seconds, and Matrix `retry_after_ms`. Provider-supplied delays are capped at 24 hours for every transport, including date values and JSON retry fields. Missing or invalid delay values use normal backoff. **Delivery attempts** shows the failure category and earliest next attempt time.

After three consecutive channel failures with the same error, automatic delivery to that channel pauses for at least 60 seconds. Channel failures include HTTP 401/403, HTTP 408, HTTP 429, 5xx responses, network failures and invalid channel configuration. Other request rejections, including HTTP 400/413 and permanent SMTP recipient failures, remain delivery errors and do not pause the channel. A message rejection clears the failure streak and releases an expired recovery probe, while preserving any active cooldown. A provider rate limit or explicit retry delay on a channel failure starts a cooldown immediately; the provider delay is capped at 24 hours. Both ordinary and alert queues share this state in PostgreSQL across workers and replicas. Queued messages stay queued without consuming attempts while paused; other channels continue delivering.

After cooldown expiry, only one queued attempt may probe the channel across both queues. A successful probe clears the failure state and releases the backlog; another failure renews the cooldown. An already-running successful send cannot clear an active cooldown. **Channels** shows the pause deadline and reason, or **Probe pending** after expiry. Recovery needs queued work; Relayix does not send synthetic probe messages. A successful channel save clears its failure state and cooldown and makes queued retries eligible immediately in both queues, preserving attempts and history. Failed saves and draft diagnostics leave the cooldown intact. Manual queue retries preserve channel cooldowns. Explicit direct **Test** sends and diagnostics remain available to check a repaired configuration.

## Failure alerts

Administrators configure **Failure alerts**:

- Choose watched channels and routes; empty selection means all. Both filters must match.
- Select the number of distinct failing deliveries, minimum attempts per delivery, rolling failure window and cooldown.
- Choose one or more destination channels and customize the warning title/message.
- Text fields: `{failures}`, `{channel}`, `{route}`, `{error}`, `{title}`. For multiple failures, channel/route/error/title describe the latest matching failure.
- Defaults warn after the first unsuccessful attempt, before all normal retries are exhausted.
- Each failing delivery triggers a rule once. Cooldown delays warnings about additional failures and prevents repeated bursts.
- Warnings have a persistent separate outbox with retries and delivery history. Their own failures never generate further failure alerts. Failed warnings can be retried manually.
- Test rule queues an explicit test warning for enabled recipients; inspect alert history for its outcome.

Use an independent working destination for warnings about another destination's failures.

## Deliveries

Filters run on the server before pagination: status, channel, route, tag, date range and title/channel/route/error text. One row represents an event, with named service recipients and individual status icons. Attempts, errors and manual retry are available by clicking a recipient or Details. Incoming-event Details opens a side panel with an Overview and a separate Technical tab, preserving table row height. Pages use the account page-size setting (50 by default), with a filtered total. API `page_size` accepts 1–100.

## Message templates

Templates are optional. Create your own under **Templates**, then select one in a route's **Message template** field. No templates are installed automatically. **Examples** open editable drafts with fictional data; only **Save template** stores them. **Preview** renders sample JSON without storing an event or sending a notification.

Use `{{field}}` or `{{build.branch}}` for JSON values. Strings, numbers, booleans and scalar arrays are supported; zero counts are preserved. Variables are literal text, except `{{message}}`, which preserves incoming bold/link formatting. Shared `**bold**` and `[label](https://example.org)` links are adapted for Telegram and Mattermost. Templates run no expressions, scripts or network requests. `title` and `tags` may be absent; priority defaults to 3. An empty title template produces no heading.

A layout can use `{{test_type}} {{suite}}` as its title and this body:

```text
Branch: {{branch}}

Status: {{status}}
Failed: {{failed}}
Passed: {{passed}}
Skipped: {{skipped}}
```

All values come from your webhook JSON. Test types and suite names are unrestricted: for example, `UnitTests` and `ExampleSuite`, or `IntTests` and `Example.Worker`.

For a runner-log link, set **Link label** to `Runner log`, **Link URL template** to `{{log_url}}` and enable **Show on failure only**. Pass `log_url` from GitLab's `$CI_JOB_URL` **inside the test job**. If notification runs in a separate job, carry the original test job URL through an artifact/dotenv variable; `$CI_JOB_URL` in that separate job points to its own log.

```bash
# Inside the test job, after collecting its actual test counters/status:
# Use your test runner's real values for TEST_TYPE, TEST_SUITE, TEST_STATUS,
# TEST_FAILED, TEST_PASSED and TEST_SKIPPED.
python3 - <<'PYCODE'
import json, os
payload = {
    'test_type': os.environ['TEST_TYPE'],
    'suite': os.environ['TEST_SUITE'],
    'branch': os.environ['CI_COMMIT_REF_NAME'],
    'status': os.environ['TEST_STATUS'],
    'failed': int(os.environ['TEST_FAILED']),
    'passed': int(os.environ['TEST_PASSED']),
    'skipped': int(os.environ['TEST_SKIPPED']),
    'log_url': os.environ['CI_JOB_URL'],
}
with open('notification.json', 'w') as output:
    json.dump(payload, output)
PYCODE
curl --fail-with-body "$RELAYIX_ENDPOINT" \
  -H 'Content-Type: application/json' -H "X-Relay-Key: $RELAYIX_ROUTE_KEY" \
  --data-binary @notification.json
```

Failure means `failed > 0`, `success: false`, or a failure/canceled status in `status` or `job_status` (including `❌`). Missing or unsafe URLs are hidden. Optional links accept only HTTP/HTTPS URLs without embedded credentials.

If required variables are missing, the original message/title is used. If neither the template nor the original payload can supply useful content, the webhook returns `400` with an explanation. Rendering and the template version are saved per event. Editing or deleting a template affects new events; retries and fallback retain the accepted event's text. Original payload and template snapshot are available in **Incoming webhooks → Details → Technical**. Historical disabled template records remain separate and are never activated automatically.

## API

Use an authenticated browser session plus `X-Relay-Client: web`, or `Authorization: Bearer ADMIN_TOKEN`. `/h/{slug}` continues to use only its own `X-Relay-Key`.

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/healthz` | Health and release version |
| POST | `/api/auth/login` | Username/password login; returns HttpOnly session cookie |
| POST | `/api/auth/logout` | Revoke current session |
| GET | `/api/auth/me` | Current account and role |
| PUT | `/api/auth/profile` | Display name/password update |
| PUT | `/api/auth/preferences` | Validate and save account preferences |
| GET | `/api/tags` | Distinct suggested route/event tags |
| GET / POST | `/api/users` | List/create users; administrator only |
| PUT / DELETE | `/api/users/{id}` | Edit/delete user; administrator only |
| GET / POST | `/api/channels` | List/create channels |
| PUT / DELETE | `/api/channels/{id}` | Edit/delete channel |
| POST | `/api/channels/{id}/test` | Direct test notification |
| POST | `/api/channels/{id}/diagnose` | Read-only diagnostics for saved settings |
| POST | `/api/channels/diagnose` | Read-only draft diagnostics; optional id restores masked secrets for unchanged destinations |
| POST | `/api/channels/preview` | Render a draft message without storing or sending; accepts kind, config, title, message and optional priority |
| GET / POST | `/api/routes` | List/create routes |
| PUT / DELETE | `/api/routes/{id}` | Edit/delete route |
| POST | `/api/routes/{id}/rotate` | Rotate route key |
| POST | `/api/routes/{id}/test` | Send a real test event through route filters and return routing diagnostics; authenticated users only |
| POST | `/api/routes/{id}/preview` | Preview routing, route template and per-channel presentations without storing or sending |
| GET | `/api/events` | Incoming event history and original routing diagnostics |
| GET | `/api/deliveries/{id}/attempts` | Latest 100 attempts, failure category, next attempt time and channel health |
| GET | `/api/stats` | Delivery and failure counters |
| GET | `/api/deliveries` | `{items,total,page,page_size}` with filters |
| POST | `/api/deliveries/{id}/retry` | Retry failed/retrying delivery |
| GET / POST | `/api/alert-policies` | List/create failure rules; administrator only |
| PUT / DELETE | `/api/alert-policies/{id}` | Edit/delete failure rule |
| POST | `/api/alert-policies/{id}/test` | Queue test warning |
| GET | `/api/alert-deliveries` | Warning history |
| POST | `/api/alert-deliveries/{id}/retry` | Retry failed warning |

Delivery query fields: `status` (`sent`, `failed`, `queued`, `processing`, `retrying`), `channel_id`, `route_id`, `event_id`, `tag`, `from`, `to` (RFC 3339 timestamps), `search`, `period` (`5m`, `15m`, `30m`, `1h`, `24h`, `7d`, `30d`), `page`, `page_size`.

## Development and verification

```bash
scripts/check.sh
```

The verification script requires Docker Compose, Python 3 and OpenSSL on the host. Docker builds use the committed Cargo.lock and npm package-lock.json. The check script builds Rust/React, runs Rust unit tests, creates a temporary PostgreSQL database and local HTTP/SMTP providers, runs API/queue/SMTP checks and then Chromium desktop/mobile tests. It always removes its isolated containers. Screenshots and failure logs are written to `/tmp/relayix-test-artifacts`, outside the source tree. Tests do not send to real provider accounts.

For development without Docker, install Rust 1.99.1+ and Node 26.13+, run `npm ci` and `npm run build` in `web/`, then `cargo test --locked` from the root.

GitHub CI, GHCR delivery and first repository setup are described in [docs/GITHUB.md](GITHUB.md). See [CONTRIBUTING.md](../CONTRIBUTING.md) for contribution guidance and [SECURITY.md](../SECURITY.md) for private vulnerability reporting.

## License and publication

Relayix original source is licensed under [Apache-2.0](../LICENSE). Preserve [NOTICE](../NOTICE) and [third-party notices](../THIRD_PARTY_NOTICES.md); dependencies keep their own licenses, collected under LICENSES/. Service trademarks remain with their owners.

Schema migrations belong in source control. Private deployment configuration, database dumps, Telegram sessions and backups do not. Local backups have been moved outside the checkout. See [publication checks](PUBLICATION.md) and [GitHub setup](GITHUB.md). Nothing is automatically pushed by those local checks.

Incoming history query fields: `route_id`, `reason`, `search`, `page`, `page_size` (1–100). Route create/update accepts `fallback_channel_ids` separately from primary `channel_ids`.

## Bulk retry of final failures

On **Deliveries**, apply the desired filters, choose **Retry failed deliveries**, review the recipient list, and confirm. The preview includes up to 500 final-failure recipients, not 500 events. Sent, queued and processing deliveries are excluded. With a status filter, choose **Failed** or **All statuses** first.

`POST /api/deliveries/retry-batch/preview` accepts `{"filters":{"route_id":"UUID","group_events":true}}`. The response includes `batch_id`, `items`, `count`, `total`, and `expires_at`. Confirm with `POST /api/deliveries/retry-batch/{batch_id}`. Previews are account-owned and expire after 15 minutes. Confirmation is atomic and idempotent; it skips recipients whose state has changed. Attempt records, event payloads and template snapshots are preserved. Retry starts a new attempt cycle and may produce a new personal warning if that cycle fails.

## Template versions and cloning

**Templates → History** compares a saved version with the current layout. Restoring creates a new version; it never rewrites previous versions or accepted event snapshots. Existing templates receive a history entry for their current version when upgrading to 0.8.2. Earlier edits made before version history was available cannot be reconstructed.

`GET /api/templates/{id}/versions` returns up to 100 versions, newest first. Use `?before=VERSION` with the response's `next_before` to load older versions. `POST /api/templates/{id}/versions/{version}/restore` restores that content as a new current version.

**Clone** opens an editable copy of a route or template. Nothing is created until saving. Template copies start at version 1 with their own history. Route copies preserve recipients, fallback recipients, icon, template assignment and payload rules, but receive a fresh endpoint and secret key when saved.

## Incoming JSON validation

**Routes → Edit → Incoming JSON validation** defines up to 40 optional rules for the original payload. Types are `string`, `integer`, `number`, `boolean`, `array`, `object`, and `url`. Numeric rules accept optional `min` and `max`. Required fields must exist and cannot be null. Optional absent or null fields are skipped. Values are never coerced: the string `"2"` is not an integer. URL fields accept HTTP/HTTPS URLs without embedded credentials. Dotted paths address nested objects, up to eight levels.

The route's `payload_rules` field is an array such as:

```json
[
  {"field":"suite","kind":"string","required":true},
  {"field":"passed","kind":"integer","required":true,"min":0},
  {"field":"failed","kind":"integer","required":true,"min":0}
]
```

Validation runs before template rendering or event/queue insertion and also applies to route previews and test sends. Invalid input returns HTTP 400, for example `{"error":"Invalid payload","fields":{"suite":"Required field","failed":"Expected integer"}}`. No event is stored. An empty rule list preserves existing behavior. An accepted idempotent replay retains its original event even if validation settings later change.

## Computed template status

Enable **Compute status from test counts** in the template editor and use `{{status}}` in the title, message or optional URL. The template fields `computed_status`, `passed_field`, and `failed_field` configure the calculation. Counter paths default to `passed` and `failed`; nested paths such as `counts.passed` are supported.

Rules run in this order:

1. A valid `failed` count greater than zero produces ❌, even if `passed` is positive.
2. A valid `failed` count of zero and a valid positive `passed` count produce ✅.
3. Otherwise the result is ⚠️.

Counters must be non-negative JSON integers. Missing, negative or incorrectly typed values produce preview warnings and cannot imply success. Enabling calculation replaces the incoming `status` in the rendered payload only. The original payload remains unchanged. Failure-only links use the computed result, including when custom counter paths are used. With calculation disabled, incoming status and previous template behavior are retained. The Test results example enables calculation, so CI only needs to supply counters.

## Personal delivery alerts

Sign in with a personal account, then open **My profile → Delivery alerts**. Choose one or more enabled channels, optionally restrict routes, select **Final failure**, **First error**, or **Failure threshold**, set the time window and repeat interval, and save. **Send test alert** verifies the saved recipients through the separate alert queue. **My alert delivery history** shows the latest 100 deliveries belonging to that account; failed alerts can be retried while the subscription is enabled.

A final-failure trigger requires an actual `failed` state, not merely a high historical attempt count. A warning is recorded once per recipient delivery and retry cycle; cooldown limits warnings about new failures. Disabling a subscription or user prevents queued personal warnings from being sent. Deleted users' subscriptions and private warning records are removed. Shared administrator rules remain separate and do not expose personal subscriptions or personal history. Alert failures never generate additional alerts.

Personal endpoints are `/api/auth/delivery-alerts` (GET/PUT), `/api/auth/delivery-alerts/test` (POST), `/api/auth/delivery-alerts/history` (GET), and `/api/auth/delivery-alerts/history/{id}/retry` (POST). They always derive ownership from the authenticated user. The administrator API token has no personal account and cannot use these endpoints. Recovery notifications are a separate future enhancement.
