import assert from 'node:assert/strict'

export const workOrdersBrowserProducts = Object.freeze(['chromium', 'chrome', 'msedge'])
export const workOrdersBrowserChecks = Object.freeze([
  'production-vite-module-and-dynamic-chunk', 'opaque-origin-null-rust-no-cookie', 'ordinary-user-real-authorization',
  'theme-no-extra-list-request', 'real-create-assignment-directory', 'real-update-conflict-preserves-draft',
  'explicit-conflict-rebase', 'real-status-filter', 'csv-host-mediated-download', 'reminder-inbox-authoritative',
  'reminder-committed-response-loss-recovery', 'create-committed-response-loss-recovery', 'create-detail-menu-isolation',
  'read-revocation-clears-server-content', 'real-soft-delete',
])
export const workOrdersBrowserCleanup = Object.freeze(['platformDatabaseRemoved', 'businessDatabaseRemoved', 'platformStopped',
  'businessStopped', 'nginxStopped', 'processesStopped', 'databasesRemoved', 'temporaryDirectoryRemoved', 'hostUnchanged', 'browserClosed'])
export const workOrdersBrowserSteps = Object.freeze(['nginx-version', 'nginx-runtime-libraries', 'openssl-version', 'tool-package-versions', 'owned-ca',
  'owned-server-request', 'owned-server-certificate', 'owned-tls-verify', 'owned-nss-xdg-create', 'owned-nss-xdg-trust',
  'owned-nss-xdg-verify', 'owned-nss-legacy-create', 'owned-nss-legacy-trust', 'owned-nss-legacy-verify', 'real-browser'])
