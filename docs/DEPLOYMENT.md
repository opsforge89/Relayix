# Run Relayix with Docker Compose

Requirements: Git and Docker Engine with Docker Compose v2.20 or newer. Compose runs the published image; Python, Rust and Node.js are included where needed in the image. Run all commands below from the repository root.

## Bundled PostgreSQL

Clone the repository and copy the settings template for a new installation:

```bash
git clone https://github.com/opsforge89/Relayix.git
cd Relayix
cp relayix.env.example .env
```

`compose.yaml` and `relayix.env.example` are already in the repository root. If you have an existing checkout, start there and keep an existing `.env` rather than overwriting it.

Open `.env` in your preferred editor. Set these two values:

| Setting | What to enter |
| --- | --- |
| `POSTGRES_PASSWORD` | A new password for the bundled database |
| `ADMIN_PASSWORD` | Your Relayix login password: 12–128 characters, uppercase/lowercase letters, a number and a symbol |

Keep passwords inside single quotes, for example `ADMIN_PASSWORD='your own password here'`. Single quotes preserve `$`, `#` and spaces literally. Avoid a single quote inside a single-quoted value. The application accepts database passwords as written, including URL-reserved characters.

Start the service:

```bash
chmod 600 .env
docker compose up -d
```

Compose pulls the ready-made images, starts PostgreSQL, waits for it to become ready and starts Relayix. Open **`http://127.0.0.1:8080`** in your browser and sign in as **`admin`** with your `ADMIN_PASSWORD`. Schema migrations and the initial administrator account are created automatically. The first startup may take a few minutes while images download.

The containers are named `relayix-app` and `relayix-db` and share `relayix-network`. PostgreSQL is available only inside that network. Its data persists in a named Docker volume.

## Existing external PostgreSQL (TLS optional)

Create the database and login role on your PostgreSQL server first. The role must be able to create and alter objects in that database.

Use the **same root `compose.yaml` and `.env` prepared above**. Set `DATABASE_MODE=external` in `.env`; no additional Compose files are needed.

In `.env`, set `ADMIN_PASSWORD` and your existing database connection settings:

| Setting | Example / purpose |
| --- | --- |
| `DATABASE_MODE` | `external` to disable the bundled PostgreSQL service |
| `POSTGRES_HOST` | `postgres.example.org`, an IP address, or the Docker host's LAN IP for PostgreSQL on the Docker host |
| `POSTGRES_PORT` | `5432` |
| `POSTGRES_USER` | Existing database login role |
| `POSTGRES_DB` | Existing database name |
| `POSTGRES_PASSWORD` | That role's existing password |
| `POSTGRES_SSLMODE` | `verify-full` for verified TLS; explicitly set `disable` for a LAN database configured without TLS |

Enter a hostname or IP in `POSTGRES_HOST`, without a URL scheme or port. `localhost` refers to the app container; use the Docker host's LAN IP for the Docker host. A host database must listen on an interface reachable from Docker and allow the Docker subnet in its firewall and `pg_hba.conf`.

```bash
chmod 600 .env
docker compose up -d
```

Open `http://127.0.0.1:8080` and sign in as `admin`. In external mode, only Relayix starts; the bundled PostgreSQL service is disabled.

### Without TLS

TLS is not required for an external PostgreSQL server. If your server does not use TLS, set the following in the same `.env` file, replacing the template's `verify-full` setting:

```dotenv
POSTGRES_SSLMODE=disable
POSTGRES_CA_PATH=
```

No certificate is needed. Do not leave `POSTGRES_SSLMODE` empty to disable TLS; select `disable` explicitly.

### Optional TLS and a private CA

Set these values in the same `.env` file:

```dotenv
POSTGRES_SSLMODE=verify-full
POSTGRES_CA_PATH=/srv/relayix/postgres-ca.crt
```

`POSTGRES_CA_PATH` is the path to the CA certificate **on the Docker host**; a relative path such as `./postgres-ca.crt` also works. The file must exist and be readable by container user 10001. Compose mounts it read-only; no overlay file or extra command-line flags are needed. The database hostname must match the server certificate.

