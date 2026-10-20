# Linux verification environment

`Dockerfile` is a build-only tool image. It pins Rust 1.97.0, verified official
Node 24.18.0 archives, npm 11.16.0 and the locked Playwright 1.64.1 dependency.
The browser layer installs real Chromium, Google Chrome and Microsoft Edge into
the image. The supported first validation target is Linux amd64. Browser versions
are recorded by each actual gate; branded browser update channels are not product
support for Firefox, WebKit or Safari.

From a clean checkout with a local Linux Docker daemon:

```sh
docker build --build-arg TARGETARCH=amd64 -f deploy/ci/Dockerfile -t saas-ci:phase04 .
docker build --target toolchain --build-arg TARGETARCH=amd64 -f deploy/ci/Dockerfile -t saas-ci:phase04-tools .
docker pull postgres@sha256:45cd22f8d32e189d245403954882f88e7a8714301fda80dab6da90f1265b25a3
docker tag postgres@sha256:45cd22f8d32e189d245403954882f88e7a8714301fda80dab6da90f1265b25a3 postgres:17.6-bookworm
node scripts/ci/linux-runner.mjs --gate fast --out-dir /absolute/new-report/fast
node scripts/ci/linux-runner.mjs --gate postgres --image saas-ci:phase04-tools --out-dir /absolute/new-report/postgres
node scripts/ci/linux-runner.mjs --gate browser --browser chromium --out-dir /absolute/new-report/chromium
node scripts/ci/linux-runner.mjs --gate browser --browser chrome --out-dir /absolute/new-report/chrome
node scripts/ci/linux-runner.mjs --gate browser --browser msedge --out-dir /absolute/new-report/msedge
```

The launcher uses the local Unix Docker socket explicitly. On Windows it invokes
the Ubuntu WSL distribution, configurable with `CI_WSL_DISTRO`; it never silently
uses a remote Docker context. Run the launcher with the repository's pinned Node.
An alternate immutable tool image can be supplied with `--image`; every run records
and uses its resolved image ID. For an exported clean source tree without Git,
pass its verified full `--source-revision`. CI image builds may use normal Docker
HTTP(S) proxy build arguments in restricted environments. No TLS verification is
disabled and no daemon configuration or production credentials are needed.

The named `toolchain` target contains the same pinned Node/npm/Rust tools without
browser downloads; PostgreSQL jobs can build it before launching their private
environment. The default final `browsers` stage adds the managed browser layer.
Output directories must be fresh and absolute, outside included source roots.
The repository's `target/` directory and fresh locations outside the checkout are
supported. Repository roots, their ancestors and locations such as `scripts/ci/`
are rejected before directory creation to prevent recursive source copies.

Each run creates an allowlisted source snapshot without `.env` files, secrets,
Git state, node_modules, build output or personal browser caches. It copies that
snapshot into an independent `/workspace` inside a fresh container. Dependencies
are installed from package locks into that workspace. Browser binaries come from
the image-owned `/opt/ci/browser/.browsers`; no developer home is mounted. Neither
the Docker socket nor the working checkout is exposed to the tool container.

