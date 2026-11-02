# Phase 12 — unified client business context selection

Date: 2026-10-28  
Status: COMPLETED

## Inputs and boundaries

- Phase 8–10 canonical OAuth grants, source-bound code redemption and refresh families; the current context-selection contract; Phase 11 MACHINE grant boundary.
- Verification covered `saas-identity` and `saas-plateform/backend`. Both repositories already contained uncommitted implementation work from earlier phases; it was preserved.
- No existing business database, production service, gateway, signing key or remote environment was changed. The `postgres` administration database was used only on a task-owned native PostgreSQL cluster to create and drop uniquely named fixture databases.
- This phase implements the identity-side context interaction and one-time result contract. It does not complete the full real HTTPS BFF login path, which is Phase 13/14 work.

## Delivered changes

- The canonical `/oauth/*` flow binds context interactions to the RP, exact callback, state/nonce hashes, PKCE challenge, prompt/max-age policy and prior session. Source validation rechecks the original SSO session, account and RP generations, grant and access-token source before choices, confirmation and consumption.
- `ACCOUNT_ONLY` flows avoid platform membership calls. A managed `PLATFORM_CONTEXT` RP cannot get a business context without the registered client purpose, membership type, application admission and current member proof. Search changes the displayed candidates only; it does not reduce the server-side candidate count or auto-confirm.
- Confirmation is one-time and source-bound. Repeating an uncertain confirmation restores the same selection only when its transaction, source, member, RP/application policy snapshots and original deadlines still match. The credential is stored as a digest at the platform; the short-lived recovery copy is authenticated encryption under the platform BFF keyring and is cleared on consumption.
- Code replay, source/session revocation, changed account/client/application/member generations and duplicate consumption cannot establish a second context. Refresh does not redeliver the one-time selection result.
- The browser fixture now exercises the canonical `/oauth` interaction and current `/api/sso/consents` path. It covers account/consent progression, zero/one/multiple memberships, customer use, reload, search, revoked consent history and mobile rendering.
- Platform canonical `0001` adds the encrypted credential recovery column and constraints against replacing it or retaining it after consumption. Platform registration and service authorization distinguish employee and customer client purposes and constrain customer consumers to explicitly allowed clients.

## Acceptance self-check

| Acceptance criterion | Result | Evidence |
|---|---|---|
| ACCOUNT_ONLY skips platform membership services; PLATFORM_CONTEXT cannot bypass business qualification | PASS | Identity PostgreSQL HTTP regression `phase12_unified_context_selection_is_source_bound_recoverable_and_one_time` verifies zero platform calls for account mode and denies an ineligible managed-client context. |
| Zero/one/multiple tenants and organizations, employee/customer use, and search without auto-confirm are verified | PASS | `saas-identity/artifacts/context-selection/report.json` records 11 passing Chromium scenarios, including `multiple-tenants-and-organizations-require-choice`, `no-members-never-auto-approves`, `customer-and-hostile-names-render-as-text` and `remembered-approval-survives-reload-search-does-not-auto-select`. |
| Interaction replacement, source revocation, repeated issue/consume, and wrong client or purpose are rejected | PASS | Identity PostgreSQL HTTP regression covers changed interaction/source and code replay; platform regressions cover wrong service/client, employee/customer purpose mismatch, source/member generation changes and one-time consumption. |
| Timeout recovery returns the original result without reissue; source deadline and generations are checked before commit | PASS | Platform database test `real_postgres_selection_is_single_use_and_transactional`, customer recovery/switch test `centralized_customer_consumption_recovery_and_switch_are_exact` and employee callback test `centralized_callback_commits_selected_employee_and_rejects_replay` pass against isolated fresh databases. |

## Verification

- `saas-identity`: `cargo fmt --all -- --check`, `cargo check --all-targets`, and the Phase 12 identity PostgreSQL HTTP regression passed.
- `saas-identity`: Chromium report contains 11/11 passing scenarios. Search still leaves the confirmation action unsubmitted until the user chooses an actual result.
- `saas-plateform/backend`: `cargo fmt --all -- --check`, `cargo check --all-targets`, `git diff --check`, `real_platform_selection_http_is_member_and_application_scoped`, `real_postgres_selection_is_single_use_and_transactional`, `centralized_customer_consumption_recovery_and_switch_are_exact`, `centralized_callback_commits_selected_employee_and_rejects_replay`, and `phase12_real_roles_enforce_mixed_user_columns_and_no_ddl_owner_escape` passed.
- One employee callback invocation using `localhost` did not reach the task-owned listener. The same test passed when rerun with explicit `127.0.0.1`; no test assertion failed on the successful run.
- The Playwright fixture is a browser-level contract test, not a real external HTTPS deployment. Real HTTPS BFF session establishment and renewal remain Phase 13/14 acceptance work.

## Temporary resources and cleanup

- No Docker container was created or reused. The WSL Docker context was `default`; `docker system df` before and after showed 0 images, containers, volumes and build cache. No Docker volume or network was created.
- One task-owned native PostgreSQL 18.6 cluster ran in WSL at `/tmp/rust-saas-phase12-context`, port `55493`, separate from the existing PostgreSQL service on port `5432`. The only databases present before cluster shutdown were the built-in `postgres`, `template0` and `template1`; all test-created fixture databases had been dropped.
- The cluster was stopped through its cleanup sentinel, its task-owned data/log/socket directory was removed, and port `55493` stopped accepting connections. The directory occupied 56 MiB immediately before cleanup. The existing PostgreSQL service on port `5432` was not changed.
- Cargo build artifacts were retained for reuse. No WSL VHDX compaction was attempted; Windows host disk recovery is not claimed.

## Risks and remaining boundaries

- The source/choice/consume HTTP fixtures use real PostgreSQL and verify database permissions, but a real HTTPS connection between deployed identity and platform services is not exercised here.
- Phase 13 and 14 must preserve the server-side recovery boundary while establishing employee and customer BFF sessions in their own session stores.

