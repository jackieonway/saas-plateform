/**
 * micro-vue — Dante Cloud 微前端子应用服务 (Vue 3, 端口硬编码 3005)
 *
 * 定位: 「仿乾坤」微前端的演示子应用之一, 证明「任意技术栈」兼容性。
 * 主应用通过 Vite 代理同源访问: /sub/vue/* → 127.0.0.1:3005 (去 /sub/vue 前缀转发)。
 * 因此 index.html 中的资源一律使用根绝对路径 (/app.js、/vendor/vue.global.prod.js),
 * 主应用 HTML 入口加载器会以入口 base (/sub/vue/) 重新解析这些资源 URL。
 *
 * 关键实现点:
 * - Bun.serve 静态服务器 (dev 脚本 bun --hot, 依赖已装: npm install vue);
 *   bun --hot 下模块重载时 Bun.serve 会复用同一端口并热替换 fetch handler。
 * - 启动时把 node_modules/vue/dist/vue.global.prod.js 复制到 public/vendor/
 *   (比运行时直读 node_modules 更稳, 不受 node_modules 布局变化影响);
 *   vendor 请求时若目标缺失会再次尝试复制 (幂等兜底)。
 * - 每次请求都从磁盘实时读取 (不做内存缓存), 保证改 public/app.js 立即生效,
 *   无需依赖 --hot 对非 import 文件的监听。
 * - html 响应带 Cache-Control: no-store, 便于主应用开发期迭代;
 *   vendor 产物允许短暂缓存 (5 分钟), app.js 同样 no-store。
 */
import { copyFileSync, existsSync, mkdirSync } from "node:fs"

/* ===== 常量 (端口硬编码, 不读环境变量) ===== */
const PORT = 3005
const ROOT = import.meta.dir
const VENDOR_DIR = `${ROOT}/public/vendor`
const VENDOR_SRC = `${ROOT}/node_modules/vue/dist/vue.global.prod.js`
const VENDOR_DST = `${VENDOR_DIR}/vue.global.prod.js`

/* ===== 启动时复制 vendor 产物 (幂等) ===== */
function ensureVendor(): boolean {
  try {
    if (!existsSync(VENDOR_DST)) {
      mkdirSync(VENDOR_DIR, { recursive: true })
      copyFileSync(VENDOR_SRC, VENDOR_DST)
    }
    return true
  } catch (err) {
    console.error(`[micro-vue] vendor 复制失败 (请先 npm install): ${err}`)
    return false
  }
}
ensureVendor()

/* ===== 响应辅助 ===== */
function textResponse(body: string, contentType: string, cache: string): Response {
  return new Response(body, {
    status: 200,
    headers: {
      "Content-Type": contentType,
      "Cache-Control": cache,
      // 同源经 Vite 代理访问时无影响; 若主应用以绝对 URL 直连本服务(跨源)亦可用
      "Access-Control-Allow-Origin": "*",
    },
  })
}

Bun.serve({
  port: PORT,
  hostname: "0.0.0.0",
  async fetch(req): Promise<Response> {
    const path = new URL(req.url).pathname
    if (req.method !== "GET" && req.method !== "HEAD") {
      return new Response("Method Not Allowed", { status: 405 })
    }
    if (path === "/" || path === "/index.html" || path === "/sub/vue" || path === "/sub/vue/") {
      // HTML 入口: 主应用的微前端加载器会解析其中的 <script src> 并在沙箱执行
      const html = await Bun.file(`${ROOT}/public/index.html`).text()
      return textResponse(html, "text/html; charset=utf-8", "no-store")
    }
    if (path === "/sandbox-test") {
      // 裸页(无任何 script): 供主应用沙箱/加载器联调时单独 fetch 脚本文本做执行实验
      const html = '<!DOCTYPE html><html lang="zh-CN"><head><meta charset="utf-8"><title>micro-vue sandbox-test</title></head><body><div id="app"></div></body></html>'
      return textResponse(html, "text/html; charset=utf-8", "no-store")
    }
    if (path === "/app.js") {
      const js = await Bun.file(`${ROOT}/public/app.js`).text()
      return textResponse(js, "application/javascript; charset=utf-8", "no-store")
    }
    if (path === "/vendor/vue.global.prod.js") {
      if (!ensureVendor()) {
        return new Response("vue.global.prod.js missing (run npm install in micro-vue)", { status: 500 })
      }
      const js = await Bun.file(VENDOR_DST).text()
      return textResponse(js, "application/javascript; charset=utf-8", "public, max-age=300")
    }
    if (path === "/health") {
      return new Response(JSON.stringify({ ok: true, service: "micro-vue", port: PORT }), {
        status: 200,
        headers: { "Content-Type": "application/json; charset=utf-8", "Cache-Control": "no-store" },
      })
    }
    return new Response("Not Found", { status: 404, headers: { "Content-Type": "text/plain; charset=utf-8" } })
  },
})

console.log(`[micro-vue] listening on http://0.0.0.0:${PORT} (Vue 子应用, HTML 入口 + __DANTE_MICRO_APP__ 契约)`)
