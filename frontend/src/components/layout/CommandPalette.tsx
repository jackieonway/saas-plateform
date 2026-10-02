import * as React from "react"
import { useNavigate } from "react-router-dom"
import { useQuery, useQueries, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"
import {
  Bell,
  Building2,
  CircleCheck,
  ExternalLink,
  History,
  Info,
  LogOut,
  Moon,
  Network,
  Shield,
  ShieldAlert,
  Sun,
  TriangleAlert,
  User,
  type LucideIcon,
} from "lucide-react"
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "@/components/ui/command"
import { useAuthStore } from "@/stores/auth"
import { useAuthorizedPaths } from "@/hooks/use-authorized-paths"
import { notificationRequestOptions, useNotificationContext } from "@/hooks/use-notification-context"
import { useTheme } from "@/components/theme-provider"
import { authApi } from "@/api/auth"
import { searchEntities, EMPTY_SEARCH_RESULT, type GlobalSearchResult } from "@/api/search"
import { notificationsApi } from "@/api/notifications"
import type { NotificationRecord, NotificationType } from "@/api/types"
import { asArray } from "@/lib/api"
import { sortMenuTree } from "@/lib/tree"
import { openExternalLink } from "@/lib/external-link"
import { AppIcon } from "@/lib/icons"
import { ROUTE_TITLES } from "@/lib/routes"
import { formatDateTime, formatRelative, useDebounced } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { MenuNode } from "@/api/types"

/* ------------------------------ 最近搜索 ------------------------------ */

const RECENT_SEARCHES_KEY = "dante-recent-searches"
const MAX_RECENT_SEARCHES = 6

function readRecentSearches(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_SEARCHES_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.filter((s): s is string => typeof s === "string").slice(0, MAX_RECENT_SEARCHES)
  } catch {
    return []
  }
}

function pushRecentSearch(term: string) {
  const t = term.trim()
  if (!t) return
  const rest = readRecentSearches().filter((s) => s !== t)
  const next = [t, ...rest].slice(0, MAX_RECENT_SEARCHES)
  try {
    localStorage.setItem(RECENT_SEARCHES_KEY, JSON.stringify(next))
  } catch {
    /* ignore */
  }
}

/* ------------------------------ 工具函数 ------------------------------ */

function flattenMenus(nodes: MenuNode[], parentTitles: string[] = []): { node: MenuNode; trail: string[] }[] {
  const out: { node: MenuNode; trail: string[] }[] = []
  for (const n of nodes) {
    if (n.type !== "BUTTON" && n.path) {
      out.push({ node: n, trail: [...parentTitles, n.title] })
    }
    if (n.children?.length) {
      out.push(...flattenMenus(n.children, [...parentTitles, n.title]))
    }
  }
  return out
}

/** 通知四色视觉映射(与通知中心 TYPE_META 同构) */
const NOTIFICATION_TYPE_META: Record<
  NotificationType,
  { icon: LucideIcon; label: string; cls: string }
> = {
  INFO: { icon: Info, label: "信息", cls: "text-primary" },
  SUCCESS: { icon: CircleCheck, label: "成功", cls: "text-emerald-600 dark:text-emerald-400" },
  WARNING: { icon: TriangleAlert, label: "警告", cls: "text-amber-600 dark:text-amber-400" },
  SECURITY: { icon: ShieldAlert, label: "安全", cls: "text-destructive" },
}

/** 实体结果归一化展示项 */
interface EntityItem {
  id: string
  /** 主文本 */
  label: string
  /** 次行文本(用户: username · 所属组织) */
  sub: string
  /** 右侧文本(租户/组织: 灰色 mono code; 角色: 所属组织名) */
  right: string
  rightMono: boolean
  /** 参与 cmdk 前端过滤的可匹配文本(含后端匹配字段,避免二次过滤丢失) */
  value: string
  to: string
}

interface EntitySection {
  key: keyof GlobalSearchResult
  label: string
  icon: LucideIcon
  items: EntityItem[]
}

