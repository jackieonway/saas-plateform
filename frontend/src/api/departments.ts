import { api } from "@/lib/api"
import type { DepartmentOrgNode, DeptPayload } from "./types"

export const departmentsApi = {
  organizations: (signal?: AbortSignal) => api.get<DepartmentOrgNode[]>("/api/departments/organizations", { signal }),
  tree: (organizationId: string, signal?: AbortSignal) =>
    api.get<unknown>(`/api/departments/tree?organizationId=${encodeURIComponent(organizationId)}`, { signal }),
  create: (body: DeptPayload) => api.post("/api/departments", body),
  update: (id: string, body: DeptPayload) => api.put(`/api/departments/${id}`, body),
  remove: (id: string) => api.del(`/api/departments/${id}`),
}
