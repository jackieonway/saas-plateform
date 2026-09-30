import { api } from "@/lib/api"
import type { DeptNode, DeptPayload } from "./types"

export const departmentsApi = {
  tree: (organizationId: string) =>
    api.get<unknown>(`/api/departments/tree?organizationId=${encodeURIComponent(organizationId)}`),
  create: (body: DeptPayload) => api.post("/api/departments", body),
  update: (id: string, body: DeptPayload) => api.put(`/api/departments/${id}`, body),
  remove: (id: string) => api.del(`/api/departments/${id}`),
}
