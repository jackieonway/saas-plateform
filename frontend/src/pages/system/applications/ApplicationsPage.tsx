import { ApplicationEditorDialog } from "./ApplicationEditorDialog"
import { useApplicationEditor, isMicroApp, isHostEsm } from "./use-application-editor"
import * as React from "react"
import { Link, useNavigate } from "react-router-dom"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { ArrowRight, ChevronDown, ChevronRight, LayoutGrid, ListTree, MoreHorizontal, Pencil, Plus, Rocket, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { PageHeader } from "@/components/layout/PageHeader"
import { StatusBadge } from "@/components/StatusBadge"
import { ConfirmDialog } from "@/components/ConfirmDialog"
import { CardGridSkeleton, EmptyState, ErrorState } from "@/components/states"
import { applicationsApi } from "@/api/applications"
import { asArray } from "@/lib/api"
import { AppIcon } from "@/lib/icons"
import { toast } from "sonner"
import { cn } from "@/lib/utils"
import { ApplicationReleasesPanel } from "./ApplicationReleasesPanel"
import type { ApplicationRecord } from "@/api/types"

export default function ApplicationsPage() {
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  const listQuery = useQuery({
    queryKey: ["applications"],
    queryFn: async () => asArray<ApplicationRecord>(await applicationsApi.list()),
    staleTime: 30_000,
    retry: 1,
  })

  const [releasesApplication, setReleasesApplication] = React.useState<ApplicationRecord | null>(null)
  const editor = useApplicationEditor(setReleasesApplication)
  const { openCreate, openEdit } = editor

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
        description="选择宿主 ESM 或上传 HTML 版本包，再配置菜单与授权"
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
                {isMicroApp(a) ? (
                  <Badge variant="outline" className="text-[11px]">{a.entryType === "ESM" ? "宿主 ESM" : "iframe 隔离"}</Badge>
                ) : (
                  <Badge variant="outline" className="text-[11px]">宿主内置</Badge>
                )}
                {/* 内置页面不使用微应用入口；独立微应用显示实际入口通道。 */}
                {isMicroApp(a) && a.entryType === "HTML" ? (
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
                      <span className="block text-xs">HTML 入口 · iframe 隔离执行</span>
                      <span className="mt-0.5 block font-mono text-xs break-all">{a.entryUrl ?? "(未配置)"}</span>
                    </TooltipContent>
                  </Tooltip>
                ) : isMicroApp(a) ? (
                  <span className="rounded-full border bg-muted px-2 py-px text-[10px] font-medium text-muted-foreground">
                    ESM
                  </span>
                ) : null}
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
              {isHostEsm(a) ? (
                <div className="rounded-md border border-dashed bg-muted/30 px-3 py-2 text-center text-xs text-muted-foreground">
                  随宿主发布 · 使用宿主构建版本
                </div>
              ) : isMicroApp(a) && (
                <Button variant="outline" size="sm" className="w-full" onClick={() => setReleasesApplication(a)}>
                  <Rocket className="size-3.5" />
                  发布版本
                </Button>
              )}

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
                    {isMicroApp(a) && !isHostEsm(a) && (
                      <DropdownMenuItem onClick={() => setReleasesApplication(a)}>
                        <Rocket className="size-4" />
                        发布版本
                      </DropdownMenuItem>
                    )}
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
      <ApplicationEditorDialog editor={editor} />

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
  { title: "登记应用", desc: "ESM 随宿主发布；HTML 按环境上传、预检并激活", to: null },
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
