import * as React from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useForm } from "react-hook-form"
import { z } from "zod"
import { zResolver } from "@/lib/form"
import {
  Loader2,
  Lock,
  Pencil,
  Plus,
  Search,
  Settings2,
  Shield,
  Trash2,
  Users,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Badge } from "@/components/ui/badge"
import { Checkbox } from "@/components/ui/checkbox"
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
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { ScrollArea } from "@/components/ui/scroll-area"
import { PageHeader } from "@/components/layout/PageHeader"
import { StatusBadge } from "@/components/StatusBadge"
import { ConfirmDialog } from "@/components/ConfirmDialog"
import { DataPagination } from "@/components/DataPagination"
import { EntityTreePicker, type PickerNode } from "@/components/EntityTreePicker"
import { TreeCheck, TreeCheckToolbar, type TreeCheckNode } from "@/components/TreeCheck"
import { TableSkeleton, EmptyState, ErrorState } from "@/components/states"
import { TableDensityToggle } from "@/components/TableDensityToggle"
import { ColumnVisibilityDropdown } from "@/components/ColumnVisibilityDropdown"
import { useSyncedKeyword } from "@/hooks/use-synced-keyword"
import { useTableDensity, DENSITY_BODY_CLASS } from "@/hooks/use-table-density"
import { useColumnVisibility, type ColumnDef } from "@/hooks/use-column-visibility"
import { rolesApi } from "@/api/roles"
import { tenantsApi } from "@/api/tenants"
import { organizationsApi } from "@/api/organizations"
import { usersApi } from "@/api/users"
import { asArray, asPage } from "@/lib/api"
import { AppIcon } from "@/lib/icons"
import { buildTree, type TreeNode } from "@/lib/tree"
import { formatDateTime, useDebounced } from "@/lib/format"
import { toast } from "sonner"
import { cn } from "@/lib/utils"
import type {
  AuthorizationMenu,
  OrgNode,
  RoleRecord,
  TenantRecord,
  UserRecord,
} from "@/api/types"

const roleSchema = z.object({
  name: z.string().min(1, "请输入角色名称").max(64),
  code: z
    .string()
    .min(2, "请输入角色编码")
    .max(32)
    .regex(/^[a-zA-Z0-9:_-]+$/, "仅支持字母、数字、冒号、下划线与中划线"),
  description: z.string().max(255).optional(),
  organizationId: z.string().min(1, "请选择所属组织"),
})
type RoleForm = z.infer<typeof roleSchema>

/** 角色表列定义:角色(名称+编码)/操作常显,其余列可隐藏(「编码」为角色列内子文本,随列常显) */
const ROLE_COLUMNS: ColumnDef[] = [
  { key: "name", label: "角色", required: true },
  { key: "org", label: "所属组织" },
  { key: "type", label: "类型(内置)" },
  { key: "menuCount", label: "菜单数" },
  { key: "userCount", label: "成员数" },
  { key: "status", label: "状态" },
  { key: "createdAt", label: "创建时间" },
  { key: "actions", label: "操作", required: true },
]

