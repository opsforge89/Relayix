# Advanced deployment with private JSON

Set application and database variables in **one private file**:

```text
../private/relayix-config/deployment.json
```

The path is relative to the repository directory. `scripts/write_config.py` creates this file with private permissions. Use JSON strings for every value, including ports and booleans. No `.env` file is required. The application image is selected separately in **`compose.yaml`**.

## 1. Download the image

Install Git, Docker Engine with Docker Compose v2, and Python 3.10+. Your user must have access to Docker. Rust and Node.js run inside the build containers.

```bash
git clone https://github.com/opsforge89/Relayix.git
cd Relayix
docker compose version
docker pull ghcr.io/opsforge89/relayix:0.8.8
```

This guide is for existing installations and automated deployments that use the private JSON wrapper. New users should follow the [standard Docker Compose setup](DEPLOYMENT.md). The published image is ready to run; source compilation is only needed for development.

## 2. Choose the database location

| Database location | `DATABASE_MODE` | Services started by Compose |
| --- | --- | --- |
| New PostgreSQL database inside Compose | `local` | `app` and `db`, with persistent database storage |
| Existing PostgreSQL on another server | `external` | `app` only |
| Existing PostgreSQL on the Docker host | `external` | `app` only; use the Docker host's LAN IP |

### Option A: PostgreSQL inside Compose

For a new installation:

```bash
python3 scripts/write_config.py ../private/relayix-config/deployment.json --database local --generate
nano ../private/relayix-config/deployment.json
```

The generator fills in the database password, administrator password and administrator API token. Check these settings in the generated file:

```json
{
  "DATABASE_MODE": "local",
  "COMPOSE_PROJECT_NAME": "relayix",
  "BIND_ADDRESS": "127.0.0.1",
  "PORT": "8080",
  "POSTGRES_USER": "relayix",
  "POSTGRES_DB": "relayix"
}
```

This is a settings excerpt: keep the generated credentials and other fields. Local mode connects to `db:5432` inside the Compose network; `POSTGRES_HOST`, `POSTGRES_PORT` and external TLS settings are not needed. PostgreSQL is not published on a host port.

`BIND_ADDRESS=127.0.0.1` exposes the interface only on the Docker host. To access it from another computer, set the host's LAN IP or `0.0.0.0` and open the server's address in your browser.

```bash
scripts/compose.sh up -d
scripts/compose.sh ps
curl --fail http://127.0.0.1:8080/healthz
```

Open `http://127.0.0.1:8080` on the Docker host. Sign in with `ADMIN_USERNAME` and `ADMIN_PASSWORD` from the private JSON. Substitute your address and port if you changed them.

Reuse the same private settings, project name and volume for an existing installation. The generator refuses to overwrite an existing file. Editing `POSTGRES_PASSWORD` does not change the password in an initialized PostgreSQL database.

### Option B: Existing external PostgreSQL

Create the database and login role on the PostgreSQL server first. The role must be able to connect and create/alter objects in the application database. Relayix applies its schema migrations automatically; it does not create an external database or role.

```bash
python3 scripts/write_config.py ../private/relayix-config/deployment.json --database external --generate
nano ../private/relayix-config/deployment.json
```

The generator creates Relayix administrator credentials but does not generate a replacement password for an existing database. Fill in these fields in the generated file; replace `CHANGE_ME` with the actual database password:

```json
{
  "DATABASE_MODE": "external",
  "POSTGRES_HOST": "postgres.example.org",
  "POSTGRES_PORT": "5432",
  "POSTGRES_USER": "relayix",
  "POSTGRES_DB": "relayix",
  "POSTGRES_PASSWORD": "CHANGE_ME",
  "POSTGRES_SSLMODE": "verify-full",
  "POSTGRES_SSLROOTCERT": ""
}
```

Enter only a hostname or IP in `POSTGRES_HOST`, without a URL scheme, port or path. Enter the password as an ordinary JSON string; the wrapper handles URL encoding for characters such as `@`, `#` and `%`.

If your provider supplies a CA certificate, save its public certificate on the Docker host and set its absolute path:

```json
{
  "POSTGRES_SSLROOTCERT": "/srv/relayix-private/postgres-ca.crt"
}
```

The certificate file must be readable by the container user. The wrapper mounts it read-only into the app and the explicitly requested database client. With `verify-full`, `POSTGRES_HOST` must match the server certificate. Without a custom CA file, the clients use their trusted root certificates.

