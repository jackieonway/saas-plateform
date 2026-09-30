import { api } from "@/lib/api"
import type { MenuNode, MenuPayload } from "./types"

export const menusApi = {
  tree: (applicationId: string) =>
    api.get<unknown>(`/api/menus/tree?applicationId=${encodeURIComponent(applicationId)}`),
  create: (body: MenuPayload) => api.post("/api/menus", body),
  update: (id: string, body: MenuPayload) => api.put(`/api/menus/${id}`, body),
  remove: (id: string) => api.del(`/api/menus/${id}`),
}
