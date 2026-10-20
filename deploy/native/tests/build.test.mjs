import assert from 'node:assert/strict'
import test from 'node:test'
import { spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { buildEnvironment, buildRelease, createSourceSnapshot, excludedSource, parseArgs } from '../build.mjs'

function fixture(t) {
  const owned = mkdtempSync(path.join(os.tmpdir(), 'platform-native-build-test-'))
  t.after(() => { assert.equal(path.dirname(owned), os.tmpdir()); assert.match(path.basename(owned), /^platform-native-build-test-/); rmSync(owned, { recursive: true, force: true }) })
  const sourceRoot = path.join(owned, 'checkout')
  mkdirSync(sourceRoot)
  const put = (name, value = 'fixture') => { const target = path.join(sourceRoot, name); mkdirSync(path.dirname(target), { recursive: true }); writeFileSync(target, value) }
  put('.nvmrc', '24.18.0\n'); put('rust-toolchain.toml', '[toolchain]\nchannel="1.97.0"\n')
  put('backend/Cargo.toml', '[package]\nname="saas-platform-backend"\nversion="1.0.0"\nedition="2021"\n\n[dependencies]\noperational-telemetry = { path = "../crates/operational-telemetry" }\n\n[dev-dependencies]\noauth-playground-api = { path = "../mini-services/oauth-playground/backend" }\n')
  put('backend/Cargo.lock', 'version = 4\n\n[[package]]\nname = "oauth-playground-api"\nversion = "1.0.0"\n\n[[package]]\nname = "operational-telemetry"\nversion = "0.1.0"\n\n[[package]]\nname = "saas-platform-backend"\nversion = "1.0.0"\ndependencies = ["oauth-playground-api", "operational-telemetry"]\n')
  put('crates/operational-telemetry/Cargo.toml', '[package]\nname="operational-telemetry"\nversion="0.1.0"\nedition="2021"\n')
  put('crates/operational-telemetry/Cargo.lock', 'version = 4\n\n[[package]]\nname = "operational-telemetry"\nversion = "0.1.0"\n')
  put('crates/operational-telemetry/src/lib.rs', 'pub fn record() {}\n')
  put('crates/deployment-contract/Cargo.toml', '[package]\nname="deployment-contract"\nversion="0.1.0"\nedition="2024"\n')
  put('crates/deployment-contract/Cargo.lock', 'version = 4\n\n[[package]]\nname = "deployment-contract"\nversion = "0.1.0"\n')
  put('crates/deployment-contract/src/lib.rs', 'pub fn protocol() {}\n')
  put('backend/src/main.rs', 'fn main() {}\n')
  put('mini-services/oauth-playground/backend/Cargo.toml', '[package]\nname="oauth-playground-api"\nversion="1.0.0"\nedition="2021"\n\n[lib]\nname="oauth_playground"\n')
  put('mini-services/oauth-playground/backend/Cargo.lock', 'version = 4\n\n[[package]]\nname = "oauth-playground-api"\nversion = "1.0.0"\n')
  put('mini-services/oauth-playground/backend/src/lib.rs', 'pub fn fixture() {}\n')
  put('mini-services/oauth-playground/backend/src/main.rs', 'fn main() {}\n')
  for (const file of ['deploy/static/finalize-build.mjs', 'deploy/gateway/policy.mjs', 'deploy/native/render.mjs']) put(file, '// controlled source fixture\n')
  for (const file of ['scripts/sync-micro-sdk.mjs', 'examples/micro-app-sdk/generated/protocol.mjs', 'examples/micro-app-sdk/generated/types.d.ts', 'business-apps/work-orders/frontend/vendor/iframe-bridge.js', 'business-apps/work-orders/frontend/vendor/micro-sdk.mjs']) put(file, '// controlled SDK source fixture\n')
  put('examples/micro-app-sdk/release.json', '{"sdkVersion":"1.0.0"}\n')
  put('deploy/README.md', '# Versioned deployment instructions\n')
  put('deploy/compose.zip.yml', 'services: {}\n')
  put('deploy/compose.env.example', 'PLATFORM_DATABASE_URL=\nJWT_SECRET=\n')
  put('deploy/compose.legacy.env.example', 'WORK_ORDERS_DATABASE_URL=\nWORK_ORDERS_PLATFORM_CLIENT_SECRET=\n')
  put('frontend/.npmrc', 'engine-strict=true\n')
  for (const name of ['frontend', 'mini-services/micro-vue', 'mini-services/micro-jquery', 'examples/websocket']) {
    put(`${name}/package.json`, '{"scripts":{"build":"fixture"}}\n'); put(`${name}/package-lock.json`, '{"lockfileVersion":3,"packages":{}}\n'); put(`${name}/index.html`, '<html>source</html>')
  }
  const git = args => { const result = spawnSync('git', ['-c', `safe.directory=${sourceRoot}`, '-C', sourceRoot, ...args], { encoding: 'utf8', windowsHide: true }); assert.equal(result.status, 0, result.stderr) }
  git(['init', '--quiet']); git(['add', '.']); git(['-c', 'user.name=Build fixture', '-c', 'user.email=fixture@example.test', 'commit', '--quiet', '-m', 'fixture'])
  return { owned, sourceRoot, put, outDir: path.join(owned, 'release') }
}

function compiler(calls, { fail, missingBinary = false } = {}) {
  return (command, args, options) => {
    calls.push({ command, args, ...options })
    if (args.at(-1) === '--version') return { status: 0, stdout: args.length === 1 ? (path.basename(command).startsWith('cargo') ? 'cargo 1.97.0 (fixture)\n' : 'v24.18.0\n') : '11.16.0\n' }
    if (args.includes(fail)) return { status: 1, stderr: 'controlled compiler failure\n' }
    if (args.includes('run') && args.includes('build')) { mkdirSync(path.join(options.cwd, 'dist'), { recursive: true }); writeFileSync(path.join(options.cwd, 'dist/index.html'), '<html>controlled fixture output</html>') }
    if (args.includes('--release') && !missingBinary) {
      const target = path.join(options.env.CARGO_TARGET_DIR, 'release', `saas-platform-backend${process.platform === 'win32' ? '.exe' : ''}`)
      mkdirSync(path.dirname(target), { recursive: true }); writeFileSync(target, 'controlled fake binary, not real build evidence')
    }
    return { status: 0, stdout: 'controlled compiler fixture\n' }
  }
}

test('CLI refuses missing, duplicate, relative and unknown parameters', () => {
  for (const args of [[], ['--out-dir'], ['--out-dir', 'relative'], ['--out-dir', os.tmpdir(), '--cargo'], ['--out-dir', os.tmpdir(), '--out-dir', os.tmpdir()], ['--mystery', 'value'], ['--out-dir', os.tmpdir(), '--source-root', 'relative']]) assert.throws(() => parseArgs(args))
  assert.equal(parseArgs(['--out-dir', os.tmpdir(), '--cargo', 'cargo']).cargo, 'cargo')
})

test('snapshot captures current edits and new allowed source, excludes private and generated data by name', t => {
  const h = fixture(t)
  h.put('frontend/src/current.ts', 'new uncommitted implementation\n')
  h.put('frontend/src/components/micro/runtime/index.ts', 'export const runtime = true\n')
  h.put('backend/src/runtime.rs', 'pub fn runtime() {}\n')
  h.put('examples/micro-app-sdk/src/context-client.mjs', 'export const currentSDK = true\n')
  h.put('business-apps/work-orders/frontend/vendor/micro-sdk.mjs', 'export const registeredVersion = "1.0.0"\n')
  h.put('business-apps/stores/frontend/vendor/micro-sdk/index.mjs', 'export const registeredVersion = "1.0.0"\n')
  h.put('business-apps/stores/frontend/vendor/micro-sdk/index.d.mts', 'export declare const registeredVersion: "1.0.0"\n')
  h.put('business-apps/stores/frontend/src/unrelated.ts', 'DO_NOT_COPY_UNRELATED_STORES_SOURCE')
  h.put('business-apps/work-orders/frontend/src/unrelated.js', 'DO_NOT_COPY_UNRELATED_WORK_ORDERS_SOURCE')
  h.put('mini-services/oauth-playground/backend/src/.env', 'DO_NOT_COPY_PRIVATE_OAUTH_ENV')
  h.put('mini-services/oauth-playground/frontend/src/unrelated.js', 'DO_NOT_COPY_OAUTH_FRONTEND')
  h.put('mini-services/notification-ws/src/unrelated.rs', 'DO_NOT_COPY_UNRELATED_SERVICE')
  h.put('backend/src/main.rs', 'fn main() { println!("working tree"); }\n')
  for (const file of ['backend/.env', 'frontend/.env.production', 'frontend/public/.env', 'frontend/public/config.env', 'frontend/public/local.env.production', 'frontend/public/.npmrc', 'deploy/compose.env', 'deploy/runtime.env', 'frontend/public/certificate.pem', 'frontend/public/secrets.json', 'frontend/public/runtime.json', 'frontend/public/private/key.txt', 'frontend/public/sub-apps/old/index.html', 'backend/target/debug/old.exe', 'frontend/node_modules/package/index.js', 'frontend/dist/old.js']) h.put(file, 'DO_NOT_READ_OR_PUBLISH')
  const source = path.join(h.owned, 'snapshot'), input = createSourceSnapshot(h.sourceRoot, source)
  assert.equal(readFileSync(path.join(source, 'frontend/src/current.ts'), 'utf8'), 'new uncommitted implementation\n')
  assert.match(readFileSync(path.join(source, 'backend/src/main.rs'), 'utf8'), /working tree/)
  assert.match(readFileSync(path.join(source, 'frontend/src/components/micro/runtime/index.ts'), 'utf8'), /runtime = true/)
  assert.match(readFileSync(path.join(source, 'backend/src/runtime.rs'), 'utf8'), /pub fn runtime/)
  assert.match(readFileSync(path.join(source, 'examples/micro-app-sdk/src/context-client.mjs'), 'utf8'), /currentSDK/)
  assert.match(readFileSync(path.join(source, 'business-apps/work-orders/frontend/vendor/micro-sdk.mjs'), 'utf8'), /1\.0\.0/)
  assert.match(readFileSync(path.join(source, 'business-apps/stores/frontend/vendor/micro-sdk/index.mjs'), 'utf8'), /1\.0\.0/)
  assert.match(readFileSync(path.join(source, 'business-apps/stores/frontend/vendor/micro-sdk/index.d.mts'), 'utf8'), /1\.0\.0/)
  assert.ok(!existsSync(path.join(source, 'business-apps/stores/frontend/src/unrelated.ts')))
  assert.ok(!existsSync(path.join(source, 'business-apps/work-orders/frontend/src/unrelated.js')))
  assert.match(readFileSync(path.join(source, 'mini-services/oauth-playground/backend/Cargo.toml'), 'utf8'), /name="oauth-playground-api"/)
  assert.ok(existsSync(path.join(source, 'mini-services/oauth-playground/backend/Cargo.lock')))
  assert.ok(existsSync(path.join(source, 'mini-services/oauth-playground/backend/src/lib.rs')))
  assert.ok(existsSync(path.join(source, 'mini-services/oauth-playground/backend/src/main.rs')))
  assert.ok(!existsSync(path.join(source, 'mini-services/oauth-playground/backend/src/.env')))
  assert.ok(!existsSync(path.join(source, 'mini-services/oauth-playground/frontend')))
  assert.ok(!existsSync(path.join(source, 'mini-services/notification-ws')))
  assert.ok(!existsSync(path.join(source, 'frontend/public/runtime.json')))
  assert.equal(readFileSync(path.join(source, 'deploy/compose.env.example'), 'utf8'), 'PLATFORM_DATABASE_URL=\nJWT_SECRET=\n')
  assert.equal(readFileSync(path.join(source, 'deploy/compose.legacy.env.example'), 'utf8'), 'WORK_ORDERS_DATABASE_URL=\nWORK_ORDERS_PLATFORM_CLIENT_SECRET=\n')
  assert.equal(readFileSync(path.join(source, 'frontend/.npmrc'), 'utf8'), 'engine-strict=true\n')
  assert.ok(!existsSync(path.join(source, 'deploy/compose.env')))
  assert.ok(!existsSync(path.join(source, 'frontend/public/config.env')))
  assert.ok(!existsSync(path.join(source, 'frontend/public/.npmrc')))
  assert.ok(input.files.every(file => !excludedSource(file.path)))
  assert.ok(!existsSync(path.join(source, 'frontend/public/sub-apps')))
  const same = createSourceSnapshot(h.sourceRoot, path.join(h.owned, 'same'))
  assert.equal(same.inputsSha256, input.inputsSha256)
  h.put('frontend/src/current.ts', 'changed again\n')
  assert.notEqual(createSourceSnapshot(h.sourceRoot, path.join(h.owned, 'changed')).inputsSha256, input.inputsSha256)
})

test('real offline Cargo resolves the snapshotted path dev-dependency and rejects a missing dependency tree', t => {
  const h = fixture(t)
  for (const missing of ['src', 'Cargo.toml', '.']) {
    const snapshot = path.join(h.owned, `cargo-snapshot-${missing === '.' ? 'directory' : missing.replaceAll('.', '-')}`)
    createSourceSnapshot(h.sourceRoot, snapshot)
    const metadata = () => spawnSync('cargo', ['metadata', '--format-version', '1', '--manifest-path', path.join(snapshot, 'backend/Cargo.toml'), '--locked', '--offline'],
      { cwd: snapshot, encoding: 'utf8', windowsHide: true, timeout: 60000 })
    const valid = metadata()
    assert.equal(valid.status, 0, String(valid.stderr || valid.error || 'Cargo metadata failed'))
    const packages = JSON.parse(valid.stdout).packages
    const platform = packages.find(item => item.name === 'saas-platform-backend')
    assert.ok(platform.dependencies.some(item => item.name === 'oauth-playground-api' && item.kind === 'dev'))
    assert.equal(packages.find(item => item.name === 'oauth-playground-api').manifest_path, path.join(snapshot, 'mini-services/oauth-playground/backend/Cargo.toml'))
    assert.equal(packages.find(item => item.name === 'operational-telemetry').manifest_path, path.join(snapshot, 'crates/operational-telemetry/Cargo.toml'))
    const dependency = path.join(snapshot, 'mini-services/oauth-playground/backend')
    const removed = path.resolve(dependency, missing)
    assert.ok(removed === dependency || removed.startsWith(dependency + path.sep), 'deletion stays inside the owned dependency fixture')
    assert.ok(dependency.startsWith(snapshot + path.sep), 'dependency stays inside the owned snapshot')
    rmSync(removed, { recursive: true, force: false })
    const invalid = metadata()
    assert.notEqual(invalid.status, 0, `Cargo must reject the missing dependency ${missing}`)
    assert.match(invalid.stderr, /oauth-playground|lib\.rs|no targets specified/i)
  }
})

test('snapshot requires each path dependency manifest, lock and Rust entry point before tools run', t => {
  for (const missing of ['Cargo.toml', 'Cargo.lock', 'src/lib.rs', 'src/main.rs']) {
    const h = fixture(t), calls = []
    rmSync(path.join(h.sourceRoot, 'mini-services/oauth-playground/backend', missing))
    assert.throws(() => buildRelease(h, { runCommand: compiler(calls) }), /Required build input is missing: mini-services\/oauth-playground\/backend\//)
    assert.equal(calls.length, 0)
  }
})

test('builder never passes ambient secrets, frontend public overrides or injection options to tools', () => {
  const env = buildEnvironment(os.tmpdir(), { PATH: '/tools', DATABASE_URL: 'postgres://private', JWT_SECRET: 'private', VITE_API_KEY: 'private', MICRO_PUBLIC_SECRET: 'private', NODE_OPTIONS: '--require private', NPM_TOKEN: 'private' })
  for (const key of ['DATABASE_URL', 'JWT_SECRET', 'VITE_API_KEY', 'MICRO_PUBLIC_SECRET', 'NODE_OPTIONS', 'NPM_TOKEN']) assert.equal(env[key], undefined)
  assert.ok(env.NPM_CONFIG_USERCONFIG.endsWith(path.join('work', 'npmrc')))
})

test('controlled compiler proves locked install order, traceable artifacts and no Node service bundle', t => {
  const h = fixture(t), calls = []
  const result = buildRelease(h, { runCommand: compiler(calls) })
  assert.equal(result.status, 'SUCCEEDED'); assert.equal(result.nodeRuntimePackaged, false)
  assert.match(result.sourceSha, /^[a-f0-9]{40,64}$/); assert.match(result.buildId, /^[a-f0-9]{12}-[a-f0-9]{16}$/)
  assert.equal(calls.filter(call => call.args.includes('ci')).length, 4)
  assert.ok(calls.filter(call => call.args.includes('ci')).every(call => call.args.includes('--ignore-scripts')))
  const drift = calls.findIndex(call => call.args.includes('--check'))
  assert.ok(drift > calls.findLastIndex(call => call.args.includes('ci')), 'SDK check uses locked frontend build dependencies')
  assert.ok(drift < calls.findIndex(call => call.args.includes('run') && call.args.includes('build')), 'SDK drift fails before any frontend is built')
  assert.deepEqual(calls[drift].args, [path.join(h.outDir, 'source/scripts/sync-micro-sdk.mjs'), '--check'])
  assert.ok(calls.find(call => call.args.includes('--release')).args.includes('--locked'))
  assert.deepEqual(result.artifacts.map(file => file.path).sort(), ['deploy/README.md', 'deploy/compose.zip.yml', 'deploy/compose.env.example', 'deploy/compose.legacy.env.example', 'deploy/gateway/policy.mjs', 'deploy/native/render.mjs', 'deploy/static/finalize-build.mjs', 'examples/jquery/index.html', 'examples/vue/index.html', 'examples/websocket/index.html', 'frontend/index.html', `platform/saas-platform-backend${process.platform === 'win32' ? '.exe' : ''}`].sort())
  assert.equal(JSON.parse(readFileSync(path.join(h.outDir, 'source-manifest.json'))).inputsSha256, result.inputsSha256)
  const before = readdirSync(h.outDir)
  assert.throws(() => buildRelease(h, { runCommand: compiler([]) }), /already exists/)
  assert.deepEqual(readdirSync(h.outDir), before)
})

test('failed compiler cannot publish an artifacts directory or success manifest', t => {
  const h = fixture(t)
  assert.throws(() => buildRelease(h, { runCommand: compiler([], { fail: 'ci' }) }), /Build step install-/)
  assert.equal(JSON.parse(readFileSync(path.join(h.outDir, 'build-result.json'))).status, 'FAILED')
  assert.ok(!existsSync(path.join(h.outDir, 'artifacts')))
})

test('SDK drift refuses before any frontend build and does not publish artifacts', t => {
  const h = fixture(t), calls = []
  assert.throws(() => buildRelease(h, { runCommand: compiler(calls, { fail: '--check' }) }), /Build step sdk-drift failed/)
  assert.equal(calls.filter(call => call.args.includes('ci')).length, 4)
  assert.ok(!calls.some(call => call.args.includes('run') && call.args.includes('build')))
  assert.ok(!calls.some(call => call.args.includes('--release')))
  assert.equal(JSON.parse(readFileSync(path.join(h.outDir, 'build-result.json'))).status, 'FAILED')
  assert.ok(!existsSync(path.join(h.outDir, 'artifacts')))
})

test('missing release binary fails even when compiler reports success', t => {
  const h = fixture(t)
  assert.throws(() => buildRelease(h, { runCommand: compiler([], { missingBinary: true }) }), /Rust release binary/)
  assert.equal(JSON.parse(readFileSync(path.join(h.outDir, 'build-result.json'))).status, 'FAILED')
  assert.ok(!existsSync(path.join(h.outDir, 'artifacts')))
})

test('missing lock refuses before any install or compiler executes', t => {
  const h = fixture(t), calls = []
  rmSync(path.join(h.sourceRoot, 'frontend/package-lock.json'))
  assert.throws(() => buildRelease(h, { runCommand: compiler(calls) }), /Required build input/)
  assert.equal(calls.length, 0)
})

test('credentialled registry dependency refuses without printing the URL', t => {
  const h = fixture(t), calls = []
  h.put('frontend/package-lock.json', JSON.stringify({ lockfileVersion: 3, packages: { package: { resolved: 'https://username:DoNotExpose@registry.example.test/package.tgz' } } }))
  assert.throws(() => buildRelease(h, { runCommand: compiler(calls) }), error => /Credentialled/.test(error.message) && !error.message.includes('DoNotExpose'))
  assert.equal(calls.length, 0)
  assert.ok(!readFileSync(path.join(h.outDir, 'build-result.json'), 'utf8').includes('DoNotExpose'))
  assert.ok(!existsSync(path.join(h.outDir, 'source/frontend/package-lock.json')), 'reject credentials before writing the lock into the snapshot')
})

test('output overlapping source refuses before creating a directory', t => {
  const h = fixture(t), output = path.join(h.sourceRoot, 'frontend/src/release')
  assert.throws(() => buildRelease({ ...h, outDir: output }, { runCommand: compiler([]) }), /overlaps/)
  assert.ok(!existsSync(output))
})

test('only the exact public frontend npmrc is accepted; npm tokens refuse before snapshot write', t => {
  for (const privateConfig of ['//registry.npmjs.org/:_authToken=NeverPublishFixtureToken', '# _authToken=NeverPublishFixtureToken']) {
    const h = fixture(t), calls = []
    h.put('frontend/.npmrc', `engine-strict=true\n${privateConfig}\n`)
    assert.throws(() => buildRelease(h, { runCommand: compiler(calls) }), error => /Only public engine-strict/.test(error.message) && !error.message.includes('NeverPublishFixtureToken'))
    assert.equal(calls.length, 0)
    assert.ok(!existsSync(path.join(h.outDir, 'source/frontend/.npmrc')))
    assert.ok(!readFileSync(path.join(h.outDir, 'build-result.json'), 'utf8').includes('NeverPublishFixtureToken'))
  }
})
