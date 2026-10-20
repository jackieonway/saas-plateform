# Phase 3 Rust transport fixture

`TRANSPORT_FIXTURE_ONLY`: this independent, unpublished test crate is an HTTP and
WebSocket peer for the real Nginx/Linux Docker tests. It has no database, platform
implementation, JWT, user authorization, or production role. Do not install its
binary into any production image or point a public gateway at it. In particular,
the small polling response resembles an Engine.IO open packet solely to exercise
query strings and POST forwarding; this is **not** a complete Engine.IO/Socket.IO
server. Rust notification compatibility belongs to Phase 22/23.

The crate is separate because the gateway test needs an independently replaceable
Rust process/container without platform/SQLx/database startup. It does not create
a root Cargo workspace or change the platform crate. Axum (HTTP/WebSocket), Tokio
(listener/runtime/shutdown), and serde_json (observable fixed test responses) reuse
the platform stack; Axum features and Tokio features are restricted to this test.
The dev-only Tower utility invokes the router for negative and transport tests.
The committed Cargo.lock owns its resolved dependency set. No new production
dependency is introduced.

Run from the repository root with the pinned Rust 1.97.0 toolchain:

```powershell
$env:BIND_ADDR = '127.0.0.1:39090'
$env:INSTANCE_ID = 'fixture-a'
cargo run --locked --manifest-path tests/fixtures/gateway-rust/Cargo.toml
```

`BIND_ADDR` is mandatory. Only loopback/private IPv4 or IPv6 loopback is allowed.
In an unpublished isolated container, explicitly supply `BIND_ADDR=0.0.0.0:3000`
and `ALLOW_CONTAINER_BIND=1`. `INSTANCE_ID` is an optional non-secret marker used
to prove upstream replacement (`fixture-a` by default). Never pass real Bearer
tokens: the fixture deliberately echoes the Authorization header for assertions.
The test uses the fixed dummy value `Bearer fixture-token`.

| Route | Transport and response |
| --- | --- |
| `GET /health` | JSON `{marker, instanceId, status:"ok"}` |
| `GET/POST /api/echo`, `/oauth/echo`, `/work-orders-api/echo` | JSON snapshot of method, full URI, path, raw query, body, byte length and selected request headers |
| `GET /socket.io/?EIO=4&transport=polling` | Text `0` plus fixed open-packet-shaped JSON; `.fixture` contains the request snapshot |
| `POST /socket.io/?EIO=4&transport=polling&sid=fixture-a` | JSON snapshot, preserving the submitted polling body |
| `GET /socket.io/?EIO=4&transport=websocket` | Real HTTP/1.1 upgrade; first text frame is the request snapshot, then text/binary echo and Ping/Pong/Close |
| `OPTIONS` on echo and socket routes | 204 with the fixture's CORS policy |

All JSON snapshots contain `marker: "TRANSPORT_FIXTURE_ONLY"` and `instanceId`.
Selected headers include Host, Authorization, Origin, Forwarded,
X-Forwarded-For/Host/Proto, X-Request-Id, Upgrade and Connection. Missing headers
are JSON null. The echo body uses UTF-8 replacement for invalid bytes and reports
the original byte length. HTTP bodies and WebSocket messages are capped at 64 KiB.

The fixture owns CORS: requests without Origin are allowed for server probes;
browser Origin must be `https://fixture.saas.test` or opaque `null`. A permitted
origin receives exactly one `Access-Control-Allow-Origin` and `Vary: Origin`.
Other origins return 403 without a CORS grant. This only tests Nginx transparency,
and does not model a platform identity or authorization decision.

```text
cargo fmt --manifest-path tests/fixtures/gateway-rust/Cargo.toml -- --check
cargo check --locked --manifest-path tests/fixtures/gateway-rust/Cargo.toml --all-targets
cargo clippy --locked --manifest-path tests/fixtures/gateway-rust/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path tests/fixtures/gateway-rust/Cargo.toml
cargo doc --locked --manifest-path tests/fixtures/gateway-rust/Cargo.toml --no-deps
cargo build --locked --manifest-path tests/fixtures/gateway-rust/Cargo.toml
node tests/fixtures/gateway-rust/smoke.mjs
```

The Node 24 smoke driver starts only its own Rust binary on an ephemeral loopback
port and terminates it on completion. It checks real HTTP and WebSocket text and
binary roundtrips. `FIXTURE_BINARY` can select a separately built test binary.

The Dockerfile is built with repository-root context and is consumed only by the
Phase 3 test Compose overlay. It contains the Rust binary and a non-root Debian
runtime; it does not contain Node or the platform/production application.
