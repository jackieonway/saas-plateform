/**
 * oauth-playground — SaaS Platform OAuth 测试服务 (端口硬编码 3004)
 *
 * 定位: 授权服务器 (:3001) 的纯消费方, 演练四条 OAuth 流程:
 *   1. 设备授权 RFC 8628 (client smart-tv, PUBLIC)
 *   2. 授权码 + PKCE S256 (client platform-web, CONFIDENTIAL, redirect_uri=http://localhost:3004/callback)
 *   3. 客户端凭据 client_credentials (client api-service, CONFIDENTIAL, 服务间调用场景)
 *   4. 资源密码 password (client resource-portal, CONFIDENTIAL, 高信任第一方应用场景)
 *
 * 关键实现点:
 * - 纯 node:http 零依赖 (Task 18-d 去 bun 化), node --watch 热重载 (node ≥ 18.11);
 *   TS 语法受 node 原生 type stripping 支持 (无 enum/namespace/decorator)。
 * - 页面侧浏览器直连 :3001 (授权服务器 CorsLayer::permissive, 简单请求/预检均放行);
 *   设备授权发起与 PKCE 生成按需求走本服务服务端代理。
 * - 授权码流程的 authorize URL 必须经前端 :3000 的 /oauth 代理发起: 授权服务器
 *   PUBLIC_APP_URL 为空 → 302 Location 是相对路径 /consent?request_id=…, 只有在
 *   3000 origin 下解析才能落到前端 consent 页 (直连 :3001 会 404, 后端无静态托管)。
 * - code_verifier 存储: 服务端内存 Map (state → verifier, 30 分钟过期, 上限 200 条) +
 *   页面 localStorage 双保险, 回调页优先 localStorage, 未命中走 GET /api/verifier 兜底。
 */
import { createHash, randomBytes } from "node:crypto"
import { createServer, type IncomingMessage, type ServerResponse } from "node:http"
import { authCodeFlowPage, callbackPage, clientCredentialsPage, deviceFlowPage, externalApiPage, homePage, passwordFlowPage, renderBrandedPage } from "./pages.ts"

/* ===== 常量 (端口硬编码, 不读环境变量) ===== */
const PORT = 3004
const BACKEND = "http://localhost:3001" // Rust Axum OAuth 授权服务器
const FRONTEND = "http://localhost:3000" // Vite 前端 (consent / device 批准页, /oauth /api 代理)
const CLIENT_ID = "platform-web"
const CLIENT_SECRET = "platform-web-secret"
const REDIRECT_URI = "http://localhost:3004/callback"
const DEVICE_CLIENT_ID = "smart-tv"
const API_SERVICE_CLIENT_ID = "api-service"
const API_SERVICE_CLIENT_SECRET = "api-service-secret"
const PORTAL_CLIENT_ID = "resource-portal"
const PORTAL_CLIENT_SECRET = "resource-portal-secret"
const SCOPE = "profile"

/* ===== PKCE state → verifier (内存 Map) ===== */
const pkceStore = new Map<string, { verifier: string; createdAt: number }>()

/** BASE64URL(SHA256(x)) — 与后端 util::hash::b64url_sha256 完全同口径 (无填充) */
function b64urlSha256(input: string): string {
  return createHash("sha256").update(input).digest("base64url")
}

/** 随机字母数字串 (PKCE verifier 合法字符集, 长度 ≥43) */
function randomToken(len: number): string {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
  const bytes = randomBytes(len)
  let out = ""
  for (let i = 0; i < len; i++) out += alphabet[bytes[i] % alphabet.length]
  return out
}

/* ===== node:http 响应辅助 ===== */

function sendJson(res: ServerResponse, status: number, body: unknown): void {
  const text = JSON.stringify(body)
  res.writeHead(status, {
    "Content-Type": "application/json; charset=utf-8",
    "Content-Length": Buffer.byteLength(text),
    "Cache-Control": "no-store",
  })
  res.end(text)
}

/** 品牌读取仅在页面请求时执行；后台不可用时按时返回默认页面。 */
async function loadPlatformName(): Promise<string> {
  try {
    const upstream = await fetch(`${BACKEND}/api/platform/branding`, {
      signal: AbortSignal.timeout(1200),
      headers: { Accept: "application/json" },
    })
    if (upstream.ok) {
      const branding: unknown = await upstream.json()
      if (branding && typeof branding === "object" && "platformName" in branding &&
          typeof branding.platformName === "string" && branding.platformName.trim()) {
        return branding.platformName.trim()
      }
    }
  } catch { /* 首次启动或后台不可用时使用中性文案，不阻塞服务启动。 */ }
  return "管理平台"
}

