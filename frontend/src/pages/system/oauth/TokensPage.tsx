import * as React from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { BrushCleaning, Loader2, ScanSearch, ShieldOff, Smartphone, Tv, UserRound, UserX, UsersRound } from "lucide-react"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { Avatar, AvatarFallback } from "@/components/ui/avatar"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { Label } from "@/components/ui/label"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { buttonVariants } from "@/components/ui/button"
import { PageHeader } from "@/components/layout/PageHeader"
import { StatusBadge } from "@/components/StatusBadge"
import { ConfirmDialog } from "@/components/ConfirmDialog"
import { TableDensityToggle } from "@/components/TableDensityToggle"
import { ColumnVisibilityDropdown } from "@/components/ColumnVisibilityDropdown"
import { TableSkeleton, EmptyState, ErrorState } from "@/components/states"
import { oauthApi } from "@/api/oauth"
import { usersApi } from "@/api/users"
import { asArray, asPage } from "@/lib/api"
import { formatDateTime, formatRelative } from "@/lib/format"
import { useTableDensity, DENSITY_BODY_CLASS } from "@/hooks/use-table-density"
import { useColumnVisibility, type ColumnDef } from "@/hooks/use-column-visibility"
import { toast } from "sonner"
import { cn } from "@/lib/utils"
import type { DeviceCodeRecord, OAuthSession, OAuthTokenRecord } from "@/api/types"

function isExpired(iso: string | null | undefined): boolean {
  if (!iso) return false
  return new Date(iso).getTime() < Date.now()
}

/** 令牌流水列定义:类型/状态/操作常显, 客户端/用户/范围/签发与过期时间可隐藏 */
const TOKEN_COLUMNS: ColumnDef[] = [
  { key: "type", label: "类型", required: true },
  { key: "client", label: "客户端" },
  { key: "user", label: "用户" },
  { key: "scope", label: "Scope 范围" },
  { key: "issuedAt", label: "签发时间" },
  { key: "expiresAt", label: "过期时间" },
  { key: "status", label: "状态", required: true },
  { key: "actions", label: "操作", required: true },
]

export default function TokensPage() {
  const queryClient = useQueryClient()
  /* 视图状态提升: 会话视图「查看令牌」需要跨 Tab 钻取(切到令牌流水并按用户名过滤) */
  const [view, setView] = React.useState("sessions")
  const [subView, setSubView] = React.useState("tokens")
  const [userFilter, setUserFilter] = React.useState("")

  /* 会话视图钻取: 切到令牌流水子 Tab 并按该用户名过滤 */
  const inspectUser = (username: string) => {
    setUserFilter(username)
    setSubView("tokens")
    setView("flow")
  }

  /* 清理过期数据: 删除过期超过 7 天的令牌与设备码记录(后端契约 18-b, 权限 system:oauth) */
  const [purging, setPurging] = React.useState(false)
  const purgeMutation = useMutation({
    mutationFn: () => oauthApi.purgeTokens(),
    onSuccess: (d) => {
      toast.success(`已清理 ${d?.tokensDeleted ?? 0} 条令牌、${d?.devicesDeleted ?? 0} 条设备码`)
      setPurging(false)
      queryClient.invalidateQueries({ queryKey: ["oauth-tokens"] })
      queryClient.invalidateQueries({ queryKey: ["oauth-device-codes"] })
      queryClient.invalidateQueries({ queryKey: ["oauth-sessions"] })
    },
    onError: (e: Error) => toast.error(e.message || "清理失败"),
  })

  return (
    <div>
      <PageHeader
        title="设备与会话"
        description="查看后台登录设备、第三方应用会话与设备授权信息"
        icon={<Smartphone className="size-5" />}
        actions={
          <div className="flex items-center gap-2">
            <Tooltip>
              <TooltipTrigger asChild>
                <Button variant="outline" onClick={() => setPurging(true)} disabled={purgeMutation.isPending}>
                  {purgeMutation.isPending ? (
                    <Loader2 className="size-4 animate-spin" />
                  ) : (
                    <BrushCleaning className="size-4" />
                  )}
                  清理过期数据
                </Button>
              </TooltipTrigger>
              <TooltipContent>删除过期超过 7 天的令牌与设备码记录</TooltipContent>
            </Tooltip>
            <TableDensityToggle />
          </div>
        }
      />

      <Tabs value={view} onValueChange={setView}>
        <TabsList className="mb-4">
          <TabsTrigger value="flow">令牌流水</TabsTrigger>
          <TabsTrigger value="sessions" className="gap-1.5">
            会话视图
            <UsersRound className="size-3.5 text-muted-foreground/70" />
          </TabsTrigger>
        </TabsList>
        <TabsContent value="flow" className="mt-0">
          {/* 现有内容原样保留:访问令牌 / 设备码 两个子 Tab */}
          <Tabs value={subView} onValueChange={setSubView}>
            <TabsList className="mb-3">
              <TabsTrigger value="tokens">访问令牌</TabsTrigger>
              <TabsTrigger value="devices">设备码</TabsTrigger>
            </TabsList>
            <TabsContent value="tokens" className="mt-0">
              <TokensTable userFilter={userFilter} onUserFilterChange={setUserFilter} />
            </TabsContent>
            <TabsContent value="devices" className="mt-0">
              <DevicesTable />
            </TabsContent>
          </Tabs>
        </TabsContent>
        <TabsContent value="sessions" className="mt-0 space-y-4">
          <PlatformDevicesTable />
          <SessionsTable onInspectUser={inspectUser} />
        </TabsContent>
      </Tabs>

      <ConfirmDialog
        open={purging}
        onOpenChange={(o) => !o && setPurging(false)}
        title="清理过期数据?"
        description={
          <>
            将删除<b>过期超过 7 天</b>的令牌与设备码记录, 当前列表中已吊销/过期数据将被清理。确定执行?
          </>
        }
        confirmText="执行清理"
        loading={purgeMutation.isPending}
        onConfirm={() => purgeMutation.mutate()}
      />
    </div>
  )
}

