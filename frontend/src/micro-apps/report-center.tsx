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

/* ============================ 对外 API 类型 ============================ */

interface ExternalRoleOrg {
  id: string
  name: string
  tenantName: string
  roles: Array<{ id: string; name: string; code: string; builtIn: boolean; menuCount: number }>
}

interface ExternalRolesResp {
  application?: { code: string; name: string }
  client?: { clientId: string; tokenType: string; username?: string }
  organizations?: ExternalRoleOrg[]
}

/* ============================ 子应用根组件 ============================ */

function ReportApp() {
  const props = latestProps
  const [, force] = React.useReducer((x: number) => x + 1, 0)
  const [seconds, setSeconds] = React.useState(0)
  const [clicks, setClicks] = React.useState(0)
  const [rolesState, setRolesState] = React.useState<{
    loading: boolean
    fired: boolean
    status: string
    ok: boolean
    message: string
    data: ExternalRolesResp | null
  }>({ loading: false, fired: false, status: "", ok: false, message: "", data: null })

  // props 模块变量 → React 状态桥: update() 时强制重渲染
  React.useEffect(() => {
    bumpListeners.push(force)
    return () => {
      bumpListeners = bumpListeners.filter((fn) => fn !== force)
    }
  }, [force])

  // 挂载时钟(不重挂证据: update 推送不重置)
  React.useEffect(() => {
    const id = window.setInterval(() => setSeconds((s) => s + 1), 1000)
    return () => window.clearInterval(id)
  }, [])

  if (!props) return null

  const isSharedView = props.routePath === `${props.basePath}/shared`
  const theme = props.theme
  const token = props.token ?? ""

  const loadRoles = async () => {
    setRolesState((s) => ({ ...s, loading: true, fired: true }))
    try {
      const res = await fetch("/api/external/roles", {
        headers: { Authorization: `Bearer ${token}` },
      })
      const body = (await res.json().catch(() => ({}))) as ExternalRolesResp & { message?: string }
      setRolesState({
        loading: false,
        fired: true,
        status: String(res.status),
        ok: res.ok,
        message: body.message ?? "",
        data: res.ok ? body : null,
      })
    } catch (e) {
      setRolesState({
        loading: false,
        fired: true,
        status: "—",
        ok: false,
        message: e instanceof Error ? e.message : String(e),
        data: null,
      })
    }
  }

  const sendNotify = (variant: MicroNotifyVariant) => {
    props.onNotify(`来自 ESM 报表中心的消息 (${variant})`, variant)
  }

  const orgs = rolesState.data?.organizations ?? []
  const maxRoles = Math.max(1, ...orgs.map((o) => o.roles.length))

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
          rolesState={rolesState}
          onLoadRoles={loadRoles}
          maxRoles={maxRoles}
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
  maxRoles,
  onNotify,
}: {
  props: MicroProps
  theme: string
  seconds: number
  updateCount: number
  lastUpdateAt: string
  rolesState: {
    loading: boolean
    fired: boolean
    status: string
    ok: boolean
    message: string
    data: ExternalRolesResp | null
  }
  onLoadRoles: () => void
  maxRoles: number
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
          <Kv k="appCode" v={props.appCode} />
          <Kv k="basePath" v={props.basePath} />
          <Kv k="routePath" v={props.routePath} mono />
          <Kv k="theme" v={theme} />
          <Kv
            k="token"
            v={props.token ? `${props.token.slice(0, 10)}…` : "(未携带)"}
            mono={Boolean(props.token)}
          />
          <Kv k="menus" v={`注入 ${props.menus.length} 项`} />
        </dl>
        <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">
          会话令牌由 POST /api/oauth/session-token 按应用编码签发, 与 HTML 通道完全同源 ——
          通道只决定「如何渲染」, 权限链路对所有通道一致。
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

      {/* ③ 组织角色分布(external API) */}
      <section className="rounded-lg border bg-card p-4 lg:col-span-2">
        <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
          <h3 className="flex items-center gap-2 text-sm font-semibold">
            <BarChart3 className="size-4 text-primary" />
            ③ 组织角色分布 · GET /api/external/roles
          </h3>
          <Button size="sm" variant="outline" onClick={onLoadRoles} disabled={rolesState.loading}>
            {rolesState.loading ? "请求中…" : "拉取角色数据"}
          </Button>
        </div>
        {rolesState.fired && (
          <p className="mb-3 text-xs text-muted-foreground">
            HTTP{" "}
            <b className={rolesState.ok ? "text-emerald-600 dark:text-emerald-400" : "text-destructive"}>
              {rolesState.status}
            </b>
            {rolesState.ok && rolesState.data?.application && ` · 应用「${rolesState.data.application.name}」`}
            {rolesState.ok && rolesState.data?.client && ` · ${rolesState.data.client.clientId} · 用户: ${props.user.username}`}
            {rolesState.ok && ` · ${orgs.length} 组织 / ${totalRoles} 角色`}
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
                <div
                  className="h-4 overflow-hidden rounded-md border bg-muted/50"
                  role="img"
                  aria-label={`${org.name} ${org.roles.length} 个角色`}
                >
                  <div
                    className="h-full rounded-[3px] bg-primary/70 transition-[width] duration-500"
                    style={{ width: `${Math.max(4, (org.roles.length / maxRoles) * 100)}%` }}
                  />
                </div>
                {org.roles.length > 0 && (
                  <div className="mt-1.5 flex flex-wrap gap-1.5">
                    {org.roles.map((role) => (
                      <Badge key={role.id} variant="secondary" className="gap-1 text-[10px] font-normal">
                        {role.name}
                        {role.builtIn && <span className="text-muted-foreground">内置</span>}
                        <span className="text-muted-foreground">{role.menuCount}菜单</span>
                      </Badge>
                    ))}
                  </div>
                )}
              </div>
            ))}
            <p className="text-[11px] text-muted-foreground">
              角色条宽按各组织角色数等比渲染, 主题色取自主应用 CSS 变量(切主题即变色)。
            </p>
          </div>
        )}
        {!rolesState.fired && !rolesState.loading && (
          <p className="text-xs text-muted-foreground">
            点击「拉取角色数据」: 子应用以 props.token 调用对外 API, 数据按应用隔离(仅 report-center 关联的授权)。
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

export const __DANTE_MICRO_APP__: MicroLifecycle = {
  name: "report-center",

  mount(container: HTMLElement, props: MicroProps) {
    return new Promise<void>((resolve, reject) => {
      try {
        if (!container) {
          reject(new Error("mount 需要一个 container HTMLElement"))
          return
        }
        latestProps = props
        updateCount = 0
        lastUpdateAt = ""
        containerEl = container
        container.innerHTML = ""
        root = createRoot(container)
        root.render(<ReportApp />)
        resolve()
      } catch (err) {
        reject(err)
      }
    })
  },

  update(props: MicroProps) {
    latestProps = props
    updateCount += 1
    const d = new Date()
    lastUpdateAt = `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`
    notifyUpdate()
  },

  unmount() {
    return new Promise<void>((resolve) => {
      try {
        const r = root
        const el = containerEl
        root = null
        containerEl = null
        latestProps = null
        bumpListeners = []
        updateCount = 0
        if (r || el) {
          // ① 容器的 effect cleanup 在主应用 React commit 阶段同步执行, 在此处同步
          //    unmount 另一个 React root 会触发 "synchronously unmount a root while
          //    React was already rendering" 竞态错误 → 延迟到微任务;
          // ② root.unmount() 必须先于 innerHTML 清空: React 卸载时要按自己的宿主
          //    节点回收 DOM, 若先清空会报 "node to be removed is not a child" 并
          //    触发 <div> 组件错误告警(两步一并延迟, 顺序保证)。
          void Promise.resolve().then(() => {
            try {
              r?.unmount()
            } catch {
              /* 已脱离文档的 root 卸载容错 */
            }
            try {
              if (el && el.isConnected) el.innerHTML = ""
            } catch {
              /* ignore */
            }
          })
        }
      } catch {
        /* 卸载容错: 无论如何 resolve */
      }
      resolve()
    })
  },
}

/** 兼容容器 3b 通道的裸导出形状(mod.mount / mod.unmount / mod.update) */
export const mount = (container: HTMLElement, props: MicroProps) =>
  __DANTE_MICRO_APP__.mount(container, props)
export const unmount = () => __DANTE_MICRO_APP__.unmount()
export const update = (props: MicroProps) => __DANTE_MICRO_APP__.update?.(props)
