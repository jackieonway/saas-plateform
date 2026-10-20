import assert from 'node:assert/strict'
import { readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { runPostgres } from './postgres.mjs'

const args = process.argv.slice(2)
assert(args.length === 3 && args[0] === '--run-tests' && args[1] === '--out-dir' && path.isAbsolute(args[2] || ''), 'Expected --run-tests --out-dir <fresh absolute path>')
const outDir = args[2]
const report = await runPostgres({ outDir, exact: true, kind: 'phase30-current-protocol-postgres', suites: [{ name: 'phase26_stores_tests::phase30::phase30_current_protocol', expected: 1 }] })
try {
  const log = readFileSync(path.join(outDir, report.suites[0].log), 'utf8')
  const marker = prefix => {
    const matches = [...log.matchAll(new RegExp(prefix + ' (\\{[^\\n]+\\})', 'g'))]
    assert.equal(matches.length, 1, prefix + ' must occur once')
    return JSON.parse(matches[0][1])
  }
  const live = marker('PHASE30_LIVE_RESULT'), budget = marker('PHASE30_BUDGET_RESULT'), cleanup = marker('STORES_LIVE_PLATFORM_CLEANUP')
  assert.equal(live.applications, 2); assert.equal(live.protocol, 'PASS')
  for (const field of ['freshCommittedGrantRemoval', 'hashIsNotCredential', 'extremePaging']) assert.equal(live[field], true)
  for (const field of ['saturation503', 'statementTimeout57014', 'lockTimeout55P03', 'poolTimeout', 'poolRecovery']) assert.equal(budget[field], true)
  for (const field of ['assertionsOk', 'databaseRemoved', 'poolClosed', 'roleRemoved']) assert.equal(cleanup[field], true)
  report.protocol = live; report.budget = budget; report.cleanup = cleanup
} catch (error) { report.status = 'FAIL'; report.error = error.message }
writeFileSync(path.join(outDir, 'postgres-result.json'), JSON.stringify(report, null, 2) + '\n')
assert.equal(report.status, 'PASS', report.error)
