import * as React from "react"
import { Link, useNavigate } from "react-router-dom"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useForm } from "react-hook-form"
import { z } from "zod"
import { zResolver } from "@/lib/form"
import { ArrowRight, ChevronDown, ChevronRight, LayoutGrid, ListTree, Loader2, MoreHorizontal, Pencil, Plus, Rocket, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Badge } from "@/components/ui/badge"
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
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
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
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { PageHeader } from "@/components/layout/PageHeader"
import { StatusBadge } from "@/components/StatusBadge"
import { ConfirmDialog } from "@/components/ConfirmDialog"
import { IconInput } from "@/components/IconInput"
import { CardGridSkeleton, EmptyState, ErrorState } from "@/components/states"
import { applicationsApi } from "@/api/applications"
import { asArray } from "@/lib/api"
import { AppIcon } from "@/lib/icons"
import { toast } from "sonner"
import { cn } from "@/lib/utils"
import { useAuthStore } from "@/stores/auth"
import { ApplicationReleasesPanel } from "./ApplicationReleasesPanel"
import type { ApplicationRecord } from "@/api/types"

const appSchema = z
  .object({
    code: z
      .string()
      .min(2, "请输入应用编码")
      .max(32)
      .regex(/^[a-z0-9-]+$/, "仅支持小写字母、数字与中划线"),
    name: z.string().min(1, "请输入应用名称").max(64),
    icon: z.string().max(64).optional(),
    basePath: z.string().max(128).optional(),
    description: z.string().max(255).optional(),
    sortOrder: z.coerce.number().int().min(0).max(9999),
    status: z.enum(["ENABLED", "DISABLED"]),
    runtimeMode: z.enum(["IFRAME", "TRUSTED"]),
    entryType: z.enum(["ESM", "HTML"]),
    entryUrl: z.string().max(500).optional(),
  })
  .superRefine((v, ctx) => {
    if (v.runtimeMode === "IFRAME" && v.entryType === "ESM") {
      ctx.addIssue({ code: z.ZodIssueCode.custom, path: ["entryType"], message: "iframe 隔离应用仅支持 HTML 入口" })
    }
    // HTML 入口留空时，后端按应用编码使用同源静态目录约定。
    if (v.entryType === "HTML") {
      const url = (v.entryUrl ?? "").trim()
      if (url && !/^(https?:\/\/|\/)/i.test(url)) {
        ctx.addIssue({ code: z.ZodIssueCode.custom, path: ["entryUrl"], message: "入口地址需以 http(s):// 或 / 开头" })
      }
    }
  })
type AppForm = z.infer<typeof appSchema>

