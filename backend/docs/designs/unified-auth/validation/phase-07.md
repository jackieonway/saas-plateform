# Phase 07 — Consent groups and user authorization management

Date: 2026-10-28  
Status: COMPLETED

## Inputs and boundaries

- Phase 4 unified-client API and Phase 6 pairwise subject/profile reports; adopted Q1 explicit same-application sharing and Q8 deletion defaults.
- Identity repository HEAD baseline: `7025a0a39cff5577bc3aabf239aaae8df529714e`; platform repository HEAD baseline: `34105757b040165daa5477bf6afff9637f617466`. Both repositories already contained Phase 1–6 changes and other task work; those edits were preserved.
- Only the canonical `auth_*` model is used by the new consent service and page endpoints. The existing `/api/sso/applications` legacy facade remains a Phase 24 retirement item.
- No production service, gateway, database or secret was changed. Public protocol issuance and back-channel delivery remain in their later phases.

## Delivered changes

- Canonical `0001` now makes consent-group boundaries explicit for application, environment, USER subject and account/business context. It adds a display name, consent `group_version`, `display_deleted_at`, a per-client revocation outbox and immutable user command receipts. Composite foreign keys prevent cross-application, cross-environment or cross-context client membership.
- Creating or explicitly adding a client to a group advances the group version. Each consent records the group version last confirmed by that user. An earlier consent becomes insufficient after membership changes, so an administrator cannot silently authorize a new client on users' behalf.
- The identity `auth_consent` service validates the live SSO account/session, approved application and current approved scope names, enabled USER client, active explicit group, exact context and client resource/scope permissions. It reuses a sufficient active consent, requires confirmation for added scopes, and only acknowledges an updated group version after explicit approval. Approval is audited in the same transaction; re-consent after revocation uses the generation advanced by revocation.
- `AUTH_ADMIN` private APIs create groups, list their members and explicitly add matching clients. The platform BFF forwards those operations through its existing authenticated operation-intent and receipt path. The unified client page now offers group creation, member management and matching group selection on client registration.
- The account authorization page now reads and mutates the canonical consent tables. Revocation locks the account, application, group clients in UUID order, group, consent, pending requests/codes, grants, refresh families and tokens. It advances the consent generation, invalidates pending and issued credentials, writes one outbox event per group client, records audit and operation receipt, then commits as one transaction.
- Deleting a revoked row sets `display_deleted_at`, removes the displayed scope rows and retains the consent generation/version fence. Reauthorization advances the display-row version; stale delete commands conflict. Logout and context changes do not delete consent.

## Files changed

- Identity service: `src/auth_consent.rs`, `src/auth_admin.rs`, `src/admin_operations.rs`, `src/lib.rs`, `src/readiness.rs`, `src/sso/routes.rs`, new `src/sso/routes/consents.rs`, identity authorization HTML/JS, and database test fixtures/regressions.
- Platform backend: canonical `backend/migrations/0001_init.sql`, identity role `provision.sql` / `verify.sql`, `src/identity_deployment_tests.rs`, private admin forwarding routes and fixed-target/response-contract tests.
- Platform frontend: identity client API/types and `UnifiedClientsPage.tsx`; the real Chromium registry fixture now covers group creation and member version advancement.
- Contracts and indexes: [implementation contract](../implementation-contract.md), [platform docs index](../../../README.md), and the current task ledger.

## Acceptance self-check

