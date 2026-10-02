/**
 * 报表中心 — 第一方 ESM 通道子应用 (entry_type='ESM', entry_url='/src/micro-apps/report-center.tsx')
 *
 * ESM 通道特性(与 HTML 通道的本质差异):
 *  - 无沙箱: 第一方信任档, 由容器动态 import() 直挂
 *  - 共享运行时: 与主应用同一 React 实例 / shadcn 组件 / sonner / Tailwind 主题变量
 *    (Vite dev server 模块 URL 去重 → 浏览器模块表同一份)
 *  - 主题级联: html.dark class 与 CSS 变量天然级联到子应用 DOM, 无需任何桥接;
 *    props.theme 变化经 lifecycle.update 增量推送, React 树同步重渲染
 *  - 实现完整生命周期契约: mount / unmount / update(增量更新, 计时器不重置为「未重挂」证据)
 */

import * as React from "react"
import { createRoot, type Root } from "react-dom/client"
import {
  Activity,
  ArrowUpRight,
  BarChart3,
  Boxes,
  Layers,
  Moon,
  Palette,
  Share2,
  Sun,
  Zap,
} from "lucide-react"
import { toast } from "sonner"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { cn } from "@/lib/utils"
import type {
  MicroLifecycle,
  MicroProps,
  MicroNotifyVariant,
} from "@/components/micro/types"

/* ============================ 模块级挂载状态 ============================ */

let root: Root | null = null
let containerEl: HTMLElement | null = null
let latestProps: MicroProps | null = null
let rolesRequest: AbortController | null = null
let teardownTask: Promise<void> | null = null
/** update 推送计数(「未重挂」证据之一) */
let updateCount = 0
let lastUpdateAt = ""
/** 强制重渲染监听(props 模块变量 → React 状态桥) */
let bumpListeners: Array<() => void> = []

function notifyUpdate() {
  for (const fn of bumpListeners) fn()
}

function pad2(n: number) {
  return (n < 10 ? "0" : "") + n
}

function authorizationKey(props: MicroProps) {
  return JSON.stringify([
    props.contextEpoch, props.contextVersion, props.token,
    props.user.id, props.tenant.id, props.organization.id,
  ])
}

function isCurrentContext(props: MicroProps) {
  return Boolean(
    latestProps && !props.signal.aborted &&
    authorizationKey(latestProps) === authorizationKey(props) &&
    Date.parse(props.expiresAt) > Date.now() &&
    Date.parse(props.tokenExpiresAt) > Date.now(),
  )
}

function cancelRolesRequest() {
  rolesRequest?.abort()
  rolesRequest = null
}

/* ============================ 对外 API 类型 ============================ */

interface ExternalRoleOrg {
  id: string
  name: string
  tenantName: string
  roles: MicroProps["roles"]
}

interface ExternalRolesResp {
  application?: { code: string; name: string }
  organizations?: ExternalRoleOrg[]
}

interface RolesState {
  contextKey: string
  loading: boolean
  fired: boolean
  status: string
  ok: boolean
  message: string
  data: ExternalRolesResp | null
}

function emptyRolesState(contextKey: string): RolesState {
  return { contextKey, loading: false, fired: false, status: "", ok: false, message: "", data: null }
}

/* ============================ 子应用根组件 ============================ */

