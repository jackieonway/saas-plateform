# Phase 15 — Platform resources and internal capabilities

Date: 2026-10-28  
Status: COMPLETED — proposed ledger acceptance

## Boundary and source

This phase implements the resource and internal-capability boundary from the ledger. It creates no public issuer, OAuth client registry or platform OAuth token endpoint. Canonical opaque access tokens remain identity-owned. Internal platform/application capabilities remain private, with explicit issuer, audience and token_use. SDK 3.0.0, public context 1.0, MicroProps 2.0 and qiankun 2.10.16 are unchanged.

The frozen runtime/schema inputs are included in root-owned snapshot `phase19-resource-r1`, SHA-256 `605ac9ef8aba39fbeed7037287380559bab9a0c075bbc61a49362f71fc1d87f3`. Existing dirty changes and independent repository boundaries were preserved; the ledger is maintained by the root agent.

The [resource capability contract](../phase-15-resource-capability-contract.md) is the interface inventory, private DTO, authentication/checkpoint and independent resource-consumer contract. [Current application authorization documentation](../../../application-authorization-contract.md) and [retirement list](../phase-24-retirement-list.md) describe the current implementation and remaining transition fixtures.

## Implemented behavior

- Internal issuer `urn:saas:platform:internal-capabilities`; console typ/token_use `platform_session` and exact audience `urn:saas:platform:api`; application typ/token_use `app_session`, exact app-code audience/internal cid, required sid/jti/positive original account version, exact scope `app:access`, maximum lifetime 300 seconds bounded by original source/session/business deadlines. Signature alone is insufficient.
- `platform_app_capabilities` replaces internal app use of legacy oauth_tokens. `platform_resource_services` replaces resource/profile service authentication through oauth_clients. Credential-purpose, audience, operation and generation changes fail closed; service secrets cannot be reused across identity/RP/context/resource authorities.
- The canonical identity private `resource-access-proof/1` verifies exact resource/scope/audience, live original USER/MACHINE grant, application/client/consent/group/SSO/credential generations and original deadline. Public opaque sub is never parsed as an internal UUID. USER PLATFORM_CONTEXT must match a COMMITTED immutable local selection and current AUTH RP/caller-app/member/target-app eligibility. ACCOUNT_ONLY remains UserInfo-only.
- Authoritative internal digest classification occurs before authority selection. Known revoked or malformed internal capabilities never fall through to canonical OAuth.
- `resource-authorization/1` separates private USER principal/context from independent MACHINE delegation. Caller OAuth application and target business application are distinct. MACHINE has no fabricated user, member, sub or employee context; absent/ambiguous/stale delegation denies access. Actual row ownership, tenant/organization and STORE data rules remain resource-service responsibilities.
- Private `resource-directory/1` POST member/organization APIs use independent Basic authentication and token/resource/action in a bounded form. Public Bearer GET APIs stay internal-app-session-only. Directory-purpose registration, permission, independent data grant, target member/STORE metadata and original authority are checked; final authority/service/projection revalidation and target reread reject changes during waits. Responses/errors prohibit caching; the original three-second window cannot be extended.
- Application capabilities and notification publication retain fresh parent/BFF/source/member/app proofs without legacy OAuth registry coupling. Customer profile resources use independent CUSTOMER_PROFILE service registration. The old public platform customer_identity adapter is absent in production and retained only in explicitly historical tests until Phase 24; the portal public `/api/profile` continues using its canonical private profile port.
- Consumed selections retain durable original source/member and switch facts after one-time credential expiry. RP and source-app grant shrink permanently invalidate old resource selections; consumed facts and permanent revocation markers cannot be removed by ordinary cleanup. Expired unconsumed flows remain eligible for bounded cleanup. Controlled post-source retention/erasure is a later lifecycle task, not an authorization to store personal data forever.

## Validation and evidence boundaries

