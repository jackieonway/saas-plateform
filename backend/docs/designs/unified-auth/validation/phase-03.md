# Phase 03 — Application and partner authentication projection

Date: 2026-10-28  
Status: COMPLETED

## Inputs and baseline

- Phase 1 implementation contract/callsite inventory and Phase 2 canonical `auth_*` model/role boundary.
- Repositories: `saas-plateform` HEAD `34105757b040165daa5477bf6afff9637f617466`; `saas-identity` HEAD `7025a0a39cff5577bc3aabf239aaae8df529714e`. The platform worktree already contained Phase 2, docs-index and user-provided untracked design/ledger inputs; the identity worktree already contained its Phase 2 README/readiness changes. Those changes were preserved.
- No production identity endpoint, signing key, gateway, database or remote deployment was changed.

## Deliverables

- Added the identity private `/internal/identity/auth/applications/{id}` read/sync API. It accepts only canonical UUIDv7 IDs, exact versioned application fields, registered active scopes and `APPLICATION_PROJECTION` credentials. It never accepts member, tenant, organization or role data; an unreviewed third-party projection cannot become `APPROVED`.
- Added the platform `/api/identity/applications/{id}/projection` read/write adapter. It reads the existing platform application under `system:application`, projects first-party applications only, translates `ENABLED`/`DISABLED` into identity `APPROVED`/`SUSPENDED`, and uses the existing platform intent/audit boundary and the original operation ID for result recovery.
- Separated projection service ID/secret from account, customer-authority and OIDC-admin credentials in runtime configuration and Compose examples. Added the `APPLICATION_PROJECTION` purpose and fixed private route to canonical migration constraints and identity provisioning/readiness checks.
- Added identity real-PostgreSQL route regression `phase03_application_projection_is_service_scoped_versioned_and_recoverable`; added strict platform request/response validation and operation-target tests. Updated [Phase 24 retirement list](../phase-24-retirement-list.md) because the current receipt/intent tables are temporarily reused.

## Self-check

| Acceptance criterion | Result | Notes |
|---|---|---|
| Create, update/disable and stale-version conflict have reproducible contract tests | PASS | The identity PostgreSQL test creates version 1, propagates suspension to version 2 and rejects a stale expected version without changing projection, receipt or audit counts. |
| Unknown outcome is queried using the original operation and does not repeat the mutation | PASS | The test queries the completed original operation, replays the exact same operation and verifies there is still one receipt/audit for that operation. Receipt reconstruction is bound to service ID, secret generation, method/path and digest. |
| Wrong service purpose, application/path mismatch and old version are rejected | PASS | `OIDC_ADMIN` cannot invoke the projection route; body application ID must match its fixed URL; stale expected version returns conflict. Platform tests also reject payload authority fields and mismatched IDs. |
| Partner ownership cannot grant business-tenant administration | PASS | Third-party data is constrained to a UUIDv7 partner reference and `PENDING_REVIEW`; the projection contract contains no business membership/tenant/role fields, and the real route test confirms it cannot be approved through this service purpose. |

## Verification evidence

- PASS: `cargo check --locked` in `saas-plateform/backend`.
- PASS: `cargo test --locked oidc::remote_admin::tests:: -- --nocapture` in `saas-plateform/backend` (9 remote-admin tests, including strict projection payload and `/internal/identity/auth/operations/{id}` routing).
- PASS: `rustfmt --check --edition 2021 src/config.rs src/oidc/clients.rs src/oidc/remote_admin.rs src/oidc/remote_admin_tests.rs src/oidc/remote_admin_pg_tests.rs` in `saas-plateform/backend`.
- PASS: `cargo test --locked phase03_application_projection_is_service_scoped_versioned_and_recoverable -- --ignored --nocapture` in `saas-identity` (real PostgreSQL 18.6 route/receipt/audit regression; reported and completed in the preceding execution turn).
- PASS: `git diff --check`; Git emitted only existing CRLF-to-LF advisory notices for modified Windows files.
- The full platform `cargo fmt --check` is not used as a gate because Phase 2 recorded unrelated pre-existing differences in `src/identity_ui_tests.rs` and `src/seed.rs`; all Phase 3 Rust files were checked directly.

## Resources and cleanup

- Reused the existing WSL PostgreSQL 18.6 service for the isolated Phase 03 test database; the test database was under `identity_phase03_projection_*` and was removed by the owned fixture/finally cleanup. The preceding test run reported no remaining task database or temporary administrator role; no existing application database was selected or modified.
- Containers created/reused: 0. Docker was not used by Phase 3, so there were no containers, volumes, networks, images, logs or build cache to clean. No Windows WSL virtual disk compaction was attempted.
- No secrets, tokens, credentials or temporary test files were added to the repository or report.

## Risks / notes

The platform adapter currently projects existing platform applications as first-party. The private identity protocol supports third-party projections only in `PENDING_REVIEW`; the Q3 review account and operational workflow are not bound, so Phase 19 remains responsible for review UI/roles and production partner intake. This phase does not claim third-party production registration or cross-service HTTPS/browser verification. Q3/Q8/Q9/Q10 remain Phase 26 release gates.
