import { api, logoutPlatformSession, qs, switchPlatformContext } from "@/lib/api"
import { capturePlatformSessionFence } from "@/stores/auth"
import type { SessionFence } from "@/stores/auth"
import type {
  LoginHistoryPage,
  MenuNode,
  MySessionsResp,
  PermissionsResp,
  ProfileResp,
} from "./types"

export const authApi = {
  /** 已登录会话中切换组织 */
  switchContext: (organizationId: string, membershipId: string, token: string) => {
    const sessionFence = capturePlatformSessionFence(token)
    return switchPlatformContext(organizationId, membershipId, sessionFence)
  },

  profile: (bearer?: string, signal?: AbortSignal) => api.get<ProfileResp>("/api/auth/profile", {
    bearer, signal, sessionFence: bearer === undefined ? undefined : capturePlatformSessionFence(bearer),
  }),

  menus: (applicationId: string, bearer?: string, signal?: AbortSignal) =>
    api.get<unknown>(`/api/auth/menus?applicationId=${encodeURIComponent(applicationId)}`, {
      bearer, signal, sessionFence: bearer === undefined ? undefined : capturePlatformSessionFence(bearer),
    }),

  permissions: (bearer?: string, signal?: AbortSignal) => api.get<PermissionsResp>("/api/auth/permissions", {
    bearer, signal, sessionFence: bearer === undefined ? undefined : capturePlatformSessionFence(bearer),
  }),

  logout: async () => ({ ok: await logoutPlatformSession() }),

  changePassword: (oldPassword: string, newPassword: string, sessionFence?: SessionFence) =>
    api.put<{ ok: boolean }>("/api/auth/password", { oldPassword, newPassword }, { sessionFence }),

  /** 本人登录历史(安全中心时间线, 含 LOGIN / LOGIN_FAILED) */
  loginHistory: (params: { page?: number; size?: number } = {}) =>
    api.get<LoginHistoryPage>(`/api/auth/login-history${qs({ page: params.page, size: params.size })}`),

  /** 本人名下按 OAuth 客户端聚合的会话概览 */
  mySessions: () => api.get<MySessionsResp>("/api/auth/my-sessions"),
  revokeSession: (id: string) => api.del<{ ok: boolean }>(`/api/auth/sessions/${encodeURIComponent(id)}`),
}
