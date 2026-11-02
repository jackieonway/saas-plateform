/** Actual ApplicationsPage and application access policy, production main/session gate and QueryClient.
 * HTTP authority is controlled; this does not establish backend/database behavior.
 * ADMISSION_POLICY_BROWSER_REPORT_DIR must be an explicit absolute output folder.
 * Run only from the root browser gate; the authoring task does not launch it.
 */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import fs from 'node:fs/promises'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { launchBrowser } from './browser/runtime.mjs'
import { waitState } from './browser/wait.mjs'
import { fixtureUuid, identityForProfile, installSession, profileWithIdentity } from './browser/identity-fixture.mjs'

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const reportPath = process.env.ADMISSION_POLICY_BROWSER_REPORT_DIR
assert(reportPath && path.isAbsolute(reportPath), 'Set explicit absolute ADMISSION_POLICY_BROWSER_REPORT_DIR')
const reportDirectory = path.resolve(reportPath)
for (const directory of ['src', 'tests', 'node_modules']) {
  const relative = path.relative(path.join(workspace, directory), reportDirectory)
  assert(relative.startsWith('..') || path.isAbsolute(relative), 'Browser artifacts must not overwrite source or dependencies')
}
const bindHost = process.env.ADMISSION_POLICY_BROWSER_BIND || (process.platform === 'win32' ? '::1' : '127.0.0.1')
const pause = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds))
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done }); return { promise, resolve } }
const id = label => fixtureUuid(`admission-policy:${label}`)
const token = 'admission-policy-browser-session'
const ids = Object.fromEntries(['actor', 'tenant', 'organization', 'membership', 'system', 'app', 'public', 'confidential', 'machine', 'console', 'receipt'].map(label => [label, id(label)]))
const system = { id: ids.system, code: 'system', name: '系统管理', icon: 'Settings2', basePath: '/system', status: 'ENABLED',
  menuCount: 2, entryType: 'HTML', runtimeMode: 'TRUSTED', sortOrder: 0, allowedMemberTypes: ['EMPLOYEE'], accessPolicyVersion: 1 }
const application = { id: ids.app, code: 'policy-demo', name: '准入验收应用', icon: 'Contact', basePath: '/micro/policy-demo', status: 'ENABLED',
  description: '独立应用身份准入', menuCount: 1, entryType: 'HTML', runtimeMode: 'TRUSTED', entryUrl: '/fixture-app/index.html', sortOrder: 1,
  allowedMemberTypes: ['EMPLOYEE'], accessPolicyVersion: 5 }
const profile = apps => profileWithIdentity({
  user: { id: ids.actor, username: 'policy-reviewer', realName: '准入验收管理员', email: '', phone: '', avatar: '', gender: 'UNKNOWN', isSuperAdmin: false },
  tenant: { id: ids.tenant, name: '平台验收租户', code: 'policy-platform', isPlatform: true }, platformContext: true,
  organization: { id: ids.organization, name: '平台验收组织', path: `/${ids.organization}` }, applications: apps,
  memberships: [{ userId: ids.actor, tenantId: ids.tenant, tenantName: '平台验收租户', organizationId: ids.organization,
    organizationName: '平台验收组织', organizationPath: `/${ids.organization}`, membershipId: ids.membership,
    memberType: 'EMPLOYEE', identityVersion: '7', status: 'ENABLED', consoleSelectable: true, selectionDisabledReason: null }],
}, { membershipId: ids.membership, identityVersion: '7', accountVersion: '0' })
const proof = identityForProfile(profile([system, application]))
let browser, vite, origin, startup = ''
const scenarios = [], screenshots = []

async function startServer() {
  const port = await new Promise((resolve, reject) => {
    const server = net.createServer().on('error', reject)
    server.listen(0, bindHost, () => { const available = server.address().port; server.close(error => error ? reject(error) : resolve(available)) })
  })
  origin = `http://${bindHost.includes(':') ? `[${bindHost}]` : bindHost}:${port}`
  vite = spawn(process.execPath, [path.join(workspace, 'node_modules/vite/bin/vite.js'), '--host', bindHost, '--port', String(port), '--strictPort'], {
    cwd: workspace, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
  })
  let failed
  vite.on('error', error => { failed = error })
  for (const stream of [vite.stdout, vite.stderr]) stream.on('data', chunk => { startup = (startup + chunk).slice(-5000) })
  await waitState(() => {
    if (failed) throw failed
    if (vite.exitCode !== null) throw new Error(`Owned Vite stopped: ${startup}`)
    return startup.includes('ready in')
  }, 'Admission policy Vite must become ready', 30_000)
}