/** 将搜索结果按分区归一化(顺序: 用户 → 租户 → 组织 → 角色) */
function buildEntitySections(data: GlobalSearchResult): EntitySection[] {
  return [
    {
      key: "users",
      label: "用户",
      icon: User,
      items: data.users.map((r) => ({
        id: r.id,
        label: r.realName || r.username,
        sub: [r.username, r.organizationName].filter(Boolean).join(" · "),
        right: "",
        rightMono: false,
        value: `${r.realName} ${r.username} ${r.organizationName ?? ""}`,
        to: `/system/users?q=${encodeURIComponent(r.username)}`,
      })),
    },
    {
      key: "tenants",
      label: "租户",
      icon: Building2,
      items: data.tenants.map((r) => ({
        id: r.id,
        label: r.name,
        sub: "",
        right: r.code,
        rightMono: true,
        value: `${r.name} ${r.code}`,
        to: `/system/tenants?q=${encodeURIComponent(r.name)}`,
      })),
    },
    {
      key: "organizations",
      label: "组织",
      icon: Network,
      items: data.organizations.map((r) => ({
        id: r.id,
        label: r.name,
        sub: r.path,
        right: r.code,
        rightMono: true,
        value: `${r.name} ${r.code} ${r.path}`,
        to: `/system/organizations?q=${encodeURIComponent(r.name)}`,
      })),
    },
    {
      key: "roles",
      label: "角色",
      icon: Shield,
      items: data.roles.map((r) => ({
        id: r.id,
        label: r.name,
        sub: "",
        right: r.organizationName ?? "",
        rightMono: false,
        value: `${r.name} ${r.code} ${r.organizationName ?? ""}`,
        to: `/system/roles?q=${encodeURIComponent(r.code || r.name)}`,
      })),
    },
  ]
}

