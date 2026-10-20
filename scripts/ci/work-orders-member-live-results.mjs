import assert from 'node:assert/strict'

export const workOrdersMemberLiveChecks = Object.freeze([
  'real-login-app-token-registered-members',
  'directory-search-pagination',
  'assigned-create-update-clear',
  'disabled-member-denied',
  'removed-direct-membership-denied',
  'missing-assignment-action-grant-denied',
  'direct-membership-and-organization-ceiling',
  'self-owner-not-assignee',
  'directory-wait-timeout-releases-business-resources',
  'preflight-locked-organization-mismatch',
])

export function validateWorkOrdersMemberFixtureResult(document, expected) {
  assert.equal(document?.cleanup?.length, expected)
  for (const cleanup of document.cleanup) assert.deepEqual(cleanup, { assertionsOk: true, databaseRemoved: true, poolClosed: true })
  assert.deepEqual(document.proofs, { lock: true, organization: true, expiry: true })
  return document
}

export function parseWorkOrdersMemberFixtureOutput(output, expected) {
  const normalized = output.split(/\r?\n/).map(line => line.replace(/^test \S+ \.\.\. (?=WORK_ORDERS_MEMBER_)/, '')).join('\n')
  const cleanup = [...normalized.matchAll(/^WORK_ORDERS_MEMBER_FIXTURE_CLEANUP assertions_ok=(true|false) database_removed=(true|false) pool_closed=(true|false)\s*$/gm)]
    .map(entry => ({ assertionsOk: entry[1] === 'true', databaseRemoved: entry[2] === 'true', poolClosed: entry[3] === 'true' }))
  const exact = marker => normalized.split('\n').filter(line => line.trimEnd() === marker).length === 1
  return validateWorkOrdersMemberFixtureResult({ cleanup, proofs: {
    lock: exact('WORK_ORDERS_MEMBER_LOCK_PROOF member_http_gate=true application_pool_idle=1 same_ticket_nowait=true timeout_left_row_unchanged=true'),
    organization: exact('WORK_ORDERS_MEMBER_ORGANIZATION_PROOF pre_read_org_changed=true version_unchanged=true assignment_rejected=true identity_trigger_reenabled=true'),
    expiry: exact('WORK_ORDERS_MEMBER_EXPIRY_PROOF assertion_ms=500 http_budget_consumed_ms=200 context_seconds=28 lock_wait_observed=true rollback=true'),
  } }, expected)
}

export function validateWorkOrdersMemberLiveResult(document) {
  assert.equal(document?.result?.schemaVersion, 1)
  assert.equal(document.result.transport, 'explicit-development-loopback-http')
  assert.equal(document.result.passed, workOrdersMemberLiveChecks.length)
  assert.equal(document.result.failed, 0)
  assert.equal(document.result.skipped, 0)
  assert.equal(document.result.checks?.length, workOrdersMemberLiveChecks.length)
  for (const name of workOrdersMemberLiveChecks) assert.equal(document.result.checks.filter(check => check.name === name && check.status === 'PASS').length, 1)
  assert.deepEqual(document.platformCleanup, { assertionsOk: true, databaseRemoved: true, poolClosed: true })
  assert.deepEqual(document.businessCleanup, { assertionsOk: true, databaseRemoved: true, poolClosed: true })
  assert.deepEqual(document.processCleanup, { stopped: true })
  return document
}

export function parseWorkOrdersMemberLiveOutput(output) {
  const prefix = 'test phase19_work_orders_tests::real_platform_member_directory_and_assignment ... '
  const normalized = output.split(/\r?\n/).map(line => line.startsWith(prefix) ? line.slice(prefix.length) : line).join('\n')
  const result = [...normalized.matchAll(/^WORK_ORDERS_MEMBER_LIVE_RESULT (\{[^\r\n]*\})\s*$/gm)]
  assert.equal(result.length, 1, 'Exactly one actual platform/work-orders member result is required')
  const cleanup = name => {
    const entries = [...normalized.matchAll(new RegExp(`^MEMBER_LIVE_${name}_CLEANUP assertions_ok=(true|false) database_removed=(true|false) pool_closed=(true|false)\\s*$`, 'gm'))]
    assert.equal(entries.length, 1, `Exactly one owned ${name} cleanup is required`)
    return { assertionsOk: entries[0][1] === 'true', databaseRemoved: entries[0][2] === 'true', poolClosed: entries[0][3] === 'true' }
  }
  const process = [...normalized.matchAll(/^MEMBER_LIVE_PROCESS_CLEANUP stopped=(true|false)\s*$/gm)]
  assert.equal(process.length, 1, 'Exactly one independent Rust process cleanup is required')
  return validateWorkOrdersMemberLiveResult({ result: JSON.parse(result[0][1]), platformCleanup: cleanup('PLATFORM'), businessCleanup: cleanup('BUSINESS'), processCleanup: { stopped: process[0][1] === 'true' } })
}
