/** Fixed candidate capacity/fault selection, inside the existing owned Linux runner. */
import assert from 'node:assert/strict'
import { readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { runPostgres } from './postgres.mjs'
import { storesPostgresEnvironment } from './stores-postgres.mjs'
import { validateCapacity } from './monitoring-results.mjs'
const args = process.argv.slice(2)
assert(args.length === 3 && args[0] === '--run-tests' && args[1] === '--out-dir' && path.isAbsolute(args[2] || ''), 'Expected --run-tests --out-dir <fresh absolute directory>')
const outDir = args[2], root = process.cwd(), binary = name => path.join(root, 'target/ci-rust/debug', name + (process.platform === 'win32' ? '.exe' : ''))
const env = { ...storesPostgresEnvironment(process.env, root, { live: true }), PLATFORM_BINARY: binary('saas-platform-backend'), MONITOR_BINARY: binary('saas-monitor'), PHASE32_BASELINE: '1', PHASE32_TEST_DB_MAX_CONNECTIONS: '10', PHASE32_TEST_DEFAULT_MODES: '1', PHASE32_TEST_METRICS_TOKEN: 'phase32-owned-operational-canary', PHASE32_REPORT: path.join(outDir, 'capacity.json') }
const report = await runPostgres({ outDir, env, exact: true, kind: 'phase32-capacity-fault-postgres', suites: [{ name: 'phase26_stores_tests::phase32::phase32_capacity_and_locks', expected: 1 }] })
try {
  const log = readFileSync(path.join(outDir, report.suites[0].log), 'utf8')
  assert(log.includes('PHASE32_CAPACITY PASS'))
  for (const kind of ['PLATFORM', 'STORES', 'WORK_ORDERS']) {
    const match = log.match(new RegExp('STORES_LIVE_' + kind + '_CLEANUP (\\{[^\\n]+\\})'))
    assert(match, 'Owned database cleanup evidence required')
    for (const key of ['assertionsOk', 'databaseRemoved', 'roleRemoved', 'poolClosed']) assert.equal(JSON.parse(match[1])[key], true)
  }
  const measured = JSON.parse(readFileSync(env.PHASE32_REPORT, 'utf8'))
  validateCapacity(measured); report.capacity = measured
  assert.equal(measured.records.workOrders + measured.records.dailyReports, 100000)
  assert.deepEqual(measured.rounds.map(round => round.activeSessions), [100,150,200])
  assert.equal(measured.faults.events.filter(e => e.component === 'stores' && e.event === 'DOWN').length, 2)
} catch (error) { report.status = 'FAIL'; report.error = error.message }
writeFileSync(path.join(outDir, 'postgres-result.json'), JSON.stringify(report, null, 2) + '\n')
assert.equal(report.status, 'PASS', report.error)
