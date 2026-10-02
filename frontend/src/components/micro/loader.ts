/**
 * HTML 入口微前端加载器(loadHtmlEntry)
 *
 * 内核: npm 包 import-html-entry(v1.17.0, 自带 TS 类型)
 *  - importEntry(url) 抓取子应用 HTML → 正则拆出 script/style → 外链再 fetch 文本
 *  - template 已把外链 <link> 样式内联为 <style>(getEmbedHTML), 随容器 DOM 一同回收
 *  - execScripts(proxy, strictGlobal=true) 以
 *    (function(window, self, globalThis){ with(window){ code } }).bind(proxy)(proxy×3)
 *    执行脚本: with(proxy) 使裸标识符(document/setTimeout/addEventListener/未声明变量)
 *    全部经沙箱 get/has 拦截 —— 计时器/监听/document 轻代理的记账因此对子应用透明生效。
 *    "use strict" 脚本安全: 指令位于 with 块内属惰性字符串(非函数/脚本序言),
 *    不会触发严格模式 with 禁令(Node 冒烟 15 断言实证, 见 worklog 21-c)。
 *
 * 选型说明: 首选 import-html-entry 落地(模板解析/脚本顺序/async script/缓存皆为
 * 生产级实现, 冒烟全绿); 自写 fallback 加载器未启用。
 *
 * 增强点(自研包裹层):
 *  - 15 秒整体超时(AbortController 贯穿全部子请求)
 *  - 失败重试缓存穿透: import-html-entry 的 scriptCache/styleCache 会缓存旧结果
 *    ("已拒绝的 Promise"或"修复前的旧脚本文本"两种形态, 后者发生在 fetch 成功但执行
 *    抛错、随后源文件被修复的场景)。本层在入口曾失败时给所有子请求追加 ?_dmr=<N>
 *    亡缓存参数, 让重试走全新缓存键; 成功后清除标记。fetch 失败自动计入,
 *    执行失败由容器调 markEntryFailed() 计入。
 */

import { importEntry } from "import-html-entry"

export interface HtmlEntryResult {
  /** 处理后的 HTML 模板(外链样式已内联为 <style>), 由容器写入挂载区 */
  templateProcessed: string
  /** 在给定沙箱 proxy 上依序执行子应用脚本(入口脚本抛错会 reject) */
  execScripts: (proxy: object) => Promise<unknown>
}

const DEFAULT_TIMEOUT_MS = 15_000

/** 入口历史失败次数(用于重试时的缓存穿透) */
const failedEntries = new Map<string, number>()
/** 成功代次保持缓存复用；失败/取消同时更换 HTML 和资源缓存键，永不退回已拒绝的 Promise。 */
const entryGenerations = new Map<string, number>()

const BUSTER_KEY = "_dmr"

function withBuster(url: string, attempt: number): string {
  try {
    const u = new URL(url, window.location.origin)
    u.searchParams.set(BUSTER_KEY, String(attempt))
    return u.toString()
  } catch {
    return url
  }
}

function normalizeMessage(e: unknown, entryUrl: string): string {
  const raw = e instanceof Error ? e.message : String(e)
  // 浏览器 fetch abort 的报文没有上下文, 补入口地址便于排查
  if (/aborted|AbortError/i.test(raw)) return `子应用加载超时: ${entryUrl}`
  return raw
}

/**
 * 加载 HTML 入口子应用。
 * @param entryUrl 入口地址(如 /sub/vue/); 同源相对路径经 Vite 代理转发到子应用服务
 * @param opts.timeoutMs 整体超时(默认 15s), 超时中止全部子请求
 */
