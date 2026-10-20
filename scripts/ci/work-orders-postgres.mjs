import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { workOrdersPostgresSuites } from './manifest.mjs'
import { runPostgres } from './postgres.mjs'

// Reuse the exact ownership, redaction, timeout and result checks of the platform
// harness while keeping the independent business crate and report separate.
export function runWorkOrdersPostgres(options = {}) {
  return runPostgres({ ...options, suites: workOrdersPostgresSuites,
    manifestPath: 'business-apps/work-orders/backend/Cargo.toml', binary: undefined,
    kind: 'work-orders-isolated-postgres' })
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const args = process.argv.slice(2)
  if (args[0] !== '--run-tests' || args[1] !== '--out-dir' || !args[2] || args.length !== 3) {
    console.error('Usage: node scripts/ci/work-orders-postgres.mjs --run-tests --out-dir <fresh absolute directory> (inside owned Linux runner)'); process.exitCode = 2
  } else runWorkOrdersPostgres({ outDir: path.resolve(args[2]) }).catch(error => { console.error(error.message); process.exitCode = 1 })
}
