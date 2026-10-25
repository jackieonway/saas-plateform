import * as React from "react"
import { useNavigate } from "react-router-dom"
import { useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"
import { useAuthStore } from "@/stores/auth"
import { useAuthorizedPaths } from "@/hooks/use-authorized-paths"
import { authMenusQueryOptions, authPermissionsQueryOptions, authQueryKeys, captureAuthSession } from "@/lib/auth-queries"
import { applicationNavigationSequence, createApplicationNavigation, invalidateApplicationSelection } from "@/lib/application-navigation"
import { evaluateRouteAccess, type RouteAuthorizationInput } from "@/lib/route-policy"
import { resolveRoutePolicy } from "@/lib/route-permissions"
import type { MenuNode, PermissionsResp } from "@/api/types"

function currentPolicy(queryClient: ReturnType<typeof useQueryClient>): RouteAuthorizationInput {
  const state = useAuthStore.getState()
  const session = captureAuthSession()
  const profile = session.profileReady ? state.profile : null
  return {
    profile,
    switching: state.contextSwitching || !session.profileReady,
    permissions: {
      status: queryClient.getQueryState(authQueryKeys.permissions(session.namespace, session.epoch))?.status ?? "pending",
      data: queryClient.getQueryData<PermissionsResp>(authQueryKeys.permissions(session.namespace, session.epoch)),
    },
    applicationMenus: (profile?.applications ?? []).map((app) => ({
      applicationId: app.id,
      status: queryClient.getQueryState(authQueryKeys.menus(app.id, session.namespace, session.epoch))?.status ?? "pending",
      data: queryClient.getQueryData<MenuNode[]>(authQueryKeys.menus(app.id, session.namespace, session.epoch)),
    })),
  }
}

/** Shared application-selection transaction and policy-checked direct navigation. */
export function useApplicationNavigation(options: { paths?: readonly string[] } = {}) {
  const routerNavigate = useNavigate()
  const queryClient = useQueryClient()
  // Observe cached authorization; selection itself only requests the selected app.
  const applications = useAuthStore((state) => state.profile?.applications)
  const applicationIds = options.paths === undefined ? undefined : (applications ?? []).filter((app) =>
    options.paths!.some((path) => app.basePath !== "/" &&
      (path === app.basePath || path.startsWith(`${app.basePath}/`)))).map((app) => app.id)
  const authorization = useAuthorizedPaths({ enabled: applicationIds !== undefined, applicationIds })
  const selectApplication = React.useMemo(() => createApplicationNavigation({
    session: () => {
      const state = useAuthStore.getState()
      const session = captureAuthSession()
      return { token: state.token, epoch: state.contextEpoch, switching: state.contextSwitching,
        identity: session.namespace,
        currentApplicationId: state.currentApplicationId, applications: session.profileReady ? state.profile?.applications ?? [] : [] }
    },
    select: (id) => useAuthStore.getState().setCurrentApplication(id),
    menus: async (id) => {
      const session = captureAuthSession()
      const menus = await queryClient.fetchQuery(authMenusQueryOptions(id, session))
      const needsPermissions = (nodes: readonly MenuNode[]): boolean => nodes.some((node) =>
        (node.type === "MENU" && !node.linkUrl && !!node.path && resolveRoutePolicy(node.path).kind === "host") ||
        needsPermissions(node.children ?? []))
      if (needsPermissions(menus)) {
        // Only host pages depend on this evidence. A micro app's explicit MENU
        // remains usable when the unrelated host-permissions endpoint fails.
        await queryClient.fetchQuery(authPermissionsQueryOptions(session)).catch(() => undefined)
      }
      return menus
    },
    allowed: (path) => evaluateRouteAccess({ ...currentPolicy(queryClient), pathname: path }).status === "allowed",
    navigate: routerNavigate,
    failed: () => toast.error("应用菜单获取失败，请重试"),
  }, applicationNavigationSequence), [queryClient, routerNavigate])

  const navigateTo = React.useCallback((path: string) => {
    invalidateApplicationSelection()
    if (!path.startsWith("/") || path.startsWith("//")) return false
    const destination = new URL(path, window.location.origin)
    if (destination.origin !== window.location.origin) return false
    const decision = evaluateRouteAccess({ ...currentPolicy(queryClient), pathname: destination.pathname, search: destination.search })
    const target = decision.status === "redirect" ? decision.to : `${destination.pathname}${destination.search}${destination.hash}`
    if (decision.status !== "allowed" && (decision.status !== "redirect" ||
        evaluateRouteAccess({ ...currentPolicy(queryClient), pathname: target }).status !== "allowed")) return false
    const state = useAuthStore.getState()
    const app = state.profile?.applications.find((candidate) => candidate.basePath !== "/" &&
      (destination.pathname === candidate.basePath || destination.pathname.startsWith(`${candidate.basePath}/`)))
    if (app && app.id !== state.currentApplicationId) state.setCurrentApplication(app.id)
    routerNavigate(target)
    return true
  }, [queryClient, routerNavigate])

  return { ...authorization, selectApplication, navigateTo }
}
