import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { workOrdersPostgresSuites, workOrdersReadPostgresSuites, postgresSuites, ciUnitTests } from './manifest.mjs'
import { runWorkOrdersPostgres } from './work-orders-postgres.mjs'
import { runWorkOrdersReadPostgres } from './work-orders-read-postgres.mjs'
import { runWorkOrdersLivePostgres } from './work-orders-live-postgres.mjs'
import { workOrdersLiveChecks, parseWorkOrdersLiveOutput } from './work-orders-live-results.mjs'
import { evaluate } from './report.mjs'
import { createSourceManifest } from './provenance.mjs'

const env = { CI_TEST_POSTGRES_OWNER: 'ci-12345678', TEST_DATABASE_URL: 'postgresql://ci:synthetic@ci-12345678-postgres:5432/postgres' }
const good = () => ({ status: 'PASS', kind: 'work-orders-isolated-postgres', failed: 0, skipped: 0,
  passed: workOrdersPostgresSuites.reduce((sum, suite) => sum + suite.expected, 0),
  suites: workOrdersPostgresSuites.map(suite => ({ ...suite, status: 'PASS', passed: suite.expected, failed: 0, skipped: 0 })) })

test('independent work-orders PostgreSQL gate uses only its locked integration target and validates ownership', async t => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-work-orders-pg-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  await assert.rejects(runWorkOrdersPostgres({ outDir: path.join(directory, 'unowned'), env: {} }), /owned environment/)
  const calls = []
  const report = await runWorkOrdersPostgres({ outDir: path.join(directory, 'pass'), env, runCargo: async (args, root, childEnv) => {
    calls.push(args)
    assert.equal(childEnv.DATABASE_URL, env.TEST_DATABASE_URL)
    assert.equal(childEnv.CI_TEST_POSTGRES_OWNER, env.CI_TEST_POSTGRES_OWNER)
    return { code: 0, timedOut: false, output: `test result: ok. ${workOrdersPostgresSuites[0].expected} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` }
  } })
  assert.equal(calls.length, 1)
  assert.deepEqual(calls[0], ['test', '--manifest-path', 'business-apps/work-orders/backend/Cargo.toml', '--locked', '--test', 'migrations', '-j', '2', '--', '--ignored', '--test-threads=1', '--nocapture'])
  assert.equal(report.kind, 'work-orders-isolated-postgres')
  assert.equal(report.status, 'PASS')
  assert.equal(postgresSuites.length, 12)
  assert.equal(postgresSuites.reduce((sum, suite) => sum + suite.expected, 0), 76)
})

test('work-orders PostgreSQL summary rejects absent, wrong, failed, zero, skipped or duplicate selections', () => {
  const report = good(), verdict = document => evaluate({ format: 'work-orders-postgres', exitCode: 0, document }).status
  assert.equal(verdict(report), 'PASS')
  for (const changed of [undefined, { ...report, kind: 'existing-platform-isolated-postgres' }, { ...report, status: 'FAIL' },
    { ...report, passed: 0 }, { ...report, failed: 1 }, { ...report, skipped: 1 }, { ...report, suites: [] },
    { ...report, suites: [...report.suites, report.suites[0]] },
    { ...report, suites: [{ ...report.suites[0], passed: 0 }] },
    { ...report, suites: [{ ...report.suites[0], name: 'platform-suite' }] }]) assert.equal(verdict(changed), 'FAIL')
  assert.equal(evaluate({ format: 'postgres', exitCode: 0, document: report }).status, 'FAIL')
  assert.equal(evaluate({ format: 'work-orders-postgres', exitCode: 1, document: report }).status, 'FAIL')
})

