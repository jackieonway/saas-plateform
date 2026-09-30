import { api } from "@/lib/api"
import type { ConsentDecisionResp, ConsentInfo, DeviceInfo } from "./types"

export const deviceApi = {
  info: (userCode: string) =>
    api.get<DeviceInfo>(`/api/device/${encodeURIComponent(userCode)}`, { suppress401: true }),
  decide: (userCode: string, approve: boolean) =>
    api.post<{ ok: boolean }>(
      `/api/device/${encodeURIComponent(userCode)}/decision`,
      { approve },
      { suppress401: true }
    ),
}

export const consentApi = {
  info: (requestId: string) =>
    api.get<ConsentInfo>(`/api/oauth/consent/${encodeURIComponent(requestId)}`, { suppress401: true }),
  decide: (requestId: string, approve: boolean) =>
    api.post<ConsentDecisionResp>(
      `/api/oauth/consent/${encodeURIComponent(requestId)}/decision`,
      { approve },
      { suppress401: true }
    ),
}
