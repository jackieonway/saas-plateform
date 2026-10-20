import assert from 'node:assert/strict'

export const storesBrowserProducts = Object.freeze(['chromium', 'chrome', 'msedge'])
export const storesBrowserChecks = Object.freeze([
  'production-react-vite-independent-build', 'opaque-origin-null-rust-no-cookie', 'ordinary-user-authorization-menu11',
  'store-create-manager-directory', 'store-update-status', 'staff-association-unassignment', 'daily-create-update-duplicate-summary',
  'staff-report-without-store-read', 'theme-keeps-draft-no-request', 'route-cancellation-and-not-found',
  'normal-renewal-keeps-legal-input', 'version-conflict-explicit-rebase', 'real-write-response-loss-readback',
  'write-only-unknown-no-retry', 'revocation403-freezes-clears', 'unauthorized401-clears',
  'dual-host-shared-session-org-switch', 'dual-host-refresh-logout-fences', 'stale-form-handler-cannot-cross-org',
  'cross-app-bearer-permission-grant-denials', 'dual-actual-controlled-member-http',
])
export const storesBrowserCleanup = Object.freeze(['platformDatabaseRemoved', 'businessDatabaseRemoved', 'platformStopped',
  'businessStopped', 'nginxStopped', 'processesStopped', 'databasesRemoved', 'databaseRolesRemoved', 'temporaryDirectoryRemoved', 'hostUnchanged', 'browserClosed'])
export const storesBrowserSteps = Object.freeze(['nginx-version', 'nginx-runtime-libraries', 'openssl-version', 'tool-package-versions', 'owned-ca',
  'owned-server-request', 'owned-server-certificate', 'owned-tls-verify', 'owned-nss-xdg-create', 'owned-nss-xdg-trust', 'owned-nss-xdg-verify',
  'owned-nss-legacy-create', 'owned-nss-legacy-trust', 'owned-nss-legacy-verify', 'real-browser'])
