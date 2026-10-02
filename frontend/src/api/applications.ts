import { api, qs } from "@/lib/api"
import type {
  ApplicationDeploymentRecord,
  ApplicationEnvironment,
  ApplicationPayload,
  ApplicationRecord,
  ApplicationReleasePayload,
  ApplicationReleaseRecord,
} from "./types"

export const applicationsApi = {
  list: () => api.get<unknown>("/api/applications"),
  create: (body: ApplicationPayload) => api.post<ApplicationRecord>("/api/applications", body),
  update: (id: string, body: ApplicationPayload) => api.put(`/api/applications/${id}`, body),
  remove: (id: string) => api.del(`/api/applications/${id}`),
  releases: (id: string, environment?: ApplicationEnvironment) =>
    api.get<ApplicationReleaseRecord[]>(`/api/applications/${id}/releases${qs({ environment })}`),
  registerRelease: (id: string, body: ApplicationReleasePayload) =>
    api.post<ApplicationReleaseRecord>(`/api/applications/${id}/releases`, body),
  preflightRelease: (id: string, releaseId: string) =>
    api.post<ApplicationReleaseRecord>(`/api/applications/${id}/releases/${releaseId}/preflight`),
  activateRelease: (id: string, releaseId: string) =>
    api.post<ApplicationDeploymentRecord>(`/api/applications/${id}/releases/${releaseId}/activate`),
  deployments: (id: string) =>
    api.get<ApplicationDeploymentRecord[]>(`/api/applications/${id}/deployments`),
  rollbackDeployment: (id: string, environment: ApplicationEnvironment) =>
    api.post<ApplicationDeploymentRecord>(`/api/applications/${id}/deployments/${environment}/rollback`),
}
