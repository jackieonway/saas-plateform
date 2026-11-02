# Unified OAuth/OIDC Implementation Contract

Status: Phase 1 contract baseline, 2026-10-27. This freezes the target behavior from the architecture and adopted policy documents; it does not claim implementation or production approval.

Inputs: [architecture](../unified-oauth-oidc-sso-architecture.md), [policy decisions](../unified-oauth-oidc-sso-policy-decisions.md), eight repository READMEs and current source at the commits recorded in [the callsite inventory](callsite-inventory.md).

## 1. Authority and boundaries

- `saas-identity` is the only authority for account authentication, SSO, application/client authentication projection, OAuth/OIDC issuance, user consent, subject mapping, credentials, signing keys, token state, introspection and revocation.
- `saas-plateform` owns partner and application business records, staff/customer memberships, tenant/organization context, resource policy and management authorization. It sends versioned, authenticated commands to identity; it does not read or write identity-owned tables or issue public OAuth/OIDC credentials.
- Each resource API enforces its registered audience and scopes, then checks the current business membership/context against platform authority. Identity does not infer roles or query the full membership set.
- Platform and customer portal remain separate relying parties (BFFs), with distinct callbacks, cookies, sessions, refresh storage and staff/customer policy.
- Notification remains a context-bound transport, not a login or token issuer. The micro-app SDK transports a host-issued business context and does not authenticate users.
- No service-unavailable fallback may restore platform OAuth issuance. Private proof tokens/capabilities are purpose-bound and are not a second public authorization server.

## 2. Identity and authorization dimensions

Application ownership (`FIRST_PARTY` / `THIRD_PARTY`), partner, environment, client type, runtime, subject mode, context mode, grant types, resource permissions and consent group are separate fields. None implies another permission.

| Dimension | Contract | Rejected inference |
|---|---|---|
| `subject_mode` | `USER` or `MACHINE`; one mode per client | A machine token is not a user or employee token |
| `context_mode` | `ACCOUNT_ONLY` or `PLATFORM_CONTEXT`; machine clients have no user context | Account-only login does not require a platform member |
| client type | `CONFIDENTIAL` or `PUBLIC`; auth method is explicit | Public clients never receive a shared secret |
| environment | Explicit test/production boundary on every client and credential | Test callbacks, grants, consent or credentials do not cross to production |
| member type | `EMPLOYEE` / `CUSTOMER`, only when platform context is requested | A customer does not inherit employee permissions |
| ownership | `FIRST_PARTY` / `THIRD_PARTY` | First-party is not preapproval; partner is not a tenant |
| audience/scope | Fixed registered resource plus approved scope set | Client input cannot choose an arbitrary URL or resource |

Third-party default is USER + ACCOUNT_ONLY + `openid profile`. Email, phone, business scopes, offline access and cross-application account linking remain denied unless separately approved by the policy gate. No dynamic registration, password/implicit grant, device flow, SAML/SCIM, external IdP, or `private_key_jwt` is advertised until implemented and accepted.

## 3. Target records and version rules

The following are logical records; Phase 2 fixes canonical SQL names, owner/runtime roles, constraints and initialization procedures. Identity-owned tables have UUIDv7 IDs and monotonically increasing `version` / policy generation where mutable. Secret material is never returned by read/list APIs.

| Record | Required binding / invariant |
|---|---|
| `auth_applications` | Platform application ID, ownership, partner projection, review status, approved purpose/scopes and projection version |
| `auth_clients` | One application + environment + subject mode; explicit runtime/client/context mode, grant/auth methods, exact callbacks, status, policy version |
| `auth_client_credentials` | Client, version, secret digest/public key, validity/revocation; secret is one-time response only |
| `auth_resources`, `auth_scopes`, `auth_client_permissions` | Fixed audience and scope ownership; client grant is a subset of reviewed application permission |
| `auth_consent_groups` | Explicit same application/environment/owner/purpose/USER context group and enumerated client membership; adding a member advances the group version and forces each existing account consent to be confirmed again |
| `auth_consents`, `auth_consent_scopes` | Account + group + acknowledged group version + resource-qualified scope set + generation + state + timestamps; new grant generation after revoke |
| `auth_subject_links` | Internal account + approved sector + stable opaque sub; uniqueness prevents reuse; no UUID disclosure |
| authorization requests/codes, grants, access/refresh tokens | Immutable client, original user/session/context and policy/consent generations, scopes/audiences, expiry and revocation source |
| operation receipts and audit/outbox | Actor/service, operation ID, canonical request digest, outcome/version; no secrets or bearer material |

