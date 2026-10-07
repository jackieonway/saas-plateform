import * as React from "react"
import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query"
import { useForm } from "react-hook-form"
import { z } from "zod"
import { zResolver } from "@/lib/form"
import {
  Check,
  Copy,
  Eye,
  KeyRound,
  Loader2,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Shield,
  Trash2,
  UserCheck,
  UserPlus,
  UserX,
  Users,
  UsersRound,
  X,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Avatar, AvatarFallback } from "@/components/ui/avatar"
import { Badge } from "@/components/ui/badge"
import { Checkbox } from "@/components/ui/checkbox"
import { Switch } from "@/components/ui/switch"
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
import { Separator } from "@/components/ui/separator"
import { PageHeader } from "@/components/layout/PageHeader"
import { StatusBadge } from "@/components/StatusBadge"
import { ConfirmDialog } from "@/components/ConfirmDialog"
import { DataPagination } from "@/components/DataPagination"
import { CopyButton } from "@/components/CopyButton"
import { EntityTreePicker, flattenPicker } from "@/components/EntityTreePicker"
import { UserDetailDrawer } from "@/components/UserDetailDrawer"
import { TableSkeleton, EmptyState, ErrorState } from "@/components/states"
import { TableDensityToggle } from "@/components/TableDensityToggle"
import { ColumnVisibilityDropdown } from "@/components/ColumnVisibilityDropdown"
import { useSyncedKeyword } from "@/hooks/use-synced-keyword"
import { useTableDensity, DENSITY_BODY_CLASS } from "@/hooks/use-table-density"
import { useColumnVisibility, type ColumnDef } from "@/hooks/use-column-visibility"
import { useAuthStore } from "@/stores/auth"
import { usersApi } from "@/api/users"
import { rolesApi } from "@/api/roles"
import { tenantsApi } from "@/api/tenants"
import { organizationsApi } from "@/api/organizations"
import { departmentsApi } from "@/api/departments"
import { asArray, asPage } from "@/lib/api"
import { formatDateTime, formatRelative, generatePassword, useDebounced, GENDER_LABELS } from "@/lib/format"
import { toast } from "sonner"
import type { DeptNode, OrgNode, RoleRecord, TenantRecord, UserRecord } from "@/api/types"

const userSchema = z.object({
  username: z
    .string()
    .min(3, "用户名至少 3 位")
    .max(32)
    .regex(/^[a-z0-9_-]+$/i, "仅支持字母、数字、下划线与中划线"),
  password: z.string().min(12, "密码至少 12 位").max(64),
  realName: z.string().min(1, "请输入姓名").max(64),
  email: z.string().email("邮箱格式不正确").optional().or(z.literal("")),
  phone: z.string().regex(/^1\d{10}$/, "手机号格式不正确").optional().or(z.literal("")),
  gender: z.enum(["MALE", "FEMALE", "UNKNOWN"]),
})
type UserForm = z.infer<typeof userSchema>

/** 用户表列定义:用户/姓名/操作常显,其余列可隐藏(任务书中的「部门/创建时间」列在当前表不存在,以实际列适配) */
const USER_COLUMNS: ColumnDef[] = [
  { key: "username", label: "用户", required: true },
  { key: "realName", label: "姓名", required: true },
  { key: "contact", label: "联系方式" },
  { key: "status", label: "状态" },
  { key: "org", label: "所属组织" },
  { key: "roles", label: "角色" },
  { key: "lastLogin", label: "最后登录" },
  { key: "actions", label: "操作", required: true },
]

