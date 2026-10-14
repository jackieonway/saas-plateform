import type { MicroLifecycle, MicroNotifyVariant, MicroProps, MicroMenuItem } from "./types"

interface IframeOptions {
  entryUrl: string
  appCode: string
  /** 整个挂载实例的 lifetime；组织切换或移除挂载节点前必须 abort。 */
  signal?: AbortSignal
  handshakeTimeoutMs?: number
  operationTimeoutMs?: number
  /** 宿主保存适配；仅由宿主代码提供，子 frame 无法覆盖。 */
  saveCsv?: (filename: string, content: string) => void
}

type PropsDTO = Omit<MicroProps, "signal" | "onNavigate" | "onNotify">
type Pending = { epoch: number; resolve: () => void; reject: (error: Error) => void; timer: ReturnType<typeof setTimeout> }
const variants = new Set<MicroNotifyVariant>(["default", "success", "error", "warning", "info"])
const abortError = () => new DOMException("Iframe application context cancelled", "AbortError")

function menuDTO(item: MicroMenuItem): MicroMenuItem {
  return { id: item.id, parentId: item.parentId, type: item.type, title: item.title,
    path: item.path, icon: item.icon, sortOrder: item.sortOrder, visible: item.visible,
    ...(item.permission === undefined ? {} : { permission: item.permission }),
    ...(item.children ? { children: item.children.map(menuDTO) } : {}) }
}

/** 明确字段边界，禁止附带平台会话、client secret 或函数进入子 frame。 */
function propsDTO(props: MicroProps): PropsDTO {
  return { contractVersion: props.contractVersion, contextVersion: props.contextVersion,
    contextEpoch: props.contextEpoch, expiresAt: props.expiresAt, tokenExpiresAt: props.tokenExpiresAt,
    appCode: props.appCode, basePath: props.basePath, routePath: props.routePath, token: props.token,
    ...(props.deploymentVersion === undefined ? {} : { deploymentVersion: props.deploymentVersion }),
    ...(props.entryVersionId === undefined ? {} : { entryVersionId: props.entryVersionId }),
    user: { id: props.user.id, username: props.user.username, displayName: props.user.displayName },
    tenant: { id: props.tenant.id, code: props.tenant.code, name: props.tenant.name },
    organization: { id: props.organization.id, name: props.organization.name, path: props.organization.path },
    organizationScopeIds: props.organizationScopeIds === null ? null
      : [...(props.organizationScopeIds ?? [props.organization.id])],
    roles: props.roles.map(({ id, name, code }) => ({ id, name, code })), permissions: [...props.permissions],
    dataGrants: props.dataGrants.map(({ resource, action, scope, subjectUserId, tenantId, organizationIds }) =>
      ({ resource, action, scope, subjectUserId, tenantId, organizationIds: [...organizationIds] })),
    menus: props.menus.map(menuDTO), theme: props.theme }
}

/**
 * HTML 子应用的隔离入口。子应用自己加载 classic iframe-bridge.js 并配置宿主 origin 白名单。
 * sandbox 不包含 allow-same-origin；唯一转移给该 contentWindow 的 MessagePort 是通信能力边界。
 */
