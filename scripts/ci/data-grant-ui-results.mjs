import assert from 'node:assert/strict'

export const dataGrantUiTest = 'data_grant_ui_tests::browser_management_closes_grant_loop'
export const dataGrantUiProducts = ['chromium', 'chrome', 'msedge', 'firefox', 'webkit']
export const dataGrantUiChecks = Object.freeze([
  'root-empty-default-deny',
  'preview-readonly',
  'save-self-real-business',
  'scope-update-preserves-uuid',
  'same-scope-noop',
  'stale-version-conflict',
  'audit-request-correlation',
  'revoke-next-business-request',
  'readonly-root-no-write',
  'organization-admin-ui-and-forged403',
  'negative-target-capability-matrix',
  'independent-read-route-guard',
  'late-snapshot-target-switch',
  'late-preview-edit-fence',
  'late-response-session-fence',
  'unknown-save-reconciliation',
])
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

function oneMarker(output, marker) {
  const matches = [...output.matchAll(new RegExp(`${marker} (\\{[^\\r\\n]*\\})`, 'g'))]
  assert.equal(matches.length, 1, `Exactly one ${marker} result is required`)
  return JSON.parse(matches[0][1])
}

export function validateDataGrantUiReport(document, { product, expectedChecks = dataGrantUiChecks } = {}) {
  assert.ok(dataGrantUiProducts.includes(product), 'An approved product must be selected explicitly')
  assert.ok(expectedChecks.length > 0, 'The mandatory live UI check list must be registered')
  assert.equal(document?.schemaVersion, 1)
  assert.equal(document?.kind, 'real-platform-postgres-ui')
  assert.equal(document?.status, 'PASS')
  assert.equal(document?.product, product)
  assert.equal(document?.rust?.passed, 1)
  assert.equal(document?.rust?.failed, 0)
  assert.equal(document?.rust?.skipped, 0)
  const browser = document.browser
  assert.equal(browser?.schemaVersion, 1)
  assert.equal(browser?.product, product)
  assert.match(browser?.browserVersion || '', /^\d+\.\d+(?:\.\d+){0,2}$/)
  assert.equal(browser?.playwrightVersion, '1.64.1')
  assert.ok(Array.isArray(browser?.checks))
  assert.equal(browser.checks.length, expectedChecks.length)
  assert.ok(expectedChecks.every(id => browser.checks.filter(check => check.id === id && check.status === 'PASS').length === 1))
  assert.equal(browser.passed, expectedChecks.length)
  assert.equal(browser.failed, 0)
  assert.equal(browser.skipped, 0)
  assert.ok(Array.isArray(browser.requestIds) && browser.requestIds.length === 4)
  assert.ok(browser.requestIds.every(value => typeof value === 'string' && uuid.test(value)))
  assert.equal(new Set(browser.requestIds).size, browser.requestIds.length)
  const database = document.database
  assert.equal(database?.auditRows, 4)
  for (const key of ['requestIdsMatch', 'membershipUuidPreserved', 'grantRevoked']) assert.equal(database[key], true)
  assert.deepEqual(database.requestIds, browser.requestIds)
  assert.equal(database.auditRows, browser.requestIds.length)
  assert.deepEqual(browser.workOrders, { selfCount: 1, orgCount: 2, revokedStatus: 403,
    sameToken: true, independentDatabase: true, outsideNeverReturned: true })
  assert.equal(typeof browser.screenshot?.path, 'string')
  assert.ok(browser.screenshot.path.startsWith('/') && browser.screenshot.path.endsWith('/management-preview.png') && !browser.screenshot.path.split('/').includes('..'))
  assert.match(browser.screenshot?.sha256 || '', /^[a-f0-9]{64}$/)
  assert.deepEqual(document.cleanup, { assertionsOk: true, databaseRemoved: true, poolClosed: true })
  assert.deepEqual(document.businessCleanup, { assertionsOk: true, databaseRemoved: true, poolClosed: true })
  return { passed: expectedChecks.length, failed: 0, skipped: 0 }
}

export function parseDataGrantUiOutput(output, { product, expectedChecks = dataGrantUiChecks } = {}) {
  const summaries = [...output.matchAll(/test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; \d+ measured; (\d+) filtered out/g)]
  assert.equal(summaries.length, 1, 'Exactly one selected Rust test result is required')
  const [, status, passed, failed, skipped] = summaries[0]
  assert.equal(status, 'ok')
  const cleanup = [...output.matchAll(/entity creation cleanup assertions_ok=(true|false) database_removed=(true|false) pool_closed=(true|false)/g)]
  assert.equal(cleanup.length, 1, 'Exactly one owned database cleanup is required')
  const businessCleanup = [...output.matchAll(/PHASE15_BUSINESS_CLEANUP assertions_ok=(true|false) database_removed=(true|false) pool_closed=(true|false)/g)]
  assert.equal(businessCleanup.length, 1, 'Exactly one independent business database cleanup is required')
  const document = {
    schemaVersion: 1, kind: 'real-platform-postgres-ui', status: 'PASS', product,
    rust: { passed: Number(passed), failed: Number(failed), skipped: Number(skipped) },
    browser: oneMarker(output, 'PHASE15_LIVE_RESULT'),
    database: oneMarker(output, 'PHASE15_LIVE_DATABASE'),
    cleanup: { assertionsOk: cleanup[0][1] === 'true', databaseRemoved: cleanup[0][2] === 'true', poolClosed: cleanup[0][3] === 'true' },
    businessCleanup: { assertionsOk: businessCleanup[0][1] === 'true', databaseRemoved: businessCleanup[0][2] === 'true', poolClosed: businessCleanup[0][3] === 'true' },
  }
  validateDataGrantUiReport(document, { product, expectedChecks })
  return document
}