/** 全局命令面板:Ctrl+K 搜索菜单 / 用户 / 租户 / 组织 / 角色 / 通知并快速跳转 */
export function CommandPalette({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const profile = useAuthStore((s) => s.profile)
  const setCurrentApplication = useAuthStore((s) => s.setCurrentApplication)
  const currentApplicationId = useAuthStore((s) => s.currentApplicationId)
  const logout = useAuthStore((s) => s.logout)
  const notificationScope = useNotificationContext()
  const { resolvedTheme, setTheme } = useTheme()
  const apps = profile?.applications ?? []

  /* ---------- 已授权路径集合(「常用页面」分区过滤用,菜单缓存与全局共享) ---------- */
  const { allowedPaths, ready: pathsReady } = useAuthorizedPaths()

  /* ---------- 输入与防抖 ---------- */
  const [rawInput, setRawInput] = React.useState("")
  const hasQuery = rawInput.trim().length >= 1
  const debouncedQuery = useDebounced(rawInput.trim(), 300)

  // 面板打开时重置输入;关闭时清理,避免残留
  React.useEffect(() => {
    if (open) setRawInput("")
  }, [open])

  /* ---------- 最近搜索 ---------- */
  const [recentSearches, setRecentSearches] = React.useState<string[]>([])
  React.useEffect(() => {
    if (open) setRecentSearches(readRecentSearches())
  }, [open])

  /* ---------- 菜单查询(所有应用) ---------- */
  const menusQueries = useQueries({
    queries: apps.map((app) => ({
      queryKey: ["auth-menus", app.id],
      queryFn: async () =>
        sortMenuTree(asArray<MenuNode>(await authApi.menus(app.id), "menus")),
      staleTime: 60_000,
      retry: 1,
      enabled: open,
    })),
  })

  /* ---------- 实体搜索(服务端过滤) ---------- */
  const searchQuery = useQuery({
    queryKey: ["global-search", debouncedQuery],
    queryFn: () => searchEntities(debouncedQuery, 5),
    enabled: open && debouncedQuery.length >= 1,
    staleTime: 10_000,
    retry: 1,
    placeholderData: (prev) => prev,
  })
  const searchResult = hasQuery ? searchQuery.data ?? EMPTY_SEARCH_RESULT : EMPTY_SEARCH_RESULT

  /* ---------- 通知搜索(后端 q 参数 ILIKE 搜索, 输入 <2 字符不调) ---------- */
  const notificationsQuery = useQuery({
    queryKey: [...(notificationScope?.key ?? ["notifications", "inactive"]), "palette", debouncedQuery],
    queryFn: async ({ signal }) => {
      if (!notificationScope?.current()) throw new DOMException("通知上下文已失效", "AbortError")
      const data = await notificationsApi.list(
        { q: debouncedQuery, filter: "ALL", page: 1, size: 5 },
        notificationRequestOptions(notificationScope, signal)
      )
      if (!notificationScope.current()) throw new DOMException("通知上下文已失效", "AbortError")
      return data
    },
    enabled: !!notificationScope && open && debouncedQuery.length >= 2,
    staleTime: 10_000,
    retry: 1,
    placeholderData: (prev, previousQuery) =>
      notificationScope?.current() && notificationScope.key.every((part, i) => previousQuery?.queryKey[i] === part)
        ? prev
        : undefined,
  })
  const notificationHits: NotificationRecord[] = React.useMemo(
    () => (notificationScope?.current() && debouncedQuery.length >= 2 ? notificationsQuery.data?.records ?? [] : []),
    [notificationScope, debouncedQuery, notificationsQuery.data]
  )

  /** 点击通知:未读先标记已读再跳转(无 link 落到通知中心) */
  const openNotification = (n: NotificationRecord) => {
    if (!notificationScope?.current()) return
    if (!n.isRead) {
      void notificationsApi
        .markRead(n.id, notificationRequestOptions(notificationScope))
        .then(() => {
          if (notificationScope.current()) {
            return queryClient.invalidateQueries({ queryKey: notificationScope.key })
          }
        })
        .catch(() => undefined)
    }
    goToPath(n.link?.trim() || "/system/notifications")
  }

  React.useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault()
        onOpenChange(true)
      }
    }
    window.addEventListener("keydown", handler)
    return () => window.removeEventListener("keydown", handler)
  }, [onOpenChange])

  /* ---------- 跳转 ---------- */
  const go = (appId: string, path: string) => {
    if (appId !== currentApplicationId) setCurrentApplication(appId)
    onOpenChange(false)
    navigate(path)
  }

  /** 深链跳转:目标路径属于其它应用时同步切换应用上下文 */
  const goToPath = (path: string) => {
    const app = apps.find(
      (a) =>
        a.basePath &&
        a.basePath !== "/" &&
        (path === a.basePath || path.startsWith(`${a.basePath}/`))
    )
    if (app && app.id !== currentApplicationId) setCurrentApplication(app.id)
    onOpenChange(false)
    navigate(path)
  }

  /** 选中实体/菜单结果:记录搜索词后跳转 */
  const selectWithHistory = (term: string, run: () => void) => {
    if (term.trim()) {
      pushRecentSearch(term)
      setRecentSearches(readRecentSearches())
    }
    run()
  }

  const isDark = resolvedTheme === "dark"

  /* 常用页面:静态全量表按已授权路径过滤(个人功能 /profile 已在快捷操作分区,恒排除);
     未就绪或无符合项时整组不渲染,避免空分组闪烁 */
  const commonPages = React.useMemo(
    () =>
      pathsReady
        ? Object.entries(ROUTE_TITLES).filter(([p]) => p !== "/profile" && allowedPaths.has(p))
        : [],
    [allowedPaths, pathsReady]
  )

  return (
    <CommandDialog
      open={open}
      onOpenChange={onOpenChange}
      title="命令面板"
      description="搜索菜单、用户、租户、组织、角色与通知"
      className="sm:max-w-xl"
    >
      <CommandInput
        value={rawInput}
        onValueChange={setRawInput}
        placeholder="搜索菜单、用户、租户、组织、角色、通知..."
      />
      <CommandList className="max-h-[60vh]">
        <CommandEmpty>未找到匹配的结果</CommandEmpty>

        {/* ---------------- 有输入:菜单 ---------------- */}
        {hasQuery &&
          apps.map((app, i) => {
            const q = menusQueries[i]
            const items = q.data ? flattenMenus(q.data) : []
            return (
              <CommandGroup key={app.id} heading={`${app.name} · ${app.code}`}>
                {q.isLoading ? (
                  <div className="px-2 py-3 text-xs text-muted-foreground">加载菜单中...</div>
                ) : q.isError ? (
                  <div className="px-2 py-3 text-xs text-muted-foreground">菜单加载失败</div>
                ) : items.length === 0 ? (
                  <div className="px-2 py-3 text-xs text-muted-foreground">该应用暂无可跳转菜单</div>
                ) : (
                  items.map(({ node }) => (
                    <CommandItem
                      key={`${app.id}-${node.id}`}
                      value={`${app.name} ${node.title} ${node.path ?? ""} ${node.permission ?? ""} ${node.linkUrl ?? ""}`}
                      onSelect={() =>
                        selectWithHistory(rawInput, () => {
                          // 外链菜单: BLANK 新窗口 / SELF 内部嵌入(与侧栏同款跳转工具)
                          if (!openExternalLink(node, (to) => go(app.id, to))) {
                            go(app.id, node.path!)
                          }
                        })
                      }
                    >
                      <AppIcon name={node.icon} className="size-4 text-muted-foreground" />
                      <span className="truncate">{node.title}</span>
                      {node.linkUrl && (
                        <ExternalLink className="size-3 shrink-0 text-muted-foreground" aria-label="外链" />
                      )}
                      <span className="ml-auto truncate pl-4 font-mono text-xs text-muted-foreground/60">
                        {node.linkUrl ? node.linkUrl : node.path}
                      </span>
                    </CommandItem>
                  ))
                )}
              </CommandGroup>
            )
          })}

        {/* ---------------- 有输入:实体分区(仅显示有结果的分区) ---------------- */}
        {hasQuery &&
          buildEntitySections(searchResult).map((section) => {
            if (!section.items.length) return null
            const Icon = section.icon
            return (
              <CommandGroup key={section.key} heading={section.label}>
                {section.items.map((r) => (
                  <CommandItem
                    key={r.id}
                    value={r.value}
                    keywords={[r.value]}
                    onSelect={() => selectWithHistory(rawInput, () => goToPath(r.to))}
                  >
                    <Icon className="size-4 shrink-0 text-muted-foreground" />
                    <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                      <span className="truncate leading-tight">{r.label}</span>
                      {r.sub && (
                        <span className="truncate text-xs leading-tight text-muted-foreground/70">
                          {r.sub}
                        </span>
                      )}
                    </span>
                    {r.right && (
                      <span
                        className={cn(
                          "ml-2 max-w-[9rem] shrink-0 truncate text-xs",
                          r.rightMono
                            ? "font-mono text-muted-foreground/60"
                            : "text-muted-foreground/70"
                        )}
                      >
                        {r.right}
                      </span>
                    )}
                    <span className="ml-1 hidden shrink-0 rounded border bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground sm:inline-block">
                      {section.label}
                    </span>
                  </CommandItem>
                ))}
              </CommandGroup>
            )
          })}

        {/* ---------------- 有输入:通知分区(后端搜索, 仅显示有结果) ---------------- */}
        {hasQuery && notificationHits.length > 0 && (
          <CommandGroup heading="通知">
            {notificationHits.map((n) => {
              const meta = NOTIFICATION_TYPE_META[n.type] ?? NOTIFICATION_TYPE_META.INFO
              const Icon = meta.icon
              return (
                <CommandItem
                  key={n.id}
                  value={`${n.title} ${n.content ?? ""}`}
                  keywords={[n.type]}
                  disabled={!notificationScope}
                  onSelect={() => {
                    if (notificationScope?.current()) selectWithHistory(rawInput, () => openNotification(n))
                  }}
                >
                  <Icon className={cn("size-4 shrink-0", meta.cls)} />
                  <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <span className="truncate leading-tight">{n.title}</span>
                    <span
                      className="truncate text-xs leading-tight text-muted-foreground/70"
                      title={formatDateTime(n.createdAt)}
                    >
                      {formatRelative(n.createdAt)}
                    </span>
                  </span>
                  {!n.isRead && (
                    <span
                      className="ml-2 size-1.5 shrink-0 rounded-full bg-primary"
                      title="未读"
                      aria-label="未读"
                    />
                  )}
                  <span className="ml-1 hidden shrink-0 rounded border bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground sm:inline-block">
                    {meta.label}
                  </span>
                </CommandItem>
              )
            })}
          </CommandGroup>
        )}

        {/* ---------------- 有输入:加载提示 ---------------- */}
        {hasQuery && searchQuery.isFetching && !searchQuery.data && (
          <div className="px-2 py-2 text-xs text-muted-foreground">正在搜索用户 / 租户 / 组织 / 角色...</div>
        )}
        {hasQuery && searchQuery.isError && (
          <div className="px-2 py-2 text-xs text-muted-foreground">
            实体搜索暂不可用({searchQuery.error instanceof Error ? searchQuery.error.message : "网络错误"})
          </div>
        )}

        {hasQuery && <CommandSeparator />}

        {/* ---------------- 无输入:最近搜索 ---------------- */}
        {!hasQuery && recentSearches.length > 0 && (
          <CommandGroup heading="最近搜索">
            {recentSearches.map((term) => (
              <CommandItem
                key={term}
                value={`最近搜索 ${term}`}
                onSelect={() => setRawInput(term)}
              >
                <History className="size-4 text-muted-foreground" />
                <span className="truncate">{term}</span>
              </CommandItem>
            ))}
          </CommandGroup>
        )}

        {/* ---------------- 无输入:快捷操作 ---------------- */}
        {!hasQuery && (
          <CommandGroup heading="快捷操作">
            <CommandItem
              value="通知中心 notifications bell"
              disabled={!notificationScope}
              onSelect={() => {
                if (notificationScope?.current()) goToPath("/system/notifications")
              }}
            >
              <Bell className="size-4 text-muted-foreground" />
              <span>通知中心</span>
              <span className="ml-auto pl-4 font-mono text-xs text-muted-foreground/60">/system/notifications</span>
            </CommandItem>
            <CommandItem value="个人中心 profile" onSelect={() => goToPath("/profile")}>
              <User className="size-4 text-muted-foreground" />
              <span>个人中心</span>
              <span className="ml-auto pl-4 font-mono text-xs text-muted-foreground/60">/profile</span>
            </CommandItem>
            <CommandItem
              value={`切换${isDark ? "浅色" : "深色"}主题 深浅色 dark light`}
              onSelect={() => {
                setTheme(isDark ? "light" : "dark")
                onOpenChange(false)
              }}
            >
              {isDark ? (
                <Sun className="size-4 text-muted-foreground" />
              ) : (
                <Moon className="size-4 text-muted-foreground" />
              )}
              <span>{isDark ? "切换浅色主题" : "切换深色主题"}</span>
            </CommandItem>
            <CommandItem
              value="退出登录 logout"
              onSelect={() => {
                onOpenChange(false)
                void (async () => {
                  try {
                    await authApi.logout()
                  } catch {
                    toast.warning("本机已退出，服务端会话撤销未确认")
                  } finally {
                    logout()
                    navigate("/login", { replace: true })
                  }
                })()
              }}
            >
              <LogOut className="size-4 text-muted-foreground" />
              <span>退出登录</span>
            </CommandItem>
          </CommandGroup>
        )}

        {!hasQuery && <CommandSeparator />}

        {/* ---------------- 常用页面(按已授权路径过滤 + cmdk 自带输入过滤;无符合项不渲染分组) ---------------- */}
        {commonPages.length > 0 && (
          <CommandGroup heading="常用页面">
            {commonPages.map(([path, title]) => (
              <CommandItem
                key={path}
                value={`${title} ${path}`}
                onSelect={() => goToPath(path)}
              >
                <AppIcon name="Compass" className="size-4 text-muted-foreground" />
                <span>{title}</span>
                <span className="ml-auto pl-4 font-mono text-xs text-muted-foreground/60">{path}</span>
              </CommandItem>
            ))}
          </CommandGroup>
        )}
      </CommandList>
    </CommandDialog>
  )
}
