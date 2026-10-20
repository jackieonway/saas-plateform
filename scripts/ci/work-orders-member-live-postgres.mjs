import { readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { workOrdersMemberLivePostgresSuites } from './manifest.mjs'
import { runPostgres } from './postgres.mjs'
import { parseWorkOrdersMemberLiveOutput } from './work-orders-member-live-results.mjs'

export async function runWorkOrdersMemberLivePostgres(options = {}) {
  const root = options.root || process.cwd(), env = options.env || process.env
  const report = await runPostgres({ ...options, root,
    env: { ...env, WORK_ORDERS_BINARY: path.join(root, 'target/ci-rust/debug', process.platform === 'win32' ? 'work-orders-api.exe' : 'work-orders-api') },
    suites: workOrdersMemberLivePostgresSuites, kind: 'work-orders-member-real-platform-postgres' })
  try {
    report.live = parseWorkOrdersMemberLiveOutput(readFileSync(path.join(options.outDir, report.suites[0].log), 'utf8'))
  } catch {
    report.status = 'FAIL'
    report.failedSuites++
    report.error = 'Actual platform/work-orders member result or owned process/database cleanup is incomplete'
  }
  writeFileSync(path.join(options.outDir, 'postgres-result.json'), JSON.stringify(report, null, 2) + '\n')
  if (report.status !== 'PASS') throw new Error(report.error)
  return report
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const args = process.argv.slice(2)
  if (args[0] !== '--run-tests' || args[1] !== '--out-dir' || !args[2] || args.length !== 3) {
    console.error('Usage: node scripts/ci/work-orders-member-live-postgres.mjs --run-tests --out-dir <fresh absolute directory> (inside owned Linux runner)'); process.exitCode = 2
  } else runWorkOrdersMemberLivePostgres({ outDir: path.resolve(args[2]) }).catch(error => { console.error(error.message); process.exitCode = 1 })
}
