import assert from 'node:assert/strict'
import test from 'node:test'
import { evaluate } from '../scripts/ci/report.mjs'

const report = browser => ({ kind: 'notification-real-platform-postgres', status: 'PASS', browser,
  steps: ['npm-notification','build-platform','build-notification','build-proxy','real-platform-scheduler',
    ...(browser ? ['npm-frontend','npm-tests-browser',`two-page-browser-${browser}`] : [])].map(name => ({
      name, status: 'PASS', exitCode: 0, counts: name === 'real-platform-scheduler' || name.startsWith('two-page')
        ? { tests: 1, pass: 1, fail: 0, skip: 0, cancelled: 0, todo: 0 } : undefined,
    })) })
const verdict = (document, expectedProduct) => evaluate({ format: 'notification', exitCode: 0, document, expectedProduct }).status

test('notification gate requires the actual named Rust/platform/PG steps and zero-skip tests', () => {
  assert.equal(verdict(report()), 'PASS')
  assert.equal(verdict(undefined), 'FAIL')
  for (const change of [r => { r.steps.pop() }, r => { r.steps.at(-1).counts.skip = 1 },
    r => { r.steps.at(-1).counts.tests = 0; r.steps.at(-1).counts.pass = 0 },
    r => { r.steps[0].exitCode = 1 }, r => { r.steps[1].timedOut = true },
    r => { r.steps.push(r.steps[1]) }, r => { r.status = 'FAIL' }]) {
    const document = report(); change(document); assert.equal(verdict(document), 'FAIL')
  }
})

test('notification browser evidence must include the selected actual product and page suite', () => {
  assert.equal(verdict(report('chrome'), 'chrome'), 'PASS')
  assert.equal(verdict(report('chromium'), 'chrome'), 'FAIL')
  assert.equal(verdict(report(), 'chrome'), 'FAIL')
  const document = report('chrome'); document.steps.at(-1).counts.cancelled = 1
  assert.equal(verdict(document, 'chrome'), 'FAIL')
})

const imageChecks = [
  'clean-Rust-source-locked-release-build-and-image-identity', 'nonroot-no-Node-runtime',
  'default-closed-stdin-keeps-production-service-running', 'liveness-UP-readiness-DOWN-with-dependencies-unavailable',
  'metrics-require-independent-Bearer', 'read-only-capability-free-private-container-without-host-mounts',
  'SIGTERM-graceful-zero-exit', 'logs-exclude-private-credentials', 'candidate-inputs-match-current-Rust-source',
]
function imageReport() {
  const sourceRevision = 'a'.repeat(40), inputsSha256 = 'b'.repeat(64)
  return { schemaVersion: 1, status: 'PASS', sourceRevision, inputsSha256, currentSourceMatch: true,
    publication: 'NOT_REQUESTED', productionDeployment: 'NOT_REQUESTED', checks: [...imageChecks], cleanupErrors: [],
    images: [{ role: 'candidate', imageId: 'sha256:' + 'c'.repeat(64), labels: {
      'org.opencontainers.image.revision': sourceRevision, 'saas.ci.inputs-sha256': inputsSha256,
    } }], runtime: { user: '10003:10003', readOnly: true, capabilities: [], noNewPrivileges: true,
      hostPorts: [], sourceOrSocketMounts: [], openStdin: false },
    shutdown: { signal: 'SIGTERM', exitCode: 0, elapsedMs: 3000 },
    wslLease: { ready: true, stopped: true, exitCode: 0 },
  }
}
const imageVerdict = (document, extra = {}) => evaluate({ format: 'notification-image', exitCode: 0,
  document, expectedSourceRevision: 'a'.repeat(40), ...extra }).status

test('notification image gate requires all checks, exact current inputs and completed cleanup', () => {
  assert.equal(imageVerdict(imageReport()), 'PASS')
  assert.equal(imageVerdict(undefined), 'FAIL')
  assert.equal(imageVerdict(imageReport(), { exitCode: 1 }), 'FAIL')
  assert.equal(imageVerdict(imageReport(), { timedOut: true }), 'FAIL')
  for (const change of [r => { r.status = 'FAIL' }, r => { r.checks = [] }, r => { r.checks.pop() },
    r => { r.checks[8] = r.checks[0] }, r => { r.checks[8] = 'SKIP' },
    r => { r.currentSourceMatch = false }, r => { r.sourceRevision = 'd'.repeat(40) },
    r => { r.cleanupErrors.push('owned container remains') }, r => { r.wslLease.stopped = false },
    r => { r.images = [null] }, r => { r.images[0].imageId = 'notification:latest' },
    r => { r.images[0].labels['saas.ci.inputs-sha256'] = 'e'.repeat(64) },
    r => { r.images.push(r.images[0]) }, r => { r.publication = 'PUSHED' }]) {
    const document = imageReport(); change(document); assert.equal(imageVerdict(document), 'FAIL')
  }
})

test('notification image gate rejects root, writable, exposed and forced-killed runtime claims', () => {
  for (const change of [r => { r.runtime.user = '0:0' }, r => { r.runtime.readOnly = false },
    r => { r.runtime.capabilities.push('NET_ADMIN') }, r => { r.runtime.noNewPrivileges = false },
    r => { r.runtime.hostPorts.push(3003) }, r => { r.runtime.sourceOrSocketMounts.push('/var/run/docker.sock') },
    r => { r.runtime.openStdin = true }, r => { r.shutdown.signal = 'SIGKILL' },
    r => { r.shutdown.exitCode = 137 }, r => { delete r.shutdown }]) {
    const document = imageReport(); change(document); assert.equal(imageVerdict(document), 'FAIL')
  }
  const linux = imageReport(); delete linux.wslLease
  assert.equal(imageVerdict(linux), 'PASS')
})
