import assert from 'node:assert/strict'
import test from 'node:test'
import { spawn, spawnSync } from 'node:child_process'
import { createHash, randomBytes } from 'node:crypto'
import { request } from 'node:https'
import { request as httpRequest } from 'node:http'
import { appendFileSync, cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { renderGateway, renderMicroStatic } from '../deploy/gateway/render.mjs'
import { exportHistory } from '../deploy/static/prepare-history.mjs'

// Actual Linux Docker integration gate: a missing runtime or trusted TLS
// connection fails rather than silently skipping a mandatory acceptance check.
// The separate Rust fixture proves raw HTTP/WebSocket transport only; it is
// deliberately not an Engine.IO or Socket.IO compatibility implementation.
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const owner = `phase03-${Date.now()}-${randomBytes(3).toString('hex')}`
const evidenceRoot = process.env.PHASE03_EVIDENCE_ROOT ? path.resolve(process.env.PHASE03_EVIDENCE_ROOT) : path.join(root, 'target', owner)
const windows = process.platform === 'win32'
const dockerPrefix = windows ? ['-d', process.env.PHASE03_WSL_DISTRO || 'Ubuntu', '--exec', 'docker', '--host', 'unix:///var/run/docker.sock'] : ['--host', 'unix:///var/run/docker.sock']
const dockerProgram = windows ? 'wsl.exe' : (process.env.DOCKER_BIN || 'docker')
const certificateOrigin = 'https://fixture.saas.test'
const containers = new Set(), networks = new Set(), volumes = new Set(), images = new Set()
const report = { schemaVersion: 1, owner, startedAt: new Date().toISOString(), fixture: 'RUST_TRANSPORT_ONLY_NOT_SOCKET_IO_COMPATIBILITY', checks: [], commands: [] }
assert(!existsSync(evidenceRoot), 'Runtime evidence output must be a new owned directory')
mkdirSync(evidenceRoot, { recursive: true })
const linuxPath = (value) => windows ? `/mnt/${path.resolve(value)[0].toLowerCase()}/${path.resolve(value).slice(3).replaceAll('\\', '/')}` : path.resolve(value)
const hash = (bytes) => createHash('sha256').update(bytes).digest('hex')
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
function write(relative, value) { const destination = path.join(evidenceRoot, relative); mkdirSync(path.dirname(destination), { recursive: true }); writeFileSync(destination, value); return destination }
function save() { write('result.json', `${JSON.stringify(report, null, 2)}\n`) }
function command(program, args, { timeout = 900_000, log, allowFailure = false } = {}) {
  report.commands.push({ program, args }); log ??= `command-${report.commands.length}.log`
  const outputFile = write(log, '')
  return new Promise((resolve, reject) => {
    const child = spawn(program, args, { cwd: root, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] }); let output = ''
    const timer = setTimeout(() => { child.kill(); reject(new Error(`${program} ${args[0]} timed out after ${timeout}ms`)) }, timeout)
    child.stdout.on('data', (data) => { output += data; appendFileSync(outputFile, data) }); child.stderr.on('data', (data) => { output += data; appendFileSync(outputFile, data) })
    child.on('error', (error) => { clearTimeout(timer); reject(error) })
    child.on('close', (code) => { clearTimeout(timer); write(log, output); if (code && !allowFailure) reject(new Error(`${program} exited ${code}: ${output.slice(-6000)}`)); else resolve(output.trim()) })
  })
}
const docker = (args, options) => command(dockerProgram, [...dockerPrefix, ...args], options)
const inspect = async (name, kind = 'container') => JSON.parse(await docker([kind, 'inspect', name]))[0]
const mount = (file, destination) => ['--mount', `type=bind,source=${linuxPath(file)},target=${destination},readonly`]
function cleanSource(destination) {
  // Allowlisted source; npm ci runs inside the builder without ambient .env,
  // node_modules, dist, keys, target output or working-tree Git data.
  mkdirSync(path.join(destination, 'scripts'), { recursive: true })
  for (const relative of ['frontend', 'deploy/static', 'examples/micro-app-sdk', 'scripts/sync-micro-sdk.mjs']) cpSync(path.join(root, relative), path.join(destination, relative), { recursive: true, filter(source) {
    return !path.relative(root, source).split(path.sep).some((part) => ['node_modules', 'dist', 'target', 'history'].includes(part) || part.startsWith('.env') || /\.(?:pem|key|tsbuildinfo)$/.test(part))
  } })
  mkdirSync(path.join(destination, 'deploy/static/history'), { recursive: true }); writeFileSync(path.join(destination, 'deploy/static/history/.gitkeep'), '')
  cpSync(path.join(root, '.dockerignore'), path.join(destination, '.dockerignore'))
}
async function build(tag, dockerfile, context, buildArgs = []) {
  console.log(`Building isolated image ${tag}`)
  const proxy = process.env.PHASE03_BUILD_PROXY
  // Optional task-local CONNECT proxy for restricted WSL networks. Docker's
  // predefined proxy arguments do not become image ENV/history; normal CA
  // verification remains enabled. Never log proxy credentials or alter daemon.
  if (proxy) assert.match(proxy, /^http:\/\/(?:127\.0\.0\.1|localhost):[0-9]{1,5}$/)
  const transport = proxy ? ['--network', 'host', ...['HTTPS_PROXY', 'HTTP_PROXY', 'https_proxy', 'http_proxy'].flatMap(key => ['--build-arg', `${key}=${proxy}`])] : []
  await docker(['build', '--force-rm', ...transport, '--label', `saas.phase03.owner=${owner}`, '--file', linuxPath(dockerfile), '--tag', tag, ...buildArgs.flatMap(([key, value]) => ['--build-arg', `${key}=${value}`]), linuxPath(context)], { log: `${tag.split(':').at(-1)}-build.log` })
  images.add(tag); return (await inspect(tag, 'image')).Id
}
async function run(name, image, options = []) {
  containers.add(name)
  return docker(['run', '--detach', '--name', name, '--label', `saas.phase03.owner=${owner}`, '--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', '--tmpfs', '/tmp:rw,nosuid,noexec,size=32m', ...options, image])
}
async function ownedResource(name, kind) {
  let state
  try { state = await inspect(name, kind) } catch (error) { if (/No such (?:container|network|volume|object)/i.test(error.message)) return null; throw error }
  assert.equal((kind === 'container' ? state.Config.Labels : state.Labels)?.['saas.phase03.owner'], owner, `Refusing to remove resource not owned by this run: ${name}`)
  return state
}
async function removeContainer(name) { assert(containers.has(name)); assert(await ownedResource(name, 'container')); await docker(['rm', '--force', name]); containers.delete(name) }
function https(port, url, { method = 'GET', body, headers = {}, ca } = {}) {
  return new Promise((resolve, reject) => {
    const req = request({ hostname: '127.0.0.1', port, path: url, method, servername: 'fixture.saas.test', ca, headers: { host: 'fixture.saas.test', ...headers }, timeout: 5000 }, (res) => {
      const chunks = []; res.on('data', (chunk) => chunks.push(chunk)); res.on('end', () => resolve({ status: res.statusCode, headers: res.headers, rawHeaders: res.rawHeaders, body: Buffer.concat(chunks) }))
    }); req.on('error', reject); req.on('timeout', () => req.destroy(new Error('HTTPS request timeout'))); if (body) req.write(body); req.end()
  })
}
async function eventually(action, timeout = 30_000) { let last; const until = Date.now() + timeout; do { try { return await action() } catch (error) { last = error; await delay(300) } } while (Date.now() < until); throw last }
function single(response, name) { assert.equal(response.rawHeaders.filter((value, index) => index % 2 === 0 && value.toLowerCase() === name.toLowerCase()).length, 1, `${name} must occur exactly once`) }
function websocket(port, ca) {
  return new Promise((resolve, reject) => {
    const key = randomBytes(16).toString('base64'), received = []; let buffer = Buffer.alloc(0), socket, settled = false
    const timer = setTimeout(() => finish(new Error('WebSocket Upgrade/frame timeout')), 8000)
    function finish(error) { if (settled) return; settled = true; clearTimeout(timer); socket?.destroy(); error ? reject(error) : resolve(received) }
    function frames(chunk) {
      buffer = Buffer.concat([buffer, chunk])
      while (buffer.length >= 2) {
        const opcode = buffer[0] & 15; let length = buffer[1] & 127, offset = 2
        if (length === 126) { if (buffer.length < 4) return; length = buffer.readUInt16BE(2); offset = 4 }
        if (length === 127) return finish(new Error('Unexpected oversized frame'))
        if (buffer.length < offset + length) return
        const payload = buffer.subarray(offset, offset + length); buffer = buffer.subarray(offset + length)
        if (opcode === 1) { received.push(payload.toString()); if (received.length === 1) {
          const data = Buffer.from('phase03-websocket-roundtrip'), mask = randomBytes(4), frame = Buffer.alloc(6 + data.length); frame[0] = 0x81; frame[1] = 0x80 | data.length; mask.copy(frame, 2); for (let i = 0; i < data.length; i++) frame[6 + i] = data[i] ^ mask[i % 4]; socket.write(frame)
        } else if (received.length === 2) finish() }
      }
    }
    const req = request({ hostname: '127.0.0.1', port, path: '/socket.io/?EIO=4&transport=websocket&probe=phase03', servername: 'fixture.saas.test', ca, headers: { host: 'fixture.saas.test', origin: certificateOrigin, authorization: 'Bearer phase03-ws', connection: 'Upgrade', upgrade: 'websocket', 'sec-websocket-version': '13', 'sec-websocket-key': key } })
    req.on('upgrade', (response, upgraded, head) => { try {
      assert.equal(response.statusCode, 101); assert.equal(response.headers['sec-websocket-accept'], createHash('sha1').update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest('base64')); socket = upgraded; socket.on('error', finish); socket.on('data', frames); if (head.length) frames(head)
    } catch (error) { upgraded.destroy(); finish(error) } }); req.on('response', (response) => { response.resume(); finish(new Error(`Expected WebSocket 101; got ${response.statusCode}`)) }); req.on('error', finish); req.end()
  })
}

