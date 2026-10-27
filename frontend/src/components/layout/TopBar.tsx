import * as React from "react"
import { useLocation, useNavigate } from "react-router-dom"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import {
  Bell,
  Building2,
  Check,
  CheckCheck,
  ChevronRight,
  CircleCheck,
  Command,
  Info,
  KeyRound,
  Loader2,
  LogOut,
  Menu as MenuIcon,
  Moon,
  PanelLeftOpen,
  ShieldAlert,
  TriangleAlert,
  UserCog,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { Avatar, AvatarFallback } from "@/components/ui/avatar"
import { Badge } from "@/components/ui/badge"
import { Skeleton } from "@/components/ui/skeleton"
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { AlertDialog, AlertDialogContent, AlertDialogHeader, AlertDialogTitle, AlertDialogDescription,
  AlertDialogFooter, AlertDialogCancel } from "@/components/ui/alert-dialog"
import { useAuthStore } from "@/stores/auth"
import { usePlatformBranding } from "@/hooks/use-platform-branding"
import { useMenusQuery, useSwitchContext } from "@/hooks/use-auth-data"
import { useApplicationNavigation } from "@/hooks/use-application-navigation"
import { useNotificationSocket } from "@/hooks/use-notification-socket"
import {
  notificationRequestOptions,
  useNotificationContext,
  type NotificationContext,
} from "@/hooks/use-notification-context"
import { notificationsApi } from "@/api/notifications"
import { logoutPlatformSession } from "@/lib/api"
import { AppIcon } from "@/lib/icons"
import { formatRelative } from "@/lib/format"
import { routeTitle } from "@/lib/routes"
import { toast } from "sonner"
import { cn } from "@/lib/utils"
import { canSelectConsoleMembership, membershipSelectionDisabledReason, membershipSelectionKey } from "@/lib/context-identity"
import type { Membership, MenuNode, NotificationRecord, NotificationType } from "@/api/types"

export function TopBar({
  onOpenMobileNav,
  onOpenCommandPalette,
  menuCollapsed = false,
  onExpandMenu,
}: {
  onOpenMobileNav: () => void
  onOpenCommandPalette: () => void
  menuCollapsed?: boolean
  onExpandMenu?: () => void
}) {
  const { pathname } = useLocation()
  const { platformName } = usePlatformBranding()
  const profile = useAuthStore((s) => s.profile)
  const currentApplicationId = useAuthStore((s) => s.currentApplicationId)
  const app = profile?.applications.find((a) => a.id === currentApplicationId) ?? null
  const menusQuery = useMenusQuery(currentApplicationId)

  const crumbs = React.useMemo(() => {
    const titles: string[] = []
    const find = (nodes: MenuNode[], target: string): MenuNode[] | null => {
      for (const n of nodes) {
        if (n.path === target) return [n]
        if (n.children?.length) {
          const sub = find(n.children, target)
          if (sub) return [n, ...sub]
        }
      }
      return null
    }
    const chain = menusQuery.data ? find(menusQuery.data, pathname) : null
    if (chain) {
      chain.forEach((n) => titles.push(n.title))
    } else {
      titles.push(routeTitle(pathname))
    }
    return titles
  }, [menusQuery.data, pathname])

  /* 实时通知 socket —— 整个应用只挂载一次 (TopBar 常驻) */
  const { connected: liveConnected, lastNotificationAt, prefs } = useNotificationSocket()
  const notificationScope = useNotificationContext()

  return (
    <header className="flex h-14 shrink-0 items-center gap-2 border-b bg-background px-3 md:px-4">
      <Button
        variant="ghost"
        size="icon"
        className="lg:hidden"
        aria-label="打开导航"
        onClick={onOpenMobileNav}
      >
        <MenuIcon className="size-5" />
      </Button>

      {menuCollapsed && onExpandMenu && (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button variant="ghost" size="icon" className="hidden lg:inline-flex"
              aria-label="展开菜单" aria-expanded="false" onClick={onExpandMenu}>
              <PanelLeftOpen className="size-5" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>展开菜单</TooltipContent>
        </Tooltip>
      )}

      {/* 面包屑 */}
      <nav aria-label="面包屑" className="min-w-0 flex-1">
        <Breadcrumb>
          <BreadcrumbList className="flex-nowrap">
            <BreadcrumbItem className="hidden sm:inline-flex">
              <BreadcrumbLink className="flex items-center gap-1.5 text-muted-foreground">
                {app ? (
                  <>
                    <AppIcon name={app.icon} className="size-3.5" />
                    <span className="max-w-[8rem] truncate" title={app.name}>
                      {app.name}
                    </span>
                  </>
                ) : (
                  platformName
                )}
              </BreadcrumbLink>
            </BreadcrumbItem>
            {crumbs.map((c, i) => (
              <React.Fragment key={`${c}-${i}`}>
                <BreadcrumbSeparator className="hidden sm:inline-flex" />
                <BreadcrumbItem>
                  {i === crumbs.length - 1 ? (
                    <BreadcrumbPage
                      className="max-w-[10rem] truncate font-medium sm:max-w-[18rem] lg:max-w-[24rem]"
                      title={c}
                    >
                      {c}
                    </BreadcrumbPage>
                  ) : (
                    <span className="max-w-[8rem] truncate text-muted-foreground" title={c}>
                      {c}
                    </span>
                  )}
                </BreadcrumbItem>
              </React.Fragment>
            ))}
          </BreadcrumbList>
        </Breadcrumb>
      </nav>

      <div className="flex shrink-0 items-center gap-1">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="outline"
              size="sm"
              className="hidden h-8 gap-2 px-2 text-muted-foreground md:flex"
              onClick={onOpenCommandPalette}
              aria-label="打开命令面板"
            >
              <Command className="size-3.5" />
              <span className="text-xs">Ctrl K</span>
              <kbd className="pointer-events-none inline-flex h-5 select-none items-center gap-0.5 rounded border bg-muted px-1.5 font-mono text-[10px] font-medium text-muted-foreground">
                K
              </kbd>
            </Button>
          </TooltipTrigger>
          <TooltipContent>全局搜索菜单,快速跳转</TooltipContent>
        </Tooltip>
        <Button
          variant="ghost"
          size="icon"
          className="md:hidden"
          aria-label="打开命令面板"
          onClick={onOpenCommandPalette}
        >
          <Command className="size-4.5" />
        </Button>

        <NotificationBell
          key={JSON.stringify(notificationScope?.key ?? null)}
          scope={notificationScope}
          liveConnected={!!notificationScope && liveConnected}
          lastNotificationAt={notificationScope ? lastNotificationAt : 0}
          dndPrefs={notificationScope && prefs?.dndEnabled ? prefs : undefined}
        />
        <OrgContextChip />
        <UserMenu />
      </div>
    </header>
  )
}

/* ============ 通知铃铛 ============ */

const NOTIFY_META: Record<NotificationType, { icon: typeof Info; chip: string }> = {
  INFO: { icon: Info, chip: "bg-primary/12 text-primary" },
  SUCCESS: { icon: CircleCheck, chip: "bg-emerald-500/12 text-emerald-600 dark:text-emerald-400" },
  WARNING: { icon: TriangleAlert, chip: "bg-amber-500/15 text-amber-600 dark:text-amber-400" },
  SECURITY: { icon: ShieldAlert, chip: "bg-red-500/12 text-red-600 dark:text-red-400" },
}

function NotificationBell({
  scope,
  liveConnected,
  lastNotificationAt,
  dndPrefs,
}: {
  scope: NotificationContext | null
  liveConnected: boolean
  lastNotificationAt: number
  /** 免打扰开启时的偏好数据 (undefined = 未开启或未加载) */
  dndPrefs?: { dndStart: string; dndEnd: string }
}) {
  const { navigateTo: navigate, access } = useApplicationNavigation()
  const queryClient = useQueryClient()
  const countQuery = useQuery({
    queryKey: [...(scope?.key ?? ["notifications", "inactive"]), "unread-count"],
    queryFn: async ({ signal }) => {
      if (!scope?.current()) throw new DOMException("通知上下文已失效", "AbortError")
      const data = await notificationsApi.unreadCount(notificationRequestOptions(scope, signal))
      if (!scope.current()) throw new DOMException("通知上下文已失效", "AbortError")
      return data
    },
    enabled: !!scope,
    staleTime: 15_000,
    refetchInterval: 60_000,
    retry: 1,
  })
  const [open, setOpen] = React.useState(false)

  /* 收到实时通知 → 铃铛一次性摆动 (key 变更重挂载以重启动画) */
  const [swingKey, setSwingKey] = React.useState(0)
  React.useEffect(() => {
    if (lastNotificationAt > 0) setSwingKey((k) => k + 1)
  }, [lastNotificationAt])

  const listQuery = useQuery({
    queryKey: [...(scope?.key ?? ["notifications", "inactive"]), "bell", 1, 5],
    queryFn: async ({ signal }) => {
      if (!scope?.current()) throw new DOMException("通知上下文已失效", "AbortError")
      const data = await notificationsApi.list(
        { filter: "ALL", page: 1, size: 5 },
        notificationRequestOptions(scope, signal)
      )
      if (!scope.current()) throw new DOMException("通知上下文已失效", "AbortError")
      return data
    },
    enabled: !!scope && open,
    staleTime: 10_000,
    retry: 1,
  })

  const unread = scope?.current() ? countQuery.data?.count ?? 0 : 0
  const items: NotificationRecord[] = scope?.current() ? listQuery.data?.records ?? [] : []

  const readAllMutation = useMutation({
    mutationFn: async (requestScope: NotificationContext) => {
      if (!requestScope.current()) throw new DOMException("通知上下文已失效", "AbortError")
      const data = await notificationsApi.readAll(notificationRequestOptions(requestScope))
      if (!requestScope.current()) throw new DOMException("通知上下文已失效", "AbortError")
      return data
    },
    onSuccess: (d, requestScope) => {
      if (!requestScope.current()) return
      if (d.updated > 0) toast.success(`已读 ${d.updated} 条`)
      void queryClient.invalidateQueries({ queryKey: requestScope.key })
    },
  })

  const openItem = (n: NotificationRecord) => {
    if (!scope?.current()) return
    if (!n.isRead) {
      void notificationsApi
        .markRead(n.id, notificationRequestOptions(scope))
        .then(() => {
          if (scope.current()) return queryClient.invalidateQueries({ queryKey: scope.key })
        })
        .catch(() => undefined)
    }
    if (n.link && navigate(n.link)) setOpen(false)
  }

  return (
    <Popover open={open && !!scope} onOpenChange={(o) => {
      if (!o || scope?.current()) setOpen(o)
    }}>
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className="relative"
          aria-label={unread > 0 ? `${unread} 条未读通知` : "通知"}
          disabled={!scope}
        >
          <Bell key={swingKey} className={cn("size-4.5", swingKey > 0 && "bell-swing")} />
          {unread > 0 && (
            <span className="absolute -right-0.5 -top-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 font-mono text-[10px] font-bold leading-none text-primary-foreground shadow-sm">
              {unread > 99 ? "99+" : unread}
            </span>
          )}
          {/* 状态角标:DND 开启 → amber 小月亮;否则 socket 在线时 teal 实时点 */}
          {dndPrefs ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <span
                  aria-label={`免打扰中 ${dndPrefs.dndStart}-${dndPrefs.dndEnd}`}
                  className="absolute bottom-1 right-1 leading-none"
                >
                  <Moon className="size-3 fill-amber-400 text-amber-500 drop-shadow-sm" />
                </span>
              </TooltipTrigger>
              <TooltipContent side="bottom" className="text-xs">
                免打扰中 {dndPrefs.dndStart}-{dndPrefs.dndEnd},提醒已静默
              </TooltipContent>
            </Tooltip>
          ) : (
            liveConnected && (
              <Tooltip>
                <TooltipTrigger asChild>
                  <span aria-label="实时推送已连接" className="absolute bottom-1 right-1 leading-none">
                    <span className="block size-1.5 rounded-full bg-emerald-500 ring-2 ring-background" />
                  </span>
                </TooltipTrigger>
                <TooltipContent side="bottom" className="text-xs">
                  实时推送已连接
                </TooltipContent>
              </Tooltip>
            )
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-96 p-0">
        <div className="flex items-center justify-between border-b px-4 py-2.5">
          <p className="flex items-center gap-1.5 text-sm font-semibold">
            通知
            {unread > 0 && (
              <Badge variant="secondary" className="h-5 bg-primary/15 px-1.5 tabular-nums text-primary">
                {unread} 条未读
              </Badge>
            )}
          </p>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 gap-1 px-2 text-xs text-muted-foreground"
            onClick={() => {
              if (scope?.current()) readAllMutation.mutate(scope)
            }}
            disabled={!scope || readAllMutation.isPending || unread === 0}
          >
            {readAllMutation.isPending ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <CheckCheck className="size-3.5" />
            )}
            全部已读
          </Button>
        </div>
        <div className="max-h-80 overflow-y-auto scroll-area">
          {open && listQuery.isLoading ? (
            <div className="space-y-3 p-4">
              {Array.from({ length: 4 }).map((_, i) => (
                <div key={i} className="flex items-start gap-3">
                  <Skeleton className="size-8 shrink-0 rounded-lg" />
                  <div className="flex-1 space-y-1.5">
                    <Skeleton className="h-3.5 w-3/5" />
                    <Skeleton className="h-3 w-full" />
                  </div>
                </div>
              ))}
            </div>
          ) : listQuery.isError ? (
            <div className="flex flex-col items-center gap-2 p-6 text-center text-xs text-muted-foreground">
              通知加载失败
              <Button variant="outline" size="sm" disabled={!scope} onClick={() => {
                if (scope?.current()) void listQuery.refetch()
              }}>
                重试
              </Button>
            </div>
          ) : items.length === 0 ? (
            <p className="p-6 text-center text-xs text-muted-foreground">暂无通知</p>
          ) : (
            <ul className="divide-y">
              {items.map((n) => {
                const meta = NOTIFY_META[n.type] ?? NOTIFY_META.INFO
                return (
                  <li key={n.id}>
                    <button
                      type="button"
                      disabled={!scope}
                      onClick={() => openItem(n)}
                      className={cn(
                        "flex w-full items-start gap-3 px-4 py-3 text-left outline-none transition-colors hover:bg-accent/60 focus-visible:bg-accent",
                        n.isRead ? "opacity-75" : ""
                      )}
                    >
                      <span
                        className={cn(
                          "mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg",
                          meta.chip
                        )}
                      >
                        <meta.icon className="size-4" />
                      </span>
                      <div className="min-w-0 flex-1">
                        <p className={cn("truncate text-xs", n.isRead ? "font-medium" : "font-semibold")}>
                          {n.title}
                        </p>
                        <p className="mt-0.5 line-clamp-2 text-[11px] leading-relaxed text-muted-foreground">
                          {n.content || "—"}
                        </p>
                        <p className="mt-1 text-[10px] text-muted-foreground/60">
                          {formatRelative(n.createdAt)}
                          {n.link ? " · 点击查看" : ""}
                        </p>
                      </div>
                      {!n.isRead && (
                        <span aria-hidden className="mt-1.5 size-1.5 shrink-0 rounded-full bg-primary" />
                      )}
                    </button>
                  </li>
                )
              })}
            </ul>
          )}
        </div>
        <button
          type="button"
          disabled={!scope}
          className="flex w-full items-center justify-center gap-1 border-t px-3 py-2.5 text-xs text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          onClick={() => {
            if (!scope?.current()) return
            setOpen(false)
            if (access("/system/notifications").status === "allowed") navigate("/system/notifications")
          }}
        >
          查看全部通知
          <ChevronRight className="size-3.5" />
        </button>
      </PopoverContent>
    </Popover>
  )
}