| Acceptance criterion | Result | Evidence |
|---|---|---|
| First approval, sufficient reuse, scope expansion confirmation and group isolation | PASS | PostgreSQL regression checks sufficient openid reuse; adding profile is insufficient until approved; same-group sibling reuse works; ungrouped and other-application clients cannot inherit consent. |
| Explicit membership growth cannot silently reuse an old group consent | PASS | PostgreSQL regression adds an existing USER client through `AUTH_ADMIN`, observes the group version advance and consent become insufficient, then verifies an explicit user confirmation acknowledges that version. |
| Revoke all applicable group clients while leaving other accounts/applications unchanged | PASS | PostgreSQL route test revokes two group members, denies pending requests, consumes authorization codes, revokes grants/access/refresh tokens and creates two outbox events; a second account and another application remain active. |
| Active/other-account delete is denied; display delete cannot restore grants; stale deletion after reauthorization conflicts | PASS | HTTP regression checks active delete conflict, cross-account 404, hidden revoked display row, removed scopes, retained revoked grant and stale-version conflict after reauthorization. |
| Logout retains consent; sensitive writes require current account source, CSRF/origin, account lock and audit | PASS | HTTP regressions check logout preservation, missing CSRF and foreign-origin rejection, account-scoped session revalidation, and rollback when revoke or approval audit insertion is suppressed. Consent is keyed by account + group and contains no organization ID; organization switching does not call a consent mutation path. |

## Verification

- `saas-identity`: `cargo fmt`; `cargo check --locked`; `cargo test --locked --no-run`; `cargo test --locked phase07 -- --ignored --nocapture` — 3 PostgreSQL tests passed. The tests cover group API versioning/context checks, scope/group consent coverage and the revoke/delete/reauthorize/logout lifecycle.
- `saas-plateform/backend`: `cargo fmt`; `cargo check --locked`; `cargo test --locked --bin saas-platform-backend phase07_consent_group_admin_port` — 1 fixed-target, strict DTO and receipt-recovery test passed.
- Canonical schema/ACL regression: `cargo test --locked phase02_auth_model_constraints_and_runtime_boundary -- --ignored --nocapture` — 1 test passed against the current canonical `0001`, `provision.sql` and read-only `verify.sql`, including real LOGIN-role allow/deny checks. Its fixture was updated for the new group context columns.
- Platform frontend: `npm test` — 431 passed; `npm run typecheck` passed; ESLint passed for the three changed TypeScript files; `npm run build` and Qiankun delivery check passed.
- Chromium: `npm run test:identity:clients:browser` — 7 scenarios passed, including explicit group creation, user-context scoping, client membership versioning and the “reconfirm existing consent” warning; desktop and 390px page scenarios also passed. This is browser UI evidence with an isolated HTTP fixture, not live backend evidence.
- Build emitted two Rollup notices about removable `@__PURE__` comments in the installed Zod package; the build and delivery checks completed successfully.

## Temporary resources and cleanup

- Reused the already-running WSL PostgreSQL 18.6 service on the existing cluster; no Docker containers were created. PostgreSQL cluster identifier: `7694215878237645672`.
- Phase 7 HTTP regressions created three marked `identity_phase07_consent_*` databases and one temporary `CREATEDB` login. Each test removed its own database and pool; the surrounding `finally` path removed the login and verified zero marked database residue.
- The canonical schema/ACL regression created one `identity_phase02_*` database, six task-scoped owner/runtime roles and one temporary test superuser login. The fixture and finalizer removed all of them; final Phase 02 test database/role residue counts were both zero.
- Browser screenshots/JSON were written only under task-specific ignored `frontend/target/phase07-consent-groups-browser{,-retry}/` directories. The successful 7-scenario report is retained as local evidence; the first run's diagnostic screenshot is also still present. A targeted removal command for those two folders was blocked by the shell policy, so they were not deleted. These files contain fixture IDs and screenshots only, no credentials or service data.
- WSL Docker context was `default`. `docker system df` before and after verification: 0 images, 0 containers, 0 volumes and 0 build cache. No Docker image, container, network, volume or cache was created. No WSL VHDX compaction was attempted.

## Risks and remaining boundaries

- Revocation writes a durable local outbox event only. Delivery, retry scheduling and remote back-channel logout remain Phase 20 work.
- The consent service is ready for the unified authorization protocol, but the public `/oauth/authorize` path consumes it in Phase 8. The current legacy `/oidc/*` flow is not treated as evidence for the new `auth_*` consent implementation.
- Q8 jurisdiction/contracts, Q3 real reviewer accounts, Q9 operational measurements and Q10 deployment facts remain release gates; this phase did not make legal or production claims.
