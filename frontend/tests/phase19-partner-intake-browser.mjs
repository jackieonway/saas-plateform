/** Real React/Vite intake controls with an isolated HTTP fixture; no production review or delivery claim. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import fs from 'node:fs/promises'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { launchBrowser } from './browser/runtime.mjs'
import { waitState } from './browser/wait.mjs'
import { fixtureUuid, identityForProfile, installSession, profileWithIdentity } from './browser/identity-fixture.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const report = process.env.IDENTITY_PHASE19_BROWSER_REPORT_DIR
assert(report && path.isAbsolute(report), 'Set an explicit absolute IDENTITY_PHASE19_BROWSER_REPORT_DIR')
for (const directory of ['src', 'tests', 'node_modules']) {
  const relative = path.relative(path.join(root, directory), path.resolve(report))
  assert(relative.startsWith('..') || path.isAbsolute(relative), 'Artifacts cannot overwrite source or dependencies')
}
await fs.mkdir(report, { recursive: true })
const id = label => fixtureUuid(`phase19-browser:${label}`)
const ids = { actor: id('actor'), applicant: id('applicant'), tenant: id('tenant'), org: id('org'), member: id('member'), system: id('system'), partner: id('partner'), app: id('app'), request: id('request'), contact: id('contact'), domain: id('domain'), test: id('test') }
const token = 'owned-phase19-browser-session'
const fine = ['view', 'register', 'review', 'publish', 'rotate', 'disable', 'scopes', 'invite', 'assign'].map(name => `system:oauth:${name}`)
const system = { id: ids.system, code: 'system', name: '系统管理', icon: 'Settings2', basePath: '/system', status: 'ENABLED', menuCount: 1, entryType: 'HTML', runtimeMode: 'TRUSTED', sortOrder: 0, allowedMemberTypes: ['EMPLOYEE'], accessPolicyVersion: 1 }
const profile = profileWithIdentity({ user: { id: ids.actor, username: 'phase19-reviewer', realName: '验收审核员', email: '', phone: '', avatar: '', gender: 'UNKNOWN', isSuperAdmin: false }, tenant: { id: ids.tenant, name: '验收平台', code: 'phase19', isPlatform: true }, platformContext: true, organization: { id: ids.org, name: '验收根组织', path: `/${ids.org}` }, applications: [system], memberships: [{ userId: ids.actor, tenantId: ids.tenant, tenantName: '验收平台', organizationId: ids.org, organizationName: '验收根组织', organizationPath: `/${ids.org}`, membershipId: ids.member, memberType: 'EMPLOYEE', identityVersion: '1', status: 'ENABLED', consoleSelectable: true, selectionDisabledReason: null }] }, { membershipId: ids.member, identityVersion: '1', accountVersion: '1' })
const proof = identityForProfile(profile)
const manifest = () => ({ displayName: 'Partner Intake Application', purpose: 'Owned account login with minimal profile', privacyUrl: 'https://partner.example.invalid/privacy', domains: ['partner.example.invalid'], redirectUris: ['https://partner.example.invalid/callback'], postLogoutRedirectUris: [], scopes: ['openid', 'profile'], contactReference: ids.contact, domainEvidenceReference: ids.domain, logoutMode: 'SHORT_SESSION_ONLINE_CHECK' })
let browser, vite, origin, startup = ''
const scenarios = [], screenshots = []
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
async function start() {
  const port = await new Promise((resolve, reject) => { const server = net.createServer().on('error', reject); server.listen(0, '127.0.0.1', () => { const available = server.address().port; server.close(error => error ? reject(error) : resolve(available)) }) })
  origin = `http://127.0.0.1:${port}`
  vite = spawn(process.execPath, [path.join(root, 'node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', String(port), '--strictPort'], { cwd: root, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
  let failed
  vite.on('error', error => { failed = error })
  for (const stream of [vite.stdout, vite.stderr]) stream.on('data', chunk => { startup = (startup + chunk).slice(-4000) })
  await waitState(() => { if (failed) throw failed; if (vite.exitCode !== null) throw new Error('Owned Vite stopped'); return startup.includes('ready in') }, 'Phase19 Vite ready', 30_000)
}
async function fixture({ self = false, permissions = fine, request = true, recent = true, totpEnabled = true, reauthenticateUrl = '/auth/step-up?return=/system/oauth/clients', mobile = false, production = false, superAdmin = false } = {}) {
  const context = await browser.newContext({ viewport: mobile ? { width: 390, height: 844 } : { width: 1440, height: 1040 }, reducedMotion: 'reduce' })
  const page = await context.newPage()
  page.setDefaultTimeout(12_000)
  await installSession(context, token, proof)
  const activeProfile = { ...profile, user: { ...profile.user, isSuperAdmin: superAdmin } }
  const state = { permissions: ['system:oauth', ...permissions], recent, production, writes: [], reads: [], errors: [], unexpected: [], operationReads: 0, nextUnknown: false, conflictNextDecision: false, operations: new Map(), invitations: [],
    partners: [{ id: ids.partner, name: 'Owned Partner', contactName: 'Owned contact', contactChannel: 'verified-channel:owned', subjectReference: id('subject'), status: 'ACTIVE', version: 1 }],
    applications: [{ applicationId: ids.app, partnerId: ids.partner, displayName: 'Partner Intake Application', createdBy: self ? ids.actor : ids.applicant, requests: request ? [{ id: ids.request, applicationId: ids.app, environment: 'test', manifest: manifest(), requestedBy: self ? ids.actor : ids.applicant, submittedBy: self ? ids.actor : ids.applicant, status: 'SUBMITTED', reviewedBy: null, publishedBy: null, version: 1, syncedVersion: 0 }] : [] }],
  }
  page.on('pageerror', error => state.errors.push(error.message))
  await context.route(url => url.origin === origin && url.pathname === '/src/main.tsx', async route => {
    const response = await route.fetch()
    let body = await response.text()
    const appImport = /import App from ["']\/src\/App\.tsx(?:\?[^"']*)?["']/
    assert.match(body, appImport)
    body = body.replace(appImport, `import IntakePage from "/src/pages/system/oauth/UnifiedClientsPage.tsx";
      import { useProfileQuery as intakeProfile, useMenusQuery as intakeMenus, usePermissionsQuery as intakePermissions } from "/src/hooks/use-auth-data.ts";
      function App() { const stored=sessionStore(state=>state.profile), epoch=sessionStore(state=>state.contextEpoch); const query=intakeProfile(); intakeMenus(stored?${JSON.stringify(ids.system)}:null); intakePermissions(); React.useEffect(()=>{if(query.data)sessionStore.getState().setProfile(query.data,epoch)},[query.data,epoch]); return stored?React.createElement(IntakePage):React.createElement("main",null,"正在核验员工权限"); }`)
    body = body.replace('ReactDOM.createRoot', `await sessionStore.getState().initializeSessionCoordination();sessionStore.getState().setProfile(${JSON.stringify(activeProfile)},sessionStore.getState().contextEpoch);sessionStore.getState().setCurrentApplication(${JSON.stringify(system)});window.__phase19Fixture={queryData:()=>queryClient.getQueryCache().getAll().map(query=>query.state.data)};ReactDOM.createRoot`)
    await route.fulfill({ response, body })
  })
  const fulfill = (route, body, status = 200) => route.fulfill({ status, contentType: 'application/json', headers: { 'cache-control': 'no-store' }, body: JSON.stringify(body) })
  const reject = (route, status, code, message, operationId) => fulfill(route, { code, message, ...(operationId ? { operationId } : {}) }, status)
  const complete = (route, item, result) => {
    state.operations.set(item.operationId, result)
    if (state.nextUnknown) { state.nextUnknown = false; return reject(route, 503, 'OIDC_REMOTE_RESULT_UNKNOWN', '操作结果未知', item.operationId) }
    return fulfill(route, result)
  }
  await context.route(url => url.origin === origin && url.pathname.startsWith('/api/'), async route => {
    const incoming = route.request(), url = new URL(incoming.url()), method = incoming.method()
    const item = { path: url.pathname, method, operationId: incoming.headers()['x-identity-operation'], body: incoming.postData() ? incoming.postDataJSON() : undefined }
    try {
      assert.equal(incoming.headers().authorization, `Bearer ${token}`)
      if (method === 'GET') state.reads.push(item)
      else { assert.match(item.operationId, /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/); state.writes.push(item) }
      if (url.pathname === '/api/auth/profile') return fulfill(route, activeProfile)
      if (url.pathname === '/api/auth/permissions') return fulfill(route, { permissions: state.permissions, roles: [] })
      if (url.pathname === '/api/auth/menus') return fulfill(route, [{ id: id('menu'), parentId: null, type: 'MENU', title: '统一注册簿', path: '/system/oauth/clients', permission: 'system:oauth', sortOrder: 1, visible: true, children: [] }])
      if (url.pathname === '/api/applications') return fulfill(route, [system])
      if (url.pathname === '/api/identity/clients' && method === 'GET') return fulfill(route, { records: [], page: { limit: 10, offset: 0, total: 0 } })
      if (url.pathname === '/api/identity/review/security') return fulfill(route, { totpEnabled, recent: state.recent, securityUrl: 'https://identity.example.invalid/sso/security', reauthenticateUrl, productionReady: state.production, permissions: state.permissions })
      if (url.pathname === '/api/identity/review/partners') {
        if (method === 'GET') return fulfill(route, { partners: state.partners })
        const result = { id: id(`partner-${state.partners.length}`), ...item.body, status: 'ACTIVE', version: 1 }; state.partners.push(result); return complete(route, item, result)
      }
      if (url.pathname === '/api/identity/review/applications') {
        if (method === 'GET') return fulfill(route, { applications: state.applications, productionReady: state.production, permissions: state.permissions })
        const result = { applicationId: id(`application-${state.applications.length}`), ...item.body, createdBy: ids.actor, requests: [] }; state.applications.push(result); return complete(route, item, result)
      }
      if (url.pathname === `/api/identity/review/applications/${ids.app}/requests` && method === 'POST') {
        const app = state.applications[0], previous = app.requests.find(row => row.environment === item.body.environment)
        if ((previous?.version ?? null) !== item.body.expectedVersion) return reject(route, 409, 'CONFLICT', '申请版本已变化')
        const result = { id: previous?.id ?? id(`request-${item.body.environment}`), applicationId: ids.app, environment: item.body.environment, manifest: item.body.manifest, requestedBy: previous?.requestedBy ?? ids.actor, submittedBy: ids.actor, status: 'SUBMITTED', reviewedBy: null, publishedBy: null, version: (previous?.version ?? 0) + 1, syncedVersion: previous?.syncedVersion ?? 0 }
        app.requests = app.requests.filter(row => row.environment !== result.environment).concat(result)
        return complete(route, item, result)
      }
      const decision = /^\/api\/identity\/review\/requests\/([^/]+)\/decisions$/.exec(url.pathname)
      if (decision && method === 'POST') {
        const request = state.applications.flatMap(app => app.requests).find(row => row.id === decision[1])
        assert(request)
        if (state.conflictNextDecision) { state.conflictNextDecision = false; request.version++; return reject(route, 409, 'CONFLICT', '申请版本已变化') }
        assert.equal(request.version, item.body.expectedVersion)
        if (['PRODUCTION_APPROVED', 'ENABLED'].includes(item.body.status) && !state.production) return reject(route, 403, 'FORBIDDEN', '生产责任尚未绑定')
        if (item.body.status !== 'SUSPENDED' && [request.requestedBy, request.submittedBy, state.applications[0].createdBy].includes(ids.actor)) return reject(route, 403, 'FORBIDDEN', '申请人不可自批')
        if (['TEST_APPROVED', 'PRODUCTION_APPROVED'].includes(item.body.status)) { assert.equal(item.body.verifiedContactReference, ids.contact); assert.equal(item.body.verifiedDomainReference, ids.domain) }
        if (item.body.status === 'TEST_ACCEPTED') assert.equal(item.body.testEvidenceReference, ids.test)
        Object.assign(request, { status: item.body.status, reason: item.body.reason, version: request.version + 1, reviewedBy: ids.actor, ...(item.body.status === 'ENABLED' ? { publishedBy: ids.actor } : {}) })
        if (['TEST_APPROVED', 'TEST_ACCEPTED', 'ENABLED', 'SUSPENDED'].includes(request.status)) request.syncedVersion = request.version
        return complete(route, item, request)
      }
      if (url.pathname === '/api/identity/invitations') {
        if (method === 'GET') return fulfill(route, { invitations: state.invitations })
        assert.deepEqual(Object.keys(item.body).sort(), ['purpose', 'recipientRef'])
        const now = Date.now(), result = { id: id(`invitation-${state.invitations.length}`), recipientRef: item.body.recipientRef, purpose: 'ACCOUNT_ONBOARDING', status: 'ACTIVE', version: 1, createdAt: new Date(now).toISOString(), expiresAt: new Date(now + 86400000).toISOString(), revokedAt: null, consumedAt: null }
        state.invitations.push(result); return complete(route, item, result)
      }
      const revoke = /^\/api\/identity\/invitations\/([^/]+)\/revoke$/.exec(url.pathname)
      if (revoke && method === 'POST') { const invitation = state.invitations.find(row => row.id === revoke[1]); assert(invitation); assert.equal(invitation.version, item.body.expectedVersion); Object.assign(invitation, { status: 'REVOKED', version: invitation.version + 1, revokedAt: new Date().toISOString() }); return complete(route, item, invitation) }
      const operation = /^\/api\/identity\/client-operations\/([^/]+)$/.exec(url.pathname)
      if (operation && method === 'GET') { assert(state.operations.has(operation[1])); state.operationReads++; if (state.operationReads === 1) return reject(route, 503, 'OIDC_REMOTE_RESULT_UNKNOWN', '原操作仍在处理中', operation[1]); return fulfill(route, { operationId: operation[1], state: 'COMPLETED', result: state.operations.get(operation[1]) }) }
      state.unexpected.push({ path: item.path, method }); return reject(route, 503, 'FIXTURE_UNEXPECTED', 'Unexpected fixture route')
    } catch (error) { state.errors.push(error.message); return reject(route, 503, 'FIXTURE_ASSERTION', 'Owned fixture assertion failed') }
  })
  await page.goto(`${origin}/system/oauth/clients`, { waitUntil: 'domcontentloaded' })
  await page.getByRole('heading', { name: '统一客户端注册簿', exact: true }).waitFor()
  return { context, page, state }
}
async function scenario(name, options, run) {
  const f = await fixture(options)
  try { await run(f); assert.deepEqual(f.state.errors, []); assert.deepEqual(f.state.unexpected, []); scenarios.push({ name, status: 'PASS' }); console.log(`PASS phase19 intake: ${name}`) }
  catch (error) { await f.page.screenshot({ path: path.join(report, 'phase19-failure.png'), fullPage: true }).catch(() => {}); console.error('Phase19 fixture failure', { name, errors: f.state.errors, unexpected: f.state.unexpected, body: await f.page.locator('body').innerText().catch(() => '') }); throw error }
  finally { await f.context.close() }
}
async function openPartners(f) { await f.page.getByRole('button', { name: '合作方接入', exact: true }).click(); const dialog = f.page.getByRole('dialog'); await dialog.getByRole('heading', { name: '合作方接入与账号邀请', exact: true }).waitFor(); await dialog.getByRole('button', { name: /Partner Intake Application/ }).click(); return dialog }
async function screenshot(page, file, fullPage = true) { await page.screenshot({ path: path.join(report, file), ...(fullPage ? { fullPage: true } : {}), animations: 'disabled' }); screenshots.push(file) }

try {
  browser = await launchBrowser(); await start()
  await scenario('registrar submits only minimal account scope and exact owned callbacks', { self: true, request: false }, async f => {
    const dialog = await openPartners(f)
    await dialog.getByRole('button', { name: '提交环境申请', exact: true }).click()
    for (const [field, value] of Object.entries({ 'intake-request-purpose': '受控账号登录并展示最小基础资料', 'intake-request-privacy': 'https://partner.example.invalid/privacy', 'intake-request-domains': 'partner.example.invalid', 'intake-request-redirects': 'https://partner.example.invalid/callback', 'intake-request-contact-ref': ids.contact, 'intake-request-domain-ref': ids.domain })) await dialog.locator(`#${field}`).fill(value)
    await dialog.getByRole('button', { name: '提交审核资料', exact: true }).click()
    await dialog.getByText('待审核', { exact: true }).waitFor()
    const write = f.state.writes.find(row => row.path.endsWith('/requests'))
    assert.deepEqual(write.body.manifest.scopes, ['openid', 'profile'])
    assert.equal(write.body.expectedVersion, null)
    assert.equal(await dialog.getByRole('button', { name: '批准测试接入', exact: true }).isDisabled(), true)
    await screenshot(f.page, 'phase19-submitted-application.png')
  })
  await scenario('390px back-channel request requires an exact owned endpoint before independent review', { self: true, request: false, mobile: true }, async f => {
    const dialog = await openPartners(f)
    await dialog.getByRole('button', { name: '提交环境申请', exact: true }).click()
    for (const [field, value] of Object.entries({ 'intake-request-purpose': '隔离fixture的精确back-channel申请', 'intake-request-privacy': 'https://partner.example.invalid/privacy', 'intake-request-domains': 'partner.example.invalid', 'intake-request-redirects': 'https://partner.example.invalid/callback', 'intake-request-contact-ref': ids.contact, 'intake-request-domain-ref': ids.domain })) await dialog.locator(`#${field}`).fill(value)
    await dialog.getByRole('combobox', { name: '退出接入合同' }).click()
    await f.page.getByRole('option', { name: '独立审核 back-channel 地址', exact: true }).click()
    await dialog.locator('#intake-request-backchannel').fill('https://other.example.invalid/logout')
    await dialog.getByRole('button', { name: '提交审核资料', exact: true }).click()
    await dialog.getByRole('alert').filter({ hasText: 'back-channel地址须无查询参数且属于当前审核域名' }).waitFor()
    assert.equal(f.state.writes.length, 0)
    await dialog.locator('#intake-request-backchannel').fill('https://partner.example.invalid/auth/backchannel-logout')
    await dialog.locator('#intake-request-backchannel').scrollIntoViewIfNeeded()
    assert.equal(await f.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
    await screenshot(dialog, 'phase20-review-backchannel-mobile.png', false)
    await dialog.getByRole('button', { name: '提交审核资料', exact: true }).click()
    await dialog.getByText('待审核', { exact: true }).waitFor()
    const write = f.state.writes.find(row => row.path.endsWith('/requests'))
    assert.equal(write.body.manifest.logoutMode, 'BACK_CHANNEL')
    assert.equal(write.body.manifest.backchannelLogoutUri, 'https://partner.example.invalid/auth/backchannel-logout')
  })
  await scenario('independent reviewer returns then approves a revised test request with explicit evidence', {}, async f => {
    const dialog = await openPartners(f)
    await dialog.getByRole('button', { name: '退回申请', exact: true }).click()
    await dialog.locator('#intake-decision-reason').fill('本次域名与责任联系人证明需要补充核验记录')
    await dialog.getByRole('button', { name: '确认退回申请', exact: true }).click()
    await dialog.getByText('已退回', { exact: true }).waitFor()
    Object.assign(f.state.applications[0].requests[0], { status: 'SUBMITTED', version: 3, reason: '', submittedBy: ids.applicant })
    await dialog.getByRole('button', { name: '刷新状态', exact: true }).click()
    await dialog.getByRole('button', { name: '批准测试接入', exact: true }).click()
    await dialog.locator('#intake-decision-reason').fill('已人工核对本次主体用途及域名联系人证明材料')
    await dialog.locator('#intake-verified-contact').fill(ids.contact)
    await dialog.locator('#intake-verified-domain').fill(ids.domain)
    await dialog.getByRole('checkbox').check()
    await dialog.getByRole('button', { name: '确认批准测试接入', exact: true }).click()
    await dialog.getByText('测试已批准', { exact: true }).waitFor()
    await dialog.getByRole('button', { name: '记录测试验收', exact: true }).click()
    await dialog.locator('#intake-decision-reason').fill('标准回调资料范围与撤销场景的测试证据已核验')
    await dialog.locator('#intake-test-evidence').fill(ids.test)
    await dialog.getByRole('button', { name: '确认记录测试验收', exact: true }).click()
    await dialog.getByText('测试已验收', { exact: true }).waitFor()
    assert.equal(f.state.writes.filter(row => row.path.endsWith('/decisions')).length, 3)
    await screenshot(f.page, 'phase19-independent-review.png')
  })
  for (const variant of [{ name: 'missing review permission', permissions: ['system:oauth:view', 'system:oauth:register'] }, { name: 'applicant cannot self approve', self: true }, { name: 'stale second authentication blocks sensitive controls', recent: false }]) await scenario(variant.name, variant, async f => {
    const dialog = await openPartners(f)
    assert.equal(await dialog.getByRole('button', { name: '批准测试接入', exact: true }).isDisabled(), true)
    assert.equal(f.state.writes.length, 0)
  })
  await scenario('platform reauthentication confirms draft loss and exposes only the fixed same-origin entry', {}, async f => {
    const dialog = await openPartners(f)
    await dialog.getByRole('button', { name: '修订并重新提交', exact: true }).click()
    await dialog.locator('#intake-request-purpose').fill('重新验证前取消确认时必须保留的未提交草稿')
    await dialog.getByRole('button', { name: '重新验证平台登录', exact: true }).click()
    const confirmation = f.page.getByRole('alertdialog')
    await confirmation.getByText(/未提交的申请、审核和邀请草稿不会保留/).waitFor()
    const entry = confirmation.getByRole('link', { name: '开始重新验证', exact: true })
    assert.equal(await entry.getAttribute('href'), '/auth/step-up?return=/system/oauth/clients')
    assert.equal(await entry.evaluate(link => new URL(link.href).origin), origin)
    assert.equal(await entry.getAttribute('target'), null)
    await screenshot(f.page, 'phase19-reauthentication-confirm.png')
    await confirmation.getByRole('button', { name: '继续编辑', exact: true }).click()
    assert.equal(await dialog.locator('#intake-request-purpose').inputValue(), '重新验证前取消确认时必须保留的未提交草稿')
    assert.equal(f.page.url(), `${origin}/system/oauth/clients`)
    assert.equal(f.state.writes.length, 0)
  })
  await scenario('first-factor guidance distinguishes security management from a new platform session', { recent: false, totpEnabled: false, mobile: true }, async f => {
    const dialog = await openPartners(f)
    await dialog.getByText(/首次未绑定时，先重新验证平台登录/).waitFor()
    assert.equal(await dialog.getByRole('link', { name: '打开账号安全页', exact: true }).getAttribute('href'), 'https://identity.example.invalid/sso/security')
    assert.equal(await dialog.getByRole('button', { name: '重新验证平台登录', exact: true }).isEnabled(), true)
    assert.equal(await dialog.getByRole('button', { name: '批准测试接入', exact: true }).isDisabled(), true)
    assert.equal(f.state.writes.length, 0)
    await dialog.getByRole('button', { name: '重新验证平台登录', exact: true }).scrollIntoViewIfNeeded()
    assert.equal(await f.page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true)
    await screenshot(dialog, 'phase19-reauthentication-mobile.png', false)
  })
  await scenario('an untrusted reauthentication address never becomes a navigation control', { recent: false, reauthenticateUrl: 'https://evil.example.invalid/auth/step-up?return=/system/oauth/clients' }, async f => {
    const dialog = await openPartners(f)
    await dialog.getByText('敏感操作需要重新完成 TOTP 二次认证', { exact: true }).waitFor()
    assert.equal(await dialog.getByRole('button', { name: '重新验证平台登录', exact: true }).count(), 0)
    assert.equal(await dialog.locator('a[href*="evil.example.invalid"]').count(), 0)
    assert.equal(f.state.writes.length, 0)
  })
  await scenario('verified administrator still needs each fine permission while registration scope stays explicit', { superAdmin: true, permissions: ['system:oauth:view', 'system:oauth:register'] }, async f => {
    const dialog = await openPartners(f)
    await dialog.locator('summary').waitFor()
    assert.equal(await dialog.getByRole('button', { name: '批准测试接入', exact: true }).isDisabled(), true)
    assert.equal(f.state.writes.length, 0)
  })
  await scenario('production approval stays closed without verified responsibility bindings', {}, async f => {
    f.state.applications[0].requests.push({ ...f.state.applications[0].requests[0], id: id('production-request'), environment: 'production', status: 'SUBMITTED' })
    const dialog = await openPartners(f)
    await dialog.getByText('生产审批与启用保持关闭', { exact: true }).waitFor()
    assert.equal(await dialog.getByRole('button', { name: '批准生产申请', exact: true }).isDisabled(), true)
    assert.equal(f.state.writes.length, 0)
  })
  await scenario('explicit configured fixture permits independent production approval and controlled enablement', { production: true }, async f => {
    f.state.applications[0].requests[0].status = 'TEST_ACCEPTED'
    f.state.applications[0].requests[0].syncedVersion = 1
    f.state.applications[0].requests.push({ ...f.state.applications[0].requests[0], id: id('configured-production-request'), environment: 'production', status: 'SUBMITTED', syncedVersion: 0 })
    const dialog = await openPartners(f)
    await dialog.getByRole('button', { name: '批准生产申请', exact: true }).click()
    await dialog.locator('#intake-decision-reason').fill('本隔离验收fixture已明确设置责任绑定并核验接入申请证明')
    await dialog.locator('#intake-verified-contact').fill(ids.contact)
    await dialog.locator('#intake-verified-domain').fill(ids.domain)
    await dialog.getByRole('checkbox').check()
    await dialog.getByRole('button', { name: '确认批准生产申请', exact: true }).click()
    await dialog.getByText('生产已批准', { exact: true }).waitFor()
    await dialog.getByRole('button', { name: '启用生产接入', exact: true }).click()
    await dialog.locator('#intake-decision-reason').fill('配置齐备的独立验收fixture已完成测试与发布检查清单')
    await dialog.getByRole('button', { name: '确认启用生产接入', exact: true }).click()
    await dialog.getByText('生产已启用', { exact: true }).waitFor()
    assert.equal(f.state.writes.filter(row => row.path.endsWith('/decisions')).length, 2)
    assert.equal(f.state.applications[0].requests.find(row => row.environment === 'production').publishedBy, ids.actor)
  })
  await scenario('390px application form keeps its actions reachable within the viewport', { mobile: true, request: false }, async f => {
    const dialog = await openPartners(f)
    await dialog.getByRole('button', { name: '提交环境申请', exact: true }).click()
    await dialog.locator('#intake-request-domain-ref').scrollIntoViewIfNeeded()
    await dialog.getByRole('button', { name: '提交审核资料', exact: true }).scrollIntoViewIfNeeded()
    assert.equal(await f.page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true)
    const close = await dialog.getByRole('button', { name: '关闭接入管理', exact: true }).boundingBox()
    assert(close && close.y >= 0 && close.y + close.height <= 844)
    await screenshot(dialog, 'phase19-application-mobile.png', false)
  })
  await scenario('a version conflict preserves the decision draft until explicit latest-state reload', {}, async f => {
    const dialog = await openPartners(f)
    await dialog.getByRole('button', { name: '批准测试接入', exact: true }).click()
    await dialog.locator('#intake-decision-reason').fill('这份旧草稿在冲突后必须保留并等待明确载入最新状态')
    await dialog.locator('#intake-verified-contact').fill(ids.contact)
    await dialog.locator('#intake-verified-domain').fill(ids.domain)
    await dialog.getByRole('checkbox').check()
    f.state.conflictNextDecision = true
    await dialog.getByRole('button', { name: '确认批准测试接入', exact: true }).click()
    await dialog.getByRole('alert').filter({ hasText: '申请版本已变化' }).waitFor()
    assert.equal(await dialog.getByRole('button', { name: '确认批准测试接入', exact: true }).isDisabled(), true)
    assert.equal(await dialog.locator('#intake-decision-reason').inputValue(), '这份旧草稿在冲突后必须保留并等待明确载入最新状态')
    await dialog.getByRole('button', { name: '载入最新状态', exact: true }).click()
    await dialog.getByRole('button', { name: '批准测试接入', exact: true }).click()
    assert.equal(await dialog.locator('#intake-decision-reason').inputValue(), '')
    assert.equal(await dialog.getByRole('checkbox').isChecked(), false)
    assert.equal(f.state.writes.length, 1)
  })
  await scenario('390px invitation management issues and revokes metadata without exposing token material', { mobile: true }, async f => {
    await f.page.getByRole('button', { name: '账号邀请', exact: true }).click()
    const dialog = f.page.getByRole('dialog')
    await dialog.getByLabel('邀请收件人引用').fill('verified-recipient:phase19-owned')
    await dialog.getByRole('button', { name: '签发受控邀请', exact: true }).click()
    await dialog.getByText('verified-recipient:phase19-owned', { exact: true }).waitFor()
    assert.equal(new Date(f.state.invitations[0].expiresAt) - new Date(f.state.invitations[0].createdAt), 86400000)
    assert.equal(await f.page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true)
    await dialog.getByText('verified-recipient:phase19-owned', { exact: true }).scrollIntoViewIfNeeded()
    await f.page.getByText('账号邀请已签发，等待受控投递流程', { exact: true }).waitFor({ state: 'hidden' })
    await screenshot(dialog, 'phase19-invitation-mobile.png', false)
    await dialog.getByRole('button', { name: '撤销此邀请', exact: true }).click()
    await dialog.getByText('REVOKED', { exact: true }).waitFor()
    assert.equal(f.state.writes.length, 2)
    const persisted = await f.page.evaluate(() => JSON.stringify({ session: Object.values(sessionStorage), local: Object.values(localStorage), queries: window.__phase19Fixture.queryData() }))
    assert.equal(/invitationToken|deliveryMaterial|clientSecret/.test(persisted), false)
    assert.equal(await dialog.locator('input[type=password]').count(), 0)
  })
  await scenario('unknown intake writes recover the original operation before another mutation', {}, async f => {
    const dialog = await openPartners(f)
    await dialog.locator('summary').click()
    for (const [field, value] of Object.entries({ 'intake-partner-name': 'Unknown Result Partner', 'intake-partner-contact': 'Owned fixture reviewer', 'intake-partner-channel': 'verified-channel:unknown-fixture', 'intake-partner-proof': id('unknown-subject') })) await dialog.locator(`#${field}`).fill(value)
    f.state.nextUnknown = true
    await dialog.getByRole('button', { name: '登记合作方', exact: true }).click()
    await f.page.getByText('查询失败：原操作仍在处理中', { exact: false }).waitFor()
    assert.equal(f.state.writes.length, 1)
    assert.equal(await f.page.getByRole('button', { name: '登记客户端', exact: true }).isDisabled(), true)
    const stored = await f.page.evaluate(() => Object.entries(sessionStorage).filter(([key]) => key.startsWith('platform-identity-operation-v1:')).map(([, value]) => JSON.parse(value)))
    assert.equal(stored.length, 1)
    assert.equal(stored[0].action, 'review')
    assert.equal(stored[0].operationId, f.state.writes[0].operationId)
    await f.page.getByRole('button', { name: /查询原操作/ }).click()
    await waitState(() => f.state.operationReads === 2, 'Query the original review operation')
    await waitState(async () => !(await f.page.getByRole('button', { name: '登记客户端', exact: true }).isDisabled()), 'Release the write gate only after a confirmed terminal receipt')
    assert.equal(f.state.writes.length, 1)
    assert.equal(f.state.partners.filter(partner => partner.name === 'Unknown Result Partner').length, 1)
  })
  await scenario('unexpected invitation token fields never enter the page or Query cache', {}, async f => {
    const now = Date.now()
    f.state.invitations.push({ id: id('unsafe-invitation'), recipientRef: 'owned-recipient', purpose: 'ACCOUNT_ONBOARDING', status: 'ACTIVE', version: 1, createdAt: new Date(now).toISOString(), expiresAt: new Date(now + 86400000).toISOString(), revokedAt: null, consumedAt: null, token: 'private-test-invitation-material' })
    await f.page.getByRole('button', { name: '账号邀请', exact: true }).click()
    await f.page.getByRole('dialog').getByText(/邀请响应无法按管理元数据合同核验/).waitFor()
    const exposed = await f.page.evaluate(() => `${document.body.innerText}\n${JSON.stringify(window.__phase19Fixture.queryData())}\n${Object.values(sessionStorage).join('\n')}\n${Object.values(localStorage).join('\n')}`)
    assert.equal(exposed.includes('private-test-invitation-material'), false)
    assert.equal(f.state.writes.length, 0)
  })
  await fs.writeFile(path.join(report, 'phase19-partner-intake-browser.json'), JSON.stringify({ status: 'PASS', scope: 'Real React/Vite UI and production session gate with isolated HTTP fixture; reauthentication evidence covers the URL whitelist, link and cancellation only, without executing SSO/TOTP or establishing a new BFF session; no live PostgreSQL, actual production approval, domain verification or invitation delivery claim', browser: await browser.version(), scenarios, screenshots }, null, 2) + '\n')
  console.log(`Phase19 intake browser passed (${scenarios.length} scenarios). Report: ${report}`)
} finally {
  await browser?.close()
  if (vite && vite.exitCode === null) { const exited = new Promise(resolve => vite.once('exit', resolve)); vite.kill(); await Promise.race([exited, pause(5000)]); if (vite.exitCode === null) vite.kill('SIGKILL') }
}
