import * as React from "react"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "react-router-dom"
import { ArrowRight, Check, CheckCircle2, Circle, FileCheck2, ListTree, Loader2, PackageOpen, RefreshCw, Rocket, ShieldCheck } from "lucide-react"
import { applicationSetupApi, type ApplicationAccessTarget } from "@/api/application-setup"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import { ErrorState } from "@/components/states"
import { hostEsmEntryForCode } from "@/micro-apps/registry"
import { cn } from "@/lib/utils"
import type { ApplicationEnvironment, ApplicationRecord } from "@/api/types"
import { ApplicationAccessSetup } from "./ApplicationAccessSetup"
import { ApplicationReleasesPanel } from "./ApplicationReleasesPanel"
import { applicationSetupChecks, type ApplicationSetupStep } from "./application-setup-policy"

type Props = {
  application: ApplicationRecord
  environment: ApplicationEnvironment
  step: ApplicationSetupStep
  onEnvironmentChange: (environment: ApplicationEnvironment) => void
  onStepChange: (step: ApplicationSetupStep) => void
  onClose: () => void
  onEdit: () => void
}

const environments: { value: ApplicationEnvironment; label: string }[] = [
  { value: "development", label: "开发环境" }, { value: "staging", label: "预发环境" }, { value: "production", label: "生产环境" },
]