Leave `POSTGRES_CA_PATH` empty to use the image's default trusted CA certificates. For a LAN database configured without TLS, explicitly set `POSTGRES_SSLMODE=disable` and leave the CA path empty.

Apply either configuration with:

```bash
docker compose up -d
```

## Optional settings

All settings go in `.env`; no JSON generator is needed.

| Setting | Default | Purpose |
| --- | --- | --- |
| `DATABASE_MODE` | `local` | `local` starts bundled PostgreSQL; `external` uses your existing server |
| `POSTGRES_CA_PATH` | Empty | Optional host path to a PostgreSQL CA certificate |
| `ADMIN_USERNAME` | `admin` | Initial administrator username |
| `BIND_ADDRESS` | `127.0.0.1` | Local host access; use the included Caddy example for HTTPS |
| `PORT` | `8080` | Web UI and webhook port on the Docker host |
| `COOKIE_SECURE` | `false` | Set `true` when serving the UI behind HTTPS |
| `ADMIN_TOKEN` | Empty | Optional administrator API token for automation |
| `TELEGRAM_API_ID` / `TELEGRAM_API_HASH` | Empty | Only needed for Telegram MTProto / WEB proxies; ordinary Bot API delivery does not need them |
| `ALLOW_INSECURE_HTTP` | `false` | Allow HTTP notification destinations and unencrypted SMTP when explicitly enabled |
| `RUST_LOG` | `relayix=info` | Application log filter |

`ADMIN_PASSWORD` bootstraps a new account. Change an existing account's password in the UI. Changing `POSTGRES_PASSWORD` in `.env` does not change the password in an initialized PostgreSQL database.

The `.env` file contains credentials. Keep it on the deployment host and outside source control. The repository ignores `.env` files; the committed template contains no credentials.

## Existing installations using the previous standalone files

Use the root `compose.yaml` and keep your existing `.env`, passwords and Compose project name. Add these lines to `.env`:

```dotenv
DATABASE_MODE=local
COMPOSE_PROFILES=${DATABASE_MODE}
```

For an existing external database, use `DATABASE_MODE=external` and retain its connection settings. Replace the former TLS overlay with `POSTGRES_CA_PATH` in `.env`. Then run `docker compose up -d` with no `-f` flags. The bundled database keeps its existing named volume. Switching database locations does not copy data between them.

## Updating

Change the app image tag in your `compose.yaml`, then run:

```bash
docker compose pull app
docker compose up -d --no-deps app
```

Migrations run automatically. Keep `.env`, the directory/project name and database volume unchanged. Updating the app does not require changing the PostgreSQL image.

## Logs and status

```bash
docker compose ps
docker compose logs -f app
```

After editing `.env`, run `docker compose up -d` to apply changes. To stop containers, use `docker compose stop`; `docker compose up -d` starts them again. Do not use `docker compose down -v` when retaining database data.

Existing private-JSON installations and GitHub-hosted deployment settings are documented in [the advanced deployment guide](DEPLOYMENT_ADVANCED.md) and [GitHub delivery](GITHUB.md). They continue to use their existing configuration and volume; changing the Compose configuration does not migrate data between installations.

## Startup diagnostics

Run `docker compose logs app` if the application exits. Configuration checks validate `PORT`, boolean settings, retry timing and paired Telegram application credentials. PostgreSQL errors identify authentication (`POSTGRES_USER` / `POSTGRES_PASSWORD`), database name (`POSTGRES_DB`), TLS (`POSTGRES_SSLMODE` / `POSTGRES_SSLROOTCERT`, mounted via `POSTGRES_CA_PATH`) or network (`POSTGRES_HOST` / `POSTGRES_PORT`). Installations using `DATABASE_URL` should check the equivalent URL fields. Credentials and full connection URLs are omitted.

With both Telegram application credentials empty, direct/HTTP Bot API delivery remains available; MTProto/WEB checks report the missing settings. The application does not request Telegram login codes or create API credentials.

