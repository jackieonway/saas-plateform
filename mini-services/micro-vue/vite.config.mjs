import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vite'

export default defineConfig({
  root: fileURLToPath(new URL('.', import.meta.url)),
  base: './',
  // A missing version/resource must remain a 404 instead of returning the entry HTML.
  appType: 'mpa',
  envPrefix: 'MICRO_PUBLIC_',
  plugins: [{
    name: 'classic-micro-app-entry',
    // The host executes classic scripts in its lifecycle sandbox. Its loader cannot
    // import the development-only ESM/HMR client; refresh the micro app to reload.
    transformIndexHtml: {
      order: 'post',
      handler: html => html
        .replace(/<script type="module" src="[^"\n]*\/@vite\/client"><\/script>\s*/g, '')
        // Vite dev otherwise rewrites public assets to root-absolute URLs,
        // which would escape /sub/vue/ when the host resolves this entry.
        .replace(/src="\/(?:[^"]*\/)?(vendor\/vue\.global\.prod\.js|vendor\/micro-sdk\.js|app\.js)"/g, 'src="$1"'),
    },
  }],
  server: { host: '127.0.0.1', port: 3005, strictPort: true, hmr: false, cors: true },
  preview: { host: '127.0.0.1', port: 3005, strictPort: true, cors: true },
  build: { outDir: 'dist', emptyOutDir: true },
})
