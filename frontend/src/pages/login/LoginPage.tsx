import * as React from "react"
import { Link, useNavigate, useSearchParams } from "react-router-dom"
import { useForm } from "react-hook-form"
import { z } from "zod"
import { zResolver } from "@/lib/form"
import { motion } from "framer-motion"
import {
  AlertCircle,
  ArrowLeft,
  ArrowRight,
  Building2,
  CheckCircle2,
  Eye,
  EyeOff,
  Globe,
  Layers,
  LayoutGrid,
  Loader2,
  Lock,
  MonitorSmartphone,
  Search,
  Server,
  ShieldCheck,
  Tv,
  User,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Checkbox } from "@/components/ui/checkbox"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { ThemeToggleGroup } from "@/components/theme-toggle"
import { BrandMark } from "@/components/brand"
import { authApi } from "@/api/auth"
import { captureSessionFence, useAuthStore, type SessionFence } from "@/stores/auth"
import { useDocumentTitle } from "@/hooks/use-document-title"
import { usePlatformBranding } from "@/hooks/use-platform-branding"
import { toast } from "sonner"
import { cn } from "@/lib/utils"
import { ApiError, logoutPlatformSession, switchPlatformContext } from "@/lib/api"
import { safeLegacyReturn, safePlatformReturn, startUnifiedLogin } from "@/lib/authentication-mode"
import BffLoginPage from "./BffLoginPage"
import type { LoginResp, Membership } from "@/api/types"
import { canSelectConsoleMembership, membershipSelectionDisabledReason, membershipSelectionKey, requireContextIdentity } from "@/lib/context-identity"

const REMEMBER_KEY = "platform-remember-username"

function rememberedUsername() {
  try { return localStorage.getItem(REMEMBER_KEY) ?? "" } catch { return "" }
}

const FEATURES = [
  { icon: Layers, title: "多租户隔离", desc: "租户、组织与应用边界由服务端统一校验" },
  { icon: ShieldCheck, title: "多层级组织权限", desc: "租户授权 ⊇ 组织授权 ⊇ 角色菜单,层层收窄、精确可控" },
  { icon: Tv, title: "多应用统一接入", desc: "统一登录，按租户和组织授权访问应用" },
]

/** 展示支持的接入方式，不把演示客户端当成已部署应用。 */
const ECO_APPS = [
  { icon: Globe, name: "独立微应用", desc: "统一上下文接入" },
  { icon: MonitorSmartphone, name: "终端设备", desc: "设备码授权" },
  { icon: Server, name: "业务 API", desc: "逐请求授权校验" },
  { icon: LayoutGrid, name: "平台控制台", desc: "租户与应用管理" },
]

const schema = z.object({
  username: z.string().min(1, "请输入登录账号"),
  password: z.string().min(1, "请输入密码"),
  remember: z.boolean(),
})
type FormData = z.infer<typeof schema>

export default function LoginPage() {
  const authMode = useAuthStore(state => state.authMode)
  if (authMode === "BFF") return <BffLoginEntry />
  if (authMode === "LEGACY") return <LegacyLoginPage />
  return <main className="grid min-h-dvh place-items-center p-6" role="status">正在确认登录方式…</main>
}

function BffLoginEntry() {
  const session = useAuthStore(state => state.bffSession)
  const token = useAuthStore(state => state.token)
  const reconcile = useAuthStore(state => state.reconcileSession)
  const navigate = useNavigate()
  const [params] = useSearchParams()
  const returnTo = safePlatformReturn(params.get("return"))

  React.useEffect(() => {
    if (token) navigate(returnTo, { replace: true })
  }, [token, returnTo, navigate])

  return <BffLoginPage session={session} returnTo={returnTo}
    onLogin={() => startUnifiedLogin(returnTo)}
    onReload={reconcile}
    onSelect={async member => {
      const expected = captureSessionFence()
      await switchPlatformContext(member.organizationId, member.membershipId, expected)
      // Only the committed store token drives the effect above. In-flight or
      // superseded selections cannot navigate from an unmounted login form.
    }}
    onLogout={async scope => {
      if (await logoutPlatformSession(captureSessionFence(), scope)) navigate("/login?signedOut=1", { replace: true })
    }} />
}

