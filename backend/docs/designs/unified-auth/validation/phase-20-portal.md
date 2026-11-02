# Phase 20 portal component

Date: 2026-10-28. Status: component verification complete; Phase 20 as a whole
remains owned by the main coordinator, including actual cross-service delivery.

## Scope and inputs

The customer portal keeps its independent database, CUSTOMER-only source and
host-only Cookie. Starting HEAD was `648e71e082929605c67e4e9f7e19fc9ecdbd89d4`;
the already accepted Phase 14 changes were dirty and preserved. This component
adds no dependency on platform/identity implementation crates and no new crate.
The schema owner separately maintains the canonical portal baseline, restricted
roles and startup readiness; handler work does not modify that ownership.

Source hashes and result counts are recorded in
[the component manifest](phase-20-portal-components.json). The task ledger is
updated by the root agent, not this component.

## Resulting behavior

- The initial ID Token supplies an opaque 64-hex RP `sid` and opaque public
  `sub`. They are immutable session fields, separate from private account,
  grant and SSO UUIDs. A refresh ID Token cannot change them; a response without
  an ID Token retains the originals. ID Tokens use `typ=JWT` and logout tokens
  use `typ=logout+jwt`.
- Both local `PORTAL` and `GLOBAL` logout use canonical `/oauth/logout` with
  the original refresh, public source, scope and operation ID. The portal first
  queries `/oauth/logout/operations/{id}` with exactly that binding. Only an
  explicit 404 `NOT_FOUND` permits submitting the same operation. A pending
  original command can recover after its source was revoked, while a newer
  browser pointer remains protected.
- `/auth/backchannel-logout` requires a single bounded Form `logout_token`.
  It validates the configured issuer's fixed JWKS, RS256, strict header/type,
  exact issuer and single RP audience, required public sid/sub, canonical
  UUIDv7 jti, exact empty back-channel event and original 120-second window.
  Future iat is limited to five seconds. Query, nonce, duplicate or unknown
  fields and wrong token kinds are rejected.
- Signature verification has no dependency on access/account proof validity.
  A local transaction records jti replay metadata and a permanent sid/sub
  digest source fact, then closes only matching local sessions. Duplicate jti
  returns `409 BACKCHANNEL_REPLAY`; another valid delivery attempt for an
  already closed source has no additional effect. A known sid with another
  sub is rejected.
- Even an unknown local sid records a revocation fact before acknowledgement.
  Callback and refresh commits take the same source advisory lock before
  browser/session row locks and reject that fact. A proof returned before
  the notification therefore cannot insert or restore a local session after
  the notification committed. A different authorized sid can still log in.
- A late notification clears a browser pointer only when it still references
  that source. Original expiry is checked after lock waits; expiry rolls back
  receipt, source fact, pointer and session changes together.

The final narrow error-classification correction returns HTTP 400 for a
notification that expires after either the source advisory wait or the final
row-lock wait; database unavailability remains 503. The actual router lock-wait
regression was rerun and passed, requiring 400 and zero receipt/source-fact
rows, with the original session and browser unchanged. The original-operation
RPC recovery unit was also rerun after making its receipt parser strict about
unknown/duplicate fields. These two focused checks and final strict Clippy/fmt
passed after the full 13-case PostgreSQL run; that full run was not repeated for
the error-only correction.

Current implementation and operator instructions are in the
[portal README](../../../../../../saas-customer-portal/backend/README.md).

## Verification

| Command or matrix | Result | Layer |
| --- | --- | --- |
| `cargo fmt --all -- --check` | PASS | Final portal crate |
| `cargo check --locked --all-targets --all-features` | PASS | All current targets |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | PASS | No warnings |
| `cargo test --locked --all-targets --all-features` | 21 PASS; 13 ignored | Ignored tests are not counted as passed |
| `cargo test --locked --lib tests::pg:: -- --ignored --test-threads=1 --nocapture` | 13/13 PASS, 0 ignored | Actual router/local SQL and actual LOGIN-role schema matrix |
| `cargo doc --locked --no-deps` | PASS | Current library and binary docs |
| Scoped diff and document link check | PASS | Current changed files |

The explicit PostgreSQL suite includes the previous eight portal transaction
regressions, four new Phase 20 tests and the schema owner's real-role test.
New coverage proves signed source isolation, wrong subject and replay rejection,
late notification/new browser protection, original revoked operation recovery,
actual independently held browser row lock followed by expiry/complete rollback,
and a real callback paused after its private proof returned. In the latter case
an unknown-sid notification commits first; the old callback is denied with no
session inserted, another sid returns 303, and a returned refresh result cannot
restore its subsequently revoked source.

JWT/JWKS and canonical logout authority responses in these component tests are
explicit transport fixtures. Signatures, handlers and local PostgreSQL are
actual. They do not constitute an actual identity-to-portal HTTPS worker
delivery, key rotation drill or full release-candidate acceptance. Those remain
with the corresponding main phase gates.

## Resources and cleanup

The tests require explicit `IDENTITY_PHASE11_TEST_DATABASE_URL` and
`CONTEXT_SELECTION_TEST_CLUSTER_ID`, verify cluster identifier
`7701548153213487841` on the owned loopback 55494 instance, and create only
random `identity_phase11_portal_test_` databases. Every final suite database was
deleted and pools closed; the real-role case also reported role deletion and
runtime/owner pool closure. An independent catalog query after verification
confirmed the expected cluster and zero databases with that fixture prefix.

No container, network, volume, dedicated image, external message or new target
directory was created. The shared native PostgreSQL and existing build cache
remain under root task lifecycle control. Committed test-only RSA fixture code
is not deployment key material. No browser trace, runtime secret, token, Cookie,
private environment file or exported runtime log was created by this component.
This report does not claim Windows virtual-disk compaction or final task cleanup.
