/** Candidate performance observations from actual host + opaque microapps + three Rust services. */
import assert from 'node:assert/strict'
import { createHash, randomUUID } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { launchBrowser, browserName, playwrightVersion } from './browser/runtime.mjs'
import { startPhase27Services, wait, delay } from './helpers/phase27-stores-services.mjs'
let services, browser, cleanup, failure, browserVersion
const contexts = [], measurements = [], denials = []
let privateMetrics
const dataset = { workOrders: 50000, dailyReports: 50000, parentStores: 4, groups: 4, softDeletedTicketRatio: 0.05, reportDateStart: '1990-01-02', recordsPerGroup: 12500 }
async function seedRepresentativeData() {
  const actor = services.actors.ordinary
  const root = (await services.platformDb.query('SELECT o.id,o.tenant_id FROM organizations o JOIN tenants t ON t.id=o.tenant_id WHERE t.is_platform AND o.parent_id IS NULL LIMIT 1')).rows[0]
  assert(root)
  const descendant = await services.adminApi('/api/organizations','POST',{tenantId:actor.tenantId,parentId:services.organizations.A.id,name:'Phase32 representative child',code:'phase32-child'})
  const groups = [{id:services.organizations.A.id,tenant_id:actor.tenantId},{id:descendant.id,tenant_id:actor.tenantId},{id:services.organizations.B.id,tenant_id:actor.tenantId},root]
  await services.setGrants(actor,{scope:'ORG_SUBTREE'})
  await services.setGrants(actor,{appCode:'work-orders',scope:'ORG_SUBTREE'})
  for (const [index, group] of groups.entries()) {
    await services.workOrdersDb.query("INSERT INTO work_orders(id,number,tenant_id,organization_id,organization_name,owner_user_id,owner_display_name,title,description,priority,status,created_at,deleted_at) SELECT gen_random_uuid(),$4||g::text,$1,$2,'owned',$3,'owned','Representative ticket '||g,repeat('bounded workload ',8),CASE WHEN g%3=0 THEN 'HIGH' WHEN g%3=1 THEN 'LOW' ELSE 'NORMAL' END,CASE WHEN g%3=0 THEN 'RESOLVED' WHEN g%3=1 THEN 'IN_PROGRESS' ELSE 'OPEN' END,now()-g*interval '1 minute',CASE WHEN g%20=0 THEN now() ELSE NULL END FROM generate_series(1,12500) g",[group.tenant_id,group.id,actor.userId,`P32-${index}-`])
    const store = randomUUID()
    await services.businessDb.query("INSERT INTO stores(id,tenant_id,organization_id,owner_user_id,name,business_status,business_contact) VALUES($1,$2,$3,$4,'Owned benchmark store','OPEN','local')",[store,group.tenant_id,group.id,actor.userId])
    await services.businessDb.query("INSERT INTO store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount,note) SELECT gen_random_uuid(),$1,$2,$3,$4,DATE '1990-01-01'+g,(g%100000)::numeric/100,repeat('bounded workload ',8) FROM generate_series(1,12500) g",[group.tenant_id,store,group.id,actor.userId])
  }
  for (const [db,table] of [[services.workOrdersDb,'work_orders'],[services.businessDb,'store_daily_reports']]) { await db.query(`ANALYZE ${table}`); assert.equal((await db.query(`SELECT count(*)::int AS count FROM ${table}`)).rows[0].count,50000) }
}
try {
  services = await startPhase27Services(); await seedRepresentativeData(); browser = await launchBrowser(); browserVersion = browser.version()
  for (const app of ['stores', 'work-orders']) {
    const actor = app === 'stores' ? services.actors.ordinary : await services.authorize(services.actors.ordinary, 'A', app)
    const context = await browser.newContext({ viewport: { width: 1540, height: 1120 }, deviceScaleFactor: 1 }); contexts.push(context)
    await context.addInitScript(({ session, refreshToken, applicationId }) => {
      window.__phase32 = { lcp: [], interactions: [], timing: [], eventTimingSupported: PerformanceObserver.supportedEntryTypes.includes('event'), lcpSupported: PerformanceObserver.supportedEntryTypes.includes('largest-contentful-paint') }
      try { new PerformanceObserver(list => { for (const e of list.getEntries()) window.__phase32.lcp.push(e.startTime) }).observe({ type: 'largest-contentful-paint', buffered: true }) } catch {}
      try { new PerformanceObserver(list => { for (const e of list.getEntries()) if (e.interactionId) window.__phase32.interactions.push({ id: e.interactionId, duration: e.duration }) }).observe({ type: 'event', buffered: true, durationThreshold: 16 }) } catch {}
      addEventListener('saas:micro-timing', e => window.__phase32.timing.push(e.detail))
      if (window.top !== window || sessionStorage.getItem('phase32-seeded')) return
      localStorage.setItem('platform-session-v1', JSON.stringify({ schema: 1, version: 1, sessionId: crypto.randomUUID(), status: 'active', token: session, refreshToken }))
      localStorage.setItem('platform-current-app', applicationId); localStorage.setItem('platform-theme', 'light'); sessionStorage.setItem('phase32-seeded', 'true')
    }, { session: actor.session, refreshToken: actor.sessionResponse.refreshToken, applicationId: services.apps[app].application.id })
    const page = await context.newPage(); page.setDefaultTimeout(30000)
    const cdp = await context.newCDPSession(page)
    await cdp.send('Network.enable'); await cdp.send('Network.emulateNetworkConditions', { offline: false, latency: 20, downloadThroughput: 10 * 1024 * 1024 / 8, uploadThroughput: 5 * 1024 * 1024 / 8 }); await cdp.send('Emulation.setCPUThrottlingRate', { rate: 1 })
    for (const cache of ['cold-context', 'warm-context']) {
      await page.goto(services.origin + '/micro/' + app, { waitUntil: 'domcontentloaded' })
      await page.getByTestId('micro-status-bar').getByText('运行中', { exact: true }).waitFor()
      const frame = page.frameLocator('.micro-mount-zone iframe')
      await frame.getByTestId(app === 'stores' ? 'stores-app' : 'work-orders-app').waitFor()
      // Genuine browser input on the microapp refresh button and host navigation.
      const buttons = frame.getByRole('button', { name: /刷新|查询|搜索/ })
      assert(await buttons.count() > 0, 'actual microapp refresh control required')
      await buttons.first().click(); await delay(300)
      await page.locator('body').click({ position: { x: 4, y: 4 } }); await delay(300)
      const host = await page.evaluate(() => window.__phase32)
      const child = page.frames().find(f => f !== page.mainFrame() && /sub-apps/.test(f.url()))
      assert(child, 'actual microapp frame required')
      const micro = await child.evaluate(() => window.__phase32)
      assert(host.eventTimingSupported && host.lcpSupported && micro.eventTimingSupported && micro.lcpSupported, 'Native performance observers must be supported')
      assert(host.timing.some(e => e.stage === 'entry') && host.timing.some(e => e.stage === 'ready') && host.timing.some(e => e.stage === 'mount'), 'entry/ready/mount must come from real lifecycle')
      const lcp = Math.max(...host.lcp, 0), eventDurations = [...host.interactions, ...micro.interactions].map(e => e.duration)
      // Event Timing rounds to 8 ms; events shorter than 16 ms are omitted.
      const inp = eventDurations.length ? Math.max(...eventDurations) : 16
      measurements.push({ app, cache, lcpMs: lcp, observedInteractionUpperBoundMs: inp, eventTimingSamples: eventDurations.length, hostLcpEntries: host.lcp.length, microLcpMs: Math.max(...micro.lcp, 0), lifecycle: host.timing, candidateLcpPass: lcp > 0 && lcp <= 2500, candidateInteractionPass: inp <= 200 })
    }
    if (app === 'stores') {
      await services.setGrants(actor, { remove: ['store:read'] })
      const start = performance.now()
      assert.equal((await services.businessCall(actor, '/stores-api/stores')).status, 403, 'fresh post-revocation request must deny')
      let awarenessMs = null, awarenessSignal = null
      await wait(async () => {
        const text = await page.getByTestId('micro-status-bar').innerText()
        if (/权限|失效|不可|故障|失败|受限/.test(text)) { awarenessSignal = 'host-runtime-status'; return true }
        const frame = page.frameLocator('.micro-mount-zone iframe')
        if (await frame.getByText('当前授权没有此项读取权限。', { exact: false }).isVisible().catch(() => false)) { awarenessSignal = 'microapp-current-grant-denial'; return true }
        return false
      }, 'real browser revocation awareness', 22000).then(() => { awarenessMs = performance.now() - start }, () => {})
      measurements.push({ app, kind: 'revocation', nextRequest: 403, awarenessMs, awarenessSignal, observationWindowMs: 22000, candidatePass: awarenessMs !== null && awarenessMs <= 20000 })
      await services.setGrants(actor)
    }
  }
  for (const endpoint of ['/api/internal/metrics', '/stores-api/internal/metrics', '/work-orders-api/internal/metrics', '/notification-api/metrics', '/internal/metrics', '/metrics', '/socket.io/metrics']) {
    const response = await services.request(services.origin, endpoint, { token: 'phase32-public-canary' })
    assert(!response.text.includes('# TYPE ') && [401,403,404].includes(response.status), 'public gateway must reject private metrics')
    denials.push({ path: endpoint, status: response.status })
  }
  privateMetrics = await services.observePrivateMetrics()
  for (const stage of ['entry', 'ready', 'mount']) assert(privateMetrics[0].metrics.includes(`stage="${stage}",outcome="ok"`), 'actual runtime timing must reach private platform metrics')
} catch (error) { failure = error }
finally {
  for (const context of contexts) await context.close().catch(() => {})
  if (browser) await browser.close()
  if (services) cleanup = await services.close({ browserClosed: true })
}
if (failure) throw failure
assert(cleanup && Object.values(cleanup).every(value => value === true))
console.log('PHASE32_BROWSER_RESULT ' + JSON.stringify({ schemaVersion: 1, status: 'PASS', product: browserName, browserVersion, playwrightVersion, source: Object.fromEntries(['platformBinary','businessBinary','workOrdersBinary','adminBinary'].map(key => [key, services.source[key]])), producerSha256: createHash('sha256').update(readFileSync(fileURLToPath(import.meta.url))).digest('hex'), profile: { viewport: [1540,1120], cpuRate: 1, latencyMs: 20, downloadMbps: 10, uploadMbps: 5, networkScope: 'top-level CDP target; backend-to-platform private loopback unshaped; separate iframe target inheritance not independently enforced', tls: 'owned trusted CA, actual Nginx', sampleCount: 4, scope: 'lab navigation and scripted real input; Event Timing max for short interaction sequence, not field INP or production Core Web Vitals' }, dataset, measurements, publicMetricsDenials: denials, privateMetrics, cleanup }))
