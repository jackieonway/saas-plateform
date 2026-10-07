import assert from 'node:assert/strict'
import test from 'node:test'
import { spawn, spawnSync } from 'node:child_process'
import { createServer, request } from 'node:http'
import { createRequire } from 'node:module'
import {
  existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync,
} from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const tempBase = realpathSync(os.tmpdir())
const tempPrefix = 'dante-micro-release-gateways-test-'
const ownedTempRoots = new Set()
const caddyBinary = toolPath('CADDY_BIN', 'target/phase9-gateway-tools/caddy/caddy.exe')
const nginxBinary = toolPath('NGINX_BIN', 'target/phase9-gateway-tools/nginx/nginx-1.28.0/nginx.exe')
const hostHtml = '<!doctype html><html><body>HOST_SPA_FIXTURE</body></html>'
const releaseHtml = '<!doctype html><html><head><link rel="stylesheet" href="./style.css"></head><body>VERSION_1_0_0<script src="./app.js"></script></body></html>'
const releaseScript = 'globalThis.gatewayRelease = "VERSION_1_0_0";\n'
const releaseEsm = 'export const gatewayRelease = "VERSION_1_0_0";\n'
const releaseCommonJs = 'module.exports = { gatewayRelease: "VERSION_1_0_0" };\n'
const releaseStyle = '.gateway-release { color: #123456; }\n'
const legacyHtml = '<!doctype html><html><body>LEGACY_MICRO_FIXTURE</body></html>'
const SOCKET_ORIGIN = 'https://gateway-socket-fixture.example.test'
const SOCKET_PATH = '/socket.io/'
const SOCKET_QUERY = 'versions/1.0.0?label=测试'
const notificationRequire = createRequire(path.join(workspace, 'mini-services/notification-ws/package.json'))

function toolPath(variable, fallback) {
  const binary = path.resolve(process.env[variable] || path.join(workspace, fallback))
  return existsSync(binary) ? binary : null
}

const slashPath = (value) => value.split(path.sep).join('/')
const quotedPath = (value) => `"${slashPath(value)}"`
const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds))

function safeCleanup(root) {
  const absolute = path.resolve(root)
  const relative = path.relative(tempBase, absolute)
  assert.ok(ownedTempRoots.has(absolute), 'cleanup must target a root created by this test')
  assert.ok(relative && !relative.startsWith('..') && !path.isAbsolute(relative), 'cleanup must remain inside the known temp directory')
  assert.equal(path.dirname(absolute), tempBase)
  assert.ok(path.basename(absolute).startsWith(tempPrefix))
  assert.ok(!lstatSync(absolute).isSymbolicLink(), 'cleanup root must not be replaced by a link')
  rmSync(absolute, { recursive: true, force: true })
  ownedTempRoots.delete(absolute)
}

function writeTree(root, files) {
  for (const [relative, contents] of Object.entries(files)) {
    const destination = path.join(root, relative)
    mkdirSync(path.dirname(destination), { recursive: true })
    writeFileSync(destination, contents)
  }
}

function upstreamRequest(port, url, { timeout = 2000, host = '127.0.0.1', headers = {}, method = 'GET', body } = {}) {
  return new Promise((resolve, reject) => {
    const bytes = body === undefined ? undefined : Buffer.from(body)
    const outgoing = request({ host, port, path: url, method, agent: false,
      headers: bytes ? { ...headers, 'Content-Length': bytes.length } : headers }, (response) => {
      const chunks = []
      response.on('data', (chunk) => chunks.push(chunk))
      response.on('error', reject)
      response.on('end', () => {
        const bodyBytes = Buffer.concat(chunks)
        resolve({ status: response.statusCode, headers: response.headers, body: bodyBytes.toString('utf8'), bodyBytes })
      })
    })
    outgoing.on('error', reject)
    outgoing.setTimeout(timeout, () => outgoing.destroy(new Error(`HTTP ${url} timed out`)))
    outgoing.end(bytes)
  })
}

const get = (port, url, timeout = 2000, host = '127.0.0.1', headers = {}) => upstreamRequest(port, url, { timeout, host, headers })

