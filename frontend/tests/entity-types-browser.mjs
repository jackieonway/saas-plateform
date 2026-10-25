/**
 * Tenant/organization type and simplified creation integration through real pages.
 * Vite mounts the production React pages; HTTP fixtures isolate persistence.
 * Run with Node 24. Browsers are installed and launched by tests/browser.
 */
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import net from "node:net"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { launchBrowser } from "./browser/runtime.mjs"
import { fixtureUuid, identityForProfile, installLegacyAuthMode, installSession, profileWithIdentity, SESSION_KEY } from "./browser/identity-fixture.mjs"

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const bindHost = process.env.ENTITY_TYPES_BROWSER_BIND || (process.platform === "win32" ? "::1" : "127.0.0.1")
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const freePort = () => new Promise((resolve, reject) => {
  const server = net.createServer().on("error", reject)
  server.listen(0, bindHost, () => {
    const port = server.address().port
    server.close((error) => error ? reject(error) : resolve(port))
  })
})
async function waitState(predicate, message, timeout = 12_000) {
  const deadline = Date.now() + timeout
  while (!predicate() && Date.now() < deadline) await pause(25)
  assert(predicate(), message)
}

const types = {
  tenant: [["PERSONAL", "个人"], ["COMPANY", "公司"], ["PUBLIC_INSTITUTION", "事业单位"], ["GOVERNMENT", "政府"], ["OTHER", "其他"]],
  organization: [["COMPANY", "公司"], ["SUBSIDIARY", "子公司"], ["BRANCH", "分公司"], ["STORE", "门店"], ["PUBLIC_INSTITUTION", "事业单位"], ["GOVERNMENT", "政府"], ["OTHER", "其他"]],
}
const tenant = { id: fixtureUuid("type-tenant"), name: "类型验收租户", code: "type-tenant", tenantType: "GOVERNMENT",
  contactName: "既有联系人", contactPhone: "13800138000", contactEmail: "existing@example.com",
  status: "ENABLED", isPlatform: false, description: "", expiresAt: null, organizationCount: 2, userCount: 0,
  authorizedAppCount: 0, createdAt: "2026-10-01T00:00:00Z" }
const rootOrganization = { id: fixtureUuid("type-root-org"), name: "类型验收根组织", code: "type-root-org", organizationType: "COMPANY",
  contactName: "既有联系人", contactPhone: "13800138000", contactEmail: "existing@example.com",
  tenantId: tenant.id, parentId: null, status: "ENABLED", sortOrder: 0, description: "", expiresAt: null }
const childOrganization = { ...rootOrganization, id: fixtureUuid("type-child-org"), name: "类型验收子公司", code: "type-child-org",
  organizationType: "SUBSIDIARY", parentId: rootOrganization.id, children: [] }
const fixtureProfile = profileWithIdentity({ user: { id: fixtureUuid("entity-user"), username: "entity-fixture", realName: "类型验收管理员" },
  tenant: { ...tenant, isPlatform: true }, organization: { ...rootOrganization, path: rootOrganization.name },
  platformContext: true, applications: [], memberships: [] })
const typeField = (kind) => kind === "tenant" ? "tenantType" : "organizationType"
const typeLabel = (kind) => kind === "tenant" ? "租户类型" : "组织类型"
const formId = (kind) => kind === "tenant" ? "tenant-form" : "org-form"
const record = (kind) => kind === "tenant" ? tenant : childOrganization
let browser, vite, origin, startupOutput = ""

