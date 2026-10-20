/** Real production React/module graph with isolated API fixtures and injected faults. */
import assert from 'node:assert/strict'
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import path from 'node:path'
import { launchBrowser, browserName } from './browser/runtime.mjs'
import { servePhase29 } from './browser/phase29-server.mjs'

const label = process.argv[2] || 'after'
assert(label === 'after' || /^gate-(chromium|chrome|msedge|firefox|webkit)$/.test(label))
const evidence = path.resolve(process.env.PHASE29_EVIDENCE || `target/phase-29/${label}`)
const build = JSON.parse(readFileSync(`target/phase-29/${label}/build.json`))
const server = await servePhase29(`target/phase-29/${label}/dist`)
const { origin } = server
const chunkFor = module => {
  const matches = build.inventory.filter(item => item.modules?.some(id => id.endsWith(module)))
  assert.equal(matches.length, 1, module)
  return matches[0].file
}
const user = { id: 'p29-user', username: 'phase29', realName: '验收用户', isSuperAdmin: true }
const application = { id: 'p29-system', code: 'system', name: '系统管理', icon: 'LayoutGrid', basePath: '/system' }
const profile = { user, tenant: { id: 'p29-tenant', name: '验收租户', code: 'phase29', isPlatform: true }, platformContext: true,
  organization: { id: 'p29-org', name: '验收组织', path: 'p29-org' }, applications: [application], memberships: [] }
const menu = (suffix, title) => ({ id: suffix, parentId: null, type: 'MENU', title, path: '/system/' + suffix,
  icon: 'Boxes', permission: null, sortOrder: 1, visible: true, children: [] })
const existing = { id: 'existing', code: 'existing', name: '测试应用', icon: 'Boxes', basePath: '/micro/existing', description: '',
  sortOrder: 0, status: 'ENABLED', entryType: 'HTML', runtimeMode: 'IFRAME', entryUrl: '/sub-apps/existing/versions/v1/index.html' }