const uuid = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/i
const digest = /^[a-f0-9]{64}$/
const assetPath = value => typeof value === 'string' && value.length > 0 && !value.startsWith('/') && !/[\\?#]/.test(value) && value.split('/').every(part => part && part !== '.' && part !== '..')

function validateTransport(transport) {
  assert.equal(transport.origin, 'null'); assert.equal(transport.cookiePresent, false)
  assert.equal(transport.credentials, 'omit', 'Actual native fetch options must explicitly omit credentials')
  assert.equal(transport.appBearerPresent, true); assert.equal(transport.requestIdPresent, true)
  assert.equal(transport.rustPreflightStatus, 204); assert.equal(transport.credentialsAllowed, false)
  const records = transport.records, faults = transport.faults
  assert.ok(Array.isArray(records) && records.length > 0, 'Every native fetch attempt must be accounted for')
  assert.ok(Array.isArray(faults) && faults.length === 2, 'Both committed-response faults must be individually verified')
  const counts = { nativeAttemptCount: records.length, browserRequestEventCount: 0, confirmedWireCount: 0, responseBodyCancelledCount: 0,
    abortBeforeRequestEventCount: 0, abortedWithoutWireConfirmationCount: 0, faultInjectedCount: 0 }
  const ids = new Set(), faultIds = new Set()
  const headers = value => {
    assert.equal(value.cookiePresent, false, 'Observed business headers cannot contain Cookie')
    assert.equal(value.bearerPresent, true); assert.equal(value.requestIdPresent, true)
  }
  for (const record of records) {
    assert.match(record.requestId, uuid); assert.ok(!ids.has(record.requestId), 'Native request IDs must be unique'); ids.add(record.requestId)
    assert.ok(['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD'].includes(record.method))
    assert.ok(typeof record.path === 'string' && record.path.startsWith('/work-orders-api/') && !/[\\?#\s]/.test(record.path) && !record.path.split('/').some(part => part === '.' || part === '..'))
    assert.equal(record.credentials, 'omit'); assert.equal(typeof record.signalAborted, 'boolean')
    assert.ok(['fulfilled', 'rejected'].includes(record.nativeOutcome), 'Every native fetch must settle')
    headers(record.nativeHeaders)
    const event = record.browserEvent
    if (event !== null) {
      counts.browserRequestEventCount++
      assert.equal(event.requestId, record.requestId, 'Browser event must match the observed native request ID')
      assert.equal(event.method, record.method); assert.equal(event.path, record.path)
      assert.ok(event.origin === 'null' || event.origin === null, 'Any observed Origin must be the opaque null origin')
      headers(event); assert.equal(event.frameType, 'iframe')
      assert.ok(typeof event.framePath === 'string' && event.framePath.startsWith('/sub-apps/work-orders/versions/phase21/'))
      assert.ok(['finished', 'failed'].includes(event.outcome), 'Browser request events must have a terminal outcome')
    }
    switch (record.classification) {
      case 'browser-response':
        counts.confirmedWireCount++
        assert.ok(event); assert.equal(event.origin, 'null'); assert.equal(event.outcome, 'finished')
        assert.ok(Number.isSafeInteger(event.responseStatus) && event.responseStatus >= 100 && event.responseStatus <= 599)
        assert.equal(event.failureCode, null); assert.equal(record.nativeOutcome, 'fulfilled'); assert.equal(record.nativeErrorName, null)
        break
      case 'browser-error-body-cancelled':
        counts.confirmedWireCount++; counts.responseBodyCancelledCount++
        assert.ok(event); assert.equal(event.origin, 'null'); assert.equal(event.outcome, 'failed')
        assert.ok(Number.isSafeInteger(event.responseStatus) && event.responseStatus >= 400 && event.responseStatus <= 599)
        assert.equal(event.failureCode, 'net::ERR_ABORTED')
        assert.equal(record.nativeOutcome, 'fulfilled'); assert.equal(record.nativeErrorName, null)
        break
      case 'aborted-before-request-event':
      case 'aborted-without-wire-confirmation':
        assert.equal(record.method, 'GET', 'Only a proved cancelled read may lack wire confirmation')
        assert.equal(record.signalAborted, true); assert.equal(record.nativeOutcome, 'rejected'); assert.equal(record.nativeErrorName, 'AbortError')
        if (record.classification === 'aborted-before-request-event') {
          counts.abortBeforeRequestEventCount++; assert.equal(event, null)
        } else {
          counts.abortedWithoutWireConfirmationCount++; assert.ok(event)
          assert.equal(event.outcome, 'failed'); assert.equal(event.responseStatus, null); assert.equal(event.failureCode, 'net::ERR_ABORTED')
        }
        break
      case 'fixture-response-drop':
        counts.faultInjectedCount++
        assert.equal(record.method, 'POST'); assert.equal(record.nativeOutcome, 'rejected'); assert.equal(record.nativeErrorName, 'TypeError')
        assert.ok(event); assert.equal(event.origin, 'null'); assert.equal(event.outcome, 'failed'); assert.equal(event.responseStatus, null); assert.equal(event.failureCode, 'net::ERR_CONNECTION_RESET')
        faultIds.add(record.requestId)
        break
      default: assert.fail('Unknown transport classification cannot be omitted from mandatory results')
    }
  }
  assert.ok(counts.confirmedWireCount > 0)
  for (const [name, count] of Object.entries(counts)) assert.equal(transport[name], count, `Transport count must be derived from all records: ${name}`)
  assert.equal(counts.faultInjectedCount, 2)
  assert.equal(counts.nativeAttemptCount, counts.confirmedWireCount + counts.abortBeforeRequestEventCount + counts.abortedWithoutWireConfirmationCount + counts.faultInjectedCount)
  assert.equal(counts.browserRequestEventCount, counts.nativeAttemptCount - counts.abortBeforeRequestEventCount)
  assert.equal(transport.actualRequestCount, counts.confirmedWireCount + counts.faultInjectedCount, 'Wire count excludes attempts without HTTP confirmation')
  assert.equal(transport.explicitOmitObservationCount, counts.nativeAttemptCount, 'Every native attempt needs its own explicit omit observation')
  assert.deepEqual(new Set(faults.map(fault => fault.kind)), new Set(['create', 'reminder']))
  assert.deepEqual(new Set(faults.map(fault => fault.requestId)), faultIds, 'Each response-drop record needs exactly one matching isolated upstream fault')
  for (const fault of faults) {
    assert.match(fault.requestId, uuid); assert.equal(fault.forwardedRequestId, fault.requestId)
    const record = records.find(record => record.requestId === fault.requestId)
    assert.equal(record.classification, 'fixture-response-drop')
    if (fault.kind === 'create') assert.equal(record.path, '/work-orders-api/tickets')
    else assert.match(record.path, /^\/work-orders-api\/tickets\/[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}\/remind$/i)
    assert.equal(fault.upstreamStatus, fault.kind === 'create' ? 201 : 200)
    assert.equal(fault.freshCookieJar, true); assert.equal(fault.normalTls, true); assert.equal(fault.disposed, true)
    assert.equal(fault.forwardedOrigin, 'null'); assert.equal(fault.forwardedCookiePresent, false)
    assert.equal(fault.forwardedBearerPresent, true); assert.equal(fault.forwardedRequestIdPresent, true)
  }
}

export function validateWorkOrdersBrowserResult(value, { product } = {}) {
  assert.ok(workOrdersBrowserProducts.includes(product), 'Select a declared real browser product')
  const { result, cleanup } = value
  assert.equal(result.schemaVersion, 1)
  assert.equal(result.status, 'PASS')
  assert.equal(result.mode, 'actual-production-host-vite-opaque-rust-postgres-nginx')
  assert.ok(Array.isArray(result.checks) && result.checks.length === workOrdersBrowserChecks.length)
  assert.ok(workOrdersBrowserChecks.every(id => result.checks.filter(check => check?.id === id && check.status === 'PASS').length === 1))
  assert.equal(result.passed, workOrdersBrowserChecks.length)
  assert.equal(result.failed, 0); assert.equal(result.skipped, 0)
  for (const key of workOrdersBrowserCleanup) assert.equal(cleanup[key], true, `Missing owned cleanup: ${key}`)
  assert.deepEqual(result.cleanup, cleanup)
  assert.equal(result.product, product); assert.match(result.sourceSha256, digest)
  const e = result.evidence
  assert.equal(e.browser.product, product)
  assert.ok(typeof e.browser.version === 'string' && e.browser.version.length > 0)
  assert.equal(e.browser.playwrightVersion, '1.64.1')
  assert.deepEqual(e.ordinary, { superAdmin: false, managementPermissions: false, directMembership: true })
  assert.equal(e.assets.sandbox, 'allow-scripts allow-forms'); assert.equal(e.assets.actualVueMounted, true)
  assert.ok(assetPath(e.assets.moduleEntry))
  assert.ok(Array.isArray(e.assets.dynamicChunks) && e.assets.dynamicChunks.length > 0)
  assert.ok(Array.isArray(e.assets.manifestDynamicImports) && e.assets.manifestDynamicImports.length > 0 && e.assets.manifestDynamicImports.every(assetPath))
  assert.ok(e.assets.dynamicChunks.every(assetPath))
  assert.equal(new Set([e.assets.moduleEntry, ...e.assets.dynamicChunks]).size, e.assets.dynamicChunks.length + 1)
  for (const resource of [e.assets.moduleEntry, ...e.assets.dynamicChunks]) {
    const loaded = e.assets.loaded.filter(item => item.path.split(/[?#]/, 1)[0] === '/sub-apps/work-orders/versions/phase21/' + resource)
    assert.ok(loaded.some(item => item.status === 200 && /javascript/i.test(item.contentType)
      && ['*', 'null'].includes(item.allowOrigin) && item.origin === 'null' && item.resourceType === 'script'), `Actual CORS/module response missing: ${resource}`)
  }
  validateTransport(e.transport)
  assert.ok(Number.isSafeInteger(e.theme.before) && e.theme.before >= 0); assert.equal(e.theme.after, e.theme.before)
  assert.match(e.crud.ticketId, uuid); assert.equal(e.crud.createdStatus, 201); assert.equal(e.crud.assignee, true)
  assert.equal(e.crud.directoryRustRequest, true); assert.equal(e.crud.status, 'IN_PROGRESS')
  assert.equal(e.crud.filterVerified, true); assert.equal(e.crud.softDeleted, true)
  for (const field of ['realRustExport', 'hostDownload', 'sandboxUnchanged']) assert.equal(e.crud.csv[field], true)
  assert.equal(e.conflict.status, 409); assert.equal(e.conflict.draftPreserved, true); assert.equal(e.conflict.automaticOverwrite, false)
  assert.ok(Number.isSafeInteger(e.conflict.explicitVersionAccepted) && e.conflict.explicitVersionAccepted > 1)
  assert.equal(e.reminder.publicationCount, 1); assert.equal(e.reminder.deliveryCount, 1)
  assert.equal(e.reminder.recipientInboxVerified, true); assert.equal(e.reminder.ticketVersionUnchanged, true)
  assert.equal(e.reminder.realtimeStarted, false)
  for (const recovery of [e.reminderRecovery, e.createRecovery]) {
    for (const key of ['committedResponseDropped', 'sameBody', 'sameKey']) assert.equal(recovery[key], true)
    assert.equal(recovery.automaticRetryCount, 0)
  }
  assert.equal(e.reminderRecovery.publicationCount, 1); assert.equal(e.reminderRecovery.deliveryCount, 1)
  assert.equal(e.createRecovery.receiptCount, 1); assert.equal(e.createRecovery.ticketCount, 1)
  for (const key of ['createCannotRead', 'readCannotCreate', 'actualContextAndRust403']) assert.equal(e.menuIsolation[key], true)
  assert.equal(e.revocation.actualRustStatus, 403); assert.equal(e.revocation.oldRowsVisible, false)
  const source = result.source
  for (const binary of ['platformBinary', 'businessBinary']) assert.match(source[binary].sha256, digest)
  for (const tree of [source.productionHost, source.viteBuild]) {
    assert.ok(Array.isArray(tree) && tree.length > 1)
    assert.equal(new Set(tree.map(item => item.path)).size, tree.length)
    assert.ok(tree.some(item => item.path === 'index.html'))
    for (const item of tree) { assert.ok(assetPath(item.path)); assert.match(item.sha256, digest); assert.ok(Number.isSafeInteger(item.bytes) && item.bytes > 0) }
  }
  for (const resource of ['build-info.json', 'asset-manifest.json', e.assets.moduleEntry, ...e.assets.dynamicChunks]) assert.ok(source.viteBuild.some(item => item.path === resource))
  assert.equal(source.releaseManifest.environment, 'development'); assert.equal(source.releaseManifest.version, 'phase21')
  assert.equal(source.releaseManifest.entryType, 'HTML'); assert.equal(source.releaseManifest.entryUrl, '/sub-apps/work-orders/versions/phase21/index.html')
  const resources = source.releaseManifest.resources
  assert.ok(Array.isArray(resources) && resources.length === source.viteBuild.length - 1)
  for (const item of source.viteBuild.filter(item => item.path !== 'index.html')) assert.equal(resources.filter(resource => resource.url === item.path).length, 1)
  assert.equal(source.developmentLoopbackUpstreams, true); assert.equal(source.gatewayTlsVerified, true)
  if (e.screenshots !== undefined) {
    assert.ok(Array.isArray(e.screenshots) && e.screenshots.length > 0)
    for (const screenshot of e.screenshots) { assert.ok(typeof screenshot.path === 'string' && /[\\/]screenshots[\\/][a-z0-9-]+\.png$/.test(screenshot.path)); assert.match(screenshot.sha256, digest) }
  }
  return value
}

export function parseWorkOrdersBrowserOutput(output, options) {
  const one = prefix => {
    const lines = String(output).split(/\r?\n/).filter(line => line.startsWith(prefix))
    assert.equal(lines.length, 1, `Expected exactly one ${prefix.trim()} record`)
    return JSON.parse(lines[0].slice(prefix.length))
  }
  return validateWorkOrdersBrowserResult({ result: one('PHASE21_BROWSER_RESULT '), cleanup: one('PHASE21_BROWSER_CLEANUP ') }, options)
}

export function validateWorkOrdersBrowserReport(report, { product } = {}) {
  assert.equal(report.schemaVersion, 1); assert.equal(report.kind, 'work-orders-real-platform-browser')
  assert.equal(report.status, 'PASS'); assert.equal(report.product, product)
  assert.equal(report.privateWorkRemoved, true)
  assert.ok(Array.isArray(report.cleanupErrors) && report.cleanupErrors.length === 0)
  assert.equal(report.sourceUnchanged, true)
  assert.match(report.sourceRevision, /^[a-f0-9]{40}$/); assert.match(report.inputsSha256, /^[a-f0-9]{64}$/)
  assert.match(report.ownership.owner, /^ci-[a-z0-9-]{8,80}$/)
  assert.equal(report.ownership.hostname, report.ownership.owner + '-postgres'); assert.equal(report.ownership.database, 'postgres')
  assert.deepEqual(report.steps.map(step => step.id), workOrdersBrowserSteps)
  assert.ok(report.steps.every(step => step.status === 'PASS' && step.exitCode === 0 && step.timedOut === false))
  assert.equal(report.tools.node, 'v24.18.0'); assert.match(report.tools.nginx, /nginx version: nginx\/1\.28\.0(?:\s|$)/)
  assert.match(report.tools.nginxLibraries, /libc\.so/); assert.doesNotMatch(report.tools.nginxLibraries, /not found/i)
  assert.match(report.tools.openssl, /^OpenSSL [0-9.]+/)
  for (const name of ['nginx', 'openssl', 'libnss3-tools']) assert.match(report.tools.packages, new RegExp('^' + name + '=[^\\r\\n]+$', 'm'))
  for (const [name, key] of [['platform', 'platformBinary'], ['business', 'businessBinary']]) {
    const binary = report.binaries[name]
    assert.match(binary.sha256, digest); assert.ok(binary.bytes >= 20 && Number.isSafeInteger(binary.bytes)); assert.ok([62, 183].includes(binary.elfMachine))
    assert.equal(binary.sha256, report.result.source[key].sha256)
  }
  for (const key of ['caSha256', 'serverSha256']) assert.match(report.tls[key], digest)
  assert.equal(report.tls.ipVerified, true); assert.equal(report.tls.ownedTrustOnly, true)
  return validateWorkOrdersBrowserResult(report, { product })
}
