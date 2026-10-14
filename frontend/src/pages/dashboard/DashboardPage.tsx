import * as React from "react"
import { Link, useNavigate } from "react-router-dom"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip as ReTooltip,
  XAxis,
  YAxis,
} from "recharts"
import {
  ArrowRight,
  Bell,
  Building2,
  ChevronRight,
  Clock,
  History,
  KeyRound,
  Landmark,
  LayoutGrid,
  Map,
  Plus,
  Shield,
  ShieldCheck,
  Sparkles,
  TrendingDown,
  TrendingUp,
  User,
  Users,
} from "lucide-react"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Skeleton } from "@/components/ui/skeleton"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { ErrorState, StatSkeleton, ChartSkeleton } from "@/components/states"
import { AppIcon } from "@/lib/icons"
import { ApiError, asArray } from "@/lib/api"
import { sortMenuTree } from "@/lib/tree"
import { monitorApi } from "@/api/monitor"
import { authApi } from "@/api/auth"
import { useAuthStore } from "@/stores/auth"
import { usePermissionsQuery } from "@/hooks/use-auth-data"
import { useRecentVisits, type RecentVisit } from "@/hooks/use-recent-visits"
import { useAuthorizedPaths } from "@/hooks/use-authorized-paths"
import { formatNumber, formatRelative, greeting } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { MenuNode } from "@/api/types"

/** mock 微趋势(仅展示行,标注为估算) */
const MOCK_TREND: Record<string, number> = {
  tenant: 2,
  user: 12,
  app: 0,
  role: 3,
}

function ChartTip({ active, payload, label }: { active?: boolean; payload?: { value: number }[]; label?: string }) {
  if (!active || !payload?.length) return null
  return (
    <div className="rounded-lg border bg-popover px-3 py-2 text-xs shadow-lg">
      <p className="mb-1 font-medium text-popover-foreground">{label}</p>
      {payload.map((p, i) => (
        <p key={i} className="tabular-nums text-muted-foreground">
          数量:<span className="font-semibold text-foreground">{formatNumber(p.value)}</span>
        </p>
      ))}
    </div>
  )
}

function isForbiddenError(e: unknown): boolean {
  return e instanceof ApiError && (e.status === 403 || e.code === "FORBIDDEN")
}

/* ============ 两视图共用:高频访问候选提取 ============ */

/** 仪表盘整页骨架: 横幅(头像/问候/徽章/快捷按钮) + 统计卡 + 双图表,与真实布局同构 */
function DashboardSkeleton() {
  return (
    <div className="space-y-6" aria-busy="true" aria-label="仪表盘加载中">
      <Card className="overflow-hidden border-primary/20">
        <CardContent className="flex flex-col gap-4 p-5 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex min-w-0 items-center gap-4">
            <Skeleton className="size-12 shrink-0 rounded-2xl" />
            <div className="space-y-2.5">
              <Skeleton className="h-5 w-44" />
              <div className="flex items-center gap-1.5">
                <Skeleton className="h-5 w-24 rounded-full" />
                <Skeleton className="h-5 w-28 rounded-full" />
              </div>
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            {Array.from({ length: 4 }).map((_, i) => (
              <Skeleton key={i} className="h-8 w-28 rounded-md" />
            ))}
          </div>
        </CardContent>
      </Card>
      <StatSkeleton />
      <div className="grid gap-4 lg:grid-cols-2">
        {[0, 1].map((i) => (
          <Card key={i}>
            <CardHeader className="pb-2">
              <Skeleton className="h-5 w-36" />
              <Skeleton className="h-3 w-52" />
            </CardHeader>
            <CardContent className="h-64">
              <ChartSkeleton />
            </CardContent>
          </Card>
        ))}
      </div>
    </div>
  )
}

/** 「高频访问」排除清单(两视图共用):固定入口 path 与仪表盘自身 */
const QUICK_EXCLUDED_PATHS = new Set(["/profile", "/system/notifications", "/dashboard"])

