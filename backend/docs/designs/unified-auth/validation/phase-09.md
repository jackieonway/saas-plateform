# Phase 09 — Resource-bound access tokens, introspection and revocation

Date: 2026-10-28  
Status: COMPLETED

## Inputs and boundaries

- Phase 8 personnel authorization-code grants and [Phase 08 report](phase-08.md), the [implementation contract](../implementation-contract.md), fixed resource/scope registration and consent-generation rules.
- `saas-identity` baseline HEAD: `7025a0a39cff5577bc3aabf239aaae8df529714e`; `saas-plateform` baseline HEAD: `34105757b040165daa5477bf6afff9637f617466`.
- The worktrees already contained uncommitted Phase 1–8 changes. They were preserved. This phase made no production service, gateway, existing business database, signing key or remote environment changes.
- No schema migration was needed: canonical `0001` already has `auth_resources`, `auth_scopes`, client permission rows, opaque access-token digests, grant audiences and token-specific resource scopes.

## Delivered changes

- Authorization-code requests can now include scopes registered for the client across active resources. Each requested scope must match an active `auth_client_permissions` row and the application's approved scope set. `openid` is accepted only on the `userinfo` resource; resource scopes do not become ID Token/UserInfo claims.
- The opaque bearer remains digest-stored. Its grant records exact resource audiences and resource-qualified scopes, and each token records its own scope rows. ID Token claims are built only from `userinfo` scopes.
- Added public `/oauth/introspect` and `/oauth/revoke` routes and corresponding discovery metadata. Both require `client_secret_basic` from an enabled, approved, confidential client. Introspection callers must be registered MACHINE/SERVICE clients with `NONE` context; response audiences and scopes are limited to active resource permissions held by that caller. Optional `resource` narrows the result. `sub` remains pairwise for the original token client; internal account IDs and profile claims are not returned.
- Introspection revalidates token/grant expiry and revocation, account status/version, source SSO session, client and application state/version, current resources/scopes, consent generation and group version. Unknown, expired, revoked, wrong-audience, unconsented or otherwise unverifiable tokens return `active:false`; authority/database failures fail closed.
- At the Phase 9 boundary, standard revoke authenticated the client and revoked only that client's individual access token, with a fixed-text audit event and indistinguishable success for unknown or foreign tokens. Phase 10 subsequently extended the same endpoint to revoke an owned refresh family; see the [Phase 10 report](phase-10.md).
- Reworked UserInfo to use the same live access-token validation path. It requires an active `userinfo` audience and `openid` scope, then returns only claims covered by `userinfo` scopes and the same pairwise `sub` as the ID Token.
- Fixed a lock-upgrade deadlock found while rerunning the Phase 8 concurrent code-redemption regression. Read-only sufficient-consent checks now use a shared account lock; explicit approval keeps the exclusive lock. This preserves account-version fencing without two read transactions trying to upgrade the same row.

The main implementation changes are in `saas-identity/src/oidc/oauth.rs`, `saas-identity/src/oidc/protocol.rs`, `saas-identity/src/auth_consent.rs`, and `saas-identity/src/util/audit_tests/auth_protocol.rs`. The OAuth discovery and resource-token details are recorded in `implementation-contract.md`.

## Acceptance self-check

| Acceptance criterion | Result | Evidence |
|---|---|---|
| Wrong audience, unapproved scope, unrelated introspector and ID Token used as an API credential are rejected | PASS | PostgreSQL HTTP regression rejects an unregistered scope; returns inactive for a token outside the caller's resource permission; returns inactive for ID Token introspection; UserInfo rejects ID Tokens. |
| Revoked token is inactive; invalid original source fails closed | PASS | Revoke makes online introspection inactive and UserInfo return 401. Revoking the source SSO session makes the remaining access token inactive and UserInfo return 401. |
| Token records/logs contain no bearer text; revoking one source leaves other authorization active | PASS | Database stores the SHA-256 token digest; the revocation audit uses fixed text with no token. A second token from a distinct grant remains active after the first token is revoked. A non-owning client cannot revoke it. |
| UserInfo returns only claims supported by a valid, consented UserInfo scope | PASS | A token with `openid` at the `userinfo` audience and only `orders.read` at the Orders API audience returns the pairwise `sub` but no `name`; the same `sub` appears in the ID Token. |

## Verification

- `saas-identity`: `cargo test --locked phase09 -- --ignored --nocapture` — 1 passed, 0 failed against task-owned PostgreSQL 18.6.
- `saas-identity`: `cargo test --locked phase08 -- --ignored --nocapture` — 2 passed, 0 failed after the lock fix.
- `saas-identity`: the concurrent code-redemption PostgreSQL test was then repeated three additional times; all three passed with exactly one successful redemption per race.
- `saas-identity`: `cargo test --locked` — 54 passed, 0 failed, 15 ignored. The ignored PostgreSQL regressions were run separately as listed above.
- `saas-identity`: `cargo fmt --all -- --check`, `cargo check --locked` and `cargo clippy --locked --all-targets` passed. Clippy reported three pre-existing warnings in `admin_operations.rs`, `auth_admin.rs` and `auth_consent.rs`; there were no new OAuth/resource-token warnings.
- Both repositories: `git diff --check` passed; Git printed line-ending conversion advisories for existing working-tree files.

## Temporary resources and cleanup

- One temporary WSL PostgreSQL 18.6 cluster was created at `/tmp/identity-phase09-final-20261028`, listening on `127.0.0.1:55490`. Cluster identifier: `7701472062135853480`.
- Each Phase 8/9 HTTP test created a marked, uniquely named database and removed it in its `finish` cleanup. Before stopping the cluster, post-test queries found zero `identity_phase08_oauth_*` or `identity_phase09_oauth_*` databases and zero Phase 9 roles.
- The cluster used 56,528 KiB immediately before removal. It was stopped with `pg_ctl`; its resolved path and PostgreSQL major version were checked; the task directory was removed. Afterwards port 55490 had no response and `df -Pk /tmp` returned to 0 used blocks.
- The existing PostgreSQL service on port 5432 remained accepting connections and was not used for test data or modified. No task-specific temporary RSA-key directories remained in Windows `%TEMP%`.
- WSL Docker context was `default`; `docker system df` before and after showed 0 images, containers, local volumes and build cache. No Docker container, image, network, volume or cache was created.
- No WSL VHDX compaction was attempted. Removal of `/tmp` data does not shrink the Windows virtual disk file.

## Risks and remaining boundaries

- The authenticated MACHINE/SERVICE introspection client path is available, but issuing MACHINE `client_credentials` tokens remains Phase 11. Resource APIs and BFF consumers still require their later phases (12–18) for integration and business-context checks.
- PLATFORM_CONTEXT, business membership/data-range authorization and derived private resource proofs remain later phases; identity does not infer them from a valid opaque token.
- Refresh issuance, rotation and family revocation are implemented in Phase 10; legacy `/oidc/*` handlers remain until Phase 24 retirement.
- Q3 reviewer identities, Q8 legal/contract facts, Q9 operational measurements and Q10 deployment facts remain production-release gates. This phase is local verification, not deployment or external client certification.
