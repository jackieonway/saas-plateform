import * as React from "react"
import { Link, useSearchParams } from "react-router-dom"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { Building2, Info, Loader2, Save, ShieldCheck } from "lucide-react"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { Badge } from "@/components/ui/badge"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { ScrollArea } from "@/components/ui/scroll-area"
import { PageHeader } from "@/components/layout/PageHeader"
import { EntityTreePicker, type PickerNode } from "@/components/EntityTreePicker"
import { TreeCheck, TreeCheckToolbar, type TreeCheckNode } from "@/components/TreeCheck"
import { EmptyState, ErrorState } from "@/components/states"
import { tenantsApi } from "@/api/tenants"
import { organizationsApi } from "@/api/organizations"
import { asArray, asPage } from "@/lib/api"
import { AppIcon } from "@/lib/icons"
import { buildTree, type TreeNode } from "@/lib/tree"
import { toast } from "sonner"
import { cn } from "@/lib/utils"
import { useAuthStore } from "@/stores/auth"
import { usePermissionsQuery } from "@/hooks/use-auth-data"
import { useAuthorizedPaths } from "@/hooks/use-authorized-paths"
import { hasPlatformContext } from "@/lib/platform-context"
import type { AuthorizationMenu, AuthorizationResp, OrgNode, TenantRecord } from "@/api/types"

