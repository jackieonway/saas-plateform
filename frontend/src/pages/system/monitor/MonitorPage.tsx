import * as React from "react"
import { useQuery } from "@tanstack/react-query"
import {
  Activity,
  ArrowDownWideNarrow,
  ArrowUpWideNarrow,
  BarChart3,
  BookmarkCheck,
  Crown,
  Database,
  Download,
  Globe,
  KeyRound,
  Landmark,
  ListFilter,
  Loader2,
  LogIn,
  Network,
  Server,
  ShieldCheck,
  Users,
} from "lucide-react"
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  LabelList,
  ResponsiveContainer,
  Tooltip as ReTooltip,
  XAxis,
  YAxis,
} from "recharts"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { PageHeader } from "@/components/layout/PageHeader"
import { StatusBadge } from "@/components/StatusBadge"
import { CardGridSkeleton, EmptyState, ErrorState, StatSkeleton } from "@/components/states"
import { monitorApi, type TenantInsight } from "@/api/monitor"
import { usePermissionsQuery } from "@/hooks/use-auth-data"
import { useAuthStore } from "@/stores/auth"
import { hasPlatformContext } from "@/lib/platform-context"
import { durationHuman, formatBytes, formatDateTime, formatNumber } from "@/lib/format"
import { toast } from "sonner"
import { cn } from "@/lib/utils"
import type { MonitorSystemInfo } from "@/api/types"

export default function MonitorPage() {
  const isPlatformContext = useAuthStore((s) => hasPlatformContext(s.profile))
  return (
    <div className="space-y-6">
      <PageHeader
        title="平台监控"
        description={isPlatformContext ? "后端服务健康状态、数据库连通性与租户级洞察" : "当前组织及下级组织范围内的数据洞察"}
        icon={<Activity className="size-5" />}
      />

      <Tabs key={isPlatformContext ? "platform" : "scoped"} defaultValue={isPlatformContext ? "status" : "tenants"}>
        <TabsList className="mb-4">
          {isPlatformContext && <TabsTrigger value="status" className="gap-1.5">
            <Server className="size-4" />
            平台状态
          </TabsTrigger>}
          <TabsTrigger value="tenants" className="gap-1.5">
            <Landmark className="size-4" />
            {isPlatformContext ? "租户洞察" : "组织洞察"}
          </TabsTrigger>
        </TabsList>
        {isPlatformContext && <TabsContent value="status" className="mt-0">
          <PlatformStatusTab />
        </TabsContent>}
        <TabsContent value="tenants" className="mt-0">
          <TenantInsightsTab platformContext={isPlatformContext} />
        </TabsContent>
      </Tabs>
    </div>
  )
}

/* ============ Tab 1: 平台状态 (原有内容) ============ */

function PlatformStatusTab() {
  const healthQuery = useQuery({
    queryKey: ["health"],
    queryFn: () => monitorApi.health(),
    refetchInterval: 30_000,
    retry: 1,
  })
  /* 平台监控系统信息(服务进程/数据库, 权限 system:monitor): 旧后端 404 时卡片静默展示错误态, 不阻塞其余内容 */
  const systemQuery = useQuery({
    queryKey: ["monitor-system"],
    queryFn: () => monitorApi.system(),
    staleTime: 30_000,
    retry: 1,
  })

  const health = healthQuery.data

  return (
    <div className="space-y-6">
      {/* 状态卡 */}
      <div className="grid gap-4 sm:grid-cols-3">
        <StatusCard
          title="后端服务"
          icon={<Server className="size-5" />}
          isLoading={healthQuery.isLoading}
          isError={healthQuery.isError}
          onRetry={() => healthQuery.refetch()}
          error={healthQuery.error}
          value={health?.status === "UP" ? "运行中 (UP)" : health ? `${health.status}` : undefined}
          ok={health?.status === "UP"}
        />
        <StatusCard
          title="数据库"
          icon={<Database className="size-5" />}
          isLoading={healthQuery.isLoading}
          isError={healthQuery.isError}
          onRetry={() => healthQuery.refetch()}
          error={healthQuery.error}
          value={health ? `${health.database}${health.database === "UP" ? " (UP)" : ""}` : undefined}
          ok={health?.database === "UP"}
          warnWhen={health?.database === "PENDING"}
        />
        <StatusCard
          title="后端版本"
          icon={<Globe className="size-5" />}
          isLoading={healthQuery.isLoading}
          isError={healthQuery.isError}
          onRetry={() => healthQuery.refetch()}
          error={healthQuery.error}
          value={health ? `v${health.version}` : undefined}
          ok
          mono
        />
      </div>

      {/* 系统信息: 服务进程 / 数据库 (GET /api/monitor/system) */}
      <div className="grid gap-4 lg:grid-cols-2">
        <ServiceInfoCard
          data={systemQuery.data}
          isLoading={systemQuery.isLoading}
          isError={systemQuery.isError}
          error={systemQuery.error}
          onRetry={() => systemQuery.refetch()}
        />
        <DatabaseInfoCard
          data={systemQuery.data}
          isLoading={systemQuery.isLoading}
          isError={systemQuery.isError}
          error={systemQuery.error}
          onRetry={() => systemQuery.refetch()}
        />
      </div>


    </div>
  )
}