function ReportApp() {
  const props = latestProps
  const [, force] = React.useReducer((x: number) => x + 1, 0)
  const [seconds, setSeconds] = React.useState(0)
  const [clicks, setClicks] = React.useState(0)
  const [rolesState, setRolesState] = React.useState<RolesState>(() => emptyRolesState(""))
  const contextKey = props ? authorizationKey(props) : ""

  // props 模块变量 → React 状态桥: update() 时强制重渲染
  React.useEffect(() => {
    bumpListeners.push(force)
    return () => {
      cancelRolesRequest()
      bumpListeners = bumpListeners.filter((fn) => fn !== force)
    }
  }, [force])

  // 挂载时钟(不重挂证据: update 推送不重置)
  React.useEffect(() => {
    const id = window.setInterval(() => setSeconds((s) => s + 1), 1000)
    return () => window.clearInterval(id)
  }, [])

  // 同一 React 根保留演示计时器；授权上下文或令牌替换时清除旧业务结果。
  React.useEffect(() => {
    setRolesState(emptyRolesState(contextKey))
  }, [contextKey])

  if (!props) return null

  const isSharedView = props.routePath === `${props.basePath}/shared`
  const theme = props.theme
  // 先按 key 屏蔽旧结果，避免 effect 清理前的一帧暴露旧组织数据。
  const currentRolesState = rolesState.contextKey === contextKey
    ? rolesState
    : emptyRolesState(contextKey)

  const loadRoles = async () => {
    if (!isCurrentContext(props)) return
    cancelRolesRequest()
    const controller = new AbortController()
    rolesRequest = controller
    const abort = () => controller.abort()
    props.signal.addEventListener("abort", abort, { once: true })
    const canApply = () => !controller.signal.aborted && rolesRequest === controller && isCurrentContext(props)
    setRolesState({ ...emptyRolesState(contextKey), loading: true, fired: true })
    try {
      const res = await fetch("/api/external/roles", {
        headers: { Authorization: `Bearer ${props.token}` },
        signal: controller.signal,
      })
      const body = (await res.json().catch(() => ({}))) as ExternalRolesResp & { message?: string }
      if (!canApply()) return
      setRolesState({
        contextKey,
        loading: false,
        fired: true,
        status: String(res.status),
        ok: res.ok,
        message: body.message ?? "",
        data: res.ok ? body : null,
      })
    } catch (e) {
      if (!canApply()) return
      setRolesState({
        contextKey,
        loading: false,
        fired: true,
        status: "—",
        ok: false,
        message: e instanceof Error ? e.message : String(e),
        data: null,
      })
    } finally {
      props.signal.removeEventListener("abort", abort)
      if (rolesRequest === controller) rolesRequest = null
    }
  }

  const sendNotify = (variant: MicroNotifyVariant) => {
    if (isCurrentContext(props)) props.onNotify(`来自 ESM 报表中心的消息 (${variant})`, variant)
  }

  return (
    <div className="space-y-4 p-4 sm:p-6">
      {/* ---------- 子应用头部 ---------- */}
      <header className="flex flex-wrap items-center gap-2 border-b pb-3">
        <span className="inline-flex items-center gap-1 rounded-full border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 text-[11px] font-medium text-emerald-700 dark:text-emerald-300">
          <Zap className="size-3" />
          ESM 通道 · 第一方直挂
        </span>
        <span className="inline-flex items-center gap-1 rounded-full border bg-muted px-2 py-0.5 text-[11px] font-medium text-muted-foreground">
          <Share2 className="size-3" />
          React {React.version} · 共享运行时
        </span>
        <span className="ml-auto font-mono text-[11px] text-muted-foreground">
          routePath: <code className="rounded bg-muted px-1.5 py-px">{props.routePath}</code>
        </span>
      </header>

      {isSharedView ? (
        <SharedRuntimeView
          props={props}
          theme={theme}
          clicks={clicks}
          onCounter={() => setClicks((c) => c + 1)}
          onNotify={sendNotify}
          seconds={seconds}
          updateCount={updateCount}
          lastUpdateAt={lastUpdateAt}
        />
      ) : (
        <OverviewView
          props={props}
          theme={theme}
          seconds={seconds}
          updateCount={updateCount}
          lastUpdateAt={lastUpdateAt}
          rolesState={currentRolesState}
          onLoadRoles={loadRoles}
          onNotify={sendNotify}
        />
      )}
    </div>
  )
}

/* ============================ 视图: 数据总览 ============================ */

