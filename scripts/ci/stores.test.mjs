import assert from 'node:assert/strict'
import { lstatSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { ciUnitTests, postgresSuites, storesPostgresSuites, storesLivePostgresSuites } from './manifest.mjs'
import { runStoresPostgres, storesPostgresEnvironment } from './stores-postgres.mjs'
import { runStoresLivePostgres } from './stores-live-postgres.mjs'
import { parseStoresFixtureOutput, parseStoresLiveOutput, storesFixtureChecks, storesLiveChecks } from './stores-live-results.mjs'
import { createSourceManifest, sourceRoots } from './provenance.mjs'
import { createSnapshot } from './linux-runner.mjs'
import { evaluate } from './report.mjs'

const revision = 'a'.repeat(40)
const env = { CI_TEST_POSTGRES_OWNER: 'ci-12345678', TEST_DATABASE_URL: 'postgresql://ci:synthetic@ci-12345678-postgres:5432/postgres' }
const rust = 'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out'
const fixtureText = () => [...storesFixtureChecks.map(id => `STORES_SQL_CHECK name=${id} status=PASS`), 'STORES_FIXTURE_CLEANUP assertions_ok=true database_removed=true pool_closed=true', rust].join('\n')
function liveRecords() {
  const clean = () => ({ assertionsOk: true, databaseRemoved: true, roleRemoved: true, poolClosed: true })
  return { STORES_LIVE_RESULT: { schemaVersion: 1, transport: 'explicit-development-loopback-http', passed: 16, failed: 0, skipped: 0, checks: storesLiveChecks.map(id => ({ id, status: 'PASS' })) },
    STORES_LIVE_DATABASE: { distinctAccounts: true, nonSuperuser: true, crossDatabaseConnectDenied: true, storesRows: 2, staffRows: 3, dailyRows: 4, grantRows: 5, grantAudits: 6, grantWritesOnlyViaHttp: true },
    STORES_LIVE_PLATFORM_CLEANUP: clean(), STORES_LIVE_STORES_CLEANUP: clean(), STORES_LIVE_WORK_ORDERS_CLEANUP: clean(),
    STORES_LIVE_PROCESS_CLEANUP: { storesStopped: true, workOrdersStopped: true, platformStopped: true } }
}
const liveText = (records = liveRecords()) => [...Object.entries(records).map(([key, value]) => key + ' ' + JSON.stringify(value)), rust].join('\n')
const verdict = (format, document, options = {}) => evaluate({ format, document, exitCode: 0, ...options })
function temporary(t) {
  const parent = realpathSync(os.tmpdir()), root = realpathSync(mkdtempSync(path.join(parent, 'saas-stores-ci-policy-')))
  t.after(() => { assert.equal(path.dirname(realpathSync(root)), parent); assert.match(path.basename(root), /^saas-stores-ci-policy-/); assert(!lstatSync(root).isSymbolicLink()); rmSync(root, { recursive: true, force: true }) })
  return root
}
function put(root, file, content = 'reviewed public source') { const absolute = path.join(root, file); mkdirSync(path.dirname(absolute), { recursive: true }); writeFileSync(absolute, content) }

test('stores SQL uses its exact locked ignored parent while semantic proof keeps all eight database checks', async t => {
  const directory = temporary(t), calls = []
  assert.deepEqual(storesPostgresSuites, [{ name: 'stores-postgres-contracts', target: 'postgres_contracts', filter: 'postgres_contracts', expected: 1 }])
  const actual = readFileSync(new URL('../../business-apps/stores/backend/tests/postgres_contracts.rs', import.meta.url), 'utf8')
  assert.deepEqual([...actual.matchAll(/check\("([a-z0-9_]+)"\)/g)].map(match => match[1]), storesFixtureChecks)
  const report = await runStoresPostgres({ outDir: path.join(directory, 'pass'), env, runCargo: async (args, root, childEnv) => {
    calls.push(args); assert.equal(childEnv.DATABASE_URL, env.TEST_DATABASE_URL)
    assert.equal(childEnv.CARGO_TARGET_DIR, path.join(root, 'target/ci-rust'))
    return { code: 0, timedOut: false, output: fixtureText() }
  } })
  assert.deepEqual(calls, [['test', '--manifest-path', 'business-apps/stores/backend/Cargo.toml', '--locked', '--test', 'postgres_contracts', '-j', '2', 'postgres_contracts', '--', '--exact', '--ignored', '--test-threads=1', '--nocapture']])
  assert.equal(report.passed, 1); assert.equal(report.fixture.checks.length, 8)
  assert.equal(verdict('stores-postgres', report).status, 'PASS')
  assert.equal(postgresSuites.length, 12); assert.equal(postgresSuites.reduce((sum, suite) => sum + suite.expected, 0), 76)
})

test('stores SQL cannot pass missing, duplicate, skipped, malformed or failed checks and cleanup', () => {
  const text = fixtureText()
  assert.equal(parseStoresFixtureOutput('test postgres_contracts ... ' + text.replaceAll('\n', '\r\n')).checks.length, 8)
  const invalid = ['', text + '\n' + rust, text.replace('1 passed', '0 passed'), text.replace('0 ignored', '1 ignored'),
    text.replace('status=PASS', 'status=SKIP'), text.replace('database_removed=true', 'database_removed=false'),
    text + '\nSTORES_SQL_CHECK', text + '\nSTORES_FIXTURE_CLEANUP', text + '\nSTORES_SQL_CHECK name=unknown status=PASS']
  for (const id of storesFixtureChecks) { const marker = `STORES_SQL_CHECK name=${id} status=PASS`; invalid.push(text.replace(marker, ''), text + '\n' + marker) }
  for (const output of invalid) assert.throws(() => parseStoresFixtureOutput(output))
})

test('live stores selects one exact real platform case and receives only owned configuration and both selected ELFs', async t => {
  const directory = temporary(t), calls = [], sentinel = { JWT_SECRET: 'private', DATABASE_URL: 'private', STORES_DATABASE_URL: 'private', PLATFORM_CLIENT_SECRET: 'private', NODE_OPTIONS: '--require private', STORES_BINARY: '/private/forged', WORK_ORDERS_BINARY: '/private/forged' }
  const report = await runStoresLivePostgres({ root: directory, outDir: path.join(directory, 'pass'), env: { ...env, ...sentinel }, runCargo: async (args, root, childEnv) => {
    calls.push(args); const extension = process.platform === 'win32' ? '.exe' : ''
    assert.equal(root, directory)
    assert.equal(childEnv.STORES_BINARY, path.join(directory, 'target/ci-rust/debug/stores-api' + extension))
    assert.equal(childEnv.WORK_ORDERS_BINARY, path.join(directory, 'target/ci-rust/debug/work-orders-api' + extension))
    assert.equal(childEnv.DATABASE_URL, env.TEST_DATABASE_URL)
    for (const key of ['JWT_SECRET', 'STORES_DATABASE_URL', 'PLATFORM_CLIENT_SECRET', 'NODE_OPTIONS']) assert.equal(childEnv[key], undefined)
    return { code: 0, timedOut: false, output: liveText() }
  } })
  assert.deepEqual(calls, [['test', '--manifest-path', 'backend/Cargo.toml', '--locked', '--bin', 'saas-platform-backend', '-j', '2', 'phase26_stores_tests::real_platform_stores_and_cross_app_isolation', '--', '--exact', '--ignored', '--test-threads=1', '--nocapture']])
  assert.equal(storesLivePostgresSuites.length, 1); assert.equal(storesLiveChecks.length, 16)
  assert.equal(report.passed, 1); assert.equal(report.live.result.passed, 16)
  assert.equal(verdict('stores-live-postgres', report).status, 'PASS')
  for (const invalid of [{}, { ...env, TEST_DATABASE_URL: 'postgresql://ci:synthetic@production.invalid:5432/postgres' }, { ...env, TEST_DATABASE_URL: env.TEST_DATABASE_URL + '#unowned' }]) {
    await assert.rejects(runStoresLivePostgres({ outDir: path.join(directory, 'unowned'), env: invalid }), /owned environment|temporary runner|fragment/)
  }
  const clean = storesPostgresEnvironment({ ...env, ...sentinel }, directory)
  assert.equal(clean.STORES_BINARY, undefined); assert.equal(clean.WORK_ORDERS_BINARY, undefined)
})

test('live stores closes sixteen unique checks, three DB roles, stopped processes and actual account isolation', () => {
  const text = liveText(), records = liveRecords()
  const prefixed = 'test phase26_stores_tests::real_platform_stores_and_cross_app_isolation ... ' + text.replaceAll('\n', '\r\n')
  assert.equal(parseStoresLiveOutput(prefixed).result.passed, 16)
  const mutations = [
    value => { value.STORES_LIVE_RESULT.schemaVersion = '1' }, value => { value.STORES_LIVE_RESULT.transport = 'fixture' },
    value => { value.STORES_LIVE_RESULT.passed = 15 }, value => { value.STORES_LIVE_RESULT.failed = 1 }, value => { value.STORES_LIVE_RESULT.skipped = 1 },
    value => { value.STORES_LIVE_RESULT.checks.pop() }, value => { value.STORES_LIVE_RESULT.checks[0] = value.STORES_LIVE_RESULT.checks[1] },
    value => { value.STORES_LIVE_RESULT.checks[0].status = 'SKIP' }, value => { value.STORES_LIVE_RESULT.checks[0].id = 'unknown-check' },
    ...['distinctAccounts', 'nonSuperuser', 'crossDatabaseConnectDenied', 'grantWritesOnlyViaHttp'].map(key => value => { value.STORES_LIVE_DATABASE[key] = false }),
    ...['storesRows', 'staffRows', 'dailyRows', 'grantRows', 'grantAudits'].flatMap(key => [value => { value.STORES_LIVE_DATABASE[key] = 0 }, value => { value.STORES_LIVE_DATABASE[key] = '1' }, value => { delete value.STORES_LIVE_DATABASE[key] }]),
    value => { value.STORES_LIVE_DATABASE.privateConnection = 'must-not-publish' },
    ...['PLATFORM', 'STORES', 'WORK_ORDERS'].flatMap(kind => ['assertionsOk', 'databaseRemoved', 'roleRemoved', 'poolClosed'].map(key => value => { value[`STORES_LIVE_${kind}_CLEANUP`][key] = false })),
    ...['storesStopped', 'workOrdersStopped', 'platformStopped'].map(key => value => { value.STORES_LIVE_PROCESS_CLEANUP[key] = false }),
  ]
  for (const mutate of mutations) { const value = structuredClone(records); mutate(value); assert.throws(() => parseStoresLiveOutput(liveText(value))) }
  for (const marker of Object.keys(records)) {
    const value = structuredClone(records); delete value[marker]
    assert.throws(() => parseStoresLiveOutput(liveText(value)))
    assert.throws(() => parseStoresLiveOutput(text + '\n' + marker + ' ' + JSON.stringify(records[marker])))
    assert.throws(() => parseStoresLiveOutput(text + '\n' + marker))
  }
  for (const output of ['', text + '\n' + rust, text.replace('1 passed', '0 passed'), text.replace('0 ignored', '1 ignored')]) assert.throws(() => parseStoresLiveOutput(output))
})

test('structured stores gates reject wrong kinds, suite totals, failed commands or missing semantic evidence', async t => {
  const directory = temporary(t)
  for (const [format, run, output] of [['stores-postgres', runStoresPostgres, fixtureText()], ['stores-live-postgres', runStoresLivePostgres, liveText()]]) {
    const report = await run({ outDir: path.join(directory, format), env, runCargo: async () => ({ code: 0, timedOut: false, output }) })
    for (const mutate of [
      value => { value.kind = 'existing-platform-isolated-postgres' }, value => { value.status = 'FAIL' }, value => { value.passed = 16 }, value => { value.skipped = 1 },
      value => { value.failedSuites = 1 }, value => { value.suites = [] }, value => { value.suites.push(value.suites[0]) },
      value => { value.suites[0].exitCode = 1 }, value => { value.suites[0].timedOut = true }, value => { value.suites[0].expected = 2 },
      value => { value.ownership.hostname = 'production' }, value => { delete value.fixture; delete value.live },
    ]) { const changed = structuredClone(report); mutate(changed); assert.equal(verdict(format, changed).status, 'FAIL') }
    assert.equal(verdict('postgres', report).status, 'FAIL')
    assert.equal(verdict(format === 'stores-postgres' ? 'stores-live-postgres' : 'stores-postgres', report).status, 'FAIL')
    assert.equal(verdict(format, report, { exitCode: 1 }).status, 'FAIL'); assert.equal(verdict(format, report, { timedOut: true }).status, 'FAIL')
  }
})

test('stores drivers preserve actual failures and reject successful Rust without semantic cleanup while redacting secrets', async t => {
  const directory = temporary(t)
  for (const [label, run, output] of [['fixture', runStoresPostgres, fixtureText()], ['live', runStoresLivePostgres, liveText()]]) {
    const noProof = path.join(directory, label + '-missing')
    await assert.rejects(run({ outDir: noProof, env, runCargo: async () => ({ code: 0, timedOut: false, output: rust }) }), /incomplete/)
    const missing = JSON.parse(readFileSync(path.join(noProof, 'postgres-result.json'), 'utf8'))
    assert.equal(missing.status, 'FAIL'); assert.equal(missing.passed, 1); assert.equal(missing.failedSuites, 1)
    for (const [name, text, code, timedOut] of [
      ['failed', 'test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out', 1, false],
      ['ignored', 'test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out', 0, false],
      ['timeout', output, 0, true],
    ]) {
      const outDir = path.join(directory, label + '-' + name)
      await assert.rejects(run({ outDir, env, runCargo: async () => ({ code, timedOut, output: env.TEST_DATABASE_URL + '\nBearer synthetic-token\n' + text }) }), /PostgreSQL gate failed/)
      const report = JSON.parse(readFileSync(path.join(outDir, 'postgres-result.json'), 'utf8'))
      assert.equal(report.status, 'FAIL'); assert.equal(report.failedSuites, 1)
      assert.doesNotMatch(readFileSync(path.join(outDir, report.suites[0].log), 'utf8'), /synthetic|postgresql:\/\//)
      if (name === 'failed') assert.equal(report.failed, 1)
      if (name === 'ignored') assert.equal(report.skipped, 1)
    }
  }
})

test('source selection retains independent stores code, migrations, lock and public config while excluding generated and private inputs', t => {
  const directory = temporary(t), root = path.join(directory, 'source')
  const included = ['.gitignore', 'business-apps/stores/backend/Cargo.toml', 'business-apps/stores/backend/Cargo.lock', 'business-apps/stores/backend/config.example',
    'business-apps/stores/backend/policy.example.json', 'business-apps/stores/backend/src/lib.rs', 'business-apps/stores/backend/src/authorization/client.rs',
    'business-apps/stores/backend/migrations/0001_stores.sql', 'business-apps/stores/backend/tests/postgres_contracts.rs', 'backend/src/phase26_stores_tests.rs']
  for (const file of included) put(root, file)
  for (const file of ['business-apps/stores/backend/.env', 'business-apps/stores/backend/target/debug/stores-api', 'business-apps/stores/backend/private/client-secret']) put(root, file, 'private')
  const source = createSourceManifest(root, { sourceRevision: revision }), snapshot = createSnapshot(path.join(directory, 'snapshot'), root)
  assert.deepEqual(source.files.map(file => file.path).sort(), [...included].sort())
  assert.deepEqual(snapshot.files.map(file => file.path).sort(), [...included].sort())
  assert.equal(sourceRoots.filter(entry => entry === 'business-apps/stores/backend').length, 1)
  assert.ok(readFileSync(new URL('../../.gitignore', import.meta.url), 'utf8').includes('/business-apps/stores/backend/target/'))
})

test('shared fast and PostgreSQL register stores while preserving delivery isolation and historical gates', () => {
  const runner = readFileSync(new URL('./run.mjs', import.meta.url), 'utf8')
  const fast = runner.split("if (options.gate === 'fast') {")[1].split('} else if')[0]
  for (const name of ['fmt', 'check', 'clippy', 'units', 'doc', 'build']) assert.equal((fast.match(new RegExp("step\\('stores-" + name + "'", 'g')) || []).length, 1)
  assert.match(fast, /'stores-units',[^\n]*'--lib'/); assert.match(fast, /'stores-clippy',[^\n]*'-D','warnings'/)
  const postgres = runner.split("} else if (options.gate === 'postgres') {")[1].split('} else if')[0]
  for (const name of ['stores-postgres', 'stores-live-postgres']) assert.match(postgres, new RegExp("step\\('" + name + "'[^\\n]*format:'" + name + "'[^\\n]*env:pgEnv"))
  assert.ok(postgres.indexOf("step('work-orders-live-build'") < postgres.indexOf("step('stores-live-postgres'"))
  assert.ok(postgres.indexOf("step('stores-live-build'") < postgres.indexOf("step('stores-live-postgres'"))
  const delivery = runner.split("} else if (options.gate === 'delivery') {")[1].split('} else if')[0]
  assert.doesNotMatch(delivery, /step\('stores-/)
  assert.ok(ciUnitTests.includes('scripts/ci/stores.test.mjs'))
  const pkg = JSON.parse(readFileSync(new URL('../../package.json', import.meta.url), 'utf8'))
  for (const name of ['build', 'check', 'test', 'fmt', 'clippy', 'doc']) assert.ok(pkg.scripts[name + ':stores'].includes('--manifest-path business-apps/stores/backend/Cargo.toml'))
})