Every state-changing management request carries `expectedVersion` (or `If-Match`) and an operation ID. A stale version returns `version_conflict` without mutation. A client's application, environment, subject mode, owner and consent meaning are immutable after creation; change those by registering a new client and retiring the old one.

## 4. Command and DTO contracts

### 4.1 Application projection

Platform sends only the minimum reviewed projection. Example (illustrative IDs, no credentials):

```json
{
  "operationId": "019a0000-0000-7000-8000-000000000001",
  "applicationId": "019a0000-0000-7000-8000-000000000002",
  "expectedVersion": 3,
  "projection": {
    "ownership": "THIRD_PARTY",
    "partnerId": "019a0000-0000-7000-8000-000000000003",
    "status": "APPROVED",
    "displayName": "Example CRM",
    "purpose": "Customer sign-in",
    "approvedScopes": ["openid", "profile"]
  }
}
```

Projection status is `DRAFT`, `PENDING_REVIEW`, `APPROVED`, `SUSPENDED`, or `RETIRED`. `APPROVED` is necessary, not sufficient: each client, callback, resource/scope and environment remains independently constrained. A timeout never means a projection succeeded; caller queries the same operation receipt.

### 4.2 Client management shape

Create/update commands use typed fields, not a free-form `client_type` shortcut.
The platform API accepts the client fields at the top level; the operation ID and
digest are forwarded in the private service headers, not as client-controlled JSON:

```json
{
  "expectedVersion": null,
  "applicationId": "019a0000-0000-7000-8000-000000000002",
  "environment": "TEST",
  "displayName": "Example CRM Web",
  "clientType": "CONFIDENTIAL",
  "runtimeKind": "WEB_BFF",
  "subjectMode": "USER",
  "contextMode": "ACCOUNT_ONLY",
  "allowedMemberTypes": [],
  "grantTypes": ["authorization_code", "refresh_token"],
  "tokenEndpointAuthMethod": "client_secret_basic",
  "redirectUris": ["https://client.example.test/callback"],
  "postLogoutRedirectUris": [],
  "status": "DISABLED",
  "permissions": [{"resourceKey": "userinfo", "scopes": ["openid", "profile"]}],
  "consentGroupId": null
}
```

The service validates cross-field combinations, exact HTTPS callback/logout URL registration, approved application status, scope/resource subset, environment and group membership. The API normalizes environment names to lowercase before storage. MACHINE permits only approved machine grants and no member/context fields. PUBLIC permits no secret-based auth or client_credentials. `refresh_token` policy differs between online PLATFORM_CONTEXT and approved offline ACCOUNT_ONLY; a grant request cannot widen client policy.

### 4.3 Operation ID / receipt

Mutating private commands carry one canonical UUIDv7 `operationId`, authenticated service identity, typed method/path, canonical JSON digest and optional expected version. Receipt key is operation ID scoped to the authenticated service. Same ID + same digest returns the original receipt/result; same ID + different digest returns `operation_conflict`. On timeout or connection loss, query `GET /internal/identity/operations/{operationId}` with the same service identity. `PENDING` remains pending; do not create a replacement ID or infer success from transport status. Secret creation response is reproducible only for the same authorized service, operation ID and digest, and is never included in ordinary receipt/listing.

Receipt states: `APPLIED`, `REJECTED`, `PENDING`, `NOT_FOUND`. An operation not found may be resubmitted only with the identical operation ID and digest. Receipt retention must exceed the longest recovery/retry window and remain consistent with Q8; Phase 1 sets no unsupported legal retention assertion.

## 5. Public protocol contract

One explicit HTTPS issuer. Public discovery advertises only live, implemented features. Target endpoints are `/.well-known/openid-configuration`, `/oauth/authorize`, `/oauth/token`, `/oauth/userinfo`, `/oauth/jwks`, authenticated `/oauth/introspect`, `/oauth/revoke`, and a declared logout endpoint. Context interaction and account consent management are authenticated, CSRF-protected endpoints; private platform management/proof endpoints are never routed through the public gateway.

Authorization request requires registered `client_id`, exact registered `redirect_uri`, `response_type=code`, S256 PKCE (`code_challenge_method=S256`), opaque state, nonce for OIDC, approved scopes, correct issuer and supported prompt/max_age semantics. Code is single use, digest-stored, client/callback/verifier-bound, and expires after 60 seconds. The client/BFF verifies state, issuer and nonce. `prompt=consent` forces explicit confirmation; default login reuses a sufficient active consent.

Token grants:

