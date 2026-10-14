import type { ReactNode } from "react"
import { useLocation } from "react-router-dom"
import { Loader2, ShieldAlert } from "lucide-react"
import { useQueries } from "@tanstack/react-query"
import { EmptyState, ErrorState } from "@/components/states"
import { usePermissionsQuery } from "@/hooks/use-auth-data"
import { normalizedRoutePath, routePermission } from "@/lib/route-permissions"
import { useAuthStore } from "@/stores/auth"
import { authApi } from "@/api/auth"
import { asArray } from "@/lib/api"
import type { MenuNode } from "@/api/types"

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
    <EmptyState
      className="min-h-[50vh]"
      title="无访问权限"
      description="当前租户或组织未授权访问此页面，请联系管理员。"
      icon={<ShieldAlert className="size-6" />}
    />
  )
}

function externalUrl(raw: string | null | undefined): string | null {
  try {
    const url = new URL(raw?.trim() ?? "")
    return ["http:", "https:"].includes(url.protocol) ? url.href : null
  } catch { return null }
}

function hasExternalMenu(nodes: MenuNode[], url: string): boolean {
  return nodes.some((node) =>
    (node.type !== "BUTTON" && externalUrl(node.linkUrl) === url) ||
    (!!node.children?.length && hasExternalMenu(node.children, url)))
}

/** External URLs also need a menu returned by the current authorized context. */
function EmbedPermissionGuard({ children }: { children: ReactNode }) {
  const { search } = useLocation()
  const token = useAuthStore((s) => s.token)
  const profile = useAuthStore((s) => s.profile)
  const contextEpoch = useAuthStore((s) => s.contextEpoch)
  const switching = useAuthStore((s) => s.contextSwitching)
  const url = externalUrl(new URLSearchParams(search).get("url"))
  const menusQueries = useQueries({
    queries: (profile?.applications ?? []).map((app) => ({
      queryKey: ["auth-menus", app.id, contextEpoch],
      queryFn: async ({ signal }: { signal: AbortSignal }) =>
        asArray<MenuNode>(await authApi.menus(app.id, token!, signal), "menus"),
      enabled: !!token && !switching && !!url,
      staleTime: 20_000,
      refetchInterval: 20_000,
      retry: 1,
    })),
  })

  // The existing embed page presents its own missing/invalid URL explanation.
  if (!url) return children
  if (switching || !profile || menusQueries.some((q) => q.isPending)) return <PermissionLoading />
  if (menusQueries.some((q) => q.isError)) {
    return <ErrorState message="访问权限验证失败，请重试" onRetry={() => {
      for (const query of menusQueries) void query.refetch()
    }} />
  }
  if (!menusQueries.some((q) => q.data && hasExternalMenu(q.data, url))) return <PermissionDenied />
  return children
}

/** Check effective permissions before a page mounts or requests business data. */
export function RoutePermissionGuard({ children }: { children: ReactNode }) {
  const { pathname } = useLocation()
  const requiredPermission = routePermission(pathname)
  const permissionsQuery = usePermissionsQuery()

  if (normalizedRoutePath(pathname) === "/embed") {
    return <EmbedPermissionGuard>{children}</EmbedPermissionGuard>
  }

  // Personal profile/inbox routes have no application grant. Micro applications
  // validate their authorized application and live context in their own container.
  if (!requiredPermission) return children

  if (permissionsQuery.isError) {
    return (
      <ErrorState
        message="访问权限验证失败，请重试"
        onRetry={() => permissionsQuery.refetch()}
      />
    )
  }

  if (!permissionsQuery.isSuccess) {
    return <PermissionLoading />
  }

  if (!permissionsQuery.data.permissions.includes(requiredPermission)) {
    return <PermissionDenied />
  }

  return children
}
