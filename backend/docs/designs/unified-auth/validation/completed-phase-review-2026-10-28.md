# Review of completed Phases 1–12

Date: 2026-10-28. This review accompanies execution of all remaining phases.
It rechecks completed work against current source and reruns focused tests;
earlier reports remain historical evidence for their stated versions.

## Baseline and scope

| Repository | Input HEAD | Initial state |
|---|---|---|
| saas-identity | `7025a0a39cff5577bc3aabf239aaae8df529714e` | Existing Phase 1–12 uncommitted source, tests and documents preserved |
| saas-plateform | `34105757b040165daa5477bf6afff9637f617466` | Existing Phase 1–12 uncommitted source, tests and documents preserved |
| saas-customer-portal | `648e71e082929605c67e4e9f7e19fc9ecdbd89d4` | Clean |
| saas-stores | `01ea4eb30801e89429b4a4f1469cc0792f17b1f9` | Clean |
| saas-work-orders | `f0c87d7aea301dddb38ac4a3435431660b8b7632` | Clean |
| saas-notification | `6a16d74be2e079892d2528f68058c9556ffc3b37` | Clean |
| saas-micro-app-sdk | `155f5cc04f98509f5eee3e092f716d4d8f5013d8` | Clean |
| saas-examples | `f22ee4f0fe601b94a0f38783d3a76ef113ee47b0` | Clean |

The review read the workspace/parent engineering rules, ledger, implementation
contract, callsite inventory, reports and affected handlers, database functions,
role grants and browser consumers. It found no basis to treat Phase 13–26 as
completed or to count the existing local development deployment as this
version's HTTPS evidence.

## Findings and repairs

| Phase | Finding | Repair and regression |
|---|---|---|
| 3 | Private application projection could change ownership/partner in place, including changing a third-party projection into an approved first-party projection | Freeze the existing ownership and partner binding; PostgreSQL command regression. This was a private-protocol invariant gap, not a demonstrated ordinary-user endpoint bypass. |
| 4/5 | A browser could lose a committed management response without retaining the server-generated operation ID, then dispatch a new create/rotation | Browser creates and retains UUIDv7 before dispatch; platform binds actor/session/method/path/body to that ID; network-unknown state queries that original operation. Browser connection-loss regression verifies one mutation and no secret persistence. |
| 4 | A client could not be disabled/retired after application approval removed its original scopes | Permit a pure terminal restriction without requiring withdrawn permissions to remain approved, while rejecting expansion/reactivation; PostgreSQL regression. |
| 5 | PLATFORM_CONTEXT form accepted SPA/NATIVE or both employee/customer despite stricter server/schema contract | Match the form schema and controls to CONFIDENTIAL WEB_BFF with exactly one member type; actual form-schema regression. |
| 10 | Replaying a consumed refresh token after its own expiry did not revoke a still-live family | Handle a verified consumed token through the existing revocation transaction before token expiry/permission checks; PostgreSQL RED → GREEN regression. |
| 10 | Expiry of an original scope permission could mask consumed-token replay while a narrower successor remained live | The same revocation-first path invalidates the family and successor access; separate PostgreSQL RED → GREEN regression. |
| 12 | Context/client error branches redirected before checking the registered callback | Check exact callback registration before every callback error branch; PostgreSQL RED → GREEN coverage for USER context and MACHINE clients. |
| 12/13 | Platform context-service authentication directly queried the identity-owned credential table, which a correctly isolated platform runtime cannot read | Use a narrow identity-owned boolean capability for digest-reuse rejection; retain direct-table denial and test with real LOGIN roles. |
| 7 | Canonical command-intent and identity-receipt path constraints omitted consent-group creation and membership commands; simplified identity fixtures did not expose the failure | Add only the two exact unified consent-group command forms to both canonical constraints and test accepted/rejected paths against the fresh baseline. |
| 2/8 | The shared user baseline defaulted the account generation to zero, while canonical requests and subjects require a positive generation; fixtures explicitly seeded one and hid the new-account failure | Start new accounts at generation one, keep create's expected zero as the absence precondition, and cover actual account creation before canonical authorization. No existing database is rewritten. |
| 12/13 | The actual platform source HTTP adapter still used the legacy private target for AUTH registrations; a real HTTPS login failed despite fake source-port tests passing | Choose the fixed canonical source endpoint from the registered registry and reject a mismatched registry without fallback; real HTTP-target regression and the subsequent HTTPS run cover this boundary. |
| 2/12/13 | Canonical source revalidation used `SELECT ... FOR SHARE` on a service registry granted SELECT only; PostgreSQL also requires UPDATE permission for that row lock | Keep runtime registration immutable and introduce a narrow owner lock function. The [real HTTPS runtime failure](phase-13-https-browser-runtime-acl-red.json) preserves the discovered boundary; final successful role/HTTPS verification remains required. |

