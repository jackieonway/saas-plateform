/** Owned test control plane. Product operations always use the actual Rust HTTP services. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { randomBytes, randomUUID, createHash } from 'node:crypto'
import { createRequire } from 'node:module'
import http from 'node:http'
import https from 'node:https'
import net from 'node:net'
import { chmodSync, chownSync, closeSync, existsSync, lstatSync, mkdirSync, mkdtempSync, openSync, readFileSync, readdirSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { renderGateway, renderMicroStatic } from '../../deploy/gateway/render.mjs'

export const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
export const API = '/work-orders-api/tickets'
export const APP = 'work-orders'
const required = ['TEST_DATABASE_URL', 'CI_TEST_POSTGRES_OWNER', 'WORK_ORDERS_BROWSER_PLATFORM_BINARY', 'WORK_ORDERS_BROWSER_BUSINESS_BINARY',
  'WORK_ORDERS_BROWSER_HOST_DIST', 'NGINX_BIN', 'NGINX_MIME_TYPES',
  'WORK_ORDERS_BROWSER_TLS_CERT', 'WORK_ORDERS_BROWSER_TLS_KEY', 'WORK_ORDERS_BROWSER_TLS_CA']
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

export async function startPhase21Services() {
  assert.equal(process.platform, 'linux', 'The owned production browser fixture requires its managed Linux environment')
  for (const key of required) assert(process.env[key], `Missing required Phase21 browser setting: ${key}`)
  const adminEndpoint = new URL(process.env.TEST_DATABASE_URL)
  const postgresOwner = process.env.CI_TEST_POSTGRES_OWNER
  assert(/^ci-[a-z0-9-]{8,80}$/.test(postgresOwner), 'Explicit managed PostgreSQL owner required')
  assert(adminEndpoint.protocol === 'postgresql:' && adminEndpoint.hostname === postgresOwner + '-postgres' &&
    adminEndpoint.port === '5432' && adminEndpoint.username === 'ci' && adminEndpoint.password &&
    adminEndpoint.pathname === '/postgres' && !adminEndpoint.search && !adminEndpoint.hash,
  'The browser fixture requires its own managed disposable PostgreSQL administration endpoint')
  const pg = createRequire(path.join(repository, 'mini-services/notification-ws/package.json'))('pg')
  const platformBinary = file(process.env.WORK_ORDERS_BROWSER_PLATFORM_BINARY, 'Platform binary')
  const businessBinary = file(process.env.WORK_ORDERS_BROWSER_BUSINESS_BINARY, 'Business binary')
  const nginxBinary = file(process.env.NGINX_BIN, 'Nginx binary')
  const hostDist = realpathSync(process.env.WORK_ORDERS_BROWSER_HOST_DIST)
  const html = readFileSync(path.join(hostDist, 'index.html'), 'utf8')
  assert(/assets\/.+\.js/.test(html) && !/@vite\/client|\/src\/main/.test(html), 'Use the actual built production host')
  const hostBefore = tree(hostDist)
  const certificate = file(process.env.WORK_ORDERS_BROWSER_TLS_CERT, 'TLS certificate')
  const certificateKey = file(process.env.WORK_ORDERS_BROWSER_TLS_KEY, 'TLS key')
  const ca = readFileSync(file(process.env.WORK_ORDERS_BROWSER_TLS_CA, 'TLS CA'))
  const temporaryParent = realpathSync(os.tmpdir())
  const ownedRoot = realpathSync(mkdtempSync(path.join(temporaryParent, 'saas-phase21-browser-')))
  // Nginx workers use their configured unprivileged account. The directory is
  // traversable, while secrets remain only in the separate launcher-owned TLS tree.
  chmodSync(ownedRoot, 0o755)
  const processes = [], databases = [], logs = []
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
    if (item.nginx) console.log('PHASE21_NGINX_STOP ' + JSON.stringify({ label: item.label, master: item.identity,
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
    if (token) headers = { ...headers, Authorization: 'Bearer ' + token }
    headers = { ...headers, 'X-Request-Id': randomUUID() }
    if (body !== undefined) headers = { ...headers, 'Content-Type': 'application/json' }
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
      req.end(body === undefined ? undefined : JSON.stringify(body))
    })
  }
  async function createDatabase(kind) {
    const name = 'phase21_work_orders_' + kind + '_' + randomUUID().replaceAll('-', '')
    const url = new URL(adminEndpoint)
    assert(['postgres:', 'postgresql:'].includes(url.protocol), 'Explicit PostgreSQL administration URL required')
    assert.equal(url.pathname, '/postgres', 'Administration URL must select postgres, never an application database')
    const admin = new pg.Client({ connectionString: url.href, connectionTimeoutMillis: 10_000, query_timeout: 15_000 })
    const item = { name, kind, admin, created: false, writer: null }; databases.push(item)
    admin.on('error', () => {})
    await admin.connect(); await admin.query(`CREATE DATABASE "${name}" TEMPLATE template0`); item.created = true
    url.pathname = '/' + name; secretValues.add(url.href)
    item.url = url.href
    item.writer = new pg.Client({ connectionString: url.href, connectionTimeoutMillis: 10_000, query_timeout: 15_000 })
    item.writer.on('error', () => {}); await item.writer.connect()
    assert.equal((await item.writer.query('SELECT current_database() AS name')).rows[0].name, name)
    return item
  }
  function makeOwnedDirectoriesRemovable(directory = ownedRoot) {
    const absolute = path.resolve(directory), relative = path.relative(ownedRoot, absolute)
    assert(absolute === ownedRoot || (relative && !path.isAbsolute(relative) && relative !== '..' && !relative.startsWith('..' + path.sep)),
      'Temporary cleanup cannot leave its verified owned root')
    const stat = lstatSync(absolute)
    assert(!stat.isSymbolicLink(), 'Temporary cleanup rejects links')
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
        assert.match(item.name, /^phase21_work_orders_[pb]_[0-9a-f]{32}$/)
        if (item.created) { await item.admin.query(`DROP DATABASE "${item.name}" WITH (FORCE)`); item.created = false }
      } catch { errors.push('database') }
      try { await item.admin.end() } catch { errors.push('administration') }
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
        assert(path.basename(ownedRoot).startsWith('saas-phase21-browser-') && !lstatSync(ownedRoot).isSymbolicLink())
        assert(processes.every(processStopped), 'Temporary ownership recovery requires all owned processes and workers to have stopped')
        makeOwnedDirectoriesRemovable()
        rmSync(ownedRoot, { recursive: true, force: true })
      } catch { errors.push('temporary-directory') }
    }
    const cleanup = { platformDatabaseRemoved: databases.filter(item => item.kind === 'p').every(item => !item.created),
      businessDatabaseRemoved: databases.filter(item => item.kind === 'b').every(item => !item.created),
      platformStopped: processes.filter(item => item.label === 'platform').every(processStopped),
      businessStopped: processes.filter(item => item.label === 'business').every(processStopped),
      nginxStopped: processes.filter(item => item.nginx).every(processStopped),
      processesStopped: processes.every(processStopped), databasesRemoved: databases.every(item => !item.created),
      temporaryDirectoryRemoved: !existsSync(ownedRoot), hostUnchanged, browserClosed }
    console.log('PHASE21_BROWSER_CLEANUP ' + JSON.stringify(cleanup))
    if (errors.length) { const error = new Error('Owned resource cleanup failed: ' + [...new Set(errors)].join(',')); error.phase21Cleanup = cleanup; throw error }
    return cleanup
  }
  try {
    const platformDb = await createDatabase('p'), businessDb = await createDatabase('b')
    const [platformPort, businessPort, gatewayPort, hostPort, microPort, redirectPort] = await Promise.all(Array.from({ length: 6 }, freePort))
    const platformOrigin = 'http://127.0.0.1:' + platformPort, businessOrigin = 'http://127.0.0.1:' + businessPort
    const origin = 'https://127.0.0.1:' + gatewayPort
    const jwtSecret = randomBytes(48).toString('hex'), metricsToken = randomBytes(40).toString('hex'), businessMetricsToken = randomBytes(40).toString('hex')
    for (const secret of [jwtSecret, metricsToken, businessMetricsToken]) secretValues.add(secret)
    writeFileSync(path.join(ownedRoot, '.env'), '')
    // Cold reference-data bootstrap is setup, outside the capacity measurement.
    const platformEnv = { ...runtimeEnvironment, DATABASE_URL: platformDb.url, JWT_SECRET: jwtSecret, METRICS_TOKEN: metricsToken, DB_STATEMENT_TIMEOUT_MS: '60000', BIND_ADDR: '127.0.0.1:' + platformPort,
      PUBLIC_APP_URL: origin, MICRO_GATEWAY_ORIGIN: origin, MICRO_ALLOWED_ORIGINS: origin, CORS_ALLOWED_ORIGINS: origin,
      APPLICATION_ENVIRONMENT: 'development', AUTO_CREATE_DATABASE: 'false', STATIC_DIR: '', RUST_LOG: 'warn',
      OAUTH_INTROSPECTION_CLIENT_REQUESTS_PER_MINUTE: '600', OAUTH_INTROSPECTION_ATTEMPTS_PER_MINUTE: '1200',
      MICRO_CA_CERT_FILE: process.env.WORK_ORDERS_BROWSER_TLS_CA,
      DATA_GRANT_POLICY: JSON.stringify({ applications: [], managementScopes: [] }) }
    let platform = launch('platform', platformBinary, [], platformEnv)
    const ready = async () => wait(async () => {
      assert(!platform.error && !platform.closed, 'Owned platform exited before readiness: ' + platform.output.slice(-2400))
      try { return (await request(platformOrigin, '/api/health')).status === 200 } catch { return false }
    }, 'Real platform readiness', 60_000)
    await ready()
    const platformApi = async (endpoint, options = {}, status = 200) => {
      const response = await request(platformOrigin, endpoint, options)
      assert.equal(response.status, status, `Real platform ${options.method || 'GET'} ${endpoint} rejected: ${response.data?.code || response.status}`)
      return response.data
    }
    async function login(username, organizationId, password = 'admin123') {
      const proof = await platformApi('/api/auth/login', { method: 'POST', body: { username, password } }); secretValues.add(proof.ticket)
      const selected = await platformApi('/api/auth/context', { token: proof.ticket, method: 'POST', body: { organizationId } }); secretValues.add(selected.accessToken)
      return selected
    }
    const adminOrg = (await platformDb.writer.query('SELECT o.id FROM organizations o JOIN tenants t ON t.id=o.tenant_id WHERE t.is_platform AND o.parent_id IS NULL')).rows[0].id
    const admin = await login('admin', adminOrg)
    const adminApi = (endpoint, method, body) => platformApi(endpoint, { token: admin.accessToken, method, body })
    const tenant = await adminApi('/api/tenants', 'POST', { name: 'Phase21 owned tenant', code: 'phase21-owned' })
    const tenantRoot = (await platformDb.writer.query('SELECT id FROM organizations WHERE tenant_id=$1 AND parent_id IS NULL', [tenant.id])).rows[0].id
    const organization = await adminApi('/api/organizations', 'POST', { tenantId: tenant.id, parentId: tenantRoot, name: 'Phase21 business', code: 'phase21-business' })
    const application = await adminApi('/api/applications', 'POST', { name: 'Phase21 work orders', code: APP, entryType: 'HTML', runtimeMode: 'IFRAME',
      entryUrl: '/sub-apps/work-orders/versions/phase21/index.html', basePath: '/micro/work-orders' })
    const menus = new Map(), menuIds = []
    const definitions = JSON.parse(readFileSync(path.join(repository, 'tools/platform-admin/manifests/bootstrap.manifest.json'), 'utf8')).menus
    for (const definition of definitions) {
      const { key, parentKey, ...body } = definition
      const menu = await adminApi('/api/menus', 'POST', { ...body, applicationId: application.id, ...(parentKey ? { parentId: menus.get(parentKey) } : {}) })
      menus.set(key, menu.id); menuIds.push(menu.id)
    }
    const authorization = { applications: [{ applicationId: application.id, authorized: true }], menus: menuIds.map(menuId => ({ menuId, authorized: true })) }
    await adminApi('/api/tenants/' + tenant.id + '/authorizations', 'PUT', authorization)
    for (const org of [tenantRoot, organization.id]) await adminApi('/api/organizations/' + org + '/authorizations', 'PUT', authorization)
    const password = 'Phase21-Owned-Password-1!'; secretValues.add(password)
    const actors = {}
    for (const key of ['ordinary', 'peer', 'createOnly', 'readOnly']) {
      const user = await adminApi('/api/users', 'POST', { username: 'phase21-' + key.toLowerCase(), password, realName: 'Phase21 ' + key })
      await adminApi('/api/users/' + user.id + '/memberships', 'PUT', { items: [{ organizationId: organization.id }] })
      const membership = (await adminApi('/api/users/' + user.id + '/memberships', 'GET')).find(row => row.organizationId === organization.id)
      const selectedMenus = key === 'createOnly' ? [menus.get('root'), menus.get('ticket:create')]
        : key === 'readOnly' ? [menus.get('root'), menus.get('ticket:read')] : menuIds
      const role = await adminApi('/api/roles', 'POST', { organizationId: organization.id, name: 'Phase21 ' + key })
      await adminApi('/api/roles/' + role.id + '/menus', 'PUT', { menuIds: selectedMenus })
      await adminApi('/api/roles/' + role.id + '/users', 'PUT', { userIds: [user.id] })
      actors[key] = { userId: user.id, membershipId: membership.membershipId, roleId: role.id,
        tenantId: tenant.id, organizationId: organization.id, username: user.username || 'phase21-' + key.toLowerCase() }
    }
    const oauth = await adminApi('/api/oauth/clients', 'POST', { clientName: 'Phase21 work orders', clientType: 'CONFIDENTIAL', grantTypes: ['password', 'refresh_token'], scopes: ['profile'] })
    await adminApi('/api/oauth/clients/' + oauth.id, 'PUT', { applicationId: application.id })
    secretValues.add(oauth.clientSecret)
    const capabilities = ['read', 'create', 'update', 'delete', 'export'].map(action => ({ resource: 'ticket', action, scopes: ['SELF', 'ORG', 'ORG_SUBTREE', 'TENANT'] }))
    capabilities.push({ resource: 'notification', action: 'publish', scopes: ['SELF', 'ORG', 'ORG_SUBTREE', 'TENANT'] })
    platformEnv.DATA_GRANT_POLICY = JSON.stringify({ applications: [{ applicationId: application.id, capabilities,
      memberLookups: ['create', 'update'].map(action => ({ resource: 'ticket', action })) }],
    managementScopes: [{ tenantId: tenant.id, organizationId: tenantRoot, includeDescendants: true, read: true, write: true }] })
    await stop(platform); platform = launch('platform', platformBinary, [], platformEnv); await ready()
    async function setGrants(actor, removeRead = false) {
      const target = { tenantId: actor.tenantId, organizationId: actor.organizationId, userId: actor.userId,
        membershipId: actor.membershipId, applicationId: application.id }
      const endpoint = '/api/data-grants?' + new URLSearchParams(target)
      const before = await adminApi(endpoint, 'GET')
      const upsert = capabilities.filter(item => !(removeRead && item.resource === 'ticket' && item.action === 'read'))
        .map(item => ({ resource: item.resource, action: item.action, scope: 'ORG' }))
      const body = { target, expectedVersion: before.version, upsert, remove: removeRead ? [{ resource: 'ticket', action: 'read' }] : [] }
      await adminApi('/api/data-grants/preview', 'POST', body)
      await adminApi('/api/data-grants', 'PATCH', body)
    }
    for (const actor of Object.values(actors)) {
      await setGrants(actor)
      actor.session = (await login(actor.username, organization.id, password)).accessToken
      actor.token = (await platformApi('/api/oauth/session-token', { token: actor.session, method: 'POST', body: { appCode: APP } }, 201)).accessToken
      secretValues.add(actor.token)
    }
    const businessEnv = { ...runtimeEnvironment, WORK_ORDERS_DATABASE_URL: businessDb.url, WORK_ORDERS_METRICS_TOKEN: businessMetricsToken, WORK_ORDERS_HOST: '127.0.0.1', WORK_ORDERS_PORT: String(businessPort),
      WORK_ORDERS_ALLOWED_ORIGINS: 'null', PLATFORM_API_ORIGIN: platformOrigin, PLATFORM_CLIENT_ID: oauth.clientId,
      PLATFORM_CLIENT_SECRET: oauth.clientSecret, NODE_ENV: 'test', RUST_LOG: 'warn' }
    const business = launch('business', businessBinary, [], businessEnv)
    await wait(async () => { assert(!business.error && !business.closed, 'Owned Rust business exited before readiness')
      try { return (await request(businessOrigin, '/work-orders-api/health/ready')).status === 200 } catch { return false }
    }, 'Real Rust business readiness', 45_000)
    const appDist = path.join(ownedRoot, 'app'), releases = path.join(ownedRoot, 'releases')
    await command('Actual Vite app build', ['business-apps/work-orders/frontend/scripts/build.mjs', '--out-dir', appDist, '--version', 'phase21', '--allowed-host-origin', origin])
    await command('Actual immutable app package', ['scripts/package-microapp.mjs', '--app', APP, '--environment', 'development', '--version', 'phase21', '--input', appDist, '--output-root', releases])
    const releaseDirectory = path.join(releases, APP, 'versions/phase21')
    const manifest = JSON.parse(readFileSync(path.join(releaseDirectory, 'release.json'), 'utf8'))
    for (const resource of tree(appDist)) {
      const bytes = readFileSync(path.join(appDist, resource.path))
      for (const secret of secretValues) if (secret) assert(!bytes.includes(Buffer.from(secret)), 'Built frontend cannot contain runtime credentials')
    }
    const nginxConfig = (text, label) => text.replace('worker_processes auto;', 'worker_processes 1;').replace(/pid [^;]+;/, `pid "${ownedRoot}/${label}.pid";`)
    const configs = [
      ['nginx-host', readFileSync(path.join(repository, 'deploy/static/nginx.conf'), 'utf8').replace('listen 8080;', 'listen 127.0.0.1:' + hostPort + ';').replace('/usr/share/nginx/html', hostDist)],
      ['nginx-micro', renderMicroStatic({ publicOrigin: origin, listener: '127.0.0.1:' + microPort, microRoot: releases, mimeTypes: process.env.NGINX_MIME_TYPES })],
      ['nginx-edge', renderGateway({ publicOrigin: origin, listener: '127.0.0.1:' + gatewayPort, httpListener: '127.0.0.1:' + redirectPort,
        certificate, certificateKey, mimeTypes: process.env.NGINX_MIME_TYPES,
        backend: '127.0.0.1:' + platformPort, frontend: '127.0.0.1:' + hostPort, workOrders: '127.0.0.1:' + businessPort,
        zipStatic: '127.0.0.1:' + microPort, trustedProxyIps: ['127.0.0.1'] })],
    ]
    for (const [label, source] of configs) {
      const prefix = path.join(ownedRoot, label); mkdirSync(prefix)
      let content = nginxConfig(source, label).replaceAll('/etc/nginx/mime.types', process.env.NGINX_MIME_TYPES)
      for (const name of ['client_body_temp', 'proxy_temp', 'fastcgi_temp', 'uwsgi_temp', 'scgi_temp', 'client_body', 'proxy', 'fastcgi', 'uwsgi', 'scgi']) {
        content = content.replaceAll('/tmp/' + name + ';', prefix + '/' + name + ';').replaceAll('"/tmp/' + name + '"', '"' + prefix + '/' + name + '"')
      }
      if (!content.includes('daemon off;')) content = 'daemon off;\n' + content
      const config = path.join(prefix, 'nginx.conf'); writeFileSync(config, content)
      const nginxProcess = launch(label, nginxBinary, ['-p', prefix + '/', '-c', config])
      const readiness = { lastHttpStatus: null, lastTransportCode: null }
      try {
        await wait(async () => {
          discoverNginxWorkers(nginxProcess)
          assert(!nginxProcess.error && !nginxProcess.closed, label + ' exited before readiness')
          try {
            const result = await request(label === 'nginx-edge' ? origin : 'http://127.0.0.1:' + (label === 'nginx-host' ? hostPort : microPort),
              label === 'nginx-edge' ? '/' : '/healthz', label === 'nginx-edge' ? { headers: { Accept: 'text/html' } } : {})
            readiness.lastHttpStatus = result.status; readiness.lastTransportCode = null
            return result.status === 200
          } catch (error) { readiness.lastTransportCode = /^[A-Z0-9_]{2,80}$/.test(error.code || '') ? error.code : 'OWNED_TRANSPORT_FAILURE'; return false }
        }, label + ' readiness')
        discoverNginxWorkers(nginxProcess)
        assert(nginxProcess.identity && nginxProcess.workers.length > 0, 'Actual Nginx master and worker identities must be registered')
        console.log('PHASE21_NGINX_READY ' + JSON.stringify({ label, master: nginxProcess.identity, workers: nginxProcess.workers, ...readiness }))
      } catch (error) {
        error.message = `${error.message}; ${JSON.stringify(readiness)}\n${nginxDiagnostic()}`
        throw error
      }
    }
    const release = await adminApi('/api/applications/' + application.id + '/releases', 'POST', manifest)
    const preflight = await adminApi('/api/applications/' + application.id + '/releases/' + release.id + '/preflight', 'POST')
    assert.equal(preflight.validationStatus, 'PASSED', 'Production Vite release must pass actual platform preflight')
    await adminApi('/api/applications/' + application.id + '/releases/' + release.id + '/activate', 'POST')
    const businessCall = (actor, endpoint = API, options = {}) => request(origin, endpoint, { ...options, token: actor.token })
    return { origin, application, actors, menus, platformDb: platformDb.writer, businessDb: businessDb.writer, request, platformApi, adminApi, redact,
      businessCall, setGrants, close, source: { platformBinary: { sha256: sha(readFileSync(platformBinary)) }, businessBinary: { sha256: sha(readFileSync(businessBinary)) },
        productionHost: hostBefore, viteBuild: tree(appDist), releaseManifest: manifest, developmentLoopbackUpstreams: true, gatewayTlsVerified: true }, ownedRoot }
  } catch (error) {
    try { error.phase21StartupCleanup = await close() }
    catch (cleanupError) { error.phase21StartupCleanup = cleanupError.phase21Cleanup || { failed: true } }
    error.message = redact(error.message)
    throw error
  }
}
