import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import vm from 'node:vm'
import { ciUnitTests, postgresSuites, workOrdersPostgresSuites, workOrdersReadPostgresSuites, workOrdersWritePostgresSuites, workOrdersMemberPostgresSuites, workOrdersMemberLivePostgresSuites } from './manifest.mjs'
import { runWorkOrdersMemberPostgres } from './work-orders-member-postgres.mjs'
import { runWorkOrdersMemberLivePostgres } from './work-orders-member-live-postgres.mjs'
import { parseWorkOrdersMemberFixtureOutput, parseWorkOrdersMemberLiveOutput, workOrdersMemberLiveChecks } from './work-orders-member-live-results.mjs'
import { workOrdersLiveChecks } from './work-orders-live-results.mjs'
import { workOrdersWriteLiveChecks } from './work-orders-write-live-results.mjs'
import { evaluate } from './report.mjs'
import { createSnapshot } from './linux-runner.mjs'
import { createSourceManifest } from './provenance.mjs'

const env = { CI_TEST_POSTGRES_OWNER: 'ci-12345678', TEST_DATABASE_URL: 'postgresql://ci:synthetic@ci-12345678-postgres:5432/postgres' }
const output = passed => `test result: ok. ${passed} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`
const total = workOrdersMemberPostgresSuites.reduce((sum, suite) => sum + suite.expected, 0)
const memberFixtureOutput = () => [
  ...Array.from({ length: total }, () => 'WORK_ORDERS_MEMBER_FIXTURE_CLEANUP assertions_ok=true database_removed=true pool_closed=true'),
  'WORK_ORDERS_MEMBER_LOCK_PROOF member_http_gate=true application_pool_idle=1 same_ticket_nowait=true timeout_left_row_unchanged=true',
  'WORK_ORDERS_MEMBER_ORGANIZATION_PROOF pre_read_org_changed=true version_unchanged=true assignment_rejected=true identity_trigger_reenabled=true',
  'WORK_ORDERS_MEMBER_EXPIRY_PROOF assertion_ms=500 http_budget_consumed_ms=200 context_seconds=28 lock_wait_observed=true rollback=true',
  output(total),
].join('\n')
const memberReport = () => ({ status: 'PASS', kind: 'work-orders-member-isolated-postgres', passed: total, failed: 0, skipped: 0,
  fixture: parseWorkOrdersMemberFixtureOutput(memberFixtureOutput(), total),
  suites: workOrdersMemberPostgresSuites.map(suite => ({ ...suite, status: 'PASS', passed: suite.expected, failed: 0, skipped: 0 })) })
function memberLiveOutput(change = result => result) {
  return [
    'WORK_ORDERS_MEMBER_LIVE_RESULT ' + JSON.stringify(change({ schemaVersion: 1, transport: 'explicit-development-loopback-http',
      passed: workOrdersMemberLiveChecks.length, failed: 0, skipped: 0, checks: workOrdersMemberLiveChecks.map(name => ({ name, status: 'PASS' })) })),
    'MEMBER_LIVE_PLATFORM_CLEANUP assertions_ok=true database_removed=true pool_closed=true',
    'MEMBER_LIVE_BUSINESS_CLEANUP assertions_ok=true database_removed=true pool_closed=true',
    'MEMBER_LIVE_PROCESS_CLEANUP stopped=true',
    output(1),
  ].join('\n')
}

test('member PG requires owned environment and selects the exact independent locked target', async t => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-work-orders-member-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  for (const invalid of [{}, { ...env, TEST_DATABASE_URL: 'postgresql://ci:synthetic@production.invalid:5432/postgres' }]) {
    await assert.rejects(runWorkOrdersMemberPostgres({ outDir: path.join(directory, 'unowned'), env: invalid }), /owned environment|temporary runner/)
  }
  const calls = []
  const report = await runWorkOrdersMemberPostgres({ outDir: path.join(directory, 'pass'), env, runCargo: async (args, _root, childEnv) => {
    calls.push(args); assert.equal(childEnv.DATABASE_URL, env.TEST_DATABASE_URL)
    return { code: 0, timedOut: false, output: memberFixtureOutput() }
  } })
  assert.deepEqual(calls, [['test','--manifest-path','business-apps/work-orders/backend/Cargo.toml','--locked','--test','member_contracts','-j','2','--','--ignored','--test-threads=1','--nocapture']])
  assert.equal(total, 12)
  assert.equal(evaluate({ format: 'work-orders-member-postgres', exitCode: 0, document: report }).status, 'PASS')
  for (const [suites, count] of [[postgresSuites,76],[workOrdersPostgresSuites,27],[workOrdersReadPostgresSuites,13],[workOrdersWritePostgresSuites,14]]) {
    assert.equal(suites.reduce((sum, suite) => sum + suite.expected, 0), count)
  }
})

