# syntax=docker/dockerfile:1
# Build from the repository root: docker build -f frontend/Dockerfile .
FROM node:24.18.0-alpine AS build
WORKDIR /app/frontend
RUN npm install --global npm@11.16.0
COPY frontend/package.json frontend/package-lock.json frontend/.npmrc ./
RUN npm ci --ignore-scripts --no-audit --no-fund
COPY frontend/ ./
COPY examples/micro-app-sdk/ /app/examples/micro-app-sdk/
COPY scripts/sync-micro-sdk.mjs /app/scripts/sync-micro-sdk.mjs
RUN node /app/scripts/sync-micro-sdk.mjs --check --scope sdk
COPY deploy/static/finalize-build.mjs /tools/finalize-build.mjs
COPY deploy/static/history/ /history/
ARG SOURCE_REVISION
ARG HOST_BUILD_ID
RUN npm run build && node /tools/finalize-build.mjs \
    --dist /app/frontend/dist --source /app/frontend --history /history \
    --source-revision "$SOURCE_REVISION" --build-id "$HOST_BUILD_ID"

# Node and npm are build-only. Configuration is immutable at runtime; bypass
# the upstream entrypoint scripts which otherwise try to modify /etc/nginx.
FROM nginxinc/nginx-unprivileged:1.28.0-alpine AS runtime
USER 101:101
COPY --from=build --chown=101:101 /app/frontend/dist/ /usr/share/nginx/html/
COPY --chown=101:101 deploy/static/nginx.conf /etc/nginx/nginx.conf
COPY --chown=101:101 deploy/static/mime.types /etc/nginx/mime.types
EXPOSE 8080
STOPSIGNAL SIGQUIT
HEALTHCHECK --interval=15s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -q -O /dev/null http://127.0.0.1:8080/healthz || exit 1
ENTRYPOINT ["nginx"]
CMD ["-g", "daemon off;"]
