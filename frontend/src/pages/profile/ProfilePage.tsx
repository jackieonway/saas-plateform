import * as React from "react"
import { useNavigate } from "react-router-dom"
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useForm } from "react-hook-form"
import { z } from "zod"
import { zResolver } from "@/lib/form"
import {
  BadgeCheck,
  Bell,
  Building2,
  CheckCircle2,
  ChevronDown,
  Clock,
  KeyRound,
  Landmark,
  Loader2,
  Mail,
  MonitorSmartphone,
  Phone,
  ShieldCheck,
  UserCog,
} from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form"
import { Skeleton } from "@/components/ui/skeleton"
import { PageHeader } from "@/components/layout/PageHeader"
import { EmptyState, ErrorState } from "@/components/states"
import { useAuthStore } from "@/stores/auth"
import { usePermissionsQuery } from "@/hooks/use-auth-data"
import { authApi } from "@/api/auth"
import { formatDateTime, formatRelative, GENDER_LABELS } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { LoginHistoryRecord, MySessionRecord } from "@/api/types"

const passwordSchema = z
  .object({
    oldPassword: z.string().min(1, "请输入当前密码"),
    newPassword: z
      .string()
      .min(8, "新密码至少 8 位")
      .regex(/[A-Za-z]/, "需包含字母")
      .regex(/\d/, "需包含数字"),
    confirmPassword: z.string().min(1, "请再次输入新密码"),
  })
  .refine((v) => v.newPassword === v.confirmPassword, {
    message: "两次输入的新密码不一致",
    path: ["confirmPassword"],
  })
type PasswordForm = z.infer<typeof passwordSchema>

/* 登录历史分页大小(「加载更多」按页追加) */
const LOGIN_HISTORY_PAGE_SIZE = 10

