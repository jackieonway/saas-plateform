import assert from 'node:assert/strict'
import test from 'node:test'
import { parsePlatformAdminOutput, platformAdminChecks, platformAdminTest } from './platform-admin.mjs'

const expectedChecks = [
  'plan-and-no-apply-no-database-writes',
  'bootstrap-repeat-preserves-ids-and-counts',
  'actual-client-id-associated-with-own-application',
  'foreign-client-and-conflicting-menu-rejected',
  'register-only-existing-allowed-fields',
  'existing-release-version-rejected-and-queryable',
  'url-preflight-two-activations-deployments-rollback',
  'grant-plan-and-no-apply-offline-readonly',
  'grant-apply-formal-api-and-exact-direct-membership',
  'grant-independent-read-write-and-root-context',
  'grant-no-permission-organization-and-cross-app-denied',
  'same-token-next-context-and-member-scope',
  'grant-request-bound-audit-and-secret-free-output',
]

function evidence() {
  return {
    live: { schemaVersion: 1, passed: 13, failed: 0, skipped: 0,
      checks: expectedChecks.map(id => ({ id, status: 'PASS' })) },
    database: { bootstrapApplications: 1, bootstrapMenus: 7, urlReleases: 2, zipArtifacts: 0,
      grantAudits: 3, actualClientIdAssociated: true, grantWritesOnlyViaHttp: true },
  }
}

const cleanup = 'entity creation cleanup assertions_ok=true database_removed=true pool_closed=true'
const rust = 'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 428 filtered out'

function output({ live, database } = evidence()) {
  return `test ${platformAdminTest} ... PHASE25_LIVE_DATABASE ${JSON.stringify(database)}\n` +
    `PHASE25_LIVE_RESULT ${JSON.stringify(live)}\n${cleanup}\nok\n${rust}\n`
}

test('Phase25 parser accepts exactly 13 declared checks, SQL proof, cleanup, and Cargo line prefix', () => {
  assert.equal(platformAdminTest, 'platform_admin_cli_tests::real_cli_platform_round_trip')
  assert.deepEqual(platformAdminChecks, expectedChecks)
  assert.equal(new Set(platformAdminChecks).size, 13)
  assert(Object.isFrozen(platformAdminChecks))
  for (const text of [output(), output().replaceAll('\n', '\r\n'), output().replace(`test ${platformAdminTest} ... `, '')]) {
    const parsed = parsePlatformAdminOutput(text)
    assert.deepEqual(parsed.rust, { passed: 1, failed: 0, skipped: 0 })
    assert.deepEqual(parsed.live, evidence().live)
    assert.deepEqual(parsed.database, evidence().database)
    assert.deepEqual(parsed.cleanup, { assertionsOk: true, databaseRemoved: true, poolClosed: true })
  }
})

test('Phase25 parser rejects missing and duplicate result or SQL markers', () => {
  for (const marker of ['PHASE25_LIVE_RESULT', 'PHASE25_LIVE_DATABASE']) {
    assert.throws(() => parsePlatformAdminOutput(output().replace(marker, 'MISSING_MARKER')), marker + ' missing')
    const value = marker === 'PHASE25_LIVE_RESULT' ? evidence().live : evidence().database
    assert.throws(() => parsePlatformAdminOutput(output() + `${marker} ${JSON.stringify(value)}\n`), marker + ' duplicate')
  }
  assert.throws(() => parsePlatformAdminOutput(output().replace('PHASE25_LIVE_RESULT {', 'PHASE25_LIVE_RESULT invalid {')))
})

test('Phase25 parser rejects missing, duplicate, unknown, and extra check IDs', () => {
  const mutations = [
    live => live.checks.pop(),
    live => { live.checks[1] = structuredClone(live.checks[0]) },
    live => { live.checks[0].id = 'undeclared-check' },
    live => live.checks.push({ id: 'extra-check', status: 'PASS' }),
    live => { delete live.checks },
  ]
  for (const mutate of mutations) {
    const value = evidence()
    mutate(value.live)
    assert.throws(() => parsePlatformAdminOutput(output(value)))
  }
})

test('Phase25 parser rejects failed or skipped checks and inconsistent live totals', () => {
  for (const status of ['FAIL', 'SKIP', 'NOT_RUN', 'PASS_WITH_SKIP']) {
    const value = evidence()
    value.live.checks[0].status = status
    assert.throws(() => parsePlatformAdminOutput(output(value)), status)
  }
  for (const [key, replacement] of [['schemaVersion', 2], ['passed', 12], ['passed', 14], ['failed', 1], ['skipped', 1]]) {
    const value = evidence()
    value.live[key] = replacement
    assert.throws(() => parsePlatformAdminOutput(output(value)), key)
  }
})

test('Phase25 parser requires every exact SQL count and HTTP-only grant proof', () => {
  const replacements = { bootstrapApplications: 2, bootstrapMenus: 6, urlReleases: 1, zipArtifacts: 1,
    grantAudits: 2, actualClientIdAssociated: false, grantWritesOnlyViaHttp: false }
  for (const [key, replacement] of Object.entries(replacements)) {
    const wrong = evidence()
    wrong.database[key] = replacement
    assert.throws(() => parsePlatformAdminOutput(output(wrong)), key + ' mismatched')
    const missing = evidence()
    delete missing.database[key]
    assert.throws(() => parsePlatformAdminOutput(output(missing)), key + ' absent')
  }
})

test('Phase25 parser rejects missing, duplicate, or unsuccessful owned PG cleanup', () => {
  assert.throws(() => parsePlatformAdminOutput(output().replace(cleanup, '')))
  assert.throws(() => parsePlatformAdminOutput(output() + cleanup + '\n'))
  for (const key of ['assertions_ok', 'database_removed', 'pool_closed']) {
    assert.throws(() => parsePlatformAdminOutput(output().replace(key + '=true', key + '=false')), key)
  }
})

test('Phase25 parser requires exactly one executed Rust test without failures or skips', () => {
  for (const summary of [
    '',
    rust.replace('1 passed', '0 passed'),
    rust.replace('1 passed', '2 passed'),
    rust.replace('0 failed', '1 failed'),
    rust.replace('0 ignored', '1 ignored'),
    rust.replace('result: ok', 'result: FAILED'),
    rust + '\n' + rust,
  ]) {
    assert.throws(() => parsePlatformAdminOutput(output().replace(rust, summary)), summary)
  }
})
