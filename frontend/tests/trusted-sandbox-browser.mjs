/** Real browser DOM/MutationObserver checks for the production trusted HTML sandbox. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { launchBrowser } from './browser/runtime.mjs'

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const bindHost = process.env.TRUSTED_SANDBOX_BROWSER_BIND || (process.platform === 'win32' ? '::1' : '127.0.0.1')
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
const resourcePrefix = '/__trusted-sandbox/'
let browser, vite, origin, startup = ''

async function startServer() {
  const port = await new Promise((resolve, reject) => {
    const probe = net.createServer().on('error', reject)
    probe.listen(0, bindHost, () => {
      const available = probe.address().port
      probe.close(error => error ? reject(error) : resolve(available))
    })
  })
  origin = `http://${bindHost.includes(':') ? '[' + bindHost + ']' : bindHost}:${port}`
  vite = spawn(process.execPath, [path.join(workspace, 'node_modules/vite/bin/vite.js'), '--host', bindHost, '--port', String(port), '--strictPort'], {
    cwd: workspace, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
  })
  let spawnError
  vite.on('error', error => { spawnError = error })
  for (const stream of [vite.stdout, vite.stderr]) stream.on('data', chunk => { startup = (startup + chunk).slice(-6000) })
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    if (spawnError) throw spawnError
    if (vite.exitCode !== null) throw new Error('Owned Vite stopped: ' + startup)
    if (startup.includes('ready in')) return
    await pause(100)
  }
  throw new Error('Owned Vite did not become ready: ' + startup)
}

async function fixture(run) {
  const context = await browser.newContext()
  const page = await context.newPage()
  page.setDefaultTimeout(15_000)
  const errors = [], requests = []
  page.on('pageerror', error => errors.push(error.message))
  await context.route(url => url.origin === origin && url.pathname === '/src/main.tsx', route => route.fulfill({
    status: 200, contentType: 'application/javascript', body: `
      import { MicroSandbox } from '/src/components/micro/sandbox.ts';
      import { loadHtmlEntry } from '/src/components/micro/loader.ts';
      window.__trustedSandboxFixture = { MicroSandbox, loadHtmlEntry };
      document.body.dataset.sandboxFixture = 'ready';
    `,
  }))
  await context.route(url => url.origin === origin && url.pathname.startsWith(resourcePrefix), route => {
    const url = new URL(route.request().url())
    requests.push(url.pathname)
    if (url.pathname.endsWith('.css')) return route.fulfill({ status: 200, contentType: 'text/css', body: ':root { --trusted-fixture-css: loaded; }' })
    if (url.pathname.endsWith('.html')) {
      const entry = url.pathname.includes('failure') ? 'failure.js' : 'success.js'
      const before = url.pathname.includes('interleaved') ? `<script src="${resourcePrefix}queue-host.js"></script>` : ''
      return route.fulfill({ status: 200, contentType: 'text/html', body: `<!doctype html><html><head></head><body><main>Isolated trusted entry</main>${before}<script src="${resourcePrefix}${entry}" entry></script></body></html>` })
    }
    if (url.pathname.endsWith('/queue-host.js')) return route.fulfill({ status: 200, contentType: 'application/javascript', body: 'window.__qaQueueHostStyle();' })
    if (url.pathname.endsWith('.js')) return route.fulfill({ status: 200, contentType: 'application/javascript', body: `
      var style = document.createElement('style');
      style.textContent = ':root { --trusted-fixture-application: present; }';
      style.dataset.sandboxResource = 'application';
      document.head.appendChild(style);
      var link = document.createElement('link');
      link.rel = 'stylesheet'; link.href = '${resourcePrefix}application.css';
      document.head.appendChild(link);
      window.__qaOwnedNodes = [style, link];
      document.addEventListener('trusted-fixture-application', function () {});
      ${url.pathname.includes('failure') ? "throw new Error('expected-trusted-entry-failure');" : 'window.__MICRO_APP__ = { mount: function () {}, unmount: function () {} };'}
    ` })
    return route.fulfill({ status: 404, body: 'Unexpected sandbox fixture resource' })
  })
  try {
    await page.goto(origin)
    await page.waitForFunction(() => document.body.dataset.sandboxFixture === 'ready')
    await run({ page, requests })
    assert.deepEqual(errors, [], 'Caught entry failure must not leak an uncaught browser exception.')
  } finally { await context.close() }
}

const scenarios = {
  synchronous: () => fixture(async ({ page }) => {
    const result = await page.evaluate(async resourcePrefix => {
      const { MicroSandbox } = window.__trustedSandboxFixture
      const application = new MicroSandbox('synchronous-browser')
      const loaded = element => new Promise((resolve, reject) => { element.onload = resolve; element.onerror = reject })
      const addStyle = value => {
        const style = document.createElement('style')
        style.textContent = `:root { --${value}: present; }`
        return document.head.appendChild(style)
      }
      const addLink = name => {
        const link = document.createElement('link')
        link.rel = 'stylesheet'; link.href = resourcePrefix + name + '.css'
        const ready = loaded(link)
        document.head.appendChild(link)
        return { link, ready }
      }
      const hostBefore = addStyle('trusted-host-before')
      let applicationStyle, applicationLink
      application.run(() => {
        applicationStyle = addStyle('trusted-application')
        applicationLink = addLink('synchronous-application')
      })
      const hostStyle = addStyle('trusted-host-after')
      const hostLink = addLink('synchronous-host')
      const hostScript = document.createElement('script')
      hostScript.textContent = 'window.__qaHostScriptRuns = (window.__qaHostScriptRuns || 0) + 1'
      document.body.appendChild(hostScript)
      await Promise.all([applicationLink.ready, hostLink.ready])
      const owned = application.getStats().insertedNodes
      const applied = getComputedStyle(document.documentElement).getPropertyValue('--trusted-application').trim()
      application.teardown()
      await Promise.resolve()
      return { owned, applied,
        applicationRemoved: !applicationStyle.isConnected && !applicationLink.link.isConnected,
        hostPreserved: [hostBefore, hostStyle, hostLink.link, hostScript].every(node => node.isConnected),
        hostStyleApplied: getComputedStyle(document.documentElement).getPropertyValue('--trusted-host-after').trim(),
        applicationStyleRemoved: getComputedStyle(document.documentElement).getPropertyValue('--trusted-application').trim(),
        hostScriptRuns: window.__qaHostScriptRuns, stats: application.getStats() }
    }, resourcePrefix)
    assert.equal(result.owned, 2)
    assert.equal(result.applied, 'present')
    assert.equal(result.applicationRemoved, true)
    assert.equal(result.hostPreserved, true)
    assert.equal(result.hostStyleApplied, 'present')
    assert.equal(result.applicationStyleRemoved, '')
    assert.equal(result.hostScriptRuns, 1)
    assert.equal(result.stats.insertedNodes + result.stats.listeners + result.stats.timers, 0)
  }),

  callbacks: () => fixture(async ({ page }) => {
    const result = await page.evaluate(async () => {
      const { MicroSandbox } = window.__trustedSandboxFixture
      const application = new MicroSandbox('callback-browser'), owned = [], hosts = []
      const add = nodes => nodes.push(document.head.appendChild(document.createElement('style')))
      await new Promise(resolve => application.proxy.setTimeout(() => { add(owned); resolve() }, 0))
      add(hosts)
      await new Promise(resolve => application.proxy.requestAnimationFrame(() => { add(owned); resolve() }))
      add(hosts)
      application.proxy.addEventListener('trusted-fixture-event', () => add(owned))
      window.dispatchEvent(new Event('trusted-fixture-event'))
      add(hosts)
      await Promise.resolve()
      const tracked = application.getStats().insertedNodes
      application.teardown()
      window.dispatchEvent(new Event('trusted-fixture-event'))
      return { tracked, applicationCount: owned.length, applicationRemoved: owned.every(node => !node.isConnected),
        hostPreserved: hosts.every(node => node.isConnected), listeners: application.getStats().listeners }
    })
    assert.equal(result.tracked, 3)
    assert.equal(result.applicationCount, 3)
    assert.equal(result.applicationRemoved, true)
    assert.equal(result.hostPreserved, true)
    assert.equal(result.listeners, 0)
  }),

  nested: () => fixture(async ({ page }) => {
    const result = await page.evaluate(async () => {
      const { MicroSandbox } = window.__trustedSandboxFixture
      const outer = new MicroSandbox('outer-browser'), inner = new MicroSandbox('inner-browser')
      const nativeAdd = document.addEventListener, nodes = []
      const add = () => { const node = document.head.appendChild(document.createElement('style')); nodes.push(node); return node }
      let innerStyle
      outer.run(() => { add(); inner.run(() => { innerStyle = add() }); add() })
      await Promise.resolve()
      const counts = [outer.getStats().insertedNodes, inner.getStats().insertedNodes]
      outer.teardown()
      const innerPreserved = innerStyle.isConnected, outerRemoved = !nodes[0].isConnected && !nodes[2].isConnected
      inner.teardown()
      return { counts, innerPreserved, outerRemoved, innerRemoved: !innerStyle.isConnected,
        documentRestored: document.addEventListener === nativeAdd }
    })
    assert.deepEqual(result, { counts: [2, 1], innerPreserved: true, outerRemoved: true, innerRemoved: true, documentRestored: true })
  }),

  scriptBatches: () => fixture(async ({ page }) => {
    const result = await page.evaluate(async () => {
      const { MicroSandbox } = window.__trustedSandboxFixture
      const outer = new MicroSandbox('outer-script-browser'), inner = new MicroSandbox('inner-script-browser')
      const nativeAdd = document.addEventListener, outerNodes = [], innerNodes = [], hosts = []
      const add = nodes => nodes.push(document.head.appendChild(document.createElement('style')))
      // Both disposers are deliberately omitted, as with nested failing scripts.
      outer.beginScriptScope()
      add(outerNodes)
      inner.beginScriptScope()
      add(innerNodes)
      await Promise.resolve()
      const restoredAfterFailure = document.addEventListener === nativeAdd
      add(hosts)
      // A later event-loop turn starts a fresh batch and exits normally.
      await new Promise(resolve => setTimeout(resolve, 0))
      const leave = outer.beginScriptScope()
      add(outerNodes)
      leave()
      await Promise.resolve()
      add(hosts)
      const counts = [outer.getStats().insertedNodes, inner.getStats().insertedNodes]
      outer.teardown()
      const innerPreserved = innerNodes.every(node => node.isConnected)
      inner.teardown()
      return { counts, restoredAfterFailure, innerPreserved,
        applicationRemoved: [...outerNodes, ...innerNodes].every(node => !node.isConnected),
        hostPreserved: hosts.every(node => node.isConnected), documentRestored: document.addEventListener === nativeAdd }
    })
    assert.deepEqual(result, { counts: [2, 1], restoredAfterFailure: true, innerPreserved: true,
      applicationRemoved: true, hostPreserved: true, documentRestored: true })
  }),

  htmlSuccess: () => verifyHtmlEntry(false),
  htmlFailure: () => verifyHtmlEntry(true),
  interleavedFailure: () => verifyHtmlEntry(true, true),
}

async function verifyHtmlEntry(failed, interleaved = false) {
  return fixture(async ({ page, requests }) => {
    const result = await page.evaluate(async ({ failed, interleaved, resourcePrefix }) => {
      const { MicroSandbox, loadHtmlEntry } = window.__trustedSandboxFixture
      const application = new MicroSandbox(failed ? 'failed-html-browser' : 'html-browser')
      const nativeAdd = document.addEventListener
      const hosts = []
      application.proxy.__qaQueueHostStyle = () => Promise.resolve().then(() => {
        const host = document.createElement('style')
        host.textContent = ':root { --trusted-queued-host: present; }'
        hosts.push(document.head.appendChild(host))
      })
      const entry = await loadHtmlEntry(resourcePrefix + (interleaved ? 'interleaved-failure.html' : failed ? 'failure.html' : 'success.html'), {
        beforeScript: () => application.beginScriptScope(),
      })
      let message = null
      try { await entry.execScripts(application.proxy) } catch (error) { message = error.message }
      // Real entry evaluation and MutationObserver delivery have completed.
      // Host work in subsequent microtasks must not inherit the script scope.
      await Promise.resolve()
      const hostStyle = document.createElement('style')
      hostStyle.textContent = ':root { --trusted-host-after-script: present; }'
      hosts.push(document.head.appendChild(hostStyle))
      const hostLink = document.createElement('link')
      hostLink.rel = 'stylesheet'; hostLink.href = resourcePrefix + 'after-script-host.css'
      const loaded = new Promise((resolve, reject) => { hostLink.onload = resolve; hostLink.onerror = reject })
      hosts.push(document.head.appendChild(hostLink))
      const hostScript = document.createElement('script')
      hostScript.textContent = 'window.__qaHostScriptAfterEntry = true'
      hosts.push(document.body.appendChild(hostScript))
      await loaded
      const nodes = application.proxy.__qaOwnedNodes, stats = application.getStats()
      const lifecycleExported = typeof application.proxy.__MICRO_APP__?.mount === 'function'
      application.teardown()
      return { message, lifecycleExported, owned: stats.insertedNodes, listeners: stats.listeners,
        applicationRemoved: nodes.length === 2 && nodes.every(node => !node.isConnected),
        hostPreserved: hosts.every(node => node.isConnected), documentRestored: document.addEventListener === nativeAdd,
        queuedHostPreserved: !interleaved || hosts.length === 4 && hosts[0].isConnected,
        hostScriptExecuted: window.__qaHostScriptAfterEntry,
        hostStyleApplied: getComputedStyle(document.documentElement).getPropertyValue('--trusted-host-after-script').trim(),
        remaining: application.getStats().insertedNodes + application.getStats().listeners }
    }, { failed, interleaved, resourcePrefix })
    assert.equal(result.message, failed ? 'expected-trusted-entry-failure' : null)
    assert.equal(result.lifecycleExported, !failed)
    assert.equal(result.owned, 2)
    assert.equal(result.listeners, 1)
    assert.equal(result.applicationRemoved, true)
    assert.equal(result.hostPreserved, true)
    assert.equal(result.queuedHostPreserved, true, 'An earlier script queued host microtask must not inherit the failed later script scope.')
    assert.equal(result.documentRestored, true)
    assert.equal(result.hostScriptExecuted, true)
    assert.equal(result.hostStyleApplied, 'present')
    assert.equal(result.remaining, 0)
    assert(requests.includes(resourcePrefix + (failed ? 'failure.js' : 'success.js')), 'The actual HTML loader must fetch and evaluate the fixture entry script.')
    if (interleaved) assert(requests.includes(resourcePrefix + 'queue-host.js'))
  })
}

let passed = 0
try {
  const selector = process.argv[2] || 'all'
  assert(selector === 'all' || Object.hasOwn(scenarios, selector), `Unknown sandbox scenario ${selector}`)
  await startServer()
  browser = await launchBrowser()
  for (const [name, run] of Object.entries(scenarios)) {
    if (selector !== 'all' && selector !== name) continue
    await run()
    passed++
    console.log('PASS trusted sandbox browser: ' + name)
  }
  console.log(`Trusted sandbox browser regression: ${passed}/${selector === 'all' ? Object.keys(scenarios).length : 1} passed (production sandbox/loader, real DOM and MutationObserver; isolated HTML/CSS/script fixtures).`)
} finally {
  await browser?.close()
  if (vite && vite.exitCode === null) {
    const exited = new Promise(resolve => vite.once('exit', resolve))
    vite.kill()
    await Promise.race([exited, pause(5000)])
    if (vite.exitCode === null) vite.kill('SIGKILL')
  }
}