function listen(server, host = '127.0.0.1') {
  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, host, () => {
      server.off('error', reject)
      resolve(server.address().port)
    })
  })
}

async function unusedPort(host) {
  const server = createServer()
  const port = await listen(server, host)
  await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()))
  return port
}

function socketTransportFixture() {
  // Independent echo transport fixture; no platform authentication or real
  // notification authorization is exercised by these proxy tests.
  const { Server } = notificationRequire('socket.io')
  const { io: createClient } = notificationRequire('socket.io-client')
  const httpServer = createServer((_, response) => { response.writeHead(404); response.end('TRANSPORT_FIXTURE_ONLY') })
  const io = new Server(httpServer, {
    path: SOCKET_PATH, serveClient: false,
    cors: { origin: [SOCKET_ORIGIN], methods: ['GET', 'POST'], credentials: false },
    allowRequest: (incoming, callback) => callback(null, incoming.headers.origin === SOCKET_ORIGIN),
  })
  const requests = []
  const clients = new Set()
  let connections = 0
  const record = (incoming, kind) => requests.push({ kind, method: incoming.method, url: incoming.url, origin: incoming.headers.origin })
  httpServer.prependListener('request', (incoming) => record(incoming, 'http'))
  httpServer.prependListener('upgrade', (incoming, socket) => {
    record(incoming, 'upgrade')
    if (incoming.headers.origin !== SOCKET_ORIGIN) {
      socket.end('HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n')
      socket.destroy()
    }
  })
  io.on('connection', (socket) => {
    connections++
    socket.on('gateway:echo', (payload, acknowledge) => {
      const packet = { fixture: 'SOCKET_IO_TRANSPORT_ONLY', payload, transport: socket.conn.transport.name }
      if (typeof acknowledge === 'function') acknowledge(packet)
      socket.emit('gateway:packet', packet)
    })
  })
  return {
    httpServer, io, createClient, clients, requests, connections: () => connections,
    disconnectClients: () => { for (const client of clients) client.disconnect(); clients.clear() },
    stop: () => new Promise((resolve) => io.close(resolve)),
  }
}

async function stopChild(service) {
  if (!service) return
  const child = service.child
  if (child.exitCode === null && child.signalCode === null) {
    if (service.shutdown) service.shutdown()
    else child.kill('SIGTERM')
  }
  await Promise.race([service.exited, delay(2000)])
  if (child.exitCode === null && child.signalCode === null) {
    if (process.platform === 'win32' && service.shutdown) {
      // The only process tree targeted here belongs to our directly spawned master.
      const taskkill = path.join(process.env.SystemRoot || 'C:/Windows', 'System32', 'taskkill.exe')
      spawnSync(taskkill, ['/PID', String(child.pid), '/T', '/F'], { windowsHide: true, encoding: 'utf8', timeout: 3000 })
    } else child.kill('SIGKILL')
    await Promise.race([service.exited, delay(2000)])
  }
  assert.ok(child.exitCode !== null || child.signalCode !== null, 'test-owned gateway must exit before deleting its files')
}

