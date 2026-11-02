# Phase 24 Transitional Structure Retirement List

Phase 24 execution: **COMPLETED**, 2026-10-29. The inventory below preserves the
original removal gates. Current source has retired the two legacy registries,
their issuers, schemas and management facades. Current-source compilation,
canonical BFF transaction regressions, final empty-database/LOGIN-role checks,
actual Nginx routing and independent review passed. Historical failures and
separately executed test groups retain their own inputs and results. The next
whole-version/browser acceptance belongs to Phase 25.

Created in Phase 2 because the canonical fresh-install `0001` now includes the target `auth_*` model while current source consumers still use the prior OAuth/OIDC structures. This list records temporary compatibility structure only; it authorizes no dual writes, data migration or runtime fallback. Remove each item only after its listed consumer phase has passed and produced current evidence.

## Database structures

| Transitional object | Current owner / consumer | Required replacement evidence before removal |
|---|---|---|
| Platform `oauth_clients` and client-secret/status/access-policy columns | Platform admin API/UI and legacy platform issuer; `saas-plateform/backend/src/routes/oauth_admin.rs`, `src/routes/oauth2.rs` | Unified client CRUD/credentials in identity and single management page: Phases 3–5; all clients use unified identity registry: Phases 8–11; no remaining resource/client consumers: Phases 13–21 |
| Platform `oauth_tokens`, `oauth_authorization_codes`, `oauth_authorization_requests`, `oauth_device_codes` and their indexes/constraints | Platform `/oauth/*` issuer, BFF-derived legacy token contracts, OAuth playground, stores/work-orders introspection: `saas-plateform/backend/src/routes/oauth2.rs`, `saas-examples/oauth-playground`, resource authorization clients | Identity code/token/revoke/introspection and machine flows: Phases 8–11; BFF/resource/example replacements: Phases 13–18 and 21 |
| Identity `oidc_clients`, `oidc_consents`, `oidc_authorization_requests`, `oidc_authorization_codes`, `oidc_grants`, `oidc_access_tokens`, `oidc_refresh_tokens`, `oidc_signing_keys`, `oidc_provider_settings` | Identity `/oidc/*`, legacy `/api/sso/applications` handlers and existing BFFs. The static account-authorizations page now uses the Phase 7 `auth_*` consent API. | Pairwise subject/consent/user grant/token/key lifecycle and BFF replacements: Phases 6–14, 20, 23; remove legacy account-authorization handlers after the canonical route is the only consumer in Phase 24 |
| Identity `oidc_context_clients`, `oidc_context_interactions`, `oidc_account_logout_receipts` | Identity context-selection/private result and current logout flows | Unified context-interaction protocol, both BFFs, one-time selection/recovery and outbox logout: Phases 12–15, 18, 20 |
| Legacy protocol helper functions and triggers that bind BFF/customer proofs to `oidc_*` (`bff_oidc_proof_active`, `customer_portal_account_source_active`, `identity_lock_oidc_source`, `oidc_console_source_active`, `revoke_oidc_*`, context-generation/immutability guards) | Platform SQL functions and identity readiness/proof checks | Replacement SQL/proof and resource contract is live and every BFF/resource regression confirms the new source: Phases 13–18, 20; no callsite remains |
| Transitional privilege/readiness entries for old OAuth/OIDC tables in `ops/identity-deployment/{provision,verify}.sql` and `saas-identity/src/readiness.rs` | Current runtime compatibility until consumer migration | Remove old-table ACL grants, helper-function grants and checks together with the last corresponding runtime query; new `auth_*` owner/runtime boundary must remain enforced: Phase 24 |

## Transitional command receipts shared with application projection

Phases 3–4 deliberately reuse `identity_oidc_admin_receipts` and `platform_oidc_command_intents` for the versioned application projection and unified client/credential administration commands. Their constraints accept the fixed `/internal/identity/auth/applications/{uuid-v7}` projection target and `/internal/identity/auth/clients` CRUD/credential targets; service purposes are separated as `APPLICATION_PROJECTION` and `AUTH_ADMIN`. These are transitional names, not a second protocol authority. Phase 24 must generalize or replace both stores with purpose-neutral names, remove obsolete OIDC-only route/ACL/check references, and preserve the single idempotent operation/recovery contract.

