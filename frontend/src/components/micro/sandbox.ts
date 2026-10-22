/**
 * 可信 HTML 微应用的资源回收沙箱，不是不可信代码的安全边界。
 * window 写入保存在私有属性表，计时器和 window 监听器由沙箱回收。
 * document 必须保持真实对象身份，供 jQuery/Sizzle 做委托事件匹配。
 * run() 仅在同步调用期间包装真实 document 的监听器方法，并在 finally 恢复；
 * mount/update/unmount 及沙箱回调应通过 run() 调用。await 后的资源由应用
 * 使用 MicroProps.signal 和 unmount 自行停止，不延长 document 全局包装时间。
 * MutationObserver 只记账同步应用作用域内新增的 STYLE/LINK/SCRIPT，避免回收宿主资源。
 */

export type MicroGlobalProxy = Record<string, unknown> & {
  window: unknown
  self: unknown
  globalThis: unknown
  __MICRO_APP__?: unknown
}

type TimerKind = "timeout" | "interval"

interface ListenerRecord {
  target: EventTarget
  type: string
  listener: EventListenerOrEventListenerObject
  wrapped: EventListener
  capture: boolean
  abortSignal?: AbortSignal
  abortListener?: EventListener
}

export interface MicroSandboxStats {
  name: string
  ownProps: number
  overriddenKeys: string[]
  timers: number
  listeners: number
  insertedNodes: number
  titleTouched: boolean
  tornDown: boolean
}

const CONSTRUCTOR_KEY_RE = /^[A-Z]/
const TRACKED_TAGS = new Set(["STYLE", "LINK", "SCRIPT"])

interface DocumentMethods {
  add: EventTarget["addEventListener"]
  remove: EventTarget["removeEventListener"]
}

// 嵌套 run() 仍调用最初的原生方法，避免给另一个沙箱重复记账。
const documentMethods = new WeakMap<Document, DocumentMethods>()
const mutationOwners = new WeakMap<Document, MicroSandbox>()
const scriptScopeBatches = new WeakMap<Document, Set<() => void>>()

function getDocumentMethods(): DocumentMethods {
  let methods = documentMethods.get(document)
  if (!methods) {
    methods = { add: document.addEventListener, remove: document.removeEventListener }
    documentMethods.set(document, methods)
  }
  return methods
}

export class MicroSandbox {
  readonly proxy: MicroGlobalProxy

  private readonly ownProps = new Map<string | symbol, unknown>()
  private readonly overriddenKeys = new Set<string>()
  private readonly timers = new Map<number, TimerKind>()
  private readonly animationFrames = new Set<number>()
  private readonly listeners = new Set<ListenerRecord>()
  private readonly insertedNodes = new Set<Node>()
  private readonly boundWindowFns = new Map<string, { raw: unknown; bound: unknown }>()
  private readonly nativeDocument = getDocumentMethods()
  private observer: MutationObserver | null = null
  private observing = false
  private tornDown = false

  constructor(readonly name: string) {
    this.proxy = new Proxy(window, {
      get: (_target, key) => this.handleGet(key),
      set: (_target, key, value) => {
        this.handleSet(key, value)
        return true
      },
      deleteProperty: (_target, key) => {
        this.ownProps.delete(key)
        return true
      },
      has: () => true,
    }) as unknown as MicroGlobalProxy
  }

  /** 同步调用作用域，真实 document 身份不变；返回 Promise 时立即恢复宿主方法。 */
  run<T>(callback: () => T): T {
    const leave = this.enterScope()
    try { return callback() }
    finally { leave() }
  }

