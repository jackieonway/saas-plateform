import { api } from "@/lib/api"
import { capturePlatformSessionFence } from "@/stores/auth"

export interface PlatformBranding {
  platformName: string
  platformDescription: string
  logoUrl: string
  loginTitle: string
  loginDescription: string
  footerText: string
  primaryColor: string
}

export interface PlatformSettings extends PlatformBranding {
  platformTenantCode: "admin-tenant"
  platformTenantName: string
}

export type PlatformSettingsPayload = Omit<PlatformSettings, "platformTenantCode">

export const platformApi = {
  branding: (signal?: AbortSignal) =>
    api.get<PlatformBranding>("/api/platform/branding", { skipAuth: true, suppress401: true, signal }),
  settings: (token: string, signal?: AbortSignal) =>
    api.get<PlatformSettings>("/api/platform/settings", { bearer: token, signal, sessionFence: capturePlatformSessionFence(token) }),
  updateSettings: (body: PlatformSettingsPayload, token: string) =>
    api.put<PlatformSettings>("/api/platform/settings", body, { bearer: token, sessionFence: capturePlatformSessionFence(token) }),
}