function OverviewView({
  props,
  theme,
  seconds,
  updateCount,
  lastUpdateAt,
  rolesState,
  onLoadRoles,
  onNotify,
}: {
  props: MicroProps
  theme: string
  seconds: number
  updateCount: number
  lastUpdateAt: string
  rolesState: RolesState
  onLoadRoles: () => void
  onNotify: (v: MicroNotifyVariant) => void
}) {
  const orgs = rolesState.data?.organizations ?? []
  const totalRoles = orgs.reduce((acc, o) => acc + o.roles.length, 0)

  return (
    <div className="grid gap-4 lg:grid-cols-2">
      {/* ① 主应用注入的 props */}
      <section className="rounded-lg border bg-card p-4">
        <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold">
          <Layers className="size-4 text-primary" />
          ① 主应用注入的 props
        </h3>
        <dl className="grid gap-2 text-xs">
          <Kv k="user.username" v={props.user.username} />
          <Kv k="user.displayName" v={props.user.displayName} />
          <Kv k="tenant" v={`${props.tenant.name} (${props.tenant.code})`} />
          <Kv k="organization" v={props.organization.name} />
          <Kv k="appCode" v={props.appCode} />
          <Kv k="basePath" v={props.basePath} />
          <Kv k="routePath" v={props.routePath} mono />
          <Kv k="theme" v={theme} />
          <Kv k="contractVersion" v={props.contractVersion} />
          <Kv k="contextVersion" v={props.contextVersion} mono />
          <Kv k="contextEpoch" v={String(props.contextEpoch)} />
          <Kv k="token" v="应用专用令牌已注入" />
          <Kv k="tokenExpiresAt" v={props.tokenExpiresAt} />
          <Kv k="expiresAt" v={props.expiresAt} />
          <Kv k="menus" v={`注入 ${props.menus.length} 项`} />
          <Kv k="permissions" v={props.permissions.join(", ") || "无操作权限"} />
        </dl>
        <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">
          宿主同时注入应用上下文和最多 5 分钟有效的应用令牌。角色控制菜单和操作，
          数据范围由独立的成员关系授权决定；业务后端须再次校验两类授权。
        </p>
      </section>

      {/* ② 增量更新指标(未重挂证据) */}
      <section className="rounded-lg border bg-card p-4">
        <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold">
          <Activity className="size-4 text-primary" />
          ② 增量更新 · 未重挂证据
        </h3>
        <div className="grid grid-cols-2 gap-3">
          <div className="rounded-lg border bg-muted/40 p-3 text-center">
            <p className="font-mono text-xl font-semibold tabular-nums leading-none">{seconds}s</p>
            <p className="mt-1.5 text-[10px] text-muted-foreground">挂载时钟(不重置)</p>
          </div>
          <div className="rounded-lg border bg-muted/40 p-3 text-center">
            <p className="font-mono text-xl font-semibold tabular-nums leading-none">×{updateCount}</p>
            <p className="mt-1.5 text-[10px] text-muted-foreground">update 推送次数</p>
          </div>
        </div>
        <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">
          {updateCount > 0
            ? `最近一次推送 ${lastUpdateAt}。切换左侧菜单或主应用主题, 计时器持续走秒即为「增量更新·未重挂」的实证。`
            : "本子应用实现 lifecycle.update 契约: 子路径/主题/菜单变化时容器只推送新 props 不重挂。切换左侧菜单试试。"}
        </p>
      </section>

      {/* ③ 当前用户授权与对外 API 演示 */}
      <section className="rounded-lg border bg-card p-4 lg:col-span-2">
        <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
          <h3 className="flex items-center gap-2 text-sm font-semibold">
            <BarChart3 className="size-4 text-primary" />
            ③ 当前用户角色 · GET /api/external/roles
          </h3>
          <Button size="sm" variant="outline" onClick={onLoadRoles} disabled={rolesState.loading}>
            {rolesState.loading ? "请求中…" : "请求当前用户角色"}
          </Button>
        </div>
        <div className="mb-3 flex flex-wrap gap-1.5">
          {props.roles.map((role) => (
            <Badge key={role.id} variant="secondary">{role.name} · {role.code}</Badge>
          ))}
          {props.roles.length === 0 && <span className="text-xs text-muted-foreground">当前组织没有可用角色</span>}
        </div>
        <p className="mb-2 text-xs font-medium">独立数据授权</p>
        <div className="mb-3 flex flex-wrap gap-1.5">
          {props.dataGrants.map((grant, index) => (
            <Badge key={`${grant.resource}:${grant.action}:${grant.scope}:${index}`} variant="outline">
              {grant.resource}:{grant.action} · {grant.scope}
            </Badge>
          ))}
          {props.dataGrants.length === 0 && <span className="text-xs text-muted-foreground">未授予业务数据访问范围</span>}
        </div>
        {rolesState.fired && (
          <p className="mb-3 text-xs text-muted-foreground">
            HTTP{" "}
            <b className={rolesState.ok ? "text-emerald-600 dark:text-emerald-400" : "text-destructive"}>
              {rolesState.status}
            </b>
            {rolesState.ok && rolesState.data?.application && ` · 应用「${rolesState.data.application.name}」`}
            {rolesState.ok && ` · 当前组织 / ${totalRoles} 个有效角色`}
          </p>
        )}
        {rolesState.loading && (
          <div className="space-y-2">
            <Skeleton className="h-6 w-full" />
            <Skeleton className="h-6 w-5/6" />
            <Skeleton className="h-6 w-4/6" />
          </div>
        )}
        {rolesState.fired && !rolesState.loading && !rolesState.ok && (
          <p className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
            {rolesState.message || "请求失败"}
          </p>
        )}
        {rolesState.ok && !rolesState.loading && (
          <div className="space-y-3">
            {orgs.map((org) => (
              <div key={org.id}>
                <div className="mb-1 flex items-baseline gap-2 text-xs">
                  <span className="font-semibold">{org.name}</span>
                  <span className="text-muted-foreground">· {org.tenantName}</span>
                  <span className="ml-auto font-mono text-muted-foreground">{org.roles.length} 角色</span>
                </div>
                {org.roles.length > 0 && (
                  <div className="mt-1.5 flex flex-wrap gap-1.5">
                    {org.roles.map((role) => (
                      <Badge key={role.id} variant="secondary" className="gap-1 text-[10px] font-normal">
                        {role.name}
                        <span className="text-muted-foreground">{role.code}</span>
                      </Badge>
                    ))}
                  </div>
                )}
              </div>
            ))}
            <p className="text-[11px] text-muted-foreground">
              接口只返回当前用户在当前组织的有效角色，包含继承的祖先组织角色。
            </p>
          </div>
        )}
        {!rolesState.fired && !rolesState.loading && (
          <p className="text-xs text-muted-foreground">
            上方直接展示宿主注入的当前用户上下文。点击按钮可使用 props.token 调用对外 API；
            上下文切换会取消请求并丢弃迟到响应。
          </p>
        )}
      </section>
    </div>
  )
}

