import type { AppBrief, MenuNode } from "@/api/types"

export interface NavigationSession {
  token: string | null
  epoch: number
  switching: boolean
  identity: string | null
  currentApplicationId: string | null
  applications: readonly AppBrief[]
}

export function createNavigationSequence() {
  let generation = 0
  return { invalidate: () => ++generation, current: (value: number) => generation === value }
}

// Shared across the rail, dashboard and direct navigation, including separate hook instances.
export const applicationNavigationSequence = createNavigationSequence()
export const invalidateApplicationSelection = () => applicationNavigationSequence.invalidate()

/** Only concrete MENU destinations can be application landing pages. */
export function firstApplicationPath(nodes: readonly MenuNode[], allowed: (path: string) => boolean): string | null {
  for (const node of nodes) {
    if (node.visible === false) continue
    if (node.type === "MENU" && node.path && !node.linkUrl &&
        !node.path.split("/").some((part) => part.startsWith(":") || part.includes("*")) && allowed(node.path)) {
      return node.path
    }
    const child = firstApplicationPath(node.children ?? [], allowed)
    if (child) return child
  }
  return null
}

interface NavigationDependencies {
  session: () => NavigationSession
  select: (appId: string) => void
  menus: (appId: string, session: NavigationSession) => Promise<readonly MenuNode[]>
  allowed: (path: string, menus: readonly MenuNode[], session: NavigationSession) => boolean
  navigate: (path: string) => void
  failed: () => void
}

/** One selection transaction; late success and failure are both inert after any newer intent. */
export function createApplicationNavigation(
  dependencies: NavigationDependencies,
  sequence = createNavigationSequence(),
) {
  return async (appId: string, currentPath: string) => {
    const generation = sequence.invalidate()
    const selected = dependencies.session()
    const app = selected.applications.find((candidate) => candidate.id === appId)
    if (!selected.token || selected.switching || !app) return
    dependencies.select(appId)
    const current = () => {
      const next = dependencies.session()
      return sequence.current(generation) && !next.switching && next.epoch === selected.epoch &&
        !!next.token && next.identity === selected.identity && next.currentApplicationId === appId &&
        next.applications.some((candidate) => candidate.id === appId && candidate.basePath === app.basePath)
    }
    if (app.basePath !== "/" && (currentPath === app.basePath || currentPath.startsWith(`${app.basePath}/`))) return
    try {
      const menus = await dependencies.menus(appId, selected)
      if (!current()) return
      const path = firstApplicationPath(menus, (candidate) => dependencies.allowed(candidate, menus, selected))
      if (path && current()) dependencies.navigate(path)
    } catch {
      if (current()) dependencies.failed()
    }
  }
}
