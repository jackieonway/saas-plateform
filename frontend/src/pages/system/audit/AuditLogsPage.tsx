import * as React from "react"
import { useQuery } from "@tanstack/react-query"
import { Crown, Download, FileText, Loader2, Search } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { PageHeader } from "@/components/layout/PageHeader"
import { DataPagination } from "@/components/DataPagination"
import { usePageRecovery } from "@/hooks/use-page-recovery"
import { TableSkeleton, EmptyState, ErrorState } from "@/components/states"
import { TableDensityToggle } from "@/components/TableDensityToggle"
import { ColumnVisibilityDropdown } from "@/components/ColumnVisibilityDropdown"
import { useTableDensity, DENSITY_BODY_CLASS } from "@/hooks/use-table-density"
import { useColumnVisibility, type ColumnDef } from "@/hooks/use-column-visibility"
import { auditApi } from "@/api/audit"
import { tenantsApi } from "@/api/tenants"
import { asPage } from "@/lib/api"
import { useAuthStore } from "@/stores/auth"
import { hasPlatformContext } from "@/lib/platform-context"
import { AppIcon } from "@/lib/icons"
import { parseUserAgent } from "@/lib/user-agent"
import { auditActionLabel, auditActionIcon, auditActionTone } from "@/components/layout/TopBar"
import { formatDateTime, formatRelative, useDebounced } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { AuditLogRecord, TenantRecord } from "@/api/types"

const ACTION_OPTIONS = [
  "LOGIN",
  "LOGIN_FAILED",
  "CONTEXT_SWITCH",
  "LOGOUT",
  "CREATE_TENANT",
  "UPDATE_TENANT",
  "DELETE_TENANT",
  "CREATE_ORGANIZATION",
  "UPDATE_ORGANIZATION",
  "DELETE_ORGANIZATION",
  "CREATE_USER",
  "UPDATE_USER",
  "DELETE_USER",
  "CREATE_ROLE",
  "UPDATE_ROLE",
  "DELETE_ROLE",
  "TENANT_AUTH",
  "ORG_AUTH",
  "ROLE_MENUS",
  "CONSENT_APPROVE",
  "CONSENT_DENY",
  "DEVICE_APPROVE",
  "DEVICE_DENY",
  "TOKEN_ISSUED",
  "TOKEN_REVOKED",
  "ADMIN_REVOKE_SESSION",
  "KICK_SESSIONS",
]

/** 审计表列定义:时间/用户/动作常显(标识性列), 租户/组织/详情/IP/UA 可隐藏
 *  租户列仅在平台租户上下文渲染(非平台用户所有行均属本租户, 该列冗余) */
const AUDIT_COLUMNS: ColumnDef[] = [
  { key: "time", label: "时间", required: true },
  { key: "user", label: "用户", required: true },
  { key: "tenant", label: "租户" },
  { key: "org", label: "组织" },
  { key: "action", label: "动作", required: true },
  { key: "detail", label: "详情" },
  { key: "ip", label: "IP" },
  { key: "ua", label: "UA" },
]

