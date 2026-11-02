# Phase 10 — Online and offline refresh lifecycle

Date: 2026-10-28  
Status: COMPLETED

## Inputs and boundaries

- Phase 9 canonical user grants and opaque access tokens, the [implementation contract](../implementation-contract.md), and adopted Q7 lifetime policy.
- The existing workspace contained uncommitted work from Phases 1–9. It was preserved. No production service, gateway, signing key, existing business database or remote environment was changed.
- Canonical `0001` already defines `auth_refresh_families` and `auth_refresh_tokens`; no schema migration was needed.
- Refresh is implemented for `USER` / `ACCOUNT_ONLY` clients. `PLATFORM_CONTEXT` refresh remains unavailable until its context interaction is implemented.

## Delivered changes

- `/oauth/token` now accepts `refresh_token` for enabled clients that explicitly register that grant. Authorization-code exchange creates a family only for clients allowed to refresh, returns an opaque refresh token once, and stores only its SHA-256 digest.
- Access and ID tokens remain limited to 5 minutes. Online families inherit the original grant/session absolute deadline and use a 30-minute idle limit. Each refresh rechecks the live, non-revoked SSO session, account generation, current consent and group version, client policy and application projection. Rotation advances the idle deadline without changing the family absolute deadline.
- Offline issuance requires all of: the `offline_access` scope registered on the UserInfo resource, current application and client approval for that scope, the client's `refresh_token` grant, and explicit user consent. Only that ACCOUNT_ONLY grant receives a 7-day absolute / 24-hour idle family.
- Offline verification and refresh tolerate natural browser-session expiry while still requiring the original source-session row to remain unrevoked and bound to the same account generation. Global logout, account disable/version change, consent revocation, and application/client policy changes continue to invalidate the grant.
- Rotation consumes the current token and inserts a single successor in the same family. The optional refresh `scope` must be a subset of the original grant; the new access token records only the selected subset. It cannot add `offline_access`, another scope, audience, account, or a newer consent/policy version.
- Concurrent refresh serializes on the family. Reuse of a consumed token commits family revocation, revokes every refresh token in the family and invalidates the family's access tokens before returning `invalid_grant`.
- `/oauth/revoke` now recognizes an owned refresh token and revokes its family plus attached access tokens in one audited transaction. Unknown and foreign tokens keep the same successful response; revocation still works after the source account/session becomes invalid.
- Discovery now advertises `refresh_token` and `offline_access`. The actual offline capability remains client-, application- and user-consent-gated.

## Acceptance self-check

| Acceptance criterion | Result | Evidence |
|---|---|---|
| Rotation, replay family revocation, concurrency, idle/absolute expiry and source upper bounds have real database tests | PASS | The Phase 10 PostgreSQL HTTP regression checks online absolute/idle limits, refresh rotation, same-token concurrency (one success, one `invalid_grant`), family/access-token revocation on reuse, online source expiry, offline idle expiry, and offline absolute expiry. |
| Refresh cannot expand scope, replace an invalid subject/member, or repair old versions | PASS | `scope=openid` narrows the returned token; requesting `offline_access` on the online grant returns `invalid_grant`. Refresh compares stored grant, account, consent/group, client and application generations and only accepts ACCOUNT_ONLY grants. No member-switching context is accepted. |
| An online BFF cannot use a mistaken `offline_access` request to exceed source expiry | PASS | The regression configures offline scope permission but omits it from the online grant; scope expansion is denied. The online family remains bounded by its original SSO expiry and refresh fails after that session naturally expires. |
| Offline ACCOUNT_ONLY survives natural browser-session expiry but honors explicit revocation facts | PASS | UserInfo and refresh remain valid after only the session expiry timestamp passes. The same test rejects refresh after global logout, account disable/version change and application suspension; client family revocation is also verified after account invalidation. |

## Verification

- `saas-identity`: `cargo check --all-targets` — passed.
- `saas-identity`: `cargo test` — 54 passed, 0 failed, 16 ignored. Database-backed tests requiring explicit task-owned PostgreSQL were run separately.
- `saas-identity`: Phase 10 refresh lifecycle PostgreSQL HTTP regression — 1 passed, 0 failed.
- `saas-identity`: Phase 9 resource/introspection/access-token revocation PostgreSQL HTTP regression — 1 passed, 0 failed after the refresh revocation extension.
- `saas-identity`: Phase 8 authorization-code/OIDC PostgreSQL HTTP regression — 1 passed, 0 failed. It was run before the refresh-only `/oauth/revoke` branch was added; that branch is covered by the Phase 10 test.
- PostgreSQL fixtures reported `assertions_ok=true`, `database_removed=true`, and `pool_closed=true` for each run.

## Temporary resources and cleanup

- A task-owned WSL PostgreSQL 18.6 cluster configuration used `/tmp/codex-phase10-refresh-20261028`, bound to `127.0.0.1:55491`. It was initialized in two sequential verification passes after the refresh-revocation endpoint was added; the same cluster was reused within each pass, and no copies ran concurrently. The Phase 8/9/10 fixtures created uniquely named, marked databases and removed each database on completion. The final database query found no Phase 8/9/10 test databases.
- Each cluster instance was stopped with `pg_ctl`; its exact resolved temporary directory was removed before any later pass or final return. Port 55491 has no response; the existing PostgreSQL service on port 5432 remains accepting connections and was not used for test data.
- Docker context was `default`. `docker system df` before and after showed 0 images, containers, local volumes and build cache. No Docker resources were created. An attempted image pull timed out before creating an image or container.
- WSL filesystem usage changed from 13,480,308,736 to 13,374,963,712 bytes used (about 100.5 MiB reclaimed in WSL). Windows D: available space decreased by 45,568,000 bytes while Rust build outputs were produced; those reusable Cargo build artifacts were retained. Windows C: available space decreased by 5,238,784 bytes. No WSL virtual disk compaction was performed, so this does not claim a Windows VHDX shrink.

## Risks and remaining boundaries

- These checks use explicit test limits and a temporary database; they do not assert that a deployed environment has approved or configured offline access.
- `PLATFORM_CONTEXT`, MACHINE `client_credentials`, BFF integration and business membership/data-range enforcement remain later phases. Q3/Q8/Q9/Q10 release facts remain open gates.
- The transitional `/oidc/*` facade remains until Phase 24 retirement.
