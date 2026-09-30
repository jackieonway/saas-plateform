import * as React from "react"
import { Link } from "react-router-dom"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useForm } from "react-hook-form"
import { z } from "zod"
import { zResolver } from "@/lib/form"
import { Crown, Info, Landmark, Loader2, Pencil, Plus, Search, Settings2, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Label } from "@/components/ui/label"
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
import { Switch } from "@/components/ui/switch"
import { PageHeader } from "@/components/layout/PageHeader"
import { StatusBadge } from "@/components/StatusBadge"
import { ConfirmDialog } from "@/components/ConfirmDialog"
import { DataPagination } from "@/components/DataPagination"
import { TableSkeleton, EmptyState, ErrorState } from "@/components/states"
import { useSyncedKeyword } from "@/hooks/use-synced-keyword"
import { useAuthStore } from "@/stores/auth"
import { tenantsApi } from "@/api/tenants"
import { asPage } from "@/lib/api"
import { formatDateTime, useDebounced } from "@/lib/format"
import { ExpiryCell, isoToLocalInput } from "@/lib/expiry"
import { toast } from "sonner"
import type { TenantRecord } from "@/api/types"

const tenantSchema = z.object({
  name: z.string().min(1, "请输入租户名称").max(64, "名称过长"),
  code: z
    .string()
    .min(2, "请输入租户编码")
    .max(32)
    .regex(/^[a-zA-Z0-9-]+$/, "仅支持字母、数字与中划线"),
  description: z.string().max(255, "描述过长").optional(),
  /** 过期时间: 空=永不过期; 接受 datetime-local("2026-12-31T23:59")或合法 ISO */
  expiresAt: z
    .string()
    .refine((v) => !v || !Number.isNaN(new Date(v).getTime()), "无效的过期时间")
    .optional(),
})
type TenantForm = z.infer<typeof tenantSchema>

