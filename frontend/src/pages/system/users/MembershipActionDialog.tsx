import * as React from "react"
import { useQuery } from "@tanstack/react-query"
import { ArrowRight, Loader2 } from "lucide-react"
import { toast } from "sonner"
import { membershipsApi } from "@/api/memberships"
import type { MembershipCapabilities, MembershipMemberType, MembershipRecord, MembershipStatus } from "@/api/memberships"
import { useAuthStore } from "@/stores/auth"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Label } from "@/components/ui/label"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { EntityTreePicker, type PickerNode } from "@/components/EntityTreePicker"
import { ErrorState } from "@/components/states"
import { usersApi } from "@/api/users"
import { newMemberPayload, type NewMemberAccount } from "./new-member-policy"
import { buildIdentityTransition, buildMembershipDelete, buildMembershipStatusChange, currentEmployeeMutationBlockedReason, membershipWriteBlockedReason, oppositeMemberType } from "./membership-model"

export type MembershipAction =
  | { kind: "create"; memberType: MembershipMemberType }
  | { kind: "status"; source: MembershipRecord; nextStatus: MembershipStatus }
  | { kind: "delete" | "secondIdentity" | "replaceIdentity"; source: MembershipRecord }

const typeLabel = (type: MembershipMemberType) => type === "EMPLOYEE" ? "员工" : "客户"
const actionLabel = (action: MembershipAction) => action.kind === "create" ? `新建${typeLabel(action.memberType)}` : action.kind === "status" ? `${action.nextStatus === "ENABLED" ? "启用" : "停用"}成员` : action.kind === "delete" ? "删除成员关系" : action.kind === "secondIdentity" ? "新增第二身份" : "替换身份"

function IdentityCard({ title, member, memberType }: { title: string; member?: MembershipRecord | null; memberType: MembershipMemberType }) {
  return <div className="min-w-0 flex-1 rounded-lg border bg-muted/20 p-3">
    <p className="mb-2 text-xs text-muted-foreground">{title}</p>
    <div className="flex flex-wrap items-center gap-1.5"><Badge variant={memberType === "EMPLOYEE" ? "secondary" : "outline"}>{typeLabel(memberType)}</Badge>{member ? <><span className="text-xs">{member.status === "ENABLED" ? "已启用" : "已停用"}</span><span className="font-mono text-xs text-muted-foreground">v{member.identityVersion}</span></> : <span className="text-xs text-muted-foreground">将建立独立成员关系</span>}</div>
    {member && <p className="mt-2 break-all font-mono text-[11px] text-muted-foreground">{member.id}</p>}
  </div>
}

