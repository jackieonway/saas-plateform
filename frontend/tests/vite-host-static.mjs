import assert from 'node:assert/strict'
import test from 'node:test'
import { mkdtempSync, mkdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createServer, loadConfigFromFile } from '../node_modules/vite/dist/node/index.js'

const workspace = fileURLToPath(new URL('../', import.meta.url))
const tempBase = realpathSync(os.tmpdir())

test('business API proxies match service defaults and support independent target overrides', async () => {
  const keys = ['WORK_ORDERS_DEV_PROXY_TARGET', 'STORES_DEV_PROXY_TARGET', 'DEV_ALLOWED_HOSTS']
  const previous = keys.map(key => process.env[key])
  try {
    for (const key of keys) delete process.env[key]
    const load = async () => (await loadConfigFromFile({ command: 'serve', mode: 'test' }, path.join(workspace, 'vite.config.ts'), workspace, 'silent')).config
    const defaults = await load()
    assert.equal(defaults.server.proxy['/work-orders-api'].target, 'http://127.0.0.1:3004')
    assert.equal(defaults.server.proxy['/stores-api'].target, 'http://127.0.0.1:3031')
    assert.deepEqual(defaults.server.allowedHosts, [])
    process.env.WORK_ORDERS_DEV_PROXY_TARGET = 'http://127.0.0.1:4014'
    process.env.STORES_DEV_PROXY_TARGET = 'http://127.0.0.1:4031'
    process.env.DEV_ALLOWED_HOSTS = ' Preview.Example.test ,preview.example.test,other.test '
    const overrides = await load()
    assert.equal(overrides.server.proxy['/work-orders-api'].target, 'http://127.0.0.1:4014')
    assert.equal(overrides.server.proxy['/stores-api'].target, 'http://127.0.0.1:4031')
    assert.deepEqual(overrides.server.allowedHosts, ['preview.example.test', 'other.test'])
    for (const invalid of ['true', '*', '.example.test', '*.example.test', 'https://example.test', 'example.test:3000', '-bad.test', 'bad-.test']) {
      process.env.DEV_ALLOWED_HOSTS = invalid
      if (invalid === 'true') {
        // A literal hostname named "true" stays a narrow list, never boolean true.
        assert.deepEqual((await load()).server.allowedHosts, ['true'])
      } else await assert.rejects(load, /exact hostnames/)
    }

    const plugin = overrides.plugins.find(plugin => plugin?.name === 'opaque-microapp-api-preflight')
    let middleware
    plugin.configureServer({ middlewares: { use: handler => { middleware = handler } } })
    for (const url of ['/work-orders-api', '/work-orders-api/orders', '/stores-api', '/stores-api/stores/123']) {
      const headers = {}
      let ended = false
      const res = { setHeader: (name, value) => { headers[name] = value }, end: () => { ended = true } }
      middleware({ url, method: 'OPTIONS', headers: { origin: 'null' } }, res, () => assert.fail(`unexpected fallthrough: ${url}`))
      assert.equal(res.statusCode, 204)
      assert.equal(ended, true)
      assert.equal(headers['Access-Control-Allow-Origin'], 'null')
      assert.match(headers['Access-Control-Allow-Methods'], /PATCH/)
    }
    for (const [url, method, origin] of [
      ['/stores-api-other/stores', 'OPTIONS', 'null'],
      ['/work-orders-api-other/orders', 'OPTIONS', 'null'],
      ['/stores-api/stores', 'OPTIONS', 'https://other.test'],
      ['/stores-api/stores', 'GET', 'null'],
    ]) {
      let passed = false
      middleware({ url, method, headers: { origin } }, {}, () => { passed = true })
      assert.equal(passed, true)
    }
  } finally {
    keys.forEach((key, index) => {
      if (previous[index] === undefined) delete process.env[key]
      else process.env[key] = previous[index]
    })
  }
})

