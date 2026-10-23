/**
 * 微前端编排容器(/micro/:appCode/*)
 *
 * 职责:
 *  1. 按 appCode 从用户已授权应用列表(profile.applications, GET /api/auth/profile)发现应用；
 *     可执行完整入口统一取实时 context — 不走 GET /api/applications(需 system:application
 *     管理权限, 普通用户 403); 未授权应用在此直接得到「未知微应用」空态。
 *  2. 微应用统一由 qiankun 加载可信 HTML 入口，每次取上下文重新校验服务端配置。
 *  3. 先取得应用 token 与受众限定的完整上下文，再挂载；定时刷新上下文并续期 token。
 *  4. MicroProps 2.0 完整授权内容变化走 update；没有 update 时串行卸载后重挂。
 *  5. 成功挂载仅显示业务内容；加载与错误恢复由宿主提供。
 *  6. 卸载: 撤销授权 → lifecycle.unmount() → 移除容器(StrictMode 双挂载用挂载序号防竞态)
 */

import * as React from "react"
import { Navigate, useLocation, useNavigate, useParams } from "react-router-dom"
import {
  ChevronRight,
  CircleAlert,
  RotateCw,
  ShieldQuestion,
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
import { cn } from "@/lib/utils"
import { reconcileSessionSync, useAuthStore } from "@/stores/auth"
import { useTheme } from "@/components/theme-provider"
import { api, ApiError } from "@/lib/api"
import { microRouteAccess } from "@/lib/micro-route-permissions"
import { normalizedRoutePath } from "@/lib/route-permissions"
import type { OAuthSessionTokenResp } from "@/api/types"
import { MicroContextRuntime, MicroLifecycleSession, isMicroRuntimeFault, microPropsSignature, microPropsDelivery, requiresMicroRemount,
  trustedMicroRuntimeDomain } from "./runtime"
import type { MicroContextSnapshot, MicroRuntimeFault } from "./runtime"
import { EmptyState } from "@/components/states"
import { createQiankunLifecycle } from "./qiankun"
import { assertQiankunCapabilities } from "./capabilities"
import { resolveMicroEntry, microEntrySignature, assertMicroEntrySnapshot } from "./entry"
import { SDK_VERSION, MICRO_PROPS_CONTRACT_VERSION } from "../../../vendor/micro-app-sdk/protocol.mjs"
import type {
  ApplicationEntryInfo,
  AppAuthorizationContext,
  MicroNotifyVariant,
  MicroProps,
  MicroStatus,
} from "./types"

/* ============================ 外层: 入口配置解析 ============================ */

export default function MicroAppContainer() {
  const { appCode = "" } = useParams()
  const { pathname, search = "", hash = "" } = useLocation()
  const normalizedPath = normalizedRoutePath(pathname)
  const basePath = `/micro/${appCode.toLowerCase()}`
  const activeRoute = !!appCode && (normalizedPath === basePath || normalizedPath.startsWith(`${basePath}/`))
  const profile = useAuthStore((s) => s.profile)
  const sessionToken = useAuthStore((s) => s.token)
  const contextEpoch = useAuthStore((s) => s.contextEpoch)
  const switching = useAuthStore((s) => s.contextSwitching)
  const selectedApplicationId = useAuthStore((s) => s.currentApplicationId)
  const navigate = useNavigate()

  const app = React.useMemo(
    () => profile?.applications.find((a) => a.code.toLowerCase() === appCode.toLowerCase()) ?? null,
    [profile?.applications, appCode]
  )

  React.useEffect(() => {
    if (activeRoute && profile && !switching && !app) {
      toast.error("当前组织无法访问此应用，已退出应用")
      navigate("/", { replace: true })
    }
  }, [activeRoute, app, profile, switching, navigate])

  if (!activeRoute) return null

  // React Router decodes route parameters and matches case-insensitively. Use
  // the registered spelling before the layout selects the route's application.
  const canonicalPath = app ? pathname.replace(/^\/[^/]+\/[^/]+/, `/micro/${app.code}`) : pathname
  if (canonicalPath !== pathname) return <Navigate to={`${canonicalPath}${search}${hash}`} replace />

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

  if (app.entryType !== "HTML" || app.runtimeMode !== "TRUSTED") {
    return <EmptyState className="py-20" title="应用需要重新配置"
      description="平台已统一使用 qiankun，请管理员将此应用重新配置为 HTML 入口与可信运行模式" />
  }
  const entryUrl = (app.entryUrl ?? "").trim()

  const entryInfo: ApplicationEntryInfo = {
    id: app.id,
    code: app.code,
    name: app.name,
    icon: app.icon || "LayoutGrid",
    entryType: "HTML",
    entryUrl,
    runtimeMode: "TRUSTED",
    deploymentVersion: app.deploymentVersion,
    entryVersionId: app.entryVersionId,
  }

  // 组织或应用改变时重新取得 token；版本变化只替换挂载实例。
  return <MicroMountZone key={`${contextEpoch}:${app.code}:${entryInfo.runtimeMode}`} entryInfo={entryInfo} />
}

/* ============================ 内层: 挂载编排 ============================ */

interface ActiveEntry {
  entry: ApplicationEntryInfo
  session: MicroLifecycleSession
  controller: AbortController
  node: HTMLElement
  contextSignal?: AbortSignal
  propsSignal?: AbortSignal
}

// 同页应用必须先完成最终卸载，才能开始下一个 qiankun 生命周期。
let trustedDisposalBarrier: Promise<void> = Promise.resolve()

function MicroMountZone({ entryInfo: discovery }: { entryInfo: ApplicationEntryInfo }) {
  const navigate = useNavigate()
  const navigateRef = React.useRef(navigate)
  navigateRef.current = navigate
  const { pathname, search = "", hash = "" } = useLocation()
  const routePath = `${pathname}${search}${hash}`
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

  /* ---------- 授权上下文与生命周期 ---------- */
  const containerRef = React.useRef<HTMLDivElement>(null)
  const seqRef = React.useRef(0)
  const activeRef = React.useRef<ActiveEntry | null>(null)
  const runtimeRef = React.useRef<MicroContextRuntime | null>(null)
  const lastPushSigRef = React.useRef<string | null>(null)
  const lastDeliveryRef = React.useRef<ReturnType<typeof microPropsDelivery> | null>(null)
  const aliveRef = React.useRef(true)
  const faultRef = React.useRef<MicroRuntimeFault | null>(trustedMicroRuntimeDomain.fault)
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
  // Menu grants authorize the pathname. Query and fragment are application
  // state, delivered intact without changing the route's permission identity.
  const pathnameRef = React.useRef(pathname)
  pathnameRef.current = pathname
  const [status, setStatus] = React.useState<MicroStatus>(faultRef.current ? "error" : "loading")
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null)
  const [reloadNonce, setReloadNonce] = React.useState(0)
  const [mountNonce, setMountNonce] = React.useState(0)

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
    // Revoke interaction immediately, but keep qiankun's container available
    // until its unmount lifecycle and sandbox cleanup have finished.
    entry.node.hidden = true
    entry.node.inert = true
    // Session faults carry fixed diagnostics; never log arbitrary lifecycle error objects.
    const disposal = entry.session.stop().catch(() => {}).finally(() => entry.node.remove())
    trustedDisposalBarrier = Promise.all([trustedDisposalBarrier, disposal]).then(() => {})
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
  }, [disposeEntry])

  React.useEffect(() => {
    const unsubscribe = trustedMicroRuntimeDomain.subscribe(latchTrustedFault)
    if (trustedMicroRuntimeDomain.fault) latchTrustedFault(trustedMicroRuntimeDomain.fault)
    return unsubscribe
  }, [latchTrustedFault])

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
    if (!microRouteAccess(context.menus, appCode, pathnameRef.current).allowed) {
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
    if (trustedMicroRuntimeDomain.fault) {
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
    setErrorMessage(null)
    try { assertQiankunCapabilities() }
    catch (error) {
      const message = error instanceof Error ? error.message : "Required browser capabilities are unavailable"
      setStatus("error")
      setErrorMessage(message)
      return
    }
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
        resolveMicroEntry(context, discoveryRef.current)
        if (!microRouteAccess(context.menus, appCode, pathnameRef.current).allowed) {
          throw new ApiError("当前组织未授权访问此应用菜单", "FORBIDDEN", 403)
        }
        return context
      },
      onChange: (value) => {
        if (faultRef.current || trustedMicroRuntimeDomain.fault) {
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
        }
        setSnapshot(value)
        if (!value) setStatus("loading")
      },
      onRecovery: (state) => {
        if (faultRef.current || trustedMicroRuntimeDomain.fault) return
        if (state) setStatus("loading")
      },
      onError: (error) => {
        if (faultRef.current || trustedMicroRuntimeDomain.fault) return
        const message = error instanceof Error ? error.message : "应用授权上下文获取失败"
        setStatus("error")
        setErrorMessage(message)
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
    profile?.organization.id, discovery.id, latchTrustedFault, disposeEntry])

  React.useEffect(() => {
    if (observedProfileEntry.current === profileEntrySignature) return
    observedProfileEntry.current = profileEntrySignature
    if (!faultRef.current && !trustedMicroRuntimeDomain.fault) void runtimeRef.current?.refresh()
  }, [profileEntrySignature])

  const hasSnapshot = !faultRef.current && !!snapshot && !snapshot.signal.aborted &&
    Date.parse(snapshot.context.expiresAt) > Date.now() && Date.parse(snapshot.tokenExpiresAt) > Date.now() &&
    microRouteAccess(snapshot.context.menus, appCode, pathname).allowed
  React.useEffect(() => {
    if (snapshot && !snapshot.signal.aborted && !microRouteAccess(snapshot.context.menus, appCode, pathname).allowed) {
      void runtimeRef.current?.refresh()
    }
  }, [snapshot, appCode, pathname])
  React.useEffect(() => {
    if (!hasSnapshot || faultRef.current) return
    const entryInfo = entryInfoRef.current
    if (trustedMicroRuntimeDomain.fault) {
      latchTrustedFault(trustedMicroRuntimeDomain.fault)
      return
    }
    const container = containerRef.current
    if (!container) return
    const sequence = ++seqRef.current
    const controller = new AbortController()
    const node = document.createElement("div")
    const titleSnapshot = document.title
    const previous = trustedDisposalBarrier
    const session = new MicroLifecycleSession(
      () => { node.remove() },
      undefined,
      { domain: trustedMicroRuntimeDomain, signal: controller.signal,
        abort: () => controller.abort(), onFault: latchTrustedFault },
    )
    const active: ActiveEntry = { entry: entryInfo, session, controller, node }
    activeRef.current = active
    lastPushSigRef.current = null
    setStatus("loading")
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
      if (!microRouteAccess(current.context.menus, appCode, pathnameRef.current).allowed) {
        throw new ApiError("当前组织未授权访问此应用菜单", "FORBIDDEN", 403)
      }
    }
    let mountedProps: MicroProps | null = null

    void (async () => {
      trustedMicroRuntimeDomain.assertHealthy()
      // Wait outside the domain operation: final unmount also needs that operation queue.
      await previous
      trustedMicroRuntimeDomain.assertHealthy()
      check()
      // A new trusted application must not detach the previous container while
      // that application's asynchronous unmount is still using it.
      container.replaceChildren(node)
      return session.mount(async () => {
        check()
        assertQiankunCapabilities()
        trustedMicroRuntimeDomain.assertHealthy()
        return createQiankunLifecycle({ entryUrl: entryInfo.entryUrl, appCode,
          signal: controller.signal, check })
      }, node, () => {
        const props = propsBuilderRef.current(routeRef.current)
        mountedProps = props
        lastPushSigRef.current = microPropsSignature(props)
        lastDeliveryRef.current = microPropsDelivery(props)
        return props
      })
    })().then(() => {
      if (stale() || trustedMicroRuntimeDomain.fault) return
      try { check() } catch { return }
      setStatus("mounted")
    }).catch((error) => {
      if (isMicroRuntimeFault(error)) { latchTrustedFault(error); return }
      if (stale()) return
      if (error instanceof Error && error.name === "AbortError" &&
          (!mountedProps || mountedProps.signal.aborted)) {
        // 续期可能取消初次加载；用最新有效上下文重建实例。
        setMountNonce((value) => value + 1)
        return
      }
      reportRuntimeFailure("LOAD_FAILED", "mount")
      runtimeRef.current?.stop()
      setStatus("error")
      setErrorMessage("可信应用加载或挂载失败，请重试")
      void disposeEntry(active)
    })

    return () => {
      seqRef.current = sequence + 1
      if (activeRef.current === active) activeRef.current = null
      void disposeEntry(active)
      if (document.title !== titleSnapshot) document.title = titleSnapshot
    }
  }, [hasSnapshot, appCode, entryInfo.runtimeMode, entryInfo.entryType, entryInfo.entryUrl,
    entryInfo.entryVersionId, entryInfo.deploymentVersion, reloadNonce, mountNonce, disposeEntry,
    reportRuntimeFailure, latchTrustedFault])

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
      if (isMicroRuntimeFault(error)) { latchTrustedFault(error); return }
      if (faultRef.current) return
      if (active.controller.signal.aborted || (props.signal.aborted && error instanceof Error && error.name === "AbortError")) return
      reportRuntimeFailure("LIFECYCLE_FAILED", "update")
      runtimeRef.current?.stop()
      setStatus("error")
      setErrorMessage("可信应用运行时更新失败，请重试")
      void disposeEntry(active)
    })
  }, [snapshot, routePath, resolvedTheme, status, buildProps, hasSnapshot, disposeEntry, reportRuntimeFailure, latchTrustedFault])

  const fault = runtimeFault ?? trustedMicroRuntimeDomain.fault
  const handleReload = () => {
    if (trustedMicroRuntimeDomain.fault) {
      latchTrustedFault(trustedMicroRuntimeDomain.fault)
      return
    }
    setReloadNonce((n) => n + 1)
  }
  const loading = !fault && status === "loading"

  return <MicroMountContent entryName={entryInfo.name} status={status} loading={loading}
    fault={fault} hasRuntimeToken={!!snapshot?.token} errorMessage={errorMessage}
    containerRef={containerRef} onRetry={handleReload} />
}

