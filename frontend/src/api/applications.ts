import { api } from "@/lib/api"
import type { ApplicationPayload, ApplicationRecord } from "./types"

export const applicationsApi = {
  list: () => api.get<unknown>("/api/applications"),
  create: (body: ApplicationPayload) => api.post("/api/applications", body),
  update: (id: string, body: ApplicationPayload) => api.put(`/api/applications/${id}`, body),
  remove: (id: string) => api.del(`/api/applications/${id}`),
}
