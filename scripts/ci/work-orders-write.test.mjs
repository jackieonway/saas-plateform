import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { ciUnitTests, postgresSuites, workOrdersReadPostgresSuites, workOrdersWritePostgresSuites } from './manifest.mjs'
import { runWorkOrdersWritePostgres } from './work-orders-write-postgres.mjs'
import { runWorkOrdersWriteLivePostgres } from './work-orders-write-live-postgres.mjs'
import { parseWorkOrdersWriteLiveOutput, workOrdersWriteLiveChecks } from './work-orders-write-live-results.mjs'
import { evaluate } from './report.mjs'
import { createSnapshot } from './linux-runner.mjs'
import { createSourceManifest } from './provenance.mjs'

const env = { CI_TEST_POSTGRES_OWNER: 'ci-12345678', TEST_DATABASE_URL: 'postgresql://ci:synthetic@ci-12345678-postgres:5432/postgres' }
const output = passed => `test result: ok. ${passed} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`

test('write contract runner is owned and selects the exact locked independent target', async t => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-work-orders-write-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  await assert.rejects(runWorkOrdersWritePostgres({ outDir: path.join(directory, 'unowned'), env: {} }), /owned environment/)
  const calls = [], expected = workOrdersWritePostgresSuites.reduce((sum, suite) => sum + suite.expected, 0)
  const report = await runWorkOrdersWritePostgres({ outDir: path.join(directory, 'pass'), env, runCargo: async (args, root, childEnv) => {
    calls.push(args); assert.equal(childEnv.TEST_DATABASE_URL, env.TEST_DATABASE_URL)
    return { code: 0, timedOut: false, output: output(expected) }
  } })
  assert.deepEqual(calls, [['test','--manifest-path','business-apps/work-orders/backend/Cargo.toml','--locked','--test','write_contracts','-j','2','--','--ignored','--test-threads=1','--nocapture']])
  assert.equal(report.kind, 'work-orders-write-isolated-postgres')
  assert.equal(evaluate({ format: 'work-orders-write-postgres', exitCode: 0, document: report }).status, 'PASS')
  for (const format of ['postgres','work-orders-postgres','work-orders-read-postgres','work-orders-live-postgres']) assert.equal(evaluate({ format, exitCode: 0, document: report }).status, 'FAIL')
  assert.equal(postgresSuites.reduce((sum, suite) => sum + suite.expected, 0), 76)
  assert.equal(workOrdersReadPostgresSuites.reduce((sum, suite) => sum + suite.expected, 0), 13)
})

test('write reports reject absent, duplicate, wrong, failed, skipped or empty results', async t => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-work-orders-write-fail-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const total = workOrdersWritePostgresSuites.reduce((sum, suite) => sum + suite.expected, 0)
  const report = { status: 'PASS', kind: 'work-orders-write-isolated-postgres', passed: total, failed: 0, skipped: 0,
    suites: workOrdersWritePostgresSuites.map(suite => ({ ...suite, status: 'PASS', passed: suite.expected, failed: 0, skipped: 0 })) }
  for (const changed of [undefined, { ...report, kind: 'work-orders-read-isolated-postgres' }, { ...report, status: 'FAIL' }, { ...report, passed: 0 },
    { ...report, failed: 1 }, { ...report, skipped: 1 }, { ...report, suites: [] }, { ...report, suites: [...report.suites, report.suites[0]] }]) {
    assert.equal(evaluate({ format: 'work-orders-write-postgres', exitCode: 0, document: changed }).status, 'FAIL')
  }
  await assert.rejects(runWorkOrdersWritePostgres({ outDir: path.join(directory, 'skipped'), env, runCargo: async () => ({ code: 0, timedOut: false, output: output(total).replace('0 ignored','1 ignored') }) }), /PostgreSQL gate failed/)
  const failed = JSON.parse(readFileSync(path.join(directory, 'skipped/postgres-result.json'), 'utf8'))
  assert.equal(failed.status, 'FAIL'); assert.equal(failed.skipped, 1)
})

