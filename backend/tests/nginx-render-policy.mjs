import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { once } from 'node:events'
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import http from 'node:http'
import https from 'node:https'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { renderGateway, renderMicroStatic, validateTrustedProxyIps } from '../nginx/render.mjs'

const gateway = { publicOrigin: 'https://platform.example.test' }
const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')

function directory(t) {
  const root = path.resolve(mkdtempSync(path.join(os.tmpdir(), 'platform-gateway-render-')))
  t.after(() => {
    assert.equal(path.dirname(root), path.resolve(os.tmpdir()))
    assert.ok(path.basename(root).startsWith('platform-gateway-render-'))
    assert.ok(!lstatSync(root).isSymbolicLink())
    rmSync(root, { recursive: true, force: true })
  })
  return root
}

test('gateway accepts only exact HTTPS origins and explicit private service targets', () => {
  for (const publicOrigin of ['http://platform.example.test', 'https://platform.example.test/', 'https://user:pass@platform.example.test', 'https://platform.example.test/path', 'https://platform.example.test?debug=1', 'https://platform.example.test#fragment', 'https://platform.example.test;include']) {
    assert.throws(() => renderGateway({ ...gateway, publicOrigin }), /origin/i)
  }
  for (const backend of ['https://backend:3001', 'evil.example.test:3001', '169.254.169.254:80', '8.8.8.8:443', '0.0.0.0:3001', '10.20.0.9:3001', 'backend:3001/path', 'backend:3001;return 200', 'backend:70000']) {
    assert.throws(() => renderGateway({ ...gateway, backend }), /upstream|port/i)
  }
  assert.doesNotThrow(() => renderGateway({ ...gateway, backend: '10.20.0.9:3001', allowedUpstreamIps: ['10.20.0.9'] }))
  assert.throws(() => renderGateway({ ...gateway, backend: '8.8.8.8:443', allowedUpstreamIps: ['8.8.8.8'] }))
  assert.doesNotThrow(() => renderGateway({ ...gateway, backend: 'platform-api:3001', allowedServices: ['platform-api', 'frontend-web'] }))
})

test('gateway treats forwarded headers as untrusted unless an exact private proxy is declared', () => {
  const direct = renderGateway(gateway)
  assert.match(direct, /proxy_set_header X-Forwarded-For \$remote_addr;/)
  assert.ok(!direct.includes('$proxy_add_x_forwarded_for'))
  assert.ok(!direct.includes('set_real_ip_from'))
  assert.match(renderGateway({ ...gateway, trustedProxyIps: ['10.20.0.4'] }), /set_real_ip_from 10\.20\.0\.4;/)
  for (const address of ['0.0.0.0/0', '10.0.0.0/8', '*', '8.8.8.8', '10.20.0.4;allow all', 'metadata.internal']) {
    assert.throws(() => validateTrustedProxyIps([address]))
  }
})

test('gateway owns TLS and dynamic service DNS while Rust owns business CORS', () => {
  const config = renderGateway({ ...gateway, notification: 'notification-ws:3003', workOrders: 'work-orders-api:3004' })
  assert.match(config, /ssl_protocols TLSv1\.2 TLSv1\.3;/)
  assert.match(config, /resolver 127\.0\.0\.11 valid=5s ipv6=off;/)
  assert.match(config, /proxy_pass http:\/\/\$platform\$request_uri;/)
  assert.match(config, /proxy_pass http:\/\/\$notification\$request_uri;/)
  assert.match(config, /proxy_http_version 1\.1;/)
  assert.match(config, /proxy_set_header Upgrade \$http_upgrade;/)
  assert.match(config, /proxy_set_header Connection \$connection_upgrade;/)
  assert.ok(!config.includes('Access-Control-Allow-Origin'))
  assert.ok(!config.includes('proxy_hide_header Access-Control'))
  assert.ok(!config.includes('proxy_set_header Origin'))
  assert.ok(!config.includes('proxy_set_header Authorization'))
  assert.equal((config.match(/add_header Content-Security-Policy/g) ?? []).length, 1, 'only the host response gets an edge CSP')
  assert.ok(config.includes("script-src 'self' 'unsafe-eval'"), 'the qiankun host permits its trusted HTML/UMD evaluator')
  assert.ok(!renderGateway({ ...gateway, trustedHtmlEval: false }).includes('unsafe-eval'), 'operators can explicitly disable trusted HTML execution')
  assert.match(config, /proxy_set_header Host "platform\.example\.test";/)
  assert.match(config, /return 308 https:\/\/platform\.example\.test\$request_uri;/)
})

