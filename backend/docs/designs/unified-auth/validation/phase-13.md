# Phase 13 — Management platform BFF

Date: 2026-10-28  
Status: COMPLETED

## Inputs and boundary

The inputs are the reviewed Phase 10 refresh and Phase 12 context-selection
contracts, canonical `auth_*` grants, and the platform BFF/session/frontend
consumers. Input HEADs are identity
`7025a0a39cff5577bc3aabf239aaae8df529714e` and platform
`34105757b040165daa5477bf6afff9637f617466`; existing uncommitted Phase 1–12
changes were retained. The [completed-phase review](completed-phase-review-2026-10-28.md)
records repairs found while checking the dependencies.

This phase establishes employee sessions. It does not alter the portal Cookie,
provide a password fallback, complete the resource capability contract, or
declare the retained old logout path retired. Phase 14 and 19 are proceeding in
parallel under the user's explicit instruction, with separate acceptance.

## Changes

- Platform login and context switch use the pinned issuer's canonical
  `/oauth/authorize` and request only `openid profile`. Discovery, token,
  UserInfo, JWKS, revoke and advertised endpoint checks use canonical paths.
- Browser navigation rejects old `/oidc/authorize` and foreign/invalid targets.
  Upstream access/refresh credentials remain encrypted on the server.
- The public pairwise subject is a string. A dedicated authenticated private
  `platform-account-proof/1` supplies internal account/session/grant IDs; its
  issuer, client, purpose, member type, scopes, generations and deadlines are
  checked before use. ID Token and UserInfo must agree with its opaque subject.
- Private proof/context requests use the configured private identity origin
  and CA. Deployment examples distinguish the public issuer from that private
  origin and require an independent source service credential.
- The canonical employee SQL source fence checks account/SSO, grant,
  client/application, consent/group, original context/request, resource
  permissions and online refresh family. The selected employee member is bound
  to the grant without granting identity access to platform membership tables.
- Context-service credential reuse checks use a narrow identity-owned boolean
  function. The platform runtime retains no direct access to identity secrets.
- Refresh checks its verified source expiry after waiting for locks, before
  replacing encrypted credentials and again before commit. An expired or
  uncertain response cannot update the current session or replay old refresh.
- New canonical accounts start at generation one. CREATE's expected zero means
  absence; its receipt is exactly `0 → 1`. Both a real private CREATE handler
  and canonical migration/production bootstrap reproduced the old defect and
  pass with the repair.
- The real context-source HTTP adapter selects the fixed canonical target for
  AUTH registrations and validates that the returned registry matches. A
  rejection never triggers a legacy retry.

## Acceptance self-check

| Acceptance criterion | Result | Current evidence / remaining gate |
|---|---|---|
| Administrator and ordinary employee complete login, session restore and online renewal in a fresh environment | PASS | Real TLS 1.3 browser login, actual host reload/session restore and access-boundary refresh-family rotation passed; a freshly API-created employee also completed canonical login. |
| Failed code/selection leaves no partial employee session; customers cannot establish employee sessions | PASS | PostgreSQL BFF rollback/replay tests passed; the real browser CUSTOMER case reached the genuine empty-employee-candidate state with no BFF session. |
| Old tabs/late responses cannot replace newer versions; revoked origins are rejected | PASS | BFF concurrent-flow/lease/late-response transaction regressions, canonical original-source matrix and real HTTPS organization switch/source-revoke/other-account isolation passed. |
| Browser holds no upstream refresh; configuration has no old login fallback | PASS | Strict discovery/private-proof/navigation checks plus real browser host-only Secure/HttpOnly Cookies, secret-free local/session storage and no browser token-endpoint call passed. |

## Current verification

- New BFF private-proof/endpoint tests: 3/3 passed.
- Frontend BFF tests: 39/39 passed; TypeScript, production build and delivery
  checks passed. The completed-phase review separately records the final
  440-test frontend management regression.
- Canonical schema and actual LOGIN-role/HTTP regressions: 8/8 passed, including
  source shrinkage, original member binding, privilege denial and digest-reuse
  rejection. Production bootstrap generation-one regression: 1/1 passed.
