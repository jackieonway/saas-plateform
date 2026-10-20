import { spawn } from 'node:child_process'
import { mkdirSync, writeFileSync, existsSync } from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { redact as redactReport } from './report.mjs'
import { postgresSuites } from './manifest.mjs'
export { postgresSuites } from './manifest.mjs'

// These are existing platform tests, not promises about future Rust services.
export function validateOwnedPostgres(env) {
  const owner = env.CI_TEST_POSTGRES_OWNER
  if (!/^ci-[a-z0-9-]{8,80}$/.test(owner || '')) throw new Error('PostgreSQL gate requires the owned environment created by scripts/ci/linux-runner.mjs')
  let url
  try { url = new URL(env.TEST_DATABASE_URL) } catch { throw new Error('Missing owned TEST_DATABASE_URL; never fall back to .env.test') }
  if (url.protocol !== 'postgresql:' || url.hostname !== `${owner}-postgres` || url.port !== '5432' || url.username !== 'ci' || !url.password || url.pathname !== '/postgres' || url.search) {
    throw new Error('PostgreSQL gate refuses a database outside the temporary runner container')
  }
  return { owner, hostname: url.hostname, database: 'postgres', credentials: 'synthetic ephemeral; redacted' }
}

function cargo(args, root, env) {
  return new Promise((resolve, reject) => {
    const child = spawn('cargo', args, { cwd: root, env, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true })
    let output = '', timedOut = false
    const timer = setTimeout(() => { timedOut = true; child.kill('SIGTERM') }, 20 * 60_000)
    child.stdout.on('data', data => { output += data }); child.stderr.on('data', data => { output += data })
    child.on('error', error => { clearTimeout(timer); reject(error) })
    child.on('close', code => { clearTimeout(timer); resolve({ code, output, timedOut }) })
  })
}

export function readRustCounts(output) {
  const summaries = [...output.matchAll(/test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out/g)]
  if (summaries.length !== 1) throw new Error('Expected exactly one selected Rust binary test result')
  const [, status, passed, failed, skipped] = summaries[0]
  return { status, passed: Number(passed), failed: Number(failed), skipped: Number(skipped) }
}

export function parseRustResult(output, expected) {
  const { status, ...counts } = readRustCounts(output)
  if (status !== 'ok' || counts.passed !== expected || counts.failed || counts.skipped) throw new Error(`Required PostgreSQL suite expected ${expected} passes with no failures or skips; observed ${JSON.stringify(counts)}`)
  return counts
}

export async function runPostgres({ root = process.cwd(), outDir, env = process.env, runCargo = cargo, suites = postgresSuites,
  manifestPath = 'backend/Cargo.toml', binary = 'saas-platform-backend', kind = 'existing-platform-isolated-postgres', exact = false } = {}) {
  const ownership = validateOwnedPostgres(env)
  if (!path.isAbsolute(outDir || '') || existsSync(outDir)) throw new Error('PostgreSQL report directory must be a fresh absolute directory')
  mkdirSync(outDir, { recursive: true })
  const report = { schemaVersion: 1, kind, startedAt: new Date().toISOString(), ownership, suites: [], passed: 0, failed: 0, skipped: 0, failedSuites: 0 }
  const secrets = [env.TEST_DATABASE_URL, new URL(env.TEST_DATABASE_URL).password].filter(Boolean)
  const redact = value => redactReport(secrets.reduce((text, secret) => text.replaceAll(secret, '[REDACTED]'), value))
  // Never inherit an unrelated production database or dotenv configuration.
  const childEnv = { ...env, DATABASE_URL: env.TEST_DATABASE_URL, CARGO_TARGET_DIR: path.join(root, 'target/ci-rust'), RUST_BACKTRACE: '0' }
  delete childEnv.SQLX_OFFLINE_DIR
  try {
    for (const suite of suites) {
      const selection = suite.target ? ['--test', suite.target] : ['--bin', binary]
      const args = ['test', '--manifest-path', manifestPath, '--locked', ...selection, '-j', '2', ...(suite.target ? (suite.filter ? [suite.filter] : []) : [suite.name]), '--', ...(exact ? ['--exact'] : []), '--ignored', '--test-threads=1', '--nocapture']
      const result = await runCargo(args, root, childEnv)
      const logfile = `${suite.name.replaceAll('::', '-')}.log`
      writeFileSync(path.join(outDir, logfile), redact(result.output))
      const entry = { name: suite.name, expected: suite.expected, command: ['cargo', ...args], exitCode: result.code, timedOut: result.timedOut, log: logfile }
      try {
        const { status: _rustStatus, ...counts } = readRustCounts(result.output)
        Object.assign(entry, counts)
        report.passed += counts.passed; report.failed += counts.failed; report.skipped += counts.skipped
      } catch { entry.resultMissing = true }
      try {
        parseRustResult(result.output, suite.expected)
        if (result.code !== 0 || result.timedOut) throw new Error('Rust command failed or timed out')
        entry.status = 'PASS'
      } catch (error) {
        Object.assign(entry, { status: 'FAIL', error: redact(error.message) }); report.failedSuites++
      }
      report.suites.push(entry)
    }
  } catch (error) { report.error = redact(error.message) }
  report.finishedAt = new Date().toISOString()
  report.status = report.suites.length !== suites.length || !suites.length || report.failedSuites || report.failed || report.skipped || report.error ? 'FAIL' : 'PASS'
  writeFileSync(path.join(outDir, 'postgres-result.json'), `${JSON.stringify(report, null, 2)}\n`)
  if (report.status !== 'PASS') throw new Error(`PostgreSQL gate failed; inspect ${path.join(outDir, 'postgres-result.json')}`)
  return report
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const args = process.argv.slice(2)
  if (args[0] !== '--run-tests' || args[1] !== '--out-dir' || !args[2] || args.length !== 3) {
    console.error('Usage: node scripts/ci/postgres.mjs --run-tests --out-dir <fresh absolute directory> (inside owned Linux runner)'); process.exitCode = 2
  } else runPostgres({ outDir: path.resolve(args[2]) }).catch(error => { console.error(error.message); process.exitCode = 1 })
}