test('work-orders PostgreSQL failed and skipped command evidence fails without falsifying counts or exposing synthetic credentials', async t => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-work-orders-failed-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  for (const [name, summary, code] of [
    ['failed', 'test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out', 1],
    ['skipped', 'test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out', 0],
    ['zero', 'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out', 0],
    ['missing', 'compilation failed', 1],
  ]) {
    const outDir = path.join(directory, name)
    await assert.rejects(runWorkOrdersPostgres({ outDir, env, runCargo: async () => ({ code, timedOut: false, output: `${env.TEST_DATABASE_URL}\n${summary}` }) }), /PostgreSQL gate failed/)
    const report = JSON.parse(readFileSync(path.join(outDir, 'postgres-result.json'), 'utf8'))
    assert.equal(report.status, 'FAIL'); assert.equal(report.failedSuites, 1)
    const log = readFileSync(path.join(outDir, report.suites[0].log), 'utf8')
    assert.doesNotMatch(log, /synthetic|postgresql:\/\//)
    if (name === 'failed') assert.equal(report.failed, 1)
    if (name === 'skipped') assert.equal(report.skipped, 1)
  }
})

test('read-contract PostgreSQL gate is owned, locked, distinct from baseline migration and original platform counts', async t => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-work-orders-read-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  await assert.rejects(runWorkOrdersReadPostgres({ outDir: path.join(directory, 'unowned'), env: {} }), /owned environment/)
  const calls = [], total = workOrdersReadPostgresSuites.reduce((sum, suite) => sum + suite.expected, 0)
  const report = await runWorkOrdersReadPostgres({ outDir: path.join(directory, 'pass'), env, runCargo: async (args) => {
    calls.push(args)
    return { code: 0, timedOut: false, output: `test result: ok. ${total} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` }
  } })
  assert.deepEqual(calls, [['test', '--manifest-path', 'business-apps/work-orders/backend/Cargo.toml', '--locked', '--test', 'read_contracts', '-j', '2', '--', '--ignored', '--test-threads=1', '--nocapture']])
  assert.equal(evaluate({ format: 'work-orders-read-postgres', exitCode: 0, document: report }).status, 'PASS')
  assert.equal(evaluate({ format: 'work-orders-postgres', exitCode: 0, document: report }).status, 'FAIL')
  assert.equal(evaluate({ format: 'postgres', exitCode: 0, document: report }).status, 'FAIL')
  assert.equal(report.kind, 'work-orders-read-isolated-postgres')
  for (const changed of [{ ...report, kind: 'work-orders-isolated-postgres' }, { ...report, passed: 0 }, { ...report, skipped: 1 },
    { ...report, suites: [] }, { ...report, suites: [...report.suites, report.suites[0]] },
    { ...report, suites: [{ ...report.suites[0], expected: 999 }] }]) {
    assert.equal(evaluate({ format: 'work-orders-read-postgres', exitCode: 0, document: changed }).status, 'FAIL')
  }
  assert.equal(postgresSuites.reduce((sum, suite) => sum + suite.expected, 0), 76)
  assert.equal(workOrdersPostgresSuites.reduce((sum, suite) => sum + suite.expected, 0), 27)
})

test('work-orders fast and database commands are mandatory independent gates with source and lock provenance', t => {
  const runner = readFileSync(new URL('./run.mjs', import.meta.url), 'utf8')
  for (const id of ['work-orders-fmt', 'work-orders-check', 'work-orders-clippy', 'work-orders-units', 'work-orders-doc']) assert.ok(runner.includes(`step('${id}'`))
  assert.match(runner, /'work-orders-clippy'[^\n]*'--locked'[^\n]*'-D','warnings'/)
  assert.match(runner, /'work-orders-units'[^\n]*'--lib'/)
  assert.match(runner, /'work-orders-postgres'[^\n]*format:'work-orders-postgres'[^\n]*env:pgEnv/)
  assert.match(runner, /'work-orders-read-postgres'[^\n]*format:'work-orders-read-postgres'[^\n]*env:pgEnv/)
  assert.match(runner, /'work-orders-live-postgres'[^\n]*format:'work-orders-live-postgres'[^\n]*env:pgEnv/)
  assert.ok(runner.indexOf("step('work-orders-live-build'") < runner.indexOf("step('work-orders-live-postgres'"))
  const pgBranch = runner.split("} else if (options.gate === 'postgres') {")[1].split('} else if')[0]
  assert.ok(pgBranch.indexOf("install('business-apps/work-orders/backend')") < pgBranch.indexOf("step('work-orders-postgres'"))
  assert.ok(ciUnitTests.includes('scripts/ci/work-orders.test.mjs'))
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-work-orders-provenance-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const base = 'business-apps/work-orders/backend/', included = ['Cargo.toml', 'Cargo.lock', 'src/lib.rs', 'src/main.rs', 'tests/migrations.rs', 'migrations/0001_work_orders.sql']
  for (const file of [...included, '.env', 'private/credentials.json', 'target/debug/work-orders']) {
    const absolute = path.join(directory, base, file); mkdirSync(path.dirname(absolute), { recursive: true }); writeFileSync(absolute, 'test source')
  }
  const manifest = createSourceManifest(directory, { sourceRevision: 'a'.repeat(40) })
  assert.deepEqual(manifest.files.map(file => file.path), included.map(file => base + file).sort((a,b) => a.localeCompare(b, 'en')))
})

