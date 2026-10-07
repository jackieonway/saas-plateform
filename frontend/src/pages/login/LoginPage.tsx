import * as React from "react"
import { Link, useNavigate, useSearchParams } from "react-router-dom"
import { useQueryClient } from "@tanstack/react-query"
import { useForm } from "react-hook-form"
import { z } from "zod"
import { zResolver } from "@/lib/form"
import { motion } from "framer-motion"
import {
  AlertCircle,
  ArrowLeft,
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
import { useAuthStore } from "@/stores/auth"
import { useDocumentTitle } from "@/hooks/use-document-title"
import { toast } from "sonner"
import { cn } from "@/lib/utils"
import type { LoginResp, Membership } from "@/api/types"

const REMEMBER_KEY = "dante-remember-username"

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
  username: z.string().min(1, "请输入用户名"),
  password: z.string().min(1, "请输入密码"),
  remember: z.boolean(),
})
type FormData = z.infer<typeof schema>

export default function LoginPage() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [params] = useSearchParams()
  const returnTo = params.get("return") || "/"
  const setToken = useAuthStore((s) => s.setToken)
  useDocumentTitle("登录")

  const [step, setStep] = React.useState<"form" | "select">("form")
  const [loginData, setLoginData] = React.useState<LoginResp | null>(null)
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
      username: localStorage.getItem(REMEMBER_KEY) ?? "",
      password: "",
      remember: !!localStorage.getItem(REMEMBER_KEY),
    },
  })
  const remember = watch("remember")

  // 单租户单组织自动跳过选择
  const memberships = loginData?.memberships ?? []
  const tenantGroups = React.useMemo(() => {
    const groups = new Map<string, { tenantId: string; tenantName: string; items: Membership[] }>()
    for (const m of memberships) {
      if (!groups.has(m.tenantId)) groups.set(m.tenantId, { tenantId: m.tenantId, tenantName: m.tenantName, items: [] })
      groups.get(m.tenantId)!.items.push(m)
    }
    return [...groups.values()]
  }, [memberships])

  React.useEffect(() => {
    if (step === "select" && loginData && tenantGroups.length === 1 && tenantGroups[0].items.length === 1) {
      void selectContext(tenantGroups[0].items[0].organizationId, loginData)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step, loginData])

  const onSubmit = async (data: FormData) => {
    setError(null)
    try {
      if (data.remember) localStorage.setItem(REMEMBER_KEY, data.username)
      else localStorage.removeItem(REMEMBER_KEY)
      const resp = await authApi.login(data.username, data.password)
      if (!resp.memberships || resp.memberships.length === 0) {
        setError("该账号未加入任何组织,请联系管理员")
        return
      }
      setLoginData(resp)
      setStep("select")
    } catch (e) {
      setError(e instanceof Error ? e.message : "登录失败,请稍后重试")
    }
  }

  const selectContext = async (organizationId: string, data: LoginResp) => {
    if (!data.ticket) {
      setError("登录票据缺失,请重新登录")
      setStep("form")
      return
    }
    setSelecting(organizationId)
    setError(null)
    try {
      const ctx = await authApi.selectContext(organizationId, data.ticket)
      // 清空上一会话的所有缓存,避免残留他人数据
      queryClient.clear()
      setToken(ctx.accessToken)
      toast.success(`欢迎回来,${ctx.user.realName || ctx.user.username}`)
      navigate(returnTo, { replace: true })
    } catch (e) {
      setError(e instanceof Error ? `${e.message}(请重新登录)` : "建立会话失败")
      setStep("form")
      setLoginData(null)
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
              <div>
                <p className="text-lg font-semibold tracking-tight">Dante Cloud</p>
                <p className="text-xs text-white/60">多租户权限平台</p>
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
                  <h2 className="text-2xl font-semibold tracking-tight">欢迎回来</h2>
                  <p className="text-sm text-muted-foreground">登录 Dante Cloud,管理您的租户与应用权限</p>
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
                      用户名 <span className="text-destructive">*</span>
                    </Label>
                    <div className="relative">
                      <User className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
                      <Input
                        id="username"
                        autoComplete="username"
                        placeholder="请输入用户名"
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
                  <div className="flex items-center gap-2">
                    <Button variant="ghost" size="sm" className="h-7 px-2" onClick={() => { setStep("form"); setLoginData(null); setError(null) }}>
                      <ArrowLeft className="size-4" />
                      返回
                    </Button>
                  </div>
                  <h2 className="text-xl font-semibold tracking-tight">选择登录组织</h2>
                  <p className="text-sm text-muted-foreground">
                    欢迎回来,{loginData ? "" : ""}您属于多个组织,请选择本次会话的组织上下文
                  </p>
                </div>

                {error && (
                  <Alert variant="destructive" className="mb-4">
                    <AlertCircle className="size-4" />
                    <AlertDescription>{error}</AlertDescription>
                  </Alert>
                )}

                <div className="max-h-[52vh] space-y-5 overflow-y-auto scroll-area pr-1">
                  {tenantGroups.map((g) => (
                    <section key={g.tenantId}>
                      <div className="mb-2 flex items-center gap-2">
                        <Building2 className="size-4 text-primary" />
                        <h3 className="text-sm font-semibold">{g.tenantName}</h3>
                        <span className="text-xs text-muted-foreground">({g.items.length} 个组织)</span>
                      </div>
                      <div className="space-y-2">
                        {g.items.map((m) => (
                          <button
                            key={m.organizationId}
                            type="button"
                            disabled={selecting !== null}
                            onClick={() => selectContext(m.organizationId, loginData!)}
                            className={cn(
                              "group flex w-full items-center gap-3 rounded-xl border bg-card p-3.5 text-left outline-none transition-all hover:border-primary/40 hover:shadow-sm focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
                            )}
                          >
                            <span className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
                              <Building2 className="size-5" />
                            </span>
                            <span className="min-w-0 flex-1">
                              <span className="flex flex-wrap items-center gap-1.5">
                                <span className="text-sm font-medium">{m.organizationName}</span>
                                {m.departmentName && (
                                  <span className="rounded border bg-muted px-1.5 py-px text-[10px] text-muted-foreground">
                                    {m.departmentName}
                                  </span>
                                )}
                              </span>
                              <span className="mt-0.5 block truncate text-xs text-muted-foreground">
                                {m.organizationPath}
                              </span>
                            </span>
                            {selecting === m.organizationId ? (
                              <Loader2 className="size-4 shrink-0 animate-spin text-primary" />
                            ) : (
                              <CheckCircle2 className="size-4 shrink-0 text-muted-foreground/30 transition-colors group-hover:text-primary" />
                            )}
                          </button>
                        ))}
                      </div>
                    </section>
                  ))}
                </div>
              </motion.div>
            )}
          </div>
        </main>
      </div>

      {/* 页脚 */}
      <footer className="sticky bottom-0 border-t bg-background/95 px-4 py-3 text-center text-xs text-muted-foreground backdrop-blur">
        © 2026 Dante Cloud · Powered by Rust &amp; React
      </footer>
    </div>
  )
}