async function gatewayFixture(t, name) {
  const root = realpathSync(mkdtempSync(path.join(tempBase, tempPrefix)))
  ownedTempRoots.add(root)
  const hostRoot = path.join(root, 'host')
  const releaseRoot = path.join(root, 'micro-apps')
  writeTree(hostRoot, { 'index.html': hostHtml, 'sub-apps/foo/index.html': legacyHtml })
  writeTree(releaseRoot, {
    'foo/versions/1.0.0/index.html': releaseHtml,
    'foo/versions/1.0.0/app.js': releaseScript,
    'foo/versions/1.0.0/entry.mjs': releaseEsm,
    'foo/versions/1.0.0/chunk.cjs': releaseCommonJs,
    'foo/versions/1.0.0/style.css': releaseStyle,
  })
  const upstream = createServer((incoming, outgoing) => {
    outgoing.writeHead(200, { 'Content-Type': 'application/json' })
    outgoing.end(JSON.stringify({ method: incoming.method, url: incoming.url, fixture: 'FAKE_BACKEND' }))
  })
  // Transport-only fixture, independent from the platform and notification
  // upstreams; actual business authorization is tested by its platform E2E.
  const businessUpstream = createServer(async (incoming, outgoing) => {
    const chunks = []
    for await (const chunk of incoming) chunks.push(chunk)
    const missing = incoming.url === '/work-orders-api' || incoming.url.startsWith('/work-orders-api/missing')
    outgoing.writeHead(missing ? 404 : 200, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' })
    outgoing.end(JSON.stringify({ fixture: 'WORK_ORDERS_TRANSPORT_ONLY', method: incoming.method,
      url: incoming.url, body: Buffer.concat(chunks).toString('utf8'), authorization: incoming.headers.authorization,
      origin: incoming.headers.origin }))
  })
  let service
  let socketFixture
  t.after(async () => {
    socketFixture?.disconnectClients()
    try {
      await stopChild(service)
    } finally {
      try { await socketFixture?.stop() } finally {
        if (businessUpstream.listening) {
          businessUpstream.closeAllConnections()
          await new Promise((resolve, reject) => businessUpstream.close((error) => error ? reject(error) : resolve()))
        }
        if (upstream.listening) {
          upstream.closeAllConnections()
          await new Promise((resolve, reject) => upstream.close((error) => error ? reject(error) : resolve()))
        }
      }
    }
    safeCleanup(root)
  })
  // Prefer the repository's Windows IPv6 test route for the native Nginx binary.
  const host = name === 'Nginx' && process.platform === 'win32' ? '::1' : '127.0.0.1'
  const originHost = host.includes(':') ? `[${host}]` : host
  const backendPort = await listen(upstream, host)
  const businessPort = await listen(businessUpstream, host)
  socketFixture = socketTransportFixture()
  const socketPort = await listen(socketFixture.httpServer, host)
  const gatewayPort = await unusedPort(host)
  const env = {
    ...process.env,
    XDG_CONFIG_HOME: path.join(root, 'caddy-config'),
    XDG_DATA_HOME: path.join(root, 'caddy-data'),
    APPDATA: path.join(root, 'appdata'),
    LOCALAPPDATA: path.join(root, 'localappdata'),
  }
  let config
  let configFile
  let binary
  let validationArgs
  let startArgs
  if (name === 'Caddy') {
    binary = caddyBinary
    configFile = path.join(root, 'Caddyfile')
    const source = readFileSync(path.join(workspace, 'deploy/Caddyfile'), 'utf8')
    assert.match(source, /^:80\s*\{/m, 'Caddy test must adapt the current production listener')
    assert.match(source, /root \* \/micro-apps/)
    assert.match(source, /root \* \/srv\/sub-apps/)
    assert.match(source, /reverse_proxy notification-ws:3003/, 'Caddy must retain the Socket.IO proxy upstream')
    assert.match(source, /reverse_proxy work-orders-api:3004/, 'Caddy must retain the independent business API upstream')
    config = source
      .replace(/^:80\s*\{/m, `http://127.0.0.1:${gatewayPort} {`)
      .replaceAll('backend:3001', `127.0.0.1:${backendPort}`)
      .replaceAll('notification-ws:3003', `127.0.0.1:${socketPort}`)
      .replaceAll('work-orders-api:3004', `127.0.0.1:${businessPort}`)
      .replaceAll('root * /micro-apps', `root * ${quotedPath(releaseRoot)}`)
      .replaceAll('root * /srv/sub-apps', `root * ${quotedPath(path.join(hostRoot, 'sub-apps'))}`)
      .replaceAll('root * /srv', `root * ${quotedPath(hostRoot)}`)
    validationArgs = ['validate', '--config', configFile, '--adapter', 'caddyfile']
    startArgs = ['run', '--config', configFile, '--adapter', 'caddyfile']
  } else {
    binary = nginxBinary
    configFile = path.join(root, 'nginx.conf')
    mkdirSync(path.join(root, 'logs'))
    const nginxTemp = path.join(root, 'nginx-temp')
    mkdirSync(nginxTemp)
    const temporaryPaths = ['client_body', 'proxy', 'fastcgi', 'uwsgi', 'scgi']
      .map((kind) => `    ${kind}_temp_path ${quotedPath(path.join(nginxTemp, kind))};`)
      .join('\n')
    const mimeSource = [path.join(path.dirname(binary), 'conf', 'mime.types'), path.resolve(path.dirname(binary), '../conf/mime.types'), path.join(path.dirname(binary), 'mime.types'), '/etc/nginx/mime.types'].find(existsSync)
    assert.ok(mimeSource, 'Nginx binary distribution must provide mime.types')
    const mimeFile = path.join(root, 'mime.types')
    writeFileSync(mimeFile, readFileSync(mimeSource))
    const source = readFileSync(path.join(workspace, 'deploy/nginx.conf'), 'utf8')
    assert.match(source, /listen 80;/)
    assert.match(source, /alias \/srv\/dante\/micro-apps\//)
    assert.match(source, /root \/srv\/dante\/frontend\/dist;/)
    assert.match(source, /proxy_pass http:\/\/127\.0\.0\.1:3003;/, 'Nginx must retain the Socket.IO proxy upstream')
    assert.match(source, /proxy_pass http:\/\/127\.0\.0\.1:3004;/, 'Nginx must retain the independent business API upstream')
    config = `daemon off;\npid ${quotedPath(path.join(root, 'nginx.pid'))};\nerror_log ${quotedPath(path.join(root, 'nginx-error.log'))} notice;\n` + source
      .replace('worker_processes auto;', 'worker_processes 1;')
      .replace('listen 80;', `listen ${originHost}:${gatewayPort};`)
      .replace('include mime.types;', `include ${quotedPath(mimeFile)};`)
      .replace('http {', `http {\n    access_log ${quotedPath(path.join(root, 'nginx-access.log'))};\n${temporaryPaths}`)
      .replaceAll('http://127.0.0.1:3001', `http://${originHost}:${backendPort}`)
      .replaceAll('http://127.0.0.1:3003', `http://${originHost}:${socketPort}`)
      .replaceAll('http://127.0.0.1:3004', `http://${originHost}:${businessPort}`)
      .replaceAll('root /srv/dante/frontend/dist;', `root ${quotedPath(hostRoot)};`)
      .replaceAll('alias /srv/dante/micro-apps/$1/versions/$2/$3;', `alias "${slashPath(releaseRoot)}/$1/versions/$2/$3";`)
    const prefix = `${slashPath(root)}/`
    validationArgs = ['-t', '-p', prefix, '-c', configFile]
    startArgs = ['-p', prefix, '-c', configFile]
  }
  writeFileSync(configFile, config)
  const validation = spawnSync(binary, validationArgs, { cwd: root, encoding: 'utf8', timeout: 5000, windowsHide: true, env })
  assert.ifError(validation.error)
  assert.equal(validation.status, 0, `${name} must validate the adapted production configuration:\n${validation.stderr}\n${validation.stdout}`)
  const child = spawn(binary, startArgs, { cwd: root, windowsHide: true, env, stdio: ['ignore', 'pipe', 'pipe'] })
  let logs = ''
  let startupError
  child.stdout.on('data', (value) => { logs = (logs + value.toString()).slice(-32_768) })
  child.stderr.on('data', (value) => { logs = (logs + value.toString()).slice(-32_768) })
  child.on('error', (error) => { startupError = error })
  service = { child, exited: new Promise((resolve) => child.once('exit', resolve)) }
  if (name === 'Nginx') service.shutdown = () => {
    const pidFile = path.join(root, 'nginx.pid')
    if (existsSync(pidFile)) {
      assert.equal(Number(readFileSync(pidFile, 'utf8').trim()), child.pid, 'only the test-owned Nginx master may receive a stop signal')
      const stopped = spawnSync(binary, [...startArgs, '-s', 'quit'], { cwd: root, env, windowsHide: true, encoding: 'utf8', timeout: 3000 })
      assert.ifError(stopped.error)
      assert.equal(stopped.status, 0, `test-owned Nginx quit failed: ${stopped.stderr}`)
    } else child.kill('SIGTERM')
  }
  const deadline = Date.now() + 8000
  let lastError
  while (Date.now() < deadline) {
    assert.ifError(startupError)
    assert.ok(child.exitCode === null && child.signalCode === null, `${name} exited during startup:\n${logs}`)
    try {
      const response = await get(gatewayPort, '/', 400, host)
      if (response.status === 200) return { port: gatewayPort, host, validation, socketFixture }
      lastError = new Error(`readiness returned HTTP ${response.status}`)
    } catch (error) { lastError = error }
    await delay(75)
  }
  const errorLog = path.join(root, 'nginx-error.log')
  if (existsSync(errorLog)) logs += `\n${readFileSync(errorLog, 'utf8')}`
  if (name === 'Nginx' && process.platform === 'win32') {
    const netstat = path.join(process.env.SystemRoot || 'C:/Windows', 'System32', 'netstat.exe')
    const snapshot = spawnSync(netstat, ['-ano'], { windowsHide: true, encoding: 'utf8', timeout: 3000 })
    const ownPortLines = snapshot.stdout?.split(/\r?\n/).filter((line) => line.trim().split(/\s+/)[1]?.endsWith(`:${gatewayPort}`)).join('\n')
    logs += `\nTest-owned Nginx master PID ${child.pid}; listener snapshot: ${ownPortLines || snapshot.stderr || snapshot.error?.message || '(none)'}`
  }
  assert.fail(`${name} did not become ready within 8 seconds: ${lastError?.message}\n${logs}`)
}

function cacheHeader(response) {
  assert.equal(typeof response.headers['cache-control'], 'string', 'response must state its cache policy')
  return response.headers['cache-control']
}

async function checkHttpContracts(t, port, host) {
  const read = (uri, headers = {}) => get(port, uri, 2000, host, headers)
  for (const uri of ['/sub-apps/foo/versions/1.0.0/index.html', '/sub-apps/foo/versions/1.0.0/']) {
    await t.test(`${uri} serves version HTML with no-cache`, async () => {
      const response = await read(uri)
      assert.equal(response.status, 200)
      assert.equal(response.body, releaseHtml)
      assert.match(response.headers['content-type'], /^text\/html\b/i)
      assert.equal(cacheHeader(response), 'no-cache')
    })
  }
  for (const [file, contents, mime] of [
    ['app.js', releaseScript, /^(?:text|application)\/javascript\b/i],
    ['entry.mjs', releaseEsm, /^(?:text|application)\/javascript\b/i],
    ['chunk.cjs', releaseCommonJs, /^(?:text|application)\/javascript\b/i],
    ['style.css', releaseStyle, /^text\/css\b/i],
  ]) {
    await t.test(`${file} retains correct MIME, immutable caching and public CORS for an opaque iframe origin`, async () => {
      const response = await read(`/sub-apps/foo/versions/1.0.0/${file}`, { Origin: 'null' })
      assert.equal(response.status, 200)
      assert.equal(response.body, contents)
      assert.deepEqual(response.bodyBytes, Buffer.from(contents), 'static response must preserve the exact release bytes')
      assert.match(response.headers['content-type'], mime)
      assert.match(cacheHeader(response), /\bpublic\b/)
      assert.match(cacheHeader(response), /\bmax-age=31536000\b/)
      assert.match(cacheHeader(response), /\bimmutable\b/)
      assert.equal(response.headers['access-control-allow-origin'], '*')
      assert.equal(response.headers['access-control-allow-credentials'], undefined, 'public version resources must not enable credentialed CORS')
    })
  }
  for (const uri of ['/sub-apps/foo/versions/1.0.0/missing.js', '/sub-apps/foo/versions/1.0.0/missing.html', '/sub-apps/foo/versions/9.9.9/index.html']) {
    await t.test(`missing ${uri} is a strict 404 with no-store`, async () => {
      const response = await read(uri)
      assert.equal(response.status, 404)
      assert.equal(cacheHeader(response), 'no-store')
      assert.ok(!response.body.includes('HOST_SPA_FIXTURE'))
      assert.ok(!response.body.includes('VERSION_1_0_0'))
    })
  }
  await t.test('host history route falls back to the host SPA', async () => {
    const response = await read('/tenant/orders/42')
    assert.equal(response.status, 200)
    assert.equal(response.body, hostHtml)
    assert.match(response.headers['content-type'], /^text\/html\b/i)
    assert.equal(cacheHeader(response), 'no-cache')
  })
  for (const uri of ['/sub-apps/foo/index.html', '/sub-apps/foo/']) {
    await t.test(`legacy ${uri} serves its own HTML with no-cache`, async () => {
      const response = await read(uri)
      assert.equal(response.status, 200)
      assert.equal(response.body, legacyHtml)
      assert.match(response.headers['content-type'], /^text\/html\b/i)
      assert.equal(cacheHeader(response), 'no-cache')
    })
  }
  for (const uri of ['/sub-apps/foo/missing.js', '/sub-apps/foo/missing.html']) {
    await t.test(`legacy missing ${uri} returns 404 and no-store`, async () => {
      const response = await read(uri)
      assert.equal(response.status, 404)
      assert.equal(cacheHeader(response), 'no-store')
      assert.ok(!response.body.includes('HOST_SPA_FIXTURE'))
      assert.ok(!response.body.includes('LEGACY_MICRO_FIXTURE'))
    })
  }
  for (const uri of ['/api/gateway-probe?x=1&path=%2Fsub-apps%2Ffoo', '/oauth/token?from=gateway']) {
    await t.test(`${uri} preserves the complete original upstream path`, async () => {
      const response = await read(uri, { Origin: 'null' })
      assert.equal(response.status, 200)
      assert.match(response.headers['content-type'], /^application\/json\b/i)
      assert.deepEqual(JSON.parse(response.body), { method: 'GET', url: uri, fixture: 'FAKE_BACKEND' })
      assert.equal(response.headers['access-control-allow-origin'], undefined, 'public version resource CORS must not extend to API or OAuth routes')
    })
  }
}

async function socketUntil(predicate, message, timeout = 3000) {
  const deadline = Date.now() + timeout
  while (!predicate()) {
    assert.ok(Date.now() < deadline, message)
    await delay(10)
  }
}

async function checkBusinessProxyContracts(t, gateway) {
  const authorization = 'Bearer WORK_ORDERS_TRANSPORT_FIXTURE_ONLY'
  const headers = { Authorization: authorization, Origin: 'null', 'Content-Type': 'application/json' }
  for (const [method, uri, body] of [
    ['GET', '/work-orders-api/health', undefined],
    ['GET', '/work-orders-api/tickets?status=OPEN&search=%E5%B7%A5%E5%8D%95%2Fone', undefined],
    ['POST', '/work-orders-api/tickets?source=iframe', '{"title":"实际前缀代理","priority":"HIGH"}'],
    ['PATCH', '/work-orders-api/tickets/fixture-id?version=2', '{"version":2,"status":"IN_PROGRESS"}'],
    ['DELETE', '/work-orders-api/tickets/fixture-id', '{"version":3}'],
  ]) await t.test(`work-orders transport fixture: ${method} ${uri} preserves prefix/query/body and application Bearer`, async () => {
    const response = await upstreamRequest(gateway.port, uri, { host: gateway.host, method, headers, body })
    assert.equal(response.status, 200)
    assert.deepEqual(JSON.parse(response.body), { fixture: 'WORK_ORDERS_TRANSPORT_ONLY', method, url: uri,
      body: body ?? '', authorization, origin: 'null' })
    assert.equal(response.headers['access-control-allow-origin'], undefined, 'public version CORS must not extend to the business API')
    assert.equal(response.headers['access-control-allow-credentials'], undefined)
    assert.equal(cacheHeader(response), 'no-store')
  })
  for (const uri of ['/work-orders-api', '/work-orders-api/missing?source=iframe']) {
    await t.test(`work-orders transport fixture: missing ${uri} remains an API 404`, async () => {
      const response = await upstreamRequest(gateway.port, uri, { host: gateway.host, headers })
      assert.equal(response.status, 404)
      assert.equal(JSON.parse(response.body).fixture, 'WORK_ORDERS_TRANSPORT_ONLY')
      assert.ok(!response.body.includes('HOST_SPA_FIXTURE'))
      assert.equal(response.headers['access-control-allow-origin'], undefined)
      assert.equal(cacheHeader(response), 'no-store')
    })
  }
}

async function checkSocketProxyContracts(t, gateway) {
  const fixture = gateway.socketFixture
  const originHost = gateway.host.includes(':') ? `[${gateway.host}]` : gateway.host
  const base = `http://${originHost}:${gateway.port}`
  function open(caseName, options = {}) {
    const socket = fixture.createClient(base, {
      path: SOCKET_PATH, query: { gatewayCase: caseName, queryMarker: SOCKET_QUERY },
      extraHeaders: { Origin: SOCKET_ORIGIN }, transports: ['websocket'],
      reconnection: false, forceNew: true, timeout: 1500, autoConnect: false, ...options,
    })
    fixture.clients.add(socket)
    const state = { socket, connectErrors: [], upgradeErrors: [], upgrades: [], packets: [] }
    socket.on('connect_error', (error) => state.connectErrors.push(error))
    socket.on('gateway:packet', (packet) => state.packets.push(packet))
    socket.connect()
    socket.io.engine.on('upgradeError', (error) => state.upgradeErrors.push(error))
    socket.io.engine.on('upgrade', (transport) => state.upgrades.push(transport.name))
    return state
  }
  const caseRequests = (caseName) => fixture.requests.filter((entry) => new URL(entry.url, base).searchParams.get('gatewayCase') === caseName)
  function preservedRequests(caseName) {
    const entries = caseRequests(caseName)
    assert.ok(entries.length > 0, 'the transport fixture must observe the actual proxied requests')
    for (const entry of entries) {
      const url = new URL(entry.url, base)
      assert.equal(url.pathname, SOCKET_PATH, 'proxy must retain the complete Socket.IO path')
      assert.equal(url.searchParams.get('EIO'), '4')
      assert.equal(url.searchParams.get('queryMarker'), SOCKET_QUERY, 'proxy must preserve encoded query parameters')
      assert.equal(entry.origin, SOCKET_ORIGIN, 'proxy must preserve the exact browser Origin')
    }
    return entries
  }
  async function echo(state, caseName, transport) {
    const payload = { caseName, text: '双向 packet', bytes: [0, 255], nested: { marker: SOCKET_QUERY } }
    const reply = await new Promise((resolve, reject) => state.socket.timeout(1500).emit('gateway:echo', payload, (error, response) => error ? reject(error) : resolve(response)))
    await socketUntil(() => state.packets.length === 1, 'the server-to-client packet must also traverse the gateway')
    const expected = { fixture: 'SOCKET_IO_TRANSPORT_ONLY', payload, transport }
    assert.deepEqual(reply, expected)
    assert.deepEqual(state.packets, [expected])
  }
  for (const [caseName, transports, expectedTransport] of [
    ['polling-upgrade', ['polling', 'websocket'], 'websocket'],
    ['pure-polling', ['polling'], 'polling'],
    ['pure-websocket', ['websocket'], 'websocket'],
  ]) await t.test(`Socket.IO transport fixture: ${caseName} preserves path/query/Origin and duplex packets`, async () => {
    const state = open(caseName, { transports })
    try {
      await socketUntil(() => state.socket.connected || state.connectErrors.length, 'the Socket.IO client must connect through the actual gateway')
      assert.ok(state.socket.connected, state.connectErrors[0]?.message)
      await socketUntil(() => state.socket.io.engine.transport.name === expectedTransport, 'polling must complete its real WebSocket upgrade')
      if (caseName === 'polling-upgrade') assert.deepEqual(state.upgrades, ['websocket'])
      await echo(state, caseName, expectedTransport)
      const entries = preservedRequests(caseName)
      if (transports.includes('polling')) {
        assert.ok(entries.some((entry) => entry.kind === 'http' && entry.method === 'GET'))
        assert.ok(entries.some((entry) => entry.kind === 'http' && entry.method === 'POST'), 'Socket.IO polling must proxy real client POST packets')
      }
      if (expectedTransport === 'websocket') assert.ok(entries.some((entry) => entry.kind === 'upgrade'), 'an actual HTTP Upgrade must reach the independent fixture')
      const sid = state.socket.io.engine.id
      if (transports.includes('polling')) assert.ok(entries.some((entry) => new URL(entry.url, base).searchParams.get('sid') === sid), 'polling session query must survive proxying')
    } finally { state.socket.disconnect(); fixture.clients.delete(state.socket) }
  })
  await t.test('Socket.IO transport fixture: wrong polling Origin is rejected without public static CORS', async () => {
    const response = await get(gateway.port, `${SOCKET_PATH}?EIO=4&transport=polling&gatewayCase=bad-polling`, 2000, gateway.host, { Origin: 'https://unapproved-origin.example.test' })
    assert.equal(response.status, 403)
    assert.equal(response.headers['access-control-allow-origin'], undefined)
    assert.equal(response.headers['access-control-allow-credentials'], undefined)
  })
  await t.test('Socket.IO transport fixture: wrong WebSocket Origin cannot establish a namespace connection', async () => {
    const before = fixture.connections()
    const state = open('bad-websocket', { extraHeaders: { Origin: 'https://unapproved-origin.example.test' } })
    try {
      await socketUntil(() => state.connectErrors.length || state.socket.connected, 'the actual WebSocket handshake must reject an unapproved Origin')
      assert.ok(!state.socket.connected)
      assert.equal(fixture.connections(), before)
    } finally { state.socket.disconnect(); fixture.clients.delete(state.socket) }
  })
  await t.test('Socket.IO transport fixture: unapproved upgrade Origin retains only the approved polling transport', async () => {
    const state = open('bad-upgrade', {
      transports: ['polling', 'websocket'],
      transportOptions: {
        polling: { extraHeaders: { Origin: SOCKET_ORIGIN } },
        websocket: { extraHeaders: { Origin: 'https://unapproved-upgrade.example.test' } },
      },
    })
    try {
      await socketUntil(() => state.socket.connected && state.upgradeErrors.length, 'the proxied WebSocket upgrade with an unapproved Origin must fail')
      assert.deepEqual(state.upgrades, [])
      assert.equal(state.socket.io.engine.transport.name, 'polling')
      await echo(state, 'bad-upgrade', 'polling')
      const entries = caseRequests('bad-upgrade')
      assert.ok(entries.some((entry) => entry.kind === 'upgrade' && entry.origin === 'https://unapproved-upgrade.example.test'), 'proxy must preserve the denied upgrade Origin for the upstream guard')
      for (const entry of entries) assert.equal(new URL(entry.url, base).pathname, SOCKET_PATH)
    } finally { state.socket.disconnect(); fixture.clients.delete(state.socket) }
  })
}

test('Caddy validates and serves the production release gateway contract', {
  skip: caddyBinary ? false : 'Caddy binary unavailable; set CADDY_BIN to an installed binary path',
}, async (t) => {
  const gateway = await gatewayFixture(t, 'Caddy')
  assert.match(`${gateway.validation.stdout}${gateway.validation.stderr}`, /valid configuration/i)
  await checkHttpContracts(t, gateway.port, gateway.host)
  await checkBusinessProxyContracts(t, gateway)
  await checkSocketProxyContracts(t, gateway)
})

test('Nginx validates and serves the production release gateway contract', {
  skip: nginxBinary ? false : 'Nginx binary unavailable; set NGINX_BIN to an installed binary path',
}, async (t) => {
  const gateway = await gatewayFixture(t, 'Nginx')
  assert.match(gateway.validation.stderr, /syntax is ok/i)
  assert.match(gateway.validation.stderr, /test is successful/i)
  await checkHttpContracts(t, gateway.port, gateway.host)
  await checkBusinessProxyContracts(t, gateway)
  await checkSocketProxyContracts(t, gateway)
})
