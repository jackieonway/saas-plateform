import { api, qs } from "@/lib/api"
import type { ApplicationEnvironment, Status } from "./types"

export interface ApplicationOrganizationOption {
  id: string
  name: string
  code: string
  status: Status
  tenantId: string
  parentId: string | null
  expiresAt: string | null
  children: ApplicationOrganizationOption[]
}

export interface ApplicationAccessTarget {
  tenantId: string
  organizationId: string
  roleId: string
}

export interface ApplicationAccessPayload extends ApplicationAccessTarget {
  menuIds: string[]
}

export interface ApplicationSetupStatus {
  applicationId: string
  environment: ApplicationEnvironment
  applicationEnabled: boolean
  /** Management reads may include archived metadata of disabled applications. */
  entryType: string
  runtimeMode: string
  releaseRequired: boolean
  activeVersion: string | null
  releaseValidated: boolean
  /** Optional during rolling upgrades with older platform servers. */
  entrySource?: "URL" | "RELEASE" | "LEGACY"
  urlEntryConfigured?: boolean
  proxyEnabled?: boolean
  menuCount: number
  entryMenuCount: number
  canConfigureAccess: boolean
  access: (ApplicationAccessTarget & {
    tenantAuthorized: boolean
    organizationAuthorized: boolean
    roleAuthorized: boolean
    assignedUserCount: number
  }) | null
}

export const applicationSetupApi = {
  organizations: (id: string, tenantId: string) =>
    api.get<ApplicationOrganizationOption[]>(`/api/applications/${encodeURIComponent(id)}/setup/organizations${qs({ tenantId })}`),
  status: (id: string, environment?: ApplicationEnvironment, target?: ApplicationAccessTarget) =>
    api.get<ApplicationSetupStatus>(`/api/applications/${encodeURIComponent(id)}/setup${qs({ environment,
      tenantId: target?.tenantId, organizationId: target?.organizationId, roleId: target?.roleId })}`),
  grantAccess: (id: string, body: ApplicationAccessPayload) =>
    api.post<{ ok: boolean }>(`/api/applications/${encodeURIComponent(id)}/setup/access`, body),
}
