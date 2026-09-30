import { api, qs } from "@/lib/api"
import type {
  ContextResp,
  LoginHistoryPage,
  LoginResp,
  MenuNode,
  MySessionsResp,
  PermissionsResp,
  ProfileResp,
} from "./types"

export const authApi = {
  login: (username: string, password: string) =>
    api.post<LoginResp>("/api/auth/login", { username, password }, { skipAuth: true, suppress401: true }),

  /** 登录第二步:用 credential ticket 选择组织建立会话 */
  selectContext: (organizationId: string, ticket: string) =>
    api.post<ContextResp>("/api/auth/context", { organizationId }, { bearer: ticket, suppress401: true }),

  /** 已登录会话中切换组织 */
  switchContext: (organizationId: string, token: string) =>
    api.post<ContextResp>("/api/auth/context", { organizationId }, { bearer: token, suppress401: true }),

  profile: () => api.get<ProfileResp>("/api/auth/profile"),

  menus: (applicationId: string) =>
    api.get<unknown>(`/api/auth/menus?applicationId=${encodeURIComponent(applicationId)}`),

  permissions: () => api.get<PermissionsResp>("/api/auth/permissions"),

  logout: () => api.post<{ ok: boolean }>("/api/auth/logout"),

  changePassword: (oldPassword: string, newPassword: string) =>
    api.put<{ ok: boolean }>("/api/auth/password", { oldPassword, newPassword }),

  /** 本人登录历史(安全中心时间线, 含 LOGIN / LOGIN_FAILED) */
  loginHistory: (params: { page?: number; size?: number } = {}) =>
    api.get<LoginHistoryPage>(`/api/auth/login-history${qs({ page: params.page, size: params.size })}`),

  /** 本人名下按 OAuth 客户端聚合的会话概览 */
  mySessions: () => api.get<MySessionsResp>("/api/auth/my-sessions"),
  revokeSession: (id: string) => api.del<{ ok: boolean }>(`/api/auth/sessions/${encodeURIComponent(id)}`),
}
