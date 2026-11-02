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
const workOrdersProxyTarget = process.env.WORK_ORDERS_DEV_PROXY_TARGET || "http://127.0.0.1:3004";
const storesProxyTarget = process.env.STORES_DEV_PROXY_TARGET || "http://127.0.0.1:3031";
const additionalAllowedHosts = [...new Set((process.env.DEV_ALLOWED_HOSTS || "")
  .split(",").map((host) => host.trim().toLowerCase()).filter(Boolean))];
if (additionalAllowedHosts.some((host) => host.length > 253 || host.split(".").some((label) =>
  label.length > 63 || !/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/.test(label)))) {
  throw new Error("DEV_ALLOWED_HOSTS must contain exact hostnames separated by commas, without URLs, ports or wildcards");
}

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
    {
      name: "external-microapp-versions",
      configureServer(server) {
        server.middlewares.use((req, res, next) => {
          const pathname = req.url?.split("?", 1)[0] || "";
          if (!pathname.startsWith("/sub-apps/")) return next();
          const missing = () => { res.statusCode = 404; res.setHeader("Cache-Control", "no-store"); res.end(); };
          const match = /^\/sub-apps\/([a-z0-9-]+)\/versions\/([A-Za-z0-9][A-Za-z0-9._-]*)\/(.+)$/.exec(pathname);
          if (!match || match[2].includes("..") || !["GET", "HEAD"].includes(req.method || "")) return missing();
          let segments: string[];
          try { segments = match[3].split("/").map(decodeURIComponent); } catch { return missing(); }
          if (segments.some((segment) => !segment || segment === "." || segment === ".." ||
            segment.includes("/") || segment.includes("\\") || segment.includes(":") || /[\x00-\x1f]/.test(segment))) return missing();
          try {
            // The optional release store may not exist in a fresh checkout.
            // Resolve it only for a version request so dev can start and missing releases stay 404.
            const root = realpathSync(microAppsDevDir);
            const parts = [match[1], "versions", match[2], ...segments];
            const file = realpathSync(path.resolve(root, ...parts));
            if (!file.startsWith(root + path.sep) || !statSync(file).isFile()) return missing();
            res.setHeader("Content-Type", versionMimeTypes[path.extname(file).toLowerCase()] || "application/octet-stream");
            res.setHeader("Access-Control-Allow-Origin", "*");
            res.setHeader("Cache-Control", path.extname(file).toLowerCase() !== ".html" ? "public, max-age=31536000, immutable" : "no-cache");
            if (req.method === "HEAD") { res.end(); return; }
            createReadStream(file).on("error", () => {
              if (res.headersSent) res.destroy();
              else missing();
            }).pipe(res);
          } catch { missing(); }
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
    // Preserve Vite's localhost/IP defaults; external previews require explicit hostnames.
    allowedHosts: additionalAllowedHosts,
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
      // Cookie/Origin-bound BFF routes must reach the backend under the public
      // console authority; they are never host SPA navigation or legacy OAuth.
      "^/auth(?:/|\\?|$)": {
        target: backendProxyTarget,
        changeOrigin: false,
      },
      // Registered URL resources are resolved by Rust; /micro/* remains host SPA navigation.
      "/micro-proxy": {
        target: backendProxyTarget,
        changeOrigin: true,
      },
      "/work-orders-api": {
        target: workOrdersProxyTarget,
        changeOrigin: true,
      },
      "/stores-api": {
        target: storesProxyTarget,
        changeOrigin: true,
      },
      // Development aliases for the independent Vite compatibility examples.
      // Their entry resources remain relative to /sub/vue/ or /sub/jquery/.
      "/sub/vue": {
        target: process.env.MICRO_VUE_DEV_PROXY_TARGET || "http://127.0.0.1:3005",
        changeOrigin: true,
        rewrite: (p) => p.replace(/^\/sub\/vue/, ""),
      },
      "/sub/jquery": {
        target: process.env.MICRO_JQUERY_DEV_PROXY_TARGET || "http://127.0.0.1:3006",
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
  // circular frontend/vendor chunks: Sonner accessed React before its wrapper initialized.
  // Route-level dynamic imports still keep business pages loaded on demand.
});
