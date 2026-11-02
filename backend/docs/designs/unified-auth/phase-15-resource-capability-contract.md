# Platform resource and private capability contract

Status: completed; the Phase 15 validation report records the acceptance evidence.

The canonical public OAuth issuer is identity only. Its access tokens are opaque. Public subjects are never parsed as platform UUIDs. The private identity `/internal/identity/auth/resource-access-proof` contract is `resource-access-proof/1`; it proves canonical resource, scope, audience, original grant and source generations online. OAuth caller application and target business application are distinct entities.

Internal authority is `urn:saas:platform:internal-capabilities`, with no public discovery or token endpoint. Console `typ` and `token_use` are `platform_session`, audience is `urn:saas:platform:api`. Application `typ` and `token_use` are `app_session`; audience and `cid` are the exact target `applications.code`. Here `cid` is an internal application reference, never an OAuth client ID. An app capability must have a UUIDv7 jti, exact parent sid, positive original account version, `app:access` scope, and a lifetime at most 300 seconds bounded by the original session/source. SDK 3.0.0 session-token response, context 1.0 and MicroProps 2.0 remain unchanged.

`platform_app_capabilities` stores only a digest and immutable original bindings. An authoritative digest lookup selects internal capability verification; a known revoked or malformed internal capability cannot fall through to external OAuth. Unknown tokens may only enter the canonical online verifier. ID/refresh tokens cannot enter either authority.

Resource backends authenticate independently with `platform_resource_services`, purpose `APP_RESOURCE`. Customer profile uses distinct purpose `CUSTOMER_PROFILE`, member types exactly CUSTOMER, and operation `customer:profile`. Services bind a target business application, exact audience and operation allowlist, credential version, secret digest and enabled status. These registrations do not issue OAuth credentials or register public clients.

External USER PLATFORM_CONTEXT proof returns selected member ID/type and context reference. Platform loads the immutable `context_selections` source/member snapshot and compares account, source SSO, client, auth time and generations before checking current member, tenant, organization and target application eligibility. ACCOUNT_ONLY and CUSTOMER cannot access employee management resources. Account-only third parties use public UserInfo.

MACHINE proof contains no user/member/employee DTO. Explicit platform delegation binds source canonical client and caller OAuth application separately from target business application, tenant/organization, resource/action, data scope and original credential version. No matching active record denies access. Personnel operations may reject MACHINE explicitly; resource services retain local business row filtering.

Public Bearer app context/member/organization/notification APIs use internal app capability only. Independent services use the explicit Basic-authenticated POST directory routes described below. `/api/apps/{appCode}/authorize` is the independent resource backend boundary, with versioned private proof and exact resource/action permission plus independent data grant. The 30-second context projection and contextVersion never bypass fresh authority checks.

## Credential entry inventory

| Entry | Authority / audience / use | Subject and original binding | Required scope and business check |
| --- | --- | --- | --- |
| Employee administration and `/api/auth/*` Bearer APIs | private internal issuer; `urn:saas:platform:api`; `platform_session` | EMPLOYEE, exact parent BFF/session, account/member/source generations | current roles/permissions and each management operation's tenant/organization boundary |
| `/api/oauth/session-token` | same console verifier | current EMPLOYEE; no caller-supplied user, member, source or service client | both target tenant/application and organization/application grants; current target app member policy |
| Public Bearer `/api/apps/{code}/context`, member/organization directories and app notification publication | private internal issuer; exact app code; `app_session` | exact internal digest/jti/parent sid/account/member/app/source snapshot | `app:access` allows asking for current projection, never replaces menu permission, registered directory purpose or independent data grant |
| `/api/apps/{code}/authorize` with internal token | independently authenticated APP_RESOURCE service + strict internal capability verifier | original USER member and target business app | exact service operation, resource mapping, server capability policy, current operation permission and independent data grant |
| Same private authorize boundary with external opaque token | exact configured canonical issuer + identity private proof; exact registered resource/scope/audience; `access_token` | USER PLATFORM_CONTEXT source and original immutable selection; public opaque sub retained | target business app eligibility separate from OAuth caller application; current EMPLOYEE business permissions/data grant |
| `/internal/platform/apps/{code}/{members,organizations}` POST directory routes | independent APP_RESOURCE Basic identity + bounded token form; exact app/resource/action mapping | EMPLOYEE only, strict internal capability or canonical USER COMMITTED selection; MACHINE rejected | registered directory purpose, current permission/data grant/target state; final source/service/projection and target rechecks |
| Same boundary with MACHINE opaque token | same canonical proof and private service | source client, caller OAuth application and exact credential version; no person | one explicit current delegation to target app/resource/action/data scope; resource policy scope ceiling; downstream local business SQL |
| Notification inbox/socket-context | strict internal `platform_session` and exact platform API audience | EMPLOYEE/CUSTOMER actual member; original session/source and account/member generations | own inbox/delivery tuple; online source and fresh local member proof |
| `/internal/customer-portal/{memberships,context,profile,revoke}` | independent CUSTOMER_PROFILE service, exact service audience app code; canonical customer access proof plus digest-bound customer context | CUSTOMER only; original RP/grant/SSO/account and selected customer membership; original service credential/app generations | `customer:profile`; only authenticated account presentation and its exact selected customer context |
| Retired platform `/api/customer/{context,profile,applications}` | absent in production (404) | test-only historical issuer adapter until Phase 24 | portal public `/api/profile` remains and calls the canonical private profile port |
| Transitional platform `/oauth/*` protocol helpers | retained legacy route implementation until Phase 24; not the canonical public issuer contract | old client/token structures and regression fixtures remain under the retirement gate | internal app capabilities are explicitly rejected by old UserInfo/introspection; canonical public UserInfo belongs to identity |