function LegacyLoginPage() {
  const { platformName, platformDescription, loginTitle, loginDescription, footerText } = usePlatformBranding()
  const navigate = useNavigate()
  const [params] = useSearchParams()
  const returnTo = safeLegacyReturn(params.get("return"))
  const setSession = useAuthStore((s) => s.setSession)
  useDocumentTitle("登录")

  const [step, setStep] = React.useState<"form" | "tenant" | "organization">("form")
  const [loginData, setLoginData] = React.useState<(LoginResp & { sessionFence: SessionFence }) | null>(null)
  const [selectedTenantId, setSelectedTenantId] = React.useState<string | null>(null)
  const [selectionSearch, setSelectionSearch] = React.useState("")
  const [error, setError] = React.useState<string | null>(null)
  const [showPassword, setShowPassword] = React.useState(false)
  const [selecting, setSelecting] = React.useState<string | null>(null)

  const {
    register,
    handleSubmit,
    setValue,
    watch,
    formState: { errors, isSubmitting },
  } = useForm<FormData>({
    resolver: zResolver(schema),
    defaultValues: {
      username: rememberedUsername(),
      password: "",
      remember: !!rememberedUsername(),
    },
  })
  const remember = watch("remember")

  const memberships = React.useMemo(() => loginData?.memberships ?? [], [loginData])
  const tenantGroups = React.useMemo(() => {
    const groups = new Map<string, { tenantId: string; tenantName: string; items: Membership[] }>()
    for (const m of memberships) {
      if (!groups.has(m.tenantId)) groups.set(m.tenantId, { tenantId: m.tenantId, tenantName: m.tenantName, items: [] })
      groups.get(m.tenantId)!.items.push(m)
    }
    return [...groups.values()]
  }, [memberships])

  const selectedTenant = tenantGroups.find((group) => group.tenantId === selectedTenantId)
  const searchTerm = selectionSearch.trim().toLocaleLowerCase()
  const filteredTenants = tenantGroups.filter((group) => group.tenantName.toLocaleLowerCase().includes(searchTerm))
  const filteredMemberships = (selectedTenant?.items ?? []).filter((membership) =>
    [membership.organizationName, membership.organizationPath, membership.departmentName,
      membership.realName, membership.username].some((value) => value?.toLocaleLowerCase().includes(searchTerm)))

  const chooseTenant = (tenantId: string) => {
    setSelectedTenantId(tenantId)
    setSelectionSearch("")
    setError(null)
    setStep("organization")
  }

  const reselectTenant = () => {
    setSelectedTenantId(null)
    setSelectionSearch("")
    setError(null)
    setStep("tenant")
  }

  const onSubmit = async (data: FormData) => {
    const sessionFence = captureSessionFence()
    setError(null)
    try {
      try {
        if (data.remember) localStorage.setItem(REMEMBER_KEY, data.username)
        else localStorage.removeItem(REMEMBER_KEY)
      } catch { /* Optional username preference never supplies session coordination. */ }
      const resp = await authApi.login(data.username, data.password)
      if (!resp.memberships || resp.memberships.length === 0) {
        setError("该账号未加入任何组织,请联系管理员")
        return
      }
      setLoginData({ ...resp, sessionFence })
      setSelectedTenantId(null)
      setSelectionSearch("")
      setStep("tenant")
    } catch (e) {
      setError(e instanceof Error ? e.message : "登录失败,请稍后重试")
    }
  }

  const selectContext = async (member: Membership, data: LoginResp & { sessionFence: SessionFence }) => {
    if (!canSelectConsoleMembership(member)) {
      setError(membershipSelectionDisabledReason(member))
      return
    }
    if (!data.ticket) {
      setError("登录票据缺失,请重新登录")
      setStep("form")
      return
    }
    setSelecting(membershipSelectionKey(member))
    setError(null)
    try {
      const ctx = await authApi.selectContext(member.organizationId, member.membershipId, data.ticket, member.userId)
      const identity = requireContextIdentity(ctx)
      if (membershipSelectionKey(identity) !== membershipSelectionKey(member)) throw new Error("登录响应与所选成员身份不一致，请重新登录")
      // The captured login intent cannot replace a login made by another page.
      // Epoch subscribers clear the prior cache only when this commit succeeds.
      const committed = await setSession(ctx.accessToken, ctx.refreshToken, data.sessionFence, identity)
      if (!committed) {
        setError("其他页面的登录已变化，请重新登录后选择组织")
        setStep("form")
        setLoginData(null)
        return
      }
      toast.success(`欢迎回来,${ctx.user.realName || ctx.user.username}`)
      navigate(returnTo, { replace: true })
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) {
        setError(`${e.message}，请重新登录`)
        setStep("form")
        setLoginData(null)
        setSelectedTenantId(null)
      } else {
        setError(e instanceof Error ? e.message : "建立会话失败，请重试")
      }
    } finally {
      setSelecting(null)
    }
  }

  return (
    <div className="flex min-h-dvh flex-col bg-background">
      <div className="flex flex-1 flex-col sm:flex-row">
        {/* 左侧品牌区 */}
        <aside className="relative hidden overflow-hidden bg-gradient-to-br from-[#0f766e] via-[#115e59] to-[#042f2e] p-10 text-white sm:flex sm:w-[46%] lg:w-[44%]">
          <div
            aria-hidden
            className="pointer-events-none absolute -right-24 -top-24 size-96 rounded-full bg-teal-300/10 blur-3xl"
          />
          <div
            aria-hidden
            className="pointer-events-none absolute -bottom-32 -left-16 size-80 rounded-full bg-teal-500/10 blur-3xl"
          />
          <div className="relative z-10 flex h-full w-full flex-col">
            <div className="flex items-center gap-3">
              <span className="flex size-11 items-center justify-center rounded-2xl bg-white/10 ring-1 ring-white/20">
                <BrandMark className="size-7 text-white" />
              </span>
              <div className="min-w-0">
                <p className="break-words text-lg font-semibold tracking-tight">{platformName}</p>
                <p className="break-words text-xs text-white/60">{platformDescription}</p>
              </div>
            </div>

            <div className="mt-auto max-w-md">
              <motion.h1
                initial={{ opacity: 0, y: 16 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ duration: 0.5, ease: "easeOut" }}
                className="text-3xl font-bold leading-snug tracking-tight"
              >
                多租户 · 多层级组织 · 多应用统一权限
              </motion.h1>
              <p className="mt-3 text-sm leading-relaxed text-white/70">
                以 Rust 为骨、React 为翼,为每一个组织提供精确到按钮的权限治理与标准 OAuth 2.0 开放能力。
              </p>

              <ul className="mt-8 space-y-4">
                {FEATURES.map((f, i) => (
                  <motion.li
                    key={f.title}
                    initial={{ opacity: 0, x: -12 }}
                    animate={{ opacity: 1, x: 0 }}
                    transition={{ delay: 0.15 + i * 0.1, duration: 0.4 }}
                    className="flex items-start gap-3"
                  >
                    <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-white/10 ring-1 ring-white/15">
                      <f.icon className="size-4.5 text-teal-200" />
                    </span>
                    <div>
                      <p className="text-sm font-medium">{f.title}</p>
                      <p className="text-xs leading-relaxed text-white/60">{f.desc}</p>
                    </div>
                  </motion.li>
                ))}
              </ul>
            </div>

            <div className="mt-10 flex items-center gap-4 text-[11px] text-white/40">
              <span>Rust · Axum</span>
              <span className="size-1 rounded-full bg-white/30" />
              <span>PostgreSQL 17</span>
              <span className="size-1 rounded-full bg-white/30" />
              <span>OAuth 2.0 / RFC 8628</span>
            </div>

            {/* 支持的接入方式；实际应用由平台管理员登记和授权。 */}
            <div className="mt-5">
              <p className="text-[10px] font-medium tracking-widest text-white/40">接入方式</p>
              <div className="mt-2 flex flex-wrap gap-1.5">
                {ECO_APPS.map((app) => (
                  <div
                    key={app.name}
                    className="flex items-center gap-2 rounded-lg border border-white/10 bg-white/[0.06] px-2 py-1.5 transition-all duration-200 hover:-translate-y-0.5 hover:border-teal-300/40 hover:bg-white/10"
                  >
                    <span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-white/10 text-teal-200 ring-1 ring-white/10">
                      <app.icon className="size-3.5" />
                    </span>
                    <span className="min-w-0">
                      <span className="block whitespace-nowrap text-xs font-medium leading-tight">
                        {app.name}
                      </span>
                      <span className="mt-0.5 block whitespace-nowrap text-[10px] leading-tight text-white/55">
                        {app.desc}
                      </span>
                    </span>
                  </div>
                ))}
              </div>
            </div>
          </div>
        </aside>

        {/* 右侧登录卡片 */}
        <main className="relative flex flex-1 items-center justify-center px-4 py-10 sm:px-8">
          <div className="absolute right-4 top-4">
            <ThemeToggleGroup />
          </div>
          <div className="w-full max-w-sm">
            {step === "form" ? (
              <motion.div
                initial={{ opacity: 0, y: 12 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ duration: 0.35 }}
              >
                <div className="mb-7 flex flex-col items-start gap-2 sm:items-center sm:text-center">
                  <span className="flex size-12 items-center justify-center rounded-2xl bg-primary/10 text-primary sm:hidden">
                    <BrandMark className="size-7" />
                  </span>
                  <h2 className="break-words text-2xl font-semibold tracking-tight">{loginTitle}</h2>
                  <p className="break-words text-sm text-muted-foreground">{loginDescription}</p>
                </div>

                {error && (
                  <Alert variant="destructive" className="mb-4">
                    <AlertCircle className="size-4" />
                    <AlertTitle>登录失败</AlertTitle>
                    <AlertDescription>{error}</AlertDescription>
                  </Alert>
                )}

                <form onSubmit={handleSubmit(onSubmit)} className="space-y-4" noValidate>
                  <div className="space-y-2">
                    <Label htmlFor="username">
                      登录账号 <span className="text-destructive">*</span>
                    </Label>
                    <div className="relative">
                      <User className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
                      <Input
                        id="username"
                        autoComplete="username"
                        placeholder="用户名 / 电话 / Email"
                        className="pl-9"
                        {...register("username")}
                      />
                    </div>
                    {errors.username && <p className="text-xs text-destructive">{errors.username.message}</p>}
                  </div>

                  <div className="space-y-2">
                    <Label htmlFor="password">
                      密码 <span className="text-destructive">*</span>
                    </Label>
                    <div className="relative">
                      <Lock className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
                      <Input
                        id="password"
                        type={showPassword ? "text" : "password"}
                        autoComplete="current-password"
                        placeholder="请输入密码"
                        className="pl-9 pr-10"
                        {...register("password")}
                      />
                      <button
                        type="button"
                        aria-label={showPassword ? "隐藏密码" : "显示密码"}
                        onClick={() => setShowPassword((v) => !v)}
                        className="absolute right-2 top-1/2 flex size-7 -translate-y-1/2 items-center justify-center rounded-md text-muted-foreground hover:text-foreground"
                      >
                        {showPassword ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
                      </button>
                    </div>
                    {errors.password && <p className="text-xs text-destructive">{errors.password.message}</p>}
                  </div>

                  <div className="flex items-center justify-between">
                    <label className="flex cursor-pointer items-center gap-2 text-sm text-muted-foreground select-none">
                      <Checkbox
                        checked={remember}
                        onCheckedChange={(v) => setValue("remember", v === true)}
                        aria-label="记住用户名"
                      />
                      记住用户名
                    </label>
                  </div>

                  <Button type="submit" className="h-10 w-full" disabled={isSubmitting}>
                    {isSubmitting && <Loader2 className="size-4 animate-spin" />}
                    登 录
                  </Button>
                </form>

                <div className="mt-6 flex items-center justify-center gap-1.5 text-sm text-muted-foreground">
                  <Tv className="size-4" />
                  <span>在电视等设备上登录?</span>
                  <Link
                    to="/device"
                    className="font-medium text-primary underline-offset-4 hover:underline"
                  >
                    设备授权入口
                  </Link>
                </div>

                {/* 安全提示条(纯信任背书, 无交互) */}
                <p className="mt-5 flex items-center justify-center gap-1.5 text-xs text-muted-foreground/80">
                  <ShieldCheck className="size-3.5 shrink-0" aria-hidden />
                  密码经 Argon2id 加密保护,操作全程审计留痕
                </p>
              </motion.div>
            ) : (
              <motion.div
                initial={{ opacity: 0, y: 12 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ duration: 0.35 }}
              >
                <div className="mb-6 flex flex-col gap-1">
                  <div className="mb-2 flex items-center justify-between gap-2">
                    <Button variant="ghost" size="sm" className="h-7 px-2" disabled={selecting !== null}
                      onClick={() => {
                        if (step === "organization") reselectTenant()
                        else { setStep("form"); setLoginData(null); setError(null); setSelectionSearch("") }
                      }}>
                      <ArrowLeft className="size-4" />
                      {step === "organization" ? "重新选择租户" : "返回登录"}
                    </Button>
                    <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
                      <span className={cn(step === "tenant" && "font-medium text-primary")}>1 租户</span>
                      <ArrowRight className="size-3" aria-hidden />
                      <span className={cn(step === "organization" && "font-medium text-primary")}>2 组织</span>
                    </span>
                  </div>
                  <h2 className="text-xl font-semibold tracking-tight">{step === "tenant" ? "选择登录租户" : "选择组织与身份"}</h2>
                  <p className="text-sm text-muted-foreground">
                    {step === "tenant" ? "账号已验证，请先选择本次登录的租户" : "请选择有效员工身份进入控制台；客户身份使用独立入口"}
                  </p>
                </div>

                {error && (
                  <Alert variant="destructive" className="mb-4">
                    <AlertCircle className="size-4" />
                    <AlertDescription>{error}</AlertDescription>
                  </Alert>
                )}

                {step === "organization" && selectedTenant && (
                  <div className="mb-4 flex items-start gap-2 rounded-lg border bg-primary/5 px-3 py-2.5">
                    <Building2 className="mt-0.5 size-4 shrink-0 text-primary" aria-hidden />
                    <div className="min-w-0">
                      <p className="break-words text-sm font-medium">{selectedTenant.tenantName}</p>
                      <p className="mt-0.5 text-xs text-muted-foreground">
                        {selectedTenant.items.filter(canSelectConsoleMembership).length} 个可选员工身份
                      </p>
                    </div>
                  </div>
                )}

                <div className="relative mb-3">
                  <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden />
                  <Input value={selectionSearch} onChange={(event) => setSelectionSearch(event.target.value)}
                    aria-label={step === "tenant" ? "搜索租户" : "搜索组织或登录身份"}
                    placeholder={step === "tenant" ? "搜索租户名称" : "搜索组织、部门或登录身份"}
                    className="pl-9" disabled={selecting !== null} />
                </div>

                <div className="mb-2 text-xs text-muted-foreground" aria-live="polite">
                  {step === "tenant" ? `${filteredTenants.length} 个租户` : `${filteredMemberships.length} 个成员身份`}
                </div>
                <div className="max-h-[44vh] space-y-2 overflow-y-auto scroll-area pr-1">
                  {step === "tenant" ? filteredTenants.map((group) => (
                    <button key={group.tenantId} type="button" onClick={() => chooseTenant(group.tenantId)}
                      className="group flex w-full items-center gap-3 rounded-xl border bg-card p-3.5 text-left outline-none transition-all hover:border-primary/40 hover:shadow-sm focus-visible:ring-2 focus-visible:ring-ring">
                      <span className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
                        <Building2 className="size-4 text-primary" />
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="block break-words text-sm font-medium">{group.tenantName}</span>
                        <span className="mt-0.5 block text-xs text-muted-foreground">
                          {new Set(group.items.map((item) => item.organizationId)).size} 个组织
                        </span>
                      </span>
                      <ArrowRight className="size-4 shrink-0 text-muted-foreground/50 transition-colors group-hover:text-primary" aria-hidden />
                    </button>
                  )) : filteredMemberships.map((m) => (
                    <button
                      key={membershipSelectionKey(m)}
                      type="button"
                      disabled={selecting !== null || !canSelectConsoleMembership(m)}
                      onClick={() => loginData && selectContext(m, loginData)}
                      className="group flex w-full items-center gap-3 rounded-xl border bg-card p-3.5 text-left outline-none transition-all hover:border-primary/40 hover:shadow-sm focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
                    >
                      <span className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
                        <Building2 className="size-5" />
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="flex flex-wrap items-center gap-1.5">
                          <span className="text-sm font-medium">{m.organizationName}</span>
                          <span className="rounded border bg-muted px-1.5 py-px text-[10px] text-muted-foreground">
                            {m.memberType === "EMPLOYEE" ? "员工" : "客户"} · {m.status === "ENABLED" ? "有效" : "已停用"}
                          </span>
                          {m.memberType === "EMPLOYEE" && m.departmentName && (
                            <span className="rounded border bg-muted px-1.5 py-px text-[10px] text-muted-foreground">
                              {m.departmentName}
                            </span>
                          )}
                        </span>
                        <span className="mt-0.5 block truncate text-xs text-muted-foreground">
                          {m.organizationPath}
                        </span>
                        {(loginData?.accountCount ?? 1) > 1 && <span className="mt-0.5 block text-xs text-muted-foreground">
                          登录身份：{m.realName || m.username}（{m.username}）
                        </span>}
                        {membershipSelectionDisabledReason(m) && <span className="mt-1 block text-xs text-muted-foreground">
                          {membershipSelectionDisabledReason(m)}
                        </span>}
                      </span>
                      {selecting === membershipSelectionKey(m) ? (
                        <Loader2 className="size-4 shrink-0 animate-spin text-primary" />
                      ) : (
                        <CheckCircle2 className="size-4 shrink-0 text-muted-foreground/30 transition-colors group-hover:text-primary" />
                      )}
                    </button>
                  ))}
                  {(step === "tenant" ? filteredTenants : filteredMemberships).length === 0 && (
                    <p className="rounded-xl border border-dashed px-4 py-8 text-center text-sm text-muted-foreground">
                      {step === "tenant" ? "未找到匹配的租户" : "未找到匹配的组织或登录身份"}
                    </p>
                  )}
                </div>
              </motion.div>
            )}
          </div>
        </main>
      </div>

      {/* 页脚 */}
      {footerText && <footer className="sticky bottom-0 break-words border-t bg-background/95 px-4 py-3 text-center text-xs text-muted-foreground backdrop-blur">
        {footerText}
      </footer>}
    </div>
  )
}