export default function ProfilePage() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const logout = useAuthStore((s) => s.logout)
  const profile = useAuthStore((s) => s.profile)
  const user = profile?.user
  const displayName = user?.realName || user?.username || "用户"

  const permissionsQuery = usePermissionsQuery()

  /* ---------- 修改密码 ---------- */
  const form = useForm<PasswordForm>({
    resolver: zResolver(passwordSchema),
    defaultValues: { oldPassword: "", newPassword: "", confirmPassword: "" },
  })

  const changeMutation = useMutation({
    mutationFn: (v: PasswordForm) => authApi.changePassword(v.oldPassword, v.newPassword),
    onSuccess: () => {
      toast.success("密码修改成功，请重新登录")
      form.reset({ oldPassword: "", newPassword: "", confirmPassword: "" })
      logout()
      queryClient.clear()
      navigate("/login", { replace: true })
    },
    onError: (e: Error) => toast.error(e.message || "修改失败"),
  })

  /* ---------- 安全中心 · 登录历史(时间线, 无限加载) ---------- */
  const historyQuery = useInfiniteQuery({
    queryKey: ["auth-login-history", user?.id],
    queryFn: ({ pageParam }) => authApi.loginHistory({ page: pageParam, size: LOGIN_HISTORY_PAGE_SIZE }),
    initialPageParam: 1,
    getNextPageParam: (last, all) => {
      const loaded = all.reduce((n, p) => n + p.records.length, 0)
      return loaded < last.total ? all.length + 1 : undefined
    },
    enabled: !!user,
    staleTime: 30_000,
    retry: 1,
  })
  const historyItems: LoginHistoryRecord[] = historyQuery.data?.pages.flatMap((p) => p.records) ?? []
  const historyTotal = historyQuery.data?.pages[0]?.total ?? 0
  const historyLoaded = historyItems.length

  /* ---------- 安全中心 · 我的活跃会话 ---------- */
  const sessionsQuery = useQuery({
    queryKey: ["auth-my-sessions", user?.id],
    queryFn: () => authApi.mySessions(),
    enabled: !!user,
    staleTime: 30_000,
    retry: 1,
  })
  const mySessions = sessionsQuery.data?.sessions ?? []
  const platformSessions = sessionsQuery.data?.platformSessions ?? []
  const revokeSessionMutation = useMutation({
    mutationFn: authApi.revokeSession,
    onSuccess: () => {
      toast.success("设备会话已撤销")
      void sessionsQuery.refetch()
    },
    onError: (e: Error) => toast.error(e.message || "撤销失败"),
  })
  const activeSessionCount = mySessions.filter((s) => s.activeAccess + s.activeRefresh > 0).length
  const revokedTotal = sessionsQuery.data?.revokedTotal ?? 0

  /* ---------- 安全操作:锚点滚动到改密卡片 + 一次性 ring 脉冲 ---------- */
  const [pwPulse, setPwPulse] = React.useState(false)
  const pwTimer = React.useRef<number | undefined>(undefined)
  React.useEffect(() => () => window.clearTimeout(pwTimer.current), [])
  const gotoPassword = () => {
    document.getElementById("password")?.scrollIntoView({ behavior: "smooth", block: "center" })
    // 重置后再触发,保证连续点击可重新播放脉冲动画
    window.clearTimeout(pwTimer.current)
    setPwPulse(false)
    window.requestAnimationFrame(() => setPwPulse(true))
    pwTimer.current = window.setTimeout(() => setPwPulse(false), 2_200)
  }

  const roles = permissionsQuery.data?.roles ?? []

  return (
    <div>
      <PageHeader
        title="个人中心"
        description="账号信息与安全中心:登录历史、活跃会话与密码管理"
        icon={<UserCog className="size-5" />}
      />

      <div className="grid gap-4 lg:grid-cols-3">
        {/* 顶卡:账号信息 */}
        <Card className="lg:col-span-3">
          <CardContent className="p-6">
            <div className="flex flex-col gap-5 sm:flex-row sm:items-center">
              <div className="relative shrink-0">
                <div className="flex size-20 items-center justify-center rounded-full bg-gradient-to-br from-primary/85 to-primary/55 text-2xl font-bold text-primary-foreground shadow-lg shadow-primary/20">
                  {displayName.slice(0, 1).toUpperCase()}
                </div>
                {user?.isSuperAdmin && (
                  <span
                    className="absolute -bottom-1 -right-1 flex size-7 items-center justify-center rounded-full border-2 border-card bg-warning/15 text-warning"
                    title="超级管理员"
                    aria-label="超级管理员"
                  >
                    <ShieldCheck className="size-4" />
                  </span>
                )}
              </div>

              <div className="min-w-0 flex-1 space-y-2.5">
                <div className="flex flex-wrap items-center gap-2">
                  <h2 className="text-xl font-semibold tracking-tight">{displayName}</h2>
                  <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-muted-foreground">
                    @{user?.username}
                  </code>
                  {user?.isSuperAdmin && (
                    <Badge className="border-warning/40 bg-warning/10 text-warning">超级管理员</Badge>
                  )}
                </div>

                <div className="flex flex-wrap items-center gap-2">
                  <Badge variant="secondary" className="gap-1.5">
                    <Landmark className="size-3" />
                    {profile?.tenant.name ?? "—"}
                  </Badge>
                  <Badge variant="secondary" className="gap-1.5">
                    <Building2 className="size-3" />
                    {profile?.organization.name ?? "—"}
                  </Badge>
                  {roles.length > 0 ? (
                    roles.map((r) => (
                      <Badge key={r.id} variant="outline" className="gap-1.5 border-primary/30 text-primary">
                        <BadgeCheck className="size-3" />
                        {r.name}
                      </Badge>
                    ))
                  ) : (
                    <span className="text-xs text-muted-foreground">未分配角色</span>
                  )}
                </div>

                <div className="flex flex-wrap gap-x-5 gap-y-1.5 text-sm text-muted-foreground">
                  {user?.email && (
                    <span className="flex items-center gap-1.5">
                      <Mail className="size-3.5" />
                      {user.email}
                    </span>
                  )}
                  {user?.phone && (
                    <span className="flex items-center gap-1.5">
                      <Phone className="size-3.5" />
                      {user.phone}
                    </span>
                  )}
                  <span className="flex items-center gap-1.5">
                    <UserCog className="size-3.5" />
                    {GENDER_LABELS[user?.gender ?? "UNKNOWN"] ?? "未知"}
                  </span>
                  <span className="hidden items-center gap-1.5 sm:flex">
                    <Building2 className="size-3.5" />
                    组织路径:{profile?.organization.path ?? "—"}
                  </span>
                </div>
              </div>
            </div>
          </CardContent>
        </Card>

        {/* 安全中心:登录历史(左,占大) + 活跃会话/安全操作(右) */}
        <Card className="lg:col-span-3">
          <CardHeader className="pb-4">
            <CardTitle className="flex items-center gap-2 text-base">
              <ShieldCheck className="size-4 text-primary" />
              安全中心
            </CardTitle>
            <CardDescription>登录历史与活跃的第三方会话,帮助您随时掌握账号安全动态</CardDescription>
          </CardHeader>
          <CardContent>
            <div className="grid gap-8 lg:grid-cols-5 lg:gap-0">
              {/* 左栏(3/5):登录历史时间线 */}
              <section className="lg:col-span-3 lg:border-r lg:pr-7" aria-label="登录历史">
                <div className="mb-3 flex items-center gap-2">
                  <Clock className="size-4 text-primary/80" />
                  <h3 className="text-[13px] font-semibold text-foreground/90">登录历史</h3>
                  {!historyQuery.isLoading && !historyQuery.isError && (
                    <span className="ml-auto text-[11px] tabular-nums text-muted-foreground/70">
                      共 {historyTotal} 条
                    </span>
                  )}
                </div>

                {historyQuery.isLoading ? (
                  <div className="space-y-5">
                    {Array.from({ length: 4 }).map((_, i) => (
                      <div key={i} className="flex items-start gap-[13px]">
                        <Skeleton className="size-[15px] shrink-0 rounded-full" />
                        <div className="flex-1 space-y-1.5 pt-0.5">
                          <Skeleton className="h-3.5 w-2/5" />
                          <Skeleton className="h-3 w-4/5" />
                        </div>
                      </div>
                    ))}
                  </div>
                ) : historyQuery.isError ? (
                  <ErrorState
                    className="border-0 bg-transparent py-6"
                    message={historyQuery.error instanceof Error ? historyQuery.error.message : undefined}
                    onRetry={() => historyQuery.refetch()}
                  />
                ) : historyItems.length === 0 ? (
                  <EmptyState
                    className="py-6"
                    icon={<Clock className="size-7" />}
                    title="暂无登录记录"
                    description="登录账号后将在此展示成功与失败的登录轨迹"
                  />
                ) : (
                  <div>
                    <div className="max-h-[28rem] overflow-y-auto scroll-area pr-1">
                    <ol className="relative">
                      {/* 时间线贯穿竖线 */}
                      <span
                        aria-hidden
                        className="absolute bottom-2 left-[7px] top-2 w-px bg-gradient-to-b from-border via-border to-transparent"
                      />
                      {historyItems.map((h) => (
                        <LoginTimelineItem key={h.id} record={h} />
                      ))}
                    </ol>
                    </div>

                    {/* 加载更多 / 全量提示 */}
                    <div className="mt-1 pl-7">
                      {historyQuery.hasNextPage ? (
                        <Button
                          variant="outline"
                          size="sm"
                          className="h-8 w-full gap-1.5 text-xs"
                          onClick={() => void historyQuery.fetchNextPage()}
                          disabled={historyQuery.isFetchingNextPage}
                        >
                          {historyQuery.isFetchingNextPage ? (
                            <Loader2 className="size-3.5 animate-spin" />
                          ) : (
                            <ChevronDown className="size-3.5" />
                          )}
                          加载更多 · 剩余 {historyTotal - historyLoaded} 条
                        </Button>
                      ) : (
                        <Button
                          variant="outline"
                          size="sm"
                          disabled
                          className="h-8 w-full cursor-default gap-1.5 text-xs text-muted-foreground/70"
                        >
                          <CheckCircle2 className="size-3.5" />
                          已全部加载 {historyTotal} 条
                        </Button>
                      )}
                    </div>
                  </div>
                )}
              </section>

              {/* 右栏(2/5):我的活跃会话 + 安全操作 */}
              <div className="space-y-7 lg:col-span-2 lg:pl-7">
                <section aria-label="我的活跃会话">
                  <div className="mb-3 flex items-center gap-2">
                    <MonitorSmartphone className="size-4 text-primary/80" />
                    <h3 className="text-[13px] font-semibold text-foreground/90">我的活跃会话</h3>
                    {!sessionsQuery.isLoading && !sessionsQuery.isError && (mySessions.length > 0 || platformSessions.length > 0) && (
                      <span className="ml-auto text-[11px] tabular-nums text-muted-foreground/70">
                        活跃 {activeSessionCount + platformSessions.filter((s) => s.active).length}/{mySessions.length + platformSessions.length}
                      </span>
                    )}
                  </div>

                  {!sessionsQuery.isLoading && !sessionsQuery.isError && platformSessions.length > 0 && (
                    <div className="mb-4 space-y-2">
                      <p className="text-[11px] font-medium text-muted-foreground">平台登录设备</p>
                      {platformSessions.map((s) => (
                        <div key={s.id} className="flex items-center gap-2 rounded-lg border bg-background/70 p-3 text-xs">
                          <MonitorSmartphone className="size-4 shrink-0 text-primary" />
                          <div className="min-w-0 flex-1">
                            <p className="truncate font-medium">{s.tenantName} / {s.organizationName}</p>
                            <p className="truncate text-[11px] text-muted-foreground" title={s.userAgent}>
                              {s.ip || "未知 IP"} · {formatDateTime(s.issuedAt)}
                            </p>
                          </div>
                          {s.isCurrent ? <Badge variant="outline">当前设备</Badge> : s.active ? (
                            <Button
                              variant="outline"
                              size="sm"
                              disabled={revokeSessionMutation.isPending}
                              onClick={() => revokeSessionMutation.mutate(s.id)}
                            >
                              撤销
                            </Button>
                          ) : <Badge variant="outline">已失效</Badge>}
                        </div>
                      ))}
                    </div>
                  )}

                  {sessionsQuery.isLoading ? (
                    <div className="space-y-2.5">
                      {Array.from({ length: 2 }).map((_, i) => (
                        <Skeleton key={i} className="h-[4.25rem] w-full rounded-lg" />
                      ))}
                    </div>
                  ) : sessionsQuery.isError ? (
                    <ErrorState
                      className="border-0 bg-transparent py-6"
                      message={sessionsQuery.error instanceof Error ? sessionsQuery.error.message : undefined}
                      onRetry={() => sessionsQuery.refetch()}
                    />
                  ) : mySessions.length === 0 ? (
                    <div>
                      <EmptyState
                        className="py-5"
                        icon={<MonitorSmartphone className="size-7" />}
                        title="当前没有第三方会话"
                        description="OAuth 客户端获得授权后,将在此显示其活跃令牌"
                      />
                      {revokedTotal > 0 && (
                        <p className="text-center text-[11px] text-muted-foreground/70">
                          已吊销 <b className="tabular-nums">{revokedTotal}</b> 个历史令牌
                        </p>
                      )}
                    </div>
                  ) : (
                    <div className="max-h-[24rem] space-y-2.5 overflow-y-auto scroll-area pr-1">
                      {mySessions.map((s) => (
                        <MySessionCard key={s.clientId} session={s} />
                      ))}
                      {revokedTotal > 0 && (
                        <p className="pt-0.5 text-[11px] text-muted-foreground/70">
                          已吊销 <b className="tabular-nums">{revokedTotal}</b> 个历史令牌
                        </p>
                      )}
                    </div>
                  )}
                </section>

                <section aria-label="安全操作">
                  <div className="mb-3 flex items-center gap-2">
                    <ShieldCheck className="size-4 text-primary/80" />
                    <h3 className="text-[13px] font-semibold text-foreground/90">安全操作</h3>
                  </div>
                  <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-1 xl:grid-cols-2">
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-9 justify-start gap-2"
                      onClick={gotoPassword}
                    >
                      <KeyRound className="size-4 text-primary" />
                      修改密码
                    </Button>
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-9 justify-start gap-2"
                      onClick={() => navigate("/system/notifications")}
                    >
                      <Bell className="size-4 text-primary" />
                      通知中心偏好
                    </Button>
                  </div>
                  <p className="mt-2.5 text-[11px] leading-relaxed text-muted-foreground/70">
                    修改密码后下次登录需使用新密码;免打扰时段与类型静音可在通知中心设置。
                  </p>
                </section>
              </div>
            </div>
          </CardContent>
        </Card>

        {/* 修改密码(安全操作入口锚点定位至此,触发一次性 ring 脉冲) */}
        <Card className={cn("lg:col-span-2", pwPulse && "pw-card-pulse")} id="password">
          <CardHeader className="pb-3">
            <CardTitle className="flex items-center gap-2 text-base">
              <KeyRound className="size-4 text-primary" />
              修改密码
            </CardTitle>
            <CardDescription>新密码需至少 8 位,且同时包含字母与数字</CardDescription>
          </CardHeader>
          <CardContent>
            <Form {...form}>
              <form
                onSubmit={form.handleSubmit((v) => changeMutation.mutate(v))}
                className="max-w-md space-y-4"
                noValidate
              >
                <FormField
                  control={form.control}
                  name="oldPassword"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        当前密码 <span className="text-destructive">*</span>
                      </FormLabel>
                      <FormControl>
                        <Input type="password" autoComplete="current-password" placeholder="请输入当前密码" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="newPassword"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        新密码 <span className="text-destructive">*</span>
                      </FormLabel>
                      <FormControl>
                        <Input type="password" autoComplete="new-password" placeholder="至少 8 位,含字母与数字" {...field} />
                      </FormControl>
                      <FormDescription>建议混合大小写字母、数字与符号</FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="confirmPassword"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        确认新密码 <span className="text-destructive">*</span>
                      </FormLabel>
                      <FormControl>
                        <Input type="password" autoComplete="new-password" placeholder="再次输入新密码" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <Button type="submit" disabled={changeMutation.isPending}>
                  {changeMutation.isPending && <CheckCircle2 className="size-4 animate-pulse" />}
                  确认修改
                </Button>
              </form>
            </Form>
          </CardContent>
        </Card>
      </div>
    </div>
  )
}