`verify-full` verifies TLS and the server name. For a server deliberately configured without TLS on a trusted network, explicitly set `POSTGRES_SSLMODE=disable`. Other supported values are `require`, `verify-ca`, `prefer` and `allow`; `verify-ca` requires a CA file. `require` encrypts traffic but does not verify the server name. See [PostgreSQL TLS modes](https://www.postgresql.org/docs/17/libpq-ssl.html).

Start with the same commands:

```bash
scripts/compose.sh up -d
scripts/compose.sh ps
scripts/compose.sh logs --tail=50 app
curl --fail http://127.0.0.1:8080/healthz
```

External mode creates no bundled database service or database volume. The wrapper selects the correct Compose file automatically.

### PostgreSQL installed on the Docker host

Use external mode with these values:

```json
{
  "DATABASE_MODE": "external",
  "POSTGRES_HOST": "192.0.2.10",
  "POSTGRES_PORT": "5432"
}
```

`localhost` inside a container refers to that container. To reach PostgreSQL installed on the Docker host, set `POSTGRES_HOST` to that host's LAN IP address.

The host database must listen on an interface reachable from Docker. Its `pg_hba.conf` and firewall must allow the database role and Docker subnet. A PostgreSQL server listening only on `127.0.0.1` cannot accept this connection. Select TLS settings to match the server configuration.

## Check the connection and back up

These commands work in both database modes:

```bash
python3 scripts/database.py psql -X -c 'SELECT current_database(), current_user;'
```

Create a database backup:

```bash
mkdir -p ../private/relayix-backups
chmod 700 ../private/relayix-backups
umask 077
python3 scripts/database.py pg_dump -Fc > "../private/relayix-backups/manual-$(date +%Y%m%d-%H%M%S).dump"
```

The database helper runs an ephemeral PostgreSQL client only when you explicitly request a SQL command or backup. Compose contains only the app and optional database services. It uses the same connection and TLS settings as the app. Its default PostgreSQL client version is 17.11. For a newer server, set a matching or newer `POSTGRES_CLIENT_IMAGE`, such as `postgres:18.6-trixie` for PostgreSQL 18. `pg_dump` cannot dump a server with a newer major version than its own.

## Settings reference

| Variable | Purpose |
| --- | --- |
| `DATABASE_MODE` | `local` or `external`; defaults to `local` |
| `POSTGRES_HOST` / `POSTGRES_PORT` | External database address and port; default port `5432` |
| `POSTGRES_USER` / `POSTGRES_DB` | Role and database; both default to `relayix` |
| `POSTGRES_PASSWORD` | Existing database password, or generated password for a new bundled database |
| `POSTGRES_SSLMODE` | Defaults to `verify-full` externally; local connections use `disable` |
| `POSTGRES_SSLROOTCERT` | Optional absolute path to a CA certificate on the Docker host |
| `POSTGRES_CLIENT_IMAGE` | PostgreSQL image for SQL checks and backups |
| `POSTGRES_IMAGE` | Bundled database image; used only in local mode |
| `RELAYIX_DB_VOLUME` | Existing local volume name; defaults to `<project>_db_data` |
| `COMPOSE_PROJECT_NAME` | Compose installation name; keep it stable for existing data |
| `BIND_ADDRESS` / `PORT` | Address and port publishing the web interface |
| `ADMIN_USERNAME` / `ADMIN_PASSWORD` | Bootstrap administrator; used when creating the account |
| `ADMIN_TOKEN` | Optional administrator API token; generated automatically |
| `RELAYIX_NETWORK_SUBNET` / `RELAYIX_NETWORK_GATEWAY` | Stable Compose bridge, defaults `172.30.84.0/24` / `172.30.84.1`; choose a separate non-overlapping pair per installation |
| `DELIVERY_PROXY_CONCURRENCY` | Shared MTProto/WEB attempt budget across channels/queues/replicas, default 2, range 1–32; keep below normal worker count to reserve capacity |
| `DELIVERY_CHANNEL_CONCURRENCY` | Active attempts per channel across both queues/replicas, default 2, range 1–32; set consistently on all replicas |
| `WEBHOOK_RATE_LIMIT_PER_MINUTE` | Webhook quota per client per minute across routes, default 600, range 1–1,000,000 |
| `TRUSTED_PROXIES` | IPs/CIDRs allowed to supply X-Forwarded-For; Compose defaults to its fixed bridge gateway, direct launches default to empty; /0 rejected ([details](DEPLOYMENT.md#trusted-reverse-proxies-084)) |
| `COOKIE_SECURE` | `true` behind HTTPS; `false` for direct HTTP |
| `TELEGRAM_API_ID` / `TELEGRAM_API_HASH` | Required for Telegram MTProto/WEB; ordinary Bot API delivery does not need them |

To use another private settings path:

```bash
export RELAYIX_CONFIG_FILE=/srv/relayix-private/deployment.json
scripts/compose.sh up -d
```

## Updating

Change the app image tag in `compose.yaml`, then run:

```bash
scripts/compose.sh pull app
scripts/compose.sh up -d --no-deps app
```

Database migrations run automatically. Keep the same database settings and data volume. When adopting the named containers and network in 0.6.3 for the first time, run `scripts/compose.sh up -d` once to apply those Compose changes to both services.

## Common operations

```bash
scripts/compose.sh logs -f app
scripts/compose.sh restart app
scripts/compose.sh stop
scripts/compose.sh up -d
```

After changing private settings, use `up -d` to recreate affected containers; `restart` does not reload their environment. Avoid `down -v` when retaining local data. Changing database mode or host does not transfer existing data automatically.