export function auditActionLabel(action: string): string {
  const map: Record<string, string> = {
    LOGIN: "登录成功",
    LOGIN_FAILED: "登录失败",
    CONTEXT_SWITCH: "切换组织上下文",
    LOGOUT: "退出登录",
    CONSENT_APPROVE: "同意授权",
    CONSENT_DENY: "拒绝授权",
    DEVICE_APPROVE: "批准设备",
    DEVICE_DENY: "拒绝设备",
    TOKEN_ISSUED: "签发令牌",
    TOKEN_REVOKED: "吊销令牌",
    TENANT_AUTH: "租户授权变更",
    ORG_AUTH: "组织授权变更",
    ROLE_MENUS: "角色菜单变更",
  }
  if (map[action]) return map[action]
  if (action.startsWith("CREATE_")) return `创建 ${action.slice(7)}`
  if (action.startsWith("UPDATE_")) return `更新 ${action.slice(7)}`
  if (action.startsWith("DELETE_")) return `删除 ${action.slice(7)}`
  return action
}

export function auditActionIcon(action: string): string {
  if (action === "LOGIN") return "LogIn"
  if (action === "LOGIN_FAILED") return "XCircle"
  if (action === "LOGOUT") return "LogOut"
  if (action === "CONTEXT_SWITCH") return "RefreshCw"
  if (action.startsWith("CONSENT")) return "KeyRound"
  if (action.startsWith("DEVICE")) return "Tv"
  if (action.startsWith("TOKEN")) return "Shield"
  if (action.endsWith("_AUTH")) return "ShieldCheck"
  if (action.startsWith("CREATE_")) return "Plus"
  if (action.startsWith("UPDATE_")) return "Edit"
  if (action.startsWith("DELETE_")) return "Trash2"
  return "Activity"
}

