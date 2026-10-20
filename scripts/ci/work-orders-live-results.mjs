import assert from 'node:assert/strict'

export const workOrdersLiveChecks = Object.freeze([
  'real-confidential-introspection', 'missing-grant-denied',
  'self-scope-list-count-detail-export', 'org-scope-list-count-detail-export',
  'subtree-scope-list-count-detail-export', 'nonroot-tenant-platform-downgrade',
  'platform-root-tenant-scope', 'wrong-audience-denied', 'grant-removal-next-request',
  'logout-next-request', 'membership-removal-next-request',
  'platform-unreachable-fail-closed', 'loopback-http-explicit-development',
])

export function validateWorkOrdersLiveResult(document) {
  assert.equal(document?.result?.schemaVersion, 1)
  assert.equal(document.result.transport, 'explicit-development-loopback-http')
  assert.equal(document.result.passed, workOrdersLiveChecks.length)
  assert.equal(document.result.failed, 0)
  assert.equal(document.result.skipped, 0)
  assert.equal(document.result.checks?.length, workOrdersLiveChecks.length)
  for (const id of workOrdersLiveChecks) assert.equal(document.result.checks.filter(check => check.id === id && check.status === 'PASS').length, 1)
  assert.deepEqual(document.platformCleanup, { assertionsOk: true, databaseRemoved: true, poolClosed: true })
  assert.deepEqual(document.businessCleanup, { assertionsOk: true, databaseRemoved: true, poolClosed: true })
  assert.deepEqual(document.processCleanup, { stopped: true })
  return document
}

export function parseWorkOrdersLiveOutput(output) {
  const result = [...output.matchAll(/^WORK_ORDERS_LIVE_RESULT (\{[^\r\n]*\})\s*$/gm)]
  assert.equal(result.length, 1, 'Exactly one actual platform/work-orders result is required')
  const cleanup = name => {
    const entries = [...output.matchAll(new RegExp(`^WORK_ORDERS_LIVE_${name}_CLEANUP assertions_ok=(true|false) database_removed=(true|false) pool_closed=(true|false)\\s*$`, 'gm'))]
    assert.equal(entries.length, 1, `Exactly one owned ${name} cleanup is required`)
    return { assertionsOk: entries[0][1] === 'true', databaseRemoved: entries[0][2] === 'true', poolClosed: entries[0][3] === 'true' }
  }
  const process = [...output.matchAll(/^WORK_ORDERS_LIVE_PROCESS_CLEANUP stopped=(true|false)\s*$/gm)]
  assert.equal(process.length, 1, 'Exactly one independent Rust process cleanup is required')
  return validateWorkOrdersLiveResult({ result: JSON.parse(result[0][1]), platformCleanup: cleanup('PLATFORM'), businessCleanup: cleanup('BUSINESS'), processCleanup: { stopped: process[0][1] === 'true' } })
}
