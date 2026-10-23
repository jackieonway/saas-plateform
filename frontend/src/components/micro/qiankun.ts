import type { FrameworkConfiguration, FrameworkLifeCycles, LoadableApp, MicroApp } from "qiankun"
import type { MicroApplicationHandle, MicroProps } from "./types"
import { createMicroServices } from "./services"

type QiankunProps = { platform: MicroProps; services: ReturnType<typeof createMicroServices> }
type LoadMicroApp = (app: LoadableApp<QiankunProps>, configuration: FrameworkConfiguration,
  lifeCycles: FrameworkLifeCycles<QiankunProps>) => MicroApp

interface QiankunOptions {
  appCode: string
  entryUrl: string
  signal: AbortSignal
  /** Recheck the live entry and route grant at every asynchronous boundary. */
  check: () => void
  loadMicroApp?: LoadMicroApp
  fetch?: typeof fetch
}

interface Scope {
  currentProps: MicroProps | null
  signal: AbortSignal
  checkEntry?: () => void
  fetch?: typeof fetch
  services: ReturnType<typeof createMicroServices>
  stopping: boolean
  executionStarted: boolean
  mountRejected: boolean
  unmountCompleted: boolean
  resourceSignal?: AbortSignal
  resourceLease?: string
}

interface Execution {
  key: string
  name: string
  id: string
  entry: string
  scope: Scope | null
  configuration: FrameworkConfiguration
  hooks: FrameworkLifeCycles<QiankunProps>
  props: QiankunProps
}

const cancelled = () => new DOMException("Micro application context cancelled", "AbortError")
const executions = new Map<string, Execution>()
let activeExecution: Execution | null = null
let frameworkStarted = false
const patchedDocuments = new WeakSet<Document>()
function checkScope(scope: Scope | null): asserts scope is Scope & { currentProps: MicroProps } {
  if (!scope || scope.stopping || scope.signal.aborted || !scope.currentProps || scope.currentProps.signal.aborted ||
      !(Date.parse(scope.currentProps.expiresAt) > Date.now()) || !(Date.parse(scope.currentProps.tokenExpiresAt) > Date.now())) {
    throw cancelled()
  }
  scope.checkEntry?.()
}

function resourceLease(scope: Scope): string {
  checkScope(scope)
  if (scope.resourceSignal !== scope.currentProps.signal) {
    scope.resourceSignal = scope.currentProps.signal
    scope.resourceLease = crypto.randomUUID()
  }
  return scope.resourceLease!
}

/** qiankun's dynamic append loader reads the global start() fetch option. */
const fetchDynamicResource: typeof fetch = (input, init) => {
  const execution = activeExecution
  const scope = execution?.scope ?? null
  checkScope(scope)
  const url = new URL(input instanceof Request ? input.url : String(input), window.location.href)
  if (!execution || url.searchParams.get("__saas_execution") !== execution.id ||
      url.searchParams.get("__saas_lease") !== resourceLease(scope)) throw cancelled()
  const fetchResource = execution.configuration.fetch
  if (typeof fetchResource !== "function") throw cancelled()
  return fetchResource(input, init)
}

/** Override only qiankun's speedy document proxy, preserving host document APIs. */
function patchDynamicResources(execution: Execution, sandbox: typeof window) {
  const document = sandbox.document
  if (patchedDocuments.has(document)) return
  patchedDocuments.add(document)
  const createElement = document.createElement.bind(document)
  document.createElement = ((tag: string, options?: ElementCreationOptions) => {
    const element = createElement(tag, options)
    const property = tag.toLowerCase() === "script" ? "src" : tag.toLowerCase() === "link" ? "href" : null
    if (!property) return element
    const scope = execution.scope
    checkScope(scope)
    const lease = resourceLease(scope)
    let prototype = Object.getPrototypeOf(element), descriptor: PropertyDescriptor | undefined
    while (prototype && !descriptor) {
      descriptor = Object.getOwnPropertyDescriptor(prototype, property)
      prototype = Object.getPrototypeOf(prototype)
    }
    if (!descriptor?.get || !descriptor.set) throw new Error("微应用动态资源接口不可用")
    const { get, set } = descriptor
    Object.defineProperty(element, property, {
      configurable: true,
      get() {
        if (property === "src" && (element as HTMLScriptElement).type.toLowerCase() === "module") {
          throw new Error("qiankun 动态脚本需要 UMD/经典脚本")
        }
        const value = get.call(element) as string
        if (!value) return value
        const url = new URL(value, window.location.href)
        url.searchParams.set("__saas_execution", execution.id)
        url.searchParams.set("__saas_lease", lease)
        return url.href
      },
      set(value) { set.call(element, value) },
    })
    return element
  }) as typeof document.createElement
}

