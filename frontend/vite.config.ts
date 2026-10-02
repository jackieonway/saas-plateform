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
      // 各子应用使用明确前缀，与宿主 HMR 和通知连接互不重叠。
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
      // 与生产网关相同的 Socket.IO 路径，保留路径、polling POST 和 WS Upgrade。
      "/socket.io": {
        target: process.env.NOTIFICATION_DEV_PROXY_TARGET || "http://127.0.0.1:3003",
        changeOrigin: true,
        ws: true,
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
