import { readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { workOrdersWriteLivePostgresSuites } from './manifest.mjs'
import { runPostgres } from './postgres.mjs'
import { parseWorkOrdersWriteLiveOutput } from './work-orders-write-live-results.mjs'

export async function runWorkOrdersWriteLivePostgres(options = {}) {
  const root = options.root || process.cwd(), env = options.env || process.env
  const report = await runPostgres({ ...options, root,
    env: { ...env, WORK_ORDERS_BINARY: path.join(root, 'target/ci-rust/debug', process.platform === 'win32' ? 'work-orders-api.exe' : 'work-orders-api') },
    suites: workOrdersWriteLivePostgresSuites, kind: 'work-orders-write-real-platform-postgres' })
  try {
    report.live = parseWorkOrdersWriteLiveOutput(readFileSync(path.join(options.outDir, report.suites[0].log), 'utf8'))
  } catch {
    report.status = 'FAIL'
    report.failedSuites++
    report.error = 'Actual platform/work-orders write result or owned process/database cleanup is incomplete'
  }
  writeFileSync(path.join(options.outDir, 'postgres-result.json'), JSON.stringify(report, null, 2) + '\n')
  if (report.status !== 'PASS') throw new Error(report.error)
  return report
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const args = process.argv.slice(2)
  if (args[0] !== '--run-tests' || args[1] !== '--out-dir' || !args[2] || args.length !== 3) {
    console.error('Usage: node scripts/ci/work-orders-write-live-postgres.mjs --run-tests --out-dir <fresh absolute directory> (inside owned Linux runner)'); process.exitCode = 2
  } else runWorkOrdersWriteLivePostgres({ outDir: path.resolve(args[2]) }).catch(error => { console.error(error.message); process.exitCode = 1 })
}