/** Business content fills the host viewport; recovery controls appear only on failure. */
function MicroMountContent({ entryName, status, loading, fault, hasRuntimeToken, errorMessage, containerRef, onRetry }: {
  entryName: string
  status: MicroStatus
  loading: boolean
  fault: MicroRuntimeFault | null
  hasRuntimeToken: boolean
  errorMessage: string | null
  containerRef: React.RefObject<HTMLDivElement | null>
  onRetry: () => void
}) {
  if (fault) return <div className="p-4 md:p-6"><TrustedRuntimeFaultCard fault={fault} hasRuntimeToken={hasRuntimeToken} /></div>
  if (status === "error") return <div className="p-4 md:p-6"><MountErrorCard message={errorMessage} onRetry={onRetry} /></div>
  return (
    <div className="relative min-h-[max(480px,calc(100dvh-3.5rem))] w-full bg-card">
      {/* React owns the overlay; the runtime exclusively owns the empty mount point. */}
      {loading && <MountAreaSkeleton />}
      <div ref={containerRef} className="micro-mount-zone min-h-[max(480px,calc(100dvh-3.5rem))] w-full"
        aria-label={`${entryName} 挂载区`} aria-busy={loading} />
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
          <p>本页暂停运行微应用。可以继续使用平台菜单；恢复微应用需要重新加载整个宿主页面。</p>
        </AlertDescription>
        <Button size="sm" variant="outline" onClick={() => window.location.reload()}>
          <RotateCw className="size-3.5" />
          重新加载宿主
        </Button>
      </div>
    </Alert>
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