export default function ApplicationsPage() {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const canManageTrusted = useAuthStore((s) => !!(s.profile?.tenant.isPlatform || s.profile?.user.isSuperAdmin))

  const listQuery = useQuery({
    queryKey: ["applications"],
    queryFn: async () => asArray<ApplicationRecord>(await applicationsApi.list()),
    staleTime: 30_000,
    retry: 1,
  })

  const [dialogOpen, setDialogOpen] = React.useState(false)
  const [editing, setEditing] = React.useState<ApplicationRecord | null>(null)
  const [releasesApplication, setReleasesApplication] = React.useState<ApplicationRecord | null>(null)
  const [quickSetup, setQuickSetup] = React.useState(true)
  const form = useForm<AppForm>({
    resolver: zResolver(appSchema),
    defaultValues: { code: "", name: "", icon: "LayoutGrid", basePath: "", description: "", sortOrder: 0, status: "ENABLED", runtimeMode: "IFRAME", entryType: "HTML", entryUrl: "" },
  })
  const entryTypeValue = form.watch("entryType")
  const runtimeModeValue = form.watch("runtimeMode")
  const codeValue = form.watch("code")
  const useQuickSetup = !editing && entryTypeValue === "HTML" && quickSetup

  const openCreate = () => {
    setEditing(null)
    setQuickSetup(true)
    form.reset({ code: "", name: "", icon: "LayoutGrid", basePath: "", description: "", sortOrder: 0, status: "ENABLED", runtimeMode: "IFRAME", entryType: "HTML", entryUrl: "" })
    setDialogOpen(true)
  }
  const openEdit = (a: ApplicationRecord) => {
    setEditing(a)
    setQuickSetup(false)
    form.reset({
      code: a.code,
      name: a.name,
      icon: a.icon ?? "LayoutGrid",
      basePath: a.basePath ?? "",
      description: a.description ?? "",
      sortOrder: a.sortOrder ?? 0,
      status: a.status,
      runtimeMode: a.runtimeMode ?? "IFRAME",
      entryType: a.entryType === "HTML" ? "HTML" : "ESM",
      entryUrl: a.entryUrl ?? "",
    })
    setDialogOpen(true)
  }

  const saveMutation = useMutation({
    mutationFn: async (values: AppForm) => {
      const quick = !editing && values.entryType === "HTML" && quickSetup
      const payload = {
        code: values.code,
        name: values.name,
        icon: values.icon || "LayoutGrid",
        basePath: quick ? `/micro/${values.code}` : values.basePath || "",
        description: values.description || "",
        sortOrder: values.sortOrder,
        status: values.status,
        runtimeMode: values.runtimeMode,
        entryType: values.entryType,
        entryUrl: quick ? `/sub-apps/${values.code}/index.html` : values.entryUrl?.trim() || "",
        autoCreateMenu: quick,
      }
      if (editing) {
        await applicationsApi.update(editing.id, payload)
        return null
      }
      /* create 返回新建应用记录(含 id), 供 toast 引导直接跳转菜单配置 */
      const created = (await applicationsApi.create(payload)) as { id?: string } | null | undefined
      return { id: created?.id ?? null, quick }
    },
    onSuccess: (created) => {
      if (created?.id) {
        toast.success("应用创建成功", {
          description: created.quick ? "首页菜单已创建，下一步配置授权" : "下一步: 为应用配置菜单树",
          action: {
            label: created.quick ? "去授权" : "去配置菜单",
            onClick: () => navigate(created.quick ? "/system/authorizations" : `/system/menus?applicationId=${created.id}`),
          },
        })
      } else {
        toast.success(editing ? "应用已更新" : "应用创建成功")
      }
      setDialogOpen(false)
      queryClient.invalidateQueries({ queryKey: ["applications"] })
      queryClient.invalidateQueries({ queryKey: ["profile"] })
    },
    onError: (e: Error) => toast.error(e.message || "保存失败"),
  })

  const [deleting, setDeleting] = React.useState<ApplicationRecord | null>(null)
  const deleteMutation = useMutation({
    mutationFn: (id: string) => applicationsApi.remove(id),
    onSuccess: () => {
      toast.success("应用已删除")
      setDeleting(null)
      queryClient.invalidateQueries({ queryKey: ["applications"] })
    },
    onError: (e: Error) => toast.error(e.message || "删除失败"),
  })

  const apps = listQuery.data ?? []

  return (
    <div>
      <PageHeader
        title="应用管理"
        description="接入平台的子应用,应用通过菜单与授权体系对接"
        icon={<LayoutGrid className="size-5" />}
        actions={
          <Button onClick={openCreate}>
            <Plus className="size-4" />
            新建微应用
          </Button>
        }
      />

      <OnboardingGuide onCreate={openCreate} />

      {listQuery.isLoading ? (
        <CardGridSkeleton count={8} />
      ) : listQuery.isError ? (
        <ErrorState
          message={listQuery.error instanceof Error ? listQuery.error.message : undefined}
          onRetry={() => listQuery.refetch()}
        />
      ) : apps.length === 0 ? (
        <EmptyState
          title="还没有应用"
          description="创建第一个应用,为其配置菜单并授权给租户"
          action={
            <Button size="sm" onClick={openCreate}>
              <Plus className="size-4" />
              新建微应用
            </Button>
          }
          icon={<LayoutGrid className="size-6" />}
        />
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
          {apps.map((a) => (
            <div
              key={a.id}
              className="group relative flex flex-col gap-3 rounded-xl border bg-card p-4 transition-all hover:-translate-y-0.5 hover:shadow-md"
            >
              <div className="flex items-start justify-between gap-2">
                <div className="flex size-11 items-center justify-center rounded-xl bg-primary/10 text-primary">
                  <AppIcon name={a.icon} className="size-6" />
                </div>
                <StatusBadge status={a.status} />
              </div>
              <div className="min-w-0">
                <p className="truncate font-semibold">{a.name}</p>
                <p className="mt-0.5 truncate font-mono text-xs text-muted-foreground">{a.code}</p>
              </div>
              {a.description && (
                <p className="line-clamp-2 text-xs text-muted-foreground">{a.description}</p>
              )}
              <div className="mt-auto flex flex-wrap items-center gap-1.5 border-t pt-3 text-xs text-muted-foreground">
                <Badge variant="secondary" className="text-[11px] tabular-nums">{a.menuCount} 菜单</Badge>
                <Badge variant="outline" className="text-[11px] tabular-nums">排序 {a.sortOrder}</Badge>
                <Badge variant="outline" className="text-[11px]">{a.runtimeMode === "TRUSTED" ? "自有可信" : "iframe 隔离"}</Badge>
                {/* 入口通道: ESM 灰 / HTML 紫 + entry_url truncate + title 悬停全量 */}
                {a.entryType === "HTML" ? (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span
                        className="inline-flex max-w-[10rem] items-center gap-1 rounded-full border border-purple-500/30 bg-purple-500/10 px-2 py-px text-[10px] font-medium text-purple-700 dark:text-purple-300"
                        title={a.entryUrl ?? "(未配置入口地址)"}
                      >
                        HTML
                        {a.entryUrl && (
                          <code className="max-w-[6.5rem] truncate font-mono">{a.entryUrl}</code>
                        )}
                      </span>
                    </TooltipTrigger>
                    <TooltipContent side="bottom" className="max-w-[20rem]">
                      <span className="block text-xs">HTML 入口 · {a.runtimeMode === "TRUSTED" ? "宿主内可信执行" : "iframe 隔离执行"}</span>
                      <span className="mt-0.5 block font-mono text-xs break-all">{a.entryUrl ?? "(未配置)"}</span>
                    </TooltipContent>
                  </Tooltip>
                ) : (
                  <span className="rounded-full border bg-muted px-2 py-px text-[10px] font-medium text-muted-foreground">
                    ESM
                  </span>
                )}
                {a.basePath && (
                  <code className="max-w-[9rem] truncate rounded bg-muted px-1.5 py-0.5 font-mono text-[10px]">
                    {a.basePath}
                  </code>
                )}
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="ml-auto h-7 gap-1 px-2 text-xs text-muted-foreground hover:text-foreground"
                      onClick={() => navigate(`/system/menus?applicationId=${a.id}`)}
                    >
                      <ListTree className="size-3.5" />
                      配置菜单
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>为该应用配置菜单</TooltipContent>
                </Tooltip>
              </div>
              <Button variant="outline" size="sm" className="w-full" onClick={() => setReleasesApplication(a)}>
                <Rocket className="size-3.5" />
                发布版本
              </Button>

              <div className="absolute right-2 top-2 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button variant="ghost" size="icon" className="size-8 bg-background/80 backdrop-blur" aria-label={`操作 ${a.name}`}>
                      <MoreHorizontal className="size-4" />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end" className="w-36">
                    <DropdownMenuItem onClick={() => openEdit(a)}>
                      <Pencil className="size-4" />
                      编辑
                    </DropdownMenuItem>
                    <DropdownMenuItem onClick={() => navigate(`/system/menus?applicationId=${a.id}`)}>
                      <ListTree className="size-4" />
                      配置菜单
                    </DropdownMenuItem>
                    <DropdownMenuItem onClick={() => setReleasesApplication(a)}>
                      <Rocket className="size-4" />
                      发布版本
                    </DropdownMenuItem>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem variant="destructive" onClick={() => setDeleting(a)}>
                      <Trash2 className="size-4" />
                      删除
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* 新建/编辑弹窗 */}
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{editing ? "编辑应用" : "新建微应用"}</DialogTitle>
            <DialogDescription>
              {editing ? `正在编辑「${editing.name}」` : "应用编码创建后不可修改"}
            </DialogDescription>
          </DialogHeader>
          <Form {...form}>
            <form id="app-form" onSubmit={form.handleSubmit((v) => saveMutation.mutate(v))} className="space-y-4">
              <div className="grid gap-4 sm:grid-cols-2">
                <FormField
                  control={form.control}
                  name="name"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        应用名称 <span className="text-destructive">*</span>
                      </FormLabel>
                      <FormControl>
                        <Input placeholder="如: 数据分析" {...field} />
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
                        应用编码 <span className="text-destructive">*</span>
                      </FormLabel>
                      <FormControl>
                        <Input placeholder="如: athena" className="font-mono" {...field} disabled={!!editing} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>
              <div className="grid gap-4 sm:grid-cols-2">
                <FormField
                  control={form.control}
                  name="icon"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>图标</FormLabel>
                      <FormControl>
                        <IconInput value={field.value ?? ""} onChange={field.onChange} />
                      </FormControl>
                      <FormDescription>从图标库中选择,展示于应用切换器与工作台</FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="basePath"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>基础路径</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="如 /athena"
                          className="font-mono"
                          {...field}
                          value={useQuickSetup ? `/micro/${codeValue || "{code}"}` : field.value}
                          readOnly={useQuickSetup}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>

              {/* 微前端入口: entry_type + entry_url (契约 21-a/21-c) */}
              <FormField
                control={form.control}
                name="runtimeMode"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>运行方式</FormLabel>
                    <Select
                      value={field.value}
                      disabled={!canManageTrusted}
                      onValueChange={(value) => {
                        field.onChange(value)
                        if (value === "IFRAME") form.setValue("entryType", "HTML", { shouldValidate: true })
                      }}
                    >
                      <FormControl><SelectTrigger className="w-full"><SelectValue /></SelectTrigger></FormControl>
                      <SelectContent>
                        <SelectItem value="IFRAME">iframe · 默认隔离</SelectItem>
                        <SelectItem value="TRUSTED">自有可信 · 宿主内执行</SelectItem>
                      </SelectContent>
                    </Select>
                    <FormDescription>
                      {runtimeModeValue === "TRUSTED"
                        ? "仅适用于平台自有、已审查的可信应用，HTML / ESM 会在宿主内执行。"
                        : "默认通过 iframe 隔离第三方 HTML 页面；平台管理员可显式标记自有可信应用。"}
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />
              {entryTypeValue === "HTML" && !editing && (
                <div className="flex items-center justify-between gap-4 rounded-md border bg-muted/30 px-3 py-2.5">
                  <div>
                    <p className="text-sm font-medium">快速接入</p>
                    <p className="text-xs text-muted-foreground">按编码生成挂载路径、静态入口和首页菜单</p>
                  </div>
                  <Switch checked={quickSetup} onCheckedChange={setQuickSetup} aria-label="快速接入微应用" />
                </div>
              )}
              <div className="grid gap-4 sm:grid-cols-2">
                <FormField
                  control={form.control}
                  name="entryType"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>入口类型</FormLabel>
                      <Select onValueChange={field.onChange} value={field.value}>
                        <FormControl>
                          <SelectTrigger className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                        </FormControl>
                        <SelectContent>
                          <SelectItem value="ESM" disabled={runtimeModeValue !== "TRUSTED"}>ESM · 可信模块</SelectItem>
                          <SelectItem value="HTML">HTML · 微前端页面</SelectItem>
                        </SelectContent>
                      </Select>
                      <FormDescription>{runtimeModeValue === "TRUSTED" ? "自有可信应用可接入 HTML 页面或 ESM 模块" : "iframe 隔离应用使用 HTML 页面入口"}</FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="entryUrl"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        入口地址
                      </FormLabel>
                      <FormControl>
                        <Input
                          placeholder="/sub-apps/example/index.html"
                          className="font-mono"
                          value={useQuickSetup ? `/sub-apps/${codeValue || "{code}"}/index.html` : field.value ?? ""}
                          onChange={field.onChange}
                          onBlur={field.onBlur}
                          name={field.name}
                          readOnly={useQuickSetup}
                        />
                      </FormControl>
                      <FormDescription>
                        留空时使用 /sub-apps/{"{"}code{"}"}/index.html；自定义地址需以 http(s):// 或 / 开头
                      </FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>
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
              <div className="grid gap-4 sm:grid-cols-2">
                <FormField
                  control={form.control}
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
                  control={form.control}
                  name="status"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>状态</FormLabel>
                      <Select onValueChange={field.onChange} value={field.value}>
                        <FormControl>
                          <SelectTrigger className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                        </FormControl>
                        <SelectContent>
                          <SelectItem value="ENABLED">启用</SelectItem>
                          <SelectItem value="DISABLED">停用</SelectItem>
                        </SelectContent>
                      </Select>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>
              <p className="text-xs leading-relaxed text-muted-foreground">保存登记信息后，可通过“发布版本”注册环境版本并手动预检、激活。租户、组织和角色授权需另行配置。</p>
            </form>
          </Form>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDialogOpen(false)} disabled={saveMutation.isPending}>
              取消
            </Button>
            <Button type="submit" form="app-form" disabled={saveMutation.isPending}>
              {saveMutation.isPending && <Loader2 className="size-4 animate-spin" />}
              {editing ? "保存更改" : "创建"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {releasesApplication && (
        <ApplicationReleasesPanel
          key={releasesApplication.id}
          application={apps.find((app) => app.id === releasesApplication.id) ?? releasesApplication}
          open
          onOpenChange={(open) => !open && setReleasesApplication(null)}
        />
      )}

      <ConfirmDialog
        open={!!deleting}
        onOpenChange={(o) => !o && setDeleting(null)}
        title={`删除应用「${deleting?.name ?? ""}」?`}
        description={
          <>
            将级联删除该应用的 <b>全部菜单与各层级授权记录</b>(租户授权 / 组织授权 / 角色菜单),不可撤销。
          </>
        }
        loading={deleteMutation.isPending}
        onConfirm={() => deleting && deleteMutation.mutate(deleting.id)}
      />
    </div>
  )
}

/* ============ 新应用接入四步曲引导卡 ============ */

const ONBOARDING_STEPS: { title: string; desc: string; to: string | null }[] = [
  { title: "登记 / 发布应用", desc: "登记应用，按环境注册、预检并手动激活版本", to: null },
  { title: "配置菜单树", desc: "按需增加子页面、目录与按钮", to: "/system/menus" },
  { title: "授权租户 / 组织", desc: "把应用授权给租户, 可细化到指定组织", to: "/system/authorizations" },
  { title: "角色绑定菜单", desc: "给角色分配菜单与按钮权限, 用户即见菜单", to: "/system/roles" },
]

/** 可折叠的接入引导卡: muted 底 + 步骤圆点 + chevron 连接, 步骤① 高亮为「本页」 */
function OnboardingGuide({ onCreate }: { onCreate: () => void }) {
  const [open, setOpen] = React.useState(true)

  return (
    <Card className="mb-4 gap-3 bg-muted/40 py-4 shadow-none">
      <CardHeader
        className="cursor-pointer select-none py-0"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
      >
        <CardTitle className="flex items-center gap-2 text-sm">
          <Rocket className="size-4 text-primary" />
          新应用接入四步曲
        </CardTitle>
        <CardDescription className="text-xs">应用 → 菜单 → 授权 → 角色绑定, 按序完成即可接入平台</CardDescription>
        <CardAction>
          <ChevronDown
            className={cn("size-4 text-muted-foreground transition-transform", !open && "-rotate-90")}
          />
        </CardAction>
      </CardHeader>
      {open && (
        <CardContent className="pt-0">
          <div className="flex flex-col gap-2.5 lg:flex-row lg:items-stretch">
            {ONBOARDING_STEPS.map((s, i) => (
              <React.Fragment key={s.title}>
                <div
                  className={cn(
                    "flex flex-1 items-start gap-3 rounded-lg border bg-card p-3 transition-colors",
                    s.to === null ? "border-primary/40 ring-1 ring-primary/25" : "hover:border-primary/30"
                  )}
                >
                  <span className="flex size-7 shrink-0 items-center justify-center rounded-full bg-primary/15 text-sm font-semibold tabular-nums text-primary">
                    {i + 1}
                  </span>
                  <div className="min-w-0 flex-1">
                    <p className="flex flex-wrap items-center gap-1.5 text-sm font-medium">
                      {s.title}
                      {s.to === null && (
                        <Badge variant="secondary" className="h-5 bg-primary/15 px-1.5 text-[10px] font-medium text-primary">
                          本页
                        </Badge>
                      )}
                    </p>
                    <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{s.desc}</p>
                    <div className="mt-2.5">
                      {s.to ? (
                        <Button asChild variant="outline" size="sm" className="h-7 gap-1 px-2.5 text-xs">
                          <Link to={s.to}>
                            前往
                            <ArrowRight className="size-3" />
                          </Link>
                        </Button>
                      ) : (
                        <Button variant="outline" size="sm" className="h-7 gap-1 px-2.5 text-xs" onClick={onCreate}>
                          开始创建
                          <Plus className="size-3" />
                        </Button>
                      )}
                    </div>
                  </div>
                </div>
                {i < ONBOARDING_STEPS.length - 1 && (
                  <div aria-hidden className="hidden shrink-0 items-center px-0.5 lg:flex">
                    <ChevronRight className="size-4 text-muted-foreground/50" />
                  </div>
                )}
              </React.Fragment>
            ))}
          </div>
        </CardContent>
      )}
    </Card>
  )
}
