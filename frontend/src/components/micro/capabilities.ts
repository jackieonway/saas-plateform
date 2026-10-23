import { detectBrowserCapabilities } from "../../../vendor/micro-app-sdk/index.mjs"

/** qiankun's DOM sandbox requirements complement the SDK's request capabilities. */
export function assertQiankunCapabilities(environment: typeof globalThis = globalThis) {
  const checks: [string, (value: typeof globalThis) => boolean][] = [
    ["Proxy", value => typeof value.Proxy === "function"],
    ["Request", value => typeof value.Request === "function"],
    ["DOMParser", value => typeof value.DOMParser === "function"],
    ["MutationObserver", value => typeof value.MutationObserver === "function"],
    ["AbortSignal.any", value => typeof value.AbortSignal?.any === "function"],
    ["AbortSignal.timeout", value => typeof value.AbortSignal?.timeout === "function"],
    ["document.createElement", value => typeof value.document?.createElement === "function"],
    ["Blob", value => typeof value.Blob === "function"],
    ["URL.createObjectURL", value => typeof value.URL?.createObjectURL === "function"],
    ["URL.revokeObjectURL", value => typeof value.URL?.revokeObjectURL === "function"],
  ]
  const missing = [...detectBrowserCapabilities("client", environment).missing, ...checks.filter(([, check]) => {
    try { return !check(environment) } catch { return true }
  }).map(([name]) => name)]
  if (missing.length) throw Object.assign(new Error("Required browser capabilities are unavailable: " + missing.join(", ")),
    { code: "BROWSER_CAPABILITIES_UNSUPPORTED", missing })
  return { supported: true as const, mode: "qiankun" as const }
}