/* ============ 登录历史时间线项 ============ */

function LoginTimelineItem({ record }: { record: LoginHistoryRecord }) {
  const ok = record.action !== "LOGIN_FAILED"
  const context = [record.tenantName, record.organizationName].filter(Boolean).join(" · ")
  return (
    <li className="relative pb-4 pl-7 last:pb-0">
      {/* 节点圆点:成功 emerald 实心 / 失败 red 空心 */}
      <span
        aria-hidden
        className={cn(
          "absolute left-0 top-0.5 flex size-[15px] items-center justify-center rounded-full",
          ok
            ? "bg-emerald-500 shadow-[0_0_0_3px_color-mix(in_oklch,oklch(0.7_0.15_162)_16%,transparent)]"
            : "border-2 border-red-500 bg-card"
        )}
      />
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <span className="text-xs font-medium text-foreground/85" title={formatDateTime(record.createdAt)}>
          {formatRelative(record.createdAt)}
        </span>
        <span
          className={cn(
            "inline-flex items-center gap-1 rounded border px-1.5 py-px text-[10px] font-medium",
            ok
              ? "border-emerald-500/25 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
              : "border-red-500/25 bg-red-500/10 text-red-600 dark:text-red-400"
          )}
        >
          {ok ? "成功" : "失败"}
        </span>
        <code className="font-mono text-[11px] text-muted-foreground" title={record.ip ?? undefined}>
          {record.ip || "未知 IP"}
        </code>
      </div>
      {context && <p className="mt-1 text-[11px] text-muted-foreground/80">{context}</p>}
      {record.detail && (
        <p className="mt-0.5 truncate text-[11px] text-muted-foreground/60" title={record.detail}>
          {record.detail}
        </p>
      )}
    </li>
  )
}

