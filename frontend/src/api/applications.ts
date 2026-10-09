import { api, qs } from "@/lib/api"
import type {
  ApplicationDeploymentRecord,
  ApplicationEnvironment,
  ApplicationPayload,
  ApplicationRecord,
  ApplicationReleaseRecord,
} from "./types"

export const applicationsApi = {
  list: () => api.get<unknown>("/api/applications"),
  create: (body: ApplicationPayload) => api.post<ApplicationRecord>("/api/applications", body),
  update: (id: string, body: ApplicationPayload) => api.put(`/api/applications/${id}`, body),
  remove: (id: string) => api.del(`/api/applications/${id}`),
  releases: (id: string, environment?: ApplicationEnvironment) =>
    api.get<ApplicationReleaseRecord[]>(`/api/applications/${id}/releases${qs({ environment })}`),
  uploadRelease: (id: string, environment: ApplicationEnvironment, version: string, entryType: "HTML" | "ESM", file: File) => {
    const format = file.name.toLowerCase().endsWith(".tar.gz") ? "tar.gz" : "zip"
    return api.postRaw<ApplicationReleaseRecord>(
      `/api/applications/${id}/releases/upload${qs({ environment, version, entryType, format })}`,
      file,
      format === "zip" ? "application/zip" : "application/gzip",
    )
  },
  preflightRelease: (id: string, releaseId: string) =>
    api.post<ApplicationReleaseRecord>(`/api/applications/${id}/releases/${releaseId}/preflight`),
  activateRelease: (id: string, releaseId: string) =>
    api.post<ApplicationDeploymentRecord>(`/api/applications/${id}/releases/${releaseId}/activate`),
  deployments: (id: string) =>
    api.get<ApplicationDeploymentRecord[]>(`/api/applications/${id}/deployments`),
  rollbackDeployment: (id: string, environment: ApplicationEnvironment) =>
    api.post<ApplicationDeploymentRecord>(`/api/applications/${id}/deployments/${environment}/rollback`),
}
