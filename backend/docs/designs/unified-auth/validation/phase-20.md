# Phase 20 — Logout, reliable delivery and controlled account invitations

Date: 2026-10-28. Status: **COMPLETED**. The 13 functional HTTPS scenarios,
separate INFO-level terminal-warning/log-redaction gate and complete owned
environment cleanup passed. The root coordinator owns the ledger update.

The implementation follows the
[logout and delivery contract](../phase-20-logout-delivery-contract.md),
current canonical fresh `0001`, SSO-only accounts and independent owner/runtime
boundaries. No existing business database was migrated or reset. Existing
uncommitted work in all repositories was preserved.

## Result and evidence

The actual three-service HTTPS run passed **13/13** on 2026-10-28, with
Chromium 156.0.8078.4 and TLS 1.3. The
[machine-readable result](phase-20-https-browser.json) records
`tlsErrorsIgnored=false`, `httpFixtures=false` and
`positiveMfaSeededBySql=false`. The executable harness is
[`phase20-browser.mjs`](../../../../tests/unified-auth-https/phase20-browser.mjs),
SHA-256 `804781b4ba0a77660373fc9a005fe7bb07d3870aa7783ae1c3d95cb41827aa36`.

The run proved these behaviors through actual authority, platform and portal
processes, current PostgreSQL roles, the trusted task TLS gateway and a bounded
in-memory peer collector:

- Real console and customer-portal login produced distinct opaque RP public
  `sid` values for the same SSO source. CURRENT_APP revoked only its original
  console grant; the portal and an independent browser's other console grant
  remained usable. Another account remained isolated.
- GLOBAL closed both RPs, the same RP's other browser/source and every original
  account SSO source while preserving consent. Worker-created signed logout
  notifications reached both actual receivers and closed their local sources
  before the harness read their sessions again.
- Each actual receiver rejected wrong signature, wrong audience, a known SID
  paired with another subject, and a repeated JTI. Valid unknown SID processing
  established its durable source/replay barrier and the repeated token returned
  409 `BACKCHANNEL_REPLAY`.
- A separate owned ACCOUNT_ONLY confidential RP used real canonical
  authorization-code/PKCE exchange and original RP logout. With its collector
  delayed by 3,000 ms, authority logout returned in **610 ms**, already revoked
  its grant, and recovered the same receipt using the now-revoked original
  token. An observed retryable failure was followed by controlled recovery;
  a permanent 400 produced a one-attempt TERMINAL job with a fixed code.
- An explicit SHORT_SESSION_ONLINE_CHECK RP produced a DEGRADED job with
  `SHORT_SESSION_CONTRACT`; a request-supplied metadata/SSRF destination was
  rejected without applying logout. The short-session/online-check limits are
  documented in the contract, not presented as instant third-party logout.
- A distinct non-super-admin reviewer enrolled TOTP through the actual SSO
  security page, then performed real password/TOTP step-up (428 followed by
  200). The BFF, SSO and selected platform source all changed, and the current
  source carried recent TOTP. Invitation issue used the real management API.
- The owner-bound recipient channel and exact invitation verification fact
  released invitation material only to the worker. The collector first failed,
  the durable job recorded bounded retry state, and the controlled retry
  delivered the original material. Raw token checks ran only in Node memory;
  no raw token entered SQL statements or durable delivery records.
- The actual public no-store form accepted that delivery, cleared its token
  and password, and created exactly one ordinary account with no member, role,
  SSO session or grant. Database receipt/invitation/account/proof/channel tuples
  matched. The exact original operation recovered ACCOUNT_CREATED, another
  operation could not consume it again, and delivered but revoked or expired
  verification sources could not create another account.

The invitation absolute 24-hour expiry, changed proof generations, mandatory
write rollback, username collision and concurrency matrices were also exercised
against real canonical PostgreSQL. The actual HTTPS expiry negative deliberately
expires the owner verification deadline; it does not claim to wait 24 hours or
alter the immutable original invitation expiry.

