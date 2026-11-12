import * as React from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { MonitorSmartphone, RefreshCw, Search, ShieldOff, Users } from "lucide-react"
import { toast } from "sonner"
import { sessionsApi } from "@/api/sessions"
import type { ManagedSession, SessionUser } from "@/api/sessions"
import { PageHeader } from "@/components/layout/PageHeader"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Badge } from "@/components/ui/badge"
import { Switch } from "@/components/ui/switch"
import { Label } from "@/components/ui/label"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Table, TableHeader, TableHead, TableRow, TableCell, TableBody } from "@/components/ui/table"
import { TableSkeleton, ErrorState, EmptyState } from "@/components/states"
import { ConfirmDialog } from "@/components/ConfirmDialog"
import { DataPagination } from "@/components/DataPagination"
import { usePermissionsQuery } from "@/hooks/use-auth-data"
import { captureAuthSession, isCurrentAuthSession } from "@/lib/auth-queries"
import type { AuthQuerySession } from "@/lib/auth-queries"
import { formatDateTime, useDebounced } from "@/lib/format"
import { useAuthStore } from "@/stores/auth"

type Target = { kind: "device" | "user"; id: string; name: string; current: boolean }

function savedOperation(key: string): string | null {
  try { const value = sessionStorage.getItem(key); return value && /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value) ? value : null } catch { return null }
}
function saveOperation(key: string, operation: string | null) {
  try { if (operation) sessionStorage.setItem(key, operation); else sessionStorage.removeItem(key) } catch { /* Keep recovery available in component state. */ }
}

export default function SessionsPage() {
  const epoch = useAuthStore(state => state.contextEpoch)
  const namespace = useAuthStore(state => state.contextKey)
  const session = captureAuthSession()
  return <SessionWorkspace key={`${namespace}:${epoch}`} session={session} />
}

