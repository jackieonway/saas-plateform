import { api, ApiError, qs } from "@/lib/api"
import { beginIdentityOperation, finishIdentityOperation } from "@/lib/identity-operation-recovery"
import type { IdentityOperationAction } from "@/lib/identity-operation-recovery"
import type {
  ApplicationAuthProjection,
  UnifiedClientListParams,
  UnifiedClientOperation,
  UnifiedClientPage,
  UnifiedClientPayload,
  UnifiedClientRecord,
  UnifiedConsentGroup,
  UnifiedCredentialRotation,
} from "./types"

// The browser owns the operation before dispatch, so losing the entire HTTP
// response cannot lose the only handle for querying an already committed write.
export async function identityMutation<T>(action: IdentityOperationAction, label: string, run: (operationId: string) => Promise<T>): Promise<T> {
  const pending = beginIdentityOperation(action, label)
  const { operationId } = pending
  try {
    const result = await run(operationId)
    finishIdentityOperation(operationId, pending.contextKey)
    return result
  }
  catch (error) {
    if (error instanceof ApiError && (error.status === 0 || error.status >= 500)) {
      throw new ApiError(error.message, "OIDC_REMOTE_RESULT_UNKNOWN", error.status, error.requestId, operationId)
    }
    if (error instanceof ApiError && error.status >= 400 && error.status < 500) finishIdentityOperation(operationId, pending.contextKey)
    throw error
  }
}

export const identityClientsApi = {
  list: (params: UnifiedClientListParams) =>
    api.get<UnifiedClientPage>(`/api/identity/clients${qs({ ...params })}`),
  get: (id: string) => api.get<UnifiedClientRecord>(`/api/identity/clients/${encodeURIComponent(id)}`),
  create: (body: UnifiedClientPayload) =>
    identityMutation("create", body.displayName, operationId => api.post<UnifiedClientRecord>("/api/identity/clients", body, { operationId })),
  update: (id: string, body: UnifiedClientPayload) =>
    identityMutation(body.status === "RETIRED" ? "retire" : "update", body.displayName, operationId => api.patch<UnifiedClientRecord>(`/api/identity/clients/${encodeURIComponent(id)}`, body, { operationId })),
  rotateCredential: (id: string, expectedVersion: number) =>
    identityMutation("rotate", id, operationId => api.post<UnifiedCredentialRotation>(
      `/api/identity/clients/${encodeURIComponent(id)}/credentials`,
      { expectedVersion },
      { operationId },
    )),
  revokeCredentials: (id: string, expectedVersion: number) =>
    identityMutation("revoke", id, operationId => api.post<{ clientId: string; revokedCredentials: number; version: number; policyVersion: number }>(
      `/api/identity/clients/${encodeURIComponent(id)}/credentials/revoke`,
      { expectedVersion },
      { operationId },
    )),
  operation: (id: string) =>
    api.get<UnifiedClientOperation>(`/api/identity/client-operations/${encodeURIComponent(id)}`),
  applicationProjection: (id: string) =>
    api.get<ApplicationAuthProjection>(`/api/identity/applications/${encodeURIComponent(id)}/projection`),
  projectApplication: (id: string, expectedProjectionVersion: number | null, approvedScopes: string[]) =>
    identityMutation("update", id, operationId => api.put<ApplicationAuthProjection>(`/api/identity/applications/${encodeURIComponent(id)}/projection`, {
      expectedProjectionVersion,
      approvedScopes,
    }, { operationId })),
  consentGroups: (applicationId: string) =>
    api.get<{ groups: UnifiedConsentGroup[] }>(`/api/identity/applications/${encodeURIComponent(applicationId)}/consent-groups`),
  createConsentGroup: (body: {
    applicationId: string
    environment: string
    displayName: string
    purpose: string
    contextMode: "ACCOUNT_ONLY" | "PLATFORM_CONTEXT"
  }) => identityMutation("group_create", body.displayName, operationId => api.post<Omit<UnifiedConsentGroup, "clients">>("/api/identity/consent-groups", body, { operationId })),
  addConsentGroupClient: (groupId: string, body: { clientId: string; expectedGroupVersion: number }) =>
    identityMutation("group_member", body.clientId, operationId => api.post<{ groupId: string; clientId: string; version: number }>(
      `/api/identity/consent-groups/${encodeURIComponent(groupId)}/clients`,
      body,
      { operationId },
    )),
}
