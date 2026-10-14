import { authStore } from "@/stores/auth"

export class ApiError extends Error {
  code: string
  status: number
  requestId?: string
  constructor(message: string, code: string, status: number, requestId?: string) {
    super(message)
    this.name = "ApiError"
    this.code = code
    this.status = status
    this.requestId = requestId
  }
}

export interface RequestOptions {
  method?: "GET" | "POST" | "PUT" | "DELETE"
  body?: unknown
  rawBody?: BodyInit
  contentType?: string
  /** 显式指定 Bearer(如登录流程的 credential ticket) */
  bearer?: string
  /** 完全不带认证头(登录本身) */
  skipAuth?: boolean
  /** 401 时跳过跳转(用于流程内自行处理) */
  suppress401?: boolean
  signal?: AbortSignal
}

function redirectToLogin() {
  const { pathname, search } = window.location
  if (pathname === "/login") return
  const returnTo = encodeURIComponent(pathname + search)
  window.location.assign(`/login?return=${returnTo}`)
}

type SessionSnapshot = { token: string; refreshToken: string; epoch: number }
let renewal: { refreshToken: string; epoch: number; promise: Promise<string> } | null = null

function contextChanged(): DOMException {
  return new DOMException("登录上下文已变化", "AbortError")
}

function snapshotCurrent(snapshot: SessionSnapshot): boolean {
  const state = authStore.getState()
  return state.contextEpoch === snapshot.epoch && state.token === snapshot.token && state.refreshToken === snapshot.refreshToken
}

/** Concurrent expired requests share one rotating refresh. A late result from an
 * old context must never overwrite the newly selected tenant's credentials. */
async function renewSession(snapshot: SessionSnapshot): Promise<string> {
  if (renewal?.refreshToken === snapshot.refreshToken && renewal.epoch === snapshot.epoch) return renewal.promise
  const entry = { refreshToken: snapshot.refreshToken, epoch: snapshot.epoch, promise: Promise.resolve("") }
  entry.promise = (async () => {
    let response: Response
    try {
      response = await fetch("/oauth/token", {
        method: "POST",
        headers: { Accept: "application/json", "Content-Type": "application/x-www-form-urlencoded" },
        body: new URLSearchParams({ grant_type: "refresh_token", refresh_token: snapshot.refreshToken }),
      })
    } catch {
      if (!snapshotCurrent(snapshot)) throw contextChanged()
      throw new ApiError("会话刷新失败，请检查网络后重试", "NETWORK_ERROR", 0)
    }
    if (!snapshotCurrent(snapshot)) throw contextChanged()
    let data: Record<string, unknown>
    try { data = await response.json() } catch { data = {} }
    if (!snapshotCurrent(snapshot)) throw contextChanged()
    if (!response.ok) {
      if (response.status === 400 || response.status === 401) {
        authStore.getState().logout()
        redirectToLogin()
      }
      throw new ApiError("会话刷新失败，请重新登录或稍后重试", "SESSION_REFRESH_FAILED", response.status)
    }
    if (typeof data.access_token !== "string" || !data.access_token ||
        typeof data.refresh_token !== "string" || !data.refresh_token) {
      throw new ApiError("会话刷新响应无效，请稍后重试", "INVALID_REFRESH_RESPONSE", 502)
    }
    if (!authStore.getState().rotateTokens(data.access_token, data.refresh_token, snapshot.token, snapshot.refreshToken, snapshot.epoch)) {
      throw contextChanged()
    }
    return data.access_token
  })()
  renewal = entry
  try { return await entry.promise } finally { if (renewal === entry) renewal = null }
}

