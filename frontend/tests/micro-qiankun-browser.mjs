/** Real qiankun + browser regression; all HTTP and authorization data are isolated fixtures. */
import assert from 'node:assert/strict'
import { execFile, spawn } from 'node:child_process'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { promisify } from 'node:util'
import { fileURLToPath } from 'node:url'
import { launchBrowser } from './browser/runtime.mjs'

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repository = path.resolve(workspace, '../..')
const bindHost = process.platform === 'win32' ? '::1' : '127.0.0.1'
const prefix = '/__qiankun-browser/'
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
let vite, browser, origin, buildDirectory, startup = ''

async function startServer() {
  const port = await new Promise((resolve, reject) => {
    const probe = net.createServer().on('error', reject)
    probe.listen(0, bindHost, () => {
      const port = probe.address().port
      probe.close(error => error ? reject(error) : resolve(port))
    })
  })
  origin = `http://${bindHost.includes(':') ? '[' + bindHost + ']' : bindHost}:${port}`
  vite = spawn(process.execPath, [path.join(workspace, 'node_modules/vite/bin/vite.js'),
    '--host', bindHost, '--port', String(port), '--strictPort'],
  { cwd: workspace, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
  let spawnError
  vite.on('error', error => { spawnError = error })
  for (const stream of [vite.stdout, vite.stderr]) stream.on('data', chunk => { startup = (startup + chunk).slice(-8000) })
  for (let i = 0; i < 300; i++) {
    if (spawnError) throw spawnError
    if (vite.exitCode !== null) throw new Error('Owned Vite stopped: ' + startup)
    if (startup.includes('ready in')) return
    await pause(100)
  }
  throw new Error('Owned Vite did not become ready: ' + startup)
}

const fixtureScript = `
(function () {
  var root, latest, interval;
  function draw() {
    root.textContent = latest.contextEpoch + ':' + latest.routePath + ':' + latest.theme;
    root.dataset.epoch = String(latest.contextEpoch);
  }
  window.qiankunBrowserFixture = {
    bootstrap: async function () {},
    mount: async function (props) {
      if (!props.platform || typeof props.services.downloadCsv !== 'function') throw new Error('Missing platform contract');
      if (Object.keys(props.platform).includes('container')) throw new Error('qiankun props polluted platform DTO');
      root = props.container.querySelector('[data-qiankun-child]'); latest = props.platform;
      var trigger = props.container.querySelector('[data-load-chunk]');
      if (trigger) trigger.addEventListener('click', function () {
        var script = document.createElement('script'); script.src = '${prefix}late.js';
        document.head.appendChild(script);
      });
      interval = setInterval(function () { root.dataset.tick = String(Number(root.dataset.tick || 0) + 1) }, 25);
      draw();
    },
    update: async function (props) { latest = props.platform; draw() },
    unmount: async function () { clearInterval(interval); root.replaceChildren(); root.dataset.unmounted = 'yes' }
  };
})();`

async function fixture(run, customRoutes) {
  const context = await browser.newContext()
  const page = await context.newPage()
  page.setDefaultTimeout(20_000)
  const errors = [], requests = []
  page.on('pageerror', error => errors.push(error.message))
  await context.addCookies([{ name: 'platform-session', value: 'must-not-reach-static-upstream', url: origin }])
  await context.route(url => url.origin === origin && url.pathname === '/src/main.tsx', route => route.fulfill({
    status: 200, contentType: 'application/javascript', body: `
      import { createQiankunLifecycle } from '/src/components/micro/qiankun.ts';
      window.__qa = { createQiankunLifecycle };
      document.body.innerHTML = '<section id="host-stays">Host layout</section><div id="mount"></div>';
      document.body.dataset.qiankunFixture = 'ready';`,
  }))
  await context.route(url => url.origin === origin && url.pathname.startsWith(prefix), async route => {
    const request = route.request(), url = new URL(request.url())
    requests.push({ path: url.pathname, headers: await request.allHeaders() })
    if (customRoutes && await customRoutes(route, url)) return
    if (url.pathname.endsWith('entry.html')) return route.fulfill({ status: 200, contentType: 'text/html',
      body: '<!doctype html><html><body><div data-qiankun-child></div><button data-load-chunk>Load chunk</button><script src="./fixture.js" entry></script></body></html>' })
    if (url.pathname.endsWith('fixture.js')) return route.fulfill({ status: 200, contentType: 'application/javascript', body: fixtureScript })
    return route.fulfill({ status: 404, body: 'Unknown isolated qiankun fixture' })
  })
  // Business applications may request their APIs; this fixture grants no business data.
  await context.route(url => url.origin === origin && /^\/(?:api|work-orders-api|stores-api|oauth-playground-api)\//.test(url.pathname),
    route => route.fulfill({ status: 403, contentType: 'application/json', body: '{"code":"FORBIDDEN","message":"Isolated fixture"}' }))
  try {
    await page.goto(origin)
    await page.waitForFunction(() => document.body.dataset.qiankunFixture === 'ready')
    await run({ page, requests, errors })
  } finally { await context.close() }
}

async function mount(page, { appCode = 'fixture', entry = prefix + 'entry.html' } = {}) {
  return page.evaluate(async ({ appCode, entry }) => {
    const qa = window.__qa
    qa.scope = new AbortController()
    qa.lease = new AbortController()
    qa.props = { sdkVersion: '2.0.0', contractVersion: '2.0', appCode, basePath: '/micro/' + appCode,
      routePath: '/micro/' + appCode, contextVersion: 'fixture-1', contextEpoch: 1,
      expiresAt: new Date(Date.now() + 120_000).toISOString(), tokenExpiresAt: new Date(Date.now() + 120_000).toISOString(),
      signal: qa.lease.signal, token: 'isolated-application-token',
      user: { id: 'fixture-user', username: 'fixture', displayName: 'Fixture' },
      tenant: { id: 'fixture-tenant', name: 'Fixture tenant', code: 'fixture' },
      organization: { id: 'fixture-org', name: 'Fixture organization', path: '/fixture-org/' }, organizationScopeIds: ['fixture-org'],
      roles: [], permissions: [], dataGrants: [],
      menus: [{ id: 'menu', parentId: null, type: 'MENU', title: 'Fixture', path: '/micro/' + appCode, icon: null, sortOrder: 1, visible: true }],
      theme: 'light', onNavigate: path => { qa.navigation = path }, onNotify: message => { qa.notification = message } }
    qa.lifecycle = qa.createQiankunLifecycle({ appCode, entryUrl: entry, signal: qa.scope.signal,
      check: () => { if (qa.scope.signal.aborted || qa.lease.signal.aborted) throw new DOMException('Revoked', 'AbortError') } })
    await qa.lifecycle.mount(document.querySelector('#mount'), qa.props)
    return document.querySelector('#mount').textContent
  }, { appCode, entry })
}

async function lifecycleScenario() {
  await fixture(async ({ page, requests, errors }) => {
    assert.match(await mount(page), /1:\/micro\/fixture:light/)
    await page.waitForFunction(() => Number(document.querySelector('[data-qiankun-child]')?.dataset.tick) > 0)
    await page.evaluate(async () => {
      const qa = window.__qa
      qa.props = { ...qa.props, contextEpoch: 2, contextVersion: 'fixture-2', routePath: '/micro/fixture/orders/123?status=open#detail', theme: 'dark' }
      await qa.lifecycle.update(qa.props)
    })
    assert.match(await page.locator('#mount').innerText(), /2:\/micro\/fixture\/orders\/123\?status=open#detail:dark/)
    await page.evaluate(async () => { window.__qa.lease.abort(); await window.__qa.lifecycle.unmount() })
    assert.equal(await page.locator('#host-stays').innerText(), 'Host layout')
    assert.equal(await page.locator('#mount').innerText(), '')
    assert.equal(requests.some(request => request.path.endsWith('fixture.js')), true)
    assert(requests.every(request => !request.headers.authorization && !request.headers.cookie), 'static fetch must omit platform credentials')
    assert.deepEqual(errors, [])
  })
  console.log('PASS qiankun browser: real lifecycle, update, revocation, credential omission and cleanup')
}

async function failureRecoveryScenario() {
  let failed = true
  await fixture(async ({ page, errors }) => {
    await assert.rejects(mount(page), /503|LOAD_ERROR|加载|资源请求失败/)
    await page.evaluate(() => window.__qa.lifecycle.unmount())
    // Source fixed at the same URL: a new mount must not reuse rejected HTML/script promises.
    failed = false
    assert.match(await mount(page), /1:\/micro\/fixture:light/)
    await page.evaluate(() => window.__qa.lifecycle.unmount())
    assert.deepEqual(errors, [])
  }, async (route, url) => {
    if (failed && url.pathname.endsWith('entry.html')) {
      await route.fulfill({ status: 503, contentType: 'text/plain', body: 'Expected fixture failure' })
      return true
    }
    return false
  })
  console.log('PASS qiankun browser: corrected source retries without rejected resource cache')
}

async function dynamicChunkScenario() {
  let release, requested
  const delayed = new Promise(resolve => { release = resolve })
  const started = new Promise(resolve => { requested = resolve })
  await fixture(async ({ page, requests, errors }) => {
    await mount(page)
    await page.locator('[data-load-chunk]').click()
    const deadline = setTimeout(() => requested(false), 15_000)
    assert.equal(await started, true, 'qiankun must fetch the dynamically appended chunk')
    clearTimeout(deadline)
    const resource = requests.find(request => request.path.endsWith('late.js'))
    assert(resource && !resource.headers.cookie && !resource.headers.authorization, 'dynamic fetch must omit platform credentials')
    await page.evaluate(async () => {
      const qa = window.__qa
      qa.lease.abort()
      qa.scope.abort()
      await qa.lifecycle.unmount()
    })
    release()
    // Flush the isolated response and queued browser error reporting after revocation.
    await page.waitForTimeout(300)
    assert.equal(await page.evaluate(() => !!window.__qa.lateExecuted), false, 'revoked chunk must never execute')
    assert.equal(await page.evaluate(() => Object.hasOwn(document.createElement('script'), 'src')), false, 'host DOM must stay native')
    // import-html-entry may separately report its aborted non-entry script promise.
    assert(errors.every(message => /abort|cancel/i.test(message)), errors.join('\n'))
  }, async (route, url) => {
    if (!url.pathname.endsWith('late.js')) return false
    requested(true)
    await delayed
    try { await route.fulfill({ status: 200, contentType: 'application/javascript', body: 'window.__qa.lateExecuted = true;' }) }
    catch { /* The authorized lease cancelled this network request. */ }
    return true
  }).finally(() => release())
  console.log('PASS qiankun browser: delayed dynamic chunk omits credentials and cannot execute after revocation')
}

async function dynamicLeaseScenario() {
  const delayed = [], waiters = []
  const waitRequest = count => {
    if (delayed.length >= count) return Promise.resolve()
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`Dynamic chunk request ${count} did not arrive`)), 20_000)
      waiters.push([count, () => { clearTimeout(timer); resolve() }])
    })
  }
  const script = fixtureScript + `
  (function () {
    var lifecycle = window.qiankunBrowserFixture, mount = lifecycle.mount, update = lifecycle.update;
    function loadChunk(attribute) {
      var script = document.createElement('script');
      var src = new URL('./lease-chunk.js', window.__INJECTED_PUBLIC_PATH_BY_QIANKUN__).href;
      if (attribute) script.setAttribute('src', src); else script.src = src;
      document.head.appendChild(script);
    }
    lifecycle.mount = async function (props) { await mount(props); loadChunk(true) };
    lifecycle.update = async function (props) { await update(props); loadChunk(false) };
  })();`
  await fixture(async ({ page, requests }) => {
    const unexpected = []
    page.on('pageerror', error => { if (error.name !== 'AbortError') unexpected.push(error.message) })
    await mount(page)
    await waitRequest(1)
    await page.evaluate(async () => {
      const qa = window.__qa
      qa.lease.abort()
      qa.lease = new AbortController()
      qa.props = { ...qa.props, signal: qa.lease.signal, contextEpoch: 2, contextVersion: 'fixture-lease-2' }
      await qa.lifecycle.update(qa.props)
    })
    await waitRequest(2)
    await delayed[0].fulfill({ status: 200, contentType: 'application/javascript', body: "document.documentElement.dataset.staleLease = 'executed'" }).catch(() => {})
    await delayed[1].fulfill({ status: 200, contentType: 'application/javascript', body: "document.documentElement.dataset.currentLease = 'executed'" })
    await page.waitForFunction(() => document.documentElement.dataset.currentLease === 'executed')
    assert.equal(await page.evaluate(() => document.documentElement.dataset.staleLease), undefined)
    await page.evaluate(async () => { window.__qa.scope.abort(); window.__qa.lease.abort(); await window.__qa.lifecycle.unmount() })
    await mount(page)
    await waitRequest(3)
    await delayed[2].fulfill({ status: 200, contentType: 'application/javascript', body: "document.documentElement.dataset.remountLease = 'executed'" })
    await page.waitForFunction(() => document.documentElement.dataset.remountLease === 'executed')
    const urls = delayed.map(route => new URL(route.request().url()))
    assert.equal(new Set(urls.map(url => url.searchParams.get('__saas_lease'))).size, 3)
    assert(requests.filter(request => request.path.endsWith('lease-chunk.js')).every(request => !request.headers.cookie && !request.headers.authorization))
    assert.equal(await page.evaluate(() => Object.hasOwn(document.createElement('script'), 'src')), false)
    await page.evaluate(async () => { window.__qa.scope.abort(); window.__qa.lease.abort(); await window.__qa.lifecycle.unmount() })
    assert.deepEqual(unexpected, [])
  }, async (route, url) => {
    if (url.pathname.endsWith('fixture.js')) {
      await route.fulfill({ status: 200, contentType: 'application/javascript', body: script })
      return true
    }
    if (!url.pathname.endsWith('lease-chunk.js')) return false
    delayed.push(route)
    for (const [count, resolve] of waiters) if (delayed.length >= count) resolve()
    return true
  })
  console.log('PASS qiankun browser: setAttribute, new authorization lease and remount cannot reuse an old dynamic chunk')
}

