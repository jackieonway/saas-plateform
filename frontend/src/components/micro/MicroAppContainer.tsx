/**
 * 微前端编排容器(/micro/:appCode/*)
 *
 * 职责:
 *  1. 按 appCode 从用户已授权应用列表(profile.applications, GET /api/auth/profile)解析
 *     入口配置(entryType/entryUrl) — 不走 GET /api/applications(需 system:application
 *     管理权限, 普通用户 403); 未授权应用在此直接得到「未知微应用」空态。
 *  2. HTML 入口: MicroSandbox + loadHtmlEntry(import-html-entry) → 沙箱执行子应用脚本
 *     → 取 window.__DANTE_MICRO_APP__ 生命周期 → mount(container, props)
 *     ESM 入口: 动态 import(entryUrl) → module.mount/unmount(第一方直挂, 无沙箱,
 *     与主应用共享 React/组件/主题变量运行时)
 *  3. Props 组装: token(POST /api/oauth/session-token, 失败不阻塞) / user / theme /
 *     menus(该应用菜单树) / routePath / basePath / onNavigate / onNotify
 *  4. 增量更新契约(v1.1): 子应用若实现 lifecycle.update(props), 则子路径/主题/菜单
 *     变化时容器只调 update 不重挂(未实现则退回 v1 全量重挂, 向后兼容)。
 *  5. 沙箱诊断(v1.1): HTML 通道挂载后状态条提供实时 getStats() 面板
 *     (计时器/监听/插入节点/自有键/覆盖键)。
 *  6. 卸载: lifecycle.unmount() → sandbox.teardown()(StrictMode 双挂载用挂载序号防竞态)
 */

import * as React from "react"
import { useLocation, useNavigate, useParams } from "react-router-dom"
import {
  Activity,
  Boxes,
  ChevronRight,
  CircleAlert,
  Ear,
  FileCode2,
  Package,
  RotateCw,
  ShieldQuestion,
  Timer,
  Zap,
} from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { AppIcon } from "@/lib/icons"
import { cn } from "@/lib/utils"
import { useAuthStore } from "@/stores/auth"
import { useTheme } from "@/components/theme-provider"
import { useMenusQuery } from "@/hooks/use-auth-data"
import { oauthApi } from "@/api/oauth"
import type { MenuNode } from "@/api/types"
import { EmptyState, ErrorState } from "@/components/states"
import { MicroSandbox } from "./sandbox"
import type { MicroSandboxStats } from "./sandbox"
import { loadHtmlEntry, markEntryFailed } from "./loader"
import { MICRO_APP_GLOBAL } from "./types"
import type {
  ApplicationEntryInfo,
  MicroEsmModule,
  MicroLifecycle,
  MicroMenuItem,
  MicroNotifyVariant,
  MicroProps,
  MicroStatus,
  MicroTokenState,
} from "./types"

/* ============================ 外层: 入口配置解析 ============================ */

export default function MicroAppContainer() {
  const { appCode = "" } = useParams()
  const profile = useAuthStore((s) => s.profile)

  const app = React.useMemo(
    () => profile?.applications.find((a) => a.code === appCode) ?? null,
    [profile?.applications, appCode]
  )

  if (!app) {
    return (
      <EmptyState
        className="py-20"
        title="未知微应用"
        description={
          <>
            应用编码 <code className="rounded bg-muted px-1.5 py-px font-mono text-xs">{appCode || "(空)"}</code>{" "}
            不在你当前组织上下文已授权的应用列表中, 请确认应用已创建/授权且编码无误
          </>
        }
        icon={<ShieldQuestion className="size-6" />}
      />
    )
  }

  const entryType = app.entryType === "HTML" || app.entryType === "ESM" ? app.entryType : null
  const entryUrl = (app.entryUrl ?? "").trim()
  if (!entryType || !entryUrl) {
    return (
      <EmptyState
        className="py-20"
        title="应用未配置微前端入口"
        description="该应用缺少 entryType / entryUrl 配置, 请在「系统管理 → 应用管理」编辑应用的入口类型与入口地址"
        icon={<CircleAlert className="size-6" />}
      />
    )
  }

  const entryInfo: ApplicationEntryInfo = {
    id: app.id,
    code: app.code,
    name: app.name,
    icon: app.icon || "LayoutGrid",
    entryType,
    entryUrl,
  }

  // key=appCode: 路由参数变化时整体重建挂载区(含全部内部状态)
  return <MicroMountZone key={app.code} entryInfo={entryInfo} />
}

