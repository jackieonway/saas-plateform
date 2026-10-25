import { isUuidV7 } from "@/lib/uuid"
import * as React from "react"
import { Link, useSearchParams } from "react-router-dom"
import { useQuery } from "@tanstack/react-query"
import { CheckCircle2, Eye, Info, Loader2, Search, ShieldCheck } from "lucide-react"
import { PageHeader } from "@/components/layout/PageHeader"
import { EmptyState, ErrorState } from "@/components/states"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { auditApi } from "@/api/audit"
import { dataGrantApi } from "@/api/data-grants"
import type { DataGrantChange, DataGrantOptions, DataGrantPreview, DataGrantRow, DataGrantScope, DataGrantSnapshot } from "@/api/data-grants"
import type { AuditLogRecord } from "@/api/types"
import { usePermissionsQuery } from "@/hooks/use-auth-data"
import { useAuthorizedPaths } from "@/hooks/use-authorized-paths"
import { asPage } from "@/lib/api"
import { captureAuthSession } from "@/lib/auth-queries"
import type { AuthQuerySession } from "@/lib/auth-queries"
import { grantKey } from "@/lib/data-grants"
import { useAuthStore } from "@/stores/auth"
import { readInView, snapshotKey, targetKey, useGrantEditor, useViewFence } from "./useGrantEditor"
import type { SaveReceipt } from "./useGrantEditor"

type Member = DataGrantOptions["records"][number]
const NO_GRANT = "__none__"
const scopeLabels: Record<DataGrantScope, string> = {
  SELF: "SELF · 本人", ORG: "ORG · 当前组织", ORG_SUBTREE: "ORG_SUBTREE · 当前组织及下级",
  TENANT: "TENANT · 租户（仍受组织边界限制）",
}

export default function DataGrantsPage() {
  const epoch = useAuthStore(state => state.contextEpoch)
  const sessionId = useAuthStore(state => state.sessionId)
  const switching = useAuthStore(state => state.contextSwitching)
  const profile = useAuthStore(state => state.profile)
  const permissions = usePermissionsQuery()
  const destinations = useAuthorizedPaths()
  const ready = permissions.isSuccess && !switching && !!profile
  const allowed = new Set(ready ? permissions.data?.permissions ?? [] : [])
  const session = captureAuthSession()
  const key = JSON.stringify([epoch, sessionId, session.identity, profile?.platformContext === true, [...allowed].sort()])
  return <div className="space-y-5">
    <PageHeader title="数据授权" description="为明确的直接成员维护应用数据范围，每次变更先预览再保存" icon={<ShieldCheck className="size-5" />} />
    <Alert><Info /><AlertTitle>功能权限与数据范围分别校验</AlertTitle><AlertDescription>
      <p>菜单与按钮决定能做什么，数据授权决定可操作哪些数据；角色继承不会传递直接成员的数据授权。</p>
      <p>没有对应授权即默认拒绝。所有范围仍受目标组织硬边界限制，不自动授予租户范围。</p>
    </AlertDescription></Alert>
    {!ready ? (permissions.isError ? <ErrorState message="无法确认当前管理权限，请重新加载权限" onRetry={() => { void permissions.refetch() }} />
      : <Loading text="正在确认当前会话与管理权限" />)
      : !allowed.has("system:data-grant:read") ? <ErrorState message="需要独立的数据授权查询权限" />
      : <GrantWorkspace key={key} session={session} canWrite={allowed.has("system:data-grant:write") && profile?.platformContext === true} canAudit={allowed.has("system:audit")} canUsers={destinations.allowedPaths.has("/system/users")} canAuthorizations={destinations.allowedPaths.has("/system/authorizations")} />}
  </div>
}

function Loading({ text }: { text: string }) {
  return <div role="status" className="flex items-center gap-2 rounded-xl border bg-card p-6 text-sm text-muted-foreground"><Loader2 className="size-4 animate-spin" />{text}</div>
}

