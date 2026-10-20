import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { workOrdersMemberPostgresSuites } from './manifest.mjs'
import { runPostgres } from './postgres.mjs'
import { readFileSync, writeFileSync } from 'node:fs'
import { parseWorkOrdersMemberFixtureOutput } from './work-orders-member-live-results.mjs'

export async function runWorkOrdersMemberPostgres(options = {}) {
  const report = await runPostgres({ ...options, suites: workOrdersMemberPostgresSuites,
    manifestPath: 'business-apps/work-orders/backend/Cargo.toml', binary: undefined,
    kind: 'work-orders-member-isolated-postgres' })
  try {
    report.fixture = parseWorkOrdersMemberFixtureOutput(readFileSync(path.join(options.outDir, report.suites[0].log), 'utf8'), report.suites[0].expected)
  } catch {
    report.status = 'FAIL'; report.failedSuites++
    report.error = 'Actual member fixture cleanup or deterministic lock/organization/expiry proof is incomplete'
  }
  writeFileSync(path.join(options.outDir, 'postgres-result.json'), JSON.stringify(report, null, 2) + '\n')
  if (report.status !== 'PASS') throw new Error(report.error)
  return report
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const args = process.argv.slice(2)
  if (args[0] !== '--run-tests' || args[1] !== '--out-dir' || !args[2] || args.length !== 3) {
    console.error('Usage: node scripts/ci/work-orders-member-postgres.mjs --run-tests --out-dir <fresh absolute directory> (inside owned Linux runner)'); process.exitCode = 2
  } else runWorkOrdersMemberPostgres({ outDir: path.resolve(args[2]) }).catch(error => { console.error(error.message); process.exitCode = 1 })
}