## HTTPS with Caddy

The default host binding is now `127.0.0.1:8080`. For remote administration without public exposure, use `ssh -L 8080:127.0.0.1:8080 your-server` and open `http://127.0.0.1:8080` locally.

For a public endpoint, point a domain at the server, install Caddy on the host, and allow inbound ports 80 and 443. Keep `BIND_ADDRESS=127.0.0.1`, set `COOKIE_SECURE=true` and use the stable bridge gateway trust described below, then recreate the application:

```bash
docker compose up -d --no-deps app
sudo env RELAYIX_DOMAIN=notify.example.org caddy run --config deploy/Caddyfile --adapter caddyfile
```

The included Caddyfile enables automatic HTTPS and proxies to the local application. Run Caddy under your service manager for persistent operation, with the same `RELAYIX_DOMAIN` environment variable. Open `https://notify.example.org` and use that origin for webhook endpoints.

## Outbound policy and runtime limits (0.8.3)

Private, loopback, link-local, multicast and reserved destination addresses are denied by default, including DNS answers, SMTP servers and channel proxies. DNS answers for direct HTTP requests are checked by the resolver used for the actual connection. Redirects and implicit environment proxies are disabled. SMTP and Telegram proxy connections use checked, pinned addresses while SMTP retains the original TLS hostname.

For an internal Gotify/ntfy or SMTP server, explicitly list its IP or subnet, for example `OUTBOUND_ALLOWLIST=192.168.10.5/32,fd12:3456::5/128`. This administrative setting trusts these destinations for all channel creators; prefer individual addresses. `ALLOW_INSECURE_HTTP=true` permits HTTP separately and does not bypass address checks. HTTP proxies resolve HTTP destinations themselves: use only trusted proxies, enforce the same destination restrictions on the proxy, and restrict its egress. Database connections use their existing separate database configuration.

