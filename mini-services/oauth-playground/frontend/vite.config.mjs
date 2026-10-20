import { fileURLToPath } from 'node:url'
import { defineConfig, loadEnv } from 'vite'

export default defineConfig(({ mode }) => {
  const root = fileURLToPath(new URL('.', import.meta.url))
  const env = loadEnv(mode, root, '')
  const target = process.env.OAUTH_PLAYGROUND_API_TARGET || env.OAUTH_PLAYGROUND_API_TARGET || 'http://127.0.0.1:3011'
  const port = Number(process.env.OAUTH_PLAYGROUND_FRONTEND_PORT || env.OAUTH_PLAYGROUND_FRONTEND_PORT || 3010)
  const proxy = { '/api': { target, changeOrigin: false }, '/health': { target, changeOrigin: false } }

  return {
    root,
    base: '/',
    appType: 'mpa',
    // Public configuration is fetched at runtime. No environment variables are
    // embedded into browser assets, including variables prefixed with VITE_.
    envPrefix: 'OAUTH_PLAYGROUND_PUBLIC_UNUSED_',
    server: { host: '127.0.0.1', port, strictPort: true, proxy },
    preview: { host: '127.0.0.1', port, strictPort: true, proxy },
    build: {
      outDir: 'dist',
      emptyOutDir: true,
      rolldownOptions: {
        input: {
          main: fileURLToPath(new URL('index.html', import.meta.url)),
          callback: fileURLToPath(new URL('callback.html', import.meta.url)),
        },
      },
    },
  }
})
