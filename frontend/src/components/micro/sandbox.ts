/**
 * ============================================================================
 * Dante Cloud 自研微前端沙箱 —— 防误伤沙箱(Accidental-Damage Guard Sandbox)
 * ----------------------------------------------------------------------------
 * 【沙箱定位】本沙箱目标是防误伤与资源回收, 不是安全边界;
 * 不可信代码请走 iframe 通道(link_target SELF, /embed 路由原生 origin 隔离)。
 *
 * 设计要点(与 qiankun SingularProxySandbox 同思路的"快照式"轻量实现):
 * - Proxy 代理 window: 自有属性表(ownProps)优先, 未命中回读真实 window;
 *   set 永远写入自有表, 真实 window 不被污染; 被覆盖的键记录在 overriddenKeys 供诊断。
 * - has 恒 true: 保证 `with(proxy){}`(strictGlobal 执行模式)与 `in`/`typeof` 检查不逃逸。
 * - 计时器记账: setTimeout/setInterval 走包装版, teardown 批量 clear。
 * - 监听器记账: window 级 add/removeEventListener 走包装版, teardown 批量移除。
 * - ⚠ document 不做代理, 直接返回真实 document(qiankun 同款决策, 实证必要):
 *   jQuery/Sizzle 的委托匹配(JQuery.event.handlers → find(sel, ctx, null, [seed]))
 *   要求 context 出现在种子元素的真实 parentNode 祖先链上 —— 代理对象永远不在
 *   真实 DOM 树中, 一旦作为 context 传入, 匹配恒为 0 → 所有 $(document) 委托事件
 *   静默失效(实测: proxy ctx=0 / real doc ctx=1)。document 上的样式/脚本插入
 *   记账改由 MutationObserver 完成(见下), 不需要代理。
 * - 插入节点记账: MutationObserver 观察 document.head/document.body 的 childList,
 *   只记录新增的 STYLE/LINK/SCRIPT 节点; 跳过带 data-vite-dev-id 的节点
 *   (Vite HMR 的主应用样式更新, 误删会破坏主应用样式)。teardown 时批量移除。
 *   代价: 与主应用"同帧"插入的其他 style 有极小误记概率(可接受, qiankun 干脆不回收)。
 * - document.title 的快照/还原由编排容器负责(挂载前快照, 卸载后还原),
 *   沙箱不再代理 document 故无法拦截 title 写入。
 * - import-html-entry 在真实 window 上挂 window.proxy(其内核要求), teardown 时清除。
 * - teardown() 幂等, 可重复调用。
 */

/** 沙箱对外暴露的全局对象形状(宽松索引; 契约键显式声明便于编排层消费) */
export type MicroGlobalProxy = Record<string, unknown> & {
  window: unknown
  self: unknown
  globalThis: unknown
  __DANTE_MICRO_APP__?: unknown
}

/** 记账计时器种类 */
type TimerKind = "timeout" | "interval"

interface TimerRecord {
  kind: TimerKind
}

/** window 级监听器记账条目 */
interface ListenerRecord {
  type: string
  listener: EventListenerOrEventListenerObject
  options: boolean | AddEventListenerOptions | undefined
}

/** 诊断统计(容器状态条/控制台排查用) */
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

/** 构造器启发: 大写开头的 window 函数属性视为类/构造器, 不做 bind(new X() 语义保持) */
const CONSTRUCTOR_KEY_RE = /^[A-Z]/

/** MutationObserver 记账的元素标签 */
const TRACKED_TAGS = new Set(["STYLE", "LINK", "SCRIPT"])

export class MicroSandbox {
  readonly proxy: MicroGlobalProxy