The first fixture did not explicitly set RUST_LOG, so its ERROR-only default
filtered the emitted WARN terminal event. Its durable TERMINAL result and
initial log-redaction scan passed, but this run alone does not prove visible
operational alerts. The coordinator restarted only the same r2 identity binary
with `RUST_LOG=info`, retaining the database and other components. The separate
[logging-focused HTTPS run](phase-20-https-browser-logging.json) passed the same
13 scenarios. These are two gates over the same scenario set, not 26 distinct
cases. The [final log scan](phase-20-log-redaction.json) checked seven logs,
3,014,692 bytes, and compared six actually delivered invitation messages in
memory. It found zero secret leaks and one visible
`identity_delivery_terminal` warning. The original 13-case result and
[before-INFO scan](phase-20-log-redaction-before-info.json) are preserved.

## Frozen execution inputs

The actual run used a composite build rather than claiming all binaries came
from one build after a narrowly scoped identity repair:

| Input | SHA-256 and boundary |
|---|---|
| Phase 20 r1 full snapshot | `1df2243ca1fada96de892869d3af1335337cc57f760d7cadfbcaea0bc65e77cf` — platform, portal, canonical schema/ACL and static frontend |
| Phase 20 r2 identity snapshot | `3c73aab55e123ede952ce9171118cd3a1661480e186f6d19a721f4cd0c8cc4a6` — identity-only rebuild |
| Running r2 identity binary | `92ee9eeb11df364701dcb427258f51700d980ad853e487f7be065feddbdf45f0` |
| Running r1 platform binary | `c4d0a23b96a157be18d680b36f28e801e412286a0eaa981550ddfc46954a2ef7` |
| Running r1 portal binary | `eda7961fef7b5b0de601125a03566e428452b4e3f316eaa64c1ab4b1db3c7c66` |
| r2 worker source | `4221aeb82db253cf0c06c12cf388cc2b2b0ac006f744c342e45d0bd266f9b780` |
| Added registration concurrency/admission tests | `ac2c2b45f3842e8a8a692b240c5a9f079167d25682ff39ed9ecd6288bfd90fbe` |

The [r1 full build attestation](phase-20-build-attestation.json),
[r2 identity build attestation](phase-20-identity-update-attestation.json) and
[composite runtime attestation](phase-20-runtime-attestation.json) record those
boundaries, the unchanged platform/portal/schema inputs and strict initial CA
health checks. The executed live harness SHA above differs from the earlier
script copied inside the r1 snapshot (`a054f3db3a2d9276a2ee9487a41481b2993a23665372e9fa6b448eabc0f36bad`).
Harness contract assertions, fixture source lookup and scoped cleanup were
corrected after that copy; the runtime attestation records both script hashes.
The actual 13-case result came from the live harness SHA `804781b4…`, not from
the earlier copied script.

The final source review caught a registration-freshness gap during signing-key
waits. The repaired worker locks application, client, admission and owner target
in the same order as registry/review administration, then rechecks exact binding
and lease/fact deadlines after signing. Third-party admission must remain
APPROVED and BACK_CHANNEL with its exact reviewed URI. Two additional real PG
cases prove cancellation-first rejection and signing-wait serialization. The
actual HTTPS run used r2 containing this repair; r1 identity was never accepted
as its runtime evidence. The guarantee concerns the committed dispatch decision
and bounded subsequent network call, not instantaneous recall of a request that
has already been dispatched.

## Component verification

Component counts are separate scopes, not numbers to add into one fictitious
full-suite total. Ignored cases are identified and are not counted as passes.

| Component | Result and evidence |
|---|---|
| Identity canonical logout and delivery worker | Final **9/9** real PG cases; Linux library **71 PASS / 46 explicitly ignored / 0 fail**; strict all-target Clippy, fmt and documentation passed. [Report](phase-20-identity.md), [JSON](phase-20-identity-components.json). |
| Bounded pinned TLS transport | **13/13** actual Linux cases, included in the Linux library result: hostname/CA, DNS snapshot, unsafe destinations, redirects, proxies, deadlines and body bounds. Native Windows loopback failures are disclosed separately. |
| Invitations/Q5 | **7/7** keys/real PG/limited LOGIN cases, plus **2/2** public-shell route checks and **6/6** recipient-form Node checks. Covers proof generation, readonly facts, expiry after lock waits, atomicity, replay and collisions. [Identity report](phase-20-identity.md). |
| Schema, ownership and readiness | Phase 20 **3/3** real LOGIN, emergency-version-only shrink **1/1**, Phase 15 regression **3/3**; current portal contract is **5 tables / 20 UUID columns** with its logout facts SELECT/INSERT-only. [Schema report](phase-20-schema.md). |
| Platform RP | **2** pure claim matrices plus **5** unique actual PG cases, including exact refresh SID and early notification barriers. [Report](phase-20-platform-bff.md), [JSON](phase-20-platform-bff-components.json). |
| Customer RP | **21** ordinary cases, **13/13** explicit PG plus focused expiry/original-operation cases; actual limited LOGIN schema/ACL verified. [Report](phase-20-portal.md), [JSON](phase-20-portal-components.json). |
| Client registration/review/UI | Identity **1 input + 1 canonical PG**, platform **19** unit, frontend **450**, HTTP-fixture UI **11 + 17** and mobile visual checks. These UI fixtures do not replace actual SSO/worker TLS evidence. [Report](phase-20-registration.md). |
| Existing protocol regressions on current source | Phase 8 **2/2**, Phase 9/10/11 **6/6**, Phase 19 canonical review/admission/pagination **3/3** real PG, each with owned database cleanup. [Identity report](phase-20-identity.md). |