async function fixture(options = {}) {
  const context = await browser.newContext({ viewport: options.mobile ? { width: 390, height: 844 } : { width: 1440, height: 1080 }, reducedMotion: 'reduce' })
  const page = await context.newPage()
  page.setDefaultTimeout(12_000)
  const state = { apps: structuredClone([system, application]), requests: [], policyResponses: [], writes: [], commits: [], errors: [], unexpected: [],
    profileReads: 0, conflictIssued: false, profileGate: deferred(), pendingGate: deferred(), saved: false }
  page.on('pageerror', error => state.errors.push(error.message))
  await installSession(context, token, proof)
  await context.addInitScript(() => localStorage.setItem('platform-theme', 'light'))
  const routePath = '/system/applications'
  const sourcePage = '/src/pages/system/applications/ApplicationsPage.tsx'
  await context.route(url => url.origin === origin && url.pathname === '/src/main.tsx', async route => {
    const response = await route.fetch()
    let body = await response.text()
    const appImport = /import App from ["']\/src\/App\.tsx(?:\?[^"']*)?["']/
    assert.match(body, appImport, 'Retain production main, gate, query cancellation and real management page')
    // The wrapper only supplies the page's normal auth queries. The original
    // main keeps its real epoch subscription and physical SessionStatusGate.
    body = body.replace(appImport, `import AdmissionPageUnderTest from ${JSON.stringify(sourcePage)};
      import { useProfileQuery as admissionProfileQuery, useMenusQuery as admissionMenusQuery, usePermissionsQuery as admissionPermissionsQuery } from "/src/hooks/use-auth-data.ts";
      function App() {
        const stored = sessionStore(state => state.profile);
        const epoch = sessionStore(state => state.contextEpoch);
        const query = admissionProfileQuery();
        admissionMenusQuery(stored ? ${JSON.stringify(ids.system)} : null);
        admissionPermissionsQuery();
        React.useEffect(() => { if (query.data) sessionStore.getState().setProfile(query.data, epoch); }, [query.data, epoch]);
        React.useEffect(() => { window.__admissionPolicyFixture.mounts.push(epoch); return () => window.__admissionPolicyFixture.unmounts.push(epoch); }, []);
        return stored ? React.createElement(AdmissionPageUnderTest) : React.createElement("main", { role: "status" }, "正在重新核验当前成员授权");
      }`)
    body = body.replace('ReactDOM.createRoot', `
      import { captureSessionSignal as admissionSessionSignal } from "/src/stores/auth.ts";
      import { api as admissionFixtureApi } from "/src/lib/api.ts";
      await sessionStore.getState().initializeSessionCoordination();
      sessionStore.getState().setProfile(${JSON.stringify(profile([system, application]))});
      sessionStore.getState().setCurrentApplication(${JSON.stringify(ids.system)});
      const admissionSnapshot = () => { const state = sessionStore.getState(); return {
        epoch: state.contextEpoch, contextKey: state.contextKey, sessionId: state.sessionId,
        hasProfile: !!state.profile, applicationIds: state.profile?.applications.map(app => app.id) ?? [],
        currentApplicationId: state.currentApplicationId, hasToken: !!state.token,
        queryKeys: queryClient.getQueryCache().getAll().map(query => query.queryKey),
        queryStates: queryClient.getQueryCache().getAll().map(query => ({ key: query.queryKey, status: query.state.status,
          fetchStatus: query.state.fetchStatus, error: query.state.error ? { name: query.state.error.name, message: query.state.error.message,
            status: query.state.error.status, code: query.state.error.code } : null }))
      }; };
      let admissionPreviousSignal = admissionSessionSignal();
      window.__admissionPolicyFixture = { mounts: [], unmounts: [], retirements: [], pending: "idle", snapshot: admissionSnapshot,
        startPending: () => {
          const state = sessionStore.getState(), lifetime = admissionSessionSignal();
          window.__admissionPolicyFixture.pending = "running";
          void queryClient.fetchQuery({ queryKey: ["admission-fixture-pending", state.contextKey, state.contextEpoch],
            queryFn: ({ signal }) => admissionFixtureApi.get("/api/admission-fixture/pending", { signal: AbortSignal.any([signal, lifetime]) })
          }).then(() => { window.__admissionPolicyFixture.pending = "fulfilled"; }, () => { window.__admissionPolicyFixture.pending = "rejected"; });
        }
      };
      sessionStore.subscribe((state, previous) => {
        if (state.contextEpoch !== previous.contextEpoch) {
          window.__admissionPolicyFixture.retirements.push({ ...admissionSnapshot(), oldSignalAborted: admissionPreviousSignal.aborted });
          admissionPreviousSignal = admissionSessionSignal();
        }
      });
      ReactDOM.createRoot`)
    await route.fulfill({ response, body })
  })
  const fulfill = async (route, value, status = 200) => {
    try { await route.fulfill({ status, contentType: 'application/json', headers: { 'x-request-id': ids.receipt }, body: JSON.stringify(value) }) }
    catch (error) { if (!/closed|disposed|Invalid Interception|Target|cancelled/i.test(error.message)) throw error }
  }
  const refuse = (route, status, message) => fulfill(route, { code: status === 409 ? 'CONFLICT' : 'FIXTURE_REFUSED', message }, status)
  await context.route(url => url.origin === origin && url.pathname.startsWith('/api/'), async route => {
    const request = route.request(), url = new URL(request.url()), method = request.method()
    const item = { path: url.pathname, method, body: request.postData() ? request.postDataJSON() : undefined }
    state.requests.push(item)
    try {
      assert.equal(request.headers().authorization, `Bearer ${token}`)
      if (url.pathname === '/api/auth/profile') {
        state.profileReads++
        if (state.saved && options.holdProfileAfterSave) await state.profileGate.promise
        return fulfill(route, profile(state.apps.filter(app => app.allowedMemberTypes.includes('EMPLOYEE'))))
      }
      if (url.pathname === '/api/auth/permissions') return fulfill(route, { permissions: ['system:application', 'system:oauth'], roles: [] })
      if (url.pathname === '/api/auth/menus') return fulfill(route, [
        { id: id('applications-menu'), parentId: null, type: 'MENU', title: '应用管理', path: '/system/applications', permission: 'system:application', sortOrder: 1, visible: true, children: [] },
        { id: id('clients-menu'), parentId: null, type: 'MENU', title: 'OAuth 客户端', path: '/system/oauth/clients', permission: 'system:oauth', sortOrder: 2, visible: true, children: [] },
      ])
      if (url.pathname === '/api/admission-fixture/pending') { await state.pendingGate.promise; return fulfill(route, { oldProjection: true }) }
      if (url.pathname === '/api/applications' && method === 'GET') return fulfill(route, state.apps)
      const match = /^\/api\/(applications)\/([^/]+)\/access-policy$/.exec(url.pathname)
      if (match) {
        const owner = state.apps.find(owner => owner.id === match[2])
        assert(owner, 'Policy must bind an exact known owner')
        const current = () => ({ id: owner.id, status: owner.status, allowedMemberTypes: owner.allowedMemberTypes,
          accessPolicyVersion: owner.accessPolicyVersion, ...(owner.subjectMode ? { subjectMode: owner.subjectMode } : {}) })
        if (method === 'GET') {
          const response = current()
          assert.equal(response.id, match[2]); assert.equal(response.status, 'ENABLED')
          assert(Number.isSafeInteger(response.accessPolicyVersion) && response.accessPolicyVersion > 0)
          assert(Array.isArray(response.allowedMemberTypes) && response.allowedMemberTypes.every(type => ['EMPLOYEE', 'CUSTOMER'].includes(type)))
          if (match[1] === 'applications') { assert.equal(Object.hasOwn(response, 'subjectMode'), false); assert(response.allowedMemberTypes.length > 0) }
          state.policyResponses.push({ path: item.path, response: structuredClone(response) })
          return fulfill(route, response)
        }
        assert.equal(method, 'PUT'); state.writes.push(item)
        assert.deepEqual(Object.keys(item.body).sort(), ['accessPolicyVersion', 'allowedMemberTypes', 'reason'])
        assert(Number.isSafeInteger(item.body.accessPolicyVersion) && item.body.accessPolicyVersion > 0)
        assert.equal(typeof item.body.reason, 'string'); assert(item.body.reason.trim())
        assert(item.body.allowedMemberTypes.every(type => ['EMPLOYEE', 'CUSTOMER'].includes(type)))
        if (options.conflictOnce && owner.id === ids.public && !state.conflictIssued) {
          state.conflictIssued = true; owner.accessPolicyVersion++; owner.allowedMemberTypes = ['EMPLOYEE', 'CUSTOMER']
          return refuse(route, 409, '策略已被其他操作更新')
        }
        if (item.body.accessPolicyVersion !== owner.accessPolicyVersion) return refuse(route, 409, '旧策略版本')
        assert(item.body.allowedMemberTypes.length > 0, 'Human/application sets stay nonempty')
        owner.allowedMemberTypes = ['EMPLOYEE', 'CUSTOMER'].filter(type => item.body.allowedMemberTypes.includes(type))
        owner.accessPolicyVersion++; state.saved = true; state.commits.push(item)
        return fulfill(route, current())
      }
      state.unexpected.push(item)
      return refuse(route, 503, `Unexpected fixture API ${method} ${url.pathname}`)
    } catch (error) { state.errors.push(`HTTP fixture: ${error.message}`); return refuse(route, 503, 'HTTP fixture assertion failed') }
  })
  try {
    await page.goto(`${origin}${routePath}`, { waitUntil: 'domcontentloaded' })
    await page.getByRole('heading', { name: options.clients ? 'OAuth 客户端' : '应用管理', exact: true }).waitFor()
    await waitState(async () => (await page.evaluate(() => window.__admissionPolicyFixture.snapshot())).queryKeys.some(key => key[0] === 'auth-menus'), 'Load real proof-bound authorization queries')
    return { context, page, state }
  } catch (error) { await context.close(); throw error }
}