  private readonly ownProps = new Map<string | symbol, unknown>()
  private readonly overriddenKeys = new Set<string>()
  private readonly timers = new Map<number, TimerRecord>()
  private readonly listeners = new Set<ListenerRecord>()
  private readonly insertedNodes = new Set<Node>()
  /** 真实 window 函数的 bind 缓存(引用稳定性, 避免每次访问产生新函数) */
  private readonly boundWindowFns = new Map<string, unknown>()
  /** document.head/body 插入节点观察器(懒启动) */
  private observer: MutationObserver | null = null
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
      // 恒 true: with(proxy) 变量探测 / `in` 检查不逃逸到外层作用域
      has: () => true,
    }) as unknown as MicroGlobalProxy
    this.ensureObserver()
  }

  /* ============================ window get ============================ */

  private handleGet(key: string | symbol): unknown {
    if (typeof key === "symbol") return Reflect.get(window, key)
    if (!this.tornDown) {
      switch (key) {
        case "window":
        case "self":
        case "globalThis":
        case "top":
        case "parent":
          return this.proxy
      }
      if (this.ownProps.has(key)) return this.ownProps.get(key)
      switch (key) {
        case "setTimeout":
          return this.trackedSetTimeout
        case "setInterval":
          return this.trackedSetInterval
        case "clearTimeout":
          return this.trackedClearTimeout
        case "clearInterval":
          return this.trackedClearInterval
        case "addEventListener":
          return this.trackedAddEventListener
        case "removeEventListener":
          return this.trackedRemoveEventListener
        // document 不代理: 真实 document(qiankun 同款决策, 详见文件头注释)
        // window 级监听记账不受影响(trackedAddEventListener); document 级监听
        // 由子应用自清或随节点回收(与 qiankun 行为一致)。
        case "document":
          return document
      }
    }
    return this.passThroughGet(key)
  }

  /** 真实 window 值透传: 函数且非构造器形态的返回 bind 后的副本(缓存) */
  private passThroughGet(key: string): unknown {
    if (CONSTRUCTOR_KEY_RE.test(key)) return Reflect.get(window, key)
    let bound = this.boundWindowFns.get(key)
    if (bound === undefined) {
      const raw = Reflect.get(window, key)
      bound = typeof raw === "function" ? (raw as (...args: unknown[]) => unknown).bind(window) : raw
      this.boundWindowFns.set(key, bound)
    }
    return bound
  }

  /* ============================ window set / 记账 API ============================ */

  private handleSet(key: string | symbol, value: unknown): void {
    if (this.tornDown) return // 卸载后写入直接丢弃, 不污染真实 window
    if (typeof key === "string" && Object.prototype.hasOwnProperty.call(window, key)) {
      this.overriddenKeys.add(key)
    }
    this.ownProps.set(key, value)
  }

  private readonly trackedSetTimeout: (
    handler: TimerHandler,
    timeout?: number,
    ...args: unknown[]
  ) => number = (handler, timeout, ...args) => {
    let id = 0
    // 到期自动出账(仅 timeout; interval 持续到 clear)
    const wrapped: TimerHandler =
      typeof handler === "function"
        ? (...inner: unknown[]) => {
            this.timers.delete(id)
            ;(handler as (...a: unknown[]) => void)(...inner)
          }
        : handler
    id = window.setTimeout(wrapped, timeout, ...args)
    if (!this.tornDown) this.timers.set(id, { kind: "timeout" })
    return id
  }

  private readonly trackedSetInterval: (
    handler: TimerHandler,
    timeout?: number,
    ...args: unknown[]
  ) => number = (handler, timeout, ...args) => {
    const id = window.setInterval(handler, timeout, ...args)
    if (!this.tornDown) this.timers.set(id, { kind: "interval" })
    return id
  }

  private readonly trackedClearTimeout: (id?: number) => void = (id) => {
    this.timers.delete(id as number)
    window.clearTimeout(id)
  }

  private readonly trackedClearInterval: (id?: number) => void = (id) => {
    this.timers.delete(id as number)
    window.clearInterval(id)
  }

  private readonly trackedAddEventListener: (
    type: string,
    listener: EventListenerOrEventListenerObject,
    options?: boolean | AddEventListenerOptions
  ) => void = (type, listener, options) => {
    window.addEventListener(type, listener, options)
    if (!this.tornDown) this.listeners.add({ type, listener, options })
  }

  private readonly trackedRemoveEventListener: (
    type: string,
    listener: EventListenerOrEventListenerObject,
    options?: boolean | AddEventListenerOptions
  ) => void = (type, listener, options) => {
    window.removeEventListener(type, listener, options)
    for (const rec of this.listeners) {
      if (rec.type === type && rec.listener === listener) {
        this.listeners.delete(rec)
        break // 与真实 window 语义一致: 只移除一条注册
      }
    }
  }

  /* ============================ 插入节点观察(MutationObserver) ============================ */

  /**
   * 观察 document.head / document.body 的直接子节点新增:
   * 只记账 STYLE/LINK/SCRIPT(子应用动态样式/脚本), 跳过 Vite HMR 样式(data-vite-dev-id)。
   * 不代理 document, 因此对 jQuery 事件委托/Sizzle 选择器零干扰(见文件头)。
   */
  private ensureObserver(): void {
    if (this.tornDown || this.observer || typeof MutationObserver === "undefined") return
    const observer = new MutationObserver((records) => {
      if (this.tornDown) return
      for (const record of records) {
        for (const node of record.addedNodes) {
          this.trackInsertedNode(node)
        }
      }
    })
    const opts: MutationObserverInit = { childList: true }
    try {
      observer.observe(document.head, opts)
      observer.observe(document.body, opts)
      this.observer = observer
    } catch {
      this.observer = null
    }
  }

  private trackInsertedNode(node: Node): void {
    if (this.insertedNodes.has(node)) return
    if (node.nodeType !== Node.ELEMENT_NODE) return
    const el = node as Element
    if (!TRACKED_TAGS.has(el.tagName)) return
    // Vite HMR 注入的主应用样式带 data-vite-dev-id, 不属于子应用, 防误删
    if (el.hasAttribute("data-vite-dev-id")) return
    this.insertedNodes.add(node)
  }

  /* ============================ teardown ============================ */

  /** 卸载: 清理计时器/监听/插入节点, 丢弃自有表; 幂等可重复调用 */
  teardown(): void {
    if (this.tornDown) return
    this.tornDown = true

    // 1. 计时器全清
    for (const [id, rec] of this.timers) {
      if (rec.kind === "timeout") window.clearTimeout(id)
      else window.clearInterval(id)
    }
    this.timers.clear()

    // 2. window 级监听器全移除
    for (const rec of this.listeners) {
      window.removeEventListener(rec.type, rec.listener, rec.options)
    }
    this.listeners.clear()

    // 3. 停止观察, 移除子应用插入的 style/link/script 节点(仅在还挂在文档上时)
    this.observer?.disconnect()
    this.observer = null
    for (const node of this.insertedNodes) {
      if (node.isConnected) node.parentNode?.removeChild(node)
    }
    this.insertedNodes.clear()

    // 4. 丢弃自有属性表与诊断记录(bind 缓存指向真实对象, 保留无害)
    this.ownProps.clear()
    this.overriddenKeys.clear()

    // 5. import-html-entry 内核在真实 window 上挂的 window.proxy
    try {
      delete (window as unknown as Record<string, unknown>).proxy
    } catch {
      /* ignore */
    }
  }

  /** 诊断统计(不参与主流程) */
  getStats(): MicroSandboxStats {
    return {
      name: this.name,
      ownProps: this.ownProps.size,
      overriddenKeys: [...this.overriddenKeys],
      timers: this.timers.size,
      listeners: this.listeners.size,
      insertedNodes: this.insertedNodes.size,
      titleTouched: false,
      tornDown: this.tornDown,
    }
  }
}
