import { api } from "@/lib/api"
import type { AuthorizationResp, AuthorizationPayload, TenantPayload, EntityCreationResult } from "./types"

export const tenantsApi = {
  list: (params: { page?: number; size?: number; keyword?: string; status?: string }) =>
    api.get<unknown>(
      `/api/tenants${paramsToString({ page: params.page, size: params.size, keyword: params.keyword, status: params.status })}`
    ),
  create: (body: TenantPayload) => api.post<EntityCreationResult>("/api/tenants", body),
  update: (id: string, body: TenantPayload) => api.put(`/api/tenants/${id}`, body),
  initializeAdministrator: (id: string) => api.post<EntityCreationResult>(`/api/tenants/${id}/initialize-administrator`, {}),
  remove: (id: string) => api.del(`/api/tenants/${id}`),
  setStatus: (id: string, status: string) => api.put(`/api/tenants/${id}/status`, { status }),

  authorizations: (id: string) => api.get<AuthorizationResp>(`/api/tenants/${id}/authorizations`),
  saveAuthorizations: (id: string, body: AuthorizationPayload) =>
    api.put(`/api/tenants/${id}/authorizations`, body),
}

export function paramsToString(params: Record<string, string | number | undefined | null>): string {
  const entries = Object.entries(params).filter(
    ([, v]) => v !== undefined && v !== null && v !== "" && v !== "ALL"
  )
  if (entries.length === 0) return ""
  return "?" + entries.map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(String(v))}`).join("&")
}
