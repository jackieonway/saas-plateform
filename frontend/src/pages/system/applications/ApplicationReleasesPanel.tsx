import * as React from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useForm } from "react-hook-form"
import { z } from "zod"
import { ArrowRight, CheckCircle2, FileCheck2, Link2, Loader2, PackageOpen, RefreshCw, Rocket, RotateCcw, XCircle } from "lucide-react"
import { toast } from "sonner"
import { applicationsApi } from "@/api/applications"
import type { ApplicationEnvironment, ApplicationRecord, ApplicationReleasePayload, ApplicationReleaseRecord } from "@/api/types"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Input } from "@/components/ui/input"
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Form, FormControl, FormField, FormItem, FormLabel, FormMessage } from "@/components/ui/form"
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
import { zResolver } from "@/lib/form"
import { cn } from "@/lib/utils"
import { ExternalReleaseForm } from "./ExternalReleaseForm"
import { ZipDistributionPanel } from "./ZipDistributionPanel"
import { ManagedDeploymentPanel } from "./ManagedDeploymentPanel"
import { usePermissionsQuery } from "@/hooks/use-auth-data"
import { ApplicationUrlEntryForm } from "./ApplicationUrlEntryForm"
import { isQiankunApp } from "./application-setup-policy"

const ENVIRONMENTS: { value: ApplicationEnvironment; label: string }[] = [
  { value: "development", label: "开发 · development" },
  { value: "staging", label: "预发 · staging" },
  { value: "production", label: "生产 · production" },
]

const releaseSchema = z.object({
  version: z.string().trim().min(1, "请输入版本号").max(64, "版本号最多 64 个字符")
    .regex(/^[A-Za-z0-9][A-Za-z0-9._-]*$/, "仅支持字母、数字、点、下划线与中划线")
    .refine((value) => !value.includes(".."), "版本号不能包含连续的两个点"),
})

type ReleaseForm = z.infer<typeof releaseSchema>
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
  environment?: ApplicationEnvironment
  onEnvironmentChange?: (environment: ApplicationEnvironment) => void
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