/* ============ 活跃会话迷你卡 ============ */

function MySessionCard({ session }: { session: MySessionRecord }) {
  /* active=0:整卡 muted(虚线边框+降不透明度), 计数徽章转灰并附「已失效」小徽章 */
  const active = session.activeAccess + session.activeRefresh > 0
  const countBadge = "border-muted-foreground/20 bg-muted/60 text-[10px] text-muted-foreground"
  return (
    <div
      className={cn(
        "rounded-lg border p-3 shadow-sm transition-all",
        active
          ? "border-solid bg-background/70 hover:shadow-md"
          : "border-dashed border-muted-foreground/25 bg-muted/25 opacity-70 hover:opacity-95"
      )}
    >
      <div className="flex flex-wrap items-center justify-between gap-x-2 gap-y-1.5">
        <code
          className={cn(
            "rounded-md px-1.5 py-0.5 font-mono text-xs font-medium",
            active ? "bg-muted text-foreground/90" : "bg-muted/70 text-muted-foreground"
          )}
        >
          {session.clientId}
        </code>
        <span className="flex items-center gap-1.5">
          {active ? (
            <>
              <Badge
                variant="outline"
                className="border-teal-500/30 bg-teal-500/10 text-[10px] text-teal-700 dark:text-teal-300"
              >
                访问 {session.activeAccess}
              </Badge>
              <Badge
                variant="outline"
                className="border-emerald-500/30 bg-emerald-500/10 text-[10px] text-emerald-600 dark:text-emerald-400"
              >
                刷新 {session.activeRefresh}
              </Badge>
            </>
          ) : (
            <>
              <Badge variant="outline" className={countBadge}>
                访问 {session.activeAccess}
              </Badge>
              <Badge variant="outline" className={countBadge}>
                刷新 {session.activeRefresh}
              </Badge>
              <Badge
                variant="outline"
                className="border-muted-foreground/30 bg-muted/50 text-[10px] text-muted-foreground"
              >
                已失效
              </Badge>
            </>
          )}
        </span>
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11px] text-muted-foreground">
        {active ? (
          session.lastExpiresAt ? (
            <span title={formatDateTime(session.lastExpiresAt)}>
              过期 {formatRelative(session.lastExpiresAt)}
            </span>
          ) : (
            <span>暂无过期时间</span>
          )
        ) : (
          <span>令牌已全部失效</span>
        )}
        {session.scope && (
          <span
            className="max-w-full truncate font-mono text-muted-foreground/60"
            title={session.scope}
          >
            {session.scope}
          </span>
        )}
      </div>
    </div>
  )
}
