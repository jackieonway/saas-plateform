import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { createReadStream, realpathSync, statSync } from "node:fs";
import type { ClientRequest } from "node:http";
import path from "node:path";

const microAppsDevDir = process.env.MICRO_APPS_DEV_DIR || path.resolve(__dirname, "../deploy/micro-apps");
if (!path.isAbsolute(microAppsDevDir)) throw new Error("MICRO_APPS_DEV_DIR must be absolute");
const versionMimeTypes: Record<string, string> = {
  ".html": "text/html; charset=utf-8", ".js": "application/javascript; charset=utf-8",
  ".mjs": "application/javascript; charset=utf-8", ".cjs": "application/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8", ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml", ".png": "image/png", ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg", ".webp": "image/webp", ".gif": "image/gif",
  ".ico": "image/x-icon", ".woff": "font/woff", ".woff2": "font/woff2",
  ".txt": "text/plain; charset=utf-8", ".map": "application/json; charset=utf-8",
};
const stripMapProxyCredentials = (request: ClientRequest) => {
  request.removeHeader("authorization");
  request.removeHeader("cookie");
  request.removeHeader("x-real-ip");
  request.removeHeader("x-forwarded-for");
};
const mapProxyHeaders = { "User-Agent": "SaaSPlatform/1.0 map-resource-proxy" };
const backendProxyTarget = process.env.BACKEND_PROXY_TARGET || "http://127.0.0.1:3001";

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
    {
      name: "external-microapp-versions",
      configureServer(server) {
        const root = realpathSync(microAppsDevDir);
        server.middlewares.use((req, res, next) => {
          const pathname = req.url?.split("?", 1)[0] || "";
          if (!/^\/sub-apps\/[^/]+\/versions\//.test(pathname)) return next();
          const missing = () => { res.statusCode = 404; res.setHeader("Cache-Control", "no-store"); res.end(); };
          const match = /^\/sub-apps\/([a-z0-9-]+)\/versions\/([A-Za-z0-9][A-Za-z0-9._-]*)\/(.+)$/.exec(pathname);
          if (!match || match[2].includes("..") || !["GET", "HEAD"].includes(req.method || "")) return missing();
          let segments: string[];
          try { segments = match[3].split("/").map(decodeURIComponent); } catch { return missing(); }
          if (segments.some((segment) => !segment || segment === "." || segment === ".." ||
            segment.includes("/") || segment.includes("\\") || segment.includes(":") || /[\x00-\x1f]/.test(segment))) return missing();
          try {
            const file = realpathSync(path.resolve(root, match[1], "versions", match[2], ...segments));
            if (!file.startsWith(root + path.sep) || !statSync(file).isFile()) return missing();
            res.setHeader("Content-Type", versionMimeTypes[path.extname(file).toLowerCase()] || "application/octet-stream");
            res.setHeader("Access-Control-Allow-Origin", "*");
            res.setHeader("Cache-Control", path.extname(file).toLowerCase() === ".html" ? "no-cache" : "public, max-age=31536000, immutable");
            if (req.method === "HEAD") { res.end(); return; }
            createReadStream(file).on("error", () => {
              if (res.headersSent) res.destroy();
              else missing();
            }).pipe(res);
          } catch { missing(); }
        });
      },
    },
    {
      name: "opaque-microapp-api-preflight",
      configureServer(server) {
        server.middlewares.use((req, res, next) => {
          const pathname = req.url?.split("?", 1)[0];
          const externalView = pathname === "/api/external/menus" || pathname === "/api/external/roles";
          const workOrdersApi = pathname === "/work-orders-api" || pathname?.startsWith("/work-orders-api/");
          if (req.method !== "OPTIONS" || req.headers.origin !== "null" || (!externalView && !workOrdersApi)) {
            next();
            return;
          }
          res.statusCode = 204;
          res.setHeader("Vary", "Origin, Access-Control-Request-Method, Access-Control-Request-Headers");
          res.setHeader("Access-Control-Allow-Origin", "null");
          res.setHeader("Access-Control-Allow-Methods", externalView ? "GET" : "GET,POST,PATCH,DELETE");
          res.setHeader("Access-Control-Allow-Headers", "authorization,content-type,x-request-id");
          res.end();
        });
      },
    },
  ],
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
      // Development host can reach the public map services even when WSL's
      // backend cannot. Production /api/maps/* is served by the Rust gateway.
      "/api/maps/tiles": {
        target: "https://tile.openstreetmap.org",
        changeOrigin: true,
        headers: mapProxyHeaders,
        rewrite: (p) => p.replace(/^\/api\/maps\/tiles/, ""),
        configure: (proxy) => proxy.on("proxyReq", stripMapProxyCredentials),
      },
      "/api/maps/search": {
        target: "https://nominatim.openstreetmap.org",
        changeOrigin: true,
        headers: mapProxyHeaders,
        rewrite: (p) => p.replace(/^\/api\/maps\/search/, "/search"),
        configure: (proxy) => proxy.on("proxyReq", stripMapProxyCredentials),
      },
      "/api/maps/reverse": {
        target: "https://nominatim.openstreetmap.org",
        changeOrigin: true,
        headers: mapProxyHeaders,
        rewrite: (p) => p.replace(/^\/api\/maps\/reverse/, "/reverse"),
        configure: (proxy) => proxy.on("proxyReq", stripMapProxyCredentials),
      },
      "/api/maps/ip-location": {
        target: "https://ipwho.is",
        changeOrigin: true,
        headers: mapProxyHeaders,
        rewrite: () => "/",
        configure: (proxy) => proxy.on("proxyReq", stripMapProxyCredentials),
      },
      "/api": {
        target: backendProxyTarget,
        changeOrigin: true,
      },
      "/oauth": {
        target: backendProxyTarget,
        changeOrigin: true,
      },
      "/work-orders-api": {
        target: "http://127.0.0.1:3104",
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
  // Let Rollup derive shared chunks from the actual module graph. Splitting
  // React, its UI consumers and their helpers by package-name substrings made
  // circular vendor chunks: Sonner accessed React before its wrapper initialized.
  // Route-level dynamic imports still keep business pages loaded on demand.
});
