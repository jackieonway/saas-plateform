/**
 * Platform branding/settings regression: real Vite/React pages and Chromium.
 * HTTP API responses are isolated fixtures; no live backend, database or credentials.
 * Run with Node 20+: node tests/platform-settings-browser.mjs [scenario]
 * PLATFORM_BROWSER_BIND and PLATFORM_BROWSER_PORT
 * may override existing runtimes and the owned Vite listener.
 */
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import net from "node:net"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { launchBrowser } from "./browser/runtime.mjs"

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const bindHost = process.env.PLATFORM_BROWSER_BIND || (process.platform === "win32" ? "::1" : "127.0.0.1")
const port = process.env.PLATFORM_BROWSER_PORT ? Number(process.env.PLATFORM_BROWSER_PORT) : await new Promise((resolve, reject) => {
  const probe = net.createServer().on('error', reject)
  probe.listen(0, bindHost, () => {
    const available = probe.address().port
    probe.close((error) => error ? reject(error) : resolve(available))
  })
})
assert(Number.isInteger(port) && port > 0 && port <= 65535, "PLATFORM_BROWSER_PORT must be a valid port.")
const origin = `http://${bindHost.includes(":") ? `[${bindHost}]` : bindHost}:${port}`
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const application = { id: "platform-browser-system", code: "system", name: "系统管理", icon: "Settings2", basePath: "/system",
  entryType: "HTML", runtimeMode: "TRUSTED" }
const originalSettings = {
  platformName: "星河管理平台", platformDescription: "星河企业统一工作空间",
  logoUrl: "/__branding/logo.svg", loginTitle: "欢迎登录星河",
  loginDescription: "使用企业账号进入工作空间", footerText: "© 2026 星河企业",
  platformTenantCode: "admin-tenant", platformTenantName: "星河平台管理租户",
  primaryColor: "#008577",
}
const labels = {
  platformName: "平台名称", platformDescription: "平台描述", logoUrl: "Logo 地址",
  loginTitle: "登录页标题", loginDescription: "登录页说明", footerText: "页脚文案",
  platformTenantName: "平台租户名称",
  primaryColor: "主题主色",
}
function branding(settings) {
  const { platformTenantCode: _code, platformTenantName: _tenant, ...publicSettings } = settings
  return publicSettings
}
function profile(state) {
  return {
    user: { id: "platform-browser-user", username: "platform-browser", realName: "品牌验收管理员",
      email: "", phone: "", avatar: "", gender: "UNKNOWN", isSuperAdmin: false },
    tenant: { id: "platform-browser-tenant", code: state.isPlatform ? "admin-tenant" : "customer-tenant",
      name: state.isPlatform ? state.settings.platformTenantName : "客户租户", isPlatform: state.isPlatform },
    organization: { id: "platform-browser-org", name: "平台管理组织", path: "/platform-browser-org/" },
    applications: [application],
    memberships: [{ tenantId: "platform-browser-tenant", tenantName: state.isPlatform ? state.settings.platformTenantName : "客户租户",
      organizationId: "platform-browser-org", organizationName: "平台管理组织", organizationPath: "/platform-browser-org/" }],
  }
}

let browser, vite, startupOutput = ""
async function startServer() {
  vite = spawn(process.execPath, [path.join(workspace, "node_modules/vite/bin/vite.js"),
    "--host", bindHost, "--port", String(port), "--strictPort"], {
    cwd: workspace, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
  })
  vite.stdout.on("data", (chunk) => { startupOutput = (startupOutput + chunk).slice(-4000) })
  vite.stderr.on("data", (chunk) => { startupOutput = (startupOutput + chunk).slice(-4000) })
  let spawnError
  vite.on("error", (error) => { spawnError = error })
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    if (spawnError) throw spawnError
    if (vite.exitCode !== null) throw new Error(`Owned Vite server stopped: ${startupOutput}`)
    if (startupOutput.includes("ready in")) return
    await pause(100)
  }
  throw new Error(`Owned Vite server did not become ready: ${startupOutput}`)
}