- `authorization_code`: one-time code + matching verifier + exact client/callback + client authentication when confidential; returns signed ID Token only for USER/OIDC, opaque resource-bound access token, and policy-allowed refresh.
- `refresh_token`: atomically consumes and rotates one refresh token; grant/client/user/consent/source versions and original absolute expiry must remain valid. Reuse of a consumed token revokes its family and the family's access tokens. It never adds scope/audience or renews the absolute deadline; an optional `scope` can only narrow the access token.
- Refresh lifetime follows its source. An online family expires no later than its original SSO/business source and uses a 30-minute idle limit. Only an ACCOUNT_ONLY grant that explicitly includes the registered, client-approved and user-consented `offline_access` scope may use the 7-day absolute / 24-hour idle limits. Natural browser-session expiry alone does not invalidate that offline grant; source-session revocation, account-generation changes, consent revocation and application/client policy changes do.
- `client_credentials`: confidential MACHINE only; approved fixed audiences/scopes; short access token, no ID Token, no user identity and no refresh token. The grant records the exact source `credential_version`; token verification rechecks that version's live revocation/expiry state and the client/application policy versions.

ID Token is for the client only. Access Token is opaque, digest-stored, audience- and scope-bound, online-introspected by an authenticated resource client. UserInfo returns only approved consented profile and exactly the same opaque `sub` as ID Token. `sub` is stable within an approved application/environment sector and is never an internal UUID. Scope/audience failures are deny-by-default.

Resource scopes resolve only through the exact active client permission rows and the application's approved-scope ceiling; scope names alone never grant a resource. `openid` is bound to the `userinfo` resource. Introspection uses a confidential MACHINE resource client authenticated with `client_secret_basic`; its current resource permissions bound the `aud` and `scope` returned. An optional `resource` parameter narrows that response. User-token introspection returns the pairwise `sub`; MACHINE introspection returns `subject_mode=MACHINE` without `sub` or `auth_time`. UserInfo rejects MACHINE tokens. Malformed, expired, revoked, wrong-audience, or unverifiable-source tokens are inactive; a client authentication failure is rejected, and a database/source failure fails closed.

The Phase 9 revoke endpoint initially accepted only confidential-client authentication and revoked an owned access token. Phase 10 extends `/oauth/revoke` to an owned refresh token: it revokes the whole refresh family and all access tokens attached to its grant. Unknown or foreign tokens receive the same successful response; clients cannot use the endpoint to discover another client's token. Revocation still works after the account/session source has become invalid because it only removes authority; online introspection remains responsible for revalidating that source.

### 5.1 Error surface

OAuth token/authorization endpoints use standard errors where defined: `invalid_request`, `invalid_client`, `unauthorized_client`, `unsupported_grant_type`, `invalid_grant`, `invalid_scope`, `access_denied`, `interaction_required`, `login_required`, `consent_required`, `server_error`, `temporarily_unavailable`. Never put secret, token, code, verifier, user UUID or internal SQL detail in an error.

Authenticated management APIs use stable machine codes: `version_conflict` (409), `operation_conflict` (409), `operation_pending` (202), `not_found` (404), `forbidden` (403), `step_up_required` (403), `invalid_projection` (422), `scope_not_approved` (422), `environment_mismatch` (422), `temporarily_unavailable` (503). HTTP status and code are both part of the contract.

## 6. Consent, grant and revocation state machines

Consent states: `UNGRANTED → ACTIVE → REVOKED → DISPLAY_DELETED`. Re-consent after revocation uses the already advanced generation; it never reactivates old grants. Consent key is account + explicit consent group; group membership is explicit, same application/environment/owner/purpose/context, and USER clients only. The consent records the group version last confirmed by the user. Adding or registering another member advances that version; existing consents remain insufficient until the user confirms the new group membership. Scopes are a resource-qualified set. Scope/resource/data-purpose expansion requires new user confirmation. Deleting a revoked UI row records `display_deleted_at`, removes its displayed scope rows, and retains the minimum consent generation/version fact needed to stop old credentials and reject stale delete requests.

Refresh-family states: `ACTIVE → ROTATED → REVOKED` or `EXPIRED`. Each successful rotation consumes one token and creates one successor in the same family with the original absolute expiry. Concurrent use serializes in the database. Reuse revokes the whole family. Access-token and grant validation checks original consent, client/application status and policy generations, subject/account status and business-source validity.

Revocation is local and transactional: lock/validate the original grant/source; advance consent/account/client generation as appropriate; revoke affected grant/families/tokens/codes; invalidate pending interactions; insert audit and logout outbox records in the same transaction; commit before bounded delivery. Current-app logout affects only its local session/associated grant policy and retains consent. Global SSO logout, account disable and consent revoke have distinct scope. Back-channel delivery is not the revocation authority and remote failure cannot undo local revocation.

