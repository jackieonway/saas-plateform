import * as React from "react"
import { Link, useSearchParams } from "react-router-dom"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useForm } from "react-hook-form"
import { z } from "zod"
import { zResolver } from "@/lib/form"
import {
  ArrowDown,
  ArrowUp,
  ChevronDown,
  ChevronRight,
  ExternalLink,
  Eye,
  EyeOff,
  FolderTree,
  Loader2,
  Pencil,
  Plus,
  Trash2,
} from "lucide-react"
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
import { ScrollArea } from "@/components/ui/scroll-area"
import { PageHeader } from "@/components/layout/PageHeader"
import { ConfirmDialog } from "@/components/ConfirmDialog"
import { IconInput } from "@/components/IconInput"
import { EntityTreePicker, type PickerNode } from "@/components/EntityTreePicker"
import { EmptyState, ErrorState } from "@/components/states"
import { applicationsApi } from "@/api/applications"
import { menusApi } from "@/api/menus"
import { asArray } from "@/lib/api"
import { AppIcon } from "@/lib/icons"
import { MENU_TYPE_META } from "@/lib/format"
import { descendantIds } from "@/lib/tree"
import { toast } from "sonner"
import { cn } from "@/lib/utils"
import type { ApplicationRecord, MenuNode } from "@/api/types"

