import * as React from "react"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { z } from "zod"
import { ArrowRight, Building2, CheckCircle2, ClipboardCheck, ExternalLink, FileCheck2, KeyRound, Loader2, LockKeyhole, MailPlus, Plus, RefreshCw, ShieldCheck, UserRoundCheck } from "lucide-react"
import { identityReviewApi } from "@/api/identity-review"
import type { IntakeApplication, IntakeDecision, IntakeManifest, IntakeRequest, IntakeStatus } from "@/api/identity-review"
import { ApiError } from "@/lib/api"
import { isUuidV7 } from "@/lib/uuid"
import { useAuthStore } from "@/stores/auth"
import { Badge } from "@/components/ui/badge"
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import { toast } from "sonner"

const uuid = z.string().trim().refine(isUuidV7, "请输入规范 UUIDv7 证明引用").transform(value => value.toLowerCase())
const text = (maximum: number) => z.string().trim().min(1, "请填写此项").max(maximum).refine(value => ![...value].some(character => character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127), "不能包含控制字符")
const partnerSchema = z.object({ name: text(120), contactName: text(80), contactChannel: text(256), subjectReference: uuid }).strict()
const applicationSchema = z.object({ partnerId: uuid, displayName: text(120) }).strict()
const exactHttps = z.string().refine(raw => {
  try {
    const url = new URL(raw)
    return url.protocol === "https:" && !!url.hostname && !url.username && !url.password && !url.hash && url.href === raw
      && !raw.includes("*") && !["code", "state", "error", "iss"].some(key => url.searchParams.has(key))
  } catch { return false }
}, "请输入精确 HTTPS 地址，不含通配符或协议响应参数")
const manifestSchema = z.object({
  displayName: text(120), purpose: text(1000).refine(value => !["PLATFORM_CONSOLE", "CUSTOMER_PORTAL"].includes(value), "不能使用内部应用用途"),
  privacyUrl: exactHttps, domains: z.array(z.string().regex(/^[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?$/)).min(1).max(16),
  redirectUris: z.array(exactHttps).min(1).max(16), postLogoutRedirectUris: z.array(exactHttps).max(16),
  scopes: z.array(z.enum(["openid", "profile"])).min(1).max(2), contactReference: uuid, domainEvidenceReference: uuid,
  logoutMode: z.enum(["SHORT_SESSION_ONLINE_CHECK", "BACK_CHANNEL"]),
  backchannelLogoutUri: exactHttps.nullable().optional(),
}).strict().superRefine((value, ctx) => {
  for (const field of ["domains", "redirectUris", "postLogoutRedirectUris", "scopes"] as const) {
    if (new Set(value[field]).size !== value[field].length) ctx.addIssue({ code: "custom", path: [field], message: "不能包含重复项" })
  }
  if (!value.scopes.includes("openid")) ctx.addIssue({ code: "custom", path: ["scopes"], message: "人员接入必须包含 openid" })
  const domains = new Set(value.domains)
  if (value.logoutMode === "BACK_CHANNEL") {
    if (!value.backchannelLogoutUri) ctx.addIssue({ code: "custom", path: ["backchannelLogoutUri"], message: "back-channel审核必须填写精确HTTPS地址" })
    else { try { const url = new URL(value.backchannelLogoutUri); if (url.search || !domains.has(url.hostname)) ctx.addIssue({ code: "custom", path: ["backchannelLogoutUri"], message: "back-channel地址须无查询参数且属于当前审核域名" }) } catch { /* Address field reports its own validation error. */ } }
  } else if (value.backchannelLogoutUri) ctx.addIssue({ code: "custom", path: ["backchannelLogoutUri"], message: "短会话合同不配置back-channel地址" })
  if (value.domains.some(domain => !domain.includes(".") || domain.includes("..") || domain.split(".").some(label => label.startsWith("-") || label.endsWith("-") || label.length > 63))) {
    ctx.addIssue({ code: "custom", path: ["domains"], message: "域名须为明确的规范 DNS 主机名" })
  }
  for (const uri of [value.privacyUrl, ...value.redirectUris, ...value.postLogoutRedirectUris]) {
    try { if (!domains.has(new URL(uri).hostname)) ctx.addIssue({ code: "custom", path: ["redirectUris"], message: "隐私说明与回调必须属于本次域名清单" }) } catch { /* URL field reports its own validation error. */ }
  }
})

interface ManifestDraft {
  displayName: string; purpose: string; privacyUrl: string; domains: string; redirectUris: string; postLogoutRedirectUris: string
  contactReference: string; domainEvidenceReference: string; logoutMode: IntakeManifest["logoutMode"]
  backchannelLogoutUri: string
}
const emptyDraft: ManifestDraft = { displayName: "", purpose: "", privacyUrl: "", domains: "", redirectUris: "", postLogoutRedirectUris: "", contactReference: "", domainEvidenceReference: "", logoutMode: "SHORT_SESSION_ONLINE_CHECK", backchannelLogoutUri: "" }
const lines = (raw: string) => raw.split(/[\r\n,]+/).map(value => value.trim()).filter(Boolean)
function buildManifest(draft: ManifestDraft): IntakeManifest {
  return manifestSchema.parse({ ...draft, backchannelLogoutUri: draft.backchannelLogoutUri || undefined, domains: lines(draft.domains), redirectUris: lines(draft.redirectUris), postLogoutRedirectUris: lines(draft.postLogoutRedirectUris), scopes: ["openid", "profile"] })
}
function draftFrom(request: IntakeRequest): ManifestDraft {
  return { ...request.manifest, backchannelLogoutUri: request.manifest.backchannelLogoutUri ?? "", domains: request.manifest.domains.join("\n"), redirectUris: request.manifest.redirectUris.join("\n"), postLogoutRedirectUris: request.manifest.postLogoutRedirectUris.join("\n") }
}
function message(error: unknown): string {
  if (error instanceof z.ZodError) return error.issues[0]?.message ?? "请检查表单"
  return error instanceof Error ? error.message : "操作未完成，请重试查询"
}
const statusLabel: Record<IntakeStatus, string> = { SUBMITTED: "待审核", RETURNED: "已退回", TEST_APPROVED: "测试已批准", TEST_ACCEPTED: "测试已验收", PRODUCTION_APPROVED: "生产已批准", ENABLED: "生产已启用", SUSPENDED: "已暂停" }
function date(raw: string) { return new Intl.DateTimeFormat("zh-CN", { dateStyle: "medium", timeStyle: "short" }).format(new Date(raw)) }
function safeSecurityUrl(raw?: string) {
  try { const url = new URL(raw ?? ""); return url.protocol === "https:" && !url.username && !url.password ? url.href : undefined } catch { return undefined }
}
function safeReauthenticateUrl(raw: string | undefined, origin: string) {
  if (!raw || raw !== raw.trim() || raw.includes("\\") || raw.includes("#") || [...raw].some(character => character.charCodeAt(0) <= 32 || character.charCodeAt(0) === 127)) return undefined
  try {
    const url = new URL(raw, origin)
    return url.origin === origin && !url.username && !url.password && url.pathname === "/auth/step-up"
      && url.searchParams.size === 1 && url.searchParams.getAll("return").length === 1 && url.searchParams.get("return") === "/system/oauth/clients"
      ? "/auth/step-up?return=/system/oauth/clients" : undefined
  } catch { return undefined }
}
function applicant(request: IntakeRequest, app: IntakeApplication, actor: string | null) {
  return !!actor && [request.requestedBy, request.submittedBy, app.createdBy].includes(actor)
}
function allowedDecisions(request: IntakeRequest): IntakeStatus[] {
  if (request.status === "SUBMITTED") return ["RETURNED", request.environment === "test" ? "TEST_APPROVED" : "PRODUCTION_APPROVED"]
  if (request.status === "TEST_APPROVED") return ["TEST_ACCEPTED", "SUSPENDED"]
  if (request.status === "PRODUCTION_APPROVED") return ["ENABLED", "SUSPENDED"]
  return ["TEST_ACCEPTED", "ENABLED"].includes(request.status) ? ["SUSPENDED"] : []
}
function needsSync(request: IntakeRequest) { return ["TEST_APPROVED", "TEST_ACCEPTED", "ENABLED", "SUSPENDED"].includes(request.status) && request.syncedVersion !== request.version }
function directoryAllowed(permissions: ReadonlySet<string>, verifiedAdministrator: boolean) {
  return permissions.has("system:oauth:register") && (verifiedAdministrator || permissions.has("system:oauth:review") || permissions.has("system:oauth:assign"))
}
const actionLabel: Partial<Record<IntakeStatus, string>> = { RETURNED: "退回申请", TEST_APPROVED: "批准测试接入", TEST_ACCEPTED: "记录测试验收", PRODUCTION_APPROVED: "批准生产申请", ENABLED: "启用生产接入", SUSPENDED: "暂停接入" }

export default function PartnerIntakePanel({ mode, onClose, onUnknown }: { mode: "partners" | "invitations"; onClose: () => void; onUnknown: () => void }) {
  const queryClient = useQueryClient()
  const scope = useAuthStore(state => `${state.contextKey ?? ""}:${state.sessionId ?? ""}`)
  const actor = useAuthStore(state => state.identity?.userId ?? null)
  const verifiedAdministrator = useAuthStore(state => state.profile?.user.isSuperAdmin === true)
  const [tab, setTab] = React.useState(mode)
  const [saving, setSaving] = React.useState(false)
  const [conflict, setConflict] = React.useState(false)
  const [error, setError] = React.useState<string | null>(null)
  const writeLock = React.useRef(false)
  const [partnerDraft, setPartnerDraft] = React.useState({ name: "", contactName: "", contactChannel: "", subjectReference: "" })
  const [appDraft, setAppDraft] = React.useState({ partnerId: "", displayName: "" })
  const [selectedApp, setSelectedApp] = React.useState("")
  const [requestDraft, setRequestDraft] = React.useState<ManifestDraft>(emptyDraft)
  const [environment, setEnvironment] = React.useState<"test" | "production">("test")
  const [editingVersion, setEditingVersion] = React.useState<number | null>(null)
  const [editorOpen, setEditorOpen] = React.useState(false)
  const [decision, setDecision] = React.useState<{ request: IntakeRequest; status: IntakeStatus } | null>(null)
  const [decisionReason, setDecisionReason] = React.useState("")
  const [contactEvidence, setContactEvidence] = React.useState("")
  const [domainEvidence, setDomainEvidence] = React.useState("")
  const [testEvidence, setTestEvidence] = React.useState("")
  const [confirmed, setConfirmed] = React.useState(false)
  const [operator, setOperator] = React.useState("")
  const [recipient, setRecipient] = React.useState("")
  const [reauthenticateConfirm, setReauthenticateConfirm] = React.useState(false)
  const securityQuery = useQuery({ queryKey: ["identity-review", "security", scope], queryFn: identityReviewApi.security, retry: false, staleTime: 15_000 })
  const security = securityQuery.isSuccess ? securityQuery.data : undefined
  const permissions = new Set(security?.permissions ?? [])
  const can = (name: string) => permissions.has(`system:oauth:${name}`)
  const writeReady = !!security?.totpEnabled && !!security?.recent && !saving && !conflict
  const canRegister = can("register")
  const canDirectory = directoryAllowed(permissions, verifiedAdministrator)
  const partnersQuery = useQuery({ queryKey: ["identity-review", "partners", scope], queryFn: identityReviewApi.partners, enabled: tab === "partners", retry: false })
  const applicationsQuery = useQuery({ queryKey: ["identity-review", "applications", scope], queryFn: identityReviewApi.applications, enabled: tab === "partners", retry: false })
  const invitationsQuery = useQuery({ queryKey: ["identity-invitations", scope], queryFn: identityReviewApi.invitations, enabled: tab === "invitations" && can("invite"), retry: false })
  const partners = partnersQuery.data?.partners ?? []
  const applications = applicationsQuery.data?.applications ?? []
  const application = applications.find(row => row.applicationId === selectedApp) ?? null
  const productionReady = security?.productionReady === true && applicationsQuery.data?.productionReady === true
  const securityHref = safeSecurityUrl(security?.securityUrl)
  const reauthenticateHref = safeReauthenticateUrl(security?.reauthenticateUrl, window.location.origin)

  const reload = async () => {
    setError(null)
    setDecision(null)
    setEditorOpen(false)
    setRequestDraft(emptyDraft)
    setEditingVersion(null)
    await Promise.all([queryClient.invalidateQueries({ queryKey: ["identity-review"] }), queryClient.invalidateQueries({ queryKey: ["identity-invitations"] })])
    setConflict(false)
  }
  const run = async (success: string, task: () => Promise<unknown>) => {
    if (writeLock.current || !writeReady) return
    writeLock.current = true
    setSaving(true)
    setError(null)
    try {
      await task()
      toast.success(success)
      await Promise.all([queryClient.invalidateQueries({ queryKey: ["identity-review"] }), queryClient.invalidateQueries({ queryKey: ["identity-invitations"] }), queryClient.invalidateQueries({ queryKey: ["identity-clients"] })])
    } catch (failure) {
      if (failure instanceof Error && failure.name === "AbortError") return
      if (failure instanceof ApiError && failure.code === "OIDC_REMOTE_RESULT_UNKNOWN") { onClose(); onUnknown(); return }
      if (failure instanceof ApiError && failure.status === 409) setConflict(true)
      setError(message(failure))
    } finally { writeLock.current = false; setSaving(false) }
  }
  const newRequest = () => {
    const existing = new Set(application?.requests.map(request => request.environment) ?? [])
    const next = !existing.has("test") ? "test" : !existing.has("production") ? "production" : null
    if (!next) { toast.info("两个环境均已有申请，请在对应申请卡片中修订"); return }
    setEditingVersion(null)
    setEnvironment(next)
    setRequestDraft({ ...emptyDraft, displayName: application?.displayName ?? "" })
    setDecision(null)
    setEditorOpen(true)
  }
  const editRequest = (request: IntakeRequest) => {
    setEditingVersion(request.version)
    setEnvironment(request.environment)
    setRequestDraft(draftFrom(request))
    setDecision(null)
    setEditorOpen(true)
  }
  const openDecision = (request: IntakeRequest, status: IntakeStatus) => {
    setDecision({ request, status })
    setDecisionReason(""); setContactEvidence(""); setDomainEvidence(""); setTestEvidence(""); setConfirmed(false)
    setEditorOpen(false)
  }
  const submitDecision = () => run("接入状态已更新；实际生效范围以同步结果为准", async () => {
    if (!decision) return
    const body: IntakeDecision = { expectedVersion: decision.request.version, status: decision.status, reason: text(1000).min(10, "请填写至少 10 字的审核或处置说明").parse(decisionReason) }
    if (["TEST_APPROVED", "PRODUCTION_APPROVED"].includes(decision.status)) {
      if (!confirmed) throw new Error("请先确认已人工核对本次资料和控制证明")
      body.verifiedContactReference = uuid.parse(contactEvidence)
      body.verifiedDomainReference = uuid.parse(domainEvidence)
    }
    if (decision.status === "TEST_ACCEPTED") body.testEvidenceReference = uuid.parse(testEvidence)
    await identityReviewApi.decide(decision.request, body)
    setDecision(null)
  })

  return <Dialog open onOpenChange={open => { if (!open && !writeLock.current) onClose() }}>
    <DialogContent className="flex max-h-[calc(100dvh-1rem)] flex-col gap-0 overflow-hidden p-0 [&>button]:text-white sm:max-w-6xl">
      <DialogHeader className="shrink-0 border-b bg-[#12332f] px-5 py-5 pr-12 text-left text-white sm:px-6">
        <div className="mb-2 flex items-center gap-2 font-mono text-[10px] uppercase tracking-[0.2em] text-emerald-100/65"><ShieldCheck className="size-3.5" />Identity / controlled admission</div>
        <DialogTitle className="text-xl">合作方接入与账号邀请</DialogTitle>
        <DialogDescription className="max-w-3xl text-xs leading-5 text-emerald-50/70">按申请、独立审核、测试与生产授权推进。合作方资料不会赋予租户管理权；客户端和凭据继续使用统一注册簿。</DialogDescription>
      </DialogHeader>
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b bg-muted/20 px-5 py-3 sm:px-6">
        <Button size="sm" variant={tab === "partners" ? "default" : "ghost"} onClick={() => setTab("partners")}><Building2 className="size-3.5" />合作方接入</Button>
        <Button size="sm" variant={tab === "invitations" ? "default" : "ghost"} onClick={() => setTab("invitations")}><MailPlus className="size-3.5" />账号邀请</Button>
        <Button className="sm:ml-auto" size="sm" variant="outline" disabled={saving} onClick={() => void reload()}><RefreshCw className="size-3.5" />{conflict ? "载入最新状态" : "刷新状态"}</Button>
      </div>
      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-4 sm:p-6">
        <div className={`flex flex-col gap-3 rounded-xl border p-4 sm:flex-row sm:items-center sm:justify-between ${security?.recent ? "border-emerald-500/25 bg-emerald-500/5" : "border-amber-500/30 bg-amber-500/5"}`}>
          <div className="flex items-start gap-3"><KeyRound className="mt-0.5 size-4 shrink-0" /><div><p className="text-sm font-medium">{security?.recent ? "最近 5 分钟的 TOTP 二次认证有效" : security?.totpEnabled ? "敏感操作需要重新完成 TOTP 二次认证" : "先在认证中心绑定并验证 TOTP"}</p><p className="mt-1 text-xs leading-5 text-muted-foreground">账号安全页用于绑定和管理 TOTP；重新验证平台登录会建立新的平台会话，完成后返回客户端页。权限和二次认证均由服务端复验。</p>{security && !security.totpEnabled && <p className="mt-1 text-xs leading-5 text-muted-foreground">首次未绑定时，先重新验证平台登录，再打开账号安全页绑定并验证 TOTP；如仍提示需要近期认证，再重新验证平台登录。此处不收集或保存认证器密钥。</p>}</div></div>
          <div className="flex shrink-0 flex-wrap gap-2 sm:max-w-48 sm:justify-end">
            {reauthenticateHref && <Button size="sm" disabled={saving} onClick={() => setReauthenticateConfirm(true)}>重新验证平台登录<ArrowRight className="size-3.5" /></Button>}
            {securityHref && <Button asChild size="sm" variant="outline"><a href={securityHref} target="_blank" rel="noreferrer">打开账号安全页<ExternalLink className="size-3.5" /></a></Button>}
          </div>
        </div>
        {!productionReady && tab === "partners" && <Notice icon={<LockKeyhole className="size-4" />} title="生产审批与启用保持关闭">实际审核／发布责任和数据政策尚未完成绑定。可以整理申请和验证测试流程，当前状态不代表生产已获批准。</Notice>}
        {securityQuery.isError && <Notice title="无法核验当前管理权限">{message(securityQuery.error)}。敏感写入暂不可用；请刷新状态。</Notice>}
        {error && <div role="alert" className="rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm text-destructive">{error}{conflict && <p className="mt-1 text-xs">当前草稿保留但已锁定。请明确载入最新状态，重新检查资料后操作。</p>}</div>}
        {tab === "partners" ? <>
          {canDirectory && <details className="rounded-xl border bg-background"><summary className="cursor-pointer px-4 py-3 text-sm font-semibold">登记合作方与应用</summary><div className="grid gap-5 border-t p-4 lg:grid-cols-2">
            <form className="space-y-3" onSubmit={event => { event.preventDefault(); void run("合作方资料已登记", async () => { await identityReviewApi.createPartner(partnerSchema.parse(partnerDraft)); setPartnerDraft({ name: "", contactName: "", contactChannel: "", subjectReference: "" }) }) }}>
              <SectionTitle icon={<Building2 className="size-4" />} title="01 / 合作方资料" />
              <Field label="合作方主体名称" id="intake-partner-name"><Input id="intake-partner-name" value={partnerDraft.name} onChange={event => setPartnerDraft({ ...partnerDraft, name: event.target.value })} /></Field>
              <div className="grid gap-3 sm:grid-cols-2"><Field label="责任联系人" id="intake-partner-contact"><Input id="intake-partner-contact" value={partnerDraft.contactName} onChange={event => setPartnerDraft({ ...partnerDraft, contactName: event.target.value })} /></Field><Field label="联系渠道" id="intake-partner-channel"><Input id="intake-partner-channel" value={partnerDraft.contactChannel} onChange={event => setPartnerDraft({ ...partnerDraft, contactChannel: event.target.value })} /></Field></div>
              <Field label="主体资料证明引用 UUIDv7" id="intake-partner-proof"><Input id="intake-partner-proof" className="font-mono text-xs" value={partnerDraft.subjectReference} onChange={event => setPartnerDraft({ ...partnerDraft, subjectReference: event.target.value })} /></Field>
              <Button type="submit" size="sm" disabled={!writeReady}><Plus className="size-3.5" />登记合作方</Button>
            </form>
            <form className="space-y-3" onSubmit={event => { event.preventDefault(); void run("合作方应用已登记，请提交环境申请", async () => { const app = await identityReviewApi.createApplication(applicationSchema.parse(appDraft)); setSelectedApp(app.applicationId); setAppDraft({ partnerId: "", displayName: "" }) }) }}>
              <SectionTitle icon={<FileCheck2 className="size-4" />} title="02 / 申请应用" />
              <Field label="所属合作方"><Select value={appDraft.partnerId || "NONE"} onValueChange={value => setAppDraft({ ...appDraft, partnerId: value === "NONE" ? "" : value })}><SelectTrigger aria-label="应用所属合作方"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="NONE">选择已登记合作方</SelectItem>{partners.map(row => <SelectItem key={row.id} value={row.id}>{row.name}</SelectItem>)}</SelectContent></Select></Field>
              <Field label="应用名称" id="intake-app-name"><Input id="intake-app-name" value={appDraft.displayName} onChange={event => setAppDraft({ ...appDraft, displayName: event.target.value })} /></Field>
              <p className="text-xs leading-5 text-muted-foreground">合作方与应用绑定后不可原地转移。仅明确委派的操作人可管理对应应用；登记不代表 scope、回调或生产权限已批准。</p>
              <Button type="submit" size="sm" disabled={!writeReady || partners.length === 0}><Plus className="size-3.5" />登记合作方应用</Button>
            </form>
          </div></details>}
          <section className="grid gap-4 lg:grid-cols-[16rem_minmax(0,1fr)]">
            <aside className="space-y-3 rounded-xl border bg-muted/15 p-3"><SectionTitle title="应用与环境申请" icon={<ClipboardCheck className="size-4" />} />
              {applicationsQuery.isError ? <p className="text-xs text-destructive">{message(applicationsQuery.error)}</p> : applications.length === 0 ? <p className="p-2 text-xs leading-5 text-muted-foreground">尚无可查看的明确应用。合作方操作人只会看到本人已获委派的条目。</p> : applications.map(app => <button type="button" key={app.applicationId} onClick={() => { setSelectedApp(app.applicationId); setEditorOpen(false); setDecision(null) }} className={`w-full rounded-lg border p-3 text-left transition-colors ${selectedApp === app.applicationId ? "border-primary/35 bg-primary/8" : "border-transparent bg-background hover:border-border"}`}><span className="block text-sm font-medium">{app.displayName}</span><span className="mt-1 block truncate text-[11px] text-muted-foreground">{partners.find(row => row.id === app.partnerId)?.name ?? "已登记合作方"}</span><code className="mt-2 block truncate font-mono text-[9px] text-muted-foreground">{app.applicationId}</code></button>)}
            </aside>
            <div className="min-w-0 space-y-4">
              {!application ? <div className="flex min-h-48 flex-col items-center justify-center gap-3 rounded-xl border border-dashed p-6 text-center"><FileCheck2 className="size-7 text-muted-foreground/50" /><p className="text-sm text-muted-foreground">选择应用，核对资料与接入阶段</p></div> : <>
                <div className="flex flex-wrap items-center justify-between gap-3"><div><h3 className="text-lg font-semibold">{application.displayName}</h3><p className="mt-1 text-xs text-muted-foreground">第三方 · USER / ACCOUNT_ONLY · openid / profile</p></div>{canRegister && <Button size="sm" variant="outline" disabled={!writeReady} onClick={newRequest}><Plus className="size-3.5" />提交环境申请</Button>}</div>
                {application.requests.length === 0 && <p className="rounded-lg border border-dashed p-4 text-sm text-muted-foreground">尚未提交测试或生产环境申请。</p>}
                {application.requests.map(request => {
                  const self = applicant(request, application, actor)
                  return <article key={request.id} className="space-y-3 rounded-xl border bg-background p-4">
                    <div className="flex flex-wrap items-start justify-between gap-2"><div className="flex flex-wrap items-center gap-2"><Badge variant={request.environment === "production" ? "default" : "secondary"}>{request.environment === "production" ? "生产环境" : "测试环境"}</Badge><span className="text-sm font-semibold">{statusLabel[request.status]}</span><span className="font-mono text-[10px] text-muted-foreground">v{request.version}</span></div><Badge variant="outline" className="text-[10px]">{["SUBMITTED", "RETURNED"].includes(request.status) ? "申请资料待审核" : request.status === "PRODUCTION_APPROVED" ? "等待生产发布授权" : request.syncedVersion === request.version ? "已同步统一权威" : "尚未确认当前版本同步"}</Badge></div>
                    <div className="flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">{["提交资料", "独立审核", request.environment === "test" ? "测试验收" : "生产启用"].map((label, index) => <React.Fragment key={label}>{index > 0 && <ArrowRight className="size-3" />}<span>{String(index + 1).padStart(2, "0")} {label}</span></React.Fragment>)}</div>
                    <dl className="grid gap-3 rounded-lg bg-muted/25 p-3 text-xs sm:grid-cols-2"><Info label="用途" value={request.manifest.purpose} /><Info label="声明范围" value={request.manifest.scopes.join(" / ")} /><Info label="回调地址" value={request.manifest.redirectUris.join("\n")} mono /><Info label="域名清单" value={request.manifest.domains.join("\n")} mono /><Info label="负责人证明引用" value={request.manifest.contactReference} mono /><Info label="域名控制证明引用" value={request.manifest.domainEvidenceReference} mono /></dl>
                    <div className="grid gap-3 rounded-lg border p-3 text-xs sm:grid-cols-2"><Info label="登出合同" value={request.manifest.logoutMode === "BACK_CHANNEL" ? "独立审核 back-channel" : "短会话与在线检查"} />{request.manifest.backchannelLogoutUri && <Info label="已提交 back-channel 地址" value={request.manifest.backchannelLogoutUri} mono />}</div>
                    <a className="inline-flex items-center gap-1 text-xs text-primary underline-offset-4 hover:underline" href={safeSecurityUrl(request.manifest.privacyUrl)} target="_blank" rel="noreferrer">查看申请隐私说明<ExternalLink className="size-3" /></a>
                    {request.reason && <p className="rounded-lg border-l-2 border-primary/40 bg-muted/20 px-3 py-2 text-xs leading-5">上次说明：{request.reason}</p>}
                    {self && <p className="text-xs text-amber-700 dark:text-amber-300">申请、最近提交或应用登记人员不能审核、测试验收或启用自己的申请。</p>}
                    <div className="flex flex-wrap gap-2">
                      {canRegister && <Button size="sm" variant="outline" disabled={!writeReady} onClick={() => editRequest(request)}>修订并重新提交</Button>}
                      {allowedDecisions(request).map(status => {
                        const required = status === "ENABLED" ? "publish" : status === "SUSPENDED" ? "disable" : "review"
                        const production = ["PRODUCTION_APPROVED", "ENABLED"].includes(status)
                        const deniedSelf = status !== "SUSPENDED" && self
                        return <Button key={status} size="sm" variant={status === "RETURNED" || status === "SUSPENDED" ? "outline" : "default"} disabled={!writeReady || !can(required) || deniedSelf || (production && !productionReady)} title={deniedSelf ? "申请人不得自批" : production && !productionReady ? "生产责任与政策尚未绑定" : !can(required) ? "缺少相应操作权限" : undefined} onClick={() => openDecision(request, status)}>{actionLabel[status]}</Button>
                      })}
                      {needsSync(request) && <Button size="sm" variant="outline" disabled={!writeReady || !(can("review") || can("publish") || can("disable"))} onClick={() => void run("已查询并同步本次批准范围", () => identityReviewApi.sync(request))}><RefreshCw className="size-3.5" />同步已批准状态</Button>}
                    </div>
                  </article>
                })}
                {can("assign") && <form className="space-y-3 rounded-xl border p-4" onSubmit={event => { event.preventDefault(); void run("应用操作人已明确委派", async () => { await identityReviewApi.assignOperator(application.applicationId, uuid.parse(operator)); setOperator("") }) }}><SectionTitle icon={<UserRoundCheck className="size-4" />} title="明确委派应用操作人" /><div className="flex flex-col gap-2 sm:flex-row"><Input aria-label="应用操作人 UUIDv7" className="font-mono text-xs" value={operator} onChange={event => setOperator(event.target.value)} placeholder="员工账号 UUIDv7" /><Button type="submit" size="sm" disabled={!writeReady}>委派操作人</Button></div><p className="text-xs leading-5 text-muted-foreground">委派只限定可操作的应用，仍须具备对应员工管理权限；不会授予租户管理员身份。</p></form>}
              </>}
            </div>
          </section>
          {editorOpen && application && <form className="space-y-4 rounded-xl border border-primary/30 bg-primary/3 p-4" onSubmit={event => { event.preventDefault(); void run("环境申请已提交，等待独立审核", async () => { const manifest = buildManifest(requestDraft); await identityReviewApi.submit(application.applicationId, { environment, expectedVersion: editingVersion, manifest }); setEditorOpen(false) }) }}>
            <SectionTitle icon={<FileCheck2 className="size-4" />} title={`${editingVersion ? "修订" : "提交"}环境申请`} />
            <div className="grid gap-3 sm:grid-cols-2"><Field label="申请环境"><Select value={environment} disabled={editingVersion !== null} onValueChange={value => setEnvironment(value as typeof environment)}><SelectTrigger aria-label="申请环境"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="test" disabled={editingVersion === null && application.requests.some(request => request.environment === "test")}>测试环境</SelectItem><SelectItem value="production" disabled={editingVersion === null && application.requests.some(request => request.environment === "production")}>生产环境</SelectItem></SelectContent></Select></Field><Field label="申请显示名称" id="intake-request-name"><Input id="intake-request-name" value={requestDraft.displayName} onChange={event => setRequestDraft({ ...requestDraft, displayName: event.target.value })} /></Field></div>
            <Field label="具体用途" id="intake-request-purpose"><Textarea id="intake-request-purpose" value={requestDraft.purpose} onChange={event => setRequestDraft({ ...requestDraft, purpose: event.target.value })} /></Field>
            <div className="grid gap-3 sm:grid-cols-2"><Field label="隐私说明 HTTPS 地址" id="intake-request-privacy"><Input id="intake-request-privacy" value={requestDraft.privacyUrl} onChange={event => setRequestDraft({ ...requestDraft, privacyUrl: event.target.value })} /></Field><Field label="精确域名 · 每行一个" id="intake-request-domains"><Textarea id="intake-request-domains" className="font-mono text-xs" value={requestDraft.domains} onChange={event => setRequestDraft({ ...requestDraft, domains: event.target.value })} /></Field><Field label="回调地址 · 每行一个" id="intake-request-redirects"><Textarea id="intake-request-redirects" className="font-mono text-xs" value={requestDraft.redirectUris} onChange={event => setRequestDraft({ ...requestDraft, redirectUris: event.target.value })} /></Field><Field label="退出后返回地址 · 可选" id="intake-request-logout"><Textarea id="intake-request-logout" className="font-mono text-xs" value={requestDraft.postLogoutRedirectUris} onChange={event => setRequestDraft({ ...requestDraft, postLogoutRedirectUris: event.target.value })} /></Field><Field label="联系人验证证明引用 UUIDv7" id="intake-request-contact-ref"><Input id="intake-request-contact-ref" className="font-mono text-xs" value={requestDraft.contactReference} onChange={event => setRequestDraft({ ...requestDraft, contactReference: event.target.value })} /></Field><Field label="域名控制证明引用 UUIDv7" id="intake-request-domain-ref"><Input id="intake-request-domain-ref" className="font-mono text-xs" value={requestDraft.domainEvidenceReference} onChange={event => setRequestDraft({ ...requestDraft, domainEvidenceReference: event.target.value })} /></Field><Field label="退出接入合同"><Select value={requestDraft.logoutMode} onValueChange={value => setRequestDraft({ ...requestDraft, logoutMode: value as ManifestDraft["logoutMode"], backchannelLogoutUri: value === "SHORT_SESSION_ONLINE_CHECK" ? "" : requestDraft.backchannelLogoutUri })}><SelectTrigger aria-label="退出接入合同"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="SHORT_SESSION_ONLINE_CHECK">短会话与在线检查</SelectItem><SelectItem value="BACK_CHANNEL">独立审核 back-channel 地址</SelectItem></SelectContent></Select></Field><div className="rounded-lg border bg-background p-3"><p className="text-xs font-medium">首版资料范围固定</p><div className="mt-2 flex gap-1.5"><Badge variant="outline">openid</Badge><Badge variant="outline">profile</Badge></div><p className="mt-2 text-[11px] leading-5 text-muted-foreground">不申请 email、phone、离线授权或业务资源。</p></div></div>
            {requestDraft.logoutMode === "BACK_CHANNEL" && <Field label="待独立审核的 back-channel HTTPS 地址" id="intake-request-backchannel"><Input id="intake-request-backchannel" className="font-mono text-xs" value={requestDraft.backchannelLogoutUri} onChange={event => setRequestDraft({ ...requestDraft, backchannelLogoutUri: event.target.value })} /><p className="mt-2 text-xs leading-5 text-muted-foreground">仅接受当前明确域名内、无查询参数的精确HTTPS地址。审核此地址不等于owner已核验实际投递目标；客户端还需独立绑定当前目标事实。</p></Field>}
            <p className="text-xs leading-5 text-muted-foreground">以上引用用于独立人工核验；提交资料不代表联系人、域名或生产接入已获批准。修改范围需要重新审核。</p>
            <div className="flex flex-wrap justify-end gap-2"><Button type="button" size="sm" variant="ghost" onClick={() => setEditorOpen(false)}>取消申请编辑</Button><Button type="submit" size="sm" disabled={!writeReady || !canRegister}>提交审核资料</Button></div>
          </form>}
          {decision && <section className="space-y-3 rounded-xl border border-primary/30 bg-primary/3 p-4" aria-label="接入决策表单"><SectionTitle icon={<CheckCircle2 className="size-4" />} title={actionLabel[decision.status] ?? "记录接入决策"} /><p className="text-xs text-muted-foreground">{decision.request.manifest.displayName} · {decision.request.environment} · expected v{decision.request.version}</p><Field label="审核或处置说明" id="intake-decision-reason"><Textarea id="intake-decision-reason" value={decisionReason} onChange={event => setDecisionReason(event.target.value)} /></Field>
            {["TEST_APPROVED", "PRODUCTION_APPROVED"].includes(decision.status) && <><div className="grid gap-3 sm:grid-cols-2"><Field label="已核验联系人证明引用 UUIDv7" id="intake-verified-contact"><Input id="intake-verified-contact" className="font-mono text-xs" value={contactEvidence} onChange={event => setContactEvidence(event.target.value)} /></Field><Field label="已核验域名证明引用 UUIDv7" id="intake-verified-domain"><Input id="intake-verified-domain" className="font-mono text-xs" value={domainEvidence} onChange={event => setDomainEvidence(event.target.value)} /></Field></div><label className="flex items-start gap-2 rounded-lg border bg-background p-3 text-xs leading-5"><Checkbox checked={confirmed} onCheckedChange={value => setConfirmed(value === true)} /><span>我已人工核对本次主体、责任联系人、用途与域名控制证明；此声明不代替实际生产责任和数据政策绑定。</span></label></>}
            {decision.status === "TEST_ACCEPTED" && <Field label="测试验收证据引用 UUIDv7" id="intake-test-evidence"><Input id="intake-test-evidence" className="font-mono text-xs" value={testEvidence} onChange={event => setTestEvidence(event.target.value)} /></Field>}
            <div className="flex flex-wrap justify-end gap-2"><Button size="sm" variant="ghost" onClick={() => setDecision(null)}>取消决策</Button><Button size="sm" disabled={!writeReady} onClick={() => void submitDecision()}>确认{actionLabel[decision.status]}</Button></div>
          </section>}
        </> : <>
          <Notice icon={<MailPlus className="size-4" />} title="受控邀请，24 小时有效">邀请绑定收件人和 ACCOUNT_ONBOARDING 用途，消费一次后失效。此处仅管理状态，不展示 token、不开放公开注册；通过已验证渠道的实际投递由后续受控流程完成。</Notice>
          {!can("invite") ? <p className="rounded-xl border p-5 text-sm text-muted-foreground">当前员工没有账号邀请管理权限。</p> : <>
            <form className="space-y-3 rounded-xl border p-4" onSubmit={event => { event.preventDefault(); void run("账号邀请已签发，等待受控投递流程", async () => { await identityReviewApi.invite(text(256).parse(recipient)); setRecipient("") }) }}><SectionTitle title="签发账号邀请" icon={<MailPlus className="size-4" />} /><div className="flex flex-col gap-2 sm:flex-row"><Input aria-label="邀请收件人引用" value={recipient} onChange={event => setRecipient(event.target.value)} placeholder="明确收件人引用，不填写 token 或验证码" /><Button type="submit" size="sm" disabled={!writeReady}>签发受控邀请</Button></div><p className="text-xs leading-5 text-muted-foreground">收件人或用途发生变化必须签发新邀请。旧邀请可明确撤销，原记录不会被编辑成另一人的邀请。</p></form>
            {invitationsQuery.isError ? <p className="text-sm text-destructive">{message(invitationsQuery.error)}</p> : (invitationsQuery.data?.invitations ?? []).length === 0 ? <p className="rounded-xl border border-dashed p-5 text-sm text-muted-foreground">尚无可查看的账号邀请。</p> : <div className="space-y-3">{invitationsQuery.data?.invitations.map(invitation => <article key={invitation.id} className="flex flex-col gap-3 rounded-xl border p-4 sm:flex-row sm:items-center sm:justify-between"><div className="min-w-0"><div className="flex flex-wrap items-center gap-2"><p className="break-all text-sm font-medium">{invitation.recipientRef}</p><Badge variant={invitation.status === "ACTIVE" ? "secondary" : "outline"}>{invitation.status}</Badge></div><p className="mt-1 font-mono text-[10px] text-muted-foreground">ACCOUNT_ONBOARDING · v{invitation.version}</p><p className="mt-1 text-xs text-muted-foreground">签发 {date(invitation.createdAt)} · 到期 {date(invitation.expiresAt)}</p><code className="mt-1 block break-all font-mono text-[10px] text-muted-foreground">{invitation.id}</code></div>{invitation.status === "ACTIVE" && <Button size="sm" variant="outline" disabled={!writeReady} onClick={() => void run("账号邀请已撤销", () => identityReviewApi.revokeInvitation(invitation))}>撤销此邀请</Button>}</article>)}</div>}
          </>}
        </>}
      </div>
      <div className="flex shrink-0 items-center justify-between gap-3 border-t bg-background px-5 py-3 sm:px-6"><p className="flex items-center gap-2 text-[11px] text-muted-foreground">{saving && <Loader2 className="size-3.5 animate-spin" />}{saving ? "正在提交；结果未知时只查询原操作" : "客户端凭据轮换仍使用统一注册簿入口"}</p><Button size="sm" variant="outline" disabled={saving} onClick={onClose}>关闭接入管理</Button></div>
    </DialogContent>
    <AlertDialog open={reauthenticateConfirm} onOpenChange={setReauthenticateConfirm}>
      <AlertDialogContent>
        <AlertDialogHeader><AlertDialogTitle>重新验证平台登录</AlertDialogTitle><AlertDialogDescription>将离开当前页面并建立新的平台登录会话。未提交的申请、审核和邀请草稿不会保留；完成登录后返回统一客户端页，重新获取权限和认证状态。</AlertDialogDescription></AlertDialogHeader>
        <AlertDialogFooter><AlertDialogCancel>继续编辑</AlertDialogCancel>{reauthenticateHref && <AlertDialogAction asChild><a href={reauthenticateHref}>开始重新验证</a></AlertDialogAction>}</AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </Dialog>
}

function Field({ label, id, children }: { label: string; id?: string; children: React.ReactNode }) { return <div className="space-y-1.5"><Label htmlFor={id} className="text-xs">{label}</Label>{children}</div> }
function SectionTitle({ title, icon }: { title: string; icon?: React.ReactNode }) { return <h3 className="flex items-center gap-2 text-sm font-semibold">{icon}{title}</h3> }
function Info({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) { return <div className="min-w-0"><dt className="text-[10px] text-muted-foreground">{label}</dt><dd className={`mt-1 whitespace-pre-line break-all leading-5 ${mono ? "font-mono text-[10px]" : ""}`}>{value}</dd></div> }
function Notice({ title, children, icon }: { title: string; children: React.ReactNode; icon?: React.ReactNode }) { return <div className="flex items-start gap-3 rounded-xl border border-amber-500/25 bg-amber-500/5 p-4">{icon && <span className="mt-0.5 shrink-0 text-amber-700 dark:text-amber-300">{icon}</span>}<div><p className="text-sm font-semibold">{title}</p><p className="mt-1 text-xs leading-5 text-muted-foreground">{children}</p></div></div> }
