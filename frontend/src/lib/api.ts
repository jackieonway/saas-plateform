import { authStore } from "@/stores/auth"

export class ApiError extends Error {
  code: string
  status: number
  constructor(message: string, code: string, status: number) {
    super(message)
    this.name = "ApiError"
    this.code = code
    this.status = status
  }
}

export interface RequestOptions {
  method?: "GET" | "POST" | "PUT" | "DELETE"
  body?: unknown
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

export async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const { method = "GET", body, bearer, skipAuth, suppress401, signal } = options

  const headers: Record<string, string> = { Accept: "application/json" }
  if (body !== undefined) headers["Content-Type"] = "application/json"

  const storeToken = authStore.getState().token
  const token = bearer !== undefined ? bearer : skipAuth ? null : storeToken
  if (token) headers["Authorization"] = `Bearer ${token}`

  let resp: Response
  try {
    resp = await fetch(path, {
      method,
      headers,
      body: body !== undefined ? JSON.stringify(body) : undefined,
      signal,
    })
  } catch (e) {
    if ((e as Error).name === "AbortError") throw e
    throw new ApiError("网络请求失败,请检查网络或服务状态", "NETWORK_ERROR", 0)
  }

  // 204 / 空响应
  const text = await resp.text()
  let data: unknown = undefined
  if (text) {
    try {
      data = JSON.parse(text)
    } catch {
      data = text
    }
  }

  if (!resp.ok) {
    let message = `请求失败 (HTTP ${resp.status})`
    let code = "SERVER_ERROR"
    if (data && typeof data === "object" && "message" in data) {
      const m = (data as Record<string, unknown>).message
      if (typeof m === "string") message = m
      const c = (data as Record<string, unknown>).code
      if (typeof c === "string") code = c
    }
    if (resp.status === 401 && !skipAuth && !suppress401 && !bearer) {
      authStore.getState().logout()
      redirectToLogin()
    }
    throw new ApiError(message, code, resp.status)
  }

  return data as T
}

export const api = {
  get: <T>(path: string, options?: RequestOptions) => request<T>(path, { ...options, method: "GET" }),
  post: <T>(path: string, body?: unknown, options?: RequestOptions) =>
    request<T>(path, { ...options, method: "POST", body }),
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
