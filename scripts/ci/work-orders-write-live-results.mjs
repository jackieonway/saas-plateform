import assert from 'node:assert/strict'

export const workOrdersWriteLiveChecks = Object.freeze([
  'real-login-context-confidential-introspection', 'formal-five-action-grants-preview',
  'create-defaults-normalized-replay', 'same-key-concurrent-single-row',
  'changed-body-idempotency-conflict', 'grant-revocation-blocks-replay',
  'cross-organization-idempotency-domain', 'optimistic-version-single-winner',
  'immutable-fields-and-assignee-fail-closed', 'state-transition-policy',
  'soft-delete-all-read-paths', 'create-receipt-transactional-storage',
])

export function validateWorkOrdersWriteLiveResult(document) {
  assert.equal(document?.result?.schemaVersion, 1)
  assert.equal(document.result.transport, 'explicit-development-loopback-http')
  assert.equal(document.result.passed, workOrdersWriteLiveChecks.length)
  assert.equal(document.result.failed, 0)
  assert.equal(document.result.skipped, 0)
  assert.equal(document.result.checks?.length, workOrdersWriteLiveChecks.length)
  for (const name of workOrdersWriteLiveChecks) assert.equal(document.result.checks.filter(check => check.name === name && check.status === 'PASS').length, 1)
  assert.deepEqual(document.platformCleanup, { assertionsOk: true, databaseRemoved: true, poolClosed: true })
  assert.deepEqual(document.businessCleanup, { assertionsOk: true, databaseRemoved: true, poolClosed: true })
  assert.deepEqual(document.processCleanup, { stopped: true })
  return document
}

export function parseWorkOrdersWriteLiveOutput(output) {
  const prefix = 'test phase18_work_orders_tests::real_platform_writes_and_idempotency ... '
  const normalized = output.split(/\r?\n/).map(line => line.startsWith(prefix) ? line.slice(prefix.length) : line).join('\n')
  const result = [...normalized.matchAll(/^WORK_ORDERS_WRITE_LIVE_RESULT (\{[^\r\n]*\})\s*$/gm)]
  assert.equal(result.length, 1, 'Exactly one actual platform/work-orders write result is required')
  const cleanup = name => {
    const entries = [...normalized.matchAll(new RegExp(`^WRITE_LIVE_${name}_CLEANUP assertions_ok=(true|false) database_removed=(true|false) pool_closed=(true|false)\\s*$`, 'gm'))]
    assert.equal(entries.length, 1, `Exactly one owned ${name} cleanup is required`)
    return { assertionsOk: entries[0][1] === 'true', databaseRemoved: entries[0][2] === 'true', poolClosed: entries[0][3] === 'true' }
  }
  const process = [...normalized.matchAll(/^WRITE_LIVE_PROCESS_CLEANUP stopped=(true|false)\s*$/gm)]
  assert.equal(process.length, 1, 'Exactly one independent Rust process cleanup is required')
  return validateWorkOrdersWriteLiveResult({ result: JSON.parse(result[0][1]), platformCleanup: cleanup('PLATFORM'), businessCleanup: cleanup('BUSINESS'), processCleanup: { stopped: process[0][1] === 'true' } })
}