/* ============================ 视图: 共享运行时 ============================ */

function SharedRuntimeView({
  props,
  theme,
  clicks,
  onCounter,
  onNotify,
  seconds,
  updateCount,
  lastUpdateAt,
}: {
  props: MicroProps
  theme: string
  clicks: number
  onCounter: () => void
  onNotify: (v: MicroNotifyVariant) => void
  seconds: number
  updateCount: number
  lastUpdateAt: string
}) {
  return (
    <div className="grid gap-4 lg:grid-cols-2">
      {/* ① 共享组件与单例 */}
      <section className="rounded-lg border bg-card p-4">
        <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold">
          <Share2 className="size-4 text-primary" />
          ① 与主应用共享的组件实例
        </h3>
        <p className="mb-3 text-xs leading-relaxed text-muted-foreground">
          本页按钮/徽章/骨架屏全部 import 自主应用源码(<code className="rounded bg-muted px-1">@/components/ui</code>
          ), 渲染在子应用独立 React 根上, 但共享同一份 React {React.version} 运行时与组件模块。
        </p>
        <div className="flex flex-wrap items-center gap-3">
          <Button size="sm" onClick={onCounter}>
            shadcn Button · 点击 {clicks} 次
          </Button>
          <Badge>Badge 组件</Badge>
          <Badge variant="secondary">secondary</Badge>
          <Badge variant="outline">outline</Badge>
        </div>
        <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">
          组件状态(count={clicks})存活于子应用根; 主应用与子应用各自持有独立的 React 根,
          但模块单例共享 —— 这是 ESM 通道与 HTML 通道(隔离沙箱)最本质的差异。
        </p>
      </section>

      {/* ② 主题级联 */}
      <section className="rounded-lg border bg-card p-4">
        <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold">
          <Palette className="size-4 text-primary" />
          ② 主题级联 · 无桥接
        </h3>
        <div className="mb-3 flex items-center gap-2 text-sm">
          {theme === "dark" ? <Moon className="size-4" /> : <Sun className="size-4" />}
          <span className="font-medium">当前主题: {theme}</span>
        </div>
        <div className="grid grid-cols-4 gap-2">
          {[
            { name: "background", cls: "bg-background" },
            { name: "card", cls: "bg-card" },
            { name: "primary", cls: "bg-primary" },
            { name: "muted", cls: "bg-muted" },
          ].map((sw) => (
            <div key={sw.name} className="space-y-1">
              <div className={cn("h-10 rounded-md border", sw.cls)} />
              <p className="text-center font-mono text-[9px] text-muted-foreground">{sw.name}</p>
            </div>
          ))}
        </div>
        <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">
          Tailwind 的 <code className="rounded bg-muted px-1">dark:</code> 变体与 CSS 变量挂在
          <code className="rounded bg-muted px-1">html.dark</code>上, 天然级联进子应用 DOM;
          props.theme 则经 update 契约增量推送, 两种机制叠加实现零桥接的主题联动。
          切换右上角主题试试 —— 色板与文案即时跟随。
        </p>
      </section>

      {/* ③ 通信与导航 */}
      <section className="rounded-lg border bg-card p-4 lg:col-span-2">
        <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold">
          <Boxes className="size-4 text-primary" />
          ③ 与主应用通信(共享 sonner 单例 + props 回调)
        </h3>
        <div className="flex flex-wrap gap-2">
          <Button size="sm" variant="default" onClick={() => onNotify("success")}>
            props.onNotify · success
          </Button>
          <Button size="sm" variant="outline" onClick={() => onNotify("info")}>
            props.onNotify · info
          </Button>
          <Button size="sm" variant="outline" onClick={() => toast("直接调 sonner 单例(共享模块)")}>
            直接 import toast
          </Button>
        </div>
        <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">
          前两个按钮走主应用注入的 props.onNotify 回调(与 HTML 通道同款);
          第三个按钮直接 import 主应用的 sonner 模块调用 toast() —— 消息同样落在主应用
          Toaster 上, 仅 ESM 通道(共享运行时)能做到, 沙箱里的 HTML 通道子应用拿不到该模块。
        </p>
        <div className="mt-3 flex flex-wrap items-center gap-3 border-t pt-3 text-xs text-muted-foreground">
          <Button size="sm" variant="ghost" className="gap-1" onClick={() => props.onNavigate(props.basePath)}>
            onNavigate → 数据总览
            <ArrowUpRight className="size-3.5" />
          </Button>
          <span className="font-mono">
            挂载时钟 {seconds}s · update ×{updateCount}
            {updateCount > 0 && ` · 最近 ${lastUpdateAt}`}
          </span>
        </div>
      </section>
    </div>
  )
}

