import assert from 'node:assert/strict'
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { evaluate } from './report.mjs'
import { parseWorkOrdersBrowserArgs, runWorkOrdersBrowser, workOrdersBrowserEnvironment } from './work-orders-browser.mjs'
import { parseWorkOrdersBrowserOutput, validateWorkOrdersBrowserResult, workOrdersBrowserChecks, workOrdersBrowserCleanup, workOrdersBrowserSteps } from './work-orders-browser-results.mjs'

const revision = 'a'.repeat(40), digest = 'b'.repeat(64)
const owned = { PATH: '/bin', HOME: '/outside-user', TEST_DATABASE_URL: 'postgresql://ci:synthetic-only@ci-12345678-postgres:5432/postgres', CI_TEST_POSTGRES_OWNER: 'ci-12345678' }
const identity = { sha256: digest, bytes: 32, elfMachine: 62 }
function transportFixture() {
  const ticket = 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee'
  const classes = ['browser-response', 'browser-response', 'aborted-before-request-event', 'aborted-without-wire-confirmation', 'fixture-response-drop', 'fixture-response-drop']
  const records = classes.map((classification, index) => {
    const requestId = String(index + 1).repeat(8) + '-1111-4111-8111-' + String(index + 1).repeat(12)
    const method = index >= 4 ? 'POST' : 'GET', resource = index === 5 ? '/work-orders-api/tickets/' + ticket + '/remind' : '/work-orders-api/tickets'
    const cancelled = classification.startsWith('aborted-'), fault = classification === 'fixture-response-drop'
    const event = classification === 'aborted-before-request-event' ? null : { requestId, method, path: resource,
      origin: cancelled ? null : 'null', cookiePresent: false, bearerPresent: true, requestIdPresent: true,
      frameType: 'iframe', framePath: '/sub-apps/work-orders/versions/phase21/index.html',
      outcome: cancelled || fault ? 'failed' : 'finished', responseStatus: cancelled || fault ? null : 200,
      failureCode: cancelled ? 'net::ERR_ABORTED' : fault ? 'net::ERR_CONNECTION_RESET' : null }
    return { requestId, method, path: resource, credentials: 'omit', signalAborted: cancelled,
      nativeOutcome: cancelled || fault ? 'rejected' : 'fulfilled', nativeErrorName: cancelled ? 'AbortError' : fault ? 'TypeError' : null,
      nativeHeaders: { cookiePresent: false, bearerPresent: true, requestIdPresent: true }, browserEvent: event, classification }
  })
  return { origin: 'null', cookiePresent: false, credentials: 'omit', appBearerPresent: true, requestIdPresent: true, rustPreflightStatus: 204, credentialsAllowed: false,
    records, nativeAttemptCount: 6, browserRequestEventCount: 5, confirmedWireCount: 2, responseBodyCancelledCount: 0, abortBeforeRequestEventCount: 1, abortedWithoutWireConfirmationCount: 1, faultInjectedCount: 2,
    actualRequestCount: 4, explicitOmitObservationCount: 6,
    faults: records.slice(4).map((record, index) => ({ kind: index ? 'reminder' : 'create', requestId: record.requestId, forwardedRequestId: record.requestId,
      upstreamStatus: index ? 200 : 201, freshCookieJar: true, forwardedOrigin: 'null', forwardedCookiePresent: false,
      forwardedBearerPresent: true, forwardedRequestIdPresent: true, normalTls: true, disposed: true })) }
}
function fixture() {
  const tree = ['index.html', 'build-info.json', 'asset-manifest.json', 'assets/entry.js', 'assets/app.js'].map(path => ({ path, bytes: 32, sha256: digest }))
  const cleanup = Object.fromEntries(workOrdersBrowserCleanup.map(key => [key, true]))
  const result = { schemaVersion: 1, status: 'PASS', mode: 'actual-production-host-vite-opaque-rust-postgres-nginx', product: 'chrome', sourceSha256: digest,
    passed: 15, failed: 0, skipped: 0, checks: workOrdersBrowserChecks.map(id => ({ id, status: 'PASS' })), cleanup: structuredClone(cleanup),
    source: { platformBinary: { sha256: digest }, businessBinary: { sha256: digest }, productionHost: tree, viteBuild: tree,
      releaseManifest: { environment: 'development', version: 'phase21', entryType: 'HTML', entryUrl: '/sub-apps/work-orders/versions/phase21/index.html', resources: tree.slice(1).map(item => ({ url: item.path, kind: 'JS' })) },
      developmentLoopbackUpstreams: true, gatewayTlsVerified: true },
    evidence: { browser: { product: 'chrome', version: '155.0.1', playwrightVersion: '1.64.1' },
      ordinary: { superAdmin: false, managementPermissions: false, directMembership: true },
      assets: { moduleEntry: 'assets/entry.js', dynamicChunks: ['assets/app.js'], manifestDynamicImports: ['src/app.mjs'], sandbox: 'allow-scripts allow-forms', actualVueMounted: true,
        loaded: ['entry', 'app'].map(name => ({ path: '/sub-apps/work-orders/versions/phase21/assets/' + name + '.js', status: 200, contentType: 'application/javascript', allowOrigin: '*', origin: 'null', resourceType: 'script' })) },
      transport: transportFixture(),
      theme: { before: 1, after: 1 }, crud: { ticketId: 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee', createdStatus: 201, assignee: true, directoryRustRequest: true, status: 'IN_PROGRESS', filterVerified: true, softDeleted: true,
        csv: { realRustExport: true, hostDownload: true, sandboxUnchanged: true } },
      conflict: { status: 409, draftPreserved: true, automaticOverwrite: false, explicitVersionAccepted: 3 },
      reminder: { publicationCount: 1, deliveryCount: 1, recipientInboxVerified: true, ticketVersionUnchanged: true, realtimeStarted: false },
      reminderRecovery: { committedResponseDropped: true, sameBody: true, sameKey: true, publicationCount: 1, deliveryCount: 1, automaticRetryCount: 0 },
      createRecovery: { committedResponseDropped: true, sameBody: true, sameKey: true, receiptCount: 1, ticketCount: 1, automaticRetryCount: 0 },
      menuIsolation: { createCannotRead: true, readCannotCreate: true, actualContextAndRust403: true }, revocation: { actualRustStatus: 403, oldRowsVisible: false } } }
  return { schemaVersion: 1, kind: 'work-orders-real-platform-browser', status: 'PASS', product: 'chrome', sourceRevision: revision, inputsSha256: digest,
    ownership: { owner: 'ci-12345678', hostname: 'ci-12345678-postgres', database: 'postgres' },
    sourceUnchanged: true, privateWorkRemoved: true, cleanupErrors: [], result, cleanup,
    steps: workOrdersBrowserSteps.map(id => ({ id, status: 'PASS', exitCode: 0, timedOut: false })),
    binaries: { platform: identity, business: identity }, tls: { caSha256: digest, serverSha256: digest, ipVerified: true, ownedTrustOnly: true },
    tools: { node: 'v24.18.0', nginx: 'nginx version: nginx/1.28.0', nginxLibraries: 'libc.so.6 => /lib/x86_64-linux-gnu/libc.so.6', openssl: 'OpenSSL 3.0.1', packages: 'nginx=1.22\nopenssl=3.0\nlibnss3-tools=3.87\n' } }
}
const verdict = document => evaluate({ format: 'work-orders-browser', document, exitCode: 0, expectedProduct: 'chrome' })
const output = ({ result, cleanup }) => 'PHASE21_BROWSER_CLEANUP ' + JSON.stringify(cleanup) + '\nPHASE21_BROWSER_RESULT ' + JSON.stringify(result) + '\n'
function temporary(t) {
  const parent = realpathSync(os.tmpdir()), directory = realpathSync(mkdtempSync(path.join(parent, 'saas-work-orders-browser-policy-')))
  t.after(() => { assert.equal(path.dirname(realpathSync(directory)), parent); assert.match(path.basename(directory), /^saas-work-orders-browser-policy-/); assert(!lstatSync(directory).isSymbolicLink()); rmSync(directory, { recursive: true, force: true }) })
  return directory
}
function put(root, relative, value) { const file = path.join(root, relative); mkdirSync(path.dirname(file), { recursive: true }); writeFileSync(file, value); return file }
function launcherFixture(t) {
  const directory = temporary(t), root = path.join(directory, 'source'), outDir = path.join(directory, 'result')
  put(root, 'package.json', '{}'); put(root, 'tests/phase21-work-orders-browser.mjs', 'reviewed browser source')
  const bytes = Buffer.alloc(32); bytes.write('ELF', 1); bytes[0] = 0x7f; bytes[4] = 2; bytes[5] = 1; bytes.writeUInt16LE(62, 18)
  for (const bin of ['saas-platform-backend', 'work-orders-api']) put(root, 'target/ci-rust/debug/' + bin + (process.platform === 'win32' ? '.exe' : ''), bytes)
  return { root, outDir, product: 'chrome', sourceRevision: revision, env: owned }
}