## Self-check

| Acceptance criterion | Result | Evidence |
|---|---|---|
| Current application and global logout have distinct exact effects and preserve consent | PASS | Actual HTTPS original grant, same RP other browser, other RP, other account and GLOBAL; canonical 3-case matrix |
| Wrong signature/audience/replay, unregistered addresses and dangerous internal destinations are denied | PASS | Actual receiver 400/409 and request destination denial; canonical registry/worker and actual Linux transport matrices |
| Peer outage does not delay local revocation, bounded retry recovers and permanent failure is observable | PASS | Actual 610 ms revoke versus 3,000 ms collector delay, same-operation recovery, delivered retry, fixed TERMINAL error and one visible terminal WARN under actual INFO logging; seven-log redaction scan passed |
| Partners without notification have an explicit short-session/online-check contract | PASS | Actual DEGRADED source and documented 5-minute local / 60-second online upper bounds |
| Invitation uses only a verified recipient channel, retries are bounded, secrets are not durable and accepted links cannot create accounts again | PASS | Actual real issue/TOTP, owner facts, failure/retry, recipient form, exact receipt tuple, repeat/revoked/expired-source denial; canonical Q5 negative/concurrency cases |
| Task-owned environment, logs, private files and temporary resources fully cleaned | PASS | [Cleanup attestation](phase-20-cleanup.json): both owned databases/roles and service/namespace/gateway/proxy/collector records zero; private runtime files and logs removed |

## Resource boundary and remaining release gates

Actual projects are identity, platform and customer portal, with one existing
task-native PostgreSQL cluster at loopback 55494,
`system_identifier=7701548153213487841`. The root created fresh owned shared and
portal databases/roles, one mount namespace, one instance of each service, one
task TLS gateway, one in-memory collector and one loopback CONNECT proxy. This
phase created and reused **0 Docker containers**. Component test databases and
LOGIN roles were individually cleaned. The
[final actual-environment cleanup](phase-20-cleanup.json) confirmed no owned
databases or roles and no service/namespace/gateway/proxy/collector records.
Windows proxy port 24445 closed; the pre-existing port 443 process (PID 5044)
was unchanged. Twelve private runtime/key/log files totaling **93,475 bytes**
were removed. Docker context was `default`; images, containers, volumes and
build cache all remained **0 bytes before and after**. The existing task-native
PostgreSQL cluster, shared build cache and frozen snapshots are deliberately
retained under root ownership for later goal phases. No Windows WSL virtual
disk compaction or claim of complete Windows disk-space recovery is made.

The collector sent no real email, message or external notification. All channel,
domain, reviewer and extra-RP facts are explicit task fixtures. They do not
establish real Q3 responsibility, Q8 contractual facts or production permission.
Real release gates remain closed until their later phase requirements are met.
This implementation does not promise every third-party local page exits
immediately and does not use an iframe as its sole logout mechanism.

## Suggested task-ledger update

Set **Phase 20: COMPLETED**, mark its five functional acceptance criteria PASS,
and cite this report, both scoped actual HTTPS reports, the visible-warning
log-redaction gate, composite build/runtime attestations and cleanup JSON.
Preserve the Phase 18 execution
pointer/status until its independent acceptance is complete; recommend Phase 21
next when the root schedules dependent work. Remaining Phase 20 items: none.
