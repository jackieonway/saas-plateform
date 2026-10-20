/** Private test control plane. All product operations use the real Rust APIs/CLI. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createHash, randomUUID } from 'node:crypto'
import { createRequire } from 'node:module'
import { existsSync, lstatSync, mkdirSync, readFileSync, realpathSync, writeFileSync } from 'node:fs'
import https from 'node:https'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

export const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
export const applicationCodes = Object.freeze(['stores', 'work-orders'])
export const phase28DeliveryChecks = Object.freeze(['url-ui-draft-preflight-activate', 'preflight-http200-failed-rejected',
  'activate-rechecks-unavailable-entry', 'versioned-chunks-rollback-retained', 'ordinary-user-sdk-rust-authorization',
  'external-origin-api-cors-csp', 'artifact-inventory-conflicts', 'image-deployment-does-not-grant', 'independent-databases-and-services'])
export const delay = ms => new Promise(resolve => setTimeout(resolve, ms))
const sha = bytes => createHash('sha256').update(bytes).digest('hex')
export async function wait(predicate, message, timeout = 30_000) {
  const deadline = Date.now() + timeout
  do { if (await predicate()) return; await delay(40) } while (Date.now() < deadline)
  assert(await predicate(), message)
}
function regularFile(value, label) {
  assert(path.isAbsolute(value || ''), `${label} requires an absolute path`)
  assert(!lstatSync(value).isSymbolicLink() && lstatSync(value).isFile(), `${label} requires a regular file`)
  return realpathSync(value)
}
function exactHttps(value) {
  const url = new URL(value)
  assert.equal(url.protocol, 'https:'); assert.equal(url.origin, value)
  assert(!url.username && !url.password)
  return value
}

/** Read versions and catalog definitions with the component's own DB account.
 * Only their hash and immutable migration metadata leave the private fixture. */
async function databaseVersionEvidence(client) {
  const version = (await client.query("SELECT pg_catalog.version() AS full_version,pg_catalog.current_setting('server_version') AS server_version")).rows[0]
  const relations = (await client.query(`SELECT c.relname AS name,c.relkind AS kind,c.relpersistence AS persistence,
    CASE WHEN c.relkind IN ('v','m') THEN pg_catalog.pg_get_viewdef(c.oid,true) ELSE NULL END AS view_definition
    FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
    WHERE n.nspname='public' AND c.relkind IN ('r','p','v','m','S') ORDER BY c.relname,c.relkind`)).rows
  const columns = (await client.query(`SELECT c.relname AS relation,a.attnum AS position,a.attname AS name,
    pg_catalog.format_type(a.atttypid,a.atttypmod) AS type,a.attnotnull AS not_null,a.attidentity AS identity,a.attgenerated AS generated,
    pg_catalog.pg_get_expr(d.adbin,d.adrelid) AS default_expression
    FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
    LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum
    WHERE n.nspname='public' AND a.attnum>0 AND NOT a.attisdropped AND c.relkind IN ('r','p','v','m','S')
    ORDER BY c.relname,a.attnum`)).rows
  const constraints = (await client.query(`SELECT r.relname AS relation,c.conname AS name,c.contype AS kind,
    pg_catalog.pg_get_constraintdef(c.oid,true) AS definition
    FROM pg_catalog.pg_constraint c JOIN pg_catalog.pg_class r ON r.oid=c.conrelid
    JOIN pg_catalog.pg_namespace n ON n.oid=r.relnamespace WHERE n.nspname='public'
    ORDER BY r.relname,c.conname`)).rows
  const indexes = (await client.query("SELECT tablename AS relation,indexname AS name,indexdef AS definition FROM pg_catalog.pg_indexes WHERE schemaname='public' ORDER BY tablename,indexname")).rows
  const enums = (await client.query(`SELECT t.typname AS name,e.enumsortorder AS position,e.enumlabel AS label
    FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace
    JOIN pg_catalog.pg_enum e ON e.enumtypid=t.oid WHERE n.nspname='public' ORDER BY t.typname,e.enumsortorder`)).rows
  const routines = (await client.query(`SELECT p.proname AS name,pg_catalog.pg_get_function_identity_arguments(p.oid) AS arguments,
    pg_catalog.pg_get_functiondef(p.oid) AS definition FROM pg_catalog.pg_proc p
    JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public' AND p.prokind IN ('f','p')
    ORDER BY p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid)`)).rows
  const histories = (await client.query("SELECT pg_catalog.to_regclass('public._sqlx_migrations') AS sqlx,pg_catalog.to_regclass('public.work_order_migrations') AS legacy")).rows[0]
  const migrations = []
  if (histories.sqlx) {
    const rows = (await client.query("SELECT version::text,success,pg_catalog.encode(checksum,'hex') AS checksum FROM public._sqlx_migrations ORDER BY version")).rows
    for (const row of rows) { assert(row.success && /^[a-f0-9]{64,128}$/.test(row.checksum)); migrations.push({ table: 'public._sqlx_migrations', ...row }) }
  }
  if (histories.legacy) {
    const rows = (await client.query('SELECT version,trim(checksum)::text AS checksum FROM public.work_order_migrations ORDER BY version')).rows
    for (const row of rows) { assert.match(row.checksum, /^[a-f0-9]{64}$/); migrations.push({ table: 'public.work_order_migrations', success: true, ...row }) }
  }
  return { serverVersion: version.server_version, postgresVersion: version.full_version,
    schemaSha256: sha(JSON.stringify({ relations, columns, constraints, indexes, enums, routines })),
    schemaFingerprintKind: 'pg_catalog-public-v1', migrationTables: [...new Set(migrations.map(row => row.table))], migrations }
}

