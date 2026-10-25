import type { AppAuthorizationContext, MicroApplicationHandle, MicroProps } from "./types"
import { CONTEXT_CONTRACT_VERSION } from "../../../vendor/micro-app-sdk/protocol.mjs"
import { authorizationContextSignature } from "../../../vendor/micro-app-sdk/index.mjs"

export interface MicroContextSnapshot {
  token: string
  tokenExpiresAt: string
  context: AppAuthorizationContext
  contextEpoch: number
  signal: AbortSignal
}

interface RuntimeOptions {
  appCode: string
  issue: (signal: AbortSignal) => Promise<{ accessToken: string; expiresIn: number; audience: string }>
  context: (token: string, signal: AbortSignal) => Promise<AppAuthorizationContext>
  onChange: (value: MicroContextSnapshot | null) => void
  onError: (error: unknown) => void
  onRecovery?: (state: { attempt: number; maximum: number; delayMs: number } | null) => void
  now?: () => number
  setTimer?: (callback: () => void, delay: number) => ReturnType<typeof setTimeout>
  clearTimer?: (timer: ReturnType<typeof setTimeout>) => void
}

let deliveredEpoch = 0
const RETRY_DELAYS_MS = [500, 1000, 2000] as const
const ATTEMPT_TIMEOUT_MS = 5000
const RECOVERY_BUDGET_MS = 20_000

class LocalExpiredLeaseError extends Error {
  readonly code = "LOCAL_CONTEXT_LEASE_EXPIRED"
  constructor() { super("应用授权上下文已过期，请重新获取授权") }
}

function retryableContextFailure(error: unknown): boolean {
  if (error instanceof LocalExpiredLeaseError) return true
  if (!error || typeof error !== "object") return false
  const failure = error as { status?: unknown; code?: unknown }
  return failure.status === 503 || (failure.status === 0 && failure.code === "NETWORK_ERROR")
}

function contextAuthorizationKey(context: AppAuthorizationContext, token: string, tokenExpiresAt: string): string | null {
  const knownContext = new Set(["contractVersion", "contextVersion", "expiresAt", "appCode", "application",
    "user", "tenant", "organization", "organizationScopeIds", "roles", "permissions", "dataGrants", "menus"])
  const knownApplication = new Set(["id", "name", "runtimeMode", "entryType", "entryUrl", "deploymentVersion", "entryVersionId"])
  if (!context.application || Object.keys(context).some(key => !knownContext.has(key)) ||
      Object.keys(context.application).some(key => !knownApplication.has(key))) return null
  const key = authorizationContextSignature({ contractVersion: context.contractVersion, appCode: context.appCode,
    contextEpoch: 0, contextVersion: context.contextVersion, token, expiresAt: context.expiresAt, tokenExpiresAt,
    deploymentVersion: context.application.deploymentVersion, entryVersionId: context.application.entryVersionId,
    user: context.user, tenant: context.tenant, organization: context.organization,
    organizationScopeIds: context.organizationScopeIds, roles: context.roles, permissions: context.permissions,
    dataGrants: context.dataGrants, menus: context.menus })
  return key === null ? null : JSON.stringify([key, context.application.id, context.application.runtimeMode,
    context.application.entryType, context.application.entryUrl])
}

/** 每个应用/平台会话独占一条管线，旧请求取消后仍通过序号拒绝迟到结果。 */
export class MicroContextRuntime {
  private options: RuntimeOptions
  private stopped = false
  private sequence = 0
  private request: AbortController | null = null
  private lifetime: AbortController | null = null
  private lease: { token: string; expires: number; renewAt: number } | null = null
  private refreshTimer: ReturnType<typeof setTimeout> | null = null
  private expiryTimer: ReturnType<typeof setTimeout> | null = null
  private attemptTimer: ReturnType<typeof setTimeout> | null = null
  private snapshot: MicroContextSnapshot | null = null
  private authorizationKey: string | null = null
  private recoveryAttempts = 0
  private episodeStartedAt: number | null = null