The PostgreSQL and browser gates create PostgreSQL 17.6 with a synthetic random credential,
private per-run network, 2 GiB tmpfs data storage, a 40-connection limit and no published ports. The storage budget accommodates the real platform region seed used by notification integration. It explicitly
runs eleven selected platform database modules: `production_security_tests`,
`platform_settings_tests`, `authorization_isolation_tests`, `entity_creation_tests`,
`release_tests::zip_registration`, `oauth_issuance_tests`,
`platform_session_refresh_tests`, `login_protection_tests`, `management_audit_tests`
`notification_tests`, `data_grant_tests` and `data_grant_discovery_tests`.
The first five contained 27 checks in Phase 5; Phase 11 selected 41 checks.
Phase 12 added five login-protection checks for eight modules and 46 checks.
Phase 13 selected ten modules and 64 checks, adding eight management-audit checks
and the existing eight notification plus two application-member checks.
The current twelve modules require 74 checks, preserving Phase 14 eleven/71 and adding three discovery checks. Phase 14 added seven controlled data-grant
management checks to that preserved Phase 13 selection.
The exact
selection and expected counts are defined by `scripts/ci/manifest.mjs`.
The OAuth group covers atomic one-time consumption, at least 20 concurrent refresh
exchanges, partial-write rollback, live identity state and legacy token storage.
Existing application-token and platform-refresh boundaries remain separate
regressions. See the [OAuth contract](../../docs/oauth-issuance.md) and
[Phase 11 results](../../docs/validation/phase-11.md). Login admission, shared-alias
deadlines, HTTP password lifecycle and mixed Argon2 pressure are described in
[login protection](../../docs/login-protection.md); its historical execution results
remain in [Phase 12](../../docs/validation/phase-12.md).
The management-audit group requires both SQL exceptions and trigger `RETURN NULL`
(zero inserted audit rows) in owned temporary databases. Each selected mutation
must roll back its authoritative write, collection replacement or cascade effects
with the rejected audit; after removing the fault, one retry must produce one
successful change and its corresponding audit. The matrix covers roles/grants,
users/passwords/platform settings, memberships/user roles, tenants/organizations,
OAuth administration, applications/menus, releases and notifications. Release and
notification regressions retain their existing transaction and idempotency
boundaries. These are mandatory coverage requirements; actual execution results,
source hashes and fault evidence for that stage remain in [Phase 13](../../docs/validation/phase-13.md).
The Phase 14 data-grant group requires independent read/write management permissions,
platform-root and explicit business-tenant scope, valid direct membership and target
context, registered app/resource/action/scope ceilings, UUID-preserving deltas,
conflict detection, immediate effect on new protected requests, and atomic audit
failure rollback. These requirements and the selected counts do not assert an
execution passed; current results and frozen inputs belong to
[Phase 14](../../docs/validation/phase-14.md).
PostgreSQL execution includes real migrations
and isolated database creation/removal. Module separators in log names are
converted from `::` to `-` for Windows. Zero selected tests,
an unexpected count, a failure or an ignored test fails the gate. This validates
the existing platform only; future business backends are not included.

The managed browser gate adds `data-grant-ui-live` after the unchanged ten-suite HTTP fixture matrix, for 14 mandatory steps. Its single exact ignored Rust test starts real Axum, uses owned platform and independent business PostgreSQL databases and launches the selected actual product. The existing work-orders backend performs actual introspection/context and business SQL: the same app token sees one SELF record, two ORG records, no outside record and 403 immediately after revoke. Sixteen named UI checks and four ordered audit request IDs must match the database proof; zero, skipped, missing, wrong-product or either database cleanup result fails even when the process exits zero. A sanitized preview PNG is written only inside the owned report directory and its bytes/hash are verified; no browser trace, headers or token dump is uploaded. Existing Node fixtures receive no database environment. The standalone `tests/browser/run.mjs` remains fixture-only; optional Firefox/WebKit workflow runs retain that narrower scope. This existing Node service test does not establish the future Rust business migration. Current evidence belongs to [Phase 15](../../docs/validation/phase-15.md).

The fast and browser gates run `node scripts/sync-micro-sdk.mjs --check` after
locked frontend dependency installation and before any frontend build. CI never
generates SDK artifacts before checking them. The source snapshot includes the
canonical SDK and work-orders frontend source directory with the existing
private-file/generated-output exclusions. Phase 15 additionally includes the
existing work-orders backend public source, package lock and migrations for the
real UI/business authorization proof, preserving their bytes. Developer
environment, private and generated files remain excluded; deploy still uses its
explicit selected-file list.

