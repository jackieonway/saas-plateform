import type { AppAuthorizationContext, MicroLifecycle, MicroProps } from "./types"

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
  now?: () => number
  setTimer?: (callback: () => void, delay: number) => ReturnType<typeof setTimeout>
  clearTimer?: (timer: ReturnType<typeof setTimeout>) => void
}

let deliveredEpoch = 0

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
    this.request?.abort()
    const request = new AbortController()
    this.request = request
    const current = () => !this.stopped && sequence === this.sequence && !request.signal.aborted
    try {
      let lease = this.lease
      if (!lease || lease.renewAt <= this.now()) {
        const issuedAt = this.now()
        const issued = await this.options.issue(request.signal)
        if (!current()) return
        if (!issued.accessToken || issued.audience !== this.options.appCode || issued.expiresIn <= 0) {
          throw new Error("应用令牌响应无效")
        }
        const ttl = issued.expiresIn * 1000
        lease = { token: issued.accessToken, expires: issuedAt + ttl,
          renewAt: issuedAt + ttl - Math.min(30_000, ttl / 5) }
      }
      const context = await this.options.context(lease.token, request.signal)
      if (!current()) return
      const expires = Math.min(Date.parse(context.expiresAt), lease.expires)
      if (context.contractVersion !== "1.0" || context.appCode !== this.options.appCode ||
          !context.contextVersion || !Number.isFinite(expires) || expires <= this.now()) {
        throw new Error("应用授权上下文已过期或与目标应用不一致")
      }
      this.lease = lease
      this.lifetime?.abort()
      this.clear(this.expiryTimer)
      const lifetime = new AbortController()
      this.lifetime = lifetime
      this.options.onChange({ token: lease.token, tokenExpiresAt: new Date(lease.expires).toISOString(),
        context, contextEpoch: ++deliveredEpoch, signal: lifetime.signal })
      this.expiryTimer = this.schedule(() => {
        lifetime.abort()
        if (!this.stopped && this.lifetime === lifetime) this.options.onChange(null)
      }, expires - this.now())
      this.refreshTimer = this.schedule(() => { void this.refresh() },
        Math.min(20_000, Math.max(100, (expires - this.now()) * 0.8),
          Math.max(100, lease.renewAt - this.now())))
    } catch (error) {
      if (!current()) return
      this.stop()
      this.options.onChange(null)
      this.options.onError(error)
    }
  }

  stop(): void {
    this.stopped = true
    this.sequence++
    this.request?.abort()
    this.lifetime?.abort()
    this.clear(this.refreshTimer)
    this.clear(this.expiryTimer)
    this.lease = null
  }
}

/** 串行 mount/update/unmount；stop 立即阻止新操作，最后等待在途任务后卸载。 */
export class MicroLifecycleSession {
  private chain: Promise<void> = Promise.resolve()
  private lifecycle: MicroLifecycle | null = null
  private mounted = false
  private stopped = false
  private disposal: Promise<void> | null = null
  private cleanup: () => void
  private run: <T>(callback: () => T) => T

  constructor(cleanup: () => void, run: <T>(callback: () => T) => T = (callback) => callback()) {
    this.cleanup = cleanup
    this.run = run
  }
  get supportsUpdate() { return typeof this.lifecycle?.update === "function" }

  mount(load: () => Promise<MicroLifecycle>, container: HTMLElement, props: MicroProps | (() => MicroProps)): Promise<void> {
    const task = this.chain.then(async () => {
      if (this.stopped) return
      this.lifecycle = await load()
      if (this.stopped) return
      this.mounted = true
      await this.run(() => this.lifecycle!.mount(container, typeof props === "function" ? props() : props))
    })
    this.chain = task.catch(() => {})
    return task
  }

  update(props: MicroProps): Promise<void> {
    const task = this.chain.then(async () => {
      if (!this.stopped && this.mounted && !props.signal.aborted &&
          Date.parse(props.expiresAt) > Date.now() && Date.parse(props.tokenExpiresAt) > Date.now()) {
        await this.run(() => this.lifecycle?.update?.(props))
      }
    })
    this.chain = task.catch(() => {})
    return task
  }

  stop(): Promise<void> {
    this.stopped = true
    if (!this.disposal) {
      this.disposal = this.chain.then(async () => {
        try {
          if (this.mounted) await this.run(() => this.lifecycle?.unmount())
        } finally { this.cleanup() }
      })
    }
    return this.disposal
  }
}

/** 包括 token 原值、菜单内容和完整授权字段，避免同数量变化漏更新。 */
export function microPropsSignature(props: MicroProps): string {
  return JSON.stringify([props.contractVersion, props.contextVersion, props.contextEpoch,
    props.expiresAt, props.tokenExpiresAt, props.token, props.appCode, props.user,
    props.deploymentVersion, props.entryVersionId,
    props.tenant, props.organization, props.organizationScopeIds, props.roles, props.permissions, props.dataGrants,
    props.menus, props.routePath, props.theme])
}