  constructor(options: RuntimeOptions) { this.options = options }
  private now() { return (this.options.now ?? Date.now)() }
  private schedule(callback: () => void, delay: number) {
    return (this.options.setTimer ?? setTimeout)(callback, Math.max(1, delay))
  }
  private clear(timer: ReturnType<typeof setTimeout> | null) {
    if (timer !== null) (this.options.clearTimer ?? clearTimeout)(timer)
  }

  async refresh(): Promise<void> {
    if (this.stopped) return
    const sequence = ++this.sequence
    this.clear(this.refreshTimer)
    this.refreshTimer = null
    this.clear(this.attemptTimer)
    this.request?.abort()
    const request = new AbortController()
    this.request = request
    const current = () => !this.stopped && sequence === this.sequence && !request.signal.aborted
    const sameAttempt = () => !this.stopped && sequence === this.sequence
    this.episodeStartedAt ??= this.now()
    let timedOut = false
    let detachAttempt = () => {}
    try {
      const work = async () => {
        let lease = this.lease
        if (!lease || lease.renewAt <= this.now()) {
          const issuedAt = this.now()
          const issued = await this.options.issue(request.signal)
          if (!current()) return null
          if (!issued || typeof issued.accessToken !== "string" || !issued.accessToken ||
              issued.audience !== this.options.appCode || !Number.isFinite(issued.expiresIn) || issued.expiresIn <= 0 ||
              !Number.isFinite(issuedAt + issued.expiresIn * 1000)) throw new Error("应用令牌响应无效")
          const ttl = issued.expiresIn * 1000
          lease = { token: issued.accessToken, expires: issuedAt + ttl,
            renewAt: issuedAt + ttl - Math.min(30_000, ttl / 5) }
        }
        const context = await this.options.context(lease.token, request.signal)
        if (!current()) return null
        const expires = context && Math.min(Date.parse(context.expiresAt), lease.expires)
        if (!context || typeof context !== "object" || Array.isArray(context) ||
            context.contractVersion !== CONTEXT_CONTRACT_VERSION || context.appCode !== this.options.appCode ||
            typeof context.contextVersion !== "string" || !context.contextVersion ||
            typeof context.expiresAt !== "string" || !Number.isFinite(expires)) {
          throw new Error("应用授权上下文已过期或与目标应用不一致")
        }
        if (expires <= this.now()) throw new LocalExpiredLeaseError()
        return { lease, context, expires }
      }
      const result = await new Promise<Awaited<ReturnType<typeof work>>>((resolve, reject) => {
        const onAbort = () => reject(new DOMException("Authorization request cancelled", "AbortError"))
        request.signal.addEventListener("abort", onAbort, { once: true })
        detachAttempt = () => request.signal.removeEventListener("abort", onAbort)
        this.attemptTimer = this.schedule(() => {
          timedOut = true
          reject(Object.assign(new Error("应用授权服务暂时不可用"), { code: "NETWORK_ERROR", status: 0 }))
          request.abort()
        }, Math.min(ATTEMPT_TIMEOUT_MS, Math.max(1, RECOVERY_BUDGET_MS - (this.now() - this.episodeStartedAt!))))
        // The original promise is observed even when transport ignores cancellation.
        work().then(resolve, reject)
      })
      if (!current() || !result) return
      const { lease, context, expires } = result
      if (expires <= this.now()) throw new LocalExpiredLeaseError()
      this.clear(this.attemptTimer)
      this.attemptTimer = null
      this.lease = lease
      const tokenExpiresAt = new Date(lease.expires).toISOString()
      const key = contextAuthorizationKey(context, lease.token, tokenExpiresAt)
      const reusable = key !== null && key === this.authorizationKey && this.snapshot && !this.snapshot.signal.aborted
      if (!reusable) this.lifetime?.abort()
      this.clear(this.expiryTimer)
      const lifetime = reusable ? this.lifetime! : new AbortController()
      this.lifetime = lifetime
      const snapshot = { token: lease.token, tokenExpiresAt, context,
        contextEpoch: reusable ? this.snapshot!.contextEpoch : ++deliveredEpoch, signal: lifetime.signal }
      this.snapshot = snapshot
      this.authorizationKey = key
      this.recoveryAttempts = 0
      this.episodeStartedAt = null
      this.options.onRecovery?.(null)
      if (!current()) return
      this.options.onChange(snapshot)
      if (!current()) { lifetime.abort(); return }
      this.expiryTimer = this.schedule(() => {
        lifetime.abort()
        if (!this.stopped && this.lifetime === lifetime) {
          this.snapshot = null
          this.authorizationKey = null
          this.options.onChange(null)
        }
      }, expires - this.now())
      this.refreshTimer = this.schedule(() => { void this.refresh() },
        Math.min(20_000, Math.max(100, (expires - this.now()) * 0.8),
          Math.max(100, lease.renewAt - this.now())))
    } catch (error) {
      if (!sameAttempt() || (request.signal.aborted && !timedOut)) return
      this.clear(this.attemptTimer)
      this.attemptTimer = null
      request.abort()
      this.lifetime?.abort()
      this.clear(this.expiryTimer)
      this.expiryTimer = null
      this.snapshot = null
      this.authorizationKey = null
      this.lease = null
      this.options.onChange(null)
      if (!sameAttempt()) return
      const delay = RETRY_DELAYS_MS[this.recoveryAttempts]
      if (retryableContextFailure(error) && delay !== undefined &&
          this.now() - this.episodeStartedAt! + delay < RECOVERY_BUDGET_MS) {
        this.recoveryAttempts++
        this.options.onRecovery?.({ attempt: this.recoveryAttempts, maximum: RETRY_DELAYS_MS.length, delayMs: delay })
        if (sameAttempt()) this.refreshTimer = this.schedule(() => { void this.refresh() }, delay)
        return
      }
      this.stop()
      this.options.onRecovery?.(null)
      this.options.onError(error)
    } finally {
      detachAttempt()
      if (sameAttempt()) {
        this.clear(this.attemptTimer)
        this.attemptTimer = null
      }
    }
  }

