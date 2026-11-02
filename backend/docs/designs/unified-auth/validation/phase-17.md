# Phase 17 — Work-orders resource API

Date: 2026-10-28  
Status: COMPLETED — component acceptance; root maintains the ledger.

## Boundary and inputs

The Phase 15 [resource capability contract](../phase-15-resource-capability-contract.md)
and [acceptance report](phase-15.md) are the protocol inputs. The work-orders
repository started clean at `f0c87d7aea301dddb38ac4a3435431660b8b7632`.
It remains an independent Rust application and business database; no platform
implementation crate, identity table access or top-level workspace was added.
The canonical work-orders `0001` and lockfiles were not changed.

This phase covers the typed private resource adapter and actual independent
PostgreSQL business operations. Its authority HTTP fixtures are explicit test
adapters; they are not running identity/platform services or a TLS acceptance
environment. Full current-version canonical/TLS interoperability remains the
Phase 25 gate. No old report is used as a current-version pass.

## Resulting behavior

- Only POST `/api/apps/work-orders/authorize` with an independent APP_RESOURCE
  Basic credential and bounded `token/resource/action` form is used. The reply
  must be `resource-authorization/1`, target the exact work-orders resource and
  audience, and match the requested ticket action. There is no legacy
  introspection/context runtime path or automatic fallback.
- USER reads internal IDs and positive original member/account versions from
  the private EMPLOYEE `principal`. Its context must agree on user, tenant,
  selected organization and target application, and still satisfy current
  operation permission, independent data grant and hard organization ceiling.
  Public `sub` remains opaque and is never a business database key.
- External access tokens require exact `PLATFORM_IDENTITY_ISSUER` and the
  current operation's `PLATFORM_RESOURCE_SCOPE_BINDINGS` entry from the same
  reviewed deployment mapping as platform_resource_bindings. Missing mappings,
  wrong scope/use/audience/issuer, duplicate JSON keys, unknown operations and
  malformed scopes fail closed. Internal app_session uses its fixed private
  issuer, work-orders audience, original jti and exact app:access domain.
- MACHINE has a separate client/credential/delegation actor, no fabricated user
  or member, and no dummy organization for TENANT scope. Only approved read and
  export are accepted. ORG, ORG_SUBTREE and TENANT predicates remain local SQL
  boundaries; SELF, personnel directories, creation, update, delete and
  reminders are denied. List/count/detail/CSV use the same scoped predicate.
- Member list and single-reference checks use Basic + Form POST
  `/internal/platform/apps/work-orders/members[/userId]`, never bearer GET or
  token query parameters. The resource-directory/1 wrapper and member DTO are
  independently checked. Its server expiry and original pre-HTTP window are
  combined by their minimum; responses and business-lock waits cannot renew
  the three-second witness.
- Personnel writes repeat online authorization using the same original token
  after receipt/member preparation and before opening a short business
  transaction. A changed actor, original versions, authority/projection or
  data scope is rejected; only an earlier deadline may survive. HTTP never
  holds a business connection or row lock. Local version CAS, transaction
  rollback, idempotency domains and unknown-result recovery remain in force.
  This is an authorization checkpoint, not an atomic cross-service commit.
- Internal USER app_session reminders retain their independent ticket-read
  and notification-publish checks and durable original event. Canonical
  external USER reminders explicitly return 403 before creating an intent or
  publishing: the current public notification API accepts internal app_session
  only. Phase 18 must add the private notification publication boundary.
  MACHINE reminders remain denied as a personnel operation.

## Acceptance self-check

| Acceptance criterion | Result | Current evidence |
|---|---|---|
| Valid employee source completes representative business flows within its original scope | PASS | Actual independent PostgreSQL create/idempotent replay, update/status/version races, directory assignment, scoped list/detail and CSV cases passed through the current typed HTTP adapter. |
| Cross-organization expansion, scope expansion, inactive source, wrong token_use/audience are rejected | PASS | Private principal/opaque-sub matrices, explicit scope-binding negatives, tenant/hierarchy/SELF ceilings, next-request revocation and a second-authorize 401/503 before writing passed. |
| Machine authorization uses only explicitly approved data scope | PASS | Actual read/export SQL for ORG/SUBTREE/TENANT; wrong audience/action, absent delegation, SELF/person-field substitution, personnel/write/reminder calls and invalid scope mappings were denied without business side effects. |
| Independent database, connections and temporary test resources leave no residue | PASS | Every owned fixture cleanup, including failed and retried attempts, confirmed database removal and pool closure; final prefix database/connection counts were zero. No roles or containers were created. |

