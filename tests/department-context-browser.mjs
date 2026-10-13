/**
 * Department management context regression through the production React page.
 * Vite and an existing Chromium/Edge run real selectors/forms; HTTP fixtures
 * isolate persistence. Run with Node 24. No backend or browser install is needed.
 */
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { existsSync } from "node:fs"
import { createRequire } from "node:module"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const require = createRequire(import.meta.url)
const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const bundledNode = path.join(os.homedir(), ".cache/codex-runtimes/codex-primary-runtime/dependencies/node")
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || path.join(bundledNode, "node_modules/playwright"))
assert(Number(process.versions.node.split(".")[0]) >= 20, "Run this script with Node 24.")
const browserExecutable = process.env.BROWSER_EXECUTABLE || [chromium.executablePath(),
  path.join(process.env["ProgramFiles(x86)"] || "C:/Program Files (x86)", "Microsoft/Edge/Application/msedge.exe"),
  path.join(process.env.ProgramFiles || "C:/Program Files", "Microsoft/Edge/Application/msedge.exe"),
].find(existsSync)
assert(browserExecutable, "Existing Chromium or Edge is required; this test does not install browsers.")
const bindHost = process.env.DEPARTMENT_CONTEXT_BROWSER_BIND || "::1"
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const deferred = () => {
  let release
  const promise = new Promise((resolve) => { release = resolve })
  return { promise, release }
}
async function waitState(predicate, message, timeout = 12_000) {
  const deadline = Date.now() + timeout
  while (!(await predicate()) && Date.now() < deadline) await pause(25)
  assert(await predicate(), message)
}
const freePort = () => new Promise((resolve, reject) => {
  const server = net.createServer().on("error", reject)
  server.listen(0, bindHost, () => {
    const port = server.address().port
    server.close((error) => error ? reject(error) : resolve(port))
  })
})
const org = (id, name, tenantId, parentId = null, children = []) => ({
  id, name, code: id, tenantId, parentId, organizationType: "COMPANY", status: "ENABLED",
  sortOrder: 0, description: "", children,
})
const tenantA = { id: "context-tenant-a", name: "部门验收租户甲", code: "tenant-a", isPlatform: true }
const tenantB = { id: "context-tenant-b", name: "部门验收租户乙", code: "tenant-b", isPlatform: false }
const deepChild = org("deep-child", "四级下属组织", tenantA.id, "grandchild")
const grandchild = org("grandchild", "三级下属组织", tenantA.id, "child-a", [deepChild])
const childA = org("child-a", "下属子公司甲", tenantA.id, "current-a", [grandchild])
const childB = org("child-b", "下属分公司乙", tenantA.id, "current-a")
const currentA = org("current-a", "登录组织甲", tenantA.id, "ancestor", [childA, childB])
const siblingChild = org("sibling-child", "切换后下属分公司", tenantA.id, "sibling")
const sibling = org("sibling", "同租户另一登录组织", tenantA.id, "ancestor", [siblingChild])
const ancestor = org("ancestor", "禁止切换的上级组织", tenantA.id, null, [currentA, sibling])
const unrelated = org("unrelated", "禁止切换的独立组织", tenantA.id)
const otherChild = org("other-child", "新租户下属公司", tenantB.id, "other-current")
const otherCurrent = org("other-current", "新租户登录组织", tenantB.id, "other-root", [otherChild])
const otherRoot = org("other-root", "新租户上级组织", tenantB.id, null, [otherCurrent])
const profile = (tenant, organization) => ({
  user: { id: "department-fixture-user", username: "department-fixture", realName: "部门验收管理员",
    email: "", phone: "", avatar: "", gender: "UNKNOWN", isSuperAdmin: true },
  tenant, organization: { id: organization.id, name: organization.name, path: `/${organization.id}` },
  applications: [], memberships: [],
})
const sessions = [
  { token: "department-session-a", profile: profile(tenantA, currentA), tree: [unrelated, ancestor] },
  { token: "department-session-sibling", profile: profile(tenantA, sibling), tree: [unrelated, ancestor] },
  { token: "department-session-other", profile: profile(tenantB, otherCurrent), tree: [otherRoot] },
]
const knownOrganizations = [currentA, childA, childB, grandchild, deepChild, sibling, siblingChild,
  otherCurrent, otherChild, ancestor, unrelated, otherRoot]
