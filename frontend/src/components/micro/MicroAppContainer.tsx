/**
 * 微前端编排容器(/micro/:appCode/*)
 *
 * 职责:
 *  1. 按 appCode 从用户已授权应用列表(profile.applications, GET /api/auth/profile)发现应用；
 *     可执行完整入口统一取实时 context — 不走 GET /api/applications(需 system:application
 *     管理权限, 普通用户 403); 未授权应用在此直接得到「未知微应用」空态。
 *  2. 缺省 IFRAME: opaque sandbox + MessageChannel；TRUSTED 才允许 HTML 沙箱或 ESM
 *     在宿主页执行。每次获取应用上下文重新校验服务端实时运行模式。
 *  3. 先取得应用 token 与受众限定的完整上下文，再挂载；定时刷新上下文并续期 token。
 *  4. MicroProps 2.0 完整授权内容变化走 update；没有 update 时串行卸载后重挂。
 *  5. 沙箱诊断(v1.1): HTML 通道挂载后状态条提供实时 getStats() 面板
 *     (计时器/监听/插入节点/自有键/覆盖键)。
 *  6. 卸载: lifecycle.unmount() → sandbox.teardown()(StrictMode 双挂载用挂载序号防竞态)
 */

import * as React from "react"
import { loadHostEsm } from "@/micro-apps/registry"
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
import { useQueryClient } from "@tanstack/react-query"
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
import { reconcileSessionSync, useAuthStore } from "@/stores/auth"
import { useTheme } from "@/components/theme-provider"
import { api, ApiError } from "@/lib/api"
import { microRouteAccess } from "@/lib/micro-route-permissions"
import type { OAuthSessionTokenResp } from "@/api/types"
import { MicroContextRuntime, MicroLifecycleSession, isMicroRuntimeFault, microPropsSignature, microPropsDelivery, requiresMicroRemount,
  trustedMicroRuntimeDomain } from "./runtime"
import type { MicroContextSnapshot, MicroRuntimeFault } from "./runtime"
import { EmptyState, ErrorState } from "@/components/states"
import { MicroSandbox } from "./sandbox"
import type { MicroSandboxStats } from "./sandbox"
import { loadHtmlEntry, markEntryFailed } from "./loader"
import { createIframeLifecycle } from "./iframe"
import { resolveMicroEntry, microEntrySignature, assertMicroEntrySnapshot } from "./entry"
import { SDK_VERSION, MICRO_PROPS_CONTRACT_VERSION } from "../../../../examples/micro-app-sdk/generated/protocol.mjs"
import { assertBrowserCapabilities } from "../../../../examples/micro-app-sdk/src/capabilities.mjs"
import { MICRO_APP_GLOBAL } from "./types"
import type {
  ApplicationEntryInfo,
  AppAuthorizationContext,
  MicroEsmModule,
  MicroLifecycle,
  MicroNotifyVariant,
  MicroProps,
  MicroStatus,
  MicroTokenState,
} from "./types"

/* ============================ 外层: 入口配置解析 ============================ */

