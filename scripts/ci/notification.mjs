import { existsSync, mkdirSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { commandEnvironment, evaluate, runCommand } from './report.mjs'
import { validateOwnedPostgres } from './postgres.mjs'

// Real integration entrypoint. Missing configuration, zero tests and skips fail.
export async function runNotificationIntegration({ root = process.cwd(), outDir, env = process.env,
  browser, run = runCommand } = {}) {
  const ownership = validateOwnedPostgres(env)
  if (!path.isAbsolute(outDir || '') || existsSync(outDir)) throw new Error('Notification report output must be a fresh absolute directory')
  if (browser && !['chromium', 'chrome', 'msedge'].includes(browser)) throw new Error('Unknown required notification browser')
  mkdirSync(outDir, { recursive: true })
  const target = path.join(root, 'target', 'ci-rust-notification')
  const extension = process.platform === 'win32' ? '.exe' : ''
  const childEnv = { ...commandEnvironment(env), TEST_DATABASE_URL: env.TEST_DATABASE_URL,
    CI_TEST_POSTGRES_OWNER: env.CI_TEST_POSTGRES_OWNER,
    NOTIFICATION_E2E_BACKEND_BINARY: path.join(target, 'debug', 'saas-platform-backend' + extension),
    NOTIFICATION_E2E_SERVICE_BINARY: path.join(target, 'debug', 'saas-notification-ws' + extension),
    NOTIFICATION_E2E_PROXY_BINARY: path.join(target, 'debug', 'phase23-notification-platform-proxy' + extension),
    NOTIFICATION_TEST_ARTIFACTS_DIR: path.join(outDir, 'service-logs'),
    ...(browser ? { SAAS_TEST_BROWSER: browser, NOTIFICATION_BROWSER_ARTIFACTS_DIR: path.join(outDir, 'browser-artifacts') } : {}),
  }
  const report = { schemaVersion: 1, kind: 'notification-real-platform-postgres', ownership,
    browser, startedAt: new Date().toISOString(), status: 'FAIL', steps: [] }
  const save = () => writeFileSync(path.join(outDir, 'notification-result.json'), JSON.stringify(report, null, 2) + '\n')
  const step = async (name, command, args, format = 'command', timeout = 20 * 60_000) => {
    const result = await run(command, args, { cwd: root, env: childEnv, timeout, log: path.join(outDir, name + '.log') })
    const evaluated = evaluate({ format, exitCode: result.exitCode, output: result.output, timedOut: result.timedOut })
    report.steps.push({ name, command: [command, ...args], exitCode: result.exitCode, timedOut: result.timedOut, ...evaluated })
    save()
    if (evaluated.status !== 'PASS') throw new Error(`Required Rust notification step failed: ${name}`)
  }
  save()
  try {
    await step('npm-notification', 'npm', ['ci', '--prefix', 'mini-services/notification-ws', '--ignore-scripts', '--no-audit', '--no-fund'])
    for (const [name, manifest] of [['platform', 'backend/Cargo.toml'],
      ['notification', 'mini-services/notification-ws/Cargo.toml'], ['proxy', 'tests/fixtures/notification-platform-proxy/Cargo.toml']]) {
      await step('build-' + name, 'cargo', ['build', '--locked', '--manifest-path', manifest, '--target-dir', target, '-j', '2'])
    }
    await step('real-platform-scheduler', process.execPath, ['--test', '--test-reporter=tap', '--test-concurrency=1',
      'mini-services/notification-ws/tests/rust-platform-e2e.mjs', 'mini-services/notification-ws/tests/rust-scheduler.mjs',
      'mini-services/notification-ws/tests/rust-boundaries.mjs', 'mini-services/notification-ws/tests/rust-auth-races.mjs'], 'tap', 9 * 60_000)
    if (browser) {
      for (const directory of ['frontend', 'tests/browser']) {
        await step('npm-' + directory.replace('/', '-'), 'npm', ['ci', '--prefix', directory, '--ignore-scripts', '--no-audit', '--no-fund'])
      }
      await step('two-page-browser-' + browser, process.execPath, ['--test', '--test-reporter=tap',
        'mini-services/notification-ws/tests/rust-browser.mjs'], 'tap', 6 * 60_000)
    }
    report.status = 'PASS'
  } catch (error) { report.error = error.message }
  finally { report.finishedAt = new Date().toISOString(); save() }
  if (report.status !== 'PASS') throw new Error(report.error || 'Rust notification integration failed')
  return report
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const args = process.argv.slice(2)
  try {
    if (args[0] !== '--run-tests' || args[1] !== '--out-dir' || !args[2] ||
      ![3, 5].includes(args.length) || (args.length === 5 && args[3] !== '--browser')) {
      throw new Error('Usage: node scripts/ci/notification.mjs --run-tests --out-dir <fresh absolute directory> [--browser chromium|chrome|msedge]')
    }
    await runNotificationIntegration({ outDir: path.resolve(args[2]), browser: args[4] })
  } catch (error) { console.error(error.message); process.exitCode = 1 }
}