export default function RolesPage() {
  const queryClient = useQueryClient()
  const [page, setPage] = React.useState(1)
  const [size, setSize] = React.useState(10)
  // 关键字与 URL ?q= 双向同步(命令面板实体跳转预填)
  const [keyword, setKeyword] = useSyncedKeyword()
  const [tenantId, setTenantId] = React.useState<string | null>(null)
  const [orgFilter, setOrgFilter] = React.useState<string | null>(null)
  const debounced = useDebounced(keyword, 300)

  const tenantsQuery = useQuery({
    queryKey: ["tenants", "options"],
    queryFn: async () => asPage<TenantRecord>(await tenantsApi.options()).records,
    staleTime: 60_000,
    retry: 1,
  })
  const tenants = tenantsQuery.data ?? []
  React.useEffect(() => {
    if (!tenantId && tenants.length > 0) setTenantId(tenants[0].id)
  }, [tenants, tenantId])

  const orgTreeQuery = useQuery({
    queryKey: ["org-tree", tenantId],
    queryFn: async () => asArray<OrgNode>(await organizationsApi.tree(tenantId!)),
    enabled: !!tenantId,
    staleTime: 60_000,
    retry: 1,
  })
  const orgTree = orgTreeQuery.data ?? []

  const listQuery = useQuery({
    queryKey: ["roles", page, size, debounced, tenantId, orgFilter],
    queryFn: async () =>
      asPage<RoleRecord>(
        await rolesApi.list({ page, size, keyword: debounced, tenantId: tenantId ?? undefined, organizationId: orgFilter ?? undefined })
      ),
    staleTime: 30_000,
    retry: 1,
    placeholderData: (prev) => prev,
  })
  React.useEffect(() => {
    setPage(1)
  }, [debounced, tenantId, orgFilter, size])

  const invalidate = () => queryClient.invalidateQueries({ queryKey: ["roles"] })

  /* ---- 新建 / 编辑 ---- */
  const [dialogOpen, setDialogOpen] = React.useState(false)
  const [editing, setEditing] = React.useState<RoleRecord | null>(null)
  const form = useForm<RoleForm>({
    resolver: zResolver(roleSchema),
    defaultValues: { name: "", code: "", description: "", organizationId: "" },
  })

  const openCreate = () => {
    setEditing(null)
    form.reset({ name: "", code: "", description: "", organizationId: "" })
    setDialogOpen(true)
  }
  const openEdit = (r: RoleRecord) => {
    setEditing(r)
    form.reset({ name: r.name, code: r.code, description: r.description ?? "", organizationId: r.organizationId })
    setDialogOpen(true)
  }

  const saveMutation = useMutation({
    mutationFn: async (v: RoleForm) => {
      if (editing) await rolesApi.update(editing.id, v)
      else await rolesApi.create(v)
    },
    onSuccess: () => {
      toast.success(editing ? "角色已更新" : "角色创建成功")
      setDialogOpen(false)
      invalidate()
    },
    onError: (e: Error) => toast.error(e.message || "保存失败"),
  })

  const [deleting, setDeleting] = React.useState<RoleRecord | null>(null)
  const deleteMutation = useMutation({
    mutationFn: (id: string) => rolesApi.remove(id),
    onSuccess: () => {
      toast.success("角色已删除")
      setDeleting(null)
      invalidate()
    },
    onError: (e: Error) => toast.error(e.message || "删除失败"),
  })

  /* ---- 菜单权限 / 成员 弹窗 ---- */
  const [menusRole, setMenusRole] = React.useState<RoleRecord | null>(null)
  const [usersRole, setUsersRole] = React.useState<RoleRecord | null>(null)

  /* ---- 表格密度 ---- */
  const { density } = useTableDensity()

  const records = listQuery.data?.records ?? []
  const total = listQuery.data?.total ?? 0

  /* ---- 列显隐 ---- */
  const { visibleSet: roleCols, isVisible, setVisible } = useColumnVisibility("roles", ROLE_COLUMNS)

  return (
    <div>
      <PageHeader
        title="角色管理"
        description="角色基于组织的授权范围建立,精确分配菜单与按钮权限"
        icon={<Shield className="size-5" />}
        actions={
          <Button onClick={openCreate}>
            <Plus className="size-4" />
            新建角色
          </Button>
        }
      />

      {/* 筛选 */}
      <div className="mb-4 flex flex-col gap-2 lg:flex-row lg:items-center">
        <div className="relative flex-1 lg:max-w-xs">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={keyword}
            onChange={(e) => setKeyword(e.target.value)}
            placeholder="搜索角色名称 / 编码"
            aria-label="搜索角色"
            className="pl-9"
          />
        </div>
        <Select value={tenantId ?? ""} onValueChange={(v) => (setTenantId(v), setOrgFilter(null))}>
          <SelectTrigger className="w-full lg:w-44" aria-label="租户筛选">
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
          nodes={orgTree.map((n) => ({ id: n.id, name: n.name, code: n.code, children: toPicker(n.children) }))}
          value={orgFilter}
          onSelect={setOrgFilter}
          placeholder="按组织筛选"
          className="w-full lg:w-52"
        />
        <div className="flex items-center gap-2 lg:ml-auto">
          <span className="text-xs text-muted-foreground">
            {listQuery.isFetching && <Loader2 className="mr-1 inline size-3 animate-spin" />}
            共 {total} 个角色
          </span>
          <ColumnVisibilityDropdown
            table="roles"
            columns={ROLE_COLUMNS}
            visible={[...roleCols]}
            onChange={setVisible}
          />
          <TableDensityToggle />
        </div>
      </div>

      {/* 表格 */}
      <div className="rounded-xl border bg-card">
        {listQuery.isLoading ? (
          <div className="p-2">
            <TableSkeleton cols={8} />
          </div>
        ) : listQuery.isError ? (
          <ErrorState
            className="m-2 border-0 bg-transparent"
            message={listQuery.error instanceof Error ? listQuery.error.message : undefined}
            onRetry={() => listQuery.refetch()}
          />
        ) : records.length === 0 ? (
          <EmptyState
            title={keyword || orgFilter ? "没有匹配的角色" : "还没有角色"}
            description={keyword || orgFilter ? "调整筛选条件试试" : "创建第一个角色,为其分配菜单权限"}
            action={
              !keyword && !orgFilter && (
                <Button size="sm" onClick={openCreate}>
                  <Plus className="size-4" />
                  新建角色
                </Button>
              )
            }
          />
        ) : (
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className="pl-5 text-xs font-medium uppercase tracking-wider text-muted-foreground">角色</TableHead>
                {isVisible("org") && (
                  <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">所属组织</TableHead>
                )}
                {isVisible("type") && (
                  <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">类型</TableHead>
                )}
                {isVisible("menuCount") && (
                  <TableHead className="text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">菜单数</TableHead>
                )}
                {isVisible("userCount") && (
                  <TableHead className="text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">成员数</TableHead>
                )}
                {isVisible("status") && (
                  <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">状态</TableHead>
                )}
                {isVisible("createdAt") && (
                  <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">创建时间</TableHead>
                )}
                <TableHead className="pr-5 text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody className={DENSITY_BODY_CLASS[density]}>
              {records.map((r) => (
                <TableRow key={r.id}>
                  <TableCell className="pl-5">
                    <div className="flex items-center gap-2.5">
                      <span className="flex size-8 items-center justify-center rounded-lg bg-primary/10 text-primary">
                        <Shield className="size-4" />
                      </span>
                      <div className="min-w-0">
                        <p className="flex items-center gap-1.5 font-medium">
                          <span className="truncate">{r.name}</span>
                          {r.builtIn && (
                            <Tooltip>
                              <TooltipTrigger asChild>
                                <Badge variant="outline" className="gap-1 text-[10px]">
                                  <Lock className="size-2.5" />
                                  内置
                                </Badge>
                              </TooltipTrigger>
                              <TooltipContent>内置角色不可编辑或删除</TooltipContent>
                            </Tooltip>
                          )}
                        </p>
                        <code className="block truncate font-mono text-xs text-muted-foreground">{r.code}</code>
                      </div>
                    </div>
                  </TableCell>
                  {isVisible("org") && (
                    <TableCell className="max-w-[11rem]" title={r.organizationName}>
                      <span className="block truncate text-sm">{r.organizationName}</span>
                    </TableCell>
                  )}
                  {isVisible("type") && (
                    <TableCell>
                      <Badge variant={r.builtIn ? "secondary" : "outline"} className="text-[11px]">
                        {r.builtIn ? "内置角色" : "自定义"}
                      </Badge>
                    </TableCell>
                  )}
                  {isVisible("menuCount") && (
                    <TableCell className="text-right tabular-nums">{r.menuCount}</TableCell>
                  )}
                  {isVisible("userCount") && (
                    <TableCell className="text-right tabular-nums">{r.userCount}</TableCell>
                  )}
                  {isVisible("status") && (
                    <TableCell>
                      <StatusBadge status={r.status} />
                    </TableCell>
                  )}
                  {isVisible("createdAt") && (
                    <TableCell className="text-xs text-muted-foreground">{formatDateTime(r.createdAt)}</TableCell>
                  )}
                  <TableCell className="pr-5">
                    <div className="flex items-center justify-end gap-0.5">
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button variant="ghost" size="icon" className="size-8" aria-label="菜单权限" disabled={r.builtIn && ["tenant_admin", "org_admin"].includes(r.code)} onClick={() => setMenusRole(r)}>
                            <Settings2 className="size-4" />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>{r.builtIn && ["tenant_admin", "org_admin"].includes(r.code) ? "默认角色菜单随授权自动同步" : "菜单权限"}</TooltipContent>
                      </Tooltip>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button variant="ghost" size="icon" className="size-8" aria-label="成员管理" onClick={() => setUsersRole(r)}>
                            <Users className="size-4" />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>成员管理</TooltipContent>
                      </Tooltip>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="size-8"
                            aria-label={r.builtIn ? "内置角色不可编辑" : "编辑"}
                            disabled={r.builtIn}
                            onClick={() => openEdit(r)}
                          >
                            <Pencil className="size-4" />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>{r.builtIn ? "内置角色不可编辑" : "编辑"}</TooltipContent>
                      </Tooltip>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="size-8 text-destructive hover:text-destructive"
                            aria-label={r.builtIn ? "内置角色不可删除" : "删除"}
                            disabled={r.builtIn}
                            onClick={() => setDeleting(r)}
                          >
                            <Trash2 className="size-4" />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>{r.builtIn ? "内置角色不可删除" : "删除"}</TooltipContent>
                      </Tooltip>
                    </div>
                  </TableCell>
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

      {/* 新建/编辑 */}
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{editing ? `编辑角色「${editing.name}」` : "新建角色"}</DialogTitle>
            <DialogDescription>
              {editing ? "所属组织创建后不可变更" : "角色可分配的菜单由该组织的授权范围决定"}
            </DialogDescription>
          </DialogHeader>
          <Form {...form}>
            <form id="role-form" onSubmit={form.handleSubmit((v) => saveMutation.mutate(v))} className="space-y-4">
              <FormField
                control={form.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      角色名称 <span className="text-destructive">*</span>
                    </FormLabel>
                    <FormControl>
                      <Input placeholder="如: 数据管理员" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="code"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      角色编码 <span className="text-destructive">*</span>
                    </FormLabel>
                    <FormControl>
                      <Input placeholder="如: data_admin" className="font-mono" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="organizationId"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      所属组织 <span className="text-destructive">*</span>
                    </FormLabel>
                    <FormControl>
                      <EntityTreePicker
                        nodes={orgTree.map((n) => ({ id: n.id, name: n.name, code: n.code, children: toPicker(n.children) }))}
                        value={field.value || null}
                        onSelect={(id) => field.onChange(id ?? "")}
                        placeholder="选择组织(必选)"
                        allowClear={false}
                      />
                    </FormControl>
                    <FormDescription>角色可分配的菜单由该组织的授权范围决定</FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
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
            <Button variant="outline" onClick={() => setDialogOpen(false)} disabled={saveMutation.isPending}>
              取消
            </Button>
            <Button type="submit" form="role-form" disabled={saveMutation.isPending}>
              {saveMutation.isPending && <Loader2 className="size-4 animate-spin" />}
              {editing ? "保存更改" : "创建"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* 菜单权限 / 成员 */}
      <RoleMenusDialog role={menusRole} onClose={() => setMenusRole(null)} onSaved={invalidate} />
      <RoleUsersDialog role={usersRole} onClose={() => setUsersRole(null)} onSaved={invalidate} />

      <ConfirmDialog
        open={!!deleting}
        onOpenChange={(o) => !o && setDeleting(null)}
        title={`删除角色「${deleting?.name ?? ""}」?`}
        description="将删除该角色及其全部菜单权限与用户分配,不可撤销。"
        loading={deleteMutation.isPending}
        onConfirm={() => deleting && deleteMutation.mutate(deleting.id)}
      />
    </div>
  )
}

function toPicker(children?: OrgNode[]): PickerNode[] {
  return (children ?? []).map((c) => ({ id: c.id, name: c.name, code: c.code, children: toPicker(c.children) }))
}

/* ============ 菜单权限弹窗 ============ */

function RoleMenusDialog({
  role,
  onClose,
  onSaved,
}: {
  role: RoleRecord | null
  onClose: () => void
  onSaved: () => void
}) {
  const queryClient = useQueryClient()
  const [selectedAppId, setSelectedAppId] = React.useState<string | null>(null)
  const [checked, setChecked] = React.useState<Set<string>>(new Set())
  const [initial, setInitial] = React.useState<Set<string>>(new Set())

  // 组织授权范围(应用 + 菜单)
  const authQuery = useQuery({
    queryKey: ["authorizations", "org", role?.organizationId],
    queryFn: () => organizationsApi.authorizations(role!.organizationId),
    enabled: !!role,
    staleTime: 30_000,
    retry: 1,
  })

  const roleMenusQuery = useQuery({
    queryKey: ["role-menus", role?.id],
    queryFn: () => rolesApi.menus(role!.id),
    enabled: !!role,
    staleTime: 30_000,
    retry: 1,
  })

  React.useEffect(() => {
    if (roleMenusQuery.data?.menuIds) {
      setChecked(new Set(roleMenusQuery.data.menuIds))
      setInitial(new Set(roleMenusQuery.data.menuIds))
    }
  }, [roleMenusQuery.data])

  React.useEffect(() => {
    if (authQuery.data) {
      const first = authQuery.data.applications.find((a) => a.authorized) ?? authQuery.data.applications[0]
      setSelectedAppId((prev) => (prev && authQuery.data?.applications.some((a) => a.id === prev) ? prev : first?.id ?? null))
    }
  }, [authQuery.data])

  const saveMutation = useMutation({
    mutationFn: () => rolesApi.saveMenus(role!.id, [...checked]),
    onSuccess: () => {
      toast.success("菜单权限已保存")
      onSaved()
      queryClient.invalidateQueries({ queryKey: ["role-menus", role?.id] })
      onClose()
    },
    onError: (e: Error) => toast.error(e.message || "保存失败"),
  })

  const appTrees = React.useMemo(() => {
    const map = new Map<string, TreeCheckNode[]>()
    const data = authQuery.data
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
      // 范围 = 组织已授权菜单(authorized=true 的才可选)
      const flat = (data.menus[app.id] ?? []).filter((m) => m.authorized)
      map.set(app.id, convert(buildTree(flat)))
    }
    return map
  }, [authQuery.data])

  if (!role) return null

  const data = authQuery.data
  const selectedApp = data?.applications.find((a) => a.id === selectedAppId) ?? null
  const dirty = checked.size !== initial.size || [...checked].some((id) => !initial.has(id))

  return (
    <Dialog open={!!role} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>菜单权限 · {role.name}</DialogTitle>
          <DialogDescription>
            可分配范围为「{role.organizationName}」的组织授权;保存后立即对拥有该角色的用户生效
          </DialogDescription>
        </DialogHeader>

        {authQuery.isLoading || roleMenusQuery.isLoading ? (
          <div className="grid gap-4 sm:grid-cols-[240px_1fr]">
            <div className="space-y-2">
              {Array.from({ length: 3 }).map((_, i) => (
                <div key={i} className="h-14 animate-pulse rounded-lg bg-muted" />
              ))}
            </div>
            <div className="space-y-2">
              {Array.from({ length: 6 }).map((_, i) => (
                <div key={i} className="h-8 animate-pulse rounded-md bg-muted" style={{ marginLeft: `${(i % 3) * 20}px` }} />
              ))}
            </div>
          </div>
        ) : authQuery.isError ? (
          <ErrorState
            message={authQuery.error instanceof Error ? authQuery.error.message : undefined}
            onRetry={() => authQuery.refetch()}
          />
        ) : !data || data.applications.length === 0 ? (
          <EmptyState
            className="py-10"
            title="该组织暂无已授权应用"
            description="请先在「授权管理」为该组织开启应用授权"
          />
        ) : (
          <div className="grid gap-4 sm:grid-cols-[240px_1fr]">
            {/* 左栏 应用 */}
            <aside className="rounded-xl border">
              <p className="border-b px-3 py-2.5 text-xs font-medium uppercase tracking-wider text-muted-foreground">
                已授权应用
              </p>
              <ScrollArea className="h-72">
                <div className="space-y-1 p-2">
                  {data.applications.map((app) => {
                    const active = app.id === selectedAppId
                    return (
                      <button
                        key={app.id}
                        type="button"
                        onClick={() => setSelectedAppId(app.id)}
                        className={cn(
                          "flex w-full items-center gap-2.5 rounded-lg border p-2.5 text-left transition-all",
                          active ? "border-primary/50 bg-primary/10" : "border-transparent hover:bg-muted/60"
                        )}
                      >
                        <AppIcon name={app.icon} className="size-4.5 shrink-0 text-primary" />
                        <span className="min-w-0 flex-1">
                          <span className={cn("block truncate text-sm font-medium", active && "text-primary")}>{app.name}</span>
                          <span className="block text-[11px] text-muted-foreground">
                            已授权 {app.authorizedMenuCount} 菜单
                          </span>
                        </span>
                      </button>
                    )
                  })}
                </div>
              </ScrollArea>
            </aside>

            {/* 右栏 菜单树 */}
            <section className="flex flex-col rounded-xl border">
              <div className="flex flex-col gap-2 border-b px-3 py-2.5 sm:flex-row sm:items-center sm:justify-between">
                <p className="text-sm font-semibold">{selectedApp?.name ?? "选择应用"}</p>
                {selectedAppId && (
                  <TreeCheckToolbar
                    nodes={appTrees.get(selectedAppId) ?? []}
                    checked={checked}
                    onSet={setChecked}
                  />
                )}
              </div>
              <ScrollArea className="h-72">
                <div className="p-2">
                  {(appTrees.get(selectedAppId ?? "") ?? []).length === 0 ? (
                    <EmptyState className="py-10" title="该应用无可分配菜单" description="组织授权范围为空" />
                  ) : (
                    <TreeCheck
                      nodes={appTrees.get(selectedAppId!) ?? []}
                      checked={checked}
                      onToggle={(ids, v) =>
                        setChecked((prev) => {
                          const next = new Set(prev)
                          for (const id of ids) {
                            if (v) next.add(id)
                            else next.delete(id)
                          }
                          return next
                        })
                      }
                    />
                  )}
                </div>
              </ScrollArea>
            </section>
          </div>
        )}
        <DialogFooter className="gap-2 sm:gap-0">
          <span className="mr-auto text-xs text-muted-foreground">已选 {checked.size} 个菜单</span>
          <Button variant="outline" onClick={onClose} disabled={saveMutation.isPending}>
            取消
          </Button>
          <Button disabled={saveMutation.isPending || !dirty} onClick={() => saveMutation.mutate()}>
            {saveMutation.isPending && <Loader2 className="size-4 animate-spin" />}
            保存权限
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/* ============ 成员弹窗 ============ */

function RoleUsersDialog({
  role,
  onClose,
  onSaved,
}: {
  role: RoleRecord | null
  onClose: () => void
  onSaved: () => void
}) {
  const queryClient = useQueryClient()
  const [checked, setChecked] = React.useState<Set<string>>(new Set())
  const [keyword, setKeyword] = React.useState("")

  const roleUsersQuery = useQuery({
    queryKey: ["role-users", role?.id],
    queryFn: async () => asArray<{ userId: string; username: string; realName: string }>(
      await rolesApi.users(role!.id)
    ),
    enabled: !!role,
    staleTime: 30_000,
    retry: 1,
  })

  const usersQuery = useQuery({
    queryKey: ["users", "role-org", role?.organizationId, keyword],
    queryFn: async () =>
      asPage<UserRecord>(await usersApi.list({ organizationId: role!.organizationId, keyword, page: 1, size: 200 })).records,
    enabled: !!role,
    staleTime: 30_000,
    retry: 1,
  })

  React.useEffect(() => {
    if (roleUsersQuery.data) setChecked(new Set(roleUsersQuery.data.map((u) => u.userId)))
  }, [roleUsersQuery.data])

  const saveMutation = useMutation({
    mutationFn: () => rolesApi.saveUsers(role!.id, [...checked]),
    onSuccess: () => {
      toast.success("角色成员已保存")
      onSaved()
      queryClient.invalidateQueries({ queryKey: ["role-users", role?.id] })
      onClose()
    },
    onError: (e: Error) => toast.error(e.message || "保存失败"),
  })

  if (!role) return null

  const users = (usersQuery.data ?? []).filter(
    (u) => !keyword || u.username.toLowerCase().includes(keyword.toLowerCase()) || (u.realName ?? "").includes(keyword)
  )

  return (
    <Dialog open={!!role} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>角色成员 · {role.name}</DialogTitle>
          <DialogDescription>勾选「{role.organizationName}」下拥有该角色的用户</DialogDescription>
        </DialogHeader>

        <div className="relative">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={keyword}
            onChange={(e) => setKeyword(e.target.value)}
            placeholder="搜索用户"
            className="pl-9"
          />
        </div>

        {roleUsersQuery.isLoading || usersQuery.isLoading ? (
          <div className="space-y-2">
            {Array.from({ length: 5 }).map((_, i) => (
              <div key={i} className="h-10 animate-pulse rounded-lg bg-muted" />
            ))}
          </div>
        ) : roleUsersQuery.isError ? (
          <ErrorState
            message={roleUsersQuery.error instanceof Error ? roleUsersQuery.error.message : undefined}
            onRetry={() => roleUsersQuery.refetch()}
          />
        ) : users.length === 0 ? (
          <EmptyState className="py-8" title="该组织暂无用户" description="请先在「用户管理」中创建用户" />
        ) : (
          <ScrollArea className="h-72 pr-3">
            <div className="space-y-1">
              {users.map((u) => (
                <label
                  key={u.id}
                  className="flex cursor-pointer items-center gap-3 rounded-lg border px-3 py-2 transition-colors hover:bg-muted/50"
                >
                  <Checkbox
                    checked={checked.has(u.id)}
                    onCheckedChange={(v) =>
                      setChecked((prev) => {
                        const next = new Set(prev)
                        if (v) next.add(u.id)
                        else next.delete(u.id)
                        return next
                      })
                    }
                    aria-label={`用户 ${u.realName || u.username}`}
                  />
                  <span className="min-w-0 flex-1 truncate text-sm font-medium">{u.realName || u.username}</span>
                  <code className="shrink-0 font-mono text-xs text-muted-foreground">@{u.username}</code>
                </label>
              ))}
            </div>
          </ScrollArea>
        )}
        <DialogFooter>
          <span className="mr-auto text-xs text-muted-foreground">已选 {checked.size} 位成员</span>
          <Button variant="outline" onClick={onClose} disabled={saveMutation.isPending}>
            取消
          </Button>
          <Button disabled={saveMutation.isPending} onClick={() => saveMutation.mutate()}>
            {saveMutation.isPending && <Loader2 className="size-4 animate-spin" />}
            保存成员
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
