// Commands are reviewed source, never input URLs or arbitrary command strings.
export const gates = ['fast', 'postgres', 'browser', 'delivery', 'business-delivery']
export const postgresSuites = Object.freeze([
  { name: 'production_security_tests', expected: 8 },
  { name: 'platform_settings_tests', expected: 4 },
  { name: 'authorization_isolation_tests', expected: 3 },
  { name: 'entity_creation_tests', expected: 6 },
  { name: 'release_tests::zip_registration', expected: 6 },
  { name: 'oauth_issuance_tests', expected: 10 },
  { name: 'platform_session_refresh_tests', expected: 4 },
  { name: 'login_protection_tests', expected: 5 },
  { name: 'management_audit_tests', expected: 8 },
  { name: 'notification_tests', expected: 12 },
  { name: 'data_grant_tests', expected: 7 },
  { name: 'data_grant_discovery_tests', expected: 3 },
])
// The historical platform 12/74 selection is retained; Phase19 adds two member-policy cases in notification_tests (current total 76).
export const workOrdersPostgresSuites = Object.freeze([
  { name: 'work-orders-migrations', target: 'migrations', expected: 27 },
])
// Real business SQL and HTTP authorization regression; separate from migration
// compatibility and from the historical platform selection above.
export const workOrdersReadPostgresSuites = Object.freeze([
  { name: 'work-orders-read-contracts', target: 'read_contracts', expected: 13 },
])
export const workOrdersWritePostgresSuites = Object.freeze([
  { name: 'work-orders-write-contracts', target: 'write_contracts', expected: 14 },
])
export const workOrdersMemberPostgresSuites = Object.freeze([
  { name: 'work-orders-member-contracts', target: 'member_contracts', expected: 12 },
])
export const workOrdersReminderPostgresSuites = Object.freeze([
  { name: 'work-orders-reminder-contracts', target: 'reminder_contracts', expected: 12 },
])
export const workOrdersReminderLivePostgresSuites = Object.freeze([
  { name: 'phase20_work_orders_tests::real_platform_reminder_publication_and_recovery', expected: 1 },
])
export const workOrdersLivePostgresSuites = Object.freeze([
  { name: 'phase17_work_orders_tests::real_platform_authorization_and_business_scope', expected: 1 },
])
export const workOrdersWriteLivePostgresSuites = Object.freeze([
  { name: 'phase18_work_orders_tests::real_platform_writes_and_idempotency', expected: 1 },
])
export const workOrdersMemberLivePostgresSuites = Object.freeze([
  { name: 'phase19_work_orders_tests::real_platform_member_directory_and_assignment', expected: 1 },
])
export const storesPostgresSuites = Object.freeze([
  { name: 'stores-postgres-contracts', target: 'postgres_contracts', filter: 'postgres_contracts', expected: 1 },
])
export const storesLivePostgresSuites = Object.freeze([
  { name: 'phase26_stores_tests::real_platform_stores_and_cross_app_isolation', expected: 1 },
])
export const frontendPackages = ['frontend', 'mini-services/micro-vue', 'mini-services/micro-jquery', 'examples/websocket', 'mini-services/oauth-playground/frontend', 'business-apps/work-orders/frontend', 'business-apps/stores/frontend']
export const contractTests = ['tests/application-release-policy.mjs', 'tests/platform-admin-contract.mjs', 'tests/data-grants.mjs', 'tests/data-grants-transport.mjs', 'tests/auth-queries.mjs', 'tests/route-policy.mjs', 'tests/route-policy-guard.mjs', 'tests/application-navigation.mjs', 'tests/micro-context-runtime.mjs', 'tests/micro-context-updates.mjs', 'tests/micro-sandbox-lifecycle.mjs', 'tests/micro-runtime-faults.mjs', 'tests/micro-iframe-protocol.mjs', 'tests/micro-sdk-client.mjs', 'tests/micro-sdk-generation.mjs', 'tests/notification-context.mjs', 'tests/session-refresh.mjs', 'tests/session-coordinator.mjs', 'tests/session-integration.mjs', 'tests/map-location-regions.mjs', 'mini-services/micro-vue/smoke-test.cjs', 'mini-services/micro-jquery/smoke-test.cjs', 'business-apps/work-orders/frontend/tests/client.mjs', 'business-apps/work-orders/frontend/tests/app.mjs', 'business-apps/work-orders/backend/tests/cors.test.mjs', 'examples/app-backend/authorize.test.mjs', 'tests/package-microapp.mjs']
export const deploymentUnitTests = ['deploy/microapps/tests/static-config.mjs', 'tests/nginx-render-policy.mjs', 'deploy/static/tests/static-image-policy.mjs', 'deploy/native/tests/build.test.mjs']
export const ciUnitTests = ['scripts/ci/zip-distribution-results.test.mjs', 'tests/phase28-delivery-fixture-contract.mjs', 'tests/business-images-policy.mjs', 'tests/phase28-ci-policy.mjs', 'scripts/ci/business-delivery-results.test.mjs', 'tests/ci-runner.mjs', 'tests/ci-workflow.mjs', 'tests/phase21-ci-policy.mjs', 'tests/phase27-ci-policy.mjs', 'tests/notification-ci-policy.mjs', 'tests/notification-e2e-launch-policy.mjs', 'scripts/ci/postgres.test.mjs', 'scripts/ci/platform-admin.test.mjs', 'scripts/ci/work-orders.test.mjs', 'scripts/ci/work-orders-write.test.mjs', 'scripts/ci/work-orders-member.test.mjs', 'scripts/ci/work-orders-reminder.test.mjs', 'scripts/ci/work-orders-browser.test.mjs', 'scripts/ci/stores.test.mjs', 'scripts/ci/stores-browser-results.test.mjs', 'scripts/ci/browser-source-ownership.test.mjs', 'scripts/ci/linux-runner.test.mjs', 'scripts/ci/release.test.mjs', 'tests/browser/results.test.mjs','tests/browser/wait.test.mjs','tests/zip-release-e2e-policy.mjs']
ciUnitTests.push('scripts/ci/monitoring-results.test.mjs')
ciUnitTests.push('scripts/ci/recovery-results.test.mjs')
ciUnitTests.push('scripts/ci/standalone-release.test.mjs')
ciUnitTests.push('tests/runtime-entrypoints.mjs')
ciUnitTests.push('scripts/ci/managed-deployment-results.test.mjs')
ciUnitTests.push('scripts/ci/rehearsal-images.test.mjs')
export const declaredLimits = [
  'Phase32 monitoring reports candidate load and real fault observations separately from production SLO; its owned PostgreSQL capacity gate starts at 100 active sessions and 100000 business records. No external receiver is configured. Phase33 separately implements the Rust pilot recovery executor; physical production PITR remains unaccepted.',
  'Business delivery builds immutable work-orders/stores candidate archives and separately verifies an owned registry, digest-pinned Compose and real versioned browser routes. Registry rehearsal is isolated; external publication requires the protected manual workflow.',
  'Platform Rust is one crate with a narrow policy library and binary target. Public paging, binding, ceiling and snapshot tests run in fast. Explicit config, data-grant policy/use-case, archive installation, staging maintenance, public-contract, ordinary OAuth issuance, login limit, password and client IP units plus the independent transport fixture run in fast; selected platform DB suites run in postgres.',
  'The historical default Rust group has ignored DB/network tests; it is not counted as passing by this selection. Existing strict fmt/Clippy debt is disclosed in phase-01, not silently repaired or disabled here.',
  'The standalone browser manifest uses real frontends with isolated API/Socket.IO fixtures; the managed browser gate separately runs real Axum/owned-PostgreSQL grant UI and Rust notification/two-page recovery suites. Notification migration evidence is scoped to its selected real-platform cases and does not claim production deployment.',
  'The separate work-orders browser step runs the actual production host and Vite module/dynamic chunks in an opaque iframe through owned HTTPS/Nginx to real Rust and two owned PostgreSQL databases; fixture browser results cannot substitute for its fifteen mandatory checks.',
  'Delivery builds verified OCI archives and actual isolated HTTPS smoke. It never pushes a registry or deploys a production target.',
  'ZIP installation/reconciliation is validated separately from future business Rust services, monitoring and encrypted disaster recovery.',
  'Stores SQL contracts run in their independent crate; a separate real-platform suite runs stores and work-orders ELFs with three owned databases and dedicated non-superuser roles. Phase26 proves backend HTTP/SQL and account isolation, without claiming Phase27 browser behavior or production deployment.',
  'The separate stores browser step uses the production host, work-orders and stores Rust binaries, three owned PostgreSQL databases and normal verified HTTPS/Nginx. Its typed report is mandatory; the ordinary fixture runner does not receive its database environment.',
]
