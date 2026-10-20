import { spawn, spawnSync } from 'node:child_process'
import { writeFileSync } from 'node:fs'
import { postgresSuites, workOrdersPostgresSuites, workOrdersReadPostgresSuites, workOrdersWritePostgresSuites, workOrdersMemberPostgresSuites, workOrdersLivePostgresSuites, workOrdersWriteLivePostgresSuites, workOrdersMemberLivePostgresSuites, workOrdersReminderPostgresSuites, workOrdersReminderLivePostgresSuites, storesPostgresSuites, storesLivePostgresSuites } from './manifest.mjs'
import { validateDataGrantUiReport } from './data-grant-ui-results.mjs'
import { validateWorkOrdersLiveResult } from './work-orders-live-results.mjs'
import { validateWorkOrdersWriteLiveResult } from './work-orders-write-live-results.mjs'
import { validateWorkOrdersReminderFixtureResult, validateWorkOrdersReminderLiveResult } from './work-orders-reminder-live-results.mjs'
import { validateWorkOrdersMemberFixtureResult, validateWorkOrdersMemberLiveResult } from './work-orders-member-live-results.mjs'
import { validateWorkOrdersBrowserReport } from './work-orders-browser-results.mjs'
import { validateStoresFixtureResult, validateStoresLiveResult } from './stores-live-results.mjs'
import { validateStoresBrowserReport } from './stores-browser-results.mjs'
import { validateBusinessDeliveryReport } from './business-delivery-results.mjs'
import { validateCapacity } from './monitoring-results.mjs'