/**
 * 高频访问候选(两视图共用):先按已授权路径过滤(未就绪时返回空,不闪现无权项),
 * 再排除固定项与 /dashboard 自身,按 count 降序(同 count 按最近访问 at 降序)取前 limit 项。
 */
function pickTopVisits(
  recentVisits: RecentVisit[],
  limit: number,
  authorized: { allowedPaths: Set<string>; ready: boolean }
): RecentVisit[] {
  if (!authorized.ready) return []
  return [...recentVisits]
    .filter((v) => !QUICK_EXCLUDED_PATHS.has(v.path) && authorized.allowedPaths.has(v.path))
    .sort((a, b) => b.count - a.count || b.at - a.at)
    .slice(0, limit)
}

export default function DashboardPage() {
  const profile = useAuthStore((s) => s.profile)
  const navigate = useNavigate()
  const recentVisits = useRecentVisits()
  /* 已授权路径集合:高频访问展示前按权限过滤(记录照存,权限恢复后自动重现) */
  const { allowedPaths, ready: pathsReady } = useAuthorizedPaths()
  const overviewQuery = useQuery({
    queryKey: ["monitor-overview"],
    queryFn: () => monitorApi.overview(),
    staleTime: 30_000,
    retry: 1,
  })
  const permissionsQuery = usePermissionsQuery()
  const permissions = new Set(permissionsQuery.data?.permissions ?? [])
  /* 权限未就绪时一律视为无权限: 避免「先全部可见、加载后消失」的按钮闪现。
     空权限集也不授予快捷操作。 */
  const permsReady = permissionsQuery.isSuccess || permissionsQuery.isError
  const can = (p: string) => {
    if (!permsReady) return false
    return permissions.has(p)
  }

  const displayName = profile?.user.realName || profile?.user.username || ""

  /** 横幅「高频访问」Top 3(与个人工作台共用过滤+排序,仅条数不同) */
  const topVisits = React.useMemo(
    () => pickTopVisits(recentVisits, 3, { allowedPaths, ready: pathsReady }),
    [recentVisits, allowedPaths, pathsReady]
  )

  /* 监控总览加载中: 整页骨架占位。避免无 monitor 权限的用户在 403 返回前
     闪现平台视图(含新建租户/用户/角色等快捷按钮)后再切到个人工作台 */
  if (overviewQuery.isLoading) {
    return <DashboardSkeleton />
  }

  /* 无 monitor 权限(如普通组织用户) → 个人工作台视图 */
  if (overviewQuery.isError && isForbiddenError(overviewQuery.error)) {
    return <PersonalDashboard displayName={displayName} permissionCount={permissions.size} />
  }

  const data = overviewQuery.data

  const stats = [
    { key: "tenant", label: "租户总数", value: data?.tenantCount, icon: Landmark, to: "/system/tenants", perm: "system:tenant" },
    { key: "user", label: "用户总数", value: data?.userCount, icon: Users, to: "/system/users", perm: "system:user" },
    { key: "app", label: "接入应用", value: data?.applicationCount, icon: LayoutGrid, to: "/system/applications", perm: "system:application" },
    { key: "role", label: "角色总数", value: data?.roleCount, icon: Shield, to: "/system/roles", perm: "system:role" },
  ]

  const quickActions = [
    { label: "新建租户", to: "/system/tenants", icon: Plus, perm: "system:tenant" },
    { label: "新建用户", to: "/system/users", icon: Users, perm: "system:user" },
    { label: "新建角色", to: "/system/roles", icon: Shield, perm: "system:role" },
    { label: "新建 OAuth 客户端", to: "/system/oauth/clients", icon: KeyRound, perm: "system:oauth" },
  ].filter((a) => can(a.perm))

  return (
    <div className="space-y-6">
      {/* 欢迎横幅(右侧 sm+ 分栏:快捷入口 + 高频访问;移动端在横幅内换行堆叠) */}
      <Card className="overflow-hidden border-primary/20 bg-gradient-to-br from-primary/8 via-card to-card">
        <CardContent className="flex flex-col gap-4 p-5 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex min-w-0 items-center gap-4">
            <div className="flex size-12 shrink-0 items-center justify-center rounded-2xl bg-primary/15 text-xl font-bold text-primary">
              {(displayName || "U").slice(0, 1).toUpperCase()}
            </div>
            <div className="min-w-0">
              <p className="truncate text-lg font-semibold tracking-tight">
                {greeting()},{displayName}
              </p>
              <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                <Badge variant="secondary" className="gap-1">
                  <Landmark className="size-3" />
                  {profile?.tenant.name ?? "—"}
                </Badge>
                <Badge variant="secondary" className="gap-1">
                  <Building2 className="size-3" />
                  {profile?.organization.name ?? "—"}
                </Badge>
                <span className="text-xs text-muted-foreground">
                  {data ? `今日登录 ${formatNumber(data.todayLoginCount)} 次` : ""}
                </span>
              </div>
            </div>
          </div>
          <div className="flex min-w-0 flex-col gap-3 sm:items-end">
            <div className="flex flex-wrap gap-2">
              {quickActions.map((a) => (
                <Button key={a.to} asChild size="sm" variant={a.label.includes("租户") ? "default" : "outline"}>
                  <Link to={a.to}>
                    <a.icon className="size-4" />
                    {a.label}
                  </Link>
                </Button>
              ))}
            </div>
            {/* 高频访问(横幅内轻量胶囊区;无符合项时整块不渲染,横幅保持原状) */}
            {topVisits.length > 0 && (
              <div className="flex flex-col gap-1.5 sm:items-end">
                <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
                  <Clock className="size-3.5 shrink-0 text-primary/70" aria-hidden />
                  <span className="font-medium text-foreground/80">高频访问</span>
                  <span aria-hidden>·</span>
                  <span>按访问频次</span>
                </p>
                <nav aria-label="高频访问" className="flex flex-wrap gap-1.5">
                  {topVisits.map((v) => (
                    <button
                      key={v.path}
                      type="button"
                      onClick={() => navigate(v.path)}
                      title={v.title}
                      className="flex items-center gap-2 rounded-full border border-border/40 bg-card/50 py-1.5 pl-2.5 pr-3 text-xs outline-none transition-colors hover:border-primary/40 hover:bg-primary/10 focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      <AppIcon name={v.icon} size={14} className="shrink-0 text-primary" />
                      <span className="max-w-32 truncate font-medium">{v.title}</span>
                      <span className="shrink-0 tabular-nums text-muted-foreground">{v.count} 次</span>
                    </button>
                  ))}
                </nav>
              </div>
            )}
          </div>
        </CardContent>
      </Card>

      {/* 统计卡 */}
      {overviewQuery.isError ? (
        <ErrorState
          message={overviewQuery.error instanceof Error ? overviewQuery.error.message : undefined}
          onRetry={() => overviewQuery.refetch()}
        />
      ) : overviewQuery.isLoading ? (
        <StatSkeleton />
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
          {stats.map((s) => {
            const trend = MOCK_TREND[s.key] ?? 0
            const up = trend >= 0
            return (
              <Card key={s.key} className="relative overflow-hidden transition-shadow hover:shadow-md">
                <div
                  aria-hidden
                  className="absolute -right-6 -top-6 flex size-24 items-center justify-center rounded-full bg-primary/8 text-primary/15"
                >
                  <s.icon className="size-12" />
                </div>
                <CardHeader className="pb-2">
                  <CardDescription>{s.label}</CardDescription>
                  <CardTitle className="text-3xl font-bold tabular-nums tracking-tight">
                    {s.value === undefined ? "—" : formatNumber(s.value)}
                  </CardTitle>
                </CardHeader>
                <CardContent>
                  <div className="flex items-center justify-between">
                    <span
                      className={cn(
                        "flex items-center gap-1 text-xs",
                        up ? "text-primary" : "text-destructive"
                      )}
                    >
                      {up ? <TrendingUp className="size-3.5" /> : <TrendingDown className="size-3.5" />}
                      较昨日 {up ? "+" : ""}
                      {trend}
                    </span>
                    <Button asChild variant="ghost" size="sm" className="h-7 gap-1 px-2 text-xs text-muted-foreground">
                      <Link to={s.to}>
                        明细
                        <ArrowRight className="size-3.5" />
                      </Link>
                    </Button>
                  </div>
                </CardContent>
              </Card>
            )
          })}
        </div>
      )}

      {/* 图表 */}
      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-base">近 7 日登录趋势</CardTitle>
            <CardDescription>平台整体登录次数(含失败)</CardDescription>
          </CardHeader>
          <CardContent className="h-64">
            {overviewQuery.isLoading ? (
              <ChartSkeleton />
            ) : overviewQuery.isError ? (
              <div className="flex h-full items-center justify-center">
                <ErrorState onRetry={() => overviewQuery.refetch()} />
              </div>
            ) : (
              <ResponsiveContainer width="100%" height="100%">
                <AreaChart data={data?.loginTrend7d ?? []} margin={{ top: 8, right: 8, left: -18, bottom: 0 }}>
                  <defs>
                    <linearGradient id="loginGradient" x1="0" y1="0" x2="0" y2="1">
                      <stop offset="0%" stopColor="var(--chart-1)" stopOpacity={0.35} />
                      <stop offset="100%" stopColor="var(--chart-1)" stopOpacity={0.02} />
                    </linearGradient>
                  </defs>
                  <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" vertical={false} />
                  <XAxis dataKey="date" tick={{ fontSize: 11, fill: "var(--muted-foreground)" }} tickLine={false} axisLine={false} />
                  <YAxis tick={{ fontSize: 11, fill: "var(--muted-foreground)" }} tickLine={false} axisLine={false} width={44} allowDecimals={false} />
                  <ReTooltip content={<ChartTip />} cursor={{ stroke: "var(--border)" }} />
                  <Area type="monotone" dataKey="count" name="登录次数" stroke="var(--chart-1)" strokeWidth={2} fill="url(#loginGradient)" />
                </AreaChart>
              </ResponsiveContainer>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-base">近 7 日审计事件</CardTitle>
            <CardDescription>关键安全与变更审计条数</CardDescription>
          </CardHeader>
          <CardContent className="h-64">
            {overviewQuery.isLoading ? (
              <ChartSkeleton />
            ) : overviewQuery.isError ? (
              <div className="flex h-full items-center justify-center">
                <ErrorState onRetry={() => overviewQuery.refetch()} />
              </div>
            ) : (
              <ResponsiveContainer width="100%" height="100%">
                <BarChart data={data?.auditTrend7d ?? []} margin={{ top: 8, right: 8, left: -18, bottom: 0 }}>
                  <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" vertical={false} />
                  <XAxis dataKey="date" tick={{ fontSize: 11, fill: "var(--muted-foreground)" }} tickLine={false} axisLine={false} />
                  <YAxis tick={{ fontSize: 11, fill: "var(--muted-foreground)" }} tickLine={false} axisLine={false} width={44} allowDecimals={false} />
                  <ReTooltip content={<ChartTip />} cursor={{ fill: "var(--muted)", opacity: 0.4 }} />
                  <Bar dataKey="count" name="审计条数" fill="var(--chart-2)" radius={[4, 4, 0, 0]} maxBarSize={36} />
                </BarChart>
              </ResponsiveContainer>
            )}
          </CardContent>
        </Card>
      </div>

      {/* 最近登录 */}
      <Card>
        <CardHeader className="flex flex-row items-center justify-between pb-3">
          <div>
            <CardTitle className="text-base">最近登录</CardTitle>
            <CardDescription>平台最近的成功登录记录</CardDescription>
          </div>
          <Button asChild variant="ghost" size="sm" className="gap-1 text-muted-foreground">
            <Link to="/system/audit-logs">
              全部记录
              <ArrowRight className="size-3.5" />
            </Link>
          </Button>
        </CardHeader>
        <CardContent className="px-0 pb-0">
          {overviewQuery.isLoading ? (
            <div className="space-y-2 px-6 pb-6">
              {Array.from({ length: 5 }).map((_, i) => (
                <Skeleton key={i} className="h-10" />
              ))}
            </div>
          ) : overviewQuery.isError ? (
            <div className="px-6 pb-6">
              <ErrorState onRetry={() => overviewQuery.refetch()} />
            </div>
          ) : (data?.recentLogins ?? []).length === 0 ? (
            <p className="px-6 pb-6 pt-2 text-center text-sm text-muted-foreground">暂无登录记录</p>
          ) : (
            <div className="table-scroll max-h-[26rem] overflow-auto scroll-area">
            <Table>
              <TableHeader className="sticky top-0 z-10 bg-card">
                <TableRow className="hover:bg-transparent">
                  <TableHead className="pl-6 text-xs uppercase tracking-wider text-muted-foreground">用户</TableHead>
                  <TableHead className="text-xs uppercase tracking-wider text-muted-foreground">组织</TableHead>
                  <TableHead className="text-xs uppercase tracking-wider text-muted-foreground">IP 地址</TableHead>
                  <TableHead className="pr-6 text-right text-xs uppercase tracking-wider text-muted-foreground">时间</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {data!.recentLogins.map((login, i) => (
                  <TableRow key={`${login.username}-${i}`}>
                    <TableCell className="pl-6 font-medium">{login.username}</TableCell>
                    <TableCell className="text-muted-foreground">{login.organizationName ?? "—"}</TableCell>
                    <TableCell className="font-mono text-xs text-muted-foreground">{login.ip ?? "—"}</TableCell>
                    <TableCell className="pr-6 text-right text-xs text-muted-foreground">
                      {formatRelative(login.createdAt)}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}

/* ============ 个人工作台(无平台监控权限的降级视图) ============ */

function PersonalDashboard({ displayName, permissionCount }: { displayName: string; permissionCount: number }) {
  const profile = useAuthStore((s) => s.profile)
  const navigate = useNavigate()
  const setCurrentApplication = useAuthStore((s) => s.setCurrentApplication)
  const currentApplicationId = useAuthStore((s) => s.currentApplicationId)
  const queryClient = useQueryClient()
  const recentVisits = useRecentVisits()
  /* 已授权路径集合(与平台视图同一 hook,菜单缓存全局共享) */
  const { allowedPaths, ready: pathsReady } = useAuthorizedPaths()

  const apps = profile?.applications ?? []
  const memberships = profile?.memberships ?? []
  const currentOrgId = profile?.organization.id
  /* 展示层过滤:记录照存 localStorage,仅渲染已授权路径(未就绪时为空,不闪现无权项) */
  const visibleVisits = React.useMemo(
    () => (pathsReady ? recentVisits.filter((v) => allowedPaths.has(v.path)) : []),
    [recentVisits, allowedPaths, pathsReady]
  )
  const hasVisits = visibleVisits.length > 0

  /** 高频访问 Top 4(共用 pickTopVisits;无符合项时保持原两项布局) */
  const topVisits = React.useMemo(
    () => pickTopVisits(recentVisits, 4, { allowedPaths, ready: pathsReady }),
    [recentVisits, allowedPaths, pathsReady]
  )

  /** 通知中心入口:有 system 应用(即 /system/* 路由可达)才跳通知中心,否则回个人中心 */
  const canOpenNotifications = apps.some(
    (a) => a.basePath && a.basePath !== "/" && "/system/notifications".startsWith(`${a.basePath}/`)
  )

  const myStats = [
    {
      key: "apps",
      label: "可用应用",
      value: apps.length,
      icon: LayoutGrid,
      hint: "授权给我的应用",
    },
    {
      key: "orgs",
      label: "所属组织",
      value: memberships.length,
      icon: Building2,
      hint: "可切换的组织上下文",
    },
    {
      key: "perms",
      label: "功能权限",
      value: permissionCount,
      icon: ShieldCheck,
      hint: "当前上下文有效权限点",
    },
  ]

  const enterApp = async (appId: string) => {
    setCurrentApplication(appId)
    try {
      const menus = await queryClient.fetchQuery({
        queryKey: ["auth-menus", appId],
        queryFn: async () =>
          sortMenuTree(asArray<MenuNode>(await authApi.menus(appId), "menus")),
        staleTime: 30_000,
      })
      const firstPath = (function walk(nodes: MenuNode[]): string | null {
        for (const n of nodes) {
          if (n.type !== "BUTTON" && n.path) return n.path
          if (n.children?.length) {
            const found = walk(n.children)
            if (found) return found
          }
        }
        return null
      })(menus)
      if (firstPath) navigate(firstPath)
    } catch {
      /* 菜单拉取失败时仅切换侧栏,不跳转 */
    }
  }

  /** 跳转到任意已授权路径:目标属于其它应用时同步切换应用上下文 */
  const goTo = (path: string) => {
    const app = apps.find(
      (a) =>
        a.basePath &&
        a.basePath !== "/" &&
        (path === a.basePath || path.startsWith(`${a.basePath}/`))
    )
    if (app && app.id !== currentApplicationId) setCurrentApplication(app.id)
    navigate(path)
  }

  /* ---------- 最近访问卡片 ---------- */
  const recentVisitsCard = (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          <History className="size-4 text-primary" />
          最近访问
        </CardTitle>
        <CardDescription>最近浏览过的页面,点击快速返回</CardDescription>
      </CardHeader>
      <CardContent>
        {visibleVisits.length === 0 ? (
          <p className="flex items-center gap-2 rounded-xl border border-dashed py-6 text-center text-sm text-muted-foreground">
            <span className="flex-1">暂无访问记录,从下方我的应用开始探索</span>
          </p>
        ) : (
          <div className="scroll-area -mx-1 flex gap-3 overflow-x-auto px-1 pb-1">
            {visibleVisits.map((v) => (
              <button
                key={v.path}
                type="button"
                onClick={() => goTo(v.path)}
                title={v.title}
                className="group flex w-44 shrink-0 flex-col gap-2.5 rounded-xl border bg-card p-3.5 text-left outline-none transition-all hover:-translate-y-0.5 hover:border-primary/40 hover:shadow-md focus-visible:ring-2 focus-visible:ring-ring"
              >
                <span className="flex items-center gap-2.5">
                  <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary transition-colors group-hover:bg-primary/15">
                    <AppIcon name={v.icon} size={18} />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-medium">{v.title}</span>
                    <span className="block truncate text-xs text-muted-foreground">
                      {formatRelative(new Date(v.at))}
                    </span>
                  </span>
                </span>
                <span className="truncate font-mono text-[10px] text-muted-foreground/50">{v.path}</span>
              </button>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  )

  /* ---------- 快捷操作卡片(固定入口 + 高频访问个性化) ---------- */
  const quickActionsCard = (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          <Sparkles className="size-4 text-primary" />
          快捷操作
        </CardTitle>
        {topVisits.length > 0 && (
          <CardDescription className="flex items-center gap-1.5">
            <TrendingUp className="size-3 shrink-0 text-primary/70" aria-hidden />
            含高频访问 · 按您的访问频次推荐
          </CardDescription>
        )}
      </CardHeader>
      <CardContent className="grid gap-3 sm:grid-cols-2">
        <button
          type="button"
          onClick={() => goTo("/profile")}
          className="group flex items-center gap-3 rounded-xl border bg-card p-3.5 text-left outline-none transition-all hover:-translate-y-0.5 hover:border-primary/40 hover:shadow-md focus-visible:ring-2 focus-visible:ring-ring"
        >
          <span className="flex size-10 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary transition-colors group-hover:bg-primary/15">
            <User className="size-5" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block text-sm font-semibold">个人中心</span>
            <span className="block truncate text-xs text-muted-foreground">资料与登录信息</span>
          </span>
          <ChevronRight className="size-4 shrink-0 text-muted-foreground/50 transition-transform group-hover:translate-x-0.5 group-hover:text-primary" />
        </button>
        <button
          type="button"
          onClick={() => goTo(canOpenNotifications ? "/system/notifications" : "/profile")}
          className="group flex items-center gap-3 rounded-xl border bg-card p-3.5 text-left outline-none transition-all hover:-translate-y-0.5 hover:border-primary/40 hover:shadow-md focus-visible:ring-2 focus-visible:ring-ring"
        >
          <span className="flex size-10 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary transition-colors group-hover:bg-primary/15">
            <Bell className="size-5" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block text-sm font-semibold">通知中心</span>
            <span className="block truncate text-xs text-muted-foreground">
              {canOpenNotifications ? "查看我的通知消息" : "在个人中心查看消息"}
            </span>
          </span>
          <ChevronRight className="size-4 shrink-0 text-muted-foreground/50 transition-transform group-hover:translate-x-0.5 group-hover:text-primary" />
        </button>
        {/* 高频访问(与固定项同款按钮卡;图标/标题来自访问记录,描述位展示频次) */}
        {topVisits.map((v) => (
          <button
            key={v.path}
            type="button"
            onClick={() => goTo(v.path)}
            title={v.title}
            className="group flex items-center gap-3 rounded-xl border bg-card p-3.5 text-left outline-none transition-all hover:-translate-y-0.5 hover:border-primary/40 hover:shadow-md focus-visible:ring-2 focus-visible:ring-ring"
          >
            <span className="flex size-10 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary transition-colors group-hover:bg-primary/15">
              <AppIcon name={v.icon} size={20} />
            </span>
            <span className="min-w-0 flex-1">
              <span className="block truncate text-sm font-semibold">{v.title}</span>
              <span className="block truncate text-xs text-muted-foreground">
                访问 <span className="tabular-nums">{v.count}</span> 次
              </span>
            </span>
            <ChevronRight className="size-4 shrink-0 text-muted-foreground/50 transition-transform group-hover:translate-x-0.5 group-hover:text-primary" />
          </button>
        ))}
      </CardContent>
    </Card>
  )

  /* ---------- 我的应用卡片 ---------- */
  const appsCard = (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          <LayoutGrid className="size-4 text-primary" />
          我的应用
        </CardTitle>
        <CardDescription>租户与组织授权范围内可进入的应用</CardDescription>
      </CardHeader>
      <CardContent>
        {apps.length === 0 ? (
          <p className="py-8 text-center text-sm text-muted-foreground">
            暂无可用应用,请联系管理员授权
          </p>
        ) : (
          <div className={cn("grid gap-3 sm:grid-cols-2", !hasVisits && "xl:grid-cols-3")}>
            {apps.map((app) => (
              <button
                key={app.id}
                type="button"
                onClick={() => void enterApp(app.id)}
                className="group flex items-center gap-3 rounded-xl border bg-card p-3.5 text-left outline-none transition-all hover:-translate-y-0.5 hover:border-primary/40 hover:shadow-md focus-visible:ring-2 focus-visible:ring-ring"
              >
                <span className="flex size-11 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary transition-colors group-hover:bg-primary/15">
                  <AppIcon name={app.icon} size={22} />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-semibold">{app.name}</span>
                  <span className="block truncate font-mono text-[11px] text-muted-foreground">
                    {app.code} · {app.basePath}
                  </span>
                </span>
                <ChevronRight className="size-4 shrink-0 text-muted-foreground/50 transition-transform group-hover:translate-x-0.5 group-hover:text-primary" />
              </button>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  )

  /* ---------- 我的组织卡片 ---------- */
  const orgsCard = (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          <Map className="size-4 text-primary" />
          我的组织
        </CardTitle>
        <CardDescription>当前登录身份可进入的组织</CardDescription>
      </CardHeader>
      <CardContent className="space-y-2.5">
        {memberships.map((m) => {
          const active = m.organizationId === currentOrgId
          return (
            <div
              key={`${m.tenantId}-${m.organizationId}`}
              className={cn(
                "flex items-start gap-3 rounded-xl border p-3 transition-colors",
                active ? "border-primary/40 bg-primary/5" : "bg-card hover:bg-accent/50"
              )}
            >
              <span
                className={cn(
                  "mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg",
                  active ? "bg-primary/15 text-primary" : "bg-muted text-muted-foreground"
                )}
              >
                <Building2 className="size-4" />
              </span>
              <div className="min-w-0 flex-1">
                <p className="flex flex-wrap items-center gap-1.5 text-sm font-medium">
                  {m.organizationName}
                  {active && (
                    <Badge variant="outline" className="h-5 gap-1 border-primary/30 px-1.5 text-[10px] text-primary">
                      当前
                    </Badge>
                  )}
                </p>
                <p className="mt-0.5 truncate text-xs text-muted-foreground">{m.organizationPath}</p>
                {m.departmentName && (
                  <p className="mt-1 text-[11px] text-muted-foreground/70">部门:{m.departmentName}</p>
                )}
              </div>
            </div>
          )
        })}
        <Button asChild variant="ghost" size="sm" className="w-full gap-1 text-muted-foreground">
          <Link to="/profile">
            在个人中心管理
            <ArrowRight className="size-3.5" />
          </Link>
        </Button>
      </CardContent>
    </Card>
  )

  return (
    <div className="space-y-6">
      {/* 欢迎横幅 */}
      <Card className="overflow-hidden border-primary/20 bg-gradient-to-br from-primary/8 via-card to-card">
        <CardContent className="flex flex-col gap-4 p-5 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex items-center gap-4">
            <div className="flex size-12 shrink-0 items-center justify-center rounded-2xl bg-primary/15 text-xl font-bold text-primary">
              {(displayName || "U").slice(0, 1).toUpperCase()}
            </div>
            <div>
              <p className="text-lg font-semibold tracking-tight">
                {greeting()},{displayName}
              </p>
              <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                <Badge variant="secondary" className="gap-1">
                  <Landmark className="size-3" />
                  {profile?.tenant.name ?? "—"}
                </Badge>
                <Badge variant="secondary" className="gap-1">
                  <Building2 className="size-3" />
                  {profile?.organization.name ?? "—"}
                </Badge>
              </div>
            </div>
          </div>
          <Button asChild size="sm" variant="outline">
            <Link to="/profile">
              个人中心
              <ChevronRight className="size-4" />
            </Link>
          </Button>
        </CardContent>
      </Card>

      {/* 个人统计 */}
      <div className="grid gap-4 sm:grid-cols-3">
        {myStats.map((s) => (
          <Card key={s.key} className="relative overflow-hidden transition-shadow hover:shadow-md">
            <div
              aria-hidden
              className="absolute -right-6 -top-6 flex size-24 items-center justify-center rounded-full bg-primary/8 text-primary/15"
            >
              <s.icon className="size-12" />
            </div>
            <CardHeader className="pb-2">
              <CardDescription>{s.label}</CardDescription>
              <CardTitle className="bg-gradient-to-br from-primary to-primary/60 bg-clip-text text-3xl font-bold tabular-nums tracking-tight text-transparent">
                {formatNumber(s.value)}
              </CardTitle>
            </CardHeader>
            <CardContent>
              <span className="text-xs text-muted-foreground">{s.hint}</span>
            </CardContent>
          </Card>
        ))}
      </div>

      {/* 最近访问(整行) */}
      {hasVisits && recentVisitsCard}

      {/* 我的应用 + 快捷操作/我的组织:有最近访问时两列,无则我的应用占满整行 */}
      <div className={cn("grid gap-4", hasVisits ? "lg:grid-cols-5" : "lg:grid-cols-1")}>
        <div className={cn(hasVisits && "lg:col-span-3")}>{appsCard}</div>
        {hasVisits ? (
          <div className="space-y-4 lg:col-span-2">
            {quickActionsCard}
            {orgsCard}
          </div>
        ) : (
          <div className="grid gap-4 lg:grid-cols-2">
            {quickActionsCard}
            {orgsCard}
          </div>
        )}
      </div>
    </div>
  )
}
