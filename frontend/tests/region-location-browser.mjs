/**
 * Managed-region and map integration through real Vite/React tenant/org pages.
 * The intercepted entry mounts the production pages with a fixture session;
 * catalog, persistence and map-provider HTTP responses are isolated fixtures.
 * Run with Node 24. Browsers are installed and launched by tests/browser.
 */
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import net from "node:net"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { launchBrowser } from "./browser/runtime.mjs"
import { fixtureUuid, identityForProfile, installSession, profileWithIdentity } from "./browser/identity-fixture.mjs"

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const bindHost = process.env.REGION_BROWSER_BIND || (process.platform === "win32" ? "::1" : "127.0.0.1")
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const deferred = () => { let resolve; const promise = new Promise((done) => { resolve = done }); return { promise, resolve } }
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

const version = "managed-fixture-2023"
const record = (code, parentCode, name, level) => ({ id: `region-${code}`, code, parentCode, name, level,
  version, namePinyin: "", nameShort: "", type: null, typeName: null, postCode: null, sourceUrl: null })
const catalog = [record("330000", "0", "浙江省", 2), record("440000", "0", "广东省", 2),
  record("330100", "330000", "杭州市", 3), record("330200", "330000", "宁波市", 3),
  record("440300", "440000", "深圳市", 3), record("330106", "330100", "西湖区", 4),
  record("330203", "330200", "海曙区", 4), record("440305", "440300", "南山区", 4),
  record("330106001", "330106", "西湖街道", 5), record("330106001001", "330106001", "杭州社区", 6),
  record("330203001", "330203", "海曙乡", 5), record("330203001001", "330203001", "宁波村", 6),
  record("440305001", "440305", "粤海街道", 5), record("440305001001", "440305001", "深圳社区", 6),
  record("440304", "440300", "福田区", 4), record("440304001", "440304", "福田街道", 5),
  record("440304001001", "440304001", "福田社区", 6), record("440305002", "440305", "南山街道", 5),
  record("440305002001", "440305002", "南山社区", 6), record("441900", "440000", "东莞市", 3),
  record("441900101", "441900", "石碣镇", 5), record("441900101204", "441900101", "唐洪村", 6)]
const regions = (...codes) => codes.map((code) => catalog.find((item) => item.code === code))
const resolution = (codes = []) => {
  const [province = null, city = null, district = null, township = null, village = null] = regions(...codes)
  return { version, province, city, district, township, village }
}
const existingLocation = { province: "浙江省", city: "杭州市", district: "西湖区", address: "旧地址 1 号",
  township: "西湖街道", village: "杭州社区", longitude: 120.123456, latitude: 30.123456 }
const tenant = { id: fixtureUuid("region-tenant"), name: "地区验收租户", code: "region-tenant", status: "ENABLED", isPlatform: false,
  description: "", expiresAt: null, organizationCount: 1, userCount: 1, authorizedAppCount: 0,
  createdAt: "2026-10-01T00:00:00Z", ...existingLocation }
const organization = { id: fixtureUuid("region-org"), name: "地区验收组织", code: "region-org", status: "ENABLED", parentId: null,
  tenantId: tenant.id, sortOrder: 0, description: "", expiresAt: null, children: [], ...existingLocation }
const fixtureProfile = profileWithIdentity({ user: { id: fixtureUuid("region-user"), username: "region-fixture", realName: "地区验收管理员" },
  tenant: { ...tenant, isPlatform: true }, organization: { ...organization, path: organization.name },
  platformContext: true, applications: [], memberships: [] })
const canonicalRaw = { address: { state: "广东", city: "深圳", city_district: "南山区", town: "粤海街道", village: "深圳社区", road: "科技路", house_number: "8号" } }
const searchItem = { display_name: "浙江旧搜索结果", lat: "30.2", lon: "120.2",
  address: { state: "浙江", city: "杭州", city_district: "西湖区", town: "西湖街道", village: "杭州社区", road: "旧搜索路" } }
const canonicalNames = { province: "广东省", city: "深圳市", district: "南山区", township: "粤海街道", village: "深圳社区" }
const regionFields = ["province", "city", "district", "township", "village"]
const regionLabels = ["省份", "城市", "区县", "乡镇/街道", "村/社区"]
const regionValues = (value) => regionFields.map((field) => value[field] || "")
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

