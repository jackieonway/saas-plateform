/** Real non-root, read-only production Nginx; retain baseline hashes beside current assets. */
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { cpSync, mkdirSync, readFileSync, writeFileSync, existsSync, copyFileSync } from 'node:fs'
import { randomUUID, createHash } from 'node:crypto'
import path from 'node:path'
const root = process.cwd(), out = path.resolve(process.argv[2] || 'target/phase-29/nginx'), dist = path.join(out, 'dist')
mkdirSync(out, { recursive: true }); cpSync('target/phase-29/after/dist', dist, { recursive: true })
const before = JSON.parse(readFileSync('target/phase-29/before/build.json'))
const oldEntry = before.inventory.find(item => item.isEntry).file
for (const item of before.inventory.filter(item => item.file.startsWith('assets/'))) {
  const target = path.join(dist, item.file), from = path.resolve('target/phase-29/before/dist', item.file)
  mkdirSync(path.dirname(target), { recursive: true })
  if (existsSync(target)) assert(readFileSync(target).equals(readFileSync(from)), 'Same hash path cannot be replaced')
  else copyFileSync(from, target)
}
const owner = 'phase29-nginx-' + randomUUID()
const command = process.platform === 'win32' ? 'wsl.exe' : 'docker'
const prefix = process.platform === 'win32' ? ['-d', 'Ubuntu', '--', 'docker'] : []
const linuxPath = p => process.platform === 'win32' ? '/mnt/' + p[0].toLowerCase() + p.slice(2).replaceAll('\\', '/') : p
const docker = args => {
  const r = spawnSync(command, [...prefix, ...args], { encoding: 'utf8', windowsHide: true, timeout: 60000 })
  if (r.status !== 0) throw new Error('docker ' + args[0] + ': ' + r.stderr)
  return r.stdout.trim()
}
const image = docker(['image', 'inspect', 'nginxinc/nginx-unprivileged:1.28.0-alpine', '--format', '{{.Id}}'])
const records = []; let id, cleanup = false
try {
  id = docker(['run', '-d', '--name', owner, '--label', `saas.phase29.owner=${owner}`, '--read-only', '--user', '101:101',
    '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges:true', '--tmpfs', '/tmp:rw,noexec,nosuid,size=64m,mode=1777',
    '--mount', `type=bind,src=${linuxPath(dist)},dst=/usr/share/nginx/html,readonly`,
    '--mount', `type=bind,src=${linuxPath(path.join(root, 'deploy/static/nginx.conf'))},dst=/etc/nginx/nginx.conf,readonly`,
    '--mount', `type=bind,src=${linuxPath(path.join(root, 'deploy/static/mime.types'))},dst=/etc/nginx/mime.types,readonly`,
    '-p', '127.0.0.1::8080', '--entrypoint', 'nginx', image, '-g', 'daemon off;'])
  docker(['exec', id, 'nginx', '-t'])
  const port = docker(['port', id, '8080/tcp']).match(/:(\d+)$/)[1]
  for (const [uri, accept, status] of [['/login', 'text/html', 200], ['/system/applications', 'text/html', 200],
    ['/assets/obsolete-phase29.js', '*/*', 404], ['/assets/obsolete-phase29.js', 'text/html', 404],
    ['/assets/obsolete-phase29.css', 'text/css', 404], ['/unknown-route', 'text/html', 404], ['/' + oldEntry, '*/*', 200]]) {
    const r = await fetch(`http://127.0.0.1:${port}${uri}`, { headers: { Accept: accept } }), bytes = Buffer.from(await r.arrayBuffer())
    assert.equal(r.status, status)
    const record = { uri, accept, status: r.status, contentType: r.headers.get('content-type'), cacheControl: r.headers.get('cache-control'),
      bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') }
    if (uri.startsWith('/assets/obsolete')) {
      assert.match(record.contentType, /^text\/plain/); assert.equal(record.cacheControl, 'no-store'); assert(!bytes.includes(Buffer.from('<html')))
    }
    if (uri === '/' + oldEntry) assert.equal(record.sha256, before.inventory.find(item => item.file === oldEntry).sha256)
    records.push(record)
  }
} finally {
  if (id) {
    assert.equal(docker(['inspect', id, '--format', '{{index .Config.Labels "saas.phase29.owner"}}']), owner)
    docker(['rm', '-f', id])
    cleanup = docker(['ps', '-aq', '--filter', `label=saas.phase29.owner=${owner}`]) === ''
  }
  writeFileSync(path.join(out, 'result.json'), JSON.stringify({ owner, image, id, records, cleanup }, null, 2) + '\n')
}
assert.equal(records.length, 7); assert(cleanup)
console.log('PASS production Nginx: 7 requests, old asset retained, missing JS/CSS 404 text/plain no-store, owned container removed')