test('Nginx Linux Docker HTTPS / private Rust transport / immutable static deployment', { timeout: 1_800_000 }, async (t) => {
  let edge, port, ca
  const network = `${owner}-private`, ingress = `${owner}-ingress`, first = process.env.PHASE03_HOST_IMAGE || `${owner}-host:first`, second = `${owner}-host:second`, rust = `${owner}-rust:fixture`, gatewayImage = process.env.PHASE03_EDGE_IMAGE || `${owner}-edge:nginx`
  const frontend = `${owner}-frontend`, backend = `${owner}-backend`, zip = `${owner}-zip`, volume = `${owner}-zip-store`
  async function check(name, action) { await t.test(name, async () => { try { const details = await action(); report.checks.push({ name, result: 'PASS', details }); save() } catch (error) { report.checks.push({ name, result: 'FAIL', error: error.stack }); save(); throw error } }) }
  try {
    report.dockerVersion = JSON.parse(await docker(['version', '--format', '{{json .}}'], { timeout: 30_000, log: 'docker-version.json' }))
    report.composeVersion = await docker(['compose', 'version'], { timeout: 20_000, allowFailure: true, log: 'compose-version.log' })
    const source = path.join(evidenceRoot, 'source'); cleanSource(source)
    const head = spawnSync('git', ['-c', `safe.directory=${root.replaceAll('\\', '/')}`, 'rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).stdout.trim(); assert.match(head, /^[a-f0-9]{40}$/)
    if (process.env.PHASE03_HOST_IMAGE) {
      assert.match(process.env.PHASE03_HOST_IMAGE, /^sha256:[a-f0-9]{64}$/)
      report.firstImageId = (await inspect(process.env.PHASE03_HOST_IMAGE, 'image')).Id
      assert.equal(report.firstImageId, process.env.PHASE03_HOST_IMAGE)
    } else report.firstImageId = await build(first, path.join(source, 'frontend/Dockerfile'), source, [['SOURCE_REVISION', head], ['HOST_BUILD_ID', `${owner}-first`]])
    report.rustImageId = await build(rust, path.join(root, 'tests/fixtures/gateway-rust/Dockerfile'), root)
    if (process.env.PHASE03_EDGE_IMAGE) {
      assert.match(process.env.PHASE03_EDGE_IMAGE, /^sha256:[a-f0-9]{64}$/)
      report.gatewayImageId = (await inspect(process.env.PHASE03_EDGE_IMAGE, 'image')).Id
      assert.equal(report.gatewayImageId, process.env.PHASE03_EDGE_IMAGE)
    } else report.gatewayImageId = await build(gatewayImage, path.join(root, 'deploy/gateway/Dockerfile'), root)
    networks.add(network); await docker(['network', 'create', '--internal', '--label', `saas.phase03.owner=${owner}`, network])
    networks.add(ingress); await docker(['network', 'create', '--label', `saas.phase03.owner=${owner}`, ingress])
    volumes.add(volume); await docker(['volume', 'create', '--label', `saas.phase03.owner=${owner}`, volume])
    await run(frontend, first, ['--network', network, '--network-alias', 'frontend-web'])
    const backendOptions = ['--network', network, '--network-alias', 'backend', '--env', 'BIND_ADDR=0.0.0.0:3000', '--env', 'ALLOW_CONTAINER_BIND=1']
    await run(backend, rust, [...backendOptions, '--env', 'INSTANCE_ID=fixture-a'])
    write('tls/server.cnf', '[req]\ndistinguished_name=dn\nprompt=no\n[dn]\nCN=fixture.saas.test\n[extensions]\nsubjectAltName=DNS:fixture.saas.test\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n')
    const openssl = async (args) => command(windows ? 'wsl.exe' : 'openssl', windows ? ['-d', process.env.PHASE03_WSL_DISTRO || 'Ubuntu', '--exec', 'openssl', ...args] : args, { timeout: 30_000, log: `openssl-${args[0]}-${report.commands.length}.log` })
    await openssl(['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1', '-subj', '/CN=Phase03 Isolated Test CA', '-keyout', linuxPath(path.join(evidenceRoot, 'tls/ca.key')), '-out', linuxPath(path.join(evidenceRoot, 'tls/ca.pem'))])
    await openssl(['req', '-newkey', 'rsa:2048', '-nodes', '-config', linuxPath(path.join(evidenceRoot, 'tls/server.cnf')), '-keyout', linuxPath(path.join(evidenceRoot, 'tls/server.key')), '-out', linuxPath(path.join(evidenceRoot, 'tls/server.csr'))])
    await openssl(['x509', '-req', '-days', '1', '-in', linuxPath(path.join(evidenceRoot, 'tls/server.csr')), '-CA', linuxPath(path.join(evidenceRoot, 'tls/ca.pem')), '-CAkey', linuxPath(path.join(evidenceRoot, 'tls/ca.key')), '-CAcreateserial', '-extfile', linuxPath(path.join(evidenceRoot, 'tls/server.cnf')), '-extensions', 'extensions', '-out', linuxPath(path.join(evidenceRoot, 'tls/server.pem'))])
    ca = readFileSync(path.join(evidenceRoot, 'tls/ca.pem'))
    const microConf = write('micro-static.conf', renderMicroStatic({ publicOrigin: certificateOrigin }))
    await run(zip, gatewayImage, ['--network', network, '--network-alias', 'micro-static', ...mount(microConf, '/etc/nginx/nginx.conf'), '--mount', `type=volume,source=${volume},target=/micro-apps,readonly`])
    const config = write('gateway.conf', renderGateway({ publicOrigin: certificateOrigin, backend: 'backend:3000', notification: 'backend:3000', workOrders: 'backend:3000', zipStatic: 'micro-static:8080', versionRoutes: [{ app: 'image-micro', version: '1.0.0', upstream: 'frontend-web:8080' }] }))
    edge = `${owner}-edge`
    // Docker disables published ports on an internal-only network. Only the
    // edge joins this ingress bridge; all upstreams remain internal-only.
    await run(edge, gatewayImage, ['--network', ingress, '--publish', '127.0.0.1::8443', '--publish', '127.0.0.1::8080', ...mount(config, '/etc/nginx/nginx.conf'), ...mount(path.join(evidenceRoot, 'tls/server.pem'), '/etc/nginx/tls/fullchain.pem'), ...mount(path.join(evidenceRoot, 'tls/server.key'), '/etc/nginx/tls/privkey.pem')])
    await docker(['network', 'connect', network, edge])
    const edgeInspection = await inspect(edge); port = Number(edgeInspection.NetworkSettings.Ports['8443/tcp'][0].HostPort); report.httpsPort = port; report.httpPort = Number(edgeInspection.NetworkSettings.Ports['8080/tcp'][0].HostPort)
    await eventually(async () => assert.equal((await https(port, '/healthz', { ca })).status, 200))
    await check('nonroot read-only images, private upstreams, edge-only published ports', async () => {
      for (const name of [edge, frontend, zip]) assert.match(await docker(['exec', name, 'nginx', '-t', '-c', '/etc/nginx/nginx.conf']), /test is successful/)
      for (const name of [edge, frontend, backend, zip]) { const state = await inspect(name); assert.equal(state.HostConfig.ReadonlyRootfs, true); assert(state.Config.User && state.Config.User !== '0'); if (name !== edge) assert.equal(Object.keys(state.NetworkSettings.Ports || {}).filter((key) => state.NetworkSettings.Ports[key]?.length).length, 0); write(`${name}-inspect.json`, JSON.stringify(state, null, 2)) }
      assert.equal((await inspect(network, 'network')).Internal, true)
      assert.equal((await inspect(ingress, 'network')).Internal, false)
      assert.deepEqual(Object.keys((await inspect(ingress, 'network')).Containers), [(await inspect(edge)).Id], 'Only edge may join the ingress network')
      assert.equal(await docker(['exec', frontend, 'sh', '-c', 'id -u; for binary in node npm bun caddy; do if command -v "$binary"; then exit 1; fi; done; if touch /usr/share/nginx/html/phase03-probe 2>/dev/null; then exit 1; fi']), '101')
      assert.match(await docker(['exec', frontend, 'nginx', '-v']), /nginx\/1\.28\.0/)
      return { uid: 101, readOnlyRootfs: true, runtime: 'nginx', publishedPorts: 'edge only', internalNetwork: network }
    })
    await check('trusted HTTPS prefixes, POST bytes, query, Authorization, CORS, overwritten proxy headers', async () => {
      const redirect = await new Promise((resolve, reject) => { const req = httpRequest({ hostname: '127.0.0.1', port: report.httpPort, path: '/api/echo?keep=a%2Fb', headers: { host: 'fixture.saas.test' }, timeout: 5000 }, (response) => { response.resume(); resolve(response) }); req.on('error', reject); req.on('timeout', () => req.destroy(new Error('HTTP redirect timeout'))); req.end() }); assert.equal(redirect.statusCode, 308); assert.equal(redirect.headers.location, `${certificateOrigin}/api/echo?keep=a%2Fb`)
      for (const prefix of ['/api', '/oauth', '/work-orders-api']) {
        const body = '{"text":"中文 & + =","phase":3}', url = `${prefix}/echo?encoded=%E4%B8%AD%E6%96%87&keep=a%2Fb&repeat=1&repeat=2`
        const response = await https(port, url, { ca, method: 'POST', body, headers: { origin: certificateOrigin, authorization: 'Bearer phase03-opaque-token', 'content-type': 'application/json', 'x-forwarded-for': '203.0.113.9', 'x-forwarded-host': 'attacker.invalid', 'x-forwarded-proto': 'http', forwarded: 'for=203.0.113.9;proto=http' } })
        assert.equal(response.status, 200); const value = JSON.parse(response.body); assert.equal(value.marker, 'TRANSPORT_FIXTURE_ONLY'); assert.equal(value.uri, url); assert.equal(value.body, body); assert.equal(value.bodyBytes, Buffer.byteLength(body)); assert.equal(value.headers.authorization, 'Bearer phase03-opaque-token'); assert.equal(value.headers.origin, certificateOrigin); assert.equal(value.headers.host, 'fixture.saas.test'); assert.equal(value.headers['x-forwarded-host'], 'fixture.saas.test'); assert.equal(value.headers['x-forwarded-proto'], 'https'); assert.notEqual(value.headers['x-forwarded-for'], '203.0.113.9'); assert.equal(value.headers.forwarded, null); single(response, 'access-control-allow-origin'); assert.equal(response.headers['access-control-allow-origin'], certificateOrigin); assert.equal(response.headers['cache-control'], 'no-store')
      }
      const rejected = await https(port, '/api/echo', { ca, headers: { origin: 'https://attacker.invalid' } }); assert.equal(rejected.status, 403); assert.equal(rejected.headers['access-control-allow-origin'], undefined)
      const opaque = await https(port, '/work-orders-api/echo', { ca, headers: { origin: 'null' } }); assert.equal(opaque.status, 200); single(opaque, 'access-control-allow-origin'); assert.equal(opaque.headers['access-control-allow-origin'], 'null'); assert.equal(JSON.parse(opaque.body).headers.origin, 'null')
      const preflight = await https(port, '/api/echo', { ca, method: 'OPTIONS', headers: { origin: certificateOrigin, 'access-control-request-method': 'POST' } }); assert.equal(preflight.status, 204); single(preflight, 'access-control-allow-origin')
      return { prefixes: ['/api', '/oauth', '/work-orders-api'], exactBodyAndQuery: true, unauthorizedOrigin: 403, trustedCaVerification: true }
    })
    await check('Rust polling GET/POST, real TLS WebSocket Upgrade and text echo', async () => {
      const response = await https(port, '/socket.io/?EIO=4&transport=polling&token=a%2Fb', { ca, headers: { origin: certificateOrigin } }); assert.equal(response.status, 200); const open = JSON.parse(response.body.toString().slice(1)); assert.equal(open.fixture.uri, '/socket.io/?EIO=4&transport=polling&token=a%2Fb')
      const body = '42["fixture","中文"]'; const posted = await https(port, '/socket.io/?EIO=4&transport=polling&sid=fixture-a', { ca, method: 'POST', body, headers: { origin: certificateOrigin } }); assert.equal(posted.status, 200); assert.equal(JSON.parse(posted.body).body, body)
      const frames = await websocket(port, ca); const metadata = JSON.parse(frames[0]); assert.equal(metadata.marker, 'TRANSPORT_FIXTURE_ONLY'); assert.equal(metadata.headers.upgrade.toLowerCase(), 'websocket'); assert.equal(metadata.headers.authorization, 'Bearer phase03-ws'); assert.equal(frames[1], 'phase03-websocket-roundtrip')
      return { pollingGet: 200, pollingPost: 200, websocket: 101, textEcho: true, scope: 'raw transport only' }
    })
    let oldAsset, oldBytes, metadata
    await check('static MIME/cache/CSP, strict navigation, explicit immutable version route', async () => {
      const index = await https(port, '/index.html', { ca }); assert.equal(index.status, 200); assert.match(index.headers['content-type'], /text\/html/); assert.equal(index.headers['cache-control'], 'no-cache'); single(index, 'content-security-policy'); assert.match(index.headers['content-security-policy'], /default-src 'none'/); assert.match(index.headers['content-security-policy'], /frame-ancestors 'none'/)
      metadata = JSON.parse((await https(port, '/release-metadata.json', { ca })).body)
      if (process.env.PHASE03_RELEASE_BUILD_ID) assert.equal(metadata.buildId, process.env.PHASE03_RELEASE_BUILD_ID)
      const entry = /<script\b[^>]*\bsrc="(\/assets\/[^"?#]+\.js)"/.exec(index.body.toString())?.[1]; assert(entry, 'Built index must identify its real JS entry')
      oldAsset = metadata.assets.find((asset) => `/${asset.path}` === entry); assert(oldAsset, 'Actual old entry must be present in verified inventory')
      const asset = await https(port, `/${oldAsset.path}`, { ca }); assert.equal(asset.status, 200); assert.match(asset.headers['content-type'], /(?:application|text)\/javascript/); assert.match(asset.headers['cache-control'], /immutable/); assert.equal(hash(asset.body), oldAsset.sha256); oldBytes = asset.body
      assert.equal((await https(port, '/dashboard', { ca, headers: { accept: 'text/html' } })).status, 200)
      for (const [url, headers] of [['/dashboard', {}], ['/dashboard', { accept: 'text/html;q=0' }], ['/not-a-route', { accept: 'text/html' }], ['/assets/missing.js', {}], ['/missing.css', {}], ['/api/internal/secret', {}], ['/metrics', {}], ['/@vite/client', {}], ['/sub-apps/image-micro/versions/9.9.9/index.html', { accept: 'text/html' }], ['/sub-apps/missing/versions/1.0.0/assets.js', {}]]) { const response = await https(port, url, { ca, headers }); assert.equal(response.status, 404, url); assert.doesNotMatch(response.body.toString(), /<div id="root"/); assert.equal(response.headers['cache-control'], 'no-store', url) }
      const version = await https(port, '/sub-apps/image-micro/versions/1.0.0/index.html', { ca }); assert.equal(version.status, 200); assert.equal(version.body.toString(), index.body.toString())
      return { sourceSha256: metadata.sourceSha256, buildId: metadata.buildId, oldEntryAsset: oldAsset.path, assetSha256: oldAsset.sha256, versionRouteScope: 'explicit image upstream URI mapping; ZIP check proves iframe CSP' }
    })
    await check('Docker DNS recovers backend name after recreation changes its IP', async () => {
      const before = (await inspect(backend)).NetworkSettings.Networks[network].IPAddress; await removeContainer(backend)
      const blocker = `${owner}-old-ip`; containers.add(blocker); await docker(['run', '--detach', '--name', blocker, '--label', `saas.phase03.owner=${owner}`, '--read-only', '--network', network, '--ip', before, '--entrypoint', 'sleep', gatewayImage, '120'])
      await run(backend, rust, [...backendOptions, '--env', 'INSTANCE_ID=fixture-b'])
      const after = (await inspect(backend)).NetworkSettings.Networks[network].IPAddress; assert.notEqual(after, before)
      await eventually(async () => { const response = await https(port, '/api/echo', { ca }); assert.equal(response.status, 200); assert.equal(JSON.parse(response.body).instanceId, 'fixture-b') }, 25_000)
      return { before, after, instanceBefore: 'fixture-a', instanceAfter: 'fixture-b', strategy: 'Docker DNS valid=5s variable proxy_pass' }
    })
    await check('ZIP volume survives static recreation, mounted read-only, staging/missing version denied', async () => {
      const writer = `${owner}-zip-writer`
      containers.add(writer)
      // A one-shot owned volume seed is not a platform API implementation.
      await docker(['run', '--name', writer, '--label', `saas.phase03.owner=${owner}`, '--read-only', '--user', '0:0', '--network', 'none', '--mount', `type=volume,source=${volume},target=/micro-apps`, '--entrypoint', 'sh', gatewayImage, '-c', 'mkdir -p /micro-apps/zip-fixture/versions/1.0.0 /micro-apps/zip-fixture/staging; printf "ZIP_PERSISTENT_PHASE03" > /micro-apps/zip-fixture/versions/1.0.0/index.html; printf "export const fixture = 3;" > /micro-apps/zip-fixture/versions/1.0.0/app.mjs; printf "body{color:red}" > /micro-apps/zip-fixture/versions/1.0.0/style.css; printf "PRIVATE_STAGING" > /micro-apps/zip-fixture/staging/secret.txt'])
      const url = '/sub-apps/zip-fixture/versions/1.0.0/index.html', html = await https(port, url, { ca }); assert.equal(html.body.toString(), 'ZIP_PERSISTENT_PHASE03'); assert.equal(html.status, 200); single(html, 'content-security-policy'); assert.match(html.headers['content-security-policy'], /frame-ancestors https:\/\/fixture\.saas\.test/); assert.equal(html.headers['x-frame-options'], undefined); single(html, 'cache-control'); assert.equal(html.headers['cache-control'], 'no-cache')
      const directory = await https(port, '/sub-apps/zip-fixture/versions/1.0.0/', { ca }); assert.equal(directory.status, 200); assert.equal(directory.body.toString(), 'ZIP_PERSISTENT_PHASE03')
      const js = await https(port, '/sub-apps/zip-fixture/versions/1.0.0/app.mjs', { ca }); assert.match(js.headers['content-type'], /(?:application|text)\/javascript/); assert.match(js.headers['cache-control'], /immutable/); single(js, 'cache-control'); single(js, 'access-control-allow-origin'); assert.equal(js.headers['access-control-allow-origin'], '*')
      const css = await https(port, '/sub-apps/zip-fixture/versions/1.0.0/style.css', { ca }); assert.match(css.headers['content-type'], /text\/css/)
      for (const url of ['/sub-apps/zip-fixture/staging/secret.txt', '/sub-apps/zip-fixture/versions/no-such/index.html', '/sub-apps/zip-fixture/versions/1.0.0/missing.js']) { const missing = await https(port, url, { ca, headers: { accept: 'text/html' } }); assert.equal(missing.status, 404, url); single(missing, 'cache-control'); assert.equal(missing.headers['cache-control'], 'no-store') }
      assert.equal((await inspect(zip)).Mounts.find((entry) => entry.Destination === '/micro-apps').RW, false)
      assert.equal(await docker(['exec', zip, 'sh', '-c', 'if touch /micro-apps/write-probe 2>/dev/null; then exit 1; fi; printf readonly']), 'readonly')
      await removeContainer(zip); await run(zip, gatewayImage, ['--network', network, '--network-alias', 'micro-static', ...mount(microConf, '/etc/nginx/nginx.conf'), '--mount', `type=volume,source=${volume},target=/micro-apps,readonly`]); await eventually(async () => assert.equal((await https(port, url, { ca })).body.toString(), 'ZIP_PERSISTENT_PHASE03'), 25_000)
      return { volume, persistedAfterRecreation: true, staticMountReadOnly: true, staging: 404, missingVersion: 404 }
    })
    await check('history export retains actual old hashed JS chunk in next image without shared volume', async () => {
      assert(oldAsset && oldBytes, 'prior static check must select a real old JS chunk')
      // Only Docker cp destinations need WSL translation; hash validation and
      // safe temporary cleanup stay in the actual operator export tool.
      exportHistory({ imageId: report.firstImageId, output: path.join(source, 'deploy/static/history'), docker: dockerProgram, run(program, args, options) { const mapped = [...args]; if (mapped[0] === 'cp') mapped[2] = linuxPath(mapped[2]); if (mapped[0] === 'create') mapped.splice(1, 0, '--label', `saas.phase03.owner=${owner}`); return spawnSync(program, [...dockerPrefix, ...mapped], options) } })
      const entry = path.join(source, 'frontend/src/main.tsx'); writeFileSync(entry, `${readFileSync(entry, 'utf8')}\nconsole.info(${JSON.stringify(`${owner}-next-release`)});\n`)
      report.secondImageId = await build(second, path.join(source, 'frontend/Dockerfile'), source, [['SOURCE_REVISION', head], ['HOST_BUILD_ID', `${owner}-second`]])
      assert.notEqual(report.secondImageId, report.firstImageId); await removeContainer(frontend); await run(frontend, second, ['--network', network, '--network-alias', 'frontend-web'])
      let next; await eventually(async () => { const response = await https(port, '/release-metadata.json', { ca }); assert.equal(response.status, 200); next = JSON.parse(response.body); assert.equal(next.buildId, `${owner}-second`) }, 25_000)
      assert.notEqual(next.sourceSha256, metadata.sourceSha256); assert(next.retainedReleases.some((release) => release.image === report.firstImageId && release.identityKind === 'docker-image-id'))
      const nextIndex = await https(port, '/index.html', { ca }), nextEntry = /<script\b[^>]*\bsrc="(\/assets\/[^"?#]+\.js)"/.exec(nextIndex.body.toString())?.[1]; assert(nextEntry); assert.notEqual(nextEntry, `/${oldAsset.path}`); assert(Array.isArray(next.currentAssets)); assert(!next.currentAssets.some((asset) => asset.path === oldAsset.path), 'Old entry must be retained-only, absent from current build asset inventory')
      const retained = await https(port, `/${oldAsset.path}`, { ca }); assert.equal(retained.status, 200); assert.deepEqual(retained.body, oldBytes); assert.match(retained.headers['cache-control'], /immutable/); assert.equal((await inspect(frontend)).Mounts.length, 0)
      return { previousImageId: report.firstImageId, nextImageId: report.secondImageId, retainedUrl: `/${oldAsset.path}`, nextEntry, retainedOnly: true, bytesSha256: hash(retained.body), newSourceSha256: next.sourceSha256, sharedRuntimeVolume: false }
    })
    report.result = report.checks.every((check) => check.result === 'PASS') ? 'PASS' : 'FAIL'
  } catch (error) { report.result = 'FAIL'; report.error = error.stack; throw error }
  finally {
    for (const name of containers) try { if (await ownedResource(name, 'container')) { write(`${name}-logs.txt`, await docker(['logs', name], { allowFailure: true, timeout: 15_000 })); await docker(['rm', '--force', name], { timeout: 20_000 }) } } catch (error) { report.cleanupErrors ??= []; report.cleanupErrors.push(`${name}: ${error.message}`) }
    for (const name of volumes) try { if (await ownedResource(name, 'volume')) await docker(['volume', 'rm', name], { timeout: 20_000 }) } catch (error) { report.cleanupErrors ??= []; report.cleanupErrors.push(error.message) }
    for (const name of networks) try { if (await ownedResource(name, 'network')) await docker(['network', 'rm', name], { timeout: 20_000 }) } catch (error) { report.cleanupErrors ??= []; report.cleanupErrors.push(error.message) }
    // Labelled immutable images remain for local audit; no base images or
    // unrelated Docker resources/source/evidence directories are removed.
    if (report.cleanupErrors?.length) report.result = 'FAIL'
    report.ownedImages = [...images]; report.finishedAt = new Date().toISOString(); save(); console.log(`Phase 3 runtime evidence: ${evidenceRoot}`)
    if (report.cleanupErrors?.length) throw new Error(`Owned resource cleanup failed: ${report.cleanupErrors.join('; ')}`)
  }
})
