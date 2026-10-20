#!/usr/bin/env node
// Local candidate builds and owned, isolated HTTPS verification only.
import assert from 'node:assert/strict'
import { createHash, randomBytes } from 'node:crypto'
import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createDocker, createSnapshot, validateOutputLocation } from './linux-runner.mjs'
import { createSourceManifest } from './provenance.mjs'
import { startWslLease } from '../../tests/zip-release-e2e.mjs'
import { renderEdge } from '../../mini-services/oauth-playground/deploy/render-edge.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const label = 'saas.phase24.owner'
const linuxPath = value => process.platform === 'win32' ? `/mnt/${path.resolve(value)[0].toLowerCase()}/${path.resolve(value).slice(3).replaceAll('\\', '/')}` : path.resolve(value)

export function parseDeliveryArgs(args) {
  const options = { toolsImage: 'saas-ci:phase10-browser-155-r1' }
  const keys = { '--out-dir': 'outDir', '--source-revision': 'sourceRevision', '--tools-image': 'toolsImage', '--build-proxy': 'buildProxy' }
  const seen = new Set()
  for (let index = 0; index < args.length; index += 2) {
    const key = keys[args[index]]
    if (!key || seen.has(key) || !args[index + 1]) throw new Error('Unknown, repeated or missing OAuth delivery argument')
    seen.add(key); options[key] = args[index + 1]
  }
  if (!path.isAbsolute(options.outDir || '') || !/^[a-f0-9]{40}$/.test(options.sourceRevision || '')) throw new Error('An absolute fresh --out-dir and full --source-revision are required')
  if (options.buildProxy && !/^http:\/\/(?:127\.0\.0\.1|localhost):[1-9][0-9]{0,4}$/.test(options.buildProxy)) throw new Error('Build proxy must be a credential-free loopback HTTP URL')
  return options
}

