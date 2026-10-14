import { matchPath } from "react-router-dom"
import { normalizedRoutePath } from "@/lib/route-permissions"

interface RouteMenu {
  type: string
  path: string | null
  linkUrl?: string | null
  children?: readonly RouteMenu[]
}

/** Exact menu routes, or explicitly configured :parameters/* patterns, grant access.
 * A base/parent menu never implicitly grants an unlisted descendant page.
 * Detail pages need their own authorized MENU, for example an invisible
 * /micro/work-orders/tickets/:id node with the corresponding read permission.
 * Configure creation pages separately when they require a different permission. */
export function microRouteAccess(menus: readonly RouteMenu[], appCode: string, pathname: string): {
  allowed: boolean
  redirectTo?: string
} {
  const basePath = normalizedRoutePath(`/micro/${appCode}`)
  const path = normalizedRoutePath(pathname)
  if (path !== basePath && !path.startsWith(`${basePath}/`)) return { allowed: false }
  const routes: { pattern: string; path: string }[] = []
  const collect = (nodes: readonly RouteMenu[]) => {
    for (const menu of nodes) {
      if (menu.type === "MENU" && menu.path && !menu.linkUrl) {
        const pattern = normalizedRoutePath(menu.path)
        if (pattern === basePath || pattern.startsWith(`${basePath}/`)) routes.push({ pattern, path: menu.path })
      }
      if (menu.children?.length) collect(menu.children)
    }
  }
  collect(menus)
  if (routes.some((route) => matchPath({ path: route.pattern, end: true }, path))) return { allowed: true }
  if (path === basePath) {
    const first = routes.find((route) => route.pattern !== basePath && !/[:*]/.test(route.pattern))
    if (first) return { allowed: false, redirectTo: first.path }
  }
  return { allowed: false }
}