export async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const { method = "GET", body, rawBody, contentType, bearer, skipAuth, suppress401, signal } = options
  const initial = authStore.getState()
  const epoch = initial.contextEpoch
  const storeToken = initial.token
  let token = bearer !== undefined ? bearer : skipAuth ? null : storeToken
  const sessionRequest = !!token && !skipAuth && token === storeToken
  const checkContext = () => {
    if (signal?.aborted || (sessionRequest && authStore.getState().contextEpoch !== epoch)) throw contextChanged()
  }
  const send = async (): Promise<Response> => {
    checkContext()
    const headers: Record<string, string> = { Accept: "application/json" }
    if (body !== undefined) headers["Content-Type"] = "application/json"
    if (rawBody !== undefined && contentType) headers["Content-Type"] = contentType
    if (token) headers.Authorization = `Bearer ${token}`
    try {
      return await fetch(path, { method, headers, body: rawBody ?? (body !== undefined ? JSON.stringify(body) : undefined), signal })
    } catch (error) {
      checkContext()
      if ((error as Error).name === "AbortError") throw error
      throw new ApiError("网络请求失败,请检查网络或服务状态", "NETWORK_ERROR", 0)
    }
  }
  let response = await send()
  checkContext()
  if (response.status === 401 && sessionRequest && initial.refreshToken) {
    const current = authStore.getState()
    // Another request may already have completed this same session's rotation.
    if (current.token && current.token !== storeToken) token = current.token
    else token = await renewSession({ token: storeToken!, refreshToken: initial.refreshToken, epoch })
    checkContext()
    response = await send()
    checkContext()
  }
  const text = await response.text()
  checkContext()
  let data: unknown = undefined
  if (text) {
    try { data = JSON.parse(text) } catch { data = text }
  }
  if (!response.ok) {
    let message = `请求失败 (HTTP ${response.status})`
    let code = "SERVER_ERROR"
    if (data && typeof data === "object" && "message" in data) {
      const value = data as Record<string, unknown>
      if (typeof value.message === "string") message = value.message
      if (typeof value.code === "string") code = value.code
    }
    if (response.status === 401 && sessionRequest && !suppress401 &&
        authStore.getState().token === token && !authStore.getState().contextSwitching) {
      authStore.getState().logout()
      redirectToLogin()
    }
    const candidate = response.headers.get("x-request-id")
    const requestId = candidate && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(candidate) ? candidate : undefined
    throw new ApiError(requestId ? `${message}（请求编号：${requestId}）` : message, code, response.status, requestId)
  }
  return data as T
}

export const api = {
  get: <T>(path: string, options?: RequestOptions) => request<T>(path, { ...options, method: "GET" }),
  post: <T>(path: string, body?: unknown, options?: RequestOptions) =>
    request<T>(path, { ...options, method: "POST", body }),
  postRaw: <T>(path: string, rawBody: BodyInit, contentType: string, options?: RequestOptions) =>
    request<T>(path, { ...options, method: "POST", rawBody, contentType }),
  put: <T>(path: string, body?: unknown, options?: RequestOptions) =>
    request<T>(path, { ...options, method: "PUT", body }),
  del: <T>(path: string, body?: unknown, options?: RequestOptions) =>
    request<T>(path, { ...options, method: "DELETE", body }),
}

/** 兼容响应可能是数组或 {records} 分页结构 */
export function asPage<T>(data: unknown, page = 1, size = 10): { records: T[]; total: number; page: number; size: number } {
  if (Array.isArray(data)) {
    return { records: data as T[], total: data.length, page: 1, size: data.length || size }
  }
  if (data && typeof data === "object" && "records" in data) {
    const d = data as { records: T[]; total?: number; page?: number; size?: number }
    return {
      records: Array.isArray(d.records) ? d.records : [],
      total: typeof d.total === "number" ? d.total : (d.records ?? []).length,
      page: typeof d.page === "number" ? d.page : page,
      size: typeof d.size === "number" ? d.size : size,
    }
  }
  return { records: [], total: 0, page, size }
}

/** 兼容响应可能是 {menus:[]} 或直接数组 */
export function asArray<T>(data: unknown, key?: string): T[] {
  if (Array.isArray(data)) return data as T[]
  if (data && typeof data === "object" && key && key in data) {
    const inner = (data as Record<string, unknown>)[key]
    if (Array.isArray(inner)) return inner as T[]
  }
  return []
}

export function qs(params: Record<string, string | number | undefined | null>): string {
  const entries = Object.entries(params).filter(
    ([, v]) => v !== undefined && v !== null && v !== "" && v !== "ALL"
  )
  if (entries.length === 0) return ""
  return "?" + entries.map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(String(v))}`).join("&")
}