function liveOutput() {
  return [
    'WORK_ORDERS_LIVE_RESULT ' + JSON.stringify({ schemaVersion: 1, transport: 'explicit-development-loopback-http',
      checks: workOrdersLiveChecks.map(id => ({ id, status: 'PASS' })), passed: workOrdersLiveChecks.length, failed: 0, skipped: 0 }),
    'WORK_ORDERS_LIVE_PLATFORM_CLEANUP assertions_ok=true database_removed=true pool_closed=true',
    'WORK_ORDERS_LIVE_BUSINESS_CLEANUP assertions_ok=true database_removed=true pool_closed=true',
    'WORK_ORDERS_LIVE_PROCESS_CLEANUP stopped=true',
    'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out',
  ].join('\n')
}

test('actual platform joint gate requires every named check and both database/process cleanup proofs', async t => {
  const output = liveOutput(), directory = mkdtempSync(path.join(tmpdir(), 'ci-work-orders-live-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  assert.equal(parseWorkOrdersLiveOutput(output).result.passed, workOrdersLiveChecks.length)
  for (const changed of ['', output + '\n' + output, output.replace('database_removed=true', 'database_removed=false'),
    output.replace('stopped=true', 'stopped=false'), output.replace('WORK_ORDERS_LIVE_PROCESS_CLEANUP stopped=true', ''),
    output.replace(workOrdersLiveChecks[0], 'unknown-check'), output.replace('"status":"PASS"', '"status":"SKIP"'),
    output.replace('explicit-development-loopback-http', 'production-https')]) assert.throws(() => parseWorkOrdersLiveOutput(changed))
  await assert.rejects(runWorkOrdersLivePostgres({ outDir: path.join(directory, 'unowned'), env: {} }), /owned environment/)
  const calls = []
  const report = await runWorkOrdersLivePostgres({ outDir: path.join(directory, 'pass'), env, runCargo: async (args, root, childEnv) => {
    calls.push(args)
    assert.match(childEnv.WORK_ORDERS_BINARY.replaceAll('\\', '/'), /\/target\/ci-rust\/debug\/work-orders-api(?:\.exe)?$/)
    return { code: 0, timedOut: false, output }
  } })
  assert.equal(calls.length, 1)
  assert.deepEqual(calls[0], ['test', '--manifest-path', 'backend/Cargo.toml', '--locked', '--bin', 'saas-platform-backend', '-j', '2', 'phase17_work_orders_tests::real_platform_authorization_and_business_scope', '--', '--ignored', '--test-threads=1', '--nocapture'])
  assert.equal(evaluate({ format: 'work-orders-live-postgres', exitCode: 0, document: report }).status, 'PASS')
  assert.equal(evaluate({ format: 'postgres', exitCode: 0, document: report }).status, 'FAIL')
  assert.equal(evaluate({ format: 'work-orders-live-postgres', exitCode: 0, document: { ...report, live: undefined } }).status, 'FAIL')
  await assert.rejects(runWorkOrdersLivePostgres({ outDir: path.join(directory, 'missing'), env, runCargo: async () => ({ code: 0, timedOut: false, output: output.split('\n').at(-1) }) }), /incomplete/)
  assert.equal(JSON.parse(readFileSync(path.join(directory, 'missing/postgres-result.json'), 'utf8')).status, 'FAIL')
})
