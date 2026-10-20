import { mkdirSync, existsSync, readFileSync, writeFileSync } from 'node:fs'
import { createHash } from 'node:crypto'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { createSourceManifest } from './provenance.mjs'
import { commandEnvironment, redact, runCommand } from './report.mjs'
import { validateOwnedPostgres } from './postgres.mjs'
import { validateOutputLocation } from './linux-runner.mjs'
import { dataGrantUiProducts, dataGrantUiTest, parseDataGrantUiOutput } from './data-grant-ui-results.mjs'

export function liveUiEnvironment(source, product, root) {
  validateOwnedPostgres(source)
  if (!dataGrantUiProducts.includes(product)) throw new Error('Select an approved live browser product')
  return { ...commandEnvironment(source), TEST_DATABASE_URL: source.TEST_DATABASE_URL,
    CI_TEST_POSTGRES_OWNER: source.CI_TEST_POSTGRES_OWNER, SAAS_TEST_BROWSER: product,
    CARGO_TARGET_DIR: path.join(root, 'target/ci-rust') }
}

export async function runDataGrantUi({ root = process.cwd(), outDir, product, sourceRevision, env = process.env, execute = runCommand } = {}) {
  const childEnv = liveUiEnvironment(env, product, root)
  if (!path.isAbsolute(outDir || '') || existsSync(outDir)) throw new Error('Live browser evidence requires a fresh absolute directory')
  validateOutputLocation(outDir, root)
  mkdirSync(outDir, { recursive: true })
  const screenshotPath = path.join(outDir, 'management-preview.png')
  childEnv.PHASE15_SCREENSHOT_PATH = screenshotPath
  const source = createSourceManifest(root, { sourceRevision })
  const secrets = [env.TEST_DATABASE_URL, new URL(env.TEST_DATABASE_URL).password].filter(Boolean)
  const clean = value => redact(secrets.reduce((text, secret) => text.replaceAll(secret, '[REDACTED]'), String(value)))
  const report = { schemaVersion: 1, kind: 'real-platform-postgres-ui', status: 'FAIL', product,
    startedAt: new Date().toISOString(), sourceRevision: source.sourceRevision, inputsSha256: source.inputsSha256,
    ownership: validateOwnedPostgres(env) }
  const args = ['test', '--manifest-path', 'backend/Cargo.toml', '--locked', '--bin', 'saas-platform-backend', '-j', '2',
    dataGrantUiTest, '--', '--exact', '--ignored', '--test-threads=1', '--nocapture']
  try {
    const result = await execute('cargo', args, { cwd: root, env: childEnv, timeout: 25 * 60_000 })
    const text = clean(result.output || '')
    writeFileSync(path.join(outDir, 'cargo.log'), text, { mode: 0o600 })
    report.command = ['cargo', ...args]
    report.exitCode = result.exitCode
    report.timedOut = result.timedOut
    const parsed = parseDataGrantUiOutput(text, { product })
    if (parsed.browser.screenshot.path !== screenshotPath) throw new Error('Preview screenshot must remain in the owned evidence directory')
    const screenshot = readFileSync(screenshotPath)
    if (screenshot.subarray(0, 8).toString('hex') !== '89504e470d0a1a0a' || createHash('sha256').update(screenshot).digest('hex') !== parsed.browser.screenshot.sha256) throw new Error('Preview PNG bytes differ from the browser evidence')
    Object.assign(report, parsed)
    if (result.exitCode !== 0 || result.timedOut) throw new Error('Live browser Cargo command failed or timed out')
  } catch (error) { report.status = 'FAIL'; report.error = clean(error.message) }
  report.finishedAt = new Date().toISOString()
  writeFileSync(path.join(outDir, 'result.json'), JSON.stringify(report, null, 2) + '\n', { mode: 0o600 })
  return report
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const args = process.argv.slice(2)
  if (args.length !== 6 || args[0] !== '--browser' || args[2] !== '--out-dir' || args[4] !== '--source-revision' || !/^[a-f0-9]{40}$/.test(args[5])) {
    console.error('Usage: node scripts/ci/data-grant-ui.mjs --browser <approved-product> --out-dir <fresh-absolute-directory> --source-revision <full-sha>')
    process.exitCode = 2
  } else runDataGrantUi({ product: args[1], outDir: args[3], sourceRevision: args[5] }).then(report => {
    console.log(`LIVE_DATA_GRANT_UI ${report.status}`)
    if (report.status !== 'PASS') process.exitCode = 1
  }).catch(error => { console.error(redact(error.message)); process.exitCode = 1 })
}