function GrantWorkspace({ session, canWrite, canAudit, canUsers, canAuthorizations }: { session: AuthQuerySession; canWrite: boolean; canAudit: boolean; canUsers: boolean; canAuthorizations: boolean }) {
  const [params] = useSearchParams()
  const current = useViewFence(session)
  const [requestedApp, setRequestedApp] = React.useState(params.get("applicationId") ?? "")
  const [keyword, setKeyword] = React.useState("")
  const [search, setSearch] = React.useState("")
  const [page, setPage] = React.useState(1)
  const [selected, setSelected] = React.useState("")
  const generation = React.useRef(0)
  const userId = params.get("userId") ?? undefined
  const validUserFilter = !userId || isUuidV7(userId)
  const apps = useQuery({
    queryKey: ["data-grants", "options", "applications", session.epoch, userId ?? null],
    queryFn: ({ signal }) => readInView(current, () => dataGrantApi.options({ userId }, signal)),
    enabled: validUserFilter, retry: false, staleTime: 0, refetchOnWindowFocus: false,
  })
  const applicationsReady = apps.isSuccess && !apps.isFetching
  const authorizedApps = applicationsReady ? apps.data.applications : []
  const applicationId = authorizedApps.some(app => app.id === requestedApp) ? requestedApp : ""
  const members = useQuery({
    queryKey: ["data-grants", "options", "members", session.epoch, applicationId, userId ?? null, search, page],
    queryFn: async ({ signal }) => {
      const revision = generation.current
      return readInView(() => current() && generation.current === revision,
        () => dataGrantApi.options({ applicationId, userId, keyword: search || undefined, page, size: 10 }, signal))
    }, enabled: !!applicationId && validUserFilter, retry: false, staleTime: 0, refetchOnWindowFocus: false,
  })
  const records = !members.isFetching && members.isSuccess ? members.data.records : []
  const member = records.find(record => record.target.membershipId === selected)
  function reset() { generation.current++; setSelected("") }
  const appName = authorizedApps.find(app => app.id === applicationId)?.name
  return <>
    <Card>
      <CardHeader><CardTitle>选择应用与直接成员</CardTitle><CardDescription>只列出当前会话与服务端策略明确允许的目标。不会自动选择第一项。</CardDescription></CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap items-end gap-3">
          <div className="space-y-2"><p className="text-sm font-medium">目标应用</p><Select value={applicationId} disabled={!applicationsReady} onValueChange={value => { reset(); setRequestedApp(value); setPage(1); setKeyword(""); setSearch("") }}>
            <SelectTrigger aria-label="选择数据授权应用" className="w-72"><SelectValue placeholder="请选择应用" /></SelectTrigger>
            <SelectContent>{authorizedApps.map(app => <SelectItem key={app.id} value={app.id}>{app.name} · {app.code}</SelectItem>)}</SelectContent>
          </Select></div>
          <form className="flex gap-2" onSubmit={event => { event.preventDefault(); reset(); setSearch(keyword.trim()); setPage(1) }}>
            <Input aria-label="搜索成员" placeholder="姓名或账号" className="w-56" maxLength={100} value={keyword} disabled={!applicationId} onChange={event => setKeyword(event.target.value)} />
            <Button variant="outline" disabled={!applicationId || members.isFetching}><Search className="size-4" />搜索</Button>
          </form>
          {!canWrite && <Badge variant="secondary"><Eye className="mr-1 size-3" />只读管理</Badge>}
        </div>
        {!validUserFilter && <p role="alert" className="text-sm text-destructive">用户筛选无效，请从成员管理重新选择目标。</p>}
        {requestedApp && applicationsReady && !applicationId && <p role="alert" className="text-sm text-destructive">指定应用不在当前可查询范围内，请明确选择允许的应用。</p>}
        {apps.isError ? <ErrorState message="应用选项读取失败" onRetry={() => { void apps.refetch() }} />
          : apps.isFetching ? <Loading text="正在读取当前可查询的应用" />
          : !applicationId ? <EmptyState title="请选择应用" description="随后查询该应用下可管理的直接成员，不接受手填成员身份。" className="py-8" />
          : members.isFetching ? <Loading text="正在读取当前范围内的成员" />
          : members.isError ? <ErrorState message="成员选项读取失败，请重新查询" onRetry={() => { void members.refetch() }} />
          : <>
            <div className="grid gap-2 lg:grid-cols-2">{records.map(record => <button key={record.target.membershipId} type="button"
              data-testid={`grant-member-${record.target.membershipId}`}
              aria-label={`选择成员 ${record.displayName || record.username} · ${record.organizationName}`}
              aria-pressed={record.target.membershipId === selected}
              onClick={() => { reset(); setSelected(record.target.membershipId) }}
              className={`rounded-lg border p-3 text-left transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring ${record.target.membershipId === selected ? "border-primary bg-primary/5" : "bg-background"}`}>
              <span className="block text-sm font-medium">{record.displayName || record.username} <span className="font-normal text-muted-foreground">@{record.username}</span></span>
              <span className="mt-1 block text-xs text-muted-foreground">{record.tenantName} / {record.organizationName}</span>
            </button>)}</div>
            {records.length === 0 && <EmptyState title="没有可查询的成员" description="目标需有有效直接成员、应用授权及明确的管理范围。" className="py-8" />}
            <div className="flex items-center justify-between text-xs text-muted-foreground"><span>共 {members.data?.total ?? 0} 项 · 第 {page} 页</span><div className="flex gap-2">
              <Button size="sm" variant="outline" disabled={page <= 1} onClick={() => { reset(); setPage(page - 1) }}>上一页</Button>
              <Button size="sm" variant="outline" disabled={page * 10 >= (members.data?.total ?? 0)} onClick={() => { reset(); setPage(page + 1) }}>下一页</Button>
            </div></div>
          </>}
      </CardContent>
    </Card>
    {member && <TargetLoader key={JSON.stringify([...targetKey(member.target), member.canWrite, canWrite, canAudit])} member={member} applicationName={appName ?? ""} session={session} canWrite={canWrite && member.canWrite} canAudit={canAudit} canUsers={canUsers} canAuthorizations={canAuthorizations} />}
  </>
}