export function ApplicationSetupPanel({ application, environment, step, onEnvironmentChange, onStepChange, onClose, onEdit }: Props) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [releaseBusy, setReleaseBusy] = React.useState(false)
  const [accessBusy, setAccessBusy] = React.useState(false)
  const [selectedTarget, setSelectedTarget] = React.useState<ApplicationAccessTarget | null>(null)
  const busy = releaseBusy || accessBusy
  const statusQuery = useQuery({
    queryKey: ["application-setup", application.id, environment, application.status, application.entryType, application.entryUrl],
    queryFn: () => applicationSetupApi.status(application.id, environment),
    retry: 1,
    staleTime: 0,
  })
  const status = statusQuery.isSuccess && !statusQuery.isPlaceholderData
    && statusQuery.data.applicationId === application.id && statusQuery.data.environment === environment ? statusQuery.data : null
  const targetStatusQuery = useQuery({
    queryKey: ["application-setup-target", application.id, environment, selectedTarget?.tenantId ?? "", selectedTarget?.organizationId ?? "", selectedTarget?.roleId ?? ""],
    queryFn: () => applicationSetupApi.status(application.id, environment, selectedTarget ?? undefined),
    enabled: !!selectedTarget && !!status?.canConfigureAccess,
    retry: 1,
    staleTime: 0,
  })
  const targetStatus = status?.canConfigureAccess && selectedTarget && targetStatusQuery.isSuccess && !targetStatusQuery.isPlaceholderData
    && targetStatusQuery.data.applicationId === application.id && targetStatusQuery.data.environment === environment
    && targetStatusQuery.data.access?.tenantId === selectedTarget.tenantId
    && targetStatusQuery.data.access.organizationId === selectedTarget.organizationId
    && targetStatusQuery.data.access.roleId === selectedTarget.roleId ? targetStatusQuery.data : null
  const hostEntry = hostEsmEntryForCode(application.code)
  const hostModuleAvailable = !!hostEntry && application.entryUrl?.trim() === hostEntry
  const checks = status ? applicationSetupChecks(application, { ...status, access: targetStatus?.access ?? null }, hostModuleAvailable)
    .map(check => check.id === "access" && selectedTarget && !targetStatus
      ? { ...check, complete: false, description: !status.canConfigureAccess ? "请由授权管理员核对所选访问范围。" : targetStatusQuery.isError ? "当前访问范围检查失败，请刷新后重试。" : "正在检查所选租户、组织和角色的实际授权…" }
      : check) : []
  const entryReady = checks.find(check => check.id === "release")?.complete ?? false
  const html = application.entryType === "HTML"
  const steps = [
    { id: "overview" as const, title: "接入检查", detail: "查看还缺哪些配置", icon: FileCheck2 },
    { id: "release" as const, title: html ? "发布版本" : "宿主模块", detail: html ? "上传包或使用已有地址" : "检查已构建的模块", icon: PackageOpen },
    { id: "access" as const, title: "访问范围", detail: "选择组织、角色与菜单", icon: ShieldCheck },
  ]
  const refresh = React.useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: ["application-setup", application.id] })
    void queryClient.invalidateQueries({ queryKey: ["application-setup-target", application.id] })
  }, [queryClient, application.id])
  const updateTarget = React.useCallback((next: ApplicationAccessTarget | null) => {
    setSelectedTarget(previous => previous?.tenantId === next?.tenantId && previous?.organizationId === next?.organizationId && previous?.roleId === next?.roleId ? previous : next)
  }, [])
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
          <SheetDescription>发布应用，检查页面，再明确谁可以访问。所有步骤都可稍后继续。</SheetDescription>
          <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
            <div className="flex items-center gap-2"><Badge variant="outline" className="font-mono">{application.code}</Badge><Badge variant="secondary">{html ? "独立 HTML 应用" : "第一方宿主模块"}</Badge></div>
            <Select value={environment} onValueChange={value => { if (!busy) onEnvironmentChange(value as ApplicationEnvironment) }} disabled={busy}>
              <SelectTrigger className="w-36" aria-label="接入环境"><SelectValue /></SelectTrigger>
              <SelectContent>{environments.map(item => <SelectItem key={item.value} value={item.value}>{item.label}</SelectItem>)}</SelectContent>
            </Select>
          </div>
        </SheetHeader>
        <nav className="grid shrink-0 grid-cols-3 border-b bg-muted/20" aria-label="应用接入步骤">
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
              <div><h3 className="font-semibold">应用已创建，接着完成以下配置</h3><p className="mt-1 text-sm text-muted-foreground">检查结果来自当前环境。访问范围需按具体租户、组织和角色核对。</p></div>
              <Button variant="ghost" size="icon" aria-label="刷新接入检查" disabled={statusQuery.isFetching || targetStatusQuery.isFetching || busy} onClick={refresh}><RefreshCw className={cn("size-4", (statusQuery.isFetching || targetStatusQuery.isFetching) && "animate-spin")} /></Button>
            </div>
            {statusQuery.isError ? <ErrorState message={statusQuery.error.message} onRetry={() => statusQuery.refetch()} /> : !status ? <div role="status" className="flex items-center gap-2 py-10 text-sm text-muted-foreground"><Loader2 className="size-4 animate-spin" />正在检查应用配置…</div> : <div className="overflow-hidden rounded-xl border">
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
            <div className="flex justify-end"><Button disabled={busy} onClick={() => go(entryReady ? "access" : "release")}>{entryReady ? "配置访问范围" : html ? "继续发布版本" : "检查宿主模块"}<ArrowRight className="size-4" /></Button></div>
          </div>}
          {step === "release" && (html ? <ApplicationReleasesPanel application={application} open onOpenChange={() => {}} embedded environment={environment}
            onEnvironmentChange={onEnvironmentChange} onBusyChange={setReleaseBusy} onDeploymentChanged={refresh} onContinue={() => go("access")} /> : <div className="space-y-5">
            <div className="rounded-xl border p-5"><p className="flex items-center gap-2 font-medium"><Check className="size-5 text-primary" />第一方模块随宿主发布</p>
              <p className="mt-2 text-sm leading-relaxed text-muted-foreground">此应用无需上传独立版本包。当前宿主需要包含已选择的模块，更新模块时发布宿主版本。</p>
              <p className="mt-3 rounded-md bg-muted px-3 py-2 font-mono text-xs">{hostEsmEntryForCode(application.code) ?? "当前宿主未包含此模块"}</p>
              <p className={cn("mt-3 text-sm", entryReady ? "text-emerald-700 dark:text-emerald-400" : "text-muted-foreground")}>{statusQuery.isError ? "模块检查未完成，请返回接入检查后重试。" : entryReady ? "当前模块和入口检查通过，可以继续配置访问范围。" : "请先确认模块已随当前宿主构建，且应用入口配置有效。"}</p>
            </div><div className="flex justify-end"><Button onClick={() => go("access")} disabled={busy}>继续配置访问范围<ArrowRight className="size-4" /></Button></div>
          </div>)}
          {step === "access" && (statusQuery.isError ? <ErrorState message={statusQuery.error.message} onRetry={() => statusQuery.refetch()} /> : !status ? <div role="status" className="flex items-center gap-2 py-10 text-sm text-muted-foreground"><Loader2 className="size-4 animate-spin" />正在读取配置权限…</div> : <ApplicationAccessSetup key={`${application.id}:${environment}`} application={application} environment={environment} canConfigureAccess={status.canConfigureAccess}
            entryReady={entryReady} initialTarget={selectedTarget} onTargetChange={updateTarget} onConfigured={refresh} onBusyChange={setAccessBusy} />)}
        </div>
        <div className="flex shrink-0 items-center justify-between gap-3 border-t bg-muted/10 px-5 py-3">
          <p role={busy ? "status" : undefined} className="text-xs text-muted-foreground">{busy ? "正在处理，请等待当前操作完成。" : "进度由实际配置保存，可随时从应用卡片继续接入。"}</p>
          <Button variant="outline" size="sm" disabled={busy} onClick={onClose}>稍后继续</Button>
        </div>
      </SheetContent>
    </Sheet>
  )
}