Phase 19 adds `AUTH_REVIEW` and `AUTH_INVITATION` commands and fixed receipt paths to the same operation protocol. Preserve their fine-permission, original-session, approved-snapshot and digest fences when renaming the stores. Invitation key-reuse checks currently include transitional `oidc_clients` alongside current service/client credentials; remove only that old-table check with its final table retirement, retaining TOTP/service/canonical credential separation. Legacy client PATCH requires scopes permission until removed; the new AUTH client DTO alone supports a strictly verified disable-only permission.

## Routes, configuration and documentation

| Transitional path or configuration | Consumer | Removal gate |
|---|---|---|
| Platform public `/oauth/*` and `/api/oauth/clients` | Existing platform issuer and OAuth admin page | Identity single issuer plus Phase 5 UI and Phases 8–11 protocol evidence; remove in Phase 24 |
| Platform `/api/oidc/clients`, `/api/oidc/keys/*`, `/api/oidc/operations/*` | Identity OIDC admin proxy/UI | Unified platform management command/projection client from Phases 3–5 and 23; remove duplicate management facade in Phase 24 |
| Identity public `/oidc/*` routes and static `/oidc/interaction/*` client calls | Identity protocol UI; platform and portal BFFs | Both BFFs, discovery, public third-party sample and back-channel flow consume only the agreed issuer/paths: Phases 8, 12–14, 20–21 |
| Platform BFF `GlobalLogout` path `/oidc/account/logout` and the explicit legacy-authority adapter in `src/bff_tests.rs` | Global logout remains a Phase 20 replacement; test-only endpoint/DTO and source-function adapters preserve the prior BFF transaction/concurrency cases during Phase 13 | Replace global logout with the canonical Phase 20 contract. Remove the legacy fixture authority and its task-database SQL overrides in Phase 24 after equivalent canonical regressions exist; fixture results do not count as real identity/HTTPS interoperability evidence |
| Stores `policy::validate_intro`, `client::{introspect,context}` and old raw HTTP response fixtures, all restricted to `cfg(test)` | Phase 16 production uses canonical-only `resource-authorization/1` and Basic/Form private `resource-directory/1`; these adapters retain prior body/redirect/error-limit tests only | Remove legacy test adapters or express their negative transport cases through current resource contracts in Phase 24; keep the Phase 16 canonical principal/scope/STORE/source-roll-back regressions |
| Legacy Identity `/api/sso/applications` list/revoke/delete handlers | Old static-page consent API backed by `oidc_consents`; the current static page has moved to `/api/sso/consents` backed by `auth_consents` | Remove old handlers and OIDC consent tables only after Phase 24 confirms there are no legacy callers |
| Identity private `/internal/identity/auth/consent-groups*` endpoints and platform `/api/identity/*/consent-groups` proxy | Phase 7 explicit group creation/member registration and group-version advancement | Keep as the sole consent-group management path; remove transitional receipts under the Phase 24 receipt-store consolidation gate |
| Old issuer/path environment keys and gateway rewrites | Platform/portal deployment, resource APIs and OAuth playground | All current consumers/config examples point to one explicit issuer; public gateway denies all internal/private routes: Phase 24 |
| Transitional customer profile test registration in platform `oauth_clients` | Phase 15 production `customer_portal/store.rs` authenticates through `platform_resource_services` with purpose `CUSTOMER_PROFILE`; historical Phase 14 and protocol fixtures still retain old registration data, while the CUSTOMER RP uses canonical `auth_clients` | Phase 24 removes old fixture registration data after the Phase 15 service registry and portal profile/recovery regressions pass. Production no longer reads the old registration/secret lookup. It is never an alternate OAuth issuer or customer login fallback. |
| Docs describing old clients as current authority | Workspace, platform, identity, portal and example docs | Search all eight repositories and update current doc indexes from verified source after retirement: Phase 24 |

## Required Phase 24 close-out

Phase 14 removes the production same-process customer account-proof adapter: when remote identity is not configured, the production capability returns unavailable. The historical adapter in `src/oidc/customer_portal.rs` is compiled only under `cfg(test)` and the old customer-resource suite explicitly installs `src/customer_portal_tests/legacy_source_projection.sql` in its owned fixture database. Remove these test adapters with the remaining `oidc_*` fixtures in Phase 24; they are not canonical protocol or HTTPS acceptance evidence. Current canonical customer source/role regression is `identity_deployment_tests::phase14`; the main Phase 14 report must separately record the identity/platform/portal HTTPS verification.

