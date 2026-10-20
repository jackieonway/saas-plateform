import type { MenuNode, PermissionsResp, ProfileResp } from "@/api/types"
import { hasPlatformContext } from "@/lib/platform-context"
import { microRouteAccess } from "@/lib/micro-route-permissions"
import { normalizedRoutePath, PERSONAL_ROUTE_PATHS, resolveRoutePolicy } from "@/lib/route-permissions"

export interface AuthorizationQuery<T> {
  status: "pending" | "success" | "error"
  data?: T
}

export interface ApplicationMenuEvidence extends AuthorizationQuery<readonly MenuNode[]> {
  applicationId: string
}

export interface RouteAuthorizationInput {
  profile: ProfileResp | null
  switching?: boolean
  permissions: AuthorizationQuery<PermissionsResp>
  applicationMenus: readonly ApplicationMenuEvidence[]
}

export type RouteAccess =
  | { status: "allowed" | "denied" | "pending" | "error" }
  | { status: "redirect"; to: string }

export function externalMenuUrl(raw: string | null | undefined): string | null {
  try {
    const url = new URL(raw?.trim() ?? "")
    return ["http:", "https:"].includes(url.protocol) ? url.href : null
  } catch { return null }
}

function someMenu(nodes: readonly MenuNode[], predicate: (menu: MenuNode) => boolean): boolean {
  return nodes.some((menu) => predicate(menu) || (!!menu.children?.length && someMenu(menu.children, predicate)))
}

function successfulMenus(input: RouteAuthorizationInput): ApplicationMenuEvidence[] {
  const appIds = new Set(input.profile?.applications.map((app) => app.id) ?? [])
  return input.applicationMenus.filter((query) => appIds.has(query.applicationId) && query.status === "success")
}

function absentMenuAccess(queries: readonly ApplicationMenuEvidence[]): RouteAccess {
  if (queries.some((query) => query.status === "pending")) return { status: "pending" }
  if (queries.some((query) => query.status === "error")) return { status: "error" }
  return { status: "denied" }
}

/** Shared by route mounting and every displayed destination. Only positive evidence grants access. */
export function evaluateRouteAccess(input: RouteAuthorizationInput & { pathname: string; search?: string }): RouteAccess {
  const policy = resolveRoutePolicy(input.pathname)
  if (policy.kind === "unknown") return { status: "denied" }
  if (!input.profile || input.switching) return { status: "pending" }
  if (policy.kind === "redirect") return { status: "redirect", to: policy.to }
  if (policy.kind === "personal") return { status: "allowed" }
  const grantedIds = new Set(input.profile.applications.map((app) => app.id))
  const queries = input.applicationMenus.filter((query) => grantedIds.has(query.applicationId))
  if (policy.kind === "micro") {
    const app = input.profile.applications.find((candidate) => candidate.code.toLowerCase() === policy.appCode)
    if (!app) return { status: "denied" }
    const query = queries.find((candidate) => candidate.applicationId === app.id)
    if (!query) return { status: "pending" }
    if (query.status !== "success") return { status: query.status }
    const access = microRouteAccess(query.data ?? [], app.code, input.pathname)
    if (access.redirectTo) return { status: "redirect", to: access.redirectTo }
    return { status: access.allowed ? "allowed" : "denied" }
  }
  if (policy.kind === "host") {
    if (policy.platformRootOnly && !hasPlatformContext(input.profile)) return { status: "denied" }
    if (input.permissions.status !== "success") return { status: input.permissions.status }
    if (!input.permissions.data?.permissions.includes(policy.permission)) return { status: "denied" }
    if (successfulMenus(input).some((query) => someMenu(query.data ?? [], (menu) =>
      menu.type === "MENU" && !menu.linkUrl && !!menu.path && normalizedRoutePath(menu.path) === policy.path))) {
      return { status: "allowed" }
    }
    return absentMenuAccess(queries)
  }
  const url = externalMenuUrl(new URLSearchParams(input.search ?? "").get("url"))
  if (!url) return { status: "denied" }
  if (successfulMenus(input).some((query) => someMenu(query.data ?? [], (menu) =>
    menu.type === "MENU" && externalMenuUrl(menu.linkUrl) === url))) return { status: "allowed" }
  return absentMenuAccess(queries)
}

/** Both SELF and BLANK external links require the same explicit returned URL MENU. */
export function routeMenuAccess(menu: MenuNode, input: RouteAuthorizationInput): boolean {
  if (menu.type !== "MENU") return false
  if (menu.linkUrl) {
    const url = externalMenuUrl(menu.linkUrl)
    return !!url && evaluateRouteAccess({ ...input, pathname: "/embed", search: new URLSearchParams({ url }).toString() }).status === "allowed"
  }
  return !!menu.path && evaluateRouteAccess({ ...input, pathname: menu.path }).status === "allowed"
}

/** Compatibility path set for exact destinations; access() also supports dynamic and external routes. */
export function collectAuthorizedPaths(input: RouteAuthorizationInput): Set<string> {
  const paths = new Set<string>()
  for (const path of PERSONAL_ROUTE_PATHS) {
    if (evaluateRouteAccess({ ...input, pathname: path }).status === "allowed") paths.add(path)
  }
  const collect = (nodes: readonly MenuNode[]) => {
    for (const node of nodes) {
      if (!node.linkUrl && node.path && routeMenuAccess(node, input)) paths.add(normalizedRoutePath(node.path))
      if (node.children?.length) collect(node.children)
    }
  }
  for (const query of successfulMenus(input)) collect(query.data ?? [])
  return paths
}
