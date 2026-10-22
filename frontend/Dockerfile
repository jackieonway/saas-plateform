# syntax=docker/dockerfile:1
# Build from frontend/: docker build -t saas-platform-web .
FROM node:24.18.0-alpine AS build
WORKDIR /app/frontend
RUN npm install --global npm@11.16.0
COPY package.json package-lock.json .npmrc ./
RUN npm ci --ignore-scripts --no-audit --no-fund
COPY . ./
RUN node scripts/check-sdk.mjs
ARG SOURCE_REVISION
ARG HOST_BUILD_ID
RUN npm run build && node deploy/finalize-build.mjs --dist /app/frontend/dist --source /app/frontend --history /app/frontend/deploy/history --source-revision "$SOURCE_REVISION" --build-id "$HOST_BUILD_ID"

FROM nginxinc/nginx-unprivileged:1.28.0-alpine AS runtime
USER 101:101
COPY --from=build --chown=101:101 /app/frontend/dist/ /usr/share/nginx/html/
COPY --chown=101:101 deploy/nginx.conf /etc/nginx/nginx.conf
COPY --chown=101:101 deploy/mime.types /etc/nginx/mime.types
EXPOSE 8080
STOPSIGNAL SIGQUIT
HEALTHCHECK --interval=15s --timeout=3s --start-period=5s --retries=3 CMD wget -q -O /dev/null http://127.0.0.1:8080/healthz || exit 1
ENTRYPOINT ["nginx"]
CMD ["-g", "daemon off;"]
