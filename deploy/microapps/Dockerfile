# syntax=docker/dockerfile:1
# Build from the repository root. The driver validates APP_CODE before build.
ARG NODE_IMAGE=node:24.18.0-alpine
ARG STATIC_IMAGE=nginxinc/nginx-unprivileged:1.28.0-alpine
FROM ${NODE_IMAGE} AS build
ARG APP_CODE
WORKDIR /app
RUN npm install --global npm@11.16.0
COPY business-apps/${APP_CODE}/frontend/package.json business-apps/${APP_CODE}/frontend/package-lock.json ./
RUN npm ci --ignore-scripts --no-audit --no-fund
COPY business-apps/${APP_CODE}/frontend/ ./
COPY deploy/microapps/render-static.mjs deploy/microapps/build-static.mjs /tools/microapps/
COPY deploy/gateway/policy.mjs /tools/gateway/policy.mjs
ARG APP_VERSION
ARG HOST_ORIGIN
ARG API_BASE
RUN APP_CODE="$APP_CODE" APP_VERSION="$APP_VERSION" HOST_ORIGIN="$HOST_ORIGIN" API_BASE="$API_BASE" node /tools/microapps/build-static.mjs

FROM ${STATIC_IMAGE} AS runtime
ARG APP_CODE
ARG APP_VERSION
ARG SOURCE_REVISION
ARG INPUTS_SHA256
ARG BUILD_ID
LABEL org.opencontainers.image.title="${APP_CODE} immutable static frontend" \
    org.opencontainers.image.revision="${SOURCE_REVISION}" \
    org.opencontainers.image.version="${APP_VERSION}" \
    saas.application="${APP_CODE}" \
    saas.ci.inputs-sha256="${INPUTS_SHA256}" \
    saas.build-id="${BUILD_ID}"
COPY --from=build --chown=101:101 /app/dist/ /usr/share/nginx/html/
COPY --from=build --chown=101:101 /out/nginx.conf /etc/nginx/nginx.conf
USER 101:101
EXPOSE 8080
STOPSIGNAL SIGQUIT
HEALTHCHECK --interval=5s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -q -O /dev/null http://127.0.0.1:8080/healthz || exit 1
ENTRYPOINT ["nginx"]
CMD ["-c", "/etc/nginx/nginx.conf"]