test('real work-order report requires all fifteen actual checks, bound sources, verified TLS and owned cleanup', () => {
  const good = fixture()
  assert.equal(verdict(good).status, 'PASS'); assert.equal(verdict(good).counts.pass, 15)
  assert.equal(parseWorkOrdersBrowserOutput(output(good), { product: 'chrome' }).result.passed, 15)
  const mutations = [
    value => { value.result.checks.pop() }, value => { value.result.checks[0] = value.result.checks[1] },
    value => { value.result.checks[0].status = 'SKIP' }, value => { value.result.skipped = 1 }, value => { value.result.passed = '15' },
    value => { value.product = 'chromium' }, value => { value.result.evidence.browser.product = 'msedge' }, value => { value.result.evidence.browser.playwrightVersion = 'latest' },
    value => { value.result.evidence.ordinary.superAdmin = true }, value => { value.result.evidence.ordinary.managementPermissions = true },
    value => { value.result.evidence.assets.dynamicChunks = [] }, value => { value.result.evidence.assets.loaded[1].status = 404 },
    value => { value.result.evidence.assets.manifestDynamicImports = [] }, value => { value.result.evidence.assets.loaded[0].origin = 'https://platform.example.com' },
    value => { value.result.evidence.assets.loaded[1].allowOrigin = 'https://platform.example.com' }, value => { value.result.evidence.assets.loaded[0].contentType = 'text/html' },
    value => { value.result.evidence.transport.origin = 'https://platform.example.com' }, value => { value.result.evidence.transport.cookiePresent = true },
    value => { delete value.result.evidence.transport.credentials }, value => { value.result.evidence.transport.credentials = 'include' }, value => { value.result.evidence.transport.credentials = 'same-origin' },
    value => { delete value.result.evidence.transport.explicitOmitObservationCount }, value => { value.result.evidence.transport.explicitOmitObservationCount = 11 },
    value => { value.result.evidence.transport.credentialsAllowed = true }, value => { value.result.evidence.theme.after++ },
    value => { value.result.evidence.crud.ticketId = 'fixture-ticket' }, value => { value.result.evidence.crud.csv.realRustExport = false },
    value => { value.result.evidence.conflict.status = 200 }, value => { value.result.evidence.conflict.draftPreserved = false }, value => { value.result.evidence.conflict.explicitVersionAccepted = true },
    value => { value.result.evidence.reminder.publicationCount++ }, value => { value.result.evidence.reminder.realtimeStarted = true },
    value => { value.result.evidence.reminderRecovery.automaticRetryCount = 1 }, value => { value.result.evidence.reminderRecovery.sameKey = false },
    value => { value.result.evidence.createRecovery.sameBody = false }, value => { value.result.evidence.createRecovery.ticketCount = 2 },
    value => { value.result.evidence.menuIsolation.actualContextAndRust403 = false }, value => { value.result.evidence.revocation.oldRowsVisible = true },
    value => { value.result.evidence.revocation.actualRustStatus = 200 }, value => { value.result.source.productionHost = [] },
    value => { value.result.source.viteBuild = value.result.source.viteBuild.slice(0, -1) }, value => { value.result.source.releaseManifest.entryUrl = '/fixture' },
    value => { value.result.source.gatewayTlsVerified = false }, value => { value.result.source.platformBinary.sha256 = 'c'.repeat(64) },
    value => { value.steps.pop() }, value => { value.steps[0].status = 'SKIP' }, value => { value.steps[0].exitCode = 1 }, value => { value.steps[0].timedOut = true },
    value => { value.binaries.platform.elfMachine = 0 }, value => { value.tls.ownedTrustOnly = false }, value => { value.tools.nginx = '' }, value => { value.tools.nginx = 'nginx version: nginx/1.22.1' }, value => { value.tools.nginxLibraries = 'libc.so.6 => not found' },
    value => { value.sourceUnchanged = false }, value => { value.privateWorkRemoved = false }, value => { value.cleanupErrors = ['private TLS retained'] },
    value => { value.ownership.hostname = 'production' },
    ...workOrdersBrowserCleanup.map(key => value => { value.cleanup[key] = false }),
  ]
  for (const mutate of mutations) { const value = structuredClone(good); mutate(value); assert.equal(verdict(value).status, 'FAIL') }
  for (const changed of [{ exitCode: 1 }, { timedOut: true }, { expectedProduct: 'msedge' }]) assert.equal(evaluate({ format: 'work-orders-browser', document: good, exitCode: 0, expectedProduct: 'chrome', ...changed }).status, 'FAIL')
})