const digest = /^[a-f0-9]{64}$/, uuid = /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i
const safeAsset = value => typeof value === 'string' && value.length && !value.startsWith('/') && !/[\\?#]/.test(value) && value.split('/').every(part => part && part !== '.' && part !== '..')
export function validateStoresBrowserResult(value, { product } = {}) {
  const { result, cleanup } = value
  assert.equal(result.schemaVersion, 1); assert.equal(result.product, product); assert(storesBrowserProducts.includes(product))
  assert.match(result.browserVersion, /^\d+\./); assert.match(result.playwright, /^\d+\./); assert.match(result.sourceSha256, digest)
  assert.equal(result.passed, storesBrowserChecks.length); assert.equal(result.failed, 0); assert.equal(result.skipped, 0)
  assert.deepEqual(result.checks.map(item => item.id), storesBrowserChecks)
  assert(result.checks.every(item => item.status === 'PASS'))
  for (const key of storesBrowserCleanup) assert.equal(cleanup[key], true, 'Actual owned cleanup: ' + key)
  for (const key of storesBrowserChecks) assert(result.evidence[key] && typeof result.evidence[key] === 'object', 'Missing actual evidence: ' + key)
  const renewal = result.evidence['normal-renewal-keeps-legal-input']
  assert.equal(renewal.actualIssueStatus, 201); assert(Number.isSafeInteger(renewal.initialIssueCount) && renewal.initialIssueCount > 0)
  assert(renewal.finalIssueCount > renewal.initialIssueCount); assert.equal(renewal.sameIdentityDraft, '11.22')
  for (const key of ['platformBinary', 'businessBinary', 'workOrdersBinary', 'adminBinary']) assert.match(result.source[key].sha256, digest)
  for (const key of ['productionHost', 'viteBuild', 'workOrdersViteBuild']) {
    const files = result.source[key]; assert(Array.isArray(files) && files.length > 1); assert.equal(new Set(files.map(file => file.path)).size, files.length)
    assert(files.some(file => file.path === 'index.html'))
    for (const file of files) { assert(safeAsset(file.path)); assert.match(file.sha256, digest); assert(Number.isSafeInteger(file.bytes) && file.bytes > 0) }
  }
  assert.deepEqual(result.source.productionServices, ['platform', 'stores', 'work-orders']); assert.equal(result.source.gatewayTlsVerified, true)
  assert.equal(result.source.databases.length, 3); assert.equal(new Set(result.source.databases.map(item => item.name)).size, 3)
  for (const db of result.source.databases) { assert.match(db.name, /^phase27_[psw]_[a-f0-9]{32}$/); assert.equal(db.role, db.name + '_writer'); for (const key of ['rolsuper', 'rolcreatedb', 'rolcreaterole']) assert.equal(db[key], false) }
  assert.deepEqual(result.source.menus, { stores: 11, 'work-orders': 7 })
  for (const key of ['outsideRepository', 'npmCi', 'npmBuild', 'secretSentinelAbsent', 'emptyCacheBeforeInstall', 'noNodeModulesBeforeInstall']) assert.equal(result.source.separateFrontend[key], true)
  for (const key of ['rootNodeModulesDependency', 'personalCacheDependency']) assert.equal(result.source.separateFrontend[key], false)
  assert.equal(result.source.separateFrontend.cache.kind, 'fresh-owned-npm-cache'); assert.equal(result.source.separateFrontend.cache.personal, false)
  assert.deepEqual(result.source.separateFrontend.steps.map(step => step.id), ['standalone-npm-ci', 'standalone-npm-build'])
  assert(result.source.separateFrontend.steps.every(step => step.status === 'PASS' && step.exitCode === 0 && !step.timedOut && /^artifacts\/logs\/[a-z-]+\.log$/.test(step.log)))
  for (const code of ['stores', 'work-orders']) {
    const calls = result.source.cli.filter(call => call.applicationCode === code)
    assert(calls.filter(call => call.command === 'bootstrap' && call.applied && call.ok).length >= 2)
    assert(calls.some(call => call.command === 'grants-plan' && call.ok)); assert(calls.some(call => call.command === 'grants-apply' && call.applied && call.ok))
  }
  const transport = result.transport
  assert.equal(transport.origin, 'null'); assert.equal(transport.cookiePresent, false); assert.equal(transport.credentials, 'omit')
  assert.equal(transport.rustPreflightStatus, 204); assert.equal(transport.credentialsAllowed, false)
  assert(Array.isArray(transport.records) && transport.records.length > 0)
  assert.equal(transport.nativeAttemptCount, transport.records.length)
  const ids = new Set()
  for (const record of transport.records) {
    assert.match(record.requestId, uuid); assert(!ids.has(record.requestId)); ids.add(record.requestId)
    assert.match(record.path, /^\/(?:stores|work-orders)-api\//); assert.equal(record.credentials, 'omit')
    assert.equal(record.cookiePresent, false); assert.equal(record.bearerPresent, true)
    if (record.path.startsWith('/stores-api/')) assert.equal(record.idempotencyKeyPresent, false, 'Stores has no idempotency-key protocol')
    assert(['fulfilled', 'rejected'].includes(record.nativeOutcome))
    if (record.wire) {
      if (record.wire.origin !== 'null') { assert.equal(record.wire.origin, null); assert.equal(record.method, 'GET'); assert.equal(record.signalAborted, true); assert.equal(record.nativeOutcome, 'rejected'); assert.equal(record.wire.responseStatus, null) }
      assert.equal(record.wire.cookiePresent, false); assert.equal(record.wire.bearerPresent, true)
      if (record.wire.frameType !== 'iframe') { assert.equal(record.wire.frameType, 'retired'); assert.equal(record.method, 'GET'); assert.equal(record.signalAborted, true); assert.equal(record.nativeOutcome, 'rejected'); assert.equal(record.wire.responseStatus, null) }
    }
    else { assert.equal(record.method, 'GET'); assert.equal(record.signalAborted, true); assert.equal(record.nativeOutcome, 'rejected') }
  }
  assert(transport.records.some(record => record.wire?.responseStatus === 201))
  assert(Array.isArray(transport.faults) && transport.faults.length >= 2)
  for (const fault of transport.faults) { assert.match(fault.requestId, uuid); assert(ids.has(fault.requestId)); assert.equal(fault.actualUpstreamCommitted, true); assert.equal(fault.forwardedRequestId, fault.requestId); assert.equal(fault.automaticRetryCount, 0) }
  const members = result.members
  for (const code of ['stores', 'work-orders']) { assert(members[code].requests.length > 0); assert(members[code].records.length > 0); assert.equal(members[code].actualPlatformHttp, true) }
  assert.equal(members.stores.idField, 'id'); assert.equal(members['work-orders'].idField, 'userId')
  assert.equal(members.crossAppStatus, 401); assert.equal(members.platformCrossAppStatus, 403); assert.equal(members.undeclaredPurposeStatus, 400)
  assert(Array.isArray(result.evidence.screenshots) && result.evidence.screenshots.length > 0)
  for (const item of result.evidence.screenshots) { assert.match(item.path, /[\\/]screenshots[\\/][a-z0-9-]+\.png$/); assert.match(item.sha256, digest) }
  return value
}
export function parseStoresBrowserOutput(output, options) {
  const one = prefix => { const lines = String(output).split(/\r?\n/).filter(line => line.startsWith(prefix)); assert.equal(lines.length, 1, 'Exactly one ' + prefix.trim()); return JSON.parse(lines[0].slice(prefix.length)) }
  return validateStoresBrowserResult({ result: one('PHASE27_BROWSER_RESULT '), cleanup: one('PHASE27_BROWSER_CLEANUP ') }, options)
}
export function validateStoresBrowserReport(report, { product } = {}) {
  assert.equal(report.schemaVersion, 1); assert.equal(report.kind, 'stores-real-platform-browser'); assert.equal(report.status, 'PASS'); assert.equal(report.product, product)
  assert.equal(report.privateWorkRemoved, true); assert.deepEqual(report.cleanupErrors, []); assert.equal(report.sourceUnchanged, true)
  assert.match(report.sourceRevision, /^[a-f0-9]{40}$/); assert.match(report.inputsSha256, digest)
  assert.match(report.ownership.owner, /^ci-[a-z0-9-]{8,80}$/); assert.equal(report.ownership.hostname, report.ownership.owner + '-postgres')
  assert.equal(report.ownership.database, 'postgres'); assert.deepEqual(report.steps.map(step => step.id), storesBrowserSteps)
  assert(report.steps.every(step => step.status === 'PASS' && step.exitCode === 0 && !step.timedOut))
  assert.equal(report.tools.node, 'v24.18.0'); assert.match(report.tools.nginx, /nginx version: nginx\/1\.28\.0(?:\s|$)/)
  assert.match(report.tools.nginxLibraries, /libc\.so/); assert.doesNotMatch(report.tools.nginxLibraries, /not found/i)
  assert.match(report.tools.openssl, /^OpenSSL \d+\./)
  for (const [name, key] of [['platform', 'platformBinary'], ['business', 'businessBinary'], ['workOrders', 'workOrdersBinary'], ['admin', 'adminBinary']]) {
    const binary = report.binaries[name]; assert.match(binary.sha256, digest); assert(binary.bytes > 20); assert([62, 183].includes(binary.elfMachine)); assert.equal(binary.sha256, report.result.source[key].sha256)
  }
  for (const key of ['caSha256', 'serverSha256']) assert.match(report.tls[key], digest)
  assert.equal(report.tls.ipVerified, true); assert.equal(report.tls.ownedTrustOnly, true)
  return validateStoresBrowserResult(report, { product })
}
