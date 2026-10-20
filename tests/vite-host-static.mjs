import assert from 'node:assert/strict'
import test from 'node:test'
import { mkdtempSync, mkdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createServer } from '../frontend/node_modules/vite/dist/node/index.js'

const workspace = fileURLToPath(new URL('../', import.meta.url))
const tempBase = realpathSync(os.tmpdir())

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
    root: path.join(workspace, 'frontend'),
    configFile: path.join(workspace, 'frontend/vite.config.ts'),
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
