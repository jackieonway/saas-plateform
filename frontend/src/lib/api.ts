import { authStore, captureSessionFence, captureSessionSignal, reconcileSessionSync } from "@/stores/auth"
import type { SessionFence } from "@/stores/auth"
import { sessionCoordinator, SessionCoordinationError, SessionOperationError } from "@/lib/session-coordinator"
import type { SessionRecord, SessionTokens } from "@/lib/session-coordinator"
import type { ContextResp } from "@/api/types"

export class ApiError extends Error {
  constructor(message: string, public code: string, public status: number, public requestId?: string) {
    super(message)
    this.name = "ApiError"
  }
}
export interface RequestOptions {
  method?: "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
  body?: unknown
  rawBody?: BodyInit
  contentType?: string
  /** Credential/application bearers never trigger platform-session refresh. */
  bearer?: string
  /** Captured platform bearers must opt in to logical-session fencing. */
  sessionFence?: SessionFence
  skipAuth?: boolean
  suppress401?: boolean
  signal?: AbortSignal
  /** Successful response metadata is delivered only after the session fence. */
  onResponse?: (meta: { requestId?: string }) => void
}
function redirectToLogin() {
  const { pathname, search } = window.location
  if (pathname !== "/login") window.location.assign(`/login?return=${encodeURIComponent(pathname + search)}`)
}
function contextChanged(): DOMException { return new DOMException("登录上下文已变化", "AbortError") }

/** Runs only under the common lock. Context operations call this directly to
 * avoid recursively acquiring their own lock through request(). */
