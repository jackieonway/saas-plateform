/** Opt-in Phase24 browser evidence entry point. Runs only the owned test fixture. */
import assert from 'node:assert/strict'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { createHash } from 'node:crypto'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { commandEnvironment, redact, runCommand } from './report.mjs'
import { validateOwnedPostgres, parseRustResult } from './postgres.mjs'
import { createSourceManifest } from './provenance.mjs'
import { validateOutputLocation } from './linux-runner.mjs'

export const oauthPlaygroundTest = 'oauth_playground_ui_tests::real_platform_four_flows_and_callback_browser'
export const oauthPlaygroundChecks = Object.freeze([
  'built-dist-no-private-material', 'pkce-real-consent-callback', 'state-wrong-session-replay',
  'authorization-userinfo-introspection-refresh-revoke', 'device-real-approval-and-poll',
  'public-device-introspection-explicitly-unsupported', 'client-credentials-controlled-service',
  'legacy-password-explicitly-enabled', 'ordinary-oauth-external-default-deny',
  'app-bound-external-current-user-positive-negative', 'origin-and-verifier-endpoint-rejected',
  'browser-storage-url-response-no-private-material',
])
const products = ['chromium', 'chrome', 'msedge', 'firefox', 'webkit']
function marker(output, name) {
  const matches = output.split(/\r?\n/).filter(line => line.startsWith(name + ' '))
  assert(matches.length === 1, 'Exactly one required evidence marker: ' + name)
  return JSON.parse(matches[0].slice(name.length + 1))
}
export function parseOauthPlaygroundOutput(output, product) {
  const rust = parseRustResult(output, 1)
  const browser = marker(output, 'PHASE24_LIVE_RESULT')
  const database = marker(output, 'PHASE24_LIVE_DATABASE')
  assert(browser.schemaVersion === 1 && browser.product === product && browser.playwrightVersion === '1.64.1', 'Browser identity evidence is required')
  assert(typeof browser.browserVersion === 'string' && /^\d+(?:\.\d+){1,3}$/.test(browser.browserVersion), 'Actual browser version is required')
  assert(browser.passed === oauthPlaygroundChecks.length && browser.failed === 0 && browser.skipped === 0, 'No missing/failed/skipped mandatory browser checks')
  assert(Array.isArray(browser.checks) && browser.checks.length === oauthPlaygroundChecks.length && oauthPlaygroundChecks.every(id => browser.checks.filter(check => check.id === id && check.status === 'PASS').length === 1), 'Every mandatory real-platform check must pass exactly once')
  assert(database.consentApprovals >= 1 && database.deviceApprovals === 1 && database.issuedClientCount === 4 && database.rawTokensPersisted === 0 && database.retainedPairsRevoked === true, 'Real database proof must cover each issued flow and revocation')
  const cleanup = [...output.matchAll(/entity creation cleanup assertions_ok=(true|false) database_removed=(true|false) pool_closed=(true|false)/g)]
  assert(cleanup.length === 1 && cleanup[0].slice(1).every(value => value === 'true'), 'Owned PostgreSQL cleanup must succeed')
  return { rust, browser, database, cleanup: { assertionsOk: true, databaseRemoved: true, poolClosed: true } }
}

export async function runOauthPlayground({ root = process.cwd(), outDir, product, sourceRevision, env = process.env, execute = runCommand } = {}) {
  const ownership = validateOwnedPostgres(env)
  assert(products.includes(product), 'Select an approved actual browser explicitly')
  assert(path.isAbsolute(outDir || '') && !existsSync(outDir), 'Fresh absolute evidence directory required')
  validateOutputLocation(outDir, root)
  mkdirSync(outDir, { recursive: true })
  const source = createSourceManifest(root, { sourceRevision })
  const screenshotPath = path.join(outDir, 'oauth-playground.png')
  const childEnv = { ...commandEnvironment(env), TEST_DATABASE_URL: env.TEST_DATABASE_URL,
    CI_TEST_POSTGRES_OWNER: env.CI_TEST_POSTGRES_OWNER, SAAS_TEST_BROWSER: product,
    PHASE24_SCREENSHOT_PATH: screenshotPath, CARGO_TARGET_DIR: path.join(root, 'target/ci-rust') }
  const secrets = [env.TEST_DATABASE_URL, new URL(env.TEST_DATABASE_URL).password].filter(Boolean)
  const clean = value => redact(secrets.reduce((text, secret) => text.replaceAll(secret, '[REDACTED]'), String(value)))
  const args = ['test', '--manifest-path', 'backend/Cargo.toml', '--locked', '--bin', 'saas-platform-backend', '-j', '2',
    oauthPlaygroundTest, '--', '--exact', '--ignored', '--test-threads=1', '--nocapture']
  const report = { schemaVersion: 1, kind: 'real-platform-postgres-oauth-playground', status: 'FAIL', product,
    startedAt: new Date().toISOString(), sourceRevision: source.sourceRevision, inputsSha256: source.inputsSha256, ownership }
  try {
    const result = await execute('cargo', args, { cwd: root, env: childEnv, timeout: 25 * 60_000 })
    const output = clean(result.output || '')
    writeFileSync(path.join(outDir, 'cargo.log'), output, { mode: 0o600 })
    report.command = ['cargo', ...args]
    report.exitCode = result.exitCode
    report.timedOut = result.timedOut
    Object.assign(report, parseOauthPlaygroundOutput(output, product))
    assert(result.exitCode === 0 && !result.timedOut, 'Actual Cargo browser fixture command must succeed')
    assert(report.browser.screenshot?.path === screenshotPath, 'Screenshot remains in owned evidence directory')
    const screenshot = readFileSync(screenshotPath)
    assert(screenshot.subarray(0, 8).toString('hex') === '89504e470d0a1a0a' && createHash('sha256').update(screenshot).digest('hex') === report.browser.screenshot.sha256, 'Actual screenshot bytes match the evidence')
    report.status = 'PASS'
  } catch (error) { report.status = 'FAIL'; report.error = clean(error.message) }
  report.finishedAt = new Date().toISOString()
  writeFileSync(path.join(outDir, 'result.json'), JSON.stringify(report, null, 2) + '\n', { mode: 0o600 })
  return report
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const args = process.argv.slice(2)
  if (args.length !== 6 || args[0] !== '--browser' || args[2] !== '--out-dir' || args[4] !== '--source-revision' || !/^[a-f0-9]{40}$/.test(args[5])) {
    console.error('Usage: node scripts/ci/oauth-playground.mjs --browser <approved-product> --out-dir <fresh-absolute-directory> --source-revision <full-sha>')
    process.exitCode = 2
  } else runOauthPlayground({ product: args[1], outDir: args[3], sourceRevision: args[5] }).then(report => {
    console.log('LIVE_OAUTH_PLAYGROUND ' + report.status)
    if (report.status !== 'PASS') process.exitCode = 1
  }).catch(error => { console.error(redact(error.message)); process.exitCode = 1 })
}