export default function UsersPage() {
  const queryClient = useQueryClient()
  const [page, setPage] = React.useState(1)
  const [size, setSize] = React.useState(10)
  // 关键字与 URL ?q= 双向同步(命令面板实体跳转预填)
  const [keyword, setKeyword] = useSyncedKeyword()
  const [orgFilter, setOrgFilter] = React.useState<string | null>(null)
  const debounced = useDebounced(keyword, 300)

  const tenantsQuery = useQuery({
    queryKey: ["tenants", "options"],
    queryFn: async () => asPage<TenantRecord>(await tenantsApi.list({ page: 1, size: 100 })).records,
    staleTime: 60_000,
    retry: 1,
  })
  const tenants = tenantsQuery.data ?? []

  const orgTreesQueries = useQueries({
    queries: tenants.map((t) => ({
      queryKey: ["org-tree", t.id],
      queryFn: async () => asArray<OrgNode>(await organizationsApi.tree(t.id)),
      staleTime: 60_000,
      retry: 1,
    })),
  })
  const allOrgTree = React.useMemo(() => {
    const roots: { id: string; name: string; code?: string; children?: OrgNode[] }[] = []
    orgTreesQueries.forEach((q, i) => {
      if (q.data) {
        for (const org of q.data) roots.push({ ...org, name: `${tenants[i]?.name ?? ""} / ${org.name}` })
      }
    })
    return roots
  }, [orgTreesQueries, tenants])

  const listQuery = useQuery({
    queryKey: ["users", page, size, debounced, orgFilter],
    queryFn: async () =>
      asPage<UserRecord>(
        await usersApi.list({
          page,
          size,
          keyword: debounced,
          organizationId: orgFilter ?? undefined,
        })
      ),
    staleTime: 30_000,
    retry: 1,
    placeholderData: (prev) => prev,
  })
  React.useEffect(() => {
    setPage(1)
  }, [debounced, orgFilter, size])

  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: ["users"] })
  }

  /* ---- 新建 / 编辑 ---- */
  const [dialogOpen, setDialogOpen] = React.useState(false)
  const [editing, setEditing] = React.useState<UserRecord | null>(null)
  const form = useForm<UserForm>({
    resolver: zResolver(userSchema),
    defaultValues: { username: "", password: "", realName: "", email: "", phone: "", gender: "UNKNOWN" },
  })

  const openCreate = () => {
    setEditing(null)
    form.reset({ username: "", password: generatePassword(), realName: "", email: "", phone: "", gender: "UNKNOWN" })
    setDialogOpen(true)
  }
  const openEdit = (u: UserRecord) => {
    setEditing(u)
    form.reset({
      username: u.username,
      password: "____________",
      realName: u.realName ?? "",
      email: u.email ?? "",
      phone: u.phone ?? "",
      gender: (u.gender as "MALE" | "FEMALE" | "UNKNOWN") ?? "UNKNOWN",
    })
    setDialogOpen(true)
  }

  const saveMutation = useMutation({
    mutationFn: async (v: UserForm) => {
      if (editing) {
        await usersApi.update(editing.id, {
          realName: v.realName,
          email: v.email || undefined,
          phone: v.phone || undefined,
          gender: v.gender,
        })
      } else {
        await usersApi.create({
          username: v.username,
          password: v.password,
          realName: v.realName,
          email: v.email || undefined,
          phone: v.phone || undefined,
          gender: v.gender,
        })
      }
    },
    onSuccess: () => {
      toast.success(editing ? "用户已更新" : "用户创建成功")
      setDialogOpen(false)
      invalidate()
    },
    onError: (e: Error) => toast.error(e.message || "保存失败"),
  })

  /* ---- 删除 / 状态 ---- */
  const [deleting, setDeleting] = React.useState<UserRecord | null>(null)
  const deleteMutation = useMutation({
    mutationFn: (id: string) => usersApi.remove(id),
    onSuccess: () => {
      toast.success("用户已删除")
      setDeleting(null)
      invalidate()
    },
    onError: (e: Error) => toast.error(e.message || "删除失败"),
  })
  const statusMutation = useMutation({
    mutationFn: ({ id, next }: { id: string; next: string }) => usersApi.setStatus(id, next),
    onSuccess: () => {
      toast.success("状态已更新")
      invalidate()
    },
    onError: (e: Error) => toast.error(e.message || "状态更新失败"),
  })

  /* ---- 批量启用/禁用 ---- */
  const currentUserId = useAuthStore((s) => s.profile?.user.id)
  /** 当前页选中的用户 id(翻页/切筛选/搜索自动清空, 避免跨页误操作) */
  const [selected, setSelected] = React.useState<Set<string>>(new Set())
  /** 待确认的批量动作(ConfirmDialog 打开态) */
  const [batchConfirm, setBatchConfirm] = React.useState<"ENABLED" | "DISABLED" | null>(null)
  const batchStatusMutation = useMutation({
    mutationFn: ({ ids, status }: { ids: string[]; status: "ENABLED" | "DISABLED" }) =>
      usersApi.batchStatus(ids, status),
    onSuccess: (d) => {
      // 成功 N、失败 M、跳过 K —— 任一非零都在 toast 体现
      const parts = [`成功 ${d.succeeded.length}`]
      if (d.failed.length > 0) parts.push(`失败 ${d.failed.length}`)
      if (d.skipped.length > 0) parts.push(`跳过 ${d.skipped.length}`)
      if (d.failed.length > 0 || d.skipped.length > 0) toast.warning(parts.join("，"))
      else toast.success(parts.join("，"))
      setSelected(new Set())
      setBatchConfirm(null)
      invalidate()
    },
    onError: (e) => toast.error(e instanceof Error ? e.message : "批量操作失败"),
  })

  /* ---- 重置密码 ---- */
  const [resetting, setResetting] = React.useState<UserRecord | null>(null)
  const [newPassword, setNewPassword] = React.useState<string | null>(null)
  const resetMutation = useMutation({
    mutationFn: ({ id, pwd }: { id: string; pwd: string }) => usersApi.resetPassword(id, pwd),
    onSuccess: () => {
      toast.success("密码已重置,请将新密码告知用户")
      setNewPassword((p) => p) // 保持在展示态
    },
    onError: (e: Error) => {
      toast.error(e.message || "重置失败")
      setNewPassword(null)
    },
  })
  const openReset = (u: UserRecord) => {
    const pwd = generatePassword()
    setResetting(u)
    setNewPassword(pwd)
    resetMutation.mutate({ id: u.id, pwd })
  }

  /* ---- 成员关系 ---- */
  const [membershipUser, setMembershipUser] = React.useState<UserRecord | null>(null)

  /* ---- 分配角色 ---- */
  const [rolesUser, setRolesUser] = React.useState<UserRecord | null>(null)

  /* ---- 详情抽屉 ---- */
  const [detailUser, setDetailUser] = React.useState<UserRecord | null>(null)

  /* ---- 表格密度 ---- */
  const { density } = useTableDensity()

  const records = listQuery.data?.records ?? []
  const total = listQuery.data?.total ?? 0

  /* 翻页 / 切筛选 / 搜索时清空选择(对齐通知中心语义) */
  React.useEffect(() => {
    setSelected(new Set())
  }, [page, debounced, orgFilter, size])
  /* 记录集变化时把选择裁剪到当前页(被删除等已不存在的行自动移出选择) */
  React.useEffect(() => {
    setSelected((prev) => {
      if (prev.size === 0) return prev
      const ids = new Set(records.map((u) => u.id))
      const next = new Set([...prev].filter((id) => ids.has(id)))
      return next.size === prev.size ? prev : next
    })
  }, [listQuery.data])

  /* 可选行 = 当前页排除当前登录用户自身(不能禁用当前登录用户, 服务端同步兜底) */
  const selectableIds = records.filter((u) => u.id !== currentUserId).map((u) => u.id)
  const selectedOnPage = selectableIds.filter((id) => selected.has(id))
  const allSelected = selectableIds.length > 0 && selectedOnPage.length === selectableIds.length
  const someSelected = selectedOnPage.length > 0 && !allSelected
  const toggleOne = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  const selectAllOnPage = () => setSelected(new Set(selectableIds))
  const clearSelection = () => setSelected(new Set())

  /* ---- 列显隐 ---- */
  const { visibleSet: userCols, isVisible, setVisible } = useColumnVisibility("users", USER_COLUMNS)
  return (
    <div>
      <PageHeader
        title="用户管理"
        description="平台用户账号、成员关系与角色分配"
        icon={<Users className="size-5" />}
        actions={
          <Button onClick={openCreate}>
            <UserPlus className="size-4" />
            新建用户
          </Button>
        }
      />

      {/* 工具栏 */}
      <div className="mb-4 flex flex-col gap-2 sm:flex-row sm:items-center">
        <div className="relative flex-1 sm:max-w-xs">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={keyword}
            onChange={(e) => setKeyword(e.target.value)}
            placeholder="搜索用户名 / 姓名 / 邮箱"
            aria-label="搜索用户"
            className="pl-9"
          />
        </div>
        <EntityTreePicker
          nodes={allOrgTree}
          value={orgFilter}
          onSelect={setOrgFilter}
          placeholder="按组织筛选"
          className="w-full sm:w-56"
          icon={<Shield className="size-4 shrink-0 text-muted-foreground" />}
        />
        <div className="flex items-center gap-2 sm:ml-auto">
          <span className="text-xs text-muted-foreground">
            {listQuery.isFetching && <Loader2 className="mr-1 inline size-3 animate-spin" />}
            共 {total} 位用户
          </span>
          <ColumnVisibilityDropdown
            table="users"
            columns={USER_COLUMNS}
            visible={[...userCols]}
            onChange={setVisible}
          />
          <TableDensityToggle />
        </div>
      </div>

      {/* 批量操作栏:选中 > 0 时浮现, sticky 于表格上方(样式与通知中心批量栏同款) */}
      {selected.size > 0 && (
        <div
          role="toolbar"
          aria-label="批量操作"
          className="sticky top-0 z-20 mb-3 flex flex-wrap items-center gap-2 rounded-xl border bg-muted/70 px-3 py-2 shadow-sm backdrop-blur animate-in fade-in slide-in-from-top-2 duration-200"
        >
          <span className="inline-flex items-center gap-2 text-sm font-medium">
            <span className="flex size-5 items-center justify-center rounded-md bg-primary text-primary-foreground">
              <Check className="size-3" />
            </span>
            已选 <span className="tabular-nums">{selected.size}</span> 项
          </span>
          <span className="h-5 w-px bg-border" aria-hidden />
          <Button
            variant="outline"
            size="sm"
            onClick={() => setBatchConfirm("ENABLED")}
            disabled={batchStatusMutation.isPending}
          >
            {batchStatusMutation.isPending && batchStatusMutation.variables?.status === "ENABLED" ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <UserCheck className="size-4" />
            )}
            批量启用
          </Button>
          <Button
            variant="destructive"
            size="sm"
            onClick={() => setBatchConfirm("DISABLED")}
            disabled={batchStatusMutation.isPending}
          >
            {batchStatusMutation.isPending && batchStatusMutation.variables?.status === "DISABLED" ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <UserX className="size-4" />
            )}
            批量禁用
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="ml-auto size-11 text-muted-foreground hover:text-foreground"
            onClick={clearSelection}
            aria-label="清空选择"
            title="清空选择"
          >
            <X className="size-4" />
          </Button>
        </div>
      )}

      {/* 表格 */}
      <div className="rounded-xl border bg-card">
        {listQuery.isLoading ? (
          <div className="p-2">
            <TableSkeleton cols={9} />
          </div>
        ) : listQuery.isError ? (
          <ErrorState
            className="m-2 border-0 bg-transparent"
            message={listQuery.error instanceof Error ? listQuery.error.message : undefined}
            onRetry={() => listQuery.refetch()}
          />
        ) : records.length === 0 ? (
          <EmptyState
            title={keyword || orgFilter ? "没有匹配的用户" : "还没有用户"}
            description={keyword || orgFilter ? "调整筛选条件试试" : "创建第一个用户账号"}
            action={
              !keyword && !orgFilter && (
                <Button size="sm" onClick={openCreate}>
                  <UserPlus className="size-4" />
                  新建用户
                </Button>
              )
            }
          />
        ) : (
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className="w-10 pl-5">
                  <Checkbox
                    checked={allSelected ? true : someSelected ? "indeterminate" : false}
                    onCheckedChange={() => (allSelected ? clearSelection() : selectAllOnPage())}
                    aria-label="全选本页"
                  />
                </TableHead>
                <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">用户</TableHead>
                <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">姓名</TableHead>
                {isVisible("contact") && (
                  <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">联系方式</TableHead>
                )}
                {isVisible("status") && (
                  <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">状态</TableHead>
                )}
                {isVisible("org") && (
                  <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">所属组织</TableHead>
                )}
                {isVisible("roles") && (
                  <TableHead className="text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">角色</TableHead>
                )}
                {isVisible("lastLogin") && (
                  <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">最后登录</TableHead>
                )}
                <TableHead className="pr-5 text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody className={DENSITY_BODY_CLASS[density]}>
              {records.map((u) => (
                <TableRow key={u.id} data-state={selected.has(u.id) ? "selected" : undefined}>
                  <TableCell className="pl-5">
                    {u.id === currentUserId ? (
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <span tabIndex={-1} className="inline-flex">
                            <Checkbox disabled aria-label="不能禁用当前登录用户" />
                          </span>
                        </TooltipTrigger>
                        <TooltipContent>不能禁用当前登录用户</TooltipContent>
                      </Tooltip>
                    ) : (
                      <Checkbox
                        checked={selected.has(u.id)}
                        onCheckedChange={() => toggleOne(u.id)}
                        aria-label={`选择用户 ${u.realName || u.username}`}
                      />
                    )}
                  </TableCell>
                  <TableCell>
                    <div className="flex items-center gap-2.5">
                      <Avatar className="size-8 border">
                        <AvatarFallback className="bg-primary/10 text-xs font-semibold text-primary">
                          {(u.realName || u.username).slice(0, 1).toUpperCase()}
                        </AvatarFallback>
                      </Avatar>
                      <div className="min-w-0">
                        <button
                          type="button"
                          className="truncate text-left text-sm font-medium hover:text-primary hover:underline"
                          onClick={() => setDetailUser(u)}
                          title="查看用户详情"
                        >
                          {u.realName || u.username}
                        </button>
                        <p className="truncate font-mono text-xs text-muted-foreground">@{u.username}</p>
                      </div>
                    </div>
                  </TableCell>
                  <TableCell>{u.realName || "—"}</TableCell>
                  {isVisible("contact") && (
                    <TableCell>
                      <div className="max-w-[13rem] space-y-0.5 text-xs">
                        <p className="truncate text-muted-foreground" title={u.email || undefined}>
                          {u.email || "—"}
                        </p>
                        <p className="truncate text-muted-foreground" title={u.phone || undefined}>
                          {u.phone || "—"}
                        </p>
                      </div>
                    </TableCell>
                  )}
                  {isVisible("status") && (
                    <TableCell>
                      <div className="flex items-center gap-2">
                        <StatusBadge status={u.status} />
                        <Switch
                          checked={u.status === "ENABLED"}
                          onCheckedChange={(v) => statusMutation.mutate({ id: u.id, next: v ? "ENABLED" : "DISABLED" })}
                          aria-label={`切换用户 ${u.username} 状态`}
                          disabled={statusMutation.isPending}
                        />
                      </div>
                    </TableCell>
                  )}
                  {isVisible("org") && (
                    <TableCell>
                      <div className="flex max-w-[16rem] flex-wrap items-center gap-1">
                        {u.memberships.length === 0 && <span className="text-xs text-muted-foreground">—</span>}
                        {u.memberships.slice(0, 2).map((m, i) => (
                          <Badge key={`${m.organizationId}-${i}`} variant="secondary" className="max-w-[8rem] truncate text-[11px] font-normal">
                            {m.organizationName}
                          </Badge>
                        ))}
                        {u.memberships.length > 2 && (
                          <Tooltip>
                            <TooltipTrigger asChild>
                              <Badge variant="outline" className="cursor-default text-[11px]">
                                +{u.memberships.length - 2}
                              </Badge>
                            </TooltipTrigger>
                            <TooltipContent className="max-w-56">
                              <p className="mb-1 text-xs font-medium">全部组织</p>
                              {u.memberships.slice(2).map((m, i) => (
                                <p key={i} className="text-xs">
                                  {m.organizationName}
                                  {m.departmentName ? ` · ${m.departmentName}` : ""}
                                </p>
                              ))}
                            </TooltipContent>
                          </Tooltip>
                        )}
                      </div>
                    </TableCell>
                  )}
                  {isVisible("roles") && (
                    <TableCell className="text-right tabular-nums">{u.roleCount}</TableCell>
                  )}
                  {isVisible("lastLogin") && (
                    <TableCell className="text-xs text-muted-foreground" title={formatDateTime(u.lastLoginAt)}>
                      {u.lastLoginAt ? formatRelative(u.lastLoginAt) : "从未登录"}
                    </TableCell>
                  )}
                  <TableCell className="pr-5">
                    <div className="flex items-center justify-end gap-0.5">
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="size-8"
                            aria-label="用户详情"
                            onClick={() => setDetailUser(u)}
                          >
                            <Eye className="size-4" />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>详情</TooltipContent>
                      </Tooltip>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="size-8"
                            aria-label="成员关系"
                            onClick={() => setMembershipUser(u)}
                          >
                            <UsersRound className="size-4" />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>成员关系</TooltipContent>
                      </Tooltip>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button variant="ghost" size="icon" className="size-8" aria-label="分配角色" onClick={() => setRolesUser(u)}>
                            <Shield className="size-4" />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>分配角色</TooltipContent>
                      </Tooltip>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button variant="ghost" size="icon" className="size-8" aria-label="重置密码" onClick={() => openReset(u)}>
                            <KeyRound className="size-4" />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>重置密码</TooltipContent>
                      </Tooltip>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button variant="ghost" size="icon" className="size-8" aria-label="编辑" onClick={() => openEdit(u)}>
                            <Pencil className="size-4" />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>编辑</TooltipContent>
                      </Tooltip>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="size-8 text-destructive hover:text-destructive"
                            aria-label="删除"
                            onClick={() => setDeleting(u)}
                          >
                            <Trash2 className="size-4" />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>删除</TooltipContent>
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
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{editing ? `编辑用户「${editing.realName || editing.username}」` : "新建用户"}</DialogTitle>
            <DialogDescription>{editing ? "用户名创建后不可修改" : "创建后默认不属于任何组织,可在列表中配置成员关系"}</DialogDescription>
          </DialogHeader>
          <Form {...form}>
            <form id="user-form" onSubmit={form.handleSubmit((v) => saveMutation.mutate(v))} className="space-y-4">
              <div className="grid gap-4 sm:grid-cols-2">
                <FormField
                  control={form.control}
                  name="username"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        用户名 <span className="text-destructive">*</span>
                      </FormLabel>
                      <FormControl>
                        <Input placeholder="如 zhangsan" className="font-mono" {...field} disabled={!!editing} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="realName"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        姓名 <span className="text-destructive">*</span>
                      </FormLabel>
                      <FormControl>
                        <Input placeholder="如 张三" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>

              {!editing && (
                <FormField
                  control={form.control}
                  name="password"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        初始密码 <span className="text-destructive">*</span>
                      </FormLabel>
                      <div className="flex gap-2">
                        <FormControl>
                          <Input className="font-mono" {...field} autoComplete="new-password" />
                        </FormControl>
                        <Button
                          type="button"
                          variant="outline"
                          size="icon"
                          aria-label="生成强密码"
                          title="生成强密码"
                          onClick={() => form.setValue("password", generatePassword(), { shouldValidate: true })}
                        >
                          <RefreshCw className="size-4" />
                        </Button>
                        <CopyButton text={field.value} size="icon" />
                      </div>
                      <FormDescription>已自动生成强密码,可复制后告知用户</FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              )}

              <div className="grid gap-4 sm:grid-cols-2">
                <FormField
                  control={form.control}
                  name="email"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>邮箱</FormLabel>
                      <FormControl>
                        <Input type="email" placeholder="选填" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="phone"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>手机号</FormLabel>
                      <FormControl>
                        <Input placeholder="选填" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>

              <FormField
                control={form.control}
                name="gender"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>性别</FormLabel>
                    <Select onValueChange={field.onChange} value={field.value}>
                      <FormControl>
                        <SelectTrigger className="w-full sm:w-40">
                          <SelectValue />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        <SelectItem value="MALE">男</SelectItem>
                        <SelectItem value="FEMALE">女</SelectItem>
                        <SelectItem value="UNKNOWN">未知</SelectItem>
                      </SelectContent>
                    </Select>
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
            <Button type="submit" form="user-form" disabled={saveMutation.isPending}>
              {saveMutation.isPending && <Loader2 className="size-4 animate-spin" />}
              {editing ? "保存更改" : "创建"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* 重置密码结果 */}
      <Dialog open={!!resetting} onOpenChange={(o) => !o && (setResetting(null), setNewPassword(null))}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>重置密码</DialogTitle>
            <DialogDescription>
              {resetting ? `已为「${resetting.realName || resetting.username}」生成一次性新密码` : ""}
            </DialogDescription>
          </DialogHeader>
          {resetMutation.isPending || !newPassword ? (
            <div className="flex items-center justify-center gap-2 py-8 text-sm text-muted-foreground">
              <Loader2 className="size-4 animate-spin" />
              正在重置...
            </div>
          ) : (
            <div className="space-y-3">
              <div className="rounded-lg border bg-muted/50 p-3">
                <code className="block break-all font-mono text-lg tracking-wider">{newPassword}</code>
              </div>
              <div className="flex items-center justify-between">
                <CopyButton text={newPassword} label="复制新密码" />
                <Button variant="outline" size="sm" onClick={() => openReset(resetting!)}>
                  <RefreshCw className="size-3.5" />
                  重新生成
                </Button>
              </div>
              <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
                <Copy className="mt-0.5 size-3 shrink-0" />
                请立即复制并妥善转达给用户,关闭后不再展示。
              </p>
            </div>
          )}
          <DialogFooter>
            <Button onClick={() => (setResetting(null), setNewPassword(null))}>完成</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* 成员关系 */}
      <MembershipDialog user={membershipUser} onClose={() => setMembershipUser(null)} allOrgTree={allOrgTree} onSaved={invalidate} />

      {/* 分配角色 */}
      <RoleAssignDialog user={rolesUser} onClose={() => setRolesUser(null)} onSaved={invalidate} />

      {/* 用户详情抽屉 */}
      <UserDetailDrawer
        user={detailUser}
        onOpenChange={(o) => !o && setDetailUser(null)}
        onEdit={(u) => {
          setDetailUser(null)
          openEdit(u)
        }}
        onMemberships={(u) => {
          setDetailUser(null)
          setMembershipUser(u)
        }}
        onRoles={(u) => {
          setDetailUser(null)
          setRolesUser(u)
        }}
        onResetPassword={(u) => {
          setDetailUser(null)
          openReset(u)
        }}
      />

      <ConfirmDialog
        open={!!deleting}
        onOpenChange={(o) => !o && setDeleting(null)}
        title={`删除用户「${(deleting?.realName || deleting?.username) ?? ""}」?`}
        description="将删除该账号及其成员关系与角色分配,此操作不可撤销。"
        loading={deleteMutation.isPending}
        onConfirm={() => deleting && deleteMutation.mutate(deleting.id)}
      />

      {/* 批量启用/禁用二次确认(文案随动作切换, 禁用强调不可登录但可随时重新启用) */}
      <ConfirmDialog
        open={!!batchConfirm}
        onOpenChange={(o) => !o && setBatchConfirm(null)}
        title={
          batchConfirm === "DISABLED"
            ? `批量禁用选中的 ${selected.size} 个用户?`
            : `批量启用选中的 ${selected.size} 个用户?`
        }
        description={
          batchConfirm === "DISABLED" ? (
            <>
              将禁用当前选中的 <b className="tabular-nums">{selected.size}</b>{" "}
              个用户,禁用后这些用户将无法登录,可随时重新启用。
            </>
          ) : (
            <>
              将启用当前选中的 <b className="tabular-nums">{selected.size}</b>{" "}
              个用户,启用后即可正常登录。
            </>
          )
        }
        confirmText={batchConfirm === "DISABLED" ? "批量禁用" : "批量启用"}
        loading={batchStatusMutation.isPending}
        onConfirm={() =>
          batchConfirm && batchStatusMutation.mutate({ ids: [...selected], status: batchConfirm })
        }
      />
    </div>
  )
}

/* ============ 成员关系弹窗 ============ */

interface MembershipRow {
  organizationId: string
  departmentId: string | null
  position: string
}

function MembershipDialog({
  user,
  onClose,
  allOrgTree,
  onSaved,
}: {
  user: UserRecord | null
  onClose: () => void
  allOrgTree: { id: string; name: string; code?: string; children?: OrgNode[] }[]
  onSaved: () => void
}) {
  const queryClient = useQueryClient()
  const [rows, setRows] = React.useState<MembershipRow[]>([])
  const [saving, setSaving] = React.useState(false)

  const query = useQuery({
    queryKey: ["user-memberships", user?.id],
    queryFn: async () => asArray<{ organizationId: string; tenantId: string; departmentId: string | null; position: string }>(
      await usersApi.memberships(user!.id)
    ),
    enabled: !!user,
    retry: 1,
  })

  React.useEffect(() => {
    if (query.data) {
      setRows(query.data.map((m) => ({ organizationId: m.organizationId, departmentId: m.departmentId, position: m.position ?? "" })))
    }
  }, [query.data])

  const findOrgName = (id: string | null): string => {
    if (!id) return "—"
    const flat = flattenPicker(allOrgTree as never)
    return flat.find((n) => n.id === id)?.name ?? id.slice(0, 8)
  }

  const updateRow = (i: number, patch: Partial<MembershipRow>) => {
    setRows((rs) => rs.map((r, idx) => (idx === i ? { ...r, ...patch } : r)))
  }

  const save = async () => {
    if (!user) return
    const invalid = rows.find((r) => !r.organizationId)
    if (invalid !== undefined && rows.some((r) => !r.organizationId)) {
      toast.error("存在未选择组织的行,请补全或删除")
      return
    }
    setSaving(true)
    try {
      await usersApi.saveMemberships(
        user.id,
        rows.map((r) => ({ organizationId: r.organizationId, departmentId: r.departmentId || null, position: r.position }))
      )
      toast.success("成员关系已保存")
      onSaved()
      queryClient.invalidateQueries({ queryKey: ["user-memberships", user.id] })
      onClose()
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "保存失败")
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog open={!!user} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>成员关系 · {user?.realName || user?.username}</DialogTitle>
          <DialogDescription>配置用户所属组织、部门与职位,一个用户可归属多个组织</DialogDescription>
        </DialogHeader>

        {query.isLoading ? (
          <div className="space-y-2">
            {Array.from({ length: 3 }).map((_, i) => (
              <div key={i} className="h-16 animate-pulse rounded-lg bg-muted" />
            ))}
          </div>
        ) : query.isError ? (
          <ErrorState
            message={query.error instanceof Error ? query.error.message : undefined}
            onRetry={() => query.refetch()}
          />
        ) : (
          <ScrollArea className="max-h-[55vh] pr-3">
            <div className="space-y-2">
              {rows.length === 0 && (
                <EmptyState className="py-8" title="暂无成员关系" description="点击下方按钮添加第一行" />
              )}
              {rows.map((r, i) => (
                <MembershipRowEditor
                  key={i}
                  index={i}
                  row={r}
                  allOrgTree={allOrgTree}
                  onChange={(p) => updateRow(i, p)}
                  onRemove={() => setRows((rs) => rs.filter((_, idx) => idx !== i))}
                />
              ))}
              <Button variant="outline" size="sm" className="w-full" onClick={() => setRows((rs) => [...rs, { organizationId: "", departmentId: null, position: "" }])}>
                <Plus className="size-4" />
                添加成员关系
              </Button>
            </div>
          </ScrollArea>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={saving}>
            取消
          </Button>
          <Button onClick={save} disabled={saving || query.isLoading || query.isError}>
            {saving && <Loader2 className="size-4 animate-spin" />}
            保存成员关系
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function MembershipRowEditor({
  row,
  allOrgTree,
  onChange,
  onRemove,
}: {
  index: number
  row: MembershipRow
  allOrgTree: { id: string; name: string; code?: string; children?: OrgNode[] }[]
  onChange: (patch: Partial<MembershipRow>) => void
  onRemove: () => void
}) {
  const deptQuery = useQuery({
    queryKey: ["dept-tree", row.organizationId],
    queryFn: async () => asArray<DeptNode>(await departmentsApi.tree(row.organizationId)),
    enabled: !!row.organizationId,
    staleTime: 30_000,
    retry: 1,
  })

  return (
    <div className="flex flex-col gap-2 rounded-lg border p-3 sm:flex-row sm:items-start">
      <div className="grid flex-1 gap-2 sm:grid-cols-2">
        <EntityTreePicker
          nodes={allOrgTree as never}
          value={row.organizationId || null}
          onSelect={(id) => onChange({ organizationId: id ?? "", departmentId: null })}
          placeholder="选择组织"
          allowClear={false}
          icon={<Shield className="size-4 shrink-0 text-muted-foreground" />}
        />
        <EntityTreePicker
          nodes={deptQuery.data ?? []}
          value={row.departmentId}
          onSelect={(id) => onChange({ departmentId: id })}
          placeholder={row.organizationId ? "选择部门(可空)" : "请先选组织"}
          disabled={!row.organizationId}
          icon={<Users className="size-4 shrink-0 text-muted-foreground" />}
        />
        <Input
          value={row.position}
          onChange={(e) => onChange({ position: e.target.value })}
          placeholder="职位,如:数据分析师"
          className="sm:col-span-2"
        />
      </div>
      <Button variant="ghost" size="icon" className="size-8 shrink-0 text-destructive hover:text-destructive" aria-label="删除此行" onClick={onRemove}>
        <Trash2 className="size-4" />
      </Button>
    </div>
  )
}

/* ============ 分配角色弹窗 ============ */

function RoleAssignDialog({
  user,
  onClose,
  onSaved,
}: {
  user: UserRecord | null
  onClose: () => void
  onSaved: () => void
}) {
  const queryClient = useQueryClient()
  const [checked, setChecked] = React.useState<Set<string>>(new Set())
  const [saving, setSaving] = React.useState(false)

  const userRolesQuery = useQuery({
    queryKey: ["user-roles", user?.id],
    queryFn: async () =>
      asArray<{ roleId: string; organizationId: string }>(await usersApi.roles(user!.id)),
    enabled: !!user,
    retry: 1,
  })

  const orgIds = React.useMemo(() => {
    const ids = new Set<string>()
    for (const m of user?.memberships ?? []) ids.add(m.organizationId)
    return [...ids]
  }, [user])

  const rolesQueries = useQueries({
    queries: orgIds.map((orgId) => ({
      queryKey: ["roles", "by-org", orgId],
      queryFn: async () => asPage<RoleRecord>(await rolesApi.list({ organizationId: orgId, page: 1, size: 200 })).records,
      staleTime: 30_000,
      retry: 1,
      enabled: !!user,
    })),
  })

  React.useEffect(() => {
    if (userRolesQuery.data) {
      setChecked(new Set(userRolesQuery.data.map((r) => r.roleId)))
    }
  }, [userRolesQuery.data])

  const save = async () => {
    if (!user) return
    const allRoles = rolesQueries.flatMap((q) => q.data ?? [])
    const items = allRoles
      .filter((r) => checked.has(r.id))
      .map((r) => ({ roleId: r.id, organizationId: r.organizationId }))
    setSaving(true)
    try {
      await usersApi.saveRoles(user.id, items)
      toast.success("角色分配已保存")
      onSaved()
      queryClient.invalidateQueries({ queryKey: ["user-roles", user.id] })
      onClose()
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "保存失败")
    } finally {
      setSaving(false)
    }
  }

  const orgName = (orgId: string) => user?.memberships.find((m) => m.organizationId === orgId)?.organizationName ?? orgId.slice(0, 8)

  return (
    <Dialog open={!!user} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>分配角色 · {user?.realName || user?.username}</DialogTitle>
          <DialogDescription>仅在用户所属组织范围内分配;角色可用菜单由该组织授权范围决定</DialogDescription>
        </DialogHeader>

        {userRolesQuery.isLoading ? (
          <div className="space-y-2">
            {Array.from({ length: 3 }).map((_, i) => (
              <div key={i} className="h-10 animate-pulse rounded-lg bg-muted" />
            ))}
          </div>
        ) : userRolesQuery.isError ? (
          <ErrorState
            message={userRolesQuery.error instanceof Error ? userRolesQuery.error.message : undefined}
            onRetry={() => userRolesQuery.refetch()}
          />
        ) : orgIds.length === 0 ? (
          <EmptyState className="py-8" title="该用户暂无组织归属" description="请先在「成员关系」中为其添加组织" />
        ) : (
          <ScrollArea className="max-h-[55vh] pr-3">
            <div className="space-y-4">
              {orgIds.map((orgId, oi) => {
                const roles = rolesQueries[oi]?.data ?? []
                return (
                  <div key={orgId}>
                    <p className="mb-2 flex items-center gap-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                      <Shield className="size-3.5" />
                      {orgName(orgId)}
                      <Separator className="flex-1" />
                      <span className="font-normal normal-case">{checked.size > 0 ? `${roles.filter((r) => checked.has(r.id)).length}/${roles.length}` : `${roles.length} 个角色`}</span>
                    </p>
                    {rolesQueries[oi]?.isLoading ? (
                      <div className="space-y-1.5">
                        {Array.from({ length: 2 }).map((_, i) => (
                          <div key={i} className="h-9 animate-pulse rounded-md bg-muted" />
                        ))}
                      </div>
                    ) : roles.length === 0 ? (
                      <p className="rounded-lg border border-dashed p-3 text-center text-xs text-muted-foreground">
                        该组织暂无角色,请先在「角色管理」创建
                      </p>
                    ) : (
                      <div className="space-y-1">
                        {roles.map((r) => (
                          <label
                            key={r.id}
                            className="flex cursor-pointer items-center gap-3 rounded-lg border px-3 py-2 transition-colors hover:bg-muted/50"
                          >
                            <Checkbox
                              checked={checked.has(r.id)}
                              onCheckedChange={(v) =>
                                setChecked((prev) => {
                                  const next = new Set(prev)
                                  if (v) next.add(r.id)
                                  else next.delete(r.id)
                                  return next
                                })
                              }
                              aria-label={`角色 ${r.name}`}
                            />
                            <span className="min-w-0 flex-1">
                              <span className="flex items-center gap-2">
                                <span className="truncate text-sm font-medium">{r.name}</span>
                                {r.builtIn && <Badge variant="outline" className="text-[10px]">内置</Badge>}
                              </span>
                              <span className="block truncate font-mono text-[11px] text-muted-foreground">{r.code}</span>
                            </span>
                            <span className="shrink-0 text-xs text-muted-foreground">{r.menuCount} 菜单</span>
                          </label>
                        ))}
                      </div>
                    )}
                  </div>
                )
              })}
            </div>
          </ScrollArea>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={saving}>
            取消
          </Button>
          <Button onClick={save} disabled={saving || userRolesQuery.isLoading}>
            {saving && <Loader2 className="size-4 animate-spin" />}
            保存角色分配
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
