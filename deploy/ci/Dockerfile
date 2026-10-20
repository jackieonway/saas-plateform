# Build-only verification tools. This is never a deployed application image.
# Fixed linux/amd64 official tool binary, matching the required GitHub browser runner.
FROM nginx:1.28.0-bookworm@sha256:3b03dfd827515a07ccf4946000f0bbdd2a4fb6aca7499249fcb6b1f9726823d6 AS nginx-tools

FROM rust:1.97.0-bookworm AS toolchain
ARG TARGETARCH
ARG NODE_VERSION=24.18.0
ARG NPM_VERSION=11.16.0
ENV DEBIAN_FRONTEND=noninteractive \
    PLAYWRIGHT_BROWSERS_PATH=/opt/ci/browser/.browsers \
    CI=true
RUN set -eu; \
    case "$TARGETARCH" in \
      amd64) node_arch=x64; node_sha=55aa7153f9d88f28d765fcdad5ae6945b5c0f98a36881703817e4c450fa76742 ;; \
      arm64) node_arch=arm64; node_sha=58c9520501f6ae2b52d5b210444e24b9d0c029a58c5011b797bc1fe7105886f6 ;; \
      *) echo 'Unsupported CI target architecture' >&2; exit 1 ;; \
    esac; \
    test "$NODE_VERSION" = 24.18.0; test "$NPM_VERSION" = 11.16.0; \
    curl --fail --show-error --location --retry 3 "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-${node_arch}.tar.xz" -o /tmp/node.tar.xz; \
    echo "$node_sha  /tmp/node.tar.xz" | sha256sum --check --strict; \
    tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1; \
    rm /tmp/node.tar.xz; \
    npm install --global --ignore-scripts "npm@${NPM_VERSION}"; \
    test "$(node --version)" = v24.18.0; test "$(npm --version)" = 11.16.0; \
    rustup component add rustfmt clippy
WORKDIR /workspace
CMD ["node", "scripts/ci/run.mjs", "--gate", "fast", "--out-dir", "/reports/fast"]

FROM toolchain AS browsers
WORKDIR /opt/ci/browser
COPY tests/browser/package.json tests/browser/package-lock.json ./
RUN export http_proxy="${HTTP_PROXY:-}" https_proxy="${HTTPS_PROXY:-}"; \
    sed -i 's|http://deb.debian.org|https://deb.debian.org|g' /etc/apt/sources.list.d/debian.sources; \
    npm ci --ignore-scripts --no-audit --no-fund \
    && node node_modules/playwright/cli.js install --with-deps chromium chrome msedge \
    && test "$(node -p "require('./node_modules/playwright/package.json').version")" = 1.64.1 \
    && rm -rf /var/lib/apt/lists/* /root/.npm
# Phase21 serves the actual Vite dist over owned HTTPS using production Nginx.
# Keep this separate from the locked browser installation so that adding these
# declared test tools does not invalidate the existing browser-download layer.
RUN export http_proxy="${HTTP_PROXY:-}" https_proxy="${HTTPS_PROXY:-}"; \
    apt-get update \
    && apt-get install --yes --no-install-recommends nginx openssl libnss3-tools \
    && nginx -v && openssl version \
    && dpkg-query --show --showformat='${Package}=${Version}\n' nginx openssl libnss3-tools \
    && rm -rf /var/lib/apt/lists/*
# Debian's nginx package supplies runtime libraries and MIME types. The official
# 1.28 binary accepts the production renderer's http2 directive unchanged.
COPY --from=nginx-tools /usr/sbin/nginx /usr/sbin/nginx
RUN (getent passwd nginx || useradd --system --user-group --no-create-home --shell /usr/sbin/nologin nginx) \
    && getent passwd nginx && getent group nginx \
    && nginx -V && ldd /usr/sbin/nginx \
    && mkdir -p /var/cache/nginx \
    && printf 'events { worker_connections 16; }\nhttp { server { listen 127.0.0.1:18999; return 200; } }\n' > /tmp/phase21-nginx-probe.conf \
    && nginx -t -c /tmp/phase21-nginx-probe.conf \
    && rm /tmp/phase21-nginx-probe.conf
# Every run copies a sanitized, clean snapshot to /workspace. No developer home
# or source-tree dependency cache is mounted into this tool image.
WORKDIR /workspace
CMD ["node", "scripts/ci/run.mjs", "--gate", "fast", "--out-dir", "/reports/fast"]
