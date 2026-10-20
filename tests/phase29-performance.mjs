/** Cold production navigation, actual gzip HTTP bytes and Event Timing; no backend required. */
import assert from 'node:assert/strict'
import { writeFileSync, existsSync } from 'node:fs'
import path from 'node:path'
import { launchBrowser, playwrightVersion } from './browser/runtime.mjs'
import { servePhase29 } from './browser/phase29-server.mjs'
const label = process.argv[2]
assert(['before', 'after'].includes(label))
const dir = path.resolve('target/phase-29', label), dist = path.join(dir, 'dist')
const reportName = process.argv[3] || 'performance.json'
assert(/^performance(?:-[a-z0-9-]+)?\.json$/.test(reportName))
const reportFile = path.join(dir, reportName)
assert(!existsSync(reportFile), 'Performance evidence exists; choose a fresh report name')
const server = await servePhase29(dist)
const { origin } = server
let browser
try {
  browser = await launchBrowser()
  const samples = []
  for (let i = 0; i < 5; i++) {
    const context = await browser.newContext({ viewport: { width: 1365, height: 900 }, reducedMotion: 'reduce' })
    const page = await context.newPage(), errors = []
    page.on('pageerror', e => errors.push(e.message))
    const cdp = await context.newCDPSession(page)
    await cdp.send('Network.enable')
    await cdp.send('Network.emulateNetworkConditions', { offline: false, latency: 40, downloadThroughput: 1600000 / 8, uploadThroughput: 750000 / 8 })
    await cdp.send('Emulation.setCPUThrottlingRate', { rate: 4 })
    await context.addInitScript(() => {
      window.__p29 = { lcp: [], events: [] }
      new PerformanceObserver(list => { for (const e of list.getEntries()) window.__p29.lcp.push({ startTime: e.startTime, size: e.size }) }).observe({ type: 'largest-contentful-paint', buffered: true })
      new PerformanceObserver(list => { for (const e of list.getEntries()) if (e.interactionId) window.__p29.events.push({ name: e.name, duration: e.duration, interactionId: e.interactionId, startTime: e.startTime }) }).observe({ type: 'event', buffered: true, durationThreshold: 16 })
    })
    await page.goto(origin + '/login', { waitUntil: 'networkidle' })
    await page.locator('input[name="username"]').waitFor()
    await page.evaluate(() => document.fonts.ready)
    const initial = await page.evaluate(() => ({ navigation: performance.getEntriesByType('navigation').map(e => e.toJSON()),
      resources: performance.getEntriesByType('resource').map(e => e.toJSON()), lcp: window.__p29.lcp }))
    await page.locator('input[name="username"]').pressSequentially('phase29-user', { delay: 75 })
    await page.locator('input[name="password"]').pressSequentially('fixture-password', { delay: 75 })
    await page.keyboard.press('Tab')
    await page.waitForTimeout(150)
    const events = await page.evaluate(() => window.__p29.events)
    assert.deepEqual(errors, [])
    const resources = [...initial.navigation, ...initial.resources]
    samples.push({ run: i + 1, ...initial, events,
      initialTransferBytes: resources.reduce((sum, e) => sum + e.transferSize, 0),
      initialEncodedBodyBytes: resources.reduce((sum, e) => sum + e.encodedBodySize, 0),
      lcpMs: initial.lcp.at(-1)?.startTime ?? null,
      labInteractionMs: events.length ? Math.max(...events.map(e => e.duration)) : null,
      interactionNote: 'Event Timing max for controlled login typing; null means all observed interactions below 16ms reporting threshold. This is a lab INP proxy, not field INP.' })
    await context.close()
  }
  const report = { label, browser: browser.version(), playwrightVersion, node: process.version,
    method: { coldContexts: 5, viewport: '1365x900', cpuThrottle: 4, latencyMs: 40, downBitsPerSecond: 1600000, gzip: true, tls: false }, samples }
  writeFileSync(reportFile, JSON.stringify(report, null, 2) + '\n', { flag: 'wx' })
  console.log(JSON.stringify(samples.map(({ run, initialTransferBytes, initialEncodedBodyBytes, lcpMs, labInteractionMs }) => ({ run, initialTransferBytes, initialEncodedBodyBytes, lcpMs, labInteractionMs }))))
  const missing = await fetch(origin + '/assets/obsolete-phase29.js')
  assert.equal(missing.status, 404); assert(!missing.headers.get('content-type').includes('html'))
} finally { await browser?.close(); await server.close() }
