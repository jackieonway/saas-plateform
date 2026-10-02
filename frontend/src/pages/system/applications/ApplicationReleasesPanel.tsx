import * as React from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useFieldArray, useForm } from "react-hook-form"
import { z } from "zod"
import { CheckCircle2, FileCheck2, Loader2, Plus, RefreshCw, Rocket, RotateCcw, Trash2, XCircle } from "lucide-react"
import { toast } from "sonner"
import { applicationsApi } from "@/api/applications"
import type { ApplicationEnvironment, ApplicationRecord, ApplicationReleaseRecord } from "@/api/types"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Input } from "@/components/ui/input"
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Form, FormControl, FormDescription, FormField, FormItem, FormLabel, FormMessage } from "@/components/ui/form"
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

const ENVIRONMENTS: { value: ApplicationEnvironment; label: string }[] = [
  { value: "development", label: "开发 · development" },
  { value: "staging", label: "预发 · staging" },
  { value: "production", label: "生产 · production" },
]

function isEntryAddress(value: string) {
  return /^(https?:\/\/|\/(?!\/))/i.test(value)
}

function isResourceAddress(value: string) {
  try {
    return ["http:", "https:"].includes(new URL(value, window.location.origin).protocol)
  } catch {
    return false
  }
}

const releaseSchema = z.object({
  version: z.string().trim().min(1, "请输入版本号").max(64, "版本号最多 64 个字符")
    .regex(/^[A-Za-z0-9][A-Za-z0-9._-]*$/, "仅支持字母、数字、点、下划线与中划线")
    .refine((value) => !value.includes(".."), "版本号不能包含连续的两个点"),
  entryType: z.enum(["HTML", "ESM"]),
  entryUrl: z.string().trim().min(1, "请输入版本入口").max(500).refine(isEntryAddress, "需以 http(s):// 或 / 开头"),
  healthUrl: z.string().trim().max(500).refine((value) => !value || isEntryAddress(value), "需以 http(s):// 或 / 开头"),
  resources: z.array(z.object({
    kind: z.enum(["SCRIPT", "STYLE", "ASSET"]),
    url: z.string().trim().min(1, "请输入资源地址").max(500).refine(isResourceAddress, "请输入 HTTP 地址或相对资源路径"),
  })).max(48, "最多登记 48 个资源"),
}).superRefine((value, ctx) => {
  try {
    const url = new URL(value.entryUrl, window.location.origin)
    const segments = url.pathname.split("/").slice(0, -1).map((segment) => decodeURIComponent(segment))
    if (!segments.includes(value.version)) {
      ctx.addIssue({ code: z.ZodIssueCode.custom, path: ["entryUrl"], message: "入口路径需包含版本号的完整目录，如 /versions/1.0.0/index.html" })
    }
    if (url.search || url.hash) {
      ctx.addIssue({ code: z.ZodIssueCode.custom, path: ["entryUrl"], message: "版本入口不能使用查询参数或锚点，请使用固定版本目录" })
    }
  } catch {
    ctx.addIssue({ code: z.ZodIssueCode.custom, path: ["entryUrl"], message: "请输入有效的版本入口地址" })
  }
})

type ReleaseForm = z.infer<typeof releaseSchema>
type DeploymentAction =
  | { kind: "activate"; release: ApplicationReleaseRecord }
  | { kind: "rollback"; environment: ApplicationEnvironment; version: string }

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

