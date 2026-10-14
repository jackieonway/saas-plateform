import * as React from "react"
import { Link, useSearchParams } from "react-router-dom"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useForm } from "react-hook-form"
import { z } from "zod"
import { zResolver } from "@/lib/form"
import { Building2, ChevronDown, ChevronRight, Landmark, ListTree, Loader2, Network, Pencil, Plus, Settings2, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Badge } from "@/components/ui/badge"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { Separator } from "@/components/ui/separator"
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form"
import { PageHeader } from "@/components/layout/PageHeader"
import { StatusBadge } from "@/components/StatusBadge"
import { ConfirmDialog } from "@/components/ConfirmDialog"
import { TreeEditor, type EditableNode } from "@/components/TreeEditor"
import { EmptyState, ErrorState } from "@/components/states"
import { OrgChartView } from "@/components/OrgChart"
import { tenantsApi } from "@/api/tenants"
import { organizationsApi } from "@/api/organizations"
import { departmentsApi } from "@/api/departments"
import { asArray, asPage } from "@/lib/api"
import { formatDateTime } from "@/lib/format"
import { expiryBadge, ExpiryCell, isoToLocalInput } from "@/lib/expiry"
import { coordinateField, coordinateOrNull, coordinatePoint, hasCoordinatePair, locationText } from "@/lib/location"
import { organizationTypes, organizationTypeLabels } from "@/lib/entity-types"
import { contactFields, requireContact } from "@/lib/contact"
import { ContactFields } from "@/components/ContactFields"
import { InitializeAdministratorButton } from "@/components/InitializeAdministratorButton"
import { ContactAdministratorDialog } from "@/components/ContactAdministratorDialog"
import { LocationPicker } from "@/components/LocationPicker"
import { RegionPicker } from "@/components/RegionPicker"
import { toast } from "sonner"
import type { ContactAdministrator, DeptNode, OrgNode, TenantRecord } from "@/api/types"

const nodeSchema = z.object({
  name: z.string().min(1, "请输入名称").max(64),
  code: z.string().min(1, "请输入编码").max(32).regex(/^[a-zA-Z0-9-]+$/, "仅支持字母、数字与中划线"),
  sortOrder: z.coerce.number().int().min(0).max(9999),
  description: z.string().max(255).optional(),
})
type NodeForm = z.infer<typeof nodeSchema>

/** 组织表单在部门表单基础上增加「过期时间」(空=永不过期; 接受 datetime-local 或合法 ISO) */
const orgNodeSchema = nodeSchema.extend({
  ...contactFields,
  organizationType: z.enum(organizationTypes),
  province: z.string().max(100, "省份名称过长"),
  city: z.string().max(100, "城市名称过长"),
  district: z.string().max(100, "区县名称过长"),
  township: z.string().max(128, "乡镇街道名称过长"),
  village: z.string().max(128, "村社区名称过长"),
  address: z.string().max(300, "详细地址过长"),
  longitude: coordinateField("经度", -180, 180),
  latitude: coordinateField("纬度", -90, 90),
  expiresAt: z
    .string()
    .refine((v) => !v || !Number.isNaN(new Date(v).getTime()), "无效的过期时间")
    .optional(),
}).refine(hasCoordinatePair, { message: "经纬度需同时填写或同时留空", path: ["longitude"] })
type OrgNodeForm = z.infer<typeof orgNodeSchema>

function orgToEditable(nodes: OrgNode[]): EditableNode[] {
  return nodes.map((n) => ({
    id: n.id,
    name: n.name,
    code: n.code,
    status: n.status,
    description: n.description,
    expiresAt: n.expiresAt ?? null,
    children: n.children ? orgToEditable(n.children) : [],
  }))
}

function deptToEditable(nodes: DeptNode[]): EditableNode[] {
  return nodes.map((n) => ({
    id: n.id,
    name: n.name,
    code: n.code,
    description: n.description,
    children: n.children ? deptToEditable(n.children) : [],
  }))
}

function findNode(nodes: OrgNode[] | DeptNode[], id: string | null): OrgNode | DeptNode | null {
  if (!id) return null
  for (const n of nodes) {
    if (n.id === id) return n
    const sub = n.children?.length ? findNode(n.children, id) : null
    if (sub) return sub
  }
  return null
}

