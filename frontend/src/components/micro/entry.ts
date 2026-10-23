import type { AppAuthorizationContext, ApplicationEntryInfo } from "./types"

type ApplicationIdentity = Pick<ApplicationEntryInfo, "id" | "code" | "name" | "icon">
const invalidEntry = () => Object.assign(new Error("应用入口契约无效，请刷新配置后重试"), { code: "INVALID_APPLICATION_ENTRY" })
const reconfigureEntry = () => Object.assign(new Error("平台已统一使用 qiankun，请管理员将此应用重新配置为 HTML 入口与可信运行模式"),
  { code: "MICRO_ENTRY_RECONFIGURATION_REQUIRED" })

/** Profile discovers the app; all executable entry fields come from this one validated context. */
export function resolveMicroEntry(context: AppAuthorizationContext, identity: ApplicationIdentity): ApplicationEntryInfo {
  const app = context?.application
  if (!app || app.id !== identity.id || context.appCode !== identity.code) throw invalidEntry()
  if (app.runtimeMode !== "TRUSTED" || app.entryType !== "HTML") throw reconfigureEntry()
  if (typeof app.entryUrl !== "string" || !app.entryUrl.trim()) throw invalidEntry()
  const entryUrl = app.entryUrl.trim()
  try {
    const url = new URL(entryUrl, "https://host.invalid/")
    if (!/^https?:$/.test(url.protocol) || url.username || url.password ||
        [...url.searchParams.keys()].some(key => /^(?:token|access_token|session(?:_token)?|client_secret)$/i.test(key))) throw invalidEntry()
  } catch { throw invalidEntry() }
  const version = app.deploymentVersion ?? null, releaseId = app.entryVersionId ?? null
  if ((version !== null && (typeof version !== "string" || !version)) ||
      (releaseId !== null && (typeof releaseId !== "string" || !releaseId))) throw invalidEntry()
  return { id: identity.id, code: identity.code, name: identity.name, icon: identity.icon,
    runtimeMode: app.runtimeMode, entryType: app.entryType, entryUrl,
    deploymentVersion: version, entryVersionId: releaseId }
}

/** In-memory comparison only. URLs are never logged as authorization diagnostics. */
export function microEntrySignature(entry: ApplicationEntryInfo): string {
  return JSON.stringify([entry.runtimeMode, entry.entryType, entry.entryUrl, entry.deploymentVersion ?? null, entry.entryVersionId ?? null])
}

/** Recheck after every async load boundary and before props are built, without waiting for React cleanup. */
export function assertMicroEntrySnapshot(captured: ApplicationEntryInfo, context: AppAuthorizationContext): void {
  let current: ApplicationEntryInfo
  try { current = resolveMicroEntry(context, captured) }
  catch { throw new DOMException("Application entry snapshot changed", "AbortError") }
  if (microEntrySignature(captured) !== microEntrySignature(current)) {
    throw new DOMException("Application entry snapshot changed", "AbortError")
  }
}