/** Cached qiankun hooks dispatch into the current lease, never their first mount's props. */
function executionFor(appCode: string, entry: URL): Execution {
  const key = JSON.stringify([appCode, entry.href])
  const cached = executions.get(key)
  if (cached && !cached.scope) return cached
  const id = crypto.randomUUID()
  const salted = (value: string) => {
    const url = new URL(value, entry)
    if (/^https?:$/.test(url.protocol)) url.searchParams.set("__saas_execution", id)
    return url.href
  }
  const execution: Execution = {
    key, id, name: `saas-${appCode}-${id}`, entry: salted(entry.href), scope: null,
    // qiankun retains the original app descriptor in its lifecycle cache. These
    // getters keep expired tokens/services out of that cached descriptor.
    props: {
      get platform() { checkScope(execution.scope); return execution.scope.currentProps },
      get services() { checkScope(execution.scope); return execution.scope.services },
    },
    configuration: {
      prefetch: false,
      singular: false,
      sandbox: { experimentalStyleIsolation: true },
      async fetch(input, init) {
        const scope = execution.scope
        checkScope(scope)
        const props = scope.currentProps
        const requestSignal = input instanceof Request ? input.signal : undefined
        const signal = AbortSignal.any([scope.signal, props.signal,
          ...(init?.signal ? [init.signal] : requestSignal ? [requestSignal] : [])])
        const headers = new Headers(init?.headers ?? (input instanceof Request ? input.headers : undefined))
        headers.delete("Authorization")
        headers.delete("Cookie")
        const response = await (scope.fetch ?? fetch)(input, { ...init, headers, signal, credentials: "omit" })
        const checkResponse = () => {
          checkScope(scope)
          if (execution.scope !== scope || scope.currentProps.signal !== props.signal || signal.aborted) throw cancelled()
        }
        checkResponse()
        if (!response.ok) throw new Error(`微应用资源请求失败 (HTTP ${response.status})`)
        // Bodies can finish after fetch() resolves; check the same lease before
        // import-html-entry can parse or evaluate its HTML/script/style content.
        const readText = response.text.bind(response)
        response.text = async () => { const text = await readText(); checkResponse(); return text }
        return response
      },
      getTemplate(html) {
        checkScope(execution.scope)
        const document = new DOMParser().parseFromString(html, "text/html")
        if (document.querySelector('script[type="module"]')) {
          throw new Error("qiankun HTML 入口需要 UMD/经典脚本，请先将 ESM 子应用构建为 qiankun 格式")
        }
        for (const script of document.querySelectorAll<HTMLScriptElement>("script[src]")) {
          script.src = salted(script.getAttribute("src")!)
        }
        for (const link of document.querySelectorAll<HTMLLinkElement>('link[rel="stylesheet"][href]')) {
          link.href = salted(link.getAttribute("href")!)
        }
        return document.documentElement.outerHTML
      },
    },
    hooks: {
      beforeLoad: async (_app, sandbox) => {
        checkScope(execution.scope)
        execution.scope.executionStarted = true
        if (sandbox?.document) patchDynamicResources(execution, sandbox)
      },
      // Cached lifecycles skip beforeLoad, but their mount code still runs.
      beforeMount: async (_app, sandbox) => {
        checkScope(execution.scope)
        execution.scope.executionStarted = true
        // qiankun creates a new document proxy when its sandbox mounts/remounts.
        if (sandbox?.document) patchDynamicResources(execution, sandbox)
      },
      afterMount: async () => { checkScope(execution.scope) },
      afterUnmount: async () => { if (execution.scope) execution.scope.unmountCompleted = true },
    },
  }
  executions.set(key, execution)
  return execution
}