test('checked-in gateway sample and host Nginx example permit qiankun with exact source boundaries', (t) => {
  const root = directory(t), output = path.join(root, 'nginx.conf')
  const samplePath = path.join(repository, 'backend/nginx/config.example.json')
  const { kind, ...sample } = JSON.parse(readFileSync(samplePath, 'utf8'))
  assert.equal(kind, 'gateway')
  assert.equal(sample.trustedHtmlEval, true, 'the deployment sample must enable the current qiankun runtime')
  const result = spawnSync(process.execPath, [path.join(repository, 'backend/nginx/render.mjs'), samplePath, output], { encoding: 'utf8', timeout: 5000, windowsHide: true })
  assert.equal(result.status, 0, result.stderr)
  const config = readFileSync(output, 'utf8')
  assert.equal(config, renderGateway(sample))
  const policy = (text) => {
    const match = text.match(/add_header Content-Security-Policy "([^"]+)" always;/)
    assert.ok(match, 'host response has a single generated CSP')
    return Object.fromEntries(match[1].split(';').map(directive => {
      const [name, ...values] = directive.trim().split(/\s+/)
      return [name, values]
    }))
  }
  const csp = policy(config)
  assert.deepEqual(csp['script-src'], ["'self'", "'unsafe-eval'"])
  assert.deepEqual(csp['connect-src'], ["'self'", sample.publicOrigin, sample.publicOrigin.replace('https:', 'wss:')])
  assert.deepEqual(csp['frame-ancestors'], ["'none'"])
  assert.ok(!config.includes("script-src 'self' 'unsafe-inline'"))
  const direct = policy(renderGateway({ ...sample, applicationOrigins: ['https://apps.example.test'] }))
  assert.deepEqual(direct['script-src'], csp['script-src'], 'fetch approval does not approve external script tags')
  assert.deepEqual(direct['connect-src'], ["'self'", 'https://apps.example.test', sample.publicOrigin, sample.publicOrigin.replace('https:', 'wss:')])
  for (const trustedHtmlEval of [null, 'true', 1]) assert.throws(() => renderGateway({ ...sample, trustedHtmlEval }), /boolean/)
  for (const applicationOrigins of [['https://*.example.test'], ['https://apps.example.test/'], ['https:'], ['*']]) {
    assert.throws(() => renderGateway({ ...sample, applicationOrigins }), /origin|hostname/i)
  }
  const hostExample = readFileSync(path.join(repository, 'backend/docs/examples/host-nginx.conf.example'), 'utf8')
  assert.deepEqual(policy(hostExample), policy(renderGateway({ publicOrigin: 'https://saas.example.com' })))
  assert.match(hostExample, /location \/ \{[\s\S]*?proxy_hide_header Content-Security-Policy;/)
})

test('approved HTTPS branding images expand only the image policy', () => {
  for (const render of [renderGateway, renderMicroStatic]) {
    const config = render({ ...gateway, imageOrigins: ['https://images.example.test'] })
    assert.match(config, /img-src 'self' data: blob: https:\/\/images\.example\.test;/)
    assert.ok(!config.includes('img-src https:'))
    assert.ok(!/connect-src[^;]*images\.example\.test/.test(config))
    assert.ok(!/frame-src[^;]*images\.example\.test/.test(config))
    for (const imageOrigins of [['https://images.example.test/'], ['http://images.example.test'], ['https://*.example.test'], ['https://images.example.test;script-src *']]) {
      assert.throws(() => render({ ...gateway, imageOrigins }))
    }
  }
})