export default function MicroAppContainer() {
  const { appCode = "" } = useParams()
  const { pathname } = useLocation()
  const activeRoute = !!appCode && (pathname === `/micro/${appCode}` || pathname.startsWith(`/micro/${appCode}/`))
  const profile = useAuthStore((s) => s.profile)
  const sessionToken = useAuthStore((s) => s.token)
  const contextEpoch = useAuthStore((s) => s.contextEpoch)
  const switching = useAuthStore((s) => s.contextSwitching)
  const selectedApplicationId = useAuthStore((s) => s.currentApplicationId)
  const navigate = useNavigate()

  const app = React.useMemo(
    () => profile?.applications.find((a) => a.code === appCode) ?? null,
    [profile?.applications, appCode]
  )

  React.useEffect(() => {
    if (activeRoute && profile && !switching && !app) {
      toast.error("当前组织无法访问此应用，已退出应用")
      navigate("/", { replace: true })
    }
  }, [activeRoute, app, profile, switching, navigate])

  if (!activeRoute) return null

  if (!profile || switching || !sessionToken) return <div className="relative min-h-[480px]"><MountAreaSkeleton /></div>
  if (app && selectedApplicationId !== app.id) return <div className="relative min-h-[480px]"><MountAreaSkeleton /></div>

  if (!app) {
    return (
      <EmptyState
        className="py-20"
        title="应用不可用"
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

  const entryType = app.entryType === "ESM" ? "ESM" : "HTML"
  const entryUrl = (app.entryUrl ?? "").trim()

  const entryInfo: ApplicationEntryInfo = {
    id: app.id,
    code: app.code,
    name: app.name,
    icon: app.icon || "LayoutGrid",
    entryType,
    entryUrl,
    runtimeMode: app.runtimeMode === "TRUSTED" ? "TRUSTED" : "IFRAME",
    deploymentVersion: app.deploymentVersion,
    entryVersionId: app.entryVersionId,
  }

  // 组织、应用或信任模式改变时重新取得 token；版本变化只替换挂载实例。
  return <MicroMountZone key={`${contextEpoch}:${app.code}:${entryInfo.runtimeMode}`} entryInfo={entryInfo} />
}

/* ============================ 内层: 挂载编排 + 状态条 UI ============================ */

interface ActiveEntry {
  entry: ApplicationEntryInfo
  runtimeMode: ApplicationEntryInfo["runtimeMode"]
  sandbox: MicroSandbox | null
  session: MicroLifecycleSession
  controller: AbortController
  node: HTMLElement
  contextSignal?: AbortSignal
  propsSignal?: AbortSignal
}

// 同页可信代码必须先完成最终卸载，才能复用 ESM 单例及 document 记账；iframe 有自己的可销毁边界。
let trustedDisposalBarrier: Promise<void> = Promise.resolve()

function MicroMountZone({ entryInfo: discovery }: { entryInfo: ApplicationEntryInfo }) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const navigateRef = React.useRef(navigate)
  navigateRef.current = navigate
  const { pathname: routePath } = useLocation()
  const profile = useAuthStore((s) => s.profile)
  const sessionId = useAuthStore((s) => s.sessionId)
  const contextEpoch = useAuthStore((s) => s.contextEpoch)
  const { resolvedTheme } = useTheme()

  const appCode = discovery.code
  const discoveryRef = React.useRef(discovery)
  discoveryRef.current = discovery
  const profileEntrySignature = microEntrySignature(discovery)
  const observedProfileEntry = React.useRef(profileEntrySignature)
  const basePath = `/micro/${appCode}`
  const isTrusted = discovery.runtimeMode === "TRUSTED"

  /* ---------- 授权上下文与生命周期 ---------- */
  const containerRef = React.useRef<HTMLDivElement>(null)
  const seqRef = React.useRef(0)
  const activeRef = React.useRef<ActiveEntry | null>(null)
  const runtimeRef = React.useRef<MicroContextRuntime | null>(null)
  const lastPushSigRef = React.useRef<string | null>(null)
  const lastDeliveryRef = React.useRef<ReturnType<typeof microPropsDelivery> | null>(null)
  const aliveRef = React.useRef(true)
  const faultRef = React.useRef<MicroRuntimeFault | null>(isTrusted ? trustedMicroRuntimeDomain.fault : null)
  const [runtimeFault, setRuntimeFault] = React.useState<MicroRuntimeFault | null>(faultRef.current)
  const [snapshot, setSnapshot] = React.useState<MicroContextSnapshot | null>(null)
  const snapshotRef = React.useRef(snapshot)
  // The discovery profile never supplies executable fields once a context is available.
  const entryInfo: ApplicationEntryInfo = snapshot ? resolveMicroEntry(snapshot.context, discovery)
    : { ...discovery, entryType: "HTML", entryUrl: "", deploymentVersion: null, entryVersionId: null }
  const entryInfoRef = React.useRef(entryInfo)
  entryInfoRef.current = entryInfo
  const routeRef = React.useRef(routePath)
  routeRef.current = routePath
  const [status, setStatus] = React.useState<MicroStatus>(faultRef.current ? "error" : "loading")
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null)
  const [tokenState, setTokenState] = React.useState<MicroTokenState>(faultRef.current ? "missing" : "pending")
  const [tokenNote, setTokenNote] = React.useState<string | null>(null)
  const [reloadNonce, setReloadNonce] = React.useState(0)
  const [mountNonce, setMountNonce] = React.useState(0)
  const [supportsUpdate, setSupportsUpdate] = React.useState(false)
  const [stats, setStats] = React.useState<MicroSandboxStats | null>(null)
  const [recoveryAttempt, setRecoveryAttempt] = React.useState(0)

  React.useEffect(() => {
    aliveRef.current = true
    return () => { aliveRef.current = false }
  }, [])

  const reportRuntimeFailure = React.useCallback((kind: "LOAD_FAILED" | "LIFECYCLE_FAILED", stage: "mount" | "update") => {
    const value = snapshotRef.current
    if (!value || value.signal.aborted || Date.parse(value.context.expiresAt) <= Date.now() ||
        Date.parse(value.tokenExpiresAt) <= Date.now()) return
    // Static, bounded diagnostics only. Lifecycle cancellation must not cancel
    // this report, and failures must not change the host session or error UI.
    void fetch(`/api/apps/${encodeURIComponent(appCode)}/runtime-events`, {
      method: "POST", credentials: "omit", keepalive: true,
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${value.token}` },
      body: JSON.stringify({ kind, stage }), signal: AbortSignal.timeout(3000),
    }).catch(() => {})
  }, [appCode])

  const disposeEntry = React.useCallback((entry: ActiveEntry) => {
    entry.controller.abort()
    entry.node.remove()
    // Session faults carry fixed diagnostics; never log arbitrary lifecycle error objects.
    const disposal = entry.session.stop().catch(() => {})
    if (entry.runtimeMode === "TRUSTED") {
      trustedDisposalBarrier = Promise.all([trustedDisposalBarrier, disposal]).then(() => {})
    }
    return disposal
  }, [])

  const latchTrustedFault = React.useCallback((fault: MicroRuntimeFault) => {
    faultRef.current ??= fault
    // This cleanup remains necessary after React has unmounted the old zone. The domain itself
    // owns the permanent page latch, so leaving the route cannot make trusted execution safe again.
    runtimeRef.current?.stop()
    runtimeRef.current = null
    snapshotRef.current = null
    const active = activeRef.current
    if (active) {
      activeRef.current = null
      void disposeEntry(active)
    }
    lastPushSigRef.current = null
    lastDeliveryRef.current = null
    if (!aliveRef.current) return
    setSnapshot(null)
    setRuntimeFault(faultRef.current)
    setStatus("error")
    setErrorMessage("可信应用运行故障，需要重新加载宿主")
    setTokenState("missing")
    setTokenNote("可信运行域已停止复用")
    setRecoveryAttempt(0)
    setSupportsUpdate(false)
    setStats(null)
  }, [disposeEntry])

  React.useEffect(() => {
    if (!isTrusted) return
    const unsubscribe = trustedMicroRuntimeDomain.subscribe(latchTrustedFault)
    if (trustedMicroRuntimeDomain.fault) latchTrustedFault(trustedMicroRuntimeDomain.fault)
    return unsubscribe
  }, [isTrusted, latchTrustedFault])

  const buildProps = React.useCallback((path: string): MicroProps => {
    const value = snapshotRef.current
    const active = activeRef.current
    if (!value || value.signal.aborted || Date.parse(value.context.expiresAt) <= Date.now() ||
        Date.parse(value.tokenExpiresAt) <= Date.now() || !active || active.controller.signal.aborted) {
      throw new DOMException("应用授权上下文不可用", "AbortError")
    }
    if (active.contextSignal !== value.signal) {
      active.contextSignal = value.signal
      active.propsSignal = AbortSignal.any([value.signal, active.controller.signal])
    }
    const signal = active.propsSignal!
    const context = value.context
    assertMicroEntrySnapshot(active.entry, context)
    if (!microRouteAccess(context.menus, appCode, path).allowed) {
      throw new ApiError("当前组织未授权访问此应用菜单", "FORBIDDEN", 403)
    }
    return {
      sdkVersion: SDK_VERSION, contractVersion: MICRO_PROPS_CONTRACT_VERSION, contextVersion: context.contextVersion,
      contextEpoch: value.contextEpoch, expiresAt: context.expiresAt,
      tokenExpiresAt: value.tokenExpiresAt, signal,
      deploymentVersion: active.entry.deploymentVersion, entryVersionId: active.entry.entryVersionId,
      token: value.token, appCode, basePath, routePath: path,
      user: context.user, tenant: context.tenant, organization: context.organization,
      organizationScopeIds: context.organizationScopeIds === null ? null
        : [...(context.organizationScopeIds ?? [context.organization.id])],
      roles: context.roles, permissions: context.permissions,
      dataGrants: context.dataGrants, menus: context.menus,
      theme: resolvedTheme,
      onNavigate: (path: string) => {
        if (signal.aborted || typeof path !== "string" || !path) return
        if (/^https?:\/\//i.test(path)) {
          window.open(path, "_blank", "noopener,noreferrer")
          return
        }
        navigate(path.startsWith("/") ? path : "/" + path)
      },
      onNotify: (message: string, variant?: MicroNotifyVariant) => {
        if (signal.aborted) return
        const text = typeof message === "string" && message ? message : "(空消息)"
        switch (variant) {
          case "success": toast.success(text); break
          case "error": toast.error(text); break
          case "warning": toast.warning(text); break
          case "info": toast.info(text); break
          default: toast(text)
        }
      },
    }
  }, [appCode, basePath, resolvedTheme, navigate])
  const propsBuilderRef = React.useRef(buildProps)
  propsBuilderRef.current = buildProps

  React.useEffect(() => {
    // Bind issuance to the identity that rendered this zone. Token-only
    // rotation uses the latest pair without disposing the mounted application.
    const sessionFence = { sessionId, version: useAuthStore.getState().sessionVersion }
    if (isTrusted && trustedMicroRuntimeDomain.fault) {
      latchTrustedFault(trustedMicroRuntimeDomain.fault)
      return
    }
    snapshotRef.current = null
    const previousEntry = activeRef.current
    activeRef.current = null
    if (previousEntry) void disposeEntry(previousEntry)
    lastPushSigRef.current = null
    lastDeliveryRef.current = null
    setSnapshot(null)
    setStatus("loading")
    setTokenState("pending")
    setTokenNote(null)
    setErrorMessage(null)
    setRecoveryAttempt(0)
    try { assertBrowserCapabilities("host") }
    catch (error) {
      const message = error instanceof Error ? error.message : "Required browser capabilities are unavailable"
      setStatus("error")
      setErrorMessage(message)
      setTokenState("missing")
      setTokenNote(message)
      return
    }
    let modeRefreshRequested = false
    const runtime = new MicroContextRuntime({
      appCode,
      issue: (signal) => {
        reconcileSessionSync()
        const current = useAuthStore.getState()
        if (!current.token || current.contextSwitching || current.sessionId !== sessionId || current.contextEpoch !== contextEpoch) {
          throw new DOMException("登录上下文已变化", "AbortError")
        }
        return api.post<OAuthSessionTokenResp>("/api/oauth/session-token",
          { appCode }, { signal, suppress401: true, sessionFence })
      },
      context: async (token, signal) => {
        const context = await api.get<AppAuthorizationContext>(
          "/api/apps/" + encodeURIComponent(appCode) + "/context",
          { bearer: token, signal, suppress401: true })
        if (!context || !context.user || !context.tenant || !context.organization ||
            context.user.id !== profile?.user.id || context.tenant.id !== profile?.tenant.id ||
            context.organization.id !== profile?.organization.id) {
          throw new Error("应用授权上下文与平台会话不一致")
        }
        const currentEntry = resolveMicroEntry(context, discoveryRef.current)
        if (currentEntry.runtimeMode !== discovery.runtimeMode) {
          // 签发新 token 后必须重新核对信任边界，不能用缓存 profile 决定宿主执行。
          if (!modeRefreshRequested) {
            modeRefreshRequested = true
            void queryClient.invalidateQueries({ queryKey: ["profile"] })
          }
          throw Object.assign(new Error("应用运行模式已变更，请确认最新配置后重试"), { code: "MODE_MISMATCH" })
        }
        if (!microRouteAccess(context.menus, appCode, routeRef.current).allowed) {
          throw new ApiError("当前组织未授权访问此应用菜单", "FORBIDDEN", 403)
        }
        return context
      },
      onChange: (value) => {
        if (faultRef.current || (isTrusted && trustedMicroRuntimeDomain.fault)) {
          runtimeRef.current?.stop()
          return
        }
        snapshotRef.current = value
        const active = activeRef.current
        let replaceEntry = !value
        if (active && value) {
          try { assertMicroEntrySnapshot(active.entry, value.context) }
          catch { replaceEntry = true }
        }
        if (replaceEntry) {
          activeRef.current = null
          if (active) void disposeEntry(active)
          lastPushSigRef.current = null
          lastDeliveryRef.current = null
          setSupportsUpdate(false)
        }
        setSnapshot(value)
        if (value) setTokenState("injected")
        else { setTokenState("pending"); setStatus("loading") }
      },
      onRecovery: (state) => {
        if (faultRef.current || (isTrusted && trustedMicroRuntimeDomain.fault)) return
        setRecoveryAttempt(state?.attempt ?? 0)
        if (!state) setTokenNote(null)
        if (state) {
          setStatus("loading")
          setTokenState("missing")
          setTokenNote("授权服务暂时不可用，正在重新验证")
        }
      },
      onError: (error) => {
        if (faultRef.current || (isTrusted && trustedMicroRuntimeDomain.fault)) return
        const message = error instanceof Error ? error.message : "应用授权上下文获取失败"
        setStatus("error")
        setErrorMessage(message)
        setTokenState("missing")
        setTokenNote(message)
        if (error instanceof ApiError && [401, 403, 404].includes(error.status)) {
          toast.error("当前应用授权已失效，已退出应用")
          navigateRef.current("/", { replace: true })
        }
      },
    })
    runtimeRef.current = runtime
    void runtime.refresh()
    return () => {
      runtime.stop()
      if (runtimeRef.current === runtime) runtimeRef.current = null
    }
  }, [appCode, sessionId, contextEpoch, reloadNonce, profile?.user.id, profile?.tenant.id,
    profile?.organization.id, discovery.id, discovery.runtimeMode, queryClient, isTrusted, latchTrustedFault, disposeEntry])

  React.useEffect(() => {
    if (observedProfileEntry.current === profileEntrySignature) return
    observedProfileEntry.current = profileEntrySignature
    if (!faultRef.current && !(isTrusted && trustedMicroRuntimeDomain.fault)) void runtimeRef.current?.refresh()
  }, [profileEntrySignature, isTrusted])

  const hasSnapshot = !faultRef.current && !!snapshot && !snapshot.signal.aborted &&
    Date.parse(snapshot.context.expiresAt) > Date.now() && Date.parse(snapshot.tokenExpiresAt) > Date.now() &&
    microRouteAccess(snapshot.context.menus, appCode, routePath).allowed
  React.useEffect(() => {
    if (snapshot && !snapshot.signal.aborted && !microRouteAccess(snapshot.context.menus, appCode, routePath).allowed) {
      void runtimeRef.current?.refresh()
    }
  }, [snapshot, appCode, routePath])
  React.useEffect(() => {
    if (!hasSnapshot || faultRef.current) return
    const entryInfo = entryInfoRef.current
    if (isTrusted && trustedMicroRuntimeDomain.fault) {
      latchTrustedFault(trustedMicroRuntimeDomain.fault)
      return
    }
    const container = containerRef.current
    if (!container) return
    const sequence = ++seqRef.current
    const controller = new AbortController()
    const node = document.createElement("div")
    container.replaceChildren(node)
    const titleSnapshot = document.title
    const previous = isTrusted ? trustedDisposalBarrier : null
    let sandbox: MicroSandbox | null = null
    const session = new MicroLifecycleSession(
      () => { sandbox?.teardown(); node.remove() },
      (callback) => sandbox ? sandbox.run(callback) : callback(),
      isTrusted ? { domain: trustedMicroRuntimeDomain, signal: controller.signal,
        abort: () => controller.abort(), onFault: latchTrustedFault } : undefined,
    )
    const active: ActiveEntry = { entry: entryInfo, runtimeMode: entryInfo.runtimeMode, sandbox: null, session, controller, node }
    activeRef.current = active
    lastPushSigRef.current = null
    setStatus("loading")
    setSupportsUpdate(false)
    setStats(null)
    const stale = () => {
      if (controller.signal.aborted || sequence !== seqRef.current || !!faultRef.current) return true
      const current = snapshotRef.current
      if (!current || current.signal.aborted) return true
      try { assertMicroEntrySnapshot(active.entry, current.context); return false }
      catch { return true }
    }
    const check = () => {
      if (stale()) throw new DOMException("Unmounted", "AbortError")
      const current = snapshotRef.current
      if (!current || current.signal.aborted) throw new DOMException("应用授权上下文不可用", "AbortError")
      assertMicroEntrySnapshot(active.entry, current.context)
      if (!microRouteAccess(current.context.menus, appCode, routeRef.current).allowed) {
        throw new ApiError("当前组织未授权访问此应用菜单", "FORBIDDEN", 403)
      }
    }
    let mountedProps: MicroProps | null = null

    void (async () => {
      if (isTrusted) {
        trustedMicroRuntimeDomain.assertHealthy()
        // Wait outside the domain operation: final unmount also needs that operation queue.
        await previous
        trustedMicroRuntimeDomain.assertHealthy()
      }
      check()
      return session.mount(async () => {
        check()
        assertBrowserCapabilities("host")
        if (!isTrusted) {
          if (entryInfo.entryType !== "HTML") throw new Error("隔离 iframe 应用需要 HTML 入口，请联系平台管理员配置")
          return createIframeLifecycle({ entryUrl: entryInfo.entryUrl, appCode, signal: controller.signal,
            timing: (stage, durationMs) => {
              const value = snapshotRef.current
              if (!value || value.signal.aborted || controller.signal.aborted) return
              const application = appCode === "work-orders" || appCode === "stores" ? appCode : "other"
                performance.clearMeasures(`saas.micro.${stage}`)
              performance.measure(`saas.micro.${stage}`, { start: Math.max(0, performance.now() - durationMs), duration: durationMs })
              window.dispatchEvent(new CustomEvent("saas:micro-timing", { detail: { application, stage, durationMs } }))
              void fetch(`/api/apps/${encodeURIComponent(appCode)}/runtime-events`, {
                method: "POST", credentials: "omit", keepalive: true,
                headers: { "Content-Type": "application/json", Authorization: `Bearer ${value.token}` },
                body: JSON.stringify({ kind: "TIMING", stage, duration_ms: durationMs }), signal: AbortSignal.timeout(3000),
              }).catch(() => {})
            } })
        }
        trustedMicroRuntimeDomain.assertHealthy()
        if (entryInfo.entryType === "HTML") {
          sandbox = new MicroSandbox(appCode)
          active.sandbox = sandbox
          const entry = await loadHtmlEntry(entryInfo.entryUrl, { signal: controller.signal })
          check()
          node.innerHTML = entry.templateProcessed
          await entry.execScripts(sandbox.proxy)
          check()
          const lifecycle = (sandbox.proxy as Record<string, unknown>)[MICRO_APP_GLOBAL] as MicroLifecycle | undefined
          if (!lifecycle || typeof lifecycle.mount !== "function" || typeof lifecycle.unmount !== "function") {
            throw new Error("子应用缺少 mount/unmount 生命周期")
          }
          return lifecycle
        }
        // Code-bundled ESM shares the host's React and UI module instances.
        const mod = (!entryInfo.entryVersionId
          ? await loadHostEsm(entryInfo.entryUrl)
          : await import(/* @vite-ignore */ entryInfo.entryUrl)) as MicroEsmModule
        check()
        const lifecycle = mod.__MICRO_APP__ ??
          (mod.mount && mod.unmount ? { name: entryInfo.name, mount: mod.mount, unmount: mod.unmount, update: mod.update } : null)
        if (!lifecycle) throw new Error("ESM 模块缺少 mount/unmount 生命周期")
        return lifecycle
      }, node, () => {
        const props = propsBuilderRef.current(routeRef.current)
        mountedProps = props
        lastPushSigRef.current = microPropsSignature(props)
        lastDeliveryRef.current = microPropsDelivery(props)
        return props
      })
    })().then(() => {
      if (stale() || (isTrusted && trustedMicroRuntimeDomain.fault)) return
      try { check() } catch { return }
      setSupportsUpdate(session.supportsUpdate)
      setStatus("mounted")
    }).catch((error) => {
      if (isTrusted && isMicroRuntimeFault(error)) { latchTrustedFault(error); return }
      if (stale()) return
      if (error instanceof Error && error.name === "AbortError" &&
          (!mountedProps || mountedProps.signal.aborted)) {
        // 续期可能取消初次握手；用最新有效上下文重建实例。
        setMountNonce((value) => value + 1)
        return
      }
      if (entryInfo.runtimeMode === "TRUSTED" && entryInfo.entryType === "HTML") markEntryFailed(entryInfo.entryUrl)
      reportRuntimeFailure("LOAD_FAILED", "mount")
      runtimeRef.current?.stop()
      setStatus("error")
      setErrorMessage(isTrusted ? "可信应用加载或挂载失败，请重试" : error instanceof Error ? error.message : String(error))
      void disposeEntry(active)
    })

    return () => {
      seqRef.current++
      if (activeRef.current === active) activeRef.current = null
      void disposeEntry(active)
      if (document.title !== titleSnapshot) document.title = titleSnapshot
    }
  }, [hasSnapshot, appCode, entryInfo.runtimeMode, entryInfo.entryType, entryInfo.entryUrl,
    entryInfo.entryVersionId, entryInfo.deploymentVersion, reloadNonce, mountNonce, disposeEntry,
    reportRuntimeFailure, isTrusted, latchTrustedFault])

  React.useEffect(() => {
    if (status !== "mounted" || !hasSnapshot) return
    const active = activeRef.current
    if (!active || active.controller.signal.aborted) return
    // 入口切换的同一 commit 中，旧 mounted 状态不能更新刚建立的新实例。
    if (lastPushSigRef.current === null) return
    let props: MicroProps
    try { props = buildProps(routePath) } catch { return }
    const signature = microPropsSignature(props)
    if (lastPushSigRef.current === signature) return
    const previousDelivery = lastDeliveryRef.current
    lastPushSigRef.current = signature
    lastDeliveryRef.current = microPropsDelivery(props)
    if (!active.session.supportsUpdate) {
      if (!requiresMicroRemount(previousDelivery, props)) return
      setReloadNonce((value) => value + 1)
      return
    }
    void active.session.update(props).catch((error) => {
      if (active.runtimeMode === "TRUSTED" && isMicroRuntimeFault(error)) { latchTrustedFault(error); return }
      if (faultRef.current) return
      if (active.controller.signal.aborted || (props.signal.aborted && error instanceof Error && error.name === "AbortError")) return
      reportRuntimeFailure("LIFECYCLE_FAILED", "update")
      runtimeRef.current?.stop()
      setStatus("error")
      setErrorMessage(active.runtimeMode === "TRUSTED" ? "可信应用运行时更新失败，请重试" : error instanceof Error ? error.message : String(error))
      void disposeEntry(active)
    })
  }, [snapshot, routePath, resolvedTheme, status, buildProps, hasSnapshot, disposeEntry, reportRuntimeFailure, latchTrustedFault])

  /* ---------- 沙箱诊断(v1.1): HTML 通道挂载后 1s 轮询 getStats ---------- */
  React.useEffect(() => {
    if (entryInfo.runtimeMode !== "TRUSTED" || entryInfo.entryType !== "HTML" || status !== "mounted") {
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
  }, [entryInfo.runtimeMode, entryInfo.entryType, status, reloadNonce])

  const fault = runtimeFault ?? (isTrusted ? trustedMicroRuntimeDomain.fault : null)
  const handleReload = () => {
    if (isTrusted && trustedMicroRuntimeDomain.fault) {
      latchTrustedFault(trustedMicroRuntimeDomain.fault)
      return
    }
    setReloadNonce((n) => n + 1)
  }
  const loading = !fault && status === "loading"

  return (
    <div className="space-y-4">
      {/* ---------- 状态条 ---------- */}
      <div
        className="flex flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border bg-card px-4 py-3 shadow-sm"
        data-testid="micro-status-bar"
        data-token-state={tokenState}
        data-recovery-attempt={recoveryAttempt}
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
        {entryInfo.runtimeMode === "IFRAME" ? (
          <span className="inline-flex shrink-0 items-center gap-1 rounded-full border bg-muted px-2 py-0.5 text-[11px] font-medium text-muted-foreground">
            <ShieldQuestion className="size-3" />
            隔离 iframe
          </span>
        ) : entryInfo.entryType === "HTML" ? (
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

        {entryInfo.deploymentVersion && (
          <span className="font-mono text-[11px] text-muted-foreground" title={entryInfo.entryVersionId ?? undefined}>
            v{entryInfo.deploymentVersion}
          </span>
        )}

        {/* 运行时属性更新能力，不代表重新部署前端代码。 */}
        {supportsUpdate && status === "mounted" && (
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="inline-flex shrink-0 items-center gap-1 rounded-full border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 text-[11px] font-medium text-emerald-700 dark:text-emerald-300">
                <Zap className="size-3" />
                运行时同步
              </span>
            </TooltipTrigger>
            <TooltipContent side="bottom" className="max-w-xs text-xs">
              授权上下文续期、子路径或主题变化时只更新运行时属性；不重新部署代码，
              也不重挂页面（计时器、输入等内部状态保留）。
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
                ? `授权上下文获取失败: ${tokenNote ?? "未知原因"}`
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
                disabled={loading || !!fault}
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
      {fault ? (
        <TrustedRuntimeFaultCard fault={fault} hasRuntimeToken={!!snapshot?.token} />
      ) : status === "error" ? (
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

/** A full page reload creates a new trusted execution domain; a component remount cannot do that. */
function TrustedRuntimeFaultCard({ fault, hasRuntimeToken }: { fault: MicroRuntimeFault; hasRuntimeToken: boolean }) {
  const stages = { load: "入口加载", mount: "挂载", update: "更新", unmount: "卸载" } as const
  return (
    <Alert variant="destructive" className="items-start gap-4 py-4" data-testid="trusted-runtime-fault"
      data-runtime-stage={fault.stage} data-has-runtime-token={String(hasRuntimeToken)}>
      <CircleAlert className="mt-0.5 size-5" />
      <div className="min-w-0 flex-1 space-y-3">
        <AlertTitle className="text-sm font-semibold">可信应用运行故障，需要重新加载宿主</AlertTitle>
        <AlertDescription className="space-y-2 text-xs leading-relaxed">
          <p>{stages[fault.stage]}未能安全完成，当前授权与挂载内容已停用。</p>
          <p>本页暂停运行其他可信应用。可以继续使用宿主菜单和隔离 iframe 应用；恢复可信应用需要重新加载整个宿主页面。</p>
        </AlertDescription>
        <Button size="sm" variant="outline" onClick={() => window.location.reload()}>
          <RotateCw className="size-3.5" />
          重新加载宿主
        </Button>
      </div>
    </Alert>
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
        正在加载应用入口与授权上下文...
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
