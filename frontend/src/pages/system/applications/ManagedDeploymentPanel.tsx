import * as React from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { managedDeploymentsApi, type DeploymentIntent, type ManagedDeploymentJob } from "@/api/managed-deployments"
import type { ApplicationReleaseRecord } from "@/api/types"
import { usePermissionsQuery } from "@/hooks/use-auth-data"
import { useAuthStore } from "@/stores/auth"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Badge } from "@/components/ui/badge"
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "@/components/ui/alert-dialog"

const states: Record<ManagedDeploymentJob["state"], string> = {
  QUEUED: "等待执行", RUNNING: "正在执行", READY: "部署就绪", FAILED: "部署失败", CLEANED: "已清理",
}
const selectClass = "h-9 w-full rounded-md border bg-background px-2 text-sm"

type Props = {
  app: string; code: string; environment: string; releases: ApplicationReleaseRecord[]; open: boolean
}
export function ManagedDeploymentPanel(props: Props) {
  const epoch = useAuthStore(s => s.contextEpoch)
  return <ManagedDeploymentContent key={`${props.app}:${props.environment}:${epoch}`} {...props} />
}
function ManagedDeploymentContent({ app, code, environment, releases, open }: Props) {
  const permissions = usePermissionsQuery({ enabled: open })
  const epoch = useAuthStore(s => s.contextEpoch)
  const read = !!permissions.data?.permissions.includes("system:deployment:read")
  const write = !!permissions.data?.permissions.includes("system:deployment:write")
  const queryClient = useQueryClient()
  const targets = useQuery({ queryKey: ["deployment-targets", epoch], queryFn: managedDeploymentsApi.targets, enabled: open && read })
  const jobsKey = ["managed-deployments", app, epoch]
  const jobs = useQuery({ queryKey: jobsKey, queryFn: () => managedDeploymentsApi.list(app), enabled: open && read && !!targets.data?.length, refetchInterval: open ? 2000 : false })
  const [releaseId, setReleaseId] = React.useState("")
  const [targetId, setTargetId] = React.useState("")
  const [templateId, setTemplateId] = React.useState("")
  const [imageRef, setImageRef] = React.useState("")
  const [error, setError] = React.useState("")
  const [cleanupJob, setCleanupJob] = React.useState<ManagedDeploymentJob | null>(null)
  const lastIntent = React.useRef<{ key: string; body: DeploymentIntent } | null>(null)
  const alive = React.useRef(true)
  React.useEffect(() => { alive.current = true; return () => { alive.current = false } }, [])
  const mutation = useMutation({
    mutationFn: async (operation: { kind: "create"; body: DeploymentIntent } | { kind: "retry" | "cleanup"; job: string }) => {
      if (operation.kind === "create") return managedDeploymentsApi.create(app, operation.body)
      return managedDeploymentsApi[operation.kind](app, operation.job)
    },
    onSuccess: () => { if (alive.current) { setError(""); setCleanupJob(null); void queryClient.invalidateQueries({ queryKey: jobsKey }) } },
    onError: err => { if (alive.current) setError(err instanceof Error ? err.message : "结果暂未确认，请读取任务状态后重试") },
  })
  if (!open || !read || !targets.data?.length) return null
  const target = targets.data.find(t => t.id === targetId)
  const available = targets.data.filter(t => t.templates.some(template => template.applicationCode === code))
  if (!available.length) return null
  const selected = releases.find(r => r.id === releaseId)
  const expected = selected && target ? `${target.publicOrigin}/sub-apps/${code}/versions/${selected.version}/` : ""
  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    if (!write || mutation.isPending) return
    const value = imageRef.trim()
    const key = JSON.stringify([releaseId, targetId, templateId, value, epoch])
    if (lastIntent.current?.key !== key) lastIntent.current = { key, body: { releaseId, targetId, templateId, imageRef: value, requestId: crypto.randomUUID() } }
    if (lastIntent.current) mutation.mutate({ kind: "create", body: lastIntent.current.body })
  }
  return <section className="space-y-4 rounded-lg border p-4" aria-label="受管镜像部署">
    <div><h3 className="font-medium">受管镜像部署</h3><p className="mt-1 text-xs leading-relaxed text-muted-foreground">提交固定镜像版本后等待部署就绪，再对该发布版本执行预检和激活。部署不会授予应用访问权限。</p></div>
    {write && <form onSubmit={submit} className="grid gap-3 sm:grid-cols-2">
      <label className="space-y-1 text-xs">发布版本<select aria-label="镜像发布版本" className={selectClass} value={releaseId} onChange={e => setReleaseId(e.target.value)} required disabled={mutation.isPending}><option value="">选择已登记的 HTML 版本</option>{releases.filter(r => r.entryType === "HTML").map(r => <option key={r.id} value={r.id}>{r.version}</option>)}</select></label>
      <label className="space-y-1 text-xs">部署目标<select aria-label="部署目标" className={selectClass} value={targetId} onChange={e => { setTargetId(e.target.value); setTemplateId("") }} required disabled={mutation.isPending}><option value="">选择目标</option>{available.map(t => <option key={t.id} value={t.id}>{t.name}</option>)}</select></label>
      <label className="space-y-1 text-xs">服务模板<select aria-label="服务模板" className={selectClass} value={templateId} onChange={e => setTemplateId(e.target.value)} required disabled={mutation.isPending}><option value="">选择模板</option>{target?.templates.filter(t => t.applicationCode === code).map(t => <option key={t.id} value={t.id}>{t.id}</option>)}</select></label>
      <label className="space-y-1 text-xs">镜像地址（固定摘要）<Input aria-label="固定镜像地址" placeholder="registry/repository@sha256:…" value={imageRef} onChange={e => setImageRef(e.target.value)} required disabled={mutation.isPending} /></label>
      {expected && <p className="break-all text-xs text-muted-foreground sm:col-span-2">版本入口：{expected}{selected?.entryUrl !== expected && <span className="ml-1 text-destructive">请先将此版本登记到该入口。</span>}</p>}
      <div className="sm:col-span-2"><Button type="submit" size="sm" disabled={mutation.isPending || !target || !templateId || !selected || selected.entryUrl !== expected || !/@sha256:[a-f0-9]{64}$/.test(imageRef.trim())}>{mutation.isPending ? "正在提交…" : "提交镜像部署"}</Button></div>
    </form>}
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    {jobs.isError && <p role="alert" className="text-sm text-destructive">任务状态读取失败，<button className="underline" onClick={() => void jobs.refetch()}>重新读取</button></p>}
    <div className="space-y-2" aria-live="polite">{jobs.data?.filter(j => j.environment === environment).map(job => <article key={job.id} className="space-y-2 rounded-md border p-3 text-xs" data-deployment-job={job.id}>
      <div className="flex flex-wrap items-center justify-between gap-2"><span className="font-medium">{job.version} · {job.targetId}{job.operation === "CLEANUP" ? " · 清理" : ""}</span><Badge variant={job.state === "FAILED" ? "destructive" : "outline"}>{states[job.state]}</Badge></div>
      <p className="break-all font-mono text-muted-foreground">{job.imageRef}</p><p className="text-muted-foreground">第 {job.attempt} 次执行{job.errorCode ? ` · ${job.errorCode}` : ""}</p>
      {write && <div className="flex gap-2">{job.state === "FAILED" && <Button size="sm" variant="outline" disabled={mutation.isPending} onClick={() => mutation.mutate({ kind: "retry", job: job.id })}>重试部署任务</Button>}{!["QUEUED", "RUNNING", "CLEANED"].includes(job.state) && <Button size="sm" variant="outline" disabled={mutation.isPending} onClick={() => setCleanupJob(job)}>清理候选</Button>}</div>}
    </article>)}</div>
    <AlertDialog open={!!cleanupJob} onOpenChange={value => { if (!value && !mutation.isPending) setCleanupJob(null) }}><AlertDialogContent><AlertDialogHeader><AlertDialogTitle>清理候选版本 {cleanupJob?.version}？</AlertDialogTitle><AlertDialogDescription>将清理该候选的容器和版本路由。当前版本、上一版本、其他环境引用、在途任务和保留窗口中的资源会由服务器再次检查并拒绝清理。</AlertDialogDescription></AlertDialogHeader><AlertDialogFooter><AlertDialogCancel disabled={mutation.isPending}>取消</AlertDialogCancel><AlertDialogAction disabled={mutation.isPending} onClick={e => { e.preventDefault(); if (cleanupJob) mutation.mutate({ kind: "cleanup", job: cleanupJob.id }) }}>提交清理</AlertDialogAction></AlertDialogFooter></AlertDialogContent></AlertDialog>
  </section>
}
