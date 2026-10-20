import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { workOrdersReadPostgresSuites } from './manifest.mjs'
import { runPostgres } from './postgres.mjs'

// This target executes a real owned business database and an explicit protocol
// fixture. The separate platform-live suite proves the actual platform wiring.
export function runWorkOrdersReadPostgres(options = {}) {
  return runPostgres({ ...options, suites: workOrdersReadPostgresSuites,
    manifestPath: 'business-apps/work-orders/backend/Cargo.toml', binary: undefined,
    kind: 'work-orders-read-isolated-postgres' })
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const args = process.argv.slice(2)
  if (args[0] !== '--run-tests' || args[1] !== '--out-dir' || !args[2] || args.length !== 3) {
    console.error('Usage: node scripts/ci/work-orders-read-postgres.mjs --run-tests --out-dir <fresh absolute directory> (inside owned Linux runner)'); process.exitCode = 2
  } else runWorkOrdersReadPostgres({ outDir: path.resolve(args[2]) }).catch(error => { console.error(error.message); process.exitCode = 1 })
}
