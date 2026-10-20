/** Opt-in Phase25 CLI -> real platform -> owned PostgreSQL evidence entry point. */
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { commandEnvironment, redact, runCommand } from './report.mjs'
import { validateOwnedPostgres, parseRustResult } from './postgres.mjs'
import { createSourceManifest } from './provenance.mjs'
import { validateOutputLocation } from './linux-runner.mjs'

export const platformAdminTest = 'platform_admin_cli_tests::real_cli_platform_round_trip'
export const platformAdminChecks = Object.freeze([
  'plan-and-no-apply-no-database-writes',
  'bootstrap-repeat-preserves-ids-and-counts',
  'actual-client-id-associated-with-own-application',
  'foreign-client-and-conflicting-menu-rejected',
  'register-only-existing-allowed-fields',
  'existing-release-version-rejected-and-queryable',
  'url-preflight-two-activations-deployments-rollback',
  'grant-plan-and-no-apply-offline-readonly',
  'grant-apply-formal-api-and-exact-direct-membership',
  'grant-independent-read-write-and-root-context',
  'grant-no-permission-organization-and-cross-app-denied',
  'same-token-next-context-and-member-scope',
  'grant-request-bound-audit-and-secret-free-output',
])

function marker(output, name) {
  // Cargo's first --nocapture write can follow its still-open `test ... ` line.
  const cargoPrefix = `test ${platformAdminTest} ... `
  const matches = output.split(/\r?\n/)
    .map(line => line.startsWith(cargoPrefix) ? line.slice(cargoPrefix.length) : line)
    .filter(line => line.startsWith(name + ' '))
  assert(matches.length === 1, 'Exactly one required evidence marker: ' + name)
  return JSON.parse(matches[0].slice(name.length + 1))
}

export function parsePlatformAdminOutput(output) {
  const rust = parseRustResult(output, 1)
  const live = marker(output, 'PHASE25_LIVE_RESULT')
  const database = marker(output, 'PHASE25_LIVE_DATABASE')
  assert(live.schemaVersion === 1 && live.passed === platformAdminChecks.length && live.failed === 0 && live.skipped === 0,
    'Every mandatory CLI/platform check must execute without failures or skips')
  assert(Array.isArray(live.checks) && live.checks.length === platformAdminChecks.length &&
    platformAdminChecks.every(id => live.checks.filter(check => check.id === id && check.status === 'PASS').length === 1),
  'The exact closed list of CLI/platform checks is required')
  assert(database.bootstrapApplications === 1 && database.bootstrapMenus === 7 && database.urlReleases === 2 &&
    database.zipArtifacts === 0 && database.grantAudits === 3 && database.actualClientIdAssociated === true && database.grantWritesOnlyViaHttp === true,
  'Actual SQL evidence must cover bootstrap, URL releases, formal data grants, and audits')
  const cleanup = [...output.matchAll(/entity creation cleanup assertions_ok=(true|false) database_removed=(true|false) pool_closed=(true|false)/g)]
  assert(cleanup.length === 1 && cleanup[0].slice(1).every(value => value === 'true'), 'Owned PostgreSQL cleanup must succeed')
  return { rust, live, database, cleanup: { assertionsOk: true, databaseRemoved: true, poolClosed: true } }
}

export async function runPlatformAdmin({ root = process.cwd(), outDir, sourceRevision, env = process.env, execute = runCommand } = {}) {
  const ownership = validateOwnedPostgres(env)
  assert(path.isAbsolute(outDir || '') && !existsSync(outDir), 'Fresh absolute evidence directory required')
  validateOutputLocation(outDir, root)
  const binary = env.PHASE25_CLI_BIN
  assert(typeof binary === 'string' && path.isAbsolute(binary) && existsSync(binary) && statSync(binary).isFile(),
    'Build the standalone CLI first and provide its explicit absolute PHASE25_CLI_BIN')
  const source = createSourceManifest(root, { sourceRevision })
  assert(source.files.some(file => file.path === 'tools/platform-admin/src/lib.rs'), 'CLI source must be included in the input manifest')
  mkdirSync(outDir, { recursive: true })
  writeFileSync(path.join(outDir, 'source-manifest.json'), JSON.stringify(source, null, 2) + '\n', { mode: 0o600 })
  const childEnv = { ...commandEnvironment(env), TEST_DATABASE_URL: env.TEST_DATABASE_URL,
    CI_TEST_POSTGRES_OWNER: env.CI_TEST_POSTGRES_OWNER, PHASE25_CLI_BIN: binary,
    CARGO_TARGET_DIR: path.join(root, 'target/ci-rust') }
  const secrets = [env.TEST_DATABASE_URL, new URL(env.TEST_DATABASE_URL).password].filter(Boolean)
  const clean = value => redact(secrets.reduce((text, secret) => text.replaceAll(secret, '[REDACTED]'), String(value)))
  const args = ['test', '--manifest-path', 'backend/Cargo.toml', '--locked', '--bin', 'saas-platform-backend', '-j', '2',
    platformAdminTest, '--', '--exact', '--ignored', '--test-threads=1', '--nocapture']
  const report = { schemaVersion: 1, kind: 'real-cli-platform-owned-postgres', status: 'FAIL',
    startedAt: new Date().toISOString(), sourceRevision: source.sourceRevision, inputsSha256: source.inputsSha256,
    binarySha256: createHash('sha256').update(readFileSync(binary)).digest('hex'), ownership,
    command: ['cargo', ...args] }
  try {
    const result = await execute('cargo', args, { cwd: root, env: childEnv, timeout: 25 * 60_000 })
    const output = clean(result.output || '')
    writeFileSync(path.join(outDir, 'cargo.log'), output, { mode: 0o600 })
    report.exitCode = result.exitCode
    report.timedOut = result.timedOut
    Object.assign(report, parsePlatformAdminOutput(output))
    assert(result.exitCode === 0 && !result.timedOut, 'Actual Cargo CLI/platform fixture must succeed')
    report.status = 'PASS'
  } catch (error) { report.error = clean(error.message) }
  report.finishedAt = new Date().toISOString()
  writeFileSync(path.join(outDir, 'result.json'), JSON.stringify(report, null, 2) + '\n', { mode: 0o600 })
  return report
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const args = process.argv.slice(2)
  if (args.length !== 4 || args[0] !== '--out-dir' || args[2] !== '--source-revision' || !/^[a-f0-9]{40}$/.test(args[3])) {
    console.error('Usage: node scripts/ci/platform-admin.mjs --out-dir <fresh-absolute-directory> --source-revision <full-sha> (with explicit PHASE25_CLI_BIN)')
    process.exitCode = 2
  } else runPlatformAdmin({ outDir: args[1], sourceRevision: args[3] }).then(report => {
    console.log('LIVE_PLATFORM_ADMIN ' + report.status)
    if (report.status !== 'PASS') process.exitCode = 1
  }).catch(error => { console.error(redact(error.message)); process.exitCode = 1 })
}