export default function TenantsPage() {
  const queryClient = useQueryClient()
  // 平台租户上下文: 平台租户(含超管)可管理全部租户, 其余租户仅能查看本租户只读视角
  const isPlatformContext = useAuthStore((s) => s.profile?.tenant?.isPlatform ?? false)
  const [page, setPage] = React.useState(1)
  const [size, setSize] = React.useState(10)
  // 关键字与 URL ?q= 双向同步(命令面板实体跳转预填)
  const [keyword, setKeyword] = useSyncedKeyword()
  const [status, setStatus] = React.useState("ALL")
  const debouncedKeyword = useDebounced(keyword, 300)

  const listQuery = useQuery({
    queryKey: ["tenants", page, size, debouncedKeyword, status],
    queryFn: async () =>
      asPage<TenantRecord>(
        await tenantsApi.list({ page, size, keyword: debouncedKeyword, status })
      ),
    staleTime: 30_000,
    retry: 1,
    placeholderData: (prev) => prev,
  })

  React.useEffect(() => {
    setPage(1)
  }, [debouncedKeyword, status, size])

  /* ---- 新建 / 编辑 ---- */
  const [dialogOpen, setDialogOpen] = React.useState(false)
  const [editing, setEditing] = React.useState<TenantRecord | null>(null)
  const form = useForm<TenantForm>({
    resolver: zResolver(tenantSchema),
    defaultValues: { name: "", code: "", description: "", expiresAt: "" },
  })

  const openCreate = () => {
    setEditing(null)
    form.reset({ name: "", code: "", description: "", expiresAt: "" })
    setDialogOpen(true)
  }
  const openEdit = (t: TenantRecord) => {
    setEditing(t)
    form.reset({
      name: t.name,
      code: t.code,
      description: t.description ?? "",
      expiresAt: isoToLocalInput(t.expiresAt),
    })
    setDialogOpen(true)
  }

  const saveMutation = useMutation({
    mutationFn: async (values: TenantForm) => {
      /* datetime-local("2026-12-31T23:59") → ISO; 空串 → null(清除/永久) */
      const body = {
        ...values,
        expiresAt: values.expiresAt ? new Date(values.expiresAt).toISOString() : null,
      }
      if (editing) await tenantsApi.update(editing.id, body)
      else await tenantsApi.create(body)
    },
    onSuccess: () => {
      toast.success(editing ? "租户已更新" : "租户创建成功")
      setDialogOpen(false)
      queryClient.invalidateQueries({ queryKey: ["tenants"] })
    },
    onError: (e: Error) => toast.error(e.message || "保存失败"),
  })

  /* ---- 删除 ---- */
  const [deleting, setDeleting] = React.useState<TenantRecord | null>(null)
  const deleteMutation = useMutation({
    mutationFn: (id: string) => tenantsApi.remove(id),
    onSuccess: () => {
      toast.success("租户已删除")
      setDeleting(null)
      queryClient.invalidateQueries({ queryKey: ["tenants"] })
    },
    onError: (e: Error) => toast.error(e.message || "删除失败"),
  })

  /* ---- 状态切换 ---- */
  const statusMutation = useMutation({
    mutationFn: ({ id, next }: { id: string; next: string }) => tenantsApi.setStatus(id, next),
    onSuccess: () => {
      toast.success("状态已更新")
      queryClient.invalidateQueries({ queryKey: ["tenants"] })
    },
    onError: (e: Error) => toast.error(e.message || "状态更新失败"),
  })

  const records = listQuery.data?.records ?? []
  const total = listQuery.data?.total ?? 0

  return (
    <div>
      <PageHeader
        title="租户管理"
        description="管理平台租户,租户是组织、用户与应用授权的顶层边界"
        icon={<Landmark className="size-5" />}
        actions={
          isPlatformContext ? (
            <Button onClick={openCreate}>
              <Plus className="size-4" />
              新建租户
            </Button>
          ) : undefined
        }
      />

      {/* 非平台租户只读提示条 */}
      {!isPlatformContext && (
        <div className="mb-4 flex items-start gap-2.5 rounded-lg border border-border/60 bg-muted/40 px-3.5 py-2.5 text-xs text-muted-foreground">
          <Info className="mt-0.5 size-3.5 shrink-0" />
          <span>
            当前为租户内只读视角 —— 平台租户可管理所有租户，您仅能查看自己租户的信息；组织、用户与角色等租户内数据请在对应页面管理。
          </span>
        </div>
      )}

      {/* 工具栏 */}
      <div className="mb-4 flex flex-col gap-2 sm:flex-row sm:items-center">
        <div className="relative flex-1 sm:max-w-xs">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={keyword}
            onChange={(e) => setKeyword(e.target.value)}
            placeholder="搜索租户名称 / 编码"
            aria-label="搜索租户"
            className="pl-9"
          />
        </div>
        <Select value={status} onValueChange={setStatus}>
          <SelectTrigger className="w-full sm:w-32" aria-label="状态筛选">
            <SelectValue placeholder="状态" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="ALL">全部状态</SelectItem>
            <SelectItem value="ENABLED">启用</SelectItem>
            <SelectItem value="DISABLED">停用</SelectItem>
          </SelectContent>
        </Select>
        <span className="text-xs text-muted-foreground sm:ml-auto">
          {listQuery.isFetching && <Loader2 className="mr-1 inline size-3 animate-spin" />}
          共 {total} 个租户
        </span>
      </div>

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
            title={keyword ? "没有匹配的租户" : "还没有租户"}
            description={keyword ? "换个关键词试试" : "创建第一个租户,开始构建组织与授权体系"}
            action={
              !keyword && isPlatformContext && (
                <Button size="sm" onClick={openCreate}>
                  <Plus className="size-4" />
                  新建租户
                </Button>
              )
            }
          />
        ) : (
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className="pl-5 text-xs font-medium uppercase tracking-wider text-muted-foreground">名称</TableHead>
                <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">编码</TableHead>
                <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">状态</TableHead>
                <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">有效期</TableHead>
                <TableHead className="text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">组织数</TableHead>
                <TableHead className="text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">用户数</TableHead>
                <TableHead className="text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">已授权应用</TableHead>
                <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">创建时间</TableHead>
                <TableHead className="pr-5 text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {records.map((t) => (
                <TableRow key={t.id}>
                  <TableCell className="pl-5">
                    <div className="flex items-center gap-2.5">
                      <span
                        className={
                          "flex size-8 items-center justify-center rounded-lg text-primary " +
                          (t.isPlatform ? "bg-primary/15 ring-1 ring-primary/25" : "bg-primary/10")
                        }
                      >
                        {t.isPlatform ? <Crown className="size-4" /> : <Landmark className="size-4" />}
                      </span>
                      <div className="min-w-0">
                        <div className="flex items-center gap-1.5">
                          <p className="truncate font-medium">{t.name}</p>
                          {t.isPlatform && (
                            <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-primary/10 px-1.5 py-px text-[10px] font-medium text-primary ring-1 ring-primary/25">
                              <Crown className="size-2.5" />
                              平台租户
                            </span>
                          )}
                        </div>
                        {t.description && (
                          <p className="truncate text-xs text-muted-foreground" title={t.description}>
                            {t.description}
                          </p>
                        )}
                      </div>
                    </div>
                  </TableCell>
                  <TableCell>
                    <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">{t.code}</code>
                  </TableCell>
                  <TableCell>
                    <div className="flex items-center gap-2">
                      <StatusBadge status={t.status} />
                      {t.isPlatform ? (
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <span tabIndex={0} aria-label="平台租户不可停用">
                              <Switch checked disabled />
                            </span>
                          </TooltipTrigger>
                          <TooltipContent>平台租户不可停用</TooltipContent>
                        </Tooltip>
                      ) : isPlatformContext ? (
                        <Switch
                          checked={t.status === "ENABLED"}
                          onCheckedChange={(v) => statusMutation.mutate({ id: t.id, next: v ? "ENABLED" : "DISABLED" })}
                          aria-label={`切换租户 ${t.name} 状态`}
                          disabled={statusMutation.isPending}
                        />
                      ) : (
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <span tabIndex={0} aria-label="仅平台租户可修改租户状态">
                              <Switch checked={t.status === "ENABLED"} disabled />
                            </span>
                          </TooltipTrigger>
                          <TooltipContent>仅平台租户可修改租户状态</TooltipContent>
                        </Tooltip>
                      )}
                    </div>
                  </TableCell>
                  <TableCell className="whitespace-nowrap">
                    <ExpiryCell expiresAt={t.expiresAt ?? null} />
                  </TableCell>
                  <TableCell className="text-right tabular-nums">{t.organizationCount}</TableCell>
                  <TableCell className="text-right tabular-nums">{t.userCount}</TableCell>
                  <TableCell className="text-right tabular-nums">{t.authorizedAppCount}</TableCell>
                  <TableCell className="text-xs text-muted-foreground">{formatDateTime(t.createdAt)}</TableCell>
                  <TableCell className="pr-5">
                    {isPlatformContext ? (
                      <div className="flex items-center justify-end gap-0.5">
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <Button asChild variant="ghost" size="icon" className="size-8">
                              <Link to={`/system/authorizations?tenantId=${t.id}`} aria-label={`管理 ${t.name} 的授权`}>
                                <Settings2 className="size-4" />
                              </Link>
                            </Button>
                          </TooltipTrigger>
                          <TooltipContent>授权管理</TooltipContent>
                        </Tooltip>
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <Button variant="ghost" size="icon" className="size-8" aria-label={`编辑 ${t.name}`} onClick={() => openEdit(t)}>
                              <Pencil className="size-4" />
                            </Button>
                          </TooltipTrigger>
                          <TooltipContent>编辑</TooltipContent>
                        </Tooltip>
                        {t.isPlatform ? (
                          <Tooltip>
                            <TooltipTrigger asChild>
                              <span tabIndex={0} aria-label="平台租户不可删除">
                                <Button variant="ghost" size="icon" className="size-8 text-destructive hover:text-destructive" disabled>
                                  <Trash2 className="size-4" />
                                </Button>
                              </span>
                            </TooltipTrigger>
                            <TooltipContent>平台租户不可删除</TooltipContent>
                          </Tooltip>
                        ) : (
                          <Tooltip>
                            <TooltipTrigger asChild>
                              <Button
                                variant="ghost"
                                size="icon"
                                className="size-8 text-destructive hover:text-destructive"
                                aria-label={`删除 ${t.name}`}
                                onClick={() => setDeleting(t)}
                              >
                                <Trash2 className="size-4" />
                              </Button>
                            </TooltipTrigger>
                            <TooltipContent>删除</TooltipContent>
                          </Tooltip>
                        )}
                      </div>
                    ) : (
                      <span className="mr-2 block text-right text-xs text-muted-foreground">只读</span>
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
        {!listQuery.isLoading && !listQuery.isError && total > 0 && (
          <div className="border-t px-4 py-3">
            <DataPagination
              page={page}
              size={size}
              total={total}
              onPageChange={setPage}
              onSizeChange={(s) => setSize(s)}
            />
          </div>
        )}
      </div>

      {/* 新建/编辑弹窗 */}
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{editing ? "编辑租户" : "新建租户"}</DialogTitle>
            <DialogDescription>
              {editing ? `正在编辑「${editing.name}」` : "租户是组织结构与授权体系的顶层容器"}
            </DialogDescription>
          </DialogHeader>
          <Form {...form}>
            <form
              id="tenant-form"
              onSubmit={form.handleSubmit((v) => saveMutation.mutate(v))}
              className="space-y-4"
            >
              <FormField
                control={form.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      名称 <span className="text-destructive">*</span>
                    </FormLabel>
                    <FormControl>
                      <Input placeholder="如: Dante 云平台" {...field} />
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
                      编码 <span className="text-destructive">*</span>
                    </FormLabel>
                    <FormControl>
                      <Input placeholder="如: dante-cloud" className="font-mono" {...field} disabled={!!editing} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="expiresAt"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>过期时间</FormLabel>
                    <FormControl>
                      <Input
                        type="datetime-local"
                        aria-label="租户过期时间"
                        {...field}
                        value={field.value ?? ""}
                      />
                    </FormControl>
                    <FormDescription>留空表示永不过期；到期后该租户下所有用户无法登录</FormDescription>
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
                      <Textarea rows={3} placeholder="选填,简要说明租户用途" {...field} />
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
            <Button type="submit" form="tenant-form" disabled={saveMutation.isPending}>
              {saveMutation.isPending && <Loader2 className="size-4 animate-spin" />}
              {editing ? "保存更改" : "创建"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={!!deleting}
        onOpenChange={(o) => !o && setDeleting(null)}
        title={`删除租户「${deleting?.name ?? ""}」?`}
        description={
          <>
            将级联删除该租户下的 <b>全部组织、部门、成员关系、授权与角色</b>,此操作不可撤销。
          </>
        }
        loading={deleteMutation.isPending}
        onConfirm={() => deleting && deleteMutation.mutate(deleting.id)}
      />
    </div>
  )
}
