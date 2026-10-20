import assert from 'node:assert/strict'
import test from 'node:test'
import path from 'node:path'
import { evaluate } from './report.mjs'
import { parseStoresBrowserArgs, storesBrowserEnvironment } from './stores-browser.mjs'
import { storesBrowserChecks, storesBrowserCleanup, storesBrowserSteps, parseStoresBrowserOutput, validateStoresBrowserReport } from './stores-browser-results.mjs'
import { phase27MenuCounts } from '../../tests/helpers/phase27-stores-services.mjs'
const digest = 'a'.repeat(64), revision = 'b'.repeat(40), id = '11111111-1111-4111-8111-111111111111'
const ownership = { owner: 'ci-12345678', hostname: 'ci-12345678-postgres', database: 'postgres' }
function fixture() {
  const binary = { sha256: digest, bytes: 32, elfMachine: 62 }, files = [{ path: 'index.html', sha256: digest, bytes: 40 }, { path: 'assets/main.js', sha256: digest, bytes: 20 }]
  const applications = Object.fromEntries([['stores', 11], ['work-orders', 7]].map(([code, count]) => [code, { menus: new Map(Array.from({ length: count }, (_, index) => [index, id])) }]))
  const records = [0, 1].map(index => ({ requestId: index ? '22222222-2222-4222-8222-222222222222' : id, method: 'POST', path: '/stores-api/stores', credentials: 'omit',
    cookiePresent: false, bearerPresent: true, idempotencyKeyPresent: false, nativeOutcome: 'fulfilled', signalAborted: false, wire: { origin: 'null', cookiePresent: false, bearerPresent: true, frameType: 'iframe', responseStatus: 201 } }))
  const source = { platformBinary: binary, businessBinary: binary, workOrdersBinary: binary, adminBinary: binary, productionHost: files, viteBuild: files, workOrdersViteBuild: files,
    productionServices: ['platform', 'stores', 'work-orders'], gatewayTlsVerified: true, databases: ['p', 's', 'w'].map(key => ({ name: 'phase27_' + key + '_' + '1'.repeat(32), role: 'phase27_' + key + '_' + '1'.repeat(32) + '_writer', rolsuper: false, rolcreatedb: false, rolcreaterole: false })),
    menus: phase27MenuCounts(applications), separateFrontend: { outsideRepository: true, npmCi: true, npmBuild: true, secretSentinelAbsent: true, rootNodeModulesDependency: false, personalCacheDependency: false,
      emptyCacheBeforeInstall: true, noNodeModulesBeforeInstall: true, cache: { kind: 'fresh-owned-npm-cache', personal: false }, steps: ['standalone-npm-ci', 'standalone-npm-build'].map(id => ({ id, status: 'PASS', exitCode: 0, timedOut: false, log: 'artifacts/logs/' + id + '.log' })) },
    cli: ['stores', 'work-orders'].flatMap(applicationCode => ['bootstrap', 'bootstrap', 'grants-plan', 'grants-apply'].map(command => ({ applicationCode, command, applied: command !== 'grants-plan', ok: true }))) }
  const result = { schemaVersion: 1, product: 'chrome', browserVersion: '140.0.1', playwright: '1.55.1', sourceSha256: digest, passed: storesBrowserChecks.length, failed: 0, skipped: 0,
    checks: storesBrowserChecks.map(id => ({ id, status: 'PASS' })), evidence: Object.fromEntries(storesBrowserChecks.map(id => [id, { actual: true }])), source,
    members: { stores: { requests: [{}], records: [{}], idField: 'id', actualPlatformHttp: true }, 'work-orders': { requests: [{}], records: [{}], idField: 'userId', actualPlatformHttp: true }, crossAppStatus: 401, platformCrossAppStatus: 403, undeclaredPurposeStatus: 400 },
    transport: { origin: 'null', cookiePresent: false, credentials: 'omit', rustPreflightStatus: 204, credentialsAllowed: false, records, nativeAttemptCount: records.length,
      faults: records.map(record => ({ requestId: record.requestId, forwardedRequestId: record.requestId, actualUpstreamCommitted: true, automaticRetryCount: 0 })) } }
  result.evidence.screenshots = [{ path: '/owned/screenshots/stores.png', sha256: digest }]
  result.evidence['normal-renewal-keeps-legal-input'] = { actualIssueStatus: 201, initialIssueCount: 1, finalIssueCount: 2, sameIdentityDraft: '11.22' }
  return { schemaVersion: 1, kind: 'stores-real-platform-browser', status: 'PASS', product: 'chrome', privateWorkRemoved: true, cleanupErrors: [], sourceUnchanged: true,
    sourceRevision: revision, inputsSha256: digest, ownership, steps: storesBrowserSteps.map(id => ({ id, status: 'PASS', exitCode: 0, timedOut: false })),
    binaries: { platform: binary, business: binary, workOrders: binary, admin: binary }, tools: { node: 'v24.18.0', nginx: 'nginx version: nginx/1.28.0', nginxLibraries: 'libc.so', openssl: 'OpenSSL 3.0.0' },
    tls: { caSha256: digest, serverSha256: digest, ipVerified: true, ownedTrustOnly: true }, result, cleanup: Object.fromEntries(storesBrowserCleanup.map(key => [key, true])) }
}
const verdict = (document, overrides = {}) => evaluate({ format: 'stores-browser', exitCode: 0, expectedProduct: 'chrome', expectedSourceRevision: revision, document, ...overrides })
test('strict stores producer binds all actual groups and three independently owned non-superuser databases', () => {
  const good = fixture(); assert.equal(validateStoresBrowserReport(good, { product: 'chrome' }), good)
  assert.equal(verdict(good).status, 'PASS'); assert.equal(verdict(good).counts.pass, 21)
  const text = 'PHASE27_BROWSER_RESULT ' + JSON.stringify(good.result) + '\nPHASE27_BROWSER_CLEANUP ' + JSON.stringify(good.cleanup)
  assert.equal(parseStoresBrowserOutput(text, { product: 'chrome' }).result.passed, 21)
  assert.throws(() => parseStoresBrowserOutput(text + '\n' + text, { product: 'chrome' }))
})
test('actual service menu metadata reaches the strict parser with application-code keys', () => {
  const value = fixture()
  assert.deepEqual(value.result.source.menus, { stores: 11, 'work-orders': 7 })
  assert.equal(validateStoresBrowserReport(value, { product: 'chrome' }), value)
  value.result.source.menus = { stores: 11, workOrders: 7 }
  assert.throws(() => validateStoresBrowserReport(value, { product: 'chrome' }))
})
test('central stores format rejects incomplete result, wrong product, process failure and timeout', () => {
  for (const override of [{ expectedProduct: 'msedge' }, { document: null }, { exitCode: 1 }, { timedOut: true }, { expectedSourceRevision: 'c'.repeat(40) }]) assert.equal(verdict(fixture(), override).status, 'FAIL')
})
test('stores result refuses skipped groups, secret-bearing transport, missing consumer and cleanup failures', () => {
  const mutations = [value => value.result.checks.pop(), value => value.result.skipped++, value => value.result.transport.records[0].cookiePresent = true,
    value => value.result.transport.records[0].wire.origin = 'https://host.invalid', value => value.result.source.databases[0].rolsuper = true,
    value => value.result.source.separateFrontend.personalCacheDependency = true, value => value.result.members.stores.requests = [],
    value => value.result.transport.faults[0].actualUpstreamCommitted = false, value => value.cleanup.databaseRolesRemoved = false,
    value => value.binaries.admin = { sha256: 'f'.repeat(64), bytes: 32, elfMachine: 62 }, value => value.sourceUnchanged = false]
  for (const mutate of mutations) { const value = fixture(); mutate(value); assert.equal(verdict(value).status, 'FAIL') }
})
test('stores browser arguments reject unknown duplicate and incomplete selectors', () => {
  const args = ['--browser', 'chrome', '--out-dir', path.resolve('owned-stores-output'), '--source-revision', revision]
  assert.equal(parseStoresBrowserArgs(args).product, 'chrome')
  for (const invalid of [args.slice(0, 4), [...args, '--browser', 'chrome'], [...args, '--secret', 'synthetic'], args.map(value => value === 'chrome' ? 'firefox' : value)]) assert.throws(() => parseStoresBrowserArgs(invalid))
})
test('stores browser environment contains only owned TLS, selected binaries and required PostgreSQL identity', () => {
  const root = path.resolve('owned-source'), workDir = path.resolve('owned-work')
  const env = storesBrowserEnvironment({ PATH: '/bin', HOME: '/personal-home', PLATFORM_CLIENT_SECRET: 'synthetic-secret-must-not-pass',
    TEST_DATABASE_URL: 'postgresql://ci:synthetic@ci-12345678-postgres:5432/postgres', CI_TEST_POSTGRES_OWNER: ownership.owner }, { product: 'chrome', root, workDir, screenshotsDir: path.resolve('screenshots') })
  assert.equal(env.PLATFORM_CLIENT_SECRET, undefined); assert.equal(env.HOME, path.join(workDir, 'browser-home'))
  assert.equal(env.STORES_BROWSER_WORK_ORDERS_BINARY, path.join(root, 'target/ci-rust/debug/work-orders-api' + (process.platform === 'win32' ? '.exe' : '')))
  assert.equal(env.STORES_BROWSER_ADMIN_BINARY, path.join(root, 'target/phase25-platform-admin/debug/saas-platform-admin' + (process.platform === 'win32' ? '.exe' : '')))
  assert.throws(() => storesBrowserEnvironment({ TEST_DATABASE_URL: 'postgresql://ci:synthetic@production.invalid:5432/postgres', CI_TEST_POSTGRES_OWNER: ownership.owner }, { product: 'chrome', root, workDir }))
})
