import * as React from "react"
import { Link } from "react-router-dom"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { CheckCircle2, Circle, Info, Loader2, ShieldCheck } from "lucide-react"
import { toast } from "sonner"
import { applicationSetupApi, type ApplicationAccessPayload, type ApplicationAccessTarget, type ApplicationOrganizationOption, type ApplicationSetupStatus } from "@/api/application-setup"
import { tenantsApi } from "@/api/tenants"
import { rolesApi } from "@/api/roles"
import type { ApplicationEnvironment, ApplicationRecord, AuthorizationMenu, RoleRecord, TenantRecord } from "@/api/types"
import { asArray, asPage } from "@/lib/api"
import { buildTree } from "@/lib/tree"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { EntityTreePicker, flattenPicker } from "@/components/EntityTreePicker"
import { TreeCheck, fromAuthMenus, type TreeCheckNode } from "@/components/TreeCheck"
import { EmptyState, ErrorState } from "@/components/states"

type Props = {
  application: ApplicationRecord
  environment: ApplicationEnvironment
  canConfigureAccess: boolean
  /** Validated qiankun entry readiness from the parent setup checks. */
  entryReady?: boolean
  initialTarget?: ApplicationAccessTarget | null
  onTargetChange?: (target: ApplicationAccessTarget | null) => void
  onConfigured?: () => void
  onBusyChange?: (busy: boolean) => void
}

type SetupMenu = AuthorizationMenu & { visible?: boolean; linkUrl?: string }
const EMPTY_MENUS: SetupMenu[] = []
type Selection = { key: string; ids: Set<string> }
type GrantRequest = { applicationId: string; environment: ApplicationEnvironment; payload: ApplicationAccessPayload }

/** Candidate pagination must finish before a target can be submitted. */
async function loadRoleCandidates(tenantId: string, organizationId: string): Promise<RoleRecord[]> {
  const collected = new Map<string, RoleRecord>()
  for (let page = 1; ; page++) {
    const result = asPage<RoleRecord>(await rolesApi.list({ tenantId, organizationId, page, size: 200 }))
    const before = collected.size
    for (const item of result.records) collected.set(item.id, item)
    if (collected.size >= result.total) break
    if (collected.size === before) throw new Error("角色列表未完整返回，请重试")
  }
  return [...collected.values()].filter(role => role.tenantId === tenantId && role.organizationId === organizationId
    && role.status === "ENABLED" && !(role.builtIn && ["tenant_admin", "org_admin"].includes(role.code)))
}

function enabledOrganizations(nodes: ApplicationOrganizationOption[]): ApplicationOrganizationOption[] {
  return nodes.filter(node => node.status === "ENABLED" && (!node.expiresAt || Date.parse(node.expiresAt) > Date.now()))
    .map(node => ({ ...node, children: enabledOrganizations(node.children ?? []) }))
}

function matchingStatus(status: ApplicationSetupStatus | undefined, applicationId: string, environment: ApplicationEnvironment, target: ApplicationAccessTarget): status is ApplicationSetupStatus {
  return status?.applicationId === applicationId && status.environment === environment
    && status.access?.tenantId === target.tenantId && status.access.organizationId === target.organizationId
    && status.access.roleId === target.roleId
}

function toMenuTree(flat: SetupMenu[]): TreeCheckNode[] {
  const convert = (nodes: ReturnType<typeof buildTree<TreeCheckNode>>): TreeCheckNode[] =>
    nodes.map(({ node, children }) => ({ ...node, children: convert(children) }))
  return convert(buildTree(fromAuthMenus(flat)))
}

export function ApplicationAccessSetup(props: Props) {
  return <ApplicationAccessSetupContent key={`${props.application.id}:${props.environment}`} {...props} />
}

