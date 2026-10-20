import { api } from "@/lib/api"
export type ZipTarget = { id: string; name: string; applications: string[]; nodes: { id: string; origin: string }[] }
export type ZipIntent = { releaseId: string; groupId: string; requestId: string }
export type ZipJob = {
  id: string; releaseId: string; version: string; environment: string; groupId: string; artifactDigest: string
  state: "QUEUED" | "RUNNING" | "READY" | "FAILED"; attempt: number; errorCode: string | null
  nodes: { id: string; state: string; errorCode: string | null }[]
}
export const zipDistributionsApi = {
  targets: () => api.get<ZipTarget[]>("/api/distribution-targets"),
  list: (app: string) => api.get<{ jobs: ZipJob[]; candidates: { releaseId: string; environment: string; version: string }[] }>(`/api/applications/${app}/zip-distributions`),
  create: (app: string, body: ZipIntent) => api.post<{ id: string; reused: boolean }>(`/api/applications/${app}/zip-distributions`, body),
  retry: (app: string, job: string) => api.post(`/api/applications/${app}/zip-distributions/${job}/retry`, {}),
  verify: (app: string, job: string) => api.post(`/api/applications/${app}/zip-distributions/${job}/verify`, {}),
}
