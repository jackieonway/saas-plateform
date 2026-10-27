import * as React from "react"
import { Link } from "react-router-dom"
import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query"
import { ArrowLeftRight, Database, Loader2, Plus, RefreshCw, Shield, Trash2, UserCheck, UserX } from "lucide-react"
import { membershipsApi } from "@/api/memberships"
import type { MembershipMemberType, MembershipRecord, MembershipStatus } from "@/api/memberships"
import type { OrgNode, TenantRecord } from "@/api/types"
import { tenantsApi } from "@/api/tenants"
import { organizationsApi } from "@/api/organizations"
import { asArray, asPage } from "@/lib/api"
import { formatDateTime } from "@/lib/format"
import { useAuthStore } from "@/stores/auth"
import { useAuthorizedPaths } from "@/hooks/use-authorized-paths"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { EntityTreePicker, flattenPicker, type PickerNode } from "@/components/EntityTreePicker"
import { DataPagination } from "@/components/DataPagination"
import { CopyButton } from "@/components/CopyButton"
import { EmptyState, ErrorState, TableSkeleton } from "@/components/states"
import { MembershipAccountPicker, type SelectedMembershipAccount } from "./MembershipAccountPicker"
import { MembershipActionDialog, type MembershipAction } from "./MembershipActionDialog"
import { currentEmployeeMutationBlockedReason, membershipWriteBlockedReason, oppositeMemberType } from "./membership-model"