  stop(): void {
    this.stopped = true
    this.sequence++
    this.request?.abort()
    this.lifetime?.abort()
    this.clear(this.refreshTimer)
    this.clear(this.expiryTimer)
    this.clear(this.attemptTimer)
    this.lease = null
    this.snapshot = null
    this.authorizationKey = null
  }
}

export type MicroLifecycleStage = "load" | "mount" | "update" | "unmount"
export const TRUSTED_LIFECYCLE_TIMEOUT_MS = 10_000
const FAULT_BRAND = Symbol.for("saas.micro.runtime-fault.v1")
const DOMAIN_BRAND = Symbol.for("saas.micro.trusted-runtime-domain.brand.v1")
const PAGE_DOMAIN_KEY = Symbol.for("saas.micro.trusted-runtime-domain.v1")

/** A timeout diagnoses an unproven stop; it cannot terminate trusted JavaScript. */
export class MicroRuntimeFault extends Error {
  readonly code = "TRUSTED_RUNTIME_FAULT"
  readonly stage: MicroLifecycleStage
  readonly reason: "TIMEOUT" | "OPERATION_FAILED"

  constructor(stage: MicroLifecycleStage, reason: "TIMEOUT" | "OPERATION_FAILED") {
    super(reason === "TIMEOUT"
      ? `Trusted application ${stage} timed out; reload the host to recover`
      : `Trusted application ${stage} could not be safely cleaned up; reload the host to recover`)
    this.name = "MicroRuntimeFault"
    this.stage = stage
    this.reason = reason
    Object.defineProperty(this, FAULT_BRAND, { value: true })
  }
}