async function scenario(name, options, run) {
  const f = await fixture(options)
  try {
    await run(f)
    assert.deepEqual(f.state.errors, []); assert.deepEqual(f.state.unexpected, [])
    scenarios.push({ name, status: 'PASS', writes: f.state.writes.length, commits: f.state.commits.length })
    console.log(`PASS admission policy: ${name}`)
  } catch (error) {
    await f.page.screenshot({ path: path.join(reportDirectory, 'admission-policy-failure.png'), fullPage: true, animations: 'disabled' }).catch(() => {})
    console.error('Admission policy diagnostics', { name, errors: f.state.errors, unexpected: f.state.unexpected, requests: f.state.requests,
      policyResponses: f.state.policyResponses, writes: f.state.writes,
      snapshot: await f.page.evaluate(() => window.__admissionPolicyFixture?.snapshot()).catch(() => null),
      body: await f.page.locator('body').innerText().catch(() => '') })
    throw error
  } finally { f.state.profileGate.resolve(); f.state.pendingGate.resolve(); await f.context.close() }
}
async function screenshot(f, filename) {
  await f.page.screenshot({ path: path.join(reportDirectory, filename), fullPage: true, animations: 'disabled' }); screenshots.push(filename)
}
async function dialogReady(f) {
  const dialog = f.page.getByRole('dialog')
  await dialog.getByRole('heading', { name: '身份准入', exact: true }).waitFor()
  await waitState(async () => {
    if (await dialog.getByText('策略读取失败，请重新加载', { exact: true }).count()) {
      const snapshot = await f.page.evaluate(() => window.__admissionPolicyFixture.snapshot())
      throw new Error(`Policy query failed before editing: ${JSON.stringify(snapshot.queryStates.filter(query => query.key[0] === 'admission-policy'))}; responses=${JSON.stringify(f.state.policyResponses)}`)
    }
    return !await dialog.getByText('读取中', { exact: true }).count()
  }, 'Read exact current policy before editing')
  return dialog
}
async function openApplication(f) {
  const card = f.page.locator('div.group.relative').filter({ has: f.page.getByText(application.name, { exact: true }) })
  await card.getByRole('button', { name: '身份准入', exact: true }).click()
  return dialogReady(f)
}
async function openClient(f, name) {
  await f.page.getByRole('button', { name: `管理 ${name} 的身份准入`, exact: true }).click()
  return dialogReady(f)
}
const reason = dialog => dialog.getByRole('textbox', { name: /^变更原因/ })
const confirmation = dialog => dialog.getByRole('checkbox', { name: '我已确认对现有登录和授权的影响', exact: true })
const save = dialog => dialog.getByRole('button', { name: '保存准入策略', exact: true })