test('typed transport closes attempts, browser events, proved HTTP, cancellation and isolated faults by request ID', () => {
  const good = fixture(), base = good.result.evidence.transport
  assert.notEqual(base.nativeAttemptCount, base.browserRequestEventCount)
  assert.notEqual(base.explicitOmitObservationCount, base.actualRequestCount)
  assert.equal(verdict(good).status, 'PASS')
  const noCancellation = structuredClone(good), reduced = noCancellation.result.evidence.transport
  reduced.records = reduced.records.filter(record => record.classification === 'browser-response' || record.classification === 'fixture-response-drop')
  Object.assign(reduced, { nativeAttemptCount: 4, browserRequestEventCount: 4, abortBeforeRequestEventCount: 0, abortedWithoutWireConfirmationCount: 0, explicitOmitObservationCount: 4 })
  assert.equal(verdict(noCancellation).status, 'PASS', 'No fixed diagnostic totals or required cancelled reads may be assumed')
  const mutations = [
    transport => { transport.records.pop() }, transport => { transport.records[0].requestId = transport.records[1].requestId },
    transport => { transport.records[0].browserEvent.requestId = transport.records[1].requestId },
    transport => { transport.records[0].browserEvent.method = 'POST' }, transport => { transport.records[0].browserEvent.path = '/work-orders-api/other' },
    transport => { transport.records[0].browserEvent.outcome = 'failed'; transport.records[0].browserEvent.failureCode = 'net::ERR_FAILED' },
    transport => { transport.records[0].classification = 'ignored-failed-request' }, transport => { transport.records[0].nativeOutcome = 'pending' },
    transport => { transport.records[2].credentials = 'same-origin' }, transport => { transport.records[2].nativeHeaders.cookiePresent = true },
    transport => { transport.records[2].nativeHeaders.bearerPresent = false }, transport => { transport.records[2].nativeHeaders.requestIdPresent = false },
    transport => { transport.records[2].method = 'POST' }, transport => { transport.records[2].signalAborted = false },
    transport => { transport.records[2].nativeErrorName = 'TypeError' }, transport => { transport.records[2].nativeOutcome = 'fulfilled' },
    transport => { transport.records[3].browserEvent.origin = 'https://platform.example.com' }, transport => { transport.records[3].browserEvent.cookiePresent = true },
    transport => { delete transport.records[3].browserEvent.origin },
    transport => { transport.records[3].browserEvent.bearerPresent = false }, transport => { transport.records[3].browserEvent.requestIdPresent = false },
    transport => { transport.records[3].browserEvent.responseStatus = 200 }, transport => { transport.records[3].browserEvent.failureCode = 'net::ERR_FAILED' },
    transport => { transport.records[3].browserEvent.frameType = 'host' }, transport => { transport.records[3].signalAborted = false },
    transport => { transport.faults.pop() }, transport => { transport.faults[0].kind = 'reminder' }, transport => { transport.faults[0].requestId = transport.records[0].requestId },
    transport => { transport.faults[0].forwardedRequestId = transport.records[1].requestId }, transport => { transport.faults[0].freshCookieJar = false },
    transport => { transport.faults[0].normalTls = false }, transport => { transport.faults[0].disposed = false }, transport => { transport.faults[0].upstreamStatus = 200 },
    transport => { transport.records[4].browserEvent.origin = null },
    transport => { transport.faults[0].forwardedOrigin = null }, transport => { transport.faults[0].forwardedCookiePresent = true },
    transport => { transport.faults[0].forwardedBearerPresent = false }, transport => { transport.faults[0].forwardedRequestIdPresent = false },
    ...['nativeAttemptCount', 'browserRequestEventCount', 'confirmedWireCount', 'responseBodyCancelledCount', 'abortBeforeRequestEventCount', 'abortedWithoutWireConfirmationCount', 'faultInjectedCount', 'actualRequestCount', 'explicitOmitObservationCount'].flatMap(key => [
      transport => { transport[key]++ }, transport => { delete transport[key] },
    ]),
  ]
  for (const mutate of mutations) { const changed = structuredClone(good); mutate(changed.result.evidence.transport); assert.equal(verdict(changed).status, 'FAIL', mutate.toString()) }
})