async function startServer() {
  const port = await freePort()
  origin = `http://${bindHost.includes(":") ? `[${bindHost}]` : bindHost}:${port}`
  vite = spawn(process.execPath, [path.join(workspace, "node_modules/vite/bin/vite.js"),
    "--host", bindHost, "--port", String(port), "--strictPort"], {
    cwd: workspace, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
  })
  vite.stdout.on("data", (chunk) => { startupOutput = (startupOutput + chunk).slice(-4000) })
  vite.stderr.on("data", (chunk) => { startupOutput = (startupOutput + chunk).slice(-4000) })
  let spawnError
  vite.on("error", (error) => { spawnError = error })
  await waitState(() => {
    if (spawnError) throw spawnError
    if (vite.exitCode !== null) throw new Error(`Vite stopped before readiness: ${startupOutput}`)
    return startupOutput.includes("ready in")
  }, `Vite did not become ready: ${startupOutput}`, 30_000)
}

async function fixture(kind, legacy = false, organizationType) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1100 } })
  const page = await context.newPage()
  page.setDefaultTimeout(12_000)
  const state = { requests: [], saves: [], errors: [], unexpected: [] }
  const tenantRecord = { ...tenant }, childRecord = { ...childOrganization }
  if (organizationType) childRecord.organizationType = organizationType
  if (legacy) { delete tenantRecord.tenantType; delete childRecord.organizationType }
  page.on("pageerror", (error) => state.errors.push(error.message))
  await installSession(context, "test-entity-types-session", identityForProfile(fixtureProfile))
  await context.addInitScript(() => {
    localStorage.setItem("platform-theme", "light")
  })
  // Disable HMR: a reload would undo the fixture entry's production-page import.
  await context.routeWebSocket(() => true, () => {})
  await context.route((url) => url.origin === origin && url.pathname === "/src/main.tsx", async (route) => {
    const response = await route.fetch()
    let body = await response.text()
    const appImport = /import App from ["']\/src\/App\.tsx(?:\?[^"']*)?["']/
    assert.match(body, appImport, "Mount the production page from the real Vite entry.")
    body = body.replace(appImport, `import App from "/src/pages/system/${kind === "tenant" ? "tenants/TenantsPage" : "organizations/OrganizationsPage"}.tsx"`)
    body = body.replace("ReactDOM.createRoot", `import { authStore as entityFixtureAuthStore } from "/src/stores/auth.ts";\nawait entityFixtureAuthStore.getState().initializeSessionCoordination();\nentityFixtureAuthStore.getState().setProfile(${JSON.stringify(fixtureProfile)});\nReactDOM.createRoot`)
    await route.fulfill({ response, body })
  })
  const fulfill = async (route, value, status = 200) => {
    try { await route.fulfill({ status, contentType: "application/json", body: JSON.stringify(value) }) }
    catch (error) { if (!/closed|disposed|Invalid Interception|Target|cancelled/i.test(error.message)) throw error }
  }
  await context.route((url) => url.origin === origin && url.pathname.startsWith("/api/"), async (route) => {
    const request = route.request(), url = new URL(request.url())
    const item = { pathname: url.pathname, method: request.method(), params: Object.fromEntries(url.searchParams),
      body: request.postData() ? request.postDataJSON() : undefined }
    state.requests.push(item)
    if (request.headers().authorization !== "Bearer test-entity-types-session") {
      state.unexpected.push({ ...item, reason: "missing fixture session" })
      return fulfill(route, { message: `Unexpected API ${url.pathname}` }, 503)
    }
    if (url.pathname === "/api/tenants" && request.method() === "GET") {
      return fulfill(route, { records: [tenantRecord], total: 1, page: 1, size: 100 })
    }
    if (url.pathname === "/api/tenants/options" && request.method() === "GET") {
      return fulfill(route, [{ id: tenantRecord.id, name: tenantRecord.name, code: tenantRecord.code, isPlatform: tenantRecord.isPlatform }])
    }
    if (url.pathname === "/api/organizations/tree" && request.method() === "GET") {
      assert.equal(item.params.tenantId, tenant.id, "The organization tree keeps its selected tenant.")
      return fulfill(route, [{ ...rootOrganization, children: [childRecord] }])
    }
    if (url.pathname === "/api/departments/tree" && request.method() === "GET") return fulfill(route, [])
    if (url.pathname === "/api/regions/options" && request.method() === "GET") {
      return fulfill(route, { version: "type-fixture", records: [] })
    }
    if (url.pathname === "/api/regions/versions" && request.method() === "GET") return fulfill(route, [])
    const base = kind === "tenant" ? "/api/tenants" : "/api/organizations"
    if (url.pathname.startsWith(base + "/") && url.pathname.endsWith("/initialize-administrator") && request.method() === "POST") {
      const count = state.requests.filter((r) => r.pathname.endsWith("/initialize-administrator")).length
      return fulfill(route, { administrator: { userId: "initialized-user", username: "13800138000", roleName: "超级管理员",
        created: count === 1, alreadyInitialized: count > 1, ...(count === 1 ? { initialPassword: "Initialize-once-123!" } : {}) } })
    }

    if ((url.pathname === base && request.method() === "POST") ||
        (url.pathname === `${base}/${record(kind).id}` && request.method() === "PUT")) {
      state.saves.push(item)
      return fulfill(route, { id: `created-type-${state.saves.length}`, ...(request.method() === "POST" ? { administrator: {
        userId: "contact-created-user", username: item.body.contactPhone || item.body.contactEmail,
        roleName: "超级管理员", created: !state.reuseAccount,
        ...(!state.reuseAccount ? { initialPassword: "Browser-contact-password-123!" } : {}),
      } } : {}) })
    }
    state.unexpected.push(item)
    return fulfill(route, { message: `Unexpected API ${url.pathname}` }, 503)
  })
  try {
    await page.goto(`${origin}/system/${kind === "tenant" ? "tenants" : "organizations"}`, { waitUntil: "domcontentloaded" })
    if (kind === "tenant") await page.getByRole("button", { name: `编辑 ${tenant.name}`, exact: true }).waitFor()
    else {
      await page.getByRole("treeitem").filter({ hasText: childOrganization.name }).waitFor()
      for (const [label, href] of [["用户管理", "/system/users"], ["角色管理", "/system/roles"]]) {
        const link = page.getByRole("link", { name: label, exact: true })
        await link.waitFor()
        assert.equal(await link.getAttribute("href"), href, "Later administrator maintenance uses existing management pages.")
      }
    }
    return { kind, context, page, state, form: page.locator(`#${formId(kind)}`), dialog: page.getByRole("dialog") }
  } catch (error) {
    console.error("Entity types fixture startup diagnostic:", { kind, legacy, errors: state.errors, unexpected: state.unexpected })
    await context.close()
    throw error
  }
}