export default function AuditLogsPage() {
  const [page, setPage] = React.useState(1)
  const [size, setSize] = React.useState(10)
  const [keyword, setKeyword] = React.useState("")
  const [action, setAction] = React.useState("ALL")
  /** 租户筛选: ALL=全部 / none=仅平台级行 / 租户 uuid=指定租户(仅平台上下文下发) */
  const [tenantFilter, setTenantFilter] = React.useState("ALL")
  const debounced = useDebounced(keyword, 300)
  const token = useAuthStore((s) => s.token)
  const [exporting, setExporting] = React.useState(false)
  /* 仅平台根组织上下文展示跨租户筛选，其他组织使用服务端限定的范围。 */
  const isPlatformTenant = useAuthStore((s) => hasPlatformContext(s.profile))
  /* 表格密度 + 列显隐 */
  const { density } = useTableDensity()
  const { visibleSet: auditCols, isVisible, setVisible } = useColumnVisibility("audit-logs", AUDIT_COLUMNS)
  /* 列显隐下拉选项按上下文派生: 非平台用户看不到「租户」列 */
  const dropdownColumns = React.useMemo(
    () => (isPlatformTenant ? AUDIT_COLUMNS : AUDIT_COLUMNS.filter((c) => c.key !== "tenant")),
    [isPlatformTenant]
  )
  /* 列显隐存储一次性迁移: 旧合并列 tenantOrg → tenant + org(幂等; 迁移后广播事件令 hook 重读) */
  React.useEffect(() => {
    try {
      const raw = localStorage.getItem("platform-cols-audit-logs")
      if (!raw) return
      const parsed: unknown = JSON.parse(raw)
      if (!Array.isArray(parsed) || !parsed.some((k) => k === "tenantOrg")) return
      const migrated = parsed.flatMap((k) => (k === "tenantOrg" ? ["tenant", "org"] : [k]))
      localStorage.setItem("platform-cols-audit-logs", JSON.stringify(migrated))
      window.dispatchEvent(new Event("platform-cols-change-audit-logs"))
    } catch {
      /* ignore */
    }
  }, [])

  /* 列显隐一次性迁移: 新增「UA」列对已存偏好的老用户默认可见
   * (标志位防重复回加: 用户主动隐藏 UA 后不会再被迁移加回; 无历史偏好时默认全显无需迁移) */
  React.useEffect(() => {
    try {
      const FLAG = "platform-cols-audit-logs-ua-added"
      if (localStorage.getItem(FLAG)) return
      const raw = localStorage.getItem("platform-cols-audit-logs")
      if (!raw) return
      const parsed: unknown = JSON.parse(raw)
      if (!Array.isArray(parsed) || parsed.includes("ua")) return
      localStorage.setItem("platform-cols-audit-logs", JSON.stringify([...parsed, "ua"]))
      localStorage.setItem(FLAG, "1")
      window.dispatchEvent(new Event("platform-cols-change-audit-logs"))
    } catch {
      /* ignore */
    }
  }, [])

  /* 租户清单(仅平台上下文拉取): 平台租户 id 集合驱动 Crown 徽章; 失败静默降级为无徽章, 不阻塞表格 */
  const tenantsQuery = useQuery({
    queryKey: ["audit-tenants"],
    queryFn: async () => asPage<TenantRecord>(await tenantsApi.options()),
    staleTime: 60_000,
    retry: 1,
    enabled: isPlatformTenant,
  })
  const platformTenantIds = React.useMemo(() => {
    const ids = new Set<string>()
    for (const t of tenantsQuery.data?.records ?? []) if (t.isPlatform) ids.add(t.id)
    return ids
  }, [tenantsQuery.data])

  const listQuery = useQuery({
    queryKey: ["audit-logs", page, size, debounced, action, tenantFilter],
    queryFn: async () =>
      asPage<AuditLogRecord>(
        await auditApi.list({
          page,
          size,
          keyword: debounced,
          action,
          tenantId: isPlatformTenant && tenantFilter !== "ALL" ? tenantFilter : undefined,
        })
      ),
    staleTime: 30_000,
    retry: 1,
    placeholderData: (prev) => prev,
  })
  React.useEffect(() => {
    setPage(1)
  }, [debounced, action, tenantFilter, size])
  usePageRecovery(page, size, listQuery, setPage)

  const records = listQuery.data?.records ?? []
  const total = listQuery.data?.total ?? 0
  const hasFilter = Boolean(keyword) || action !== "ALL" || (isPlatformTenant && tenantFilter !== "ALL")

  /** 按当前筛选条件导出 CSV(后端返回带 BOM 的 text/csv) */
  const exportCsv = async () => {
    setExporting(true)
    try {
      const qs = new URLSearchParams()
      if (debounced) qs.set("keyword", debounced)
      if (action && action !== "ALL") qs.set("action", action)
      if (isPlatformTenant && tenantFilter !== "ALL") qs.set("tenant_id", tenantFilter)
      const query = qs.toString()
      const res = await fetch(`/api/audit-logs/export${query ? `?${query}` : ""}`, {
        headers: token ? { Authorization: `Bearer ${token}` } : undefined,
      })
      if (!res.ok) throw new Error(`导出失败 (HTTP ${res.status})`)
      const blob = await res.blob()
      const disposition = res.headers.get("content-disposition") ?? ""
      const filename =
        /filename="?([^";]+)"?/.exec(disposition)?.[1] ??
        `audit-logs-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.csv`
      const url = URL.createObjectURL(blob)
      const a = document.createElement("a")
      a.href = url
      a.download = filename
      document.body.appendChild(a)
      a.click()
      a.remove()
      URL.revokeObjectURL(url)
      toast.success("审计日志 CSV 已开始下载")
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "导出失败,请稍后重试")
    } finally {
      setExporting(false)
    }
  }

  return (
    <div>
      <PageHeader
        title="审计日志"
        description="登录、授权、组织切换与关键数据变更的完整审计轨迹"
        icon={<FileText className="size-5" />}
      />

      {/* 筛选 */}
      <div className="mb-4 flex flex-col gap-2 sm:flex-row sm:items-center">
        <div className="relative flex-1 sm:max-w-xs">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={keyword}
            onChange={(e) => setKeyword(e.target.value)}
            placeholder="搜索用户 / 详情关键词"
            aria-label="搜索审计日志"
            className="pl-9"
          />
        </div>
        <Select value={action} onValueChange={setAction}>
          <SelectTrigger className="w-full sm:w-52" aria-label="动作筛选">
            <SelectValue placeholder="全部动作" />
          </SelectTrigger>
          <SelectContent className="max-h-72">
            <SelectItem value="ALL">全部动作</SelectItem>
            {ACTION_OPTIONS.map((a) => (
              <SelectItem key={a} value={a}>
                {auditActionLabel(a)}({a})
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {isPlatformTenant && (
          <Select value={tenantFilter} onValueChange={setTenantFilter}>
            <SelectTrigger className="w-full sm:w-44" aria-label="租户筛选">
              <SelectValue placeholder="全部租户" />
            </SelectTrigger>
            <SelectContent className="max-h-72">
              <SelectItem value="ALL">全部租户</SelectItem>
              {(tenantsQuery.data?.records ?? []).map((t) => (
                <SelectItem key={t.id} value={t.id}>
                  <span className="truncate">{t.name}</span>
                  {t.isPlatform && <Crown className="size-3 shrink-0 text-primary" aria-hidden />}
                </SelectItem>
              ))}
              <Tooltip>
                <TooltipTrigger asChild>
                  <SelectItem value="none">平台级行</SelectItem>
                </TooltipTrigger>
                <TooltipContent>租户为空的平台级系统事件</TooltipContent>
              </Tooltip>
            </SelectContent>
          </Select>
        )}
        <Button variant="ghost" size="sm" className="text-muted-foreground" onClick={() => listQuery.refetch()} aria-label="刷新">
          {listQuery.isFetching ? <Loader2 className="size-4 animate-spin" /> : <span className="text-xs">刷新</span>}
        </Button>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button variant="outline" size="sm" onClick={exportCsv} disabled={exporting}>
              {exporting ? <Loader2 className="size-4 animate-spin" /> : <Download className="size-4" />}
              导出 CSV
            </Button>
          </TooltipTrigger>
          <TooltipContent>按当前筛选条件导出</TooltipContent>
        </Tooltip>
        <div className="flex items-center gap-2 sm:ml-auto">
          <span className="text-xs text-muted-foreground">共 <span className="tabular-nums">{total}</span> 条记录</span>
          <ColumnVisibilityDropdown
            table="audit-logs"
            columns={dropdownColumns}
            visible={[...auditCols]}
            onChange={(next) =>
              /* 非平台上下文「重置」仅含派生列: 补回租户列既有偏好, 避免切回平台时丢显隐状态 */
              setVisible(
                isPlatformTenant || !auditCols.has("tenant") || next.includes("tenant") ? next : [...next, "tenant"]
              )
            }
          />
          <TableDensityToggle />
        </div>
      </div>

      {/* 表格 */}
      <div className="rounded-xl border bg-card">
        {listQuery.isLoading ? (
          <div className="p-2">
            <TableSkeleton cols={isPlatformTenant ? 8 : 7} />
          </div>
        ) : listQuery.isError ? (
          <ErrorState
            className="m-2 border-0 bg-transparent"
            message={listQuery.error instanceof Error ? listQuery.error.message : undefined}
            onRetry={() => listQuery.refetch()}
          />
        ) : records.length === 0 ? (
          <EmptyState
            title={hasFilter ? "没有匹配的审计记录" : "暂无审计记录"}
            description={hasFilter ? "调整筛选条件试试" : "系统产生登录与变更事件后将在此记录"}
          />
        ) : (
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className="pl-5 text-xs font-medium uppercase tracking-wider text-muted-foreground">时间</TableHead>
                <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">用户</TableHead>
                {isPlatformTenant && isVisible("tenant") && (
                  <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">租户</TableHead>
                )}
                {isVisible("org") && (
                  <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">组织</TableHead>
                )}
                <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">动作</TableHead>
                {isVisible("detail") && (
                  <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">详情</TableHead>
                )}
                {isVisible("ip") && (
                  <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">IP</TableHead>
                )}
                {isVisible("ua") && (
                  <TableHead className="pr-5 text-xs font-medium uppercase tracking-wider text-muted-foreground">UA</TableHead>
                )}
              </TableRow>
            </TableHeader>
            <TableBody className={DENSITY_BODY_CLASS[density]}>
              {records.map((log) => (
                <TableRow key={log.id}>
                  <TableCell className="pl-5 whitespace-nowrap text-xs text-muted-foreground" title={formatDateTime(log.createdAt)}>
                    {formatDateTime(log.createdAt)}
                    <span className="ml-1.5 text-muted-foreground/60">({formatRelative(log.createdAt)})</span>
                  </TableCell>
                  <TableCell className="text-sm font-medium">{log.username || "—"}</TableCell>
                  {isPlatformTenant && isVisible("tenant") && (
                    <TableCell className="text-xs text-muted-foreground">
                      {log.tenantId ? (
                        <p className="flex items-center gap-1.5">
                          <span className="truncate">{log.tenantName || "—"}</span>
                          {platformTenantIds.has(log.tenantId) && (
                            <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-primary/10 px-1.5 py-px text-[10px] font-medium text-primary ring-1 ring-primary/25">
                              <Crown className="size-2.5" aria-hidden />
                              平台租户
                            </span>
                          )}
                        </p>
                      ) : (
                        <span className="inline-flex items-center rounded-full border px-1.5 py-px text-[10px] font-medium text-muted-foreground">
                          平台
                        </span>
                      )}
                    </TableCell>
                  )}
                  {isVisible("org") && (
                    <TableCell className="text-xs text-muted-foreground">{log.organizationName || "—"}</TableCell>
                  )}
                  <TableCell>
                    <span
                      className={cn(
                        "inline-flex items-center gap-1.5 rounded-md border px-2 py-0.5 text-xs font-medium",
                        auditActionTone(log.action)
                      )}
                    >
                      <AppIcon name={auditActionIcon(log.action)} className="size-3" />
                      {auditActionLabel(log.action)}
                    </span>
                  </TableCell>
                  {isVisible("detail") && (
                    <TableCell className="max-w-[20rem]">
                      {log.detail ? (
                        <Popover>
                          <PopoverTrigger asChild>
                            <button
                              type="button"
                              className="block max-w-[20rem] truncate text-left text-xs text-muted-foreground hover:text-foreground hover:underline"
                            >
                              {log.detail}
                            </button>
                          </PopoverTrigger>
                          <PopoverContent align="start" className="w-96">
                            <p className="mb-1 text-xs font-medium text-muted-foreground">
                              {auditActionLabel(log.action)} · {log.username}
                            </p>
                            <p className="max-h-48 overflow-y-auto scroll-area whitespace-pre-wrap break-all text-sm">
                              {log.detail}
                            </p>
                            {log.requestId && <p className="mt-3 break-all font-mono text-xs text-muted-foreground">请求编号：{log.requestId}</p>}
                            {log.applicationId && <p className="mt-1 break-all font-mono text-xs text-muted-foreground">应用 ID：{log.applicationId}</p>}
                          </PopoverContent>
                        </Popover>
                      ) : (
                        <span className="text-xs text-muted-foreground">—</span>
                      )}
                    </TableCell>
                  )}
                  {isVisible("ip") && (
                    <TableCell className="font-mono text-xs text-muted-foreground">{log.ip || "—"}</TableCell>
                  )}
                  {isVisible("ua") && (
                    <TableCell className="pr-5">
                      <UserAgentCell ua={log.userAgent ?? ""} />
                    </TableCell>
                  )}
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
        {!listQuery.isLoading && !listQuery.isError && total > 0 && (
          <div className="border-t px-4 py-3">
            <DataPagination page={page} size={size} total={total} onPageChange={setPage} onSizeChange={setSize} />
          </div>
        )}
      </div>
    </div>
  )
}

/* ============ UA 列单元格 ============ */

/** 审计日志 UA 列: 轻量解析徽章(浏览器/OS 两枚小 pill) + mono 截断原文 + popover 完整 UA; 空串显示 — */
function UserAgentCell({ ua }: { ua: string }) {
  if (!ua) return <span className="text-xs text-muted-foreground">—</span>
  const parsed = parseUserAgent(ua)
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="flex max-w-[16rem] items-center gap-1.5 text-left"
          aria-label="查看完整 User-Agent"
        >
          {parsed && (
            <>
              <span className="inline-flex shrink-0 items-center rounded-full border border-primary/25 bg-primary/10 px-1.5 py-px text-[10px] font-medium text-primary">
                {parsed.browser}
              </span>
              <span className="inline-flex shrink-0 items-center rounded-full border px-1.5 py-px text-[10px] font-medium text-muted-foreground">
                {parsed.os}
              </span>
            </>
          )}
          <span className="max-w-[10rem] truncate font-mono text-[10px] text-muted-foreground">{ua}</span>
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-96">
        <p className="mb-1 text-xs font-medium text-muted-foreground">
          User-Agent{parsed ? ` · ${parsed.browser} / ${parsed.os}` : ""}
        </p>
        <p className="max-h-48 overflow-y-auto scroll-area whitespace-pre-wrap break-all font-mono text-xs">{ua}</p>
      </PopoverContent>
    </Popover>
  )
}