test('confirmed HTTP error responses retain a separately typed body-cancelled terminal event', () => {
  const good = fixture(), transport = good.result.evidence.transport
  for (const [index, status, method] of [[0, 403, 'GET'], [1, 409, 'PATCH']]) {
    const record = transport.records[index]
    record.classification = 'browser-error-body-cancelled'; record.method = method
    if (method === 'PATCH') record.path += '/aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee'
    Object.assign(record.browserEvent, { method, path: record.path, outcome: 'failed', responseStatus: status, failureCode: 'net::ERR_ABORTED' })
  }
  transport.responseBodyCancelledCount = 2
  assert.equal(verdict(good).status, 'PASS')
  const parsed = parseWorkOrdersBrowserOutput(output(good), { product: 'chrome' })
  assert.deepEqual(parsed.result.evidence.transport.records.slice(0, 2).map(record => [record.browserEvent.outcome, record.browserEvent.responseStatus, record.nativeOutcome, record.browserEvent.failureCode]),
    [['failed', 403, 'fulfilled', 'net::ERR_ABORTED'], ['failed', 409, 'fulfilled', 'net::ERR_ABORTED']])
  assert.equal(parsed.result.evidence.transport.confirmedWireCount, 2)
  assert.equal(parsed.result.evidence.transport.abortedWithoutWireConfirmationCount, 1)
  const mutations = [
    value => { delete value.responseBodyCancelledCount }, value => { value.responseBodyCancelledCount = 1 },
    value => { value.records[0].browserEvent.responseStatus = null }, value => { delete value.records[0].browserEvent.responseStatus },
    value => { value.records[0].browserEvent.responseStatus = 200 }, value => { value.records[0].browserEvent.responseStatus = 399 },
    value => { value.records[0].browserEvent.responseStatus = 600 }, value => { value.records[0].browserEvent.responseStatus = '403' },
    value => { value.records[0].nativeOutcome = 'rejected'; value.records[0].nativeErrorName = 'AbortError'; value.records[0].signalAborted = true },
    value => { value.records[0].nativeErrorName = 'TypeError' }, value => { value.records[0].browserEvent.outcome = 'finished' },
    value => { value.records[0].browserEvent.failureCode = 'net::ERR_FAILED' }, value => { value.records[0].browserEvent.failureCode = null },
    value => { value.records[0].browserEvent.origin = null }, value => { value.records[0].browserEvent.origin = 'https://platform.example.com' },
    value => { value.records[0].browserEvent.cookiePresent = true }, value => { value.records[0].browserEvent.bearerPresent = false },
    value => { value.records[0].browserEvent.requestIdPresent = false }, value => { value.records[0].classification = 'aborted-without-wire-confirmation' },
    value => { value.records[0].classification = 'browser-response' },
  ]
  for (const mutate of mutations) { const changed = structuredClone(good); mutate(changed.result.evidence.transport); assert.equal(verdict(changed).status, 'FAIL', mutate.toString()) }
})

