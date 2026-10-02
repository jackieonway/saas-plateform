import assert from 'node:assert/strict'
import test from 'node:test'
import { spawn, spawnSync } from 'node:child_process'
import { createServer, request } from 'node:http'
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

function get(port, url, timeout = 2000, host = '127.0.0.1', headers = {}) {
  return new Promise((resolve, reject) => {
    const outgoing = request({ host, port, path: url, method: 'GET', agent: false, headers }, (response) => {
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
    outgoing.end()
  })
}

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
  let service
  t.after(async () => {
    try {
      await stopChild(service)
    } finally {
      if (upstream.listening) {
        upstream.closeAllConnections()
        await new Promise((resolve, reject) => upstream.close((error) => error ? reject(error) : resolve()))
      }
    }
    safeCleanup(root)
  })
  // Prefer the repository's Windows IPv6 test route for the native Nginx binary.
  const host = name === 'Nginx' && process.platform === 'win32' ? '::1' : '127.0.0.1'
  const originHost = host.includes(':') ? `[${host}]` : host
  const backendPort = await listen(upstream, host)
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
    config = source
      .replace(/^:80\s*\{/m, `http://127.0.0.1:${gatewayPort} {`)
      .replaceAll('backend:3001', `127.0.0.1:${backendPort}`)
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
    config = `daemon off;\npid ${quotedPath(path.join(root, 'nginx.pid'))};\nerror_log ${quotedPath(path.join(root, 'nginx-error.log'))} notice;\n` + source
      .replace('worker_processes auto;', 'worker_processes 1;')
      .replace('listen 80;', `listen ${originHost}:${gatewayPort};`)
      .replace('include mime.types;', `include ${quotedPath(mimeFile)};`)
      .replace('http {', `http {\n    access_log ${quotedPath(path.join(root, 'nginx-access.log'))};\n${temporaryPaths}`)
      .replaceAll('http://127.0.0.1:3001', `http://${originHost}:${backendPort}`)
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
      if (response.status === 200) return { port: gatewayPort, host, validation }
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

test('Caddy validates and serves the production release gateway contract', {
  skip: caddyBinary ? false : 'Caddy binary unavailable; set CADDY_BIN to an installed binary path',
}, async (t) => {
  const gateway = await gatewayFixture(t, 'Caddy')
  assert.match(`${gateway.validation.stdout}${gateway.validation.stderr}`, /valid configuration/i)
  await checkHttpContracts(t, gateway.port, gateway.host)
})

test('Nginx validates and serves the production release gateway contract', {
  skip: nginxBinary ? false : 'Nginx binary unavailable; set NGINX_BIN to an installed binary path',
}, async (t) => {
  const gateway = await gatewayFixture(t, 'Nginx')
  assert.match(gateway.validation.stderr, /syntax is ok/i)
  assert.match(gateway.validation.stderr, /test is successful/i)
  await checkHttpContracts(t, gateway.port, gateway.host)
})