export default function OrganizationsPage() {
  const queryClient = useQueryClient()
  const [params] = useSearchParams()

  /** 视图切换: 树形管理 / 架构图总览 */
  const [view, setView] = React.useState<"tree" | "chart">("tree")

  const tenantsQuery = useQuery({
    queryKey: ["tenants", "options"],
    queryFn: async () => asPage<TenantRecord>(await tenantsApi.options()).records,
    staleTime: 60_000,
    retry: 1,
  })
  const tenants = tenantsQuery.data ?? []
  const [tenantId, setTenantId] = React.useState<string | null>(null)
  React.useEffect(() => {
    if (!tenantId && tenants.length > 0) setTenantId(tenants[0].id)
  }, [tenants, tenantId])

  const treeQuery = useQuery({
    queryKey: ["org-tree", tenantId],
    queryFn: async () => asArray<OrgNode>(await organizationsApi.tree(tenantId!)),
    enabled: !!tenantId,
    staleTime: 30_000,
    retry: 1,
  })
  const orgTree = treeQuery.data ?? []

  const [selectedId, setSelectedId] = React.useState<string | null>(null)
  React.useEffect(() => {
    if (orgTree.length > 0 && !findNode(orgTree, selectedId)) {
      setSelectedId(orgTree[0].id)
    }
    if (orgTree.length === 0) setSelectedId(null)
  }, [orgTree, selectedId])
  const selected = findNode(orgTree, selectedId) as OrgNode | null

  // 层级路径面包屑
  const pathChain = React.useMemo(() => {
    const chain: OrgNode[] = []
    const walk = (nodes: OrgNode[], target: string, acc: OrgNode[]): boolean => {
      for (const n of nodes) {
        const next = [...acc, n]
        if (n.id === target) {
          chain.push(...next)
          return true
        }
        if (n.children?.length && walk(n.children, target, next)) return true
      }
      return false
    }
    if (selectedId) walk(orgTree, selectedId, [])
    return chain
  }, [orgTree, selectedId])

  /* ---------- 组织 CRUD ---------- */
  const [orgDialog, setOrgDialog] = React.useState(false)
  const [regionVersion, setRegionVersion] = React.useState<string | null>(null)
  const [orgEditing, setOrgEditing] = React.useState<OrgNode | null>(null)
  const [administrator, setAdministrator] = React.useState<ContactAdministrator | null>(null)
  const [orgParent, setOrgParent] = React.useState<string | null>(null)
  const orgForm = useForm<OrgNodeForm>({
    resolver: zResolver(orgNodeSchema.superRefine((values, ctx) => { if (!orgEditing) requireContact(values, ctx) })),
    defaultValues: { contactName: "", contactPhone: "", contactEmail: "", name: "", code: "", organizationType: "COMPANY", sortOrder: 0, description: "", province: "", city: "", district: "", township: "", village: "", address: "", longitude: "", latitude: "", expiresAt: "" },
  })

  const openOrgCreate = (parentId: string | null) => {
    setOrgEditing(null)
    setOrgParent(parentId)
    orgForm.reset({ contactName: "", contactPhone: "", contactEmail: "", name: "", code: "", organizationType: "COMPANY", sortOrder: 0, description: "", province: "", city: "", district: "", township: "", village: "", address: "", longitude: "", latitude: "", expiresAt: "" })
    setOrgDialog(true)
  }
  const openOrgEdit = (node: EditableNode) => {
    const full = findNode(orgTree, node.id) as OrgNode | null
    if (!full) return
    setOrgEditing(full)
    setOrgParent(full.parentId)
    orgForm.reset({
      name: full.name,
      code: full.code,
      organizationType: full.organizationType ?? "COMPANY",
      contactName: full.contactName ?? "", contactPhone: full.contactPhone ?? "", contactEmail: full.contactEmail ?? "",
      sortOrder: full.sortOrder ?? 0,
      description: full.description ?? "",
      province: full.province ?? "", city: full.city ?? "", district: full.district ?? "", township: full.township ?? "", village: full.village ?? "", address: full.address ?? "",
      longitude: full.longitude == null ? "" : String(full.longitude), latitude: full.latitude == null ? "" : String(full.latitude),
      expiresAt: isoToLocalInput(full.expiresAt),
    })
    setOrgDialog(true)
  }

  const orgSave = useMutation({
    mutationFn: async (v: OrgNodeForm) => {
      /* datetime-local("2026-12-31T23:59") → ISO; 空串 → null(清除/永久) */
      const payload = {
        tenantId: tenantId!,
        parentId: orgParent,
        name: v.name,
        code: v.code,
        organizationType: v.organizationType,
        contactName: v.contactName, contactPhone: v.contactPhone, contactEmail: v.contactEmail,
        sortOrder: v.sortOrder,
        description: v.description || "",
        province: v.province, city: v.city, district: v.district, township: v.township, village: v.village, address: v.address,
        longitude: coordinateOrNull(v.longitude), latitude: coordinateOrNull(v.latitude),
        expiresAt: v.expiresAt ? new Date(v.expiresAt).toISOString() : null,
      }
      if (orgEditing) await organizationsApi.update(orgEditing.id, payload)
      else return (await organizationsApi.create(payload)).administrator
    },
    onSuccess: (createdAdministrator) => {
      if (createdAdministrator) setAdministrator(createdAdministrator)
      toast.success(orgEditing ? "组织已更新" : "组织创建成功")
      setOrgDialog(false)
      queryClient.invalidateQueries({ queryKey: ["org-tree"] })
      queryClient.invalidateQueries({ queryKey: ["department-organizations"] })
      queryClient.invalidateQueries({ queryKey: ["roles"] })
      queryClient.invalidateQueries({ queryKey: ["users"] })
      queryClient.invalidateQueries({ queryKey: ["tenants"] })
      queryClient.invalidateQueries({ queryKey: ["profile"] })
    },
    onError: (e: Error) => toast.error(e.message || "保存失败"),
  })

  const [orgDeleting, setOrgDeleting] = React.useState<EditableNode | null>(null)
  const orgDelete = useMutation({
    mutationFn: (id: string) => organizationsApi.remove(id),
    onSuccess: () => {
      toast.success("组织已删除")
      setOrgDeleting(null)
      queryClient.invalidateQueries({ queryKey: ["org-tree"] })
      queryClient.invalidateQueries({ queryKey: ["department-organizations"] })
    },
    onError: (e: Error) => toast.error(e.message || "删除失败"),
  })

  /* ---------- 部门 CRUD(选中组织的部门树) ---------- */
  const deptTreeQuery = useQuery({
    queryKey: ["dept-tree", selectedId],
    queryFn: async () => asArray<DeptNode>(await departmentsApi.tree(selectedId!)),
    enabled: !!selectedId,
    staleTime: 30_000,
    retry: 1,
  })
  const deptTree = deptTreeQuery.data ?? []

  const [deptDialog, setDeptDialog] = React.useState(false)
  const [deptEditing, setDeptEditing] = React.useState<DeptNode | null>(null)
  const [deptParent, setDeptParent] = React.useState<string | null>(null)
  const deptForm = useForm<NodeForm>({
    resolver: zResolver(nodeSchema),
    defaultValues: { name: "", code: "", sortOrder: 0, description: "" },
  })

  const openDeptCreate = (parentId: string | null) => {
    setDeptEditing(null)
    setDeptParent(parentId)
    deptForm.reset({ name: "", code: "", sortOrder: 0, description: "" })
    setDeptDialog(true)
  }
  const openDeptEdit = (node: EditableNode) => {
    const full = findNode(deptTree, node.id) as DeptNode | null
    if (!full) return
    setDeptEditing(full)
    setDeptParent(full.parentId)
    deptForm.reset({ name: full.name, code: full.code, sortOrder: full.sortOrder ?? 0, description: full.description ?? "" })
    setDeptDialog(true)
  }

  const deptSave = useMutation({
    mutationFn: async (v: NodeForm) => {
      const payload = {
        organizationId: selectedId!,
        parentId: deptParent,
        name: v.name,
        code: v.code,
        sortOrder: v.sortOrder,
        description: v.description || "",
      }
      if (deptEditing) await departmentsApi.update(deptEditing.id, payload)
      else await departmentsApi.create(payload)
    },
    onSuccess: () => {
      toast.success(deptEditing ? "部门已更新" : "部门创建成功")
      setDeptDialog(false)
      queryClient.invalidateQueries({ queryKey: ["dept-tree"] })
    },
    onError: (e: Error) => toast.error(e.message || "保存失败"),
  })

  const [deptDeleting, setDeptDeleting] = React.useState<EditableNode | null>(null)
  const deptDelete = useMutation({
    mutationFn: (id: string) => departmentsApi.remove(id),
    onSuccess: () => {
      toast.success("部门已删除")
      setDeptDeleting(null)
      queryClient.invalidateQueries({ queryKey: ["dept-tree"] })
    },
    onError: (e: Error) => toast.error(e.message || "删除失败"),
  })

  return (
    <div>
      <PageHeader
        title="组织管理"
        description="维护租户下的多层级组织树与部门结构"
        icon={<Building2 className="size-5" />}
        actions={
          <>
            <Tabs value={view} onValueChange={(v) => setView(v as "tree" | "chart")}>
              <TabsList aria-label="视图切换">
                <TabsTrigger value="tree">
                  <ListTree className="size-3.5" />
                  树形管理
                </TabsTrigger>
                <TabsTrigger value="chart">
                  <Network className="size-3.5" />
                  架构图
                </TabsTrigger>
              </TabsList>
            </Tabs>
            {tenants.length > 0 && (
              <Select value={tenantId ?? ""} onValueChange={(v) => setTenantId(v)}>
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
            )}
          </>
        }
      />

      {view === "chart" ? (
        <OrgChartView
          nodes={orgTree}
          isLoading={tenantsQuery.isLoading || (!!tenantId && treeQuery.isLoading)}
          isError={tenantsQuery.isError || (!!tenantId && treeQuery.isError)}
          error={treeQuery.error ?? tenantsQuery.error}
          onRetry={() => (tenantsQuery.isError ? tenantsQuery.refetch() : treeQuery.refetch())}
          hasTenant={!!tenantId}
          selectedId={selectedId}
          onSelect={setSelectedId}
          onLocate={() => setView("tree")}
        />
      ) : (
      <div className="grid grid-cols-1 items-start gap-4 lg:grid-cols-2">
        {/* 左侧组织树 */}
        <aside className="min-w-0 rounded-xl border bg-card">
          <div className="flex items-center justify-between border-b px-4 py-3">
            <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">组织树</p>
            <Button size="sm" variant="outline" onClick={() => openOrgCreate(null)} disabled={!tenantId}>
              <Plus className="size-3.5" />
              新增根组织
            </Button>
          </div>
          <div className="max-h-[32rem] overflow-y-auto scroll-area p-2 lg:max-h-[calc(100dvh-17rem)]">
            {tenantsQuery.isLoading ? (
              <div className="space-y-1.5 p-2">
                {Array.from({ length: 6 }).map((_, i) => (
                  <div key={i} className="h-9 animate-pulse rounded-md bg-muted" style={{ marginLeft: `${(i % 3) * 14}px` }} />
                ))}
              </div>
            ) : tenantsQuery.isError ? (
              <div className="p-2">
                <ErrorState
                  className="border-0 bg-transparent"
                  message={tenantsQuery.error instanceof Error ? tenantsQuery.error.message : undefined}
                  onRetry={() => tenantsQuery.refetch()}
                />
              </div>
            ) : !tenantId ? (
              <EmptyState className="py-10" title="请先选择租户" description="左侧顶部选择租户后展示其组织树" />
            ) : treeQuery.isLoading ? (
              <div className="space-y-1.5 p-2">
                {Array.from({ length: 6 }).map((_, i) => (
                  <div key={i} className="h-9 animate-pulse rounded-md bg-muted" style={{ marginLeft: `${(i % 3) * 14}px` }} />
                ))}
              </div>
            ) : treeQuery.isError ? (
              <div className="p-2">
                <ErrorState
                  className="border-0 bg-transparent"
                  message={treeQuery.error instanceof Error ? treeQuery.error.message : undefined}
                  onRetry={() => treeQuery.refetch()}
                />
              </div>
            ) : orgTree.length === 0 ? (
              <EmptyState className="py-10" title="该租户暂无组织" description="从根组织开始搭建层级结构" />
            ) : (
              <div className="[&_button]:ring-inset">
                <SelectableTree
                  nodes={orgToEditable(orgTree)}
                  selectedId={selectedId}
                  onSelect={setSelectedId}
                  onAdd={openOrgCreate}
                  onEdit={openOrgEdit}
                  onDelete={setOrgDeleting}
                  onAdministrator={setAdministrator}
                />
              </div>
            )}
          </div>
        </aside>

        {/* 右侧详情 */}
        <section className="min-w-0 space-y-4">
          {!selected ? (
            <div className="rounded-xl border bg-card">
              <EmptyState
                className="py-16"
                title="选择一个组织"
                description="在左侧组织树中选择节点查看详情与管理部门"
                icon={<Building2 className="size-6" />}
              />
            </div>
          ) : (
            <>
              {/* 基本信息 */}
              <div className="rounded-xl border bg-card p-5">
                <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                  <div className="min-w-0 space-y-2">
                    <div className="flex flex-wrap items-center gap-2">
                      <h2 className="text-lg font-semibold">{selected.name}</h2>
                      <Badge variant="outline">{organizationTypeLabels[selected.organizationType ?? "COMPANY"]}</Badge>
                      <StatusBadge status={selected.status} />
                      {expiryBadge(selected.expiresAt ?? null)}
                      <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-muted-foreground">{selected.code}</code>
                    </div>
                    {pathChain.length > 0 && (
                      <Breadcrumb>
                        <BreadcrumbList>
                          {pathChain.map((n, i) => (
                            <React.Fragment key={n.id}>
                              {i > 0 && <BreadcrumbSeparator />}
                              <BreadcrumbItem>
                                {i === pathChain.length - 1 ? (
                                  <span className="font-medium text-foreground">{n.name}</span>
                                ) : (
                                  <BreadcrumbLink asChild>
                                    <button type="button" onClick={() => setSelectedId(n.id)} className="text-muted-foreground">
                                      {n.name}
                                    </button>
                                  </BreadcrumbLink>
                                )}
                              </BreadcrumbItem>
                            </React.Fragment>
                          ))}
                        </BreadcrumbList>
                      </Breadcrumb>
                    )}
                    {selected.description && <p className="text-sm text-muted-foreground">{selected.description}</p>}
                    {(selected.contactName || selected.contactPhone || selected.contactEmail) && <p className="text-sm text-muted-foreground">
                      联系人：{[selected.contactName, selected.contactPhone, selected.contactEmail].filter(Boolean).join(" · ")}
                    </p>}
                    {locationText(selected) && <p className="text-sm text-muted-foreground">地址：{locationText(selected)}</p>}
                    {selected.longitude != null && selected.latitude != null && (
                      <p className="text-xs text-muted-foreground">坐标：{selected.longitude}, {selected.latitude}</p>
                    )}
                    <p className="text-xs text-muted-foreground">
                      排序 #{selected.sortOrder ?? 0} · 所属租户 {tenants.find((t) => t.id === selected.tenantId)?.name ?? "—"}
                      <span className="mx-1.5 text-muted-foreground/40">·</span>
                      有效期{" "}
                      {selected.expiresAt ? (
                        <ExpiryCell expiresAt={selected.expiresAt} />
                      ) : (
                        <span className="whitespace-nowrap">永不过期</span>
                      )}
                    </p>
                    <p className="text-xs text-muted-foreground">
                      管理员可在 <Link to="/system/users" className="text-primary hover:underline">用户管理</Link> 中维护组织成员，
                      再到 <Link to="/system/roles" className="text-primary hover:underline">角色管理</Link> 分配内置管理员角色。
                    </p>
                  </div>
                  <div className="flex shrink-0 items-center gap-1 self-start">
                  <InitializeAdministratorButton key={selected.id} name={selected.name} initialize={() => organizationsApi.initializeAdministrator(selected.id)} onSuccess={setAdministrator} />
                  <Button asChild>
                    <Link to={`/system/authorizations?organizationId=${selected.id}`}>
                      <Settings2 className="size-4" />
                      授权管理
                    </Link>
                  </Button>
                  </div>
                </div>
              </div>

              {/* 部门管理 */}
              <div className="rounded-xl border bg-card">
                <div className="flex flex-col gap-2 border-b px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
                  <div>
                    <p className="text-sm font-semibold">部门管理</p>
                    <p className="text-xs text-muted-foreground">{selected.name} 下的部门树</p>
                  </div>
                  <Button size="sm" variant="outline" onClick={() => openDeptCreate(null)}>
                    <Plus className="size-3.5" />
                    新增根部门
                  </Button>
                </div>
                <div className="p-3">
                  {deptTreeQuery.isLoading ? (
                    <div className="space-y-1.5 p-2">
                      {Array.from({ length: 4 }).map((_, i) => (
                        <div key={i} className="h-9 animate-pulse rounded-md bg-muted" style={{ marginLeft: `${(i % 2) * 14}px` }} />
                      ))}
                    </div>
                  ) : deptTreeQuery.isError ? (
                    <ErrorState
                      className="border-0 bg-transparent"
                      message={deptTreeQuery.error instanceof Error ? deptTreeQuery.error.message : undefined}
                      onRetry={() => deptTreeQuery.refetch()}
                    />
                  ) : deptTree.length === 0 ? (
                    <EmptyState
                      className="py-10"
                      title="暂无部门"
                      description="为该组织创建部门,用于用户归属与数据权限"
                    />
                  ) : (
                    <TreeEditor
                      nodes={deptToEditable(deptTree)}
                      onAdd={openDeptCreate}
                      onEdit={openDeptEdit}
                      onDelete={setDeptDeleting}
                      onAddRoot={() => openDeptCreate(null)}
                      addLabel="新增根部门"
                      className="max-h-[32rem]"
                    />
                  )}
                </div>
              </div>
            </>
          )}
        </section>
      </div>
      )}

      {/* 组织弹窗 */}
      <Dialog open={orgDialog} onOpenChange={setOrgDialog}>
        <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{orgEditing ? `编辑组织「${orgEditing.name}」` : "新增组织"}</DialogTitle>
            <DialogDescription>
              {orgEditing ? "维护组织基本信息" : orgParent ? "将在所选组织下创建子组织；管理员可在创建后通过用户管理与角色管理单独维护。" : "将创建为根组织；管理员可在创建后通过用户管理与角色管理单独维护。"}
            </DialogDescription>
          </DialogHeader>
          <Form {...orgForm}>
            <form id="org-form" onSubmit={orgForm.handleSubmit((v) => orgSave.mutate(v))} className="space-y-4">
              <FormField
                control={orgForm.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      组织名称 <span className="text-destructive">*</span>
                    </FormLabel>
                    <FormControl>
                      <Input placeholder="如: 华东区分公司" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={orgForm.control}
                name="code"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      组织编码 <span className="text-destructive">*</span>
                    </FormLabel>
                    <FormControl>
                      <Input placeholder="如: east-china" className="font-mono" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={orgForm.control}
                name="organizationType"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>组织类型 <span className="text-destructive">*</span></FormLabel>
                    <Select value={field.value} onValueChange={field.onChange}>
                      <FormControl>
                        <SelectTrigger aria-label="组织类型"><SelectValue placeholder="选择组织类型" /></SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        {organizationTypes.map((type) => <SelectItem key={type} value={type}>{organizationTypeLabels[type]}</SelectItem>)}
                      </SelectContent>
                    </Select>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={orgForm.control}
                name="expiresAt"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>过期时间</FormLabel>
                    <FormControl>
                      <Input
                        type="datetime-local"
                        aria-label="组织过期时间"
                        {...field}
                        value={field.value ?? ""}
                      />
                    </FormControl>
                    <FormDescription>留空表示永不过期；到期后该组织下所有用户无法登录</FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={orgForm.control}
                name="sortOrder"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>排序值</FormLabel>
                    <FormControl>
                      <Input type="number" min={0} {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={orgForm.control}
                name="description"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>描述</FormLabel>
                    <FormControl>
                      <Textarea rows={2} placeholder="选填" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <ContactFields creating={!orgEditing} />
              <div className="space-y-3 border-t pt-4">
                <p className="text-sm font-medium">地点标注</p>
                <RegionPicker version={regionVersion} onVersionChange={setRegionVersion} />
                <FormField control={orgForm.control} name="address" render={({ field }) => <FormItem>
                  <FormLabel>详细地址</FormLabel><FormControl><Input placeholder="街道、门牌号等" {...field} /></FormControl><FormMessage />
                </FormItem>} />
                <LocationPicker
                  regionVersion={regionVersion}
                  value={coordinatePoint(orgForm.watch("longitude"), orgForm.watch("latitude"))}
                  searchHint={locationText(orgForm.watch())}
                  onChange={(point) => {
                    orgForm.setValue("longitude", point ? String(point.longitude) : "", { shouldDirty: true })
                    orgForm.setValue("latitude", point ? String(point.latitude) : "", { shouldDirty: true })
                    for (const key of ["province", "city", "district", "township", "village", "address"] as const) {
                      const value = point?.location?.[key]
                      if (value !== undefined) orgForm.setValue(key, value, { shouldDirty: true })
                    }
                    void orgForm.trigger(["longitude", "latitude", "province", "city", "district", "township", "village", "address"])
                  }}
                />
                <div className="grid grid-cols-2 gap-2">
                  <FormField control={orgForm.control} name="longitude" render={({ field }) => <FormItem>
                    <FormLabel>经度</FormLabel><FormControl><Input type="number" step="any" placeholder="-180 ～ 180" {...field} /></FormControl><FormMessage />
                  </FormItem>} />
                  <FormField control={orgForm.control} name="latitude" render={({ field }) => <FormItem>
                    <FormLabel>纬度</FormLabel><FormControl><Input type="number" step="any" placeholder="-90 ～ 90" {...field} /></FormControl><FormMessage />
                  </FormItem>} />
                </div>
                <p className="text-xs text-muted-foreground">使用 WGS-84 坐标；经纬度需同时填写，留空表示尚未标注。</p>
              </div>
            </form>
          </Form>
          <DialogFooter>
            <Button variant="outline" onClick={() => setOrgDialog(false)} disabled={orgSave.isPending}>
              取消
            </Button>
            <Button type="submit" form="org-form" disabled={orgSave.isPending}>
              {orgSave.isPending && <Loader2 className="size-4 animate-spin" />}
              {orgEditing ? "保存更改" : "创建"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* 部门弹窗 */}
      <Dialog open={deptDialog} onOpenChange={setDeptDialog}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{deptEditing ? `编辑部门「${deptEditing.name}」` : "新增部门"}</DialogTitle>
            <DialogDescription>
              {deptEditing ? "" : deptParent ? "将在所选部门下创建子部门" : "将创建为根部门"}
            </DialogDescription>
          </DialogHeader>
          <Form {...deptForm}>
            <form id="dept-form" onSubmit={deptForm.handleSubmit((v) => deptSave.mutate(v))} className="space-y-4">
              <FormField
                control={deptForm.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      部门名称 <span className="text-destructive">*</span>
                    </FormLabel>
                    <FormControl>
                      <Input placeholder="如: 研发部" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={deptForm.control}
                name="code"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      部门编码 <span className="text-destructive">*</span>
                    </FormLabel>
                    <FormControl>
                      <Input placeholder="如: rd" className="font-mono" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={deptForm.control}
                name="sortOrder"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>排序值</FormLabel>
                    <FormControl>
                      <Input type="number" min={0} {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={deptForm.control}
                name="description"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>描述</FormLabel>
                    <FormControl>
                      <Textarea rows={2} placeholder="选填" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </form>
          </Form>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeptDialog(false)} disabled={deptSave.isPending}>
              取消
            </Button>
            <Button type="submit" form="dept-form" disabled={deptSave.isPending}>
              {deptSave.isPending && <Loader2 className="size-4 animate-spin" />}
              {deptEditing ? "保存更改" : "创建"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ContactAdministratorDialog administrator={administrator} onClose={() => setAdministrator(null)} />
      <ConfirmDialog
        open={!!orgDeleting}
        onOpenChange={(o) => !o && setOrgDeleting(null)}
        title={`删除组织「${orgDeleting?.name ?? ""}」?`}
        description={
          <>
            将级联删除 <b>子组织、部门、成员关系、组织授权及其下角色</b>,不可撤销。
          </>
        }
        loading={orgDelete.isPending}
        onConfirm={() => orgDeleting && orgDelete.mutate(orgDeleting.id)}
      />
      <ConfirmDialog
        open={!!deptDeleting}
        onOpenChange={(o) => !o && setDeptDeleting(null)}
        title={`删除部门「${deptDeleting?.name ?? ""}」?`}
        description={
          <>
            将<b>级联删除子部门</b>并解除相关用户归属,不可撤销。
          </>
        }
        loading={deptDelete.isPending}
        onConfirm={() => deptDeleting && deptDelete.mutate(deptDeleting.id)}
      />
    </div>
  )
}

/** 可选择的树编辑器(在 TreeEditor 行为上叠加选中) */
function SelectableTree({
  nodes,
  selectedId,
  onSelect,
  onAdd,
  onEdit,
  onDelete,
  onAdministrator,
}: {
  nodes: EditableNode[]
  selectedId: string | null
  onSelect: (id: string) => void
  onAdd: (parentId: string) => void
  onEdit: (node: EditableNode) => void
  onDelete: (node: EditableNode) => void
  onAdministrator: (administrator: ContactAdministrator) => void
}) {
  return (
    <ul role="tree" className="space-y-0.5">
      {nodes.map((n) => (
        <SelectRow
          key={n.id}
          node={n}
          siblings={nodes}
          depth={0}
          selectedId={selectedId}
          onSelect={onSelect}
          onAdd={onAdd}
          onEdit={onEdit}
          onDelete={onDelete}
          onAdministrator={onAdministrator}
        />
      ))}
    </ul>
  )
}

function SelectRow({
  node,
  siblings,
  depth,
  selectedId,
  onSelect,
  onAdd,
  onEdit,
  onDelete,
  onAdministrator,
}: {
  node: EditableNode
  siblings: EditableNode[]
  depth: number
  selectedId: string | null
  onSelect: (id: string) => void
  onAdd: (parentId: string) => void
  onEdit: (node: EditableNode) => void
  onDelete: (node: EditableNode) => void
  onAdministrator: (administrator: ContactAdministrator) => void
}) {
  const [open, setOpen] = React.useState(true)
  const hasChildren = node.children.length > 0
  const active = node.id === selectedId
  return (
    <li>
      <div
        className={cnRow(active, depth)}
        onClick={() => onSelect(node.id)}
        role="treeitem"
        aria-selected={active}
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault()
            onSelect(node.id)
          }
        }}
      >
        {hasChildren ? (
          <button
            type="button"
            aria-label={open ? "折叠" : "展开"}
            onClick={(e) => {
              e.stopPropagation()
              setOpen((o) => !o)
            }}
            className="flex size-5 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-muted"
          >
            {open ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
          </button>
        ) : (
          <span className="w-5 shrink-0" />
        )}
        <span className={`size-1.5 shrink-0 rounded-full ${node.status === "DISABLED" ? "bg-muted-foreground/40" : "bg-primary"}`} />
        <span className={cnText(active)}>{node.name}</span>
        {expiryBadge(node.expiresAt ?? null)}
        {node.code && <code className="hidden font-mono text-[10px] text-muted-foreground/50 sm:inline">{node.code}</code>}
        <div className="ml-auto flex shrink-0 items-center gap-0.5">
          <div onClick={(event) => event.stopPropagation()} onKeyDown={(event) => event.stopPropagation()}>
            <InitializeAdministratorButton name={node.name} initialize={() => organizationsApi.initializeAdministrator(node.id)} onSuccess={onAdministrator} />
          </div>
          <IconBtn label="新增子组织" onClick={() => onAdd(node.id)}>
            <Plus className="size-3.5" />
          </IconBtn>
          <IconBtn label="编辑" onClick={() => onEdit(node)}>
            <Pencil className="size-3.5" />
          </IconBtn>
          <IconBtn label="删除" danger onClick={() => onDelete(node)}>
            <Trash2 className="size-3.5" />
          </IconBtn>
        </div>
      </div>
      {hasChildren && open && (
        <ul className="space-y-0.5">
          {node.children.map((c) => (
            <SelectRow
              key={c.id}
              node={c}
              siblings={node.children}
              depth={depth + 1}
              selectedId={selectedId}
              onSelect={onSelect}
              onAdd={onAdd}
              onEdit={onEdit}
              onDelete={onDelete}
              onAdministrator={onAdministrator}
            />
          ))}
        </ul>
      )}
    </li>
  )
}

function cnRow(active: boolean, depth: number) {
  return [
    "group flex cursor-pointer select-none items-center gap-2 rounded-lg px-2 py-2 transition-colors",
    active ? "bg-primary/10 ring-1 ring-primary/25" : "hover:bg-muted/60",
    depth > 0 ? "ml-3 border-l border-border/60 pl-3" : "",
  ].join(" ")
}
function cnText(active: boolean) {
  return `min-w-0 truncate text-sm font-medium ${active ? "text-primary" : ""}`
}

function IconBtn({
  children,
  label,
  onClick,
  danger,
}: {
  children: React.ReactNode
  label: string
  onClick: () => void
  danger?: boolean
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className={`size-7 ${danger ? "text-destructive hover:text-destructive" : ""}`}
          aria-label={label}
          onClick={(e) => {
            e.stopPropagation()
            onClick()
          }}
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}
