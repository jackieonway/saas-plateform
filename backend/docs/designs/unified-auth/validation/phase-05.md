# Phase 05 — Unified client management page

Date: 2026-10-28  
Status: COMPLETED

## Inputs and baseline

- Phase 4 unified client management API and DTOs, including stable paging, ownership metadata, expected-version writes, one-time credential rotation and operation lookup.
- Repository: `saas-plateform` HEAD `34105757b040165daa5477bf6afff9637f617466`. Earlier phase edits and the user-provided design/ledger inputs were already present and retained.
- Only the platform frontend and its browser fixture/report files were changed for this phase. No business database, identity deployment, gateway or live credential was selected.

## Deliverables

- Replaced the previous business OAuth/OIDC tabs at `/system/oauth/clients` with one unified registry using only `/api/identity/clients` and the Phase 4 application projection API. The table and mobile cards show the registered application, ownership, partner, environment, subject/runtime, policy and version information.
- Added URL-restored filters for application UUID, partner UUID, ownership, environment, PUBLIC/CONFIDENTIAL, USER/MACHINE and status, plus server pagination. Previous successful rows remain visible after a failed refresh.
- Added one typed create/edit form. It derives client type and token authentication from runtime, limits USER/MACHINE and context combinations, checks exact HTTPS callback URLs and required OIDC scope, and rejects `openid` on MACHINE clients. MACHINE selection switches to SERVICE/NONE and requires a resource-specific machine scope.
- Added optimistic-version conflict recovery: a 409 preserves the draft, disables resubmission and requires an explicit latest-version reload that names the draft being discarded. Same-frame submissions are guarded by a synchronous write lock.
- Added one-time secret display in a non-query response path. Unknown writes close open editor/confirmation dialogs and expose operation lookup; a confirmed `NOT_APPLIED` result can reopen the original create/edit draft. Secret values are held only in component memory and are not written to URL, local/session storage or TanStack Query cache.
- Removed the obsolete OIDC client panel from the management route; retained the `ClientsPage` module path as a one-line route alias for compatibility with the existing lazy route.
- Added form contract unit coverage and an isolated Chromium browser suite for the unified page. The existing admission-policy browser suite now limits its scope to `ApplicationsPage`.

## Self-check

| Acceptance criterion | Result | Notes |
|---|---|---|
| Platform, portal and third-party clients are browsed in one registry; filters and page state restore | PASS | Browser fixture includes platform console, customer portal and third-party applications through one list endpoint. URL restoration, ownership/environment filters, detail view and server offsets were checked. |
| PUBLIC/CONFIDENTIAL, USER/MACHINE and context combinations cannot submit invalid values | PASS | Zod schema unit tests cover valid USER web/SPA/native and MACHINE service contracts plus illegal combinations. Chromium verified that a 390px MACHINE create submits SERVICE/NONE, `client_credentials`, no member types and no redirects. |
| Read failures retain the last good configuration; duplicate writes and version conflicts are handled | PASS | Browser fixture forces a failed refresh and verifies the previous rows remain, sends two synchronous submit events and observes one PATCH, then returns a 409 and verifies draft preservation, blocked save and explicit latest-version reload. |
| Desktop and 390px layouts expose the full form/actions without persistent secret storage | PASS | Chromium 156.0.8078.4 / Playwright 1.64.1 verified internal form scrolling and a visible fixed action bar at 390×844, no horizontal page overflow, successful mobile create, recoverable operation query, one credential write and no secret in URL/storage/Query cache. |

## Verification evidence

- PASS: `npm run lint` — zero errors; 45 warnings remain in existing unrelated frontend files. The changed unified page and API modules produced no lint warnings.
- PASS: `npm run typecheck`.
- PASS: `npm test` — 431 tests passed.
- PASS: `npm run build` — Vite production build and `check-qiankun-delivery.mjs` passed (403 delivery checks; SDK 3.0.0).
- PASS: `npm run test:identity:clients:browser` — six isolated Chromium scenarios: directory filters/pagination and first-party/third-party sources; 390px scrolling and MACHINE create; failed refresh retention; same-frame duplicate submit; 409 conflict reload; unknown credential-result recovery.
- PASS: `npm run test:admission:policy:browser` — one `ApplicationsPage` session/cache invalidation scenario after removing obsolete client-panel coverage.
- PASS: `node --test tests/oauth-client-policy.mjs` — three form contract tests.
- `git diff --check` reports no whitespace errors after the browser-suite update; Git may print Windows CRLF/LF advisory notices for existing files.

## Resources and cleanup

- Projects used: `saas-plateform/frontend` only. Local Node 24.22.1, npm 11.19.1, the existing Playwright 1.64.1 installation and project-cached Chromium 156.0.8078.4 were reused.
- Containers created/reused: 0. Docker and WSL PostgreSQL were not used. No Docker resources, volumes, networks, images, logs or build caches were created; Docker disk usage did not change.
- The browser runner started one Vite process per run and terminated it in its `finally` cleanup. Browser JSON and desktop/mobile screenshots are under the ignored `frontend/target/unified-clients-browser/` report folder.

## Risks / notes

Browser HTTP fixtures establish page behavior, not live identity-service connectivity, database policy enforcement or production third-party approval. Third-party records shown in the fixture are approved examples only; production reviewer binding and intake remain subject to Q3 and Phase 19. Existing `/api/oauth/clients` calls remain in other token/session surfaces until their later ledger phases; this page no longer calls the old client registries.

Phase 5 is complete. The next dependency-eligible phase is Phase 6: pairwise subject and profile projection.
