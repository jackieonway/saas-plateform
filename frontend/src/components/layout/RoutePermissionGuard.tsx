import type { ReactNode } from "react"
import { Navigate, useLocation } from "react-router-dom"
import { Loader2, ShieldAlert } from "lucide-react"
import { EmptyState, ErrorState } from "@/components/states"
import { useApplicationMenusQueries, usePermissionsQuery } from "@/hooks/use-auth-data"
import { resolveRoutePolicy } from "@/lib/route-permissions"
import { evaluateRouteAccess } from "@/lib/route-policy"
import { useAuthStore } from "@/stores/auth"

function PermissionLoading() {
  return (
    <div className="flex min-h-[50vh] items-center justify-center gap-2 text-sm text-muted-foreground"
      role="status" aria-label="正在验证访问权限">
      <Loader2 className="size-4 animate-spin" />
      正在验证访问权限...
    </div>
  )
}

function PermissionDenied() {
  return (
    <EmptyState className="min-h-[50vh]" title="无访问权限"
      description="当前租户或组织未授权访问此页面，请联系管理员。" icon={<ShieldAlert className="size-6" />} />
  )
}

/** The same typed decision filters navigation and gates page mounting before business requests. */
export function RoutePermissionGuard({ children }: { children: ReactNode }) {
  const { pathname, search } = useLocation()
  const policy = resolveRoutePolicy(pathname)
  const profile = useAuthStore((state) => state.profile)
  const currentApp = policy.kind === "micro"
    ? profile?.applications.find((app) => app.code.toLowerCase() === policy.appCode) : undefined
  const menuEnabled = policy.kind === "host" || policy.kind === "embed" || policy.kind === "micro"
  const menus = useApplicationMenusQueries({ enabled: menuEnabled,
    applicationIds: policy.kind === "micro" ? (currentApp ? [currentApp.id] : []) : undefined })
  const permissions = usePermissionsQuery({ enabled: policy.kind === "host" })
  const access = evaluateRouteAccess({ pathname, search, profile: menus.profile, switching: menus.switching,
    permissions, applicationMenus: menus.queries })

  if (access.status === "allowed") return children
  if (access.status === "redirect") return <Navigate to={access.to} replace />
  if (access.status === "pending") return <PermissionLoading />
  if (access.status === "error") return <ErrorState message="访问权限验证失败，请重试" onRetry={() => {
    if (policy.kind === "host") void permissions.refetch()
    for (const query of menus.queries) void query.refetch()
  }} />
  return <PermissionDenied />
}
