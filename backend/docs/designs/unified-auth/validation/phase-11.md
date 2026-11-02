# Phase 11 — MACHINE client credentials

Date: 2026-10-28  
Status: COMPLETED

## Inputs and boundaries

- Phase 9 resource-bound opaque tokens and introspection, Phase 4 unified credential management, and the adopted scope and client-subject contract.
- `saas-identity` baseline HEAD: `7025a0a39cff5577bc3aabf239aaae8df529714e`; `saas-plateform` baseline HEAD: `34105757b040165daa5477bf6afff9637f617466`.
- Both worktrees already contained uncommitted Phase 1–10 work. It was preserved. No production service, gateway, signing key, existing business database, or remote environment was changed.
- The canonical fresh-install `0001` schema was extended. No old database was migrated or cleared.
- Phase 11 adds token issuance only. It does not impersonate a user, issue an ID Token or user refresh token, or authorize a client to act on behalf of a user.

## Delivered changes

- The canonical token endpoint advertises and accepts `client_credentials` only for an enabled, approved, confidential `MACHINE` / `SERVICE` client with `NONE` context and `client_secret_basic` authentication.
- Every requested scope must be active, included in the application-approved scope ceiling, and present in the exact active client permission rows. Audiences come from the registered resources; request parameters cannot supply or widen an audience. UserInfo and human OIDC scopes are not valid machine permissions.
- Access tokens remain opaque and digest-stored. A machine grant stores its exact scope IDs, audiences, client/application policy versions, and the credential version that authenticated the request. The token lifetime is capped at five minutes and at the source credential expiry.
- Machine token verification rechecks the original credential version, its revocation and expiry state, current client/application status and versions, and current resource/scope permissions. Credential emergency revocation invalidates already issued tokens. Credential rotation can keep a short overlap; tokens remain bound to the version that issued them.
- Machine grants have no user, consent, source session, account version, authentication time, or business context. Machine introspection returns `subject_mode=MACHINE` and omits `sub` and `auth_time`; UserInfo rejects the token.
- Discovery advertises `client_credentials`. Canonical `0001` now requires every MACHINE grant to reference a persisted credential version belonging to that same client, while USER grants retain a null credential version.

The main implementation is in `saas-identity/src/oidc/oauth.rs`, `saas-identity/src/oidc/protocol.rs`, and `saas-identity/src/auth_admin.rs`. The schema and canonical constraint regression are in `saas-plateform/backend/migrations/0001_init.sql` and `saas-plateform/backend/src/identity_deployment_tests.rs`. HTTP coverage is in `saas-identity/src/util/audit_tests/auth_protocol.rs`.

## Acceptance self-check

| Acceptance criterion | Result | Evidence |
|---|---|---|
| A valid confidential client receives the correct machine audience and scope | PASS | The PostgreSQL HTTP regression exchanges `client_credentials`, then introspects the opaque token and checks its registered Orders audience and exact `orders.read` scope. |
| PUBLIC, USER, disabled credentials, out-of-resource permissions and unapproved data scopes are rejected | PASS | PUBLIC and USER clients cannot use the grant. `orders.admin` is present in the client permission rows but outside the application approval ceiling; `inventory.read` is application-approved but outside that client's resource permissions. Both are rejected. After emergency revocation, both the rotated credential and the overlapping old credential fail authentication. |
| Machine tokens carry no person identity and cannot be used as a person at UserInfo | PASS | The response and grant have no user, consent, session or account binding; introspection omits `sub` and `auth_time`; UserInfo returns 401 for the machine token. |
| Emergency revocation and credential-version source validation take effect | PASS | Rotation issues version 2, the resulting grant records version 2, and emergency revocation makes tokens from versions 1 and 2 inactive and prevents either credential from obtaining a new token. Canonical schema tests reject missing or unknown MACHINE credential versions. |

## Verification

- `saas-identity`: `cargo fmt --all -- --check`, `cargo check --all-targets`, and `cargo test --locked` passed. The normal suite reported 54 passed, 0 failed, 17 ignored; the database-backed regressions below were invoked separately.
- `saas-identity`: `cargo clippy --locked --all-targets` passed. It reported seven warnings in earlier admin/consent and refresh/revocation paths; there were no warnings in the Phase 11 MACHINE issuance, verification, schema or HTTP test paths.
- `saas-identity`: Phase 11 MACHINE HTTP/PostgreSQL regression — 1 passed, 0 failed (repeated after the final code changes).
- `saas-identity`: Phase 4 unified client and credential lifecycle regression — 1 passed, 0 failed.
- `saas-identity`: Phase 8 authorization-code/OIDC and TOTP HTTP/PostgreSQL regressions — 2 passed, 0 failed.
- `saas-identity`: Phase 9 resource, introspection and revocation HTTP/PostgreSQL regression — 1 passed, 0 failed.
- `saas-identity`: Phase 10 refresh rotation and source-boundary HTTP/PostgreSQL regression — 1 passed, 0 failed (repeated after the shared OAuth query-structure cleanup).
- `saas-plateform/backend`: `cargo fmt --all -- --check` and `cargo check --all-targets` passed.
- `saas-plateform/backend`: `cargo test --locked phase11_machine_grants_require_a_persisted_client_credential_version -- --ignored --nocapture` — 1 passed, 0 failed against the canonical `0001` migration. The test confirmed that a MACHINE grant cannot omit its source credential version or point to an unregistered version.
- Both repositories: `git diff --check` passed. Git emitted only existing line-ending conversion advisories.

## Temporary resources and cleanup

- Verification covered two repositories: `saas-identity` and `saas-plateform/backend`.
- No Docker containers were created or reused. The WSL Docker context was `default`; `docker system df` before and after showed 0 images, containers, local volumes and build cache.
- One task-owned native PostgreSQL 18.6 cluster ran in WSL Ubuntu at `/tmp/codex-phase11-machine-20261028`, bound to `127.0.0.1:55492` (cluster identifier `7701493770002498624`). It used the WSL `/tmp` tmpfs and was separate from the existing PostgreSQL service on port 5432.
- Each fixture created a uniquely named database. Before stopping the cluster, queries found zero remaining `identity_phase02_*`, `identity_phase04_*`, `identity_phase08_*`, `identity_phase09_*`, `identity_phase10_*`, or `identity_phase11_*` databases and zero `id02_*` roles. Test output confirmed each pool was closed and each fixture database was removed.
- The PostgreSQL cluster was stopped and its verified task-owned directory, including data, socket and log files, was removed. Port 55492 no longer accepts connections; the existing service on port 5432 remained available. The temporary RSA-key fixture directories were also absent after the tests.
- `/tmp` used 0 KiB before cluster creation, 56,404 KiB while it ran, and 0 KiB after cleanup; 56,404 KiB was released from the WSL tmpfs. Docker usage remained at zero. Cargo build artifacts were retained for reuse. No WSL VHDX compaction was attempted, and Windows disk recovery is not claimed.

## Risks and remaining boundaries

- These tests exercise the issuer and registry against a task-owned fresh database. Resource services still need their later consumer phases to enforce machine subject, audience, scope, and explicit tenant/data-range bindings on real API requests.
- No production deployment or third-party interoperability claim is made. Q3/Q8/Q9/Q10 release facts remain gated as recorded in the ledger.