const menuSchema = z.object({
  type: z.enum(["DIR", "MENU", "BUTTON"]),
  parentId: z.string().nullable(),
  title: z.string().min(1, "请输入标题").max(64),
  path: z.string().max(128).optional(),
  component: z.string().max(128).optional(),
  icon: z.string().max(64).optional(),
  permission: z.string().max(64).optional(),
  sortOrder: z.coerce.number().int().min(0).max(9999),
  visible: z.boolean(),
  // 外链: 空 = 非外链; 非空必须 http(s):// 开头且 ≤ 500 字符(与后端 VARCHAR(500) 同口径)
  linkUrl: z
    .string()
    .max(500, "外链地址最长 500 字符")
    .refine((v) => !v.trim() || /^https?:\/\//i.test(v.trim()), "外链地址需以 http:// 或 https:// 开头")
    .optional(),
  linkTarget: z.enum(["BLANK", "SELF", "MICRO"]).optional(),
})
type MenuForm = z.infer<typeof menuSchema>

export default function MenusPage() {
  const queryClient = useQueryClient()

  const appsQuery = useQuery({
    queryKey: ["applications"],
    queryFn: async () => asArray<ApplicationRecord>(await applicationsApi.list()),
    staleTime: 30_000,
    retry: 1,
  })
  const apps = appsQuery.data ?? []
  const [selectedAppId, setSelectedAppId] = React.useState<string | null>(null)
  React.useEffect(() => {
    if (!selectedAppId && apps.length > 0) setSelectedAppId(apps[0].id)
    if (selectedAppId && apps.length > 0 && !apps.some((a) => a.id === selectedAppId)) {
      setSelectedAppId(apps[0].id)
    }
  }, [apps, selectedAppId])

  /* ?applicationId= 深链(应用页「配置菜单」/创建引导 toast 跳转):
     ① AppLayout 页面过渡(AnimatePresence)会短暂重挂载本组件 → 深链参数必须在
        URL 上存活到重挂载完成, 不能立刻用路由导航删除(重挂载后参数已丢会回退默认选中);
     ② applications 缓存可能正在后台 refetch(创建应用后 invalidate) → 等抓取完成
        再用最新列表判定, 避免新应用尚未入缓被误判「不存在」;
     ③ 消费后用 history.replaceState 静默清理地址栏(不走路由导航、不触发重挂载),
        刷新后即回到默认选中态。 */
  const [searchParams] = useSearchParams()
  const pendingAppId = searchParams.get("applicationId")
  const [deeplinkDone, setDeeplinkDone] = React.useState(false)
  React.useEffect(() => {
    if (deeplinkDone || !pendingAppId || apps.length === 0) return
    if (appsQuery.isFetching) return
    setDeeplinkDone(true)
    if (apps.some((a) => a.id === pendingAppId)) {
      setSelectedAppId(pendingAppId)
    }
    const next = new URLSearchParams(window.location.search)
    if (next.has("applicationId")) {
      next.delete("applicationId")
      const qs = next.toString()
      window.history.replaceState(null, "", qs ? `${window.location.pathname}?${qs}` : window.location.pathname)
    }
  }, [deeplinkDone, pendingAppId, apps, appsQuery.isFetching])
  const selectedApp = apps.find((a) => a.id === selectedAppId) ?? null

  const treeQuery = useQuery({
    queryKey: ["menus-tree", selectedAppId],
    queryFn: async () => asArray<MenuNode>(await menusApi.tree(selectedAppId!), "menus"),
    enabled: !!selectedAppId,
    staleTime: 30_000,
    retry: 1,
  })

  const refresh = () => {
    queryClient.invalidateQueries({ queryKey: ["menus-tree"] })
    queryClient.invalidateQueries({ queryKey: ["applications"] })
    queryClient.invalidateQueries({ queryKey: ["auth-menus"] })
  }

  /* ---------- 弹窗 ---------- */
  const [dialogOpen, setDialogOpen] = React.useState(false)
  const [editing, setEditing] = React.useState<MenuNode | null>(null)
  const [presetParent, setPresetParent] = React.useState<string | null>(null)
  const form = useForm<MenuForm>({
    resolver: zResolver(menuSchema),
    defaultValues: { type: "MENU", parentId: null, title: "", path: "", component: "", icon: "", permission: "", sortOrder: 0, visible: true, linkUrl: "", linkTarget: "BLANK" },
  })
  const menuType = form.watch("type")
  const linkUrlValue = (form.watch("linkUrl") ?? "").trim()
  const linkTargetValue = form.watch("linkTarget") ?? "BLANK"

  const openCreate = (parentId: string | null) => {
    setEditing(null)
    setPresetParent(parentId)
    const siblings = parentId
      ? findNode(treeQuery.data ?? [], parentId)?.children ?? []
      : treeQuery.data ?? []
    form.reset({
      type: "MENU",
      parentId,
      title: "",
      path: "",
      component: "",
      icon: "",
      permission: "",
      sortOrder: siblings.length,
      visible: true,
      linkUrl: "",
      linkTarget: "BLANK",
    })
    setDialogOpen(true)
  }

  const openEdit = (node: MenuNode) => {
    setEditing(node)
    setPresetParent(node.parentId)
    form.reset({
      type: node.type,
      parentId: node.parentId,
      title: node.title,
      path: node.path ?? "",
      component: (node as MenuNode & { component?: string }).component ?? "",
      icon: node.icon ?? "",
      permission: node.permission ?? "",
      sortOrder: node.sortOrder ?? 0,
      visible: node.visible ?? true,
      linkUrl: node.linkUrl ?? "",
      linkTarget: node.linkTarget === "SELF" ? "SELF" : node.linkTarget === "MICRO" ? "MICRO" : "BLANK",
    })
    setDialogOpen(true)
  }

  const toPayload = (v: MenuForm) => ({
    applicationId: selectedAppId!,
    parentId: v.parentId,
    type: v.type,
    title: v.title,
    path: v.path || undefined,
    icon: v.icon || undefined,
    component: v.component || undefined,
    permission: v.permission || undefined,
    sortOrder: v.sortOrder,
    visible: v.visible,
    // 空串显式下发以便清除外链(后端 Some(""))
    linkUrl: v.type === "MENU" ? v.linkUrl?.trim() || "" : "",
    // MICRO(微前端路由)无需 linkUrl, 单独下发; BLANK/SELF 仅在有外链时才有意义
    linkTarget:
      v.type === "MENU"
        ? v.linkUrl?.trim()
          ? v.linkTarget ?? "BLANK"
          : v.linkTarget === "MICRO"
            ? "MICRO"
            : undefined
        : undefined,
  })

  const saveMutation = useMutation({
    mutationFn: async (v: MenuForm) => {
      if (editing) await menusApi.update(editing.id, toPayload(v))
      else await menusApi.create(toPayload(v))
    },
    onSuccess: () => {
      toast.success(editing ? "菜单已更新" : "菜单创建成功")
      setDialogOpen(false)
      refresh()
    },
    onError: (e: Error) => toast.error(e.message || "保存失败"),
  })

  const [deleting, setDeleting] = React.useState<MenuNode | null>(null)
  const deleteMutation = useMutation({
    mutationFn: (id: string) => menusApi.remove(id),
    onSuccess: () => {
      toast.success("菜单已删除")
      setDeleting(null)
      refresh()
    },
    onError: (e: Error) => toast.error(e.message || "删除失败"),
  })

  const moveMutation = useMutation({
    mutationFn: async ({ a, b }: { a: MenuNode; b: MenuNode }) => {
      // 交换两个同级菜单的 sortOrder
      await menusApi.update(a.id, nodeToPayload(a, b.sortOrder ?? 0))
      await menusApi.update(b.id, nodeToPayload(b, a.sortOrder ?? 0))
    },
    onSuccess: () => {
      refresh()
    },
    onError: (e: Error) => toast.error(e.message || "移动失败"),
  })

  const nodeToPayload = (n: MenuNode, sortOrder: number) => ({
    applicationId: selectedAppId!,
    parentId: n.parentId,
    type: n.type,
    title: n.title,
    path: n.path ?? undefined,
    icon: n.icon ?? undefined,
    component: (n as MenuNode & { component?: string }).component ?? undefined,
    permission: n.permission ?? undefined,
    sortOrder,
    visible: n.visible ?? true,
    // 上下移动仅交换排序, 外链字段原样保留(MICRO 无 linkUrl 也需保留 linkTarget)
    linkUrl: n.linkUrl || "",
    linkTarget: n.linkUrl ? (n.linkTarget ?? "BLANK") : n.linkTarget === "MICRO" ? "MICRO" : undefined,
  })

  const tree = treeQuery.data ?? []

  // 父级选择候选(排除编辑中节点及其后代)
  const parentCandidates = React.useMemo<PickerNode[]>(() => {
    const exclude = editing ? new Set([editing.id, ...descendantIds(tree, editing.id)]) : new Set<string>()
    const map = (nodes: MenuNode[]): PickerNode[] =>
      nodes
        .filter((n) => n.type === "DIR" && !exclude.has(n.id))
        .map((n) => ({ id: n.id, name: n.title, code: n.path ?? undefined, children: map(n.children ?? []) }))
    return map(tree)
  }, [tree, editing])

  return (
    <div>
      <PageHeader
        title="菜单管理"
        description="定义每个应用的菜单树:目录 / 菜单 / 按钮三级粒度,按钮通过权限码控制"
        icon={<FolderTree className="size-5" />}
        actions={
          selectedApp && (
            <Button onClick={() => openCreate(null)}>
              <Plus className="size-4" />
              新增根菜单
            </Button>
          )
        }
      />

      <div className="flex flex-col gap-4 lg:flex-row">
        {/* 左栏:应用列表 */}
        <aside className="w-full shrink-0 rounded-xl border bg-card lg:w-[280px]">
          <p className="border-b px-4 py-3 text-xs font-medium uppercase tracking-wider text-muted-foreground">
            应用({apps.length})
          </p>
          <ScrollArea className="h-[30rem] lg:h-[calc(100dvh-16rem)]">
            <div className="space-y-1.5 p-3">
              {appsQuery.isLoading ? (
                Array.from({ length: 4 }).map((_, i) => (
                  <div key={i} className="flex items-center gap-3 rounded-lg border p-3">
                    <div className="size-9 animate-pulse rounded-lg bg-muted" />
                    <div className="flex-1 space-y-1.5">
                      <div className="h-3.5 w-2/3 animate-pulse rounded bg-muted" />
                      <div className="h-3 w-1/3 animate-pulse rounded bg-muted" />
                    </div>
                  </div>
                ))
              ) : appsQuery.isError ? (
                <div className="p-2">
                  <ErrorState
                    className="border-0 bg-transparent"
                    message={appsQuery.error instanceof Error ? appsQuery.error.message : undefined}
                    onRetry={() => appsQuery.refetch()}
                  />
                </div>
              ) : apps.length === 0 ? (
                <EmptyState
                  className="py-10"
                  title="先创建应用"
                  description="菜单归属于应用,请先在「应用管理」创建"
                  action={
                    <Button asChild size="sm" variant="outline">
                      <Link to="/system/applications">去创建应用</Link>
                    </Button>
                  }
                />
              ) : (
                apps.map((a) => {
                  const active = a.id === selectedAppId
                  return (
                    <button
                      key={a.id}
                      type="button"
                      onClick={() => setSelectedAppId(a.id)}
                      aria-current={active ? "true" : undefined}
                      className={cn(
                        "flex w-full items-center gap-3 rounded-lg border p-3 text-left transition-all",
                        active
                          ? "border-primary/50 bg-primary/10 ring-1 ring-primary/30"
                          : "border-border hover:border-primary/30 hover:bg-muted/50"
                      )}
                    >
                      <span
                        className={cn(
                          "flex size-9 shrink-0 items-center justify-center rounded-lg",
                          active ? "bg-primary/15 text-primary" : "bg-muted text-muted-foreground"
                        )}
                      >
                        <AppIcon name={a.icon} className="size-5" />
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className={cn("block truncate text-sm font-medium", active && "text-primary")}>{a.name}</span>
                        <span className="block text-xs tabular-nums text-muted-foreground">
                          {a.menuCount} 个菜单 · <span className="font-mono">{a.code}</span>
                        </span>
                      </span>
                    </button>
                  )
                })
              )}
            </div>
          </ScrollArea>
        </aside>

        {/* 右栏:菜单树编辑器 */}
        <section className="min-w-0 flex-1 rounded-xl border bg-card">
          {!selectedApp ? (
            <EmptyState
              className="py-16"
              title="请选择应用"
              description="从左侧选择要编辑菜单的应用"
              icon={<FolderTree className="size-6" />}
            />
          ) : (
            <>
              <div className="flex flex-col gap-2 border-b px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
                <div className="flex items-center gap-2.5">
                  <AppIcon name={selectedApp.icon} className="size-4 text-primary" />
                  <p className="text-sm font-semibold">{selectedApp.name}</p>
                  <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-muted-foreground">{selectedApp.code}</code>
                </div>
                <Button size="sm" variant="outline" onClick={() => openCreate(null)}>
                  <Plus className="size-4" />
                  新增根菜单
                </Button>
              </div>

              {treeQuery.isLoading ? (
                <div className="space-y-2 p-4">
                  {Array.from({ length: 8 }).map((_, i) => (
                    <div key={i} className="h-10 animate-pulse rounded-md bg-muted" style={{ marginLeft: `${(i % 3) * 16}px` }} />
                  ))}
                </div>
              ) : treeQuery.isError ? (
                <div className="p-4">
                  <ErrorState
                    className="border-0 bg-transparent"
                    message={treeQuery.error instanceof Error ? treeQuery.error.message : undefined}
                    onRetry={() => treeQuery.refetch()}
                  />
                </div>
              ) : tree.length === 0 ? (
                <EmptyState
                  className="py-16"
                  title="该应用还没有菜单"
                  description="从根菜单开始,搭建目录 → 菜单 → 按钮的层级结构"
                  action={
                    <Button size="sm" onClick={() => openCreate(null)}>
                      <Plus className="size-4" />
                      新增根菜单
                    </Button>
                  }
                />
              ) : (
                <div className="p-3">
                  <MenuTreeEditor
                    nodes={tree}
                    onAdd={(parentId) => openCreate(parentId)}
                    onEdit={openEdit}
                    onDelete={(n) => setDeleting(n)}
                    onMove={(a, b) => moveMutation.mutate({ a, b })}
                    moving={moveMutation.isPending}
                  />
                </div>
              )}
            </>
          )}
        </section>
      </div>

      {/* 新建/编辑弹窗 */}
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{editing ? `编辑菜单「${editing.title}」` : "新增菜单"}</DialogTitle>
            <DialogDescription>
              {editing ? "" : presetParent ? "将在所选目录下创建子项" : "将创建为根级菜单"}
            </DialogDescription>
          </DialogHeader>
          <Form {...form}>
            <form id="menu-form" onSubmit={form.handleSubmit((v) => saveMutation.mutate(v))} className="space-y-4">
              <div className="grid gap-4 sm:grid-cols-2">
                <FormField
                  control={form.control}
                  name="type"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        类型 <span className="text-destructive">*</span>
                      </FormLabel>
                      <Select onValueChange={field.onChange} value={field.value}>
                        <FormControl>
                          <SelectTrigger className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                        </FormControl>
                        <SelectContent>
                          <SelectItem value="DIR">目录(DIR)</SelectItem>
                          <SelectItem value="MENU">菜单(MENU)</SelectItem>
                          <SelectItem value="BUTTON">按钮(BUTTON)</SelectItem>
                        </SelectContent>
                      </Select>
                      <FormDescription>按钮类型通过权限码参与授权</FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="parentId"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>父级</FormLabel>
                      <FormControl>
                        <EntityTreePicker
                          nodes={parentCandidates}
                          value={field.value}
                          onSelect={field.onChange}
                          placeholder="根级(不选父级)"
                          allowClear
                          emptyText="暂无可选目录"
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>

              <FormField
                control={form.control}
                name="title"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      标题 <span className="text-destructive">*</span>
                    </FormLabel>
                    <FormControl>
                      <Input placeholder="如: 用户管理" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              {menuType !== "BUTTON" && (
                <div className="grid gap-4 sm:grid-cols-2">
                  <FormField
                    control={form.control}
                    name="path"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>路由路径</FormLabel>
                        <FormControl>
                          <Input placeholder="如 /system/users" className="font-mono" {...field} />
                        </FormControl>
                        {menuType === "MENU" && linkUrlValue && (
                          <FormDescription>已配置外链,path 仅作唯一标识,不参与跳转</FormDescription>
                        )}
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                  <FormField
                    control={form.control}
                    name="component"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>组件</FormLabel>
                        <FormControl>
                          <Input placeholder="如 system/users/index" className="font-mono" {...field} />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                </div>
              )}

              {/* 外链: 仅菜单类型可配置(目录/按钮不参与跳转) */}
              {menuType === "MENU" && (
                <div className="grid gap-4 sm:grid-cols-2">
                  <FormField
                    control={form.control}
                    name="linkUrl"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>外链地址</FormLabel>
                        <FormControl>
                          <Input
                            placeholder="https://example.com"
                            className="font-mono"
                            value={field.value ?? ""}
                            onChange={field.onChange}
                            onBlur={field.onBlur}
                            name={field.name}
                          />
                        </FormControl>
                        <FormDescription>填写后菜单按外链打开,留空则为普通路由菜单</FormDescription>
                        {linkTargetValue === "MICRO" && (
                          <FormDescription className="text-purple-700 dark:text-purple-300">
                            MICRO 模式无需外链地址, 保持留空即可
                          </FormDescription>
                        )}
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                  <FormField
                    control={form.control}
                    name="linkTarget"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>打开方式</FormLabel>
                        <Select onValueChange={field.onChange} value={field.value ?? "BLANK"}>
                          <FormControl>
                            <SelectTrigger className="w-full">
                              <SelectValue />
                            </SelectTrigger>
                          </FormControl>
                          <SelectContent>
                            <SelectItem value="BLANK">新窗口打开</SelectItem>
                            <SelectItem value="SELF">内部嵌入</SelectItem>
                            <SelectItem value="MICRO">MICRO · 微前端路由</SelectItem>
                          </SelectContent>
                        </Select>
                        {linkTargetValue === "MICRO" ? (
                          <FormDescription className="text-purple-700 dark:text-purple-300">
                            菜单 path 填主应用路由 /micro/{"{"}appCode{"}"}, 由微前端容器加载该应用的子应用渲染
                          </FormDescription>
                        ) : (
                          <FormDescription>内部嵌入将在应用内 iframe 中加载</FormDescription>
                        )}
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                </div>
              )}

              {menuType !== "BUTTON" && (
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
                        <FormDescription>从图标库中选择,展示于导航菜单</FormDescription>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
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
                </div>
              )}

              {menuType === "BUTTON" && (
                <div className="grid gap-4 sm:grid-cols-2">
                  <FormField
                    control={form.control}
                    name="permission"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>
                          权限码 <span className="text-destructive">*</span>
                        </FormLabel>
                        <FormControl>
                          <Input placeholder="如 system:user" className="font-mono" {...field} />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
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
                </div>
              )}

              {menuType !== "BUTTON" && (
                <FormField
                  control={form.control}
                  name="permission"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>权限码</FormLabel>
                      <FormControl>
                        <Input placeholder="选填,如 system:user" className="font-mono" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              )}

              <FormField
                control={form.control}
                name="visible"
                render={({ field }) => (
                  <FormItem className="flex flex-row items-center justify-between rounded-lg border p-3">
                    <div>
                      <FormLabel>是否可见</FormLabel>
                      <FormDescription>隐藏的菜单仍可被授权与访问,仅不在导航中显示</FormDescription>
                    </div>
                    <FormControl>
                      <Switch checked={field.value} onCheckedChange={field.onChange} aria-label="是否可见" />
                    </FormControl>
                  </FormItem>
                )}
              />
            </form>
          </Form>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDialogOpen(false)} disabled={saveMutation.isPending}>
              取消
            </Button>
            <Button type="submit" form="menu-form" disabled={saveMutation.isPending}>
              {saveMutation.isPending && <Loader2 className="size-4 animate-spin" />}
              {editing ? "保存更改" : "创建"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={!!deleting}
        onOpenChange={(o) => !o && setDeleting(null)}
        title={`删除菜单「${deleting?.title ?? ""}」?`}
        description={
          <>
            将<b>级联删除其全部子菜单</b>及各层级已分配的授权,此操作不可撤销。
          </>
        }
        loading={deleteMutation.isPending}
        onConfirm={() => deleting && deleteMutation.mutate(deleting.id)}
      />
    </div>
  )
}

function findNode(nodes: MenuNode[], id: string): MenuNode | null {
  for (const n of nodes) {
    if (n.id === id) return n
    const sub = n.children?.length ? findNode(n.children, id) : null
    if (sub) return sub
  }
  return null
}

/* ============ 树编辑器 ============ */

function MenuTreeEditor({
  nodes,
  onAdd,
  onEdit,
  onDelete,
  onMove,
  moving,
}: {
  nodes: MenuNode[]
  onAdd: (parentId: string) => void
  onEdit: (n: MenuNode) => void
  onDelete: (n: MenuNode) => void
  onMove: (a: MenuNode, b: MenuNode) => void
  moving: boolean
}) {
  return (
    <ul role="tree" className="space-y-0.5">
      {nodes.map((n) => (
        <MenuRow
          key={n.id}
          node={n}
          siblings={nodes}
          depth={0}
          onAdd={onAdd}
          onEdit={onEdit}
          onDelete={onDelete}
          onMove={onMove}
          moving={moving}
        />
      ))}
    </ul>
  )
}

function MenuRow({
  node,
  siblings,
  depth,
  onAdd,
  onEdit,
  onDelete,
  onMove,
  moving,
}: {
  node: MenuNode
  siblings: MenuNode[]
  depth: number
  onAdd: (parentId: string) => void
  onEdit: (n: MenuNode) => void
  onDelete: (n: MenuNode) => void
  onMove: (a: MenuNode, b: MenuNode) => void
  moving: boolean
}) {
  const [open, setOpen] = React.useState(true)
  const children = node.children ?? []
  const hasChildren = children.length > 0
  const idx = siblings.findIndex((s) => s.id === node.id)
  const typeMeta = MENU_TYPE_META[node.type]

  return (
    <li role="treeitem" aria-expanded={hasChildren ? open : undefined}>
      <div
        className={cn(
          "group relative flex items-center gap-2 rounded-lg border border-transparent py-2 pr-2 transition-colors hover:border-border hover:bg-muted/50",
          depth > 0 && "border-l border-border/60"
        )}
        style={{ paddingLeft: 4 + depth * 20 }}
      >
        {hasChildren ? (
          <button
            type="button"
            aria-label={open ? "折叠" : "展开"}
            onClick={() => setOpen((o) => !o)}
            className="flex size-5 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-muted"
          >
            {open ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
          </button>
        ) : (
          <span className="w-5 shrink-0" />
        )}

        <AppIcon name={node.icon} className="size-4 shrink-0 text-muted-foreground" />

        <span className="min-w-0 truncate text-sm font-medium">{node.title}</span>

        {node.linkUrl && (
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="flex shrink-0 items-center text-muted-foreground" aria-label="外链菜单">
                <ExternalLink className="size-3" />
              </span>
            </TooltipTrigger>
            <TooltipContent className="max-w-[20rem]">
              <span className="block font-mono text-xs">{node.linkUrl}</span>
            </TooltipContent>
          </Tooltip>
        )}

        <span className={cn("shrink-0 rounded border px-1.5 py-px text-[10px] leading-4", typeMeta?.className)}>
          {typeMeta?.label ?? node.type}
        </span>

        {node.linkTarget === "MICRO" && (
          <span className="shrink-0 rounded border border-purple-500/30 bg-purple-500/10 px-1.5 py-px text-[10px] leading-4 text-purple-700 dark:text-purple-300">
            MICRO
          </span>
        )}

        {node.permission && (
          <code className="hidden shrink-0 rounded bg-muted px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground md:inline">
            {node.permission}
          </code>
        )}
        {node.path && (
          <code className="ml-auto hidden shrink-0 font-mono text-xs text-muted-foreground/70 lg:inline">
            {node.path}
          </code>
        )}
        <span className="ml-auto flex shrink-0 items-center gap-1 text-xs text-muted-foreground/60 lg:ml-2">
          <span className="tabular-nums">#{node.sortOrder ?? 0}</span>
          {node.visible ? <Eye className="size-3.5" /> : <EyeOff className="size-3.5 text-muted-foreground/40" />}
        </span>

        {/* 操作按钮组 */}
        <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="ghost" size="icon" className="size-8" aria-label="新增子项" onClick={() => onAdd(node.id)}>
                <Plus className="size-3.5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>新增子项</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="size-8"
                aria-label="上移"
                disabled={idx <= 0 || moving}
                onClick={() => onMove(node, siblings[idx - 1])}
              >
                <ArrowUp className="size-3.5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>上移(交换排序)</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="size-8"
                aria-label="下移"
                disabled={idx >= siblings.length - 1 || moving}
                onClick={() => onMove(node, siblings[idx + 1])}
              >
                <ArrowDown className="size-3.5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>下移(交换排序)</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="ghost" size="icon" className="size-8" aria-label="编辑" onClick={() => onEdit(node)}>
                <Pencil className="size-3.5" />
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
                onClick={() => onDelete(node)}
              >
                <Trash2 className="size-3.5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>删除(级联子菜单)</TooltipContent>
          </Tooltip>
        </div>
      </div>

      {hasChildren && open && (
        <ul role="group" className="space-y-0.5">
          {children.map((c) => (
            <MenuRow
              key={c.id}
              node={c}
              siblings={children}
              depth={depth + 1}
              onAdd={onAdd}
              onEdit={onEdit}
              onDelete={onDelete}
              onMove={onMove}
              moving={moving}
            />
          ))}
        </ul>
      )}
    </li>
  )
}