// Runs inside the isolated Linux tools container. Only the public test CA is
// mounted here; TLS private keys stay in the operator-owned private directory.
const clientProgram = String.raw`import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import https from 'node:https'
const ca = readFileSync('/smoke/ca.pem')
const checks = []
let cookie = ''
function request(path, { method = 'GET', headers = {}, body } = {}) {
  return new Promise((resolve, reject) => {
    const attempt = https.request({ hostname: 'oauth.test', port: 8443, path, method,
      ca, rejectUnauthorized: true, servername: 'oauth.test', headers }, response => {
      const bytes = []; let size = 0
      response.on('data', chunk => { size += chunk.length; if (size > 1024 * 1024) attempt.destroy(new Error('response too large')); else bytes.push(chunk) })
      response.on('end', () => resolve({ status: response.statusCode, headers: response.headers, body: Buffer.concat(bytes).toString('utf8'), authorized: response.socket?.authorized }))
    })
    attempt.on('error', reject)
    attempt.setTimeout(5000, () => attempt.destroy(new Error('HTTPS timeout')))
    if (body !== undefined) attempt.write(body)
    attempt.end()
  })
}
function safe(text) {
  assert.ok(!text.includes(process.env.OAUTH_PRIVATE_SENTINEL), 'private runtime secret leaked')
}
const health = await request('/api/health')
assert.equal(health.status, 200)
assert.equal(JSON.parse(health.body).status, 'ok')
assert.equal(health.headers['cache-control'], 'no-store')
assert.equal(health.headers['referrer-policy'], 'no-referrer')
checks.push('actual-HTTPS-private-API-health')
const homepage = await request('/')
assert.equal(homepage.status, 200)
assert.match(homepage.headers['content-type'], /text\/html/)
assert.match(homepage.headers['content-security-policy'], /script-src 'self'/)
assert.equal(homepage.headers['referrer-policy'], 'no-referrer')
assert.match(homepage.headers['strict-transport-security'], /max-age=/)
safe(homepage.body)
const callback = await request('/callback?code=public-test-code&state=public-test-state')
assert.equal(callback.status, 200)
assert.match(callback.headers['content-type'], /text\/html/)
assert.equal(callback.headers['cache-control'], 'no-store')
safe(callback.body)
const assets = [...new Set([...homepage.body.matchAll(/(?:src|href)="(\/assets\/[^"?#]+)"/g), ...callback.body.matchAll(/(?:src|href)="(\/assets\/[^"?#]+)"/g)].map(match => match[1]))]
assert.ok(assets.length >= 2, 'real hashed assets must be referenced')
for (const asset of assets) {
  assert.match(asset, /-[A-Za-z0-9_-]+\.[a-z]+$/)
  const response = await request(asset)
  assert.equal(response.status, 200)
  assert.match(response.headers['cache-control'], /immutable/)
  safe(response.body)
}
checks.push('real-index-callback-and-hashed-assets')
for (const missing of ['/assets/missing-fixture.js', '/unknown-page', '/api/missing/api']) {
  const response = await request(missing)
  assert.equal(response.status, 404)
  assert.equal(response.headers['cache-control'], 'no-store')
}
checks.push('strict-404-no-store')
const configuration = await request('/api/config')
assert.equal(configuration.status, 200)
safe(configuration.body)
const cookies = configuration.headers['set-cookie']
assert.equal(cookies.length, 1)
assert.match(cookies[0], /^__Host-oauth_playground=[A-Za-z0-9_-]+;/)
for (const flag of ['HttpOnly', 'Secure', 'SameSite=Lax', 'Path=/']) assert.ok(cookies[0].includes(flag))
cookie = cookies[0].split(';')[0]
assert.equal(JSON.parse(configuration.body).flows.password.enabled, false)
checks.push('secure-HttpOnly-host-cookie-password-disabled')
const badOrigin = await request('/api/auth-start', { method: 'POST', headers: { Cookie: cookie, Origin: 'https://attacker.test', 'Content-Type': 'application/json' }, body: '{}' })
assert.equal(badOrigin.status, 403)
assert.equal(JSON.parse(badOrigin.body).error, 'origin_rejected')
checks.push('wrong-Origin-rejected')
const start = await request('/api/auth-start', { method: 'POST', headers: { Cookie: cookie, Origin: 'https://oauth.test', 'Content-Type': 'application/json' }, body: '{}' })
assert.equal(start.status, 200)
safe(start.body)
const result = JSON.parse(start.body)
assert.deepEqual(Object.keys(result).sort(), ['authorizeUrl', 'state'])
const authorize = new URL(result.authorizeUrl)
assert.equal(authorize.origin, 'https://platform.test')
assert.equal(authorize.pathname, '/oauth/authorize')
assert.equal(authorize.searchParams.get('redirect_uri'), 'https://oauth.test/callback')
assert.equal(authorize.searchParams.get('state'), result.state)
assert.equal(authorize.searchParams.get('code_challenge_method'), 'S256')
assert.match(authorize.searchParams.get('code_challenge'), /^[A-Za-z0-9_-]{43}$/)
assert.equal(authorize.searchParams.has('code_verifier'), false)
assert.equal(authorize.searchParams.has('client_secret'), false)
assert.equal(authorize.searchParams.has('access_token'), false)
assert.equal(authorize.searchParams.has('refresh_token'), false)
const verifier = await request('/api/verifier?state=' + encodeURIComponent(result.state))
assert.equal(verifier.status, 404)
safe(verifier.body)
checks.push('auth-start-fixed-callback-S256-no-secret-or-verifier')
console.log(JSON.stringify({ status: 'PASS', trustedTLS: 'custom CA verified; rejectUnauthorized true', checks, assetCount: assets.length }))
`