No business member means no platform employee resource context. ACCOUNT_ONLY third parties retain identity UserInfo only. MACHINE cannot use the employee directory or notification inbox as a person.

## Private resource authorization reply

`contractVersion` is `resource-authorization/1`. USER returns `principal` with internal account/member IDs and positive original account/identity versions, `context` with the unchanged public 1.0 projection, and an `introspection` summary. External USER additionally returns canonical `clientId` and `sourceApplicationId`; `introspection.sub` stays opaque. Internal `introspection.client_id` is the application code, explicitly an internal reference.

MACHINE returns `clientId`, `sourceApplicationId`, `targetApplicationId`, `credentialVersion`, exact `scope` list and `delegation` (`delegationId`, `version`, target app, tenant, optional organization, resource/action, scope and explicit organization IDs). It never returns principal/context/sub/user/member fields. Multiple matching delegations without a unique tenant selector fail closed.

Authorization `expiresAt` never exceeds the credential and current business deadlines. USER context also caps at 30 seconds and the earliest relevant organization-chain/tenant deadline. MACHINE caps at delegation, tenant and every explicitly scoped organization-chain deadline. These response deadlines are informational upper bounds; every business request still requires fresh authorization and local row checks.

## Registration and failure behavior

Initialize private resource services through the platform owner deployment channel, using a distinct random secret digest, fixed target application code audience, exact purpose/member types and operation allowlist. Configure `RESOURCE_PROOF_SERVICE_CLIENT_ID/SECRET` with a separate identity service registration of purpose `RESOURCE_ACCESS_PROOF` and `allowed_auth_resources` containing only the canonical resource UUIDs it may prove. Neither registry grants OAuth public client credentials.

`platform_resource_bindings` maps target app + business resource/action to canonical resource key, exact audience and required single OAuth scope. Configuration or mapping shrink cannot be repaired by token claims. Client/application/consent/group/SSO revocation is checked by the canonical authority, while platform member/app/data permissions are checked locally. Known revoked internal digests never use the external verifier.

Return 401 for invalid credentials or source/generation substitution; 403 for missing member type, operation, independent data grant or machine delegation; 503 when a fresh identity/projection result cannot be obtained. Duplicate credentials, cookies on private service authentication, unknown fields, wrong issuer/audience/use and expired proofs fail closed. No response carries an OAuth secret or raw upstream token.

Consumed `context_selections` are durable source/member and switch facts. Their 120-second credential/recovery expiry does not authorize deletion of a still-referenced original selection. The issue-capacity cleanup removes expired unconsumed flows only; database guards reject deleting consumed facts or clearing a permanent resource revocation marker. RP registration and caller application grant shrink permanently mark old selections for resource rejection, so disable/re-enable or grant re-add cannot repair an old selection. Current caller-app eligibility and target-resource-app eligibility are checked independently.

## Private directory contract for independent resources

Keep public GET member/organization directories internal-app-session-only. Independent resource services use these explicit POST routes:

| Method/path | Form body beyond `token`, `resource`, `action` | Result |
| --- | --- | --- |
| `POST /internal/platform/apps/{appCode}/members` | required `organizationId`; optional `page`, `size`, `q` | current minimal member list |
| `POST /internal/platform/apps/{appCode}/members/{userId}` | required `organizationId`; no pagination/search | current minimal user/member reference |
| `POST /internal/platform/apps/{appCode}/organizations` | optional `page`, `size`, `q` | current STORE directory list |
| `POST /internal/platform/apps/{appCode}/organizations/{organizationId}` | no pagination/search | current STORE reference |

Authentication is independent `Authorization: Basic base64(resource_service_id:secret)`. The bounded form is `application/x-www-form-urlencoded` and at most 32 KiB; access tokens are never accepted in URL query parameters or custom token headers. Reject cookies, duplicate credentials, unknown form fields, wrong audience/app/operation and MACHINE. USER requires EMPLOYEE, a committed original selection, exact original source and both caller/target app eligibility. Internal app_session tokens use their authoritative digest domain and original parent binding.

The response wrapper is `{contractVersion:"resource-directory/1",directory:"members"|"organizations",expiresAt,data}`. `data` preserves the existing directory DTO, including the STORE directory 1.0 contract/contextVersion/revision where applicable. Wrapper expiry is at most three seconds and bounded by source/context/target business deadlines. Member `userId` paths refer to the private account ID from a validated `principal` or directory response, never the public pairwise `sub`.

The operation must be additionally registered in `DATA_GRANT_POLICY` memberLookups or organizationLookups. Scope alone grants no directory permission. The private service, mapping, exact original authority and actor projection are revalidated after target reads; target metadata is then independently reread and compared before returning. Authority unavailability, version change, late target shrink or expiry cannot return an earlier positive projection.

These are authorization checkpoints across independently owned services, not an atomic transaction spanning HTTP response transmission. A directory response is a bounded controlled reference read and cannot authorize a later write by itself. Resource writes must revalidate the original operation/authority before their own local commit and retain local tenant/organization/owner and STORE constraints. Do not turn directory data, contextVersion or expiresAt into an offline credential.
