# Unified OAuth/OIDC Callsite Inventory

Phase 1 source inventory, read on 2026-10-27. Paths and behavior below describe the current worktree; this is not an implementation or passing-test report. HEADs and complete porcelain worktree status were rechecked after the first formatted summary hid multiline status entries.

## Repository baseline

| Repository | HEAD | Worktree | Auth-related responsibility |
|---|---|---|---|
| `saas-identity` | `7025a0a39cff5577bc3aabf239aaae8df529714e` | pre-existing modified `README.md`; Phase 2 modifies readiness in `src/readiness.rs` | Account SSO, OIDC RP registry/protocol, grants, keys, private identity operations |
| `saas-plateform` | `34105757b040165daa5477bf6afff9637f617466` | pre-existing modified `backend/docs/README.md`; pre-existing untracked architecture, policy and task-ledger inputs; Phase 1/2 outputs are new untracked files; Phase 2 modifies migration, ACL scripts and PostgreSQL tests | Business OAuth registry/issuer, OIDC admin proxy, management UI, platform BFF/context |
| `saas-customer-portal` | `648e71e082929605c67e4e9f7e19fc9ecdbd89d4` | clean | Independent customer OIDC BFF and business-context consumer |
| `saas-stores` | `01ea4eb30801e89429b4a4f1469cc0792f17b1f9` | clean | Resource API token introspection + platform context/permission checks |
| `saas-work-orders` | `f0c87d7aea301dddb38ac4a3435431660b8b7632` | clean | Resource API token introspection + platform context/permission checks |
| `saas-notification` | `6a16d74be2e079892d2528f68058c9556ffc3b37` | clean | Revalidates platform sessions/context for notification sockets |
| `saas-micro-app-sdk` | `155f5cc04f98509f5eee3e092f716d4d8f5013d8` | clean | Consumes host platform context; no authentication or token issuer |
| `saas-examples` | `f22ee4f0fe601b94a0f38783d3a76ef113ee47b0` | clean | OAuth playground flow consumer and independent demo APIs |

Workspace Rust baseline is 1.97.0 and Node/npm baseline 24.22.1/11.16.0. Each repository has its own Git history; there is no top-level Cargo workspace.

## Current registration and issuance authority split

| Capability / source | Current implementation | Target replacement / responsible phase |
|---|---|---|
| Identity-side login RP records | `saas-identity/src/oidc/clients.rs` reads/writes `oidc_clients`; shared physical schema is in `saas-plateform/backend/migrations/0001_init.sql` (`oidc_clients`, `oidc_consents`) | Identity canonical `auth_clients` and related model: Phases 2, 4, 6–10; remove old path in 24 |
| Identity OIDC protocol | `saas-identity/src/oidc/protocol.rs`, `src/oidc/mod.rs`, `src/oidc/store.rs`, `src/oidc/tokens.rs`, `src/oidc/context.rs`; public paths currently `/oidc/*` | One `/oauth/*` issuer/route set in identity: Phases 8–12, 20–23; route retirement in 24 |
| Identity user application approvals | `saas-identity/src/oidc/consents.rs`, `src/sso/routes/applications.rs`; per-client `oidc_consents` / grants and revoke/delete routes | Explicit consent groups, resource/scope generations, cross-client revoke/delete: Phases 6–10, 20 |
| Identity private admin/operations | `saas-identity/src/admin_operations.rs`, `src/oidc/clients.rs`, `src/internal.rs`; current `/internal/identity/oidc/*`, operation receipts and service auth | Projection plus unified management command/receipt protocol: Phases 3–4; old command retirement in 24 |
| Business client registry and OAuth | `saas-plateform/backend/src/routes/oauth_admin.rs` serves `/api/oauth/clients`; `src/routes/oauth2.rs` serves `/oauth/authorize`, token, userinfo, introspection, revoke, device and discovery; SQL reads/writes `oauth_clients` | One management facade in platform calling identity and identity-only public issuer: Phases 4, 8–11, 13, 15; remove platform issuance in 24 |
| Identity client admin proxy | `saas-plateform/backend/src/oidc/remote_admin.rs` serves `/api/oidc/clients`, keys and operations and calls identity private API | Merge into one management API and client page: Phases 3–5, 19; keep only explicit platform command boundary |
| Admin user interface | `saas-plateform/frontend/src/pages/system/oauth/ClientsPage.tsx`, `src/api/oauth.ts`; separate OAuth/OIDC API/list paths | One typed client list/form, stable filtering/paging and permissions: Phases 5, 19 |
| Shared schema / ACL | `saas-plateform/backend/migrations/0001_init.sql`; deployment role scripts in `backend/ops/identity-deployment/`; readiness/bootstrap in platform and identity | Canonical clean baseline, owner/runtime ACL, readiness and PostgreSQL proof: Phase 2; remove transitional entities Phase 24 |

## Current relying parties and resource consumers

