# Phase 14 — Customer portal BFF

Date: 2026-10-28  
Status: COMPLETED — real three-service HTTPS acceptance passed 10/10.

## Inputs and scope

Inputs are the reviewed Phase 10 online refresh and Phase 12 one-time selection
contracts. Portal input HEAD is `648e71e082929605c67e4e9f7e19fc9ecdbd89d4`
and was clean; identity/platform input HEADs and pre-existing changes are
recorded in the [completed-phase review](completed-phase-review-2026-10-28.md).
Implementation ran in parallel with Phase 13/19 under the user's instruction.

The portal retains its own database, Cookie, encryption keys, sessions and
CUSTOMER purpose. It does not read platform/account tables or inherit employee
permissions. This phase does not retire the remaining internal profile-service
registration or implement reliable global logout.

## Changes

- Canonical `/oauth/*`, exact discovery checks, `openid profile` online grants,
  and a distinct opaque public subject. Code requires a valid ID Token; refresh
  may omit it and is never introspected as an access token.
- Private customer account proof uses its own service credential and CA at
  `/internal/identity/auth/customer-account-proof`. Only the registered USER /
  CONFIDENTIAL / WEB_BFF / PLATFORM_CONTEXT CUSTOMER RP is accepted. The
  platform's reverse `CUSTOMER_ACCESS_PROOF` service is a separate direction and
  credential with an exact RP allowlist.
- The platform source fence now validates canonical access/grant/SSO/account,
  consent/group, client/application generations, original context/request,
  authentication time and exact source deadline. Employee, wrong tuple and
  deadline-extension cases fail closed. Production's local old authority fails
  closed; the remaining old adapter exists only under `cfg(test)`.
- Portal pending state remains AEAD-encrypted and bound to its original
  transaction, BFF ID/version and credential. Recovery queries the original
  result before consuming PENDING and cannot replace the source. Access/result
  deadlines are checked again at commit; refresh can only preserve or shorten
  its original absolute deadline.
- Authenticated account generations are positive. The portal retains anonymous
  version zero only for its unauthenticated state.
- Database fixture creation now cleans up failed connection/migration paths and
  verifies the task cluster. The reusable HTTPS preparation has an optional
  independent portal database/owner/runtime with no shared-database CONNECT.

## Acceptance self-check

| Acceptance criterion | Result | Evidence / remaining gate |
|---|---|---|
| Customer login, automatic single candidate, switch and own profile complete real HTTPS verification | PASS | Real canonical login, one CUSTOMER candidate, profile, actual refresh-family rotation and central organization switch passed. |
| Platform commit followed by local save failure restores the same capability without duplicate consume/context | PASS | The narrow portal trigger caused session/profile 503 only after the real platform commit. Original-result recovery retained the same capability digest, context ID, BFF session/version and expiry; consumption audit count remained 1. |
| Wrong BFF/version, employee purpose and revoked source are rejected | PASS | Real private binding probes returned 403/409/403/401 for wrong BFF/version/RP/source. Pure EMPLOYEE received no customer session; original source revocation returned session/profile 401. |
| Portal and employee sessions do not overwrite each other; cross-database temporary data is cleaned | PASS | Same-browser dual-member administrator reused one SSO session with separate host Cookies and EMPLOYEE/CUSTOMER contexts. Runtime cross-database CONNECT was denied. Browser contexts and fault objects closed, extra fixture membership disabled; shared environment remains explicitly in use by subsequent phases. |

## Component verification

- Portal backend: fmt, all-target/all-feature check and strict Clippy, 16
  ordinary tests, documentation build passed. Eight ignored PostgreSQL tests
  were explicitly executed and passed, including pending local-save failure,
  returned-snapshot races, uncertain refresh, shrinking absolute deadline,
  wrong local version and 401 without repeated consumption.
- Portal frontend: 56 tests, lint, typecheck and production build passed.
  Desktop/mobile production-bundle Chromium HTTP fixtures: 2/2 passed; these
  are not real identity-service HTTPS evidence.
- Platform: canonical fresh-schema / real LOGIN-role / source-lock/customer
  matrix 10/10 and customer reverse DTO unit tests 5/5 passed. The current
  Linux customer PostgreSQL suite passed 9/9 in 11.15 seconds, including the
  independent-connection account-share regression and pre-revoke → renewal →
  exact BFF/source preservation → switch → old-source renewal denial. All nine
  owned fixture databases/pools were deleted/closed; no roles or containers
  were created. That suite uses the new profile service registry but an
  explicitly test-only legacy source adapter, so it does not replace the
  canonical real HTTPS evidence. The initial component all-target check and
  strict Clippy gates also passed.
- Identity: customer proof PostgreSQL/HTTP 1/1, Phase 9 and Phase 12 regressions
  each 1/1, 54 ordinary tests, check/Clippy/fmt passed.
- AUTH source endpoint transport regression passed against a real HTTP listener
  and PostgreSQL in WSL Linux. Windows could not reach its local Rust listener;
  that Windows attempt is not reported as a pass.

## Fixed real-environment inputs