async function withFixture(kind, legacy, run, organizationType) {
  const f = await fixture(kind, legacy, organizationType)
  try {
    await run(f)
    assert.deepEqual(f.state.errors, [], "Production pages should have no uncaught browser errors.")
    assert.deepEqual(f.state.unexpected, [], "All API fixtures are intentional.")
  } catch (error) {
    console.error("Entity types scenario diagnostic:", { kind, legacy, errors: f.state.errors,
      unexpected: f.state.unexpected, saves: f.state.saves, text: await f.page.locator("body").innerText().catch(() => "") })
    throw error
  } finally { await f.context.close() }
}
const typeControl = (f) => f.form.getByRole("combobox", { name: new RegExp(`^${typeLabel(f.kind)}`) })
async function openCreate(f, parentId = rootOrganization.id) {
  if (f.kind === "tenant") await f.page.getByRole("button", { name: "新建租户", exact: true }).click()
  else if (parentId) await f.page.getByRole("treeitem").filter({ hasText: rootOrganization.name })
    .getByRole("button", { name: "新增子组织", exact: true }).click()
  else await f.page.getByRole("button", { name: "新增根组织", exact: true }).click()
  await f.dialog.getByRole("heading", { name: f.kind === "tenant" ? "新建租户" : "新增组织", exact: true }).waitFor()
}
async function openEdit(f) {
  if (f.kind === "tenant") await f.page.getByRole("button", { name: `编辑 ${tenant.name}`, exact: true }).click()
  else await f.page.getByRole("treeitem").filter({ hasText: childOrganization.name })
    .getByRole("button", { name: "编辑", exact: true }).click()
  await f.dialog.getByRole("heading", { name: f.kind === "tenant" ? "编辑租户" : `编辑组织「${childOrganization.name}」`, exact: true }).waitFor()
}
async function assertRemovedFields(f) {
  assert.equal(await f.form.locator('[name="rootOrganizationName"], [name^="admin"]').count(), 0,
    "Creation/editing must not contain root organization or administrator inputs.")
  assert.equal(await f.form.getByLabel(/根组织名称|管理员用户名|管理员姓名|管理员邮箱|初始密码/).count(), 0)
}
function assertSimplifiedPayload(payload) {
  for (const field of ["rootOrganizationName", "administrator", "adminUsername", "adminPassword", "adminRealName", "adminEmail"]) {
    assert.equal(Object.hasOwn(payload, field), false, `${field} must not be submitted by entity creation/editing.`)
  }
}
async function chooseType(f, label, verifyOptions = false) {
  await typeControl(f).click()
  const options = f.page.getByRole("option")
  await options.first().waitFor()
  if (verifyOptions) assert.deepEqual((await options.allTextContents()).map((text) => text.trim()),
    types[f.kind].map(([, optionLabel]) => optionLabel), "The selector exposes the supported entity types.")
  await f.page.getByRole("option", { name: label, exact: true }).click()
  assert.equal((await typeControl(f).innerText()).trim(), label)
}
async function save(f, editing = false) {
  const previous = f.state.saves.length
  await f.dialog.getByRole("button", { name: editing ? "保存更改" : "创建", exact: true }).click()
  await waitState(() => f.state.saves.length === previous + 1, "The real form submits its type without administrator/root-name validation.")
  await f.form.waitFor({ state: "hidden" })
  const submitted = f.state.saves.at(-1)
  assert.equal(submitted.method, editing ? "PUT" : "POST")
  assertSimplifiedPayload(submitted.body)
  if (!editing) {
    const credentials = f.page.getByRole("dialog").filter({ has: f.page.getByRole("heading", { name: "首位管理员已开通", exact: true }) })
    await credentials.waitFor()
    assert.equal(await credentials.getByLabel("登录用户名", { exact: true }).inputValue(), submitted.body.contactPhone || submitted.body.contactEmail)
    if (f.state.reuseAccount) {
      assert.equal(await credentials.getByLabel("初始密码", { exact: true }).count(), 0)
      assert.match(await credentials.innerText(), /使用原密码登录/)
    } else assert.equal(await credentials.getByLabel("初始密码", { exact: true }).inputValue(), "Browser-contact-password-123!")
    await credentials.getByRole("button", { name: "已保存，关闭", exact: true }).click()
  }
  return submitted.body
}

