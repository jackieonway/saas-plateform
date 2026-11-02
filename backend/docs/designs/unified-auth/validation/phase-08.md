# Phase 08 — Unified authorization code, OIDC issuance and TOTP

Date: 2026-10-28  
Status: COMPLETED

## Inputs and boundaries

- Phase 4 unified clients, Phase 6 pairwise subject/profile projection, Phase 7 group-version-aware consent, the [implementation contract](../implementation-contract.md), and adopted Q5/Q6 decisions.
- `saas-identity` baseline HEAD: `7025a0a39cff5577bc3aabf239aaae8df529714e`; `saas-plateform` baseline HEAD: `34105757b040165daa5477bf6afff9637f617466`.
- Both repositories already contained uncommitted Phase 1–7 changes. Those changes were preserved; no branch reset or checkout replacement was performed.
- This phase changed only the local source, canonical fresh-install schema, deployment configuration/readiness checks, tests and current docs. No production service, gateway, existing business database, signing key or remote environment was changed.

## Delivered changes

- Added the canonical `/oauth/*` personnel authorization-code path over `auth_*`: exact registered redirect matching, state/nonce validation, S256 PKCE, public and confidential client authentication, supported `prompt`/`max_age`, one-time code redemption, signed ID Tokens and UserInfo.
- Connected code issuance and redemption to the Phase 6 pairwise subject/profile projection and Phase 7 consent generation and group-version fences. A group membership or requested scope expansion requires the user to confirm again.
- Discovery now advertises the implemented `/oauth/*` endpoints and authorization-code, pairwise, S256, `openid` and `profile` capabilities only. It does not declare refresh, introspection, revoke or logout endpoints. Transitional `/oidc/*` routes remain registered for the Phase 24 retirement inventory and are not advertised as the canonical protocol.
- Added encrypted TOTP enrollment, time-step replay prevention, rate-limited attempts, ten digest-stored one-use recovery codes, step-up authentication and security-event outbox records. Production startup requires `SSO_TOTP_ENCRYPTION_KEY` as 64 hexadecimal characters (32 bytes).
- Added the private `AUTH_SECURITY` MFA-recovery endpoint. It accepts two different platform-signed approval statements with recent primary and TOTP authentication, clears the target factor/recovery codes, advances the account session version, revokes old sessions and records an outbox event in one transaction. Platform-role enforcement on sensitive administration paths remains Phase 19; outbox delivery remains Phase 20.
- Updated canonical `0001`, identity role provisioning/readiness checks, deployment examples and the identity security page. Public self-registration remains absent; account creation continues through the existing permission-protected platform account-management path.

Phase 8 implementation files include `saas-identity/src/oidc/oauth.rs`, `saas-identity/src/oidc/protocol.rs`, `saas-identity/src/sso/mfa.rs`, `saas-identity/src/sso/mfa_recovery.rs`, the SSO MFA routes and security page, the canonical platform `0001` migration, identity deployment provisioning/readiness files, and `saas-identity/src/util/audit_tests/auth_protocol.rs`. Other dirty files include earlier Phase 1–7 work and were not attributed solely to this phase.

## Phase boundary adjustment

Q5's one-time, 24-hour invitation remains a required policy. Repository review found protected platform account creation but no recipient-bound invitation token lifecycle. Issuance and role authorization are now explicitly assigned to Phase 19; delivery through a verified channel is assigned to Phase 20. Phase 8 keeps public self-registration disabled and does not claim invitation-link onboarding is available. This preserves Q5 while placing the implementation beside its required administration and notification controls.

## Acceptance self-check

