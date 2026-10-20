import { readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { storesLivePostgresSuites } from './manifest.mjs'
import { runPostgres } from './postgres.mjs'
import { storesPostgresEnvironment } from './stores-postgres.mjs'
import { parseStoresLiveOutput } from './stores-live-results.mjs'

export async function runStoresLivePostgres(options = {}) {
  const root = options.root || process.cwd()
  const report = await runPostgres({ ...options, root, env: storesPostgresEnvironment(options.env || process.env, root, { live: true }), exact: true,
    suites: storesLivePostgresSuites, kind: 'stores-real-platform-postgres' })
  try { report.live = parseStoresLiveOutput(readFileSync(path.join(options.outDir, report.suites[0].log), 'utf8')) }
  catch { report.status = 'FAIL'; report.failedSuites++; report.error = 'Actual platform/stores/work-orders results, database roles or owned resource cleanup are incomplete' }
  writeFileSync(path.join(options.outDir, 'postgres-result.json'), JSON.stringify(report, null, 2) + '\n')
  if (report.status !== 'PASS') throw new Error(report.error)
  return report
}
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const args = process.argv.slice(2)
  if (args.length !== 3 || args[0] !== '--run-tests' || args[1] !== '--out-dir' || !path.isAbsolute(args[2] || '')) {
    console.error('Usage: node scripts/ci/stores-live-postgres.mjs --run-tests --out-dir <fresh absolute directory> (inside owned Linux runner)'); process.exitCode = 2
  } else runStoresLivePostgres({ outDir: path.resolve(args[2]) }).catch(error => { console.error(error.message); process.exitCode = 1 })
}