/** Same-page module replacement must recognize errors created by the prior module identity. */
export function isMicroRuntimeFault(value: unknown): value is MicroRuntimeFault {
  try {
    if (!(value instanceof Error) || Object.getOwnPropertyDescriptor(value, FAULT_BRAND)?.value !== true) return false
    const fault = value as MicroRuntimeFault
    return fault.code === "TRUSTED_RUNTIME_FAULT" &&
      ["load", "mount", "update", "unmount"].includes(fault.stage) &&
      (fault.reason === "TIMEOUT" || fault.reason === "OPERATION_FAILED")
  } catch { return false }
}

/** One page owns one trusted execution domain; only a whole-page reload clears its fault. */
export class TrustedMicroRuntimeDomain {
  private latched: MicroRuntimeFault | null = null
  private chain: Promise<void> = Promise.resolve()
  private controller = new AbortController()
  private listeners = new Set<(fault: MicroRuntimeFault) => void>()

  constructor() { Object.defineProperty(this, DOMAIN_BRAND, { value: true }) }

  get fault() { return this.latched }
  get signal() { return this.controller.signal }
  assertHealthy(): void { if (this.latched) throw this.latched }

  subscribe(callback: (fault: MicroRuntimeFault) => void): () => void {
    this.listeners.add(callback)
    if (this.latched) { try { callback(this.latched) } catch { /* Diagnostics must not unlock the domain. */ } }
    return () => { this.listeners.delete(callback) }
  }

  poison(fault: MicroRuntimeFault): MicroRuntimeFault {
    if (!this.latched) {
      this.latched = fault
      this.controller.abort()
      for (const callback of [...this.listeners]) {
        try { callback(fault) } catch { /* Keep cancellation and other subscribers observable. */ }
      }
    }
    return this.latched
  }

  run<T>(operation: () => Promise<T>): Promise<T> {
    const task = this.chain.then(() => { this.assertHealthy(); return operation() })
    this.chain = task.then(() => {}, () => {})
    return task
  }
}

function pageTrustedDomain(): TrustedMicroRuntimeDomain {
  const page = globalThis as unknown as Record<symbol, unknown>
  if (Object.hasOwn(page, PAGE_DOMAIN_KEY)) {
    const existing = page[PAGE_DOMAIN_KEY] as TrustedMicroRuntimeDomain | undefined
    if (!existing || Object.getOwnPropertyDescriptor(existing, DOMAIN_BRAND)?.value !== true ||
        typeof existing.run !== "function" || typeof existing.assertHealthy !== "function" ||
        typeof existing.subscribe !== "function" || typeof existing.poison !== "function") {
      // An occupied but invalid slot cannot be replaced with a healthy domain.
      throw new MicroRuntimeFault("load", "OPERATION_FAILED")
    }
    return existing
  }
  const domain = new TrustedMicroRuntimeDomain()
  // Preserve the whole active queue, listeners and fault across Vite HMR or
  // duplicate module URLs in this page. A fresh page realm owns a fresh slot.
  Object.defineProperty(page, PAGE_DOMAIN_KEY, { value: domain })
  return domain
}

export const trustedMicroRuntimeDomain = pageTrustedDomain()

export interface MicroLifecycleSessionOptions {
  domain?: TrustedMicroRuntimeDomain
  deadlinesMs?: Partial<Record<MicroLifecycleStage, number>>
  setTimer?: (callback: () => void, delay: number) => ReturnType<typeof setTimeout>
  clearTimer?: (timer: ReturnType<typeof setTimeout>) => void
  signal?: AbortSignal
  /** The host cancels its token/context runtime as well as the entry scope. */
  abort?: () => void
  onFault?: (fault: MicroRuntimeFault) => void
}