The successful fixed source snapshot is
`1ce73c2a960bc2b9acfcb760a1fea86d04b671a0dab3b5f5b3c944986872edd7`.
The [build attestation](phase-14-build-attestation.json) retains parent/patch,
source-file and binary hashes. The original fixture and schema come from
`0c3f9426b398bd44d363e665daa2fe860cb3516500d91aa29d26e1c517aaacdb`;
the intermediate `ae28` source changes the two account locks. The final
derivation adds the reviewed renewal association and committed-fact retention
correction. Identity and portal files are unchanged across these derivations.
The matching snapshot supplies provisioning SQL through
`prepare.py --backend-source`; concurrent Phase 15/19 schema and service changes
do not enter this fixture. No old checksum is rewritten.

The initial real browser run reached the original encrypted portal pending
operation, but failed before the platform committed its selection. The
[retained RED report](phase-14-https-browser-red-lock.json) records this failure;
it is not local-save recovery evidence. Independent PostgreSQL diagnostics
identified a cross-service lock cycle: customer selection and context issuance
held the account row `FOR UPDATE`, then the final original-source HTTPS proof
asked identity to acquire the same account row `FOR SHARE` on another
connection. The source recheck timed out and the transaction rolled back.
The correction changes only the read-only account fences to shared locks and
retains the final online source check, generation fences and other write locks.
The regression adds an independent real PostgreSQL account-share source port.
The final real browser run verifies the correction with separate running
identity/platform processes, retaining the final online source call.

The first corrected `ae28` run passed real role isolation, all four negative
binding probes (`403/409/403/401`), platform-commit/local-save failure and the
same-operation recovery with one consumption and no expiry extension. Its
[retained renewal RED report](phase-14-https-browser-red-renewal.json) then
failed online renewal: the refresh family rotated successfully, while the
platform's renewal association required a non-revoked old capability after the
portal had deliberately revoked that capability before rotation. The live
grant and SSO source remained valid; no capability or new selection was
fabricated. A correction must preserve the original confirmed selection/BFF
binding and the old capability's revocation while independently authorizing
renewal from the current source. The final correction implements those checks,
requires the unique committed source/member/application/service association,
and rejects renewal from a BFF version whose switch has already committed.
Routine selection cleanup retains consumed facts so that recovery-window
expiry cannot erase this renewal/switch authority. Phase 23 must implement any
eventual purge only after the source is terminal and its retention obligations
are satisfied; retention is not an authorization to keep personal data forever.

## Real HTTPS result

The [formal browser JSON](phase-14-https-browser.json) records PASS 10/10 at
`2026-10-28T05:45:48.277Z`, Chromium `156.0.8078.4`, TLS 1.3,
`tlsErrorsIgnored=false`, `httpFixtures=false`, and
`fixtureBoundaryReset=true`. The script uses no request interception, disabled
TLS verification, SQL-generated successful capability, browser storage export,
trace, form screenshot or token-bearing report.

The ten checks cover real runtime isolation; four private binding denials;
platform commit followed by portal save failure; same-operation recovery;
online refresh with unchanged original member; central context switch; pure
employee rejection; same-browser SSO with independent Cookies/local contexts;
original source revocation while another account remains usable; and absence
of upstream credentials in browser storage. The observed session/profile 503s
are the intentional, verified local-save fault; the corresponding later 401s
are the intentional source-revocation case.

The command is reproducible from the platform repository:

```powershell
node backend/tests/unified-auth-https/customer-browser.mjs `
  --environment /tmp/rust-saas-unified-auth-20261028/https-environment.json `
  --proxy http://127.0.0.1:24445 `
  --output backend/docs/designs/unified-auth/validation/phase-14-https-browser.json
```

The task's native PostgreSQL cluster, private mount namespace, trusted TLS
gateway, CONNECT proxy and dependency build cache are reused. Portal data is in
a separate UUID-named database with an independent NOLOGIN owner and real LOGIN
runtime. RP, portal proof source, reverse proof, profile service, selection
consumer and database credentials are distinct; no secret is in this report.

The fixture includes a dual-member administrator for same-account employee/
customer isolation plus pure EMPLOYEE and CUSTOMER accounts for denial cases.
The portal runtime has no CONNECT to the shared platform/identity database,
DDL, owner membership or migration-history access. Current legacy profile
service registration is explicitly recorded in the Phase 24 retirement list.

## Resource and evidence limits

All component fixture databases/roles/pools were deleted in their cleanup paths.
The final browser run closed all browser contexts/processes, removed the marked
portal trigger/function and disabled the extra customer membership with its
required identity generation increment. After acceptance, the three real HTTPS
services were stopped and the original manifest's shared/portal databases and
owner/runtime roles were removed; a separate catalog check returned `0|0`
remaining databases/roles. The namespace, gateway and Windows CONNECT proxy
exited, leaving no service PID records or listener on 24445. The pre-existing
Windows loopback 443 service remained untouched. The task-owned native
PostgreSQL cluster and dependency build cache remain under root lifecycle
control for later stages. Thirteen obsolete private environment/key files and
application/gateway/proxy logs were individually deleted after shutdown; old
source snapshots await final task cleanup. No Docker
containers/images/volumes/networks/cache were created or reused. Formal reports
are retained, while private keys/env/logs and task-only builds remain subject to
final ownership-checked cleanup. No Windows VHDX recovery is claimed.

Normal TOTP login and production partner approval are separate Phase 19/25
gates. Global logout and the old profile internal-service registration remain
explicit Phase 20/15/24 work. This phase's real HTTPS acceptance completes only
the customer BFF scope; it does not declare the final release candidate
accepted, complete those later stages or authorize existing-data reset.