try {
  await fs.mkdir(reportDirectory, { recursive: true }); await startServer(); browser = await launchBrowser()

  await scenario('application multi-select save immediately retires old profile menus subtree and active request', { holdProfileAfterSave: true }, async f => {
    const before = await f.page.evaluate(() => ({ ...window.__admissionPolicyFixture.snapshot(), unmounts: window.__admissionPolicyFixture.unmounts.length }))
    await f.page.evaluate(() => window.__admissionPolicyFixture.startPending())
    await waitState(() => f.state.requests.some(item => item.path === '/api/admission-fixture/pending'), 'Have one real fenced query active before save')
    const dialog = await openApplication(f)
    assert(await dialog.getByRole('checkbox', { name: '允许员工身份', exact: true }).isChecked())
    assert.equal(await dialog.getByRole('checkbox', { name: '允许客户身份', exact: true }).isChecked(), false)
    await dialog.getByRole('checkbox', { name: '允许客户身份', exact: true }).check()
    await dialog.getByRole('checkbox', { name: '允许员工身份', exact: true }).uncheck()
    await reason(dialog).fill('将已登记客户用途应用调整为客户专用入口')
    assert(await save(dialog).isDisabled(), 'Impact acknowledgement is required')
    await confirmation(dialog).check(); await screenshot(f, 'desktop-application-admission.png')
    await save(dialog).click()
    await waitState(() => f.state.commits.length === 1, 'Application policy commits once')
    await waitState(async () => await f.page.evaluate(() => window.__admissionPolicyFixture.retirements.length > 0), 'Retire projections synchronously after successful save')
    await waitState(async () => await f.page.evaluate(previous => window.__admissionPolicyFixture.unmounts.length > previous, before.unmounts), 'Physically unmount the old page subtree after retirement')
    const retired = await f.page.evaluate(() => ({ snapshot: window.__admissionPolicyFixture.snapshot(), observation: window.__admissionPolicyFixture.retirements.at(-1), unmounts: window.__admissionPolicyFixture.unmounts.length }))
    assert.equal(retired.snapshot.epoch, before.epoch + 1); assert.equal(retired.snapshot.contextKey, before.contextKey)
    assert.equal(retired.snapshot.sessionId, before.sessionId); assert.equal(retired.snapshot.hasProfile, false)
    assert.equal(retired.snapshot.currentApplicationId, null); assert.equal(retired.observation.oldSignalAborted, true)
    assert.deepEqual(retired.observation.queryKeys, [], 'Production epoch observer clears old authorization and page cache')
    assert(retired.unmounts > before.unmounts, 'Production session gate physically unmounts the old page subtree')
    await waitState(async () => await f.page.evaluate(() => window.__admissionPolicyFixture.pending === 'rejected'), 'Old activity cannot finish into refreshed projection')
    await f.page.getByRole('status').filter({ hasText: '正在重新核验当前成员授权' }).waitFor()
    await screenshot(f, 'desktop-projection-retired.png')
    assert.deepEqual(f.state.writes[0].body, { allowedMemberTypes: ['CUSTOMER'], accessPolicyVersion: 5, reason: '将已登记客户用途应用调整为客户专用入口' })
    f.state.profileGate.resolve()
    await f.page.getByRole('heading', { name: '应用管理', exact: true }).waitFor()
    const refreshed = await f.page.evaluate(() => window.__admissionPolicyFixture.snapshot())
    assert.equal(refreshed.applicationIds.includes(ids.app), false, 'Fresh profile cannot retain revoked app projection')
    for (const key of refreshed.queryKeys.filter(key => ['profile', 'auth-menus', 'permissions'].includes(key[0]))) assert.equal(key.at(-1), refreshed.epoch)
  })

  await fs.writeFile(path.join(reportDirectory, 'admission-policy-browser.json'), JSON.stringify({
    status: 'PASS', scope: 'Actual ApplicationsPage, production session gate and cache with controlled HTTP authority; no database or live backend claim',
    browser: await browser.version(), scenarios, screenshots,
  }, null, 2) + '\n')
  console.log(`Admission policy browser passed (${scenarios.length} scenarios). Report: ${reportDirectory}`)
} finally {
  await browser?.close()
  if (vite && vite.exitCode === null) {
    const exited = new Promise(resolve => vite.once('exit', resolve)); vite.kill(); await Promise.race([exited, pause(5000)])
    if (vite.exitCode === null) vite.kill('SIGKILL')
  }
}