test('missing, duplicate, skipped and malformed producer markers cannot become mandatory PASS', () => {
  const good = fixture(), text = output(good)
  for (const malformed of ['', text + text, text.replace('PHASE21_BROWSER_CLEANUP ', 'FIXTURE_CLEANUP '), text.replace('"skipped":0', '"skipped":1'), text.replace('"status":"PASS"', '"status":"SKIP"'), 'PHASE21_BROWSER_RESULT {}\nPHASE21_BROWSER_CLEANUP {}\n']) assert.throws(() => parseWorkOrdersBrowserOutput(malformed, { product: 'chrome' }))
  assert.throws(() => validateWorkOrdersBrowserResult(good, { product: 'firefox' }))
})

test('browser CLI and environment require a declared product, explicit owned PostgreSQL and private per-run trust stores', () => {
  const args = ['--browser', 'chrome', '--out-dir', path.resolve('target/new-report'), '--source-revision', revision]
  assert.equal(parseWorkOrdersBrowserArgs(args).product, 'chrome')
  for (const invalid of [args.slice(0, -2), [...args, '--browser', 'chrome'], args.map(value => value === 'chrome' ? 'firefox' : value), args.map(value => value === revision ? 'HEAD' : value)]) assert.throws(() => parseWorkOrdersBrowserArgs(invalid))
  const root = path.resolve('source'), workDir = path.resolve('target/new-report/work'), screenshotsDir = path.resolve('target/new-report/screenshots')
  const env = workOrdersBrowserEnvironment({ ...owned, JWT_SECRET: 'private', DATABASE_URL: 'private', NODE_OPTIONS: 'private', NPM_TOKEN: 'private', WORK_ORDERS_BROWSER_TLS_CA: '/outside-ca', HOME: '/user-home', XDG_DATA_HOME: '/user-nss', SAAS_TEST_BROWSER: 'msedge' }, { product: 'chrome', root, workDir, screenshotsDir })
  for (const key of ['JWT_SECRET', 'DATABASE_URL', 'NODE_OPTIONS', 'NPM_TOKEN']) assert.equal(env[key], undefined)
  assert.equal(env.TEST_DATABASE_URL, owned.TEST_DATABASE_URL); assert.equal(env.SAAS_TEST_BROWSER, 'chrome')
  assert.equal(env.HOME, path.join(workDir, 'browser-home')); assert.equal(env.XDG_DATA_HOME, path.join(workDir, 'browser-data'))
  assert.equal(env.WORK_ORDERS_BROWSER_TLS_CA, path.join(workDir, 'tls/ca.pem')); assert.equal(env.WORK_ORDERS_BROWSER_REPORT_DIR, screenshotsDir)
  for (const invalid of [{ ...owned, TEST_DATABASE_URL: 'postgresql://ci:synthetic@production:5432/postgres' }, { ...owned, TEST_DATABASE_URL: owned.TEST_DATABASE_URL + '#unowned' }, { ...owned, CI_TEST_POSTGRES_OWNER: undefined }]) assert.throws(() => workOrdersBrowserEnvironment(invalid, { product: 'chrome', root, workDir, screenshotsDir }))
})

