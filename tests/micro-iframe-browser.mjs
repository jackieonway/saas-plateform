import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { createServer } from 'node:http'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { launchBrowser } from './browser/runtime.mjs'

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sdkSource = readFileSync(path.join(workspace, 'examples/micro-app-sdk/iframe-bridge.js'), 'utf8')
const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b }); return { promise, resolve, reject } }
const until = async (predicate) => {
  const end = Date.now() + 2000
  while (!predicate()) { assert.ok(Date.now() < end, 'protocol event did not arrive'); await new Promise((resolve) => setImmediate(resolve)) }
}
function props(epoch) {
  return { contractVersion: '2.0', contextVersion: 'context-' + epoch, contextEpoch: epoch,
    expiresAt: new Date(Date.now() + 60_000).toISOString(), tokenExpiresAt: new Date(Date.now() + 60_000).toISOString(),
    appCode: 'orders', basePath: '/micro/orders', routePath: '/micro/orders', token: 'mock-application-' + epoch,
    user: { id: 'u', username: 'user', displayName: 'User' }, tenant: { id: 't', name: 'Tenant', code: 'tenant' },
    organization: { id: 'o', name: 'Organization', path: '/o/' }, roles: [], permissions: ['orders:read'],
    dataGrants: [], menus: [], theme: 'light' }
}

// Explicit browser gate: node --test tests/micro-iframe-browser.mjs
test('real same-origin document remains isolated; renewal preserves DOM and only application token reaches CORS API',
  { timeout: 30_000 }, async () => {
    const require = createRequire(import.meta.url)
    const ts = require(path.join(workspace, 'frontend/node_modules/typescript'))
    const hostModule = ts.transpileModule(readFileSync(path.join(workspace, 'frontend/src/components/micro/iframe.ts'), 'utf8'),
      { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText
      .replaceAll('../../../../examples/micro-app-sdk/', '/sdk/')
    const requests = [], hold = deferred()
    let origin
    const server = createServer(async (request, response) => {
      const url = new URL(request.url, origin)
      if (url.pathname === '/iframe.js') { response.writeHead(200, { 'Content-Type': 'text/javascript' }); response.end(hostModule) }
      else if (['/sdk/generated/protocol.mjs', '/sdk/src/capabilities.mjs'].includes(url.pathname)) {
        response.writeHead(200, { 'Content-Type': 'text/javascript' })
        response.end(readFileSync(path.join(workspace, 'examples/micro-app-sdk', url.pathname.slice('/sdk/'.length)), 'utf8'))
      }
      else if (url.pathname === '/sdk.js') { response.writeHead(200, { 'Content-Type': 'text/javascript' }); response.end(sdkSource) }
      else if (url.pathname === '/app') { response.writeHead(200, { 'Content-Type': 'text/html' });
        response.end(readFileSync(path.join(workspace, 'tests/fixtures/iframe-app.html'), 'utf8').replaceAll('__HOST_ORIGIN__', origin)) }
      else if (url.pathname === '/api/orders') {
        response.setHeader('Access-Control-Allow-Origin', 'null')
        response.setHeader('Access-Control-Allow-Methods', 'GET, OPTIONS')
        response.setHeader('Access-Control-Allow-Headers', 'Authorization,X-Request-Id')
        if (request.method === 'OPTIONS') { response.writeHead(204); response.end(); return }
        requests.push({ authorization: request.headers.authorization, cookie: request.headers.cookie, origin: request.headers.origin })
        if (url.searchParams.has('hold')) await hold.promise
        response.writeHead(200, { 'Content-Type': 'application/json' }); response.end(JSON.stringify({ current: !url.searchParams.has('hold') }))
      } else { response.writeHead(200, { 'Content-Type': 'text/html' }); response.end('<!doctype html><div id="mount"></div><script type="module">import {createIframeLifecycle} from "/iframe.js"; window.factory=createIframeLifecycle</script>') }
    })
    await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '::1', resolve) })
    origin = `http://[::1]:${server.address().port}`
    let browser
    try {
      browser = await launchBrowser()
      const page = await browser.newPage()
      await page.goto(origin); await page.waitForFunction(() => !!window.factory)
      await page.evaluate(async (dto) => {
        window.__parentMarker = 'protected'; localStorage.setItem('platform-session', 'never-share')
        window.scope = new AbortController(); window.context = new AbortController()
        window.props = { ...dto, signal: window.context.signal, onNavigate: () => {}, onNotify: () => {} }
        window.app = window.factory({ entryUrl: '/app', appCode: 'orders', signal: window.scope.signal })
        await window.app.mount(document.querySelector('#mount'), window.props)
      }, JSON.parse(JSON.stringify(props(1))))
      const frame = page.frameLocator('iframe')
      await frame.locator('#state').waitFor()
      assert.equal(await page.locator('iframe').getAttribute('sandbox'), 'allow-scripts allow-forms')
      assert.equal(await frame.locator('#state').getAttribute('data-parent-blocked'), 'true')
      assert.equal(await frame.locator('#state').getAttribute('data-storage-blocked'), 'true')
      assert.equal(await page.evaluate(() => window.__parentMarker), 'protected')
      await frame.locator('#input').fill('preserved')
      await frame.locator('#slow').click()
      await until(() => requests.length === 1)
      await page.evaluate(async (dto) => { window.context.abort(); window.context = new AbortController()
        window.props = { ...dto, signal: window.context.signal, onNavigate: () => {}, onNotify: () => {} }; await window.app.update(window.props) }, JSON.parse(JSON.stringify(props(2))))
      assert.equal(await frame.locator('#input').inputValue(), 'preserved')
      await frame.locator('#request').click()
      await frame.locator('#state[data-request="current"]').waitFor()
      assert.equal(requests.at(-1).authorization, 'Bearer mock-application-2')
      assert.equal(requests.at(-1).cookie, undefined)
      assert.equal(requests.at(-1).origin, 'null')
      hold.resolve(); await frame.locator('#state[data-old="aborted"]').waitFor()
      await page.evaluate(() => window.scope.abort())
      assert.equal(await page.locator('iframe').count(), 0)
      await page.evaluate(async (dto) => { window.context = new AbortController(); window.scope = new AbortController()
        window.app = window.factory({ entryUrl: '/app?noBridge=1', appCode: 'orders', signal: window.scope.signal })
        window.pending = window.app.mount(document.querySelector('#mount'), { ...dto, signal: window.context.signal, onNavigate() {}, onNotify() {} }).catch((error) => error.name)
      }, JSON.parse(JSON.stringify(props(3))))
      await page.locator('iframe').waitFor(); await page.evaluate(() => window.scope.abort())
      assert.equal(await page.evaluate(() => window.pending), 'AbortError')
      assert.equal(await page.locator('iframe').count(), 0)
    } finally { hold.resolve(); await browser?.close(); await new Promise((resolve) => server.close(resolve)) }
  })
