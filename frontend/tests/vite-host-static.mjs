import assert from 'node:assert/strict'
import test from 'node:test'
import { mkdtempSync, mkdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createServer as createHttpServer } from 'node:http'
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

    assert.equal(overrides.plugins.some(plugin => plugin?.name === 'external-user-view-preflight'), false,
      'The qiankun host must not synthesize the retired opaque iframe preflight')
  } finally {
    keys.forEach((key, index) => {
      if (previous[index] === undefined) delete process.env[key]
      else process.env[key] = previous[index]
    })
  }
})

test('OAuth exercise proxies have independent explicit targets and fixed namespace rewrites', async () => {
  const keys = ['OAUTH_PLAYGROUND_DEV_PROXY_TARGET', 'OAUTH_PLAYGROUND_STATIC_DEV_PROXY_TARGET']
  const previous = keys.map(key => process.env[key])
  const load = async () => (await loadConfigFromFile({ command: 'serve', mode: 'test' }, path.join(workspace, 'vite.config.ts'), workspace, 'silent')).config
  try {
    for (const key of keys) delete process.env[key]
    const defaults = await load(), apiKey = '^/oauth-playground-api(?:/|\\?|$)', callbackKey = '^/callback(?:\\?|$)'
    assert.equal(defaults.server.proxy[apiKey].target, 'http://127.0.0.1:3011')
    assert.equal(defaults.server.proxy[callbackKey].target, 'http://127.0.0.1:3010')
    assert.equal(defaults.server.proxy['^/sub/oauth-playground/'].target, 'http://127.0.0.1:3010')
    assert.equal(defaults.server.proxy[apiKey].rewrite('/oauth-playground-api/config?flow=code'), '/api/config?flow=code')
    assert.equal(defaults.server.proxy[apiKey].rewrite('/oauth-playground-api?flow=code'), '/api?flow=code')
    assert.equal(defaults.server.proxy[callbackKey].rewrite('/callback?code=secret&state=opaque'), '/callback.html?code=secret&state=opaque')
    assert.equal(new RegExp(apiKey).test('/oauth-playground-api-other/config'), false)
    assert.equal(new RegExp(callbackKey).test('/callback.html'), false)
    assert.ok(Object.keys(defaults.server.proxy).indexOf(apiKey) < Object.keys(defaults.server.proxy).indexOf('/oauth'))
    process.env.OAUTH_PLAYGROUND_DEV_PROXY_TARGET = 'http://127.0.0.1:4011'
    process.env.OAUTH_PLAYGROUND_STATIC_DEV_PROXY_TARGET = 'http://127.0.0.1:4010'
    const overrides = await load()
    assert.equal(overrides.server.proxy[apiKey].target, 'http://127.0.0.1:4011')
    assert.equal(overrides.server.proxy[callbackKey].target, 'http://127.0.0.1:4010')
    for (const invalid of ['http://user:secret@127.0.0.1:4011', 'http://127.0.0.1:4011/path', 'http://127.0.0.1:4011?token=secret', 'file:///private']) {
      process.env.OAUTH_PLAYGROUND_DEV_PROXY_TARGET = invalid
      await assert.rejects(load, /exact HTTP\(S\) origin/)
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

test('retired non-versioned microapp aliases always return a real 404', async (t) => {
  const f = fixture(t)
  const origin = await host(t, f, path.join(f.root, 'not-created'))
  for (const suffix of ['index.html', 'app.js', 'vendor/micro-sdk.js', 'missing.css', 'versions-not-a-route/index.html']) {
    const response = await fetch(`${origin}/sub-apps/vue/${suffix}`, { headers: { Accept: 'text/html,*/*' } })
    assert.equal(response.status, 404, suffix)
    assert.equal(response.headers.get('cache-control'), 'no-store')
    assert.doesNotMatch(await response.text(), /<html/)
  }
})

test('OAuth exercise HTTP proxy isolates platform credentials, preserves origin and cookies, and bounds callback methods', async t => {
  const f = fixture(t), seen = [], staticSeen = []
  const api = createHttpServer(async (request, response) => {
    let body = ''
    for await (const chunk of request) body += chunk
    seen.push({ path: request.url, method: request.method, headers: request.headers, body })
    response.setHeader('Content-Type', 'application/json')
    response.setHeader('Set-Cookie', ['oauth_playground=next; HttpOnly; SameSite=Lax; Path=/', '__Host-oauth_playground=secure; HttpOnly; Secure; SameSite=Lax; Path=/'])
    response.end(JSON.stringify({ ok: true }))
  })
  const assets = createHttpServer((request, response) => {
    staticSeen.push({ path: request.url, method: request.method, headers: request.headers })
    if (!['/callback.html', '/index.html', '/assets/callback.js'].includes(request.url.split('?')[0])) { response.writeHead(404); response.end(); return }
    response.setHeader('Content-Type', request.url.includes('.js') ? 'text/javascript' : 'text/html')
    response.end(request.url.includes('.js') ? 'window.__callbackReady = true' : '<!doctype html><div data-oauth-playground></div>')
  })
  await Promise.all([new Promise(resolve => api.listen(0, '127.0.0.1', resolve)), new Promise(resolve => assets.listen(0, '127.0.0.1', resolve))])
  const keys = ['OAUTH_PLAYGROUND_DEV_PROXY_TARGET', 'OAUTH_PLAYGROUND_STATIC_DEV_PROXY_TARGET'], previous = keys.map(key => process.env[key])
  t.after(async () => {
    for (const server of [api, assets]) { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)) }
    keys.forEach((key, index) => { if (previous[index] === undefined) delete process.env[key]; else process.env[key] = previous[index] })
  })
  process.env.OAUTH_PLAYGROUND_DEV_PROXY_TARGET = `http://127.0.0.1:${api.address().port}`
  process.env.OAUTH_PLAYGROUND_STATIC_DEV_PROXY_TARGET = `http://127.0.0.1:${assets.address().port}`
  const origin = await host(t, f, path.join(f.root, 'no-versions'))
  const request = await fetch(`${origin}/oauth-playground-api/callback?kind=code`, { method: 'POST', headers: {
    Origin: origin, Authorization: 'Bearer platform-private-token', 'Content-Type': 'application/json',
    Cookie: 'platform_session=platform-private-cookie; oauth_playground=demo-session; another=private; __Host-oauth_playground=secure-demo-session; oauth_playground_evil=blocked',
  }, body: '{"code":"exercise-code","state":"exercise-state"}' })
  assert.equal(request.status, 200)
  assert.deepEqual(request.headers.getSetCookie(), ['oauth_playground=next; HttpOnly; SameSite=Lax; Path=/', '__Host-oauth_playground=secure; HttpOnly; Secure; SameSite=Lax; Path=/'])
  assert.equal(seen[0].path, '/api/callback?kind=code')
  assert.equal(seen[0].method, 'POST')
  assert.equal(seen[0].headers.origin, origin)
  assert.equal(seen[0].headers.authorization, undefined)
  assert.equal(seen[0].headers.cookie, 'oauth_playground=demo-session; __Host-oauth_playground=secure-demo-session')
  assert.equal(seen[0].body, '{"code":"exercise-code","state":"exercise-state"}')
  await fetch(`${origin}/oauth-playground-api/config`, { headers: { Cookie: 'platform_session=private', Authorization: 'Bearer private' } })
  assert.equal(seen[1].headers.cookie, undefined)
  assert.equal(seen[1].headers.authorization, undefined)
  for (const suffix of ['internal', 'internal/events', 'metrics', 'metrics/private', '%69nternal/secret']) {
    const response = await fetch(`${origin}/oauth-playground-api/${suffix}`)
    assert.equal(response.status, 404)
  }
  assert.equal(seen.length, 2)
  for (const method of ['GET', 'HEAD']) {
    const callback = await fetch(`${origin}/callback?code=callback-code&state=callback-state`, { method, headers: { Authorization: 'Bearer private', Cookie: 'platform_session=private; oauth_playground=demo' } })
    assert.equal(callback.status, 200)
    assert.equal(callback.headers.get('cache-control'), 'no-store')
    assert.equal(staticSeen.at(-1).path, '/callback.html?code=callback-code&state=callback-state')
    assert.equal(staticSeen.at(-1).headers.authorization, undefined)
    assert.equal(staticSeen.at(-1).headers.cookie, undefined)
    if (method === 'HEAD') assert.equal(await callback.text(), '')
  }
  for (const method of ['POST', 'PUT', 'DELETE', 'OPTIONS']) {
    const callback = await fetch(`${origin}/callback?code=blocked`, { method })
    assert.equal(callback.status, 405)
    assert.equal(callback.headers.get('allow'), 'GET, HEAD')
    assert.equal((await fetch(`${origin}/sub/oauth-playground/assets/callback.js`, { method })).status, 405)
  }
  assert.equal(staticSeen.length, 2)
  const script = await fetch(`${origin}/sub/oauth-playground/assets/callback.js`, { headers: { Authorization: 'Bearer private', Cookie: 'platform_session=private' } })
  assert.equal(script.status, 200)
  assert.equal(staticSeen.at(-1).path, '/assets/callback.js')
  assert.equal(staticSeen.at(-1).headers.authorization, undefined)
  assert.equal(staticSeen.at(-1).headers.cookie, undefined)
  const redirect = await fetch(`${origin}/sub/oauth-playground?view=code`, { redirect: 'manual' })
  assert.equal(redirect.status, 308)
  assert.equal(redirect.headers.get('location'), '/sub/oauth-playground/?view=code')
  assert.equal((await fetch(`${origin}/sub/oauth-playground/missing.js`)).status, 404)
})