The context/employee BFF SQL source functions still using `oidc_*` were an
expected Phase 13 starting boundary. They are being replaced and tested as part
of Phase 13 under the [implementation contract](../implementation-contract.md), rather than counted as evidence
that the earlier consumer migration was complete.

## Current verification

| Check | Result and evidence boundary |
|---|---|
| Identity ordinary unit tests | 54 passed; database tests remain explicitly ignored by default |
| Phase 6 pairwise subject PostgreSQL suite | 1/1 passed |
| Phase 7 consent/group PostgreSQL suites | 3/3 passed |
| Phase 8–12 canonical protocol PostgreSQL/HTTP suites | 9/9 passed, including the three new security regressions |
| Identity format, all-target check, strict all-target Clippy and documentation | Passed with the current fixes |
| Canonical schema/source and real LOGIN-role regressions | 8/8 passed, including digest-reuse HTTP checks and positive default account generation; canonical production bootstrap 1/1 passed |
| Unified client browser regression | [10/10 passed](completed-phase-review-browser.json), including complete response loss, page reload and original-operation/session ownership recovery |
| Unified client form and pending-operation contract | 8 targeted cases passed; full frontend suite 440/440, TypeScript and production build passed |
| Platform remote management regression | 20/20 passed, including 5 real PostgreSQL tests for intents, receipt paths, unknown outcomes and cross-session rejection |
| Documentation links | All relative links in `unified-auth/` resolved at review time |

Identity commands included `cargo test --locked --lib`, exact ignored
`util::audit_tests::auth_protocol` / subject / consent tests,
`cargo fmt --all -- --check`, `cargo check --locked --all-targets`,
`cargo clippy --locked --all-targets -- -D warnings` and
`cargo doc --locked --no-deps`. All ignored PostgreSQL tests used the explicit
task-owned loopback cluster on port 55494 and created/dropped their own fixture
databases. Their schema fixtures do not replace canonical migration/ACL checks.

One overly broad `cargo test phase12_ -- --ignored` invocation also selected
three older tests sealed to another cluster identifier. Those tests refused the
new cluster before creating data. The exact canonical-context tests were then
run successfully; the broad invocation is not recorded as a pass.

## Remaining gates

- Full normal TOTP login, recovery-code login and disable-route workflows,
  including loss of the enrollment confirmation response, remain explicit
  Phase 19/25 coverage. This review did not invent a new recovery policy.
- Browser fixtures and embedded transitional BFF-authority adapters cannot
  establish real canonical identity/platform HTTPS interoperability. Phase 13
  and 14 retain that separate acceptance requirement.
- Fine-grained partner administration, production approval and recent TOTP
  step-up belong to Phase 19. Existing `system:oauth` checks do not satisfy it.
- Ledger metadata is synchronized: the duplicated unchecked Phase 1 item was
  removed, Phase 5/6 detail checkboxes now reflect the current evidence, and the
  stale closing pointer was corrected. The user's explicit parallel instruction
  is recorded for the active dependency-eligible phases.
- The user identified the eventual target as the local machine with
  `auth.saas.test` and `console.saas.test`. Exact databases, affected services,
  operational responsibilities and a concrete switch/recovery checklist remain
  Phase 25/26 gates; this does not authorize resetting an existing database.

## Resources and cleanup

No Docker containers, images, volumes, networks or build cache were created or
reused for this review. The shared native PostgreSQL task cluster remains live
for subsequent phase verification and has an ownership-checked exit cleanup
sentinel. Individual test databases, LOGIN roles and temporary RSA fixtures were
removed by their fixture cleanup paths.

The previously reported Phase 7 temporary screenshot directories were inspected
and removed: 12 known files totaling 1,753,357 bytes and two empty directories.
A batch recursive cleanup was rejected by automatic policy review; the successful
cleanup enumerated the exact files and removed them without recursion. Formal
phase reports and current repository artifacts were retained. No WSL disk
compaction or Windows disk-space restoration is claimed.

The final current-review browser result is retained as sanitized JSON in this
directory. Its seven temporary report/screenshot files (905,046 bytes) were
verified and deleted individually after preserving that formal result.

Pending management-operation recovery stores only a seven-field metadata
allowlist in sessionStorage, partitioned by the proven account, session and
business context. It persists no body, token, Cookie, verifier or secret. The
operation is recorded before dispatch, survives reload, blocks new writes until
resolved, and is removed only on a terminal result. The server binds lookup to
the original actor/session. Initial operation acceptance uses the UUIDv7 time
within the database's -30/+60-second window; acceptance and absence lookup share
an advisory transaction lock. A late request cannot execute after a synchronized
absence check has conclusively reported NOT_APPLIED.
