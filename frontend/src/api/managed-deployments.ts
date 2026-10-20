import { api } from "@/lib/api"

export type DeploymentTarget = {
  id: string; name: string; publicOrigin: string; repositories: string[]
  templates: { id: string; applicationCode: string; healthPath: string }[]
}
export type ManagedDeploymentJob = {
  id: string; releaseId: string; version: string; environment: string; targetId: string
  templateId: string; imageRef: string; publicUrl: string; operation: "DEPLOY" | "CLEANUP"
  state: "QUEUED" | "RUNNING" | "READY" | "FAILED" | "CLEANED"
  attempt: number; errorCode: string | null; createdAt: string; updatedAt: string
}
export type DeploymentIntent = {
  releaseId: string; requestId: string; targetId: string; templateId: string; imageRef: string
}
export const managedDeploymentsApi = {
  targets: () => api.get<DeploymentTarget[]>("/api/deployment-targets"),
  list: (app: string) => api.get<ManagedDeploymentJob[]>(`/api/applications/${app}/managed-deployments`),
  create: (app: string, body: DeploymentIntent) => api.post<{ id: string; reused: boolean }>(`/api/applications/${app}/managed-deployments`, {
    releaseId: body.releaseId, requestId: body.requestId, targetId: body.targetId, templateId: body.templateId, imageRef: body.imageRef,
  }),
  retry: (app: string, job: string) => api.post(`/api/applications/${app}/managed-deployments/${job}/retry`),
  cleanup: (app: string, job: string) => api.post(`/api/applications/${app}/managed-deployments/${job}/cleanup`),
}
