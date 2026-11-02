# Phase 02 — Unified empty-database model and role isolation

Date: 2026-10-28  
Status: COMPLETED

## Inputs and baseline

- Phase 1 contract/callsite inventory; platform fresh-install and identity deployment docs; current `0001`, role provisioning/verification SQL, identity readiness, and database ACL test fixture.
- Repositories: `saas-plateform` HEAD `34105757b040165daa5477bf6afff9637f617466`; `saas-identity` HEAD `7025a0a39cff5577bc3aabf239aaae8df529714e`. The platform docs and identity README had pre-existing worktree changes; these were retained and updated only where needed for this phase.
- PostgreSQL server version `180006` (18.6), above the repository's 17+ baseline. A new randomly named empty test DB and role set were created by the test fixture. A temporary LOGIN superuser existed only to let the fixture create/drop its own database and owner/runtime roles; it was removed after each run.

## Deliverables

- Canonical `0001` now creates the target `auth_applications`, `auth_resources`, `auth_scopes`, `auth_clients`, credential/permission, consent group/consent, subject sector/link, authorization request/code, grant/scope, opaque access-token/scope and refresh-family/token structures. The projection stores platform application/partner IDs as opaque values with no identity-to-platform FK; a composite FK prevents PUBLIC clients from owning credentials.
- `provision.sql` assigns every new `auth_*` table to `_identity_owner` and grants required DML only to `_identity_runtime`. `verify.sql` checks those owners, identity DML and absence of platform table/column privileges. `saas-identity` readiness performs the same effective ACL checks while retaining explicitly temporary checks for current old consumers.
- Added real PostgreSQL test `phase02_auth_model_constraints_and_runtime_boundary`: canonical init and repeated version-1 verification, all target entity inserts/references, unique/version/FK rejections, separate LOGIN-role allow/deny, no DDL, and the deployment verifier.
- [Phase 24 retirement list](../phase-24-retirement-list.md) records current staging tables, routes, helpers, ACL/readiness references and removal gates.
- Updated fresh-install, identity role-deployment and platform doc-index guidance to distinguish target storage from not-yet-migrated protocol consumers.

## Self-check

| Acceptance criterion | Result | Notes |
|---|---|---|
| Dedicated empty database initializes; repeated version-1 verification does not rewrite history | PASS | Test compares `_sqlx_migrations` version-1 checksum/history before and after a second SQLx migration run. |
| Target entities and key unique/version/reference constraints have real PostgreSQL coverage | PASS | PostgreSQL 18.6 test inserted every target entity and checked duplicate resource key, invalid client version, resource/scope mismatch, duplicate account/group consent and invalid subject account FK. |
| Platform cannot read/write identity secrets; identity cannot read business membership/roles; runtime cannot DDL/change owner | PASS | `provision.sql`/`verify.sql` run on the task DB; test connected using actual separate LOGIN roles, proved client credential access only in identity, denied platform secret/client writes, denied identity memberships/roles and denied DDL. |
| Schema/role verification command, transitional retirement list and failure cleanup are reproducible | PASS | Test fixture owns `identity_phase02_<uuid>` database plus `id02_<prefix>` roles and has awaited cleanup on success/error; all three execution attempts left zero Phase 02 DBs/roles. |

## Verification evidence

- PASS: `cargo check --locked --manifest-path saas-plateform/backend/Cargo.toml`.
- PASS: `cargo check --locked --manifest-path saas-identity/Cargo.toml`.
- PASS: `rustfmt --edition 2021 --check saas-plateform/backend/src/identity_deployment_tests.rs`.
- PASS: `cargo test --locked --manifest-path saas-plateform/backend/Cargo.toml phase02_auth_model_constraints_and_runtime_boundary -- --ignored --nocapture` (1 PostgreSQL integration test; SQLx init, real LOGIN roles and read-only deployment verifier).
- `cargo fmt --manifest-path saas-plateform/backend/Cargo.toml -- --check` reports pre-existing formatting differences in `src/identity_ui_tests.rs` and `src/seed.rs`; the changed test file was formatted and checked independently. Unrelated files were not reformatted.
- Docker attempt: `docker pull postgres:17` timed out at Docker Hub before downloading an image. No container, image, volume, network or build cache was created. The already-running WSL PostgreSQL service was reused instead; its configuration and existing databases were not changed.

## Resources and cleanup

- Temporary test resources: one isolated database; six task-scoped migration/owner/runtime roles; one temporary superuser login used by the host test process. The Rust fixture removed the database and its six roles; the PowerShell `finally` path removed any exact-name residue and the temporary test login.
- Before/after database-name inventories matched. Final queries found zero `identity_phase02_*` databases, zero `id02_*` roles and zero temporary admin roles.
- Containers created/reused: 0. Docker `system df` before and after: 0 images, 0 containers, 0 local volumes and 0 build cache. No Docker cache or image remained to remove. No WSL VHDX compaction was attempted.
- No exported logs, bind mounts, source-tree secrets or temporary host files were created.

## Risks / notes

The new target schema is intentionally storage-only. Existing `/oauth/*` and `/oidc/*` consumers still use transitional tables; no live protocol was switched and no dual write was added. `auth_*` tables and old transition structures coexist in the canonical baseline until each consumer is replaced and Phase 24 retires the old paths. Q3/Q8/Q9/Q10 production facts remain unaffected.