## Verification

The [component evidence JSON](phase-17-component-validation.json) records the
current changed-file SHA-256 values, baseline, test counts, evidence layer and
resource boundary. It deliberately contains no credentials, tokens, cookies,
private environment values or successful SQL-generated resource capability.

| Verification | Result |
|---|---|
| cargo fmt --all -- --check | PASS |
| cargo check --locked --all-targets | PASS |
| cargo clippy --locked --all-targets -- -D warnings | PASS |
| cargo test --locked --all-targets | PASS: 75 library + 6 startup/CLI tests |
| cargo test --locked --doc | PASS: 1 |
| Explicit native-WSL PostgreSQL member_contracts | PASS: 14/14 |
| Explicit native-WSL PostgreSQL read_contracts | PASS: 15/15 |
| Explicit native-WSL PostgreSQL write_contracts | PASS: 16/16 |
| Explicit native-WSL PostgreSQL reminder_contracts | PASS: 13/13 |
| Current documentation links and whitespace | PASS: 22 file links; git diff --check |

After the final directory-wrapper helper was fixed, root independently reran
member_contracts against the native task PostgreSQL: 14/14 passed, zero ignored,
4.29 seconds. This is a replay of the same 14 member cases, not 14 additional
unique business cases. The command was `cargo test --locked --manifest-path
/mnt/d/WorkSpace/rust/rust-saas/saas-work-orders/backend/Cargo.toml --test
member_contracts -- --ignored --test-threads=2`, using the same guarded native
environment and existing task/target-linux cache.

All 58 PostgreSQL business cases were explicitly executed; the normal ignored
listing is not their pass evidence. The three unchanged migration-only ignored
cases were not executed in this authorization phase. Every business fixture
does initialize the current fresh canonical 0001.

Windows native Rust fixture HTTP could not accept a local connection in one
focused attempt, before its authorization assertions; cleanup still succeeded.
Unit HTTP tests therefore used the repository's explicit owned Node listener
adapter. Business PostgreSQL/HTTP suites ran natively in WSL, with no Docker
fallback. Intermediate failures from old fixture response/error shapes were
corrected to the actual versioned contract; final assertions retain denial,
rollback, no-publication and no-side-effect requirements.

The suites cover original idempotent create/response-loss recovery, version and
status transitions, deleted/invisible records, member display snapshots and
expiry, network waits without business locks/connections, source failure,
request/SQL/lock deadlines, CSV row/byte/formula limits, literal search,
pagination and consistent list/count/export visibility. The wire unit matrices
also reject wrong directory version/kind, unknown fields, expired or excessive
deadlines and any attempt to renew the original witness window.

## Reproduction and resource record

Run ordinary checks from saas-work-orders/backend. For Windows unit fixture
HTTP, explicitly set WORK_ORDERS_TEST_NODE to the installed Node executable;
this affects the test listener only. For real business PostgreSQL cases, first
verify the explicitly task-owned cluster. This run used loopback port 55494,
system identifier 7701548153213487841, maintenance database postgres and the
matching CI_TEST_POSTGRES_OWNER / WORK_ORDERS_TEST_LOCAL_POSTGRES_OWNER markers.
No existing business database or the Phase 19 manifest environment was used.

```sh
cargo test --locked --test member_contracts --test read_contracts \
  --test write_contracts --test reminder_contracts \
  -- --ignored --test-threads=2
```

The existing task-native PostgreSQL and task/target-linux build cache were
reused. Each fixture creates a unique independent database, closes every pool
and drops its own database even after failures. Final owned-prefix database and
connection counts were zero. Created/reused Docker containers: 0/0; created
Docker images, volumes and networks: 0. No new role, service, environment,
additional target cache or global cleanup operation was created. The existing
shared task PG/build cache remains root-owned for subsequent stages and final
cleanup; no Windows WSL/VHDX space recovery is claimed.

Both root and the phase owner independently queried the final catalog after
the replay: the cluster matched 7701548153213487841, and databases plus active
connections matching `^work_orders_phase(17|18|19|20)_` were each zero.

## Suggested task ledger update

Phase 17: COMPLETED. Record canonical-only typed USER/MACHINE authorization,
explicit issuer/scope mapping, private employee directory, no-lock online write
checkpoint and actual 58-case business PostgreSQL evidence with cleanup.
Keep Phase 18's external USER notification publication work explicit; do not
claim Phase 25 full-version TLS interoperability or deployment completion.
