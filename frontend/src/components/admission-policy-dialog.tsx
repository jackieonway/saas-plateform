import * as React from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { AlertTriangle, Loader2, RefreshCw, ShieldCheck, UsersRound } from "lucide-react"
import { toast } from "sonner"
import { admissionMemberLabels, admissionPoliciesApi, isAdmissionReasonValid, type AdmissionMemberType, type AdmissionPolicy, type AdmissionPolicyChange, type AdmissionPolicyTarget } from "@/api/admission-policies"
import { ApiError } from "@/lib/api"
import { captureAuthSession, isCurrentAuthSession } from "@/lib/auth-queries"
import { captureSessionFence, useAuthStore, type SessionFence } from "@/stores/auth"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"

export function AdmissionPolicyDialog({ target, onClose, onSaved }: {
  target: AdmissionPolicyTarget
  onClose: () => void
  onSaved: (fence: SessionFence) => void
}) {
  const contextKey = useAuthStore(state => state.contextKey)
  const epoch = useAuthStore(state => state.contextEpoch)
  const [session] = React.useState(captureAuthSession)
  const [fence] = React.useState(captureSessionFence)
  const mounted = React.useRef(true)
  const loaded = React.useRef(false)
  const writing = React.useRef(false)
  const queryClient = useQueryClient()
  const [snapshot, setSnapshot] = React.useState<AdmissionPolicy | null>(null)
  const [selected, setSelected] = React.useState<AdmissionMemberType[]>(["EMPLOYEE"])
  const [reason, setReason] = React.useState("")
  const [confirmed, setConfirmed] = React.useState(false)
  const [message, setMessage] = React.useState<string | null>(null)
  const [conflict, setConflict] = React.useState(false)
  const current = React.useCallback(() => mounted.current && session.profileReady && isCurrentAuthSession(session), [session])
  const assertCurrent = () => { if (!current()) throw new DOMException("准入策略视图已变化", "AbortError") }
  // Restore this lifetime before query subscriptions restart during StrictMode
  // effect replay; cleanup still fences real unmounts and late callbacks.
  React.useEffect(() => {
    mounted.current = true
    return () => { mounted.current = false }
  }, [])
  const queryKey = ["admission-policy", session.namespace, session.epoch, target.kind, target.id] as const
  const query = useQuery({
    queryKey,
    queryFn: async ({ signal }) => {
      assertCurrent()
      const policy = await admissionPoliciesApi.read(target, signal)
      assertCurrent()
      return policy
    },
    enabled: current(),
    staleTime: 0,
    retry: false,
    refetchOnWindowFocus: false,
  })
  const apply = React.useCallback((policy: AdmissionPolicy) => {
    loaded.current = true
    setSnapshot(policy)
    setSelected([...policy.allowedMemberTypes])
    setReason("")
    setConfirmed(false)
  }, [])
  React.useEffect(() => {
    if (!isCurrentAuthSession(session)) onClose()
  }, [contextKey, epoch, session, onClose])
  React.useEffect(() => {
    if (!loaded.current && query.data && current()) apply(query.data)
  }, [query.data, current, apply])

  const reload = async () => {
    setSnapshot(null)
    setReason("")
    setConfirmed(false)
    try {
      const result = await query.refetch({ throwOnError: true })
      assertCurrent()
      if (result.data) apply(result.data)
    } catch (error) {
      if (current() && !(error instanceof Error && error.name === "AbortError")) setMessage(error instanceof Error ? error.message : "当前策略读取失败，请重新加载")
    }
  }
  const save = useMutation({
    mutationFn: async (change: AdmissionPolicyChange) => {
      assertCurrent()
      const policy = await admissionPoliciesApi.save(target, change)
      assertCurrent()
      return policy
    },
    retry: false,
    onSuccess: policy => {
      if (!current()) return
      queryClient.setQueryData(queryKey, policy)
      toast.success("身份准入策略已保存，相关旧凭据将失效")
      onClose()
      onSaved(fence)
    },
    onError: async error => {
      if (!current() || error.name === "AbortError") return
      if (error instanceof ApiError && error.status === 409) {
        setConflict(true)
        setMessage("策略已被其他操作更新。已重新读取当前值，请重新选择身份、填写原因并确认影响后保存。")
        await reload()
      } else setMessage(error.message || "策略保存失败")
    },
    onSettled: () => { writing.current = false },
  })
  const readOnly = target.fixedEmployee === true
  const changed = !!snapshot && ["EMPLOYEE", "CUSTOMER"].some(type => snapshot.allowedMemberTypes.includes(type as AdmissionMemberType) !== selected.includes(type as AdmissionMemberType))
  const trimmedReason = reason.trim()
  const validReason = isAdmissionReasonValid(reason)
  const removed = snapshot?.allowedMemberTypes.filter(type => !selected.includes(type)) ?? []
  const canSave = !!snapshot && !readOnly && changed && selected.length > 0 && validReason && confirmed && !save.isPending && !query.isFetching && current()

  return <Dialog open onOpenChange={open => { if (!open && !writing.current) onClose() }}>
    <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-xl" showCloseButton={!save.isPending}>
      <DialogHeader>
        <DialogTitle className="flex items-center gap-2"><ShieldCheck className="size-5 text-primary" />身份准入</DialogTitle>
        <DialogDescription>{target.name} · 应用</DialogDescription>
      </DialogHeader>
      <div className="flex items-center justify-between gap-3 border-y py-3 text-xs text-muted-foreground">
        <span>当前策略<Badge variant="outline" className="ml-2 font-mono">{snapshot ? `v${snapshot.accessPolicyVersion}` : "读取中"}</Badge></span>
        <Button variant="ghost" size="sm" disabled={query.isFetching || save.isPending} onClick={() => { setMessage(null); void reload() }}>
          <RefreshCw className={query.isFetching ? "size-3.5 animate-spin" : "size-3.5"} />重新加载
        </Button>
      </div>
      {message && <Alert variant={conflict ? "default" : "destructive"}>
        <AlertTriangle /><AlertTitle>{conflict ? "请基于最新策略重新确认" : "操作未完成"}</AlertTitle><AlertDescription>{message}</AlertDescription>
      </Alert>}
      {!snapshot ? <div className="flex min-h-28 items-center justify-center gap-2 text-sm text-muted-foreground" role="status">
        {query.isFetching ? <><Loader2 className="size-4 animate-spin" />正在读取权威策略…</> : query.isError ? "策略读取失败，请重新加载" : "正在准备策略…"}
      </div> : <>
        {target.fixedEmployee && <p className="rounded-lg border bg-muted/30 p-3 text-sm">内置控制台来源固定为员工入口。请为客户入口单独登记客户端。</p>}
        <fieldset disabled={save.isPending || target.fixedEmployee} className="space-y-3">
          <legend className="mb-3 text-sm font-medium">允许的成员身份</legend>
          <div className="grid gap-3 sm:grid-cols-2">
            {(["EMPLOYEE", "CUSTOMER"] as const).map(type => <label key={type}
              className={`flex cursor-pointer items-start gap-3 rounded-lg border p-4 transition-colors ${selected.includes(type) ? "border-primary/50 bg-primary/5" : "hover:bg-muted/30"}`}>
              <Checkbox checked={selected.includes(type)} onCheckedChange={checked => {
                setSelected(previous => checked === true ? [...previous.filter(value => value !== type), type] : previous.filter(value => value !== type))
                setConfirmed(false)
              }} aria-label={`允许${admissionMemberLabels[type]}身份`} />
              <span className="min-w-0"><span className="flex items-center gap-1.5 text-sm font-medium"><UsersRound className="size-3.5" />{admissionMemberLabels[type]}</span>
                <span className="mt-1 block text-xs leading-relaxed text-muted-foreground">{type === "EMPLOYEE" ? "后台与内部业务；新建时的安全默认值" : "仅适用于已登记客户用途的入口"}</span></span>
            </label>)}
          </div>
          {selected.length === 0 && <p className="text-xs text-destructive" role="alert">应用至少允许一种成员身份。</p>}
        </fieldset>
        {!readOnly && <>
          <div className="space-y-2"><Label htmlFor="admission-policy-reason">变更原因<span className="text-destructive">*</span></Label>
            <Textarea id="admission-policy-reason" value={reason} maxLength={1000} disabled={save.isPending} rows={2}
              placeholder="说明本次调整的实际原因，1～500 字" onChange={event => setReason(event.target.value)} aria-describedby="admission-reason-count" />
            <p id="admission-reason-count" className="text-right text-xs text-muted-foreground">{Array.from(trimmedReason).length} / 500</p>
            {reason && !validReason && <p className="text-xs text-destructive">原因须为 1～500 字，且不能包含换行或其他控制字符。</p>}
          </div>
          <div className="rounded-lg border border-warning/35 bg-warning/5 p-4 text-sm">
            <p className="font-medium">{removed.length ? `将收紧${removed.map(type => admissionMemberLabels[type]).join("、")}准入` : "策略变更将更新授权版本"}</p>
            <p className="mt-1 text-xs leading-relaxed text-muted-foreground">相关旧会话、授权码和令牌将失效，仍符合准入条件的人员需要重新授权。应用与来源客户端的允许集合共同生效。</p>
            <label className="mt-3 flex items-start gap-2 text-xs"><Checkbox checked={confirmed} disabled={save.isPending} onCheckedChange={value => setConfirmed(value === true)} />我已确认对现有登录和授权的影响</label>
          </div>
        </>}
      </>}
      <DialogFooter>
        <Button variant="outline" disabled={save.isPending} onClick={onClose}>{readOnly ? "关闭" : "取消"}</Button>
        {!readOnly && <Button disabled={!canSave} onClick={() => {
          if (!canSave || writing.current || !snapshot) return
          writing.current = true
          setMessage(null); setConflict(false)
          save.mutate({ allowedMemberTypes: [...selected], accessPolicyVersion: snapshot.accessPolicyVersion, reason: trimmedReason })
        }}>{save.isPending && <Loader2 className="size-4 animate-spin" />}保存准入策略</Button>}
      </DialogFooter>
    </DialogContent>
  </Dialog>
}
