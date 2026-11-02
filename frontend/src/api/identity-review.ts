import { api, ApiError } from "@/lib/api"
import { identityMutation } from "@/api/identity-clients"
import { isUuidV7 } from "@/lib/uuid"
import { z } from "zod"

export interface IntakePartner {
  id: string
  name: string
  contactName: string
  contactChannel: string
  subjectReference: string
  status: "ACTIVE" | "SUSPENDED" | "RETIRED"
  version: number
}
export interface IntakeManifest {
  displayName: string
  purpose: string
  privacyUrl: string
  domains: string[]
  redirectUris: string[]
  postLogoutRedirectUris: string[]
  scopes: ("openid" | "profile")[]
  contactReference: string
  domainEvidenceReference: string
  logoutMode: "SHORT_SESSION_ONLINE_CHECK" | "BACK_CHANNEL"
  backchannelLogoutUri?: string | null
}
export type IntakeStatus = "SUBMITTED" | "RETURNED" | "TEST_APPROVED" | "TEST_ACCEPTED" | "PRODUCTION_APPROVED" | "ENABLED" | "SUSPENDED"
export interface IntakeRequest {
  id: string
  applicationId: string
  environment: "test" | "production"
  manifest: IntakeManifest
  requestedBy: string
  submittedBy: string
  status: IntakeStatus
  reviewedBy: string | null
  publishedBy: string | null
  version: number
  syncedVersion: number
  reason?: string
  syncState?: string
}
export interface IntakeApplication {
  applicationId: string
  partnerId: string
  displayName: string
  createdBy: string
  requests: IntakeRequest[]
}
export interface IntakeSecurity {
  totpEnabled: boolean
  recent: boolean
  securityUrl: string
  reauthenticateUrl?: string
  productionReady: boolean
  permissions: string[]
}
export interface IntakeDecision {
  expectedVersion: number
  status: IntakeStatus
  reason: string
  verifiedContactReference?: string
  verifiedDomainReference?: string
  testEvidenceReference?: string
}
export interface AccountInvitation {
  id: string
  recipientRef: string
  purpose: "ACCOUNT_ONBOARDING"
  status: "ACTIVE" | "REVOKED" | "CONSUMED" | "EXPIRED"
  version: number
  createdAt: string
  expiresAt: string
  revokedAt: string | null
  consumedAt: string | null
}

const BASE = "/api/identity/review"
const invitationMetadata = z.object({
  id: z.string().refine(isUuidV7), recipientRef: z.string().min(1).max(256), purpose: z.literal("ACCOUNT_ONBOARDING"),
  status: z.enum(["ACTIVE", "REVOKED", "CONSUMED", "EXPIRED"]), version: z.number().int().positive().max(Number.MAX_SAFE_INTEGER),
  createdAt: z.string().refine(value => Number.isFinite(Date.parse(value))), expiresAt: z.string().refine(value => Number.isFinite(Date.parse(value))),
  revokedAt: z.string().nullable(), consumedAt: z.string().nullable(),
}).strict()
function checkedInvitation(raw: unknown): AccountInvitation {
  const parsed = invitationMetadata.safeParse(raw)
  if (!parsed.success) throw new ApiError("邀请响应无法按管理元数据合同核验，请查询原操作结果", "INVITATION_RESPONSE_INVALID", 503)
  return parsed.data
}
export const identityReviewApi = {
  security: () => api.get<IntakeSecurity>(`${BASE}/security`),
  partners: () => api.get<{ partners: IntakePartner[] }>(`${BASE}/partners`),
  createPartner: (body: Omit<IntakePartner, "id" | "status" | "version">) =>
    identityMutation("review", body.name, operationId => api.post<IntakePartner>(`${BASE}/partners`, body, { operationId })),
  applications: () => api.get<{ applications: IntakeApplication[]; productionReady: boolean; permissions: string[] }>(`${BASE}/applications`),
  createApplication: (body: { partnerId: string; displayName: string }) =>
    identityMutation("review", body.displayName, operationId => api.post<IntakeApplication>(`${BASE}/applications`, body, { operationId })),
  submit: (applicationId: string, body: { environment: "test" | "production"; expectedVersion: number | null; manifest: IntakeManifest }) =>
    identityMutation("review", body.manifest.displayName, operationId => api.post<IntakeRequest>(`${BASE}/applications/${encodeURIComponent(applicationId)}/requests`, body, { operationId })),
  decide: (request: IntakeRequest, body: IntakeDecision) =>
    identityMutation("review", request.manifest.displayName, operationId => api.post<IntakeRequest>(`${BASE}/requests/${encodeURIComponent(request.id)}/decisions`, body, { operationId })),
  sync: (request: IntakeRequest) =>
    identityMutation("review", request.manifest.displayName, operationId => api.post<IntakeRequest>(`${BASE}/requests/${encodeURIComponent(request.id)}/sync`, { expectedVersion: request.version }, { operationId })),
  assignOperator: (applicationId: string, userId: string) =>
    identityMutation("review", "应用操作人委派", operationId => api.post<{ applicationId: string; userId: string }>(`${BASE}/applications/${encodeURIComponent(applicationId)}/operators`, { userId }, { operationId })),
  invitations: async () => {
    const response = await api.get<{ invitations: unknown[] }>("/api/identity/invitations")
    if (!response || !Array.isArray(response.invitations)) throw new ApiError("邀请列表响应无法核验", "INVITATION_RESPONSE_INVALID", 503)
    return { invitations: response.invitations.map(checkedInvitation) }
  },
  invite: (recipientRef: string) =>
    identityMutation("invite", "受控账号邀请", async operationId => checkedInvitation(await api.post<unknown>("/api/identity/invitations", { recipientRef, purpose: "ACCOUNT_ONBOARDING" }, { operationId }))),
  revokeInvitation: (invitation: AccountInvitation) =>
    identityMutation("invite", "撤销账号邀请", async operationId => checkedInvitation(await api.post<unknown>(`/api/identity/invitations/${encodeURIComponent(invitation.id)}/revoke`, { expectedVersion: invitation.version }, { operationId }))),
}
