/** Personal routes are available to every authenticated context, without an app grant. */
export const PERSONAL_ROUTE_PATHS = ["/dashboard", "/profile", "/system/notifications"] as const

/** Host routes use the same permission codes as the server and seeded menus. */
const ROUTE_PERMISSIONS: Readonly<Record<string, string>> = {
  "/system/tenants": "system:tenant",
  "/system/settings": "system:settings",
  "/system/applications": "system:application",
  "/system/menus": "system:menu",
  "/system/organizations": "system:organization",
  "/system/departments": "system:department",
  "/system/authorizations": "system:authorization",
  "/system/data-grants": "system:data-grant:read",
  "/system/users": "system:user",
  "/system/customers": "system:user",
  "/system/roles": "system:role",
  "/system/regions": "system:region",
  "/system/oauth/clients": "system:oauth",
  "/system/oauth/tokens": "system:oauth",
  "/system/audit-logs": "system:audit",
  "/system/monitor": "system:monitor",
  "/athena/overview": "athena:overview",
  "/athena/reports/sales": "athena:report:sales",
  "/athena/reports/operations": "athena:report:operations",
  "/athena/sources": "athena:source",
  "/hermes/inbox": "hermes:inbox",
  "/hermes/notifications": "hermes:notification",
}

const PLATFORM_ROOT_PATHS = new Set([
  "/system/settings", "/system/applications", "/system/menus", "/system/oauth/clients", "/system/oauth/tokens",
])

export type RoutePolicy =
  | { kind: "redirect"; path: string; to: string }
  | { kind: "personal"; path: string }
  | { kind: "host"; path: string; permission: string; platformRootOnly: boolean }
  | { kind: "micro"; path: string; appCode: string }
  | { kind: "embed"; path: string }
  | { kind: "unknown"; path: string }

export function normalizedRoutePath(pathname: string): string {
  // React Router accepts case variations, encoded segments, and trailing slashes.
  // Authorization must use that same path identity so these cannot bypass a gate.
  let path = pathname
  try { path = decodeURIComponent(path) } catch { /* Invalid paths reach the 404 route. */ }
  return path.replace(/\/+$/, "").toLowerCase() || "/"
}

export function routePermission(pathname: string): string | undefined {
  return ROUTE_PERMISSIONS[normalizedRoutePath(pathname)]
}

/** Every protected route has an explicit policy; an unregistered path never inherits access. */
export function resolveRoutePolicy(pathname: string): RoutePolicy {
  const path = normalizedRoutePath(pathname)
  if (path === "/") return { kind: "redirect", path, to: "/dashboard" }
  if (PERSONAL_ROUTE_PATHS.some((personal) => personal === path)) return { kind: "personal", path }
  const permission = ROUTE_PERMISSIONS[path]
  if (permission) return { kind: "host", path, permission, platformRootOnly: PLATFORM_ROOT_PATHS.has(path) }
  if (path === "/embed") return { kind: "embed", path }
  const micro = /^\/micro\/([^/]+)(?:\/.*)?$/.exec(path)
  if (micro) return { kind: "micro", path, appCode: micro[1] }
  return { kind: "unknown", path }
}