- Re-run repository-wide source/config/test/docs searches for `oauth_clients`, `oidc_clients`, old `/oidc/*` and platform `/oauth/*` issuance references; classify history/negative tests separately from live paths.
- Remove old schemas, indexes, constraints, functions, triggers, keys/consents/grants, ACL entries, route mounts, endpoint settings, stale tests/fixtures and current-document instructions in one canonical `0001` review.
- Keep the public `/oauth/*` issuer in identity, but no second platform issuer and no hidden fallback. Keep private BFF/session/context capabilities only when their distinct purpose, audience and verifier are explicit.
- Initialize and verify a new database using only the Phase 24 target baseline; prove owner/runtime/readiness and no old-token, UUID or checksum compatibility path.

## Phase 2 status

At the end of Phase 2 all listed objects remained as transition structures.
Phase 2 added the `auth_*` model and ownership boundary; the retirement changes
below belong to Phase 24 and are not attributed to the earlier phase.

## Phase 15 replacement boundary

Production application capabilities use `platform_app_capabilities`, exact internal issuer/token_use/audience and the original BFF source. Resource backend authentication uses `platform_resource_services`; public canonical opaque tokens use the private identity resource-access-proof contract. These are internal capability/resource registrations, never a second OAuth client registry. Remaining legacy public OAuth handlers and test-only OAuth/source fixtures retire in Phase 24; do not cite them as canonical resource interoperability evidence.

The legacy public platform customer resource adapter (`customer_identity`, `/api/customer/context`, `/api/customer/profile`, `/api/customer/applications`) is closed in production in Phase 15 and compiled only for historical tests. Remove that test adapter and its old OAuth rows in Phase 24. The portal's own public `/api/profile` and `customer-profile/1` presentation contract remain current; its backend calls `/internal/customer-portal/profile` authenticated by the independent CUSTOMER_PROFILE service registration. The old `entryPath` string in the unchanged presentation DTO is metadata and is not a live platform HTTP route.

## Phase 24 current disposition

The original [R01–R16 preparation inventory](validation/phase-24-preparation.md)
maps to these current changes. Final platform test-only corrections passed their
complete affected groups with an independently verified unchanged-case boundary.

| Original ID | Current disposition | Verification location |
|---|---|---|
| R01 | Platform public OAuth issuer and its route mounts removed | Router retirement matrix; actual Nginx old-route rejection |
| R02 | Old client/token/device management removed; current platform device sessions retained | Unified management frontend; current session/audit PG groups |
| R03 | Platform same-process SSO/OIDC provider removed; retired provider options rejected | Config negative matrix and route absence across deployment flags |
| R04 | Identity old public/private protocol and context aliases removed | Identity router regressions; canonical protocol PG; task gateway alias rejection |
| R05 | Old SSO applications consent API removed; account page uses canonical consents | Identity consent groups and current static application page |
| R06 | Sole canonical signing/settings stores retained under neutral names and KEY_ADMIN | Final identity key lifecycle/LOGIN-role PG and catalog ACL checks |
| R07 | Legacy OAuth/OIDC DDL, FK, indexes, triggers and old helper grants removed | Final empty-database 103 checks; 105 tables including SQLx history |
| R08 | Only AUTH registry and exact canonical allowlists remain | Source/context PG and strict registration DDL |
| R09 | Old token purge/count/revocation branches removed; derived capability revocation preserved | Actual member deletion/audit rollback and application-policy PG |
| R10 | Shared command receipt/intent engine preserved with neutral stores and fixed current paths | Identity commands, platform remote management PG, append-only ACL |
| R11 | Obsolete frontend routes/DTOs and client association CLI paths removed | Frontend/type/build/component evidence and Linux CLI 57 |
| R12 | Old provider/SQL overrides retired; canonical BFF/customer/source transaction fixtures retained | Nine BFF transaction groups plus customer/source/lock PG; final affected groups passed |
| R13 | Resource test adapters use current combined proof/directory contracts | Stores 83, work-orders 77, private directory/resource and notification PG |
| R14 | Old playground tracked implementation/CI/defaults removed; standard RP retained | RP 46 and example/SDK provenance; pre-existing ignored artifacts separately disclosed |
| R15 | Current DDL/readiness/recovery inventory synchronized; old generic restore/RESET closed before I/O | Empty DB, identity final, canonical recovery 119 and R15 7+1 component evidence |
| R16 | Current guides/config examples/public gateway reject retired and private paths | 141 guide links; actual Nginx 14 top-level checks and strict Node TLS controls |

