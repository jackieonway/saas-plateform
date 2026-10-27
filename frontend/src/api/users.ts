import { api } from "@/lib/api"
import type { UserMembershipItem, UserPayload, UserRecord, UserRoleItem } from "./types"

export const usersApi = {
  list: (params: {
    page?: number
    size?: number
    keyword?: string
    organizationId?: string
    tenantId?: string
    memberType?: "EMPLOYEE" | "CUSTOMER"
  }) => {
    const qs = new URLSearchParams()
    if (params.page) qs.set("page", String(params.page))
    if (params.size) qs.set("size", String(params.size))
    if (params.keyword) qs.set("keyword", params.keyword)
    if (params.organizationId) qs.set("organizationId", params.organizationId)
    if (params.tenantId) qs.set("tenantId", params.tenantId)
    if (params.memberType) qs.set("memberType", params.memberType)
    const q = qs.toString()
    return api.get<unknown>(`/api/users${q ? `?${q}` : ""}`)
  },
  create: (body: UserPayload) => api.post("/api/users", body),
  update: (id: string, body: { realName: string; email?: string; phone?: string; gender?: string }) =>
    api.put(`/api/users/${id}`, body),
  remove: (id: string) => api.del(`/api/users/${id}`),
  setStatus: (id: string, status: string) => api.put(`/api/users/${id}/status`, { status }),
  /** 批量启用/禁用 → { succeeded, failed, skipped }(自身 id 由服务端跳过并计入 skipped, 上限 100) */
  batchStatus: (userIds: string[], status: "ENABLED" | "DISABLED") =>
    api.post<{ succeeded: string[]; failed: string[]; skipped: string[] }>("/api/users/batch-status", {
      userIds,
      status,
    }),
  resetPassword: (id: string, newPassword: string) =>
    api.post(`/api/users/${id}/reset-password`, { newPassword }),

  memberships: (id: string) => api.get<unknown>(`/api/users/${id}/memberships`),
  saveMemberships: (
    id: string,
    items: { organizationId: string; departmentId: string | null; position: string }[]
  ) => api.put(`/api/users/${id}/memberships`, { items }),

  roles: (id: string) => api.get<unknown>(`/api/users/${id}/roles`),
  saveRoles: (id: string, items: UserRoleItem[]) => api.put(`/api/users/${id}/roles`, { items }),
}