/** Read the private, launcher-owned runtime contract; never print this object. */
export function readPhase28Runtime(file = process.env.PHASE28_RUNTIME_CONFIG) {
  const runtime = JSON.parse(readFileSync(regularFile(file, 'Phase28 runtime contract'), 'utf8'))
  assert.equal(runtime.schemaVersion, 1)
  assert.match(runtime.owner, /^(?:ci-)?phase28-[a-z0-9-]{8,100}$/)
  runtime.adminUsername = runtime.admin.username; runtime.adminPassword = runtime.admin.password
  runtime.databases = Object.fromEntries(Object.entries(runtime.databaseUrls).map(([code, url]) => [code, { url }]))
  runtime.oauth = runtime.clients; runtime.workDir = path.join(path.dirname(runtime.stateFile), 'fixture-inputs')
  runtime.source = { sourceRevision: runtime.sourceRevision, inputsSha256: runtime.inputsSha256, deliveryProof: runtime.deliveryProof }
  exactHttps(runtime.origin); exactHttps(runtime.externalOrigin)
  assert.notEqual(runtime.origin, runtime.externalOrigin)
  for (const key of ['caFile', 'adminBinary']) regularFile(runtime[key], key)
  for (const key of ['stateFile', 'policyFile', 'inventoryDir', 'reportDir', 'workDir']) assert(path.isAbsolute(runtime[key] || ''), `${key} requires an absolute path`)
  assert(runtime.adminUsername && runtime.adminPassword)
  for (const code of ['platform', ...applicationCodes]) {
    const database = new URL(runtime.databases[code].url)
    assert(['postgres:', 'postgresql:'].includes(database.protocol) && database.username && database.password)
    assert(/^[a-z][a-z0-9_]{0,62}$/.test(database.pathname.slice(1)))
  }
  for (const code of applicationCodes) {
    assert(runtime.oauth[code].clientId && runtime.oauth[code].clientSecret)
    assert(runtime.releases[code].v1 && runtime.releases[code].v2 && runtime.releases[code].external)
    for (const [kind, candidate] of Object.entries(runtime.releases[code])) {
      candidate.artifact ||= runtime.releases[code].v1.artifact
      candidate.buildInfo ||= runtime.releases[code].v1.buildInfo
      mkdirSync(runtime.workDir, { recursive: true, mode: 0o700 })
      for (const [field, suffix] of [['release', 'releaseFile'], ['artifact', 'artifactFile'], ['buildInfo', 'buildInfoFile']]) {
        const destination = path.join(runtime.workDir, code + '-' + kind + '-' + field + '.json')
        const bytes = JSON.stringify(candidate[field]) + '\n'
        if (existsSync(destination)) assert.equal(readFileSync(destination, 'utf8'), bytes, 'Private immutable fixture inputs changed')
        else writeFileSync(destination, bytes, { mode: 0o600, flag: 'wx' })
        candidate[suffix] = destination
      }
      regularFile(candidate.releaseFile, 'Release definition'); regularFile(candidate.artifactFile, 'Artifact definition')
    }
  }
  return runtime
}

