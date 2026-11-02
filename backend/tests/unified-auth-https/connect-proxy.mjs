// Windows browser transport for the task WSL gateway. This is deliberately
// limited to the three public test hosts and never forwards plaintext requests.
import http from 'node:http'
import fs from 'node:fs'
import path from 'node:path'
import { spawn } from 'node:child_process'

const options = Object.fromEntries(process.argv.slice(2).map((value, index, all) =>
  value.startsWith('--') ? [value.slice(2), all[index + 1]] : []).filter(item => item.length))
const task = options['task-dir']
const distribution = options.distribution || 'Ubuntu'
if (!task || !/^\/tmp\/rust-saas-unified-auth-[A-Za-z0-9_-]+$/.test(task)
  || !/^[A-Za-z0-9_-]+$/.test(distribution) || process.platform !== 'win32') {
  throw new Error('An explicit dedicated WSL task directory and Windows host are required')
}
const diskRoot = `\\\\wsl.localhost\\${distribution}${task.replaceAll('/', '\\')}`
if (!fs.existsSync(path.join(diskRoot, 'OWNER'))) throw new Error('Task ownership marker is required')
const allowed = new Set(['auth.saas.test:443', 'console.saas.test:443', 'customer.saas.test:443'])
if (options['additional-public-origin'] !== undefined) {
  if (options['additional-public-origin'] !== 'https://customer.saas.test:24446') throw new Error('Unsupported task public origin')
  allowed.add('customer.saas.test:24446')
}
if (options['business-public-origin'] !== undefined) {
  if (options['business-public-origin'] !== 'https://console.saas.test:24447'
    || !fs.existsSync(path.join(diskRoot, 'phase25-business-manifest.json'))) throw new Error('Unsupported task business origin')
  allowed.add('console.saas.test:24447')
}
const children = new Set()
const sockets = new Set()
const tunnel = `import selectors,socket,sys
port=int(sys.argv[2])
if port not in (443,24446,24447):raise SystemExit(2)
s=socket.create_connection(('127.0.0.2',port),timeout=10);s.settimeout(None)
selector=selectors.DefaultSelector();selector.register(s,selectors.EVENT_READ,0);selector.register(sys.stdin.buffer,selectors.EVENT_READ,1)
try:
 while True:
  for key,_ in selector.select():
   if key.data==0:
    data=s.recv(65536)
    if not data:raise SystemExit(0)
    sys.stdout.buffer.write(data);sys.stdout.buffer.flush()
   else:
    data=sys.stdin.buffer.read1(65536)
    if not data:raise SystemExit(0)
    s.sendall(data)
finally:s.close()`
const server = http.createServer((_request, response) => {
  response.writeHead(405, { 'Content-Length': '0' })
  response.end()
})
server.on('connect', (request, socket, head) => {
  if (!allowed.has(request.url) || head.length > 65536 || sockets.size >= 64) {
    socket.end('HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n')
    return
  }
  // The final argument labels the otherwise inline Python process so any
  // abnormal-exit residue can be attributed to this exact temporary task.
  const port = request.url === 'customer.saas.test:24446' ? '24446' : request.url === 'console.saas.test:24447' ? '24447' : '443'
  const child = spawn('wsl.exe', ['-d', distribution, '--exec', 'python3', '-u', '-c', tunnel, task, port], {
    windowsHide: true, stdio: ['pipe', 'pipe', 'ignore'],
  })
  children.add(child)
  sockets.add(socket)
  socket.setTimeout(30000, () => socket.destroy())
  socket.write('HTTP/1.1 200 Connection Established\r\n\r\n')
  if (head.length) child.stdin.write(head)
  socket.pipe(child.stdin)
  child.stdout.pipe(socket)
  const close = () => {
    children.delete(child)
    sockets.delete(socket)
    socket.destroy()
    child.kill()
  }
  child.on('error', close)
  child.on('exit', close)
  child.stdin.on('error', () => socket.destroy())
  socket.on('error', () => child.kill())
  socket.on('close', () => child.kill())
})
const record = path.join(diskRoot, 'browser-proxy.pid')
const lease = setInterval(() => {
  if (!fs.existsSync(path.join(diskRoot, 'namespace.pid'))) cleanup()
}, 2000)
let stopping = false
function cleanup() {
  if (stopping) return
  stopping = true
  clearInterval(lease)
  try { fs.unlinkSync(record) } catch { /* The task may already have been removed. */ }
  for (const socket of sockets) socket.destroy()
  for (const child of children) child.kill()
  server.close(() => process.exit(0))
  setTimeout(() => process.exit(0), 1500).unref()
}
server.listen(24445, '127.0.0.1', () => {
  fs.writeFileSync(record, JSON.stringify({
    pid: process.pid, startedAt: new Date(Date.now() - process.uptime() * 1000).toISOString(),
    script: path.resolve(process.argv[1]), task,
  }), { flag: 'wx' })
  console.log(`TASK_BROWSER_PROXY_READY loopback=24445 exact_authorities=${allowed.size}`)
})
process.on('SIGINT', cleanup)
process.on('SIGTERM', cleanup)
