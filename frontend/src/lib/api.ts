import { isUuidV7 } from "@/lib/uuid"
import { authStore, captureSessionFence, captureSessionSignal, reconcileSessionSync } from "@/stores/auth"
import type { SessionFence } from "@/stores/auth"
import { SessionCoordinationError } from "@/lib/session-types"
import { bffSessionCoordinator } from "@/lib/bff-session"
import type { ContextResp } from "@/api/types"

export class ApiError extends Error {
  constructor(
    message: string,
    public code: string,
    public status: number,
    public requestId?: string,
    public operationId?: string,
  ) {
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
  /** Stable browser-owned ID for recoverable identity management writes. */
  operationId?: string
}
function redirectToLogin() {
  const { pathname, search } = window.location
  if (pathname !== "/login") window.location.assign(`/login?return=${encodeURIComponent(pathname + search)}`)
}
function contextChanged(): DOMException { return new DOMException("登录上下文已变化", "AbortError") }

async function renewSession(fence: SessionFence): Promise<string> {
  try { return await bffSessionCoordinator.renew(fence) }
  catch (error) {
    if (error instanceof Error && error.name === "AbortError") throw error
    const status = error && typeof error === "object" && "status" in error && typeof error.status === "number" ? error.status : 0
    throw new ApiError(error instanceof Error ? error.message : "认证服务暂时不可用，请重新检查", "BFF_SESSION_RENEWAL_FAILED", status)
  }
}
export async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const { method = "GET", body, rawBody, contentType, bearer, sessionFence, skipAuth, suppress401, signal, onResponse, operationId } = options
  if (operationId !== undefined && (!isUuidV7(operationId) || method === "GET")) throw new ApiError("管理操作标识无效", "INVALID_OPERATION_ID", 400)
  const platform = !skipAuth && (bearer === undefined || sessionFence !== undefined)
  // Capture the caller's view before any await. A queued A mutation must never
  // acquire B's identity merely because a cross-page context operation finishes.
  const invoked = authStore.getState(), epoch = invoked.contextEpoch
  const fence = sessionFence ?? { sessionId: invoked.sessionId, version: invoked.sessionVersion, contextKey: invoked.contextKey }
  const lifetime = captureSessionSignal()
  if (platform) {
    reconcileSessionSync()
    const observed = authStore.getState()
    if (observed.sessionStatus === "unknown" || observed.sessionStatus === "unavailable") {
      throw new SessionCoordinationError(observed.coordinationError ?? "请重新登录", observed.sessionStatus === "unknown" ? "SESSION_UNKNOWN" : "SESSION_UNAVAILABLE")
    }
    if (observed.contextSwitching || observed.sessionId !== fence.sessionId || observed.contextKey !== fence.contextKey || observed.contextEpoch !== epoch || observed.sessionStatus === "initializing") throw contextChanged()
  }
  if (platform) await authStore.getState().reconcileSession()
  const initial = authStore.getState()
  const sessionRequest = platform && !!invoked.token
  const requestSignal = sessionRequest ? AbortSignal.any([lifetime, ...(signal ? [signal] : [])]) : signal
  if (sessionFence && (initial.sessionId !== sessionFence.sessionId || initial.contextKey !== sessionFence.contextKey)) throw contextChanged()
  let token = platform ? initial.token : skipAuth ? null : bearer ?? null
  let sentFence = fence
  const checkContext = async () => {
    if (platform) await authStore.getState().reconcileSession()
    const state = authStore.getState()
    if (requestSignal?.aborted || (platform && (state.contextEpoch !== epoch || state.sessionId !== fence.sessionId || state.contextKey !== fence.contextKey || state.contextSwitching))) throw contextChanged()
    if (platform && (state.sessionStatus === "unknown" || state.sessionStatus === "unavailable")) {
      throw new SessionCoordinationError(state.coordinationError ?? "请重新登录", state.sessionStatus === "unknown" ? "SESSION_UNKNOWN" : "SESSION_UNAVAILABLE")
    }
  }
  const send = async (): Promise<Response> => {
    await checkContext()
    if (sessionRequest) { const state = authStore.getState(); token = state.token; sentFence = { sessionId: state.sessionId, version: state.sessionVersion, contextKey: state.contextKey } }
    const headers: Record<string, string> = { Accept: "application/json" }
    if (operationId) headers["x-identity-operation"] = operationId
    if (body !== undefined) headers["Content-Type"] = "application/json"
    if (rawBody !== undefined && contentType) headers["Content-Type"] = contentType
    if (token) headers.Authorization = `Bearer ${token}`
    try { return await fetch(path, { method, headers, credentials: "same-origin", body: rawBody ?? (body !== undefined ? JSON.stringify(body) : undefined), signal: requestSignal }) }
    catch (error) {
      await checkContext()
      if ((error as Error).name === "AbortError") throw error
      throw new ApiError("网络请求失败,请检查网络或服务状态", "NETWORK_ERROR", 0)
    }
  }
  let response = await send()
  await checkContext()
  if (response.status === 401 && sessionRequest) {
    await renewSession(sentFence)
    await checkContext()
    response = await send() // Exactly one retry, using the latest complete pair.
    await checkContext()
  }
  await checkContext() // Reconcile before reading a body even if events are delayed.
  let text: string
  try { text = await response.text() }
  catch (error) {
    await checkContext()
    if ((error as Error).name === "AbortError") throw error
    throw new ApiError("响应读取失败，请检查网络或服务状态", "NETWORK_ERROR", 0)
  }
  await checkContext()
  let data: unknown
  if (text) { try { data = JSON.parse(text) } catch { data = text } }
  if (!response.ok) {
    let message = `请求失败 (HTTP ${response.status})`, code = "SERVER_ERROR", operationId: string | undefined
    if (data && typeof data === "object" && "message" in data) {
      const value = data as Record<string, unknown>
      if (typeof value.message === "string") message = value.message
      if (typeof value.code === "string") code = value.code
      if (typeof value.operationId === "string" && isUuidV7(value.operationId)) operationId = value.operationId
    }
    if (response.status === 401 && sessionRequest && !suppress401 && authStore.getState().token === token) {
      if (await authStore.getState().logout(fence)) redirectToLogin()
    }
    const candidate = response.headers.get("x-request-id")
    const requestId = isUuidV7(candidate) ? candidate : undefined
    throw new ApiError(requestId ? `${message}（请求编号：${requestId}）` : message, code, response.status, requestId, operationId)
  }
  // A body may have completed after another page changed the logical session.
  // Receipts must obey the same fence as the successful business response.
  await checkContext()
  const candidate = response.headers.get("x-request-id")
  const requestId = isUuidV7(candidate) ? candidate : undefined
  onResponse?.({ requestId })
  return data as T
}
export async function switchPlatformContext(organizationId: string, membershipId: string, expected?: SessionFence): Promise<ContextResp> {
  if (!isUuidV7(organizationId) || !isUuidV7(membershipId)) throw new ApiError("请选择有效的具体成员身份", "INVALID_CONTEXT_SELECTION", 400)
  organizationId = organizationId.toLowerCase()
  membershipId = membershipId.toLowerCase()
  const invoked = authStore.getState()
  const fence = expected ?? { sessionId: invoked.sessionId, version: invoked.sessionVersion, contextKey: invoked.contextKey }
  return bffSessionCoordinator.selectContext(organizationId, membershipId, fence)
}
export async function logoutPlatformSession(expected = captureSessionFence(), scope: "PLATFORM" | "GLOBAL" = "PLATFORM"): Promise<boolean> {
  return bffSessionCoordinator.logout(expected, scope)
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