test('launcher refuses unsafe output before executing and cleans private work after a real command failure', async t => {
  const options = launcherFixture(t)
  await assert.rejects(runWorkOrdersBrowser({ ...options, outDir: path.join(options.root, 'new-output/..'), execute: () => assert.fail('must not execute') }), /fresh absolute/)
  await assert.rejects(runWorkOrdersBrowser({ ...options, outDir: path.join(options.root, 'tests/evidence'), execute: () => assert.fail('must not execute') }), /included source/)
  let calls = 0
  const report = await runWorkOrdersBrowser({ ...options, execute: async (_program, _args, { env }) => {
    calls++; assert.equal(env.SAAS_TEST_BROWSER, 'chrome'); assert.equal(env.HOME, path.join(options.outDir, 'work/browser-home'))
    return { exitCode: 1, timedOut: false, output: owned.TEST_DATABASE_URL + ' Bearer synthetic-token' }
  } })
  assert.equal(calls, 1); assert.equal(report.status, 'FAIL'); assert.equal(report.sourceUnchanged, true)
  assert.equal(report.privateWorkRemoved, true); assert.equal(existsSync(path.join(options.outDir, 'work')), false)
  assert.doesNotMatch(readFileSync(path.join(options.outDir, 'logs/01-nginx-version.log'), 'utf8'), /synthetic-only|synthetic-token/)
})

