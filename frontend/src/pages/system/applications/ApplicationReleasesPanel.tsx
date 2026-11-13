import * as React from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { ArrowRight, CheckCircle2, FileCheck2, Loader2, RefreshCw, Rocket, RotateCcw, Trash2, XCircle } from "lucide-react"
import { toast } from "sonner"
import { applicationsApi } from "@/api/applications"
import type { ApplicationEnvironment, ApplicationRecord, ApplicationReleasePayload, ApplicationReleaseRecord } from "@/api/types"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { EmptyState, ErrorState } from "@/components/states"
import { cn } from "@/lib/utils"
import { ExternalReleaseForm } from "./ExternalReleaseForm"
import { ZipDistributionPanel } from "./ZipDistributionPanel"
import { ManagedDeploymentPanel } from "./ManagedDeploymentPanel"
import { usePermissionsQuery } from "@/hooks/use-auth-data"
import { ApplicationUrlEntryForm } from "./ApplicationUrlEntryForm"
import { isQiankunApp } from "./application-setup-policy"

type ReleaseTarget = { applicationId: string; environment: ApplicationEnvironment }
type DeploymentAction = (
  | { kind: "activate"; release: ApplicationReleaseRecord }
  | { kind: "rollback"; environment: ApplicationEnvironment; version: string }
) & { target: ReleaseTarget }
type PanelProps = {
  application: ApplicationRecord
  open: boolean
  onOpenChange: (open: boolean) => void
  embedded?: boolean
  onDeploymentChanged?: () => void
  onContinue?: () => void
  onBusyChange?: (busy: boolean) => void
  environment: ApplicationEnvironment
}

function dateLabel(value: string | null | undefined) {
  if (!value) return "尚未预检"
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString("zh-CN", { hour12: false })
}

