import * as React from "react"
import { useQuery } from "@tanstack/react-query"
import { KeyRound, RefreshCw } from "lucide-react"
import { oauthApi, type OidcClientRecord } from "@/api/oauth"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { CopyButton } from "@/components/CopyButton"
import { DataPagination } from "@/components/DataPagination"
import { StatusBadge } from "@/components/StatusBadge"
import { EmptyState, ErrorState, TableSkeleton } from "@/components/states"

const PURPOSES: Record<OidcClientRecord["purpose"], string> = { PLATFORM_CONSOLE: "平台管理登录", CUSTOMER_PORTAL: "客户门户登录", ACCOUNT: "账号认证" }

export function OidcClientsPanel() {
  const query = useQuery({ queryKey: ["oidc-clients"], queryFn: () => oauthApi.oidcClients(), staleTime: 30_000, retry: 1 })
  const [page, setPage] = React.useState(1)
  const [size, setSize] = React.useState(10)
  const [selected, setSelected] = React.useState<OidcClientRecord | null>(null)
  const records = query.data?.records ?? []
  const current = Math.min(page, Math.max(1, Math.ceil(records.length / size)))
  React.useEffect(() => { if (page !== current) setPage(current) }, [current, page])
  return <div className="space-y-4">
    <div className="flex items-start justify-between gap-4 rounded-lg border bg-muted/30 p-4">
      <div className="space-y-1 text-sm">
        <p className="font-medium">管理平台和客户门户的登录客户端在这里登记</p>
        <p className="text-muted-foreground">列表实时读取认证中心的 OIDC 登记。客户端标识与应用部署配置关联，用于统一登录和账号授权。</p>
      </div>
      <Button variant="outline" size="sm" disabled={query.isFetching} onClick={() => void query.refetch()} aria-label="刷新统一登录客户端"><RefreshCw className="size-4" />刷新</Button>
    </div>
    <div className="rounded-xl border bg-card">
      {query.isLoading ? <TableSkeleton cols={6} /> : query.isError ? <ErrorState message={query.error instanceof Error ? query.error.message : undefined} onRetry={() => void query.refetch()} /> : records.length === 0 ? <EmptyState title="暂无统一登录客户端" description="认证中心登记的管理平台、客户门户等登录客户端将在这里显示" /> :
        <div className="overflow-x-auto"><Table>
          <TableHeader><TableRow><TableHead className="pl-5">客户端</TableHead><TableHead>登录用途</TableHead><TableHead>允许身份</TableHead><TableHead>回调地址</TableHead><TableHead>状态</TableHead><TableHead className="pr-5 text-right">操作</TableHead></TableRow></TableHeader>
          <TableBody>{records.slice((current - 1) * size, current * size).map(client => <TableRow key={client.id}>
            <TableCell className="pl-5"><p className="font-medium">{client.name}</p><div className="mt-1 flex items-center gap-1"><code className="text-xs text-muted-foreground">{client.clientId}</code><CopyButton text={client.clientId} size="icon" className="size-7 border-0 bg-transparent" /></div></TableCell>
            <TableCell>{PURPOSES[client.purpose]}</TableCell>
            <TableCell><div className="flex gap-1">{client.allowedMemberTypes.map(type => <Badge key={type} variant="outline">{type === "EMPLOYEE" ? "员工" : "客户"}</Badge>)}</div></TableCell>
            <TableCell><div className="max-w-72 space-y-1 text-xs">{client.redirectUris.map(uri => <p key={uri} className="break-all font-mono">{uri}</p>)}</div></TableCell>
            <TableCell><StatusBadge status={client.status} /></TableCell>
            <TableCell className="pr-5 text-right"><Button variant="ghost" size="sm" onClick={() => setSelected(client)} aria-label={`查看 ${client.name} 的登录客户端`}>查看配置</Button></TableCell>
          </TableRow>)}</TableBody>
        </Table></div>}
    </div>
    {query.isSuccess && <DataPagination page={current} size={size} total={records.length} onPageChange={setPage} onSizeChange={value => { setSize(value); setPage(1) }} />}
    <Dialog open={!!selected} onOpenChange={open => { if (!open) setSelected(null) }}><DialogContent className="sm:max-w-xl">
      <DialogHeader className="pr-6"><DialogTitle className="flex items-center gap-2"><KeyRound className="size-5" />{selected?.name}</DialogTitle><DialogDescription>统一登录客户端的当前登记配置</DialogDescription></DialogHeader>
      {selected && <dl className="space-y-4 text-sm">
        <div><dt className="text-muted-foreground">客户端标识</dt><dd className="mt-1 break-all font-mono">{selected.clientId}</dd></div>
        <div><dt className="text-muted-foreground">用途</dt><dd>{PURPOSES[selected.purpose]}</dd></div>
        <div><dt className="text-muted-foreground">回调地址</dt><dd className="mt-1 space-y-2">{selected.redirectUris.map(uri => <p key={uri} className="break-all font-mono">{uri}</p>)}</dd></div>
        <div><dt className="text-muted-foreground">退出后返回地址</dt><dd className="mt-1 space-y-2">{selected.postLogoutRedirectUris.length ? selected.postLogoutRedirectUris.map(uri => <p key={uri} className="break-all font-mono">{uri}</p>) : "未登记"}</dd></div>
        <div><dt className="text-muted-foreground">保持应用登录</dt><dd>{selected.refreshEnabled ? "允许" : "不允许"}</dd></div>
        <div><dt className="text-muted-foreground">策略版本</dt><dd>{selected.accessPolicyVersion}</dd></div>
      </dl>}
    </DialogContent></Dialog>
  </div>
}