Evidence and input boundaries are consolidated in the [Phase 24 report](validation/phase-24.md).
Reports retain failures and separately executed groups; these rows do not turn
the Phase 25 current multi-service/browser matrix into a completed result.

| Original item | Current disposition and evidence |
|---|---|
| Platform `oauth_clients` and OAuth grant/carrier tables | Removed from current `0001`, provision and verify. Platform issuer/admin/device route modules and their mounts are removed. Unified client management uses identity `auth_clients` through the authenticated `/api/identity` facade. |
| Identity old client/consent/grant/token/context/logout tables | Removed from `0001`; old public `/oidc/*`, private legacy client management and `/api/sso/applications` handlers are removed. Canonical `/oauth/*`, `/api/sso/consents` and `/internal/identity/auth/*` remain. |
| Signing keys/provider settings | Retained under `auth_signing_keys` and `auth_provider_settings`; these are consumed by the sole canonical issuer. Management requires independent `KEY_ADMIN`, `/internal/identity/auth/keys/*` and original `/key-operations/{id}` lookup. Platform configuration uses `IDENTITY_KEY_ADMIN_SERVICE_CLIENT_ID/SECRET`. |
| Shared management receipt and intent stores | Renamed to `identity_auth_command_receipts` and `platform_auth_command_intents`; the original operation, digest, method/path, source, permission and assurance fences remain. The narrow definer is `identity_auth_command_intent_active`. |
| Source functions already using canonical state | Retained and renamed to `bff_auth_proof_active`, `identity_lock_auth_source` and `auth_console_source_active`. No old-table or alternate-registry branch remains. |
| Context registrations and service allowlists | `client_registry` is fixed to `AUTH`; application and application policy version are required. `allowed_oidc_clients` is removed; exact `allowed_auth_clients/resources` remain. |
| Internal application capabilities | `platform_app_capabilities`, independent `token_use=app_session`, audience/source checks and resource-service credentials remain. Issue path is `/api/apps/session-token`; this is not an OAuth authorization server. |
| Account revocation bridge | Old `oauth_token_count` storage and token-table cleanup are removed. Platform session revocation counts remain. The existing account-command `revokedTokens=0` field does not claim a count of canonical grants; account generation independently invalidates them. |
| UI, CLI and examples | Old tokens/device/consent pages, client admission branch and platform issuer proxies are removed. CLI maintains business applications and menus. `oauth-playground` tracked files are retired; `oidc-rp` is the standard-client example. |
| Historical test adapters | Business/member/customer/resource fixtures use canonical records or explicit proof-only transport fakes. BFF server transaction/concurrency migration is complete and its current groups passed real PG; SQL/ACL or browser unit tests were not substituted for these cases. |
| Recovery declarations | Current typed inventory uses the new names, 86 shared-library tables plus 5 portal tables, and 950 fields. Actual empty-PG comparison validates all declared shared tables against the catalog; old inventory/schema checksums are rejected. |

Final current-schema evidence: [empty database](validation/phase-24-empty-database-final.json)
records 103 passing checks, including real migration/repeat, provision/verify,
two LOGIN runtimes, cross-owner/DDL/owner-role rejection and typed catalog
comparison. Its database and six roles are absent after `finally`. It includes
the capability membership cascade fix. [Platform grouped verification](validation/phase-24-platform-final.json)
covers 432 unique current cases through 50 current affected cases and 382
independently proven unchanged cases; this is not a single 432-case run.
[Identity final verification](validation/phase-24-identity-final.json) includes all
141 library cases and the separate main case. These results do not replace
Phase 25's whole-version service/browser acceptance.

The old playground's pre-existing ignored `frontend/dist` and
`frontend/node_modules` remain, logically 48,198,145 bytes. Automatic approval
review rejected their exact-path recursive cleanup with `blocked by policy`.
They have no current source, build, route or CI reference. This is an explicit
unremoved existing artifact, not a claim of complete deletion or a newly created
validation resource.