export function redact(value) {
  return String(value).replace(/\bpostgres(?:ql)?:\/\/[^\s"'<>]+/gi, '[database-url-redacted]')
    .replace(/\bBearer\s+[^\s"']+/gi, 'Bearer [redacted]')
    .replace(/((?:JWT_SECRET|CLIENT_SECRET|NPM_TOKEN|PASSWORD|ACCESS_TOKEN)["']?\s*[:=]\s*)["']?[^\s"',}]+/gi, '$1[redacted]')
}
export function commandEnvironment(source = process.env) {
  const names = ['PATH','Path','PATHEXT','SystemRoot','SYSTEMROOT','WINDIR','COMSPEC','ComSpec','TEMP','TMP','TMPDIR','HOME','USERPROFILE','LOCALAPPDATA','APPDATA','ProgramFiles','ProgramFiles(x86)','PROGRAMFILES','PROGRAMFILES(X86)','SystemDrive','CARGO_HOME','RUSTUP_HOME','DISPLAY','XDG_RUNTIME_DIR','LANG','LC_ALL']
  const result = Object.fromEntries(names.filter(key => source[key]).map(key => [key, source[key]]))
  Object.assign(result, { CI: 'true', CARGO_BUILD_JOBS: '2', RUST_BACKTRACE: '0', NPM_CONFIG_UPDATE_NOTIFIER: 'false' })
  if (source.PLAYWRIGHT_BROWSERS_PATH === '/opt/ci/browser/.browsers') result.PLAYWRIGHT_BROWSERS_PATH = source.PLAYWRIGHT_BROWSERS_PATH
  return result
}
const zero = () => ({ tests: 0, pass: 0, fail: 0, skip: 0, cancelled: 0, todo: 0, filtered: 0 })
const platformAdminTlsChecks = Object.freeze([
  'trusted-CA-and-hostname-with-private-bearer',
  'untrusted-certificate-rejected',
  'wrong-certificate-hostname-rejected',
  'private-key-as-CA-rejected-without-echo',
  'HTTPS-redirect-rejected-and-redacted',
  'HTTPS-response-size-limit',
  'successful-summary-bearer-echo-rejected',
  'write-summary-bearer-echo-preserves-reconciliation',
  'HTTPS-total-deadline-and-redaction',
])
const notificationImageChecks = Object.freeze([
  'clean-Rust-source-locked-release-build-and-image-identity',
  'nonroot-no-Node-runtime',
  'default-closed-stdin-keeps-production-service-running',
  'liveness-UP-readiness-DOWN-with-dependencies-unavailable',
  'metrics-require-independent-Bearer',
  'read-only-capability-free-private-container-without-host-mounts',
  'SIGTERM-graceful-zero-exit',
  'logs-exclude-private-credentials',
  'candidate-inputs-match-current-Rust-source',
])
export function evaluate({ format = 'command', exitCode, output = '', document, expectedSuites = [], expectedChecks = [], expectedProduct, expectedSourceRevision, expectedInputsSha256, timedOut = false } = {}) {
  const counts = zero(); let valid = true, reason
  if (format === 'tap' || format === 'platform-admin-tls') {
    const mapping = { tests: 'tests', pass: 'pass', fail: 'fail', skipped: 'skip', cancelled: 'cancelled', todo: 'todo' }, seen = new Set()
    for (const match of output.matchAll(/^# (tests|pass|fail|skipped|cancelled|todo) (\d+)\s*$/gm)) { counts[mapping[match[1]]] = Number(match[2]); seen.add(match[1]) }
    valid = seen.size === 6 && counts.tests > 0 && counts.pass > 0
    if (format === 'platform-admin-tls') {
      const checks = document?.checks
      valid &&= counts.tests === 1 && counts.pass === 1 && document?.schemaVersion === 1 && document?.status === 'PASS'
        && document?.passed === 9 && document?.failed === 0 && document?.skipped === 0
        && document?.privateTlsRemoved === true && typeof document?.binarySha256 === 'string' && /^[0-9a-f]{64}$/.test(document.binarySha256)
        && Array.isArray(checks) && checks.length === platformAdminTlsChecks.length
        && platformAdminTlsChecks.every(id => checks.filter(check => check?.id === id && check?.status === 'PASS').length === 1)
    }
  } else if (format === 'cargo') {
    const summaries = [...output.matchAll(/test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; \d+ measured; (\d+) filtered out/g)]
    valid = summaries.length > 0
    for (const [, status, passed, failed, ignored, filtered] of summaries) { counts.pass += Number(passed); counts.fail += Number(failed); counts.skip += Number(ignored); counts.filtered += Number(filtered); if (status !== 'ok') valid = false }
    counts.tests = counts.pass + counts.fail + counts.skip; valid &&= counts.pass > 0
  } else if (format === 'notification') {
    const expected = ['npm-notification','build-platform','build-notification','build-proxy','real-platform-scheduler',
      ...(expectedProduct ? ['npm-frontend','npm-tests-browser',`two-page-browser-${expectedProduct}`] : [])]
    const steps = document?.steps || []
    for (const step of steps) if (step.counts) {
      for (const name of Object.keys(counts)) counts[name] += step.counts[name] || 0
    }
    valid = document?.kind === 'notification-real-platform-postgres' && document.status === 'PASS' &&
      (document.browser || undefined) === expectedProduct && steps.length === expected.length &&
      expected.every(name => steps.filter(step => step.name === name && step.status === 'PASS' && step.exitCode === 0 && !step.timedOut).length === 1) &&
      counts.tests > 0 && counts.pass > 0 && !counts.fail && !counts.skip && !counts.cancelled && !counts.todo
  } else if (format === 'business-delivery') {
    const checks = Array.isArray(document?.checks) ? document.checks : []
    Object.assign(counts, { tests: checks.length, pass: checks.filter(check => check?.status === 'PASS').length,
      fail: checks.filter(check => check?.status === 'FAIL').length, skip: checks.filter(check => ['SKIP','NOT_RUN'].includes(check?.status)).length })
    try { validateBusinessDeliveryReport(document, { sourceRevision: expectedSourceRevision, inputsSha256: expectedInputsSha256 }) } catch { valid = false }
  } else if (format === 'notification-image') {
    const checks = Array.isArray(document?.checks) ? document.checks : []
    const candidates = Array.isArray(document?.images) ? document.images.filter(image => image?.role === 'candidate') : []
    const candidate = candidates[0], runtime = document?.runtime, shutdown = document?.shutdown
    Object.assign(counts, { tests: checks.length, pass: checks.filter(id => notificationImageChecks.includes(id)).length })
    valid = document?.schemaVersion === 1 && document.status === 'PASS' && document.currentSourceMatch === true &&
      /^[a-f0-9]{40}$/.test(document.sourceRevision || '') && (!expectedSourceRevision || document.sourceRevision === expectedSourceRevision) &&
      /^[a-f0-9]{64}$/.test(document.inputsSha256 || '') && document.publication === 'NOT_REQUESTED' && document.productionDeployment === 'NOT_REQUESTED' &&
      checks.length === notificationImageChecks.length && notificationImageChecks.every(id => checks.filter(value => value === id).length === 1) &&
      Array.isArray(document.cleanupErrors) && document.cleanupErrors.length === 0 && candidates.length === 1 &&
      /^sha256:[a-f0-9]{64}$/.test(candidate?.imageId || '') && candidate?.labels?.['org.opencontainers.image.revision'] === document.sourceRevision &&
      candidate?.labels?.['saas.ci.inputs-sha256'] === document.inputsSha256 && runtime?.user === '10003:10003' && runtime.readOnly === true &&
      runtime.noNewPrivileges === true && runtime.openStdin === false && ['capabilities','hostPorts','sourceOrSocketMounts'].every(name => Array.isArray(runtime[name]) && runtime[name].length === 0) &&
      shutdown?.signal === 'SIGTERM' && shutdown.exitCode === 0 && Number.isFinite(shutdown.elapsedMs) && shutdown.elapsedMs >= 0 &&
      (!document.wslLease || (document.wslLease.ready === true && document.wslLease.stopped === true && document.wslLease.exitCode === 0))
  } else if (format === 'browser') {
    const suites = document?.suites || [], totals = document?.totals
    Object.assign(counts, { tests: (totals?.pass || 0) + (totals?.fail || 0) + (totals?.skip || 0), pass: totals?.pass || 0, fail: totals?.fail || 0, skip: totals?.skip || 0 })
    valid = Boolean(document?.browserVersion) && suites.length === expectedSuites.length && expectedSuites.every(id => suites.filter(s => s.id === id && s.pass > 0 && !s.fail && !s.skip && s.exitCode === 0).length === 1) && counts.pass > 0
  } else if (format === 'monitoring-postgres') {
    Object.assign(counts, { pass: document?.passed || 0, fail: document?.failed || 0, skip: document?.skipped || 0 }); counts.tests = counts.pass + counts.fail + counts.skip
    valid = document?.schemaVersion === 1 && document.status === 'PASS' && document.kind === 'phase32-capacity-fault-postgres' && counts.pass === 1 && !counts.fail && !counts.skip && document.suites?.length === 1 && document.suites[0].name === 'phase26_stores_tests::phase32::phase32_capacity_and_locks' && document.suites[0].status === 'PASS' && document.suites[0].expected === 1 && document.suites[0].passed === 1 && !document.suites[0].failed && !document.suites[0].skipped
    try { validateCapacity(document?.capacity) } catch { valid = false }
  } else if (format === 'stores-postgres' || format === 'stores-live-postgres') {
    const manifest = format === 'stores-postgres' ? storesPostgresSuites : storesLivePostgresSuites
    Object.assign(counts, { pass: document?.passed || 0, fail: document?.failed || 0, skip: document?.skipped || 0 }); counts.tests = counts.pass + counts.fail + counts.skip
    valid = document?.schemaVersion === 1 && document.status === 'PASS' && document.failedSuites === 0 && document.passed === 1 && document.failed === 0 && document.skipped === 0
      && document.kind === (format === 'stores-postgres' ? 'stores-isolated-postgres' : 'stores-real-platform-postgres')
      && Array.isArray(document.suites) && document.suites.length === manifest.length
      && manifest.every(expected => document.suites.filter(suite => suite?.name === expected.name && suite.expected === expected.expected && suite.passed === expected.expected
        && suite.failed === 0 && suite.skipped === 0 && suite.status === 'PASS' && suite.exitCode === 0 && suite.timedOut === false).length === 1)
      && /^ci-[a-z0-9-]{8,80}$/.test(document.ownership?.owner || '') && document.ownership?.hostname === document.ownership?.owner + '-postgres' && document.ownership?.database === 'postgres'
    try { if (format === 'stores-postgres') validateStoresFixtureResult(document?.fixture); else validateStoresLiveResult(document?.live) } catch { valid = false }
  } else if (format === 'postgres' || format === 'work-orders-postgres' || format === 'work-orders-read-postgres' || format === 'work-orders-write-postgres' || format === 'work-orders-member-postgres' || format === 'work-orders-live-postgres' || format === 'work-orders-write-live-postgres' || format === 'work-orders-member-live-postgres' || format === 'work-orders-reminder-postgres' || format === 'work-orders-reminder-live-postgres') {
    const manifest = format === 'postgres' ? postgresSuites : format === 'work-orders-postgres' ? workOrdersPostgresSuites : format === 'work-orders-read-postgres' ? workOrdersReadPostgresSuites : format === 'work-orders-write-postgres' ? workOrdersWritePostgresSuites : format === 'work-orders-member-postgres' ? workOrdersMemberPostgresSuites : format === 'work-orders-live-postgres' ? workOrdersLivePostgresSuites : format === 'work-orders-write-live-postgres' ? workOrdersWriteLivePostgresSuites : format === 'work-orders-member-live-postgres' ? workOrdersMemberLivePostgresSuites : format === 'work-orders-reminder-postgres' ? workOrdersReminderPostgresSuites : workOrdersReminderLivePostgresSuites
    Object.assign(counts, { pass: document?.passed || 0, fail: document?.failed || 0, skip: document?.skipped || 0 }); counts.tests = counts.pass + counts.fail + counts.skip
    valid = document?.status === 'PASS' && document.suites?.length === manifest.length && manifest.every(expected=>document.suites.filter(s=>s.name===expected.name&&s.status==='PASS'&&s.expected===expected.expected&&s.passed===expected.expected&&!s.failed&&!s.skipped).length===1) && counts.pass === manifest.reduce((total,suite)=>total+suite.expected,0)
    if (format === 'work-orders-postgres') valid &&= document?.kind === 'work-orders-isolated-postgres'
    if (format === 'work-orders-read-postgres') valid &&= document?.kind === 'work-orders-read-isolated-postgres'
    if (format === 'work-orders-write-postgres') valid &&= document?.kind === 'work-orders-write-isolated-postgres'
    if (format === 'work-orders-member-postgres') {
      valid &&= document?.kind === 'work-orders-member-isolated-postgres'
      try { validateWorkOrdersMemberFixtureResult(document?.fixture, workOrdersMemberPostgresSuites.reduce((sum, suite) => sum + suite.expected, 0)) } catch { valid = false }
    }
    if (format === 'work-orders-live-postgres') {
      valid &&= document?.kind === 'work-orders-real-platform-postgres'
      try { validateWorkOrdersLiveResult(document?.live) } catch { valid = false }
    }
    if (format === 'work-orders-write-live-postgres') {
      valid &&= document?.kind === 'work-orders-write-real-platform-postgres'
      try { validateWorkOrdersWriteLiveResult(document?.live) } catch { valid = false }
    }
    if (format === 'work-orders-member-live-postgres') {
      valid &&= document?.kind === 'work-orders-member-real-platform-postgres'
      try { validateWorkOrdersMemberLiveResult(document?.live) } catch { valid = false }
    }
    if (format === 'work-orders-reminder-postgres') {
      valid &&= document?.kind === 'work-orders-reminder-isolated-postgres'
      try { validateWorkOrdersReminderFixtureResult(document?.fixture, workOrdersReminderPostgresSuites.reduce((sum, suite) => sum + suite.expected, 0)) } catch { valid = false }
    }
    if (format === 'work-orders-reminder-live-postgres') {
      valid &&= document?.kind === 'work-orders-reminder-real-platform-postgres'
      try { validateWorkOrdersReminderLiveResult(document?.live) } catch { valid = false }
    }
  } else if (format === 'data-grant-ui') {
    const browser = document?.browser
    Object.assign(counts, { tests: (browser?.passed || 0) + (browser?.failed || 0) + (browser?.skipped || 0), pass: browser?.passed || 0, fail: browser?.failed || 0, skip: browser?.skipped || 0 })
    try { validateDataGrantUiReport(document, { product: expectedProduct }) } catch { valid = false }
  } else if (format === 'work-orders-browser') {
    const result = document?.result
    Object.assign(counts, { tests: (result?.passed || 0) + (result?.failed || 0) + (result?.skipped || 0), pass: result?.passed || 0, fail: result?.failed || 0, skip: result?.skipped || 0 })
    try { validateWorkOrdersBrowserReport(document, { product: expectedProduct }) } catch { valid = false }
  } else if (format === 'stores-browser') {
    const result = document?.result
    Object.assign(counts, { tests: (result?.passed || 0) + (result?.failed || 0) + (result?.skipped || 0), pass: result?.passed || 0, fail: result?.failed || 0, skip: result?.skipped || 0 })
    try { validateStoresBrowserReport(document, { product: expectedProduct }) } catch { valid = false }
    if (expectedSourceRevision) valid &&= document?.sourceRevision === expectedSourceRevision
  } else if (format === 'zip-release') {
    const checks = document?.checks || []
    Object.assign(counts, { tests: checks.length, pass: checks.filter(check=>check.status==='PASS').length, fail: checks.filter(check=>check.status==='FAIL').length, skip: checks.filter(check=>check.status==='SKIP'||check.status==='NOT_RUN').length })
    valid = document?.status === 'PASS' && expectedChecks.length > 0 && checks.length === expectedChecks.length && expectedChecks.every(id=>checks.filter(check=>check.id===id&&check.status==='PASS').length===1) && Array.isArray(document?.cleanupErrors) && document.cleanupErrors.length===0
  } else if (format === 'release') {
    const probe=document?.images?.find(image=>image.name==='platform')?.runtimeVerification
    valid = document?.status === 'PASS' && document.build === 'PASS' && document.smoke === 'PASS' && document.images?.length === 3 && document.images.every(image => /^sha256:[0-9a-f]{64}$/.test(image.ociManifestDigest || '') && image.commit === document.commit && image.buildId === document.buildId) && probe?.uid===10001 && probe.sharedLibrariesResolved===true && /^[a-f0-9]{64}$/.test(probe.binarySha256||'') && [62,183].includes(probe.elfMachine)
  } else if (format !== 'command') valid = false
  const status = exitCode === 0 && !timedOut && valid && !counts.fail && !counts.skip && !counts.cancelled && !counts.todo ? 'PASS' : 'FAIL'
  if (status === 'FAIL') reason = timedOut ? 'deadline exceeded' : exitCode !== 0 ? 'command failed' : !valid ? 'missing or incomplete mandatory result' : 'mandatory failures, skipped, cancelled or todo tests'
  return { status, counts, ...(reason ? { reason } : {}) }
}

export function runCommand(program, args, { cwd, env, timeout = 20 * 60_000, log } = {}) {
  return new Promise((resolve) => {
    const child = spawn(program, args, { cwd, env, windowsHide: true, detached: process.platform !== 'win32', stdio: ['ignore', 'pipe', 'pipe'] })
    let output = '', timedOut = false, error, overflow = false
    const stop = () => {
      if (!child.pid) return
      if (process.platform === 'win32') spawnSync('taskkill.exe', ['/PID', String(child.pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore', timeout: 10_000 })
      else { try { process.kill(-child.pid, 'SIGKILL') } catch { /* already exited */ } }
    }
    const timer = setTimeout(() => { timedOut = true; stop() }, timeout)
    const collect = data => { if (output.length + data.length > 64 * 1024 * 1024) { overflow = true; stop() } else output += data }
    child.stdout.on('data', collect); child.stderr.on('data', collect)
    child.on('error', failure => { error = failure.message })
    child.on('close', (exitCode, signal) => {
      clearTimeout(timer)
      const safe = redact(output + (error ? `\n${error}\n` : '') + (overflow ? '\nOutput limit exceeded\n' : ''))
      if (log) writeFileSync(log, safe, { mode: 0o600 })
      resolve({ exitCode: error || overflow ? 1 : exitCode, signal, timedOut, output: safe })
    })
  })
}
