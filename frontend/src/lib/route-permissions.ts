/** Host routes use the same permission codes as the server and seeded menus. */
const ROUTE_PERMISSIONS: Readonly<Record<string, string>> = {
  "/dashboard": "dashboard:view",
  "/system/tenants": "system:tenant",
  "/system/settings": "system:settings",
  "/system/applications": "system:application",
  "/system/menus": "system:menu",
  "/system/organizations": "system:organization",
  "/system/departments": "system:department",
  "/system/authorizations": "system:authorization",
  "/system/users": "system:user",
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

export function normalizedRoutePath(pathname: string): string {
  // React Router accepts case variations, encoded segments, and trailing slashes.
  // Authorization must use that same path identity so these cannot bypass a gate.
  let path = pathname
  try { path = decodeURIComponent(path) } catch { /* Invalid paths reach the 404 route. */ }
  return path.replace(/\/+$/, "").toLowerCase()
}

export function routePermission(pathname: string): string | undefined {
  return ROUTE_PERMISSIONS[normalizedRoutePath(pathname)]
}