- BFF PostgreSQL transaction/concurrency suite: the first full run passed 14/15
  and exposed the lock-wait expiry defect. After repair the exact failing test
  passed. Its embedded provider/legacy projection adapters are explicitly
  test-only; these results do not prove real identity HTTPS interoperability.
- Platform all-target check, format, strict binary Clippy and focused receipt
  unit tests passed. Identity strict Clippy and CREATE PostgreSQL regression
  passed after the account-generation repair.
- The first actual HTTPS run reached SSO login over trusted TLS but failed at
  the still-legacy context-source target. That failure is retained as a real
  transport regression, rather than called a successful login.
- After correcting the source target, the real runtime exposed PostgreSQL's
  UPDATE requirement for `FOR SHARE` on the SELECT-only service registry. The
  owner lock projection and actual LOGIN-role/concurrent-revocation regression
  resolve that failure without widening runtime credential privileges.
- Final [real HTTPS browser result](phase-13-https-browser.json): **8/8 PASS**,
  Chromium 156.0.8078.4 / Playwright 1.64.1, TLS 1.3, no ignored TLS errors,
  no HTTP fixture, successful `finally` boundary reset. The final run contains
  no 503/409; the source-revocation case intentionally returns 401.
- The actual new-account command returned `beforeVersion=0 / afterVersion=1`,
  and the account at generation one completed canonical login without SQL
  version adjustment. User creation and member assignment used real APIs.
- Source snapshot and binary hashes are retained in the sanitized
  [build attestation](phase-13-build-attestation.json). Snapshot SHA-256 is
  `17d9fa7399571282d44f3d3ff5dd5e354773753bdebf514044b4d6f9cea0341a`.

The browser's controlled API and expiry-boundary requests use an actual
same-origin JSON probe page after the real host login/reload assertions. This
avoids introducing a second refresh coordinator beside React's manager. BFF
multi-consumer/lease behavior is covered separately by real PostgreSQL tests;
this run is not a capacity or long-running host renewal benchmark.

## Environment and resource handling

The owned native PostgreSQL cluster on loopback port 55494 is separate from the
existing service on port 5432. Only UUID-named fixture databases and role
prefixes are created; tests remove their own data/roles on both success and
failure. A fresh baseline is always used; changed checksums are never adopted.

Current-source Linux identity/platform binaries run in a private mount
namespace. Only that namespace maps the fixed local test domains to 127.0.0.2;
the existing hosts file, Windows 127.0.0.1 HTTPS listener and old application
processes are unchanged. The TLS gateway and Windows CONNECT proxy are scoped
to this task. Browser and server clients validate the trusted local CA and host
names; no TLS checks are disabled. The public gateway rejects `/internal/*`.
The private gateway allows registered internal calls and the exact identity
`GET/HEAD /health/ready` probe without a query.

The reusable scripts are in `backend/tests/unified-auth-https/`. Private keys,
passwords, environment files, logs and task build products stay in the owned
temporary directory. Supervisors validate PID/start time/namespace ownership
and have exit cleanup. Docker containers created/reused: 0; its initial
images/containers/volumes/cache usage was zero. The shared task environment is
still in use for Phase 14/19 verification; final shared-environment cleanup is
not yet claimed. All Phase 13 browser contexts/processes are closed; the second
fixture employee was disabled with the required identity-generation advance.
Earlier test databases/roles were removed before fresh baseline retries.

## Risks and next phase

The real transport, runtime ACL and fixture failures are retained as sanitized
RED reports in this directory. Fixture permission seeding and status-generation
updates were repaired without relaxing product authorization or disabling its
triggers. Later source changes require new evidence; this snapshot is not
automatically evidence for Phase 25's release candidate.

Phase 13 has no remaining acceptance work. Phase 15 can now implement the
resource/internal capability boundary while Phase 14 and 19 continue in parallel.
Global logout remains an explicitly tracked Phase 20/24 replacement; the
embedded legacy test adapters are tracked for Phase 24 removal. No production
service or existing database has been reset or switched.
