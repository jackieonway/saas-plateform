import { posix } from 'node:path';
import { defineConfig } from 'vite';
import { parseChatEndpoint } from './src/chat-config.ts';

// Expose this single public value; never copy the process environment or load local secrets.
const endpoint = parseChatEndpoint(process.env.VITE_CHAT_ENDPOINT);

export default defineConfig({
  base: './',
  appType: 'mpa',
  publicDir: false,
  envDir: false,
  envPrefix: [],
  define: { 'import.meta.env.VITE_CHAT_ENDPOINT': JSON.stringify(endpoint ?? '') },
  plugins: [{
    name: 'static-demo-boundary',
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        let pathname: string;
        try {
          pathname = posix.normalize(decodeURIComponent((request.url ?? '').split('?', 1)[0]));
        } catch {
          response.statusCode = 404;
          response.end();
          return;
        }
        if (pathname.endsWith('/server.ts') || pathname === '/backend' || pathname.startsWith('/backend/') || /(?:^|\/)\.env(?:\.|$)/.test(pathname)) {
          response.statusCode = 404;
          response.end();
          return;
        }
        next();
      });
    },
  }],
  server: {
    host: '127.0.0.1', port: 3007, strictPort: true,
    fs: { deny: ['.env', '.env.*', '*.{crt,pem}', '**/.git/**', '**/server.ts', '**/backend/**'] },
  },
  preview: { host: '127.0.0.1', port: 3007, strictPort: true },
  build: { outDir: 'dist', emptyOutDir: true, sourcemap: false },
});