### 6.1 Lock order

All grant issuance, code redemption, refresh, consent revoke/delete and client/application disable paths use one database lock order: account/security generation → application projection → client/policy row → consent group → account consent → authorization transaction/code → grant → refresh family/token rows (stable UUID order) → audit/outbox. A transaction locks only applicable rows in that order and rechecks session/source, policy and expiry after acquiring locks and immediately before commit. Cross-service platform selection is not held inside a database transaction: persist a short-lived bound interaction, obtain platform's one-time result, then revalidate interaction/client/account versions and consume that result once. Any ordering exception requires a documented deadlock analysis and concurrency regression before implementation.

## 7. Subject, context and account policy

- Pairwise opaque sub is persisted per approved application/environment sector. Same app's explicitly grouped clients may share a sector; cross-app/partner linking is out of scope. Names, email and phone do not merge users. Retired identifiers are never reassigned.
- ACCOUNT_ONLY login never queries or requires platform membership. PLATFORM_CONTEXT requests are explicitly employee/customer constrained and revalidated by platform at selection and use time. One actual eligible identity may continue through the same protected server-side confirmation; search filtering to one result is not automatic authorization.
- BFF internal proofs may carry internal UUID and business context only to the intended private platform/resource service, authenticated and audience/use bound. Public sub remains opaque.
- Invitation-only accounts; no public self-registration. Recovery requires trusted verification, revokes old sessions/refresh/recovery credentials, and records an audit. MFA assurance claims are not emitted before implementation.

## 8. Permission and step-up rules

Separate permissions: view, register/edit, review/approve, manage application scopes/resources, create/rotate/revoke credentials, disable client/application, manage signing keys, view security/audit, and operate user consent. A requester cannot approve their own application. A partner admin sees only expressly assigned applications and cannot gain tenant/platform administration from partner ownership. Production-admin TOTP is mandatory; credential creation/rotation, scope expansion, production approval/enablement, MFA recovery and global security changes require second authentication within five minutes. Server-side authorization is mandatory even when UI controls are hidden.

Resource servers require: approved application ∩ active client permission ∩ active user consent when USER grant applies ∩ correct audience ∩ token scope ∩ current subject/client/policy generations ∩ current business membership and data range. Authentication-center/database failure is `503`/deny; it is never converted into cached allow or account-data purge.

## 9. Adopted default limits and retained policy gates

| Item | Default | Gate |
|---|---|---|
| code / interaction | 60 seconds / up to 5 minutes | Reject replay and enforce original-source expiry |
| ID / opaque access token | 5 minutes / 5 minutes | ID Token never accepted by APIs; access is online introspected |
| SSO / online BFF session | 30 minute idle, 12 hour absolute | Refresh cannot exceed earliest original SSO/context source expiry |
| offline ACCOUNT_ONLY family | 7 day absolute, 24 hour idle | Explicit approved `offline_access`; no business API delegation |
| machine access | 5 minutes, no refresh | Reissue using machine credential |
| secret overlap | at most 24 hours | Regular rotation target at most 90 days; incident revoke immediately |
| security/admin audit | 180 days default | Q8 real jurisdiction/contract verification before production |
| ordinary access logs / backup rotation | 30 days defaults | Q8 is a product default, not a legal conclusion |
| capacity/trial | 10 partners, 50 clients, 10,000 accounts; target 50 code/refresh rps and 200 introspection rps for 30 minutes | Q9 measured evidence and named operations owners required before commitment |
| recovery | RPO 15 minutes / RTO 4 hours targets | Q9 protected backup/WAL and actual drill required |
| production launch | single issuer, verified environment | Q3/Q8/Q9/Q10 facts, Phase 25 candidate and task-specific Phase 26 target/operation authorization required |

Q3 reviewer/publisher accounts, Q8 jurisdiction/contracts and retention basis, Q9 capacity/drill/on-call identity, and Q10 real production target/secrets/backup/rollback remain deployment gates. Their adopted defaults may guide implementation but are not evidence that those conditions exist.

## 10. Version and rollout rules

The goal is canonical fresh-install `0001`; no old UUID migration, checksum takeover, dual write/read, automatic DDL or runtime fallback. Intermediate schemas used during phases must be on the Phase 24 retirement list. Issuer and endpoint changes must update both BFFs, discovery, gateway and third-party configuration. SDK 3.0.0 and its contract versions remain fixed unless an explicit consumer-verified version change is approved. Each implementation phase must record exact repository HEADs, dirty files, commands, verification level, risks and cleanup in its phase report.
