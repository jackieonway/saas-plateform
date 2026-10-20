import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { ciUnitTests, workOrdersReminderPostgresSuites, workOrdersReminderLivePostgresSuites } from './manifest.mjs'
import { runWorkOrdersReminderPostgres } from './work-orders-reminder-postgres.mjs'
import { runWorkOrdersReminderLivePostgres } from './work-orders-reminder-live-postgres.mjs'
import { parseWorkOrdersReminderFixtureOutput, parseWorkOrdersReminderLiveOutput, workOrdersReminderFixtureProofs, workOrdersReminderLiveChecks } from './work-orders-reminder-live-results.mjs'
import { evaluate } from './report.mjs'
import { createSnapshot } from './linux-runner.mjs'
import { createSourceManifest } from './provenance.mjs'

const env = { CI_TEST_POSTGRES_OWNER: 'ci-12345678', TEST_DATABASE_URL: 'postgresql://ci:synthetic@ci-12345678-postgres:5432/postgres' }
const output = passed => `test result: ok. ${passed} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`
const total = workOrdersReminderPostgresSuites.reduce((sum, suite) => sum + suite.expected, 0)
const cleanup = 'WORK_ORDERS_REMINDER_FIXTURE_CLEANUP assertions_ok=true database_removed=true pool_closed=true'
const fixtureOutput = () => [...Array.from({ length: total }, () => cleanup), ...Object.values(workOrdersReminderFixtureProofs), output(total)].join('\n')
function liveOutput(change = value => value) {
  return [
    'WORK_ORDERS_REMINDER_LIVE_RESULT ' + JSON.stringify(change({ schemaVersion: 1, transport: 'explicit-development-loopback-http',
      passed: workOrdersReminderLiveChecks.length, failed: 0, skipped: 0, checks: workOrdersReminderLiveChecks.map(name => ({ name, status: 'PASS' })) })),
    'REMINDER_LIVE_PLATFORM_CLEANUP assertions_ok=true database_removed=true pool_closed=true',
    'REMINDER_LIVE_BUSINESS_CLEANUP assertions_ok=true database_removed=true pool_closed=true',
    'REMINDER_LIVE_PROCESS_RESTART stopped=true',
    'REMINDER_LIVE_PROCESS_CLEANUP stopped=true', output(1),
  ].join('\n')
}

test('reminder contracts require owned PG and select the exact locked independent target', async t => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-reminder-owned-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  for (const invalid of [{}, { ...env, TEST_DATABASE_URL: 'postgresql://ci:synthetic@production.invalid:5432/postgres' }]) {
    await assert.rejects(runWorkOrdersReminderPostgres({ outDir: path.join(directory, 'unowned'), env: invalid }), /owned environment|temporary runner/)
  }
  const calls = []
  const report = await runWorkOrdersReminderPostgres({ outDir: path.join(directory, 'pass'), env, runCargo: async (args, _root, childEnv) => {
    calls.push(args); assert.equal(childEnv.DATABASE_URL, env.TEST_DATABASE_URL)
    return { code: 0, timedOut: false, output: fixtureOutput() }
  } })
  assert.deepEqual(calls, [['test','--manifest-path','business-apps/work-orders/backend/Cargo.toml','--locked','--test','reminder_contracts','-j','2','--','--ignored','--test-threads=1','--nocapture']])
  assert.equal(total, 12)
  assert.equal(evaluate({ format: 'work-orders-reminder-postgres', exitCode: 0, document: report }).status, 'PASS')
})