async function verifyCreateTypes(kind) {
  await withFixture(kind, false, async (f) => {
    for (const [index, [value, label]] of types[kind].entries()) {
      await openCreate(f)
      await assertRemovedFields(f)
      assert.equal((await typeControl(f).innerText()).trim(), "公司", "Each new form defaults to company.")
      if (kind === "tenant") {
        assert.match(await f.dialog.innerText(), /以租户名称建立根组织/)
        assert.match(await f.dialog.innerText(), /用户管理和角色管理中单独维护/)
      }
      const name = `${kind === "tenant" ? "租户" : "组织"} ${label} 验收`, code = `create-${kind}-${index}`
      await f.form.locator('input[name="name"]').fill(name)
      await f.form.locator('input[name="code"]').fill(code)
      if (index === 0) {
        const saved = f.state.saves.length
        await f.dialog.getByRole("button", { name: "创建", exact: true }).click()
        await f.form.getByText("请输入联系人姓名", { exact: true }).waitFor()
        assert.equal(f.state.saves.length, saved, "Contact-less UI creation must not submit.")
      }
      await f.form.locator('input[name="contactName"]').fill("首位联系人")
      await f.form.locator('input[name="contactPhone"]').fill("13800138010")
      await f.form.locator('input[name="contactEmail"]').fill("contact@example.com")
      await chooseType(f, label, index === 0)
      const payload = await save(f)
      assert.equal(payload.name, name)
      assert.equal(payload.code, code)
      assert.equal(payload.contactName, "首位联系人")
      assert.equal(payload.contactPhone, "13800138010")
      assert.equal(payload.contactEmail, "contact@example.com")
      assert.equal(payload[typeField(kind)], value, "The selected label submits the API enum value.")
      if (kind === "organization") {
        assert.equal(payload.tenantId, tenant.id)
        assert.equal(payload.parentId, rootOrganization.id, "Adding under a chosen tree node preserves the parent.")
      }
    }
    if (kind === "organization") {
      await openCreate(f, null)
      await assertRemovedFields(f)
      await f.form.locator('input[name="name"]').fill("新增根组织验收")
      await f.form.locator('input[name="code"]').fill("create-root-company")
      await f.form.locator('input[name="contactName"]').fill("首位联系人")
      await f.form.locator('input[name="contactEmail"]').fill("email-only@example.com")
      const payload = await save(f)
      assert.equal(payload.organizationType, "COMPANY")
      assert.equal(payload.parentId, null, "Root creation preserves a null parent.")
    }
  })
  console.log(`PASS ${kind}: all supported types submit with only name/code/type; default company, removed administrator/root-name fields${kind === "organization" ? ", child/root parents and existing user/role maintenance links" : ", root-name default guidance"}`)
}