export function MembershipActionDialog({ action, namespace, capabilities, organizations, onClose, onSaved }: {
  action: MembershipAction
  namespace: string | null
  capabilities?: MembershipCapabilities
  organizations: PickerNode[]
  onClose: () => void
  onSaved: () => void
}) {
  const currentMembershipId = useAuthStore(state => state.identity?.membershipId)
  const [newAccount, setNewAccount] = React.useState<NewMemberAccount>({ username: "", password: "", realName: "", phone: "", email: "" })
  const recoveryKey = `member-account-operation:${namespace}:${action.kind === "create" ? action.memberType : "other"}`
  const [pendingOperation, setPendingOperation] = React.useState<string | null>(() => action.kind === "create" ? sessionStorage.getItem(recoveryKey) : null)
  const [organizationId, setOrganizationId] = React.useState("")
  const [reason, setReason] = React.useState("")
  const [bindingSource, setBindingSource] = React.useState("")
  const [bindingConfirmed, setBindingConfirmed] = React.useState(false)
  const [reuseTarget, setReuseTarget] = React.useState(false)
  const [enableTarget, setEnableTarget] = React.useState(false)
  const [position, setPosition] = React.useState("")
  const [saving, setSaving] = React.useState(false)
  const [error, setError] = React.useState<string | null>(null)
  const savingRef = React.useRef(false)
  const alive = React.useRef(true)
  React.useEffect(() => { alive.current = true; return () => { alive.current = false } }, [])
  const sourceId = action.kind === "create" ? undefined : action.source.id
  const sourceQuery = useQuery({
    queryKey: ["membership-detail", namespace, sourceId],
    queryFn: ({ signal }) => membershipsApi.detail(sourceId!, signal),
    enabled: !!namespace && !!sourceId,
    staleTime: 0,
    retry: 1,
  })
  const source = action.kind === "create" ? null : sourceQuery.data
  const targetType = action.kind === "create" ? action.memberType : oppositeMemberType(action.source.memberType)
  const transition = action.kind === "secondIdentity" || action.kind === "replaceIdentity"
  const create = action.kind === "create"
  const creatingAccount = create
  const targetUserId = create ? undefined : source?.userId
  const targetOrgId = create ? organizationId : source?.organizationId
  const targetQuery = useQuery({
    queryKey: ["membership-counterpart", namespace, targetUserId, targetOrgId],
    queryFn: ({ signal }) => membershipsApi.list({ userId: targetUserId!, organizationId: targetOrgId!, page: 1, size: 10 }, signal),
    enabled: !!namespace && !!targetUserId && !!targetOrgId && (create || transition),
    staleTime: 0,
    retry: 1,
  })
  const target = targetQuery.data?.records.find(row => row.memberType === targetType) ?? null
  const targetProof = creatingAccount || !create && !transition || targetQuery.isSuccess && !targetQuery.isFetching
  const sourceProof = create || sourceQuery.isSuccess && !sourceQuery.isFetching
  const proofSignature = `${targetUserId ?? ""}:${targetOrgId ?? ""}:${target?.id ?? "new"}:${target?.identityVersion ?? ""}:${source?.identityVersion ?? ""}`
  React.useEffect(() => { setReuseTarget(false); setEnableTarget(false); setBindingConfirmed(false) }, [proofSignature])
  const gate = membershipWriteBlockedReason(capabilities, action.kind, create || transition ? targetType : undefined)
  const selfGate = source && (action.kind === "status" || action.kind === "delete" || action.kind === "replaceIdentity") ? currentEmployeeMutationBlockedReason(source, currentMembershipId) : null
  const blocked = gate || selfGate || (!namespace ? "当前身份尚未确认" : !sourceProof || !targetProof ? "正在重新核验源成员与目标成员" : null)
  const ready = !blocked && !saving && !pendingOperation && (!create || !!organizationId && !!newAccount.phone.trim())
  const close = () => { if (!savingRef.current && !pendingOperation) onClose() }
  const finish = () => { sessionStorage.removeItem(recoveryKey);setPendingOperation(null);onSaved();onClose() }

  const submit = async () => {
    if (!ready || savingRef.current) return
    setError(null)
    savingRef.current = true
    setSaving(true)
    try {
      let successMessage = `${actionLabel(action)}成功`
      if (action.kind === "create") {
        const result = await usersApi.create(newMemberPayload(newAccount, organizationId, action.memberType, position, reason))
        if (result.accountReused) successMessage = `已自动关联已有账号并新建${typeLabel(action.memberType)}`
      } else if (!source) throw new Error("源成员尚未加载")
      else if (action.kind === "status") await membershipsApi.changeStatus(source.id, buildMembershipStatusChange(source, action.nextStatus, reason, capabilities, currentMembershipId))
      else if (action.kind === "delete") await membershipsApi.remove(source.id, buildMembershipDelete(source, reason, capabilities, currentMembershipId))
      else {
        const body = buildIdentityTransition(source, { mode: action.kind, memberType: targetType, reason, target, reuseTargetConfirmed: reuseTarget, enableTarget, accountBindingConfirmed: bindingConfirmed, bindingSource }, capabilities, currentMembershipId)
        if (action.kind === "secondIdentity") await membershipsApi.secondIdentity(source.id, body)
        else await membershipsApi.replaceIdentity(source.id, body)
      }
      if (!alive.current) return
      toast.success(successMessage)
      finish()
    } catch (cause) {
      if (alive.current) {
        const message = cause instanceof Error ? cause.message : "操作失败，请重新核验成员后重试"
        setError(message)
        const operation = message.match(/操作编号\s+([0-9a-f-]{36})/i)?.[1]
        if (creatingAccount && operation) { setPendingOperation(operation);sessionStorage.setItem(recoveryKey,operation) }
      }
    } finally {
      savingRef.current = false
      if (alive.current) setSaving(false)
    }
  }

  return <Dialog open onOpenChange={open => !open && close()}>
    <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-2xl" onEscapeKeyDown={event => { if (saving) event.preventDefault() }} onPointerDownOutside={event => { if (saving) event.preventDefault() }}>
      <DialogHeader>
        <DialogTitle>{actionLabel(action)}</DialogTitle>
        <DialogDescription>{create ? "填写电话和目标组织，后台自动匹配账号。新电话需填写姓名、用户名和初始密码；已有账号沿用原资料。" : "操作具体成员关系；账号及同组织的另一身份保持独立。"}</DialogDescription>
      </DialogHeader>
      <div className="space-y-4">
        {!create && sourceQuery.isError ? <ErrorState message="源成员读取失败，不能提交变更" onRetry={() => sourceQuery.refetch()} /> : null}
        {create ? <>
          <div className="grid gap-4 sm:grid-cols-2">
            {([['phone','电话','tel',30],['realName','姓名（新账号填写）','text',100],['username','用户名（新账号填写）','text',100],['password','初始密码（新账号填写）','password',1024],['email','邮箱（可选）','email',100]] as const).map(([key,label,type,maxLength]) => <div className="space-y-2" key={key}><Label htmlFor={`new-member-${key}`}>{label}{key === 'phone' && <span className="text-destructive"> *</span>}</Label><Input id={`new-member-${key}`} type={type} autoComplete={key === 'password' ? 'new-password' : 'off'} required={key === 'phone'} maxLength={maxLength} value={newAccount[key]} disabled={saving || !!pendingOperation} onChange={event => setNewAccount(previous => ({...previous,[key]:event.target.value}))} />{key === 'phone' && <p className="text-xs text-muted-foreground">必填，已有账号自动识别并关联。</p>}{key === 'email' && <p className="text-xs text-muted-foreground">邮箱须唯一；电话和邮箱属于不同账号时不能合并。</p>}</div>)}
          </div>
          <div className="space-y-2"><Label>目标组织 <span className="text-destructive">*</span></Label><EntityTreePicker nodes={organizations} value={organizationId} onSelect={id => setOrganizationId(id ?? "")} placeholder="选择管理范围内的组织" disabled={saving || !!pendingOperation} ariaLabel="成员目标组织" /></div>
          <IdentityCard title="新增成员类型" memberType={action.memberType} />
          {action.memberType === "EMPLOYEE" && <div className="space-y-2"><Label htmlFor="member-position">岗位（可选）</Label><Input id="member-position" value={position} maxLength={50} onChange={event => setPosition(event.target.value)} disabled={saving} /></div>}
        </> : source ? <>
          <div className="space-y-1 rounded-lg border p-3"><p className="text-xs text-muted-foreground">账号 ID / 组织 ID</p><p className="break-all font-mono text-xs">{source.userId}</p><p className="break-all font-mono text-xs text-muted-foreground">{source.organizationId}</p></div>
          {transition ? <div className="flex flex-col gap-2 sm:flex-row sm:items-center"><IdentityCard title={action.kind === "replaceIdentity" ? "源成员 · 完成后停用" : "源成员 · 保留原状态"} member={source} memberType={source.memberType} /><ArrowRight className="mx-auto size-4 shrink-0 text-muted-foreground sm:mx-0" /><IdentityCard title={target ? "目标成员 · 显式复用" : "目标成员 · 新增"} member={target} memberType={targetType} /></div> : <IdentityCard title="本次操作的具体成员" member={source} memberType={source.memberType} />}
        </> : <p className="text-sm text-muted-foreground">正在重新核验成员…</p>}
        {(create || transition) && targetQuery.isError && <ErrorState message="目标成员读取失败，不能推断目标不存在" onRetry={() => targetQuery.refetch()} />}
        {transition && target && <div className="space-y-3 rounded-lg border p-3">
          <label className="flex items-start gap-2 text-sm"><Checkbox checked={reuseTarget} onCheckedChange={value => setReuseTarget(value === true)} disabled={saving} /><span>我已核对目标成员 ID 与 v{target.identityVersion}，明确复用此关系。</span></label>
          {target.status === "DISABLED" && <label className="flex items-start gap-2 text-sm"><Checkbox checked={enableTarget} onCheckedChange={value => setEnableTarget(value === true)} disabled={saving} /><span>明确启用已停用的目标。旧会话和旧代授权不会因此恢复。</span></label>}
        </div>}
        {transition && targetType === "CUSTOMER" && <div className="space-y-3 rounded-lg border p-3">
          <label className="flex items-start gap-2 text-sm"><Checkbox checked={bindingConfirmed} onCheckedChange={value => setBindingConfirmed(value === true)} disabled={saving} /><span>已核实该既有账号的归属，确认将客户关系绑定到显示的精确账号 ID。</span></label>
          <div className="space-y-2"><Label htmlFor="member-binding-source">绑定依据 <span className="text-destructive">*</span></Label><Input id="member-binding-source" value={bindingSource} onChange={event => setBindingSource(event.target.value)} maxLength={200} placeholder="填写管理员已经核实的绑定来源" disabled={saving} /></div>
        </div>}
        <div className="rounded-lg border-l-2 border-primary bg-muted/30 px-3 py-2.5 text-sm text-muted-foreground">
          {action.kind === "secondIdentity" ? "新增第二身份保留源成员，不属于登录切换；两个成员拥有独立状态、凭据与通知。" : action.kind === "replaceIdentity" ? "替换会停用源成员并使其旧会话失效，源数据授权会清理；历史通知不会改绑目标。" : action.kind === "delete" ? "删除的是此条组织成员关系，原成员凭据立即失效，历史收件记录保留其原始归属。全局账号及其他成员关系不会删除。" : action.kind === "status" ? action.nextStatus === "ENABLED" ? "启用产生新的身份代际，旧会话不会恢复。" : "停用只撤销此成员资格；员工角色配置可以保留，但停用期间不授予权限。" : "新成员使用独立成员 ID。管理端展示客户关系不代表客户可使用旧控制台登录。"}
        </div>
        <div className="space-y-2"><Label htmlFor="membership-reason">本次操作原因 {create ? "（可选）" : <span className="text-destructive">*</span>}</Label><Textarea id="membership-reason" value={reason} onChange={event => setReason(event.target.value)} maxLength={500} rows={3} placeholder={create ? "可补充创建原因" : "说明实际业务原因"} disabled={saving || !!pendingOperation} /></div>
        {blocked && <p role="status" className="rounded-lg border bg-muted/40 p-3 text-sm text-muted-foreground">{blocked}</p>}
        {pendingOperation && <div role="status" className="space-y-2 rounded-lg border p-3 text-sm"><p>创建结果待确认，请查询原操作，避免重复创建。</p><Button type="button" size="sm" disabled={saving} onClick={async () => {
          if (savingRef.current) return
          savingRef.current=true;setSaving(true)
          try { await usersApi.recoverAccountOperation(pendingOperation);toast.success("成员创建已完成");finish() }
          catch (cause) { setError(cause instanceof Error ? cause.message : "原操作结果仍待确认") }
          finally { savingRef.current=false;if(alive.current)setSaving(false) }
        }}>查询并恢复原操作</Button></div>}
        {error && <div role="alert" className="space-y-2 rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"><p>{error}</p>{!creatingAccount && <Button type="button" variant="outline" size="sm" disabled={saving || sourceQuery.isFetching || targetQuery.isFetching} onClick={() => { setError(null); if (!create) void sourceQuery.refetch(); if (create || transition) void targetQuery.refetch() }}>重新核验成员</Button>}</div>}
      </div>
      <DialogFooter><Button variant="outline" disabled={saving || !!pendingOperation} onClick={close}>取消</Button><Button variant={action.kind === "delete" || action.kind === "replaceIdentity" ? "destructive" : "default"} onClick={submit} disabled={!ready}>{saving && <Loader2 className="size-4 animate-spin" />}{actionLabel(action)}</Button></DialogFooter>
    </DialogContent>
  </Dialog>
}