async function refreshPair(record: SessionRecord, signal: AbortSignal): Promise<SessionTokens> {
  signal.throwIfAborted()
  if (!record.refreshToken) throw new SessionOperationError("会话已失效，请重新登录", "invalid", 401)
  let response: Response
  try {
    response = await fetch("/oauth/token", { method: "POST", signal,
      headers: { Accept: "application/json", "Content-Type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({ grant_type: "refresh_token", refresh_token: record.refreshToken }),
    })
  } catch { throw new SessionOperationError("会话刷新结果无法确认，请重新登录", "unknown") }
  signal.throwIfAborted()
  let data: Record<string, unknown>
  try { data = await response.json() } catch { throw new SessionOperationError("会话刷新结果无法确认，请重新登录", "unknown", response.status) }
  signal.throwIfAborted()
  if (!response.ok) {
    // There is no retry-safe 5xx/transport acknowledgement contract. Definite
    // platform 400/401 rejections are the only responses that invalidate a pair.
    throw new SessionOperationError("会话刷新失败，请重新登录", response.status === 400 || response.status === 401 ? "invalid" : "unknown", response.status)
  }
  if (typeof data.access_token !== "string" || !data.access_token || typeof data.refresh_token !== "string" || !data.refresh_token) {
    throw new SessionOperationError("会话刷新响应无法确认，请重新登录", "unknown", 502)
  }
  return { token: data.access_token, refreshToken: data.refresh_token }
}
let renewal: { sessionId: string | null; version: number; promise: Promise<string> } | null = null
async function renewSession(fence: SessionFence): Promise<string> {
  if (renewal?.sessionId === fence.sessionId && renewal.version === fence.version) return renewal.promise
  const entry = { ...fence, promise: Promise.resolve("") }
  entry.promise = (async () => {
    try {
      const result = await sessionCoordinator.operation("refresh", fence, async ({ record, signal }) => {
        const tokens = await refreshPair(record, signal)
        return { value: tokens.token, tokens }
      })
      return typeof result === "string" ? result : (result as SessionRecord).token!
    } catch (error) {
      reconcileSessionSync()
      if (error instanceof SessionOperationError) {
        if (error.disposition === "invalid" && authStore.getState().sessionStatus === "anonymous") redirectToLogin()
        throw new ApiError(error.message, error.disposition === "unknown" ? "SESSION_UNKNOWN" : "SESSION_REFRESH_FAILED", error.status)
      }
      throw error
    }
  })()
  renewal = entry
  try { return await entry.promise } finally { if (renewal === entry) renewal = null }
}
export async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const { method = "GET", body, rawBody, contentType, bearer, sessionFence, skipAuth, suppress401, signal, onResponse } = options
  const platform = !skipAuth && (bearer === undefined || sessionFence !== undefined)
  // Capture the caller's view before any await. A queued A mutation must never
  // acquire B's identity merely because a cross-page context operation finishes.
  const invoked = authStore.getState(), epoch = invoked.contextEpoch
  const fence = sessionFence ?? { sessionId: invoked.sessionId, version: invoked.sessionVersion }
  const lifetime = captureSessionSignal()
  if (platform) {
    reconcileSessionSync()
    const observed = authStore.getState()
    if (observed.sessionStatus === "unknown" || observed.sessionStatus === "unavailable") {
      throw new SessionCoordinationError(observed.coordinationError ?? "请重新登录", observed.sessionStatus === "unknown" ? "SESSION_UNKNOWN" : "SESSION_UNAVAILABLE")
    }
    if (observed.contextSwitching || observed.sessionId !== fence.sessionId || observed.contextEpoch !== epoch || observed.sessionStatus === "initializing") throw contextChanged()
  }
  if (platform) await authStore.getState().reconcileSession()
  const initial = authStore.getState()
  const sessionRequest = platform && !!invoked.token
  const requestSignal = sessionRequest ? AbortSignal.any([lifetime, ...(signal ? [signal] : [])]) : signal
  if (sessionFence && initial.sessionId !== sessionFence.sessionId) throw contextChanged()
  let token = platform ? initial.token : skipAuth ? null : bearer ?? null
  let sentFence = fence
  const checkContext = async () => {
    if (platform) await authStore.getState().reconcileSession()
    const state = authStore.getState()
    if (requestSignal?.aborted || (platform && (state.contextEpoch !== epoch || state.sessionId !== fence.sessionId || state.contextSwitching))) throw contextChanged()
    if (platform && (state.sessionStatus === "unknown" || state.sessionStatus === "unavailable")) {
      throw new SessionCoordinationError(state.coordinationError ?? "请重新登录", state.sessionStatus === "unknown" ? "SESSION_UNKNOWN" : "SESSION_UNAVAILABLE")
    }
  }
  const send = async (): Promise<Response> => {
    await checkContext()
    if (sessionRequest) { const state = authStore.getState(); token = state.token; sentFence = { sessionId: state.sessionId, version: state.sessionVersion } }
    const headers: Record<string, string> = { Accept: "application/json" }
    if (body !== undefined) headers["Content-Type"] = "application/json"
    if (rawBody !== undefined && contentType) headers["Content-Type"] = contentType
    if (token) headers.Authorization = `Bearer ${token}`
    try { return await fetch(path, { method, headers, body: rawBody ?? (body !== undefined ? JSON.stringify(body) : undefined), signal: requestSignal }) }
    catch (error) {
      await checkContext()
      if ((error as Error).name === "AbortError") throw error
      throw new ApiError("网络请求失败,请检查网络或服务状态", "NETWORK_ERROR", 0)
    }
  }
  let response = await send()
  await checkContext()
  if (response.status === 401 && sessionRequest && initial.refreshToken) {
    await renewSession(sentFence)
    await checkContext()
    response = await send() // Exactly one retry, using the latest complete pair.
    await checkContext()
  }
  await checkContext() // Reconcile before reading a body even if events are delayed.
  const text = await response.text()
  await checkContext()
  let data: unknown
  if (text) { try { data = JSON.parse(text) } catch { data = text } }
  if (!response.ok) {
    let message = `请求失败 (HTTP ${response.status})`, code = "SERVER_ERROR"
    if (data && typeof data === "object" && "message" in data) {
      const value = data as Record<string, unknown>
      if (typeof value.message === "string") message = value.message
      if (typeof value.code === "string") code = value.code
    }
    if (response.status === 401 && sessionRequest && !suppress401 && authStore.getState().token === token) {
      if (await authStore.getState().logout(fence)) redirectToLogin()
    }
    const candidate = response.headers.get("x-request-id")
    const requestId = candidate && candidate.length === 36 && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(candidate) ? candidate : undefined
    throw new ApiError(requestId ? `${message}（请求编号：${requestId}）` : message, code, response.status, requestId)
  }
  // A body may have completed after another page changed the logical session.
  // Receipts must obey the same fence as the successful business response.
  await checkContext()
  const candidate = response.headers.get("x-request-id")
  const requestId = candidate && candidate.length === 36 && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(candidate) ? candidate : undefined
  onResponse?.({ requestId })
  return data as T
}
export async function switchPlatformContext(organizationId: string, expected?: SessionFence): Promise<ContextResp> {
  const invoked = authStore.getState()
  const fence = expected ?? { sessionId: invoked.sessionId, version: invoked.sessionVersion }
  reconcileSessionSync()
  const observed = authStore.getState()
  if (observed.sessionId !== fence.sessionId || observed.contextEpoch !== invoked.contextEpoch || observed.contextSwitching) throw contextChanged()
  await authStore.getState().reconcileSession()
  const result = await sessionCoordinator.operation("context", fence, async ({ record, signal, updateTokens }) => {
    let token = record.token
    const send = () => { signal.throwIfAborted(); return fetch("/api/auth/context", { method: "POST", signal,
      headers: { Accept: "application/json", "Content-Type": "application/json", Authorization: `Bearer ${token}` },
      body: JSON.stringify({ organizationId }),
    }) }
    let response: Response
    try {
      response = await send()
      signal.throwIfAborted()
      if (response.status === 401 && record.refreshToken) {
        const tokens = await refreshPair(record, signal)
        signal.throwIfAborted()
        updateTokens(tokens)
        token = tokens.token
        response = await send()
        signal.throwIfAborted()
      }
    } catch (error) {
      if (error instanceof SessionOperationError || error instanceof SessionCoordinationError) throw error
      throw new SessionOperationError("组织切换结果无法确认，请重新登录", "unknown")
    }
    let data: ContextResp & { message?: string }
    try { data = await response.json() } catch { throw new SessionOperationError("组织切换响应无法确认，请重新登录", "unknown", response.status) }
    signal.throwIfAborted()
    if (!response.ok) throw new SessionOperationError(data.message ?? "组织切换失败",
      response.status === 400 || response.status === 403 || response.status === 404 ? "safe" : response.status === 401 ? "invalid" : "unknown", response.status)
    if (typeof data.accessToken !== "string" || !data.accessToken || typeof data.refreshToken !== "string" || !data.refreshToken) throw new SessionOperationError("组织切换响应无法确认，请重新登录", "unknown", 502)
    return { value: data, tokens: { token: data.accessToken, refreshToken: data.refreshToken } }
  })
  return result as ContextResp
}
/** Remote revoke and local clear share the lock and captured logical identity. */
export async function logoutPlatformSession(expected = captureSessionFence()): Promise<boolean> {
  try {
    const result = await sessionCoordinator.operation("logout", expected, async ({ record, signal }) => {
      signal.throwIfAborted()
      const response = await fetch("/api/auth/logout", { method: "POST", signal,
        headers: { Accept: "application/json", Authorization: `Bearer ${record.token}` } })
      signal.throwIfAborted()
      if (!response.ok && response.status !== 401) throw new SessionOperationError("退出结果无法确认，请重新登录", "unknown", response.status)
      return { value: true }
    })
    return result === true
  } catch (error) {
    if (error instanceof SessionCoordinationError && error.code === "SESSION_CHANGED") return false
    throw error
  }
}

export const api = {
  get: <T>(path: string, options?: RequestOptions) => request<T>(path, { ...options, method: "GET" }),
  post: <T>(path: string, body?: unknown, options?: RequestOptions) =>
    request<T>(path, { ...options, method: "POST", body }),
  postRaw: <T>(path: string, rawBody: BodyInit, contentType: string, options?: RequestOptions) =>
    request<T>(path, { ...options, method: "POST", rawBody, contentType }),
  put: <T>(path: string, body?: unknown, options?: RequestOptions) =>
    request<T>(path, { ...options, method: "PUT", body }),
  patch: <T>(path: string, body?: unknown, options?: RequestOptions) =>
    request<T>(path, { ...options, method: "PATCH", body }),
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
