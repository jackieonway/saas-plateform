import * as React from "react"
import { AlertCircle, ArrowRight, Building2, CheckCircle2, Loader2, LogOut, Search, ShieldCheck, User } from "lucide-react"
import { BrandMark } from "@/components/brand"
import { ThemeToggleGroup } from "@/components/theme-toggle"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import {
  AlertDialog, AlertDialogCancel, AlertDialogContent, AlertDialogDescription,
  AlertDialogFooter, AlertDialogHeader, AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { useDocumentTitle } from "@/hooks/use-document-title"
import { usePlatformBranding } from "@/hooks/use-platform-branding"
import type { Membership } from "@/api/types"
import type { BffSessionView } from "@/lib/bff-session"
import { canSelectConsoleMembership, contextIdentityKey, membershipSelectionDisabledReason, membershipSelectionKey } from "@/lib/context-identity"
import { cn } from "@/lib/utils"

export interface BffLoginPageProps {
  session: BffSessionView | null
  returnTo: string
  onLogin: () => void
  onReload: () => Promise<void>
  onSelect: (member: Membership) => Promise<void>
  onLogout: (scope: "PLATFORM" | "GLOBAL") => Promise<void>
}

type Pending = { kind: "login" | "reload" | "platform-logout" | "global-logout" } | { kind: "select"; key: string }
type Operation = { view: string }

const failureMessage = (error: unknown) => error instanceof Error && error.message.trim()
  ? error.message : "操作暂时未能完成，请重新检查登录状态后再试。"

/** The parent owns authentication, session fencing and navigation. This page only collects explicit intent. */
export default function BffLoginPage({ session, returnTo, onLogin, onReload, onSelect, onLogout }: BffLoginPageProps) {
  const { platformName, platformDescription, footerText } = usePlatformBranding()
  useDocumentTitle(session ? "选择员工身份" : "统一登录")
  const [search, setSearch] = React.useState("")
  const [pending, setPending] = React.useState<Pending | null>(null)
  const [error, setError] = React.useState<string | null>(null)
  const [confirmGlobalLogout, setConfirmGlobalLogout] = React.useState(false)
  const errorRef = React.useRef<HTMLDivElement>(null)
  const mounted = React.useRef(false)
  const operation = React.useRef<Operation | null>(null)
  const view = session ? JSON.stringify([session.sessionId, session.version, session.account.id,
    session.context ? contextIdentityKey(session.context) : null]) : "anonymous"
  const currentView = React.useRef(view)
  const currentSession = React.useRef(session)
  currentView.current = view
  currentSession.current = session

  React.useEffect(() => {
    mounted.current = true
    return () => { mounted.current = false; operation.current = null }
  }, [])
  React.useEffect(() => {
    operation.current = null
    setPending(null)
    setError(null)
    setSearch("")
    setConfirmGlobalLogout(false)
  }, [view])
  React.useEffect(() => { if (error) errorRef.current?.focus() }, [error])

  const perform = async (action: Pending, work: () => void | Promise<void>) => {
    if (!mounted.current || operation.current) return
    const captured: Operation = { view: currentView.current }
    operation.current = captured
    setPending(action)
    setError(null)
    const current = () => mounted.current && operation.current === captured && currentView.current === captured.view
    try { await work() }
    catch (failure) {
      if (current() && !(failure instanceof Error && failure.name === "AbortError")) setError(failureMessage(failure))
    } finally {
      if (current()) { operation.current = null; setPending(null) }
    }
  }

  const members = React.useMemo(() => session?.memberships.filter(member => member.userId === session.account.id) ?? [], [session])
  const selectableCount = members.filter(canSelectConsoleMembership).length
  const query = search.trim().toLocaleLowerCase()
  const groups = React.useMemo(() => {
    const grouped = new Map<string, { id: string; name: string; members: Membership[] }>()
    for (const member of members) {
      if (query && ![member.tenantName, member.organizationName, member.organizationPath, member.departmentName,
        member.username, member.realName].some(value => value?.toLocaleLowerCase().includes(query))) continue
      const group = grouped.get(member.tenantId) ?? { id: member.tenantId, name: member.tenantName, members: [] }
      group.members.push(member)
      grouped.set(member.tenantId, group)
    }
    return [...grouped.values()]
  }, [members, query])
  const matchingCount = groups.reduce((count, group) => count + group.members.length, 0)
  const busy = pending !== null
  const accountName = session?.account.realName.trim() || session?.account.username
  const destination = returnTo === "/" ? "进入平台工作台" : "继续您刚才的访问"

  const select = (member: Membership) => {
    if (operation.current) return
    const latest = currentSession.current
    const key = membershipSelectionKey(member)
    const candidate = latest?.memberships.find(value => membershipSelectionKey(value) === key)
    if (!latest || !candidate || candidate.userId !== latest.account.id || !canSelectConsoleMembership(candidate)) {
      setError("此员工身份已不可选择，请重新检查登录状态。")
      return
    }
    void perform({ kind: "select", key }, () => onSelect(candidate))
  }

  const status = pending?.kind === "select" ? "正在确认所选员工身份…"
    : pending?.kind === "reload" ? "正在重新检查登录状态…"
      : pending?.kind === "login" ? "正在打开统一登录…"
        : pending ? "正在退出登录…" : ""

  return (
    <div className="flex min-h-dvh flex-col bg-background text-foreground">
      <a href="#bff-login-content" className="sr-only focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:z-50 focus:rounded-md focus:bg-primary focus:px-4 focus:py-3 focus:text-primary-foreground">跳到登录内容</a>
      <header className="mx-auto flex w-full max-w-6xl items-center justify-between gap-5 px-5 py-6 sm:px-10 sm:py-8">
        <div className="flex min-w-0 items-center gap-3">
          <span className="flex size-10 shrink-0 items-center justify-center rounded-xl border bg-card text-primary"><BrandMark className="size-6" /></span>
          <div className="min-w-0"><p className="break-words text-base font-semibold tracking-tight">{platformName}</p><p className="mt-0.5 hidden break-words text-sm text-muted-foreground sm:block">{platformDescription}</p></div>
        </div>
        <ThemeToggleGroup />
      </header>

      <div className="mx-auto grid w-full max-w-6xl flex-1 items-center gap-12 px-5 pb-12 pt-4 sm:px-10 lg:grid-cols-[0.85fr_1.15fr] lg:gap-20 lg:pb-20">
        <aside className="hidden self-center lg:block" aria-label="访问步骤">
          <div className="mb-7 h-px w-12 bg-primary" aria-hidden="true" />
          <p className="mb-4 text-sm font-medium text-primary">统一账号 · 明确身份</p>
          <h2 className="text-4xl font-semibold leading-[1.4] tracking-tight">继续您的工作，<br />从合适的身份开始。</h2>
          <p className="mt-5 max-w-sm text-base leading-8 text-muted-foreground">账号由统一认证中心验证。进入平台前，选择本次使用的员工身份，访问对应租户与组织的工作内容。</p>
          <ol className="mt-10 space-y-5 text-sm">
            {["确认统一账号", "选择员工与组织身份", "按实际权限进入平台"].map((step, index) => <li key={step} className="flex items-center gap-4"><span className="font-mono text-xs text-muted-foreground" aria-hidden="true">0{index + 1}</span><span>{step}</span></li>)}
          </ol>
        </aside>

        <main id="bff-login-content" tabIndex={-1} className="w-full min-w-0 rounded-2xl border bg-card p-6 shadow-sm outline-offset-4 sm:p-9" aria-busy={busy}>
          <div className="mb-7">
            <p className="mb-3 flex items-center gap-2 text-sm font-medium text-primary"><ShieldCheck className="size-4" aria-hidden="true" />平台访问</p>
            <h1 className="text-2xl font-semibold leading-snug tracking-tight sm:text-3xl">{!session ? "继续统一登录" : selectableCount ? "选择本次员工身份" : "账号已认证"}</h1>
            <p className="mt-3 text-sm leading-7 text-muted-foreground">{!session ? "前往认证中心确认账号后，返回这里选择员工身份。" : selectableCount ? `请选择一个有效员工身份，${destination}。` : "当前账号暂无可用于此平台的有效员工身份。"}</p>
          </div>

          {error && <Alert ref={errorRef} variant="destructive" tabIndex={-1} className="mb-6 outline-offset-4"><AlertCircle className="size-4" aria-hidden="true" /><AlertTitle>操作未完成</AlertTitle><AlertDescription className="break-words">{error}</AlertDescription></Alert>}
          {busy && <p className="mb-5 flex items-center gap-2 text-sm text-muted-foreground" aria-hidden="true"><Loader2 className="size-4 animate-spin" />{status}</p>}

          {!session ? <div className="space-y-5">
            <div className="rounded-xl border border-dashed bg-muted/25 px-5 py-6"><p className="font-medium">在认证中心安全验证账号</p><p className="mt-2 text-sm leading-7 text-muted-foreground">此页面无需输入密码。只有在您点击下方按钮后，才会开始统一登录。</p></div>
            <Button type="button" className="h-12 w-full text-base" disabled={busy} onClick={() => { void perform({ kind: "login" }, onLogin) }}>
              {pending?.kind === "login" ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}{pending?.kind === "login" ? "正在打开统一登录…" : "继续统一登录"}<ArrowRight className="size-4" aria-hidden="true" />
            </Button>
            <Button type="button" variant="ghost" className="min-h-11 w-full" disabled={busy} onClick={() => { void perform({ kind: "reload" }, onReload) }}>重新检查登录状态</Button>
          </div> : <>
            <div className="mb-6 flex items-center gap-3 rounded-xl border bg-muted/25 px-4 py-4">
              <span className="flex size-10 shrink-0 items-center justify-center rounded-full bg-primary/10 text-primary"><User className="size-5" aria-hidden="true" /></span>
              <div className="min-w-0 flex-1"><p className="text-xs text-muted-foreground">已认证账号</p><p className="mt-0.5 break-words text-base font-semibold">{accountName}</p>{accountName !== session.account.username && <p className="mt-0.5 break-all text-sm text-muted-foreground">{session.account.username}</p>}</div>
              <CheckCircle2 className="size-4 shrink-0 text-primary" aria-label="账号已认证" />
            </div>

            {selectableCount > 0 ? <>
              <label className="sr-only" htmlFor="bff-membership-search">搜索租户、组织或员工身份</label>
              <div className="relative"><Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" /><Input id="bff-membership-search" value={search} onChange={event => setSearch(event.target.value)} placeholder="搜索租户、组织或部门" className="h-11 pl-9 text-base md:text-base" autoComplete="off" disabled={busy} /></div>
              <p className="my-3 text-xs text-muted-foreground" role="status" aria-live="polite">{query ? `找到 ${matchingCount} 个成员身份` : `${selectableCount} 个有效员工身份，请明确选择`}</p>
              <div className="max-h-[52dvh] space-y-6 overflow-y-auto pr-1 lg:max-h-[28rem]">
                {groups.map(group => <section key={group.id} aria-labelledby={`bff-tenant-${group.id}`}>
                  <h2 id={`bff-tenant-${group.id}`} className="mb-2 break-words text-sm font-semibold">{group.name}</h2>
                  <ul className="space-y-2">{group.members.map(member => {
                    const key = membershipSelectionKey(member)
                    const allowed = canSelectConsoleMembership(member)
                    const selected = pending?.kind === "select" && pending.key === key
                    const reason = membershipSelectionDisabledReason(member)
                    return <li key={key}><button type="button" data-testid={`bff-membership-${member.membershipId}`} disabled={busy || !allowed} onClick={() => select(member)} className={cn("group flex min-h-20 w-full items-center gap-3 rounded-xl border bg-background p-4 text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-default", allowed ? "hover:border-primary/50 hover:bg-primary/5" : "bg-muted/30 text-muted-foreground")}>
                      <Building2 className={cn("size-5 shrink-0", allowed ? "text-primary" : "text-muted-foreground")} aria-hidden="true" />
                      <span className="min-w-0 flex-1"><span className="flex flex-wrap items-center gap-x-2 gap-y-1"><span className="break-words text-base font-medium">{member.organizationName}</span><span className="rounded border px-1.5 py-0.5 text-xs">{member.memberType === "EMPLOYEE" ? "员工" : "客户"} · {member.status === "ENABLED" ? "有效" : "已停用"}</span></span><span className="mt-1 block break-words text-sm leading-6 text-muted-foreground">{member.organizationPath}{member.memberType === "EMPLOYEE" && member.departmentName ? ` · ${member.departmentName}` : ""}</span>{reason && <span className="mt-1 block text-sm leading-6 text-muted-foreground">{reason}</span>}</span>
                      {selected ? <Loader2 className="size-4 shrink-0 animate-spin text-primary" aria-label="正在确认身份" /> : allowed ? <ArrowRight className="size-4 shrink-0 text-muted-foreground transition-colors group-hover:text-primary" aria-hidden="true" /> : null}
                    </button></li>
                  })}</ul>
                </section>)}
                {groups.length === 0 && <p className="rounded-xl border border-dashed px-4 py-8 text-center text-sm leading-7 text-muted-foreground">没有匹配的租户、组织或员工身份。请尝试其他关键词。</p>}
              </div>
            </> : <div className="rounded-xl border border-dashed px-5 py-6">
              <p className="font-medium">暂无有效员工身份</p>
              <p className="mt-2 text-sm leading-7 text-muted-foreground">账号认证不会自动创建组织或成员关系。请联系管理员确认您的员工身份；客户身份请使用对应的客户入口。</p>
            </div>}

            <div className="mt-6 border-t pt-5">
              <Button type="button" variant="outline" className="mb-3 min-h-11 w-full" disabled={busy} onClick={() => { void perform({ kind: "login" }, onLogin) }}>
                {pending?.kind === "login" && <Loader2 className="size-4 animate-spin" aria-hidden="true" />}重新确认账号<ArrowRight className="size-4" aria-hidden="true" />
              </Button>
              <div className="flex flex-col gap-2 sm:flex-row"><Button type="button" variant="outline" className="min-h-11 flex-1" disabled={busy} onClick={() => { void perform({ kind: "reload" }, onReload) }}>{pending?.kind === "reload" && <Loader2 className="size-4 animate-spin" aria-hidden="true" />}重新检查</Button><Button type="button" variant="ghost" className="min-h-11 flex-1" disabled={busy} onClick={() => { void perform({ kind: "platform-logout" }, () => onLogout("PLATFORM")) }}><LogOut className="size-4" aria-hidden="true" />退出本平台</Button></div>
              <p className="mt-3 text-xs leading-6 text-muted-foreground">退出本平台不会自动重新发起登录。需要结束统一登录时，可选择下方操作。</p>
              <Button type="button" variant="link" className="mt-1 min-h-11 px-0 text-sm text-muted-foreground" disabled={busy} onClick={() => setConfirmGlobalLogout(true)}>退出所有应用</Button>
            </div>
          </>}
        </main>
      </div>

      <div className="sr-only" role="status" aria-live="polite" aria-atomic="true">{status}</div>
      {footerText && <footer className="mx-auto w-full max-w-6xl break-words px-5 pb-6 text-center text-xs leading-6 text-muted-foreground sm:px-10">{footerText}</footer>}

      <AlertDialog open={confirmGlobalLogout} onOpenChange={setConfirmGlobalLogout}>
        <AlertDialogContent>
          <AlertDialogHeader><AlertDialogTitle>退出所有应用？</AlertDialogTitle><AlertDialogDescription>这会结束当前账号的统一登录及其关联应用会话。再次访问时，需要重新登录。</AlertDialogDescription></AlertDialogHeader>
          <AlertDialogFooter><AlertDialogCancel disabled={busy}>保留登录</AlertDialogCancel><Button type="button" variant="destructive" disabled={busy} onClick={() => { setConfirmGlobalLogout(false); void perform({ kind: "global-logout" }, () => onLogout("GLOBAL")) }}>确认退出所有应用</Button></AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