async function sendHtml(res: ServerResponse, template: string): Promise<void> {
  const page = renderBrandedPage(template, await loadPlatformName())
  res.writeHead(200, {
    "Content-Type": "text/html; charset=utf-8",
    "Content-Length": Buffer.byteLength(page),
    "Cache-Control": "no-store",
  })
  res.end(page)
}

/* ===== 服务端代理端点 ===== */

/** POST /api/device-start — 模拟设备端: 服务端向授权服务器发起设备授权 (smart-tv, PUBLIC 无 secret) */
async function apiDeviceStart(res: ServerResponse): Promise<void> {
  const url = `${BACKEND}/oauth/device/authorization`
  try {
    const upstream = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({ client_id: DEVICE_CLIENT_ID, scope: SCOPE }).toString(),
    })
    const text = await upstream.text()
    console.log(`[device-start] POST ${url} → ${upstream.status} ${text.slice(0, 220)}`)
    res.writeHead(upstream.status, {
      "Content-Type": "application/json; charset=utf-8",
      "Content-Length": Buffer.byteLength(text),
      "Cache-Control": "no-store",
    })
    res.end(text)
  } catch (e) {
    console.warn(`[device-start] POST ${url} 失败: ${e}`)
    sendJson(res, 502, {
      error: "server_error",
      error_description: `无法连接授权服务器 ${BACKEND}: ${e}`,
    })
  }
}

/** POST /api/auth-start — 生成 PKCE verifier/S256 challenge + state, 返回 authorize URL */
function apiAuthStart(res: ServerResponse): void {
  const verifier = randomToken(56) // ≥43 字符, 满足 RFC 7636
  const state = randomToken(24)
  const challenge = b64urlSha256(verifier)
  const now = Date.now()
  // 清理过期 (30 分钟) 与超量 (200) 条目
  for (const [k, v] of pkceStore) {
    if (now - v.createdAt > 30 * 60_000) pkceStore.delete(k)
  }
  while (pkceStore.size >= 200) {
    const first = pkceStore.keys().next().value
    if (first === undefined) break
    pkceStore.delete(first)
  }
  pkceStore.set(state, { verifier, createdAt: now })
  // authorize 经前端 :3000 /oauth 代理发起 (理由见文件头注释)
  const authorizeUrl =
    `${FRONTEND}/oauth/authorize?client_id=${CLIENT_ID}` +
    `&redirect_uri=${encodeURIComponent(REDIRECT_URI)}` +
    `&response_type=code&scope=${SCOPE}&state=${state}` +
    `&code_challenge=${challenge}&code_challenge_method=S256`
  console.log(`[auth-start] state=${state} code_challenge=${challenge} → ${authorizeUrl}`)
  sendJson(res, 200, { authorizeUrl, state, verifier, codeChallenge: challenge, redirectUri: REDIRECT_URI })
}

/** GET /api/verifier?state= — 回调页 localStorage 未命中时的兜底查询 */
function apiVerifier(res: ServerResponse, url: URL): void {
  const state = url.searchParams.get("state") ?? ""
  const hit = state ? pkceStore.get(state) : undefined
  if (!hit) {
    sendJson(res, 404, {
      error: "not_found",
      error_description: "state 不存在或已过期 (服务重启后内存 Map 会丢失, localStorage 仍可用)",
    })
    return
  }
  sendJson(res, 200, { verifier: hit.verifier })
}

/* ===== 读取请求 JSON body ===== */

async function readJsonBody(req: IncomingMessage): Promise<Record<string, unknown>> {
  const chunks: Buffer[] = []
  for await (const chunk of req) chunks.push(chunk as Buffer)
  if (chunks.length === 0) return {}
  try {
    return JSON.parse(Buffer.concat(chunks).toString("utf8")) as Record<string, unknown>
  } catch {
    return {}
  }
}

function asString(v: unknown): string {
  return typeof v === "string" ? v : ""
}

/** 向授权服务器转发 form 请求并原样回传 (状态码/JSON 全透传, 页面负责渲染) */
async function proxyTokenRequest(
  res: ServerResponse,
  label: string,
  form: Record<string, string>,
  auth: { basic?: { id: string; secret: string } } = {},
): Promise<void> {
  const url = `${BACKEND}/oauth/token`
  const headers: Record<string, string> = { "Content-Type": "application/x-www-form-urlencoded" }
  if (auth.basic) {
    headers["Authorization"] = `Basic ${Buffer.from(`${auth.basic.id}:${auth.basic.secret}`).toString("base64")}`
  }
  try {
    const upstream = await fetch(url, {
      method: "POST",
      headers,
      body: new URLSearchParams(form).toString(),
    })
    const text = await upstream.text()
    console.log(`[${label}] POST ${url} → ${upstream.status} ${text.slice(0, 260)}`)
    res.writeHead(upstream.status, {
      "Content-Type": "application/json; charset=utf-8",
      "Content-Length": Buffer.byteLength(text),
      "Cache-Control": "no-store",
    })
    res.end(text)
  } catch (e) {
    console.warn(`[${label}] POST ${url} 失败: ${e}`)
    sendJson(res, 502, { error: "server_error", error_description: `无法连接授权服务器 ${BACKEND}: ${e}` })
  }
}