export async function runOauthDelivery(options) {
  const outDir = path.resolve(options.outDir)
  validateOutputLocation(outDir, root)
  if (existsSync(outDir)) throw new Error('OAuth delivery output already exists; use a fresh directory')
  for (const folder of ['logs', 'private', 'public']) mkdirSync(path.join(outDir, folder), { recursive: true, mode: folder === 'public' ? 0o755 : 0o700 })
  const owner = `phase24-${Date.now()}-${randomBytes(5).toString('hex')}`
  const network = `${owner}-network`, names = [], secret = `phase24-runtime-${randomBytes(24).toString('hex')}`
  const report = { schemaVersion: 1, status: 'RUNNING', owner, startedAt: new Date().toISOString(), sourceRevision: options.sourceRevision,
    publication: 'NOT_REQUESTED', productionDeployment: 'NOT_REQUESTED', commands: [], images: [], checks: [], cleanupErrors: [] }
  const save = () => writeFileSync(path.join(outDir, 'result.json'), JSON.stringify(report, null, 2) + '\n', { mode: 0o600 })
  const redact = value => String(value).replaceAll(secret, '[redacted]').replace(/(__Host-oauth_playground=)[^;\s]+/g, '$1[redacted]')
  const adapter = createDocker()
  let lease, networkCreated = false, cancelled = false
  const cancel = () => { cancelled = true; report.cancelled = true; save() }
  process.once('SIGINT', cancel)
  process.once('SIGTERM', cancel)
  async function docker(name, args, settings) {
    if (cancelled) throw new Error('Owned WSL lease exited unexpectedly')
    const log = `logs/${String(report.commands.length + 1).padStart(3, '0')}-${name}.log`
    const record = { name, executable: process.platform === 'win32' ? 'wsl.exe' : 'docker',
      prefix: process.platform === 'win32' ? ['-d', process.env.CI_WSL_DISTRO || 'Ubuntu', '--exec', 'docker', '--host', 'unix:///var/run/docker.sock'] : ['--host', 'unix:///var/run/docker.sock'],
      args: args.map(redact), exitCode: null, log }
    report.commands.push(record); save()
    try {
      const output = await adapter(args, settings)
      record.exitCode = 0; writeFileSync(path.join(outDir, log), redact(output) + '\n', { mode: 0o600 }); save()
      return output
    } catch (error) {
      record.exitCode = Number(/exit=(\d+)/.exec(error.message)?.[1] || 1)
      writeFileSync(path.join(outDir, log), redact(error.message) + '\n', { mode: 0o600 }); save()
      throw new Error(`${name} failed (exit ${record.exitCode}); see ${log}`, { cause: error })
    }
  }
  async function inspectOwner(kind, name) {
    let actual
    try { actual = await docker(`owner-${name}`, [kind, 'inspect', '--format', kind === 'container' ? `{{index .Config.Labels "${label}"}}` : `{{index .Labels "${label}"}}`, name]) }
    catch (error) {
      if (/No such (?:container|network|object)/i.test(error.cause?.message || '')) {
        report.commands.at(-1).expectedMissingAfterAutoRemove = true; save(); return false
      }
      throw error
    }
    if (actual !== owner) throw new Error(`Ownership mismatch for ${kind}; refusing cleanup`)
    return true
  }
  async function base(name, reference) {
    const image = JSON.parse(await docker(`base-${name}`, ['image', 'inspect', reference]))[0]
    report.images.push({ name, role: 'build-base-or-tools', requestedReference: reference, imageId: image.Id, repoDigests: image.RepoDigests || [] })
    save()
    return image.RepoDigests?.[0] || reference
  }
  async function run(name, args) {
    names.push(name)
    return docker(`run-${name}`, ['run', '--name', name, '--label', `${label}=${owner}`, ...args])
  }
  try {
    if (process.platform === 'win32') {
      lease = await startWslLease({ owner, env: process.env, distro: process.env.CI_WSL_DISTRO || 'Ubuntu', onUnexpectedExit() { cancelled = true } })
      report.wslLease = lease.evidence
    }
    report.dockerVersion = JSON.parse(await docker('docker-version', ['version', '--format', '{{json .}}']))
    const tools = await base('tools', options.toolsImage)
    const node = await base('node', 'node:24.18.0-alpine')
    const nginx = await base('nginx', 'nginxinc/nginx-unprivileged:1.28.0-alpine')
    const rust = await base('rust', 'rust:1.97.0-slim-bookworm')
    const runtime = await base('runtime', 'debian:bookworm-slim')
    const sourceDir = path.join(outDir, 'source')
    report.snapshot = createSnapshot(sourceDir, root)
    const source = createSourceManifest(sourceDir, { sourceRevision: options.sourceRevision })
    report.fullSnapshotInputsSha256 = source.inputsSha256
    const componentFiles = source.files.filter(file => /^mini-services\/oauth-playground\/(?:frontend\/|backend\/|deploy\/|\.dockerignore$)/.test(file.path))
    const componentInputsSha256 = createHash('sha256').update(JSON.stringify(componentFiles)).digest('hex')
    report.inputsSha256 = componentInputsSha256
    writeFileSync(path.join(outDir, 'component-inputs.json'), JSON.stringify({ sourceRevision: options.sourceRevision, inputsSha256: componentInputsSha256, files: componentFiles }, null, 2) + '\n')
    writeFileSync(path.join(outDir, 'source-manifest.json'), JSON.stringify(source, null, 2) + '\n')
    const component = path.join(sourceDir, 'mini-services/oauth-playground')
    const proxy = options.buildProxy ? ['--network', 'host', ...['HTTP_PROXY', 'HTTPS_PROXY', 'http_proxy', 'https_proxy'].flatMap(name => ['--build-arg', `${name}=${options.buildProxy}`])] : []
    const candidate = {}
    for (const [unit, file, args] of [
      ['frontend', 'Dockerfile.frontend', ['--build-arg', `NODE_IMAGE=${node}`, '--build-arg', `NGINX_IMAGE=${nginx}`]],
      ['api', 'Dockerfile.api', ['--build-arg', `RUST_IMAGE=${rust}`, '--build-arg', `RUNTIME_IMAGE=${runtime}`]],
    ]) {
      const tag = `${owner}-${unit}:candidate`
      await docker(`build-${unit}`, ['build', '--force-rm', '--platform', 'linux/amd64', ...proxy,
        '--label', `${label}=${owner}`, '--label', `saas.ci.inputs-sha256=${componentInputsSha256}`,
        '--build-arg', `SOURCE_REVISION=${options.sourceRevision}`, ...args,
        '--file', linuxPath(path.join(component, 'deploy', file)), '--tag', tag, linuxPath(component)], { timeout: 45 * 60_000 })
      const image = JSON.parse(await docker(`image-${unit}`, ['image', 'inspect', tag]))[0]
      assert.equal(image.Config.Labels[label], owner)
      assert.equal(image.Config.Labels['org.opencontainers.image.revision'], options.sourceRevision)
      candidate[unit] = image.Id
      report.images.push({ name: unit, role: 'candidate', tag, imageId: image.Id, os: image.Os, architecture: image.Architecture,
        labels: image.Config.Labels, identityMeaning: 'local image ID; no registry publication' })
    }
    report.checks.push('clean-source-frontend-and-Rust-image-builds')
    const privateDir = path.join(outDir, 'private'), publicDir = path.join(outDir, 'public'), tlsDir = path.join(privateDir, 'tls')
    mkdirSync(tlsDir)
    // The host-only parent stays 0700. This child is bind-mounted directly for
    // the unprivileged generator; normal Linux permits its UID to create keys.
    // DrvFS controls owner/mode itself and rejects chmod from container UIDs.
    if (process.platform !== 'win32') chmodSync(tlsDir, 0o777)
    writeFileSync(path.join(privateDir, 'api.env'), `OAUTH_BIND_ADDR=0.0.0.0:3011\nOAUTH_PUBLIC_ORIGIN=https://oauth.test\nOAUTH_PLATFORM_ORIGIN=https://platform.test\nOAUTH_ALLOW_INSECURE_LOOPBACK=false\nOAUTH_AUTH_CLIENT_ID=delivery-confidential-client\nOAUTH_AUTH_CLIENT_SECRET=${secret}\nOAUTH_PASSWORD_ENABLED=false\n`, { mode: 0o600 })
    writeFileSync(path.join(privateDir, 'client.env'), `OAUTH_PRIVATE_SENTINEL=${secret}\n`, { mode: 0o600 })
    writeFileSync(path.join(tlsDir, 'server.ext'), 'subjectAltName=DNS:oauth.test\nextendedKeyUsage=serverAuth\n', { mode: 0o644 })
    const locked = ['--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges:true', '--tmpfs', '/tmp:rw,noexec,nosuid,size=32m']
    await run(`${owner}-certificates`, ['--rm', '--network', 'none', '--user', '10001:10001', ...locked,
      '--mount', `type=bind,src=${linuxPath(tlsDir)},dst=/tls`, '--entrypoint', 'sh', tools, '-ec',
      'openssl req -x509 -newkey rsa:2048 -nodes -keyout /tls/ca.key -out /tls/ca.pem -days 2 -subj /CN=Phase24-Test-CA && openssl req -newkey rsa:2048 -nodes -keyout /tls/server.key -out /tls/server.csr -subj /CN=oauth.test && openssl x509 -req -in /tls/server.csr -CA /tls/ca.pem -CAkey /tls/ca.key -CAcreateserial -out /tls/server.pem -days 2 -extfile /tls/server.ext' + (process.platform === 'win32' ? '' : ' && chmod 644 /tls/server.key /tls/server.pem /tls/ca.pem')])
    writeFileSync(path.join(publicDir, 'ca.pem'), readFileSync(path.join(tlsDir, 'ca.pem')), { mode: 0o644 })
    writeFileSync(path.join(publicDir, 'client.mjs'), clientProgram, { mode: 0o644 })
    writeFileSync(path.join(publicDir, 'nginx.conf'), renderEdge({ publicOrigin: 'https://oauth.test', frontend: 'oauth-web:8080', api: 'oauth-api:3011' }), { mode: 0o644 })
    await docker('network-create', ['network', 'create', '--internal', '--label', `${label}=${owner}`, network]); networkCreated = true
    for (const [unit, alias, image] of [['api', 'oauth-api', candidate.api], ['frontend', 'oauth-web', candidate.frontend]]) {
      await run(`${owner}-${unit}`, ['--detach', '--network', network, '--network-alias', alias, ...locked,
        ...(unit === 'api' ? ['--env-file', linuxPath(path.join(privateDir, 'api.env'))] : []), image])
    }
    const apiProbe = await docker('api-runtime-probe', ['exec', `${owner}-api`, 'sh', '-ec', 'test "$(id -u)" = 10001; for binary in node npm bun caddy; do if command -v "$binary"; then exit 1; fi; done; curl --fail --silent http://127.0.0.1:3011/health'])
    assert.equal(JSON.parse(apiProbe).status, 'ok')
    await docker('frontend-runtime-probe', ['exec', `${owner}-frontend`, 'sh', '-ec', 'test "$(id -u)" = 101; for binary in node npm bun caddy; do if command -v "$binary"; then exit 1; fi; done; wget -q -O /dev/null http://127.0.0.1:8080/healthz'])
    const mounts = ['--mount', `type=bind,src=${linuxPath(path.join(publicDir, 'nginx.conf'))},dst=/etc/nginx/nginx.conf,readonly`,
      '--mount', `type=bind,src=${linuxPath(path.join(tlsDir, 'server.pem'))},dst=/etc/nginx/tls/fullchain.pem,readonly`,
      '--mount', `type=bind,src=${linuxPath(path.join(tlsDir, 'server.key'))},dst=/etc/nginx/tls/privkey.pem,readonly`]
    await run(`${owner}-edge`, ['--detach', '--network', network, '--network-alias', 'oauth.test', '--user', '101:101', ...locked, ...mounts,
      '--entrypoint', 'nginx', nginx, '-g', 'daemon off;'])
    await docker('edge-nginx-syntax', ['exec', `${owner}-edge`, 'nginx', '-t'])
    const smoke = JSON.parse(await run(`${owner}-client`, ['--rm', '--network', network, '--user', '10001:10001', ...locked,
      '--env-file', linuxPath(path.join(privateDir, 'client.env')), '--mount', `type=bind,src=${linuxPath(publicDir)},dst=/smoke,readonly`, '--entrypoint', 'node', tools, '/smoke/client.mjs']))
    assert.equal(smoke.status, 'PASS')
    report.https = smoke; report.checks.push(...smoke.checks, 'UID10001-API-UID101-static-and-edge-no-Node-runtime')
    report.runtime = []
    for (const unit of ['api', 'frontend', 'edge']) {
      const state = JSON.parse(await docker(`runtime-${unit}`, ['container', 'inspect', '--format', '{{json .HostConfig}}', `${owner}-${unit}`]))
      assert.equal(state.ReadonlyRootfs, true)
      assert.ok(state.CapDrop.includes('ALL'))
      assert.ok(state.SecurityOpt.some(value => value.startsWith('no-new-privileges')))
      assert.ok(!state.PortBindings || !Object.keys(state.PortBindings).length)
      report.runtime.push({ unit, readOnlyRootfs: true, capabilityDropAll: true, noNewPrivileges: true, hostPorts: [] })
    }
    const composeEnv = `OAUTH_API_IMAGE=${candidate.api}\nOAUTH_FRONTEND_IMAGE=${candidate.frontend}\nOAUTH_EDGE_IMAGE=${nginx}\nOAUTH_API_ENV_FILE=${linuxPath(path.join(privateDir, 'api.env'))}\nOAUTH_EDGE_CONFIG_FILE=${linuxPath(path.join(publicDir, 'nginx.conf'))}\nOAUTH_TLS_CERTIFICATE_FILE=${linuxPath(path.join(tlsDir, 'server.pem'))}\nOAUTH_TLS_PRIVATE_KEY_FILE=${linuxPath(path.join(tlsDir, 'server.key'))}\n`
    writeFileSync(path.join(privateDir, 'compose.env'), composeEnv, { mode: 0o600 })
    const compose = JSON.parse(await docker('compose-config', ['compose', '-f', linuxPath(path.join(component, 'deploy/compose.yml')), '--env-file', linuxPath(path.join(privateDir, 'compose.env')), 'config', '--format', 'json']))
    assert.deepEqual(Object.entries(compose.services).filter(([, value]) => value.ports?.length).map(([key]) => key), ['oauth-edge'])
    report.compose = { status: 'PASS', publicServices: ['oauth-edge'], apiAndStaticHostPorts: [] }
    report.checks.push('Compose-only-edge-publishes-ports')
    for (const unit of ['api', 'frontend', 'edge']) {
      const logs = await docker(`logs-${unit}`, ['logs', `${owner}-${unit}`])
      assert.ok(!logs.includes(secret), 'runtime logs leaked secret')
      assert.doesNotMatch(logs, /code_verifier|client_secret=/)
    }
    report.checks.push('runtime-logs-no-private-secret-or-verifier')
    report.status = 'PASS'
  } catch (error) { report.status = 'FAIL'; report.error = redact(error.message); throw error }
  finally {
    process.removeListener('SIGINT', cancel)
    process.removeListener('SIGTERM', cancel)
    cancelled = false
    for (const name of names.toReversed()) try {
      if (await inspectOwner('container', name)) await docker(`cleanup-${name}`, ['rm', '--force', name])
    } catch (error) { report.cleanupErrors.push(redact(error.message)) }
    if (networkCreated) try { if (await inspectOwner('network', network)) await docker('cleanup-network', ['network', 'rm', network]) }
    catch (error) { report.cleanupErrors.push(redact(error.message)) }
    if (lease) try { await lease.stop() } catch (error) { report.cleanupErrors.push(redact(error.message)) }
    if (report.cleanupErrors.length) report.status = 'FAIL'
    report.finishedAt = new Date().toISOString(); save()
  }
  if (report.status !== 'PASS') throw new Error('OAuth delivery cleanup failed')
  return report
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const result = await runOauthDelivery(parseDeliveryArgs(process.argv.slice(2)))
    console.log(JSON.stringify({ status: result.status, owner: result.owner, checks: result.checks.length, output: path.resolve(process.argv[process.argv.indexOf('--out-dir') + 1]) }))
  } catch (error) { console.error(error.message); process.exitCode = 1 }
}
