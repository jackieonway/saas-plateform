import assert from 'node:assert/strict'
import test from 'node:test'
import { spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, realpathSync, rmSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createServer } from '../frontend/node_modules/vite/dist/node/index.js'
import { SDK_VERSION } from '../examples/micro-app-sdk/generated/protocol.mjs'

const workspace = fileURLToPath(new URL('../', import.meta.url))
const tempBase = realpathSync(os.tmpdir())
const sentinels = {
  DATABASE_URL: 'postgresql://phase2-fixture:phase2-private-db-password@invalid.test/phase2-never-connect',
  JWT_SECRET: 'phase2-private-signing-secret-never-publish',
  PLATFORM_SESSION: 'phase2-private-host-session-never-publish',
  VITE_CLIENT_SECRET: 'phase2-private-vite-secret-never-publish',
  VITE_HOST_SESSION: 'phase2-private-vite-host-session-never-publish',
}

function command(args, env = process.env) {
  const result = spawnSync(process.execPath, args, {
    cwd: workspace, env, encoding: 'utf8', windowsHide: true, timeout: 120000, maxBuffer: 8 * 1024 * 1024,
  })
  assert.ifError(result.error)
  assert.equal(result.status, 0, `${args[0]} failed:\n${result.stdout}\n${result.stderr}`)
  return result
}

function verifyPublicTree(directory) {
  let files = 0
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    assert.ok(!entry.isSymbolicLink(), 'a static build must not contain filesystem links')
    assert.doesNotMatch(entry.name, /^\.env|^\.npmrc$/i)
    const file = path.join(directory, entry.name)
    if (entry.isDirectory()) files += verifyPublicTree(file)
    else {
      const bytes = readFileSync(file)
      for (const sentinel of Object.values(sentinels)) assert.equal(bytes.includes(Buffer.from(sentinel)), false, file)
      assert.doesNotMatch(bytes.toString('utf8'), /postgres(?:ql)?:\/\//i, file)
      files++
    }
  }
  return files
}

test('root npm build uses Vite and no private build configuration reaches any frontend output', () => {
  assert.ok(process.env.npm_execpath && existsSync(process.env.npm_execpath), 'run with npm run test:vite to use the selected npm executable')
  command([process.env.npm_execpath, 'run', 'build:all'], { ...process.env, ...sentinels, WORK_ORDERS_ALLOWED_HOST_ORIGINS: 'https://platform.example.com', STORES_ALLOWED_HOST_ORIGINS: 'https://platform.example.com' })
  for (const directory of ['frontend', 'mini-services/micro-vue', 'mini-services/micro-jquery', 'examples/websocket', 'mini-services/oauth-playground/frontend', 'business-apps/work-orders/frontend', 'business-apps/stores/frontend']) {
    assert.ok(verifyPublicTree(path.join(workspace, directory, 'dist')) > 0, directory)
  }
})

test('built compatibility apps load classic resources through immutable version URLs with real 404', async (t) => {
  const root = realpathSync(mkdtempSync(path.join(tempBase, 'platform-vite-release-')))
  const versions = path.join(root, 'versions')
  const envDir = path.join(root, 'empty-env')
  mkdirSync(envDir)
  const previous = process.env.MICRO_APPS_DEV_DIR
  let server
  t.after(async () => {
    await server?.close()
    if (previous === undefined) delete process.env.MICRO_APPS_DEV_DIR
    else process.env.MICRO_APPS_DEV_DIR = previous
    assert.equal(path.dirname(root), tempBase)
    assert.ok(path.basename(root).startsWith('platform-vite-release-'))
    assert.equal(realpathSync(root), root)
    rmSync(root, { recursive: true, force: true })
  })
  for (const [name, code] of [['micro-vue', 'phase2-vue'], ['micro-jquery', 'phase2-jquery']]) {
    command(['scripts/package-microapp.mjs', '--app', code, '--environment', 'development', '--version', '2.0.0',
      '--input', `mini-services/${name}/dist`, '--output-root', versions])
  }
  process.env.MICRO_APPS_DEV_DIR = versions
  server = await createServer({
    root: path.join(workspace, 'frontend'), configFile: path.join(workspace, 'frontend/vite.config.ts'),
    envDir, logLevel: 'silent', clearScreen: false, cacheDir: path.join(root, 'vite-cache'),
    server: { host: '::1', port: 0, strictPort: true, hmr: false, watch: null },
  })
  await server.listen()
  const origin = `http://[::1]:${server.httpServer.address().port}`
  for (const [name, code] of [['micro-vue', 'phase2-vue'], ['micro-jquery', 'phase2-jquery']]) {
    const release = JSON.parse(readFileSync(path.join(versions, code, 'versions/2.0.0/release.json'), 'utf8'))
    const entry = new URL(release.entryUrl, origin)
    const response = await fetch(entry)
    assert.equal(response.status, 200)
    const html = await response.text()
    assert.doesNotMatch(html, /type=["']module["']|\/@vite\/client/)
    const scripts = [...html.matchAll(/<script\s+src="([^"]+)"/g)].map((match) => match[1])
    assert.equal(scripts.length, 3)
    assert.equal(scripts.filter(resource => resource.replace(/^\.\//, '') === 'vendor/micro-sdk.js').length, 1)
    assert.deepEqual(readFileSync(path.join(workspace, 'mini-services', name, 'dist/vendor/micro-sdk.js')),
      readFileSync(path.join(workspace, 'examples/micro-app-sdk/releases', SDK_VERSION, 'sdk.js')),
      'the immutable version must contain the registered SDK bytes')
    for (const resource of scripts) {
      assert.ok(!resource.startsWith('/'), 'resources must remain relative to this version entry')
      const url = new URL(resource, entry)
      assert.ok(url.pathname.startsWith(`/sub-apps/${code}/versions/2.0.0/`))
      const loaded = await fetch(url)
      assert.equal(loaded.status, 200)
      assert.match(loaded.headers.get('content-type'), /javascript/)
      assert.equal(loaded.headers.get('access-control-allow-origin'), '*')
      assert.equal(Buffer.compare(Buffer.from(await loaded.arrayBuffer()), readFileSync(path.join(workspace, 'mini-services', name, 'dist', resource))), 0)
    }
    for (const resource of ['missing.js', 'vendor/missing.js', 'missing.css', 'missing.html']) {
      const missing = await fetch(new URL(resource, entry), { headers: { Accept: 'text/html,*/*' } })
      assert.equal(missing.status, 404)
      assert.doesNotMatch(await missing.text(), /<html/)
    }
  }
})