export function ApplicationReleasesPanel({ application, open, onOpenChange }: {
  application: ApplicationRecord
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const queryClient = useQueryClient()
  const [environment, setEnvironment] = React.useState<ApplicationEnvironment>("development")
  const [registerOpen, setRegisterOpen] = React.useState(false)
  const [action, setAction] = React.useState<DeploymentAction | null>(null)
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
  const releases = releasesQuery.data ?? []
  const deployment = deploymentsQuery.data?.find((item) => item.environment === environment)
  const previousRelease = releases.find((item) => item.id === deployment?.previousReleaseId)
  const trusted = application.runtimeMode === "TRUSTED"
  const previousUnsupported = !trusted && previousRelease?.entryType === "ESM"
  const enabled = application.status === "ENABLED"

  const form = useForm<ReleaseForm>({
    resolver: zResolver(releaseSchema),
    defaultValues: { version: "", entryType: "HTML", entryUrl: "", healthUrl: "", resources: [] },
  })
  const resources = useFieldArray({ control: form.control, name: "resources" })
  const versionValue = form.watch("version")

  const refreshReleaseData = () => Promise.all([
    queryClient.invalidateQueries({ queryKey: ["application-releases", application.id] }),
    queryClient.invalidateQueries({ queryKey: deploymentsKey }),
  ])
  const refreshActiveApplication = () => Promise.all([
    queryClient.invalidateQueries({ queryKey: ["applications"] }),
    queryClient.invalidateQueries({ queryKey: ["profile"] }),
  ])

  const registerMutation = useMutation({
    mutationFn: (values: ReleaseForm) => applicationsApi.registerRelease(application.id, {
      environment,
      version: values.version,
      entryType: values.entryType,
      entryUrl: values.entryUrl,
      healthUrl: values.healthUrl || undefined,
      resources: values.resources,
    }),
    onSuccess: () => {
      toast.success("版本已登记", { description: "请手动预检，确认资源可用后激活版本。" })
      form.reset({ version: "", entryType: "HTML", entryUrl: "", healthUrl: "", resources: [] })
      setRegisterOpen(false)
    },
    onError: (error: Error) => toast.error(error.message || "版本登记失败"),
    onSettled: refreshReleaseData,
  })

  const preflightMutation = useMutation({
    mutationFn: (releaseId: string) => applicationsApi.preflightRelease(application.id, releaseId),
    onSuccess: (release) => {
      queryClient.setQueryData<ApplicationReleaseRecord[]>(releasesKey, (old) => old?.map((item) => item.id === release.id ? release : item))
      if (release.validationStatus === "PASSED") toast.success(`版本 ${release.version} 预检通过`)
      else toast.error(`版本 ${release.version} 预检失败`, { description: "请查看逐资源诊断，修复部署资源后重新预检。" })
    },
    onError: (error: Error) => toast.error(error.message || "预检请求失败"),
    onSettled: refreshReleaseData,
  })

  const deploymentMutation = useMutation({
    mutationFn: (next: DeploymentAction) => next.kind === "activate"
      ? applicationsApi.activateRelease(application.id, next.release.id)
      : applicationsApi.rollbackDeployment(application.id, next.environment),
    onSuccess: async (next, requested) => {
      toast.success(requested.kind === "activate" ? `已激活版本 ${next.version}` : `已回滚至版本 ${next.version}`, {
        description: `环境：${next.environment}`,
      })
      setAction(null)
      await refreshActiveApplication()
    },
    onError: (error: Error) => {
      toast.error(error.message || "部署切换失败", { description: "部署与预检结果已重新加载，请查看最新诊断。" })
      setAction(null)
    },
    onSettled: refreshReleaseData,
  })

  const busy = registerMutation.isPending || preflightMutation.isPending || deploymentMutation.isPending
  const selectEnvironment = (value: ApplicationEnvironment) => {
    if (busy) return
    setEnvironment(value)
    setAction(null)
    setRegisterOpen(false)
    form.reset({ version: "", entryType: "HTML", entryUrl: "", healthUrl: "", resources: [] })
  }

  return (
    <>
      <Sheet open={open} onOpenChange={(next) => !busy && onOpenChange(next)}>
        <SheetContent className="w-full gap-0 sm:max-w-3xl">
          <SheetHeader className="shrink-0 border-b p-5 pr-12">
            <SheetTitle className="flex items-center gap-2"><Rocket className="size-5 text-primary" />{application.name} · 发布版本</SheetTitle>
            <SheetDescription>按环境登记不可覆盖的版本，手动预检、激活或回滚。</SheetDescription>
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <Badge variant="outline" className="font-mono">{application.code}</Badge>
              <Badge variant="secondary">{trusted ? "自有可信 · HTML / ESM" : "iframe 隔离 · HTML"}</Badge>
              {!enabled && <Badge variant="destructive">应用已停用</Badge>}
            </div>
          </SheetHeader>
          <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain p-5">
            <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
              <Select value={environment} onValueChange={(value) => selectEnvironment(value as ApplicationEnvironment)} disabled={busy}>
                <SelectTrigger className="w-[220px]" aria-label="发布环境"><SelectValue /></SelectTrigger>
                <SelectContent>{ENVIRONMENTS.map((item) => <SelectItem key={item.value} value={item.value}>{item.label}</SelectItem>)}</SelectContent>
              </Select>
              <div className="flex gap-2">
                <Button variant="ghost" size="icon" aria-label="刷新版本与部署" disabled={busy || releasesQuery.isFetching || deploymentsQuery.isFetching} onClick={() => refreshReleaseData()}>
                  <RefreshCw className={cn("size-4", (releasesQuery.isFetching || deploymentsQuery.isFetching) && "animate-spin")} />
                </Button>
                <Button size="sm" variant={registerOpen ? "secondary" : "default"} disabled={busy} onClick={() => setRegisterOpen((value) => !value)}><Plus className="size-4" />登记版本</Button>
              </div>
            </div>

            <section className="mb-4 rounded-lg border bg-muted/30 p-4" aria-label="当前环境部署">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="min-w-0 flex-1">
                  <p className="text-xs text-muted-foreground">{environment} 当前部署</p>
                  {deploymentsQuery.isLoading ? <p className="mt-2 text-sm text-muted-foreground">正在读取部署…</p> : deploymentsQuery.isError ? (
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
                    disabled={busy || !enabled || deploymentsQuery.isError || previousRelease?.validationStatus !== "PASSED" || previousUnsupported}
                    title={!enabled ? "应用停用期间不能切换部署" : previousUnsupported ? "iframe 隔离应用无法回滚至 ESM 版本" : previousRelease?.validationStatus !== "PASSED" ? "上一版本需预检通过后才能回滚" : undefined}
                    onClick={() => setAction({ kind: "rollback", environment, version: previousRelease?.version ?? deployment.previousReleaseId! })}
                  ><RotateCcw className="size-4" />回滚{previousRelease ? `至 ${previousRelease.version}` : "至上一版本"}</Button>
                )}
              </div>
              <p className="mt-3 border-t pt-3 text-xs leading-relaxed text-muted-foreground">宿主使用服务端配置的环境入口；切换此筛选只查看并管理该环境的版本。激活后仍沿用现有菜单与授权。</p>
              {!enabled && <p className="mt-2 text-xs text-destructive">应用停用期间可登记与预检，请启用应用后再激活或回滚。</p>}
              {previousUnsupported && <p className="mt-2 text-xs text-destructive">上一部署为 ESM，iframe 隔离应用无法回滚至该版本。请选择已验 HTML 版本，或由平台管理员确认自有可信运行方式。</p>}
            </section>

            {registerOpen && (
              <section className="mb-5 rounded-lg border p-4" aria-label="登记新版本">
                <div className="mb-4">
                  <h3 className="text-sm font-semibold">登记 {environment} 版本</h3>
                  <p className="mt-1 text-xs leading-relaxed text-muted-foreground">版本登记后不可修改或覆盖。入口请指向完整版本目录，资源通过 Nginx / Caddy 等静态服务部署后再预检。</p>
                </div>
                <Form {...form}>
                  <form className="space-y-4" onSubmit={form.handleSubmit((values) => {
                    if (busy) return
                    if (!trusted && values.entryType === "ESM") {
                      form.setError("entryType", { message: "iframe 隔离应用仅支持 HTML 入口" })
                      return
                    }
                    registerMutation.mutate(values)
                  })}>
                    <div className="grid gap-4 sm:grid-cols-2">
                      <FormField control={form.control} name="version" render={({ field }) => (
                        <FormItem><FormLabel>版本号 <span className="text-destructive">*</span></FormLabel><FormControl><Input {...field} placeholder="如 1.0.0" className="font-mono" disabled={busy} /></FormControl><FormMessage /></FormItem>
                      )} />
                      <FormField control={form.control} name="entryType" render={({ field }) => (
                        <FormItem><FormLabel>入口类型</FormLabel><Select value={field.value} onValueChange={field.onChange} disabled={busy}><FormControl><SelectTrigger className="w-full"><SelectValue /></SelectTrigger></FormControl><SelectContent><SelectItem value="HTML">HTML · 页面</SelectItem><SelectItem value="ESM" disabled={!trusted}>ESM · 可信模块</SelectItem></SelectContent></Select><FormMessage /></FormItem>
                      )} />
                    </div>
                    <FormField control={form.control} name="entryUrl" render={({ field }) => (
                      <FormItem><FormLabel>版本入口 <span className="text-destructive">*</span></FormLabel><FormControl><Input {...field} placeholder={`/sub-apps/${application.code}/versions/${versionValue || "1.0.0"}/index.html`} className="font-mono text-xs" disabled={busy} /></FormControl><FormDescription>路径必须含版本号的完整目录，版本号不能放在查询参数中。</FormDescription><FormMessage /></FormItem>
                    )} />
                    <FormField control={form.control} name="healthUrl" render={({ field }) => (
                      <FormItem><FormLabel>健康检查地址</FormLabel><FormControl><Input {...field} placeholder="选填，如 /sub-apps/example/health.json" className="font-mono text-xs" disabled={busy} /></FormControl><FormDescription>登记后与入口、资源一起由服务端发起 HTTP 预检。</FormDescription><FormMessage /></FormItem>
                    )} />
                    <div className="space-y-2">
                      <div className="flex items-center justify-between gap-2"><p className="text-sm font-medium">预检资源 <span className="font-normal text-muted-foreground">({resources.fields.length}/48)</span></p><Button type="button" variant="outline" size="sm" disabled={busy || resources.fields.length >= 48} onClick={() => resources.append({ kind: "SCRIPT", url: "" })}><Plus className="size-3.5" />添加资源</Button></div>
                      <p className="text-xs text-muted-foreground">登记关键脚本、样式与静态资源；支持相对于版本入口的路径，预检会逐项检查响应、内容类型与大小。</p>
                      {resources.fields.map((resource, index) => (
                        <div key={resource.id} className="flex items-start gap-2 rounded-md border bg-muted/20 p-2">
                          <FormField control={form.control} name={`resources.${index}.kind`} render={({ field }) => (
                            <FormItem className="w-24 shrink-0"><Select value={field.value} onValueChange={field.onChange} disabled={busy}><FormControl><SelectTrigger className="w-full" aria-label={`资源 ${index + 1} 类型`}><SelectValue /></SelectTrigger></FormControl><SelectContent><SelectItem value="SCRIPT">脚本</SelectItem><SelectItem value="STYLE">样式</SelectItem><SelectItem value="ASSET">资源</SelectItem></SelectContent></Select><FormMessage /></FormItem>
                          )} />
                          <FormField control={form.control} name={`resources.${index}.url`} render={({ field }) => (
                            <FormItem className="min-w-0 flex-1"><FormControl><Input {...field} aria-label={`资源 ${index + 1} 地址`} placeholder="/sub-apps/example/versions/1.0.0/assets/main.js" className="font-mono text-xs" disabled={busy} /></FormControl><FormMessage /></FormItem>
                          )} />
                          <Button type="button" variant="ghost" size="icon" disabled={busy} aria-label={`移除资源 ${index + 1}`} onClick={() => resources.remove(index)}><Trash2 className="size-4" /></Button>
                        </div>
                      ))}
                    </div>
                    <div className="flex items-center justify-between gap-3 border-t pt-3">
                      <p className="text-xs text-muted-foreground">登记只保存版本信息，预检和激活需手动完成。</p>
                      <Button type="submit" size="sm" disabled={busy}>{registerMutation.isPending && <Loader2 className="size-4 animate-spin" />}登记版本</Button>
                    </div>
                  </form>
                </Form>
              </section>
            )}

            <div className="mb-3 flex items-center justify-between"><h3 className="text-sm font-semibold">版本记录</h3><span className="text-xs text-muted-foreground">{releases.length} 个版本</span></div>
            {releasesQuery.isLoading ? <div className="flex items-center justify-center gap-2 py-12 text-sm text-muted-foreground"><Loader2 className="size-4 animate-spin" />正在读取版本…</div> : releasesQuery.isError ? (
              <ErrorState message={releasesQuery.error instanceof Error ? releasesQuery.error.message : undefined} onRetry={() => releasesQuery.refetch()} />
            ) : releases.length === 0 ? (
              <EmptyState title="该环境还没有版本" description="登记版本后，完成预检即可手动激活。" icon={<Rocket className="size-6" />} action={<Button variant="outline" size="sm" disabled={busy} onClick={() => setRegisterOpen(true)}><Plus className="size-4" />登记第一个版本</Button>} />
            ) : (
              <div className="space-y-3">
                {releases.map((release) => {
                  const active = deployment ? deployment.activeReleaseId === release.id : release.active
                  const checks = Array.isArray(release.validationResult?.checks) ? release.validationResult.checks : []
                  const preflighting = preflightMutation.isPending && preflightMutation.variables === release.id
                  const unsupported = !trusted && release.entryType === "ESM"
                  return (
                    <article key={release.id} className={cn("rounded-lg border p-4", active && "border-primary/40 bg-primary/5")}>
                      <div className="flex flex-wrap items-start justify-between gap-3">
                        <div className="min-w-0">
                          <div className="flex flex-wrap items-center gap-2"><h4 className="font-semibold tabular-nums">{release.version}</h4><ValidationBadge release={release} />{active && <Badge>当前部署</Badge>}<Badge variant="outline">{release.entryType}</Badge></div>
                          <p className="mt-2 break-all font-mono text-xs text-muted-foreground">{release.entryUrl}</p>
                        </div>
                        <div className="flex shrink-0 gap-2">
                          <Button variant="outline" size="sm" disabled={busy} onClick={() => preflightMutation.mutate(release.id)}>{preflighting ? <Loader2 className="size-4 animate-spin" /> : <FileCheck2 className="size-4" />}{release.checkedAt ? "重新预检" : "预检"}</Button>
                          <Button size="sm" disabled={busy || !enabled || active || release.validationStatus !== "PASSED" || unsupported || deploymentsQuery.isLoading || deploymentsQuery.isError} onClick={() => setAction({ kind: "activate", release })}><Rocket className="size-4" />{active ? "已激活" : "激活"}</Button>
                        </div>
                      </div>
                      <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground"><span>登记 {dateLabel(release.createdAt)}</span><span>预检 {dateLabel(release.checkedAt)}</span><span>{release.resources.length} 项登记资源</span></div>
                      {release.healthUrl && <p className="mt-1 break-all text-xs text-muted-foreground">健康检查 <span className="font-mono">{release.healthUrl}</span></p>}
                      {unsupported && <p className="mt-2 text-xs text-destructive">iframe 隔离应用无法激活 ESM，请登记 HTML 版本或由平台管理员确认自有可信运行方式。</p>}
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
          </div>
          {busy && <div className="flex shrink-0 items-center gap-2 border-t px-5 py-3 text-xs text-muted-foreground" role="status"><Loader2 className="size-3.5 animate-spin" />正在{deploymentMutation.isPending ? "验证并切换部署" : preflightMutation.isPending ? "预检资源" : "登记版本"}，完成后可切换环境或关闭面板。</div>}
        </SheetContent>
      </Sheet>

      <AlertDialog open={!!action} onOpenChange={(next) => !next && !deploymentMutation.isPending && setAction(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{action?.kind === "rollback" ? "确认回滚部署" : "确认激活版本"}</AlertDialogTitle>
            <AlertDialogDescription>
              {action?.kind === "rollback"
                ? `将 ${action.environment} 环境切回上一部署版本 ${action.version}。`
                : action ? `将 ${action.release.environment} 环境切换至版本 ${action.release.version}。` : ""}
              服务端会重新预检；验证通过后切换入口，现有授权继续生效。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={deploymentMutation.isPending}>取消</AlertDialogCancel>
            <AlertDialogAction disabled={busy} onClick={(event) => { event.preventDefault(); if (action && !busy) deploymentMutation.mutate(action) }}>{deploymentMutation.isPending && <Loader2 className="size-4 animate-spin" />}{action?.kind === "rollback" ? "确认回滚" : "确认激活"}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