function fixture(t) {
  const root = realpathSync(mkdtempSync(path.join(tempBase, 'platform-vite-host-')))
  const envDir = path.join(root, 'empty-env')
  const servers = []
  mkdirSync(envDir)
  const previous = process.env.MICRO_APPS_DEV_DIR
  t.after(async () => {
    for (const server of servers) await server.close()
    if (previous === undefined) delete process.env.MICRO_APPS_DEV_DIR
    else process.env.MICRO_APPS_DEV_DIR = previous
    assert.equal(path.dirname(root), tempBase)
    assert.ok(path.basename(root).startsWith('platform-vite-host-'))
    assert.equal(realpathSync(root), root)
    rmSync(root, { recursive: true, force: true })
  })
  return { root, envDir, servers }
}

async function host(t, { root, envDir, servers }, versions) {
  process.env.MICRO_APPS_DEV_DIR = versions
  const server = await createServer({
    root: workspace,
    configFile: path.join(workspace, 'vite.config.ts'),
    envDir, logLevel: 'silent', clearScreen: false,
    server: { host: '::1', port: 0, strictPort: true, hmr: false, watch: null },
    cacheDir: path.join(root, 'vite-cache'),
  })
  servers.push(server)
  await server.listen()
  return `http://[::1]:${server.httpServer.address().port}`
}

test('host starts without the optional versions directory and missing versions return real 404', async (t) => {
  const f = fixture(t)
  const origin = await host(t, f, path.join(f.root, 'not-created'))
  const home = await fetch(origin)
  assert.equal(home.status, 200)
  assert.match(await home.text(), /<html/)
  for (const name of ['app.js', 'styles.css', 'missing.html', 'image.png']) {
    const response = await fetch(`${origin}/sub-apps/vue/versions/v1/${name}`, { headers: { Accept: 'text/html,*/*' } })
    assert.equal(response.status, 404, name)
    assert.equal(response.headers.get('cache-control'), 'no-store')
    assert.doesNotMatch(await response.text(), /<html/)
  }
})

test('host serves relative version resources and rejects missing or malformed paths', async (t) => {
  const f = fixture(t)
  const versions = path.join(f.root, 'versions')
  const release = path.join(versions, 'vue/versions/v1')
  mkdirSync(release, { recursive: true })
  writeFileSync(path.join(release, 'index.html'), '<!doctype html><script src="./app.js"></script>')
  writeFileSync(path.join(release, 'app.js'), 'window.__release = "v1"')
  const origin = await host(t, f, versions)
  const entry = new URL('/sub-apps/vue/versions/v1/index.html', origin)
  const html = await fetch(entry)
  assert.equal(html.status, 200)
  const script = /src="([^"]+)"/.exec(await html.text())[1]
  const asset = await fetch(new URL(script, entry))
  assert.equal(asset.status, 200)
  assert.match(asset.headers.get('content-type'), /javascript/)
  assert.equal(asset.headers.get('access-control-allow-origin'), '*')
  assert.match(asset.headers.get('cache-control'), /immutable/)
  assert.equal(await asset.text(), readFileSync(path.join(release, 'app.js'), 'utf8'))
  for (const suffix of ['missing.js', 'missing.html', 'bad%2fpath.js', '%00app.js', 'nested/../missing.js']) {
    const response = await fetch(new URL(suffix, entry), { headers: { Accept: '*/*' } })
    assert.equal(response.status, 404, suffix)
    assert.doesNotMatch(await response.text(), /<html/)
  }
  const head = await fetch(new URL('app.js', entry), { method: 'HEAD' })
  assert.equal(head.status, 200)
  assert.equal(await head.text(), '')
})

test('built-in static aliases do not turn missing assets into host HTML', async (t) => {
  const f = fixture(t)
  const origin = await host(t, f, path.join(f.root, 'not-created'))
  const entry = await fetch(`${origin}/sub-apps/vue/index.html`)
  assert.equal(entry.status, 200)
  assert.match(await entry.text(), /micro-vue/)
  for (const suffix of ['missing.js', 'missing.css', 'missing.html', 'vendor/missing.js']) {
    const response = await fetch(`${origin}/sub-apps/vue/${suffix}`, { headers: { Accept: 'text/html,*/*' } })
    assert.equal(response.status, 404, suffix)
    assert.doesNotMatch(await response.text(), /<html/)
  }
})
