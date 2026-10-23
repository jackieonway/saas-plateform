import { ApplicationEditorDialog } from "./ApplicationEditorDialog"
import { useApplicationEditor, isMicroApp } from "./use-application-editor"
import * as React from "react"
import { useNavigate, useSearchParams } from "react-router-dom"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { ArrowRight, CheckCircle2, LayoutGrid, ListTree, MoreHorizontal, Pencil, Plus, Rocket, Trash2 } from "lucide-react"
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
import { PageHeader } from "@/components/layout/PageHeader"
import { StatusBadge } from "@/components/StatusBadge"
import { ConfirmDialog } from "@/components/ConfirmDialog"
import { CardGridSkeleton, EmptyState, ErrorState } from "@/components/states"
import { applicationsApi } from "@/api/applications"
import { asArray } from "@/lib/api"
import { AppIcon } from "@/lib/icons"
import { toast } from "sonner"
import { ApplicationSetupPanel } from "./ApplicationSetupPanel"
import { setupEnvironment, setupStep, isQiankunApp, type ApplicationSetupStep } from "./application-setup-policy"
import type { ApplicationRecord } from "@/api/types"

export default function ApplicationsPage() {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [params, setParams] = useSearchParams()

  const listQuery = useQuery({
    queryKey: ["applications"],
    queryFn: async () => asArray<ApplicationRecord>(await applicationsApi.list()),
    staleTime: 30_000,
    retry: 1,
  })

  const [createdApplication, setCreatedApplication] = React.useState<ApplicationRecord | null>(null)
  const openSetup = (application: ApplicationRecord, step: ApplicationSetupStep = "overview") => {
    setCreatedApplication(application)
    setParams(previous => {
      const next = new URLSearchParams(previous)
      next.set("setup", application.id)
      next.set("step", step)
      return next
    })
  }
  const editor = useApplicationEditor(application => openSetup(application, "release"))
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
  const setupId = params.get("setup")
  const setupApplication = apps.find(application => application.id === setupId)
    ?? (createdApplication?.id === setupId ? createdApplication : null)
  const setSetupParam = (key: "step" | "environment", value: string) => setParams(previous => {
    const next = new URLSearchParams(previous)
    next.set(key, value)
    return next
  }, { replace: true })
  const closeSetup = () => {
    setCreatedApplication(null)
    setParams(previous => {
      const next = new URLSearchParams(previous)
      for (const key of ["setup", "step", "environment"]) next.delete(key)
      return next
    }, { replace: true })
  }

  return (
    <div>
      <PageHeader
        title="应用管理"
        description="接入 qiankun 微应用，按环境配置 HTTP/HTTPS 入口与访问范围"
        icon={<LayoutGrid className="size-5" />}
        actions={
          <Button onClick={openCreate}>
            <Plus className="size-4" />
            新建微应用
          </Button>
        }
      />

      <OnboardingGuide onCreate={openCreate} />

      {setupId && listQuery.isSuccess && !setupApplication && <div className="mb-5"><ErrorState message="要继续接入的应用不存在，或已不在当前管理范围。" onRetry={closeSetup} /></div>}

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
                  <Badge variant={isQiankunApp(a) ? "outline" : "destructive"} className="text-[11px]">{isQiankunApp(a) ? "qiankun" : "需重新配置"}</Badge>
                ) : (
                  <Badge variant="outline" className="text-[11px]">宿主内置</Badge>
                )}
                {/* 内置页面不使用微应用入口；独立微应用显示实际入口通道。 */}
                {isMicroApp(a) && isQiankunApp(a) ? (
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
                      <span className="block text-xs">HTML 入口 · qiankun 生命周期</span>
                      <span className="mt-0.5 block font-mono text-xs break-all">{a.entryUrl ?? "(未配置)"}</span>
                    </TooltipContent>
                  </Tooltip>
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
              {isMicroApp(a) && (
                <Button variant="outline" size="sm" className="w-full justify-between" onClick={() => openSetup(a)}>
                  <span className="flex items-center gap-2"><Rocket className="size-3.5" />继续接入 / 检查配置</span><ArrowRight className="size-3.5" />
                </Button>
              )}
              {isMicroApp(a) && !isQiankunApp(a) && <p className="text-xs leading-relaxed text-muted-foreground">请重新配置 qiankun 入口，当前应用不会运行。</p>}
              {isMicroApp(a) && (
                <Button variant="ghost" size="sm" className="w-full text-muted-foreground" onClick={() => openSetup(a, "release")}>
                  <Rocket className="size-3.5" />
                  配置入口 / 发布版本
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
                    {isMicroApp(a) && (
                      <DropdownMenuItem onClick={() => openSetup(a, "release")}>
                        <Rocket className="size-4" />
                        配置入口 / 发布版本
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

      {setupApplication && (
        <ApplicationSetupPanel
          key={setupApplication.id}
          application={setupApplication}
          environment={setupEnvironment(params.get("environment"))}
          step={setupStep(params.get("step"))}
          onEnvironmentChange={value => setSetupParam("environment", value)}
          onStepChange={value => setSetupParam("step", value)}
          onClose={closeSetup}
          onEdit={() => openEdit(setupApplication)}
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

/** Keep the entry concise; detailed guidance follows the selected application. */
function OnboardingGuide({ onCreate }: { onCreate: () => void }) {
  return (
    <div className="mb-6 flex flex-col justify-between gap-4 rounded-xl border bg-muted/20 p-5 sm:flex-row sm:items-center">
      <div>
        <p className="flex items-center gap-2 text-sm font-semibold"><CheckCircle2 className="size-4 text-primary" />从一个首页开始接入</p>
        <p className="mt-2 text-sm leading-relaxed text-muted-foreground">名称与编码 → 配置应用入口 → 配置访问范围</p>
        <p className="mt-1 text-xs leading-relaxed text-muted-foreground">使用 qiankun，路径和首页自动生成。按环境填写入口地址，也可上传构建包发布版本。</p>
      </div>
      <Button variant="outline" className="shrink-0" onClick={onCreate}>开始接入<ArrowRight className="size-4" /></Button>
    </div>
  )
}