function TargetLoader(props: { member: Member; applicationName: string; session: AuthQuerySession; canWrite: boolean; canAudit: boolean; canUsers: boolean; canAuthorizations: boolean }) {
  const current = useViewFence(props.session)
  // A newly selected target always performs a fresh read; an earlier visit's
  // cached snapshot must not reappear while current authorization is unknown.
  const selection = React.useId()
  const query = useQuery({
    queryKey: [...snapshotKey(props.session.epoch, props.member.target), "selection", selection],
    queryFn: ({ signal }) => readInView(current, () => dataGrantApi.read(props.member.target, signal)),
    retry: false, staleTime: Infinity, refetchOnWindowFocus: false,
  })
  return query.isPending ? <Loading text="正在读取目标的实时授权" />
    : query.isError ? <ErrorState message="无法读取该目标的当前授权" onRetry={() => { void query.refetch() }} />
    : <GrantEditor {...props} initial={query.data} />
}

function GrantEditor({ initial, member, applicationName, session, canWrite, canAudit, canUsers, canAuthorizations }: {
  initial: DataGrantSnapshot; member: Member; applicationName: string; session: AuthQuerySession; canWrite: boolean; canAudit: boolean; canUsers: boolean; canAuthorizations: boolean
}) {
  const editor = useGrantEditor(initial, session, canWrite)
  const snapshot = editor.snapshot
  const capabilities = new Map(snapshot.capabilities.map(capability => [grantKey(capability), capability]))
  const stored = new Map(snapshot.grants.map(grant => [grantKey(grant), grant]))
  const pairs = [...snapshot.capabilities, ...snapshot.grants.filter(grant => !capabilities.has(grantKey(grant)))]
  return <Card>
    <CardHeader><div className="flex flex-wrap items-center justify-between gap-3"><div className="space-y-2"><CardTitle>{member.displayName || member.username} 的数据授权</CardTitle><CardDescription>{member.tenantName} / {member.organizationName} · {applicationName}</CardDescription></div>
      <Button variant="outline" size="sm" disabled={editor.busy} onClick={() => { void editor.reload() }}>重新读取授权</Button></div>
      <p className="break-all font-mono text-xs text-muted-foreground">直接成员 {member.target.membershipId}</p>
    </CardHeader>
    <CardContent className="space-y-5">
      {editor.message && <Alert variant={editor.requiresRead || editor.phase === "denied" ? "destructive" : "default"}><Info /><AlertTitle>{editor.message}</AlertTitle><AlertDescription>未确认的结果不会自动重试；读取当前状态后需要重新审阅和预览。</AlertDescription></Alert>}
      {!editor.writable && <div className="flex items-center gap-2 text-sm text-muted-foreground"><Eye className="size-4" />只读视图：写入需要独立写权限、实际平台根上下文及目标可写范围。</div>}
      {editor.phase === "refreshing" ? <Loading text="正在重新读取当前授权" /> : !editor.readUnavailable && <div data-testid="grant-current-snapshot" data-version={snapshot.version} data-membership-id={snapshot.target.membershipId} className="space-y-3">
        <p className="text-xs text-muted-foreground">当前版本 <span className="font-mono" title={snapshot.version}>{snapshot.version.slice(0, 12)}…</span> · {snapshot.grants.length} 条已存授权</p>
        <div className="overflow-x-auto rounded-lg border"><Table><TableHeader><TableRow><TableHead>资源 / 操作</TableHead><TableHead>已存范围</TableHead><TableHead>实际范围与组织硬边界</TableHead><TableHead>拟授予范围</TableHead></TableRow></TableHeader>
          <TableBody>{pairs.map(pair => {
            const key = grantKey(pair), grant = stored.get(key), capability = capabilities.get(key)
            const supported = Object.hasOwn(editor.draft, key)
            return <TableRow key={key}><TableCell><span className="font-mono text-sm">{pair.resource}/{pair.action}</span>{(!capability || !supported) && <Badge variant="secondary" className="mt-1 block w-fit">历史授权 · 当前不可编辑</Badge>}</TableCell>
              <TableCell><Badge variant="outline">{grant?.scope ?? "未授权"}</Badge>{grant && <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">{grant.id}</p>}</TableCell>
              <TableCell><GrantBoundary grant={grant} /></TableCell>
              <TableCell>{editor.writable && capability && supported ? <Select value={editor.draft[key] ?? NO_GRANT} disabled={!editor.editable} onValueChange={value => editor.edit(key, value === NO_GRANT ? null : value as DataGrantScope)}>
                <SelectTrigger className="min-w-52" aria-label={`${pair.resource}/${pair.action} 数据范围`}><SelectValue /></SelectTrigger><SelectContent><SelectItem value={NO_GRANT}>不授权</SelectItem>{capability.scopes.map(scope => <SelectItem key={scope} value={scope}>{scopeLabels[scope]}</SelectItem>)}</SelectContent>
              </Select> : <span className="text-sm text-muted-foreground">{grant ? `${grant.scope} · 只读` : "不授权 · 默认拒绝"}</span>}</TableCell>
            </TableRow>
          })}</TableBody></Table></div>
        {pairs.length === 0 && <EmptyState title="没有已登记的可管理能力" description="不会新增任意资源、操作或默认范围。" className="py-6" />}
        <p className="text-xs text-muted-foreground">SELF 仍需校验数据所有者。TENANT 的实际范围也可能收窄为组织子树，以服务端有效范围与组织 ID 为准。</p>
      </div>}
      {editor.writable && !editor.requiresRead && !editor.readUnavailable && <div className="flex flex-wrap items-center gap-3"><Button disabled={!editor.editable || editor.busy || !editor.hasChanges} onClick={() => { void editor.prepare() }}>{editor.phase === "previewing" && <Loader2 className="size-4 animate-spin" />}预览变更</Button>
        {!editor.hasChanges && <span className="text-sm text-muted-foreground">没有实际变更，无需保存</span>}
      </div>}
      {editor.preview && editor.previewChange && editor.writable && <PreviewPanel preview={editor.preview} change={editor.previewChange} busy={editor.busy} onSave={() => { void editor.save() }} />}
      {editor.receipt && <ReceiptPanel receipt={editor.receipt} session={session} canAudit={canAudit} />}
      <p className="text-xs text-muted-foreground">功能菜单和角色分配单独维护。{canAuthorizations && <Link className="mr-2 text-primary underline-offset-4 hover:underline" to="/system/authorizations">授权管理</Link>}{canUsers && <Link className="text-primary underline-offset-4 hover:underline" to="/system/users">用户管理</Link>}这些入口仍需各自权限。</p>
    </CardContent>
  </Card>
}

function GrantBoundary({ grant }: { grant?: DataGrantRow }) {
  if (!grant) return <span className="text-sm text-muted-foreground">默认拒绝</span>
  return <div className="space-y-1 text-sm"><span>{grant.effectiveScope}</span><details><summary className="cursor-pointer text-xs text-muted-foreground">组织硬边界（{grant.organizationIds.length}）</summary><ul className="mt-2 space-y-1 font-mono text-[11px] text-muted-foreground">{grant.organizationIds.map(id => <li key={id}>{id}</li>)}</ul></details></div>
}

function PreviewPanel({ preview, change, busy, onSave }: { preview: DataGrantPreview; change: DataGrantChange; busy: boolean; onSave: () => void }) {
  return <section aria-label="授权变更预览" className="space-y-4 rounded-lg border border-primary/30 bg-primary/5 p-4"><div><h3 className="font-medium">确认本次差量</h3><p className="mt-1 text-xs text-muted-foreground">仅保存下列明确变更，未列出的授权保持。预览不预留版本。</p></div>
    <p className="break-all text-xs text-muted-foreground">预览基于版本 <span className="font-mono">{change.expectedVersion}</span></p>
    <div className="grid gap-3 md:grid-cols-3">{([['新增', preview.changes.added], ['修改', preview.changes.updated], ['撤销', preview.changes.removed]] as const).map(([label, rows]) => <div key={label} className="rounded-md border bg-background p-3"><p className="text-xs font-medium text-muted-foreground">{label} · {rows.length}</p>{rows.map(row => <p key={grantKey(row)} className="mt-2 font-mono text-xs">{row.resource}/{row.action} · {row.scope}<span className="block font-sans text-muted-foreground">实际 {row.effectiveScope} · {row.organizationIds.length} 个组织</span></p>)}</div>)}</div>
    <div className="grid gap-3 md:grid-cols-2">{([['变更前', preview.before], ['变更后', preview.after]] as const).map(([label, rows]) => <div key={label} className="rounded-md border bg-background p-3"><h4 className="text-sm font-medium">{label}</h4>{rows.length === 0 ? <p className="mt-2 text-xs text-muted-foreground">无授权，默认拒绝</p> : rows.map(row => <div key={grantKey(row)} className="mt-2 space-y-1"><p className="font-mono text-xs">{row.resource}/{row.action} · 已存/拟存 {row.scope}</p><GrantBoundary grant={row} /></div>)}</div>)}</div>
    <details className="text-xs"><summary className="cursor-pointer text-muted-foreground">查看精确提交内容</summary><pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-all rounded border bg-background p-3 font-mono">{JSON.stringify(change, null, 2)}</pre></details>
    <Button disabled={busy} onClick={onSave}>{busy && <Loader2 className="size-4 animate-spin" />}确认保存</Button>
  </section>
}

function ReceiptPanel({ receipt, session, canAudit }: { receipt: SaveReceipt; session: AuthQuerySession; canAudit: boolean }) {
  const current = useViewFence(session)
  const [opened, setOpened] = React.useState(false)
  const target = receipt.snapshot.target
  const audit = useQuery({
    queryKey: ["data-grants", "receipt-audit", session.epoch, ...targetKey(target), receipt.requestId],
    queryFn: () => readInView(current, async () => {
      const result = asPage<AuditLogRecord>(await auditApi.list({ page: 1, size: 10, action: "DATA_GRANT_CHANGE", tenantId: target.tenantId, applicationId: target.applicationId, requestId: receipt.requestId }))
      return result.records.filter(row => row.action === "DATA_GRANT_CHANGE" && row.requestId === receipt.requestId && row.tenantId === target.tenantId && row.applicationId === target.applicationId)
    }), enabled: opened && canAudit && !!receipt.requestId, retry: false, staleTime: Infinity, refetchOnWindowFocus: false,
  })
  return <section aria-label="数据授权保存回执" className="space-y-3 rounded-lg border border-emerald-600/25 bg-emerald-500/5 p-4"><div className="flex items-center gap-2 font-medium"><CheckCircle2 className="size-4 text-emerald-600" />授权已保存</div>
    <p className="text-sm text-muted-foreground">已存 {receipt.snapshot.grants.length} 条授权。目标应用下一次受保护的新请求使用当前授权；已有页面不会因此获得额外功能权限。</p>
    <p className="break-all text-xs">请求编号 <span data-testid="grant-save-request-id" className="font-mono">{receipt.requestId ?? "未返回，无法按本次请求查询审计"}</span></p>
    {canAudit && receipt.requestId && <Button variant="outline" size="sm" disabled={audit.isFetching} onClick={() => { if (current()) { setOpened(true); if (opened) void audit.refetch() } }}>查询本次审计</Button>}
    {opened && (audit.isFetching ? <Loading text="正在核对本次审计" /> : audit.isError ? <p role="alert" className="text-sm text-destructive">审计查询不可用；本次保存结果以回执和重新读取状态为准。</p>
      : audit.isSuccess && (audit.data.length ? <ul className="space-y-2">{audit.data.map(row => <li key={row.id} className="rounded border bg-background p-2 text-xs">DATA_GRANT_CHANGE · {row.username} · {row.createdAt}<span className="mt-1 block font-mono">审计编号 {row.id}</span></li>)}</ul>
        : <p className="text-sm text-muted-foreground">本次审计记录暂不可确认，请核对查询范围或按请求编号联系审计管理员。</p>))}
  </section>
}
