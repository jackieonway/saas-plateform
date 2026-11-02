# Phase 20 identity/logout/delivery component validation

Date: 2026-10-28. Component state: implementation and directed checks passed.
The complete Phase 20 subsequently passed its fresh three-service HTTPS,
current runtime/log attestation and environment cleanup; see the
[stage report](phase-20.md). This component report does not replace those
separate gates or change the task ledger.

The [frozen contract](../phase-20-logout-delivery-contract.md) binds the
canonical `auth_*` source model, original RP `sid`/`sub`, command recovery,
central delivery and Q5 recipient verification. Only the current fresh `0001`
is used. There is no historical-table or old UUID fallback.

Delivered authority behavior:

- Code exchange creates an immutable, original grant-specific opaque 64-hex
  public `sid` and pairwise `sub`; internal SSO UUIDs are not ID Token `sid`.
- Confidential USER Basic `POST /oauth/logout` implements CURRENT_APP for one
  original grant and GLOBAL for the original account's USER sources, including
  offline grants. Both preserve consent. The original-operation endpoint
  recovers a committed matching receipt after the original token is revoked.
- Unknown/duplicate fields, Cookie/query authority, wrong RP/sid/sub, revoked
  original refresh families and individually revoked refresh tokens are denied.
  Mandatory audit failure rolls back revocation, delivery jobs and receipt.
- Grant, account/SSO, client-policy and consent mutations enqueue the same
  canonical durable jobs. Network delivery begins only after authority commit.

Delivered consumer behavior:

- SKIP LOCKED claims are bounded to four jobs, 30-second leases and fresh UUIDv7
  lease/JTI values. A stale completion cannot settle a replacement lease.
- Requests have a three-second total budget, 64 KiB bounds, fixed DNS/IP and
  original TLS host verification, no proxy/redirect/implicit retry. The exact
  `.test` loopback exception exists only under APP_ENV=test; production refuses
  the test configuration field.
- Retryable failures use 60-second exponential backoff capped at one hour,
  eight attempts and 24 hours. Permanent failures terminate with a fixed code
  and `identity_delivery_terminal` structured warning. Durable jobs contain
  no signed notification JWT, invitation token or response body.
- Signing-key lock waits are followed by fresh lease and owner-target checks.
  Logout preparation retains application/client/admission registration locks
  in registry order through the dispatch decision, then rechecks exact binding
  after signing. Third-party admission must remain APPROVED and BACK_CHANNEL.
  Invitation preparation retains job/invitation/target/verification locks in
  the same order as activation and rechecks exact proof generation and every
  deadline after signing. Owner-only facts remain SELECT-only to runtime;
  narrow owner fences provide row locks without adding UPDATE privileges.
- Consent and security sources fan out and summarize centrally. Missing
  notification support or verified recipient has explicit DEGRADED state;
  token/source revocation remains effective independently of notification.

Q5 delivers only through a unique current owner-verified ACCOUNT_ONBOARDING
channel. Public same-origin acceptance atomically creates a new ordinary
account, consumes the invitation, writes its immutable receipt and mandatory
audit, and creates no membership, role, SSO session or OAuth grant. The new
no-store recipient page accepts material manually; secrets never enter its
URL or browser storage and are cleared on submission/leave. Original-operation
recovery cannot replace the password or source proof. Old claim-only consume
and raw delivery-material HTTP routes are closed.

| Check on current source | Result | Evidence boundary |
|---|---|---|
| Canonical logout scope, receipt, wrong source, audit rollback and races | PASS 3/3 | Real isolated canonical PostgreSQL; access and refresh origins |
| Worker leases, backoff, 8/24h budgets, original JWT, target/source fanout, signing-lock expiry and current registration/admission fences | PASS 6/6 | Real isolated canonical PostgreSQL and RSA signing; all nine current core cases passed together in 14.06s |
| Full identity library on WSL Linux, after final registration-fence repair | PASS 71, 46 explicitly ignored, 0 failed | Includes all 13 transport cases; ignored database suites were run separately |
| Invitation keys/canonical PG/limited LOGIN/final-proof lock deadlines | PASS 7/7 | Invitation owner directed run; SELECT-only facts stay read-only |
| Public invitation shell routing | PASS 2/2 | No database/session authority |
| Recipient form interactions | PASS 6/6 | Node test; secret clearing, original operation, late response and invalid URL |
| Phase 8 protocol and TOTP regression | PASS 2/2 | New actual isolated PostgreSQL runs on current source |
| Phase 9/10/11 resource, refresh and machine regression | PASS 6/6 | New actual isolated PostgreSQL runs on current source |
| Phase 19 canonical review/admission/private pagination regression after registration freeze | PASS 3/3 | New actual isolated PostgreSQL runs; each database removed |
| `cargo fmt --all -- --check` | PASS | Current identity crate |
| `cargo clippy --locked --all-targets -- -D warnings` | PASS | Current identity crate |
| `cargo doc --locked --no-deps` | PASS | Current identity crate |
| Actual Phase 20 HTTPS and final environment cleanup | PASS in separate stage evidence | [Stage report](phase-20.md) links both scoped 13-case runs, visible WARN/redaction and zero-residue cleanup |

A full Windows library attempt had 62 passes, 42 ignored and six transport
failures because native Tokio loopback TCP did not accept connections in this
Windows session. These RED results are retained as an environment limitation;
they are not described as protocol failures or as a successful Windows suite.
The same production transport, CA/hostname policy and tests passed on Linux.
No policy was weakened to make the tests pass.

The final independent source review found and repaired a signing-wait race in
the current client/third-party registration check. Two new real PG cases prove
that cancellation and signing serialize before the dispatch decision, and
that suspended or short-session third-party admission cannot dispatch a logout
JWT. The final nine-case core run and Linux library result above include that
repair; an earlier Linux seven-case run is not used as its acceptance evidence.

All component database tests explicitly targeted loopback port 55494 and
cluster `7701548153213487841`, created UUID-suffixed test databases and closed
and dropped them even on failures. The latest independent catalog check found
`identity_phase20_core_%` databases = 0 and `p20_invite_%` LOGIN roles = 0.
The invitation owner also confirmed `identity_phase19_invite_%` = 0. Ephemeral
RSA directories are removed by their owned-path guard. No component container,
application service, external recipient or business database was created or
modified. Root's shared native PostgreSQL and build cache are retained for the
remaining actual validation; their cleanup belongs to the final task report.

## Self-check

| Acceptance criterion | Component result | Remaining stage evidence |
|---|---|---|
| Current/global scope differs and consent is retained | PASS | Actual two-RP HTTPS behavior |
| Signature/audience/replay/registration/SSRF denials | PASS in separate worker/transport/BFF components | Actual receiver TLS negative requests |
| Remote failure does not block revoke; bounded recovery and permanent alert | PASS in source/lease/error-code tests | Actual failing collector plus service warning |
| Explicit short-session/online-check degradation | PASS | Actual DEGRADED job in the full fixture |
| Verified invitation, bounded retry, no durable token, single-use account creation | PASS in canonical PG and UI tests | Actual worker → collector → recipient form |

The fresh HTTPS script is
[`phase20-browser.mjs`](../../../../tests/unified-auth-https/phase20-browser.mjs).
Its preparation/syntax checks do not execute it. Final stage evidence must cite
its actual JSON result, the exact built snapshot and cleanup, not this component
report or the historical Phase 19 run.