function invalidate(execution: Execution | null) {
  if (execution && executions.get(execution.key) === execution) executions.delete(execution.key)
}

/**
 * qiankun owns the HTML/script/style sandbox; the host still owns authorization,
 * cancellation and lifecycle deadlines. In particular, a timeout cannot kill JS.
 */
export function createQiankunLifecycle(options: QiankunOptions): MicroApplicationHandle {
  const entry = new URL(options.entryUrl, window.location.href)
  const scope: Scope = {
    currentProps: null, signal: options.signal, checkEntry: options.check, fetch: options.fetch,
    services: null!, stopping: false, executionStarted: false, mountRejected: false, unmountCompleted: false,
  }
  scope.services = createMicroServices(() => scope.currentProps)
  let execution: Execution | null = null
  let parcel: MicroApp | null = null
  let disposal: Promise<void> | null = null
  const revoke = () => {
    scope.currentProps = null
    // Release the caller's snapshot/check closure even if a poisoned domain
    // cannot invoke another lifecycle; cached execution records contain no token.
    scope.checkEntry = undefined
    scope.fetch = undefined
    scope.services.dispose()
  }
  const onAbort = () => { scope.stopping = true; revoke() }
  options.signal.addEventListener("abort", onAbort, { once: true })
  if (options.signal.aborted) onAbort()
  const update = async (props: MicroProps) => {
    if (scope.stopping || !parcel?.update) throw cancelled()
    scope.currentProps = props
    checkScope(scope)
    try { await parcel.update({ platform: props, services: scope.services }); checkScope(scope) }
    catch (error) { invalidate(execution); throw error }
  }

  return {
    name: options.appCode,
    async mount(container, props) {
      if (parcel || scope.stopping) throw cancelled()
      scope.currentProps = props
      try {
        checkScope(scope)
        const framework = options.loadMicroApp ? null : await import("qiankun")
        const load = options.loadMicroApp ?? framework!.loadMicroApp
        checkScope(scope)
        execution = executionFor(options.appCode, entry)
        execution.scope = scope
        activeExecution = execution
        if (framework && !frameworkStarted) {
          framework.start({ prefetch: false, singular: false, sandbox: { experimentalStyleIsolation: true },
            fetch: fetchDynamicResource })
          frameworkStarted = true
        }
        parcel = load({ name: execution.name, entry: execution.entry, container, props: execution.props },
          execution.configuration, execution.hooks)
        // Observe all parcel promises even if cancellation made the host stop
        // awaiting mount; single-spa exposes separate rejection branches.
        void parcel.loadPromise.catch(() => {})
        void parcel.bootstrapPromise.catch(() => {})
        void parcel.unmountPromise.catch(() => {})
        await parcel.mountPromise
        checkScope(scope)
      } catch (error) {
        scope.mountRejected = true
        invalidate(execution)
        revoke()
        throw error
      }
    },
    // Preserve the host's remount fallback when update was not exported.
    get update() { return typeof parcel?.update === "function" ? update : undefined },
    unmount() {
      if (disposal) return disposal
      scope.stopping = true
      revoke()
      options.signal.removeEventListener("abort", onAbort)
      disposal = (async () => {
        try {
          if (!parcel) return
          // single-spa rejects repeated unmount() for LOAD_ERROR/SKIP_BROKEN.
          // An entry that executed no code, or successful automatic cleanup
          // after a failed mount, requires no second unmount. Unproven cleanup
          // still rejects to the host's permanent trusted-domain fault latch.
          if (scope.mountRejected && (!scope.executionStarted || scope.unmountCompleted)) return
          await parcel.unmount()
        } catch (error) { invalidate(execution); throw error }
        finally {
          if (execution?.scope === scope) execution.scope = null
          if (activeExecution === execution) activeExecution = null
        }
      })()
      return disposal
    },
  }
}