/** 调用授权服务器资源端点 (userinfo/introspect/revoke) 并原样回传 */
async function proxyResourceRequest(
  res: ServerResponse,
  label: string,
  path: string,
  init: { method: "GET" | "POST"; headers?: Record<string, string>; form?: Record<string, string> },
): Promise<void> {
  const url = `${BACKEND}${path}`
  try {
    const upstream = await fetch(url, {
      method: init.method,
      headers: init.headers ?? {},
      body: init.form ? new URLSearchParams(init.form).toString() : undefined,
    })
    const text = await upstream.text()
    console.log(`[${label}] ${init.method} ${url} → ${upstream.status} ${text.slice(0, 260)}`)
    res.writeHead(upstream.status, {
      "Content-Type": "application/json; charset=utf-8",
      "Content-Length": Buffer.byteLength(text),
      "Cache-Control": "no-store",
    })
    res.end(text)
  } catch (e) {
    console.warn(`[${label}] ${init.method} ${url} 失败: ${e}`)
    sendJson(res, 502, { error: "server_error", error_description: `无法连接授权服务器 ${BACKEND}: ${e}` })
  }
}

/* ===== Task 20: client_credentials / password 两模式服务端代理 ===== */

/** POST /api/cc-token — api-service 以 Basic 认证 (RFC 6749 §2.3.1) 发起 client_credentials */
async function apiCcToken(res: ServerResponse): Promise<void> {
  await proxyTokenRequest(
    res,
    "cc-token",
    { grant_type: "client_credentials", scope: SCOPE },
    { basic: { id: API_SERVICE_CLIENT_ID, secret: API_SERVICE_CLIENT_SECRET } },
  )
}

/** POST /api/password-token — resource-portal 以 form body 通道认证, 携用户凭据发起 password */
async function apiPasswordToken(res: ServerResponse, req: IncomingMessage): Promise<void> {
  const body = await readJsonBody(req)
  const username = asString(body.username).trim()
  const password = asString(body.password)
  if (!username || !password) {
    sendJson(res, 400, { error: "invalid_request", error_description: "缺少 username 或 password" })
    return
  }
  await proxyTokenRequest(res, "password-token", {
    grant_type: "password",
    username,
    password,
    scope: asString(body.scope) || "profile email",
    client_id: PORTAL_CLIENT_ID,
    client_secret: PORTAL_CLIENT_SECRET,
  })
}

/** POST /api/refresh-token — resource-portal 用 refresh_token 换新令牌 (轮换演示) */
async function apiRefreshToken(res: ServerResponse, req: IncomingMessage): Promise<void> {
  const body = await readJsonBody(req)
  const refreshToken = asString(body.refreshToken).trim()
  if (!refreshToken) {
    sendJson(res, 400, { error: "invalid_request", error_description: "缺少 refreshToken" })
    return
  }
  const form: Record<string, string> = {
    grant_type: "refresh_token",
    refresh_token: refreshToken,
    client_id: PORTAL_CLIENT_ID,
    client_secret: PORTAL_CLIENT_SECRET,
  }
  const scope = asString(body.scope).trim()
  if (scope) form["scope"] = scope
  await proxyTokenRequest(res, "refresh-token", form)
}

/** POST /api/userinfo — Bearer access token 调 GET /oauth/userinfo (页面代理, 统一避 CORS) */
async function apiUserinfo(res: ServerResponse, req: IncomingMessage): Promise<void> {
  const body = await readJsonBody(req)
  const token = asString(body.token).trim()
  if (!token) {
    sendJson(res, 400, { error: "invalid_request", error_description: "缺少 token" })
    return
  }
  await proxyResourceRequest(res, "userinfo", "/oauth/userinfo", {
    method: "GET",
    headers: { Authorization: `Bearer ${token}` },
  })
}

/** POST /api/introspect — api-service 以 Basic 认证调 RFC 7662 令牌内省 (服务间验证演示) */
async function apiIntrospect(res: ServerResponse, req: IncomingMessage): Promise<void> {
  const body = await readJsonBody(req)
  const token = asString(body.token).trim()
  if (!token) {
    sendJson(res, 400, { error: "invalid_request", error_description: "缺少 token" })
    return
  }
  await proxyResourceRequest(res, "introspect", "/oauth/introspect", {
    method: "POST",
    headers: {
      "Content-Type": "application/x-www-form-urlencoded",
      Authorization: `Basic ${Buffer.from(`${API_SERVICE_CLIENT_ID}:${API_SERVICE_CLIENT_SECRET}`).toString("base64")}`,
    },
    form: { token, token_type_hint: "access_token" },
  })
}