/** Normal operations remain serial; a trusted timeout permanently prevents domain reuse. */
export class MicroLifecycleSession {
  private chain: Promise<void> = Promise.resolve()
  private lifecycle: MicroApplicationHandle | null = null
  private mounted = false
  private stopped = false
  private disposal: Promise<void> | null = null
  private cleanup: () => void
  private run: <T>(callback: () => T) => T
  private options: MicroLifecycleSessionOptions
  private controller = new AbortController()
  private cleaned = false
  private reportedFault = false
  private unbindDomain: (() => void) | null = null
  private unbindScope: (() => void) | null = null
  private propsParentSignal: AbortSignal | null = null
  private propsCombinedSignal: AbortSignal | null = null

  constructor(cleanup: () => void, run: <T>(callback: () => T) => T = (callback) => callback(),
    options: MicroLifecycleSessionOptions = {}) {
    this.cleanup = cleanup
    this.run = run
    this.options = options
    for (const deadline of Object.values(options.deadlinesMs ?? {})) {
      if (!Number.isSafeInteger(deadline) || deadline <= 0) throw new RangeError("Lifecycle deadlines must be positive integers")
    }
    this.unbindDomain = options.domain?.subscribe((fault) => this.onDomainFault(fault)) ?? null
    if (options.signal) {
      const stop = () => { void this.stop().catch(() => {}) }
      options.signal.addEventListener("abort", stop, { once: true })
      this.unbindScope = () => options.signal?.removeEventListener("abort", stop)
      if (options.signal.aborted) stop()
    }
  }
  get signal() { return this.controller.signal }
  get supportsUpdate() { return typeof this.lifecycle?.update === "function" }

  private onDomainFault(fault: MicroRuntimeFault): void {
    this.stopped = true
    this.controller.abort()
    if (!this.reportedFault) {
      this.reportedFault = true
      try { this.options.abort?.() } catch { /* Cancellation callbacks cannot change a latched fault. */ }
      try { this.options.onFault?.(fault) } catch { /* The fault remains available on the domain. */ }
    }
    void this.stop().catch(() => {})
  }

  private enqueue(operation: () => Promise<void>): Promise<void> {
    const previous = this.chain
    const work = async () => { await previous; return operation() }
    // Reserve the domain slot at method call time. Waiting for the local chain
    // before enqueueing could place an old unmount after the next app's load.
    const task = this.options.domain ? this.options.domain.run(work) : work()
    this.chain = task.catch(() => {})
    return task
  }

  private stage<T>(stage: MicroLifecycleStage, operation: () => T | Promise<T>): Promise<T> {
    const domain = this.options.domain
    const invoke = () => {
      domain?.assertHealthy()
      if (stage !== "unmount" && (this.stopped || this.controller.signal.aborted)) {
        throw new DOMException("Unmounted", "AbortError")
      }
      return operation()
    }
    if (!domain) return Promise.resolve().then(invoke)
    domain.assertHealthy()
    return new Promise<T>((resolve, reject) => {
      let settled = false
      const timer = (this.options.setTimer ?? setTimeout)(() => {
        if (settled) return
        settled = true
        reject(domain.poison(new MicroRuntimeFault(stage, "TIMEOUT")))
      }, this.options.deadlinesMs?.[stage] ?? TRUSTED_LIFECYCLE_TIMEOUT_MS)
      const clear = () => { (this.options.clearTimer ?? clearTimeout)(timer) }
      // Both handlers remain attached after timeout. A late resolution cannot
      // publish a lifecycle, and a late rejection never becomes unhandled.
      Promise.resolve().then(invoke).then((value) => {
        if (settled) return
        settled = true
        clear()
        if (domain.fault) reject(domain.fault)
        else resolve(value)
      }, (error: unknown) => {
        if (settled) return
        settled = true
        clear()
        reject(domain.fault ?? error)
      })
      // stop()/scope abort deliberately do not clear this deadline. Aborting
      // a signal cannot prove that trusted asynchronous execution has ended.
    })
  }