let browser, count = 0
const checks = []
async function fixture({ anonymous = false, fault, deny = false } = {}) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: 'reduce' })
  const page = await context.newPage(), requests = [], resources = [], errors = [], writes = []
  let rows = [structuredClone(existing)]
  page.setDefaultTimeout(15000)
  page.on('pageerror', e => errors.push(e.message))
  page.on('request', req => { if (!req.url().includes('/api/')) resources.push(new URL(req.url()).pathname) })
  if (!anonymous) await context.addInitScript(() => {
    if (!localStorage.getItem('platform-session-v1')) localStorage.setItem('platform-session-v1', JSON.stringify({ schema: 1, version: 1,
      sessionId: crypto.randomUUID(), status: 'active', token: 'phase29-fixture-token', refreshToken: null }))
    localStorage.setItem('platform-current-app', 'p29-system')
    localStorage.setItem('platform-theme', 'light')
  })
  await context.routeWebSocket(() => true, () => {})
  await context.route('**/api/**', async route => {
    const req = route.request(), url = new URL(req.url()), p = url.pathname
    requests.push({ path: p, method: req.method() })
    const reply = (body, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) })
    if (p === '/api/platform/branding') return reply({ platformName: '管理平台', loginTitle: '欢迎回来' })
    if (p === '/api/auth/profile') return reply(deny ? { ...profile, user: { ...user, isSuperAdmin: false } } : profile)
    if (p === '/api/auth/permissions') return reply({ permissions: deny ? [] : ['system:application', 'system:tenant'], roles: [] })
    if (p === '/api/auth/menus') return reply({ menus: deny ? [] : [menu('applications', '应用管理'), menu('tenants', '租户管理')] })
    if (p === '/api/applications' && req.method() === 'GET') return reply(rows)
    if (p === '/api/applications' && req.method() === 'POST') {
      const body = req.postDataJSON(); writes.push({ method: 'POST', body }); const created = { ...body, id: 'created' }; rows.push(created); return reply(created)
    }
    if (p === '/api/applications/existing' && req.method() === 'PUT') {
      const body = req.postDataJSON(); writes.push({ method: 'PUT', body }); rows = rows.map(row => row.id === 'existing' ? { ...row, ...body } : row); return reply({})
    }
    if (/\/applications\/[^/]+\/(releases|deployments)$/.test(p)) return reply([])
    if (p === '/api/notifications/unread-count') return reply({ count: 0 })
    if (p === '/api/notifications/prefs') return reply({ dndEnabled: false, mutedTypes: [] })
    if (p === '/api/notifications/socket-context') return reply({ userId: user.id, tenantId: profile.tenant.id, organizationId: profile.organization.id, sessionVersion: 0 })
    if (p === '/api/notifications') return reply({ records: [], total: 0 })
    if (p === '/api/search') return reply({ users: [], tenants: [], organizations: [], roles: [] })
    return reply({ message: 'Isolated fixture denies unconfigured endpoint' }, 403)
  })
  if (fault === '404') await context.route('**/' + chunkFor('pages/system/tenants/TenantsPage.tsx'), route =>
    route.fulfill({ status: 404, contentType: 'text/plain', body: 'Obsolete chunk' }))
  // Render injection uses plain valid module exports without relying on React's minified export names.
  if (fault === 'render') {
    await context.route('**/' + chunkFor('pages/system/tenants/TenantsPage.tsx'), route => route.fulfill({ contentType: 'text/javascript',
      body: 'export default function Fault(){if(!window.__phase29Recovered)throw new Error("phase29-render-fault");return "局部恢复成功"}' }))
  }
  return { context, page, requests, resources, errors, writes, close: () => context.close() }
}
const shell = page => page.getByRole('complementary', { name: '应用导航', exact: true })
const navigate = (page, url) => page.evaluate(next => { history.pushState({}, '', next); dispatchEvent(new PopStateEvent('popstate')) }, url)
async function check(name, options, run) {
  const f = await fixture(options)
  try { await run(f); checks.push({ name, status: 'PASS' }); console.log(`PASS ${++count}: ${name}`) }
  catch (error) { console.error('PHASE29_FAILURE', JSON.stringify({ name, errors: f.errors, requests: f.requests, body: await f.page.locator('body').innerText() })); throw error }
  finally { await f.close() }
}
try {
  browser = await launchBrowser()
  await check('anonymous login and protected deep-link exclude private module/connection graph', { anonymous: true }, async f => {
    await f.page.goto(origin + '/system/applications?phase29=return')
    await f.page.locator('input[name="username"]').waitFor()
    assert.match(f.page.url(), /login\?return=/)
    assert.equal(new URL(f.page.url()).searchParams.get('return'), '/system/applications?phase29=return')
    const loaded = build.inventory.filter(item => f.resources.includes('/' + item.file))
    const forbidden = /components\/layout\/(AppLayout|CommandPalette|AppRail|AppSidebar|TopBar)|use-notification-socket|node_modules\/socket.io-client/
    assert.deepEqual(loaded.flatMap(item => item.modules || []).filter(id => forbidden.test(id)), [])
    assert(!f.requests.some(req => /notifications|auth\/menus/.test(req.path)))
    assert.deepEqual(f.errors, [])
  })
  await check('palette is deferred, keyboard opens, escape closes and opens again', {}, async f => {
    await f.page.goto(origin + '/system/applications')
    await f.page.getByRole('heading', { name: '应用管理', exact: true }).waitFor()
    const paletteChunk = chunkFor('components/layout/CommandPalette.tsx')
    assert(!f.resources.includes('/' + paletteChunk))
    for (let i = 0; i < 2; i++) {
      await f.page.keyboard.press('Control+k')
      await f.page.getByRole('dialog', { name: '命令面板', exact: true }).waitFor()
      await f.page.keyboard.press('Escape')
      await f.page.getByRole('dialog', { name: '命令面板', exact: true }).waitFor({ state: 'hidden' })
    }
    assert(f.resources.includes('/' + paletteChunk)); assert.deepEqual(f.errors, [])
  })
  await check('render throw preserves shell, supports local retry and navigation', { fault: 'render' }, async f => {
    await f.page.goto(origin + '/system/tenants')
    await f.page.getByRole('heading', { name: '页面暂时无法显示' }).waitFor()
    await shell(f.page).waitFor()
    await f.page.evaluate(() => { window.__phase29Recovered = true })
    await f.page.getByRole('button', { name: '重试当前页面', exact: true }).click()
    await f.page.getByText('局部恢复成功', { exact: true }).waitFor()
    await f.page.getByRole('complementary', { name: '菜单导航', exact: true }).getByRole('button', { name: '应用管理', exact: true }).click()
    await f.page.getByRole('heading', { name: '应用管理', exact: true }).waitFor()
    assert(!f.errors.some(e => !e.includes('phase29-render-fault')))
  })
  await check('lazy 404 preserves shell and explicit recovery without auto reload', { fault: '404' }, async f => {
    let navigations = 0
    f.page.on('request', request => { if (request.isNavigationRequest() && request.frame() === f.page.mainFrame()) navigations++ })
    await f.page.goto(origin + '/system/tenants')
    await f.page.getByRole('heading', { name: '页面资源加载失败' }).waitFor()
    await shell(f.page).waitFor()
    assert.equal(await f.page.getByRole('button', { name: '重试当前页面', exact: true }).count(), 0)
    assert.equal(navigations, 1)
    await f.context.unroute('**/' + chunkFor('pages/system/tenants/TenantsPage.tsx'))
    await f.page.getByRole('button', { name: '重新加载页面', exact: true }).click()
    await f.page.getByRole('heading', { name: '租户管理', exact: true }).waitFor()
    assert.equal(navigations, 2)
    const response = await f.page.request.get(origin + '/assets/old-phase29.js')
    assert.equal(response.status(), 404); assert(!response.headers()['content-type'].includes('html'))
  })
  await check('pending lazy page keeps shell mounted and navigation usable', {}, async f => {
    let release
    const gate = new Promise(resolve => { release = resolve })
    await f.context.route('**/' + chunkFor('pages/system/tenants/TenantsPage.tsx'), async route => { await gate; await route.continue() })
    try {
      await f.page.goto(origin + '/system/applications')
      await f.page.getByRole('heading', { name: '应用管理', exact: true }).waitFor()
      await f.page.evaluate(() => { document.querySelector('header')?.setAttribute('data-phase29-persist', 'yes') })
      await navigate(f.page, '/system/tenants')
      await f.page.getByRole('status', { name: '页面加载中', exact: true }).waitFor()
      await shell(f.page).waitFor()
      await navigate(f.page, '/system/applications')
      await f.page.getByRole('heading', { name: '应用管理', exact: true }).waitFor()
      assert.equal(await f.page.locator('header[data-phase29-persist="yes"]').count(), 1)
    } finally { release() }
  })
  await check('denied application route never mounts editor or calls business API', { deny: true }, async f => {
    await f.page.goto(origin + '/system/applications')
    await f.page.getByText('无访问权限', { exact: true }).waitFor()
    assert(!f.requests.some(req => req.path === '/api/applications'))
    assert(!f.resources.includes('/' + chunkFor('pages/system/applications/ApplicationsPage.tsx')))
  })
  await check('application create validation, quick-setup payload and release handoff', {}, async f => {
    await f.page.goto(origin + '/system/applications')
    await f.page.getByRole('button', { name: '新建微应用', exact: true }).click()
    const dialog = f.page.getByRole('dialog', { name: '新建微应用', exact: true })
    await dialog.getByRole('button', { name: '创建', exact: true }).click()
    await dialog.getByText('请输入应用名称', { exact: true }).waitFor()
    assert.equal(f.writes.length, 0)
    await dialog.getByLabel('应用名称').fill('新应用')
    await dialog.getByLabel('应用编码').fill('new-phase29')
    await dialog.getByRole('button', { name: '创建', exact: true }).click()
    await f.page.getByRole('dialog', { name: /发布版本/ }).waitFor()
    assert.deepEqual(f.writes, [{ method: 'POST', body: { code: 'new-phase29', name: '新应用', icon: 'LayoutGrid', basePath: '/micro/new-phase29',
      description: '', sortOrder: 0, status: 'ENABLED', entryType: 'HTML', runtimeMode: 'IFRAME', entryUrl: '/sub-apps/new-phase29/unreleased/index.html', autoCreateMenu: true } }])
    assert.deepEqual(f.errors, [])
  })
  await check('application edit preserves published entry and runtime metadata', {}, async f => {
    await f.page.goto(origin + '/system/applications')
    await f.page.getByRole('button', { name: '操作 测试应用', exact: true }).click()
    await f.page.getByRole('menuitem', { name: '编辑', exact: true }).click()
    const dialog = f.page.getByRole('dialog', { name: '编辑应用', exact: true })
    assert(await dialog.getByLabel('应用编码').isDisabled())
    await dialog.getByLabel('应用名称').fill('更新名称')
    await dialog.getByRole('button', { name: '保存更改', exact: true }).click()
    await dialog.waitFor({ state: 'hidden' })
    assert.equal(f.writes.length, 1)
    assert.deepEqual(f.writes[0], { method: 'PUT', body: { code: 'existing', name: '更新名称', icon: 'Boxes', basePath: '/micro/existing', description: '', sortOrder: 0,
      status: 'ENABLED', entryType: 'HTML', runtimeMode: 'IFRAME', entryUrl: existing.entryUrl, autoCreateMenu: false } })
    assert.deepEqual(f.errors, [])
  })
  await check('unregistered host ESM is rejected before any application write', {}, async f => {
    await f.page.goto(origin + '/system/applications')
    await f.page.getByRole('button', { name: '新建微应用', exact: true }).click()
    const dialog = f.page.getByRole('dialog', { name: '新建微应用', exact: true })
    await dialog.getByLabel('应用名称').fill('未注册模块')
    await dialog.getByLabel('应用编码').fill('not-bundled-phase29')
    await dialog.getByRole('combobox').filter({ hasText: 'HTML' }).click()
    await f.page.getByRole('option', { name: 'ESM · 宿主构建 / 直挂', exact: true }).click()
    await dialog.getByRole('button', { name: '创建', exact: true }).click()
    await dialog.getByText('当前宿主构建中没有与此编码同名的 ESM 模块', { exact: true }).waitFor()
    assert.equal(f.writes.length, 0); assert.deepEqual(f.errors, [])
  })
  console.log(`All ${count} phase29 browser checks passed.`)
} finally {
  mkdirSync(evidence, { recursive: true })
  writeFileSync(path.join(evidence, `host-browser-${browserName}.json`), JSON.stringify({ browser: browser?.version(), checks, count }, null, 2) + '\n')
  await browser?.close(); await server.close()
}
