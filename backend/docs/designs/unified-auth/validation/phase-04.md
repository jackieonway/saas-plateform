# Phase 04 — Unified client management and credentials API

Date: 2026-10-28  
Status: COMPLETED

## Inputs and baseline

- Phase 3 application projection, Phase 1 client/credential contract, Phase 2 canonical `auth_*` schema and owner/runtime boundary.
- Repositories: `saas-plateform` HEAD `34105757b040165daa5477bf6afff9637f617466`; `saas-identity` HEAD `7025a0a39cff5577bc3aabf239aaae8df529714e`. The platform worktree already contained earlier phase changes and user-provided design/ledger inputs. The identity worktree already contained its README and readiness changes. Existing edits were retained.
- Only isolated WSL PostgreSQL 18.6 test databases were changed. No existing application database, production endpoint, signing key or gateway was selected.

## Deliverables

- Added identity `AUTH_ADMIN` endpoints for client list/detail/create/update/retire, credential rotation and emergency revocation. They operate on `auth_clients`, `auth_client_permissions`, `auth_consent_group_clients` and `auth_client_credentials`; they do not use the old `oidc_clients` registry.
- Enforced application approval and approved-scope ceilings; active resource/scope checks; immutable application, environment, type/runtime, subject/context, member/grant/auth-method and consent-group bindings; exact HTTPS callback URLs; PUBLIC/CONFIDENTIAL and USER/MACHINE combinations; registered consent groups; optimistic version/policy generations; and bounded, stable `created_at DESC, id DESC` list pagination.
- Confidential credentials are stored only as SHA-256 digests. An operation-bound HMAC gives the original `AUTH_ADMIN` operation its one-time result back without storing the raw secret. Rotation keeps the previous credential valid for at most 24 hours; emergency revocation removes all unexpired generations and advances the policy version. Public clients cannot receive shared secrets.
- Added a platform `system:oauth` proxy with fixed private targets, filter validation, independent `IDENTITY_AUTH_ADMIN_SERVICE_CLIENT_ID/SECRET`, audit accept/ack events and original operation lookup. The proxy requires the existing bearer-backed `CurrentUser` session; internal calls forward Basic authorization and operation metadata, never browser cookies or CSRF headers.
- Added `display_name`, current-credential uniqueness and the `AUTH_ADMIN` service-purpose/path constraints to canonical `0001`. Updated fresh-install, deployment and internal API documentation. The receipt/intent table reuse is tracked in the [Phase 24 retirement list](../phase-24-retirement-list.md).

## Self-check

| Acceptance criterion | Result | Notes |
|---|---|---|
| First/third-party and USER/MACHINE use one client authority; illegal combinations are rejected | PASS | Real PostgreSQL route test creates a first-party USER client, a client under an approved third-party fixture, and a valid MACHINE/SERVICE client. It rejects a PUBLIC machine combination and blocks client creation over an unapproved scope. The third-party fixture represents an already approved projection; Q3 review binding and intake remain Phase 19 work. |
| Filtering and pagination are stable with explicit out-of-range behavior | PASS | The test filters by application, owner, subject mode, environment and partner, verifies two ordered pages and total metadata, and rejects `limit=101`. A valid offset beyond the result set returns an empty page with metadata. |
| Credential overlap and emergency revocation work; replay creates no extra credential or receipt | PASS | First rotation returns a secret whose SHA-256 digest alone is stored; exact operation lookup/replay returns the same secret with one credential/receipt. Second rotation leaves one current and one 24-hour overlap credential. Emergency revocation removes both; a PUBLIC client rotation is rejected. |
| Authorization, operation/version binding and audit failures cannot leave partial writes | PASS | Wrong service purpose, old expected version, and non-approved scope are rejected. Suppressing the identity audit insert makes rotation fail 503 and leaves client version, credentials, receipts and audit counts unchanged; the failed operation lookup is 404. Platform transport tests verify fixed targets, operation headers and no ambient Cookie/CSRF forwarding. |

## Verification evidence

- PASS: `cargo check --locked` in `saas-identity` and `saas-plateform/backend`.
- PASS: `cargo test --locked oidc::remote_admin::tests:: -- --nocapture` in `saas-plateform/backend` (13 tests, including strict request/response contracts, operation route separation and Basic-only forwarding).
- PASS: `cargo test --locked admin_operations::tests:: -- --nocapture` in `saas-identity` (6 receipt/digest/secret-recovery tests).
- PASS: `cargo test --locked phase04_unified_client_api_enforces_policy_paging_and_credential_lifecycle -- --ignored --nocapture` in `saas-identity` (1 real PostgreSQL 18.6 route/transaction/receipt test using a task-owned fixture schema).
- PASS: `cargo test --locked phase02_auth_model_constraints_and_runtime_boundary -- --ignored --nocapture` in `saas-plateform/backend` (1 real PostgreSQL 18.6 canonical `0001`/role/ACL test rerun against the current schema; it also inserts both `APPLICATION_PROJECTION` and `AUTH_ADMIN` service purposes).
- PASS: targeted `rustfmt --check --edition 2021` on the Phase 4 implementation modules, dedicated client lifecycle test, platform proxy tests and identity deployment test; `git diff --check` produced only Windows CRLF/LF advisory notices. The shared `src/util/audit_tests.rs` fixture retains its existing compact formatting and was not reformatted with unrelated tests.
- The database route fixture uses a test implementation of the accepted-platform-intent function and does not prove cross-service HTTPS. The platform test validates request construction without contacting a remote identity endpoint. No browser or deployment path was exercised in this backend API phase.

## Resources and cleanup

- Reused the existing WSL PostgreSQL 18.6 service. Each real-database test created a uniquely named `identity_phase04_clients_*` or `identity_phase02_*` database. Phase 2 additionally created `id02_*` owner/runtime roles. A temporary test superuser login was provisioned for each Windows test process.
- Test fixtures removed their task database and Phase 2 roles. PowerShell `finally` removed each exact temporary administrator role. Final inventory: zero Phase 02/03/04 test databases and zero task/temporary roles. No business database name was selected or changed.
- Containers created/reused: 0. Docker was not used; there were no containers, volumes, networks, images or Docker build caches to remove. No WSL VHDX compaction was attempted.
- Temporary passwords were held in process memory, never printed or written to the repository. No exported logs or source-tree secrets were created.

## Risks / notes

The review UI and reviewer-account binding are not delivered here. Production `AUTH_ADMIN` service-client records and secrets are not populated by the migration or role handoff; they require controlled deployment configuration. The legacy `/internal/identity/oidc/*` and platform `/api/oidc/*` facade remain transitional until their consumers are replaced and Phase 24 retires them. Q3/Q8/Q9/Q10 remain release gates.