async function verifyEdit(kind, legacy) {
  await withFixture(kind, legacy, async (f) => {
    await openEdit(f)
    await assertRemovedFields(f)
    const original = record(kind)
    assert.equal(await f.form.locator('input[name="contactName"]').inputValue(), "既有联系人")
    assert.equal(await f.form.locator('input[name="contactPhone"]').inputValue(), "13800138000")
    assert.equal(await f.form.locator('input[name="contactEmail"]').inputValue(), "existing@example.com")
    await f.form.locator('input[name="contactName"]').fill("修改后的业务联系人")
    const expectedType = legacy ? "COMPANY" : original[typeField(kind)]
    const expectedLabel = types[kind].find(([value]) => value === expectedType)[1]
    assert.equal((await typeControl(f).innerText()).trim(), expectedLabel,
      legacy ? "Records missing type hydrate company and remain editable." : "Editing hydrates the persisted nondefault type.")
    if (!legacy) await chooseType(f, kind === "tenant" ? "事业单位" : "分公司")
    const changedName = `${original.name}已更新`
    await f.form.locator('input[name="name"]').fill(changedName)
    const payload = await save(f, true)
    assert.equal(payload.name, changedName)
    assert.equal(payload.contactName, "修改后的业务联系人")
    assert.equal(payload.code, original.code)
    assert.equal(payload[typeField(kind)], legacy ? "COMPANY" : kind === "tenant" ? "PUBLIC_INSTITUTION" : "BRANCH")
    if (kind === "organization") {
      assert.equal(payload.tenantId, tenant.id)
      assert.equal(payload.parentId, rootOrganization.id, "Editing preserves the existing child parent.")
    }
  })
  console.log(`PASS ${kind} edit ${legacy ? "legacy" : "typed"}: ${legacy ? "missing type defaults company and saves" : "persisted nondefault type hydrates and changes save"}`)
}