test('immutable routes require explicit app/version/service mapping and unknown versions cannot use the host', () => {
  const config = renderGateway({ ...gateway, versionRoutes: [{ app: 'orders', version: '1.2.0', upstream: 'orders-v120:8080' }], allowedServices: ['backend', 'frontend-web', 'orders-v120'] })
  assert.match(config, /location \^~ \/sub-apps\/orders\/versions\/1\.2\.0\//)
  assert.match(config, /location \/sub-apps\/ \{ return 404; \}/)
  for (const versionRoutes of [[{ app: '../orders', version: '1', upstream: 'frontend-web:8080' }], [{ app: 'orders', version: '1;return 200', upstream: 'frontend-web:8080' }], [{ app: 'orders', version: '1', upstream: 'evil:8080' }], [{ app: 'orders', version: '1', upstream: 'frontend-web:8080' }, { app: 'orders', version: '1', upstream: 'backend:3001' }]]) {
    assert.throws(() => renderGateway({ ...gateway, versionRoutes }))
  }
  assert.throws(() => renderGateway({ ...gateway, command: 'docker run arbitrary' }), /unknown/i)
})

test('URL microapp resources reach the backend without stealing host micro navigation', () => {
  const config = renderGateway(gateway)
  assert.match(config, /location = \/micro-proxy \{ return 404; \}/)
  assert.match(config, /location \^~ \/micro-proxy\/ \{[\s\S]*?set \$micro_proxy_backend "backend:3001";[\s\S]*?proxy_pass http:\/\/\$micro_proxy_backend\$request_uri;/)
  assert.match(config, /location \^~ \/micro-proxy\/ \{[\s\S]*?proxy_set_header Upgrade "";[\s\S]*?proxy_set_header Connection "";/)
  assert.ok(!/location[^\n]*\/micro\//.test(config), 'the host keeps its authorized /micro/* React routes')
  const frontend = readFileSync(path.join(repository, 'frontend/deploy/nginx.conf'), 'utf8')
  assert.match(frontend, /location \^~ \/micro-proxy\/ \{ return 404; \}/)
  assert.match(frontend, /system\|athena\|hermes\|micro/)
})

test('BFF namespace reaches its cookie owner with exact host and safe callback diagnostics', () => {
  const config = renderGateway(gateway)
  for (const prefix of ['= /auth', '/auth/']) {
    const start = config.indexOf(`location ${prefix} {`)
    assert.ok(start >= 0)
    const block = config.slice(start, config.indexOf('\n        }', start))
    assert.match(block, /proxy_pass http:\/\/\$platform\$request_uri;/)
    assert.match(block, /proxy_set_header Host "platform\.example\.test";/)
    assert.match(block, /error_log \/dev\/null crit;/)
    assert.match(block, /Referrer-Policy "no-referrer" always;/)
    assert.ok(!block.includes('proxy_set_header Cookie'))
    assert.ok(!block.includes('proxy_hide_header Set-Cookie'))
  }
  assert.match(config, /log_format gateway '[^']*\$uri[^']*';/)
  assert.ok(!/log_format[^;]*\$(?:request_uri|request|args)\b/.test(config))
})

test('retired issuer and exercise configuration cannot enable another authority', () => {
  for (const field of ['oauthPlayground', 'oauthPlaygroundStatic']) {
    for (const target of [null, '127.0.0.1:3011']) assert.throws(() => renderGateway({ ...gateway, [field]: target }), /unknown configuration field/)
  }
  const config = renderGateway(gateway)
  for (const prefix of ['/oauth', '/oidc', '/api/oauth', '/api/oidc', '/oauth-playground-api', '/sub/oauth-playground']) {
    assert.ok(config.includes(`location = ${prefix} { return 404; }`))
    assert.ok(config.includes(`location ^~ ${prefix}/ { return 404; }`))
  }
  assert.ok(!config.includes('$oauth_playground'))
  const hostExample = readFileSync(path.join(repository, 'backend/docs/examples/host-nginx.conf.example'), 'utf8')
  for (const source of [config, hostExample]) {
    const rules = [...source.matchAll(/location ~ ([^\s{}]+) \{ return 404; \}/g)].map(match => new RegExp(match[1]))
    const authorize = rules.find(rule => rule.test('/api/apps/work-orders/authorize'))
    assert.ok(authorize, 'the public gateway must deny the service-only resource authorization port')
    for (const app of ['stores', 'work-orders', 'app2', 'registered-service', 'app_with_underscore', 'UppercaseApp']) assert.equal(authorize.test(`/api/apps/${app}/authorize`), true)
    for (const publicPath of ['/api/apps/session-token', '/api/apps/stores/context', '/api/apps/work-orders/members', '/api/apps/stores/organizations']) assert.equal(authorize.test(publicPath), false, publicPath)
  }
})

test('ZIP static policy is anonymous, opaque by default, and confines files to version storage', () => {
  const config = renderMicroStatic({ publicOrigin: gateway.publicOrigin, microRoot: '/micro-apps' })
  assert.match(config, /Access-Control-Allow-Origin "\*" always;/)
  assert.ok(!config.includes('Access-Control-Allow-Credentials'))
  assert.ok(!config.includes('unsafe-eval'))
  assert.ok(!config.includes('X-Frame-Options'))
  assert.match(config, /frame-ancestors https:\/\/platform\.example\.test/)
  assert.match(config, /disable_symlinks on;/)
  assert.match(config, /versions\//)
  assert.match(config, /try_files \$uri \$uri\/index\.html =404;/)
  assert.match(config, /default "no-store";/)
  assert.match(config, /immutable/)
  assert.match(config, /application\/javascript mjs cjs;/)
  assert.match(config, /staging/)
  assert.match(config, /location \/ \{ return 404; \}/)
  for (const replacement of [{ microRoot: '/micro-apps/../secrets' }, { microRoot: '/micro-apps\ninclude /secret' }, { certificate: '/unused' }, { applicationOrigins: ['https://other.example.test/'] }]) {
    assert.throws(() => renderMicroStatic({ publicOrigin: gateway.publicOrigin, ...replacement }))
  }
  assert.ok(renderMicroStatic({ publicOrigin: gateway.publicOrigin, trustedHtmlEval: true }).includes("script-src 'self' 'unsafe-eval'"))
})

test('ZIP metadata guards deny reserved path segments and decoded private names before public versions', () => {
  const config = renderMicroStatic({ publicOrigin: gateway.publicOrigin, microRoot: '/micro-apps' })
  const guards = [...config.matchAll(/location (~\*?) ([^\s{}]+) \{ return 404; \}/g)]
    .map((match) => ({ index: match.index, expression: new RegExp(match[2], match[1] === '~*' ? 'i' : '') }))
  const release = guards.find((guard) => guard.expression.test('/release.json'))
  assert.ok(release, 'release.json is internal registration metadata')
  const versionIndex = config.indexOf('location ~ ^/sub-apps/')
  assert.ok(release.index < versionIndex, 'the first matching regex must deny metadata before static file lookup')
  const prefix = '/sub-apps/sample/versions/1.0.0/'
  for (const name of ['release.json', 'RELEASE.JSON', 'assets/Release.Json', 'assets/release.json/private.js']) {
    assert.ok(release.expression.test(prefix + name), name)
  }
  for (const name of ['index.html', 'assets/release.json.js', 'assets/my-release.json']) {
    assert.equal(release.expression.test(prefix + name), false, name)
  }
  // Nginx locations match its decoded URI path rather than raw request_uri.
  // Check encoded names and query separation against the emitted rules; the
  // actual Nginx gate separately verifies HTTP normalization and response headers.
  const normalized = (target) => decodeURIComponent(target.split('?')[0])
  for (const target of ['%72elease%2ejson?download=1', 'assets/RELEASE%2eJSON', '%2erelease-artifact.json', '%2elocks/install.lock', 'assets/%2erecovery/state.json']) {
    const uri = normalized(prefix + target)
    assert.ok(guards.some((guard) => guard.index < versionIndex && guard.expression.test(uri)), target)
  }
  assert.equal(guards.some((guard) => guard.expression.test(normalized(prefix + 'app.js?file=release.json'))), false)
  assert.match(config, /map \$status \$version_response_cache \{\s+default "no-store";/)
  assert.match(config, /add_header Cache-Control \$version_response_cache always;/)
})

test('deployment CLI validates before writing and cannot turn input into a command', (t) => {
  const root = directory(t), source = path.join(root, 'input.json'), output = path.join(root, 'nginx.conf')
  writeFileSync(source, JSON.stringify({ ...gateway, backend: 'backend:3001; include /arbitrary;' }))
  const rejected = spawnSync(process.execPath, [path.join(repository, 'backend/nginx/render.mjs'), source, output], { encoding: 'utf8', timeout: 5000, windowsHide: true })
  assert.notEqual(rejected.status, 0)
  assert.equal(existsSync(output), false)
  writeFileSync(source, JSON.stringify(gateway))
  const accepted = spawnSync(process.execPath, [path.join(repository, 'backend/nginx/render.mjs'), source, output], { encoding: 'utf8', timeout: 5000, windowsHide: true })
  assert.equal(accepted.status, 0, accepted.stderr)
  assert.equal(readFileSync(output, 'utf8'), renderGateway(gateway))
})

test('ZIP CLI selects a static-only configuration and rejects unknown deployment kinds', (t) => {
  const root = directory(t), source = path.join(root, 'input.json'), output = path.join(root, 'nginx.conf')
  const run = () => spawnSync(process.execPath, [path.join(repository, 'backend/nginx/render.mjs'), source, output], { encoding: 'utf8', timeout: 5000, windowsHide: true })
  writeFileSync(source, JSON.stringify({ kind: 'shell', ...gateway }))
  assert.notEqual(run().status, 0)
  assert.equal(existsSync(output), false)
  writeFileSync(source, JSON.stringify({ kind: 'micro-static', ...gateway }))
  const accepted = run()
  assert.equal(accepted.status, 0, accepted.stderr)
  assert.equal(readFileSync(output, 'utf8'), renderMicroStatic(gateway))
  assert.ok(!readFileSync(output, 'utf8').includes('proxy_pass'))
})

// Opt in with local executables or the same Nginx/OpenSSL versions as the edge
// image. Every listener and process is confined to this test and then closed.
test('real generated Nginx serves the trusted qiankun host and rejects retired issuer routes', {
  skip: !process.env.PLATFORM_TEST_NGINX || !process.env.PLATFORM_TEST_OPENSSL,
  timeout: 45000,
}, async (t) => {
  const rootPath = path.resolve(mkdtempSync(path.join(os.tmpdir(), 'platform-gateway-render-')))
  const root = rootPath.replaceAll('\\', '/')
  mkdirSync(`${root}/logs`)
  const servers = [], children = []
  t.after(async () => {
    const cleanupErrors = []
    for (const child of children) {
      try {
        if (child.pid !== undefined && child.exitCode === null && child.signalCode === null) {
          const stopped = once(child, 'exit').then(() => true)
          const wait = async (milliseconds) => {
            let timer
            try {
              return await Promise.race([stopped, new Promise((resolve) => { timer = setTimeout(() => resolve(false), milliseconds) })])
            } finally { clearTimeout(timer) }
          }
          child.kill()
          if (!await wait(2000)) {
            child.kill('SIGKILL')
            assert.equal(await wait(2000), true, 'owned native Nginx must exit after forced termination')
          }
        }
      } catch (error) { cleanupErrors.push(error.message) }
    }
    for (const server of servers) {
      try {
        server.closeAllConnections()
        if (server.listening) await new Promise((resolve) => server.close(resolve))
      } catch (error) { cleanupErrors.push(error.message) }
    }
    try {
      assert.equal(path.dirname(rootPath), path.resolve(os.tmpdir()))
      assert.ok(path.basename(rootPath).startsWith('platform-gateway-render-'))
      assert.ok(!lstatSync(rootPath).isSymbolicLink())
      rmSync(rootPath, { recursive: true, force: true })
    } catch (error) { cleanupErrors.push(error.message) }
    assert.deepEqual(cleanupErrors, [], 'all native fixture resources must be cleaned')
  })
  const platformRequests = []
  const upstream = async (requests, responseHeaders = {}, fixtures = {}) => {
    const server = http.createServer((req, res) => {
      requests.push({ url: req.url, method: req.method, headers: req.headers })
      if (req.url.startsWith('/auth/callback?code=owned-bff-error')) { req.socket.destroy(); return }
      const fixture = fixtures[req.url]
      if (fixture) {
        res.writeHead(200, { 'Content-Type': fixture.type, ...responseHeaders })
        res.end(fixture.body)
        return
      }
      res.writeHead(200, { 'Content-Type': 'application/json', ...responseHeaders })
      res.end(JSON.stringify(requests.at(-1)))
    })
    servers.push(server)
    server.listen(0, '127.0.0.1')
    await once(server, 'listening')
    return server.address().port
  }
  const browserFixtures = {
    '/__csp-host.html': { type: 'text/html', body: '<!doctype html><html><body><p id="host-stays">Host remains</p><div id="mount"></div><script src="/__qiankun.js"></script><script src="/__csp-host.js"></script></body></html>' },
    '/__csp-host.js': { type: 'application/javascript', body: `
      window.__cspViolations = [];
      document.addEventListener('securitypolicyviolation', event => window.__cspViolations.push(event.effectiveDirective));
      window.__fixture = qiankun.loadMicroApp({ name: 'cspChild', entry: '/sub-apps/csp-child/versions/1.0.0/entry.html',
        container: '#mount', props: { label: 'mounted' } }, { singular: false, sandbox: { experimentalStyleIsolation: true } });
      window.__fixture.mountPromise.catch(error => { window.__mountError = String(error) });` },
    '/entry.html': { type: 'text/html', body: '<!doctype html><html><body><div data-csp-child></div><script src="./child.js" entry></script></body></html>' },
    '/child.js': { type: 'application/javascript', body: `(function () {
      var element;
      window.cspChild = {
        bootstrap: function () { return Promise.resolve() },
        mount: function (props) { element = props.container.querySelector('[data-csp-child]'); element.textContent = props.label; return Promise.resolve() },
        update: function (props) { element.textContent = props.label; return Promise.resolve() },
        unmount: function () { element.replaceChildren(); return Promise.resolve() }
      };
    })();` },
  }
  if (process.env.PLATFORM_TEST_BROWSER_PACKAGE) browserFixtures['/__qiankun.js'] = {
    type: 'application/javascript', body: readFileSync(path.join(repository, 'frontend/node_modules/qiankun/dist/index.umd.js')),
  }
  // An existing static upstream can carry an obsolete restrictive CSP. The
  // generated edge must replace it on host responses, not append another CSP.
  const platformPort = await upstream(platformRequests, { 'Content-Security-Policy': "script-src 'self'" }, browserFixtures)
  const reserve = async () => {
    const server = http.createServer()
    servers.push(server)
    server.listen(0, '127.0.0.1')
    await once(server, 'listening')
    return server
  }
  const tlsReservation = await reserve(), redirectReservation = await reserve()
  const tlsPort = tlsReservation.address().port, redirectPort = redirectReservation.address().port
  const certificate = `${root}/certificate.pem`, certificateKey = `${root}/key.pem`
  const generated = spawnSync(process.env.PLATFORM_TEST_OPENSSL, ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', certificateKey, '-out', certificate, '-days', '1', '-subj', '/CN=localhost', '-addext', 'subjectAltName=DNS:localhost,IP:127.0.0.1', '-addext', 'basicConstraints=critical,CA:TRUE', '-addext', 'keyUsage=critical,digitalSignature,keyEncipherment,keyCertSign', '-addext', 'extendedKeyUsage=serverAuth'], { encoding: 'utf8', timeout: 10000, windowsHide: true })
  assert.equal(generated.status, 0, generated.stderr)
  const fixtureCertificate = readFileSync(certificate)
  const origin = `https://localhost:${tlsPort}`
  const { kind: _kind, ...sampleDeployment } = JSON.parse(readFileSync(path.join(repository, 'backend/nginx/config.example.json'), 'utf8'))
  const deployment = {
    ...sampleDeployment,
    publicOrigin: origin, listener: `127.0.0.1:${tlsPort}`, httpListener: `127.0.0.1:${redirectPort}`,
    backend: `127.0.0.1:${platformPort}`, frontend: `127.0.0.1:${platformPort}`,
    versionRoutes: [{ app: 'csp-child', version: '1.0.0', upstream: `127.0.0.1:${platformPort}` }],
    certificate, certificateKey,
  }
  // Only process/log/temp paths and MIME lookup differ on a native fixture.
  // All generated locations, rewrites and proxy headers are unchanged.
  if (process.env.PLATFORM_TEST_NGINX_MIME_TYPES) {
    assert.ok(path.isAbsolute(process.env.PLATFORM_TEST_NGINX_MIME_TYPES), 'native MIME override must be absolute')
    assert.ok(existsSync(process.env.PLATFORM_TEST_NGINX_MIME_TYPES), 'native MIME override must exist')
  }
  const mimeTypes = process.env.PLATFORM_TEST_NGINX_MIME_TYPES ?? path.resolve(path.dirname(process.env.PLATFORM_TEST_NGINX), 'conf/mime.types')
  if (existsSync(mimeTypes)) deployment.mimeTypes = mimeTypes.replaceAll('\\', '/')
  const nativeConfig = (input) => renderGateway(input).replace('daemon off;', 'daemon off;\nmaster_process off;').replace('worker_processes auto;', 'worker_processes 1;')
    .replace(/"\/tmp\/(nginx\.pid|client_body|proxy|fastcgi|uwsgi|scgi)"/g, (_match, name) => `"${root}/${name}"`)
    .replaceAll('/dev/stdout', `${root}/access.log`).replaceAll('/dev/stderr', `${root}/error.log`)
  const file = `${root}/nginx.conf`
  writeFileSync(file, nativeConfig(deployment))
  await Promise.all([tlsReservation, redirectReservation].map((server) => new Promise((resolve) => server.close(resolve))))
  const nativeArguments = ['-e', `${root}/startup-error.log`, '-p', `${root}/`, '-c', file]
  const validate = spawnSync(process.env.PLATFORM_TEST_NGINX, ['-t', ...nativeArguments], { encoding: 'utf8', timeout: 5000, windowsHide: true })
  assert.equal(validate.status, 0, validate.stderr)
  const nginx = spawn(process.env.PLATFORM_TEST_NGINX, nativeArguments, { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
  children.push(nginx)
  let diagnostics = ''
  let spawnError
  nginx.on('error', (error) => { spawnError = error })
  nginx.stderr.on('data', (chunk) => { diagnostics += chunk })
  const request = (target, { method = 'GET', headers = {}, ca = fixtureCertificate, servername = 'localhost' } = {}) => new Promise((resolve, reject) => {
    // Trust only the throwaway local fixture certificate for this test request.
    const req = https.request({ host: '127.0.0.1', port: tlsPort, path: target, method, agent: false, ca, servername, rejectUnauthorized: true, headers: { Host: `localhost:${tlsPort}`, ...headers } }, (res) => {
      const chunks = []
      res.on('data', (chunk) => chunks.push(chunk))
      res.on('end', () => resolve({ status: res.statusCode, headers: res.headers, body: Buffer.concat(chunks).toString() }))
    })
    req.on('error', reject)
    req.setTimeout(2000, () => req.destroy(new Error('local Nginx fixture request timed out')))
    req.end()
  })
  const deadline = Date.now() + 5000
  while (true) {
    try { await request('/'); break } catch (error) {
      assert.ifError(spawnError)
      assert.equal(nginx.exitCode, null, diagnostics)
      if (Date.now() >= deadline) throw error
      await new Promise((resolve) => setTimeout(resolve, 25))
    }
  }
  await t.test('HTTPS rejects an untrusted fixture certificate before reaching the upstream', async () => {
    const before = platformRequests.length
    await assert.rejects(request('/__untrusted-ca', { ca: null }), { code: 'DEPTH_ZERO_SELF_SIGNED_CERT' })
    assert.equal(platformRequests.length, before)
  })
  await t.test('HTTPS rejects an incorrect hostname before reaching the upstream', async () => {
    const before = platformRequests.length
    await assert.rejects(request('/__incorrect-hostname', { servername: 'incorrect-hostname.example.invalid' }), { code: 'ERR_TLS_CERT_ALTNAME_INVALID' })
    assert.equal(platformRequests.length, before)
  })
  const hostResponse = await request('/__csp-host.html')
  assert.equal(hostResponse.status, 200)
  assert.match(hostResponse.headers['content-security-policy'], /script-src 'self' 'unsafe-eval';/)
  assert.equal(hostResponse.headers['content-security-policy'].includes(','), false, 'upstream CSP must not be appended')
  for (const target of ['/auth/config', '/auth/callback?code=owned-bff-code&state=owned-bff-state']) {
    const result = await request(target, { headers: { Cookie: '__Host-saas_console=owned-bff-cookie', Origin: origin } })
    assert.equal(result.status, 200)
    const forwarded = platformRequests.at(-1)
    assert.equal(forwarded.url, target)
    assert.equal(forwarded.headers.host, `localhost:${tlsPort}`)
    assert.equal(forwarded.headers.cookie, '__Host-saas_console=owned-bff-cookie')
    assert.equal(forwarded.headers.origin, origin)
    assert.equal(result.headers['cache-control'], 'no-store')
    assert.equal(result.headers['referrer-policy'], 'no-referrer')
  }
  assert.ok(!readFileSync(`${root}/access.log`, 'utf8').includes('owned-bff-code'))
  assert.equal((await request('/auth/callback?code=owned-bff-error&state=owned-bff-error-state')).status, 502)
  for (const name of ['access.log', 'error.log']) {
    assert.ok(!readFileSync(`${root}/${name}`, 'utf8').includes('owned-bff-error'), `${name} must omit failed callback query`)
  }
  await t.test('browser mounts, updates and unmounts real qiankun under the sample HTTPS CSP', {
    skip: !process.env.PLATFORM_TEST_BROWSER_PACKAGE,
  }, async () => {
    const { chromium } = await import(pathToFileURL(path.resolve(process.env.PLATFORM_TEST_BROWSER_PACKAGE)).href)
    const browser = await chromium.launch({ headless: true, args: ['--no-sandbox', '--no-proxy-server'] })
    try {
      const context = await browser.newContext()
      const page = await context.newPage()
      const errors = []
      page.on('pageerror', error => errors.push(error.message))
      const response = await page.goto(`${origin}/__csp-host.html`)
      assert.equal(response.status(), 200)
      assert.match(response.headers()['content-security-policy'], /script-src 'self' 'unsafe-eval';/)
      await page.waitForFunction(() => document.querySelector('[data-csp-child]')?.textContent === 'mounted')
      await page.evaluate(() => window.__fixture.update({ label: 'updated' }))
      assert.equal(await page.locator('[data-csp-child]').textContent(), 'updated')
      await page.evaluate(() => window.__fixture.unmount())
      assert.equal(await page.locator('[data-csp-child]').count(), 0)
      assert.equal(await page.locator('#host-stays').textContent(), 'Host remains')
      assert.deepEqual(await page.evaluate(() => window.__cspViolations), [])
      assert.deepEqual(errors, [])
      await page.evaluate(() => fetch('https://unapproved.example.invalid/asset.js').catch(() => {}))
      await page.waitForFunction(() => window.__cspViolations.includes('connect-src'))
      t.diagnostic(`Real qiankun 2.10.16 HTML/UMD lifecycle passed under Nginx HTTPS CSP in Chromium ${browser.version()}`)
      await context.close()
    } finally { await browser.close() }
  })
  for (const [target, method] of [['/api/apps/session-token', 'POST'], ['/api/apps/stores/context', 'GET'], ['/api/apps/work-orders/context', 'GET']]) {
    const result = await request(target, { method })
    assert.equal(result.status, 200)
    assert.equal(platformRequests.at(-1).url, target, 'browser app capability/context traffic still reaches the platform')
    assert.equal(platformRequests.at(-1).method, method)
  }
  const before = platformRequests.length
  for (const target of ['/oauth', '/oauth/token', '/oidc/token', '/api/oauth/clients', '/api/oidc/clients', '/oauth-playground-api', '/oauth-playground-api/config', '/callback?code=fixture-disabled', '/sub/oauth-playground', '/sub/oauth-playground/assets/callback.js', '/device', '/consent', '/api/apps/stores/authorize', '/api/apps/work-orders/authorize', '/api/apps/registered-service/authorize', '/api/apps/app_with_underscore/authorize', '/api/apps/UppercaseApp/authorize', '/api/apps/stores/authorize?token=fixture-private', '/api/internal/notifications/context-proof', '/internal/platform/apps/stores/organizations']) {
    for (const method of ['GET', 'POST']) assert.equal((await request(target, { method })).status, 404)
  }
  assert.equal(platformRequests.length, before, 'retired routes never reach an application or static upstream')
})
