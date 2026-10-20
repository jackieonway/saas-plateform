import * as React from "react"
import { useLocation } from "react-router-dom"
import { AnimatePresence, motion } from "framer-motion"
import { ChevronDown, ChevronRight, ExternalLink, FolderOpen, PanelLeftClose, Search } from "lucide-react"
import { Input } from "@/components/ui/input"
import { Avatar, AvatarFallback } from "@/components/ui/avatar"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { AppIcon } from "@/lib/icons"
import { EmptyState } from "@/components/states"
import { openExternalLink } from "@/lib/external-link"
import { useApplicationNavigation } from "@/hooks/use-application-navigation"
import { invalidateApplicationSelection } from "@/lib/application-navigation"
import { routeMenuAccess, type RouteAuthorizationInput } from "@/lib/route-policy"
import { cn } from "@/lib/utils"
import type { AppBrief, MenuNode } from "@/api/types"

function readCollapsedMenus(storageKey: string | null): Set<string> {
  if (!storageKey) return new Set()
  try {
    const value: unknown = JSON.parse(localStorage.getItem(storageKey) ?? "[]")
    return new Set(Array.isArray(value) ? value.filter((id): id is string => typeof id === "string") : [])
  } catch { return new Set() }
}

function filterMenus(nodes: MenuNode[], keyword: string): MenuNode[] {
  if (!keyword.trim()) return nodes
  const kw = keyword.trim().toLowerCase()
  const walk = (list: MenuNode[]): MenuNode[] => {
    const out: MenuNode[] = []
    for (const n of list) {
      const hit =
        n.title.toLowerCase().includes(kw) ||
        (n.path ?? "").toLowerCase().includes(kw) ||
        (n.permission ?? "").toLowerCase().includes(kw)
      const subs = n.children ? walk(n.children) : []
      if (hit || subs.length > 0) out.push({ ...n, children: subs })
    }
    return out
  }
  return walk(nodes)
}

function authorizedMenus(nodes: MenuNode[], policy: RouteAuthorizationInput): MenuNode[] {
  return nodes.flatMap((node) => {
    if (node.visible === false || node.type === "BUTTON") return []
    const children = authorizedMenus(node.children ?? [], policy)
    if (children.length || routeMenuAccess(node, policy)) return [{ ...node, children }]
    return []
  })
}