const department = (organization) => ({
  id: `department-${organization.id}`, name: `${organization.name}的部门`, code: `dept-${organization.id}`,
  organizationId: organization.id, parentId: null, sortOrder: 0, description: "", children: [],
})
let browser, vite, origin, startupOutput = "", scenarioCount = 0

async function startServer() {
  const port = await freePort()
  origin = `http://${bindHost.includes(":") ? `[${bindHost}]` : bindHost}:${port}`
  vite = spawn(process.execPath, [path.join(workspace, "frontend/node_modules/vite/bin/vite.js"),
    "--host", bindHost, "--port", String(port), "--strictPort"], {
    cwd: path.join(workspace, "frontend"), windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
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

async function fixture(options = {}) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1100 } })
  const page = await context.newPage()
  page.setDefaultTimeout(12_000)
  const state = { requests: [], completed: [], saves: [], errors: [], unexpected: [],
    organizations: new Map(sessions.map((session) => [session.token, { tree: structuredClone(session.tree) }])),
    departments: new Map(knownOrganizations.map((organization) => [organization.id, { tree: [department(organization)] }])),
  }
  Object.assign(state.organizations.get(sessions[0].token), options.organizations)
  Object.assign(state.departments.get(currentA.id), options.departments)
  page.on("pageerror", (error) => state.errors.push(error.message))
  await context.addInitScript((token) => {
    localStorage.setItem("dante-token", token)
    localStorage.setItem("dante-theme", "light")
  }, sessions[0].token)
  await context.routeWebSocket(() => true, () => {})
  await context.route((url) => url.origin === origin && url.pathname === "/src/main.tsx", async (route) => {
    const response = await route.fetch()
    let body = await response.text()
    const appImport = /import App from ["']\/src\/App\.tsx(?:\?[^"']*)?["']/
    assert.match(body, appImport, "Mount the production department page from the real Vite entry.")
    body = body.replace(appImport, 'import App from "/src/pages/system/departments/DepartmentsPage.tsx"')
    body = body.replace("ReactDOM.createRoot", `import { authStore } from "/src/stores/auth.ts";
      authStore.setState({ profile: ${JSON.stringify(options.noProfile ? null : sessions[0].profile)} });
      window.__departmentFixture = {
        refreshProfile: (profile) => authStore.getState().setProfile(profile),
        beginSwitch: () => authStore.getState().beginContextSwitch(),
        finishSwitch: (profile, token) => authStore.setState({ profile, token, contextSwitching: false }),
        invalidateOrganizations: () => queryClient.invalidateQueries({ queryKey: ["department-organizations"] }),
        invalidateDepartments: () => queryClient.invalidateQueries({ queryKey: ["dept-tree"] }),
      };
      ReactDOM.createRoot`)
    await route.fulfill({ response, body })
  })
  const fulfill = async (route, item, value, status = 200) => {
    try {
      await route.fulfill({ status, contentType: "application/json", body: JSON.stringify(value) })
    } catch (error) {
      if (!/closed|disposed|Invalid Interception|Target|cancelled/i.test(error.message)) throw error
    } finally {
      state.completed.push(item)
    }
  }
  await context.route((url) => url.origin === origin && url.pathname.startsWith("/api/"), async (route) => {
    const request = route.request(), url = new URL(request.url())
    const token = request.headers().authorization?.replace(/^Bearer /, "")
    const item = { pathname: url.pathname, method: request.method(), token, params: Object.fromEntries(url.searchParams),
      body: request.postData() ? request.postDataJSON() : undefined }
    state.requests.push(item)
    if (!sessions.some((session) => session.token === token)) {
      state.unexpected.push({ ...item, reason: "missing fixture session" })
      return fulfill(route, item, { message: "Missing fixture session" }, 503)
    }
    let config
    if (item.pathname === "/api/departments/organizations" && item.method === "GET") {
      assert.deepEqual(item.params, {}, "Organization choices come from the authenticated session without a tenant picker.")
      config = state.organizations.get(token)
    } else if (item.pathname === "/api/departments/tree" && item.method === "GET") {
      config = state.departments.get(item.params.organizationId)
      assert(config, `Known department organization: ${item.params.organizationId}`)
    } else if (item.pathname === "/api/departments" && item.method === "POST") {
      state.saves.push(item)
      const saved = { ...item.body, id: `created-department-${state.saves.length}`, children: [] }
      state.departments.get(saved.organizationId).tree.push(saved)
      return fulfill(route, item, { id: saved.id })
    } else {
      state.unexpected.push(item)
      return fulfill(route, item, { message: `Unexpected API ${item.pathname}` }, 503)
    }
    const value = config.error ? { message: config.error } : structuredClone(config.tree)
    const status = config.error ? 503 : 200
    if (config.hold) await config.hold.promise
    return fulfill(route, item, value, status)
  })
  try {
    await page.goto(`${origin}/system/departments`, { waitUntil: "domcontentloaded" })
    await page.getByRole("heading", { name: "部门管理", exact: true }).waitFor()
    await page.waitForFunction(() => !!window.__departmentFixture)
    return { context, page, state }
  } catch (error) {
    console.error("Department fixture startup diagnostic:", { errors: state.errors, unexpected: state.unexpected })
    await context.close()
    throw error
  }
}