  /** import-html-entry omits afterExec when a script throws. The first script
   * schedules the whole synchronous batch's cleanup, ahead of microtasks from
   * earlier scripts that may otherwise run before a later failing script exits. */
  beginScriptScope(): () => void {
    const exitScope = this.enterScope()
    let batch = scriptScopeBatches.get(document)
    if (!batch) {
      batch = new Set()
      scriptScopeBatches.set(document, batch)
      const currentBatch = batch
      queueMicrotask(() => {
        scriptScopeBatches.delete(document)
        // Restore nested owners and document descriptors in stack order.
        for (const leave of [...currentBatch].reverse()) leave()
      })
    }
    const currentBatch = batch
    const leave = () => { currentBatch.delete(leave); exitScope() }
    currentBatch.add(leave)
    return leave
  }

  private enterScope(): () => void {
    if (this.tornDown) throw new Error(`微应用 ${this.name} 已卸载`)
    const previousOwner = mutationOwners.get(document)
    if (previousOwner !== this) {
      previousOwner?.pauseObserver()
      this.ensureObserver()
      mutationOwners.set(document, this)
    }
    const addDescriptor = Object.getOwnPropertyDescriptor(document, "addEventListener")
    const removeDescriptor = Object.getOwnPropertyDescriptor(document, "removeEventListener")
    let left = false
    const leave = () => {
      if (left) return
      left = true
      if (previousOwner !== this) {
        this.pauseObserver()
        if (previousOwner && !previousOwner.tornDown) {
          previousOwner.ensureObserver()
          mutationOwners.set(document, previousOwner)
        } else mutationOwners.delete(document)
      }
      if (addDescriptor) Object.defineProperty(document, "addEventListener", addDescriptor)
      else delete (document as unknown as Record<string, unknown>).addEventListener
      if (removeDescriptor) Object.defineProperty(document, "removeEventListener", removeDescriptor)
      else delete (document as unknown as Record<string, unknown>).removeEventListener
    }
    try {
      Object.defineProperty(document, "addEventListener", {
        configurable: true, writable: true, value: this.trackedDocumentAddEventListener,
      })
      Object.defineProperty(document, "removeEventListener", {
        configurable: true, writable: true, value: this.trackedDocumentRemoveEventListener,
      })
    } catch (error) { leave(); throw error }
    return leave
  }

  private handleGet(key: string | symbol): unknown {
    if (!this.tornDown && this.ownProps.has(key)) return this.ownProps.get(key)
    if (typeof key === "symbol") return Reflect.get(window, key)
    // 卸载后的代理仍返回受控 API，迟到代码不能重新创建未记账的计时器/监听器。
    switch (key) {
      case "window":
      case "self":
      case "globalThis":
      case "top":
      case "parent":
        return this.proxy
      case "setTimeout": return this.trackedSetTimeout
      case "setInterval": return this.trackedSetInterval
      case "clearTimeout": return this.trackedClearTimeout
      case "clearInterval": return this.trackedClearInterval
      case "requestAnimationFrame": return this.trackedRequestAnimationFrame
      case "cancelAnimationFrame": return this.trackedCancelAnimationFrame
      case "addEventListener": return this.trackedAddEventListener
      case "removeEventListener": return this.trackedRemoveEventListener
      case "document": return document
      default: return this.passThroughGet(key)
    }
  }

  private passThroughGet(key: string): unknown {
    const raw = Reflect.get(window, key)
    if (typeof raw !== "function" || CONSTRUCTOR_KEY_RE.test(key)) return raw
    const cached = this.boundWindowFns.get(key)
    if (cached && cached.raw === raw) return cached.bound
    const bound = (raw as (...args: unknown[]) => unknown).bind(window)
    this.boundWindowFns.set(key, { raw, bound })
    return bound
  }

  private handleSet(key: string | symbol, value: unknown): void {
    if (this.tornDown) return
    if (typeof key === "string" && Object.prototype.hasOwnProperty.call(window, key)) {
      this.overriddenKeys.add(key)
    }
    this.ownProps.set(key, value)
  }