/** 应用侧边栏(248px,深色):菜单树导航 */
export function AppSidebar({
  app,
  menus,
  isLoading,
  isError,
  onRetry,
  onCollapse,
  navigationScope,
  userDisplayName,
  className,
}: {
  app: AppBrief | null
  menus: MenuNode[]
  isLoading: boolean
  isError: boolean
  onRetry: () => void
  onCollapse?: () => void
  /** Persist directory preferences only within the same user/tenant/org/app. */
  navigationScope?: string
  userDisplayName: string
  className?: string
}) {
  const [keyword, setKeyword] = React.useState("")
  const { pathname } = useLocation()
  const { navigateTo: go, access, policyInput } = useApplicationNavigation()
  const filtered = filterMenus(authorizedMenus(menus, policyInput), keyword)
  const openMenu = (node: MenuNode) => {
    if (!routeMenuAccess(node, policyInput)) return
    invalidateApplicationSelection()
    if (!openExternalLink(node, go) && node.path) go(node.path)
  }

  return (
    <aside
      aria-label="菜单导航"
      className={cn(
        "flex w-[248px] shrink-0 flex-col border-r bg-sidebar text-sidebar-foreground",
        className
      )}
    >
      {/* 当前应用 */}
      <div className="flex items-center gap-3 border-b border-sidebar-border px-4 py-3.5">
        <div className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-sidebar-primary/15 text-sidebar-primary">
          <AppIcon name={app?.icon} size={18} />
        </div>
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold leading-tight">{app?.name ?? "未选择应用"}</p>
          <p className="truncate font-mono text-[11px] text-sidebar-foreground/50">{app?.code ?? "—"}</p>
        </div>
        {onCollapse && (
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                aria-label="收起菜单"
                aria-expanded="true"
                onClick={onCollapse}
                className="flex size-7 shrink-0 items-center justify-center rounded-md text-sidebar-foreground/60 outline-none transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground focus-visible:ring-2 focus-visible:ring-sidebar-ring"
              >
                <PanelLeftClose className="size-4" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="right" className="text-xs">收起菜单</TooltipContent>
          </Tooltip>
        )}
      </div>

      {/* 搜索 */}
      <div className="px-3 py-2.5">
        <div className="relative">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-sidebar-foreground/40" />
          <Input
            value={keyword}
            onChange={(e) => setKeyword(e.target.value)}
            placeholder="过滤菜单..."
            aria-label="过滤菜单"
            className="h-8 border-sidebar-border bg-sidebar-accent/40 pl-8 text-xs text-sidebar-foreground placeholder:text-sidebar-foreground/40 focus-visible:ring-sidebar-ring"
          />
        </div>
      </div>

      {/* 菜单树 */}
      <nav className="flex-1 overflow-y-auto scroll-area px-2 pb-2">
        {isLoading ? (
          <div className="space-y-1.5 px-2 py-2">
            {Array.from({ length: 8 }).map((_, i) => (
              <div key={i} className="h-8 rounded-md bg-sidebar-accent/40" style={{ opacity: 1 - i * 0.09 }} />
            ))}
          </div>
        ) : isError ? (
          <div className="px-2 py-6 text-center text-xs text-sidebar-foreground/60">
            菜单加载失败
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 block w-full rounded-md border border-sidebar-border py-1.5 hover:bg-sidebar-accent"
            >
              重试
            </button>
          </div>
        ) : filtered.length === 0 ? (
          <EmptyState
            className="py-10 text-sidebar-foreground/70 [&_p]:text-inherit"
            title={keyword ? "无匹配菜单" : "该应用暂无菜单"}
            description={keyword ? undefined : "可先在「菜单管理」中为其配置菜单"}
            icon={<FolderOpen className="size-5" />}
            action={
              !keyword && access("/system/menus").status === "allowed" && (
                <button
                  type="button"
                  onClick={() => go("/system/menus")}
                  className="rounded-md border border-sidebar-border px-3 py-1.5 text-xs hover:bg-sidebar-accent"
                >
                  去配置菜单
                </button>
              )
            }
          />
        ) : (
          <MenuList key={navigationScope ?? "unscoped"} nodes={filtered} pathname={pathname} onNavigate={openMenu}
            forceExpanded={!!keyword.trim()} storageKey={navigationScope ? `platform-menu-folds:${navigationScope}` : null} />
        )}
      </nav>

      {/* 用户小条 */}
      <div className="flex items-center gap-2.5 border-t border-sidebar-border px-3 py-2.5">
        <Avatar className="size-7 border border-sidebar-border">
          <AvatarFallback className="bg-sidebar-primary/15 text-[11px] font-semibold text-sidebar-primary">
            {(userDisplayName || "?").slice(0, 1).toUpperCase()}
          </AvatarFallback>
        </Avatar>
        <span className="truncate text-xs text-sidebar-foreground/80">{userDisplayName}</span>
      </div>
    </aside>
  )
}

function MenuList({
  nodes,
  pathname,
  onNavigate,
  forceExpanded,
  storageKey,
}: {
  nodes: MenuNode[]
  pathname: string
  onNavigate: (node: MenuNode) => void
  forceExpanded: boolean
  storageKey: string | null
}) {
  const [collapsed, setCollapsed] = React.useState(() => readCollapsedMenus(storageKey))
  React.useEffect(() => {
    if (!storageKey) return
    const sync = (event: Event) => {
      if (event instanceof StorageEvent ? event.key === storageKey || event.key === null
        : (event as CustomEvent<string>).detail === storageKey) {
        setCollapsed(readCollapsedMenus(storageKey))
      }
    }
    window.addEventListener("storage", sync)
    window.addEventListener("platform-menu-folds-change", sync)
    return () => {
      window.removeEventListener("storage", sync)
      window.removeEventListener("platform-menu-folds-change", sync)
    }
  }, [storageKey])
  const toggle = (id: string) => {
    const next = new Set(collapsed)
    if (next.has(id)) next.delete(id)
    else next.add(id)
    setCollapsed(next)
    try {
      if (storageKey) {
        localStorage.setItem(storageKey, JSON.stringify([...next]))
        window.dispatchEvent(new CustomEvent("platform-menu-folds-change", { detail: storageKey }))
      }
    }
    catch { /* Keep navigation usable when browser storage is unavailable. */ }
  }
  return (
    <ul className="space-y-0.5" role="list">
      {nodes
        .filter((n) => n.type !== "BUTTON")
        .map((node) => (
          <MenuItem key={node.id} node={node} pathname={pathname} onNavigate={onNavigate} depth={0}
            collapsed={collapsed} onToggle={toggle} forceExpanded={forceExpanded} />
        ))}
    </ul>
  )
}