/** Platform logins are durable sessions, separate from third-party OAuth tokens. */
function PlatformDevicesTable() {
  const query = useQuery({
    queryKey: ["oauth-sessions"],
    queryFn: () => oauthApi.sessions(),
    staleTime: 30_000,
    refetchInterval: 30_000,
    retry: 1,
  })
  const records = query.data?.platformSessions ?? []
  const [activeOnly, setActiveOnly] = React.useState(true)
  const visible = activeOnly ? records.filter((session) => session.active) : records
  return <section className="rounded-xl border bg-card" aria-label="平台登录设备">
    <div className="flex flex-wrap items-center justify-between gap-3 border-b px-4 py-3">
      <div><h2 className="text-sm font-semibold">平台登录设备</h2>
        <p className="text-xs text-muted-foreground">后台登录会话 · 活跃 {records.filter((session) => session.active).length} / {records.length}</p>
      </div>
      <div className="flex items-center gap-2"><Switch id="platform-devices-active" checked={activeOnly} onCheckedChange={setActiveOnly} /><Label htmlFor="platform-devices-active">仅活跃</Label></div>
    </div>
    {query.isPending ? <TableSkeleton cols={6} /> : query.isError ?
      <ErrorState message={query.error instanceof Error ? query.error.message : undefined} onRetry={() => query.refetch()} /> : visible.length === 0 ?
      <EmptyState title={activeOnly ? "当前没有活跃登录设备" : "暂无后台登录记录"} description={activeOnly ? "可关闭“仅活跃”查看已撤销或过期的登录设备。" : "用户登录后台后会在这里显示。"} /> :
      <div className="table-scroll max-h-[28rem] overflow-auto scroll-area"><Table>
        <TableHeader><TableRow><TableHead>用户</TableHead><TableHead>租户 / 组织</TableHead><TableHead>设备与 IP</TableHead><TableHead>登录时间</TableHead><TableHead>到期时间</TableHead><TableHead>状态</TableHead></TableRow></TableHeader>
        <TableBody>{visible.map((session) => <TableRow key={session.id}>
          <TableCell><p className="font-medium">{session.realName || session.username}</p><p className="text-xs text-muted-foreground">{session.username}</p></TableCell>
          <TableCell><p>{session.tenantName}</p><p className="text-xs text-muted-foreground">{session.organizationName}</p></TableCell>
          <TableCell><p className="max-w-64 truncate text-xs" title={session.userAgent}>{session.userAgent || "未知设备"}</p><p className="text-xs text-muted-foreground">{session.ip || "未知 IP"}</p></TableCell>
          <TableCell className="whitespace-nowrap text-xs">{formatDateTime(session.issuedAt)}</TableCell>
          <TableCell className="whitespace-nowrap text-xs">{formatDateTime(session.expiresAt)}</TableCell>
          <TableCell><Badge variant="outline">{session.isCurrent ? "当前设备" : session.active ? "在线" : session.revokedAt ? "已撤销" : "已失效"}</Badge></TableCell>
        </TableRow>)}</TableBody>
      </Table></div>}
  </section>
}

