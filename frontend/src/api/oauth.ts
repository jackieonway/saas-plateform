import { api } from "@/lib/api"
import { capturePlatformSessionFence } from "@/stores/auth"
import type {
  DeviceCodeRecord,
  AdminPlatformSession,
  OAuthClientCreateResp,
  OAuthClientPayload,
  OAuthClientRecord,
  OAuthSession,
  OAuthSessionTokenResp,
  OAuthTokenRecord,
} from "./types"

export const oauthApi = {
  clients: () => api.get<unknown>("/api/oauth/clients"),
  createClient: (body: OAuthClientPayload) => api.post<OAuthClientCreateResp>("/api/oauth/clients", body),
  updateClient: (id: string, body: OAuthClientPayload) => api.put(`/api/oauth/clients/${id}`, body),
  removeClient: (id: string) => api.del(`/api/oauth/clients/${id}`),
  regenerateSecret: (id: string) =>
    api.post<{ clientSecret: string }>(`/api/oauth/clients/${id}/regenerate-secret`),

  tokens: (resourceOwner?: string) =>
    api.get<unknown>(`/api/oauth/tokens${resourceOwner ? `?resourceOwner=${encodeURIComponent(resourceOwner)}` : ""}`),
  /** 会话视图:按用户聚合的活跃/吊销令牌概览 (system:oauth) */
  sessions: () => api.get<{ sessions: OAuthSession[]; platformSessions?: AdminPlatformSession[] }>("/api/oauth/tokens/sessions"),
  revokeToken: (id: string) => api.del(`/api/oauth/tokens/${id}`),
  /** 会话踢出: 吊销指定用户全部未吊销令牌 (system:oauth) */
  kickSessions: (userId: string) =>
    api.post<{ revoked: number }>("/api/oauth/tokens/kick", { userId }),

  deviceCodes: () => api.get<unknown>("/api/oauth/device-codes"),
  revokeDeviceCode: (id: string) => api.put(`/api/oauth/device-codes/${id}/revoke`),

  /** 清理过期数据: 删除过期超过 7 天的令牌与设备码记录 (权限 system:oauth, 后端契约 18-b) */
  purgeTokens: () =>
    api.post<{ tokensDeleted: number; devicesDeleted: number }>("/api/oauth/tokens/purge"),

  /** 微应用短期受众令牌；宿主在令牌和完整上下文就绪后才挂载。 */
  sessionToken: (appCode: string, bearer?: string, signal?: AbortSignal) =>
    api.post<OAuthSessionTokenResp>("/api/oauth/session-token", { appCode }, {
      bearer, signal, suppress401: true, sessionFence: bearer === undefined ? undefined : capturePlatformSessionFence(bearer),
    }),

  discovery: () => api.get<unknown>("/oauth/.well-known/openid-configuration", { skipAuth: true }),
}
