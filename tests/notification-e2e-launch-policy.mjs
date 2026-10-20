import assert from 'node:assert/strict'
import test from 'node:test'
import { chmodSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import vm from 'node:vm'

const workOrders = readFileSync(new URL('../business-apps/work-orders/tests/platform-e2e.mjs', import.meta.url), 'utf8')
const production = readFileSync(new URL('./production-native-e2e.mjs', import.meta.url), 'utf8')
// Execute the exact pure adapter without importing/registering either later
// opt-in acceptance suite or starting their databases, browsers and services.
const adapter = workOrders.match(/^export function notificationLaunchOptions\([^]*?^\}/m)?.[0]
const resolver = workOrders.match(/^export function resolveNotificationBinary\([^]*?^\}/m)?.[0]
assert.ok(adapter, 'The notification executable/environment adapter must remain directly testable')
assert.ok(resolver, 'Binary validation must precede creating owned runtime resources')
const bindings = { assert, path, realpathSync, lstatSync, process: { platform: process.platform } }
bindings.resolveNotificationBinary = vm.runInNewContext(`(${resolver.replace(/^export /, '')})`, bindings)
const launchOptions = vm.runInNewContext(`(${adapter.replace(/^export /, '')})`, {
  ...bindings,
})

function ownedFiles(t) {
  const root = mkdtempSync(path.join(os.tmpdir(), 'phase23-notification-launch-'))
  t.after(() => {
    assert.equal(path.dirname(root), os.tmpdir())
    assert.ok(path.basename(root).startsWith('phase23-notification-launch-'))
    rmSync(root, { recursive: true, force: true })
  })
  const binary = path.join(root, 'saas-notification-ws')
  writeFileSync(binary, 'Owned executable path fixture; no process is launched.\n', { mode: 0o700 })
  chmodSync(binary, 0o700)
  return { root, binary }
}
const input = binary => ({ binary, environment: { PATH: '/fixture/bin' }, databaseUrl: 'postgresql://fixture@localhost/owned',
  platformOrigin: 'http://[::1]:3001', allowedOrigin: 'http://[::1]:3000', port: 3003, production: false,
  metricsToken: 'FixtureOnlyMetricsToken0123456789AbCd' })

test('both opt-in suites require an explicit Rust notification executable and preserve owned restarts', () => {
  assert.match(workOrders, /NOTIFICATION_E2E_SERVICE_BINARY:\s*notificationBinaryArgument/)
  assert.ok(workOrders.indexOf('const notificationExecutable = resolveNotificationBinary(notificationBinaryArgument)') < workOrders.indexOf("const platformDb = await createDatabase"), 'invalid binaries must fail before databases are created')
  assert.match(production, /const required=\[[^\]]*'NOTIFICATION_E2E_SERVICE_BINARY'/)
  assert.match(workOrders, /launch\('notification',notificationBinary,notificationLaunch\.args,\{env:notificationEnvironment\}\)/)
  assert.match(production, /\['notification-health',h\.notificationProcess,h\.notificationBinary,\[\],h\.notificationEnvironment\]/)
  assert.match(production, /h\.launch\('notification',h\.notificationBinary,\[\],\{env:h\.notificationEnvironment\}\)/)
  for (const source of [workOrders, production]) {
    assert.doesNotMatch(source, /notification-ws\/(?:index\.ts|config\.mjs|postgres-store\.mjs|service\.mjs)/)
    assert.doesNotMatch(source, /(?:launch|h\.launch)\('notification',process\.execPath/)
  }
})

test('notification launch refuses missing, relative, absent, directory and non-executable paths', t => {
  const { root, binary } = ownedFiles(t)
  for (const value of [undefined, '', 'saas-notification-ws']) {
    assert.throws(() => launchOptions(input(value)), /NOTIFICATION_E2E_SERVICE_BINARY/)
  }
  assert.throws(() => launchOptions(input(path.join(root, 'missing'))))
  const directory = path.join(root, 'directory'); mkdirSync(directory)
  assert.throws(() => launchOptions(input(directory)), /regular executable file/)
  if (process.platform !== 'win32') {
    chmodSync(binary, 0o600)
    assert.throws(() => launchOptions(input(binary)), /must be executable/)
  }
})

test('the selected executable is retained while notification env drops ambient credentials and Node/TLS injection', t => {
  const { root, binary } = ownedFiles(t)
  const caFile = path.join(root, 'owned-ca.pem'); writeFileSync(caFile, 'Owned CA path fixture\n')
  const options = launchOptions({ ...input(binary), production: true, platformOrigin: 'https://platform.example.test',
    allowedOrigin: 'https://host.example.test', caFile, environment: { PATH: '/fixture/bin', LANG: 'C.UTF-8',
      JWT_SECRET: 'ambient-platform-key', DATABASE_URL: 'ambient-database-secret', METRICS_TOKEN: 'ambient-metrics-secret',
      NODE_OPTIONS: '--require malicious.cjs', NODE_TLS_REJECT_UNAUTHORIZED: '0', NODE_EXTRA_CA_CERTS: '/untrusted/ca.pem',
      HTTP_PROXY: 'http://user:password@proxy', HTTPS_PROXY: 'http://user:password@proxy', PLATFORM_CLIENT_SECRET: 'ambient-client-secret' } })
  assert.equal(options.executable, realpathSync(binary))
  assert.equal(options.args.length, 0)
  assert.equal(options.env.DATABASE_URL, input(binary).databaseUrl)
  assert.equal(options.env.METRICS_TOKEN, input(binary).metricsToken)
  assert.equal(options.env.NOTIFICATION_ENV, 'production')
  assert.equal(options.env.NOTIFICATION_PLATFORM_CA_FILE, realpathSync(caFile))
  assert.equal(options.env.NOTIFICATION_SHUTDOWN_ON_STDIN, '0')
  assert.equal(options.env.NOTIFICATION_HOST, '::1')
  assert.equal(options.env.NOTIFICATION_PORT, '3003')
  for (const key of ['JWT_SECRET', 'NODE_OPTIONS', 'NODE_TLS_REJECT_UNAUTHORIZED', 'NODE_EXTRA_CA_CERTS', 'HTTP_PROXY', 'HTTPS_PROXY', 'PLATFORM_CLIENT_SECRET']) {
    assert.equal(options.env[key], undefined, `${key} must not be inherited by the Rust notification process`)
  }
  const development = launchOptions(input(binary))
  assert.equal(development.env.NOTIFICATION_ENV, 'test')
  assert.equal(development.env.NOTIFICATION_PLATFORM_CA_FILE, undefined)
  assert.throws(() => launchOptions({ ...input(binary), caFile: 'relative.pem' }), /absolute file path/)
  assert.throws(() => launchOptions({ ...input(binary), caFile: root }), /regular file/)
})
