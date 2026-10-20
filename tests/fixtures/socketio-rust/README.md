# Phase 22 Socket.IO protocol fixture

This is an isolated, loopback-only Rust test peer. It is not the notification
service: authentication uses synthetic tokens and events use in-memory identities.
It has no platform HTTP client, JWT key, database, LISTEN loop or production route.

Run the real client matrix from the repository root with Node 24.18.0 and Rust
1.97.0: `npm run test:realtime`. The driver compiles this crate with `--locked`,
uses a random IPv4 loopback port and closes stdin to shut it down. `PHASE22_PORT`
defaults to `0`; `PHASE22_ORIGIN` defaults to `http://127.0.0.1:3000` and must be an
exact IPv4 loopback HTTP origin. Never put this fixture behind the gateway.

The fixture serves Socket.IO at `/socket.io/`, namespace `/`, with a 400ms event
authentication deadline, 200ms/500ms Engine.IO heartbeat, 8192-byte payload limit
and bounded 32-packet buffers. All transport requests, including upgrades, require
the single configured Origin. `GET /health/live` and the test control route
`POST /__fixture__/notification` are local harness interfaces and have no identity
authentication. The latter accepts `{recipient,id}` and emits to a composed
tenant/organization/user room, making cross-identity leakage testable.

| Synthetic token | Identity | Lifetime | Control recipient |
| --- | --- | --- | --- |
| `valid-a` | user-a / tenant-a / org-a | 60s | a |
| `valid-b` | user-b / tenant-b / org-b | 60s | b |
| `same-user-tenant-b` | user-a / tenant-b / org-a | 60s | tenant-b |
| `same-user-org-b` | user-a / tenant-a / org-b | 60s | org-b |
| `expiring` | user-a / tenant-a / org-a | 1s | a |
| anything else, including `expired` | rejected | none | none |

Only one strict object `authenticate {token}` is accepted. Success emits the
existing public context and notification 1.0 DTOs; rejection and expiry emit
`unauthorized {code:"SESSION_INVALID",message:"实时通知会话已失效"}` followed by a
Socket.IO server disconnect. These tests prove packet/transport compatibility,
not platform authorization, real token expiry or Phase 23 scheduling behavior.

The selection, supported matrix and limitations are documented in
[the protocol decision](../../../docs/realtime-protocol.md) and
[Phase 22 evidence](../../../docs/validation/phase-22.md).