async function verifyContactAlternatives(kind) {
  await withFixture(kind, false, async (f) => {
    for (const reuse of [false, true]) {
      f.state.reuseAccount = reuse
      await openCreate(f)
      await f.form.locator('input[name="name"]').fill("联系人邮箱开通验收")
      await f.form.locator('input[name="code"]').fill(`contact-${kind}-${reuse ? "existing" : "new"}`)
      await f.form.locator('input[name="contactName"]').fill("联系人验收")
      await f.form.locator('input[name="contactEmail"]').fill("email-only@example.com")
      const payload = await save(f)
      assert.equal(payload.contactPhone, "")
      assert.equal(payload.contactEmail, "email-only@example.com")
    }
  })
  console.log(`PASS ${kind}: email-only contact creates credentials; reused account keeps original password`)
}

async function verifyStoreOrganization() {
  await withFixture("organization", false, async (f) => {
    await f.page.getByRole("treeitem").filter({ hasText: childOrganization.name }).click()
    await f.page.locator("section").getByRole("heading", { name: childOrganization.name, exact: true }).waitFor()
    await f.page.locator("section").getByText("门店", { exact: true }).waitFor()
    await openEdit(f)
    assert.equal((await typeControl(f).innerText()).trim(), "门店", "A persisted store hydrates its organization type.")
    await f.form.locator('input[name="name"]').fill("门店资料已更新")
    const payload = await save(f, true)
    assert.equal(payload.organizationType, "STORE", "Unchanged store type survives an ordinary organization edit.")
    assert.equal(payload.tenantId, tenant.id)
    assert.equal(payload.parentId, rootOrganization.id, "Store edits keep the ordinary organization hierarchy.")
    assert.equal(payload.contactName, childOrganization.contactName)
    assert.equal(payload.contactPhone, childOrganization.contactPhone)
    assert.equal(payload.contactEmail, childOrganization.contactEmail)
  }, "STORE")
  console.log("PASS organization STORE: persisted store label, edit hydration and save preserve hierarchy and contacts")
}

async function verifyExplicitInitialization(kind) {
  await withFixture(kind, false, async (f) => {
    const button = (kind === "organization" ? f.page.locator("section") : f.page).getByRole("button", { name: "初始化首位管理员", exact: true })
    if (kind === "organization") {
      const row = f.page.getByRole("treeitem").filter({ hasText: childOrganization.name })
      const rowButton = row.getByRole("button", { name: "初始化首位管理员", exact: true })
      await rowButton.click()
      await f.page.getByRole("dialog").getByText(`为「${childOrganization.name}」使用已保存的联系人信息初始化管理员。请先保存联系人姓名，以及电话或 Email。`, { exact: true }).waitFor()
      await f.page.getByRole("dialog").getByRole("button", { name: "取消", exact: true }).click()
    }

    await button.click()
    const confirmation = f.page.getByRole("dialog")
    await confirmation.getByRole("button", { name: "取消", exact: true }).click()
    assert.equal(f.state.requests.filter((r) => r.pathname.endsWith("/initialize-administrator")).length, 0)
    for (const repeat of [false, true]) {
      await button.click()
      await confirmation.getByRole("button", { name: "确认初始化", exact: true }).click()
      await f.page.getByRole("heading", { name: repeat ? "首位管理员已初始化" : "首位管理员已开通", exact: true }).waitFor()
      assert.equal(await f.page.locator("#contact-admin-password").count(), repeat ? 0 : 1)
      assert.equal(await f.page.locator("#contact-admin-username").inputValue(), "13800138000")
      await f.page.getByRole("button", { name: "已保存，关闭", exact: true }).click()
    }
  })
  console.log(`PASS ${kind}: explicit initialization confirmation/cancel and idempotent result`)
}