Login permits 60 requests per minute per client identity, with an additional 10-failure window per identity and username for 15 minutes. `WEBHOOK_RATE_LIMIT_PER_MINUTE=600` sets webhook requests per minute per client across routes (1–1,000,000); rejection returns 429 with `Retry-After`. IPv6 identities are aggregated by `/64`. Configure `TRUSTED_PROXIES` for forwarded client addresses; otherwise a proxy shares both quotas and account lockouts across its clients. See the [trusted proxy settings](#trusted-reverse-proxies-084) below. Multiple application replicas have independent request quotas. After 30 failed logins for one normalized username across client IPs within 15 minutes, PostgreSQL admits only one password verification every two seconds for that username, including across replicas. Requests rejected by this short gate do not extend it or the failure window; a successful login clears the account counter. Other usernames remain independent. This soft account limit can briefly delay a legitimate login under attack; it does not guarantee an attacker cannot contest successive slots.

`DELIVERY_WORKERS=4` controls concurrent normal deliveries (1–32). Telegram subprocesses have a separate limit of four across diagnostics and deliveries. Shutdown stops new claims and drains active work; Compose allows 75 seconds. Expired leases recover interrupted work on startup. `/healthz` checks PostgreSQL and returns 503 when it is unavailable; the image includes a healthcheck.

`EVENT_RETENTION_DAYS=30` removes completed history older than 30 days, including delivery attempts and alert history. Set `0` to keep history indefinitely. Maintenance runs once per minute in batches of up to 1,000 events and 1,000 alerts, preserving events with queued/processing deliveries. Sessions, expired retry confirmations and login windows are also cleaned. Back up history before upgrading if you need to retain more than 30 days, and set the retention value before startup.

The migration installs PostgreSQL's trusted `pg_trgm` extension for substring-search indexes; the migration role needs `CREATE` on the database, or an administrator must install the extension first. Index creation on a large existing history can extend migration time. Distinct event tags are maintained transactionally so tag suggestions no longer scan all events.

## Trusted reverse proxies (0.8.4)

Starting in 0.8.5, the standalone Compose bundle fixes its bridge to `172.30.84.0/24` with gateway `172.30.84.1`. If `TRUSTED_PROXIES` is unset, Compose trusts only `RELAYIX_NETWORK_GATEWAY` (default `172.30.84.1`), so host Caddy needs no manually discovered Docker IP. Recreating the network keeps this address stable. Keep `BIND_ADDRESS=127.0.0.1`: processes on the Docker host are inside this trust boundary.

If that subnet overlaps an existing network, or for a second installation on the same Docker host, set **both** `RELAYIX_NETWORK_SUBNET` and `RELAYIX_NETWORK_GATEWAY` to a free matching subnet/gateway. With no explicit `TRUSTED_PROXIES`, trust follows the new gateway automatically. Docker reports subnet conflicts at startup instead of silently choosing another network. A containerized Caddy or a different routing arrangement still needs an explicit exact proxy address/CIDR; do not trust the entire bridge or client network.

**Upgrade from 0.8.4 or earlier:** replace `compose.yaml`, retain your credentials and database volume, and remove an old empty/stale `TRUSTED_PROXIES` assignment from `.env` (or the private JSON) to use the new gateway default. The network must be recreated once to apply fixed IPAM. Stop the stack with `docker compose down` **without `--volumes`**, then run `docker compose up -d`; this preserves the database volume. For private JSON deployments, use `./scripts/compose.sh down` and `./scripts/compose.sh up -d`. This briefly interrupts service. If you use a custom/external Docker network, retain its IPAM and explicitly configure the actual proxy peer.

Relayix reads `X-Forwarded-For` only from trusted TCP peers, walks the chain right-to-left through trusted proxies, and selects the first untrusted address. Multiple header lines are treated as one chain. Invalid trusted chains return 400; untrusted peers cannot change their identity using forwarding headers. Hostnames, IP:port entries and all-address `/0` networks (including mapped IPv4 equivalents) are rejected at startup. `Forwarded` and `X-Real-IP` do not override the selected address.

For direct binary/container launches, the application default remains empty. An explicitly empty `TRUSTED_PROXIES` in Compose also disables forwarding trust. At startup an empty list emits a warning about shared proxy quotas and IP/username lockouts. To confirm unusual host routing, temporarily set `RUST_LOG=relayix=debug` and inspect `peer_ip` / `client_ip` in app logs. Keep the app port accessible only through your intended proxy.

Caddy supplies forwarding headers and ignores client-supplied values by default. For additional proxies/CDNs in front of Caddy, configure the appropriate trusted proxy chains on Caddy as well as Relayix. See [Caddy's forwarding-header documentation](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy#defaults).

Both request quotas and the 15-minute failed-login window use the same selected client identity. IPv4 addresses use individual buckets; IPv6 addresses share buckets by `/64`, and IPv4-mapped IPv6 addresses use the corresponding IPv4 bucket. Proxy trust itself is checked against the full IP address before this aggregation. Rotating IPv6 interface IDs within one `/64` therefore cannot create thousands of limiter entries or bypass a lockout.

If a proxy is not trusted, or a trusted proxy omits `X-Forwarded-For`, all its clients still share the proxy's quotas **and the IP/username lockout**. Configure this setting before exposing the login through a reverse proxy. Request quotas remain per process and reset on restart; the failed-login window is stored in PostgreSQL.

Delivery leases are renewed every 20 seconds for both notifications and alert deliveries, including while waiting for a Telegram subprocess slot. Renewals require the same active attempt number and an unexpired lease; a failed renewal or ownership change cancels the transport. Completion updates are fenced by attempt number. For Telegram MTProto/WEB, the 60-second send deadline starts **after** obtaining one of the four subprocess slots; the 50-second subprocess deadline remains in place. Waiting for capacity does not time out a delivery or consume an additional retry. Other transports retain their 60-second send deadline. Shutdown drains workers for up to 65 seconds; queued slot waiters can be interrupted and recovered by lease expiry after restart. Lease recovery after a crash remains available; renewal does not make external delivery outcomes exactly-once after a network failure.