async function artifactScenario(appCode, directory) {
  await fixture(async ({ page, errors }) => {
    const content = await mount(page, { appCode, entry: `${prefix}apps/${appCode}/index.html` })
    assert(content.trim().length > 5, `${appCode} must render business content`)
    await page.evaluate(async () => {
      const qa = window.__qa
      qa.props = { ...qa.props, theme: 'dark', routePath: qa.props.basePath + '/shared' }
      await qa.lifecycle.update(qa.props)
      await qa.lifecycle.unmount()
    })
    assert.equal(await page.locator('#mount').innerText(), '')
    assert.equal(await page.locator('#host-stays').innerText(), 'Host layout')
    assert.deepEqual(errors, [], `${appCode} browser errors`)
  }, async (route, url) => {
    const base = `${prefix}apps/${appCode}/`
    if (!url.pathname.startsWith(base)) return false
    const name = decodeURIComponent(url.pathname.slice(base.length))
    assert(!name.split('/').some(segment => segment === '..'))
    const contentType = name.endsWith('.js') ? 'application/javascript' : name.endsWith('.css') ? 'text/css' : 'text/html'
    await route.fulfill({ status: 200, contentType, body: await readFile(path.join(directory, name)) })
    return true
  })
  console.log(`PASS qiankun browser: ${appCode} real built HTML/UMD, update and unmount`)
}