  private sessionProps(props: MicroProps): MicroProps {
    if (!this.options.domain || !props.signal) return props
    if (this.propsParentSignal !== props.signal) {
      this.propsParentSignal = props.signal
      this.propsCombinedSignal = AbortSignal.any([props.signal, this.controller.signal])
    }
    return { ...props, signal: this.propsCombinedSignal! }
  }

  private finishCleanup(): void {
    if (this.cleaned) return
    this.cleaned = true
    try { this.cleanup() }
    catch (error) {
      if (this.options.domain) throw this.options.domain.poison(new MicroRuntimeFault("unmount", "OPERATION_FAILED"))
      throw error
    } finally {
      this.unbindDomain?.()
      this.unbindDomain = null
      this.unbindScope?.()
      this.unbindScope = null
    }
  }

  mount(load: () => Promise<MicroApplicationHandle>, container: HTMLElement, props: MicroProps | (() => MicroProps)): Promise<void> {
    return this.enqueue(async () => {
      if (this.stopped) return
      const lifecycle = await this.stage("load", load)
      this.options.domain?.assertHealthy()
      if (this.stopped) return
      this.lifecycle = lifecycle
      const value = this.sessionProps(typeof props === "function" ? props() : props)
      await this.stage("mount", () => {
        if (value.signal?.aborted) throw new DOMException("Application context cancelled", "AbortError")
        this.mounted = true
        return this.run(() => lifecycle.mount(container, value))
      })
    })
  }

  update(props: MicroProps): Promise<void> {
    return this.enqueue(async () => {
      if (!this.stopped && this.mounted && !props.signal.aborted &&
          Date.parse(props.expiresAt) > Date.now() && Date.parse(props.tokenExpiresAt) > Date.now()) {
        await this.stage("update", () => {
          if (props.signal.aborted) return
          return this.run(() => this.lifecycle?.update?.(this.sessionProps(props)))
        })
      }
    })
  }

  stop(): Promise<void> {
    this.stopped = true
    this.controller.abort()
    if (!this.disposal) {
      const task = this.enqueue(async () => {
        try {
          if (this.mounted) {
            this.mounted = false
            try { await this.stage("unmount", () => this.run(() => this.lifecycle?.unmount())) }
            catch (error) {
              if (this.options.domain) throw this.options.domain.poison(
                isMicroRuntimeFault(error) ? error : new MicroRuntimeFault("unmount", "OPERATION_FAILED"))
              throw error
            }
          }
        } finally { this.finishCleanup() }
      })
      // A poisoned domain rejects without invoking trusted work; host-owned
      // resource cleanup still runs once, without interpreting it as a kill.
      this.disposal = task.catch((error: unknown) => { this.finishCleanup(); throw error })
      void this.disposal.catch(() => {})
    }
    return this.disposal
  }
}

/** 包括 token 原值、菜单内容和完整授权字段，避免同数量变化漏更新。 */
export function microPropsSignature(props: MicroProps): string {
  return JSON.stringify([props.sdkVersion, props.contractVersion, props.contextVersion, props.contextEpoch,
    props.expiresAt, props.tokenExpiresAt, props.token, props.appCode, props.user,
    props.deploymentVersion, props.entryVersionId,
    props.tenant, props.organization, props.organizationScopeIds, props.roles, props.permissions, props.dataGrants,
    props.menus, props.routePath, props.theme])
}

/** Cache the accepted authorization signature before application code can mutate its props. */
export function microPropsDelivery(props: MicroProps) {
  return { authorizationKey: authorizationContextSignature(props), signal: props.signal, routePath: props.routePath }
}

/** Apps without the optional update lifecycle remount for navigation or authorization changes. */
export function requiresMicroRemount(previous: ReturnType<typeof microPropsDelivery> | null, props: MicroProps): boolean {
  const key = authorizationContextSignature(props)
  return !previous || key === null || key !== previous.authorizationKey || previous.signal !== props.signal ||
    props.signal.aborted || previous.routePath !== props.routePath
}
