# Phase 06 — Pairwise subject and profile projection

Date: 2026-10-28  
Status: COMPLETED

## Inputs and baseline

- Phase 1 subject/profile contract and Q2/Q4 policy decisions; Phase 2 canonical `auth_subject_sectors` and `auth_subject_links` model; Phase 3 application projection; current trusted `AccountSessionProof` and account-retirement command.
- Repositories: `saas-plateform` HEAD `34105757b040165daa5477bf6afff9637f617466`; `saas-identity` HEAD `7025a0a39cff5577bc3aabf239aaae8df529714e`. Existing changes from earlier phases were retained.
- No issuer, gateway, signing-key deployment, production database or account data was selected.

## Deliverables

- Added `oidc::subjects::profile_for_client`, which accepts the existing crate-private `AccountSessionProof` and a stored client ID. It locks account/security generation, application projection, client policy, sector and link in the accepted order; verifies the account generation and tombstone, enabled USER client, and APPROVED application; and refuses MACHINE, unreviewed, suspended or retired-sector requests.
- A subject sector is created only for an approved application/environment and is terminal after retirement. The persisted subject is 256 bits of OS-random material, independent of names, account credentials and signing keys. `UNIQUE(user_id, sector_id)` makes clients in the same app/environment converge on one sub; uniqueness by sector prevents a collision from being shared with another account. Concurrent first use converges on one sector/link.
- Added `OidcProfile` claim builders shared by ID Token and UserInfo. `openid` is required; `profile` adds a nonempty current `name` and an exact HTTPS `picture`. The projection does not emit internal user IDs, usernames, email, phone, verification flags, membership, roles or other unimplemented claims.
- Account retirement now marks all existing subject links retired in the same transaction as account disablement and tombstone creation. A replacement account has a new internal ID and receives a newly generated sub; it cannot inherit an earlier mapping.
- Kept the new claim builders separate from the current UUID-based legacy `/oidc/*` path. Phase 8/9 will wire the shared projection into the unified protocol after consumer migration work; Phase 13/14 replace BFF assumptions that `sub` is a UUID.

## Self-check

| Acceptance criterion | Result | Notes |
|---|---|---|
| Same sector is stable; other applications/environments are isolated; unreviewed requests are denied | PASS | PostgreSQL route fixture verifies sibling clients share one sub, different app/environment and different accounts get different subs, and a PENDING_REVIEW application cannot create a sector. A concurrent first-use pair created exactly one sector/link. |
| ID Token and UserInfo projection share the same sub; internal UUID stays private | PASS | Both claim builders use the same opaque `PairwiseSubject`. Real fixture assertions confirm equal sub and no internal user ID in public claims. |
| Unconsented/unimplemented fields are absent; verification is not fabricated | PASS | `openid` alone returns only `sub`; `profile` is required for `name`/`picture`; invalid non-HTTPS pictures are omitted. Email, phone, verified flags, username, membership and role claims are absent. |
| Retired account mappings are not inherited by a new account | PASS | The actual internal account retirement route writes the tombstone and retires all three existing sector links in the same transaction. The retired proof is denied; a new account in the same sector receives a different sub. |

## Verification evidence

- PASS: `cargo check --locked`.
- PASS: `cargo fmt --check --all` and `git diff --check` (only existing Windows line-ending advisory notices).
- PASS: `cargo test --locked` — 46 passed, 9 integration tests ignored by their explicit-PostgreSQL requirements.
- PASS: `cargo test --locked oidc::subjects::tests:: -- --nocapture` — two claim/picture unit tests.
- PASS: `cargo test --locked phase06_pairwise_subjects_are_sector_scoped_stable_and_retired_with_accounts -- --ignored --nocapture` — one real PostgreSQL 18.6 integration test. It covers concurrent first creation, account/application/client locks, projection gating, claim contents, rename/key rotation stability, account retirement and sector retirement.
- The real PostgreSQL test used the identity repository's task-owned `phase04` fixture schema and actual account-command route. It does not claim a full canonical `0001` migration run; that schema/ACL baseline is separately verified in Phase 2. The approved third-party row was inserted as a test fixture because Q3/Phase 19 still gate production approval.

## Resources and cleanup

- Reused the existing WSL PostgreSQL 18.6 service. The test created one uniquely marked `identity_phase04_clients_*` database and one temporary `identity_phase06_admin_*` LOGIN/CREATEDB role. The Rust fixture removed its database; the PowerShell `finally` path removed the exact temporary role. Post-run counts for both prefixes were zero.
- Containers created/reused: 0. Docker was not used; no Docker resources, logs, images, volumes, networks or build caches were created. No Docker or WSL disk compaction was attempted.
- The generated database password remained in process memory and was not printed or written to the workspace.

## Risks / notes

The current public legacy OIDC route and BFF are deliberately not switched to opaque sub in this phase. Directly switching them would break consumers that still parse `sub` as an internal UUID; those consumers are Phase 13/14 work. The profile builder accepts an already consented scope set, while durable consent is implemented in Phase 7. Real third-party approval remains blocked by Q3 until Phase 19.

Phase 6 is complete. The next dependency-eligible phase is Phase 7: consent groups and user authorization management.
