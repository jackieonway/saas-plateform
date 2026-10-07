import assert from 'node:assert/strict'
import test from 'node:test'
import http from 'node:http'
import { createMonitor, probeTarget, validateConfig } from '../monitor.mjs'

const TOKEN = 'FixtureOnlyMetricsToken0123456789AbCd'
const input = (targets, patch = {}) => ({ environment: 'development', eventLog: '/tmp/monitor-fixture-only.jsonl',
  timeoutMs: 100, intervalMs: 1000, maxResponseBytes: 1024, downThreshold: 2, recoveryThreshold: 2, targets, ...patch })
async function fixture(t) {
  const state = { up: true, visits: 0, authorized: false }
  const server = http.createServer((req, res) => {
    if (req.url === '/hung') return
    if (req.url === '/redirect') { res.writeHead(302, { location: '/secret-target' }); res.end(); return }
    if (req.url === '/secret-target') state.visits++
    if (req.url === '/large') { res.setHeader('content-type', 'application/json'); res.end(' '.repeat(2048)); return }
    if (req.url === '/metrics') {
      state.authorized = req.headers.authorization === 'Bearer ' + TOKEN
      res.setHeader('content-type', 'text/plain; version=0.0.4')
      if (!state.authorized) res.writeHead(401)
      res.end('saas_health_up{service="fixture"} 1\n'); return
    }
    if (req.url === '/asset') { res.end('window.cfg={"version":"1.0.0"};'); return }
    res.setHeader('content-type', 'application/json')
    res.end(JSON.stringify({ status: state.up ? 'UP' : 'DOWN', sensitiveBody: 'fixture-body-never-logged' }))
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  t.after(async () => { server.closeAllConnections(); await new Promise((resolve) => server.close(resolve)) })
  return { state, base: `http://127.0.0.1:${server.address().port}` }
}

test('monitor config keeps public production URLs HTTPS and metrics private with explicit environment token', () => {
  assert.throws(() => validateConfig(input([{ name: 'platform', kind: 'health', url: 'http://outside.test/health' }], { environment: 'production' })))
  assert.throws(() => validateConfig(input([{ name: 'platform', kind: 'health', url: 'http://127.0.0.1/health' }], { environment: 'production' })))
  assert.throws(() => validateConfig(input([{ name: 'metrics', kind: 'prometheus', private: true, url: 'http://localhost/metrics', tokenEnv: 'METRICS_TOKEN' }], { environment: 'production' }), { METRICS_TOKEN: TOKEN }))
  for (const url of ['https://user:pass@platform.test/health', 'https://platform.test/health?token=secret', 'https://platform.test/health#x']) {
    assert.throws(() => validateConfig(input([{ name: 'platform', kind: 'health', url }])))
  }
  assert.throws(() => validateConfig(input([{ name: 'metrics', kind: 'prometheus', url: 'https://platform.test/metrics' }])))
  const config = validateConfig(input([{ name: 'metrics', kind: 'prometheus', private: true, url: 'http://127.0.0.1:3001/api/internal/metrics', tokenEnv: 'METRICS_TOKEN' }], { environment: 'production' }), { METRICS_TOKEN: TOKEN })
  assert.equal(config.targets[0].token, TOKEN)
  assert.throws(() => validateConfig(input([{ name: 'platform', kind: 'health', url: 'https://platform.test/health' }]), { NODE_TLS_REJECT_UNAUTHORIZED: '0' }))
})

test('actual HTTP fixtures: probes enforce protected metrics, health contract and exact release marker', async (t) => {
  const { base, state } = await fixture(t)
  const config = validateConfig(input([
    { name: 'health', kind: 'health', url: base + '/health' },
    { name: 'metrics', kind: 'prometheus', url: base + '/metrics', tokenEnv: 'METRICS_TOKEN' },
    { name: 'release', kind: 'asset', url: base + '/asset', contains: '"version":"1.0.0"' },
  ]), { METRICS_TOKEN: TOKEN })
  for (const target of config.targets) assert.deepEqual(await probeTarget(target, config), { up: true, reason: 'UP' })
  assert.equal(state.authorized, true)
  assert.deepEqual(await probeTarget({ ...config.targets[2], contains: '"version":"2.0.0"' }, config), { up: false, reason: 'VERSION' })
  state.up = false; assert.deepEqual(await probeTarget(config.targets[0], config), { up: false, reason: 'CONTRACT' })
})

test('actual HTTP fixtures: redirect, unlimited body and stalled requests fail without leaking or forwarding credentials', async (t) => {
  const { base, state } = await fixture(t)
  const config = validateConfig(input([{ name: 'metrics', kind: 'prometheus', url: base + '/redirect', tokenEnv: 'METRICS_TOKEN' }]), { METRICS_TOKEN: TOKEN })
  assert.deepEqual(await probeTarget(config.targets[0], config), { up: false, reason: 'UNAVAILABLE' }); assert.equal(state.visits, 0)
  assert.deepEqual(await probeTarget({ name: 'large', kind: 'health', url: base + '/large' }, config), { up: false, reason: 'BODY_LIMIT' })
  assert.deepEqual(await probeTarget({ name: 'hung', kind: 'health', url: base + '/hung' }, config), { up: false, reason: 'TIMEOUT' })
})

test('actual HTTP fixtures: local alerts emit one DOWN and one RECOVERED after thresholds and no quiet-state messages', async (t) => {
  const { base, state } = await fixture(t), events = []
  const config = validateConfig(input([{ name: 'platform', kind: 'health', url: base + '/health' }]))
  const monitor = createMonitor(config, { emit: async (event) => events.push(event) })
  await monitor.check(); await monitor.check(); assert.equal(events.length, 0)
  state.up = false; await monitor.check(); assert.equal(events.length, 0); await monitor.check(); await monitor.check()
  assert.deepEqual(events.map((entry) => entry.event), ['DOWN'])
  state.up = true; await monitor.check(); assert.equal(events.length, 1); await monitor.check(); await monitor.check()
  assert.deepEqual(events.map((entry) => entry.event), ['DOWN', 'RECOVERED'])
  const text = JSON.stringify(events)
  for (const value of [base, TOKEN, 'fixture-body-never-logged', 'Authorization']) assert.ok(!text.includes(value))
})