/* ============================ 小组件 ============================ */

function Kv({ k, v, mono }: { k: string; v: string; mono?: boolean }) {
  return (
    <div className="flex items-baseline gap-3">
      <dt className="w-32 shrink-0 font-mono text-[11px] text-muted-foreground">{k}</dt>
      <dd className={cn("min-w-0 break-all font-medium", mono && "font-mono text-[11px]")}>{v}</dd>
    </div>
  )
}

/* ============================ 生命周期契约 ============================ */

function teardown(): Promise<void> {
  if (teardownTask) return teardownTask
  const retiringRoot = root
  const retiringContainer = containerEl
  cancelRolesRequest()
  root = null
  containerEl = null
  latestProps = null
  bumpListeners = []
  updateCount = 0
  // 在宿主 React commit 后卸载另一棵根；Promise 完成前禁止下一次 mount。
  const task = Promise.resolve().then(() => {
    retiringRoot?.unmount()
    if (retiringContainer) retiringContainer.replaceChildren()
  }).finally(() => {
    if (teardownTask === task) teardownTask = null
  })
  teardownTask = task
  return task
}

export const __DANTE_MICRO_APP__: MicroLifecycle = {
  name: "report-center",

  async mount(container: HTMLElement, props: MicroProps) {
    if (teardownTask) await teardownTask
    if (root) await teardown()
    if (!container) throw new Error("mount 需要一个 container HTMLElement")
    if (props.signal.aborted) throw new Error("应用上下文已失效")
    latestProps = props
    updateCount = 0
    lastUpdateAt = ""
    containerEl = container
    container.replaceChildren()
    root = createRoot(container)
    root.render(<ReportApp />)
  },

  update(props: MicroProps) {
    if (!root || !latestProps || props.signal.aborted) return
    if (props.contextEpoch < latestProps.contextEpoch) return
    if (authorizationKey(props) !== authorizationKey(latestProps)) cancelRolesRequest()
    latestProps = props
    updateCount += 1
    const d = new Date()
    lastUpdateAt = `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`
    notifyUpdate()
  },

  unmount: teardown,
}

/** 兼容容器 3b 通道的裸导出形状(mod.mount / mod.unmount / mod.update) */
export const mount = (container: HTMLElement, props: MicroProps) =>
  __DANTE_MICRO_APP__.mount(container, props)
export const unmount = () => __DANTE_MICRO_APP__.unmount()
export const update = (props: MicroProps) => __DANTE_MICRO_APP__.update?.(props)