/* ============ 系统信息卡: 服务进程 / 数据库 (GET /api/monitor/system) ============ */

/** 信息行: 左标签右值(dl 语义, divide-y 分隔) */
function InfoRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-4 py-2">
      <dt className="shrink-0 text-xs text-muted-foreground">{label}</dt>
      <dd className="min-w-0 truncate text-right text-sm">{children}</dd>
    </div>
  )
}

/** 超长截断(数据库产品串等) */
function truncateText(s: string, max: number): string {
  return s.length > max ? `${s.slice(0, max)}…` : s
}

function ServiceInfoCard({
  data,
  isLoading,
  isError,
  error,
  onRetry,
}: {
  data?: MonitorSystemInfo
  isLoading: boolean
  isError: boolean
  error?: unknown
  onRetry: () => void
}) {
  const svc = data?.service
  return (
    <Card className="transition-shadow hover:shadow-md">
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          <Server className="size-4 text-primary" />
          服务信息
        </CardTitle>
        <CardDescription>GET /api/monitor/system · 后端进程运行指标</CardDescription>
      </CardHeader>
      <CardContent>
        {isLoading ? (
          <div className="space-y-3">
            {Array.from({ length: 5 }).map((_, i) => (
              <Skeleton key={i} className="h-5" style={{ width: `${92 - i * 7}%` }} />
            ))}
          </div>
        ) : isError ? (
          <ErrorState
            className="border-0 bg-transparent py-6"
            message={error instanceof Error ? error.message : undefined}
            onRetry={onRetry}
          />
        ) : (
          <dl className="divide-y">
            <InfoRow label="服务名称">
              <span className="font-medium">{svc?.name ?? "—"}</span>
              {svc?.version && (
                <Badge variant="outline" className="ml-2 border-primary/30 bg-primary/10 font-mono text-[10px] text-primary">
                  v{svc.version}
                </Badge>
              )}
            </InfoRow>
            <InfoRow label="进程 PID">
              <span className="font-mono text-xs tabular-nums">{svc?.pid ?? "—"}</span>
            </InfoRow>
            <InfoRow label="启动时间">
              <span className="text-xs text-muted-foreground">{formatDateTime(svc?.startedAt ?? null)}</span>
            </InfoRow>
            <InfoRow label="运行时长">
              <span className="text-xs tabular-nums">{durationHuman(svc?.uptimeSeconds ?? null)}</span>
            </InfoRow>
            <InfoRow label="内存 RSS">
              <span className="text-xs tabular-nums">
                {svc?.memoryRssMb != null ? `${formatNumber(svc.memoryRssMb)} MB` : "—"}
              </span>
            </InfoRow>
          </dl>
        )}
      </CardContent>
    </Card>
  )
}

