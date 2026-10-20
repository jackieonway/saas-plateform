/** Owned test control plane. Product operations always use the actual Rust HTTP services. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { randomBytes, randomUUID, createHash } from 'node:crypto'
import { createRequire } from 'node:module'
import http from 'node:http'
import https from 'node:https'
import net from 'node:net'
import { chmodSync, chownSync, cpSync, closeSync, existsSync, lstatSync, mkdirSync, mkdtempSync, openSync, readFileSync, readdirSync, realpathSync, rmSync, unlinkSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { renderGateway, renderMicroStatic } from '../../deploy/gateway/render.mjs'

export const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
export const API = '/stores-api/stores'
export const APP = 'stores'
export function phase27MenuCounts(applications) {
  return Object.fromEntries(['stores', 'work-orders'].map(code => [code, applications[code].menus.size]))
}
const required = ['TEST_DATABASE_URL', 'CI_TEST_POSTGRES_OWNER', 'STORES_BROWSER_PLATFORM_BINARY', 'STORES_BROWSER_BUSINESS_BINARY',
  'STORES_BROWSER_HOST_DIST', 'STORES_BROWSER_WORK_ORDERS_BINARY', 'STORES_BROWSER_ADMIN_BINARY', 'NGINX_BIN', 'NGINX_MIME_TYPES',
  'STORES_BROWSER_TLS_CERT', 'STORES_BROWSER_TLS_KEY', 'STORES_BROWSER_TLS_CA']
const sha = bytes => createHash('sha256').update(bytes).digest('hex')
export const delay = ms => new Promise(resolve => setTimeout(resolve, ms))
export async function wait(predicate, message, timeout = 20_000) {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) { if (await predicate()) return; await delay(25) }
  assert(await predicate(), message)
}
async function freePort() {
  const server = net.createServer()
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve) })
  const port = server.address().port
  await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()))
  return port
}
function file(value, label) {
  assert(value && path.isAbsolute(value), `${label} requires an explicit absolute path`)
  const resolved = realpathSync(value)
  assert(lstatSync(resolved).isFile(), `${label} must name a regular file`)
  return resolved
}
function tree(directory) {
  const result = []
  const visit = current => { for (const name of readdirSync(current).sort()) {
    const item = path.join(current, name), stat = lstatSync(item)
    assert(!stat.isSymbolicLink(), 'Owned static outputs cannot contain links')
    if (stat.isDirectory()) visit(item)
    else if (stat.isFile()) result.push({ path: path.relative(directory, item).replaceAll('\\', '/'), sha256: sha(readFileSync(item)), bytes: stat.size })
  } }
  visit(directory); return result
}

export async function startPhase27Services({ recovery = false } = {}) {
  assert.equal(process.platform, 'linux', 'The owned production browser fixture requires its managed Linux environment')
  for (const key of required) assert(process.env[key], `Missing required Phase27 browser setting: ${key}`)
  const adminEndpoint = new URL(process.env.TEST_DATABASE_URL)
  const postgresOwner = process.env.CI_TEST_POSTGRES_OWNER
  assert(/^ci-[a-z0-9-]{8,80}$/.test(postgresOwner), 'Explicit managed PostgreSQL owner required')
  assert(adminEndpoint.protocol === 'postgresql:' && adminEndpoint.hostname === postgresOwner + '-postgres' &&
    adminEndpoint.port === '5432' && adminEndpoint.username === 'ci' && adminEndpoint.password &&
    adminEndpoint.pathname === '/postgres' && !adminEndpoint.search && !adminEndpoint.hash,
  'The browser fixture requires its own managed disposable PostgreSQL administration endpoint')
  const pg = createRequire(path.join(repository, 'mini-services/notification-ws/package.json'))('pg')
  const platformBinary = file(process.env.STORES_BROWSER_PLATFORM_BINARY, 'Platform binary')
  const businessBinary = file(process.env.STORES_BROWSER_BUSINESS_BINARY, 'Stores binary')
  const workOrdersBinary = file(process.env.STORES_BROWSER_WORK_ORDERS_BINARY, 'Work orders binary')
  const adminBinary = file(process.env.STORES_BROWSER_ADMIN_BINARY, 'Rust platform-admin binary')
  const nginxBinary = file(process.env.NGINX_BIN, 'Nginx binary')
  const hostDist = realpathSync(process.env.STORES_BROWSER_HOST_DIST)
  const html = readFileSync(path.join(hostDist, 'index.html'), 'utf8')
  assert(/assets\/.+\.js/.test(html) && !/@vite\/client|\/src\/main/.test(html), 'Use the actual built production host')
  const hostBefore = tree(hostDist)
  const certificate = file(process.env.STORES_BROWSER_TLS_CERT, 'TLS certificate')
  const certificateKey = file(process.env.STORES_BROWSER_TLS_KEY, 'TLS key')
  const ca = readFileSync(file(process.env.STORES_BROWSER_TLS_CA, 'TLS CA'))
  const temporaryParent = realpathSync(os.tmpdir())
  const ownedRoot = realpathSync(mkdtempSync(path.join(temporaryParent, 'saas-phase27-browser-')))
  // Nginx workers use their configured unprivileged account. The directory is
  // traversable, while secrets remain only in the separate launcher-owned TLS tree.
  chmodSync(ownedRoot, 0o755)
  const processes = [], databases = [], logs = [], publicBuildSteps = []
  const runtimeEnvironment = Object.fromEntries(Object.entries(process.env).filter(([key]) =>
    /^(PATH|LANG|LC_ALL|LD_LIBRARY_PATH|HOME|SSL_CERT_FILE|NODE_EXTRA_CA_CERTS|PLAYWRIGHT_BROWSERS_PATH|TMPDIR|XDG_CONFIG_HOME|XDG_DATA_HOME|XDG_CACHE_HOME)$/.test(key)))
  const secretValues = new Set([process.env.TEST_DATABASE_URL])
  const redact = value => {
    let safe = String(value).replace(/Bearer\s+[^\s"']+/gi, 'Bearer [redacted]').replace(/postgres(?:ql)?:\/\/[^\s"']+/g, '[database-url-redacted]')
    for (const secret of secretValues) if (secret) safe = safe.replaceAll(secret, '[redacted]')
    return safe
  }
  let closed = false
  function processIdentity(pid) {
    try {
      const stat = readFileSync('/proc/' + pid + '/stat', 'utf8')
      const fields = stat.slice(stat.lastIndexOf(')') + 2).trim().split(/\s+/)
      assert(/^\d+$/.test(fields[19] || ''), 'Owned process start time is unavailable')
      return { pid: Number(pid), parentPid: Number(fields[1]), startTime: fields[19] }
    } catch (error) { if (['ENOENT', 'ESRCH'].includes(error.code)) return null; throw error }
  }
  function discoverNginxWorkers(item) {
    if (!item.nginx || !item.identity) return
    const master = processIdentity(item.identity.pid)
    if (master?.startTime !== item.identity.startTime) return
    for (const pid of readdirSync('/proc').filter(value => /^\d+$/.test(value))) {
      let identity
      try { identity = processIdentity(pid) } catch { continue }
      if (identity?.parentPid === item.identity.pid && !item.workers.some(worker => worker.pid === identity.pid && worker.startTime === identity.startTime)) item.workers.push(identity)
    }
  }
  function workersGone(item) {
    if (!item.nginx) return true
    try { return item.workers.every(worker => processIdentity(worker.pid)?.startTime !== worker.startTime) }
    catch { return false }
  }
  const processStopped = item => item.closed && workersGone(item)
  function nginxDiagnostic() {
    return processes.filter(item => item.nginx).map(item => {
      try { return item.label + ': ' + item.output.slice(-1500) }
      catch { return item.label + ': OWNED_LOG_UNAVAILABLE' }
    }).join('\n')
  }
  function launch(label, executable, args, env = runtimeEnvironment) {
    const nginx = label.startsWith('nginx')
    let logPaths, child, streamingOutput = ''
    const descriptors = []
    if (nginx) {
      const directory = path.join(ownedRoot, 'process-logs')
      if (!existsSync(directory)) mkdirSync(directory, { mode: 0o700 })
      logPaths = ['stdout', 'stderr'].map(stream => path.join(directory, label + '-' + stream + '.log'))
    }
    try {
      for (const file of logPaths || []) descriptors.push(openSync(file, 'a', 0o600))
      // Nginx reopens the renderer's /dev/stdout and /dev/stderr paths. Node's
      // pipe transport is a socket and cannot be reopened this way; regular
      // owned files keep the production config intact. -e also owns diagnostics
      // emitted before Nginx reads that config, avoiding compiled system paths.
      child = spawn(executable, nginx ? ['-e', logPaths[1], ...args] : args,
        { cwd: ownedRoot, env, windowsHide: true, stdio: nginx ? ['ignore', ...descriptors] : ['ignore', 'pipe', 'pipe'] })
    } finally { for (const descriptor of descriptors) closeSync(descriptor) }
    const item = { label, child, nginx, identity: child.pid ? processIdentity(child.pid) : null, workers: [], closed: false, error: false, logPaths, get output() {
      // Redact the complete value before taking a diagnostic tail: truncating
      // a credential first could expose an unmatched suffix at the boundary.
      return redact(logPaths ? logPaths.map(file => readFileSync(file, 'utf8')).join('\n') : streamingOutput).slice(-32_000)
    } }
    item.exited = new Promise(resolve => child.once('close', code => { item.closed = true; item.code = code; resolve() }))
    child.on('error', () => { item.error = true })
    for (const stream of [child.stdout, child.stderr].filter(Boolean)) stream.on('data', bytes => { streamingOutput += bytes; logs.push(String(bytes)) })
    processes.push(item); return item
  }
  async function stop(item) {
    discoverNginxWorkers(item)
    let forced = false
    const settled = async milliseconds => {
      const deadline = Date.now() + milliseconds
      while (!processStopped(item) && Date.now() < deadline) { discoverNginxWorkers(item); await delay(25) }
      return processStopped(item)
    }
    if (!item.closed) {
      item.child.kill(item.nginx ? 'SIGQUIT' : 'SIGTERM')
      if (!await settled(5000) && !item.closed) { forced = true; item.child.kill('SIGKILL'); await settled(3000) }
    }
    if (item.nginx) console.log('PHASE27_NGINX_STOP ' + JSON.stringify({ label: item.label, master: item.identity,
      workers: item.workers, masterExited: item.closed, workersGone: workersGone(item), forced }))
    assert(processStopped(item), 'Owned master and every registered worker exit must be confirmed')
  }
  async function command(label, args, cwd = repository) {
    const child = spawn(process.execPath, args, { cwd, env: { ...process.env }, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
    let output = '', failed
    for (const stream of [child.stdout, child.stderr]) stream.on('data', data => { output += data })
    child.on('error', () => { failed = true })
    const timer = setTimeout(() => child.kill('SIGKILL'), 120_000)
    try { const code = await new Promise(resolve => child.once('close', resolve)); assert(!failed && code === 0, `${label} failed: ${redact(output).slice(-16_000)}`) }
    finally { clearTimeout(timer) }
  }
  async function request(base, endpoint, { token, method = 'GET', body, headers = {} } = {}) {
    const payload=body===undefined?undefined:JSON.stringify(body)
    if (token) headers = { ...headers, Authorization: 'Bearer ' + token }
    headers = { ...headers, 'X-Request-Id': randomUUID() }
    if (payload !== undefined) headers = { ...headers, 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(payload) }
    const url = new URL(endpoint, base)
    return new Promise((resolve, reject) => {
      const transport = url.protocol === 'https:' ? https : http
      const req = transport.request(url, { method, headers, ...(url.protocol === 'https:' ? { ca } : {}) }, response => {
        const bytes = []; let size = 0
        response.on('data', chunk => { size += chunk.length; if (size > 2 * 1024 * 1024) req.destroy(); else bytes.push(chunk) })
        response.on('end', () => { const text = Buffer.concat(bytes).toString(); let data; try { data = JSON.parse(text) } catch { data = null }
          resolve({ status: response.statusCode, data, text, headers: response.headers }) })
        response.on('error', () => reject(new Error('Owned HTTP response failed')))
      })
      req.setTimeout(30_000, () => req.destroy(Object.assign(new Error('Owned HTTP request deadline'), { code: 'OWNED_HTTP_TIMEOUT' })))
      req.on('error', error => {
        const failure = new Error('Owned HTTP request failed')
        if (/^[A-Z0-9_]{2,80}$/.test(error.code || '')) failure.code = error.code
        reject(failure)
      })
      req.end(payload)
    })
  }
  async function createDatabase(kind) {
    const name = 'phase27_' + kind + '_' + randomUUID().replaceAll('-', '')
    const role = name + '_writer', password = randomBytes(24).toString('hex')
    const admin = new pg.Client({ connectionString: adminEndpoint.href, connectionTimeoutMillis: 10000, query_timeout: 15000 })
    const item = { name, role, kind, admin, created: false, roleCreated: false, writer: null }; databases.push(item)
    admin.on('error', () => {}); await admin.connect()
    await admin.query('CREATE ROLE "' + role + '" LOGIN PASSWORD ' + "'" + password + "'" + ' NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION')
    item.roleCreated = true
    await admin.query('CREATE DATABASE "' + name + '" OWNER "' + role + '" TEMPLATE template0'); item.created = true
    const url = new URL(adminEndpoint); url.pathname = '/' + name; url.username = role; url.password = password
    item.url = url.href; secretValues.add(item.url); secretValues.add(password)
    item.writer = new pg.Client({ connectionString: item.url, connectionTimeoutMillis: 10000, query_timeout: 15000 })
    item.writer.on('error', () => {}); await item.writer.connect()
    const identity = (await item.writer.query('SELECT current_database() AS name,current_user AS role,rolsuper,rolcreatedb,rolcreaterole FROM pg_roles WHERE rolname=current_user')).rows[0]
    assert.equal(identity.name, name); assert.equal(identity.role, role)
    assert.equal(identity.rolsuper, false); assert.equal(identity.rolcreatedb, false); assert.equal(identity.rolcreaterole, false)
    item.identity = identity
    return item
  }
  function makeOwnedDirectoriesRemovable(directory = ownedRoot) {
    const absolute = path.resolve(directory), relative = path.relative(ownedRoot, absolute)
    assert(absolute === ownedRoot || (relative && !path.isAbsolute(relative) && relative !== '..' && !relative.startsWith('..' + path.sep)),
      'Temporary cleanup cannot leave its verified owned root')
    const stat = lstatSync(absolute)
    // npm creates executable links inside this owned copy. Remove the link itself;
    // never follow its target or recurse through a link during ownership recovery.
    if (stat.isSymbolicLink()) { unlinkSync(absolute); return }
    if (stat.isDirectory()) {
      // Actual Nginx workers may own mode-0700 body/proxy directories. After
      // every owned process exits, use only the existing narrow CHOWN capability
      // to regain ownership, then ordinary owner permissions to traverse them.
      chownSync(absolute, process.getuid(), process.getgid())
      chmodSync(absolute, 0o700)
      for (const entry of readdirSync(absolute)) makeOwnedDirectoriesRemovable(path.join(absolute, entry))
    } else assert(stat.isFile(), 'Temporary cleanup accepts only regular files and directories')
  }
  async function close({ browserClosed = true } = {}) {
    if (closed) return
    closed = true
    const errors = []
    for (const item of [...processes].reverse()) try { await stop(item) } catch { errors.push('process') }
    for (const item of databases) {
      try { await item.writer?.end() } catch { errors.push('writer') }
      try {
        assert(processes.every(processStopped), 'All services and registered workers must stop before dropping databases')
        assert.match(item.name, /^(?:phase27_[psw]_[0-9a-f]{32}|recovery_[psw]_[0-9a-f]{32})$/)
        if (item.created) { await item.admin.query(`DROP DATABASE "${item.name}" WITH (FORCE)`); item.created = false }
      } catch { errors.push('database') }
      try { await item.admin.end() } catch { errors.push('administration') }
    }
    // A recovery fixture may have two databases with the same provisioned owner.
    // Dispose all owned databases before removing any of their owned roles.
    for (const item of databases) if (item.roleCreated && !item.created) {
      const control = new pg.Client({ connectionString: adminEndpoint.href });
      try { await control.connect(); await control.query(`DROP ROLE "${item.role}"`); item.roleCreated = false }
      catch { errors.push('role') } finally { await control.end().catch(() => {}) }
    }
    let hostUnchanged = false
    try { hostUnchanged = JSON.stringify(tree(hostDist)) === JSON.stringify(hostBefore) } catch { errors.push('host-inspection') }
    if (!hostUnchanged) errors.push('host-modified')
    for (const item of processes) if (item.logPaths) {
      try { logs.push(...item.logPaths.map(file => readFileSync(file, 'utf8'))) } catch { errors.push('owned-log-inspection') }
    }
    const output = logs.join('')
    for (const secret of secretValues) if (secret && output.includes(secret)) errors.push('secret-in-service-log')
    if (errors.length === 0) {
      try {
        assert.equal(path.dirname(ownedRoot), temporaryParent)
        assert(path.basename(ownedRoot).startsWith('saas-phase27-browser-') && !lstatSync(ownedRoot).isSymbolicLink())
        assert(processes.every(processStopped), 'Temporary ownership recovery requires all owned processes and workers to have stopped')
        makeOwnedDirectoriesRemovable()
        rmSync(ownedRoot, { recursive: true, force: true })
      } catch { errors.push('temporary-directory') }
    }
    const cleanup = { platformDatabaseRemoved: databases.filter(item => item.kind === 'p').every(item => !item.created),
      businessDatabaseRemoved: databases.filter(item => item.kind !== 'p').every(item => !item.created),
      platformStopped: processes.filter(item => item.label === 'platform').every(processStopped),
      businessStopped: processes.filter(item => ['stores', 'work-orders'].includes(item.label)).every(processStopped),
      nginxStopped: processes.filter(item => item.nginx).every(processStopped),
      processesStopped: processes.every(processStopped), databasesRemoved: databases.every(item => !item.created), databaseRolesRemoved: databases.every(item => !item.roleCreated),
      temporaryDirectoryRemoved: !existsSync(ownedRoot), hostUnchanged, browserClosed }
    console.log('PHASE27_BROWSER_CLEANUP ' + JSON.stringify(cleanup))
    if (errors.length) { const error = new Error('Owned resource cleanup failed: ' + [...new Set(errors)].join(',')); error.phase27Cleanup = cleanup; throw error }
    return cleanup
  }
  async function executable(label, program, args, { cwd = repository, env = runtimeEnvironment, timeout = 180000 } = {}) {
    const child = spawn(program, args, { cwd, env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
    let output = '', failed = false
    for (const stream of [child.stdout, child.stderr]) stream.on('data', bytes => { output += bytes })
    child.on('error', () => { failed = true })
    let timedOut = false
    const timer = setTimeout(() => { timedOut = true; child.kill('SIGKILL') }, timeout)
    try {
      const code = await new Promise(resolve => child.once('close', resolve))
      if (['Clean standalone npm ci', 'Actual Vite stores'].includes(label) && process.env.STORES_BROWSER_ARTIFACT_DIR) {
        const id = label === 'Clean standalone npm ci' ? 'standalone-npm-ci' : 'standalone-npm-build'
        const logRoot = path.join(process.env.STORES_BROWSER_ARTIFACT_DIR, 'logs'); mkdirSync(logRoot, { recursive: true })
        const log = 'artifacts/logs/' + id + '.log'; writeFileSync(path.join(logRoot, id + '.log'), redact(output), { mode: 0o600 })
        publicBuildSteps.push({ id, command: [path.basename(program), ...args.map(value => value.startsWith(ownedRoot) ? '<owned-work>/' + path.relative(ownedRoot, value).replaceAll('\\', '/') : value)],
          status: !failed && code === 0 && !timedOut ? 'PASS' : 'FAIL', exitCode: code, timedOut, log })
      }
      assert(!failed && code === 0, `${label} failed: ${redact(output).slice(-6000)}`)
      for (const secret of secretValues) assert(!output.includes(secret), `${label} exposed a runtime secret`)
      return output
    } finally { clearTimeout(timer) }
  }
  try {
    const platformDb = await createDatabase('p'), storesDb = await createDatabase('s'), workOrdersDb = await createDatabase('w')
    const [platformPort, storesPort, workOrdersPort, gatewayPort, hostPort, microPort, redirectPort] = await Promise.all(Array.from({ length: 7 }, freePort))
    const platformOrigin = 'http://127.0.0.1:' + platformPort, storesOrigin = 'http://127.0.0.1:' + storesPort
    const workOrdersOrigin = 'http://127.0.0.1:' + workOrdersPort, origin = 'https://127.0.0.1:' + gatewayPort
    const password = 'Phase27-Owned-Password-' + randomBytes(12).toString('hex')
    const jwtSecret = randomBytes(48).toString('hex'), metricsToken = randomBytes(40).toString('hex')
    const storesMetrics = randomBytes(40).toString('hex'), workMetrics = randomBytes(40).toString('hex')
    secretValues.add(storesMetrics); secretValues.add(workMetrics)
    for (const value of [password, jwtSecret, metricsToken]) secretValues.add(value)
    const releases = path.join(ownedRoot, 'releases'); mkdirSync(releases)
    const servedHost = recovery ? path.join(ownedRoot, 'restorable-host') : hostDist
    if (recovery) cpSync(hostDist, servedHost, { recursive: true })
    const platformEnv = { ...runtimeEnvironment, DATABASE_URL: platformDb.url, JWT_SECRET: jwtSecret, METRICS_TOKEN: metricsToken,
      // Real cold reference bootstrap; browser measurements start after readiness.
      // Capacity/API measurements retain their separate production defaults.
      DB_STATEMENT_TIMEOUT_MS: '60000',
      BIND_ADDR: '127.0.0.1:' + platformPort, PUBLIC_APP_URL: origin, MICRO_GATEWAY_ORIGIN: origin, MICRO_ALLOWED_ORIGINS: origin,
      CORS_ALLOWED_ORIGINS: origin, APPLICATION_ENVIRONMENT: 'production', APP_ENV: 'production', AUTO_CREATE_DATABASE: 'false',
      BOOTSTRAP_ADMIN_USERNAME: 'phase27-admin', BOOTSTRAP_ADMIN_PASSWORD: password, STATIC_DIR: '', RUST_LOG: 'warn',
      OAUTH_INTROSPECTION_CLIENT_REQUESTS_PER_MINUTE: '12000', OAUTH_INTROSPECTION_ATTEMPTS_PER_MINUTE: '24000',
      MICRO_CA_CERT_FILE: process.env.STORES_BROWSER_TLS_CA,
      DATA_GRANT_POLICY: JSON.stringify({ applications: [], managementScopes: [] }) }
    let platform = launch('platform', platformBinary, [], platformEnv)
    const ready = async () => wait(async () => {
      assert(!platform.error && !platform.closed, 'Owned production platform exited before readiness: ' + platform.output.slice(-1200))
      try { return (await request(platformOrigin, '/api/health')).status === 200 } catch { return false }
    }, 'Actual production platform readiness', 90000)
    await ready()
    const platformApi = async (endpoint, options = {}, status = 200) => {
      const response = await request(platformOrigin, endpoint, options)
      assert.equal(response.status, status, `Actual platform ${options.method || 'GET'} ${endpoint}: ${response.data?.code || response.status}`)
      return response.data
    }
    async function login(username, organizationId) {
      const proof = await platformApi('/api/auth/login', { method: 'POST', body: { username, password } }); secretValues.add(proof.ticket)
      const selected = await platformApi('/api/auth/context', { token: proof.ticket, method: 'POST', body: { organizationId } })
      secretValues.add(selected.accessToken); if (selected.refreshToken) secretValues.add(selected.refreshToken)
      return selected
    }
    const adminOrg = (await platformDb.writer.query('SELECT o.id FROM organizations o JOIN tenants t ON t.id=o.tenant_id WHERE t.is_platform AND o.parent_id IS NULL')).rows[0].id
    const admin = await login('phase27-admin', adminOrg)
    const adminApi = (endpoint, method = 'GET', body) => platformApi(endpoint, { token: admin.accessToken, method, body })
    const tenant = await adminApi('/api/tenants', 'POST', { name: 'Phase27 controlled tenant', code: 'phase27-controlled' })
    const tenantRoot = (await platformDb.writer.query('SELECT id FROM organizations WHERE tenant_id=$1 AND parent_id IS NULL', [tenant.id])).rows[0].id
    const organizations = {}
    for (const key of ['A', 'B', 'C']) organizations[key] = await adminApi('/api/organizations', 'POST', {
      tenantId: tenant.id, parentId: tenantRoot, name: 'Phase27 organization ' + key, code: 'phase27-org-' + key.toLowerCase() })

    const nginxConfig = (text, label) => text.replace('worker_processes auto;', 'worker_processes 1;').replace(/pid [^;]+;/, `pid "${ownedRoot}/${label}.pid";`)
    const configs = [
      ['nginx-host', readFileSync(path.join(repository, 'deploy/static/nginx.conf'), 'utf8').replace('listen 8080;', 'listen 127.0.0.1:' + hostPort + ';').replace('/usr/share/nginx/html', servedHost)],
      ['nginx-micro', renderMicroStatic({ publicOrigin: origin, listener: '127.0.0.1:' + microPort, microRoot: releases, mimeTypes: process.env.NGINX_MIME_TYPES })],
      ['nginx-edge', renderGateway({ publicOrigin: origin, listener: '127.0.0.1:' + gatewayPort, httpListener: '127.0.0.1:' + redirectPort,
        certificate, certificateKey, mimeTypes: process.env.NGINX_MIME_TYPES, backend: '127.0.0.1:' + platformPort,
        frontend: '127.0.0.1:' + hostPort, stores: '127.0.0.1:' + storesPort, workOrders: '127.0.0.1:' + workOrdersPort,
        zipStatic: '127.0.0.1:' + microPort, trustedProxyIps: ['127.0.0.1'] })],
    ]
    for (const [label, source] of configs) {
      const prefix = path.join(ownedRoot, label); mkdirSync(prefix)
      let content = nginxConfig(source, label).replaceAll('/etc/nginx/mime.types', process.env.NGINX_MIME_TYPES)
        .replace("$remote_addr $request_method $uri $status $body_bytes_sent", "$remote_addr $request_method $request_uri $status $body_bytes_sent")
      for (const name of ['client_body_temp', 'proxy_temp', 'fastcgi_temp', 'uwsgi_temp', 'scgi_temp', 'client_body', 'proxy', 'fastcgi', 'uwsgi', 'scgi']) {
        content = content.replaceAll('/tmp/' + name + ';', prefix + '/' + name + ';').replaceAll('"/tmp/' + name + '"', '"' + prefix + '/' + name + '"')
      }
      if (!content.includes('daemon off;')) content = 'daemon off;\n' + content
      const config = path.join(prefix, 'nginx.conf'); writeFileSync(config, content)
      const nginxProcess = launch(label, nginxBinary, ['-p', prefix + '/', '-c', config])
      await wait(async () => {
        discoverNginxWorkers(nginxProcess); assert(!nginxProcess.error && !nginxProcess.closed, label + ' startup: ' + nginxDiagnostic())
        try { return (await request(label === 'nginx-edge' ? origin : 'http://127.0.0.1:' + (label === 'nginx-host' ? hostPort : microPort),
          label === 'nginx-edge' ? '/' : '/healthz', { headers: { Accept: 'text/html' } })).status === 200 } catch { return false }
      }, label + ' normal verified TLS readiness')
      discoverNginxWorkers(nginxProcess); assert(nginxProcess.identity && nginxProcess.workers.length > 0)
      console.log('PHASE27_NGINX_READY ' + JSON.stringify({ label, master: nginxProcess.identity, workers: nginxProcess.workers }))
    }

    const apps = {}, cli = []
    async function runCli(appCode, args) {
      const output = await executable('Actual Rust CLI ' + args[0] + ' ' + appCode, adminBinary,
        [...args, '--app-code', appCode, '--environment', 'production'], { env: { ...runtimeEnvironment,
          PLATFORM_API_ORIGIN: origin, PLATFORM_ADMIN_BEARER: admin.accessToken, PLATFORM_CLIENT_ID: apps[appCode].oauth.clientId,
          PLATFORM_CA_FILE: process.env.STORES_BROWSER_TLS_CA } })
      const summary = JSON.parse(output)
      assert.equal(summary.ok, true)
      cli.push({ applicationCode: appCode, command: args[0], applied: args.includes('--apply'), ok: true })
      return summary
    }
    for (const code of ['work-orders', 'stores']) {
      const oauth = await adminApi('/api/oauth/clients', 'POST', { clientName: code + '-client', clientType: 'CONFIDENTIAL',
        grantTypes: ['client_credentials'], scopes: ['openid'], redirectUris: [], accessTokenValidity: 300, refreshTokenValidity: 300 })
      secretValues.add(oauth.clientSecret)
      apps[code] = { oauth }
      await runCli(code, ['bootstrap', '--apply'])
      await runCli(code, ['bootstrap', '--apply'])
      const rows = await adminApi('/api/applications')
      const application = (Array.isArray(rows) ? rows : rows.records).find(row => row.code === code)
      assert(application, 'CLI bootstrap must register actual application')
      const definitions = JSON.parse(readFileSync(path.join(repository, 'business-apps/' + code + '/deploy/bootstrap.manifest.json'), 'utf8')).menus
      const allMenus = await adminApi('/api/menus/tree?applicationId=' + application.id)
      const flatten = rows => rows.flatMap(row => [row, ...flatten(row.children || [])])
      assert(Array.isArray(allMenus.menus), 'Actual platform menu tree contract')
      const list = flatten(allMenus.menus)
      assert.equal(list.length, definitions.length, 'Exact registered application menu count')
      const menus = new Map()
      for (const definition of definitions) {
        const row = list.find(row => row.path === definition.path && row.type === definition.type && row.permission === definition.permission)
        assert(row, 'Actual CLI menu match: ' + code + '/' + definition.key); menus.set(definition.key, row.id)
        for (const field of ['sortOrder', 'visible', 'linkTarget', 'linkUrl']) assert.equal(row[field], definition[field] ?? (field === 'linkUrl' ? '' : undefined), 'Actual CLI menu ' + field)
      }
      for (const definition of definitions) {
        const row = list.find(row => row.id === menus.get(definition.key))
        assert.equal(row.parentId, definition.parentKey ? menus.get(definition.parentKey) : null, 'Actual CLI menu parent')
      }
      assert.equal(menus.size, code === 'stores' ? 11 : 7)
      const capabilities = code === 'stores'
        ? [['store', 'read'], ['store', 'create'], ['store', 'update'], ['store-staff', 'read'], ['store-staff', 'assign'], ['store-staff', 'unassign'], ['store-daily-report', 'read'], ['store-daily-report', 'create'], ['store-daily-report', 'update']]
        : [['ticket', 'read'], ['ticket', 'create'], ['ticket', 'update'], ['ticket', 'delete'], ['ticket', 'export'], ['notification', 'publish']]
      const memberLookups = code === 'stores' ? [['store', 'create'], ['store', 'update'], ['store-staff', 'assign']] : [['ticket', 'create'], ['ticket', 'update']]
      Object.assign(apps[code], { application, definitions, menus, capabilities: capabilities.map(([resource, action]) => ({ resource, action, scopes: ['SELF', 'ORG', 'ORG_SUBTREE', 'TENANT'] })),
        memberLookups: memberLookups.map(([resource, action]) => ({ resource, action })) })
    }
    platformEnv.DATA_GRANT_POLICY = JSON.stringify({ applications: Object.values(apps).map(app => ({ applicationId: app.application.id,
      capabilities: app.capabilities, memberLookups: app.memberLookups })),
      managementScopes: [{ tenantId: tenant.id, organizationId: tenantRoot, includeDescendants: true, read: true, write: true }] })
    await stop(platform); platform = launch('platform', platformBinary, [], platformEnv); await ready()

    const actors = {}
    for (const key of ['ordinary', 'peer', 'child', 'createOnly', 'readOnly']) {
      const user = await adminApi('/api/users', 'POST', { username: 'phase27-' + key.toLowerCase(), password, realName: 'Phase27 ' + key })
      const keys = key === 'ordinary' ? ['A', 'B'] : key === 'createOnly' ? ['C'] : ['A']
      await adminApi('/api/users/' + user.id + '/memberships', 'PUT', { items: keys.map(key => ({ organizationId: organizations[key].id })) })
      const memberships = await adminApi('/api/users/' + user.id + '/memberships')
      const actor = { userId: user.id, username: user.username || 'phase27-' + key.toLowerCase(), tenantId: tenant.id, organizations: {}, defaultOrganization: keys[0] }
      actors[key] = actor
      for (const orgKey of keys) {
        const organizationId = organizations[orgKey].id
        const membershipId = memberships.find(row => row.organizationId === organizationId).membershipId
        const role = await adminApi('/api/roles', 'POST', { organizationId, name: 'Phase27 ' + key + ' ' + orgKey })
        actor.organizations[orgKey] = { organizationId, membershipId, roleId: role.id }
      }
    }
    function actorAt(actor, orgKey = actor.defaultOrganization) { return { ...actor, ...actor.organizations[orgKey], orgKey } }
    async function setGrants(actor, { appCode = 'stores', remove = [], scope = 'ORG' } = {}) {
      const app = apps[appCode], selected = actor.organizationId ? actor : actorAt(actor)
      const target = { tenantId: selected.tenantId, organizationId: selected.organizationId, userId: selected.userId,
        membershipId: selected.membershipId, applicationId: app.application.id }
      const before = await adminApi('/api/data-grants?' + new URLSearchParams(target))
      const upsert = app.capabilities.filter(item => !remove.includes(item.resource + ':' + item.action)).map(({ resource, action }) => ({ resource, action, scope }))
      const body = { target, expectedVersion: before.version, upsert, remove: remove.map(pair => { const index = pair.lastIndexOf(':'); return { resource: pair.slice(0, index), action: pair.slice(index + 1) } }) }
      await adminApi('/api/data-grants/preview', 'POST', body); await adminApi('/api/data-grants', 'PATCH', body)
    }
    for (const [key, actor] of Object.entries(actors)) for (const orgKey of Object.keys(actor.organizations)) {
      const selected = actorAt(actor, orgKey)
      for (const [code, app] of Object.entries(apps)) {
        const permissions = app.capabilities.map(item => item.resource + ':' + item.action).filter(permission =>
          key === 'child' ? code === 'stores' && !permission.startsWith('store:') : key === 'createOnly' ? permission === 'store:create' : key === 'readOnly' ? permission.endsWith(':read') : true)
        const manifest = { schemaVersion: '1.0', applicationCode: code, applicationId: app.application.id, menuIds: Object.fromEntries(app.menus),
          targets: [{ tenantId: tenant.id, organizationId: selected.organizationId, roleId: selected.roleId, userId: selected.userId,
            membershipId: selected.membershipId, permissions, dataGrants: app.capabilities.filter(item => permissions.includes(item.resource + ':' + item.action)).map(({ resource, action }) => ({ resource, action, scope: 'ORG' })) }] }
        const manifestFile = path.join(ownedRoot, key + '-' + orgKey + '-' + code + '-grants.json')
        writeFileSync(manifestFile, JSON.stringify(manifest), { mode: 0o600 })
        await runCli(code, ['grants-plan', '--manifest', manifestFile])
        await runCli(code, ['grants-apply', '--manifest', manifestFile, '--apply'])
      }
    }
    async function authorize(actor, orgKey = actor.defaultOrganization, appCode = 'stores') {
      const selected = actorAt(actor, orgKey), session = await login(actor.username, selected.organizationId)
      const token = (await platformApi('/api/oauth/session-token', { token: session.accessToken, method: 'POST', body: { appCode } }, 201)).accessToken
      secretValues.add(token); return { ...selected, session: session.accessToken, sessionResponse: session, token, appCode }
    }
    for (const [key, actor] of Object.entries(actors)) Object.assign(actor, await authorize(actor))
    const storesEnv = { ...runtimeEnvironment, STORES_DATABASE_URL: storesDb.url,
      STORES_METRICS_TOKEN: storesMetrics,
      STORES_HOST: '127.0.0.1', STORES_PORT: String(storesPort), STORES_ALLOWED_ORIGINS: 'null', PLATFORM_API_ORIGIN: origin,
      PLATFORM_CLIENT_ID: apps.stores.oauth.clientId, PLATFORM_CLIENT_SECRET: apps.stores.oauth.clientSecret,
      PLATFORM_CA_CERT_FILE: process.env.STORES_BROWSER_TLS_CA, NODE_ENV: 'production', RUST_LOG: 'warn' }
    let stores = launch('stores', businessBinary, [], storesEnv)
    const workOrdersEnv = { ...runtimeEnvironment, WORK_ORDERS_DATABASE_URL: workOrdersDb.url,
      WORK_ORDERS_METRICS_TOKEN: workMetrics,
      WORK_ORDERS_HOST: '127.0.0.1', WORK_ORDERS_PORT: String(workOrdersPort), WORK_ORDERS_ALLOWED_ORIGINS: 'null', PLATFORM_API_ORIGIN: origin,
      PLATFORM_CLIENT_ID: apps['work-orders'].oauth.clientId, PLATFORM_CLIENT_SECRET: apps['work-orders'].oauth.clientSecret,
      PLATFORM_CA_CERT_FILE: process.env.STORES_BROWSER_TLS_CA, NODE_ENV: 'production', RUST_LOG: 'warn' }
    let workOrders = launch('work-orders', workOrdersBinary, [], workOrdersEnv)
    for (const [service, base, api] of [[stores, storesOrigin, '/stores-api'], [workOrders, workOrdersOrigin, '/work-orders-api']]) await wait(async () => {
      assert(!service.error && !service.closed, service.label + ' exited: ' + service.output.slice(-1000))
      try { return (await request(base, api + '/health/ready')).status === 200 } catch { return false }
    }, 'Actual production ' + service.label + ' readiness', 45000)

    const builds = {}, manifests = {}, separateFrontend = { outsideRepository: true, rootNodeModulesDependency: false, personalCacheDependency: false,
      steps: publicBuildSteps, cache: { kind: 'fresh-owned-npm-cache', personal: false, directory: '<owned-work>/npm-cache' } }
    const isolatedApp = path.join(ownedRoot, 'standalone-stores'); mkdirSync(isolatedApp)
    cpSync(path.join(repository, 'business-apps/stores/frontend'), isolatedApp, { recursive: true, filter: source => !['node_modules', 'dist', '.vite'].includes(path.basename(source)) })
    const sentinel = 'PHASE27_SECRET_SENTINEL_' + randomBytes(24).toString('hex'); secretValues.add(sentinel)
    const cleanEnv = { ...runtimeEnvironment, npm_config_cache: path.join(ownedRoot, 'npm-cache'), VITE_CLIENT_SECRET: sentinel, VITE_DATABASE_URL: sentinel }
    separateFrontend.emptyCacheBeforeInstall = !existsSync(cleanEnv.npm_config_cache)
    separateFrontend.noNodeModulesBeforeInstall = !existsSync(path.join(isolatedApp, 'node_modules'))
    assert(separateFrontend.emptyCacheBeforeInstall && separateFrontend.noNodeModulesBeforeInstall)
    await executable('Clean standalone npm ci', 'npm', ['ci', '--no-audit', '--no-fund'], { cwd: isolatedApp, env: cleanEnv })
    separateFrontend.npmCi = true
    for (const code of ['work-orders', 'stores']) {
      const appDist = path.join(ownedRoot, code + '-app')
      const buildScript = code === 'stores' ? path.join(isolatedApp, 'scripts/build.mjs') : path.join(repository, 'business-apps/work-orders/frontend/scripts/build.mjs')
      await executable('Actual Vite ' + code, process.execPath, [buildScript, '--out-dir', appDist, '--version', 'phase27', '--allowed-host-origin', origin], { env: cleanEnv })
      await command('Actual static release ' + code, ['scripts/package-microapp.mjs', '--app', code, '--environment', 'production', '--version', 'phase27', '--input', appDist, '--output-root', releases])
      const manifest = JSON.parse(readFileSync(path.join(releases, code, 'versions/phase27/release.json'), 'utf8'))
      for (const resource of tree(appDist)) for (const secret of secretValues) assert(!readFileSync(path.join(appDist, resource.path)).includes(Buffer.from(secret)), 'Frontend secret scan')
      builds[code] = tree(appDist); manifests[code] = manifest
      if (process.env.STORES_BROWSER_ARTIFACT_DIR) {
        const artifactRoot = path.resolve(process.env.STORES_BROWSER_ARTIFACT_DIR)
        assert(path.isAbsolute(process.env.STORES_BROWSER_ARTIFACT_DIR) && !artifactRoot.startsWith(repository + path.sep), 'Public artifacts stay outside source')
        mkdirSync(artifactRoot, { recursive: true, mode: 0o755 })
        const destination = path.join(artifactRoot, code + '-dist'); assert(!existsSync(destination), 'Fresh public application artifacts required')
        cpSync(appDist, destination, { recursive: true })
      }
      const app = apps[code], release = await adminApi('/api/applications/' + app.application.id + '/releases', 'POST', manifest)
      const preflight = await adminApi('/api/applications/' + app.application.id + '/releases/' + release.id + '/preflight', 'POST')
      assert.equal(preflight.validationStatus, 'PASSED', 'Actual production release preflight: ' + code)
      await adminApi('/api/applications/' + app.application.id + '/releases/' + release.id + '/activate', 'POST')
    }
    separateFrontend.npmBuild = true; separateFrontend.secretSentinelAbsent = true
    for (const resource of hostBefore) for (const secret of secretValues) assert(!readFileSync(path.join(hostDist, resource.path)).includes(Buffer.from(secret)), 'Actual production host frontend secret scan')
    const businessCall = (actor, endpoint = API, options = {}) => request(origin, endpoint, { ...options, token: actor.token })
    async function observePrivateMetrics() {
      const observations = []
      for (const [base, endpoint, token] of [[platformOrigin, '/api/internal/metrics', metricsToken], [storesOrigin, '/stores-api/internal/metrics', storesMetrics], [workOrdersOrigin, '/work-orders-api/internal/metrics', workMetrics]]) {
        const privateReply = await request(base, endpoint, { token })
        const publicReply = await request(origin, endpoint, { token })
        assert.equal(privateReply.status, 200); assert.equal(publicReply.status, 404)
        assert(!privateReply.text.includes(token) && !publicReply.text.includes('# TYPE '))
        observations.push({ endpoint, privateStatus: privateReply.status, publicStatus: publicReply.status, metrics: privateReply.text })
      }
      return observations
    }
    const memberHttpObservations = () => processes.filter(item => item.label === 'nginx-edge').flatMap(item =>
      [...item.logPaths.map(file => readFileSync(file, 'utf8')).join('\n').matchAll(/(?:^|\n)\S+ GET (\/api\/apps\/(stores|work-orders)\/members[^\s]*) (\d{3}) /g)]
        .map(match => ({ app: match[2], path: match[1], status: Number(match[3]), transport: 'actual-nginx-platform-http' })))
    const recoveryControl = recovery ? {
      platformOrigin,
      async quiesce() {
        for (const item of [stores,workOrders,platform]) await stop(item)
        for (const item of processes.filter(item=>item.nginx)) await stop(item)
        for (const item of [platformDb,storesDb,workOrdersDb]) {
          const busy = await item.admin.query("SELECT count(*)::int AS n FROM pg_stat_activity WHERE datname=$1 AND pid<>pg_backend_pid() AND state<>'idle'",[item.name])
          assert.equal(busy.rows[0].n,0,'All test writers must be drained')
        }
        return { adminUrl:adminEndpoint.href, databases:[platformDb,storesDb,workOrdersDb].map((d,i)=>({kind:['platform','stores','work_orders'][i],sourceUrl:d.url,sourceName:d.name,owner:d.role,target:'recovery_'+d.kind+'_'+randomUUID().replaceAll('-','')})),
          host:servedHost,releases,configs:configs.map(([label])=>path.join(ownedRoot,label,'nginx.conf')),
          binaries:{platform:platformBinary,stores:businessBinary,work_orders:workOrdersBinary} }
      },
      async restart(plan, restoredRoot) {
        assert(plan.databases.length===3)
        for (const [i,original] of [platformDb,storesDb,workOrdersDb].entries()) {
          const d=plan.databases[i]; assert.match(d.target,/^recovery_[psw]_[0-9a-f]{32}$/)
          const adminClient=new pg.Client({connectionString:adminEndpoint.href}); await adminClient.connect()
          const u=new URL(original.url);u.pathname='/'+d.target;const url=u.href;secretValues.add(url)
          const writer=new pg.Client({connectionString:url});await writer.connect()
          databases.push({name:d.target,role:d.owner,kind:original.kind,created:true,roleCreated:false,admin:adminClient,writer})
          if(i===0)platformEnv.DATABASE_URL=url;else if(i===1)storesEnv.STORES_DATABASE_URL=url;else workOrdersEnv.WORK_ORDERS_DATABASE_URL=url
        }
        // Only fixture-owned public roots are replaced, while every write service is stopped.
        for(const [source,dest] of [['runtime/releases',releases],['host',servedHost]]) {
          assert(dest.startsWith(ownedRoot+path.sep));rmSync(dest,{recursive:true,force:true});cpSync(path.join(restoredRoot,source),dest,{recursive:true})
        }
        const routeRestoredHashes=[]
        for(const [label] of configs){
          const prefix=path.join(ownedRoot,label),config=path.join(prefix,'nginx.conf'),saved=path.join(restoredRoot,'routes',label+'.conf')
          cpSync(saved,config);assert.equal(sha(readFileSync(saved)),sha(readFileSync(config)))
          const proc=launch(label,nginxBinary,['-p',prefix+'/','-c',config])
          await wait(async()=>{discoverNginxWorkers(proc);assert(!proc.closed&&!proc.error,'Restored Nginx must remain alive');try{return(await request(label==='nginx-edge'?origin:'http://127.0.0.1:'+(label==='nginx-host'?hostPort:microPort),label==='nginx-edge'?'/':'/healthz',{headers:{Accept:'text/html'}})).status===200}catch{return false}},'Restored actual Nginx route readiness')
          routeRestoredHashes.push({component:label,sha256:sha(readFileSync(config)),restartedFromRestoredConfig:true})
        }
        platform=launch('platform',path.join(restoredRoot,'runtime/bin/platform'),[],platformEnv);await ready()
        stores=launch('stores',path.join(restoredRoot,'runtime/bin/stores'),[],storesEnv);workOrders=launch('work-orders',path.join(restoredRoot,'runtime/bin/work_orders'),[],workOrdersEnv)
        for(const [service,base,api] of [[stores,storesOrigin,'/stores-api'],[workOrders,workOrdersOrigin,'/work-orders-api']])await wait(async()=>{assert(!service.error&&!service.closed,service.label+' restored startup failed: '+service.output.slice(-1200));try{return(await request(base,api+'/health/ready')).status===200}catch{return false}},'Restored real service readiness',90000)
        Object.assign(admin,await login('phase27-admin',adminOrg))
        return {...Object.fromEntries(databases.slice(-3).map((d,i)=>[['platform','stores','work_orders'][i],d.writer])),routeRestoredHashes}
      }
    } : undefined
    return { origin, apps, actors, organizations, memberHttpObservations, application: apps.stores.application, menus: apps.stores.menus,
      platformDb: platformDb.writer, businessDb: storesDb.writer, workOrdersDb: workOrdersDb.writer, request, platformApi, adminApi, redact,
      businessCall, setGrants, authorize, actorAt, close, ownedRoot, observePrivateMetrics, recoveryControl,
      source: { platformBinary: { sha256: sha(readFileSync(platformBinary)) }, businessBinary: { sha256: sha(readFileSync(businessBinary)) },
        workOrdersBinary: { sha256: sha(readFileSync(workOrdersBinary)) }, adminBinary: { sha256: sha(readFileSync(adminBinary)) },
        productionHost: hostBefore, viteBuild: builds.stores, workOrdersViteBuild: builds['work-orders'], releaseManifest: manifests.stores,
        workOrdersReleaseManifest: manifests['work-orders'], gatewayTlsVerified: true, productionServices: ['platform', 'stores', 'work-orders'],
        databases: databases.map(item => item.identity), separateFrontend, cli, menus: phase27MenuCounts(apps) } }
  } catch (error) {
    try { error.phase27StartupCleanup = await close() }
    catch (cleanupError) { error.phase27StartupCleanup = cleanupError.phase27Cleanup || { failed: true } }
    error.message = redact(error.message); throw error
  }
}
