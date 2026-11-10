export type TokenField = "accessTokenTtlSeconds" | "refreshTokenTtlSeconds" | "clientTokenTtlSeconds"
export interface ClientPolicyDraft {
  autoAuthorize: boolean
  refreshToken: boolean
  accessTokenTtlSeconds: string
  refreshTokenTtlSeconds: string
  clientTokenTtlSeconds: string
}

export function validTokenSeconds(raw: string, refresh = false): boolean {
  if (raw === "") return true
  if (!/^[1-9]\d*$/.test(raw)) return false
  const seconds = Number(raw)
  return Number.isSafeInteger(seconds) && seconds >= (refresh ? 300 : 60) && seconds <= (refresh ? 604800 : 3600)
}

export function tokenSeconds(raw: string, refresh = false): number | null {
  if (!validTokenSeconds(raw, refresh)) throw new Error("令牌有效期超出允许范围")
  return raw === "" ? null : Number(raw)
}