try {
  await startServer()
  browser = await launchBrowser()
  await lifecycleScenario()
  await failureRecoveryScenario()
  await dynamicChunkScenario()
  await dynamicLeaseScenario()
  if (process.argv.includes('--all-apps')) {
    buildDirectory = await mkdtemp(path.join(os.tmpdir(), 'saas-qiankun-browser-'))
    for (const [code, directory] of [
      ['vue-workshop', 'saas-examples/micro-vue/dist'],
      ['legacy-jquery', 'saas-examples/micro-jquery/dist'],
      ['report-center', 'saas-examples/report-center/dist'],
      ['websocket-chat', 'saas-examples/websocket/dist'],
      ['oauth-playground', 'saas-examples/oauth-playground/frontend/dist'],
      ['work-orders', 'saas-work-orders/frontend/dist'],
      ['stores', 'saas-stores/frontend/dist'],
    ]) {
      let output = path.join(repository, directory)
      if (code === 'work-orders' || code === 'stores') {
        output = path.join(buildDirectory, code)
        await promisify(execFile)(process.execPath, ['scripts/build.mjs',
          '--allowed-host-origin', origin, '--out-dir', output], {
          cwd: path.join(repository, code === 'work-orders' ? 'saas-work-orders/frontend' : 'saas-stores/frontend'),
          windowsHide: true, maxBuffer: 4 * 1024 * 1024,
        })
      }
      await artifactScenario(code, output)
    }
  }
} finally {
  await browser?.close()
  if (buildDirectory) await rm(buildDirectory, { recursive: true, force: true })
  if (vite && vite.exitCode === null) {
    const exited = new Promise(resolve => vite.once('exit', resolve))
    vite.kill()
    await Promise.race([exited, pause(5000)])
    if (vite.exitCode === null) vite.kill('SIGKILL')
  }
}