test('member reports fail closed for missing, duplicate, wrong, failed, zero or skipped selections', () => {
  const report = memberReport()
  for (const format of ['postgres', 'work-orders-postgres', 'work-orders-read-postgres', 'work-orders-write-postgres', 'work-orders-live-postgres', 'work-orders-write-live-postgres']) {
    assert.equal(evaluate({ format, exitCode: 0, document: report }).status, 'FAIL')
  }
  for (const document of [undefined, { ...report, kind: 'work-orders-write-isolated-postgres' }, { ...report, status: 'FAIL' }, { ...report, passed: 0 },
    { ...report, failed: 1 }, { ...report, skipped: 1 }, { ...report, suites: [] }, { ...report, suites: [...report.suites, report.suites[0]] },
    { ...report, suites: [{ ...report.suites[0], name: 'unregistered' }] }, { ...report, suites: [{ ...report.suites[0], expected: 999 }] }, { ...report, fixture: undefined }]) {
    assert.equal(evaluate({ format: 'work-orders-member-postgres', exitCode: 0, document }).status, 'FAIL')
  }
  assert.equal(evaluate({ format: 'work-orders-member-postgres', exitCode: 1, document: report }).status, 'FAIL')
  assert.equal(evaluate({ format: 'work-orders-member-postgres', exitCode: 0, timedOut: true, document: report }).status, 'FAIL')
})

test('member PG cleanup and deterministic race proofs cannot be replaced by only a passing Rust count', async t => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-work-orders-member-proof-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const text = memberFixtureOutput()
  assert.equal(parseWorkOrdersMemberFixtureOutput(text, total).cleanup.length, 12)
  assert.equal(parseWorkOrdersMemberFixtureOutput('test example ... ' + text, total).cleanup.length, 12)
  for (const changed of [output(total), text.replace('database_removed=true', 'database_removed=false'),
    text.replace('same_ticket_nowait=true', 'same_ticket_nowait=false'), text.replace('identity_trigger_reenabled=true', 'identity_trigger_reenabled=false'),
    text.replace('http_budget_consumed_ms=200', 'http_budget_consumed_ms=0'), text + '\nWORK_ORDERS_MEMBER_FIXTURE_CLEANUP assertions_ok=true database_removed=true pool_closed=true']) {
    assert.throws(() => parseWorkOrdersMemberFixtureOutput(changed, total))
  }
  await assert.rejects(runWorkOrdersMemberPostgres({ outDir: path.join(directory, 'missing'), env, runCargo: async () => ({ code: 0, timedOut: false, output: output(total) }) }), /proof is incomplete/)
  const failed = JSON.parse(readFileSync(path.join(directory, 'missing/postgres-result.json'), 'utf8'))
  assert.equal(failed.status, 'FAIL'); assert.equal(failed.passed, total); assert.equal(failed.failedSuites, 1)
})

test('member PG preserves observed failures and skipped counts while redacting ephemeral credentials', async t => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-work-orders-member-fail-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  for (const [name, text, code] of [
    ['failed', 'test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out', 1],
    ['skipped', 'test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out', 0],
    ['empty', output(0), 0], ['missing', 'compilation failed', 1],
  ]) {
    const outDir = path.join(directory, name)
    await assert.rejects(runWorkOrdersMemberPostgres({ outDir, env, runCargo: async () => ({ code, timedOut: false, output: `${env.TEST_DATABASE_URL}\n${text}` }) }), /PostgreSQL gate failed/)
    const report = JSON.parse(readFileSync(path.join(outDir, 'postgres-result.json'), 'utf8'))
    assert.equal(report.status, 'FAIL'); assert.equal(report.failedSuites, 1)
    assert.doesNotMatch(readFileSync(path.join(outDir, report.suites[0].log), 'utf8'), /synthetic|postgresql:\/\//)
    if (name === 'failed') assert.equal(report.failed, 1)
    if (name === 'skipped') assert.equal(report.skipped, 1)
  }
})