async function fixture(kind, options = {}) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1100 } })
  const page = await context.newPage()
  page.setDefaultTimeout(12_000)
  const state = { requests: [], mapRequests: [], saves: [], errors: [], unexpected: [], reverse: canonicalRaw,
    reverseStatus: 200, resolved: resolution(["440000", "440300", "440305", "440305001", "440305001001"]), resolveStatus: 200,
    holdSearch: false, heldSearch: [], legacy: false, ...options }
  page.on("pageerror", (error) => state.errors.push(error.message))
  await installSession(context, "test-region-session", identityForProfile(fixtureProfile))
  await context.addInitScript(() => { localStorage.setItem("platform-theme", "light") })
  // The fixture entry is intentionally fixed while developers may edit files.
  // A Vite HMR reload would restore its original App dependency graph.
  await context.routeWebSocket(() => true, () => {})
  await context.route((url) => url.origin === origin && url.pathname === "/src/main.tsx", async (route) => {
    const response = await route.fetch()
    let body = await response.text()
    assert.match(body, /import App from ["']\/src\/App\.tsx(?:\?[^"']*)?["']/, "Mount the actual production pages from the real Vite entry.")
    body = body.replace(/import App from ["']\/src\/App\.tsx(?:\?[^"']*)?["']/, `import App from "/src/pages/system/${kind === "tenant" ? "tenants/TenantsPage" : "organizations/OrganizationsPage"}.tsx"`)
    body = body.replace("ReactDOM.createRoot", `import { authStore } from "/src/stores/auth.ts";\nawait authStore.getState().initializeSessionCoordination();\nauthStore.getState().setProfile(${JSON.stringify(fixtureProfile)});\nReactDOM.createRoot`)
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
    if (url.pathname.startsWith("/api/maps/tiles/")) return route.fulfill({ contentType: "image/png",
      body: Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aDTsAAAAASUVORK5CYII=", "base64") })
    if (url.pathname === "/api/maps/search") {
      if (state.holdSearch) {
        const release = deferred(); state.heldSearch.push(release); await release.promise
      }
      return fulfill(route, [searchItem])
    }
    if (url.pathname === "/api/maps/reverse") return fulfill(route, state.reverse, state.reverseStatus)
    if (request.headers().authorization !== "Bearer test-region-session") {
      state.unexpected.push({ ...item, reason: "missing fixture session" })
      return fulfill(route, { message: `Unexpected API ${url.pathname}` }, 503)
    }
    if (url.pathname === "/api/regions/options") {
      const parentCode = url.searchParams.get("parentCode") || "0"
      const requestedVersion = url.searchParams.get("version")
      assert(!requestedVersion || requestedVersion === version, "A cascade uses one managed catalog version.")
      return fulfill(route, { version, records: catalog.filter((item) => item.parentCode === parentCode) })
    }
    if (url.pathname === "/api/regions/versions") return fulfill(route, [{ version, label: "受管地区验收版本", sourceVersion: null, count: catalog.length }])
    if (url.pathname === "/api/regions/resolve") {
      // Page hydration/cascade lookups use exact catalog names; map raw aliases
      // use the scenario's resolver response, independently of this fixture path.
      const provinceName = url.searchParams.get("province")
      const matchedProvince = catalog.find((item) => item.level === 2 && item.name === provinceName)
      if (matchedProvince) {
        const city = catalog.find((item) => item.parentCode === matchedProvince.code && item.name === url.searchParams.get("city"))
        const district = city && catalog.find((item) => item.parentCode === city.code && item.name === url.searchParams.get("district"))
        const townshipParent = district || city
        const township = townshipParent && catalog.find((item) => item.level === 5 && item.parentCode === townshipParent.code && item.name === url.searchParams.get("township"))
        const village = township && catalog.find((item) => item.parentCode === township.code && item.name === url.searchParams.get("village"))
        return fulfill(route, { version, province: matchedProvince, city: city || null, district: district || null,
          township: township || null, village: village || null })
      }
      if (provinceName === "浙江" && url.searchParams.get("city") === "杭州") return fulfill(route, resolution(["330000", "330100", "330106", "330106001", "330106001001"]))
      state.mapRequests.push(item)
      return fulfill(route, state.resolved, state.resolveStatus)
    }
    const original = (value) => state.legacy ? { ...value, province: "浙江", city: "杭州" } : value
    if (["/api/tenants", "/api/tenants/options"].includes(url.pathname) && request.method() === "GET") return fulfill(route, { records: [original(tenant)], total: 1, page: 1, size: 100 })
    if (url.pathname === "/api/organizations/tree") return fulfill(route, [original(organization)])
    if (url.pathname === "/api/departments/tree") return fulfill(route, [])
    if ((url.pathname === `/api/tenants/${tenant.id}` || url.pathname === `/api/organizations/${organization.id}`) && request.method() === "PUT") {
      state.saves.push(item.body); return fulfill(route, {})
    }
    state.unexpected.push(item)
    return fulfill(route, { records: [], total: 0, page: 1, size: 10 })
  })
  try {
    await page.goto(`${origin}/system/${kind === "tenant" ? "tenants" : "organizations"}`, { waitUntil: "domcontentloaded" })
    const edit = kind === "tenant" ? page.getByRole("button", { name: `编辑 ${tenant.name}`, exact: true })
      : page.getByRole("treeitem").getByRole("button", { name: "编辑", exact: true }).first()
    await edit.click()
    await page.getByRole("dialog").getByRole("heading", { name: kind === "tenant" ? "编辑租户" : `编辑组织「${organization.name}」`, exact: true }).waitFor()
    return { context, page, state, form: page.locator(kind === "tenant" ? "#tenant-form" : "#org-form") }
  } catch (error) { console.error("Region fixture startup diagnostic:", { errors: state.errors, unexpected: state.unexpected }); await context.close(); throw error }
}

async function withFixture(kind, options, run) {
  const f = await fixture(kind, options)
  try { await run(f); assert.deepEqual(f.state.errors, [], "Production page should have no uncaught browser errors."); assert.deepEqual(f.state.unexpected, [], "All API fixtures are intentional.") }
  catch (error) { console.error("Region scenario diagnostic:", { kind, options, errors: f.state.errors, unexpected: f.state.unexpected, requests: f.state.requests,
    controls: await f.form.getByRole("combobox").allTextContents().catch(() => []), text: await f.form.innerText().catch(() => "") }); throw error }
  finally { for (const release of f.state.heldSearch) release.resolve(); await f.context.close() }
}
const regionControl = (form, name) => form.getByRole("combobox", { name, exact: true })
async function assertRegion(form, expected) {
  for (const [index, field] of regionFields.entries()) {
    const name = regionLabels[index], value = expected[field]
    if (value) await regionControl(form, name).getByText(value, { exact: true }).waitFor()
    else { const text = await regionControl(form, name).innerText(); assert(!catalog.some((item) => item.name === text), `${name} should be cleared`) }
  }
}
async function choose(page, form, label, value) {
  await regionControl(form, label).click()
  await page.getByRole("option", { name: value, exact: true }).click()
}
async function save({ page, state }) {
  await page.getByRole("dialog").getByRole("button", { name: "保存更改", exact: true }).click()
  await waitState(() => state.saves.length > 0, "The real form must submit a persistence payload.")
  return state.saves.at(-1)
}
async function openMap({ page, form }) {
  await form.getByRole("button", { name: "在地图上选点", exact: true }).click()
  const map = page.getByRole("dialog", { name: "在地图上选择位置", exact: true })
  await map.waitFor()
  return map
}
async function clickMap(map) {
  const canvas = map.getByLabel("可点击选点的地图", { exact: true })
  await canvas.click({ position: { x: 350, y: 150 } })
  await map.getByText(/地图手选/).waitFor()
}
async function confirmMap(map) {
  await map.getByRole("button", { name: "使用此位置", exact: true }).click()
  await map.waitFor({ state: "hidden" })
}

async function verifyCascades(kind) {
  await withFixture(kind, {}, async (f) => {
    await assertRegion(f.form, existingLocation)
    await choose(f.page, f.form, "村/社区", "清除选择")
    await assertRegion(f.form, { ...existingLocation, village: "" })
    await choose(f.page, f.form, "村/社区", "杭州社区")
    await choose(f.page, f.form, "乡镇/街道", "清除选择")
    await assertRegion(f.form, { province: "浙江省", city: "杭州市", district: "西湖区" })
    await choose(f.page, f.form, "乡镇/街道", "西湖街道")
    await choose(f.page, f.form, "村/社区", "杭州社区")
    await choose(f.page, f.form, "区县", "清除选择")
    await assertRegion(f.form, { province: "浙江省", city: "杭州市" })
    await choose(f.page, f.form, "区县", "西湖区")
    await choose(f.page, f.form, "乡镇/街道", "西湖街道")
    await choose(f.page, f.form, "村/社区", "杭州社区")
    await choose(f.page, f.form, "城市", "清除选择")
    await assertRegion(f.form, { province: "浙江省" })
    await choose(f.page, f.form, "城市", "杭州市")
    await choose(f.page, f.form, "区县", "西湖区")
    await choose(f.page, f.form, "乡镇/街道", "西湖街道")
    await choose(f.page, f.form, "村/社区", "杭州社区")
    await choose(f.page, f.form, "省份", "清除选择")
    await assertRegion(f.form, {})
    await choose(f.page, f.form, "省份", "浙江省")
    await choose(f.page, f.form, "城市", "杭州市")
    await choose(f.page, f.form, "区县", "西湖区")
    await choose(f.page, f.form, "乡镇/街道", "西湖街道")
    await choose(f.page, f.form, "村/社区", "杭州社区")
    await choose(f.page, f.form, "城市", "宁波市")
    await assertRegion(f.form, { province: "浙江省", city: "宁波市" })
    await choose(f.page, f.form, "区县", "海曙区")
    await choose(f.page, f.form, "乡镇/街道", "海曙乡")
    await choose(f.page, f.form, "村/社区", "宁波村")
    await choose(f.page, f.form, "省份", "广东省")
    await assertRegion(f.form, { province: "广东省" })
    assert.equal(await regionControl(f.form, "区县").isDisabled(), true, "District waits for a managed city.")
    assert.equal(await regionControl(f.form, "乡镇/街道").isDisabled(), true, "Township waits for a managed district.")
    assert.equal(await regionControl(f.form, "村/社区").isDisabled(), true, "Village waits for a managed township.")
    await choose(f.page, f.form, "城市", "深圳市")
    await choose(f.page, f.form, "区县", "南山区")
    await choose(f.page, f.form, "乡镇/街道", "粤海街道")
    await choose(f.page, f.form, "村/社区", "深圳社区")
    await choose(f.page, f.form, "区县", "福田区")
    await assertRegion(f.form, { province: "广东省", city: "深圳市", district: "福田区" })
    await choose(f.page, f.form, "乡镇/街道", "福田街道")
    await choose(f.page, f.form, "村/社区", "福田社区")
    await choose(f.page, f.form, "区县", "南山区")
    await choose(f.page, f.form, "乡镇/街道", "南山街道")
    await choose(f.page, f.form, "村/社区", "南山社区")
    await choose(f.page, f.form, "乡镇/街道", "粤海街道")
    await assertRegion(f.form, { ...canonicalNames, village: "" })
    await choose(f.page, f.form, "村/社区", "深圳社区")
    if (kind === "organization" && process.env.REGION_BROWSER_SCREENSHOT) {
      await f.page.getByRole("dialog").screenshot({ path: process.env.REGION_BROWSER_SCREENSHOT })
      console.log(`Five-level form screenshot: ${process.env.REGION_BROWSER_SCREENSHOT}`)
    }
    const payload = await save(f)
    assert.deepEqual(regionValues(payload), regionValues(canonicalNames))
    assert.equal(payload.address, existingLocation.address, "Managed cascade keeps separately entered address.")
    for (const parentCode of ["440300", "440305", "440305001"]) assert.ok(f.state.requests.some((item) => item.pathname === "/api/regions/options" && item.params.parentCode === parentCode))
  })
  console.log(`PASS ${kind}: five-level hydration, explicit clears, parent resets, catalog version and submitted canonical names`)
}

async function verifyMap(kind, mode) {
  const options = mode === "unmatched" ? { reverse: { address: { state: "不在目录的省", city: "不在目录的市", road: "新地址" } }, resolved: resolution() }
    : mode === "failure" ? { resolveStatus: 503, resolved: { message: "Fixture catalog temporarily unavailable", code: "SERVICE_UNAVAILABLE" } } : {}
  await withFixture(kind, options, async (f) => {
    await assertRegion(f.form, existingLocation)
    const map = await openMap(f)
    await clickMap(map)
    await confirmMap(map)
    const expected = mode === "unmatched" ? {} : mode === "failure" ? existingLocation : canonicalNames
    await assertRegion(f.form, expected)
    const payload = await save(f)
    assert.deepEqual(regionValues(payload), regionValues(expected))
    assert.notEqual(payload.longitude, existingLocation.longitude, "Map click updates coordinates.")
    assert.equal(payload.address, mode === "unmatched" ? "新地址" : "科技路 8号")
    const match = f.state.mapRequests.at(-1)
    assert(match, "Map administrative names pass through the managed resolver.")
    assert.equal(match.params.version, version, "Map resolver uses the cascade's loaded catalog version.")
    if (mode === "canonical") assert.deepEqual(regionValues(match.params), ["广东", "深圳", "南山区", "粤海街道", "深圳社区"])
  })
  console.log(`PASS ${kind} map ${mode}: coordinates/address submitted with ${mode === "failure" ? "previous managed regions retained" : "managed resolver result"}`)
}

async function verifyAliases(kind) {
  await withFixture(kind, { legacy: true }, async (f) => {
    await assertRegion(f.form, existingLocation)
    const payload = await save(f)
    assert.deepEqual(regionValues(payload), regionValues(existingLocation), "Existing aliases hydrate into five current managed names.")
    assert.equal(payload.address, existingLocation.address)
    assert.equal(payload.longitude, existingLocation.longitude)
    assert.equal(payload.latitude, existingLocation.latitude)
  })
  console.log(`PASS ${kind}: legacy region aliases hydrate without changing address or coordinates`)
}

async function verifyDirectCity() {
  for (const kind of ["tenant", "organization"]) await withFixture(kind, {}, async (f) => {
    await assertRegion(f.form, existingLocation)
    await choose(f.page, f.form, "省份", "广东省")
    await choose(f.page, f.form, "城市", "东莞市")
    await f.form.getByText("无区县层级", { exact: true }).waitFor()
    assert.equal(await regionControl(f.form, "区县").isDisabled(), true, "Directly administered cities skip the absent county level.")
    await choose(f.page, f.form, "乡镇/街道", "石碣镇")
    await choose(f.page, f.form, "村/社区", "唐洪村")
    const expected = { province: "广东省", city: "东莞市", district: "", township: "石碣镇", village: "唐洪村" }
    await assertRegion(f.form, expected)
    const payload = await save(f)
    assert.deepEqual(regionValues(payload), regionValues(expected), "City-to-township paths retain an empty county.")
    assert.ok(f.state.requests.some((item) => item.pathname === "/api/regions/options" && item.params.parentCode === "441900"))
  })
  console.log("PASS tenant/organization direct-city: 东莞市 → 石碣镇 → 唐洪村 without a county level")
}

async function verifyStaleSearch() {
  await withFixture("tenant", {}, async (f) => {
    await assertRegion(f.form, existingLocation)
    const map = await openMap(f)
    await map.getByLabel("搜索地图地点", { exact: true }).fill("旧搜索地点")
    await map.getByRole("button", { name: "搜索", exact: true }).click()
    await map.getByRole("button", { name: searchItem.display_name, exact: true }).click()
    // Moving the marker after choosing a search result must discard its region/address fallback.
    f.state.reverse = {}
    f.state.resolved = resolution()
    await clickMap(map)
    await confirmMap(map)
    const match = f.state.mapRequests.at(-1)
    assert.deepEqual(regionValues(match.params), ["", "", "", "", ""], "Old search administrative names cannot be reused after a map click.")
    const payload = await save(f)
    assert.deepEqual(regionValues(payload), ["", "", "", "", ""])
    assert.equal(payload.address, existingLocation.address, "An empty reverse result does not fill the old search address.")
  })
  await withFixture("organization", { holdSearch: true }, async (f) => {
    await assertRegion(f.form, existingLocation)
    const map = await openMap(f)
    await map.getByLabel("搜索地图地点", { exact: true }).fill("延迟的旧搜索")
    await map.getByRole("button", { name: "搜索", exact: true }).click()
    await waitState(() => f.state.heldSearch.length === 1, "The search fixture is held in flight.")
    await clickMap(map)
    f.state.heldSearch[0].resolve()
    await pause(100)
    assert.equal(await map.getByRole("listbox", { name: "地点搜索结果", exact: true }).count(), 0, "A search completed after a map click cannot revive old results.")
    await confirmMap(map)
    const payload = await save(f)
    assert.deepEqual(regionValues(payload), regionValues(canonicalNames))
  })
  console.log("PASS stale searches: old selected fallback and in-flight search cannot override a later map click")
}

try {
  await startServer()
  browser = await launchBrowser()
  const selected = process.argv[2]
  assert(!selected || ["cascade", "map", "stale", "aliases", "direct"].includes(selected), "Unknown browser scenario group.")
  for (const kind of ["tenant", "organization"]) {
    if (!selected || selected === "cascade") await verifyCascades(kind)
    if (!selected || selected === "aliases") await verifyAliases(kind)
    if (!selected || selected === "map") for (const mode of ["canonical", "unmatched", "failure"]) await verifyMap(kind, mode)
  }
  if (!selected || selected === "direct") await verifyDirectCity()
  if (!selected || selected === "stale") await verifyStaleSearch()
  console.log(`Managed region browser integration passed (${selected || "all 14 scenarios"}); HTTP fixtures do not assert database/backend behavior.`)
} finally {
  await browser?.close()
  if (vite && vite.exitCode === null) {
    vite.kill()
    await Promise.race([new Promise((resolve) => vite.once("exit", resolve)), pause(5000)])
    if (vite.exitCode === null) vite.kill("SIGKILL")
  }
}