function MenuItem({
  node,
  pathname,
  onNavigate,
  depth,
  collapsed,
  onToggle,
  forceExpanded,
}: {
  node: MenuNode
  pathname: string
  onNavigate: (node: MenuNode) => void
  depth: number
  collapsed: Set<string>
  onToggle: (id: string) => void
  forceExpanded: boolean
}) {
  const open = forceExpanded || !collapsed.has(node.id)
  const childMenus = (node.children ?? []).filter((c) => c.type !== "BUTTON")
  const isDir = node.type === "DIR"
  // 外链菜单无路由态, active 恒 false
  const active = !node.linkUrl && !!node.path && node.path === pathname

  return (
    <li>
      {isDir || childMenus.length > 0 ? (
        <>
          <button
            type="button"
            aria-expanded={open}
            disabled={forceExpanded}
            onClick={() => onToggle(node.id)}
            className="group flex w-full items-center gap-2 rounded-md px-2 py-2 text-left text-sm text-sidebar-foreground/80 transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
            style={{ paddingLeft: 8 + depth * 14 }}
          >
            {open ? (
              <ChevronDown className="size-3.5 shrink-0 text-sidebar-foreground/40" />
            ) : (
              <ChevronRight className="size-3.5 shrink-0 text-sidebar-foreground/40" />
            )}
            <AppIcon name={node.icon} className="size-4 shrink-0 text-sidebar-foreground/60" size={16} />
            <span className="truncate">{node.title}</span>
            {node.linkUrl && (
              <ExternalLink className="size-3 shrink-0 text-muted-foreground" aria-label="外链" />
            )}
          </button>
          <AnimatePresence initial={false}>
            {open && childMenus.length > 0 && (
              <motion.div
                key="content"
                initial={{ height: 0, opacity: 0 }}
                animate={{ height: "auto", opacity: 1 }}
                exit={{ height: 0, opacity: 0 }}
                transition={{ duration: 0.18, ease: "easeInOut" }}
                className="overflow-hidden"
              >
                <ul role="list" className="ml-2 border-l border-sidebar-border/70 pl-1">
                  {childMenus.map((c) => (
                    <MenuItem
                      key={c.id}
                      node={c}
                      pathname={pathname}
                      onNavigate={onNavigate}
                      depth={depth + 1}
                      collapsed={collapsed}
                      onToggle={onToggle}
                      forceExpanded={forceExpanded}
                    />
                  ))}
                </ul>
              </motion.div>
            )}
          </AnimatePresence>
        </>
      ) : (
        <button
          type="button"
          aria-current={active ? "page" : undefined}
          onClick={() => {
            onNavigate(node)
          }}
          className={cn(
            "relative flex w-full items-center gap-2 rounded-md px-2 py-2 text-left text-sm transition-colors",
            active
              ? "bg-sidebar-primary/15 font-medium text-sidebar-primary"
              : "text-sidebar-foreground/80 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
          )}
          style={{ paddingLeft: 8 + depth * 14 }}
        >
          {active && (
            <motion.span
              layoutId="sidebar-active"
              className="absolute left-0 top-1/2 h-6 w-[3px] -translate-y-1/2 rounded-r-full bg-sidebar-primary"
            />
          )}
          <AppIcon name={node.icon} className={cn("size-4 shrink-0", active ? "text-sidebar-primary" : "text-sidebar-foreground/60")} size={16} />
          <span className="truncate">{node.title}</span>
          {node.linkUrl && (
            <ExternalLink className="size-3 shrink-0 text-muted-foreground" aria-label="外链" />
          )}
        </button>
      )}
    </li>
  )
}
