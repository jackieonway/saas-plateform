import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    port: 3000,
    strictPort: true,
    host: true,
    // 沙箱预览经外部网关域名访问 (如 *.cn-hongkong-vpc.fcapp.run), Host 头非 localhost,
    // Vite 6 默认会拦截 (403 Blocked request), 必须放行全部 Host 才能经 Caddy 网关预览。
    allowedHosts: true,
    proxy: {
      "/api": {
        target: "http://127.0.0.1:3001",
        changeOrigin: true,
      },
      "/oauth": {
        target: "http://127.0.0.1:3001",
        changeOrigin: true,
      },
      // 微前端子应用服务(21-b): /sub/vue/* → 3005(Vue 工坊), /sub/jquery/* → 3006(legacy 工单)
      // 子应用 HTML 里引用 /app.js 等根绝对路径 → 同源加载, 同样命中这两条代理。
      // 必须放在 "/" 键之前("/" 有 bypass 兜底, 键序即匹配优先级)。
      "/sub/vue": {
        target: "http://127.0.0.1:3005",
        changeOrigin: true,
        rewrite: (p) => p.replace(/^\/sub\/vue/, ""),
      },
      "/sub/jquery": {
        target: "http://127.0.0.1:3006",
        changeOrigin: true,
        rewrite: (p) => p.replace(/^\/sub\/jquery/, ""),
      },
      // notification-ws 实时通知 (socket.io, 端口 3003) —— 与 Caddy 网关行为保持一致:
      // 前端固定连 io('/?XTransformPort=3003'), dev 下经该代理转发到 3003 (含 ws upgrade)。
      // bypass: 返回 undefined 走代理; 返回字符串则交还 vite 继续处理 (键序 '/' 必须放最后)。
      // 已核对 vite 6.3.5 源码: ws upgrade 事件同样会调用 bypass, 返回字符串时不代理,
      // vite 自身 HMR (sec-websocket-protocol: vite-hmr) 不受影响。
      "/": {
        target: "http://127.0.0.1:3003",
        ws: true,
        bypass: (req) => {
          const u = req.url || ""
          if (u.includes("XTransformPort=3003")) return undefined
          return u
        },
      },
    },
  },
  build: {
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes("node_modules")) {
            if (id.includes("recharts") || id.includes("d3-")) return "charts";
            if (id.includes("react") || id.includes("scheduler")) return "react-vendor";
            if (id.includes("@radix-ui") || id.includes("cmdk") || id.includes("vaul")) return "ui-vendor";
            return "vendor";
          }
        },
      },
    },
  },
});
