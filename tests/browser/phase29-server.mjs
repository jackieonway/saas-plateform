import { createServer } from 'node:http'
import { readFileSync, existsSync, statSync } from 'node:fs'
import { gzipSync } from 'node:zlib'
import path from 'node:path'

/** Test-only production asset server: explicit static 404, SPA routes and gzip. */
export async function servePhase29(dist) {
  dist = path.resolve(dist)
  const server = createServer((req, res) => {
    const pathname = new URL(req.url, 'http://localhost').pathname
    if (pathname === '/api/platform/branding') {
      res.setHeader('Content-Type', 'application/json'); return res.end(JSON.stringify({ platformName: '管理平台', loginTitle: '欢迎回来', loginDescription: '登录平台，管理您的租户与应用权限', primaryColor: '#6366f1' }))
    }
    if (pathname.startsWith('/api/')) { res.writeHead(403, { 'Content-Type': 'application/json' }); return res.end('{}') }
    const file = path.join(dist, pathname === '/' || !path.extname(pathname) ? 'index.html' : pathname)
    if (!file.startsWith(dist + path.sep) || !existsSync(file) || !statSync(file).isFile()) { res.writeHead(404, { 'Content-Type': 'text/plain' }); return res.end('Missing static resource') }
    const mime = { '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' }[path.extname(file)] || 'application/octet-stream'
    res.writeHead(200, { 'Content-Type': mime, 'Content-Encoding': 'gzip', 'Cache-Control': 'no-store' })
    res.end(gzipSync(readFileSync(file)))
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  return { origin: `http://127.0.0.1:${server.address().port}`, close: () => new Promise(resolve => server.close(resolve)) }
}