The fast gate's authoritative step list is `scripts/ci/run.mjs`, including all nested archive units,
three staging maintenance regressions, public-contract and OAuth issuance units,
eight login-limit, eight password-service, six client-IP and six config units,
the full eight-test `data_grants::` unit selection (five policy, two use-case and one discovery-bound test)
and the existing transport selection. Counts declare required coverage, not
actual passing results. Password cancellation or response timeouts retain the
worker permits until real computation exits; process-local budgets are not a
shared quota across replicas. Its no-database contracts add pure Node iframe protocol,
SDK client and generated-artifact regression files, work-orders frontend
client/application units and a real loopback CORS HTTP check without a database
to the Phase 5 selection of 99 tests (70 existing + 29 packager). Actual counts
come from each run. A Windows symlink test
skip is reported separately and cannot establish that the required Linux gate
passed. Historical strict Rust fmt/Clippy debt and ignored default DB/network
tests are disclosed limits, not passing checks in this manifest.

Phase 23 adds a standalone Rust notification crate at
`mini-services/notification-ws/Cargo.toml`. The fast gate requires its formatting,
all-target/all-feature checks, strict Clippy, full unit selection and documentation.
The source snapshot includes its locked Rust inputs and Node test drivers, while
the runtime image contains no Node service. `tests/notification-ci-policy.mjs`
checks that incomplete integration or image evidence cannot pass with exit zero.

The PostgreSQL gate calls `scripts/ci/notification.mjs --run-tests --out-dir <fresh-output>`
only with the launcher-owned database environment. The driver builds the real Rust
platform, notification service and bounded delay proxy, then sequentially tests live revocation,
partition scheduling and transport admission against real PostgreSQL. Browser gates
add `--browser chromium|chrome|msedge` to require actual two-page organization
switching and HTTP inbox recovery. Missing dependencies, zero tests, skipped tests
or incomplete results fail. Node provides test/Vite processes, with no Node
notification backend. These live reports are separate from fixture-only browser
results and from the image smoke below.

The trusted host delivery gate separately runs `scripts/ci/notification-image.mjs`
with a fresh output path and the verified full source revision. It resolves the
locally available `rust:1.97.0-slim-bookworm` and `debian:bookworm-slim` bases,
builds the independent `linux/amd64` notification image, and requires nine exact
checks. They cover locked build identity, UID/GID 10003, absence of Node, closed
stdin without premature exit, honest live/ready behavior with dependencies down,
Bearer-protected metrics, read-only/private runtime, graceful SIGTERM, redacted
logs and complete current-input hash agreement. This service is built separately
from the native platform bundle. The image result records a local candidate ID;
it does not claim a registry digest, publish an image or deploy a target.
Only `notification-image/result.json` and its redacted `logs/` are upload inputs,
never the `source/` snapshot. Full platform/PostgreSQL behavior remains a required
separate live gate. The workflow separately uploads notification PostgreSQL/browser
`notification-result.json` and its explicitly listed sanitized driver logs; it does
not upload either harness's runtime workspace or source tree.

The per-run `linux-runner-result.json` records image IDs, source hashes, tools,
exit status and cleanup errors; nested `reports/` contains the gate report and
redacted logs. Deterministic owned container/network names are registered before
startup and removed after success, failure or signals only after owner-label
checks. A killed host or daemon cannot execute cleanup; inspect the exact owner
from its report and remove only matching resources after diagnosis. The database
container itself is temporary and must never point at production storage.

The HTTPS/Nginx delivery gate uses the separate trusted host Docker driver.
This tool environment does not claim that a successful image build deployed to an
external target, nor does it publish application images or run production data.

Phase 5 also attaches `tests/zip-release-e2e.mjs --release-report <verified-release-result.json>
--out-dir <new-absolute-output>` to the delivery gate. It reuses the three verified
candidate image IDs and runs the real Rust API, temporary PostgreSQL and Nginx
ZIP compatibility path. Its nine required checks include idempotent/concurrent
upload, lost-response reconciliation, cross-environment immutable promotion,
preflight/activation/rollback, retained resources and private metadata denial.
Missing checks or cleanup failures fail the delivery gate; the per-run report
is the authority for whether an actual execution passed.

Only `<output>/result.json` and its redacted `<output>/logs/` are CI upload inputs.
The ZIP driver keeps synthetic credentials and TLS private keys in an owned
Linux 0700 directory outside the evidence tree, then removes that directory.
Do not upload its source snapshot, scratch tree or private runtime material.