test('reminder contract success requires owned cleanup and each exact durable-intent proof once', async t => {
  const text = fixtureOutput(), directory = mkdtempSync(path.join(tmpdir(), 'ci-reminder-proofs-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  assert.equal(parseWorkOrdersReminderFixtureOutput(text, total).cleanup.length, total)
  assert.equal(parseWorkOrdersReminderFixtureOutput('test example ... ' + text.replaceAll('\n', '\r\n'), total).cleanup.length, total)
  const invalid = [output(total), text.replace('database_removed=true', 'database_removed=false'), text + '\n' + cleanup,
    text + '\nWORK_ORDERS_REMINDER_FIXTURE_CLEANUP malformed=true']
  for (const marker of Object.values(workOrdersReminderFixtureProofs)) invalid.push(text.replace(marker, ''), text.replace(marker, marker.replace('=true', '=false')),
    text + '\n' + marker, text + '\n' + marker.replace('=true', '=false'))
  for (const changed of invalid) assert.throws(() => parseWorkOrdersReminderFixtureOutput(changed, total))
  await assert.rejects(runWorkOrdersReminderPostgres({ outDir: path.join(directory, 'missing'), env, runCargo: async () => ({ code: 0, timedOut: false, output: output(total) }) }), /proof is incomplete/)
  const failed = JSON.parse(readFileSync(path.join(directory, 'missing/postgres-result.json'), 'utf8'))
  assert.equal(failed.status, 'FAIL'); assert.equal(failed.passed, total); assert.equal(failed.failedSuites, 1)
})

test('reminder reports reject wrong kinds, selections, counts and absent semantic evidence', () => {
  const report = { status: 'PASS', kind: 'work-orders-reminder-isolated-postgres', passed: total, failed: 0, skipped: 0,
    fixture: parseWorkOrdersReminderFixtureOutput(fixtureOutput(), total),
    suites: workOrdersReminderPostgresSuites.map(suite => ({ ...suite, status: 'PASS', passed: suite.expected, failed: 0, skipped: 0 })) }
  for (const format of ['postgres','work-orders-postgres','work-orders-read-postgres','work-orders-write-postgres','work-orders-member-postgres','work-orders-reminder-live-postgres']) assert.equal(evaluate({ format, exitCode: 0, document: report }).status, 'FAIL')
  for (const changed of [undefined, { ...report, kind: 'work-orders-member-isolated-postgres' }, { ...report, status: 'FAIL' }, { ...report, passed: 0 },
    { ...report, failed: 1 }, { ...report, skipped: 1 }, { ...report, suites: [] }, { ...report, suites: [...report.suites, report.suites[0]] },
    { ...report, suites: [{ ...report.suites[0], expected: 999 }] }, { ...report, fixture: undefined }]) {
    assert.equal(evaluate({ format: 'work-orders-reminder-postgres', exitCode: 0, document: changed }).status, 'FAIL')
  }
  for (const options of [{ exitCode: 1 }, { exitCode: 0, timedOut: true }]) assert.equal(evaluate({ format: 'work-orders-reminder-postgres', document: report, ...options }).status, 'FAIL')
})

test('reminder runner preserves actual failing and ignored totals and redacts ephemeral credentials', async t => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-reminder-fail-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  for (const [name, text, code] of [['failed', 'test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out', 1],
    ['skipped', 'test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out', 0], ['missing', 'compilation failed', 1]]) {
    const outDir = path.join(directory, name)
    await assert.rejects(runWorkOrdersReminderPostgres({ outDir, env, runCargo: async () => ({ code, timedOut: false, output: `${env.TEST_DATABASE_URL}\n${text}` }) }), /PostgreSQL gate failed/)
    const report = JSON.parse(readFileSync(path.join(outDir, 'postgres-result.json'), 'utf8'))
    assert.equal(report.status, 'FAIL'); assert.equal(report.failedSuites, 1)
    assert.doesNotMatch(readFileSync(path.join(outDir, report.suites[0].log), 'utf8'), /synthetic|postgresql:\/\//)
    if (name === 'failed') assert.equal(report.failed, 1)
    if (name === 'skipped') assert.equal(report.skipped, 1)
  }
})

test('reminder live parser requires twelve unique checks and dual DB cleanup plus stopped restart and final process', () => {
  const text = liveOutput()
  assert.equal(workOrdersReminderLiveChecks.length, 12)
  assert.equal(parseWorkOrdersReminderLiveOutput(text).result.passed, 12)
  assert.equal(parseWorkOrdersReminderLiveOutput('test phase20_work_orders_tests::real_platform_reminder_publication_and_recovery ... ' + text).result.passed, 12)
  for (const changed of ['', text + '\n' + text, text.replace('database_removed=true', 'database_removed=false'), text.replace('stopped=true', 'stopped=false'),
    text.replace('REMINDER_LIVE_PROCESS_RESTART stopped=true', ''), text + '\nREMINDER_LIVE_PROCESS_RESTART stopped=false',
    text.replace(workOrdersReminderLiveChecks[0], workOrdersReminderLiveChecks[1]), text.replace('"status":"PASS"', '"status":"SKIP"'),
    text.replace('explicit-development-loopback-http', 'production-https'), liveOutput(value => ({ ...value, passed: 11 })),
    liveOutput(value => ({ ...value, failed: 1 })), liveOutput(value => ({ ...value, skipped: 1 })), liveOutput(value => ({ ...value, checks: value.checks.slice(1) }))]) assert.throws(() => parseWorkOrdersReminderLiveOutput(changed))
})

test('reminder live runner selects the actual platform module and original business ELF and fails without live evidence', async t => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-reminder-live-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  await assert.rejects(runWorkOrdersReminderLivePostgres({ outDir: path.join(directory, 'unowned'), env: {} }), /owned environment/)
  const calls = []
  const report = await runWorkOrdersReminderLivePostgres({ outDir: path.join(directory, 'pass'), env, runCargo: async (args, _root, childEnv) => {
    calls.push(args); assert.match(childEnv.WORK_ORDERS_BINARY.replaceAll('\\', '/'), /\/target\/ci-rust\/debug\/work-orders-api(?:\.exe)?$/)
    return { code: 0, timedOut: false, output: liveOutput() }
  } })
  assert.deepEqual(calls, [['test','--manifest-path','backend/Cargo.toml','--locked','--bin','saas-platform-backend','-j','2', 'phase20_work_orders_tests::real_platform_reminder_publication_and_recovery','--','--ignored','--test-threads=1','--nocapture']])
  assert.equal(workOrdersReminderLivePostgresSuites[0].expected, 1)
  assert.equal(evaluate({ format: 'work-orders-reminder-live-postgres', exitCode: 0, document: report }).status, 'PASS')
  assert.equal(evaluate({ format: 'work-orders-reminder-live-postgres', exitCode: 0, document: { ...report, live: undefined } }).status, 'FAIL')
  await assert.rejects(runWorkOrdersReminderLivePostgres({ outDir: path.join(directory, 'missing'), env, runCargo: async () => ({ code: 0, timedOut: false, output: output(1) }) }), /incomplete/)
})

test('mandatory reminder gates retain prior selections, archive reports and capture public source without secrets', t => {
  const runner = readFileSync(new URL('./run.mjs', import.meta.url), 'utf8')
  for (const id of ['work-orders-postgres','work-orders-read-postgres','work-orders-write-postgres','work-orders-member-postgres','work-orders-reminder-postgres',
    'work-orders-live-postgres','work-orders-write-live-postgres','work-orders-member-live-postgres','work-orders-reminder-live-postgres','notification-live']) assert.ok(runner.includes(`step('${id}'`))
  assert.ok(runner.indexOf("step('work-orders-live-build'") < runner.indexOf("step('work-orders-reminder-live-postgres'"))
  assert.ok(ciUnitTests.includes('scripts/ci/work-orders-reminder.test.mjs'))
  const workflow = readFileSync(new URL('../../.github/workflows/production-build.yml', import.meta.url), 'utf8')
  for (const id of ['work-orders-reminder-postgres', 'work-orders-reminder-live-postgres']) for (const name of ['postgres-result.json','*.log']) assert.ok(workflow.includes(`/reports/postgres/${id}/${name}`))
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-reminder-source-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const source = path.join(directory, 'source'), snapshot = path.join(directory, 'snapshot')
  const included = ['business-apps/work-orders/backend/src/reminders.rs','business-apps/work-orders/backend/tests/reminder_contracts.rs',
    'business-apps/work-orders/backend/migrations/0003_reminder_intents.sql','backend/src/phase20_work_orders_tests.rs','scripts/ci/work-orders-reminder-live-results.mjs']
  for (const file of [...included,'business-apps/work-orders/backend/.env','backend/private/credentials.json','business-apps/work-orders/backend/target/debug/work-orders']) {
    const absolute = path.join(source, file); mkdirSync(path.dirname(absolute), { recursive: true }); writeFileSync(absolute, 'public fixture source')
  }
  createSnapshot(snapshot, source)
  assert.deepEqual(createSourceManifest(snapshot, { sourceRevision: 'a'.repeat(40) }).files.map(file => file.path), included.sort((a,b) => a.localeCompare(b,'en')))
})