/** 审计动作徽章着色 */
export function auditActionTone(action: string): string {
  if (action === "LOGIN" || action === "LOGOUT") return "text-primary bg-primary/10 border-primary/25"
  if (action === "LOGIN_FAILED" || action.startsWith("DELETE_") || action.endsWith("DENY") || action === "TOKEN_REVOKED")
    return "text-destructive bg-destructive/10 border-destructive/25"
  if (action.startsWith("CREATE_") || action.endsWith("APPROVE")) return "text-primary bg-primary/10 border-primary/25"
  if (action.startsWith("UPDATE_") || action.endsWith("_AUTH") || action === "ROLE_MENUS" || action === "CONTEXT_SWITCH")
    return "text-chart-4 bg-chart-4/10 border-chart-4/25"
  if (action.startsWith("TOKEN") || action.startsWith("DEVICE") || action.startsWith("CONSENT"))
    return "text-warning bg-warning/10 border-warning/25"
  return "text-muted-foreground bg-muted border-border"
}

/* ============ 组织上下文 Chip ============ */

function OrgContextChip() {
  const profile = useAuthStore((s) => s.profile)
  const switchContext = useSwitchContext()
  const { navigateTo: navigate, access } = useApplicationNavigation()
  const mutation = useMutation({
    mutationFn: (member: Membership) => switchContext(member.organizationId, member.membershipId),
    onSuccess: () => toast.success("已切换成员身份"),
    onError: (e: Error) => toast.error(e.message || "切换失败"),
  })

  if (!profile) return null

  /* 「管理组织」入口按权限显隐: 所有账号均按组织有效权限判断; 权限未就绪按无权限处理(防按钮闪现),
     其余需 system:organization(与 Task 15 permsReady 门控模式同款) */
  const canManageOrgs = access("/system/organizations").status === "allowed"

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="hidden max-w-[11rem] items-center gap-2 rounded-full border bg-card px-3 py-1.5 text-xs outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring md:flex"
          aria-label="切换组织与成员身份"
          title={`${profile.tenant.name} / ${profile.organization.name} · ${profile.memberType === "EMPLOYEE" ? "员工" : "客户"}`}
        >
          <Building2 className="size-3.5 shrink-0 text-primary" />
          <span className="flex min-w-0 items-center gap-1">
            <span className="max-w-[4rem] truncate text-muted-foreground" title={profile.tenant.name}>
              {profile.tenant.name}
            </span>
            <span className="shrink-0 text-muted-foreground" aria-hidden>
              /
            </span>
            <span className="max-w-[5rem] truncate font-medium" title={profile.organization.name}>
              {profile.organization.name}
            </span>
          </span>
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80 p-0">
        <div className="border-b px-3 py-2.5">
          <p className="text-sm font-semibold">组织与成员身份</p>
          <p
            className="mt-0.5 truncate text-xs text-muted-foreground"
            title={`当前:${profile.tenant.name} · ${profile.organization.path}`}
          >
            当前:{profile.tenant.name} · {profile.organization.path} · {profile.memberType === "EMPLOYEE" ? "员工" : "客户"}
          </p>
        </div>
        <div className="max-h-64 overflow-y-auto scroll-area p-1.5">
          <p className="px-2 py-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
            我的员工身份
          </p>
          {profile.memberships.map((m) => {
            const active = m.membershipId === profile.membershipId && m.memberType === profile.memberType &&
              m.identityVersion === profile.identityVersion
            const disabledReason = membershipSelectionDisabledReason(m)
            return (
              <button
                key={membershipSelectionKey(m)}
                type="button"
                disabled={active || mutation.isPending || !canSelectConsoleMembership(m)}
                onClick={() => mutation.mutate(m)}
                className={cn(
                  "flex w-full items-center gap-2.5 rounded-md px-2 py-2 text-left text-sm transition-colors disabled:cursor-default disabled:opacity-70",
                  active ? "bg-primary/10" : "hover:bg-accent"
                )}
              >
                <span
                  className={cn(
                    "flex size-7 shrink-0 items-center justify-center rounded-md",
                    active ? "bg-primary/15 text-primary" : "bg-muted text-muted-foreground"
                  )}
                >
                  <Building2 className="size-3.5" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className={cn("block truncate text-xs font-medium", active && "text-primary")}>
                    {m.organizationName}
                    {m.memberType === "EMPLOYEE" && m.departmentName ? ` · ${m.departmentName}` : ""}
                  </span>
                  <span className="block truncate text-[11px] text-muted-foreground">
                    {m.tenantName} · {m.organizationPath} · {m.memberType === "EMPLOYEE" ? "员工" : "客户"} · {m.status === "ENABLED" ? "有效" : "已停用"}
                  </span>
                  {disabledReason && <span className="mt-0.5 block text-[11px] text-muted-foreground">{disabledReason}</span>}
                </span>
                {active ? (
                  <Check className="size-4 shrink-0 text-primary" />
                ) : mutation.isPending && mutation.variables?.membershipId === m.membershipId ? (
                  <Loader2 className="size-4 shrink-0 animate-spin text-muted-foreground" />
                ) : null}
              </button>
            )
          })}
        </div>
        {canManageOrgs && (
          <button
            type="button"
            className="flex w-full items-center justify-center gap-1 border-t px-3 py-2.5 text-xs text-muted-foreground hover:bg-accent hover:text-accent-foreground"
            onClick={() => navigate("/system/organizations")}
          >
            管理组织
            <ChevronRight className="size-3.5" />
          </button>
        )}
      </PopoverContent>
    </Popover>
  )
}