function ApplicationAccessSetupContent({ application, environment, canConfigureAccess, entryReady, initialTarget, onTargetChange, onConfigured, onBusyChange }: Props) {
  const queryClient = useQueryClient()
  const [tenantId, setTenantId] = React.useState(initialTarget?.tenantId ?? "")
  const [organizationId, setOrganizationId] = React.useState(initialTarget?.organizationId ?? "")
  const [roleId, setRoleId] = React.useState(initialTarget?.roleId ?? "")
  const [selection, setSelection] = React.useState<Selection | null>(null)
  const [saved, setSaved] = React.useState(false)
  const [submitError, setSubmitError] = React.useState<string | null>(null)
  const mounted = React.useRef(true)
  const currentTarget = React.useRef({ tenantId, organizationId, roleId })
  currentTarget.current = { tenantId, organizationId, roleId }
  React.useEffect(() => { mounted.current = true; return () => { mounted.current = false } }, [])

  const tenantsQuery = useQuery({
    queryKey: ["application-access-tenants"],
    queryFn: async () => asPage<TenantRecord>(await tenantsApi.options()).records
      .filter(item => item.status === "ENABLED" && (!item.expiresAt || Date.parse(item.expiresAt) > Date.now())),
    enabled: canConfigureAccess,
    retry: 1,
  })
  const tenants = tenantsQuery.data ?? []
  const selectedTenant = tenants.find(item => item.id === tenantId)
  const orgsQuery = useQuery({
    queryKey: ["application-access-organizations", application.id, tenantId],
    queryFn: async () => enabledOrganizations(asArray<ApplicationOrganizationOption>(await applicationSetupApi.organizations(application.id, tenantId))),
    enabled: canConfigureAccess && !!selectedTenant,
    retry: 1,
  })
  const organizations = orgsQuery.data ?? []
  const selectedOrganization = flattenPicker(organizations).find(item => item.id === organizationId && item.tenantId === tenantId)
  const rolesQuery = useQuery({
    queryKey: ["application-access-roles", tenantId, organizationId],
    queryFn: () => loadRoleCandidates(tenantId, organizationId),
    enabled: canConfigureAccess && !!selectedOrganization,
    retry: 1,
  })
  const roles = rolesQuery.data ?? []
  const selectedRole = roles.find(item => item.id === roleId && item.organizationId === organizationId && item.tenantId === tenantId
    && item.status === "ENABLED" && !(item.builtIn && ["tenant_admin", "org_admin"].includes(item.code)))
  const menusQuery = useQuery({
    queryKey: ["application-access-menus", application.id, tenantId],
    queryFn: async () => {
      const response = await tenantsApi.authorizations(tenantId)
      if (!response.applications.some(item => item.id === application.id) || !Array.isArray(response.menus[application.id])) {
        throw new Error("当前应用的菜单信息不完整，请刷新后重试")
      }
      return response.menus[application.id] as SetupMenu[]
    },
    enabled: canConfigureAccess && !!selectedTenant,
    retry: 1,
  })
  const menus = menusQuery.data ?? EMPTY_MENUS
  const selectionKey = `${application.id}:${tenantId}`
  React.useEffect(() => {
    if (!canConfigureAccess || !menusQuery.isSuccess || selection?.key === selectionKey) return
    setSelection({ key: selectionKey, ids: new Set(menus.filter(item => item.type === "MENU" && item.visible !== false
      && !item.linkUrl && item.path?.replace(/\/+$/, "") === `/micro/${application.code}`).map(item => item.id)) })
  }, [application.code, canConfigureAccess, menus, menusQuery.isSuccess, selection?.key, selectionKey])
  const checked = selection?.key === selectionKey ? selection.ids : new Set<string>()
  const target = { tenantId, organizationId, roleId }
  const targetValid = !!selectedTenant && !!selectedOrganization && !!selectedRole
  const targetCandidatesReady = canConfigureAccess && tenantsQuery.isSuccess && orgsQuery.isSuccess && rolesQuery.isSuccess && targetValid
  React.useEffect(() => {
    // Restored selections survive pending reads. Only a confirmed candidate set
    // becomes the parent's readiness target; manual changes retire it below.
    if (targetCandidatesReady) onTargetChange?.({ tenantId, organizationId, roleId })
  }, [targetCandidatesReady, tenantId, organizationId, roleId, onTargetChange])
  const statusKey = ["application-setup-target", application.id, environment, tenantId, organizationId, roleId]
  const statusQuery = useQuery({
    queryKey: statusKey,
    queryFn: async () => {
      const status = await applicationSetupApi.status(application.id, environment, target)
      if (!matchingStatus(status, application.id, environment, target)) throw new Error("访问范围检查返回了其他配置，请重试")
      return status
    },
    enabled: canConfigureAccess && targetValid,
    retry: 1,
  })
  const status = statusQuery.isSuccess && matchingStatus(statusQuery.data, application.id, environment, target) ? statusQuery.data : null
  const readsReady = canConfigureAccess && tenantsQuery.isSuccess && orgsQuery.isSuccess && rolesQuery.isSuccess && menusQuery.isSuccess
    && selection?.key === selectionKey && targetValid
  const validMenuIds = new Set(menus.map(item => item.id))
  const selectedIds = [...checked].filter(id => validMenuIds.has(id))
  const canSubmit = readsReady && selectedIds.length > 0 && selectedIds.length <= 500 && selectedIds.length === checked.size
  const sameRequest = (request: GrantRequest) => mounted.current && canConfigureAccess && request.applicationId === application.id && request.environment === environment
    && Object.entries(currentTarget.current).every(([key, value]) => request.payload[key as keyof ApplicationAccessTarget] === value)
  const saveMutation = useMutation({
    mutationFn: async (request: GrantRequest) => {
      if (!sameRequest(request) || !canSubmit || request.payload.menuIds.length !== selectedIds.length
        || new Set(request.payload.menuIds).size !== request.payload.menuIds.length
        || request.payload.menuIds.some(id => !validMenuIds.has(id) || !checked.has(id))) throw new Error("请先完整选择并确认当前访问范围")
      await applicationSetupApi.grantAccess(request.applicationId, request.payload)
      const fresh = await applicationSetupApi.status(request.applicationId, request.environment, {
        tenantId: request.payload.tenantId, organizationId: request.payload.organizationId, roleId: request.payload.roleId,
      })
      if (!matchingStatus(fresh, request.applicationId, request.environment, request.payload)) throw new Error("授权已提交，但状态未核对成功，请刷新确认")
      return fresh
    },
    onSuccess: (fresh, request) => {
      queryClient.invalidateQueries({ queryKey: ["application-setup", request.applicationId] })
      queryClient.invalidateQueries({ queryKey: ["applications"] })
      queryClient.invalidateQueries({ queryKey: ["profile"] })
      queryClient.invalidateQueries({ queryKey: ["permissions"] })
      if (!sameRequest(request)) return
      queryClient.setQueryData(statusKey, fresh)
      setSaved(true)
      setSubmitError(null)
      toast.success("访问授权已保存")
      onConfigured?.()
    },
    onError: (error: Error, request) => {
      if (!sameRequest(request) || error.name === "AbortError") return
      setSubmitError(error.message || "授权结果尚未确认，请刷新当前访问范围")
      void statusQuery.refetch()
    },
  })
  const busy = saveMutation.isPending
  React.useEffect(() => { onBusyChange?.(busy); return () => { onBusyChange?.(false) } }, [busy, onBusyChange])
  const changeTenant = (next: string) => {
    if (busy) return
    onTargetChange?.(null)
    currentTarget.current = { tenantId: next, organizationId: "", roleId: "" }
    setTenantId(next); setOrganizationId(""); setRoleId(""); setSelection(null); setSaved(false); setSubmitError(null)
  }
  const changeOrganization = (next: string | null) => {
    if (busy) return
    onTargetChange?.(null)
    currentTarget.current = { tenantId, organizationId: next ?? "", roleId: "" }
    setOrganizationId(next ?? ""); setRoleId(""); setSaved(false); setSubmitError(null)
  }
  const changeRole = (next: string) => {
    if (busy) return
    onTargetChange?.(null)
    currentTarget.current = { tenantId, organizationId, roleId: next }
    setRoleId(next); setSaved(false); setSubmitError(null)
  }

  if (!canConfigureAccess) return (
    <Alert><ShieldCheck className="size-4" /><AlertTitle>请由授权管理员完成这一步</AlertTitle><AlertDescription>
      配置访问范围需要租户、组织授权和角色管理权限。应用与版本已经保留，管理员可稍后继续接入。
    </AlertDescription></Alert>
  )

  const queryError = [tenantsQuery, orgsQuery, rolesQuery, menusQuery].find(query => query.isError)
  const accessComplete = status?.access?.tenantAuthorized && status.access.organizationAuthorized && status.access.roleAuthorized
  const qiankunReady = status?.entryType === "HTML" && status.runtimeMode === "TRUSTED"
  const ready = !!(qiankunReady && entryReady && accessComplete && status?.applicationEnabled && status.releaseValidated && status.entryMenuCount > 0 && status.access!.assignedUserCount > 0)
  return (
    <div className="space-y-5">
      <div><h3 className="text-base font-semibold">选择谁可以访问</h3><p className="mt-1 text-sm leading-relaxed text-muted-foreground">选择租户、组织和角色，再确认需要开放的菜单。已有授权会保留。</p></div>
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2"><Label>租户</Label><Select value={tenantId} onValueChange={changeTenant} disabled={busy || !tenantsQuery.isSuccess}>
          <SelectTrigger className="w-full" aria-label="访问租户"><SelectValue placeholder={tenantsQuery.isLoading ? "正在加载租户…" : "选择租户"} /></SelectTrigger>
          <SelectContent>{tenants.map(item => <SelectItem key={item.id} value={item.id}>{item.name}</SelectItem>)}</SelectContent>
        </Select></div>
        <div className="space-y-2"><Label>组织</Label><EntityTreePicker ariaLabel="访问组织" nodes={organizations} value={organizationId} onSelect={changeOrganization}
          placeholder={orgsQuery.isLoading && selectedTenant ? "正在加载组织…" : "选择组织"} disabled={busy || !selectedTenant || !orgsQuery.isSuccess} /></div>
        <div className="space-y-2 sm:col-span-2"><Label>角色</Label><Select value={roleId} onValueChange={changeRole} disabled={busy || !selectedOrganization || !rolesQuery.isSuccess}>
          <SelectTrigger className="w-full" aria-label="访问角色"><SelectValue placeholder={rolesQuery.isLoading && selectedOrganization ? "正在加载角色…" : "选择角色"} /></SelectTrigger>
          <SelectContent>{roles.map(item => <SelectItem key={item.id} value={item.id}>{item.name}</SelectItem>)}</SelectContent>
        </Select>
          {selectedOrganization && rolesQuery.isSuccess && roles.length === 0 && <p className="text-xs leading-relaxed text-muted-foreground">该组织暂无可配置的普通角色。<Link className="font-medium text-primary hover:underline" to={`/system/roles?tenantId=${encodeURIComponent(tenantId)}&organizationId=${encodeURIComponent(organizationId)}`}>前往创建角色</Link>后可返回继续。租户、组织管理员角色由平台自动维护。</p>}
        </div>
      </div>

      {queryError && <ErrorState message={queryError.error instanceof Error ? queryError.error.message : undefined} onRetry={() => { void queryError.refetch() }} className="py-6" />}
      {selectedTenant && menusQuery.isSuccess && <section className="rounded-lg border" aria-label="开放菜单">
        <div className="flex items-center justify-between gap-3 border-b px-4 py-3"><div><h4 className="text-sm font-medium">开放菜单</h4><p className="mt-1 text-xs text-muted-foreground">默认仅选择首页，可按需增加页面与按钮。</p></div><span className="shrink-0 text-xs tabular-nums text-muted-foreground">已选 {checked.size} 项</span></div>
        <div className="max-h-72 overflow-y-auto p-3">{menus.length ? <TreeCheck nodes={toMenuTree(menus)} checked={checked} disabled={busy} onToggle={(ids, enabled) => {
          if (busy) return
          setSelection(previous => {
            const next = new Set(previous?.key === selectionKey ? previous.ids : [])
            for (const id of ids) if (validMenuIds.has(id)) { if (enabled) next.add(id); else next.delete(id) }
            return { key: selectionKey, ids: next }
          })
          setSaved(false)
        }} /> : <EmptyState title="当前应用还没有菜单" description="先配置首页或业务页面，再回来选择访问范围。" action={<Button variant="outline" size="sm" asChild><Link to={`/system/menus?applicationId=${encodeURIComponent(application.id)}`}>配置菜单</Link></Button>} className="py-6" />}</div>
      </section>}

      {targetValid && <div className="rounded-lg bg-muted/40 px-4 py-3 text-sm" aria-label="授权确认范围">
        <p className="font-medium">{selectedTenant!.name} / {selectedOrganization!.name} / {selectedRole!.name}</p>
        <p className="mt-1 text-xs leading-relaxed text-muted-foreground">将选中的 {selectedIds.length} 项菜单添加到租户、组织授权上限及该角色。已有租户与组织管理员的托管权限会同步更新。</p>
      </div>}
      {submitError && <Alert variant="destructive"><Info className="size-4" /><AlertTitle>请核对授权结果</AlertTitle><AlertDescription>{submitError}</AlertDescription></Alert>}
      <div className="flex flex-wrap items-center justify-between gap-3"><p className="max-w-md text-xs leading-relaxed text-muted-foreground">这里配置应用和菜单访问。用户还需在所选组织拥有该角色。</p>
        <Button disabled={busy || !canSubmit} onClick={() => { if (!busy && canSubmit) saveMutation.mutate({ applicationId: application.id, environment, payload: { ...target, menuIds: selectedIds } }) }}>
          {busy ? <Loader2 className="size-4 animate-spin" /> : <ShieldCheck className="size-4" />}确认授予访问
        </Button>
      </div>

      {statusQuery.isError && <ErrorState message={statusQuery.error instanceof Error ? statusQuery.error.message : undefined} onRetry={() => { void statusQuery.refetch() }} className="py-6" />}
      {status && <section className="space-y-3 rounded-lg border p-4" aria-label="当前范围授权状态" aria-live="polite">
        <div className="flex items-center gap-2 text-sm font-medium">{ready ? <CheckCircle2 className="size-4 text-primary" /> : <Circle className="size-4 text-muted-foreground" />}{ready ? "当前范围已具备访问条件" : saved ? "授权已保存，继续检查访问条件" : "当前范围检查"}</div>
        <div className="grid gap-2 text-xs text-muted-foreground sm:grid-cols-3"><span>租户授权：{status.access!.tenantAuthorized ? "已配置" : "待配置"}</span><span>组织授权：{status.access!.organizationAuthorized ? "已配置" : "待配置"}</span><span>角色入口：{status.access!.roleAuthorized ? "已配置" : "待配置"}</span></div>
        {!status.applicationEnabled && <p className="text-xs text-muted-foreground">应用当前已停用，需要先启用应用。</p>}
        {!qiankunReady && <p className="text-xs text-muted-foreground">此应用需重新配置 qiankun 入口，当前无法运行。</p>}
        {qiankunReady && !status.releaseValidated && <p className="text-xs text-muted-foreground">当前环境尚无可用入口，请保存直接地址或发布版本。</p>}
        {qiankunReady && status.releaseValidated && !entryReady && <p className="text-xs text-muted-foreground">入口检查尚未完成，请返回应用入口步骤核对。</p>}
        {status.entryMenuCount === 0 && <p className="text-xs text-muted-foreground">还需要配置可执行的页面入口菜单。</p>}
        <p className="text-xs text-muted-foreground">该组织中拥有此角色的有效成员：{status.access!.assignedUserCount} 人。{status.access!.assignedUserCount === 0 && <Link className="text-primary hover:underline" to={`/system/roles?tenantId=${encodeURIComponent(tenantId)}&organizationId=${encodeURIComponent(organizationId)}`}>前往角色管理分配成员</Link>}</p>
      </section>}
      <details className="rounded-lg border px-4 py-3"><summary className="cursor-pointer text-sm font-medium">应用还需要调用业务 API？</summary><p className="mt-3 text-xs leading-relaxed text-muted-foreground">业务服务还需完成机密客户端绑定、API 地址和资源动作的数据授权。这里保存的菜单权限不代表已取得业务数据权限，请按业务接入文档继续配置。</p></details>
    </div>
  )
}