async function verifyLoginIdentitySelection(sameOrganization, singleMembership = false) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1100 } })
  await installLegacyAuthMode(context)
  const page = await context.newPage()
  page.setDefaultTimeout(12_000)
  const state = { logins: 0, selections: [], errors: [], unexpected: [] }
  const first = { userId: "10000000-0000-7000-8000-000000000001", username: "contact-a", realName: "联系人甲",
    tenantId: "20000000-0000-7000-8000-000000000001", tenantName: "身份验收租户甲",
    organizationId: "30000000-0000-7000-8000-000000000001", organizationName: "身份验收组织甲", organizationPath: "身份验收组织甲",
    membershipId: fixtureUuid("entity-login-first"), memberType: "EMPLOYEE", identityVersion: "1", status: "ENABLED",
    consoleSelectable: true, selectionDisabledReason: null }
  const second = { ...first, userId: "10000000-0000-7000-8000-000000000002", username: "contact-b", realName: "联系人乙", membershipId: fixtureUuid("entity-login-second"),
    ...(sameOrganization ? {} : { tenantId: "20000000-0000-7000-8000-000000000002", tenantName: "身份验收租户乙",
      organizationId: "30000000-0000-7000-8000-000000000002", organizationName: "身份验收组织乙", organizationPath: "身份验收组织乙" }) }
  const expected = singleMembership ? first : second
  const manyOrganizations = Array.from({ length: 30 }, (_, index) => ({ ...first,
    organizationId: fixtureUuid(`first-tenant-org-${index}`), membershipId: fixtureUuid(`first-tenant-member-${index}`), organizationName: `甲租户分公司 ${index}`,
    organizationPath: `${first.organizationName} / 甲租户分公司 ${index}` }))
  const fixtureMemberships = singleMembership ? [first] : sameOrganization ? [first, second] : [first, ...manyOrganizations, second]
  page.on("pageerror", (error) => state.errors.push(error.message))
  await context.routeWebSocket(() => true, () => {})
  await context.route((url) => url.origin === origin && url.pathname === "/src/main.tsx", async (route) => {
    const response = await route.fetch()
    let body = await response.text()
    body = body.replace(/import App from ["']\/src\/App\.tsx(?:\?[^"']*)?["']/, 'import App from "/src/pages/login/LoginPage.tsx"')
    await route.fulfill({ response, body })
  })
  await context.route((url) => url.origin === origin && url.pathname.startsWith("/api/"), async (route) => {
    const request = route.request(), url = new URL(request.url())
    let value
    if (url.pathname === "/api/platform/branding") value = { platformName: "联系人验收平台", platformDescription: "", logoUrl: "",
      loginTitle: "联系人登录", loginDescription: "", footerText: "" }
    else if (url.pathname === "/api/auth/login") {
      assert.equal(request.postDataJSON().username, "13800138011")
      state.logins++
      value = { ticket: "verified-identity-fixture", accountCount: singleMembership ? 1 : 2, memberships: fixtureMemberships }
    } else if (url.pathname === "/api/auth/context") {
      assert.equal(request.headers().authorization, "Bearer verified-identity-fixture")
      const payload = request.postDataJSON()
      state.selections.push(payload)
      assert.equal(payload.userId, expected.userId)
      assert.equal(payload.organizationId, expected.organizationId)
      assert.equal(payload.membershipId, expected.membershipId)
      value = { accessToken: "selected-session-fixture", refreshToken: "selected-refresh-fixture",
        user: { id: expected.userId, username: expected.username, realName: expected.realName },
        tenant: { id: expected.tenantId }, organization: { id: expected.organizationId },
        membershipId: expected.membershipId, memberType: expected.memberType, identityVersion: expected.identityVersion, accountVersion: "0", expiresIn: 3600 }
    } else { state.unexpected.push(url.pathname); value = {} }
    await route.fulfill({ contentType: "application/json", body: JSON.stringify(value) })
  })
  try {
    await page.goto(`${origin}/login`, { waitUntil: "domcontentloaded" })
    await page.locator("#username").fill("13800138011")
    await page.locator("#password").fill("Fixture-login-password-123!")
    assert.equal(await page.getByLabel("租户编码").count(), 0, "Tenant selection must happen after password verification.")
    await page.getByRole("button", { name: "登 录", exact: true }).click()
    await page.getByRole("heading", { name: "选择登录租户", exact: true }).waitFor()
    assert.equal(state.selections.length, 0, "Even a single organization must wait for explicit selection.")
    assert.equal(await page.getByRole("button").filter({ hasText: "登录身份：" }).count(), 0,
      "Organization identities must not fill the tenant selection page.")
    const tenantSearch = page.getByRole("textbox", { name: "搜索租户", exact: true })
    await tenantSearch.fill("不存在的租户")
    await page.getByText("未找到匹配的租户", { exact: true }).waitFor()
    await tenantSearch.fill("")
    await page.getByRole("button").filter({ hasText: first.tenantName }).click()
    await page.getByRole("heading", { name: "选择组织与身份", exact: true }).waitFor()
    assert.equal(state.selections.length, 0, "Tenant choice must not establish a business session.")
    await page.getByRole("button", { name: "重新选择租户", exact: true }).click()
    await page.getByRole("heading", { name: "选择登录租户", exact: true }).waitFor()
    assert.equal(state.logins, 1, "Reselecting tenants reuses the verified credential ticket.")
    assert.equal(state.selections.length, 0)
    await page.getByRole("button").filter({ hasText: expected.tenantName }).click()
    await page.getByRole("heading", { name: "选择组织与身份", exact: true }).waitFor()
    if (!sameOrganization && !singleMembership) {
      assert.equal(await page.getByRole("button").filter({ hasText: "甲租户分公司" }).count(), 0,
        "The organization page only shows memberships in the chosen tenant.")
    }
    if (sameOrganization) assert.equal(await page.getByRole("button").filter({ hasText: first.organizationName }).count(), 2,
      "Two verified account identities remain distinguishable within the same organization.")
    const organizationSearch = page.getByRole("textbox", { name: "搜索组织或登录身份", exact: true })
    await organizationSearch.fill("不存在的组织")
    await page.getByText("未找到匹配的组织或登录身份", { exact: true }).waitFor()
    await organizationSearch.fill(singleMembership ? expected.organizationName : expected.realName)
    const choice = page.getByRole("button").filter({ hasText: singleMembership ? expected.organizationName : expected.realName })
    await choice.click()
    await waitState(() => state.selections.length === 1, "The selected account and organization must both be submitted.")
    await page.waitForURL(`${origin}/`)
    assert.equal(await page.evaluate((key) => JSON.parse(localStorage.getItem(key) || "null")?.refreshToken, SESSION_KEY), "selected-refresh-fixture",
      "Login should retain the refresh token from the selected context.")
    assert.deepEqual(state.errors, [])
    assert.deepEqual(state.unexpected, [])
  } finally { await context.close() }
  console.log(`PASS login: explicit tenant then organization selection, search, reselect tenant without reauthentication; ${singleMembership ? "single membership never auto-enters" : sameOrganization ? "distinct verified identities within the same organization" : "large tenant membership lists stay separate"}`)
}

try {
  await startServer()
  browser = await launchBrowser()
  for (const kind of ["tenant", "organization"]) {
    await verifyCreateTypes(kind)
    await verifyEdit(kind, false)
    await verifyEdit(kind, true)
    await verifyContactAlternatives(kind)
    await verifyExplicitInitialization(kind)
  }
  await verifyStoreOrganization()
  await verifyLoginIdentitySelection(false)
  await verifyLoginIdentitySelection(true)
  await verifyLoginIdentitySelection(false, true)
  console.log("Entity contacts/types/login browser integration passed (27 scenarios); HTTP fixtures do not assert database/backend behavior.")
} finally {
  await browser?.close()
  if (vite && vite.exitCode === null) {
    vite.kill()
    await Promise.race([new Promise((resolve) => vite.once("exit", resolve)), pause(5000)])
    if (vite.exitCode === null) vite.kill("SIGKILL")
  }
}