/* ============ 用户菜单 ============ */

function UserMenu() {
  const profile = useAuthStore((s) => s.profile)
  const [confirmGlobal, setConfirmGlobal] = React.useState(false)
  const [globalPending, setGlobalPending] = React.useState(false)
  const navigate = useNavigate()
  const { navigateTo } = useApplicationNavigation()
  const queryClient = useQueryClient()

  const user = profile?.user
  const displayName = user?.realName || user?.username || "用户"

  const handleLogout = async () => {
    try {
      if (!await logoutPlatformSession()) return
      queryClient.clear()
      toast.success("已退出本平台")
      navigate("/login?signedOut=1", { replace: true })
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "退出结果尚未确认，请重新登录")
    }
  }

  const handleGlobalLogout = async () => {
    if (globalPending) return
    setGlobalPending(true)
    try {
      if (!await logoutPlatformSession(undefined, "GLOBAL")) return
      queryClient.clear()
      toast.success("已退出所有应用")
      navigate("/login?signedOut=1", { replace: true })
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "退出所有应用的结果尚未确认，请重新检查登录状态")
    } finally { setGlobalPending(false) }
  }

  return (
    <>
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="flex items-center gap-2 rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label="用户菜单"
        >
          <Avatar className="size-8 border">
            <AvatarFallback className="bg-primary/15 text-xs font-semibold text-primary">
              {displayName.slice(0, 1).toUpperCase()}
            </AvatarFallback>
          </Avatar>
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuLabel className="flex items-center gap-2.5">
          <Avatar className="size-8 border">
            <AvatarFallback className="bg-primary/15 text-xs font-semibold text-primary">
              {displayName.slice(0, 1).toUpperCase()}
            </AvatarFallback>
          </Avatar>
          <div className="min-w-0">
            <p className="truncate text-sm font-medium">{displayName}</p>
            {user?.username && <p className="truncate font-mono text-[11px] text-muted-foreground">@{user.username}</p>}
          </div>
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={() => navigateTo("/profile")}>
          <UserCog className="size-4" />
          个人中心
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => navigateTo("/profile#password")}>
          <KeyRound className="size-4" />
          修改密码
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem variant="destructive" onClick={handleLogout}>
          <LogOut className="size-4" />
          退出本平台
        </DropdownMenuItem>
        <DropdownMenuItem variant="destructive" onClick={() => setConfirmGlobal(true)}>
          <LogOut className="size-4" />
          退出所有应用
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
    <AlertDialog open={confirmGlobal} onOpenChange={setConfirmGlobal}>
      <AlertDialogContent>
        <AlertDialogHeader><AlertDialogTitle>退出所有应用？</AlertDialogTitle>
          <AlertDialogDescription>当前账号在所有应用中的登录状态都会失效。其他设备和浏览器也需要重新登录。</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter><AlertDialogCancel disabled={globalPending}>取消</AlertDialogCancel>
          <Button variant="destructive" disabled={globalPending} onClick={() => { void handleGlobalLogout() }}>
            {globalPending ? "正在退出…" : "退出所有应用"}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
    </>
  )
}