test('member live parser requires the exact ten unique checks and all owned cleanup markers', () => {
  const text = memberLiveOutput()
  assert.equal(workOrdersMemberLiveChecks.length, 10)
  assert.equal(parseWorkOrdersMemberLiveOutput(text).result.passed, 10)
  assert.equal(parseWorkOrdersMemberLiveOutput('test phase19_work_orders_tests::real_platform_member_directory_and_assignment ... ' + text.replaceAll('\n', '\r\n')).result.passed, 10)
  for (const changed of ['', text + '\n' + text, text.replace('WORK_ORDERS_MEMBER_LIVE_RESULT', 'WORK_ORDERS_WRITE_LIVE_RESULT'),
    text.replace('database_removed=true', 'database_removed=false'), text.replace('pool_closed=true', 'pool_closed=false'),
    text.replace('assertions_ok=true', 'assertions_ok=false'), text.replace('stopped=true', 'stopped=false'),
    text.replace('MEMBER_LIVE_PROCESS_CLEANUP stopped=true', ''), text + '\nMEMBER_LIVE_PROCESS_CLEANUP stopped=true',
    text.replace(workOrdersMemberLiveChecks[0], workOrdersMemberLiveChecks[1]), text.replace('"status":"PASS"', '"status":"SKIP"'),
    text.replace('explicit-development-loopback-http', 'production-https'),
    memberLiveOutput(result => ({ ...result, passed: 9 })), memberLiveOutput(result => ({ ...result, failed: 1 })),
    memberLiveOutput(result => ({ ...result, skipped: 1 })), memberLiveOutput(result => ({ ...result, checks: result.checks.slice(1) }))]) {
    assert.throws(() => parseWorkOrdersMemberLiveOutput(changed))
  }
})

test('member live runner requires the real registered filter, original build path and complete live evidence', async t => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-work-orders-member-live-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  await assert.rejects(runWorkOrdersMemberLivePostgres({ outDir: path.join(directory, 'unowned'), env: {} }), /owned environment/)
  const calls = [], text = memberLiveOutput()
  const report = await runWorkOrdersMemberLivePostgres({ outDir: path.join(directory, 'pass'), env, runCargo: async (args, _root, childEnv) => {
    calls.push(args)
    assert.match(childEnv.WORK_ORDERS_BINARY.replaceAll('\\', '/'), /\/target\/ci-rust\/debug\/work-orders-api(?:\.exe)?$/)
    return { code: 0, timedOut: false, output: text }
  } })
  assert.deepEqual(calls, [['test','--manifest-path','backend/Cargo.toml','--locked','--bin','saas-platform-backend','-j','2',
    'phase19_work_orders_tests::real_platform_member_directory_and_assignment','--','--ignored','--test-threads=1','--nocapture']])
  assert.equal(workOrdersMemberLivePostgresSuites[0].expected, 1)
  assert.equal(evaluate({ format: 'work-orders-member-live-postgres', exitCode: 0, document: report }).status, 'PASS')
  for (const format of ['work-orders-live-postgres', 'work-orders-write-live-postgres', 'work-orders-member-postgres']) assert.equal(evaluate({ format, exitCode: 0, document: report }).status, 'FAIL')
  assert.equal(evaluate({ format: 'work-orders-member-live-postgres', exitCode: 0, document: { ...report, live: undefined } }).status, 'FAIL')
  await assert.rejects(runWorkOrdersMemberLivePostgres({ outDir: path.join(directory, 'missing'), env, runCargo: async () => ({ code: 0, timedOut: false, output: output(1) }) }), /incomplete/)
  assert.equal(JSON.parse(readFileSync(path.join(directory, 'missing/postgres-result.json'), 'utf8')).status, 'FAIL')
  assert.equal(workOrdersLiveChecks.length, 13); assert.equal(workOrdersWriteLiveChecks.length, 12)
})