test('launcher detects a source mutation even when its child reports successful commands', async t => {
  const options = launcherFixture(t)
  const report = await runWorkOrdersBrowser({ ...options, execute: async () => { put(options.root, 'package.json', '{"changed":true}'); return { exitCode: 1, timedOut: false, output: '' } } })
  assert.equal(report.status, 'FAIL'); assert.equal(report.sourceUnchanged, false); assert.equal(report.privateWorkRemoved, true)
  assert.match(report.error, /Source inputs changed/)
})

test('owned CA recipe verifies the IP and stops on TLS failure without launching or trusting another certificate', async t => {
  const options = launcherFixture(t), commands = []
  const report = await runWorkOrdersBrowser({ ...options, execute: async (program, args, context) => {
    commands.push({ program, args, env: context.env })
    if (program === 'openssl' && args[0] === 'req') for (const key of ['-keyout', '-out']) if (args.includes(key)) writeFileSync(args[args.indexOf(key) + 1], 'synthetic fixture bytes')
    if (program === 'openssl' && args[0] === 'x509') writeFileSync(args[args.indexOf('-out') + 1], 'synthetic fixture bytes')
    return { exitCode: program === 'openssl' && args[0] === 'verify' ? 1 : 0, timedOut: false,
      output: program === '/usr/sbin/nginx' ? 'nginx version: nginx/1.28.0' : program === 'ldd' ? 'libc.so.6 => /lib/libc.so.6' : 'controlled tool result' }
  } })
  assert.equal(report.status, 'FAIL'); assert.equal(report.privateWorkRemoved, true)
  const verify = commands.find(item => item.program === 'openssl' && item.args[0] === 'verify')
  assert.ok(verify); assert.ok(verify.args.includes('-verify_ip')); assert.ok(verify.args.includes('127.0.0.1'))
  assert.equal(commands.filter(item => item.program === 'openssl' && item.args.includes('-x509')).length, 1)
  assert.ok(!commands.some(item => item.program === 'certutil' || item.args.includes('tests/phase21-work-orders-browser.mjs')))
  assert.ok(commands.every(item => item.env.NODE_TLS_REJECT_UNAUTHORIZED === undefined))
})
