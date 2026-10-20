/** Real image-delivery acceptance. The outer owner controls Docker; this tool does not. */
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { applicationCodes, phase28DeliveryChecks, startPhase28Services, wait } from './helpers/phase28-delivery-services.mjs'

const sha = value => createHash('sha256').update(value).digest('hex')
const args = process.argv.slice(2)
assert(args.length === 1 && ['--prepare', '--browser'].includes(args[0]), 'Select --prepare or --browser explicitly')
const checks = [], evidence = {}, contexts = [], operations = [], assets = [], native = new Map(), headerTasks = new Set(), errors = []
let services, browser, product, playwrightVersion, failure, cleanup, browserClosed = false
function check(id, proof) {
  assert(phase28DeliveryChecks.includes(id) && !checks.some(value => value.id === id), 'Unexpected or duplicate Phase28 browser check')
  checks.push({ id, status: 'PASS' }); evidence[id] = proof
  console.log('PASS phase28 browser ' + id)
}
const frameFor = page => page.frameLocator('.micro-mount-zone iframe')
const form = (view, kind) => view.frame.locator(`form[data-form-kind="${kind}"]`)
async function navigate(view, pathname) {
  await view.page.evaluate(next => { history.pushState(null, '', next); dispatchEvent(new PopStateEvent('popstate')) }, pathname)
  await view.page.waitForURL(url => url.pathname === pathname)
}
async function seed(context, actor, code = 'stores') {
  await context.addInitScript(({ session, refreshToken, applicationId }) => {
    if (window !== window.top || sessionStorage.getItem('phase28-seeded')) return
    localStorage.setItem('platform-session-v1', JSON.stringify({ schema: 1, version: 1, sessionId: crypto.randomUUID(), status: 'active', token: session, refreshToken }))
    localStorage.setItem('platform-current-app', applicationId); localStorage.setItem('platform-theme', 'light')
    sessionStorage.setItem('phase28-seeded', 'true')
  }, { session: actor.session, refreshToken: actor.sessionResponse.refreshToken, applicationId: services.apps[code].application.id })
}
async function observe(context) {
  await context.exposeBinding('__phase28Fetch', (_source, value) => {
    const record = native.get(value.requestId) || { requestId: value.requestId }
    native.set(value.requestId, record)
    if (value.phase === 'start') { assert(!record.started); Object.assign(record, value, { started: true }) }
    else Object.assign(record, value)
  })
  await context.addInitScript(() => {
    if (window === window.top) return
    const actual = window.fetch.bind(window)
    window.fetch = function (input, options) {
      let observation
      try {
        const target = new URL(typeof input === 'string' || input instanceof URL ? input : input.url, location.href)
        if (/^\/(?:stores|work-orders)-api\//.test(target.pathname)) {
          const headers = new Headers(options?.headers || (input instanceof Request ? input.headers : undefined))
          observation = { requestId: headers.get('x-request-id'), method: options?.method || (input instanceof Request ? input.method : 'GET'),
            path: target.pathname, targetOrigin: target.origin, credentials: options?.credentials || (input instanceof Request ? input.credentials : 'same-origin'),
            cookiePresent: headers.has('cookie'), bearerPresent: /^Bearer .+/.test(headers.get('authorization') || '') }
          void window.__phase28Fetch({ phase: 'start', ...observation }).catch(() => {})
        }
      } catch {}
      const outcome = actual(input, options)
      if (observation) outcome.then(() => window.__phase28Fetch({ phase: 'settled', requestId: observation.requestId, nativeOutcome: 'fulfilled' }),
        error => window.__phase28Fetch({ phase: 'settled', requestId: observation.requestId, nativeOutcome: 'rejected', nativeErrorName: error?.name || 'Error' })).catch(() => {})
      return outcome
    }
  })
  await context.addCookies([{ name: 'phase28-cookie-must-not-reach-business', value: 'opaque-observer', url: services.origin, httpOnly: true, secure: true }])
}
async function open(actor, code, pathname = '/micro/' + code) {
  const context = await browser.newContext({ viewport: { width: 1540, height: 1120 }, acceptDownloads: true }); contexts.push(context)
  await seed(context, actor, code); await observe(context)
  const page = await context.newPage(); page.setDefaultTimeout(30_000)
  page.on('pageerror', error => errors.push(error.message))
  page.on('request', request => {
    const url = new URL(request.url())
    if (!/^\/(?:stores|work-orders)-api\//.test(url.pathname)) return
    const record = { method: request.method(), path: url.pathname, targetOrigin: url.origin, responseStatus: null, outcome: null }
    operations.push(record); request.__phase28 = record
    const task = request.allHeaders().then(headers => {
      record.requestId = headers['x-request-id']; record.origin = headers.origin ?? null
      record.cookiePresent = !!headers.cookie; record.bearerPresent = /^Bearer .+/.test(headers.authorization || '')
      try { record.frameType = request.frame() === page.mainFrame() ? 'host' : 'iframe'; record.frameUrl = request.frame().url() } catch { record.frameType = 'retired' }
    }); headerTasks.add(task); task.finally(() => headerTasks.delete(task)).catch(() => {})
  })
  page.on('requestfinished', request => { if (request.__phase28) request.__phase28.outcome = 'finished' })
  page.on('requestfailed', request => { if (request.__phase28) request.__phase28.outcome = 'failed' })
  page.on('response', response => {
    if (response.request().__phase28) response.request().__phase28.responseStatus = response.status()
    const url = new URL(response.url())
    if (/^\/sub-apps\/(?:stores|work-orders)\/versions\//.test(url.pathname)) assets.push({ origin: url.origin, path: url.pathname, status: response.status(),
      contentType: response.headers()['content-type'], allowOrigin: response.headers()['access-control-allow-origin'] })
  })
  await page.goto(services.origin + pathname, { waitUntil: 'domcontentloaded' })
  return { page, context, frame: frameFor(page), code }
}
async function active(view, candidate) {
  await view.page.getByTestId('micro-status-bar').getByText('运行中', { exact: true }).waitFor()
  await view.frame.getByTestId(view.code === 'stores' ? 'stores-app' : 'work-orders-app').waitFor()
  const iframe = view.page.locator('.micro-mount-zone iframe'), sandbox = await iframe.getAttribute('sandbox')
  assert(sandbox.includes('allow-scripts') && !sandbox.includes('allow-same-origin'))
  const entry = new URL(candidate.release.entryUrl, candidate.artifact.publicOrigin)
  await wait(async () => {
    const src = await iframe.getAttribute('src')
    return !!src && new URL(src, services.origin).href === entry.href
  }, 'The host must load the selected immutable release URL')
  const files = [candidate.buildInfo.entry, ...candidate.buildInfo.dynamicChunks]
  assert(files.length > 1 && candidate.buildInfo.dynamicChunks.length > 0, 'Each actual Vite build must contain a lazy business chunk')
  await wait(() => files.every(file => assets.some(asset => asset.origin === entry.origin && asset.path === new URL(file, entry).pathname && asset.status === 200 && ['*', 'null'].includes(asset.allowOrigin))),
    'Actual opaque ESM entry and lazy chunks must load from the selected version')
  return { sandbox, entryOrigin: entry.origin, entryPath: entry.pathname, moduleEntry: candidate.buildInfo.entry, dynamicChunks: candidate.buildInfo.dynamicChunks }
}
async function screenshot(page, name) {
  const directory = services.runtime.screenshotsDir || path.join(services.runtime.reportDir, 'screenshots')
  assert(path.isAbsolute(directory)); mkdirSync(directory, { recursive: true })
  const destination = path.join(directory, name + '.png'); assert(!existsSync(destination))
  await page.screenshot({ path: destination, fullPage: true })
  const bytes = readFileSync(destination)
  ;(evidence.screenshots ||= []).push({ path: destination, sha256: sha(bytes), bytes: bytes.length })
}
const releasePath = code => '/api/applications/' + services.apps[code].application.id + '/releases'
async function panel(code) {
  const view = await open(services.admin, code, '/system/applications')
  const card = view.page.locator('div.group.relative').filter({ has: view.page.getByText(services.apps[code].application.name, { exact: true }) })
  await card.getByRole('button', { name: '发布版本', exact: true }).click()
  const dialog = view.page.getByRole('dialog', { name: services.apps[code].application.name + ' · 发布版本', exact: true })
  await dialog.waitFor()
  await dialog.getByRole('combobox', { name: '发布环境' }).click()
  await view.page.getByRole('option', { name: '生产 · production', exact: true }).click()
  await dialog.getByText(/不拉取镜像、启停容器或生成 Nginx 规则/).waitFor()
  return { ...view, dialog }
}
function article(view, version) { return view.dialog.locator('article').filter({ has: view.page.getByRole('heading', { name: version, exact: true }) }) }
async function register(view, candidate) {
  const release = candidate.release
  await view.dialog.getByRole('button', { name: '登记已部署 URL', exact: true }).click()
  const region = view.dialog.getByRole('region', { name: '登记已部署 URL', exact: true })
  await region.getByLabel(/版本号/).fill(release.version)
  await region.getByLabel(/HTML 入口 URL/).fill(release.entryUrl)
  if (release.healthUrl) await region.getByLabel(/健康 URL/).fill(release.healthUrl)
  for (const [index, resource] of release.resources.entries()) {
    await region.getByRole('button', { name: '添加资源', exact: true }).click()
    await region.getByLabel(`资源 ${index + 1} URL`, { exact: true }).fill(resource.url)
    if (resource.kind !== 'SCRIPT') {
      await region.getByLabel(`资源 ${index + 1} 类型`, { exact: true }).click()
      await view.page.getByRole('option', { name: resource.kind === 'STYLE' ? '样式' : '资源', exact: true }).click()
    }
  }
  const pending = view.page.waitForResponse(response => new URL(response.url()).pathname === releasePath(view.code) && response.request().method() === 'POST')
  await region.getByRole('button', { name: '登记 URL', exact: true }).click()
  const response = await pending; assert.equal(response.status(), 200)
  const sent = JSON.parse(response.request().postData())
  assert.deepEqual(Object.keys(sent).sort(), ['entryType', 'entryUrl', 'environment', 'healthUrl', 'resources', 'version'].sort())
  assert.equal(sent.entryType, 'HTML'); assert.equal(sent.entryUrl, release.entryUrl)
  assert.deepEqual(sent.resources, release.resources)
  const saved = await response.json(); assert.equal(saved.validationStatus, 'DRAFT')
  await article(view, release.version).getByText('待预检', { exact: true }).waitFor()
  return saved
}
async function preflight(view, candidate, expected = 'PASSED') {
  const row = article(view, candidate.release.version)
  const pending = view.page.waitForResponse(response => new URL(response.url()).pathname.startsWith(releasePath(view.code) + '/') && new URL(response.url()).pathname.endsWith('/preflight') && response.request().method() === 'POST')
  await row.getByRole('button', { name: /^(?:预检|重新预检)$/ }).click()
  const response = await pending; assert.equal(response.status(), 200)
  const saved = await response.json(); assert.equal(saved.validationStatus, expected)
  return { httpStatus: response.status(), release: saved }
}
async function activate(view, candidate, expected = 200) {
  await article(view, candidate.release.version).getByRole('button', { name: '激活', exact: true }).click()
  const pending = view.page.waitForResponse(response => new URL(response.url()).pathname.startsWith(releasePath(view.code) + '/') && new URL(response.url()).pathname.endsWith('/activate') && response.request().method() === 'POST')
  await view.page.getByRole('alertdialog').getByRole('button', { name: '确认激活', exact: true }).click()
  const response = await pending; assert.equal(response.status(), expected)
  if (expected === 200) await article(view, candidate.release.version).getByText('当前部署', { exact: true }).waitFor()
  else await view.page.keyboard.press('Escape')
  return { status: response.status(), body: await response.json() }
}
async function clickBusiness(view, action, endpoint, method, expected) {
  const pending = view.page.waitForResponse(response => new URL(response.url()).pathname === endpoint && response.request().method() === method)
  await action(); const response = await pending
  if (response.status() !== expected) {
    let code = 'UNAVAILABLE'
    try { const body = await response.json(); if (/^[A-Z][A-Z0-9_]{0,63}$/.test(body?.code || '')) code = body.code } catch {}
    assert.equal(response.status(), expected, `Actual Rust ${method} ${endpoint} returned ${response.status()} (${code}); expected ${expected}`)
  }
  return [401, 403].includes(expected) ? null : response.json()
}
async function storesCrud(view, label, existingStoreId) {
  let store
  if (existingStoreId) {
    // The business contract permits exactly one store per tenant/organization.
    // A static release upgrade must continue operating on that same record.
    store = await clickBusiness(view, () => navigate(view, '/micro/stores/stores/' + existingStoreId), '/stores-api/stores/' + existingStoreId, 'GET', 200)
    assert.equal(store.id, existingStoreId)
  } else {
    await navigate(view, '/micro/stores/store-create')
    const create = form(view, 'store-create')
    await create.getByLabel('门店名称').fill(label)
    store = await clickBusiness(view, () => create.getByRole('button', { name: '创建门店', exact: true }).click(), '/stores-api/stores', 'POST', 201)
    await navigate(view, '/micro/stores/stores/' + store.id)
  }
  assert.equal(store.organizationId, services.actors.ordinary.organizationId)
  const edit = form(view, 'store-edit'); await edit.getByLabel('门店名称').fill(label + ' 已更新')
  const updated = await clickBusiness(view, () => edit.getByRole('button', { name: '保存门店', exact: true }).click(), '/stores-api/stores/' + store.id, 'PATCH', 200)
  assert.equal(updated.name, label + ' 已更新')
  await navigate(view, '/micro/stores/staff-assign'); const assign = form(view, 'staff-assign')
  await assign.getByLabel('门店ID').fill(store.id); await assign.getByLabel('成员搜索').fill('peer')
  await assign.getByRole('button', { name: '搜索成员', exact: true }).click()
  await assign.getByRole('button', { name: '选择 Phase28 peer', exact: true }).click()
  const staff = await clickBusiness(view, () => assign.getByRole('button', { name: '关联员工', exact: true }).click(), '/stores-api/stores/' + store.id + '/staff', 'POST', 201)
  await navigate(view, '/micro/stores/staff'); const query = form(view, 'staff-query'); await query.getByLabel('门店ID').fill(store.id)
  await clickBusiness(view, () => query.getByRole('button', { name: '查询员工', exact: true }).click(), '/stores-api/stores/' + store.id + '/staff', 'GET', 200)
  await clickBusiness(view, () => view.frame.locator(`[data-row-id="${staff.id}"]`).getByRole('button', { name: '解除关联', exact: true }).click(), '/stores-api/stores/' + store.id + '/staff/' + staff.id, 'DELETE', 200)
  assert.equal((await services.businessDb.query('SELECT name FROM stores WHERE id=$1', [store.id])).rows[0].name, label + ' 已更新')
  return { storeId: store.id, staffId: staff.id, storeCreated: existingStoreId ? null : 201, storeReused: !!existingStoreId,
    staffCreated: 201, read: 200, updated: 200, unassigned: 200, actualDatabase: true }
}
async function workOrdersCrud(view, label) {
  await navigate(view, '/micro/work-orders/ticket-create')
  await view.frame.getByTestId('ticket-title').fill(label)
  const ticket = await clickBusiness(view, () => view.frame.getByTestId('save-ticket').click(), '/work-orders-api/tickets', 'POST', 201)
  await navigate(view, '/micro/work-orders/tickets/' + ticket.id)
  await view.frame.getByTestId('ticket-detail').waitFor()
  const updated = await clickBusiness(view, () => view.frame.getByTestId('change-ticket-status').click(), '/work-orders-api/tickets/' + ticket.id, 'PATCH', 200)
  assert.equal(updated.status, 'IN_PROGRESS')
  await view.frame.getByTestId('delete-ticket').click()
  await clickBusiness(view, () => view.frame.getByTestId('confirm-delete').click(), '/work-orders-api/tickets/' + ticket.id, 'DELETE', 200)
  assert.equal((await services.workOrdersDb.query('SELECT deleted_at IS NOT NULL AS removed FROM work_orders WHERE id=$1', [ticket.id])).rows[0].removed, true)
  return { ticketId: ticket.id, created: 201, read: 200, updated: 200, deleted: 200, actualDatabase: true }
}
async function inventory() {
  const proof = []
  for (const code of applicationCodes) {
    for (const kind of ['v1', 'v2']) {
      const candidate = services.readCandidate(code, kind), flags = ['--release-file', candidate.releaseFile, '--artifact-file', candidate.artifactFile, '--inventory-dir', services.runtime.inventoryDir]
      const plan = await services.runCli(code, ['artifact-bind', ...flags], { offline: true }); assert.equal(plan.localWrites, false)
      const bound = await services.runCli(code, ['artifact-bind', ...flags, '--apply'], { offline: true }); assert.equal(bound.bindingState, 'BOUND')
      const verified = await services.runCli(code, ['artifact-verify', ...flags], { offline: true }); assert.equal(verified.bindingState, 'BOUND')
      assert.equal(verified.remoteDigestStored, false)
    }
    const candidate = services.readCandidate(code, 'v1')
    const stagingFile = path.join(services.runtime.workDir, code + '-staging-release.json')
    const stagingArtifact = path.join(services.runtime.workDir, code + '-staging-artifact.json')
    const relocate = value => typeof value === 'string' && value.startsWith(candidate.artifact.publicOrigin + '/') ? services.externalOrigin + value.slice(candidate.artifact.publicOrigin.length) : value
    writeFileSync(stagingFile, JSON.stringify({ ...candidate.release, environment: 'staging', entryUrl: relocate(candidate.release.entryUrl),
      healthUrl: relocate(candidate.release.healthUrl), resources: candidate.release.resources.map(resource => ({ ...resource, url: relocate(resource.url) })) }) + '\n', { mode: 0o600, flag: 'wx' })
    writeFileSync(stagingArtifact, JSON.stringify({ ...candidate.artifact, publicOrigin: services.externalOrigin }) + '\n', { mode: 0o600, flag: 'wx' })
    const flags = ['--release-file', stagingFile, '--artifact-file', stagingArtifact, '--inventory-dir', services.runtime.inventoryDir]
    await services.runCli(code, ['artifact-bind', ...flags, '--apply'], { offline: true, environment: 'staging' })
    await services.runCli(code, ['artifact-verify', ...flags], { offline: true, environment: 'staging' })
    const rejections = []
    for (const mutation of ['source', 'digest']) {
      const changed = structuredClone(candidate.artifact)
      if (mutation === 'source') changed.source.identity = changed.source.identity.replace('@sha256:', '-changed-source@sha256:')
      else { changed.digest = 'sha256:' + (changed.digest.endsWith('a'.repeat(64)) ? 'b' : 'a').repeat(64); changed.source.identity = changed.source.identity.replace(/sha256:[a-f0-9]{64}$/, changed.digest) }
      const file = path.join(services.runtime.workDir, code + '-' + mutation + '-conflict.json'); writeFileSync(file, JSON.stringify(changed) + '\n', { mode: 0o600, flag: 'wx' })
      const actual = await services.runCli(code, ['register', '--release-file', candidate.releaseFile, '--artifact-file', file, '--inventory-dir', services.runtime.inventoryDir, '--apply'], { expected: 1 })
      assert.equal(actual.category, 'ARTIFACT_CONFLICT'); rejections.push({ mutation, category: actual.category, actualRejectionLayer: 'Rust CLI persistent artifact inventory before remote API' })
    }
    proof.push({ applicationCode: code, versions: ['v1','v2'], durableAcrossCliProcesses: true, sameArtifactAcrossEnvironments: ['production','staging'], rejections, remoteDigestStored: false })
  }
  return proof
}
try {
  services = await startPhase28Services({ prepare: args[0] === '--prepare' })
  if (args[0] === '--prepare') {
    await services.close(); console.log('PHASE28_PREPARE ' + JSON.stringify(services.summary))
  } else {
    const runtime = await import('./browser/runtime.mjs'); product = runtime.browserName; playwrightVersion = runtime.playwrightVersion
    browser = await runtime.launchBrowser()
    const inventoryProof = await inventory()
    const management = {}, candidates = {}, v1Views = {}, v2Views = {}, v1Proof = {}, v2Proof = {}, ordinaryProof = {}
    for (const code of applicationCodes) {
      candidates[code] = { v1: services.readCandidate(code, 'v1'), v2: services.readCandidate(code, 'v2') }
      management[code] = await panel(code)
      const draft = await register(management[code], candidates[code].v1)
      await preflight(management[code], candidates[code].v1)
      await activate(management[code], candidates[code].v1)
      evidence[code + '-v1-release'] = { id: draft.id, draft: 'DRAFT', preflight: 'PASSED', active: true }
      const actor = code === 'stores' ? services.actors.ordinary : await services.authorize(services.state.actors.ordinary, code)
      v1Views[code] = await open(actor, code)
      v1Proof[code] = await active(v1Views[code], candidates[code].v1)
      ordinaryProof[code] = code === 'stores' ? await storesCrud(v1Views[code], 'Phase28 v1 实际门店') : await workOrdersCrud(v1Views[code], 'Phase28 v1 实际工单')
    }
    check('url-ui-draft-preflight-activate', Object.fromEntries(applicationCodes.map(code => [code, evidence[code + '-v1-release']])))
    const denied = []
    for (const code of applicationCodes) {
      // App tokens establish identity within the organization's app boundary.
      // This user still has no function permissions or membership data grants.
      const actor = await services.authorize(services.state.actors.unauthorized, code)
      const context = await services.platformApi('/api/apps/' + code + '/context', { token: actor.token })
      assert.deepEqual(context.permissions, []); assert.deepEqual(context.dataGrants, []); assert.deepEqual(context.menus, [])
      const endpoint = code === 'stores' ? '/stores-api/stores' : '/work-orders-api/tickets'
      const protectedResponse = await services.businessCall(actor, endpoint)
      assert.equal(protectedResponse.status, 403)
      const direct = await services.request(services.origin, endpoint)
      assert.equal(direct.status, 401)
      denied.push({ applicationCode: code, applicationTokenStatus: 201, permissions: 0, dataGrants: 0, menus: 0,
        ungrantedBusinessStatus: protectedResponse.status, anonymousBusinessStatus: direct.status })
    }
    check('image-deployment-does-not-grant', { actorsAreNonSuperusers: true, denials: denied, automaticGrantCount: 0 })
    const badProof = []
    for (const [kind, key] of [['entry','badEntry'],['resource','badResource'],['health','badHealth']]) {
      const candidate = services.readCandidate('stores', key)
      const row = await register(management.stores, candidate), failed = await preflight(management.stores, candidate, 'FAILED')
      assert.equal(await article(management.stores, candidate.release.version).getByRole('button', { name: '激活', exact: true }).isDisabled(), true)
      const response = await services.request(services.origin, releasePath('stores') + '/' + row.id + '/activate', { token: services.admin.session, method: 'POST' })
      assert.equal(response.status, 409)
      assert(failed.release.validationResult.checks.some(item => item.passed === false))
      badProof.push({ kind, preflightHttpStatus: failed.httpStatus, validationStatus: failed.release.validationStatus, uiActivationDisabled: true, serverActivationStatus: response.status,
        checks: failed.release.validationResult.checks.map(item => ({ kind: item.kind, passed: item.passed, statusCode: item.statusCode ?? null })) })
    }
    check('preflight-http200-failed-rejected', badProof)
    for (const code of applicationCodes) { await register(management[code], candidates[code].v2); await preflight(management[code], candidates[code].v2) }
    const signals = services.runtime.faultSignals
    assert(signals && Object.values(signals).every(value => path.isAbsolute(value)), 'Outer owner must provide the closed stop/start fault handshake')
    writeFileSync(signals.request, JSON.stringify({ appCode: 'stores', action: 'stop' }) + '\n', { mode: 0o600, flag: 'wx' })
    await wait(() => existsSync(signals.ready), 'Outer owner must stop only the owned stores-v2 container', 90_000)
    assert.equal(JSON.parse(readFileSync(signals.ready, 'utf8')).stopped, true)
    const rejected = await activate(management.stores, candidates.stores.v2, 409)
    const deployments = await services.adminApi('/api/applications/' + services.apps.stores.application.id + '/deployments')
    assert.equal(deployments.find(value => value.environment === 'production').version, candidates.stores.v1.release.version)
    writeFileSync(signals.restore, JSON.stringify({ appCode: 'stores', action: 'start' }) + '\n', { mode: 0o600, flag: 'wx' })
    await wait(() => existsSync(signals.restored), 'Outer owner must restore and health-check the same immutable container', 90_000)
    assert.equal(JSON.parse(readFileSync(signals.restored, 'utf8')).healthy, true)
    await preflight(management.stores, candidates.stores.v2)
    check('activate-rechecks-unavailable-entry', { earlierValidation: 'PASSED', actualEntryContainerStopped: true, activationStatus: rejected.status,
      activeVersionPreserved: candidates.stores.v1.release.version, sameContainerRestoredHealthy: true })
    for (const code of applicationCodes) {
      await activate(management[code], candidates[code].v2)
      const actor = code === 'stores' ? services.actors.ordinary : await services.authorize(services.state.actors.ordinary, code)
      v2Views[code] = await open(actor, code); v2Proof[code] = await active(v2Views[code], candidates[code].v2)
      assert.equal(v2Proof[code].entryOrigin, services.externalOrigin)
      ordinaryProof[code + '-external'] = code === 'stores' ? await storesCrud(v2Views[code], 'Phase28 外部入口真实门店', ordinaryProof.stores.storeId) : await workOrdersCrud(v2Views[code], 'Phase28 外部入口真实工单')
    }
    check('ordinary-user-sdk-rust-authorization', { actorsNonSuperuser: true, server: 'real Rust platform + stores + work-orders', operations: ordinaryProof })
    const external = []
    for (const code of applicationCodes) {
      const candidate = candidates[code].v2, entry = new URL(candidate.release.entryUrl, candidate.artifact.publicOrigin)
      const config = await services.request(entry.origin, new URL('assets/config.js', entry).href)
      assert.equal(config.status, 200); assert(config.text.includes(services.origin + '/' + code + '-api'), 'Externally hosted application needs its explicit absolute HTTPS API base')
      assert(config.text.includes(services.origin), 'SDK host allowlist must use the real public host origin')
      const html = await services.request(entry.origin, entry.href); assert.equal(html.status, 200)
      assert(html.headers['content-security-policy']?.includes(services.origin), 'External static CSP must allow the approved HTTPS business API origin')
      const preflight = await services.request(services.origin, code === 'stores' ? '/stores-api/stores' : '/work-orders-api/tickets', { method: 'OPTIONS',
        headers: { Origin: 'null', 'Access-Control-Request-Method': 'GET', 'Access-Control-Request-Headers': 'authorization,x-request-id' } })
      assert.equal(preflight.status, 204); assert.equal(preflight.headers['access-control-allow-origin'], 'null'); assert(!preflight.headers['access-control-allow-credentials'])
      external.push({ applicationCode: code, entryOrigin: entry.origin, explicitBusinessApiOrigin: services.origin, verifiedTls: true,
        approvedHostOrigin: services.origin, cspAllowsBusinessApi: true, opaqueOrigin: 'null', corsStatus: 204, allowCredentials: false })
    }
    await Promise.allSettled([...headerTasks])
    const externalOperations = operations.filter(value => value.responseStatus >= 200 && value.responseStatus < 300 && value.frameUrl?.startsWith(services.externalOrigin + '/sub-apps/'))
    assert(externalOperations.length >= 6)
    for (const operation of externalOperations) { assert.equal(operation.origin, 'null'); assert.equal(operation.targetOrigin, services.origin); assert.equal(operation.cookiePresent, false); assert.equal(operation.bearerPresent, true); assert.equal(operation.frameType, 'iframe') }
    check('external-origin-api-cors-csp', { applications: external, actualCrossOriginBusinessRequests: externalOperations.length, requests: externalOperations })
    const retained = []
    for (const code of applicationCodes) {
      const before = await services.adminApi('/api/applications/' + services.apps[code].application.id + '/deployments')
      assert.equal(before.find(value => value.environment === 'production').version, candidates[code].v2.release.version)
      await management[code].dialog.getByRole('button', { name: '回滚至 ' + candidates[code].v1.release.version, exact: true }).click()
      const pending = management[code].page.waitForResponse(response => new URL(response.url()).pathname.endsWith('/deployments/production/rollback') && response.request().method() === 'POST')
      await management[code].page.getByRole('alertdialog').getByRole('button', { name: '确认回滚', exact: true }).click()
      assert.equal((await pending).status(), 200)
      const after = await services.adminApi('/api/applications/' + services.apps[code].application.id + '/deployments')
      assert.equal(after.find(value => value.environment === 'production').version, candidates[code].v1.release.version)
      await v1Views[code].page.reload({ waitUntil: 'domcontentloaded' }); await active(v1Views[code], candidates[code].v1)
      for (const [kind, candidate] of Object.entries(candidates[code])) {
        const entry = new URL(candidate.release.entryUrl, candidate.artifact.publicOrigin)
        for (const resource of [candidate.buildInfo.entry, ...candidate.buildInfo.dynamicChunks]) {
          const response = await services.request(entry.origin, new URL(resource, entry).href); assert.equal(response.status, 200)
          retained.push({ applicationCode: code, version: candidate.release.version, kind, resource, status: response.status, mime: response.headers['content-type'] })
        }
      }
      const unknown = await services.request(services.origin, '/sub-apps/' + code + '/versions/unknown-phase28/index.html'); assert.equal(unknown.status, 404)
      const reconcile = await services.runCli(code, ['release-reconcile', '--release-file', candidates[code].v1.releaseFile, '--artifact-file', candidates[code].v1.artifactFile, '--inventory-dir', services.runtime.inventoryDir])
      assert.equal(reconcile.reconciliation, 'MATCHED'); assert.equal(reconcile.remoteDigestStored, false)
    }
    check('versioned-chunks-rollback-retained', { v1: v1Proof, v2: v2Proof, retainedResources: retained, actualUiRollback: true, unknownVersionStatus: 404 })
    check('artifact-inventory-conflicts', inventoryProof)
    assert(services.runtime.deliveryProof.registryPushPull && services.runtime.deliveryProof.fixedDigestCompose)
    assert.deepEqual(services.runtime.deliveryProof.unhealthyBlocked, { routes: 0, register: 0, activate: 0 })
    check('independent-databases-and-services', { databases: services.databaseIdentities, services: ['platform','stores','work-orders'], outerDelivery: services.runtime.deliveryProof })
    await screenshot(management.stores.page, 'phase28-stores-url-management')
    await screenshot(v1Views.stores.page, 'phase28-stores-rollback')
    await screenshot(v2Views['work-orders'].page, 'phase28-external-work-orders')
    await wait(() => [...native.values()].every(record => record.started && record.nativeOutcome), 'Every observed SDK native fetch must settle before closing the opaque contexts')
    await Promise.allSettled([...headerTasks])
    for (const operation of operations.filter(value => value.responseStatus >= 200 && value.responseStatus < 300)) {
      assert.equal(operation.origin, 'null'); assert.equal(operation.targetOrigin, services.origin)
      assert.equal(operation.cookiePresent, false); assert.equal(operation.bearerPresent, true); assert.equal(operation.frameType, 'iframe')
      assert(native.has(operation.requestId), 'Real browser requests must correlate with the observed SDK request ID')
    }
    assert.deepEqual(errors, [], 'Production host/micro applications must have no uncaught JavaScript errors')
  }
} catch (error) { failure = error; console.error(services ? services.redact(error.stack) : 'Phase28 fixture failed before private runtime initialization') }
finally {
  for (const context of contexts) try { await context.close() } catch {}
  if (browser) try { await browser.close(); browserClosed = true } catch {}
  await Promise.allSettled([...headerTasks])
  if (services && args[0] === '--browser') try { cleanup = await services.close({ browserClosed }) } catch (error) { failure ||= error }
  else cleanup = failure?.phase28StartupCleanup
}
if (args[0] === '--browser') {
  const complete = !failure && phase28DeliveryChecks.every(id => checks.some(value => value.id === id))
  const transport = [...native.values()]
  for (const attempt of transport) {
    if (attempt.started && (attempt.credentials !== 'omit' || attempt.cookiePresent || !attempt.bearerPresent)) failure ||= new Error('Observed SDK request violated opaque transport policy')
  }
  const result = { schemaVersion: 1, kind: 'phase28-real-image-delivery-browser', status: complete && !failure ? 'PASS' : 'FAIL', product: product || process.env.SAAS_TEST_BROWSER,
    browserVersion: browser?.version(), playwrightVersion, sourceSha256: sha(readFileSync(fileURLToPath(import.meta.url))),
    sourceRevision: services?.runtime.sourceRevision, inputsSha256: services?.runtime.inputsSha256, normalTLS: true,
    source: services?.source, checks: phase28DeliveryChecks.filter(id => checks.some(value => value.id === id)).map(id => ({ id, status: 'PASS' })),
    passed: checks.length, failed: complete && !failure ? 0 : 1, skipped: 0, evidence, screenshots: evidence.screenshots || [], transport: { records: transport, businessRequests: operations }, cleanup,
    ...(failure ? { error: services ? services.redact(failure.message) : 'Private runtime initialization failed' } : {}) }
  if (services) { mkdirSync(services.runtime.reportDir, { recursive: true }); writeFileSync(path.join(services.runtime.reportDir, 'fixture-result.json'), JSON.stringify(result, null, 2) + '\n', { mode: 0o600 }) }
  console.log('PHASE28_BROWSER ' + JSON.stringify(result))
}
if (failure) process.exitCode = 1