/* ============================ 内层: 挂载编排 + 状态条 UI ============================ */

interface ActiveEntry {
  sandbox: MicroSandbox | null
  lifecycle: MicroLifecycle | null
  disposed: boolean
}

function MicroMountZone({ entryInfo }: { entryInfo: ApplicationEntryInfo }) {
  const navigate = useNavigate()
  const { pathname: routePath } = useLocation()
  const profile = useAuthStore((s) => s.profile)
  const { resolvedTheme } = useTheme()

  const appCode = entryInfo.code
  const basePath = `/micro/${appCode}`

  // 该应用的菜单树(复用 auth-menus 查询键, 与侧栏共享缓存; 拉取失败 → 空数组不阻塞)
  const menusQuery = useMenusQuery(entryInfo.id)
  const menus: MicroMenuItem[] = React.useMemo(() => {
    const tree = (menusQuery.data ?? []) as MenuNode[]
    return tree as unknown as MicroMenuItem[]
  }, [menusQuery.data])

  /* ---------- 挂载状态 ---------- */
  const containerRef = React.useRef<HTMLDivElement>(null)
  const seqRef = React.useRef(0)
  const activeRef = React.useRef<ActiveEntry | null>(null)
  /** 会话令牌最新值(mount 时获取, update 时复用; 不再变化后失效重挂) */
  const tokenRef = React.useRef<string | undefined>(undefined)
  /** 增量更新防抖签名(挂载后首签跳过: mount props 已含同值) */
  const lastPushSigRef = React.useRef<string | null>(null)
  const [status, setStatus] = React.useState<MicroStatus>("loading")
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null)
  const [tokenState, setTokenState] = React.useState<MicroTokenState>("pending")
  const [tokenNote, setTokenNote] = React.useState<string | null>(null)
  const [reloadNonce, setReloadNonce] = React.useState(0)
  /** 子应用是否实现 update 契约(能力徽章用) */
  const [supportsUpdate, setSupportsUpdate] = React.useState(false)
  /** 沙箱诊断统计(HTML 通道, 1s 轮询) */
  const [stats, setStats] = React.useState<MicroSandboxStats | null>(null)

  const disposeEntry = React.useCallback(async (entry: ActiveEntry) => {
    if (entry.disposed) return
    entry.disposed = true
    try {
      await entry.lifecycle?.unmount()
    } catch (e) {
      console.warn(`[micro:${entry.sandbox?.name ?? appCode}] unmount 异常(已忽略):`, e)
    }
    entry.sandbox?.teardown()
  }, [appCode])

  /* ---------- Props 组装(挂载/增量更新共用) ---------- */
  const buildProps = React.useCallback(
    (path: string): MicroProps => ({
      appCode,
      basePath,
      routePath: path,
      token: tokenRef.current,
      user: {
        id: profile?.user.id ?? "",
        username: profile?.user.username ?? "",
        displayName: profile?.user.realName || profile?.user.username || "",
      },
      theme: resolvedTheme,
      menus,
      onNavigate: (path: string) => {
        if (typeof path !== "string" || !path) return
        if (/^https?:\/\//i.test(path)) {
          window.open(path, "_blank", "noopener,noreferrer")
          return
        }
        navigate(path.startsWith("/") ? path : `/${path}`)
      },
      onNotify: (message: string, variant?: MicroNotifyVariant) => {
        const text = typeof message === "string" && message ? message : "(空消息)"
        switch (variant) {
          case "success": toast.success(text); break
          case "error": toast.error(text); break
          case "warning": toast.warning(text); break
          case "info": toast.info(text); break
          default: toast(text)
        }
      },
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [appCode, basePath, profile?.user, resolvedTheme, menus, navigate]
  )

  /**
   * 挂载流程(依赖 [appCode, entryUrl, reloadNonce] — 刻意不含 routePath):
   * v1.1 子路径变化交由 lifecycle.update 增量处理(未实现该契约的子应用由
   * routePath 兜底重挂, 见下方「无 update 契约时的兜底重挂」effect)。
   * StrictMode 双挂载竞态: mountSeq 序号 + activeRef, 迟到的异步阶段全部自废。
   */
  React.useEffect(() => {
    const seq = ++seqRef.current
    const isStale = () => seqRef.current !== seq
    let localEntry: ActiveEntry | null = null
    // document.title 快照(沙箱不再代理 document, title 还原由容器负责)
    const titleSnapshot = document.title

    setStatus("loading")
    setErrorMessage(null)
    setTokenState("pending")
    setTokenNote(null)
    setSupportsUpdate(false)
    setStats(null)
    lastPushSigRef.current = null
    tokenRef.current = undefined

    void (async () => {
      /* 1. 会话令牌(失败 → undefined, 记录状态, 不阻塞挂载) */
      try {
        const resp = await oauthApi.sessionToken(appCode)
        tokenRef.current = resp.accessToken
        if (!isStale()) setTokenState("injected")
      } catch (e) {
        tokenRef.current = undefined
        if (!isStale()) {
          setTokenState("missing")
          setTokenNote(e instanceof Error ? e.message : "会话令牌获取失败")
        }
      }
      if (isStale()) return

      /* 2. 组装 Props(以挂载时刻的 routePath 为初始值) */
      const props = buildProps(routePath)

      const container = containerRef.current
      if (!container) return

      try {
        if (entryInfo.entryType === "HTML") {
          /* 3a. HTML 入口: 沙箱 + import-html-entry */
          const sandbox = new MicroSandbox(appCode)
          localEntry = { sandbox, lifecycle: null, disposed: false }
          activeRef.current = localEntry

          const entry = await loadHtmlEntry(entryInfo.entryUrl)
          if (isStale()) return
          container.innerHTML = entry.templateProcessed
          await entry.execScripts(sandbox.proxy)
          if (isStale()) return

          const lifec = (sandbox.proxy as Record<string, unknown>)[MICRO_APP_GLOBAL] as
            | MicroLifecycle
            | undefined
          if (
            !lifec ||
            typeof lifec.mount !== "function" ||
            typeof lifec.unmount !== "function"
          ) {
            throw new Error(
              `子应用未暴露生命周期契约: window.${MICRO_APP_GLOBAL} = { name, mount(container, props), unmount() }`
            )
          }
          localEntry.lifecycle = lifec
          await lifec.mount(container, props)
          if (isStale()) {
            // cleanup 已先跑: 补偿式卸载(mount 完成于 dispose 之后)
            await disposeEntry(localEntry)
            return
          }
          setSupportsUpdate(typeof lifec.update === "function")
          setStatus("mounted")
          // 记录挂载时实际推送的 props 签名: 挂载期间路由/主题若已变化,
          // 增量更新 effect 会因签名不一致而补推(竞态防护)
          lastPushSigRef.current = `${props.routePath}|${props.theme}|${props.menus.length}|${tokenRef.current ? 1 : 0}`
        } else {
          /* 3b. ESM 入口: 动态 import 直挂(第一方信任档, 无沙箱, 共享运行时) */
          const mod = (await import(/* @vite-ignore */ entryInfo.entryUrl)) as MicroEsmModule
          if (isStale()) return
          const lifec: MicroLifecycle | null =
            mod.__DANTE_MICRO_APP__ ??
            (mod.mount && mod.unmount
              ? { name: entryInfo.name, mount: mod.mount, unmount: mod.unmount, update: mod.update }
              : null)
          if (!lifec) {
            throw new Error("ESM 模块未暴露生命周期契约(__DANTE_MICRO_APP__ 或 mount/unmount 导出)")
          }
          localEntry = { sandbox: null, lifecycle: lifec, disposed: false }
          activeRef.current = localEntry
          await lifec.mount(container, props)
          if (isStale()) {
            await disposeEntry(localEntry)
            return
          }
          setSupportsUpdate(typeof lifec.update === "function")
          setStatus("mounted")
          // 同 HTML 通道: 记录挂载时实际推送的 props 签名(竞态防护)
          lastPushSigRef.current = `${props.routePath}|${props.theme}|${props.menus.length}|${tokenRef.current ? 1 : 0}`
        }
      } catch (e) {
        if (isStale()) return
        console.error(`[micro:${appCode}] 挂载失败:`, e)
        // 计入亡缓存穿透态: 修复子应用源文件后点「重试」将强制重新拉取脚本(避开 scriptCache 旧文本)
        if (entryInfo.entryType === "HTML") markEntryFailed(entryInfo.entryUrl)
        setStatus("error")
        setErrorMessage(e instanceof Error ? e.message : String(e))
        if (localEntry) await disposeEntry(localEntry)
      }
    })()

    return () => {
      seqRef.current++ // 使在途回调全部过期
      const entry = activeRef.current
      if (entry) void disposeEntry(entry)
      // 子应用可能改写 document.title(沙箱不代理 document), 卸载后还原
      if (document.title !== titleSnapshot) document.title = titleSnapshot
    }
    // routePath 刻意不入依赖: 挂载后子路径变化走 update 契约/兜底重挂
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [appCode, entryInfo.entryUrl, entryInfo.entryType, reloadNonce])

  /* ---------- 增量更新契约(v1.1): 子路径/主题/菜单变化免重挂 ---------- */
  React.useEffect(() => {
    if (status !== "mounted") return
    const lifec = activeRef.current?.lifecycle
    if (!lifec || typeof lifec.update !== "function") return

    const sig = `${routePath}|${resolvedTheme}|${menus.length}|${tokenRef.current ? 1 : 0}`
    if (lastPushSigRef.current === sig) return
    lastPushSigRef.current = sig
    try {
      void lifec.update(buildProps(routePath))
    } catch (e) {
      console.warn(`[micro:${appCode}] update 异常(已忽略):`, e)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [routePath, resolvedTheme, menus, status, buildProps, appCode])

  /* ---------- 无 update 契约时的兜底: 子路径变化全量重挂(v1 行为) ---------- */
  React.useEffect(() => {
    if (status !== "mounted") return
    const lifec = activeRef.current?.lifecycle
    if (!lifec) return // 生命周期未就绪时不决策(防御: 避免重挂循环)
    if (typeof lifec.update === "function") return // 有契约 → 增量路径生效
    // 未实现 update: 路由变化触发重挂(变更 reloadNonce 即重跑挂载 effect)
    setReloadNonce((n) => n + 1)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [routePath])

  /* ---------- 沙箱诊断(v1.1): HTML 通道挂载后 1s 轮询 getStats ---------- */
  React.useEffect(() => {
    if (entryInfo.entryType !== "HTML" || status !== "mounted") {
      setStats(null)
      return
    }
    const read = () => {
      const sandbox = activeRef.current?.sandbox
      if (sandbox) setStats(sandbox.getStats())
    }
    read()
    const id = window.setInterval(read, 1000)
    return () => window.clearInterval(id)
  }, [entryInfo.entryType, status, reloadNonce])

  const handleReload = () => setReloadNonce((n) => n + 1)
  const loading = status === "loading"

  return (
    <div className="space-y-4">
      {/* ---------- 状态条 ---------- */}
      <div
        className="flex flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border bg-card px-4 py-3 shadow-sm"
        data-testid="micro-status-bar"
      >
        <div className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
          <AppIcon name={entryInfo.icon} className="size-5" />
        </div>
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <p className="truncate text-sm font-semibold leading-tight">{entryInfo.name}</p>
            <code className="shrink-0 rounded bg-muted px-1.5 py-px font-mono text-[10px] text-muted-foreground">
              {entryInfo.code}
            </code>
          </div>
          <p
            className="mt-0.5 truncate font-mono text-[11px] leading-tight text-muted-foreground"
            title={`当前路由: ${routePath} · 基路径: ${basePath}`}
          >
            {routePath}
          </p>
        </div>

        {/* 通道徽章: HTML 入口·沙箱(紫) / ESM 模块·直挂(灰) */}
        {entryInfo.entryType === "HTML" ? (
          <span className="inline-flex shrink-0 items-center gap-1 rounded-full border border-purple-500/30 bg-purple-500/10 px-2 py-0.5 text-[11px] font-medium text-purple-700 dark:text-purple-300">
            <Boxes className="size-3" />
            HTML 入口 · 沙箱
          </span>
        ) : (
          <span className="inline-flex shrink-0 items-center gap-1 rounded-full border bg-muted px-2 py-0.5 text-[11px] font-medium text-muted-foreground">
            <Package className="size-3" />
            ESM 模块 · 直挂
          </span>
        )}

        {/* 增量更新能力徽章(v1.1) */}
        {supportsUpdate && status === "mounted" && (
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="inline-flex shrink-0 items-center gap-1 rounded-full border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 text-[11px] font-medium text-emerald-700 dark:text-emerald-300">
                <Zap className="size-3" />
                增量更新
              </span>
            </TooltipTrigger>
            <TooltipContent side="bottom" className="max-w-xs text-xs">
              该子应用实现了 lifecycle.update 契约: 子路径/主题/菜单变化时容器只推送新 props,
              不重挂载(计时器/输入等内部状态保留)
            </TooltipContent>
          </Tooltip>
        )}

        {/* 令牌状态点 */}
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="inline-flex shrink-0 cursor-default items-center gap-1.5 text-[11px] text-muted-foreground">
              <span
                aria-hidden
                className={cn(
                  "size-2 rounded-full transition-colors",
                  tokenState === "injected" && "bg-emerald-500 shadow-[0_0_0_3px_rgba(16,185,129,0.15)]",
                  tokenState === "missing" && "border border-muted-foreground/40 bg-transparent",
                  tokenState === "pending" && "animate-pulse bg-amber-500"
                )}
              />
              {tokenState === "injected" ? "令牌已注入" : tokenState === "missing" ? "未携带令牌" : "令牌获取中"}
            </span>
          </TooltipTrigger>
          <TooltipContent side="bottom" className="max-w-xs text-xs">
            {tokenState === "injected"
              ? `POST /api/oauth/session-token 已为 ${entryInfo.code} 签发访问令牌并注入 props.token`
              : tokenState === "missing"
                ? `令牌获取失败(不阻塞挂载): ${tokenNote ?? "未知原因"}`
                : "正在为子应用申请会话令牌..."}
          </TooltipContent>
        </Tooltip>

        {/* 沙箱诊断入口(v1.1, 仅 HTML 通道) */}
        {entryInfo.entryType === "HTML" && stats && (
          <SandboxDiagnostics stats={stats} />
        )}

        <div className="ml-auto flex shrink-0 items-center gap-2">
          {status === "mounted" && (
            <span className="hidden items-center gap-1.5 text-[11px] text-muted-foreground sm:inline-flex">
              <span aria-hidden className="relative flex size-2">
                <span className="absolute inline-flex size-full animate-ping rounded-full bg-emerald-400 opacity-60" />
                <span className="relative inline-flex size-2 rounded-full bg-emerald-500" />
              </span>
              运行中
            </span>
          )}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="outline"
                size="icon"
                className="size-8"
                aria-label={`重新加载 ${entryInfo.name}`}
                onClick={handleReload}
                disabled={loading}
              >
                <RotateCw className={cn("size-4", loading && "animate-spin")} />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom" className="text-xs">
              强制重新挂载子应用
            </TooltipContent>
          </Tooltip>
        </div>
      </div>

      {/* ---------- 挂载区 / 错误卡 ---------- */}
      {status === "error" ? (
        <MountErrorCard message={errorMessage} onRetry={handleReload} />
      ) : (
        <div className="relative min-h-[480px] overflow-hidden rounded-xl border bg-card">
          {/* 加载骨架: React 管理的覆盖层(不进 containerRef, 避免与 innerHTML 互相打架) */}
          {loading && <MountAreaSkeleton />}
          {/* 子应用挂载点: 禁止放任何 React 子节点(innerHTML 由微前端内核接管) */}
          <div
            ref={containerRef}
            className="micro-mount-zone min-h-[480px] w-full"
            aria-label={`${entryInfo.name} 挂载区`}
            aria-busy={loading}
          />
        </div>
      )}
    </div>
  )
}

/* ============================ 沙箱诊断面板(v1.1) ============================ */

function SandboxDiagnostics({ stats }: { stats: MicroSandboxStats }) {
  const items = [
    { icon: Timer, label: "计时器", value: stats.timers, hint: "沙箱记账的 setTimeout/setInterval 存活数, 卸载时批量清理" },
    { icon: Ear, label: "window 监听", value: stats.listeners, hint: "沙箱记账的 window 级事件监听, 卸载时批量移除" },
    { icon: FileCode2, label: "插入节点", value: stats.insertedNodes, hint: "MutationObserver 记账的 head/body 新增 STYLE/LINK/SCRIPT, 卸载时回收" },
  ]
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex shrink-0 cursor-default items-center gap-1 rounded-full border bg-muted px-2 py-0.5 font-mono text-[11px] text-muted-foreground transition-colors hover:border-primary/40 hover:text-foreground"
          aria-label="查看沙箱诊断统计"
        >
          <Activity className="size-3 text-primary" />
          <span className="tabular-nums">
            沙箱 {stats.timers}t/{stats.listeners}l/{stats.insertedNodes}n
          </span>
        </button>
      </PopoverTrigger>
      <PopoverContent side="bottom" align="start" className="w-80">
        <div className="space-y-3">
          <div className="flex items-center gap-2">
            <span className="flex size-7 items-center justify-center rounded-md bg-purple-500/10 text-purple-600 dark:text-purple-400">
              <Boxes className="size-4" />
            </span>
            <div className="min-w-0">
              <p className="text-sm font-semibold leading-tight">沙箱运行诊断</p>
              <p className="font-mono text-[10px] text-muted-foreground">{stats.name}</p>
            </div>
            {stats.tornDown && (
              <span className="ml-auto rounded-full bg-muted px-2 py-0.5 text-[10px] text-muted-foreground">已卸载</span>
            )}
          </div>
          <div className="grid grid-cols-3 gap-2">
            {items.map((it) => (
              <Tooltip key={it.label}>
                <TooltipTrigger asChild>
                  <div className="rounded-lg border bg-muted/40 px-2 py-2 text-center">
                    <it.icon className="mx-auto size-3.5 text-muted-foreground" />
                    <p className="mt-1 font-mono text-sm font-semibold tabular-nums leading-none">{it.value}</p>
                    <p className="mt-1 text-[10px] leading-tight text-muted-foreground">{it.label}</p>
                  </div>
                </TooltipTrigger>
                <TooltipContent side="bottom" className="max-w-[220px] text-xs">{it.hint}</TooltipContent>
              </Tooltip>
            ))}
          </div>
          <div className="space-y-1.5 rounded-lg border bg-muted/30 px-3 py-2">
            <div className="flex items-center justify-between text-[11px]">
              <span className="text-muted-foreground">沙箱自有全局键</span>
              <span className="font-mono font-semibold tabular-nums">{stats.ownProps}</span>
            </div>
            <div className="flex items-center justify-between gap-2 text-[11px]">
              <span className="shrink-0 text-muted-foreground">覆盖真实 window 键</span>
              <span className="min-w-0 truncate font-mono text-right" title={stats.overriddenKeys.join(", ") || "(无)"}>
                {stats.overriddenKeys.length ? stats.overriddenKeys.join(", ") : "0 个"}
              </span>
            </div>
          </div>
          <p className="text-[10px] leading-relaxed text-muted-foreground">
            沙箱目标为防误伤与资源回收(计时器/监听/样式记账清理), 不是安全边界; 不可信代码走 iframe 通道。
          </p>
        </div>
      </PopoverContent>
    </Popover>
  )
}

/* ============================ 子组件: 骨架与错误卡 ============================ */

/** 挂载区加载骨架(pulse, 模拟子应用内容块) */
function MountAreaSkeleton() {
  return (
    <div
      className="absolute inset-0 z-10 space-y-5 bg-card/80 p-6 backdrop-blur-[1px]"
      role="status"
      aria-label="子应用加载中"
    >
      <div className="flex items-center gap-3">
        <Skeleton className="size-10 rounded-xl" />
        <div className="flex-1 space-y-2">
          <Skeleton className="h-4 w-40" />
          <Skeleton className="h-3 w-64" />
        </div>
        <Skeleton className="h-8 w-20 rounded-md" />
      </div>
      <div className="grid gap-3 sm:grid-cols-3">
        <Skeleton className="h-20 rounded-lg" />
        <Skeleton className="h-20 rounded-lg" />
        <Skeleton className="h-20 rounded-lg" />
      </div>
      <Skeleton className="h-44 rounded-lg" />
      <div className="space-y-2">
        <Skeleton className="h-3.5 w-full" />
        <Skeleton className="h-3.5 w-[92%]" />
        <Skeleton className="h-3.5 w-[85%]" />
      </div>
      <p className="flex items-center gap-2 text-xs text-muted-foreground">
        <RotateCw className="size-3 animate-spin" />
        正在拉取子应用入口并执行沙箱挂载...
      </p>
    </div>
  )
}

/** 错误卡: Alert + 重试 + 原始错误折叠 */
function MountErrorCard({ message, onRetry }: { message: string | null; onRetry: () => void }) {
  const [detailOpen, setDetailOpen] = React.useState(false)
  return (
    <Alert variant="destructive" className="items-start gap-4 py-4">
      <CircleAlert className="mt-0.5 size-5" />
      <div className="min-w-0 flex-1 space-y-3">
        <div className="space-y-1">
          <AlertTitle className="text-sm font-semibold">子应用挂载失败</AlertTitle>
          <AlertDescription className="text-xs leading-relaxed">
            {message ?? "加载或执行子应用时发生未知错误"}
          </AlertDescription>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button size="sm" variant="outline" onClick={onRetry}>
            <RotateCw className="size-3.5" />
            重试
          </Button>
          <Collapsible open={detailOpen} onOpenChange={setDetailOpen}>
            <CollapsibleTrigger asChild>
              <Button size="sm" variant="ghost" className="text-xs text-muted-foreground">
                <ChevronRight
                  className={cn("size-3.5 transition-transform", detailOpen && "rotate-90")}
                />
                {detailOpen ? "收起原始错误" : "展开原始错误"}
              </Button>
            </CollapsibleTrigger>
            <CollapsibleContent>
              <pre className="mt-2 max-h-40 overflow-auto scroll-area rounded-md bg-muted/60 p-3 font-mono text-[11px] leading-relaxed whitespace-pre-wrap break-all text-destructive/90">
                {message ?? "(无详细信息)"}
              </pre>
            </CollapsibleContent>
          </Collapsible>
        </div>
      </div>
    </Alert>
  )
}
