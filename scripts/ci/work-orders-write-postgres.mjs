import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { workOrdersWritePostgresSuites } from './manifest.mjs'
import { runPostgres } from './postgres.mjs'

export function runWorkOrdersWritePostgres(options = {}) {
  return runPostgres({ ...options, suites: workOrdersWritePostgresSuites,
    manifestPath: 'business-apps/work-orders/backend/Cargo.toml', binary: undefined,
    kind: 'work-orders-write-isolated-postgres' })
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const args = process.argv.slice(2)
  if (args[0] !== '--run-tests' || args[1] !== '--out-dir' || !args[2] || args.length !== 3) {
    console.error('Usage: node scripts/ci/work-orders-write-postgres.mjs --run-tests --out-dir <fresh absolute directory> (inside owned Linux runner)'); process.exitCode = 2
  } else runWorkOrdersWritePostgres({ outDir: path.resolve(args[2]) }).catch(error => { console.error(error.message); process.exitCode = 1 })
}
