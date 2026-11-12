import { api, qs } from "@/lib/api"
import type { PageResp, PlatformSessionRecord } from "./types"

export interface ManagedSession extends PlatformSessionRecord { userId: string; username: string; realName: string }
export interface SessionUser { userId: string; username: string; realName: string; sessions: number; activeSessions: number; lastIssuedAt: string; isCurrentUser: boolean }
export type SessionFilters = { page: number; size: number; keyword: string; activeOnly: boolean }
export const sessionsApi = {
  devices: (filters: SessionFilters, signal?: AbortSignal) => api.get<PageResp<ManagedSession>>(`/api/admin/sessions${qs({ ...filters, activeOnly: String(filters.activeOnly) })}`, { signal }),
  users: (filters: SessionFilters, signal?: AbortSignal) => api.get<PageResp<SessionUser>>(`/api/admin/session-users${qs({ ...filters, activeOnly: String(filters.activeOnly) })}`, { signal }),
  revoke: (id: string) => api.post(`/api/admin/sessions/${encodeURIComponent(id)}/revoke`),
  revokeAll: (id: string) => api.post(`/api/admin/session-users/${encodeURIComponent(id)}/revoke-all`),
  recover: (operation: string) => api.post(`/api/users/account-operations/${encodeURIComponent(operation)}`),
}
