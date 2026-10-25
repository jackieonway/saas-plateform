import { newUuid } from "@/lib/uuid"
import * as React from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { zipDistributionsApi as api, type ZipIntent } from "@/api/zip-distributions"
import { usePermissionsQuery } from "@/hooks/use-auth-data"
import { useAuthStore } from "@/stores/auth"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"

type Props = { app: string; code: string; environment: string; open: boolean }
export function ZipDistributionPanel(props: Props) {
  const epoch = useAuthStore(s => s.contextEpoch)
  return <Content key={`${props.app}:${props.environment}:${epoch}`} {...props} />
}
function Content({ app, code, environment, open }: Props) {
  const epoch = useAuthStore(s => s.contextEpoch)
  const permissions = usePermissionsQuery({ enabled: open })
  const read = !!permissions.data?.permissions.includes("system:distribution:read")
  const write = !!permissions.data?.permissions.includes("system:distribution:write")
  const targets = useQuery({ queryKey: ["zip-targets", epoch], queryFn: api.targets, enabled: open && read })
  const jobsKey = ["zip-distributions", app, epoch]
  const jobs = useQuery({ queryKey: jobsKey, queryFn: () => api.list(app), enabled: open && read, refetchInterval: open ? 2000 : false })
  const client = useQueryClient()
  const [releaseId, setRelease] = React.useState("")
  const [groupId, setGroup] = React.useState("")
  const [error, setError] = React.useState("")
  const intent = React.useRef<{ key: string; body: ZipIntent } | null>(null)
  const alive = React.useRef(true)
  React.useEffect(() => { alive.current = true; return () => { alive.current = false } }, [])
  const mutation = useMutation({
    mutationFn: (op: { kind: "create"; body: ZipIntent } | { kind: "retry" | "verify"; id: string }) => op.kind === "create" ? api.create(app, op.body) : api[op.kind](app, op.id),
    onSuccess: () => { if (alive.current) { setError(""); void client.invalidateQueries({ queryKey: jobsKey }) } },
    onError: err => { if (alive.current) setError(err instanceof Error ? err.message : "结果未确认，请读取状态后重试原请求") },
  })
  const groups = targets.data?.filter(t => t.applications.includes(code)) ?? []
  if (!open || !read) return null
  if (!groups.length && !jobs.data?.jobs.length && !targets.isError) return null
  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    if (!write || mutation.isPending) return
    const key = JSON.stringify([releaseId, groupId, epoch])
    if (intent.current?.key !== key) intent.current = { key, body: { releaseId, groupId, requestId: newUuid() } }
    mutation.mutate({ kind: "create", body: intent.current.body })
  }
  const selectClass = "h-9 w-full rounded-md border bg-background px-2 text-sm"
  return <section aria-label="ZIP节点分发" className="space-y-4 rounded-lg border p-4">
    <div><h3 className="font-medium">ZIP节点分发</h3><p className="mt-1 text-xs text-muted-foreground">先上传ZIP，再分发到全部必需节点。全部节点就绪后才能预检和激活；核验失败时可查看节点原因并重试。</p></div>
    {write && groups.length > 0 && <form onSubmit={submit} className="grid gap-3 sm:grid-cols-2">
      <label className="space-y-1 text-xs">发布版本<select aria-label="ZIP发布版本" className={selectClass} required value={releaseId} onChange={e => setRelease(e.target.value)} disabled={mutation.isPending}><option value="">选择已上传的ZIP版本</option>{jobs.data?.candidates.filter(r => r.environment === environment).map(r => <option key={r.releaseId} value={r.releaseId}>{r.version}</option>)}</select></label>
      <label className="space-y-1 text-xs">分发目标<select aria-label="ZIP分发目标" className={selectClass} required value={groupId} onChange={e => setGroup(e.target.value)} disabled={mutation.isPending}><option value="">选择必需节点组</option>{groups.map(g => <option key={g.id} value={g.id}>{g.name}（{g.nodes.length}个必需节点）</option>)}</select></label>
      <div><Button size="sm" type="submit" disabled={mutation.isPending || !releaseId || !groupId}>{mutation.isPending ? "正在提交…" : "提交ZIP分发"}</Button></div>
    </form>}
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    {(jobs.isError || targets.isError) && <p role="alert" className="text-sm text-destructive">分发状态读取失败，<button className="underline" onClick={() => { void jobs.refetch(); void targets.refetch() }}>重新读取</button></p>}
    <div aria-live="polite" className="space-y-2">{jobs.data?.jobs.filter(j => j.environment === environment).map(j => <article key={j.id} data-zip-job={j.id} className="space-y-2 rounded-md border p-3 text-xs">
      <div className="flex items-center justify-between"><span>{j.version} · {j.groupId}</span><Badge variant={j.state === "FAILED" ? "destructive" : "outline"}>{({ QUEUED: "等待分发", RUNNING: "正在分发", READY: "全部节点就绪", FAILED: "分发失败" })[j.state]}</Badge></div>
      <p className="break-all font-mono text-muted-foreground">{j.artifactDigest}</p>
      {j.nodes.map(n => <p key={n.id}>{n.id} · {n.state}{n.errorCode ? ` · ${n.errorCode}` : ""}</p>)}
      {j.errorCode && <p className="text-destructive">{j.errorCode}</p>}
      {write && <div className="flex gap-2">{j.state === "FAILED" && <Button size="sm" variant="outline" disabled={mutation.isPending} onClick={() => mutation.mutate({ kind: "retry", id: j.id })}>重试ZIP分发</Button>}{j.state === "READY" && <Button size="sm" variant="outline" disabled={mutation.isPending} onClick={() => mutation.mutate({ kind: "verify", id: j.id })}>核验全部节点</Button>}</div>}
    </article>)}</div>
  </section>
}