  private invokeTimer(handler: TimerHandler, args: unknown[]): void {
    this.run(() => {
      if (typeof handler === "function") handler.apply(this.proxy, args)
      else {
        // 字符串计时器也在 proxy 上执行，不能借浏览器原生字符串求值写入宿主全局。
        const execute = new Function("window", "self", "globalThis", `with(window){${handler}\n}`)
        execute.call(this.proxy, this.proxy, this.proxy, this.proxy)
      }
    })
  }

  private readonly trackedSetTimeout = (handler: TimerHandler, timeout?: number, ...args: unknown[]): number => {
    if (this.tornDown) return 0
    const id = window.setTimeout(() => {
      if (this.tornDown || !this.timers.has(id)) return
      this.timers.delete(id)
      this.invokeTimer(handler, args)
    }, timeout)
    this.timers.set(id, "timeout")
    return id
  }

  private readonly trackedSetInterval = (handler: TimerHandler, timeout?: number, ...args: unknown[]): number => {
    if (this.tornDown) return 0
    const id = window.setInterval(() => {
      if (this.tornDown || !this.timers.has(id)) return
      this.invokeTimer(handler, args)
    }, timeout)
    this.timers.set(id, "interval")
    return id
  }

  private readonly trackedClearTimeout = (id?: number): void => {
    this.timers.delete(id as number)
    window.clearTimeout(id)
  }

  private readonly trackedClearInterval = (id?: number): void => {
    this.timers.delete(id as number)
    window.clearInterval(id)
  }

  private readonly trackedRequestAnimationFrame = (callback: FrameRequestCallback): number => {
    if (this.tornDown) return 0
    const id = window.requestAnimationFrame((time) => {
      if (this.tornDown || !this.animationFrames.delete(id)) return
      this.run(() => callback.call(this.proxy, time))
    })
    this.animationFrames.add(id)
    return id
  }

  private readonly trackedCancelAnimationFrame = (id: number): void => {
    this.animationFrames.delete(id)
    window.cancelAnimationFrame(id)
  }

  private addListener(
    target: EventTarget,
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    options?: boolean | AddEventListenerOptions
  ): void {
    if (this.tornDown || !listener) return
    const capture = typeof options === "boolean" ? options : !!options?.capture
    const signal = typeof options === "object" ? options.signal : undefined
    const once = typeof options === "object" && !!options.once
    if (signal?.aborted) return
    if ([...this.listeners].some((record) => record.target === target && record.type === type &&
      record.listener === listener && record.capture === capture)) return
    const record: ListenerRecord = {
      target, type, listener, capture,
      wrapped: (event) => {
        if (this.tornDown) return
        if (once) this.forgetListener(record)
        this.run(() => {
          if (typeof listener === "function") listener.call(target === window ? this.proxy : target, event)
          else listener.handleEvent.call(listener, event)
        })
      },
    }
    const add = target === document ? this.nativeDocument.add : window.addEventListener
    add.call(target, type, record.wrapped, options)
    this.listeners.add(record)
    if (signal) {
      record.abortSignal = signal
      record.abortListener = () => this.forgetListener(record)
      signal.addEventListener("abort", record.abortListener, { once: true })
    }
  }

  private forgetListener(record: ListenerRecord): void {
    this.listeners.delete(record)
    if (record.abortSignal && record.abortListener) {
      record.abortSignal.removeEventListener("abort", record.abortListener)
    }
  }

  private removeListener(
    target: EventTarget,
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    options?: boolean | EventListenerOptions
  ): void {
    const capture = typeof options === "boolean" ? options : !!options?.capture
    const remove = target === document ? this.nativeDocument.remove : window.removeEventListener
    const record = [...this.listeners].find((candidate) => candidate.target === target &&
      candidate.type === type && candidate.listener === listener && candidate.capture === capture)
    remove.call(target, type, record?.wrapped ?? listener, capture)
    if (record) this.forgetListener(record)
  }

  private readonly trackedAddEventListener = (
    type: string, listener: EventListenerOrEventListenerObject | null, options?: boolean | AddEventListenerOptions
  ): void => this.addListener(window, type, listener, options)