function DatabaseInfoCard({
  data,
  isLoading,
  isError,
  error,
  onRetry,
}: {
  data?: MonitorSystemInfo
  isLoading: boolean
  isError: boolean
  error?: unknown
  onRetry: () => void
}) {
  const db = data?.database
  const conn = db?.connections ?? null
  /* 核心表按大小降序 */
  const tables = React.useMemo(
    () => [...(db?.tables ?? [])].sort((a, b) => (b.sizeBytes ?? 0) - (a.sizeBytes ?? 0)),
    [db?.tables]
  )
  return (
    <Card className="transition-shadow hover:shadow-md">
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          <Database className="size-4 text-primary" />
          数据库信息
        </CardTitle>
        <CardDescription>数据库产品、连接池与核心表规模</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {isLoading ? (
          <div className="space-y-3">
            {Array.from({ length: 4 }).map((_, i) => (
              <Skeleton key={i} className="h-5" style={{ width: `${92 - i * 9}%` }} />
            ))}
            <Skeleton className="h-28" />
          </div>
        ) : isError ? (
          <ErrorState
            className="border-0 bg-transparent py-6"
            message={error instanceof Error ? error.message : undefined}
            onRetry={onRetry}
          />
        ) : (
          <>
            <dl className="divide-y">
              <InfoRow label="数据库产品">
                {db?.product ? (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span tabIndex={0} className="cursor-help font-medium" aria-label={db.product}>
                        {truncateText(db.product, 32)}
                      </span>
                    </TooltipTrigger>
                    <TooltipContent className="max-w-sm">
                      <p className="break-all">{db.product}</p>
                    </TooltipContent>
                  </Tooltip>
                ) : (
                  "—"
                )}
              </InfoRow>
              <InfoRow label="服务端启动">
                <span className="text-xs text-muted-foreground">{formatDateTime(db?.serverStartedAt ?? null)}</span>
              </InfoRow>
              <InfoRow label="数据库大小">
                <span className="text-xs tabular-nums">{formatBytes(db?.databaseSizeBytes ?? null)}</span>
              </InfoRow>
              <InfoRow label="连接数">
                <span className="flex flex-wrap items-center justify-end gap-1">
                  <Badge variant="outline" className="border-primary/30 bg-primary/10 text-primary tabular-nums">
                    共 {conn?.total ?? "—"}
                  </Badge>
                  <Badge
                    variant="outline"
                    className="border-emerald-500/30 bg-emerald-500/10 text-emerald-600 tabular-nums dark:text-emerald-400"
                  >
                    活跃 {conn?.active ?? "—"}
                  </Badge>
                  <Badge variant="outline" className="border-border bg-muted/40 text-muted-foreground tabular-nums">
                    空闲 {conn?.idle ?? "—"}
                  </Badge>
                </span>
              </InfoRow>
              <InfoRow label="连接池大小">
                <span className="text-xs tabular-nums">{db?.poolSize ?? "—"}</span>
              </InfoRow>
            </dl>
            {/* 核心表清单 */}
            <div>
              <p className="mb-2 text-xs font-medium uppercase tracking-wider text-muted-foreground">
                核心表 · 按大小降序
              </p>
              {tables.length === 0 ? (
                <p className="rounded-lg border border-dashed py-5 text-center text-xs text-muted-foreground">
                  暂无表统计数据
                </p>
              ) : (
                <div className="max-h-56 overflow-y-auto scroll-area rounded-lg border">
                  <Table>
                    <TableHeader className="sticky top-0 z-10 bg-card">
                      <TableRow className="hover:bg-transparent">
                        <TableHead className="pl-3 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">表名</TableHead>
                        <TableHead className="text-right text-[10px] font-medium uppercase tracking-wider text-muted-foreground">行数</TableHead>
                        <TableHead className="pr-3 text-right text-[10px] font-medium uppercase tracking-wider text-muted-foreground">大小</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {tables.map((t) => (
                        <TableRow key={t.name}>
                          <TableCell className="py-1.5 pl-3 font-mono text-xs">{t.name}</TableCell>
                          <TableCell className="py-1.5 text-right text-xs tabular-nums text-muted-foreground">
                            {formatNumber(t.rows ?? null)}
                          </TableCell>
                          <TableCell className="py-1.5 pr-3 text-right text-xs tabular-nums text-muted-foreground">
                            {formatBytes(t.sizeBytes ?? null)}
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                </div>
              )}
            </div>
          </>
        )}
      </CardContent>
    </Card>
  )
}

/* ============ Tab 2: 租户洞察 ============ */

function TenantInsightsTab({ platformContext }: { platformContext: boolean }) {
  const insightsQuery = useQuery({
    queryKey: ["monitor-tenant-insights"],
    queryFn: () => monitorApi.tenantInsights(),
    staleTime: 30_000,
    retry: 1,
  })

  /* 导出权限:system:monitor */
  const permissionsQuery = usePermissionsQuery()
  const permissions = new Set(permissionsQuery.data?.permissions ?? [])
  const canExport = permissions.has("system:monitor")

  const [exporting, setExporting] = React.useState(false)
  /** 导出租户洞察 CSV(与审计日志导出同款交互) */
  const exportCsv = async () => {
    setExporting(true)
    try {
      const { blob, filename } = await monitorApi.exportTenantInsights()
      const url = URL.createObjectURL(blob)
      const a = document.createElement("a")
      a.href = url
      a.download = filename
      document.body.appendChild(a)
      a.click()
      a.remove()
      URL.revokeObjectURL(url)
      toast.success(`租户洞察 CSV 已开始下载(${filename})`)
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "导出失败,请稍后重试")
    } finally {
      setExporting(false)
    }
  }

  const insights = insightsQuery.data?.insights ?? []
  const totals = React.useMemo(() => {
    return {
      tenants: insights.length,
      users: insights.reduce((s, t) => s + (t.users || 0), 0),
      logins7d: insights.reduce((s, t) => s + (t.logins7d || 0), 0),
      activePlatformSessions: insights.reduce((s, t) => s + (t.activePlatformSessions || 0), 0),
    }
  }, [insights])

  const SUMMARY = [
    { key: "tenants", label: platformContext ? "租户总数" : "当前租户", value: totals.tenants, icon: Landmark,
      hint: platformContext ? "含已禁用租户" : "当前组织及下级组织范围" },
    { key: "users", label: "用户总数", value: totals.users, icon: Users, hint: "按成员关系归属去重" },
    { key: "logins", label: "近 7 天登录", value: totals.logins7d, icon: LogIn,
      hint: platformContext ? "全部租户登录次数" : "当前组织及下级组织登录次数" },
    { key: "tokens", label: "活跃平台会话", value: totals.activePlatformSessions, icon: KeyRound, hint: "未撤销且未过期的后台登录会话" },
  ]

  return (
    <div className="space-y-6">
      {/* 汇总区标题行 + 导出按钮 */}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h3 className="text-sm font-semibold">规模与活跃度总览</h3>
          <p className="mt-0.5 text-xs text-muted-foreground">{platformContext
            ? "各租户用户 / 组织 / 角色 / 近 7 天登录与活跃平台会话汇总"
            : "当前组织及下级组织的用户、角色、近 7 天登录与活跃平台会话汇总"}</p>
        </div>
        {canExport && (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="outline" size="sm" onClick={exportCsv} disabled={exporting}>
                {exporting ? (
                  <Loader2 className="size-4 animate-spin" />
                ) : (
                  <Download className="size-4" />
                )}
                导出 CSV
              </Button>
            </TooltipTrigger>
            <TooltipContent>{platformContext ? "导出全部租户洞察数据 (CSV)" : "导出当前组织及下级组织洞察数据 (CSV)"}</TooltipContent>
          </Tooltip>
        )}
      </div>

      {/* 汇总统计卡 */}
      {insightsQuery.isError ? (
        <ErrorState
          message={insightsQuery.error instanceof Error ? insightsQuery.error.message : undefined}
          onRetry={() => insightsQuery.refetch()}
        />
      ) : insightsQuery.isLoading ? (
        <StatSkeleton />
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
          {SUMMARY.map((s) => (
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
      )}

      {/* 租户间横向对比: 维度切换 + 水平条形图 (跟随 insightsQuery 状态, 不重复请求) */}
      <TenantCompareCard
        insights={insights}
        isLoading={insightsQuery.isLoading}
        isError={insightsQuery.isError}
        error={insightsQuery.error}
        onRetry={() => insightsQuery.refetch()}
      />

      {/* 每租户卡片 */}
      {insightsQuery.isLoading ? (
        <CardGridSkeleton count={6} className="md:grid-cols-2 xl:grid-cols-3" />
      ) : insightsQuery.isError ? (
        /* 错误态已在上方汇总区渲染, 这里不再重复 */
        null
      ) : insights.length === 0 ? (
        <div className="rounded-xl border bg-card">
          <EmptyState
            icon={<Landmark className="size-6" />}
            title="暂无租户数据"
            description="创建租户并完成授权后, 这里将展示每个租户的规模与活跃度洞察"
          />
        </div>
      ) : (
        <div className="grid max-h-[40rem] gap-4 overflow-y-auto scroll-area pr-1 md:grid-cols-2 xl:grid-cols-3">
          {insights.map((t) => (
            <TenantInsightCard key={t.tenantId} insight={t} />
          ))}
        </div>
      )}
    </div>
  )
}

/* ============ 租户间横向对比 (维度切换 + 水平条形图) ============ */

/** 对比维度 key (均为 TenantInsight 数值字段) */
type CompareDim = "users" | "organizations" | "roles" | "logins7d" | "activePlatformSessions"

/** 维度元信息: 每个维度固定专属配色 (沿用项目 chart-1~5 色板) */
interface CompareDimMeta {
  key: CompareDim
  label: string
  icon: typeof Users
  /** bar / 数值填充色 (CSS 变量, 深浅主题自适应) */
  color: string
  /** chip 选中态配色 (边框/背景/文字用该维度色) */
  chip: string
}

const COMPARE_DIMS: CompareDimMeta[] = [
  { key: "users", label: "用户数", icon: Users, color: "var(--chart-1)", chip: "border-chart-1/40 bg-chart-1/10 text-chart-1" },
  { key: "organizations", label: "组织数", icon: Network, color: "var(--chart-2)", chip: "border-chart-2/40 bg-chart-2/10 text-chart-2" },
  { key: "roles", label: "角色数", icon: ShieldCheck, color: "var(--chart-3)", chip: "border-chart-3/40 bg-chart-3/10 text-chart-3" },
  { key: "logins7d", label: "近 7 天登录", icon: LogIn, color: "var(--chart-4)", chip: "border-chart-4/40 bg-chart-4/10 text-chart-4" },
  { key: "activePlatformSessions", label: "活跃平台会话", icon: KeyRound, color: "var(--chart-5)", chip: "border-chart-5/40 bg-chart-5/10 text-chart-5" },
]

/* ---- 维度与排序偏好记忆(localStorage) ---- */

/** 对比图偏好持久化 key */
const COMPARE_PREF_KEY = "platform-compare-pref"

/** 对比图偏好: 维度 + 排序方向 */
interface ComparePref {
  dimension: CompareDim
  sortDir: "desc" | "asc"
}

const COMPARE_PREF_DEFAULT: ComparePref = { dimension: "users", sortDir: "desc" }

/** 读取偏好: 解析失败/字段非法时回退默认(用户数 + 降序); dimension 必须在 COMPARE_DIMS 键值内, 防手改 localStorage 注入 */
function readComparePref(): ComparePref {
  try {
    const raw = localStorage.getItem(COMPARE_PREF_KEY)
    if (!raw) return COMPARE_PREF_DEFAULT
    const parsed: unknown = JSON.parse(raw)
    if (typeof parsed !== "object" || parsed === null) return COMPARE_PREF_DEFAULT
    const p = parsed as { dimension?: unknown; sortDir?: unknown }
    const dimension = COMPARE_DIMS.some((d) => d.key === p.dimension)
      ? (p.dimension as CompareDim)
      : COMPARE_PREF_DEFAULT.dimension
    const sortDir = p.sortDir === "asc" ? "asc" : p.sortDir === "desc" ? "desc" : COMPARE_PREF_DEFAULT.sortDir
    return { dimension, sortDir }
  } catch {
    return COMPARE_PREF_DEFAULT
  }
}

function writeComparePref(pref: ComparePref) {
  try {
    localStorage.setItem(COMPARE_PREF_KEY, JSON.stringify(pref))
  } catch {
    /* ignore */
  }
}

/** 租户数超过 8 个时, 其余聚合为一条「其他 N 个租户」灰 bar */
const COMPARE_MAX_BARS = 8

/** 条形图单行数据 */
interface CompareRow {
  key: string
  /** Y 轴展示名 (超 10 字符截断) */
  label: string
  value: number
  /** DISABLED 租户: bar 透明度降低 + 名后灰点 */
  disabled: boolean
  /** 当前维度 Top 1 (最大值租户) */
  isTop: boolean
  /** 「其他 N 个租户」聚合行 */
  isOthers: boolean
  othersCount: number
  insight?: TenantInsight
}

/** 租户名超 10 字符截断 */
function truncateTenantName(name: string): string {
  return name.length > 10 ? `${name.slice(0, 10)}…` : name
}

/** recharts 传递给自定义 tick 的渲染属性 (payload.value = 类目 label) */
interface CompareYTickRenderProps {
  x?: number
  y?: number
  index?: number
  payload?: { value?: unknown }
}

/** 租户间横向对比卡片: 5 维度切换 chips + 降/升序水平条形图 + Top-N 聚合 + 底部汇总 */
function TenantCompareCard({
  insights,
  isLoading,
  isError,
  error,
  onRetry,
}: {
  insights: TenantInsight[]
  isLoading: boolean
  isError: boolean
  error?: unknown
  onRetry: () => void
}) {
  /* 维度/排序偏好: 惰性读 localStorage, 用户切换时写回(见 chooseDim/toggleSort) */
  const [pref, setPref] = React.useState<ComparePref>(readComparePref)
  const dim = pref.dimension
  const desc = pref.sortDir === "desc"
  /* 首次写入后才提示「已记住偏好」(此前 session 写入或本次写入均算) */
  const [prefSaved, setPrefSaved] = React.useState(() => {
    try {
      return !!localStorage.getItem(COMPARE_PREF_KEY)
    } catch {
      return false
    }
  })

  const chooseDim = (key: CompareDim) => {
    const next: ComparePref = { dimension: key, sortDir: pref.sortDir }
    writeComparePref(next)
    setPrefSaved(true)
    setPref(next)
  }
  const toggleSort = () => {
    const next: ComparePref = { dimension: pref.dimension, sortDir: desc ? "asc" : "desc" }
    writeComparePref(next)
    setPrefSaved(true)
    setPref(next)
  }

  const meta = React.useMemo(() => COMPARE_DIMS.find((d) => d.key === dim) ?? COMPARE_DIMS[0], [dim])

  const { rows, grandTotal, top1 } = React.useMemo(() => {
    /* 按当前维度值降序得到名次; Top-N 与 Top1 均基于该名次, 与展示排序方向无关 */
    const sorted = [...insights].sort((a, b) => (b[meta.key] || 0) - (a[meta.key] || 0))
    const total = insights.reduce((s, t) => s + (t[meta.key] || 0), 0)
    const first: TenantInsight | undefined = sorted.length > 0 ? sorted[0] : undefined
    const hasTop = !!first && (first[meta.key] || 0) > 0
    const rest = sorted.slice(COMPARE_MAX_BARS)
    const list: CompareRow[] = sorted.slice(0, COMPARE_MAX_BARS).map((t) => ({
      key: t.tenantId,
      label: truncateTenantName(t.tenantName || t.tenantCode || "—"),
      value: t[meta.key] || 0,
      disabled: t.status === "DISABLED",
      isTop: hasTop && t.tenantId === first?.tenantId,
      isOthers: false,
      othersCount: 0,
      insight: t,
    }))
    if (rest.length > 0) {
      list.push({
        key: "__others__",
        label: `其他 ${rest.length} 个租户`,
        value: rest.reduce((s, t) => s + (t[meta.key] || 0), 0),
        disabled: false,
        isTop: false,
        isOthers: true,
        othersCount: rest.length,
      })
    }
    /* 展示排序: 全部行(含「其他」聚合行)按当前维度值排序, 降/升序仅方向不同; sort 稳定, 同值时租户行先于聚合行 */
    list.sort((a, b) => (desc ? b.value - a.value : a.value - b.value))
    return { rows: list, grandTotal: total, top1: first }
  }, [insights, meta, desc])

  const rowByLabel = React.useMemo(() => new Map(rows.map((r) => [r.label, r])), [rows])

  /** Y 轴 tick: 右对齐租户名 + Top1 行首皇冠 + 禁用租户名后灰点 */
  const renderYTick = (props: CompareYTickRenderProps) => {
    const label = props.payload?.value
    const row = (typeof label === "string" ? rowByLabel.get(label) : undefined) ?? rows[props.index ?? -1]
    return <CompareYTick x={props.x} y={props.y} row={row} />
  }

  const chartHeight = Math.max(220, rows.length * 44)
  const top1Pct = top1 && grandTotal > 0 ? ((top1[meta.key] || 0) / grandTotal) * 100 : null
  const hasOthers = rows.some((r) => r.isOthers)

  return (
    <Card className="transition-shadow hover:shadow-md">
      <CardHeader className="flex flex-row items-start justify-between space-y-0 pb-3">
        <div>
          <CardTitle className="flex items-center gap-2 text-base">
            <BarChart3 className="size-4 text-primary" />
            租户间横向对比
          </CardTitle>
          <CardDescription className="flex flex-wrap items-center gap-1.5">
            切换维度查看各租户规模与活跃度排名
            {prefSaved && (
              <span
                title="已记住您的维度偏好"
                className="inline-flex items-center gap-0.5 text-[11px] text-muted-foreground/60"
              >
                <BookmarkCheck className="size-3" aria-hidden />
                已记住偏好
              </span>
            )}
          </CardDescription>
        </div>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="outline"
              size="icon"
              className="size-11 text-muted-foreground"
              aria-label={desc ? "切换为升序排列" : "切换为降序排列"}
              onClick={toggleSort}
            >
              {desc ? <ArrowDownWideNarrow className="size-4" /> : <ArrowUpWideNarrow className="size-4" />}
            </Button>
          </TooltipTrigger>
          <TooltipContent>{desc ? "当前:降序 · 点击切换为升序" : "当前:升序 · 点击切换为降序"}</TooltipContent>
        </Tooltip>
      </CardHeader>
      <CardContent className="space-y-4">
        {/* 维度切换 chips (不依赖数据, loading 期即可交互; <sm 允许换行) */}
        <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label="对比维度">
          <ListFilter className="mr-0.5 size-3.5 text-muted-foreground/70" />
          {COMPARE_DIMS.map((d) => {
            const active = dim === d.key
            return (
              <button
                key={d.key}
                type="button"
                onClick={() => chooseDim(d.key)}
                aria-pressed={active}
                title={`按${d.label}对比`}
                className={cn(
                  "inline-flex min-h-9 items-center gap-1.5 rounded-full border px-3 text-xs font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring",
                  active ? d.chip : "border-border bg-background text-muted-foreground hover:bg-accent hover:text-foreground"
                )}
              >
                <d.icon className="size-3.5" />
                {d.label}
              </button>
            )
          })}
        </div>

        {isLoading ? (
          /* 加载骨架: 6 行 h-6 递减宽度模拟 bar */
          <div className="space-y-5 py-1" aria-hidden>
            {Array.from({ length: 6 }).map((_, i) => (
              <div key={i} className="flex items-center gap-4">
                <Skeleton className="h-4 w-28 shrink-0" />
                <Skeleton className="h-6 rounded-md" style={{ width: `${86 - i * 12}%` }} />
              </div>
            ))}
          </div>
        ) : isError ? (
          <ErrorState
            className="border-0 bg-transparent py-8"
            message={error instanceof Error ? error.message : undefined}
            onRetry={onRetry}
          />
        ) : insights.length === 0 ? (
          <EmptyState
            className="py-10"
            icon={<BarChart3 className="size-6" />}
            title="暂无租户数据"
            description="创建租户后, 可在此横向对比各租户的规模与活跃度"
          />
        ) : (
          <>
            {/* 水平条形图: 窄屏横向滚动兜底 */}
            <div
              className="overflow-x-auto scroll-area"
              role="img"
              aria-label={`各租户${meta.label}横向对比条形图, 共 ${rows.length} 条${top1 ? `, Top1 ${top1.tenantName || top1.tenantCode}` : ""}`}
            >
              <div className="min-w-[640px]" style={{ height: chartHeight }}>
                <ResponsiveContainer width="100%" height="100%">
                  <BarChart data={rows} layout="vertical" margin={{ top: 4, right: 56, left: 4, bottom: 0 }}>
                    <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" horizontal={false} />
                    <XAxis
                      type="number"
                      allowDecimals={false}
                      tick={{ fontSize: 11, fill: "var(--muted-foreground)" }}
                      tickLine={false}
                      axisLine={false}
                    />
                    <YAxis
                      type="category"
                      dataKey="label"
                      width={160}
                      tickLine={false}
                      axisLine={false}
                      tick={renderYTick}
                    />
                    <ReTooltip
                      content={<CompareTip dim={dim} grandTotal={grandTotal} />}
                      cursor={{ fill: "var(--muted)", opacity: 0.35 }}
                    />
                    <Bar
                      dataKey="value"
                      name={meta.label}
                      radius={[0, 6, 6, 0]}
                      barSize={24}
                      isAnimationActive
                    >
                      {rows.map((r) => (
                        <Cell
                          key={r.key}
                          fill={r.isOthers ? "var(--muted-foreground)" : meta.color}
                          fillOpacity={r.isOthers ? 0.35 : r.disabled ? 0.45 : r.isTop ? 1 : 0.6}
                          stroke={r.isOthers ? "var(--muted-foreground)" : undefined}
                          strokeOpacity={r.isOthers ? 0.5 : undefined}
                          strokeDasharray={r.isOthers ? "5 3" : undefined}
                          strokeWidth={r.isOthers ? 1 : undefined}
                        />
                      ))}
                      <LabelList
                        dataKey="value"
                        position="right"
                        offset={10}
                        formatter={(v: number | string) => formatNumber(Number(v))}
                        fill="var(--foreground)"
                        fontSize={12}
                        style={{ fontVariantNumeric: "tabular-nums" }}
                      />
                    </Bar>
                  </BarChart>
                </ResponsiveContainer>
              </div>
            </div>

            {/* 底部汇总行 */}
            <div className="flex flex-col gap-2.5 border-t pt-3 text-xs text-muted-foreground sm:flex-row sm:items-center sm:justify-between sm:gap-4">
              <p className="flex items-baseline gap-2">
                <span>{meta.label}总和</span>
                <span className="text-xl font-semibold leading-none tabular-nums" style={{ color: meta.color }}>
                  {formatNumber(grandTotal)}
                </span>
              </p>
              <p className="flex items-center gap-1.5">
                <Crown className="size-3.5 shrink-0 text-amber-500" aria-hidden />
                <span className="max-w-[14ch] truncate font-medium text-foreground">
                  {top1 ? top1.tenantName || top1.tenantCode || "—" : "—"}
                </span>
                <span>Top 1 · 占</span>
                <span className="font-semibold tabular-nums text-foreground">
                  {top1Pct === null ? "—" : `${top1Pct.toFixed(1)}%`}
                </span>
              </p>
              <p className="tabular-nums">
                参与对比 {insights.length} 个租户
                {hasOthers && <span className="ml-1 text-muted-foreground/70">(Top {COMPARE_MAX_BARS} + 其他聚合)</span>}
              </p>
            </div>
          </>
        )}
      </CardContent>
    </Card>
  )
}

/** Y 轴自定义 tick: 租户名右对齐(不倾斜), Top1 名前置皇冠(amber), 禁用租户名后灰圆点 */
function CompareYTick({ x = 0, y = 0, row }: { x?: number; y?: number; row?: CompareRow }) {
  const textRef = React.useRef<SVGTextElement | null>(null)
  const [textWidth, setTextWidth] = React.useState(0)

  /* 皇冠需贴在名首, 名宽须在挂载后实测 (布局期内同步完成, 无闪烁) */
  React.useLayoutEffect(() => {
    setTextWidth(textRef.current?.getComputedTextLength() ?? 0)
  }, [row?.label, row?.disabled])

  if (!row) return null
  return (
    <g transform={`translate(${x},${y})`}>
      <title>{`${row.insight?.tenantName ?? row.label}${row.disabled ? " (已禁用)" : ""}`}</title>
      {row.isTop && (
        <g transform={`translate(${-(textWidth + 22)}, -7)`}>
          <Crown className="size-3.5 text-amber-500" strokeWidth={2.25} aria-hidden />
        </g>
      )}
      <text ref={textRef} x={0} y={0} dy={4} textAnchor="end" fontSize={12} fill="var(--foreground)">
        {row.label}
        {row.disabled && (
          <tspan fill="var(--muted-foreground)" fontSize={9} dx={3}>
            ●
          </tspan>
        )}
      </text>
    </g>
  )
}

/** 对比条形图 Tooltip (TrendTip 风格): 该租户全部 5 个维度数值 + 当前维度占总和百分比 */
function CompareTip({
  active,
  payload,
  dim,
  grandTotal,
}: {
  active?: boolean
  payload?: { payload?: CompareRow }[]
  dim: CompareDim
  grandTotal: number
}) {
  if (!active || !payload?.length) return null
  const row = payload[0]?.payload
  if (!row) return null
  const meta = COMPARE_DIMS.find((d) => d.key === dim) ?? COMPARE_DIMS[0]
  const pct = grandTotal > 0 ? `${((row.value / grandTotal) * 100).toFixed(1)}%` : "—"
  return (
    <div className="rounded-lg border bg-popover px-3 py-2.5 text-xs shadow-lg">
      <p className="flex items-center gap-1.5 font-medium text-popover-foreground">
        {row.isTop && <Crown className="size-3.5 shrink-0 text-amber-500" aria-hidden />}
        <span className="max-w-[16ch] truncate">{row.insight?.tenantName ?? row.label}</span>
        {row.disabled && (
          <span className="rounded-full border bg-muted px-1.5 py-px text-[10px] font-normal text-muted-foreground">已禁用</span>
        )}
        {row.isOthers && <span className="shrink-0 text-muted-foreground">· 合计</span>}
      </p>
      {row.insight ? (
        <div className="mt-1.5 space-y-0.5">
          {COMPARE_DIMS.map((d) => {
            const current = d.key === dim
            return (
              <div key={d.key} className="flex items-center justify-between gap-8">
                <span className="flex items-center gap-1.5" style={current ? { color: d.color } : undefined}>
                  <d.icon className="size-3" />
                  <span className={current ? "font-semibold" : "text-muted-foreground"}>{d.label}</span>
                </span>
                <span
                  className={cn("tabular-nums", current ? "font-semibold text-popover-foreground" : "text-muted-foreground")}
                >
                  {formatNumber(row.insight?.[d.key] ?? 0)}
                </span>
              </div>
            )
          })}
        </div>
      ) : (
        <p className="mt-1.5 text-muted-foreground">其余 {row.othersCount} 个租户的{meta.label}合计</p>
      )}
      <p className="mt-1.5 border-t pt-1.5 text-muted-foreground">
        {meta.label}占总和 <span className="font-semibold tabular-nums text-popover-foreground">{pct}</span>
      </p>
    </div>
  )
}

function TenantInsightCard({ insight: t }: { insight: TenantInsight }) {
  const metrics: { key: string; label: string; value: number; icon: typeof Users }[] = [
    { key: "users", label: "用户", value: t.users, icon: Users },
    { key: "orgs", label: "组织", value: t.organizations, icon: Landmark },
    { key: "roles", label: "角色", value: t.roles, icon: Activity },
    { key: "logins", label: "近 7 天登录", value: t.logins7d, icon: LogIn },
  ]

  return (
    <Card className="group transition-all hover:-translate-y-0.5 hover:border-primary/40 hover:shadow-md">
      <CardHeader className="pb-3">
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0">
            <CardTitle className="flex items-center gap-2 truncate text-base">
              <span className="truncate">{t.tenantName || "—"}</span>
              {t.tenantCode && (
                <code className="shrink-0 rounded border bg-muted/60 px-1.5 py-px font-mono text-[10px] font-normal text-muted-foreground">
                  {t.tenantCode}
                </code>
              )}
            </CardTitle>
            <CardDescription className="mt-1 flex flex-wrap items-center gap-2">
              {t.status ? <StatusBadge status={t.status} /> : null}
              <span className="inline-flex items-center gap-1">
                <KeyRound className="size-3" />
                <span className="tabular-nums">{formatNumber(t.activePlatformSessions)}</span> 活跃平台会话
              </span>
            </CardDescription>
          </div>
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        {/* 四指标 */}
        <div className="grid grid-cols-4 gap-2">
          {metrics.map((m) => (
            <div
              key={m.key}
              className="rounded-lg border bg-card/60 px-2 py-2 text-center transition-colors group-hover:border-primary/25"
            >
              <p className="flex items-center justify-center gap-1 text-[10px] uppercase tracking-wide text-muted-foreground">
                <m.icon className="size-3" />
                {m.label}
              </p>
              <p className="mt-1 text-lg font-semibold tabular-nums leading-none">{formatNumber(m.value)}</p>
            </div>
          ))}
        </div>

        {/* 近 7 天登录趋势 */}
        <div className="rounded-lg border bg-muted/30 p-3">
          <div className="mb-1.5 flex items-center justify-between">
            <p className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
              <LogIn className="size-3.5 text-primary" />
              近 7 天登录趋势
            </p>
            <span className="text-xs tabular-nums text-muted-foreground">
              合计 <span className="font-semibold text-foreground">{formatNumber(t.logins7d)}</span> 次
            </span>
          </div>
          <TrendSparkline tenantId={t.tenantId} trend={t.loginsTrend} />
        </div>
      </CardContent>
    </Card>
  )
}

/** 迷你 7 点面积趋势 (与仪表盘 recharts 方案一致, 悬停显示当日数值) */
function TrendSparkline({ tenantId, trend }: { tenantId: string; trend: number[] }) {
  const data = React.useMemo(() => {
    const now = new Date()
    const len = Math.max(1, trend.length)
    return (trend.length ? trend : [0]).map((v, i) => {
      const d = new Date(now)
      d.setDate(now.getDate() - (len - 1 - i))
      return { day: `${d.getMonth() + 1}/${d.getDate()}`, count: v }
    })
  }, [trend])
  const gid = `tenant-trend-${tenantId.replace(/[^a-zA-Z0-9-]/g, "")}`

  return (
    <div className="h-16" aria-hidden={false}>
      <ResponsiveContainer width="100%" height="100%">
        <AreaChart data={data} margin={{ top: 4, right: 2, left: 2, bottom: 0 }}>
          <defs>
            <linearGradient id={gid} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="var(--chart-1)" stopOpacity={0.35} />
              <stop offset="100%" stopColor="var(--chart-1)" stopOpacity={0.02} />
            </linearGradient>
          </defs>
          <ReTooltip content={<TrendTip />} cursor={{ stroke: "var(--border)", strokeDasharray: "3 3" }} />
          <Area type="monotone" dataKey="count" name="登录次数" stroke="var(--chart-1)" strokeWidth={1.8} fill={`url(#${gid})`} />
        </AreaChart>
      </ResponsiveContainer>
    </div>
  )
}

function TrendTip({ active, payload, label }: { active?: boolean; payload?: { value?: number }[]; label?: string }) {
  if (!active || !payload?.length) return null
  const value = payload[0]?.value ?? 0
  return (
    <div className="rounded-lg border bg-popover px-3 py-2 text-xs shadow-lg">
      <p className="font-medium text-popover-foreground">{label}</p>
      <p className="tabular-nums text-muted-foreground">
        登录 <span className="font-semibold text-foreground">{formatNumber(value)}</span> 次
      </p>
    </div>
  )
}

function StatusCard({
  title,
  icon,
  value,
  ok,
  warnWhen,
  isLoading,
  isError,
  error,
  onRetry,
  mono,
}: {
  title: string
  icon: React.ReactNode
  value?: string
  ok?: boolean
  warnWhen?: boolean
  isLoading: boolean
  isError: boolean
  error?: unknown
  onRetry: () => void
  mono?: boolean
}) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardDescription className="flex items-center gap-2">
          <span className="text-primary">{icon}</span>
          {title}
        </CardDescription>
        {isLoading ? (
          <Skeleton className="h-8 w-24" />
        ) : isError ? (
          <div className="pt-1">
            <ErrorState
              className="border-0 bg-transparent py-4"
              message={error instanceof Error ? error.message : undefined}
              onRetry={onRetry}
            />
          </div>
        ) : (
          <CardTitle
            className={cn(
              "flex items-center gap-2 text-xl font-semibold",
              mono && "font-mono",
              !isError && ok === false && "text-destructive",
              !isError && warnWhen && "text-warning"
            )}
          >
            {value ?? "—"}
            {!isError && (
              <span
                className={cn(
                  "size-2 rounded-full",
                  ok === false ? "bg-destructive animate-pulse" : warnWhen ? "bg-warning animate-pulse" : "bg-primary"
                )}
                aria-hidden
              />
            )}
          </CardTitle>
        )}
      </CardHeader>
    </Card>
  )
}