/** Verify selected runtime DB identities using their own non-superuser accounts. */
export async function startPhase28Services({ prepare = false, runtime = readPhase28Runtime() } = {}) {
  assert.equal(process.platform, 'linux', 'Phase28 acceptance requires the isolated Linux runtime')
  const pg = createRequire(path.join(repository, 'mini-services/notification-ws/package.json'))('pg')
  const secrets = new Set([runtime.adminPassword, ...Object.values(runtime.databases).map(value => value.url), ...Object.values(runtime.oauth).map(value => value.clientSecret)])
  const redact = value => {
    let result = String(value).replace(/Bearer\s+[^\s"']+/gi, 'Bearer [redacted]').replace(/postgres(?:ql)?:\/\/[^\s"']+/gi, '[database-url-redacted]')
    for (const secret of secrets) if (secret) result = result.replaceAll(secret, '[redacted]')
    return result
  }
  const ca = readFileSync(runtime.caFile), clients = [], cli = [], databaseIdentities = []
  let closed = false
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => /^(PATH|LANG|LC_ALL|LD_LIBRARY_PATH|HOME|SSL_CERT_FILE|NODE_EXTRA_CA_CERTS|PLAYWRIGHT_BROWSERS_PATH|TMPDIR|XDG_CONFIG_HOME|XDG_DATA_HOME|XDG_CACHE_HOME)$/.test(key)))
  async function request(origin, endpoint, { token, method = 'GET', body, headers = {} } = {}) {
    const target = new URL(endpoint, origin)
    assert.equal(target.protocol, 'https:', 'Product requests must keep normal verified TLS')
    assert([runtime.origin, runtime.externalOrigin].includes(target.origin), 'Only launcher-approved public origins may be requested')
    if (token) headers = { ...headers, Authorization: 'Bearer ' + token }
    headers = { ...headers, 'X-Request-Id': randomUUID(), ...(body === undefined ? {} : { 'Content-Type': 'application/json' }) }
    return new Promise((resolve, reject) => {
      const req = https.request(target, { method, headers, ca, rejectUnauthorized: true }, response => {
        let size = 0; const chunks = []
        response.on('data', chunk => { size += chunk.length; if (size > 4 * 1024 * 1024) req.destroy(new Error('Owned response too large')); else chunks.push(chunk) })
        response.on('end', () => { const text = Buffer.concat(chunks).toString('utf8'); let data = null; try { data = JSON.parse(text) } catch {}
          resolve({ status: response.statusCode, headers: response.headers, text, data }) })
        response.on('error', () => reject(new Error('Owned HTTPS response failed')))
      })
      req.on('error', () => reject(new Error('Owned verified HTTPS request failed')))
      req.setTimeout(30_000, () => req.destroy(new Error('Owned HTTPS request deadline')))
      req.end(body === undefined ? undefined : JSON.stringify(body))
    })
  }
  const platformApi = async (endpoint, options = {}, expected = 200) => {
    const response = await request(runtime.origin, endpoint, options)
    assert.equal(response.status, expected, `Actual platform ${options.method || 'GET'} ${endpoint}: ${response.data?.code || response.status}`)
    return response.data
  }
  async function runCli(code, args, { expected = 0, offline = false, environment = 'production', bearer } = {}) {
    assert(applicationCodes.includes(code))
    const childEnv = { ...env, ...(!offline ? { PLATFORM_API_ORIGIN: runtime.origin, PLATFORM_ADMIN_BEARER: bearer || admin.accessToken,
      PLATFORM_CLIENT_ID: runtime.oauth[code].clientId, PLATFORM_CA_FILE: runtime.caFile } : {}) }
    const argv = [...args, '--app-code', code, '--environment', environment]
    const output = await new Promise((resolve, reject) => {
      const child = spawn(runtime.adminBinary, argv, { cwd: repository, env: childEnv, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
      let raw = '', errored = false
      child.on('error', () => { errored = true })
      for (const stream of [child.stdout, child.stderr]) stream.on('data', bytes => { raw += bytes })
      const timer = setTimeout(() => child.kill('SIGKILL'), 180_000)
      child.once('close', exit => { clearTimeout(timer); if (errored || exit !== expected) reject(new Error('Actual Rust CLI ' + args[0] + ' failed: ' + redact(raw).slice(-2000))); else resolve(raw) })
    })
    for (const secret of secrets) assert(!output.includes(secret), 'CLI output exposed a fixture secret')
    let result; try { result = JSON.parse(output) } catch { throw new Error('Actual Rust CLI did not produce machine JSON') }
    cli.push({ applicationCode: code, command: args[0], applied: args.includes('--apply'), exitCode: expected, category: result.category || result.error?.category || null,
      remoteDigestStored: result.remoteDigestStored ?? false })
    return result
  }
  async function login(username, organizationId) {
    const ticket = await platformApi('/api/auth/login', { method: 'POST', body: { username, password: runtime.adminPassword } })
    secrets.add(ticket.ticket)
    const response = await platformApi('/api/auth/context', { token: ticket.ticket, method: 'POST', body: { organizationId } })
    secrets.add(response.accessToken); if (response.refreshToken) secrets.add(response.refreshToken)
    return response
  }
  const databases = {}
  async function close({ browserClosed = true } = {}) {
    if (closed) return
    closed = true
    const errors = []
    for (const client of clients) try { await client.end() } catch { errors.push('database-client') }
    const result = { databaseClientsClosed: errors.length === 0, databaseConnectionsClosed: errors.length === 0,
      browserClosed, productResourcesOwnedByOuterDriver: true }
    console.log('PHASE28_BROWSER_CLEANUP ' + JSON.stringify(result))
    if (errors.length) throw new Error('Phase28 private client cleanup failed')
    return result
  }
  let admin, state
  try {
    for (const code of ['platform', ...applicationCodes]) {
      const client = new pg.Client({ connectionString: runtime.databases[code].url, connectionTimeoutMillis: 10_000, query_timeout: 15_000 })
      client.on('error', () => {}); clients.push(client); await client.connect(); databases[code] = client
      const actual = (await client.query('SELECT current_database() AS name,current_user AS role,rolsuper,rolcreatedb,rolcreaterole FROM pg_roles WHERE rolname=current_user')).rows[0]
      assert.equal(actual.rolsuper, false); assert.equal(actual.rolcreatedb, false); assert.equal(actual.rolcreaterole, false)
      const versions = await databaseVersionEvidence(client)
      if (!prepare) assert(versions.migrations.some(row => row.table === 'public._sqlx_migrations'), 'A running Rust component must have its actual SQLx migration history')
      databaseIdentities.push({ component: code, ...actual, ...versions })
    }
    assert.equal(new Set(databaseIdentities.map(value => value.name)).size, 3)
    assert.equal(new Set(databaseIdentities.map(value => value.role)).size, 3)
    const adminOrg = (await databases.platform.query('SELECT o.id FROM organizations o JOIN tenants t ON t.id=o.tenant_id WHERE t.is_platform AND o.parent_id IS NULL')).rows[0]?.id
    assert(adminOrg, 'The real platform bootstrap must create its root context')
    admin = await login(runtime.adminUsername, adminOrg)
    const adminApi = (endpoint, method = 'GET', body) => platformApi(endpoint, { token: admin.accessToken, method, body })
    if (prepare) {
      assert(!existsSync(runtime.stateFile) && !existsSync(runtime.policyFile), 'Private prepare outputs must be fresh')
      const apps = {}
      for (const code of applicationCodes) {
        const oauth = runtime.oauth[code]
        // Synthetic service configuration is the only identity prepared with SQL.
        // Apps, menus, roles and business grants go through their formal APIs/CLI.
        await databases.platform.query("INSERT INTO oauth_clients(client_id,client_name,client_secret_hash,client_type,redirect_uris,grant_types,scopes,access_token_validity,refresh_token_validity) VALUES($1,$2,$3,'CONFIDENTIAL','','client_credentials','openid',300,300)",
          [oauth.clientId, code + '-client', sha(oauth.clientSecret)])
        await runCli(code, ['bootstrap', '--apply'])
        const rows = await adminApi('/api/applications'), application = (Array.isArray(rows) ? rows : rows.records).find(value => value.code === code)
        assert(application)
        const definitions = JSON.parse(readFileSync(path.join(repository, 'business-apps', code, 'deploy/bootstrap.manifest.json'), 'utf8')).menus
        const tree = await adminApi('/api/menus/tree?applicationId=' + application.id)
        const flatten = rows => rows.flatMap(row => [row, ...flatten(row.children || [])])
        const actual = flatten(tree.menus)
        assert.equal(actual.length, definitions.length)
        const menus = {}
        for (const definition of definitions) {
          const row = actual.find(value => value.path === definition.path && value.type === definition.type && value.permission === definition.permission)
          assert(row, 'Formal bootstrap did not create the declared app menu'); menus[definition.key] = row.id
        }
        apps[code] = { application, menus }
      }
      const tenant = await adminApi('/api/tenants', 'POST', { name: 'Phase28 delivery tenant', code: 'phase28-delivery' })
      const tenantRoot = (await databases.platform.query('SELECT id FROM organizations WHERE tenant_id=$1 AND parent_id IS NULL', [tenant.id])).rows[0].id
      const organization = await adminApi('/api/organizations', 'POST', { tenantId: tenant.id, parentId: tenantRoot, name: 'Phase28 delivery organization', code: 'phase28-delivery-org' })
      const actors = {}
      for (const key of ['ordinary', 'peer', 'unauthorized']) {
        const user = await adminApi('/api/users', 'POST', { username: 'phase28-' + key, password: runtime.adminPassword, realName: 'Phase28 ' + key })
        await adminApi('/api/users/' + user.id + '/memberships', 'PUT', { items: [{ organizationId: organization.id }] })
        const memberships = await adminApi('/api/users/' + user.id + '/memberships')
        const membership = memberships.find(value => value.organizationId === organization.id)
        const role = await adminApi('/api/roles', 'POST', { organizationId: organization.id, name: 'Phase28 ' + key })
        assert.equal((await databases.platform.query('SELECT is_super_admin FROM users WHERE id=$1', [user.id])).rows[0].is_super_admin, false)
        actors[key] = { userId: user.id, username: 'phase28-' + key, organizationId: organization.id, tenantId: tenant.id, membershipId: membership.membershipId, roleId: role.id }
      }
      const capabilities = {
        stores: [['store','read'],['store','create'],['store','update'],['store-staff','read'],['store-staff','assign'],['store-staff','unassign'],['store-daily-report','read'],['store-daily-report','create'],['store-daily-report','update']],
        'work-orders': [['ticket','read'],['ticket','create'],['ticket','update'],['ticket','delete'],['ticket','export'],['notification','publish']],
      }
      const memberLookups = { stores: [['store','create'],['store','update'],['store-staff','assign']], 'work-orders': [['ticket','create'],['ticket','update']] }
      const dataGrantPolicy = { applications: applicationCodes.map(code => ({ applicationId: apps[code].application.id,
        capabilities: capabilities[code].map(([resource, action]) => ({ resource, action, scopes: ['SELF','ORG','ORG_SUBTREE','TENANT'] })),
        memberLookups: memberLookups[code].map(([resource, action]) => ({ resource, action })) })),
        managementScopes: [{ tenantId: tenant.id, organizationId: tenantRoot, includeDescendants: true, read: true, write: true }] }
      state = { schemaVersion: 1, owner: runtime.owner, applications: apps, actors, capabilities, tenantRoot }
      mkdirSync(path.dirname(runtime.stateFile), { recursive: true, mode: 0o700 })
      writeFileSync(runtime.stateFile, JSON.stringify(state) + '\n', { mode: 0o600, flag: 'wx' })
      writeFileSync(runtime.policyFile, JSON.stringify(dataGrantPolicy) + '\n', { mode: 0o600, flag: 'wx' })
      return { runtime, state, close, redact, summary: { schemaVersion: 1, owner: runtime.owner, stage: 'PREPARED', applicationCodes,
        sourceRevision: runtime.sourceRevision, inputsSha256: runtime.inputsSha256,
        applicationMenuCounts: Object.fromEntries(applicationCodes.map(code => [code, Object.keys(apps[code].menus).length])),
        databaseIdentities, syntheticClientConfigurationOnly: true, formalBootstrap: true, grantsApplied: false, policyReadyForOuterRestart: true } }
    }
    state = JSON.parse(readFileSync(regularFile(runtime.stateFile, 'Private prepared state'), 'utf8'))
    assert.equal(state.schemaVersion, 1); assert.equal(state.owner, runtime.owner)
    const apps = state.applications
    for (const key of ['ordinary', 'peer']) for (const code of applicationCodes) {
      const actor = state.actors[key], capabilities = state.capabilities[code]
      const manifest = { schemaVersion: '1.0', applicationCode: code, applicationId: apps[code].application.id, menuIds: apps[code].menus,
        targets: [{ tenantId: actor.tenantId, organizationId: actor.organizationId, roleId: actor.roleId, userId: actor.userId, membershipId: actor.membershipId,
          permissions: capabilities.map(([resource, action]) => resource + ':' + action), dataGrants: capabilities.map(([resource, action]) => ({ resource, action, scope: 'ORG' })) }] }
      const manifestFile = path.join(runtime.workDir, key + '-' + code + '-grants.json')
      writeFileSync(manifestFile, JSON.stringify(manifest) + '\n', { mode: 0o600, flag: 'wx' })
      await runCli(code, ['grants-plan', '--manifest', manifestFile])
      await runCli(code, ['grants-apply', '--manifest', manifestFile, '--apply'])
    }
    async function authorize(actor, appCode = 'stores', { applicationToken = true } = {}) {
      const sessionResponse = await login(actor.username, actor.organizationId)
      const selected = { ...actor, session: sessionResponse.accessToken, sessionResponse, appCode }
      if (applicationToken) {
        const token = await platformApi('/api/oauth/session-token', { token: selected.session, method: 'POST', body: { appCode } }, 201)
        secrets.add(token.accessToken); selected.token = token.accessToken
      }
      return selected
    }
    const actors = {}
    for (const key of ['ordinary', 'peer']) actors[key] = await authorize(state.actors[key])
    actors.unauthorized = await authorize(state.actors.unauthorized, 'stores', { applicationToken: false })
    const adminActor = { session: admin.accessToken, sessionResponse: admin, appCode: 'stores', userId: admin.user?.id, organizationId: adminOrg }
    const readCandidate = (code, kind) => {
      const candidate = runtime.releases[code][kind]
      return { ...candidate, release: JSON.parse(readFileSync(candidate.releaseFile, 'utf8')), artifact: JSON.parse(readFileSync(candidate.artifactFile, 'utf8')),
        ...(candidate.buildInfoFile ? { buildInfo: JSON.parse(readFileSync(candidate.buildInfoFile, 'utf8')) } : {}) }
    }
    return { runtime, state, origin: runtime.origin, externalOrigin: runtime.externalOrigin, apps, actors, admin: adminActor,
      platformDb: databases.platform, businessDb: databases.stores, workOrdersDb: databases['work-orders'], databaseIdentities,
      request, platformApi, adminApi, authorize, runCli, readCandidate, close, redact, cli, source: { ...runtime.source, databaseIdentities },
      businessCall: (actor, endpoint, options = {}) => request(runtime.origin, endpoint, { ...options, token: actor.token }) }
  } catch (error) {
    try { error.phase28StartupCleanup = await close() } catch {}
    error.message = redact(error.message); throw error
  }
}