function sizeLabel(bytes: number | null | undefined) {
  if (bytes == null) return "—"
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`
}

const CHECK_KIND_LABELS: Record<string, string> = {
  ENTRY: "入口", HEALTH: "健康", SCRIPT: "脚本", STYLE: "样式", ASSET: "资源", POLICY: "策略", LIMIT: "限制",
}

function ValidationBadge({ release }: { release: ApplicationReleaseRecord }) {
  if (release.validationStatus === "PASSED") {
    return <Badge variant="outline" className="gap-1 border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"><CheckCircle2 className="size-3" />预检通过</Badge>
  }
  if (release.validationStatus === "FAILED") {
    return <Badge variant="destructive" className="gap-1"><XCircle className="size-3" />预检失败</Badge>
  }
  return <Badge variant="secondary">待预检</Badge>
}

export function ApplicationReleasesPanel(props: PanelProps) {
  // An application change gets its own form and mutation observers.
  return <ApplicationReleasesPanelContent key={props.application.id} {...props} />
}

function ApplicationReleasesPanelContent({ application, open, onOpenChange, embedded = false, onDeploymentChanged, onContinue, onBusyChange, environment }: PanelProps) {
  const queryClient = useQueryClient()
  const [externalFormVersion, setExternalFormVersion] = React.useState(0)
  const [deliveryOpen, setDeliveryOpen] = React.useState(false)
  const [urlEntryBusy, setUrlEntryBusy] = React.useState(false)
  const permissions = usePermissionsQuery({ enabled: open })
  const canManageDelivery = permissions.isSuccess && !!permissions.data?.permissions.some(permission =>
    permission === "system:distribution:read" || permission === "system:deployment:read")
  const [action, setAction] = React.useState<DeploymentAction | null>(null)
  const [deleteTarget, setDeleteTarget] = React.useState<{ target: ReleaseTarget; release: ApplicationReleaseRecord } | null>(null)
  const mounted = React.useRef(true)
  React.useEffect(() => { mounted.current = true; return () => { mounted.current = false } }, [])
  const currentTarget = React.useRef({ applicationId: application.id, environment, open })
  currentTarget.current = { applicationId: application.id, environment, open }
  const target = { applicationId: application.id, environment }
  const isCurrentTarget = (requested: ReleaseTarget) => mounted.current && currentTarget.current.open
    && currentTarget.current.applicationId === requested.applicationId
    && currentTarget.current.environment === requested.environment
  const releasesKey = ["application-releases", application.id, environment]
  const deploymentsKey = ["application-deployments", application.id]
  const releasesQuery = useQuery({
    queryKey: releasesKey,
    queryFn: () => applicationsApi.releases(application.id, environment),
    enabled: open,
    retry: 1,
  })
  const deploymentsQuery = useQuery({
    queryKey: deploymentsKey,
    queryFn: () => applicationsApi.deployments(application.id),
    enabled: open,
    retry: 1,
  })
  const urlEntryQuery = useQuery({
    queryKey: ["application-url-entry", application.id, environment],
    queryFn: () => applicationsApi.urlEntry(application.id, environment),
    enabled: open,
    retry: 1,
    staleTime: 0,
  })
  const urlEntryReady = urlEntryQuery.isSuccess && !urlEntryQuery.isPlaceholderData
    && (urlEntryQuery.data === null || urlEntryQuery.data?.applicationId === application.id && urlEntryQuery.data.environment === environment)
  const urlEntry = urlEntryReady ? urlEntryQuery.data ?? null : null
  const releases = releasesQuery.data ?? []
  const deployment = deploymentsQuery.data?.find((item) => item.environment === environment)
  const previousRelease = releases.find((item) => item.id === deployment?.previousReleaseId)
  const previousUnsupported = !!previousRelease && previousRelease.entryType !== "HTML"
  const enabled = application.status === "ENABLED"
  const compatible = isQiankunApp(application)

  const refreshReleaseData = (requested: ReleaseTarget = target) => Promise.all([
    queryClient.invalidateQueries({ queryKey: ["application-releases", requested.applicationId] }),
    queryClient.invalidateQueries({ queryKey: ["application-deployments", requested.applicationId] }),
    queryClient.invalidateQueries({ queryKey: ["application-url-entry", requested.applicationId, requested.environment] }),
  ])
  const refreshActiveApplication = () => Promise.all([
    queryClient.invalidateQueries({ queryKey: ["applications"] }),
    queryClient.invalidateQueries({ queryKey: ["profile"] }),
  ])

  const externalMutation = useMutation({
    mutationFn: async (requested: { target: ReleaseTarget; payload: ApplicationReleasePayload }) => {
      // A registration is durable once its response arrives. A later check
      // failure must never prompt another registration of the same version.
      const release = await applicationsApi.registerRelease(requested.target.applicationId, requested.payload)
      if (!isCurrentTarget(requested.target)) return { release, preflightError: null }
      try {
        return {
          release: await applicationsApi.preflightRelease(requested.target.applicationId, release.id),
          preflightError: null,
        }
      } catch (error) {
        return { release, preflightError: error instanceof Error ? error.message : "检查请求暂未完成" }
      }
    },
    onSuccess: ({ release, preflightError }, requested) => {
      if (!isCurrentTarget(requested.target)) return
      if (preflightError) toast.error(`版本 ${release.version} 已登记，检查暂未完成`, {
        description: `${preflightError}。版本记录已保留，请在下方重试预检，无需重新登记。`,
      })
      else if (release.validationStatus === "PASSED") toast.success(`版本 ${release.version} 已登记并检查通过`, {
        description: "已自动发现并检查入口引用的脚本与样式，确认版本后即可激活。",
      })
      else toast.error(`版本 ${release.version} 已登记，检查未通过`, {
        description: "查看下方逐项诊断，修复部署后可重新预检；无需重复登记。",
      })
      setExternalFormVersion(value => value + 1)
    },
    onError: (error: Error, requested) => { if (isCurrentTarget(requested.target)) toast.error(error.message || "URL 登记失败", { description: "请求结果不明时请先刷新版本记录核对，不要改来源或重复登记。" }) },
    onSettled: (_release, _error, requested) => refreshReleaseData(requested.target),
  })

  const preflightMutation = useMutation({
    mutationFn: (requested: { target: ReleaseTarget; releaseId: string }) => applicationsApi.preflightRelease(requested.target.applicationId, requested.releaseId),
    onSuccess: (release, requested) => {
      queryClient.setQueryData<ApplicationReleaseRecord[]>(["application-releases", requested.target.applicationId, requested.target.environment], (old) => old?.map((item) => item.id === release.id ? release : item))
      if (!isCurrentTarget(requested.target)) return
      if (release.validationStatus === "PASSED") toast.success(`版本 ${release.version} 预检通过`)
      else toast.error(`版本 ${release.version} 预检失败`, { description: "请查看逐资源诊断并核对部署；产物内容变化时使用新版本号。" })
    },
    onError: (error: Error, requested) => { if (isCurrentTarget(requested.target)) toast.error(error.message || "预检请求失败") },
    onSettled: (_release, _error, requested) => refreshReleaseData(requested.target),
  })

  const deploymentMutation = useMutation({
    mutationFn: (next: DeploymentAction) => next.kind === "activate"
      ? applicationsApi.activateRelease(next.target.applicationId, next.release.id)
      : applicationsApi.rollbackDeployment(next.target.applicationId, next.environment),
    onSuccess: async (next, requested) => {
      if (isCurrentTarget(requested.target)) {
        toast.success(requested.kind === "activate" ? `已激活版本 ${next.version}` : `已回滚至版本 ${next.version}`, {
          description: `环境：${next.environment}`,
        })
        setAction(null)
        onDeploymentChanged?.()
      }
      await refreshActiveApplication()
    },
    onError: (error: Error, requested) => {
      if (!isCurrentTarget(requested.target)) return
      toast.error(error.message || "部署切换失败", { description: "部署与预检结果已重新加载，请查看最新诊断。" })
      setAction(null)
    },
    onSettled: (_deployment, _error, requested) => refreshReleaseData(requested.target),
  })

  const deleteMutation = useMutation({
    mutationFn: (requested: { target: ReleaseTarget; release: ApplicationReleaseRecord }) => applicationsApi.removeRelease(requested.target.applicationId, requested.release.id),
    onSuccess: (_result, requested) => {
      if (!isCurrentTarget(requested.target)) return
      setDeleteTarget(null)
      toast.success(`版本 ${requested.release.version} 已删除`)
    },
    onError: (error: Error, requested) => {
      if (isCurrentTarget(requested.target)) toast.error(error.message || "版本删除失败")
    },
    onSettled: (_result, _error, requested) => refreshReleaseData(requested.target),
  })

  const busy = urlEntryBusy || externalMutation.isPending || preflightMutation.isPending || deploymentMutation.isPending || deleteMutation.isPending
  React.useEffect(() => {
    onBusyChange?.(busy)
    return () => onBusyChange?.(false)
  }, [busy, onBusyChange])
  React.useEffect(() => {
    setAction(null)
    setDeleteTarget(null)
    setDeliveryOpen(false)
  }, [environment])

  const Header = embedded ? "header" : SheetHeader
  const Title = embedded ? "h3" : SheetTitle
  const Description = embedded ? "p" : SheetDescription

  const content = (
    <>
          <Header className={cn("shrink-0 border-b", embedded ? "pb-4" : "p-5 pr-12")}>
            <Title className="flex items-center gap-2 text-base font-semibold"><Rocket className="size-5 text-primary" />配置应用入口</Title>
            <Description className="mt-1 text-sm text-muted-foreground">填写已部署应用的地址即可接入，版本管理按需配置。</Description>
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <Badge variant="outline" className="font-mono">{application.code}</Badge>
              <Badge variant={compatible ? "secondary" : "destructive"}>{compatible ? "qiankun · HTML" : "需重新配置 qiankun 入口"}</Badge>
              {!enabled && <Badge variant="destructive">应用已停用</Badge>}
            </div>
          </Header>
          <div className={cn("min-h-0 flex-1", embedded ? "pt-5" : "overflow-y-auto overscroll-contain p-5")}>
            {urlEntryQuery.isError && <div className="mb-4"><ErrorState message={urlEntryQuery.error.message} onRetry={() => urlEntryQuery.refetch()} /></div>}
            <ApplicationUrlEntryForm key={`${application.id}:${environment}`} application={application} environment={environment} entry={urlEntry} ready={urlEntryReady} busy={busy} onBusyChange={setUrlEntryBusy} onChanged={() => onDeploymentChanged?.()} />

            <details className="mb-5 rounded-xl border" aria-label="版本管理（可选）">
              <summary className="cursor-pointer px-4 py-3 text-sm font-medium">版本管理（可选） · 发布与回滚</summary>
              <div className="border-t p-4">
                <p className="mb-4 text-xs leading-relaxed text-muted-foreground">需要固定版本和回滚时，再登记版本号与已部署页面地址。激活或回滚后，当前环境将使用该版本地址。</p>
                <ExternalReleaseForm key={`${application.id}:${environment}:${externalFormVersion}`} environment={environment} busy={busy} pending={externalMutation.isPending} onSubmit={payload => { if (!busy && isCurrentTarget(target)) externalMutation.mutate({ target, payload }) }} />

            <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
              <h3 className="text-sm font-semibold">版本记录 <span className="ml-2 text-xs font-normal text-muted-foreground">{releases.length} 个版本</span></h3>
              <div className="flex items-center gap-2">
                <Button variant="ghost" size="icon" aria-label="刷新版本记录" disabled={busy || releasesQuery.isFetching || deploymentsQuery.isFetching} onClick={() => refreshReleaseData()}><RefreshCw className={cn("size-4", (releasesQuery.isFetching || deploymentsQuery.isFetching) && "animate-spin")} /></Button>
                {deployment?.previousReleaseId && (
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={busy || !enabled || !compatible || !urlEntryReady || deploymentsQuery.isError || previousRelease?.validationStatus !== "PASSED" || previousUnsupported}
                    title={!compatible ? "请先重新配置 qiankun 入口" : !enabled ? "应用停用期间不能切换部署" : previousUnsupported ? "上一版本不兼容 qiankun" : previousRelease?.validationStatus !== "PASSED" ? "上一版本需预检通过后才能回滚" : undefined}
                    onClick={() => setAction({ kind: "rollback", target, environment, version: previousRelease?.version ?? deployment.previousReleaseId! })}
                  ><RotateCcw className="size-4" />回滚{previousRelease ? `至 ${previousRelease.version}` : "至上一版本"}</Button>
                )}
              </div>
            </div>
            {deploymentsQuery.isError && <ErrorState message={deploymentsQuery.error.message} onRetry={() => deploymentsQuery.refetch()} />}

            {releasesQuery.isLoading ? <div className="flex items-center justify-center gap-2 py-12 text-sm text-muted-foreground"><Loader2 className="size-4 animate-spin" />正在读取版本…</div> : releasesQuery.isError ? (
              <ErrorState message={releasesQuery.error instanceof Error ? releasesQuery.error.message : undefined} onRetry={() => releasesQuery.refetch()} />
            ) : releases.length === 0 ? (
              <EmptyState title="等待第一个版本" description="如需固定版本，在上方登记已部署地址，检查通过后即可激活。" icon={<Rocket className="size-6" />} />
            ) : (
              <div className="space-y-3">
                {releases.map((release) => {
                  const active = compatible && !urlEntry && (deployment ? deployment.activeReleaseId === release.id : release.active)
                  const checks = Array.isArray(release.validationResult?.checks) ? release.validationResult.checks : []
                  const scriptCount = checks.filter(check => check.kind === "SCRIPT").length
                  const styleCount = checks.filter(check => check.kind === "STYLE").length
                  const preflighting = preflightMutation.isPending && preflightMutation.variables?.releaseId === release.id
                  const unsupported = release.entryType !== "HTML"
                  const protectedRelease = deployment?.activeReleaseId === release.id || deployment?.previousReleaseId === release.id
                  return (
                    <article key={release.id} className={cn("rounded-lg border p-4", active && "border-primary/40 bg-primary/5")}>
                      <div className="flex flex-wrap items-start justify-between gap-3">
                        <div className="min-w-0">
                          <div className="flex flex-wrap items-center gap-2"><h4 className="font-semibold tabular-nums">{release.version}</h4><ValidationBadge release={release} />{active && <Badge>当前部署</Badge>}<Badge variant="outline">{unsupported ? "需重新发布" : "HTML"}</Badge></div>
                          <p className="mt-2 break-all font-mono text-xs text-muted-foreground">{release.entryUrl}</p>
                        </div>
                        <div className="flex shrink-0 gap-2">
                          <Button variant="outline" size="sm" disabled={busy} onClick={() => preflightMutation.mutate({ target, releaseId: release.id })}>{preflighting ? <Loader2 className="size-4 animate-spin" /> : <FileCheck2 className="size-4" />}{release.checkedAt ? "重新预检" : "预检"}</Button>
                          <Button size="sm" disabled={busy || !enabled || !compatible || !urlEntryReady || active || release.validationStatus !== "PASSED" || unsupported || deploymentsQuery.isLoading || deploymentsQuery.isError} onClick={() => setAction({ kind: "activate", target, release })}><Rocket className="size-4" />{active ? "已激活" : "激活"}</Button>
                          <Button variant="ghost" size="sm" className="text-destructive hover:text-destructive" disabled={busy || protectedRelease || !deploymentsQuery.isSuccess} title={protectedRelease ? "当前使用或用于回滚的版本不能删除" : undefined} aria-label={`删除版本 ${release.version}`} onClick={() => setDeleteTarget({ target, release })}><Trash2 className="size-4" />删除</Button>
                        </div>
                      </div>
                      <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground"><span>登记 {dateLabel(release.createdAt)}</span><span>预检 {dateLabel(release.checkedAt)}</span>{release.checkedAt && <span>已检查 {scriptCount} 个脚本 · {styleCount} 个样式</span>}{release.resources.length > 0 && <span>{release.resources.length} 项补充资源</span>}</div>
                      {release.healthUrl && <p className="mt-1 break-all text-xs text-muted-foreground">健康检查 <span className="font-mono">{release.healthUrl}</span></p>}
                      {unsupported && <p className="mt-2 text-xs text-destructive">此版本不兼容 qiankun，请登记包含标准生命周期的 HTML 版本。</p>}
                      {checks.length > 0 && (
                        <details className="mt-3 border-t pt-3" open={release.validationStatus === "FAILED"}>
                          <summary className="cursor-pointer text-xs font-medium">逐资源预检诊断 · {checks.filter((check) => check.passed).length}/{checks.length} 通过</summary>
                          <div className="mt-3 space-y-2">
                            {checks.map((check, index) => (
                              <div key={`${check.kind}-${check.url}-${index}`} className={cn("rounded-md border p-3 text-xs", !check.passed && "border-destructive/30 bg-destructive/5")}>
                                <div className="flex flex-wrap items-center gap-2">{check.passed ? <CheckCircle2 className="size-3.5 shrink-0 text-emerald-600" /> : <XCircle className="size-3.5 shrink-0 text-destructive" />}<span className="font-medium">{CHECK_KIND_LABELS[check.kind] ?? check.kind}</span><span className="text-muted-foreground">{check.statusCode == null ? "无 HTTP 响应" : `HTTP ${check.statusCode}`}</span><span className="text-muted-foreground">{check.contentType ?? "类型未知"}</span><span className="text-muted-foreground">{sizeLabel(check.bytes)}</span></div>
                                <p className="mt-1.5 break-all font-mono text-muted-foreground">{check.url || "全局校验"}</p>
                                <p className={cn("mt-1.5 break-words leading-relaxed", !check.passed && "text-destructive")}>{check.message}</p>
                              </div>
                            ))}
                          </div>
                          <p className="mt-2 text-xs text-muted-foreground">检查时间 {dateLabel(release.validationResult?.checkedAt)}</p>
                        </details>
                      )}
                    </article>
                  )
                })}
              </div>
            )}
            {canManageDelivery && (
              <details className="mt-5 rounded-lg border" open={deliveryOpen} onToggle={event => setDeliveryOpen(event.currentTarget.open)}>
                <summary className="cursor-pointer px-4 py-3 text-sm font-medium">交付与运维</summary>
                <div className="space-y-4 border-t p-4">
                  <p className="text-xs leading-relaxed text-muted-foreground">已配置节点或镜像目标时，在此处理分发、部署和重试。任务就绪后，再对版本执行预检与激活。</p>
                  <ZipDistributionPanel app={application.id} code={application.code} environment={environment} open={open && deliveryOpen} />
                  <ManagedDeploymentPanel app={application.id} code={application.code} environment={environment} releases={releasesQuery.data ?? []} open={open && deliveryOpen} />
                </div>
              </details>
            )}
              </div>
            </details>
            {onContinue && <div className="mt-5 flex flex-wrap items-center justify-between gap-3 border-t pt-4">
              <p className="max-w-sm text-xs leading-relaxed text-muted-foreground">{compatible && (urlEntry || deployment) ? "入口已配置，可查看接入检查结果。" : "保存应用入口后，可查看接入检查结果。"}</p>
              <Button type="button" disabled={busy} onClick={onContinue}>查看接入检查<ArrowRight className="size-4" /></Button>
            </div>}
          </div>
          {busy && <div className="flex shrink-0 items-center gap-2 border-t px-5 py-3 text-xs text-muted-foreground" role="status"><Loader2 className="size-3.5 animate-spin" />正在{urlEntryBusy ? "保存应用入口" : deleteMutation.isPending ? "删除版本记录" : deploymentMutation.isPending ? "验证并切换部署" : preflightMutation.isPending ? "预检资源" : "登记地址并检查脚本与样式"}，完成后可关闭面板。</div>}
    </>
  )

  return (
    <>
      {embedded ? <section aria-label={`${application.name}发布版本`} className="min-w-0">{content}</section> : (
        <Sheet open={open} onOpenChange={(next) => !busy && onOpenChange(next)}>
          <SheetContent className="w-full gap-0 sm:max-w-3xl">{content}</SheetContent>
        </Sheet>
      )}

      <AlertDialog open={!!deleteTarget} onOpenChange={(next) => { if (!next && !deleteMutation.isPending) setDeleteTarget(null) }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>删除版本 {deleteTarget?.release.version}？</AlertDialogTitle>
            <AlertDialogDescription>删除后将移除这条版本记录及预检结果，无法恢复。已部署站点的文件保留。</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={deleteMutation.isPending}>取消</AlertDialogCancel>
            <AlertDialogAction className="bg-destructive text-destructive-foreground hover:bg-destructive/90" disabled={busy} onClick={(event) => {
              event.preventDefault()
              if (deleteTarget && !busy && isCurrentTarget(deleteTarget.target)) deleteMutation.mutate(deleteTarget)
            }}>{deleteMutation.isPending && <Loader2 className="size-4 animate-spin" />}确认删除</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={!!action} onOpenChange={(next) => !next && !deploymentMutation.isPending && setAction(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{action?.kind === "rollback" ? "确认回滚部署" : "确认激活版本"}</AlertDialogTitle>
            <AlertDialogDescription>
              {action?.kind === "rollback"
                ? `将 ${action.environment} 环境切回上一部署版本 ${action.version}。`
                : action ? `将 ${action.release.environment} 环境切换至版本 ${action.release.version}。` : ""}
              服务端会重新预检；验证通过后切换入口，现有授权继续生效。
              {urlEntry && "当前环境的直接地址配置将被移除。"}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={deploymentMutation.isPending}>取消</AlertDialogCancel>
            <AlertDialogAction disabled={busy} onClick={(event) => { event.preventDefault(); if (action && !busy && isCurrentTarget(action.target)) deploymentMutation.mutate(action) }}>{deploymentMutation.isPending && <Loader2 className="size-4 animate-spin" />}{action?.kind === "rollback" ? "确认回滚" : "确认激活"}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