| Acceptance criterion | Result | Evidence |
|---|---|---|
| ACCOUNT_ONLY account without business membership can complete standard login without an organization lookup | PASS | PostgreSQL HTTP fixture has an account and SSO session with no platform membership schema; it completed authorize, code exchange, ID Token and UserInfo. |
| Invalid callback/verifier/nonce/issuer, code replay and concurrent redemption are rejected | PASS | Unregistered callback and wrong verifier returned errors; the client-verification model rejected wrong expected issuer and nonce; a redeemed code was rejected on replay; concurrent redemption produced exactly one success and one `invalid_grant`. |
| PUBLIC has no secret; CONFIDENTIAL authenticates; ID Token and UserInfo subjects agree | PASS | PostgreSQL HTTP regression verified wrong confidential secret rejection, valid basic authentication, public client with zero credential rows, and matching pairwise `sub` and profile values. |
| `prompt=login/none/consent`, `max_age`, sufficient-consent reuse and discovery declarations match implemented behavior | PASS | HTTP regression exercised each prompt, `max_age=0`, group-version/scope reconfirmation, default reuse and canonical discovery metadata. Unsupported refresh, introspection, revoke and logout metadata was absent. |
| TOTP encryption, replay/guess protection, recovery-code one-use and two-person administrator recovery revoke old credentials | PASS | PostgreSQL regression verified encrypted seed storage, same-step replay denial, rate limiting, one-use recovery code, two distinct signed approvals, cleared old recovery factors, session-version advance, old-session revocation and one outbox record. |

## Verification

- `saas-identity`: `cargo fmt --all -- --check` — passed.
- `saas-identity`: `cargo check --locked` — passed.
- `saas-identity`: `cargo test --locked phase08 -- --ignored --nocapture` — 2 passed, 0 failed. Both regressions used the task-owned temporary PostgreSQL 18.6 cluster described below.
- `saas-identity`: `cargo clippy --locked --all-targets` — passed with three warnings in existing `admin_operations.rs`, `auth_admin.rs` and `auth_consent.rs` code; the new Phase 8 OAuth/MFA modules produced no Clippy warnings.
- `saas-identity`: `node --check static/identity/security.js`, `identity.js` and `applications.js` — passed.
- `saas-identity`: `git diff --check` — passed; Git printed only line-ending conversion advisories for existing working-tree files.
- Canonical platform schema/owner-runtime boundary regression, `cargo test --locked phase02_auth_model_constraints_and_runtime_boundary -- --ignored --nocapture` — 1 passed against the Phase 8 `0001` and role provisioning changes.

## Temporary resources and cleanup

- One temporary WSL PostgreSQL 18.6 cluster was created at `/tmp/identity-phase08-final-mvr7bdk6fgpc8o0a`, listening only on `127.0.0.1:55489`. Cluster identifier: `7701465301648263835`.
- The two HTTP regressions created two `identity_phase08_oauth_*` databases. Each test's `finally` cleanup reported `database_removed=true`; a post-test query found zero Phase 8 databases and zero Phase 8 roles.
- The temporary cluster occupied about 56,516 KiB under `/tmp` immediately before deletion. It was stopped with `pg_ctl`, its resolved path and PostgreSQL major version were checked, and the directory was removed. Afterwards port 55489 had no response, the directory was absent, and `df -Pk /tmp` returned to 0 used blocks from 56,516 blocks before cleanup.
- The existing WSL PostgreSQL service on port 5432 remained accepting connections before and after; it was not used for test data or modified.
- The WSL Docker context was `default`. `docker system df` before and after showed 0 images, containers, local volumes and build cache. No Docker container, image, network, volume or build cache was created. No task-specific Windows temporary RSA-key directory remained.
- No WSL VHDX compaction was attempted; deleting temporary files does not itself shrink the Windows virtual disk.

## Risks and remaining boundaries

- Refresh token, resource introspection/revoke, machine grants and business-context selection remain later phases. The token issued here is a short-lived opaque UserInfo token, not a general resource token.
- The existing `/oidc/*` compatibility routes are still present until Phase 24. They are not evidence that two protocol paths have been retired.
- The `AUTH_SECURITY` service credential verifies two signed recent approvals, but platform administrator/approver role checks across sensitive administration writes are deferred to Phase 19. Notification delivery/retry is deferred to Phase 20.
- Account creation remains behind the existing authorized platform account-management path, and public self-registration is absent. The Q5 recipient-bound, single-use 24-hour invitation is deferred to Phase 19 issuance/authorization and Phase 20 verified-channel delivery; it remains unavailable until those phases pass.
- Q3 reviewer identities, Q8 legal/contract facts, Q9 operational measurements and Q10 deployment facts remain release gates. The local verification is not external client certification or a production rollout.