export default function AuthorizationsPage() {
  const [params, setParams] = useSearchParams()
  const grantAccess = useAuthorizedPaths()
  const canReadDataGrants = grantAccess.access("/system/data-grants").status === "allowed"

  /* 「租户授权」为平台根组织职能，并且需要 system:tenant 权限。 */
  const isPlatformTenant = useAuthStore((s) => hasPlatformContext(s.profile))
  const permissionsQuery = usePermissionsQuery()
  const permissions = new Set(permissionsQuery.data?.permissions ?? [])
  const permsReady = permissionsQuery.isSuccess || permissionsQuery.isError
  const canTenantAuthz =
    isPlatformTenant && permsReady && permissions.has("system:tenant")

  const [tab, setTab] = React.useState<"tenant" | "org">(
    params.get("organizationId") ? "org" : "tenant"
  )
  /** 有效 tab: 无租户授权资格时永远回落 org(含 ?tenantId= 深链) */
  const activeTab: "tenant" | "org" = canTenantAuthz ? tab : "org"

  const tenantsQuery = useQuery({
    queryKey: ["tenants", "options"],
    queryFn: async () => asPage<TenantRecord>(await tenantsApi.options()).records,
    staleTime: 60_000,
    retry: 1,
  })
  const tenants = tenantsQuery.data ?? []

  const [tenantId, setTenantId] = React.useState<string | null>(params.get("tenantId"))
  React.useEffect(() => {
    if (!tenantId && tenants.length > 0) {
      setTenantId(params.get("tenantId") ?? tenants[0].id)
    }
    if (params.get("tenantId") && params.get("tenantId") !== tenantId && tenants.some((t) => t.id === params.get("tenantId"))) {
      setTenantId(params.get("tenantId"))
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tenants])

  const [organizationId, setOrganizationId] = React.useState<string | null>(params.get("organizationId"))

  const orgTreeQuery = useQuery({
    queryKey: ["org-tree", tenantId],
    queryFn: async () => asArray<OrgNode>(await organizationsApi.tree(tenantId!)),
    enabled: !!tenantId,
    staleTime: 30_000,
    retry: 1,
  })
  const orgTree = orgTreeQuery.data ?? []
  const organizationInScope = !!organizationId && orgTreeQuery.isSuccess && hasOrganization(orgTree, organizationId)

  // 校验预选组织属于当前租户
  React.useEffect(() => {
    if (organizationId && orgTreeQuery.isSuccess && !hasOrganization(orgTree, organizationId)) {
      setOrganizationId(null)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [orgTree, orgTreeQuery.isSuccess])

  const switchTab = (t: string) => {
    const nextTab = (canTenantAuthz ? t : "org") as "tenant" | "org"
    setTab(nextTab)
    const next = new URLSearchParams(params)
    next.delete("tenantId")
    next.delete("organizationId")
    setParams(next, { replace: true })
  }

  return (
    <div>
      <PageHeader
        title="授权管理"
        description={
          canTenantAuthz
            ? "租户授权 ⊇ 组织授权 ⊇ 角色菜单,逐层收窄的应用与菜单授权"
            : "组织授权 ⊇ 角色菜单,在租户授权范围内为组织分配应用与菜单"
        }
        icon={<ShieldCheck className="size-5" />}
        actions={canReadDataGrants ? (
          <Button asChild variant="outline">
            <Link to="/system/data-grants">管理直接成员数据范围</Link>
          </Button>
        ) : undefined}
      />

      <p className="mb-4 text-sm text-muted-foreground">
        此处配置应用、菜单和按钮功能权限；成员数据范围需独立授权，不随祖先角色继承。
      </p>

      <Tabs value={activeTab} onValueChange={switchTab}>
        {canTenantAuthz && (
          <TabsList className="mb-4">
            <TabsTrigger value="tenant">租户授权</TabsTrigger>
            <TabsTrigger value="org">组织授权</TabsTrigger>
          </TabsList>
        )}

        {canTenantAuthz && (
        <TabsContent value="tenant" className="mt-0">
          <div className="mb-4 flex flex-wrap items-center gap-3">
            <Select value={tenantId ?? ""} onValueChange={setTenantId}>
              <SelectTrigger className="w-60" aria-label="选择租户">
                <SelectValue placeholder="选择租户" />
              </SelectTrigger>
              <SelectContent>
                {tenants.map((t) => (
                  <SelectItem key={t.id} value={t.id}>
                    {t.name}({t.code})
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {tenantsQuery.isLoading && <Loader2 className="size-4 animate-spin text-muted-foreground" />}
          </div>
          {tenantId ? (
            <AuthorizationEditor
              key={`tenant-${tenantId}`}
              targetId={tenantId}
              targetType="tenant"
              fetcher={() => tenantsApi.authorizations(tenantId)}
              saver={(payload) => tenantsApi.saveAuthorizations(tenantId, payload)}
            />
          ) : (
            <div className="rounded-xl border bg-card">
              <EmptyState className="py-16" title="请选择租户" description="选择租户后编辑其应用与菜单授权" />
            </div>
          )}
        </TabsContent>
        )}

        <TabsContent value="org" className="mt-0">
          <div className="mb-4 flex flex-wrap items-center gap-3">
            <Select value={tenantId ?? ""} onValueChange={setTenantId}>
              <SelectTrigger className="w-52" aria-label="选择租户">
                <SelectValue placeholder="选择租户" />
              </SelectTrigger>
              <SelectContent>
                {tenants.map((t) => (
                  <SelectItem key={t.id} value={t.id}>
                    {t.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <EntityTreePicker
              nodes={orgTree.map((n) => ({ id: n.id, name: n.name, code: n.code, children: toPickerChildren(n.children) }))}
              value={organizationId}
              onSelect={setOrganizationId}
              placeholder="选择组织"
              className="w-60"
              allowClear={false}
              disabled={orgTreeQuery.isLoading}
            />
            {(orgTreeQuery.isLoading || tenantsQuery.isLoading) && (
              <Loader2 className="size-4 animate-spin text-muted-foreground" />
            )}
          </div>
          {organizationInScope && organizationId ? (
            <AuthorizationEditor
              key={`org-${organizationId}`}
              targetId={organizationId}
              targetType="org"
              fetcher={() => organizationsApi.authorizations(organizationId)}
              saver={(payload) => organizationsApi.saveAuthorizations(organizationId, payload)}
            />
          ) : (
            <div className="rounded-xl border bg-card">
              <EmptyState
                className="py-16"
                title="请选择组织"
                description="先选择租户,再选择其下的组织"
                icon={<Building2 className="size-6" />}
              />
            </div>
          )}
        </TabsContent>
      </Tabs>
    </div>
  )
}

function toPickerChildren(children?: OrgNode[]): PickerNode[] {
  return (children ?? []).map((c) => ({ id: c.id, name: c.name, code: c.code, children: toPickerChildren(c.children) }))
}

function hasOrganization(nodes: OrgNode[], id: string): boolean {
  return nodes.some((node) => node.id === id || (!!node.children?.length && hasOrganization(node.children, id)))
}

/* ============ 授权编辑器(租户/组织共用) ============ */

interface AuthSnapshot {
  apps: Map<string, boolean>
  menus: Map<string, boolean>
}

function AuthorizationEditor({
  targetId,
  targetType,
  fetcher,
  saver,
}: {
  targetId: string
  targetType: "tenant" | "org"
  fetcher: () => Promise<AuthorizationResp>
  saver: (payload: {
    applications: { applicationId: string; authorized: boolean }[]
    menus: { menuId: string; authorized: boolean }[]
  }) => Promise<unknown>
}) {
  const queryClient = useQueryClient()

  const query = useQuery({
    queryKey: ["authorizations", targetType, targetId],
    queryFn: fetcher,
    staleTime: 30_000,
    retry: 1,
  })
  const data = query.data

  const [selectedAppId, setSelectedAppId] = React.useState<string | null>(null)
  const [snapshot, setSnapshot] = React.useState<AuthSnapshot | null>(null)
  const [current, setCurrent] = React.useState<AuthSnapshot | null>(null)

  React.useEffect(() => {
    if (data) {
      const apps = new Map(data.applications.map((a) => [a.id, a.authorized]))
      const menus = new Map<string, boolean>()
      for (const list of Object.values(data.menus)) {
        for (const m of list) menus.set(m.id, m.authorized)
      }
      const snap = { apps, menus }
      setSnapshot(structuredCopy(snap))
      setCurrent(structuredCopy(snap))
      const firstOn = data.applications.find((a) => a.authorized) ?? data.applications[0]
      setSelectedAppId((prev) => (prev && data.applications.some((a) => a.id === prev) ? prev : firstOn?.id ?? null))
    }
  }, [data])

  // 每个应用的菜单树(由平铺构建)
  const appTrees = React.useMemo(() => {
    const map = new Map<string, TreeCheckNode[]>()
    if (!data) return map
    const convert = (nodes: TreeNode<AuthorizationMenu>[]): TreeCheckNode[] =>
      nodes.map((t) => ({
        id: t.node.id,
        parentId: t.node.parentId,
        title: t.node.title ?? t.node.id.slice(0, 8),
        type: t.node.type,
        icon: t.node.icon ?? null,
        path: t.node.path ?? null,
        sortOrder: t.node.sortOrder,
        children: convert(t.children),
      }))
    for (const app of data.applications) {
      const flat = data.menus[app.id] ?? []
      map.set(app.id, convert(buildTree(flat)))
    }
    return map
  }, [data])

  const dirtyCount = React.useMemo(() => {
    if (!snapshot || !current) return 0
    let count = 0
    for (const [id, v] of current.apps) {
      if (snapshot.apps.get(id) !== v) count++
    }
    for (const [id, v] of current.menus) {
      if (snapshot.menus.get(id) !== v) count++
    }
    return count
  }, [snapshot, current])

  const saveMutation = useMutation({
    mutationFn: () => {
      if (!current || !data) throw new Error("数据未就绪")
      const applications = data.applications.map((a) => ({
        applicationId: a.id,
        authorized: current.apps.get(a.id) ?? false,
      }))
      const menus: { menuId: string; authorized: boolean }[] = []
      for (const app of data.applications) {
        const appOn = current.apps.get(app.id) ?? false
        for (const m of data.menus[app.id] ?? []) {
          // 应用关闭时,其菜单授权一并关闭(对齐后端清理逻辑)
          menus.push({ menuId: m.id, authorized: appOn && (current.menus.get(m.id) ?? false) })
        }
      }
      return saver({ applications, menus })
    },
    onSuccess: () => {
      toast.success("授权已保存")
      queryClient.invalidateQueries({ queryKey: ["authorizations", targetType, targetId] })
      queryClient.invalidateQueries({ queryKey: ["menus-tree"] })
      queryClient.invalidateQueries({ queryKey: ["auth-menus"] })
      queryClient.invalidateQueries({ queryKey: ["tenants"] })
    },
    onError: (e: Error) => toast.error(e.message || "保存失败"),
  })

  const toggleApp = (appId: string, v: boolean) => {
    setCurrent((c) => {
      if (!c) return c
      const apps = new Map(c.apps)
      apps.set(appId, v)
      return { ...c, apps }
    })
  }

  const toggleMenus = (ids: string[], checked: boolean) => {
    setCurrent((c) => {
      if (!c) return c
      const menus = new Map(c.menus)
      for (const id of ids) menus.set(id, checked)
      return { ...c, menus }
    })
  }

  const setAllForApp = (appId: string, checked: boolean) => {
    const flat = data?.menus[appId] ?? []
    toggleMenus(
      flat.map((m) => m.id),
      checked
    )
  }

  const selectedApp = data?.applications.find((a) => a.id === selectedAppId) ?? null
  const appOn = selectedApp ? (current?.apps.get(selectedApp.id) ?? false) : false

  if (query.isLoading) {
    return (
      <div className="grid gap-4 lg:grid-cols-[320px_1fr]">
        <div className="space-y-2 rounded-xl border bg-card p-4">
          {Array.from({ length: 4 }).map((_, i) => (
            <div key={i} className="h-16 animate-pulse rounded-lg bg-muted" />
          ))}
        </div>
        <div className="space-y-2 rounded-xl border bg-card p-4">
          {Array.from({ length: 8 }).map((_, i) => (
            <div key={i} className="h-8 animate-pulse rounded-md bg-muted" style={{ marginLeft: `${(i % 3) * 20}px` }} />
          ))}
        </div>
      </div>
    )
  }

  if (query.isError || !data) {
    return (
      <ErrorState
        message={query.error instanceof Error ? query.error.message : undefined}
        onRetry={() => query.refetch()}
      />
    )
  }

  if (data.applications.length === 0) {
    return (
      <div className="rounded-xl border bg-card">
        <EmptyState
          className="py-16"
          title="暂无可授权应用"
          description={
            targetType === "org"
              ? "该组织所属租户尚未授权任何应用,请先在「租户授权」中开启"
              : "请先在「应用管理」中创建应用"
          }
        />
      </div>
    )
  }

  return (
    <div className="space-y-4">
      {targetType === "org" && (
        <Alert>
          <Info className="size-4" />
          <AlertTitle>可授权范围受租户授权约束</AlertTitle>
          <AlertDescription>
            下方仅展示该组织所属租户已授权的应用与菜单;关闭授权会级联清理该组织下角色的相应菜单。
          </AlertDescription>
        </Alert>
      )}

      <div className="grid gap-4 lg:grid-cols-[320px_1fr]">
        {/* 左栏:应用授权列表 */}
        <aside className="rounded-xl border bg-card">
          <p className="border-b px-4 py-3 text-xs font-medium uppercase tracking-wider text-muted-foreground">
            应用授权({data.applications.filter((a) => current?.apps.get(a.id)).length}/{data.applications.length})
          </p>
          <ScrollArea className="h-[26rem] lg:h-[calc(100dvh-23rem)]">
            <div className="space-y-1 p-2">
              {data.applications.map((app) => {
                const on = current?.apps.get(app.id) ?? false
                const active = app.id === selectedAppId
                const authCount = app.authorizedMenuCount
                return (
                  <div
                    key={app.id}
                    role="button"
                    tabIndex={0}
                    onClick={() => setSelectedAppId(app.id)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault()
                        setSelectedAppId(app.id)
                      }
                    }}
                    aria-current={active ? "true" : undefined}
                    className={cn(
                      "flex w-full cursor-pointer items-center gap-3 rounded-lg border p-3 text-left outline-none transition-all focus-visible:ring-2 focus-visible:ring-ring",
                      active ? "border-primary/50 bg-primary/10 ring-1 ring-primary/25" : "border-transparent hover:bg-muted/60"
                    )}
                  >
                    <span
                      className={cn(
                        "flex size-9 shrink-0 items-center justify-center rounded-lg",
                        on ? "bg-primary/15 text-primary" : "bg-muted text-muted-foreground"
                      )}
                    >
                      <AppIcon name={app.icon} className="size-5" />
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className={cn("block truncate text-sm font-medium", active && "text-primary")}>{app.name}</span>
                      <span className="block text-xs text-muted-foreground">
                        {authCount}/{app.menuCount} 菜单 · <span className="font-mono">{app.code}</span>
                      </span>
                    </span>
                    <Switch
                      checked={on}
                      onCheckedChange={(v) => toggleApp(app.id, v)}
                      onClick={(e) => e.stopPropagation()}
                      aria-label={`${on ? "关闭" : "开启"}应用 ${app.name} 授权`}
                    />
                  </div>
                )
              })}
            </div>
          </ScrollArea>
        </aside>

        {/* 右栏:菜单树 */}
        <section className="flex min-h-[26rem] flex-col rounded-xl border bg-card">
          <div className="flex flex-col gap-2 border-b px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
            <div className="flex items-center gap-2">
              {selectedApp && <AppIcon name={selectedApp.icon} className="size-4 text-primary" />}
              <p className="text-sm font-semibold">{selectedApp?.name ?? "选择应用"}</p>
              {!appOn && selectedApp && (
                <Badge variant="outline" className="text-[11px] text-muted-foreground">
                  应用未授权,菜单树已禁用
                </Badge>
              )}
            </div>
            {selectedApp && (
              <TreeCheckToolbar
                nodes={appTrees.get(selectedApp.id) ?? []}
                checked={new Set(
                  (data.menus[selectedApp.id] ?? [])
                    .filter((m) => current?.menus.get(m.id))
                    .map((m) => m.id)
                )}
                onSet={(next) => {
                  const flat = data.menus[selectedApp.id] ?? []
                  setCurrent((c) => {
                    if (!c) return c
                    const menus = new Map(c.menus)
                    for (const m of flat) menus.set(m.id, next.has(m.id))
                    return { ...c, menus }
                  })
                }}
                disabled={!appOn}
              />
            )}
          </div>
          <ScrollArea className="h-[22rem] flex-1 lg:h-[calc(100dvh-27rem)]">
            <div className="p-3">
              {!selectedApp ? (
                <EmptyState className="py-14" title="从左侧选择应用" description="勾选菜单完成细粒度授权" />
              ) : (appTrees.get(selectedApp.id) ?? []).length === 0 ? (
                <EmptyState className="py-14" title="该应用暂无菜单" description="请先在「菜单管理」中为其配置菜单" />
              ) : (
                <TreeCheck
                  nodes={appTrees.get(selectedApp.id) ?? []}
                  checked={new Set(
                    (data.menus[selectedApp.id] ?? [])
                      .filter((m) => current?.menus.get(m.id))
                      .map((m) => m.id)
                  )}
                  onToggle={toggleMenus}
                  disabled={!appOn}
                />
              )}
            </div>
          </ScrollArea>
        </section>
      </div>

      {/* 底部保存栏 */}
      <div className="flex items-center justify-end gap-3 rounded-xl border bg-card px-4 py-3">
        {dirtyCount > 0 ? (
          <Badge variant="destructive" className="gap-1.5">
            {dirtyCount} 项未保存更改
          </Badge>
        ) : (
          <span className="text-xs text-muted-foreground">授权与后端一致</span>
        )}
        <Button
          variant="outline"
          disabled={dirtyCount === 0 || saveMutation.isPending}
          onClick={() => {
            if (snapshot) setCurrent(structuredCopy(snapshot))
          }}
        >
          重置
        </Button>
        <Button disabled={dirtyCount === 0 || saveMutation.isPending} onClick={() => saveMutation.mutate()}>
          {saveMutation.isPending ? <Loader2 className="size-4 animate-spin" /> : <Save className="size-4" />}
          保存授权
        </Button>
      </div>
    </div>
  )
}

function structuredCopy(s: AuthSnapshot): AuthSnapshot {
  return { apps: new Map(s.apps), menus: new Map(s.menus) }
}