/** POST /api/revoke — api-service 以 Basic 认证调 RFC 7009 令牌吊销 */
async function apiRevoke(res: ServerResponse, req: IncomingMessage): Promise<void> {
  const body = await readJsonBody(req)
  const token = asString(body.token).trim()
  if (!token) {
    sendJson(res, 400, { error: "invalid_request", error_description: "缺少 token" })
    return
  }
  await proxyResourceRequest(res, "revoke", "/oauth/revoke", {
    method: "POST",
    headers: {
      "Content-Type": "application/x-www-form-urlencoded",
      Authorization: `Basic ${Buffer.from(`${API_SERVICE_CLIENT_ID}:${API_SERVICE_CLIENT_SECRET}`).toString("base64")}`,
    },
    form: { token, token_type_hint: "access_token" },
  })
}

/* ===== Task 21-e: 第三方应用菜单/角色对外 API 服务端代理 ===== */

/** POST /api/external-menus — Bearer 令牌透传调 GET /api/external/menus (拉取关联应用菜单树) */
async function apiExternalMenus(res: ServerResponse, req: IncomingMessage): Promise<void> {
  const body = await readJsonBody(req)
  const token = asString(body.token).trim()
  if (!token) {
    sendJson(res, 400, { error: "invalid_request", error_description: "缺少 token (先在步骤①获取访问令牌)" })
    return
  }
  await proxyResourceRequest(res, "external-menus", "/api/external/menus", {
    method: "GET",
    headers: { Authorization: `Bearer ${token}` },
  })
}

/** POST /api/external-roles — Bearer 令牌透传调 GET /api/external/roles (拉取授权组织与角色清单) */
async function apiExternalRoles(res: ServerResponse, req: IncomingMessage): Promise<void> {
  const body = await readJsonBody(req)
  const token = asString(body.token).trim()
  if (!token) {
    sendJson(res, 400, { error: "invalid_request", error_description: "缺少 token (先在步骤①获取访问令牌)" })
    return
  }
  await proxyResourceRequest(res, "external-roles", "/api/external/roles", {
    method: "GET",
    headers: { Authorization: `Bearer ${token}` },
  })
}

/* ===== node:http 服务 ===== */

const server = createServer((req: IncomingMessage, res: ServerResponse) => {
  const url = new URL(req.url ?? "/", `http://localhost:${PORT}`)
  const path = url.pathname
  const method = req.method ?? "GET"

  if (method === "GET" || method === "HEAD") {
    if (path === "/") return void sendHtml(res, homePage)
    if (path === "/device-flow") return void sendHtml(res, deviceFlowPage)
    if (path === "/auth-code-flow") return void sendHtml(res, authCodeFlowPage)
    if (path === "/callback") return void sendHtml(res, callbackPage)
    if (path === "/client-credentials") return void sendHtml(res, clientCredentialsPage)
    if (path === "/password-flow") return void sendHtml(res, passwordFlowPage)
    if (path === "/external-api") return void sendHtml(res, externalApiPage)
    if (path === "/health") return sendJson(res, 200, { status: "UP", service: "oauth-playground" })
    if (path === "/favicon.ico") {
      res.writeHead(204)
      return res.end()
    }
    if (path === "/api/verifier") return apiVerifier(res, url)
  }
  if (method === "POST") {
    if (path === "/api/device-start") return void apiDeviceStart(res)
    if (path === "/api/auth-start") return apiAuthStart(res)
    if (path === "/api/cc-token") return void apiCcToken(res)
    if (path === "/api/password-token") return void apiPasswordToken(res, req)
    if (path === "/api/refresh-token") return void apiRefreshToken(res, req)
    if (path === "/api/userinfo") return void apiUserinfo(res, req)
    if (path === "/api/introspect") return void apiIntrospect(res, req)
    if (path === "/api/revoke") return void apiRevoke(res, req)
    if (path === "/api/external-menus") return void apiExternalMenus(res, req)
    if (path === "/api/external-roles") return void apiExternalRoles(res, req)
  }
  console.log(`[404] ${method} ${path}`)
  sendJson(res, 404, { error: "not_found", error_description: `${method} ${path} 不存在` })
})

server.listen(PORT, "0.0.0.0", () => {
  console.log(`oauth-playground 已启动: http://localhost:${PORT}`)
  console.log(`  首页 / · 设备授权 /device-flow · 授权码 /auth-code-flow · 客户端凭据 /client-credentials · 资源密码 /password-flow · 第三方对接 /external-api · 回调 /callback`)
  console.log(`  授权服务器 ${BACKEND} · 前端 ${FRONTEND} · redirect_uri ${REDIRECT_URI}`)
})