export function MembershipDirectory({ memberType }: { memberType: MembershipMemberType }) {
  const queryClient = useQueryClient()
  const namespace = useAuthStore(state => state.contextKey)
  const currentMembershipId = useAuthStore(state => state.identity?.membershipId)
  const access = useAuthorizedPaths()
  const canReadGrants = access.access("/system/data-grants").status === "allowed"
  const [page, setPage] = React.useState(1)
  const [size, setSize] = React.useState(10)
  const [orgFilter, setOrgFilter] = React.useState<string | null>(null)
  const [accountFilter, setAccountFilter] = React.useState<SelectedMembershipAccount | null>(null)
  const [showAccountFilter, setShowAccountFilter] = React.useState(false)
  const [statusFilter, setStatusFilter] = React.useState<MembershipStatus | "ALL">("ALL")
  const [action, setAction] = React.useState<MembershipAction | null>(null)
  const capabilitiesQuery = useQuery({
    queryKey: ["membership-capabilities", namespace],
    queryFn: ({ signal }) => membershipsApi.capabilities(signal),
    enabled: !!namespace,
    retry: 1,
    staleTime: 0,
  })
  const tenantsQuery = useQuery({
    queryKey: ["membership-tenants", namespace],
    queryFn: async () => asPage<TenantRecord>(await tenantsApi.options()).records,
    enabled: !!namespace,
    staleTime: 60_000,
    retry: 1,
  })
  const tenants = tenantsQuery.data ?? []
  const trees = useQueries({ queries: tenants.map(tenant => ({
    queryKey: ["membership-org-tree", namespace, tenant.id],
    queryFn: async () => asArray<OrgNode>(await organizationsApi.tree(tenant.id)),
    enabled: !!namespace,
    staleTime: 60_000,
    retry: 1,
  })) })
  const organizations: PickerNode[] = trees.flatMap((query, index) => (query.data ?? []).map(org => ({ ...org, name: `${tenants[index]?.name ?? ""} / ${org.name}` })))
  const orgNames = new Map(flattenPicker(organizations).map(org => [org.id, org.name]))
  const query = useQuery({
    queryKey: ["memberships", namespace, memberType, page, size, orgFilter, accountFilter?.id, statusFilter],
    queryFn: ({ signal }) => membershipsApi.list({ memberType, page, size, organizationId: orgFilter ?? undefined, userId: accountFilter?.id, status: statusFilter === "ALL" ? undefined : statusFilter }, signal),
    enabled: !!namespace,
    retry: 1,
  })
  React.useEffect(() => { setPage(1) }, [memberType, orgFilter, accountFilter?.id, statusFilter, size])
  React.useEffect(() => {
    if (query.isSuccess && query.data && query.data.total > 0 && query.data.records.length === 0 && page > 1) setPage(Math.ceil(query.data.total / size))
  }, [query.isSuccess, query.data, page, size])
  const capabilities = capabilitiesQuery.isSuccess && !capabilitiesQuery.isFetching ? capabilitiesQuery.data : undefined
  const createGate = membershipWriteBlockedReason(capabilities, "create", memberType)
  const records = query.data?.records ?? []
  const invalidate = () => {
    for (const name of ["memberships", "membership-detail", "membership-counterpart", "users", "user-memberships", "user-roles", "profile", "permissions", "auth-menus"]) void queryClient.invalidateQueries({ queryKey: [name] })
  }
  const openStatus = (member: MembershipRecord) => setAction({ kind: "status", source: member, nextStatus: member.status === "ENABLED" ? "DISABLED" : "ENABLED" })

  return <div className="space-y-4">
    <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
      <div className="space-y-1">
        <h2 className="text-base font-semibold">{memberType === "EMPLOYEE" ? "员工成员" : "客户成员"}<span className="ml-2 text-sm font-normal tabular-nums text-muted-foreground">{query.data?.total ?? 0} 条关系</span></h2>
        <p className="text-sm text-muted-foreground">{memberType === "EMPLOYEE" ? "按具体员工成员维护组织资格；角色与岗位只属于员工关系。" : "客户与员工分别展示和启停。客户成员不会获得员工角色、部门或岗位。"}</p>
      </div>
      <Button disabled={!!createGate} onClick={() => setAction({ kind: "create", memberType })}><Plus className="size-4" />新增{memberType === "EMPLOYEE" ? "员工" : "客户"}关系</Button>
    </div>
    <div className="flex flex-col gap-2 rounded-lg border bg-muted/20 px-3 py-2.5 text-xs text-muted-foreground sm:flex-row sm:items-center sm:justify-between">
      <span>{createGate || "成员创建与生命周期写入已由服务器授权开放。"}</span>
      <span className="shrink-0">客户管理展示不改变旧控制台仅员工登录的准入规则。</span>
    </div>
    {capabilitiesQuery.isError && <ErrorState message="无法读取成员写入能力，所有生命周期操作暂不可用" onRetry={() => capabilitiesQuery.refetch()} />}
    <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
      <EntityTreePicker nodes={organizations} value={orgFilter} onSelect={setOrgFilter} placeholder="按组织筛选" className="sm:w-64" ariaLabel="成员组织筛选" />
      <Select value={statusFilter} onValueChange={value => setStatusFilter(value as MembershipStatus | "ALL")}><SelectTrigger aria-label="成员状态筛选" className="sm:w-36"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="ALL">全部状态</SelectItem><SelectItem value="ENABLED">已启用</SelectItem><SelectItem value="DISABLED">已停用</SelectItem></SelectContent></Select>
      <Button variant="outline" onClick={() => setShowAccountFilter(value => !value)}><Shield className="size-4" />{accountFilter ? "已筛选精确账号" : "按既有账号筛选"}</Button>
      <Button variant="ghost" size="icon" className="sm:ml-auto" aria-label="刷新成员与写入能力" onClick={() => { void query.refetch(); void capabilitiesQuery.refetch() }} disabled={query.isFetching || capabilitiesQuery.isFetching}>{query.isFetching ? <Loader2 className="size-4 animate-spin" /> : <RefreshCw className="size-4" />}</Button>
    </div>
    {showAccountFilter && <div className="rounded-lg border p-3"><MembershipAccountPicker namespace={namespace} value={accountFilter} onChange={setAccountFilter} /></div>}
    {tenantsQuery.isError || trees.some(tree => tree.isError) ? <p role="status" className="text-xs text-muted-foreground">部分组织名称暂时无法读取；列表仍显示服务器返回的精确组织 ID。</p> : null}
    <div className="overflow-hidden rounded-xl border bg-card">
      {query.isLoading ? <div className="p-2"><TableSkeleton cols={memberType === "EMPLOYEE" ? 6 : 5} /></div> : query.isError ? <ErrorState className="m-2 border-0 bg-transparent" message={query.error instanceof Error ? query.error.message : undefined} onRetry={() => query.refetch()} /> : records.length === 0 ? <EmptyState title={`暂无${memberType === "EMPLOYEE" ? "员工" : "客户"}成员`} description="当前筛选范围内没有对应类型的成员关系" /> : <div className="overflow-x-auto"><Table className="min-w-[760px]">
        <TableHeader><TableRow><TableHead className="pl-4">账号 / 具体成员</TableHead><TableHead>组织</TableHead><TableHead>身份与状态</TableHead>{memberType === "EMPLOYEE" && <TableHead>岗位</TableHead>}<TableHead>更新时间</TableHead><TableHead className="pr-4 text-right">成员操作</TableHead></TableRow></TableHeader>
        <TableBody>{records.map(member => {
          const self = currentEmployeeMutationBlockedReason(member, currentMembershipId)
          const stateGate = membershipWriteBlockedReason(capabilities, "status") || self
          const replaceGate = membershipWriteBlockedReason(capabilities, "replaceIdentity", oppositeMemberType(member.memberType)) || self
          const deleteGate = membershipWriteBlockedReason(capabilities, "delete") || self
          return <TableRow key={member.id} data-membership-id={member.id} data-member-type={member.memberType} data-identity-version={member.identityVersion}>
            <TableCell className="pl-4"><div className="max-w-[19rem] space-y-1"><p className="flex items-center gap-1.5 font-mono text-xs" title={member.userId}>{accountFilter?.id === member.userId ? accountFilter.realName || accountFilter.username : `${member.userId.slice(0, 8)}…${member.userId.slice(-4)}`}<CopyButton text={member.userId} size="icon" className="size-6 shrink-0 border-0 bg-transparent" /></p><p className="break-all font-mono text-[11px] text-muted-foreground" title="具体成员 ID">{member.id}</p></div></TableCell>
            <TableCell><p className="max-w-48 truncate text-sm" title={member.organizationId}>{orgNames.get(member.organizationId) ?? member.organizationId}</p></TableCell>
            <TableCell><div className="flex flex-wrap items-center gap-1.5"><Badge variant={member.memberType === "EMPLOYEE" ? "secondary" : "outline"}>{member.memberType === "EMPLOYEE" ? "员工" : "客户"}</Badge><span className={member.status === "ENABLED" ? "text-xs text-emerald-700 dark:text-emerald-400" : "text-xs text-muted-foreground"}>{member.status === "ENABLED" ? "已启用" : "已停用"}</span><span className="font-mono text-xs text-muted-foreground">v{member.identityVersion}</span></div></TableCell>
            {memberType === "EMPLOYEE" && <TableCell className="max-w-32 truncate text-xs text-muted-foreground">{member.position || "—"}</TableCell>}
            <TableCell className="whitespace-nowrap text-xs text-muted-foreground">{formatDateTime(member.updatedAt)}</TableCell>
            <TableCell className="pr-4"><div className="flex flex-wrap justify-end gap-1">
              {member.memberType === "EMPLOYEE" && member.status === "ENABLED" && canReadGrants && <Button asChild variant="ghost" size="sm"><Link to={`/system/data-grants?userId=${encodeURIComponent(member.userId)}&organizationId=${encodeURIComponent(member.organizationId)}&membershipId=${encodeURIComponent(member.id)}`}><Database className="size-3.5" />员工授权</Link></Button>}
              <Button variant="ghost" size="sm" disabled={!!stateGate} title={stateGate ?? undefined} onClick={() => openStatus(member)}>{member.status === "ENABLED" ? <UserX className="size-3.5" /> : <UserCheck className="size-3.5" />}{member.status === "ENABLED" ? "停用" : "启用"}</Button>
              <Button variant="ghost" size="sm" disabled={!!replaceGate} title={replaceGate ?? undefined} onClick={() => setAction({ kind: "replaceIdentity", source: member })}><ArrowLeftRight className="size-3.5" />替换</Button>
              <Button variant="ghost" size="sm" className="text-destructive hover:text-destructive" disabled={!!deleteGate} title={deleteGate ?? undefined} onClick={() => setAction({ kind: "delete", source: member })}><Trash2 className="size-3.5" />删除关系</Button>
            </div></TableCell>
          </TableRow>
        })}</TableBody>
      </Table></div>}
      {!!query.data?.total && <div className="border-t px-4 py-3"><DataPagination page={page} size={size} total={query.data.total} onPageChange={setPage} onSizeChange={setSize} /></div>}
    </div>
    {action && <MembershipActionDialog key={`${namespace}:${action.kind}:${action.kind === "create" ? action.memberType : action.source.id}`} action={action} namespace={namespace} capabilities={capabilities} organizations={organizations} onClose={() => setAction(null)} onSaved={invalidate} />}
  </div>
}