test('member gates are mandatory after the original build and retain original regression selections', () => {
  const runner = readFileSync(new URL('./run.mjs', import.meta.url), 'utf8')
  for (const id of ['work-orders-postgres','work-orders-read-postgres','work-orders-write-postgres','work-orders-member-postgres',
    'work-orders-live-postgres','work-orders-write-live-postgres','work-orders-member-live-postgres']) assert.ok(runner.includes(`step('${id}'`))
  assert.match(runner, /'work-orders-member-postgres'[^\n]*format:'work-orders-member-postgres'[^\n]*env:pgEnv/)
  assert.match(runner, /'work-orders-member-live-postgres'[^\n]*format:'work-orders-member-live-postgres'[^\n]*env:pgEnv/)
  assert.ok(runner.indexOf("step('work-orders-live-build'") < runner.indexOf("step('work-orders-member-live-postgres'"))
  assert.match(runner, /'rust-app-member-units'[^\n]*'routes::app_members::tests'[^\n]*format:'cargo'/)
  assert.ok(ciUnitTests.includes('scripts/ci/work-orders-member.test.mjs'))
  const workflow = readFileSync(new URL('../../.github/workflows/production-build.yml', import.meta.url), 'utf8')
  for (const id of ['work-orders-member-postgres', 'work-orders-member-live-postgres']) {
    assert.ok(workflow.includes(`/reports/postgres/${id}/postgres-result.json`)); assert.ok(workflow.includes(`/reports/postgres/${id}/*.log`))
  }
})

test('public source snapshot captures member client, policy, real harness and CI parser without generated or secret inputs', t => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-work-orders-member-source-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const source = path.join(directory, 'source'), snapshot = path.join(directory, 'snapshot')
  const included = ['business-apps/work-orders/backend/src/members.rs', 'business-apps/work-orders/backend/tests/member_contracts.rs',
    'backend/src/routes/app_members.rs', 'backend/src/phase19_work_orders_tests.rs', 'scripts/ci/work-orders-member-live-results.mjs',
    'business-apps/work-orders/tests/platform-e2e.mjs']
  for (const file of [...included, 'business-apps/work-orders/backend/.env', 'backend/private/credentials.json', 'business-apps/work-orders/backend/target/debug/work-orders']) {
    const absolute = path.join(source, file); mkdirSync(path.dirname(absolute), { recursive: true }); writeFileSync(absolute, 'public fixture source')
  }
  createSnapshot(snapshot, source)
  assert.deepEqual(createSourceManifest(snapshot, { sourceRevision: 'a'.repeat(40) }).files.map(file => file.path), included.sort((a,b) => a.localeCompare(b,'en')))
})

test('legacy opt-in real fixture explicitly declares only its bootstrapped assignment member pairs before service use', () => {
  const source = readFileSync(new URL('../../business-apps/work-orders/tests/platform-e2e.mjs', import.meta.url), 'utf8')
  const helper = source.match(/^export function workOrdersMemberPolicy\([^]*?^\}/m)?.[0]
  assert.ok(helper)
  const policy = vm.runInNewContext(`(${helper.replace(/^export /, '')})`, { assert })
  assert.throws(() => policy(''))
  const applicationId = '11111111-1111-4111-8111-111111111111', parsed = JSON.parse(policy(applicationId))
  assert.deepEqual(parsed.managementScopes, [])
  assert.equal(parsed.applications.length, 1); assert.equal(parsed.applications[0].applicationId, applicationId)
  assert.deepEqual(parsed.applications[0].memberLookups, [{ resource: 'ticket', action: 'create' }, { resource: 'ticket', action: 'update' }])
  assert.deepEqual(parsed.applications[0].capabilities.map(({ resource, action }) => ({ resource, action })), parsed.applications[0].memberLookups)
  assert.ok(source.indexOf('await stop(platform)') < source.indexOf('platformEnvironment.DATA_GRANT_POLICY = workOrdersMemberPolicy(ids.application)'))
  assert.ok(source.indexOf('platformEnvironment.DATA_GRANT_POLICY = workOrdersMemberPolicy(ids.application)') < source.indexOf("const business = launch('business'"))
})