/* ============ 令牌表 ============ */

function TokensTable({
  userFilter,
  onUserFilterChange,
}: {
  /** 用户名过滤(与会话视图钻取联动): 空串 = 不过滤 */
  userFilter: string
  onUserFilterChange: (v: string) => void
}) {
  const queryClient = useQueryClient()
  const { density } = useTableDensity()
  const { visibleSet: tokenCols, isVisible, setVisible } = useColumnVisibility("oauth-tokens", TOKEN_COLUMNS)
  const listQuery = useQuery({
    queryKey: ["oauth-tokens"],
    queryFn: async () => asArray<OAuthTokenRecord>(await oauthApi.tokens()),
    staleTime: 30_000,
    retry: 1,
  })

  const [revoking, setRevoking] = React.useState<OAuthTokenRecord | null>(null)
  const [kicking, setKicking] = React.useState<OAuthTokenRecord | null>(null)
  const revokeMutation = useMutation({
    mutationFn: (id: string) => oauthApi.revokeToken(id),
    onSuccess: () => {
      toast.success("令牌已吊销")
      setRevoking(null)
      queryClient.invalidateQueries({ queryKey: ["oauth-tokens"] })
    },
    onError: (e: Error) => toast.error(e.message || "吊销失败"),
  })

  const tokens = listQuery.data ?? []
  /* 用户名过滤: client 端模糊匹配 */
  const filterQuery = userFilter.trim().toLowerCase()
  const visibleTokens = React.useMemo(
    () => (filterQuery ? tokens.filter((t) => t.username?.toLowerCase().includes(filterQuery)) : tokens),
    [tokens, filterQuery]
  )

  return (
    <div className="rounded-xl border bg-card">
      {/* 工具行:标题 + 用户名过滤 + 计数 + 列显隐 */}
      <div className="flex flex-wrap items-center gap-2 border-b px-4 py-1.5">
        <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">访问令牌</p>
        <div className="relative w-44">
          <ScanSearch className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            type="text"
            value={userFilter}
            onChange={(e) => onUserFilterChange(e.target.value)}
            placeholder="按用户名过滤"
            aria-label="按用户名过滤令牌"
            className="h-7 pl-7 pr-2 text-xs"
          />
        </div>
        <div className="ml-auto flex items-center gap-2">
          <span className="text-xs text-muted-foreground">
            共 <span className="tabular-nums">{visibleTokens.length}</span> 条
            {filterQuery && visibleTokens.length !== tokens.length && (
              <span className="text-muted-foreground/70"> / {tokens.length}</span>
            )}
          </span>
          <ColumnVisibilityDropdown
            table="oauth-tokens"
            columns={TOKEN_COLUMNS}
            visible={[...tokenCols]}
            onChange={setVisible}
          />
        </div>
      </div>
      {listQuery.isLoading ? (
        <div className="p-2">
          <TableSkeleton cols={7} />
        </div>
      ) : listQuery.isError ? (
        <ErrorState
          className="m-2 border-0 bg-transparent"
          message={listQuery.error instanceof Error ? listQuery.error.message : undefined}
          onRetry={() => listQuery.refetch()}
        />
      ) : tokens.length === 0 ? (
        <EmptyState title="暂无令牌" description="OAuth 客户端获得授权后将在此出现" />
      ) : visibleTokens.length === 0 ? (
        /* 过滤后无匹配行的友好空态 */
        <EmptyState
          icon={<ScanSearch className="size-8" />}
          title="没有匹配的令牌"
          description={`没有用户名包含「${userFilter.trim()}」的令牌`}
          action={
            <Button size="sm" variant="outline" onClick={() => onUserFilterChange("")}>
              清除过滤
            </Button>
          }
        />
      ) : (
        <div className="table-scroll max-h-[32rem] overflow-auto scroll-area">
        <Table>
          <TableHeader className="sticky top-0 z-10 bg-card">
            <TableRow className="hover:bg-transparent">
                <TableHead className="pl-5 text-xs font-medium uppercase tracking-wider text-muted-foreground">类型</TableHead>
                {isVisible("client") && (
                  <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">客户端</TableHead>
                )}
                {isVisible("user") && (
                  <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">用户</TableHead>
                )}
                {isVisible("scope") && (
                  <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">Scope</TableHead>
                )}
                {isVisible("issuedAt") && (
                  <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">签发时间</TableHead>
                )}
                {isVisible("expiresAt") && (
                  <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">过期时间</TableHead>
                )}
                <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">状态</TableHead>
                <TableHead className="pr-5 text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">操作</TableHead>
              </TableRow>
          </TableHeader>
          <TableBody className={DENSITY_BODY_CLASS[density]}>
            {visibleTokens.map((t) => (
              <TableRow key={t.id}>
                <TableCell className="pl-5">
                  <Badge
                    variant="outline"
                    className={
                      t.tokenType === "access"
                        ? "border-primary/30 bg-primary/10 text-primary"
                        : "border-chart-4/30 bg-chart-4/10 text-chart-4"
                    }
                  >
                    {t.tokenType === "access" ? "访问令牌" : "刷新令牌"}
                  </Badge>
                </TableCell>
                {isVisible("client") && (
                  <TableCell>
                    <code className="font-mono text-xs">{t.clientId}</code>
                  </TableCell>
                )}
                {isVisible("user") && <TableCell className="text-sm">{t.username || "—"}</TableCell>}
                {isVisible("scope") && (
                  <TableCell>
                    <div className="flex max-w-[12rem] flex-wrap gap-1">
                      {t.scope
                        ? t.scope.split(/\s+/).filter(Boolean).map((s) => (
                            <Badge key={s} variant="secondary" className="text-[10px] font-normal">
                              {s}
                            </Badge>
                          ))
                        : "—"}
                    </div>
                  </TableCell>
                )}
                {isVisible("issuedAt") && (
                  <TableCell className="text-xs text-muted-foreground" title={formatDateTime(t.createdAt)}>
                    {formatRelative(t.createdAt)}
                  </TableCell>
                )}
                {isVisible("expiresAt") && (
                  <TableCell
                    className={cn("text-xs", isExpired(t.expiresAt) ? "font-medium text-destructive" : "text-muted-foreground")}
                    title={formatDateTime(t.expiresAt)}
                  >
                    {formatRelative(t.expiresAt)}
                    {isExpired(t.expiresAt) && " · 已过期"}
                  </TableCell>
                )}
                <TableCell>
                  {t.revoked ? (
                    <StatusBadge status="REVOKED" />
                  ) : isExpired(t.expiresAt) ? (
                    <StatusBadge status="EXPIRED" />
                  ) : (
                    <StatusBadge status="ACTIVE" />
                  )}
                </TableCell>
                <TableCell className="pr-5 text-right">
                  <div className="flex items-center justify-end gap-0.5">
                    {/* 行级主操作:单条吊销(仅此令牌) */}
                    {!t.revoked && (
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="size-8 text-destructive hover:text-destructive"
                            aria-label="吊销令牌"
                            onClick={() => setRevoking(t)}
                          >
                            <ShieldOff className="size-4" />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>吊销此令牌</TooltipContent>
                      </Tooltip>
                    )}
                    {/* 用户级操作:踢出全部会话(吊销该用户所有令牌) */}
                    {t.username && (
                      <>
                        {!t.revoked && <span className="mx-0.5 h-4 w-px bg-border" aria-hidden />}
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <Button
                              variant="ghost"
                              size="icon"
                              className="size-8 text-amber-600 hover:text-amber-600 dark:text-amber-500 dark:hover:text-amber-500"
                              aria-label="踢出全部: 吊销该用户的所有令牌"
                              onClick={() => setKicking(t)}
                            >
                              <UserX className="size-4" />
                            </Button>
                          </TooltipTrigger>
                          <TooltipContent>踢出全部: 吊销该用户的所有令牌</TooltipContent>
                        </Tooltip>
                      </>
                    )}
                  </div>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
        </div>
      )}

      <ConfirmDialog
        open={!!revoking}
        onOpenChange={(o) => !o && setRevoking(null)}
        title="吊销该令牌?"
        description={
          <>
            将立即吊销 {revoking?.tokenType === "access" ? "访问令牌" : "刷新令牌"}
            (用户 <b>{revoking?.username || "—"}</b> · 客户端 <code className="font-mono">{revoking?.clientId}</code>
            ),对应客户端将需要重新走授权流程。
            <br />
            仅吊销此令牌, 不影响该用户的其他令牌。
          </>
        }
        confirmText="吊销"
        loading={revokeMutation.isPending}
        onConfirm={() => revoking && revokeMutation.mutate(revoking.id)}
      />

      <KickSessionDialog
        open={!!kicking}
        initialUsername={kicking?.username ?? ""}
        onOpenChange={(o) => !o && setKicking(null)}
      />
    </div>
  )
}

/* ============ 会话视图:按用户聚合 ============ */

function SessionsTable({ onInspectUser }: { onInspectUser: (username: string) => void }) {
  const { density } = useTableDensity()
  const listQuery = useQuery({
    queryKey: ["oauth-sessions"],
    queryFn: () => oauthApi.sessions(),
    staleTime: 15_000,
    retry: 1,
  })

  const [kicking, setKicking] = React.useState<OAuthSession | null>(null)
  /* 仅活跃筛选:临时视图,不持久化,不影响 queryKey */
  const [activeOnly, setActiveOnly] = React.useState(false)
  const sessions = listQuery.data?.sessions ?? []
  const visibleSessions = React.useMemo(
    () => (activeOnly ? sessions.filter((s) => s.activeAccess + s.activeRefresh > 0) : sessions),
    [activeOnly, sessions]
  )

  return (
    <div className="rounded-xl border bg-card">
      {/* 摘要条 */}
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5 border-b bg-muted/30 px-4 py-2.5 sm:px-5">
        <span className="text-xs font-medium text-muted-foreground">
          会话用户 <span className="tabular-nums text-foreground">{visibleSessions.length}</span>
          {activeOnly && sessions.length !== visibleSessions.length && (
            <span className="text-muted-foreground/70"> / {sessions.length}</span>
          )}
        </span>
        <span className="text-xs text-muted-foreground">
          活跃访问令牌{" "}
          <span className="tabular-nums text-primary">
            {sessions.reduce((acc, s) => acc + s.activeAccess, 0)}
          </span>
        </span>
        <span className="text-xs text-muted-foreground">
          活跃刷新令牌{" "}
          <span className="tabular-nums text-emerald-600 dark:text-emerald-400">
            {sessions.reduce((acc, s) => acc + s.activeRefresh, 0)}
          </span>
        </span>
        <div className="ml-auto flex flex-wrap items-center gap-3">
          <Tooltip>
            <TooltipTrigger asChild>
              <label
                htmlFor="sessions-active-only"
                className="flex cursor-pointer select-none items-center gap-2"
              >
                <Switch
                  id="sessions-active-only"
                  checked={activeOnly}
                  onCheckedChange={setActiveOnly}
                  aria-label="仅显示有活跃令牌的会话"
                />
                <span className="text-xs text-muted-foreground">仅活跃</span>
              </label>
            </TooltipTrigger>
            <TooltipContent>仅显示有活跃令牌的会话</TooltipContent>
          </Tooltip>
          {!activeOnly && (
            <span className="text-[11px] text-muted-foreground/70">按最近签发时间倒序</span>
          )}
        </div>
      </div>

      {listQuery.isLoading ? (
        <div className="p-2">
          <TableSkeleton cols={6} />
        </div>
      ) : listQuery.isError ? (
        <ErrorState
          className="m-2 border-0 bg-transparent"
          message={listQuery.error instanceof Error ? listQuery.error.message : undefined}
          onRetry={() => listQuery.refetch()}
        />
      ) : sessions.length === 0 ? (
        <EmptyState
          icon={<UsersRound className="size-8" />}
          title="暂无会话"
          description="用户通过 OAuth 客户端获得令牌后,将按用户聚合在此展示"
        />
      ) : visibleSessions.length === 0 ? (
        /* 开启「仅活跃」后无匹配行的友好空态 */
        <EmptyState
          icon={<UsersRound className="size-8" />}
          title="当前没有活跃会话"
          description="全部会话的令牌均已吊销或过期,可关闭「仅活跃」查看完整列表"
          action={
            <Button size="sm" variant="outline" onClick={() => setActiveOnly(false)}>
              关闭仅活跃
            </Button>
          }
        />
      ) : (
        <div className="table-scroll max-h-[32rem] overflow-auto scroll-area">
        <Table>
          <TableHeader className="sticky top-0 z-10 bg-card">
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-5 text-xs font-medium uppercase tracking-wider text-muted-foreground">用户</TableHead>
              <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">客户端</TableHead>
              <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">活跃令牌</TableHead>
              <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">最近签发</TableHead>
              <TableHead className="text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">已吊销</TableHead>
              <TableHead className="pr-5 text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">操作</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody className={DENSITY_BODY_CLASS[density]}>
            {visibleSessions.map((s) => {
              const active = s.activeAccess > 0
              const shownClients = s.clients.slice(0, 3)
              const moreClients = s.clients.length - shownClients.length
              return (
                <TableRow
                  key={s.userId}
                  className={cn(
                    "transition-[background-color,transform,box-shadow] duration-150",
                    "hover:-translate-y-[1px] hover:shadow-md hover:shadow-black/5"
                  )}
                >
                  <TableCell className="pl-5">
                    <div className="flex items-center gap-2.5">
                      {/* 活跃会话呼吸圆点 */}
                      <span
                        className={cn(
                          "size-2 shrink-0 rounded-full",
                          active && "animate-pulse bg-emerald-500"
                        )}
                        title={active ? "存在活跃访问令牌" : "无活跃会话"}
                        aria-hidden
                      />
                      <Avatar className="size-8 border">
                        <AvatarFallback className="bg-gradient-to-br from-primary/20 to-emerald-500/25 text-xs font-semibold text-primary">
                          {(s.realName || s.username || "?").slice(0, 1).toUpperCase()}
                        </AvatarFallback>
                      </Avatar>
                      <div className="min-w-0">
                        <p
                          className={cn(
                            "truncate text-sm font-medium",
                            active ? "text-foreground" : "text-muted-foreground"
                          )}
                        >
                          {s.username}
                        </p>
                        <p className="truncate text-xs text-muted-foreground">
                          {s.realName || "—"}
                        </p>
                      </div>
                    </div>
                  </TableCell>
                  <TableCell>
                    {s.clients.length === 0 ? (
                      <span className="text-xs text-muted-foreground">—</span>
                    ) : (
                      <div className="flex max-w-[16rem] flex-wrap items-center gap-1">
                        {shownClients.map((c) => (
                          <code
                            key={c}
                            className="rounded border bg-muted/50 px-1.5 py-px font-mono text-[11px] text-muted-foreground"
                          >
                            {c}
                          </code>
                        ))}
                        {moreClients > 0 && (
                          <Badge variant="secondary" className="tabular-nums text-[10px]">
                            +{moreClients}
                          </Badge>
                        )}
                      </div>
                    )}
                  </TableCell>
                  <TableCell>
                    <div className="flex items-center gap-1.5">
                      <Badge
                        variant="outline"
                        className={cn(
                          "gap-1 border-primary/30 bg-primary/10 text-primary",
                          s.activeAccess === 0 && "border-border bg-muted/40 text-muted-foreground/70"
                        )}
                      >
                        access <span className="tabular-nums">{s.activeAccess}</span>
                      </Badge>
                      <Badge
                        variant="outline"
                        className={cn(
                          "gap-1 border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
                          s.activeRefresh === 0 &&
                            "border-border bg-muted/40 text-muted-foreground/70"
                        )}
                      >
                        refresh <span className="tabular-nums">{s.activeRefresh}</span>
                      </Badge>
                    </div>
                  </TableCell>
                  <TableCell className="text-xs text-muted-foreground" title={formatDateTime(s.lastIssuedAt)}>
                    {s.lastIssuedAt ? formatRelative(s.lastIssuedAt) : "—"}
                  </TableCell>
                  <TableCell className="text-right text-xs tabular-nums text-muted-foreground">
                    {s.revokedCount}
                  </TableCell>
                  <TableCell className="pr-5 text-right">
                    <div className="flex items-center justify-end gap-0.5">
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="size-8"
                            aria-label={`查看 ${s.username} 的令牌`}
                            onClick={() => onInspectUser(s.username)}
                          >
                            <ScanSearch className="size-4" />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>查看该用户的令牌</TooltipContent>
                      </Tooltip>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="size-8 text-amber-600 hover:text-amber-600 dark:text-amber-500 dark:hover:text-amber-500"
                            aria-label={`踢出 ${s.username} 的全部会话`}
                            onClick={() => setKicking(s)}
                          >
                            <UserX className="size-4" />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>踢出全部: 吊销该用户的所有令牌</TooltipContent>
                      </Tooltip>
                    </div>
                  </TableCell>
                </TableRow>
              )
            })}
          </TableBody>
        </Table>
        </div>
      )}

      <KickSessionDialog
        open={!!kicking}
        initialUsername={kicking?.username ?? ""}
        onOpenChange={(o) => !o && setKicking(null)}
      />
    </div>
  )
}

/* ============ 会话踢出对话框 ============ */

/**
 * 行数据仅有 username (无 userId), 而踢出 API 需要 userId →
 * 对话框内嵌用户选择器: 打开时按行 username 预填, 也可改选其它用户。
 */
function KickSessionDialog({
  open,
  initialUsername,
  onOpenChange,
}: {
  open: boolean
  initialUsername: string
  onOpenChange: (o: boolean) => void
}) {
  const queryClient = useQueryClient()
  const [userId, setUserId] = React.useState("")

  const usersQuery = useQuery({
    queryKey: ["kick-session-users"],
    queryFn: async () =>
      asPage<{ id: string; username: string; realName: string }>(await usersApi.list({ page: 1, size: 200 })),
    enabled: open,
    staleTime: 60_000,
    retry: 1,
  })

  /* 打开时重置并按行用户名预填 */
  React.useEffect(() => {
    if (!open) {
      setUserId("")
      return
    }
    if (usersQuery.data && initialUsername) {
      const hit = usersQuery.data.records.find((u) => u.username === initialUsername)
      if (hit) setUserId(hit.id)
    }
  }, [open, usersQuery.data, initialUsername])

  const kickMutation = useMutation({
    mutationFn: (uid: string) => oauthApi.kickSessions(uid),
    onSuccess: (d) => {
      toast.success(d.revoked > 0 ? `已踢出会话,共吊销 ${d.revoked} 个令牌` : "该用户当前没有未吊销的令牌")
      onOpenChange(false)
      void queryClient.invalidateQueries({ queryKey: ["oauth-tokens"] })
      void queryClient.invalidateQueries({ queryKey: ["oauth-sessions"] })
    },
    onError: (e: Error) => toast.error(e.message || "踢出失败"),
  })

  const selected = usersQuery.data?.records.find((u) => u.id === userId)

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle className="flex items-center gap-2 text-destructive">
            <UserX className="size-5" />
            踢出会话?
          </AlertDialogTitle>
          <AlertDialogDescription>
            将吊销该用户全部未过期令牌,受影响设备需重新授权后才能继续访问;该用户的浏览器会话令牌同样会失效。
          </AlertDialogDescription>
        </AlertDialogHeader>

        <div className="space-y-2 py-1">
          <Label htmlFor="kick-user" className="text-xs text-muted-foreground">
            目标用户{initialUsername && userId ? "(已按令牌归属预填)" : ""}
          </Label>
          <Select value={userId} onValueChange={setUserId} disabled={kickMutation.isPending}>
            <SelectTrigger id="kick-user" aria-label="选择目标用户">
              <SelectValue
                placeholder={usersQuery.isLoading ? "加载用户中..." : "选择要踢出的用户"}
              />
            </SelectTrigger>
            <SelectContent className="max-h-64">
              {usersQuery.isLoading ? (
                <div className="flex items-center justify-center gap-2 py-6 text-xs text-muted-foreground">
                  <Loader2 className="size-4 animate-spin" />
                  加载用户中...
                </div>
              ) : usersQuery.data && usersQuery.data.records.length > 0 ? (
                usersQuery.data.records.map((u) => (
                  <SelectItem key={u.id} value={u.id}>
                    {u.realName || u.username}({u.username})
                  </SelectItem>
                ))
              ) : (
                <div className="flex items-center gap-2 px-2 py-4 text-xs text-muted-foreground">
                  <UserRound className="size-4" />
                  未加载到用户,请确认拥有用户查看权限
                </div>
              )}
            </SelectContent>
          </Select>
          {selected && (
            <p className="text-xs text-muted-foreground">
              即将踢出 <span className="font-medium text-foreground">{selected.realName || selected.username}</span>
              (账号 <code className="font-mono">{selected.username}</code>) 的全部会话。
            </p>
          )}
        </div>

        <AlertDialogFooter>
          <AlertDialogCancel disabled={kickMutation.isPending}>取消</AlertDialogCancel>
          <AlertDialogAction
            className={cn(buttonVariants({ variant: "destructive" }))}
            disabled={!userId || kickMutation.isPending}
            onClick={(e) => {
              e.preventDefault()
              if (userId) kickMutation.mutate(userId)
            }}
          >
            {kickMutation.isPending && <Loader2 className="size-4 animate-spin" />}
            踢出并吊销令牌
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

/* ============ 设备码表 ============ */

function DevicesTable() {
  const queryClient = useQueryClient()
  const { density } = useTableDensity()
  const listQuery = useQuery({
    queryKey: ["oauth-device-codes"],
    queryFn: async () => asArray<DeviceCodeRecord>(await oauthApi.deviceCodes()),
    staleTime: 15_000,
    retry: 1,
  })

  const [revoking, setRevoking] = React.useState<DeviceCodeRecord | null>(null)
  const revokeMutation = useMutation({
    mutationFn: (id: string) => oauthApi.revokeDeviceCode(id),
    onSuccess: () => {
      toast.success("设备码已吊销")
      setRevoking(null)
      queryClient.invalidateQueries({ queryKey: ["oauth-device-codes"] })
    },
    onError: (e: Error) => toast.error(e.message || "吊销失败"),
  })

  const devices = listQuery.data ?? []

  return (
    <div className="rounded-xl border bg-card">
      {listQuery.isLoading ? (
        <div className="p-2">
          <TableSkeleton cols={6} />
        </div>
      ) : listQuery.isError ? (
        <ErrorState
          className="m-2 border-0 bg-transparent"
          message={listQuery.error instanceof Error ? listQuery.error.message : undefined}
          onRetry={() => listQuery.refetch()}
        />
      ) : devices.length === 0 ? (
        <EmptyState title="暂无设备码" description="设备发起 RFC 8628 授权后将在此出现" icon={<Tv className="size-6" />} />
      ) : (
        <div className="table-scroll max-h-[32rem] overflow-auto scroll-area">
        <Table>
          <TableHeader className="sticky top-0 z-10 bg-card">
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-5 text-xs font-medium uppercase tracking-wider text-muted-foreground">设备码</TableHead>
              <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">客户端</TableHead>
              <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">状态</TableHead>
              <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">授权用户</TableHead>
              <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">Scope</TableHead>
              <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">过期时间</TableHead>
              <TableHead className="pr-5 text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">操作</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody className={DENSITY_BODY_CLASS[density]}>
            {devices.map((d) => {
              const expired = isExpired(d.expiresAt)
              const terminal = d.status !== "PENDING" || expired
              return (
                <TableRow key={d.id}>
                  <TableCell className="pl-5">
                    <code className="font-mono text-base font-semibold tracking-[0.2em]">{d.userCode}</code>
                  </TableCell>
                  <TableCell>
                    <p className="text-sm font-medium">{d.clientName || d.clientId}</p>
                    <code className="font-mono text-xs text-muted-foreground">{d.clientId}</code>
                  </TableCell>
                  <TableCell>
                    <StatusBadge status={expired && d.status === "PENDING" ? "EXPIRED" : d.status} pulse />
                  </TableCell>
                  <TableCell className="text-sm">{d.username || "—"}</TableCell>
                  <TableCell>
                    <div className="flex max-w-[10rem] flex-wrap gap-1">
                      {d.scope
                        ? d.scope.split(/\s+/).filter(Boolean).map((s) => (
                            <Badge key={s} variant="secondary" className="text-[10px] font-normal">
                              {s}
                            </Badge>
                          ))
                        : "—"}
                    </div>
                  </TableCell>
                  <TableCell
                    className={cn("text-xs", expired ? "font-medium text-destructive" : "text-muted-foreground")}
                    title={formatDateTime(d.expiresAt)}
                  >
                    {formatRelative(d.expiresAt)}
                  </TableCell>
                  <TableCell className="pr-5 text-right">
                    {!terminal && (
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="size-8 text-destructive hover:text-destructive"
                            aria-label="吊销设备码"
                            disabled={revokeMutation.isPending}
                            onClick={() => setRevoking(d)}
                          >
                            {revokeMutation.isPending && revokeMutation.variables === d.id ? (
                              <Loader2 className="size-4 animate-spin" />
                            ) : (
                              <ShieldOff className="size-4" />
                            )}
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>吊销</TooltipContent>
                      </Tooltip>
                    )}
                  </TableCell>
                </TableRow>
              )
            })}
          </TableBody>
        </Table>
        </div>
      )}

      <ConfirmDialog
        open={!!revoking}
        onOpenChange={(o) => !o && setRevoking(null)}
        title="吊销该设备码?"
        description={
          <>
            将吊销设备码 <code className="font-mono font-semibold">{revoking?.userCode}</code>
            ,设备端轮询将收到 expired_token,需重新发起授权。
          </>
        }
        confirmText="吊销"
        loading={revokeMutation.isPending}
        onConfirm={() => revoking && revokeMutation.mutate(revoking.id)}
      />
    </div>
  )
}
