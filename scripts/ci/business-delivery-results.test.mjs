import test from 'node:test'
import assert from 'node:assert/strict'
import { existsSync, mkdirSync, mkdtempSync, realpathSync, rmSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { businessBrowserChecks, businessBrowserProducts, businessDeliveryChecks, validateBusinessDeliveryReport, parseBusinessBrowserOutput } from './business-delivery-results.mjs'
import { runBusinessDelivery } from './business-delivery.mjs'
const revision = 'a'.repeat(40), inputs = 'b'.repeat(64), digest = 'sha256:' + 'c'.repeat(64)
function valid() {
  return { schemaVersion: 1, kind: 'business-delivery', status: 'PASS', sourceRevision: revision, inputsSha256: inputs, targetPlatform: 'linux/amd64', sourceUnchanged: true,
    checks: businessDeliveryChecks.map(id => ({ id, status: 'PASS' })),
    browserProducts: businessBrowserProducts.map(product => ({ product, status: 'PASS', sourceRevision: revision, inputsSha256: inputs, normalTLS: true,
      checks: businessBrowserChecks.map(id => ({ id, status: 'PASS' })), cleanup: { databaseConnectionsClosed: true, browserClosed: true } })),
    registry: { scope: 'isolated', push: 'PASS', pull: 'PASS', candidateAbsentBeforePull: true, registryRequestsObserved: true, productionPublication: false },
    images: ['work-orders-v1', 'work-orders-v2', 'stores-v1', 'stores-v2', 'work-orders-api', 'stores-api'].map(key => ({ key, imageId: digest, reference: 'localhost:5000/' + key + '@' + digest, digest, sourceRevision: revision, inputsSha256: inputs, buildId: 'test-build' })),
    unhealthy: { status: 'BLOCKED', calls: { routes: 0, register: 0, activate: 0 } }, cleanup: { ownedContainersRemoved: true, ownedNetworksRemoved: true, ownedVolumesRemoved: true, privateWorkRemoved: true, errors: [] } }
}
test('accepts a complete real delivery result with exact source identity', () => assert.equal(validateBusinessDeliveryReport(valid(), { sourceRevision: revision, inputsSha256: inputs }).status, 'PASS'))
test('rejects false success, missing browser work and stale source', () => {
  for (const mutate of [r => { r.status = 'FAIL' }, r => { r.cancelled = true }, r => { r.sourceUnchanged = false }, r => { r.browserProducts.pop() }, r => { r.browserProducts[0].checks[0].status = 'SKIP' }, r => { r.browserProducts[1].inputsSha256 = 'd'.repeat(64) }, r => { r.checks.push(r.checks[0]) }]) {
    const report = valid(); mutate(report); assert.throws(() => validateBusinessDeliveryReport(report, { sourceRevision: revision, inputsSha256: inputs }))
  }
})
test('rejects local-only publication, mutable images and cleanup failures', () => {
  for (const mutate of [r => { r.registry.pull = 'NOT_RUN' }, r => { r.registry.candidateAbsentBeforePull = false }, r => { r.registry.registryRequestsObserved = false }, r => { r.images[0].reference = 'app:latest' }, r => { r.unhealthy.calls.register = 1 }, r => { r.cleanup.errors.push('owner mismatch') }, r => { r.browserProducts[0].normalTLS = false }]) {
    const report = valid(); mutate(report); assert.throws(() => validateBusinessDeliveryReport(report))
  }
})
test('replays original browser PASS lines and typed source evidence together', () => {
  const result = { schemaVersion: 1, kind: 'phase28-real-image-delivery-browser', status: 'PASS', product: 'chromium', sourceRevision: revision, inputsSha256: inputs, sourceSha256: 'd'.repeat(64),
    checks: businessBrowserChecks.map(id => ({ id, status: 'PASS' })), passed: businessBrowserChecks.length, failed: 0, skipped: 0, normalTLS: true, browserVersion: '154.0.0.0',
    evidence: Object.fromEntries(businessBrowserChecks.map(id => [id, {}])), transport: { records: [{ requestId: 'observed' }], businessRequests: [{ status: 200 }] }, cleanup: { databaseConnectionsClosed: true, browserClosed: true } }
  const raw = businessBrowserChecks.map(id => 'PASS phase28 browser ' + id).join('\n') + '\nPHASE28_BROWSER ' + JSON.stringify(result) + '\n'
  const expected = { product: 'chromium', sourceRevision: revision, inputsSha256: inputs, sourceSha256: 'd'.repeat(64) }
  assert.equal(parseBusinessBrowserOutput(raw, expected).product, 'chromium')
  for (const changed of [raw + 'PHASE28_BROWSER ' + JSON.stringify(result), raw.replace('PASS phase28 browser ', 'SKIP phase28 browser '), raw.replace('154.0.0.0', ''), raw.replace('"failed":0', '"failed":1')]) assert.throws(() => parseBusinessBrowserOutput(changed, expected))
  assert.throws(() => parseBusinessBrowserOutput(raw, { ...expected, sourceSha256: 'e'.repeat(64) }))
})
test('SIGTERM cancellation still audits owners, removes private work and closes its lease', async () => {
  const temporary = realpathSync(mkdtempSync(path.join(os.tmpdir(), 'phase28-cancel-regression-'))), sourceRoot = path.join(temporary, 'source'), outDir = path.join(temporary, 'evidence')
  mkdirSync(sourceRoot); const calls = [], priorExitCode = process.exitCode; let stopped = false
  try {
    const result = await runBusinessDelivery({ sourceRoot, outDir, sourceRevision: revision, targetPlatform: 'linux/amd64',
      startLease: async () => ({ evidence: { ready: true }, async stop() { stopped = true } }),
      docker: async args => {
        calls.push(args)
        if (args[0] === 'image') { process.emit('SIGTERM'); throw new Error('synthetic interrupted image inspection') }
        if (args[0] === 'ps' || args[0] === 'network' && args[1] === 'ls' || args[0] === 'volume' && args[1] === 'ls') return ''
        throw new Error('Unexpected destructive action in cancellation regression')
      } })
    assert.equal(result.status, 'FAIL'); assert.equal(result.cancelled, true); assert.equal(result.cancellationReason, 'SIGTERM')
    assert(result.finishedAt); assert.equal(result.cleanup.privateWorkRemoved, true); assert(!existsSync(path.join(outDir, 'private')))
    assert.equal(result.cleanup.ownedContainersRemoved, true); assert.equal(result.cleanup.ownedNetworksRemoved, true); assert.equal(result.cleanup.ownedVolumesRemoved, true)
    assert.equal(calls.filter(args => args.includes('--filter')).length, 3)
    if (process.platform === 'win32') assert.equal(stopped, true)
  } finally {
    process.exitCode = priorExitCode
    assert(path.dirname(temporary) === realpathSync(os.tmpdir()) && path.basename(temporary).startsWith('phase28-cancel-regression-'))
    rmSync(temporary, { recursive: true, force: true })
  }
})