| Check | Result | Evidence |
| --- | --- | --- |
| Platform internal JWT issuer/use/audience/time rejection matrix | PASS 2/2 | `util::jwt::tests`; strict signature/expiry plus internal authority matrix |
| Platform original app binding/UUID/member/account/sid/scope/deadline tests | PASS 2/2 | `app_token::tests` |
| Platform resource proof wire and USER/MACHINE rejection matrix | PASS 2/2 | opaque subject retained; issuer/resource/scope/audience/use/generation/expiry rejected; MACHINE forbids even nullable human fields |
| Platform private directory input boundaries | PASS 2/2 | bounded token input, URL query credential rejection |
| Platform retired production customer routes | PASS 1/1 | production empty-route 404 unit test |
| Canonical migration + actual identity/platform LOGIN roles | PASS 3/3 | `identity_deployment_tests::phase15`; cap shrink/nonrevival/lock-after-expiry, private service generation/separation/ACL, machine scope and durable committed context fences |
| Actual context-selection HTTP handler and PostgreSQL | PASS 1/1 | exact registered member/application scope plus resource/context credential reuse denial |
| Identity resource-proof PostgreSQL HTTP matrix | PASS 1/1 | `phase15_private_resource_proof_separates_issuer_credentials_scope_and_business_sources`; actual USER/MACHINE records, ID/refresh/ACCOUNT_ONLY/offline rejection, exact scope/audience/resource/service allowlist, generation/revocation/expiry |
| Identity customer-proof PostgreSQL regression | PASS 1/1 | canonical customer private proof regression |
| Identity ordinary unit suite | PASS 55; 30 ignored | ignored tests are explicit external/database gates, not counted as passed |
| Platform actual LOGIN roles + resource/private-directory HTTP transport/handler matrix | PASS 1/1, final extended matrix, 2.36 seconds | WSL Linux; USER/MACHINE/resource authority, pending-to-COMMITTED gate, private member/STORE positives, service-version/target-member mutation during the second online proof, scope/purpose/TTL/known-cap rejection; protocol responder is not real identity/platform HTTPS interoperability |
| Current CUSTOMER_PROFILE registry and transaction regressions | PASS 9/9 | WSL existing test executable confirmed to contain the new renewal and committed-switch fence; all nine task databases removed. Historical identity source adapter remains test-only |
| Independent account SHARE-lock regression | PASS, included in 9/9 | actual second PostgreSQL connection in account proof verification during consume/recovery |
| Source/format/static checks | PASS | platform all-target check; final combined all-target clippy with warnings denied and cargo fmt check; identity all-target check/clippy/fmt/diff |
| Independent security review | PASS: no actionable P1/P2 | root-owned read-only review of authorities, SQL/ACLs, directories, claims and rejection matrix |

The native Windows resource loopback test returned 503 at the two-second HTTP timeout, matching the previously diagnosed Windows listener limitation. Its owned database/roles/pools were cleaned. The same WSL Linux test passed in 1.69 seconds. No production verifier was weakened for this fixture limitation.

The customer regression also repairs the Phase 14 actual-HTTPS renewal defect: a revoked prior capability remains only provenance for its unique COMMITTED selection/BFF tuple. A new capability requires the same current source/member/app/service generations; the old capability remains revoked. A later COMMITTED SWITCH permanently fences renewal from the old BFF/version even if its old OAuth grant is still live. Root separately validated the narrow three-file Phase 14 derivative with actual TLS 1.3 Chromium 10/10; that result does not substitute for Phase 15's independent registry and resource tests.

## Acceptance self-check

| Acceptance criterion | Result | Evidence |
| --- | --- | --- |
| External tokens and internal capabilities have accurate independent verifiers without prefix mixing | PASS | strict private internal claims, authoritative digest registry and canonical private resource proof; USER/MACHINE wire rejection matrices |
| Resource/member/application shrink rejects old corresponding abilities | PASS | real LOGIN cap/source/grant/version/clock tests, permanent selection revocation, fresh resource/service/permission/data-grant checks |
| Third parties without a business member can use account profile only | PASS | actual ACCOUNT_ONLY/ID/refresh/offline proof rejection, internal-only employee API rejection, COMMITTED employee context gate |
| Stores/work-orders can integrate independently through stable HTTP contracts without platform implementation crate | PASS | resource-authorization/1 + resource-directory/1; actual private member/STORE HTTP positives and late shrink rejection; unchanged public DTOs; target business row checks retained locally |

## Resources and cleanup

Only saas-plateform/backend and saas-identity participated in this component verification. No Docker containers, networks, volumes or dedicated images were created or reused by this agent. Each PostgreSQL fixture created only its tagged, disposable database/LOGIN roles, and reported database_removed=true, roles_removed=true where applicable, and pool_closed=true. Customer-only fixtures created no roles. No existing business database or role was changed.

The shared native PostgreSQL on port 55494 and Linux build cache remain under the root task's lifecycle for later phases. This report does not claim those shared resources have been removed, nor that Windows WSL virtual-disk space has been compacted. Docker disk-usage delta is not applicable to this component; no Docker storage change or Windows-space recovery is claimed.

## Suggested ledger update

Mark Phase 15 COMPLETED and unlock Phases 16 and 17. All four acceptance criteria are PASS; no actionable Phase 15 implementation or validation item remains. Retire remaining old issuer/test adapters only through Phase 24; do not use historical fixtures as canonical protocol/TLS evidence.


## Final frozen-source check

The final extended HTTP matrix ran against the frozen code with the consumed-selection gate and new directories, and passed. Nine focused non-database tests (JWT 2, app binding 2, resource proof wire 2, private directory input 2 and retired production routes 1) were then executed directly from that Linux test executable: 9/9 PASS. No runtime/schema changes followed the snapshot copy. Root may accept this report and advance the ledger; Phases 16/17 remain responsible for adapting their own independent APIs and real business read/write matrices.

Authorization is checked at explicit fresh online/local checkpoints. A resource proof, contextVersion, directory response or expiry is not an offline write credential. Resource writes must revalidate the original operation/authority before their own local commit. Cross-service HTTP response transmission is not represented as a distributed atomic transaction.