export async function loadHtmlEntry(entryUrl: string, opts?: { timeoutMs?: number; signal?: AbortSignal }): Promise<HtmlEntryResult> {
  const timeoutMs = opts?.timeoutMs ?? DEFAULT_TIMEOUT_MS
  const failures = failedEntries.get(entryUrl) ?? 0
  const generation = entryGenerations.get(entryUrl) ?? 0
  const importUrl = generation > 0 ? withBuster(entryUrl, generation) : entryUrl
  let invalidated = false
  const invalidate = () => {
    if (invalidated) return
    invalidated = true
    entryGenerations.set(entryUrl, Math.max(generation + 1, entryGenerations.get(entryUrl) ?? 0))
    failedEntries.set(entryUrl, failures + 1)
  }

  const controller = new AbortController()
  const abortFromParent = () => { controller.abort(); cleanup() }
  opts?.signal?.addEventListener("abort", abortFromParent, { once: true })
  if (opts?.signal?.aborted) controller.abort()
  const timerId = window.setTimeout(() => { controller.abort(); cleanup() }, timeoutMs)
  let abortListener: () => void
  const abortPromise = new Promise<never>((_resolve, reject) => {
    abortListener = () => reject(opts?.signal?.aborted
      ? new DOMException("应用加载已取消", "AbortError")
      : new Error(`子应用加载超时(${Math.round(timeoutMs / 1000)}s): ${entryUrl}`))
    controller.signal.addEventListener("abort", abortListener, { once: true })
    if (controller.signal.aborted) abortListener()
  })
  const cleanup = () => {
    window.clearTimeout(timerId)
    opts?.signal?.removeEventListener("abort", abortFromParent)
    controller.signal.removeEventListener("abort", abortListener!)
  }
  if (controller.signal.aborted) cleanup()

  /** 只覆盖当前调用的 fetch；资源 URL 的缓存键必须在 import-html-entry 查缓存之前改变。 */
  const trackedFetch: typeof fetch = (input, init) => {
    return fetch(input, { ...init, signal: controller.signal })
  }
  const check = () => {
    if (opts?.signal?.aborted) throw new DOMException("应用加载已取消", "AbortError")
    if (controller.signal.aborted) throw new Error(`子应用加载超时: ${entryUrl}`)
  }

  try {
    check()
    const entry = await Promise.race([importEntry(importUrl, {
      fetch: trackedFetch,
      getTemplate: (html) => {
        if (generation === 0) return html
        const doc = new DOMParser().parseFromString(html, "text/html")
        const base = new URL(entryUrl, window.location.href)
        for (const script of doc.querySelectorAll<HTMLScriptElement>("script[src]")) {
          script.setAttribute("src", withBuster(new URL(script.getAttribute("src")!, base).toString(), generation))
        }
        for (const link of doc.querySelectorAll<HTMLLinkElement>('link[rel="stylesheet"][href]')) {
          link.setAttribute("href", withBuster(new URL(link.getAttribute("href")!, base).toString(), generation))
        }
        return doc.documentElement.outerHTML
      },
      postProcessTemplate: (template) => ({
        ...template,
        // 生命周期挂载前按声明顺序等待全部脚本，防止 async 脚本在卸载后才执行。
        scripts: template.scripts.map((script) => typeof script === "string" ? script : script.src),
        entry: typeof template.entry === "string" ? template.entry : template.entry?.src,
      }),
    }), abortPromise])
    return {
      templateProcessed: entry.template,
      execScripts: async (proxy: object) => {
        try {
          check()
          const execOptions = {
            fetch: trackedFetch,
            beforeExec: () => { check() },
          }
          const result = await Promise.race([entry.execScripts(proxy, true, execOptions), abortPromise])
          check()
          failedEntries.delete(entryUrl)
          return result
        } catch (error) { invalidate(); throw error }
        finally { cleanup() }
      },
    }
  } catch (e) {
    cleanup()
    invalidate()
    if (opts?.signal?.aborted) throw new DOMException("应用加载已取消", "AbortError")
    throw new Error(normalizeMessage(e, entryUrl))
  }
}

/**
 * 标记入口执行失败(与 fetch 失败同权重计入穿透态)。
 * 场景: 脚本文本 fetch 成功但执行抛错(如子应用语法错误), 修复源文件后重试时
 * scriptCache 仍命中旧文本 —— 必须主动标记才能让下一轮 loadHtmlEntry 追加亡缓存参数重新拉取。
 */
export function markEntryFailed(entryUrl: string): void {
  failedEntries.set(entryUrl, (failedEntries.get(entryUrl) ?? 0) + 1)
  entryGenerations.set(entryUrl, (entryGenerations.get(entryUrl) ?? 0) + 1)
}

/** 诊断: 当前处于失败缓存穿透态的入口(重试将走全新缓存键) */
export function getFailedEntryUrls(): string[] {
  return [...failedEntries.keys()]
}
