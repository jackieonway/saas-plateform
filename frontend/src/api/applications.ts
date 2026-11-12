import { api, qs } from "@/lib/api"
import type {
  ApplicationDeploymentRecord,
  ApplicationEnvironment,
  ApplicationPayload,
  ApplicationRecord,
  ApplicationReleasePayload,
  ApplicationReleaseRecord,
  ApplicationUrlEntryPayload,
  ApplicationUrlEntryRecord,
} from "./types"

export const applicationsApi = {
  list: () => api.get<unknown>("/api/applications"),
  create: (body: ApplicationPayload) => api.post<ApplicationRecord>("/api/applications", body),
  update: (id: string, body: ApplicationPayload) => api.put(`/api/applications/${id}`, body),
  remove: (id: string) => api.del(`/api/applications/${id}`),
  urlEntry: (id: string, environment: ApplicationEnvironment) =>
    api.get<ApplicationUrlEntryRecord | null>(`/api/applications/${encodeURIComponent(id)}/url-entry${qs({ environment })}`),
  setUrlEntry: (id: string, environment: ApplicationEnvironment, body: ApplicationUrlEntryPayload) =>
    api.put<ApplicationUrlEntryRecord>(`/api/applications/${encodeURIComponent(id)}/url-entry${qs({ environment })}`, {
      entryUrl: body.entryUrl, proxyEnabled: body.proxyEnabled,
    }),
  removeUrlEntry: (id: string, environment: ApplicationEnvironment) =>
    api.del(`/api/applications/${encodeURIComponent(id)}/url-entry${qs({ environment })}`),
  releases: (id: string, environment?: ApplicationEnvironment) =>
    api.get<ApplicationReleaseRecord[]>(`/api/applications/${id}/releases${qs({ environment })}`),
  registerRelease: (id: string, body: ApplicationReleasePayload) =>
    api.post<ApplicationReleaseRecord>(`/api/applications/${id}/releases`, {
      environment: body.environment, version: body.version, entryType: "HTML",
      entryUrl: body.entryUrl, healthUrl: body.healthUrl,
      resources: body.resources.map(({ url, kind }) => ({ url, kind })),
    }),
  preflightRelease: (id: string, releaseId: string) =>
    api.post<ApplicationReleaseRecord>(`/api/applications/${id}/releases/${releaseId}/preflight`),
  removeRelease: (id: string, releaseId: string) =>
    api.del(`/api/applications/${encodeURIComponent(id)}/releases/${encodeURIComponent(releaseId)}`),
  activateRelease: (id: string, releaseId: string) =>
    api.post<ApplicationDeploymentRecord>(`/api/applications/${id}/releases/${releaseId}/activate`),
  deployments: (id: string) =>
    api.get<ApplicationDeploymentRecord[]>(`/api/applications/${id}/deployments`),
  rollbackDeployment: (id: string, environment: ApplicationEnvironment) =>
    api.post<ApplicationDeploymentRecord>(`/api/applications/${id}/deployments/${environment}/rollback`),
}
