FROM node:26.13.0-trixie-slim AS web-builder
WORKDIR /web
COPY Cargo.toml /Cargo.toml
COPY web/package*.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

FROM web-builder AS web-checks
RUN npm run lint && npm test && touch /web-checks-passed

FROM python:3.15.2-slim-trixie AS npm-licenses
COPY scripts/collect_licenses.py /collect_licenses.py
COPY --from=web-builder /web/node_modules /node_modules
RUN python /collect_licenses.py npm /node_modules /licenses

FROM rust:1.99.1-trixie AS rust-builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src src
COPY migrations migrations
COPY scripts/collect_licenses.py /collect_licenses.py
RUN apt-get update && apt-get install -y --no-install-recommends python3 && rm -rf /var/lib/apt/lists/*
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --locked --release \
    && cp target/release/relayix /app/relayix \
    && cargo metadata --locked --format-version 1 > /app/rust-metadata.json \
    && python3 /collect_licenses.py rust /app/rust-metadata.json /licenses

FROM rust-builder AS rust-checks
RUN rustup component add rustfmt clippy
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings \
    && cargo test --locked --release && touch /rust-checks-passed

FROM python:3.15.2-slim-trixie AS runtime
RUN apt-get update && apt-get upgrade -y --no-install-recommends && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /nonexistent --shell /usr/sbin/nologin relayix
COPY transport/requirements.txt /app/transport/requirements.txt
RUN pip install --no-cache-dir --require-hashes -r /app/transport/requirements.txt
COPY transport/telegram_send.py transport/faketls.py /app/transport/
COPY transport/vendor /app/transport/vendor
COPY --from=rust-builder /app/relayix /usr/local/bin/relayix
COPY --from=web-builder /web/dist /app/web/dist
COPY LICENSE NOTICE THIRD_PARTY_NOTICES.md /usr/share/doc/relayix/
COPY --from=rust-builder /licenses /usr/share/doc/relayix/licenses/
COPY --from=npm-licenses /licenses /usr/share/doc/relayix/licenses/
COPY scripts/collect_licenses.py /tmp/collect_licenses.py
RUN python /tmp/collect_licenses.py python /app/transport/requirements.txt /usr/share/doc/relayix/licenses \
    && mkdir -p /usr/share/doc/relayix/licenses/python/faketls-vendored \
    && cp /app/transport/vendor/faketls/LICENSE /usr/share/doc/relayix/licenses/python/faketls-vendored/LICENSE \
    && rm /tmp/collect_licenses.py \
    && python -m pip uninstall --yes pip \
    && python -c "import pathlib,shutil,sysconfig; shutil.rmtree(pathlib.Path(sysconfig.get_path('stdlib')) / 'ensurepip')"
WORKDIR /app
USER 10001
LABEL org.opencontainers.image.title="Relayix" org.opencontainers.image.licenses="Apache-2.0"
HEALTHCHECK --interval=30s --timeout=3s --start-period=30s --retries=3 \
    CMD python3 -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8080/healthz', timeout=2).close()"
EXPOSE 8080
CMD ["/usr/local/bin/relayix"]

# CI exercises the same runtime, with checks enforced by stage dependencies.
FROM runtime AS verified
COPY --from=web-checks /web-checks-passed /usr/share/doc/relayix/checks/web
COPY --from=rust-checks /rust-checks-passed /usr/share/doc/relayix/checks/rust

# Release builds consume the application stages without rerunning the check stages.
FROM runtime AS production
