export type AuthenticationMode = "BFF"

/** The server chooses the protocol before any durable legacy credential is read.
 * An unavailable BFF never authorizes a password or refresh-token fallback. */
export async function loadAuthenticationMode(): Promise<AuthenticationMode> {
  const controller = new AbortController()
  const deadline = setTimeout(() => controller.abort(), 15_000)
  try {
    const response = await fetch("/auth/config", {
      credentials: "same-origin", cache: "no-store", redirect: "error", signal: controller.signal,
      headers: { Accept: "application/json" },
    })
    if (!response.ok) throw new Error("认证服务暂时不可用，请稍后重新检查")
    const value: unknown = await response.json()
    if (!value || typeof value !== "object" || !("mode" in value) || value.mode !== "BFF") {
      throw new Error("认证方式无法确认，请稍后重新检查")
    }
    return value.mode
  } catch {
    throw new Error("认证服务暂时不可用，请稍后重新检查。当前页面不会切换到其他登录方式。")
  } finally { clearTimeout(deadline) }
}

/** Only known console navigation paths can survive a full authentication redirect.
 * OAuth callbacks, API paths and arbitrary external return URLs are excluded. */
export function safePlatformReturn(value: string | null | undefined): string {
  if (!value || !value.startsWith("/") || value.startsWith("//") || value.includes("\\") || value.includes("#")
    || [...value].some(character => character.charCodeAt(0) <= 0x20 || character.charCodeAt(0) === 0x7f)) return "/dashboard"
  const pathname = value.split("?", 1)[0]
  let decoded: string
  try { decoded = decodeURIComponent(pathname) } catch { return "/dashboard" }
  if (!/^(?:\/|\/(?:dashboard|profile|embed)\/?|\/(?:system|athena|hermes|micro)(?:\/[A-Za-z0-9_-]+)*\/?)$/.test(decoded)) return "/dashboard"
  return value
}

export function startUnifiedLogin(returnTo: string): void {
  window.location.assign(`/auth/login?return=${encodeURIComponent(safePlatformReturn(returnTo))}`)
}
