import * as React from "react"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "react-router-dom"
import { ArrowRight, CheckCircle2, Circle, FileCheck2, ListTree, Loader2, PackageOpen, RefreshCw, Rocket } from "lucide-react"
import { applicationSetupApi } from "@/api/application-setup"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import { ErrorState } from "@/components/states"
import { cn } from "@/lib/utils"
import type { ApplicationRecord } from "@/api/types"
import { ApplicationReleasesPanel } from "./ApplicationReleasesPanel"
import { applicationSetupChecks, isQiankunApp, type ApplicationSetupStep } from "./application-setup-policy"

type Props = {
  application: ApplicationRecord
  step: ApplicationSetupStep
  onStepChange: (step: ApplicationSetupStep) => void
  onClose: () => void
  onEdit: () => void
}

export function ApplicationSetupPanel({ application, step, onStepChange, onClose, onEdit }: Props) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [releaseBusy, setReleaseBusy] = React.useState(false)
  const busy = releaseBusy
  const statusQuery = useQuery({
    queryKey: ["application-setup", application.id, "runtime", application.status, application.entryType, application.runtimeMode, application.entryUrl],
    queryFn: () => applicationSetupApi.status(application.id),
    retry: 1,
    staleTime: 0,
  })
  const status = statusQuery.isSuccess && !statusQuery.isPlaceholderData
    && statusQuery.data.applicationId === application.id ? statusQuery.data : null
  const environment = status?.environment
  const checks = status ? applicationSetupChecks(application, status) : []
  const entryReady = checks.find(check => check.id === "release")?.complete ?? false
  const steps = [
    { id: "overview" as const, title: "接入检查", detail: "查看还缺哪些配置", icon: FileCheck2 },
    { id: "release" as const, title: "应用入口", detail: "填写已部署地址", icon: PackageOpen },
  ]
  const refresh = React.useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: ["application-setup", application.id] })
  }, [queryClient, application.id])
  const go = (next: ApplicationSetupStep) => { if (!busy) onStepChange(next) }
  const editMenus = () => {
    if (!busy) navigate(`/system/menus?applicationId=${encodeURIComponent(application.id)}`)
  }
  return (
    <Sheet open onOpenChange={open => { if (!open && !busy) onClose() }}>
      <SheetContent className="w-full gap-0 sm:max-w-4xl" onEscapeKeyDown={event => { if (busy) event.preventDefault() }} onInteractOutside={event => { if (busy) event.preventDefault() }}>
        <SheetHeader className="shrink-0 border-b p-5 pr-12">
          <div className="flex items-center gap-2 text-xs font-medium text-muted-foreground"><Rocket className="size-4" />微应用接入</div>
          <SheetTitle className="mt-1 text-xl">{application.name}</SheetTitle>
          <SheetDescription>配置应用入口，检查页面是否就绪。</SheetDescription>
          <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
            <div className="flex items-center gap-2"><Badge variant="outline" className="font-mono">{application.code}</Badge><Badge variant={isQiankunApp(application) ? "secondary" : "destructive"}>{isQiankunApp(application) ? "qiankun 微应用" : "需重新配置 qiankun 入口"}</Badge></div>
          </div>
        </SheetHeader>
        <nav className="grid shrink-0 grid-cols-2 border-b bg-muted/20" aria-label="应用接入步骤">
          {steps.map((item, index) => <button type="button" key={item.id} aria-current={step === item.id ? "step" : undefined} disabled={busy}
            className={cn("flex min-w-0 items-center gap-2.5 border-b-2 px-3 py-4 text-left transition-colors disabled:cursor-wait sm:px-5", step === item.id ? "border-primary bg-primary/5" : "border-transparent text-muted-foreground hover:bg-muted/50")}
            onClick={() => go(item.id)}>
            <span className={cn("flex size-7 shrink-0 items-center justify-center rounded-full border text-xs font-semibold tabular-nums", step === item.id && "border-primary bg-primary text-primary-foreground")}>{index + 1}</span>
            <span className="min-w-0"><span className="block truncate text-sm font-medium">{item.title}</span><span className="mt-0.5 hidden truncate text-xs sm:block">{item.detail}</span></span>
          </button>)}
        </nav>
        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain p-5">
          {step === "overview" && <div className="space-y-5">
            <div className="flex items-start justify-between gap-3">
              <div><h3 className="font-semibold">应用已创建，接着完成以下配置</h3><p className="mt-1 text-sm text-muted-foreground">检查结果来自当前环境。访问权限统一在租户、组织和角色授权中管理。</p></div>
              <Button variant="ghost" size="icon" aria-label="刷新接入检查" disabled={statusQuery.isFetching || busy} onClick={refresh}><RefreshCw className={cn("size-4", statusQuery.isFetching && "animate-spin")} /></Button>
            </div>
            {statusQuery.isError ? <ErrorState message={statusQuery.error.message} onRetry={() => statusQuery.refetch()} /> : !status || !environment ? <div role="status" className="flex items-center gap-2 py-10 text-sm text-muted-foreground"><Loader2 className="size-4 animate-spin" />正在检查应用配置…</div> : <div className="overflow-hidden rounded-xl border">
              {checks.map(check => <div key={check.id} className="flex items-start gap-3 border-b p-4 last:border-0">
                {check.complete ? <CheckCircle2 className="mt-0.5 size-5 shrink-0 text-emerald-600 dark:text-emerald-400" /> : <Circle className="mt-0.5 size-5 shrink-0 text-muted-foreground/60" />}
                <div className="min-w-0 flex-1"><p className="text-sm font-medium">{check.title}</p><p className="mt-1 text-xs leading-relaxed text-muted-foreground">{check.description}</p></div>
                {check.action && <Button variant="ghost" size="sm" disabled={busy} onClick={() => check.action === "menus" ? editMenus() : go(check.action as ApplicationSetupStep)}>{check.complete ? "查看" : "去配置"}<ArrowRight className="size-3.5" /></Button>}
              </div>)}
            </div>}
            <div className="rounded-lg border border-dashed bg-muted/20 p-4">
              <p className="flex items-center gap-2 text-sm font-medium"><ListTree className="size-4" />首页与更多页面</p>
              <p className="mt-1.5 text-xs leading-relaxed text-muted-foreground">{status && status.entryMenuCount > 0
                ? "应用已有可访问页面。需要子页面或操作按钮时，可进入菜单管理；返回本页继续接入。"
                : "先配置一个可访问的首页，再按需增加子页面或操作按钮。返回本页可继续接入。"}</p>
              <div className="mt-3 flex flex-wrap gap-2"><Button size="sm" variant="outline" onClick={editMenus} disabled={busy}>管理应用菜单</Button><Button size="sm" variant="ghost" onClick={onEdit} disabled={busy}>编辑应用信息</Button></div>
            </div>
            <div className="flex justify-end"><Button disabled={busy} onClick={() => go("release")}>{entryReady ? "查看应用入口" : "继续配置入口"}<ArrowRight className="size-4" /></Button></div>
          </div>}
          {step === "release" && (statusQuery.isError ? <ErrorState message={statusQuery.error.message} onRetry={() => statusQuery.refetch()} /> : !environment ? <div role="status" className="flex items-center gap-2 py-10 text-sm text-muted-foreground"><Loader2 className="size-4 animate-spin" />正在读取应用配置…</div> : <ApplicationReleasesPanel application={application} open onOpenChange={() => {}} embedded environment={environment}
            onBusyChange={setReleaseBusy} onDeploymentChanged={refresh} onContinue={() => go("overview")} />)}

        </div>
        <div className="flex shrink-0 items-center justify-between gap-3 border-t bg-muted/10 px-5 py-3">
          <p role={busy ? "status" : undefined} className="text-xs text-muted-foreground">{busy ? "正在处理，请等待当前操作完成。" : "进度由实际配置保存，可随时从应用卡片继续接入。"}</p>
          <Button variant="outline" size="sm" disabled={busy} onClick={onClose}>稍后继续</Button>
        </div>
      </SheetContent>
    </Sheet>
  )
}
