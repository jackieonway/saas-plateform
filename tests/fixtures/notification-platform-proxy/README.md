# Phase 23 real-platform timing proxy

This test-only Rust process delays real `GET /api/notifications/socket-context`
requests. It never creates identities, authorization answers, inbox records, or
Socket.IO packets. The platform remains the authority for every forwarded check.

Build with `cargo build --locked --manifest-path tests/fixtures/notification-platform-proxy/Cargo.toml --target-dir target/phase23-proxy`.
Set `PHASE23_UPSTREAM=http://127.0.0.1:<platform-port>` and optionally
`PHASE23_PROXY_PORT=0`. The process binds IPv4 loopback and emits
`{"event":"ready","port":...}` on stdout. EOF on stdin, Ctrl+C, and Unix SIGTERM
cancel outstanding work and gracefully stop the server.

- `GET /api/notifications/socket-context` forwards the original `Authorization`
  and optional `X-Request-Id`. Requests without a bearer also reach the real
  platform without an authorization header, so upstream health probes cannot
  succeed against a synthetic local 401. Only supplied credential format and
  size are checked locally; the platform decides identity and authorization.
  The only permitted query is one
  `notificationId=<canonical UUID>`. No arbitrary route, method, host, redirect,
  credential-bearing URL, or environment HTTP proxy is accepted.
- `POST /test/gates` accepts JSON
  `{"label":"slow","token":"<platform session>","deliveryOnly":true,"delayMs":12000}`.
  With `deliveryOnly:true`, only delivery checks with that bearer wait; initial
  and periodic identity checks continue directly to the real platform.
  With `deliveryOnly:false`, all socket-context checks for that bearer wait,
  including first authentication and periodic identity checks. This lets tests
  hold an actual platform request at an external await, send repeated auth
  events, or let the service auth deadline expire before releasing the real
  platform response. A request without a bearer never enters a gate. Gate
  release always continues to the real platform and never supplies an auth
  result. There are at most eight labels,
  each 1–48 ASCII letters/digits/underscore/dash. Credentials are at most 4096
  bytes, requests and upstream responses at most 4096 bytes, and delays at most
  15000 ms. All work, including gate delay, has one 15 second deadline and a
  32-request admission bound; excess work receives HTTP 429.
- `POST /test/release` accepts `{"label":"slow"}`, immediately releases pending
  waits, and disables future waits for that gate. Its counters remain observable.
  A released label can be replaced once its pending requests finish.
- `GET /test/state` returns
  `{"gates":[{"label":"slow","inflight":1,"highwater":1,"count":1}]}`.
  No control response or diagnostic contains bearer credentials. Upstream
  responses containing the current credential are rejected instead of echoed.

Dependencies are limited to the existing Axum/Tokio/Serde HTTP stack and
`reqwest` with default features disabled and rustls enabled. The reqwest client
disables redirects and environment proxy discovery. This fixture is independent
from production deployment and has no database access or JWT signing material.