test('write gate registration and clean snapshot include new Rust tests and both business migrations', t => {
  const runner = readFileSync(new URL('./run.mjs', import.meta.url), 'utf8')
  assert.match(runner, /'work-orders-write-postgres'[^\n]*format:'work-orders-write-postgres'[^\n]*env:pgEnv/)
  assert.match(runner, /'work-orders-write-live-postgres'[^\n]*format:'work-orders-write-live-postgres'[^\n]*env:pgEnv/)
  assert.ok(runner.indexOf("step('work-orders-live-build'") < runner.indexOf("step('work-orders-write-live-postgres'"))
  assert.ok(ciUnitTests.includes('scripts/ci/work-orders-write.test.mjs'))
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-work-orders-write-source-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const source = path.join(directory, 'source'), snapshot = path.join(directory, 'snapshot'), base = 'business-apps/work-orders/backend/'
  const files = ['Cargo.toml','Cargo.lock','src/lib.rs','src/writes.rs','tests/migrations.rs','tests/read_contracts.rs','tests/write_contracts.rs','migrations/0001_work_orders.sql','migrations/0002_create_idempotency.sql']
  for (const file of [...files, '.env', 'private/credentials.json', 'target/debug/work-orders']) {
    const absolute = path.join(source, base, file); mkdirSync(path.dirname(absolute), { recursive: true }); writeFileSync(absolute, 'public fixture source')
  }
  createSnapshot(snapshot, source)
  const selected = createSourceManifest(snapshot, { sourceRevision: 'a'.repeat(40) })
  assert.deepEqual(selected.files.map(file => file.path), files.map(file => base + file).sort((a,b) => a.localeCompare(b,'en')))
})

function writeLiveOutput() {
  return [
    'WORK_ORDERS_WRITE_LIVE_RESULT ' + JSON.stringify({ schemaVersion: 1, transport: 'explicit-development-loopback-http',
      passed: workOrdersWriteLiveChecks.length, failed: 0, skipped: 0, checks: workOrdersWriteLiveChecks.map(name => ({ name, status: 'PASS' })) }),
    'WRITE_LIVE_PLATFORM_CLEANUP assertions_ok=true database_removed=true pool_closed=true',
    'WRITE_LIVE_BUSINESS_CLEANUP assertions_ok=true database_removed=true pool_closed=true',
    'WRITE_LIVE_PROCESS_CLEANUP stopped=true',
    output(1),
  ].join('\n')
}

test('actual platform write suite requires all registered checks and both database/process cleanup', async t => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-work-orders-write-live-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const text = writeLiveOutput()
  assert.equal(parseWorkOrdersWriteLiveOutput(text).result.passed, 12)
  assert.equal(parseWorkOrdersWriteLiveOutput('test phase18_work_orders_tests::real_platform_writes_and_idempotency ... ' + text).result.passed, 12)
  for (const changed of ['', text + '\n' + text, text.replace('database_removed=true','database_removed=false'), text.replace('stopped=true','stopped=false'),
    text.replace(workOrdersWriteLiveChecks[0], workOrdersWriteLiveChecks[1]), text.replace('"status":"PASS"','"status":"SKIP"'),
    text.replace('explicit-development-loopback-http','production-https')]) assert.throws(() => parseWorkOrdersWriteLiveOutput(changed))
  await assert.rejects(runWorkOrdersWriteLivePostgres({ outDir: path.join(directory,'unowned'), env: {} }), /owned environment/)
  const calls = []
  const report = await runWorkOrdersWriteLivePostgres({ outDir: path.join(directory,'pass'), env, runCargo: async (args, root, childEnv) => {
    calls.push(args); assert.match(childEnv.WORK_ORDERS_BINARY.replaceAll('\\','/'), /\/target\/ci-rust\/debug\/work-orders-api(?:\.exe)?$/)
    return { code: 0, timedOut: false, output: text }
  } })
  assert.equal(calls.length, 1)
  assert.ok(calls[0].includes('phase18_work_orders_tests::real_platform_writes_and_idempotency'))
  assert.equal(evaluate({ format: 'work-orders-write-live-postgres', exitCode: 0, document: report }).status, 'PASS')
  assert.equal(evaluate({ format: 'work-orders-live-postgres', exitCode: 0, document: report }).status, 'FAIL')
  assert.equal(evaluate({ format: 'work-orders-write-live-postgres', exitCode: 0, document: { ...report, live: undefined } }).status, 'FAIL')
  await assert.rejects(runWorkOrdersWriteLivePostgres({ outDir: path.join(directory,'missing'), env, runCargo: async () => ({ code: 0, timedOut: false, output: output(1) }) }), /incomplete/)
  assert.equal(JSON.parse(readFileSync(path.join(directory,'missing/postgres-result.json'),'utf8')).status,'FAIL')
})