  private readonly trackedRemoveEventListener = (
    type: string, listener: EventListenerOrEventListenerObject | null, options?: boolean | EventListenerOptions
  ): void => this.removeListener(window, type, listener, options)

  private readonly trackedDocumentAddEventListener = (
    type: string, listener: EventListenerOrEventListenerObject | null, options?: boolean | AddEventListenerOptions
  ): void => this.addListener(document, type, listener, options)

  private readonly trackedDocumentRemoveEventListener = (
    type: string, listener: EventListenerOrEventListenerObject | null, options?: boolean | EventListenerOptions
  ): void => this.removeListener(document, type, listener, options)

  private ensureObserver(): void {
    if (this.tornDown || this.observing || typeof MutationObserver === "undefined") return
    const observer = this.observer ?? new MutationObserver((records) => {
      if (!this.tornDown && this.observing) this.trackMutationRecords(records)
    })
    try {
      observer.observe(document.head, { childList: true, subtree: true })
      observer.observe(document.body, { childList: true, subtree: true })
      this.observer = observer
      this.observing = true
    } catch {
      observer.disconnect()
    }
  }

  private pauseObserver(): void {
    if (!this.observer || !this.observing) return
    this.trackMutationRecords(this.observer.takeRecords())
    this.observer.disconnect()
    this.observing = false
  }

  private trackMutationRecords(records: MutationRecord[]): void {
    for (const record of records) {
      for (const node of record.addedNodes) this.trackInsertedNode(node)
    }
  }

  private trackInsertedNode(node: Node): void {
    if (node.nodeType !== Node.ELEMENT_NODE) return
    const element = node as Element
    if (TRACKED_TAGS.has(element.tagName) && !element.hasAttribute("data-vite-dev-id")) {
      this.insertedNodes.add(node)
    }
    for (const child of element.querySelectorAll("style,link,script")) {
      if (!child.hasAttribute("data-vite-dev-id")) this.insertedNodes.add(child)
    }
  }

  /** 回收幂等；同步作用域结束或卸载前取走 pending records，防止漏记。 */
  teardown(): void {
    if (this.tornDown) return
    this.pauseObserver()
    this.tornDown = true

    for (const [id, kind] of this.timers) {
      if (kind === "timeout") window.clearTimeout(id)
      else window.clearInterval(id)
    }
    this.timers.clear()
    for (const id of this.animationFrames) window.cancelAnimationFrame(id)
    this.animationFrames.clear()

    for (const record of this.listeners) {
      const remove = record.target === document ? this.nativeDocument.remove : window.removeEventListener
      remove.call(record.target, record.type, record.wrapped, record.capture)
      this.forgetListener(record)
    }

    this.observer?.disconnect()
    this.observer = null
    if (mutationOwners.get(document) === this) mutationOwners.delete(document)
    for (const node of this.insertedNodes) {
      if (node.isConnected) node.parentNode?.removeChild(node)
    }
    this.insertedNodes.clear()
    this.ownProps.clear()
    this.overriddenKeys.clear()
    this.boundWindowFns.clear()

    // 旧沙箱迟到 teardown 时，不删除新沙箱正在使用的 import-html-entry 全局代理。
    try {
      if ((window as unknown as Record<string, unknown>).proxy === this.proxy) {
        delete (window as unknown as Record<string, unknown>).proxy
      }
    } catch { /* window.proxy 可能由宿主以不可配置属性提供 */ }
  }

  getStats(): MicroSandboxStats {
    return {
      name: this.name,
      ownProps: this.ownProps.size,
      overriddenKeys: [...this.overriddenKeys],
      timers: this.timers.size + this.animationFrames.size,
      listeners: this.listeners.size,
      insertedNodes: this.insertedNodes.size,
      titleTouched: false,
      tornDown: this.tornDown,
    }
  }
}