async function withFixture(name, options, run) {
  const f = await fixture(options)
  try {
    await run(f)
    assert.deepEqual(f.state.errors, [], "Production page should have no uncaught browser errors.")
    assert.deepEqual(f.state.unexpected, [], "Department management must not fetch tenants/organization-management data or switch auth context.")
    scenarioCount += 1
    console.log(`PASS ${name}`)
  } catch (error) {
    console.error("Department scenario diagnostic:", { name, errors: f.state.errors, unexpected: f.state.unexpected,
      requests: f.state.requests, saves: f.state.saves, text: await f.page.locator("body").innerText().catch(() => "") })
    throw error
  } finally {
    for (const config of [...f.state.organizations.values(), ...f.state.departments.values()]) config.hold?.release()
    await f.context.close()
  }
}
const picker = (f) => f.page.getByRole("combobox")
const rootCreate = (f) => f.page.getByRole("button", { name: "新增根部门", exact: true }).first()
const departmentRequests = (f) => f.state.requests.filter((request) => request.pathname === "/api/departments/tree")
async function selected(f, organization, withDepartment = true) {
  await waitState(async () => (await picker(f).innerText()).trim() === organization.name,
    `Selected organization should be ${organization.name}.`)
  await f.page.getByText(`${organization.name} · 部门树`, { exact: true }).waitFor()
  if (withDepartment) await f.page.getByRole("treeitem").filter({ hasText: department(organization).name }).waitFor()
}
async function openPicker(f) {
  await picker(f).click()
  const popup = f.page.locator('[data-slot="popover-content"]')
  await popup.waitFor()
  return popup
}
async function selectOrganization(f, organization, withDepartment = true) {
  const popup = await openPicker(f)
  await popup.getByText(organization.name, { exact: true }).click()
  await popup.waitFor({ state: "hidden" })
  await selected(f, organization, withDepartment)
}
async function assertChoices(f, allowed, excluded) {
  const popup = await openPicker(f)
  for (const organization of allowed) await popup.getByText(organization.name, { exact: true }).waitFor()
  for (const organization of excluded) assert.equal(await popup.getByText(organization.name, { exact: true }).count(), 0,
    `${organization.name} must not be offered outside the logged-in organization subtree.`)
  await f.page.keyboard.press("Escape")
  await popup.waitFor({ state: "hidden" })
}
async function saveRootDepartment(f, organization) {
  const previous = f.state.saves.length
  await rootCreate(f).click()
  const dialog = f.page.getByRole("dialog")
  await dialog.getByRole("heading", { name: "新增部门", exact: true }).waitFor()
  await dialog.locator('input[name="name"]').fill(`${organization.name}新增部门`)
  await dialog.locator('input[name="code"]').fill(`create-${organization.id}`)
  await dialog.getByRole("button", { name: "创建", exact: true }).click()
  await waitState(() => f.state.saves.length === previous + 1, "Create should submit the selected department organization.")
  await dialog.waitFor({ state: "hidden" })
  assert.equal(f.state.saves.at(-1).body.organizationId, organization.id)
  assert.equal(f.state.saves.at(-1).body.parentId, null)
  assert.equal(f.state.saves.at(-1).token, sessions[0].token, "Local department organization selection keeps the login context.")
}
async function beginSwitch(f) {
  await f.page.evaluate(() => window.__departmentFixture.beginSwitch())
  await waitState(async () => await rootCreate(f).isDisabled(), "Creation should be disabled while login context changes.")
}
async function finishSwitch(f, session) {
  await f.page.evaluate(({ profile, token }) => window.__departmentFixture.finishSwitch(profile, token), session)
}
async function settleBrowser(f) {
  await f.page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
}

