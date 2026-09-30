import { api } from "@/lib/api"
import type { AuthorizationResp, AuthorizationPayload, OrgNode, OrgPayload } from "./types"

export const organizationsApi = {
  tree: (tenantId: string) => api.get<unknown>(`/api/organizations/tree?tenantId=${encodeURIComponent(tenantId)}`),
  create: (body: OrgPayload) => api.post("/api/organizations", body),
  update: (id: string, body: OrgPayload) => api.put(`/api/organizations/${id}`, body),
  remove: (id: string) => api.del(`/api/organizations/${id}`),
  setStatus: (id: string, status: string) => api.put(`/api/organizations/${id}/status`, { status }),

  authorizations: (id: string) => api.get<AuthorizationResp>(`/api/organizations/${id}/authorizations`),
  saveAuthorizations: (id: string, body: AuthorizationPayload) =>
    api.put(`/api/organizations/${id}/authorizations`, body),
}
