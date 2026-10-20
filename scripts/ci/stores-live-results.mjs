import assert from 'node:assert/strict'

export const storesFixtureChecks = Object.freeze([
  'empty_and_repeat_initialization', 'concurrent_daily_uniqueness_no_double_summary', 'numeric_precision_and_large_exact_sum',
  'immutable_ownership_date_and_optimistic_version', 'staff_active_uniqueness_and_removed_history',
  'composite_store_tenant_organization_fk', 'atomic_audit_rollback_and_append_only', 'schema_drift_and_checksum_rejected',
])
export const storesLiveChecks = Object.freeze([
  'real-login-and-confidential-introspection', 'store-staff-daily-crud-and-summary', 'self-scope-and-owner-not-staff', 'org-scope',
  'subtree-scope', 'tenant-and-hard-organization-ceiling', 'permission-and-grant-independent-denials',
  'cross-app-token-and-client-binding-denials', 'revocation-next-request', 'member-directory-and-association',
  'inactive-expired-outside-members-denied', 'immutable-tenant-organization-owner', 'daily-unique-concurrency-no-double-summary',
  'optimistic-version-conflicts', 'decimal-and-shanghai-business-date', 'platform-fail-closed-and-secret-free-output',
])
const rustPassed = Object.freeze({ passed: 1, failed: 0, skipped: 0 })
const normalize = output => String(output).split(/\r?\n/).map(line => line.replace(/^test \S+ \.\.\. (?=STORES_)/, '')).join('\n')
function rustResult(output) {
  const entries = [...output.matchAll(/^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; \d+ measured; \d+ filtered out[^\r\n]*$/gm)]
  assert.equal(entries.length, 1, 'Exactly one selected stores Rust test result is required')
  assert.equal(entries[0][1], 'ok')
  const result = { passed: Number(entries[0][2]), failed: Number(entries[0][3]), skipped: Number(entries[0][4]) }
  assert.deepEqual(result, rustPassed)
  return result
}
function checkSet(checks, expected) {
  assert.ok(Array.isArray(checks) && checks.length === expected.length)
  for (const id of expected) assert.equal(checks.filter(check => check?.id === id && check.status === 'PASS').length, 1, `Missing or repeated mandatory stores check: ${id}`)
}
function jsonRecord(output, marker) {
  const lines = output.split('\n').filter(line => line === marker || line.startsWith(marker + ' '))
  assert.equal(lines.length, 1, `Exactly one ${marker} record is required`)
  return JSON.parse(lines[0].slice(marker.length + 1))
}

export function validateStoresFixtureResult(document) {
  assert.deepEqual(document.rust, rustPassed)
  checkSet(document.checks, storesFixtureChecks)
  assert.deepEqual(document.cleanup, { assertionsOk: true, databaseRemoved: true, poolClosed: true })
  return document
}
export function parseStoresFixtureOutput(output) {
  output = normalize(output)
  const checkLines = output.split('\n').filter(line => line === 'STORES_SQL_CHECK' || line.startsWith('STORES_SQL_CHECK '))
  const checks = checkLines.map(line => {
    const match = /^STORES_SQL_CHECK name=([a-z0-9_]+) status=(PASS|FAIL|SKIP)\s*$/.exec(line)
    assert.ok(match, 'Malformed stores SQL check cannot be ignored')
    return { id: match[1], status: match[2] }
  })
  const lines = output.split('\n').filter(line => line === 'STORES_FIXTURE_CLEANUP' || line.startsWith('STORES_FIXTURE_CLEANUP '))
  assert.equal(lines.length, 1, 'Exactly one owned stores SQL database cleanup is required')
  const match = /^STORES_FIXTURE_CLEANUP assertions_ok=(true|false) database_removed=(true|false) pool_closed=(true|false)\s*$/.exec(lines[0])
  assert.ok(match, 'Malformed owned stores SQL cleanup cannot be ignored')
  return validateStoresFixtureResult({ rust: rustResult(output), checks,
    cleanup: { assertionsOk: match[1] === 'true', databaseRemoved: match[2] === 'true', poolClosed: match[3] === 'true' } })
}

export function validateStoresLiveResult(document) {
  assert.deepEqual(document.rust, rustPassed)
  assert.equal(document.result.schemaVersion, 1)
  assert.equal(document.result.transport, 'explicit-development-loopback-http')
  assert.equal(document.result.passed, storesLiveChecks.length); assert.equal(document.result.failed, 0); assert.equal(document.result.skipped, 0)
  checkSet(document.result.checks, storesLiveChecks)
  const database = document.database
  assert.deepEqual(Object.keys(database).sort(), ['distinctAccounts', 'nonSuperuser', 'crossDatabaseConnectDenied', 'storesRows', 'staffRows', 'dailyRows', 'grantRows', 'grantAudits', 'grantWritesOnlyViaHttp'].sort())
  for (const key of ['distinctAccounts', 'nonSuperuser', 'crossDatabaseConnectDenied', 'grantWritesOnlyViaHttp']) assert.equal(database[key], true, `Missing actual database isolation proof: ${key}`)
  for (const key of ['storesRows', 'staffRows', 'dailyRows', 'grantRows', 'grantAudits']) assert.ok(Number.isSafeInteger(database[key]) && database[key] > 0, `Missing actual database rows: ${key}`)
  const cleaned = { assertionsOk: true, databaseRemoved: true, roleRemoved: true, poolClosed: true }
  for (const key of ['platformCleanup', 'storesCleanup', 'workOrdersCleanup']) assert.deepEqual(document[key], cleaned)
  assert.deepEqual(document.processCleanup, { storesStopped: true, workOrdersStopped: true, platformStopped: true })
  return document
}
export function parseStoresLiveOutput(output) {
  output = normalize(output)
  return validateStoresLiveResult({ rust: rustResult(output), result: jsonRecord(output, 'STORES_LIVE_RESULT'), database: jsonRecord(output, 'STORES_LIVE_DATABASE'),
    platformCleanup: jsonRecord(output, 'STORES_LIVE_PLATFORM_CLEANUP'), storesCleanup: jsonRecord(output, 'STORES_LIVE_STORES_CLEANUP'),
    workOrdersCleanup: jsonRecord(output, 'STORES_LIVE_WORK_ORDERS_CLEANUP'), processCleanup: jsonRecord(output, 'STORES_LIVE_PROCESS_CLEANUP') })
}