try {
  await startServer()
  browser = await chromium.launch({ headless: true, executablePath: browserExecutable, args: ["--no-proxy-server"] })

  await withFixture("nested login organization defaults immediately; only descendants are selectable", {}, async (f) => {
    await selected(f, currentA)
    assert.equal(departmentRequests(f)[0].params.organizationId, currentA.id,
      "Default department request uses profile.organization.id, not the first tree/root.")
    assert.equal(f.state.requests.find((request) => request.pathname === "/api/departments/organizations").token, sessions[0].token)
    await assertChoices(f, [currentA, childA, childB, grandchild], [ancestor, sibling, siblingChild, unrelated])
    const popup = await openPicker(f)
    await popup.getByText(grandchild.name, { exact: true }).locator("..").getByRole("button", { name: "展开", exact: true }).click()
    await popup.getByText(deepChild.name, { exact: true }).click()
    await selected(f, deepChild)
    assert.equal(departmentRequests(f).at(-1).params.organizationId, deepChild.id,
      "A deeply nested descendant remains manageable.")
  })

  await withFixture("child selection loads its departments and creates there; login organization remains selectable", {}, async (f) => {
    await selected(f, currentA)
    await selectOrganization(f, childA)
    assert.equal(departmentRequests(f).at(-1).params.organizationId, childA.id)
    await saveRootDepartment(f, childA)
    await f.page.getByRole("treeitem").filter({ hasText: `${childA.name}新增部门` }).waitFor()
    await selectOrganization(f, currentA)
    await saveRootDepartment(f, currentA)
  })

  await withFixture("manual descendant choice survives same-context profile refresh and organization refetch", {}, async (f) => {
    await selected(f, currentA)
    await selectOrganization(f, grandchild)
    await f.page.evaluate((profile) => window.__departmentFixture.refreshProfile(profile), {
      ...sessions[0].profile, user: { ...sessions[0].profile.user, realName: "资料刷新后管理员" },
    })
    await selected(f, grandchild)
    const previous = f.state.requests.length
    await f.page.evaluate(() => window.__departmentFixture.invalidateOrganizations())
    assert(f.state.requests.slice(previous).some((request) => request.pathname === "/api/departments/organizations"),
      "The organization choices were actually refetched.")
    await selected(f, grandchild)
  })

  await withFixture("context changes reset the default and ignore delayed old organization responses", {}, async (f) => {
    await selected(f, currentA)
    await selectOrganization(f, childA)
    const oldTree = deferred(), siblingTree = deferred(), otherTree = deferred()
    f.state.organizations.get(sessions[0].token).hold = oldTree
    await f.page.evaluate(() => { void window.__departmentFixture.invalidateOrganizations() })
    await waitState(() => f.state.requests.filter((request) => request.pathname === "/api/departments/organizations" &&
      request.token === sessions[0].token).length >= 2, "Old organization refetch should be in flight.")
    f.state.organizations.get(sessions[1].token).hold = siblingTree
    await rootCreate(f).click()
    const oldDraft = f.page.getByRole("dialog")
    await oldDraft.getByRole("heading", { name: "新增部门", exact: true }).waitFor()
    await oldDraft.locator('input[name="name"]').fill("旧登录组织的未提交部门")
    const savedBeforeSwitch = f.state.saves.length
    await beginSwitch(f)
    await oldDraft.waitFor({ state: "hidden" })
    assert.equal(await f.page.getByText(`${childA.name} · 部门树`, { exact: true }).count(), 0)
    await finishSwitch(f, sessions[1])
    await selected(f, sibling)
    assert.equal(f.state.saves.length, savedBeforeSwitch,
      "Switching login context closes the old department draft without posting it under either organization.")
    await waitState(() => f.state.requests.some((request) => request.pathname === "/api/departments/organizations" &&
      request.token === sessions[1].token), "Same-tenant organization switch refetches its authorized subtree.")
    oldTree.release()
    await waitState(() => f.state.completed.filter((request) => request.pathname === "/api/departments/organizations" &&
      request.token === sessions[0].token).length >= 2, "The old tree response has been released or cancelled.")
    await settleBrowser(f)
    await selected(f, sibling)
    siblingTree.release()
    await waitState(async () => !(await picker(f).isDisabled()), "The new subtree becomes selectable after loading.")
    await assertChoices(f, [sibling, siblingChild], [ancestor, currentA, childA, unrelated])
    await selectOrganization(f, siblingChild)
    f.state.organizations.get(sessions[2].token).hold = otherTree
    await beginSwitch(f)
    await finishSwitch(f, sessions[2])
    await selected(f, otherCurrent)
    assert.equal(departmentRequests(f).at(-1).params.organizationId, otherCurrent.id)
    assert.equal(departmentRequests(f).at(-1).token, sessions[2].token)
    otherTree.release()
    await waitState(async () => !(await picker(f).isDisabled()), "New tenant choices finish loading.")
    await assertChoices(f, [otherCurrent, otherChild], [otherRoot, currentA, sibling, unrelated])
  })

  await withFixture("delayed old department response cannot overwrite a new login organization", {}, async (f) => {
    await selected(f, currentA)
    const oldDepartment = deferred()
    f.state.departments.get(childA.id).hold = oldDepartment
    await selectOrganization(f, childA, false)
    await waitState(() => departmentRequests(f).some((request) => request.params.organizationId === childA.id),
      "Selected descendant department request should be pending.")
    await beginSwitch(f)
    await finishSwitch(f, sessions[2])
    await selected(f, otherCurrent)
    oldDepartment.release()
    await waitState(() => f.state.completed.some((request) => request.pathname === "/api/departments/tree" &&
      request.params.organizationId === childA.id), "The old department response has been released or cancelled.")
    await settleBrowser(f)
    await selected(f, otherCurrent)
    assert.equal(await f.page.getByRole("treeitem").filter({ hasText: department(childA).name }).count(), 0)
  })

  await withFixture("profile restoration does not fetch arbitrary tenants or departments", { noProfile: true }, async (f) => {
    assert.equal(await rootCreate(f).isDisabled(), true)
    assert.deepEqual(f.state.requests, [], "No API context should be inferred before the login profile is available.")
    await f.page.evaluate((profile) => window.__departmentFixture.refreshProfile(profile), sessions[0].profile)
    await selected(f, currentA)
  })

  await withFixture("organization loading retains the current organization department default", {
    organizations: { hold: deferred() },
  }, async (f) => {
    await selected(f, currentA)
    assert.equal(await picker(f).isDisabled(), true, "Subtree switching waits for organization choices.")
    assert(departmentRequests(f).every((request) => request.params.organizationId === currentA.id))
    f.state.organizations.get(sessions[0].token).hold.release()
    await waitState(async () => !(await picker(f).isDisabled()), "Organization selector should recover when choices arrive.")
    await selectOrganization(f, childB)
  })

  for (const [name, tree] of [["empty", []], ["missing-current", [unrelated]]]) {
    await withFixture(`${name} organization tree keeps the profile default and never selects an unrelated first node`, {
      organizations: { tree },
    }, async (f) => {
      await selected(f, currentA)
      assert(departmentRequests(f).every((request) => request.params.organizationId === currentA.id))
      await assertChoices(f, [currentA], [unrelated, ancestor, sibling, childA])
    })
  }

  await withFixture("organization failure preserves current departments and retry restores descendant choices", {
    organizations: { error: "组织范围查询失败验收" },
  }, async (f) => {
    await selected(f, currentA)
    await f.page.getByText("下级组织加载失败，当前登录组织的部门仍可管理。", { exact: true }).waitFor()
    assert(departmentRequests(f).every((request) => request.params.organizationId === currentA.id))
    delete f.state.organizations.get(sessions[0].token).error
    await f.page.getByRole("button", { name: /重试/ }).click()
    await waitState(async () => !(await picker(f).isDisabled()), "Organization retry should restore available choices.")
    await selectOrganization(f, childA)
  })

  await withFixture("department failure displays its error and retry keeps the selected organization", {
    departments: { error: "部门查询失败验收" },
  }, async (f) => {
    await selected(f, currentA, false)
    await f.page.getByText("部门查询失败验收", { exact: true }).waitFor()
    delete f.state.departments.get(currentA.id).error
    await f.page.getByRole("button", { name: "重试", exact: true }).click()
    await selected(f, currentA)
  })

  console.log(`Department context browser integration passed (${scenarioCount} scenarios); HTTP fixtures do not assert database/backend behavior.`)
} finally {
  await browser?.close()
  if (vite && vite.exitCode === null) {
    vite.kill()
    await Promise.race([new Promise((resolve) => vite.once("exit", resolve)), pause(5000)])
    if (vite.exitCode === null) vite.kill("SIGKILL")
  }
}