async function withFixture(options, run) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1100 } })
  const page = await context.newPage()
  page.setDefaultTimeout(12_000)
  const state = { authenticated: true, isPlatform: true, hasPermission: true, settings: structuredClone(originalSettings),
    requests: [], saves: [], errors: [], routeErrors: [], failSave: false, theme: "light", ...options }
  page.on("pageerror", (error) => state.errors.push(error.message))
  await context.addInitScript(({ authenticated, theme }) => {
    if (authenticated) if (!localStorage.getItem("platform-session-v1")) localStorage.setItem("platform-session-v1", JSON.stringify({ schema: 1, version: 1, sessionId: crypto.randomUUID(), status: "active", token: "test-platform-settings-session", refreshToken: null }))
    localStorage.setItem("platform-theme", theme)
  }, { authenticated: state.authenticated, theme: state.theme })
  // A notification transport is unrelated to branding; prevent any live service connection.
  await context.routeWebSocket(/\/socket\.io\//, (socket) => socket.close())
  await context.route((url) => url.origin === origin && url.pathname.startsWith("/__branding/"), (route) => {
    if (new URL(route.request().url()).pathname === "/__branding/missing.svg") return route.fulfill({ status: 404, body: "missing" })
    return route.fulfill({ status: 200, contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="8" fill="#0f766e"/><circle cx="16" cy="16" r="8" fill="white"/></svg>' })
  })
  const fulfill = (route, value, status = 200) => route.fulfill({ status, contentType: "application/json", body: JSON.stringify(value) })
  await context.route((url) => url.origin === origin && url.pathname.startsWith("/api/"), async (route) => {
    try {
      const request = route.request(), url = new URL(request.url())
      const record = { pathname: url.pathname, method: request.method(), authorization: request.headers().authorization,
        body: request.postData() ? request.postDataJSON() : undefined }
      state.requests.push(record)
      if (url.pathname === "/api/platform/branding") {
        assert.equal(record.authorization, undefined, "Public branding must omit the session Authorization header, including when logged in.")
        return fulfill(route, branding(state.settings))
      }
      assert.equal(record.authorization, "Bearer test-platform-settings-session", "Private API requests require the captured session.")
      if (url.pathname === "/api/auth/profile") return fulfill(route, profile(state))
      if (url.pathname === "/api/auth/permissions") return fulfill(route, { roles: [], permissions: state.hasPermission ? ["system:settings"] : [] })
      if (url.pathname === "/api/auth/menus") return fulfill(route, [{
        id: "platform-settings-menu", parentId: null, type: "MENU", title: "平台设置", path: "/system/settings",
        icon: "Settings2", permission: "system:settings", sortOrder: 1, visible: true, children: [],
      }])
      if (url.pathname === "/api/platform/settings") {
        if (!state.isPlatform || !state.hasPermission) return fulfill(route, { code: "FORBIDDEN", message: "无平台设置权限" }, 403)
        if (record.method === "GET") return fulfill(route, state.settings)
        assert.equal(record.method, "PUT")
        state.saves.push(record.body)
        assert.equal(Object.hasOwn(record.body, "platformTenantCode"), false, "The fixed tenant code must never be serialized for updates.")
        if (state.failSave) return fulfill(route, { code: "SERVICE_UNAVAILABLE", message: "配置暂时无法保存，请稍后重试" }, 503)
        state.settings = { ...state.settings, ...record.body }
        return fulfill(route, state.settings)
      }
      if (url.pathname === "/api/notifications/unread-count") return fulfill(route, { count: 0 })
      if (url.pathname === "/api/notifications/prefs") return fulfill(route, { mutedTypes: [], dndEnabled: false, dndStart: "22:00", dndEnd: "08:00" })
      if (url.pathname === "/api/search") return fulfill(route, { users: [], tenants: [], organizations: [], roles: [] })
      throw new Error(`Unexpected API endpoint: ${record.method} ${url.pathname}`)
    } catch (error) {
      state.routeErrors.push(error.message)
      await fulfill(route, { code: "FIXTURE_ERROR", message: error.message }, 500)
    }
  })
  try {
    await page.goto(origin + (state.startPath || (state.authenticated ? "/system/settings" : "/login")))
    await run({ page, state })
    assert.deepEqual(state.errors, [], "The real application must not throw browser errors.")
    assert.deepEqual(state.routeErrors, [], "All API traffic must satisfy the isolated fixture contract.")
  } finally {
    await context.close()
  }
}

async function readySettings(page) {
  await page.getByLabel("平台租户名称", { exact: true }).waitFor()
  await page.waitForFunction((tenantName) => document.querySelector('button[aria-label="切换组织上下文"]')?.textContent.includes(tenantName), originalSettings.platformTenantName)
}
async function saveThemeColor(page, state, color) {
  const normalized = color.toUpperCase()
  const saveCount = state.saves.length
  await page.getByLabel("主题主色", { exact: true }).fill(color)
  // A disabled submit button can also mean the request is still in flight.
  // Bind completion to this color's response before inspecting the saved state.
  const responsePromise = page.waitForResponse((response) => {
    const request = response.request()
    return new URL(response.url()).pathname === "/api/platform/settings"
      && request.method() === "PUT" && request.postDataJSON()?.primaryColor === normalized
  })
  await page.getByRole("button", { name: "保存设置", exact: true }).click()
  const response = await responsePromise
  assert.equal(response.ok(), true, "The current theme update must succeed.")
  assert.equal(await response.finished(), null)
  await page.waitForFunction((savedColor) => {
    const submit = document.querySelector('button[type="submit"]')
    const fieldset = document.querySelector("fieldset")
    return document.querySelector('input[name="primaryColor"]')?.value === savedColor
      && submit?.textContent.trim() === "保存设置" && submit.disabled
      && fieldset?.disabled === false
  }, normalized)
  assert.equal(state.saves.length, saveCount + 1)
  assert.equal(state.saves.at(-1).primaryColor, normalized, "Custom colors must be normalized before saving.")
}
async function expectTitle(page, title) {
  await page.waitForFunction((expected) => document.title === expected, title)
}
async function expectLogin(page, settings) {
  await page.getByRole("heading", { name: settings.loginTitle, exact: true }).waitFor()
  await page.getByText(settings.platformName, { exact: true }).first().waitFor()
  await page.getByText(settings.platformDescription, { exact: true }).first().waitFor()
  await page.getByText(settings.loginDescription, { exact: true }).waitFor()
  if (settings.footerText) await page.getByText(settings.footerText, { exact: true }).waitFor()
  await expectTitle(page, `登录 · ${settings.platformName}`)
  assert.equal(await page.locator('meta[name="description"]').getAttribute("content"), settings.platformDescription)
  assert.equal(await page.locator('link[rel="icon"]').getAttribute("href"), settings.logoUrl || "/logo.svg")
}

async function expectThemeContrast(page, dark) {
  const ratio = await page.evaluate(() => {
    const root = document.documentElement
    const channels = (hex) => [1, 3, 5].map((offset) => parseInt(hex.slice(offset, offset + 2), 16) / 255)
    const luminance = (rgb) => {
      const values = rgb.map((value) => value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4)
      return values[0] * 0.2126 + values[1] * 0.7152 + values[2] * 0.0722
    }
    const contrast = (first, second) => {
      const a = luminance(first), b = luminance(second)
      return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05)
    }
    const variable = (name) => channels(root.style.getPropertyValue(name))
    const sidebarPrimary = variable("--sidebar-primary"), sidebar = variable("--sidebar")
    const selected = sidebar.map((channel, index) => channel * 0.8 + sidebarPrimary[index] * 0.2)
    return {
      primary: contrast(variable("--primary"), variable("--primary-foreground")),
      sidebar: contrast(sidebarPrimary, sidebar), selected: contrast(sidebarPrimary, selected),
      dark: root.classList.contains("dark"), mode: localStorage.getItem("platform-theme"),
    }
  })
  assert(ratio.primary >= 4.5 && ratio.sidebar >= 4.5 && ratio.selected >= 4.5, "Custom colors must retain readable buttons, sidebar labels and selected text.")
  assert.equal(ratio.dark, dark)
  assert.equal(ratio.mode, dark ? "dark" : "light", "Theme saves must preserve the user's preferred mode.")
}

const scenarios = {
  public_branding: async () => withFixture({ authenticated: false }, async ({ page, state }) => {
    await expectLogin(page, state.settings)
    await page.locator(`img[src="${state.settings.logoUrl}"]`).first().waitFor()
    await page.waitForFunction((src) => [...document.images].some((image) => image.getAttribute("src") === src && image.complete && image.naturalWidth > 0), state.settings.logoUrl)
    assert(state.requests.some((request) => request.pathname === "/api/platform/branding"), "Login branding must be loaded from the public endpoint.")
    assert.equal(state.requests.some((request) => request.pathname === "/api/platform/settings"), false, "Login must not read private platform settings.")
  }),
  authorization_branding: async () => withFixture({ startPath: "/device" }, async ({ page, state }) => {
    for (const [route, title] of [["/device", "设备授权"], ["/consent", "授权确认"]]) {
      if (new URL(page.url()).pathname !== route) await page.goto(origin + route)
      await page.getByText(state.settings.platformName, { exact: true }).waitFor()
      await page.getByText(state.settings.platformDescription, { exact: true }).waitFor()
      await page.getByText(state.settings.footerText, { exact: true }).waitFor()
      await page.locator(`img[src="${state.settings.logoUrl}"]`).first().waitFor()
      await expectTitle(page, `${title} · ${state.settings.platformName}`)
    }
    assert.equal(state.requests.some((request) => request.pathname === "/api/platform/settings"), false, "Device and consent branding must use the public branding contract.")
  }),
  empty_footers: async () => withFixture({ startPath: "/login", settings: { ...originalSettings, footerText: "" } }, async ({ page, state }) => {
    await expectLogin(page, state.settings)
    for (const [route, title] of [["/login", "登录"], ["/device", "设备授权"], ["/consent", "授权确认"]]) {
      if (new URL(page.url()).pathname !== route) await page.goto(origin + route)
      await page.getByText(state.settings.platformName, { exact: true }).first().waitFor()
      await expectTitle(page, `${title} · ${state.settings.platformName}`)
      assert.equal(await page.getByRole("contentinfo").count(), 0, `An empty configured footer must hide the footer on ${route}.`)
    }
  }),
  save_and_refresh: async () => withFixture({}, async ({ page, state }) => {
    await readySettings(page)
    const code = page.getByLabel("平台租户编码", { exact: true })
    assert.equal(await code.inputValue(), "admin-tenant")
    assert.equal(await code.getAttribute("readonly"), "", "The platform tenant code must be read only.")
    await expectTitle(page, `平台设置 · ${originalSettings.platformName}`)
    const next = {
      platformName: "远航管理平台", platformDescription: "跨组织统一管理入口", logoUrl: "/__branding/updated-logo.svg",
      loginTitle: "欢迎进入远航", loginDescription: "请选择您的工作组织", footerText: "© 2026 远航工作空间",
      platformTenantName: "远航平台运营租户",
      primaryColor: "#2563EB",
    }
    for (const [field, value] of Object.entries(next)) await page.getByLabel(labels[field], { exact: true }).fill(` ${value} `)
    await page.getByRole("button", { name: "保存设置", exact: true }).click()
    await page.getByText("平台设置已保存", { exact: true }).waitFor()
    assert.deepEqual(state.saves, [next], "Save must serialize exactly the configurable fields, including the trimmed theme color.")
    await expectTitle(page, `平台设置 · ${next.platformName}`)
    await page.waitForFunction((tenantName) => document.querySelector('button[aria-label="切换组织上下文"]')?.textContent.includes(tenantName), next.platformTenantName)
    assert.equal(await code.inputValue(), "admin-tenant")
    assert.equal(await page.getByLabel("平台名称", { exact: true }).inputValue(), next.platformName)
    assert.equal(await page.getByRole("button", { name: "保存设置", exact: true }).isDisabled(), true)
    // Reload a public page to prove all saved branding is read through its public contract.
    await page.goto(origin + "/login")
    await expectLogin(page, state.settings)
    await page.locator(`img[src="${next.logoUrl}"]`).first().waitFor()
  }),
  invalid_inputs: async () => withFixture({}, async ({ page, state }) => {
    await readySettings(page)
    const submit = page.getByRole("button", { name: "保存设置", exact: true })
    await page.getByLabel("平台名称", { exact: true }).fill("   ")
    await submit.click()
    await page.getByText("请输入平台名称", { exact: true }).waitFor()
    assert.equal(state.saves.length, 0, "Blank branding names must be rejected before any update request.")
    await page.getByLabel("平台名称", { exact: true }).fill(originalSettings.platformName)
    await page.getByLabel("平台租户名称", { exact: true }).fill("   ")
    await submit.click()
    await page.getByText("请输入平台租户名称", { exact: true }).waitFor()
    assert.equal(state.saves.length, 0, "Blank tenant names must be rejected before any update request.")
    await page.getByLabel("平台租户名称", { exact: true }).fill(originalSettings.platformTenantName)
    await page.getByLabel("登录页标题", { exact: true }).fill("   ")
    await submit.click()
    await page.getByText("请输入登录页标题", { exact: true }).waitFor()
    assert.equal(state.saves.length, 0)
    await page.getByLabel("登录页标题", { exact: true }).fill(originalSettings.loginTitle)
    for (const invalid of ["javascript:alert(1)", "//untrusted.example/logo.svg", "https://user:secret@example.com/logo.svg"]) {
      await page.getByLabel("Logo 地址", { exact: true }).fill(invalid)
      await submit.click()
      assert.equal(await page.getByLabel("Logo 地址", { exact: true }).getAttribute("aria-invalid"), "true")
      assert.equal(state.saves.length, 0, `Invalid logo URL must be rejected: ${invalid}`)
    }
    await page.getByRole("button", { name: "取消修改", exact: true }).click()
    assert.equal(await page.getByLabel("Logo 地址", { exact: true }).inputValue(), originalSettings.logoUrl)
    assert.equal(await submit.isDisabled(), true)
  }),
  theme_presets_preview_save: async () => withFixture({}, async ({ page, state }) => {
    await readySettings(page)
    const originalPrimary = await page.evaluate(() => document.documentElement.style.getPropertyValue("--primary"))
    const preview = page.getByLabel("导航主题预览", { exact: true })
    for (const [name, color] of [["海青", "#008577"], ["远山蓝", "#2563EB"], ["鸢尾紫", "#7C3AED"], ["琥珀", "#B45309"], ["玫瑰", "#BE185D"], ["松绿", "#15803D"]]) {
      const preset = page.getByRole("button", { name: `${name}主题`, exact: true })
      await preset.click()
      assert.equal(await preset.getAttribute("aria-pressed"), "true", "The chosen theme must be visibly selected.")
      assert.equal(await page.getByLabel("主题主色", { exact: true }).inputValue(), color)
      const previewColor = await preview.evaluate((element) => getComputedStyle(element).getPropertyValue("--primary").trim())
      assert(/^#[0-9a-f]{6}$/i.test(previewColor), "The theme preview must use a safe computed color.")
      assert.equal(await page.evaluate(() => document.documentElement.style.getPropertyValue("--primary")), originalPrimary, "Unsaved themes must remain local to the preview.")
    }
    await page.getByRole("button", { name: "取消修改", exact: true }).click()
    assert.equal(await page.getByLabel("主题主色", { exact: true }).inputValue(), originalSettings.primaryColor)
    await page.getByRole("button", { name: "远山蓝主题", exact: true }).click()
    if (process.env.PLATFORM_THEME_SCREENSHOT) {
      await page.evaluate(() => document.querySelector("main")?.scrollTo(0, 0))
      await page.screenshot({ path: process.env.PLATFORM_THEME_SCREENSHOT })
    }
    await page.getByRole("button", { name: "保存设置", exact: true }).click()
    await page.getByText("平台设置已保存", { exact: true }).waitFor()
    assert.equal(state.saves[0].primaryColor, "#2563EB")
    await page.waitForFunction((original) => document.documentElement.style.getPropertyValue("--primary") !== original, originalPrimary)
    const savedPrimary = await page.evaluate(() => document.documentElement.style.getPropertyValue("--primary"))
    await page.goto(origin + "/login")
    await expectLogin(page, state.settings)
    await page.waitForFunction((primary) => document.documentElement.style.getPropertyValue("--primary") === primary, savedPrimary)
    assert.equal(await page.evaluate(() => localStorage.getItem("platform-theme")), "light", "A shared theme must preserve the user's mode.")
  }),
  theme_custom_contrast_dark: async () => withFixture({ theme: "dark" }, async ({ page, state }) => {
    await readySettings(page)
    const input = page.getByLabel("主题主色", { exact: true })
    const submit = page.getByRole("button", { name: "保存设置", exact: true })
    for (const invalid of ["red", "#fff", "#GGGGGG"]) {
      await input.fill(invalid)
      await submit.click()
      await page.getByText("请输入 #RRGGBB 格式的颜色，如 #008577", { exact: true }).waitFor()
      assert.equal(state.saves.length, 0, "Invalid colors must not reach the update API.")
    }
    for (const color of ["#ffffff", "#000000", "#b4530a"]) {
      await saveThemeColor(page, state, color)
      await expectThemeContrast(page, true)
    }
  }),
  theme_custom_contrast_light: async () => withFixture({}, async ({ page, state }) => {
    await readySettings(page)
    for (const color of ["#FFFFFF", "#000000", "#FFFF00"]) {
      await saveThemeColor(page, state, color)
      await expectThemeContrast(page, false)
    }
  }),
  failed_save: async () => withFixture({ failSave: true }, async ({ page, state }) => {
    await readySettings(page)
    await page.getByLabel("平台名称", { exact: true }).fill("等待重试的平台")
    await page.getByRole("button", { name: "保存设置", exact: true }).click()
    await page.getByRole("alert").filter({ hasText: "配置暂时无法保存，请稍后重试" }).waitFor()
    assert.equal(state.saves.length, 1)
    assert.equal(await page.getByLabel("平台名称", { exact: true }).inputValue(), "等待重试的平台", "Failed updates must retain the editable draft.")
    await expectTitle(page, `平台设置 · ${originalSettings.platformName}`)
    state.failSave = false
    await page.getByRole("button", { name: "保存设置", exact: true }).click()
    await expectTitle(page, "平台设置 · 等待重试的平台")
    assert.equal(state.saves.length, 2)
  }),
  tenant_denied: async () => withFixture({ isPlatform: false }, async ({ page, state }) => {
    await page.getByText("无访问权限", { exact: true }).waitFor()
    assert.equal(await page.getByRole("button", { name: "保存设置", exact: true }).count(), 0)
    assert.equal(state.requests.some((request) => request.pathname === "/api/platform/settings"), false, "Customer tenant users must not request platform settings even when their fixture menu contains the route.")
  }),
  permission_denied: async () => withFixture({ hasPermission: false }, async ({ page, state }) => {
    await page.getByText("无访问权限", { exact: true }).waitFor()
    assert.equal(await page.getByRole("button", { name: "保存设置", exact: true }).count(), 0)
    assert.equal(state.requests.some((request) => request.pathname === "/api/platform/settings"), false, "Platform membership alone must not allow reading or modifying settings.")
  }),
}

let passed = 0
try {
  const selector = process.argv[2] || "all"
  assert(selector === "all" || Object.hasOwn(scenarios, selector), `Unknown scenario: ${selector}`)
  await startServer()
  browser = await launchBrowser()
  for (const [name, run] of Object.entries(scenarios)) {
    if (selector !== "all" && selector !== name) continue
    await run()
    console.log(`PASS platform settings browser: ${name}`)
    passed++
  }
  console.log(`Platform settings browser regression: ${passed}/${selector === "all" ? Object.keys(scenarios).length : 1} passed (isolated API fixtures; no live backend or PostgreSQL).`)
} finally {
  await browser?.close()
  if (vite && vite.exitCode === null) {
    const exited = new Promise((resolve) => vite.once("exit", resolve))
    vite.kill()
    await Promise.race([exited, pause(5000)])
    if (vite.exitCode === null) vite.kill("SIGKILL")
  }
}
