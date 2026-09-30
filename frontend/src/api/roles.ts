import { api } from "@/lib/api"
import type { RolePayload, RoleRecord } from "./types"

export const rolesApi = {
  list: (params: {
    page?: number
    size?: number
    keyword?: string
    tenantId?: string
    organizationId?: string
  }) => {
    const qs = new URLSearchParams()
    if (params.page) qs.set("page", String(params.page))
    if (params.size) qs.set("size", String(params.size))
    if (params.keyword) qs.set("keyword", params.keyword)
    if (params.tenantId) qs.set("tenantId", params.tenantId)
    if (params.organizationId) qs.set("organizationId", params.organizationId)
    const q = qs.toString()
    return api.get<unknown>(`/api/roles${q ? `?${q}` : ""}`)
  },
  create: (body: RolePayload) => api.post("/api/roles", body),
  update: (id: string, body: RolePayload) => api.put(`/api/roles/${id}`, body),
  remove: (id: string) => api.del(`/api/roles/${id}`),

  menus: (id: string) => api.get<{ menuIds: string[] }>(`/api/roles/${id}/menus`),
  saveMenus: (id: string, menuIds: string[]) => api.put(`/api/roles/${id}/menus`, { menuIds }),

  users: (id: string) => api.get<unknown>(`/api/roles/${id}/users`),
  saveUsers: (id: string, userIds: string[]) => api.put(`/api/roles/${id}/users`, { userIds }),
}