export function createIframeLifecycle(options: IframeOptions): MicroLifecycle {
  const { appCode } = options
  let frame: HTMLIFrameElement | null = null
  let container: HTMLElement | null = null
  let port: MessagePort | null = null
  let observer: MutationObserver | null = null
  let current: MicroProps | null = null
  let nonce = ""
  let stopped = false
  let mounted = false
  let ready = false
  let nextRequest = 0
  let expiryTimer: ReturnType<typeof setTimeout> | null = null
  let unbindContext: (() => void) | null = null
  let handshake: { resolve: () => void; reject: (error: Error) => void; timer: ReturnType<typeof setTimeout> } | null = null
  const pending = new Map<number, Pending>()
  let chain: Promise<void> = Promise.resolve()
  let lastDownloadId = 0
  let lastDownloadAt = Number.NEGATIVE_INFINITY
  const downloads = new Map<string, ReturnType<typeof setTimeout>>()
  const clearDownloads = () => {
    for (const [url, timer] of downloads) { clearTimeout(timer); URL.revokeObjectURL(url) }
    downloads.clear()
  }
  const deadline = (props: MicroProps) => Math.min(Date.parse(props.expiresAt), Date.parse(props.tokenExpiresAt))
  const attached = () => !!frame && !!container && container.isConnected && container.contains(frame)
  const usable = (epoch: number) => !stopped && attached() && !!current && current.contextEpoch === epoch &&
    !current.signal.aborted && deadline(current) > Date.now()
  const send = (kind: string, epoch: number, extra: Record<string, unknown> = {}) => {
    port?.postMessage({ kind, nonce, contextEpoch: epoch, ...extra })
  }
  const settle = (id: number, error?: Error) => {
    const task = pending.get(id)
    if (!task) return
    pending.delete(id)
    clearTimeout(task.timer)
    if (error) task.reject(error)
    else task.resolve()
  }
  const clearContext = () => {
    unbindContext?.()
    unbindContext = null
    if (expiryTimer !== null) clearTimeout(expiryTimer)
    expiryTimer = null
  }
  const dispose = (error: Error = abortError()) => {
    if (stopped) return
    stopped = true
    // 卸载不等待 ACK：父节点可已被移除，frame 销毁负责最终回收其执行环境。
    if (port && current) {
      try { send("MICRO_UNMOUNT", current.contextEpoch) } catch { /* already closed */ }
    }
    clearContext()
    clearDownloads()
    current = null
    options.signal?.removeEventListener("abort", onScopeAbort)
    observer?.disconnect()
    observer = null
    if (handshake) { clearTimeout(handshake.timer); handshake.reject(error); handshake = null }
    for (const id of [...pending.keys()]) settle(id, error)
    if (port) { port.onmessage = null; port.onmessageerror = null; port.close() }
    port = null
    if (frame) { frame.onload = null; frame.onerror = null; frame.remove() }
    frame = null
    container = null
  }
  const onScopeAbort = () => dispose()
  options.signal?.addEventListener("abort", onScopeAbort, { once: true })
  if (options.signal?.aborted) dispose()

  const activate = (props: MicroProps) => {
    if (stopped || props.signal.aborted) throw abortError()
    if (props.contractVersion !== "2.0" || props.appCode !== appCode || props.basePath !== `/micro/${appCode}` ||
        !Number.isSafeInteger(props.contextEpoch) || props.contextEpoch <= 0 || !props.contextVersion || !props.token ||
        !Number.isFinite(deadline(props)) || deadline(props) <= Date.now() ||
        (current && props.contextEpoch < current.contextEpoch)) throw new Error("Iframe application authorization context is invalid or expired")
    clearContext()
    current = props
    const cancel = () => {
      if (current !== props || stopped) return
      try { send("MICRO_CONTEXT_ABORT", props.contextEpoch) } catch { /* closed */ }
      for (const [id, task] of pending) if (task.epoch === props.contextEpoch) settle(id, abortError())
      // 续期在已挂载实例上推送新 context；首次握手不能等待已取消的旧 context。
      if (!mounted) dispose()
    }
    props.signal.addEventListener("abort", cancel, { once: true })
    unbindContext = () => props.signal.removeEventListener("abort", cancel)
    expiryTimer = setTimeout(cancel, Math.max(1, deadline(props) - Date.now()))
  }

  const navigate = (value: unknown) => {
    if (!current || typeof value !== "string" || !value || value.length > 2048 ||
        /^[a-z][a-z\d+.-]*:/i.test(value) || /[\\\u0000-\u001f\u007f]/.test(value)) return
    const base = current.basePath
    const path = value.startsWith("/") ? value : `${base}/${value}`
    if (path.startsWith("//")) return
    const url = new URL(path, window.location.origin)
    // URL normalization rejects ../, encoded separators and another application's routes.
    let decoded: string
    try { decoded = decodeURIComponent(url.pathname) } catch { return }
    if (url.origin !== window.location.origin || !(url.pathname === base || url.pathname.startsWith(base + "/")) ||
        /[\\\u0000-\u001f\u007f]/.test(decoded) ||
        decoded.split("/").some((part) => part === "." || part === "..") ||
        !(decoded === base || decoded.startsWith(base + "/"))) return
    current.onNavigate(url.pathname + url.search + url.hash)
  }

  const downloadCsv = (data: Record<string, unknown>) => {
    const id = data.requestId
    if (!Number.isSafeInteger(id) || (id as number) <= 0) return
    const acknowledge = (ok: boolean) => send("MICRO_DOWNLOAD_RESULT", data.contextEpoch as number,
      { requestId: id, ok, ...(ok ? {} : { code: "CSV_DOWNLOAD_DENIED" }) })
    if ((id as number) <= lastDownloadId) { acknowledge(false); return }
    lastDownloadId = id as number
    const { resource, filename, content } = data
    if (!current || typeof resource !== "string" || !resource || resource.length > 100 ||
        typeof filename !== "string" || !filename || filename.length > 128 || !/\.csv$/i.test(filename) ||
        /[\\/:*?"<>|\u0000-\u001f\u007f]/.test(filename) || filename.startsWith(".") ||
        typeof content !== "string" || !content || content.length > 4 * 1024 * 1024 ||
        new TextEncoder().encode(content).byteLength > 4 * 1024 * 1024 ||
        !current.permissions.includes(`${resource}:export`) || !current.dataGrants.some((grant) =>
          grant.resource === resource && grant.action === "export" && grant.subjectUserId === current!.user.id &&
          grant.tenantId === current!.tenant.id && ["SELF", "ORG", "ORG_SUBTREE", "TENANT"].includes(grant.scope)) ||
        performance.now() - lastDownloadAt < 1000) { acknowledge(false); return }
    try {
      if (options.saveCsv) options.saveCsv(filename, content)
      else {
        const url = URL.createObjectURL(new Blob([content], { type: "text/csv;charset=utf-8" }))
        const timer = setTimeout(() => { URL.revokeObjectURL(url); downloads.delete(url) }, 30_000)
        downloads.set(url, timer)
        const link = document.createElement("a")
        link.href = url
        link.download = filename
        link.style.display = "none"
        document.body.appendChild(link)
        try { link.click() } finally { link.remove() }
      }
      lastDownloadAt = performance.now()
      acknowledge(true)
    } catch { acknowledge(false) }
  }

  const receive = (event: MessageEvent) => {
    const data = event.data
    if (!data || typeof data !== "object" || data.nonce !== nonce || !usable(data.contextEpoch)) return
    if (data.kind === "MICRO_READY" && !ready && handshake && data.appCode === appCode && data.contractVersion === "2.0") {
      ready = true
      clearTimeout(handshake.timer)
      handshake.resolve()
      handshake = null
    } else if (data.kind === "MICRO_ACK" || data.kind === "MICRO_ERROR") {
      const task = pending.get(data.requestId)
      if (task?.epoch !== data.contextEpoch) return
      settle(data.requestId, data.kind === "MICRO_ERROR" ? new Error("Iframe application lifecycle failed") : undefined)
    } else if (ready && data.kind === "MICRO_NAVIGATE") {
      navigate(data.path)
    } else if (ready && mounted && data.kind === "MICRO_DOWNLOAD") {
      downloadCsv(data)
    } else if (ready && data.kind === "MICRO_NOTIFY" && typeof data.message === "string" &&
        data.message.length > 0 && data.message.length <= 1000 && variants.has(data.variant)) {
      current!.onNotify(data.message, data.variant)
    }
  }

  const request = (kind: string, props: MicroProps) => new Promise<void>((resolve, reject) => {
    if (!usable(props.contextEpoch) || current !== props || !ready) { reject(abortError()); return }
    const id = ++nextRequest
    const timer = setTimeout(() => {
      const error = new Error(`Iframe application ${kind === "MICRO_MOUNT" ? "mount" : "update"} timed out`)
      settle(id, error)
      dispose(error)
    }, options.operationTimeoutMs ?? 15_000)
    pending.set(id, { epoch: props.contextEpoch, resolve, reject, timer })
    try { send(kind, props.contextEpoch, { requestId: id, props: propsDTO(props) }) }
    catch { const error = new Error("Iframe application props could not be delivered"); settle(id, error); dispose(error) }
  })
  const serialized = (callback: () => Promise<void>) => {
    const task = chain.then(callback)
    chain = task.catch(() => {})
    return task
  }

  return {
    name: appCode,
    mount: (target, props) => serialized(async () => {
      if (frame || mounted) throw new Error("Iframe application is already mounted")
      activate(props)
      let url: URL
      try { url = new URL(options.entryUrl, window.location.href) }
      catch { dispose(); throw new Error("Iframe application entry URL is invalid") }
      if (!/^https?:$/.test(url.protocol) || url.username || url.password ||
          [...url.searchParams.keys()].some((key) => /^(?:token|access_token|session(?:_token)?|client_secret)$/i.test(key))) {
        dispose()
        throw new Error("Iframe application entry URL must be HTTP(S) without credentials")
      }
      nonce = crypto.randomUUID()
      container = target
      frame = document.createElement("iframe")
      frame.title = `${appCode} application`
      frame.setAttribute("sandbox", "allow-scripts allow-forms")
      frame.referrerPolicy = "no-referrer"
      // The mount zone has an automatic height, so height:100% collapsed to the
      // minimum and left desktop dialogs in a 480px viewport. Reserve room for
      // the host navigation while letting the frame follow viewport resizing.
      frame.style.cssText = "display:block;width:100%;min-height:480px;height:max(480px, calc(100dvh - 220px));border:0"
      const channel = new MessageChannel()
      port = channel.port1
      port.onmessage = receive
      port.onmessageerror = () => dispose(new Error("Iframe application sent an invalid protocol message"))
      port.start()
      let transferred = false
      const connected = new Promise<void>((resolve, reject) => {
        handshake = { resolve, reject, timer: setTimeout(() => {
          dispose(new Error("Iframe application handshake timed out; check iframe SDK and allowedHostOrigins"))
        }, options.handshakeTimeoutMs ?? 10_000) }
      })
      frame.onload = () => {
        if (stopped) return
        // 后续导航的文档不能继承旧授权 channel。
        if (transferred) { dispose(new Error("Iframe application navigated away from its connected document")); return }
        if (!usable(props.contextEpoch)) { dispose(); return }
        transferred = true
        try {
          frame!.contentWindow!.postMessage({ kind: "MICRO_CONNECT", nonce, appCode,
            contractVersion: "2.0", contextEpoch: props.contextEpoch }, "*", [channel.port2])
        } catch { channel.port2.close(); dispose(new Error("Iframe application handshake could not be delivered")) }
      }
      frame.onerror = () => dispose(new Error("Iframe application entry could not be loaded"))
      observer = new MutationObserver(() => { if (!attached()) dispose() })
      observer.observe(document.documentElement, { childList: true, subtree: true })
      frame.src = url.href
      target.replaceChildren(frame)
      try {
        await connected
        await request("MICRO_MOUNT", props)
        mounted = true
      } catch (error) {
        if (!transferred) channel.port2.close()
        dispose(error instanceof Error ? error : new Error("Iframe application mount failed"))
        throw error
      }
    }),
    update: (props) => {
      try { activate(props) } catch (error) { return Promise.reject(error) }
      return serialized(async () => {
        if (!mounted || stopped) throw abortError()
        await request("MICRO_UPDATE", props)
      })
    },
    unmount: () => { dispose() },
  }
}
