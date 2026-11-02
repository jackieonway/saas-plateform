# Phase 20 read-only preparation

Recorded 2026-10-28 before implementation. This inventory is not acceptance
evidence. Phase 19 and its actual HTTPS cleanup have since passed; the ledger
now authorizes Phase 20 implementation.

## Observed starting points

- Canonical `/oauth/*` has no logout/end-session flow. Both BFF GLOBAL paths
  still call the old `/oidc/account/logout`, whose authority is the old registry.
- Canonical ID Tokens have no public logout `sid`; the RPs do not persist public
  `sub`/`sid`. Private `global_session_id` must not become a public identifier.
- Existing consent/security outboxes have event metadata and retry timestamps,
  but no complete lease/worker/terminal-failure implementation.
- Client/admission records have no reviewed back-channel target. No endpoint
  may be chosen from a token or a delivery request.
- Invitation delivery/consume validate channel/verification reference syntax,
  not a persisted trusted recipient-purpose-channel verification fact.

## Required implementation boundaries

Keep current-app logout, global SSO logout, consent withdrawal and account
security changes distinct. Preserve consent where required. Commit source
invalidation, transaction audit, idempotent receipt and outbox atomically;
network delivery never participates in that revocation transaction.

Define stable opaque RP/source logout bindings. ID Token and local RP rows must
agree. Receivers validate fixed issuer, own audience, RS256/JWKS, bounded time,
`jti`, standard `events`, exact `sid`/`sub`, and absence of `nonce`. Persist
replay protection and revoke only the original local source. A delayed event
must not clear a newer browser pointer. Reception cannot require an access token
that was already revoked by the event.

Delivery uses reviewed HTTPS targets, rejects proxy/redirect/private-address
escapes, and pins the checked DNS address for the request. Use bounded database
leases, batch/concurrency limits, retries and a visible terminal failure state.
Document short-session/online-check behavior for RPs without back-channel support.

Q5 is already decided: only an effective directed invitation may create a new
account without business memberships. Use unique usernames and the existing
password hash implementation. Never re-enable an existing security-disabled
account or merge by email. Invitation consumption and account creation share
one transaction or recoverable original operation; retry cannot create another
account. Trusted channel/verification facts are owner-controlled, not arbitrary
client UUIDs. Persist only invitation identifiers, versions, channel references
and digests in retry records; recover token material only in memory. Test
delivery with an isolated owned collector; send no actual external messages.

## Reusable verification

- Identity `util/audit_tests/auth_protocol.rs` and `auth_consents.rs`: canonical
  grants, token/refresh/source invalidation and consent transaction isolation.
- Identity `auth_invitations/tests.rs`: lifetime, digest, consume and audit
  rollback; extend it with trusted channel and atomic new-account creation.
- Platform BFF tests: local/global scope, uncertain results and late-event
  isolation. Replace old issuer fixtures before counting canonical evidence.
- Portal owned-PG fixtures: preserve independent DB/runtime and purpose.
  Its current provision helper assumes three tables and sixteen UUID columns;
  update the explicit contract if replay storage adds a table/column.

Required negative/concurrency cases include cross-RP/account/source isolation,
invalid signatures/audiences/events/replay, late delivery, unsafe targets and
DNS rebinding, competing workers, lost responses, bounded permanent failures,
revoked channels, secret redaction and repeated invitation acceptance.

## Ownership

`audit_foundation` coordinates Phase 20 and identity behavior. `phase13_schema`
alone edits canonical platform/portal migrations, ownership/ACL and readiness
contracts. `phase20_bff` owns platform/portal RP behavior and tests. Root owns
ledger, task environment lifecycle and final acceptance. No existing database
is migrated or reset by this preparation record.