export function SessionWorkspace({ session }: { session: AuthQuerySession }) {
  const client = useQueryClient()
  const permissions = usePermissionsQuery()
  const canWrite = permissions.isSuccess && !!permissions.data?.permissions.includes("system:session:revoke")
  const [view, setView] = React.useState("devices")
  const [search, setSearch] = React.useState("")
  const keyword = useDebounced(search, 300)
  const [activeOnly, setActiveOnly] = React.useState(true)
  const [page, setPage] = React.useState(1)
  const [size, setSize] = React.useState(20)
  const [target, setTarget] = React.useState<Target | null>(null)
  const operationKey = `admin-session-operation:${session.namespace}`
  const [pendingOperation, setPendingOperation] = React.useState<string | null>(() => savedOperation(operationKey))
  const mounted = React.useRef(true)
  React.useEffect(() => { mounted.current = true; return () => { mounted.current = false } }, [])
  const current = () => mounted.current && isCurrentAuthSession(session)
  const filters = { page, size, keyword, activeOnly }
  const devices = useQuery({ queryKey: ["admin-sessions", session.namespace, session.epoch, "devices", filters], queryFn: ({ signal }) => sessionsApi.devices(filters, signal), enabled: view === "devices" && isCurrentAuthSession(session), refetchInterval: 30_000 })
  const users = useQuery({ queryKey: ["admin-sessions", session.namespace, session.epoch, "users", filters], queryFn: ({ signal }) => sessionsApi.users(filters, signal), enabled: view === "users" && isCurrentAuthSession(session), refetchInterval: 30_000 })
  const query = view === "devices" ? devices : users
  const mutation = useMutation({
    mutationFn: async (requested: Target) => {
      if (!current() || !canWrite) throw new Error("当前会话或操作权限已变更，请刷新")
      return requested.kind === "device" ? sessionsApi.revoke(requested.id) : sessionsApi.revokeAll(requested.id)
    },
    onSuccess: () => {
      if (!current()) return
      toast.success("会话已吊销")
      setTarget(null)
      void client.invalidateQueries({ queryKey: ["admin-sessions"] })
    },
    onError: (error: Error) => {
      if (!current()) return
      toast.error(error.message || "吊销失败")
      const operation = error.message.match(/操作编号 ([0-9a-f-]{36})/i)?.[1]
      if (operation) { setPendingOperation(operation); saveOperation(operationKey, operation) }
    },
  })
  const recovery = useMutation({
    mutationFn: async (operation: string) => { if (!current() || !canWrite) throw new Error("当前会话或操作权限已变更"); return sessionsApi.recover(operation) },
    onSuccess: () => { if (!current()) return; setPendingOperation(null); saveOperation(operationKey, null); setTarget(null); toast.success("原吊销操作已完成"); void client.invalidateQueries({ queryKey: ["admin-sessions"] }) },
    onError: (error: Error) => { if (current()) toast.error(error.message) },
  })
  const busy = mutation.isPending || recovery.isPending || !!pendingOperation
  const action = (next: Target, disabled = false) => canWrite && <Button variant="outline" size="sm" disabled={busy || disabled} onClick={() => setTarget(next)}><ShieldOff className="size-3.5" />{next.kind === "device" ? "吊销设备" : "踢出全部"}</Button>
  return <div>
    <PageHeader title="设备与会话" description="查看平台登录设备，撤销设备会话或强制用户重新登录" icon={<MonitorSmartphone className="size-5" />} actions={<Button variant="outline" disabled={query.isFetching} onClick={() => void query.refetch()}><RefreshCw className="size-4" />刷新</Button>} />
    <div className="mb-5 flex flex-wrap items-center justify-between gap-3">
      <Tabs value={view} onValueChange={value => { setView(value); setPage(1) }}><TabsList><TabsTrigger value="devices"><MonitorSmartphone className="mr-1.5 size-4" />登录设备</TabsTrigger><TabsTrigger value="users"><Users className="mr-1.5 size-4" />用户会话</TabsTrigger></TabsList></Tabs>
      <div className="flex flex-wrap items-center gap-4"><div className="relative"><Search className="absolute left-3 top-2.5 size-4 text-muted-foreground" /><Input className="w-64 pl-9" aria-label="搜索账号或姓名" placeholder="搜索账号或姓名" value={search} onChange={event => { setSearch(event.target.value); setPage(1) }} /></div><div className="flex items-center gap-2"><Switch id="session-active" checked={activeOnly} onCheckedChange={value => { setActiveOnly(value); setPage(1) }} /><Label htmlFor="session-active">仅活跃</Label></div></div>
    </div>
    {pendingOperation && <div className="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-lg border border-amber-500/30 bg-amber-500/5 p-4 text-sm" role="status"><p>吊销操作已受理，后续处理待确认。请恢复原操作后再继续。</p><Button disabled={!canWrite || recovery.isPending} onClick={() => recovery.mutate(pendingOperation)}>恢复原操作</Button></div>}
    {query.isPending ? <TableSkeleton cols={view === "devices" ? 7 : 5} /> : query.isError ? <ErrorState message={query.error.message} onRetry={() => void query.refetch()} /> : query.data?.records.length === 0 ? <EmptyState title={activeOnly ? "当前没有匹配的活跃会话" : "暂无匹配的会话记录"} description="可调整搜索条件或关闭“仅活跃”，查看历史登录记录。" /> : <div className="overflow-x-auto rounded-xl border bg-card"><Table>
      <TableHeader><TableRow><TableHead>用户</TableHead>{view === "devices" ? <><TableHead>租户 / 组织</TableHead><TableHead>设备与 IP</TableHead><TableHead>登录时间</TableHead><TableHead>到期时间</TableHead><TableHead>状态</TableHead></> : <><TableHead>活跃设备</TableHead><TableHead>历史会话</TableHead><TableHead>最近登录</TableHead></>}<TableHead className="text-right">操作</TableHead></TableRow></TableHeader>
      <TableBody>{view === "devices" ? devices.data?.records.map((row: ManagedSession) => <TableRow key={row.id}><TableCell><p className="font-medium">{row.realName || row.username}</p><p className="text-xs text-muted-foreground">{row.username}</p></TableCell><TableCell><p>{row.tenantName}</p><p className="text-xs text-muted-foreground">{row.organizationName}</p></TableCell><TableCell><p className="max-w-64 truncate text-xs" title={row.userAgent}>{row.userAgent || "未知设备"}</p><p className="font-mono text-xs text-muted-foreground">{row.ip || "未知 IP"}</p></TableCell><TableCell className="whitespace-nowrap text-xs">{formatDateTime(row.issuedAt)}</TableCell><TableCell className="whitespace-nowrap text-xs">{formatDateTime(row.expiresAt)}</TableCell><TableCell><Badge variant={row.active ? "secondary" : "outline"}>{row.isCurrent ? "当前设备" : row.active ? "活跃" : row.revokedAt ? "已吊销" : "已失效"}</Badge></TableCell><TableCell className="text-right">{action({ kind: "device", id: row.id, name: row.realName || row.username, current: row.isCurrent }, !row.active)}</TableCell></TableRow>) : users.data?.records.map((row: SessionUser) => <TableRow key={row.userId}><TableCell><p className="font-medium">{row.realName || row.username}</p><p className="text-xs text-muted-foreground">{row.username}</p></TableCell><TableCell className="tabular-nums">{row.activeSessions}</TableCell><TableCell className="tabular-nums text-muted-foreground">{row.sessions}</TableCell><TableCell className="text-xs">{formatDateTime(row.lastIssuedAt)}</TableCell><TableCell className="text-right">{action({ kind: "user", id: row.userId, name: row.realName || row.username, current: row.isCurrentUser })}</TableCell></TableRow>)}</TableBody>
    </Table></div>}
    {query.isSuccess && <DataPagination page={page} size={size} total={query.data.total} onPageChange={setPage} onSizeChange={value => { setSize(value); setPage(1) }} />}
    <ConfirmDialog open={!!target} onOpenChange={open => { if (!open && !busy) setTarget(null) }} title={target?.kind === "device" ? "吊销该设备的平台会话？" : "踢出该用户的全部会话？"} description={<>{target?.kind === "device" ? `将撤销「${target?.name}」在该设备上的平台登录，其他设备不受影响。` : `将通过认证中心撤销「${target?.name}」的全部登录会话及授权令牌，受影响的应用需要重新登录。`}{target?.current && <span className="mt-2 block font-medium text-destructive">包含当前登录，完成后你也需要重新登录。</span>}</>} confirmText={target?.kind === "device" ? "确认吊销" : "踢出全部"} loading={busy} onConfirm={() => { if (target && !busy && current() && canWrite) mutation.mutate(target) }} />
  </div>
}