| Consumer | Source locator / current call | Replacement responsibility |
|---|---|---|
| Identity login page | `saas-identity/static/identity/identity.js` requests `/oidc/interaction/*` | Unified authorize/consent/context UX: Phase 8/12; static page follows route contract |
| Platform employee BFF | `saas-plateform/backend/src/bff/routes.rs`, `bff/protocol.rs`, `bff/transport.rs`, `bff/store.rs`; current issuer discovery/authorize/token/refresh/introspection and context selection | Preserve employee-only BFF session, opaque `sub` plus private internal proof, new issuer/route: Phases 13, 15, 18, 20 |
| Customer portal BFF | `saas-customer-portal/backend/src/routes.rs`, `protocol.rs`, `transport.rs`, `resource.rs`, `store.rs`; hardcoded `/oidc/*` discovery contract, ID Token/UserInfo/refresh/introspection/revoke/context proof | New issuer and opaque-sub/internal-proof adapter; retain customer DB/session boundary: Phases 14, 15, 18, 20 |
| Stores API | `saas-stores/backend/src/authorization/client.rs` calls `/oauth/introspect` and platform context/resource checks; `src/http.rs` mounts stores routes | Enforce new audience/scope and current business source; no DB sharing: Phases 15–16, 18 |
| Work orders API | `saas-work-orders/backend/src/authorization/client.rs` calls `/oauth/introspect` and platform context/resource checks; `src/http.rs` mounts ticket APIs | Enforce new audience/scope and current business source; no DB sharing: Phases 15, 17–18 |
| Notification service | `saas-notification/src/platform.rs` calls `/api/internal/notifications/context-proof`; `src/server.rs` binds/revalidates browser session and member identity generation | Continue session/context source invalidation, disconnect/re-subscribe on identity switch: Phase 18; never use SSO alone for staff delivery |
| Host + micro-app SDK | `saas-plateform/frontend/src/components/micro/MicroAppContainer.tsx` remounts by `contextEpoch`; `saas-micro-app-sdk/src/context-client.mjs`, `context-updates.mjs`, `notification-publisher.mjs` consume host context | Freeze old requests, remount, drop stale response and notification subscription on context change: Phase 18 |
| OAuth playground | `saas-examples/oauth-playground/backend/src/upstream.rs`, `src/http.rs`; uses configured platform origin/client and authorization-code, device, machine, refresh, introspection/revoke sample flows; `README.md` documents proxy/callback | Replace with independent third-party standard OIDC example, no platform DB/internal proof: Phase 21 |

## Phase responsibility map

| Replacement concern | Phase(s) |
|---|---|
| DTO/permissions/scope/error/lock/idempotency/call map | 1 |
| Canonical empty schema and role boundary | 2 |
| Application/partner projection and operation receipts | 3 |
| Unified CRUD, paging, versions and credentials | 4 |
| Single management UI | 5 |
| Pairwise subject/profile projection | 6 |
| Consent groups and revoke/delete | 7 |
| Unified USER authorization code/OIDC issuer | 8 |
| Audience/scope access tokens, introspection and revoke | 9 |
| Refresh family lifecycle | 10 |
| MACHINE credentials | 11 |
| Account-only/platform context interactions | 12 |
| Platform BFF | 13 |
| Customer portal BFF | 14 |
| Fixed resources and internal derived capabilities | 15 |
| Stores API | 16 |
| Work-orders API | 17 |
| Host/SDK/notification context change | 18 |
| Partner review/production enrollment | 19 |
| Logout outbox/back-channel | 20 |
| Independent external example | 21 |
| Runtime metrics/bounds/capacity | 22 |
| Key lifecycle/backup recovery | 23 |
| Old table/route/config retirement | 24 |
| Candidate full fresh install and matrix | 25 |
| Authorized production switch only | 26 |

## Open release facts and gates

| Question | Implementable default | Evidence/owner still required | Gate |
|---|---|---|---|
| Q3 review | Invitation + manual review, applicant cannot self-approve, controlled production enable | Bind real reviewer/publisher accounts and response process (product/operations/security) | No production enable in 19/26 until bound |
| Q8 privacy/retention | Adopted 180-day security audit, 30-day ordinary logs/backups, deletion defaults | Operating jurisdiction, contract and applicable legal basis (operations/compliance) | Must verify before production in 26 |
| Q9 capacity/recovery | Adopted trial/capacity/RPO/RTO targets | Real measured load, backup/WAL drill, on-call names/contact (operations/security) | Must measure/report in 22/23/25 and bind before 26 |
| Q10 deployment | New isolated environment/database/key by default | Exact host/issuer/DB/certificate/callback/backup/rollback target and task-specific authorization (environment owner/user) | Phase 26 remains blocked until exact target and authorization |

Q1/Q2/Q4/Q5/Q6/Q7 decisions are closed in the linked policy document; no preference needs to be re-asked. The current source differences and historical reports are not acceptance evidence for later phases.
