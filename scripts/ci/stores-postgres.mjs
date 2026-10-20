import { readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { storesPostgresSuites } from './manifest.mjs'
import { runPostgres, validateOwnedPostgres } from './postgres.mjs'
import { commandEnvironment } from './report.mjs'
import { parseStoresFixtureOutput } from './stores-live-results.mjs'

export function storesPostgresEnvironment(env, root, { live = false } = {}) {
  validateOwnedPostgres(env)
  if (new URL(env.TEST_DATABASE_URL).hash) throw new Error('Owned stores PostgreSQL URL cannot contain a fragment')
  const child = { ...commandEnvironment(env), TEST_DATABASE_URL: env.TEST_DATABASE_URL, CI_TEST_POSTGRES_OWNER: env.CI_TEST_POSTGRES_OWNER }
  if (live) {
    const suffix = process.platform === 'win32' ? '.exe' : ''
    child.STORES_BINARY = path.join(root, 'target/ci-rust/debug/stores-api' + suffix)
    child.WORK_ORDERS_BINARY = path.join(root, 'target/ci-rust/debug/work-orders-api' + suffix)
  }
  return child
}
export async function runStoresPostgres(options = {}) {
  const root = options.root || process.cwd()
  const report = await runPostgres({ ...options, root, env: storesPostgresEnvironment(options.env || process.env, root), exact: true,
    suites: storesPostgresSuites, manifestPath: 'business-apps/stores/backend/Cargo.toml', binary: undefined, kind: 'stores-isolated-postgres' })
  try { report.fixture = parseStoresFixtureOutput(readFileSync(path.join(options.outDir, report.suites[0].log), 'utf8')) }
  catch { report.status = 'FAIL'; report.failedSuites++; report.error = 'Actual stores SQL checks, Rust result or owned database cleanup are incomplete' }
  writeFileSync(path.join(options.outDir, 'postgres-result.json'), JSON.stringify(report, null, 2) + '\n')
  if (report.status !== 'PASS') throw new Error(report.error)
  return report
}
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const args = process.argv.slice(2)
  if (args.length !== 3 || args[0] !== '--run-tests' || args[1] !== '--out-dir' || !path.isAbsolute(args[2] || '')) {
    console.error('Usage: node scripts/ci/stores-postgres.mjs --run-tests --out-dir <fresh absolute directory> (inside owned Linux runner)'); process.exitCode = 2
  } else runStoresPostgres({ outDir: path.resolve(args[2]) }).catch(error => { console.error(error.message); process.exitCode = 1 })
}
