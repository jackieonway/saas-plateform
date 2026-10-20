#!/usr/bin/env node
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createSourceManifest,sourceRoots } from './provenance.mjs'
import { commandEnvironment, evaluate, redact, runCommand } from './report.mjs'
import { gates, frontendPackages, contractTests, deploymentUnitTests, ciUnitTests, declaredLimits } from './manifest.mjs'
import { normalizeBrowserSourceOwnership } from './browser-source-ownership.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
export function parseArgs(args) {
  const options = { browser: 'chromium', platform: 'linux/amd64' }, seen = new Set()
  const names = { '--gate': 'gate', '--out-dir': 'outDir', '--source-revision': 'sourceRevision', '--browser': 'browser', '--platform': 'platform', '--build-proxy': 'buildProxy' }
  for (let i=0; i<args.length; i+=2) {
    const key = names[args[i]]
    if (!key || seen.has(key) || !args[i+1] || args[i+1].startsWith('--')) throw new Error('Unknown, duplicate or missing CI argument')
    seen.add(key); options[key] = args[i+1]
  }
  if (!gates.includes(options.gate) || !path.isAbsolute(options.outDir || '')) throw new Error('Usage: node scripts/ci/run.mjs --gate fast|postgres|browser|delivery|business-delivery --out-dir <new absolute directory> [--browser chromium|chrome|msedge] [--source-revision <full SHA>]')
  if (!['chromium','chrome','msedge','firefox','webkit'].includes(options.browser) || (!['chromium','chrome','msedge'].includes(options.browser) && options.gate!=='browser') || !/^linux\/(?:amd64|arm64)$/.test(options.platform)) throw new Error('Unsupported browser or image platform')
  if (options.gate === 'business-delivery' && options.platform !== 'linux/amd64') throw new Error('Business delivery currently requires the verified linux/amd64 target')
  if (options.buildProxy && !/^http:\/\/(?:127\.0\.0\.1|localhost):[0-9]{1,5}$/.test(options.buildProxy)) throw new Error('Build proxy must be explicit credential-free loopback HTTP')
  return options
}
function npmCli() {
  const found = [path.join(path.dirname(process.execPath),'node_modules/npm/bin/npm-cli.js'),path.resolve(path.dirname(process.execPath),'../lib/node_modules/npm/bin/npm-cli.js')].find(existsSync)
  if (!found) throw new Error('npm CLI must belong to the selected Node installation')
  return found
}
export async function runGate(options) {
  const out = path.resolve(options.outDir)
  if (existsSync(out) || out === root || root.startsWith(out+path.sep) || sourceRoots.some(entry=>out===path.join(root,entry)||out.startsWith(path.join(root,entry)+path.sep)) || !path.isAbsolute(options.outDir)) throw new Error('CI output must be a new absolute directory outside source inputs')
  const source = createSourceManifest(root, { sourceRevision: options.sourceRevision })
  mkdirSync(path.join(out, 'logs'), { recursive: true }); mkdirSync(path.join(out, 'work'), { recursive: true })
  const save = (name, value) => writeFileSync(path.join(out, name), JSON.stringify(value,null,2)+'\n', { mode: 0o600 })
  save('source-manifest.json',source)
  const summary = { schemaVersion: 1, gate: options.gate, browser: options.gate === 'browser' ? options.browser : undefined, status: 'RUNNING', startedAt: new Date().toISOString(), sourceRevision: source.sourceRevision, inputsSha256: source.inputsSha256, tools: { node: process.version, platform: process.platform, architecture: process.arch }, mandatoryFailureCount: 0, mandatorySkipCount: 0, steps: [], limits: declaredLimits }
  save('summary.json',summary)
  try {
  if (options.gate === 'browser') {
    summary.sourceOwnership = normalizeBrowserSourceOwnership({ root, sourceRevision: source.sourceRevision, expectedInputsSha256: source.inputsSha256 })
    save('summary.json',summary)
  }
  const env = commandEnvironment()
  const pathKey = process.platform === 'win32' && env.Path ? 'Path' : 'PATH'
  env[pathKey] = path.dirname(process.execPath) + path.delimiter + (env[pathKey] || '')
  for (const file of ['npmrc','npmrc-global']) writeFileSync(path.join(out,'work',file), file === 'npmrc' ? 'engine-strict=true\n' : '', { mode:0o600 })
  Object.assign(env, { NPM_CONFIG_USERCONFIG: path.join(out,'work/npmrc'), NPM_CONFIG_GLOBALCONFIG: path.join(out,'work/npmrc-global'), NPM_CONFIG_CACHE: path.join(out,'work/npm-cache') })
  const plan = [], npm = npmCli()
  env.npm_execpath=npm
  const step = (id, command, args, extra={}) => plan.push({ id,command,args,...extra })
  const install = directory => step(`install-${directory.replaceAll('/','-') || 'root'}`,process.execPath,[npm,...(directory ? ['--prefix',directory] : []),'ci','--ignore-scripts','--no-audit','--no-fund'],{ dependency:true })
  const tap = (id, files, extra={}) => step(id,process.execPath,['--test','--test-reporter=tap',...files],{ format:'tap',...extra })
  step('node-version',process.execPath,['--version'],{dependency:true,tool:'node',version:/^v24\.18\.0$/})
  step('npm-version',process.execPath,[npm,'--version'],{ dependency:true,tool:'npm',version:/^11\.16\.0$/ })
  step('rust-version','rustc',['--version'],{dependency:true,tool:'rust',version:/^rustc 1\.97\.0\b/})
  if (['fast','browser'].includes(options.gate)) { install(''); for (const directory of frontendPackages) install(directory) }
  if (options.gate === 'fast') {
    install('scripts/ci')
    step('sdk-drift',process.execPath,['scripts/sync-micro-sdk.mjs','--check'],{dependency:true})
    step('build-frontends',process.execPath,[npm,'run','build:all'],{dependency:true,env:{...env,WORK_ORDERS_ALLOWED_HOST_ORIGINS:'https://platform.example.com',STORES_ALLOWED_HOST_ORIGINS:'https://platform.example.com'}})
    step('lint',process.execPath,[npm,'--prefix','frontend','run','lint'])
    step('typecheck',process.execPath,[npm,'--prefix','frontend','run','typecheck'])
    tap('contracts',contractTests)
    for (const [id, manifest] of [['telemetry','crates/operational-telemetry/Cargo.toml'],['deployment-contract','crates/deployment-contract/Cargo.toml'],['deployment-agent','tools/deployment-agent/Cargo.toml'],['monitor','deploy/monitoring/Cargo.toml'],['recovery','deploy/recovery/Cargo.toml']]) {
      step(id+'-fmt','cargo',['fmt','--manifest-path',manifest,'--all','--','--check'])
      step(id+'-tests','cargo',['test','--locked','--manifest-path',manifest,'--all-targets','-j','2'],{format:'cargo'})
      step(id+'-clippy','cargo',['clippy','--locked','--manifest-path',manifest,'--all-targets','-j','2','--','-D','warnings'])
    }
    tap('work-orders-vite-contracts',['business-apps/work-orders/frontend/tests/build.mjs','business-apps/work-orders/frontend/tests/static-delivery.mjs','business-apps/work-orders/frontend/tests/lifecycle.mjs'])
    step('stores-frontend-contracts',process.execPath,[npm,'--prefix','business-apps/stores/frontend','test','--','--test-reporter=tap'],{format:'tap',timeout:10*60_000})
    tap('deployment-unit',deploymentUnitTests)
    tap('oauth-playground-frontend',['mini-services/oauth-playground/frontend/tests/device-poller.test.mjs','mini-services/oauth-playground/frontend/tests/api.test.mjs'])
    tap('oauth-playground-delivery',['mini-services/oauth-playground/tests/delivery.mjs'])
    tap('ci-policy',ciUnitTests)
    step('dev-and-vite',process.execPath,['--test','--test-reporter=tap','--test-concurrency=1','tests/dev-runner.mjs','tests/dev-vite-integration.mjs','tests/vite-example-delivery.mjs','tests/vite-host-static.mjs'],{format:'tap',timeout:10*60_000})
    step('rust-check','cargo',['check','--locked','--manifest-path','backend/Cargo.toml','--all-targets','-j','2'])
    step('rust-authorization-policy','cargo',['test','--locked','--manifest-path','backend/Cargo.toml','--test','authorization_policy','-j','2'],{format:'cargo'})
    step('rust-retention-policy','cargo',['test','--locked','--manifest-path','backend/Cargo.toml','--test','retention','-j','2'],{format:'cargo'})
    step('rust-zip-ownership-units','cargo',['test','--locked','--manifest-path','backend/Cargo.toml','--bin','saas-platform-backend','artifact_distribution::gates::tests','-j','2'],{format:'cargo'})
    step('rust-managed-url-units','cargo',['test','--locked','--manifest-path','backend/Cargo.toml','--bin','saas-platform-backend','managed_deployments::tests','-j','2'],{format:'cargo'})
    step('rust-config-units','cargo',['test','--locked','--manifest-path','backend/Cargo.toml','--bin','saas-platform-backend','config::tests','-j','2'],{format:'cargo'})
    step('rust-data-grant-units','cargo',['test','--locked','--manifest-path','backend/Cargo.toml','--bin','saas-platform-backend','data_grants::','-j','2'],{format:'cargo'})
    step('rust-app-member-units','cargo',['test','--locked','--manifest-path','backend/Cargo.toml','--bin','saas-platform-backend','routes::app_members::tests','-j','2'],{format:'cargo'})
    step('rust-archive-units','cargo',['test','--locked','--manifest-path','backend/Cargo.toml','--bin','saas-platform-backend','release_archives::','-j','2'],{format:'cargo'})
    step('rust-staging-maintenance-units','cargo',['test','--locked','--manifest-path','backend/Cargo.toml','--bin','saas-platform-backend','release_archive_maintenance::tests','-j','2'],{format:'cargo'})
    step('rust-public-contract-units','cargo',['test','--locked','--manifest-path','backend/Cargo.toml','--bin','saas-platform-backend','public_contract_tests','-j','2'],{format:'cargo'})
    step('rust-oauth-issuance-units','cargo',['test','--locked','--manifest-path','backend/Cargo.toml','--bin','saas-platform-backend','oauth_issuance::tests','-j','2'],{format:'cargo'})
    step('rust-login-limit-units','cargo',['test','--locked','--manifest-path','backend/Cargo.toml','--bin','saas-platform-backend','login_limits::tests','-j','2'],{format:'cargo'})
    step('rust-password-units','cargo',['test','--locked','--manifest-path','backend/Cargo.toml','--bin','saas-platform-backend','util::password::tests','-j','2'],{format:'cargo'})
    step('rust-client-ip-units','cargo',['test','--locked','--manifest-path','backend/Cargo.toml','--bin','saas-platform-backend','client_ip::tests','-j','2'],{format:'cargo'})
    step('rust-transport-units','cargo',['test','--locked','--manifest-path','tests/fixtures/gateway-rust/Cargo.toml','-j','2'],{format:'cargo'})
    step('rust-socketio-units','cargo',['test','--locked','--manifest-path','tests/fixtures/socketio-rust/Cargo.toml','--all-targets','--target-dir','target/phase22-socketio','-j','2'],{format:'cargo'})
    step('rust-chat-units','cargo',['test','--locked','--manifest-path','examples/websocket/backend/Cargo.toml','--all-targets','--target-dir','target/phase22-chat','-j','2'],{format:'cargo'})
    tap('rust-socketio-compatibility',['tests/phase22-socketio.mjs'],{timeout:10*60_000})
    const notificationManifest = 'mini-services/notification-ws/Cargo.toml'
    step('notification-fmt','cargo',['fmt','--manifest-path',notificationManifest,'--all','--','--check'])
    step('notification-check','cargo',['check','--locked','--manifest-path',notificationManifest,'--all-targets','--all-features','--target-dir','target/phase23-notification','-j','2'])
    step('notification-clippy','cargo',['clippy','--locked','--manifest-path',notificationManifest,'--all-targets','--all-features','--target-dir','target/phase23-notification','-j','2','--','-D','warnings'])
    step('notification-units','cargo',['test','--locked','--manifest-path',notificationManifest,'--all-targets','--all-features','--target-dir','target/phase23-notification','-j','2'],{format:'cargo'})
    step('notification-doc','cargo',['doc','--locked','--manifest-path',notificationManifest,'--no-deps','--target-dir','target/phase23-notification'])
    const workOrdersManifest = 'business-apps/work-orders/backend/Cargo.toml'
    step('work-orders-fmt','cargo',['fmt','--manifest-path',workOrdersManifest,'--all','--','--check'])
    step('work-orders-check','cargo',['check','--locked','--manifest-path',workOrdersManifest,'--all-targets','--all-features','-j','2'])
    step('work-orders-clippy','cargo',['clippy','--locked','--manifest-path',workOrdersManifest,'--all-targets','--all-features','-j','2','--','-D','warnings'])
    step('work-orders-units','cargo',['test','--locked','--manifest-path',workOrdersManifest,'--lib','-j','2'],{format:'cargo'})
    step('work-orders-doc','cargo',['doc','--locked','--manifest-path',workOrdersManifest,'--no-deps'])
    const storesManifest = 'business-apps/stores/backend/Cargo.toml'
    step('stores-fmt','cargo',['fmt','--manifest-path',storesManifest,'--all','--','--check'])
    step('stores-check','cargo',['check','--locked','--manifest-path',storesManifest,'--all-targets','--all-features','--target-dir','target/ci-rust','-j','2'])
    step('stores-clippy','cargo',['clippy','--locked','--manifest-path',storesManifest,'--all-targets','--all-features','--target-dir','target/ci-rust','-j','2','--','-D','warnings'])
    step('stores-units','cargo',['test','--locked','--manifest-path',storesManifest,'--lib','--all-features','--target-dir','target/ci-rust','-j','2'],{format:'cargo'})
    step('stores-doc','cargo',['doc','--locked','--manifest-path',storesManifest,'--no-deps','--all-features','--target-dir','target/ci-rust'])
    step('stores-build','cargo',['build','--locked','--manifest-path',storesManifest,'--bin','stores-api','--target-dir','target/ci-rust','-j','2'],{dependency:true})
    step('rust-oauth-playground-units','cargo',['test','--locked','--manifest-path','mini-services/oauth-playground/backend/Cargo.toml','--all-targets','-j','2'],{format:'cargo'})
    const adminManifest = 'tools/platform-admin/Cargo.toml'
    step('platform-admin-fmt','cargo',['fmt','--manifest-path',adminManifest,'--all','--','--check'])
    step('platform-admin-check','cargo',['check','--locked','--manifest-path',adminManifest,'--all-targets','--all-features','-j','2'])
    step('platform-admin-clippy','cargo',['clippy','--locked','--manifest-path',adminManifest,'--all-targets','--all-features','-j','2','--','-D','warnings'])
    // These cases fork CLI processes; isolate cases so an unrelated fork cannot
    // briefly inherit another case's held file descriptor during its drop check.
    // The multi-process competition cases still create their own concurrent writers.
    step('platform-admin-tests','cargo',['test','--locked','--manifest-path',adminManifest,'--all-targets','--all-features','-j','2','--','--test-threads=1'],{format:'cargo'})
    step('platform-admin-doc','cargo',['doc','--locked','--manifest-path',adminManifest,'--no-deps'])
    const adminTarget = 'target/phase25-platform-admin'
    step('platform-admin-build','cargo',['build','--locked','--manifest-path',adminManifest,'--bin','saas-platform-admin','--target-dir',adminTarget,'-j','2'],{dependency:true})
    const adminTlsEnv = { ...env, PHASE25_CLI_BIN:path.join(root,adminTarget,'debug',process.platform==='win32'?'saas-platform-admin.exe':'saas-platform-admin'), PHASE25_TLS_OUT:path.join(out,'platform-admin-tls') }
    tap('platform-admin-tls',['tests/platform-admin-tls.mjs'],{format:'platform-admin-tls',env:adminTlsEnv,report:path.join(out,'platform-admin-tls/result.json'),timeout:2*60_000})
  } else if (options.gate === 'postgres') {
    // Test driver only: the migration compatibility test invokes the preserved
    // Node initializer concurrently with Rust; the Rust runtime has no npm step.
    install('business-apps/work-orders/backend')
    const pgEnv = { ...env, CI_TEST_POSTGRES_OWNER:process.env.CI_TEST_POSTGRES_OWNER, TEST_DATABASE_URL:process.env.TEST_DATABASE_URL }
    step('phase30-current-protocol-postgres',process.execPath,['scripts/ci/phase30-postgres.mjs','--run-tests','--out-dir',path.join(out,'phase30-protocol')],{env:pgEnv,timeout:30*60_000})
    step('postgres',process.execPath,['scripts/ci/postgres.mjs','--run-tests','--out-dir',path.join(out,'postgres')],{format:'postgres',report:path.join(out,'postgres/postgres-result.json'),env:pgEnv,timeout:45*60_000})
    step('work-orders-postgres',process.execPath,['scripts/ci/work-orders-postgres.mjs','--run-tests','--out-dir',path.join(out,'work-orders-postgres')],{format:'work-orders-postgres',report:path.join(out,'work-orders-postgres/postgres-result.json'),env:pgEnv,timeout:25*60_000})
    step('work-orders-read-postgres',process.execPath,['scripts/ci/work-orders-read-postgres.mjs','--run-tests','--out-dir',path.join(out,'work-orders-read-postgres')],{format:'work-orders-read-postgres',report:path.join(out,'work-orders-read-postgres/postgres-result.json'),env:pgEnv,timeout:25*60_000})
    step('work-orders-write-postgres',process.execPath,['scripts/ci/work-orders-write-postgres.mjs','--run-tests','--out-dir',path.join(out,'work-orders-write-postgres')],{format:'work-orders-write-postgres',report:path.join(out,'work-orders-write-postgres/postgres-result.json'),env:pgEnv,timeout:25*60_000})
    step('work-orders-member-postgres',process.execPath,['scripts/ci/work-orders-member-postgres.mjs','--run-tests','--out-dir',path.join(out,'work-orders-member-postgres')],{format:'work-orders-member-postgres',report:path.join(out,'work-orders-member-postgres/postgres-result.json'),env:pgEnv,timeout:25*60_000})
    step('work-orders-reminder-postgres',process.execPath,['scripts/ci/work-orders-reminder-postgres.mjs','--run-tests','--out-dir',path.join(out,'work-orders-reminder-postgres')],{format:'work-orders-reminder-postgres',report:path.join(out,'work-orders-reminder-postgres/postgres-result.json'),env:pgEnv,timeout:25*60_000})
    step('stores-postgres',process.execPath,['scripts/ci/stores-postgres.mjs','--run-tests','--out-dir',path.join(out,'stores-postgres')],{format:'stores-postgres',report:path.join(out,'stores-postgres/postgres-result.json'),env:pgEnv,timeout:25*60_000})
    step('work-orders-live-build','cargo',['build','--locked','--manifest-path','business-apps/work-orders/backend/Cargo.toml','--bin','work-orders-api','--target-dir','target/ci-rust','-j','2'],{dependency:true})
    step('work-orders-live-postgres',process.execPath,['scripts/ci/work-orders-live-postgres.mjs','--run-tests','--out-dir',path.join(out,'work-orders-live-postgres')],{format:'work-orders-live-postgres',report:path.join(out,'work-orders-live-postgres/postgres-result.json'),env:pgEnv,timeout:25*60_000})
    step('work-orders-write-live-postgres',process.execPath,['scripts/ci/work-orders-write-live-postgres.mjs','--run-tests','--out-dir',path.join(out,'work-orders-write-live-postgres')],{format:'work-orders-write-live-postgres',report:path.join(out,'work-orders-write-live-postgres/postgres-result.json'),env:pgEnv,timeout:25*60_000})
    step('work-orders-member-live-postgres',process.execPath,['scripts/ci/work-orders-member-live-postgres.mjs','--run-tests','--out-dir',path.join(out,'work-orders-member-live-postgres')],{format:'work-orders-member-live-postgres',report:path.join(out,'work-orders-member-live-postgres/postgres-result.json'),env:pgEnv,timeout:25*60_000})
    step('work-orders-reminder-live-postgres',process.execPath,['scripts/ci/work-orders-reminder-live-postgres.mjs','--run-tests','--out-dir',path.join(out,'work-orders-reminder-live-postgres')],{format:'work-orders-reminder-live-postgres',report:path.join(out,'work-orders-reminder-live-postgres/postgres-result.json'),env:pgEnv,timeout:25*60_000})
    step('stores-live-build','cargo',['build','--locked','--manifest-path','business-apps/stores/backend/Cargo.toml','--bin','stores-api','--target-dir','target/ci-rust','-j','2'],{dependency:true})
    step('stores-live-postgres',process.execPath,['scripts/ci/stores-live-postgres.mjs','--run-tests','--out-dir',path.join(out,'stores-live-postgres')],{format:'stores-live-postgres',report:path.join(out,'stores-live-postgres/postgres-result.json'),env:pgEnv,timeout:30*60_000})
    step('monitoring-platform-build','cargo',['build','--locked','--manifest-path','backend/Cargo.toml','--bin','saas-platform-backend','--target-dir','target/ci-rust','-j','2'],{dependency:true})
    step('monitoring-build','cargo',['build','--locked','--manifest-path','deploy/monitoring/Cargo.toml','--target-dir','target/ci-rust','-j','2'],{dependency:true})
    step('monitoring-postgres',process.execPath,['scripts/ci/monitoring-postgres.mjs','--run-tests','--out-dir',path.join(out,'monitoring-postgres')],{format:'monitoring-postgres',report:path.join(out,'monitoring-postgres/postgres-result.json'),env:pgEnv,timeout:30*60_000})
    step('phase31-upgrade-retention-postgres',process.execPath,['scripts/ci/phase31-postgres.mjs','--run-tests','--out-dir',path.join(out,'phase31-upgrade-retention')],{env:pgEnv,timeout:30*60_000})
    step('notification-live',process.execPath,['scripts/ci/notification.mjs','--run-tests','--out-dir',path.join(out,'notification')],{format:'notification',report:path.join(out,'notification/notification-result.json'),env:pgEnv,timeout:60*60_000})
  } else if (options.gate === 'browser') {
    install('tests/browser')
    install('business-apps/work-orders/backend')
    step('sdk-drift',process.execPath,['scripts/sync-micro-sdk.mjs','--check'],{dependency:true})
    step('build-frontends',process.execPath,[npm,'run','build:all'],{dependency:true,env:{...env,WORK_ORDERS_ALLOWED_HOST_ORIGINS:'https://platform.example.com',STORES_ALLOWED_HOST_ORIGINS:'https://platform.example.com'}})
    const { mandatorySuites } = await import('../../tests/browser/manifest.mjs')
    step('browser',process.execPath,['tests/browser/run.mjs','--browser',options.browser,'--report',path.join(out,'browser-result.json')],{format:'browser',report:path.join(out,'browser-result.json'),expectedSuites:mandatorySuites.map(s=>s.id),timeout:90*60_000})
    // Only this harness receives the explicitly owned database environment.
    const liveEnv = { ...env, CI_TEST_POSTGRES_OWNER:process.env.CI_TEST_POSTGRES_OWNER, TEST_DATABASE_URL:process.env.TEST_DATABASE_URL }
    step('notification-browser-live',process.execPath,['scripts/ci/notification.mjs','--run-tests','--out-dir',path.join(out,'notification'),'--browser',options.browser],{format:'notification',report:path.join(out,'notification/notification-result.json'),expectedProduct:options.browser,env:liveEnv,timeout:60*60_000})
    step('data-grant-ui-live',process.execPath,['scripts/ci/data-grant-ui.mjs','--browser',options.browser,'--out-dir',path.join(out,'data-grant-ui'),'--source-revision',source.sourceRevision],{format:'data-grant-ui',report:path.join(out,'data-grant-ui/result.json'),expectedProduct:options.browser,env:liveEnv,timeout:30*60_000})
    step('work-orders-platform-build','cargo',['build','--locked','--manifest-path','backend/Cargo.toml','--bin','saas-platform-backend','--target-dir','target/ci-rust','-j','2'],{dependency:true})
    step('work-orders-business-build','cargo',['build','--locked','--manifest-path','business-apps/work-orders/backend/Cargo.toml','--bin','work-orders-api','--target-dir','target/ci-rust','-j','2'],{dependency:true})
    step('work-orders-browser-live',process.execPath,['scripts/ci/work-orders-browser.mjs','--browser',options.browser,'--out-dir',path.join(out,'work-orders'),'--source-revision',source.sourceRevision],{format:'work-orders-browser',report:path.join(out,'work-orders/result.json'),expectedProduct:options.browser,env:liveEnv,timeout:20*60_000})
    step('stores-business-build','cargo',['build','--locked','--manifest-path','business-apps/stores/backend/Cargo.toml','--bin','stores-api','--target-dir','target/ci-rust','-j','2'],{dependency:true})
    step('stores-platform-admin-build','cargo',['build','--locked','--manifest-path','tools/platform-admin/Cargo.toml','--bin','saas-platform-admin','--target-dir','target/phase25-platform-admin','-j','2'],{dependency:true})
    step('stores-browser-live',process.execPath,['scripts/ci/stores-browser.mjs','--browser',options.browser,'--out-dir',path.join(out,'stores'),'--source-revision',source.sourceRevision],{format:'stores-browser',report:path.join(out,'stores/result.json'),expectedProduct:options.browser,env:liveEnv,expectedSourceRevision:source.sourceRevision,timeout:40*60_000})
  } else if (options.gate === 'business-delivery') {
    const businessEnv = { ...env, ...(process.env.CI_TOOLCHAIN_IMAGE ? { CI_TOOLCHAIN_IMAGE:process.env.CI_TOOLCHAIN_IMAGE } : {}) }
    step('business-delivery',process.execPath,['scripts/ci/business-delivery.mjs','--out-dir',path.join(out,'business-delivery'),'--source-revision',source.sourceRevision,'--platform',options.platform,...(options.buildProxy?['--build-proxy',options.buildProxy]:[])],{format:'business-delivery',report:path.join(out,'business-delivery/result.json'),expectedSourceRevision:source.sourceRevision,expectedInputsSha256:source.inputsSha256,env:businessEnv,timeout:150*60_000})
  } else if (options.gate === 'delivery') {
    tap('compose-policy',['tests/production-native-config.mjs'])
    step('release',process.execPath,['scripts/ci/release.mjs','--out-dir',path.join(out,'release'),'--platform',options.platform,'--smoke',...(options.buildProxy?['--build-proxy',options.buildProxy]:[])],{format:'release',report:path.join(out,'release/release-result.json'),timeout:90*60_000,dependency:true})
    step('notification-image',process.execPath,['scripts/ci/notification-image.mjs','--out-dir',path.join(out,'notification-image'),'--source-revision',source.sourceRevision,...(options.buildProxy?['--build-proxy',options.buildProxy]:[])],{format:'notification-image',report:path.join(out,'notification-image/result.json'),expectedSourceRevision:source.sourceRevision,timeout:60*60_000})
    const { requiredCheckIds } = await import('../../tests/zip-release-e2e.mjs')
    step('zip-release',process.execPath,['tests/zip-release-e2e.mjs','--release-report',path.join(out,'release/release-result.json'),'--out-dir',path.join(out,'zip-release')],{format:'zip-release',expectedChecks:requiredCheckIds,report:path.join(out,'zip-release/result.json'),timeout:45*60_000})
  }
  let dependencyFailed = false
  try {
    for (const item of plan) {
      if (dependencyFailed) { summary.steps.push({id:item.id,mandatory:true,status:'NOT_RUN',reason:'prerequisite failed'}); continue }
      console.log(`CI ${options.gate}: ${item.id}`)
      const logfile = `logs/${String(summary.steps.length+1).padStart(2,'0')}-${item.id}.log`
      const result = await runCommand(item.command,item.args,{cwd:root,env:item.env||env,timeout:item.timeout,log:path.join(out,logfile)})
      let document, readError
      if (item.report) try { document=JSON.parse(readFileSync(item.report,'utf8')) } catch (error) { readError=error.message }
      const verdict = evaluate({...result,format:item.format,document,expectedSuites:item.expectedSuites,expectedChecks:item.expectedChecks,expectedProduct:item.expectedProduct,expectedSourceRevision:item.expectedSourceRevision,expectedInputsSha256:item.expectedInputsSha256})
      if(item.version && !item.version.test(result.output.trim())) {verdict.status='FAIL';verdict.reason='actual tool version does not match the pinned toolchain'}
      const entry = { id:item.id,mandatory:true,command:[path.basename(item.command),...item.args.map(a=>a===npm?'<selected-node-npm-cli>':a.startsWith(root)?path.relative(root,a):a)],exitCode:result.exitCode,timedOut:result.timedOut,...verdict,log:logfile,...(item.report?{report:path.relative(out,item.report)}:{}),...(readError?{reportError:redact(readError)}:{}) }
      if (item.tool && verdict.status==='PASS') summary.tools[item.tool]=result.output.trim()
      summary.steps.push(entry); summary.mandatoryFailureCount += verdict.status==='FAIL'?1:0; summary.mandatorySkipCount += verdict.counts.skip + verdict.counts.todo + verdict.counts.cancelled
      if (verdict.status==='FAIL' && item.dependency) dependencyFailed=true
      save('summary.json',summary)
      console.log(`CI ${options.gate}: ${item.id} ${verdict.status}`)
    }
    summary.status = summary.steps.length===plan.length && summary.steps.every(s=>s.status==='PASS') ? 'PASS' : 'FAIL'
  } catch (error) { summary.status='FAIL';summary.error=redact(error.message);summary.mandatoryFailureCount++ }
  finally { summary.finishedAt=new Date().toISOString();summary.stepStatusCounts=Object.fromEntries(['PASS','FAIL','SKIP','NOT_RUN'].map(status=>[status,summary.steps.filter(step=>step.status===status).length]));save('summary.json',summary) }
  } catch(error) { summary.status='FAIL';summary.error=redact(error.message);summary.mandatoryFailureCount++;summary.finishedAt=new Date().toISOString();summary.steps.push({id:'preflight',mandatory:true,status:'FAIL',reason:summary.error});save('summary.json',summary) }
  return summary
}
if (process.argv[1] && path.resolve(process.argv[1])===fileURLToPath(import.meta.url)) {
  try { const result=await runGate(parseArgs(process.argv.slice(2)));console.log(`CI_GATE ${result.gate} ${result.status}`);if(result.status!=='PASS')process.exitCode=1 }
  catch(error){console.error(redact(error.message));process.exitCode=1}
}