function ApplicationReleasesPanelContent({ application, open, onOpenChange, embedded = false, onDeploymentChanged, onContinue, onBusyChange, environment: selectedEnvironment, onEnvironmentChange }: PanelProps) {
  const queryClient = useQueryClient()
  const [localEnvironment, setLocalEnvironment] = React.useState<ApplicationEnvironment>("development")
  const environment = selectedEnvironment ?? localEnvironment
  const [sourceMode, setSourceMode] = React.useState<"archive" | "url">("archive")
  const [externalFormVersion, setExternalFormVersion] = React.useState(0)
  const [deliveryOpen, setDeliveryOpen] = React.useState(false)
  const [urlEntryBusy, setUrlEntryBusy] = React.useState(false)
  const permissions = usePermissionsQuery({ enabled: open })
  const canManageDelivery = permissions.isSuccess && !!permissions.data?.permissions.some(permission =>
    permission === "system:distribution:read" || permission === "system:deployment:read")
  const [archiveFile, setArchiveFile] = React.useState<File | null>(null)
  const archiveInput = React.useRef<HTMLInputElement>(null)
  const [action, setAction] = React.useState<DeploymentAction | null>(null)
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

  const form = useForm<ReleaseForm>({
    resolver: zResolver(releaseSchema),
    defaultValues: { version: "" },
  })

  const refreshReleaseData = (requested: ReleaseTarget = target) => Promise.all([
    queryClient.invalidateQueries({ queryKey: ["application-releases", requested.applicationId] }),
    queryClient.invalidateQueries({ queryKey: ["application-deployments", requested.applicationId] }),
    queryClient.invalidateQueries({ queryKey: ["application-url-entry", requested.applicationId, requested.environment] }),
  ])
  const refreshActiveApplication = () => Promise.all([
    queryClient.invalidateQueries({ queryKey: ["applications"] }),
    queryClient.invalidateQueries({ queryKey: ["profile"] }),
  ])

  const registerMutation = useMutation({
    mutationFn: (requested: { target: ReleaseTarget; version: string; file: File }) => applicationsApi.uploadRelease(requested.target.applicationId, requested.target.environment, requested.version, requested.file),
    onSuccess: (release, requested) => {
      if (!isCurrentTarget(requested.target)) return
      if (release.validationStatus === "PASSED") toast.success("版本包已上传，预检通过", { description: "确认版本后即可激活。" })
      else if (release.validationStatus === "DRAFT") {
        toast.success("版本包已上传，等待节点分发", { description: canManageDelivery ? "在「交付与运维」中完成 ZIP 节点分发，再预检并激活。" : "请由有分发权限的管理员完成节点分发，再预检并激活。" })
        if (canManageDelivery) setDeliveryOpen(true)
      }
      else toast.error("版本包已上传，预检未通过", { description: "查看诊断；若包内容有误，请使用新版本号重新上传。" })
      form.reset({ version: "" })
      setArchiveFile(null)
      if (archiveInput.current) archiveInput.current.value = ""
    },
    onError: (error: Error, requested) => { if (isCurrentTarget(requested.target)) toast.error(error.message || "版本登记失败") },
    onSettled: (_release, _error, requested) => refreshReleaseData(requested.target),
  })

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

  const busy = urlEntryBusy || registerMutation.isPending || externalMutation.isPending || preflightMutation.isPending || deploymentMutation.isPending
  React.useEffect(() => {
    onBusyChange?.(busy)
    return () => onBusyChange?.(false)
  }, [busy, onBusyChange])
  const selectEnvironment = (value: ApplicationEnvironment) => {
    if (busy) return
    if (onEnvironmentChange) onEnvironmentChange(value)
    else if (selectedEnvironment === undefined) setLocalEnvironment(value)
  }
  React.useEffect(() => {
    setAction(null)
    setSourceMode("archive")
    setDeliveryOpen(false)
    form.reset({ version: "" })
    setArchiveFile(null)
    if (archiveInput.current) archiveInput.current.value = ""
  }, [environment, form])

  const selectSource = (value: "archive" | "url") => {
    if (busy || sourceMode === value) return
    setSourceMode(value)
    form.reset({ version: "" })
    setArchiveFile(null)
    if (archiveInput.current) archiveInput.current.value = ""
  }
  const Header = embedded ? "header" : SheetHeader
  const Title = embedded ? "h3" : SheetTitle
  const Description = embedded ? "p" : SheetDescription

  const content = (
    <>
          <Header className={cn("shrink-0 border-b", embedded ? "pb-4" : "p-5 pr-12")}>
            <Title className="flex items-center gap-2 text-base font-semibold"><Rocket className="size-5 text-primary" />配置应用入口</Title>
            <Description className="mt-1 text-sm text-muted-foreground">按环境填写独立站点地址，或上传构建包、发布可回滚的固定版本。</Description>
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <Badge variant="outline" className="font-mono">{application.code}</Badge>
              <Badge variant={compatible ? "secondary" : "destructive"}>{compatible ? "qiankun · HTML" : "需重新配置 qiankun 入口"}</Badge>
              {!enabled && <Badge variant="destructive">应用已停用</Badge>}
            </div>
          </Header>
          <div className={cn("min-h-0 flex-1", embedded ? "pt-5" : "overflow-y-auto overscroll-contain p-5")}>
            <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
              {!embedded && <Select value={environment} onValueChange={(value) => selectEnvironment(value as ApplicationEnvironment)} disabled={busy}>
                <SelectTrigger className="w-[220px]" aria-label="发布环境"><SelectValue /></SelectTrigger>
                <SelectContent>{ENVIRONMENTS.map((item) => <SelectItem key={item.value} value={item.value}>{item.label}</SelectItem>)}</SelectContent>
              </Select>}
              <div className="flex gap-2">
                <Button variant="ghost" size="icon" aria-label="刷新版本与部署" disabled={busy || releasesQuery.isFetching || deploymentsQuery.isFetching} onClick={() => refreshReleaseData()}>
                  <RefreshCw className={cn("size-4", (releasesQuery.isFetching || deploymentsQuery.isFetching) && "animate-spin")} />
                </Button>
              </div>
            </div>

            <section className="mb-4 rounded-lg border bg-muted/30 p-4" aria-label="当前环境部署">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="min-w-0 flex-1">
                  <p className="text-xs text-muted-foreground">{environment} 当前部署</p>
                  {!compatible ? <><p className="mt-1 text-xl font-semibold">等待重新配置 qiankun</p><p className="mt-2 text-xs text-muted-foreground">现有入口不会运行，请在下方明确保存新的 qiankun HTML 入口。</p></> : urlEntryQuery.isError ? <p className="mt-2 text-sm text-destructive">直接地址读取失败，无法确认当前生效入口。</p> : !urlEntryReady ? <p className="mt-2 text-sm text-muted-foreground">正在读取入口…</p> : urlEntry ? (
                    <><p className="mt-1 text-xl font-semibold">直接地址</p><p className="mt-1 break-all font-mono text-xs text-muted-foreground">{urlEntry.entryUrl}</p><p className="mt-2 text-xs text-muted-foreground">{urlEntry.proxyEnabled ? "通过平台同域代理" : "浏览器直接加载上游"} · 更新于 {dateLabel(urlEntry.updatedAt)}</p>
                      {deployment && <p className="mt-2 text-xs text-muted-foreground">保留版本 {deployment.version}，移除直接地址后恢复。</p>}</>
                  ) : deploymentsQuery.isLoading ? <p className="mt-2 text-sm text-muted-foreground">正在读取部署…</p> : deploymentsQuery.isError ? (
                    <div className="mt-2 flex flex-wrap items-center gap-2 text-sm text-destructive">
                      <span>{deploymentsQuery.error instanceof Error ? deploymentsQuery.error.message : "部署读取失败"}</span>
                      <Button variant="outline" size="sm" onClick={() => deploymentsQuery.refetch()} disabled={busy}>重试</Button>
                    </div>
                  ) : deployment ? (
                    <>
                      <p className="mt-1 text-xl font-semibold tabular-nums">{deployment.version}</p>
                      <p className="mt-1 break-all font-mono text-xs text-muted-foreground">{deployment.entryUrl}</p>
                      <p className="mt-2 text-xs text-muted-foreground">切换时间 {dateLabel(deployment.updatedAt)}</p>
                    </>
                  ) : <p className="mt-2 text-sm text-muted-foreground">该环境尚未激活版本</p>}
                </div>
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
              <p className="mt-3 border-t pt-3 text-xs leading-relaxed text-muted-foreground">当前管理 {environment} 环境；宿主实际使用服务端指定的环境。确认激活后切换入口，访问范围单独配置。</p>
              {!enabled && <p className="mt-2 text-xs text-destructive">应用停用期间可登记与预检，请启用应用后再激活或回滚。</p>}
              {previousUnsupported && <p className="mt-2 text-xs text-destructive">上一部署不兼容 qiankun，请选择已验证的 HTML 版本。</p>}
            </section>

            {urlEntryQuery.isError && <div className="mb-4"><ErrorState message={urlEntryQuery.error.message} onRetry={() => urlEntryQuery.refetch()} /></div>}
            <ApplicationUrlEntryForm key={`${application.id}:${environment}`} application={application} environment={environment} entry={urlEntry} ready={urlEntryReady} busy={busy} onBusyChange={setUrlEntryBusy} onChanged={() => onDeploymentChanged?.()} />

            <div className="mb-3"><h3 className="text-sm font-semibold">版本发布与回滚</h3><p className="mt-1 text-xs text-muted-foreground">固定版本适合正式发布。激活或回滚会移除当前环境的直接地址配置。</p></div>

            <div className="mb-4 grid gap-3 sm:grid-cols-2" role="group" aria-label="版本来源">
              <Button type="button" variant="outline" disabled={busy} aria-pressed={sourceMode === "archive"} onClick={() => selectSource("archive")}
                className={cn("h-auto items-start justify-start gap-3 whitespace-normal p-4 text-left", sourceMode === "archive" && "border-primary bg-primary/5 ring-1 ring-primary/20")}>
                <PackageOpen className="mt-0.5 size-5 shrink-0 text-primary" />
                <span><span className="flex items-center gap-2 font-medium">上传构建包</span><span className="mt-1 block text-xs font-normal text-muted-foreground">选择 ZIP 或 tar.gz，自动生成版本入口。</span></span>
              </Button>
              <Button type="button" variant="outline" disabled={busy} aria-pressed={sourceMode === "url"} onClick={() => selectSource("url")}
                className={cn("h-auto items-start justify-start gap-3 whitespace-normal p-4 text-left", sourceMode === "url" && "border-primary bg-primary/5 ring-1 ring-primary/20")}>
                <Link2 className="mt-0.5 size-5 shrink-0 text-primary" />
                <span><span className="font-medium">已部署地址</span><span className="mt-1 block text-xs font-normal text-muted-foreground">已有静态站点时，登记固定版本的 HTML 地址。</span></span>
              </Button>
            </div>
            {sourceMode === "url" && <ExternalReleaseForm key={`${application.id}:${environment}:${externalFormVersion}`} environment={environment} busy={busy} pending={externalMutation.isPending} onSubmit={payload => { if (!busy && isCurrentTarget(target)) externalMutation.mutate({ target, payload }) }} />}

            {sourceMode === "archive" && (
              <section className="mb-5 rounded-lg border p-4" aria-label="上传新版本">
                <div className="mb-4">
                  <h3 className="text-sm font-semibold">上传 {environment} 版本</h3>
                  <p className="mt-1 text-xs leading-relaxed text-muted-foreground">上传构建后的 ZIP 或 tar.gz；系统解压到独立版本目录并生成资源清单。版本不可覆盖。</p>
                </div>
                <Form {...form}>
                  <form className="space-y-4" onSubmit={form.handleSubmit((values) => {
                    if (busy || !isCurrentTarget(target)) return
                    if (!archiveFile) {
                      toast.error("请选择 ZIP 或 tar.gz 构建包")
                      return
                    }
                    registerMutation.mutate({ target, version: values.version, file: archiveFile })
                  })}>
                    <div>
                      <FormField control={form.control} name="version" render={({ field }) => (
                        <FormItem><FormLabel>版本号 <span className="text-destructive">*</span></FormLabel><FormControl><Input {...field} placeholder="如 1.0.0" className="font-mono" disabled={busy} /></FormControl><FormMessage /></FormItem>
                      )} />
                    </div>
                    <div className="space-y-2">
                      <label htmlFor="micro-release-archive" className="text-sm font-medium">构建包 <span className="text-destructive">*</span></label>
                      <Input id="micro-release-archive" ref={archiveInput} type="file" accept=".zip,.tar.gz" disabled={busy}
                        onChange={(event) => {
                          const file = event.target.files?.[0] ?? null
                          if (file && !/\.(zip|tar\.gz)$/i.test(file.name)) { toast.error("仅支持 .zip 或 .tar.gz 文件"); event.target.value = ""; setArchiveFile(null); return }
                          if (file && file.size > 20 * 1024 * 1024) { toast.error("压缩包不能超过 20 MiB"); event.target.value = ""; setArchiveFile(null); return }
                          setArchiveFile(file)
                        }} />
                      <p className="text-xs text-muted-foreground">构建包须包含 index.html；可直接压缩构建目录，或压缩一个包含构建文件的顶层文件夹。</p>
                    </div>
                    <div className="flex items-center justify-between gap-3 border-t pt-3">
                      <p className="text-xs text-muted-foreground">上传后自动预检；通过后手动激活，不会自动切换当前版本。</p>
                      <Button type="submit" size="sm" disabled={busy}>{registerMutation.isPending && <Loader2 className="size-4 animate-spin" />}上传并登记</Button>
                    </div>
                  </form>
                </Form>
              </section>
            )}

            <div className="mb-3 flex items-center justify-between"><h3 className="text-sm font-semibold">版本记录</h3><span className="text-xs text-muted-foreground">{releases.length} 个版本</span></div>
            {releasesQuery.isLoading ? <div className="flex items-center justify-center gap-2 py-12 text-sm text-muted-foreground"><Loader2 className="size-4 animate-spin" />正在读取版本…</div> : releasesQuery.isError ? (
              <ErrorState message={releasesQuery.error instanceof Error ? releasesQuery.error.message : undefined} onRetry={() => releasesQuery.refetch()} />
            ) : releases.length === 0 ? (
              <EmptyState title="等待第一个版本" description="在上方选择版本来源并提交，检查通过后即可确认激活。" icon={<Rocket className="size-6" />} />
            ) : (
              <div className="space-y-3">
                {releases.map((release) => {
                  const active = compatible && !urlEntry && (deployment ? deployment.activeReleaseId === release.id : release.active)
                  const checks = Array.isArray(release.validationResult?.checks) ? release.validationResult.checks : []
                  const scriptCount = checks.filter(check => check.kind === "SCRIPT").length
                  const styleCount = checks.filter(check => check.kind === "STYLE").length
                  const preflighting = preflightMutation.isPending && preflightMutation.variables?.releaseId === release.id
                  const unsupported = release.entryType !== "HTML"
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
                        </div>
                      </div>
                      <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground"><span>登记 {dateLabel(release.createdAt)}</span><span>预检 {dateLabel(release.checkedAt)}</span>{release.checkedAt && <span>已检查 {scriptCount} 个脚本 · {styleCount} 个样式</span>}{release.resources.length > 0 && <span>{release.resources.length} 项补充资源</span>}</div>
                      {release.healthUrl && <p className="mt-1 break-all text-xs text-muted-foreground">健康检查 <span className="font-mono">{release.healthUrl}</span></p>}
                      {unsupported && <p className="mt-2 text-xs text-destructive">此版本不兼容 qiankun，请上传包含标准生命周期的 HTML 版本。</p>}
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
            {onContinue && <div className="mt-5 flex flex-wrap items-center justify-between gap-3 border-t pt-4">
              <p className="max-w-sm text-xs leading-relaxed text-muted-foreground">{compatible && (urlEntry || deployment) ? "入口已配置，接下来选择哪些租户、组织和角色可以访问。" : "可以先配置访问范围；该环境仍需保存 qiankun 直接地址或激活版本后才能使用。"}</p>
              <Button type="button" disabled={busy} onClick={onContinue}>继续配置访问范围<ArrowRight className="size-4" /></Button>
            </div>}
          </div>
          {busy && <div className="flex shrink-0 items-center gap-2 border-t px-5 py-3 text-xs text-muted-foreground" role="status"><Loader2 className="size-3.5 animate-spin" />正在{urlEntryBusy ? "保存应用入口" : deploymentMutation.isPending ? "验证并切换部署" : preflightMutation.isPending ? "预检资源" : externalMutation.isPending ? "登记地址并检查脚本与样式" : "上传版本包"}，完成后可切换环境或关闭面板。</div>}
    </>
  )

  return (
    <>
      {embedded ? <section aria-label={`${application.name}发布版本`} className="min-w-0">{content}</section> : (
        <Sheet open={open} onOpenChange={(next) => !busy && onOpenChange(next)}>
          <SheetContent className="w-full gap-0 sm:max-w-3xl">{content}</SheetContent>
        </Sheet>
      )}

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
