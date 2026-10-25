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

test('OAuth playground upstreams use the same explicit private allowlist as platform services', () => {
  for (const field of ['oauthPlayground', 'oauthPlaygroundStatic']) {
    for (const target of ['https://oauth.example.test', 'oauth-api:3011', '8.8.8.8:3011', '10.20.0.9:3011', '127.0.0.1:3011/path', '127.0.0.1:3011;return 200']) {
      assert.throws(() => renderGateway({ ...gateway, [field]: target }), /upstream|port/i)
    }
    assert.doesNotThrow(() => renderGateway({ ...gateway, [field]: '127.0.0.1:3011' }))
    assert.doesNotThrow(() => renderGateway({ ...gateway, [field]: '10.20.0.9:3011', allowedUpstreamIps: ['10.20.0.9'] }))
    assert.doesNotThrow(() => renderGateway({ ...gateway, [field]: 'oauth-api:3011', allowedServices: ['backend', 'frontend-web', 'oauth-api'] }))
  }
})

test('OAuth namespaces default to 404 and configured routes isolate credentials and keep upstream headers', () => {
  const disabled = renderGateway(gateway)
  for (const location of ['= /oauth-playground-api', '^~ /oauth-playground-api/', '= /callback', '= /sub/oauth-playground', '^~ /sub/oauth-playground/']) {
    assert.ok(disabled.includes(`location ${location} { return 404; }`), location)
  }
  assert.ok(!disabled.includes('$oauth_playground_cookie'))
  const config = renderGateway({ ...gateway, oauthPlayground: '127.0.0.1:3011', oauthPlaygroundStatic: '127.0.0.1:3010' })
  assert.match(config, /location = \/oauth-playground-api \{/)
  assert.match(config, /location \^~ \/oauth-playground-api\/ \{/)
  assert.match(config, /rewrite \^\/oauth-playground-api\(\/\.\*\)\?\$ \/api\$1 break;/)
  assert.match(config, /proxy_pass http:\/\/\$oauth_playground;/)
  assert.match(config, /proxy_set_header Cookie \$oauth_playground_cookies;/)
  assert.match(config, /proxy_set_header Origin \$http_origin;/)
  assert.match(config, /proxy_set_header Authorization "";/)
  assert.match(config, /map \$http_cookie \$oauth_playground_cookie \{/)
  assert.match(config, /__Host-oauth_playground=\(\?<oauth_pg_host_cookie_value>/)
  assert.ok(!config.includes('proxy_hide_header Set-Cookie'))
  assert.ok(!config.includes('proxy_cookie_'))
  assert.ok(!config.includes('proxy_hide_header Origin'))
  for (const name of ['internal', 'metrics']) {
    assert.ok(config.includes(`location = /oauth-playground-api/${name} { return 404; }`))
    assert.ok(config.includes(`location ^~ /oauth-playground-api/${name}/ { return 404; }`))
  }
  assert.match(config, /location = \/callback \{[\s\S]*?rewrite \^ \/callback\.html break;/)
  assert.match(config, /location = \/sub\/oauth-playground \{[\s\S]*?return 308 https:\/\/platform\.example\.test\/sub\/oauth-playground\/\$is_args\$args;/)
  assert.match(config, /location \^~ \/sub\/oauth-playground\/ \{[\s\S]*?rewrite \^\/sub\/oauth-playground\/\(\.\*\)\$ \/\$1 break;/)
  assert.match(config, /if \(\$request_method !~ \^\(GET\|HEAD\)\$\) \{ return 405; \}/)
  assert.match(config, /add_header Allow \$oauth_playground_static_allow always;/)
  assert.match(config, /proxy_set_header Cookie "";/)
  assert.match(config, /add_header Referrer-Policy "no-referrer" always;/)
  assert.equal((config.match(/proxy_set_header Upgrade "";/g) ?? []).length, 5, 'API, static and the existing micro proxy cannot upgrade arbitrary connections')
  assert.ok(!config.includes('location /oauth-playground-api'), 'the prefix must include a namespace boundary')
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
test('real generated Nginx serves the trusted qiankun host and isolates OAuth credentials', {
  skip: !process.env.PLATFORM_TEST_NGINX || !process.env.PLATFORM_TEST_OPENSSL,
  timeout: 45000,
}, async (t) => {
  const rootPath = path.resolve(mkdtempSync(path.join(os.tmpdir(), 'platform-gateway-render-')))
  const root = rootPath.replaceAll('\\', '/')
  mkdirSync(`${root}/logs`)
  const servers = [], children = []
  t.after(async () => {
    for (const child of children) {
      if (child.exitCode === null && child.signalCode === null) {
        child.kill()
        await once(child, 'exit')
      }
    }
    for (const server of servers) {
      server.closeAllConnections()
      if (server.listening) await new Promise((resolve) => server.close(resolve))
    }
    assert.equal(path.dirname(rootPath), path.resolve(os.tmpdir()))
    assert.ok(path.basename(rootPath).startsWith('platform-gateway-render-'))
    assert.ok(!lstatSync(rootPath).isSymbolicLink())
    rmSync(rootPath, { recursive: true, force: true })
  })
  const apiRequests = [], staticRequests = [], platformRequests = []
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
  const cookies = ['__Host-oauth_playground=fixture; Path=/; HttpOnly; Secure; SameSite=Lax', 'oauth_playground=fixture-dev; Path=/; HttpOnly; SameSite=Lax']
  const apiPort = await upstream(apiRequests, { 'Set-Cookie': cookies })
  const staticPort = await upstream(staticRequests)
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
  const generated = spawnSync(process.env.PLATFORM_TEST_OPENSSL, ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', certificateKey, '-out', certificate, '-days', '1', '-subj', '/CN=localhost'], { encoding: 'utf8', timeout: 10000, windowsHide: true })
  assert.equal(generated.status, 0, generated.stderr)
  const origin = `https://localhost:${tlsPort}`
  const { kind: _kind, ...sampleDeployment } = JSON.parse(readFileSync(path.join(repository, 'backend/nginx/config.example.json'), 'utf8'))
  const deployment = {
    ...sampleDeployment,
    publicOrigin: origin, listener: `127.0.0.1:${tlsPort}`, httpListener: `127.0.0.1:${redirectPort}`,
    backend: `127.0.0.1:${platformPort}`, frontend: `127.0.0.1:${platformPort}`,
    oauthPlayground: `127.0.0.1:${apiPort}`, oauthPlaygroundStatic: `127.0.0.1:${staticPort}`,
    versionRoutes: [{ app: 'csp-child', version: '1.0.0', upstream: `127.0.0.1:${platformPort}` }],
    certificate, certificateKey,
  }
  // Only process/log/temp paths and MIME lookup differ on a native Windows
  // fixture. All generated locations, rewrites and proxy headers are unchanged.
  const mimeTypes = path.resolve(path.dirname(process.env.PLATFORM_TEST_NGINX), 'conf/mime.types')
  if (existsSync(mimeTypes)) deployment.mimeTypes = mimeTypes.replaceAll('\\', '/')
  const nativeConfig = (input) => renderGateway(input).replace('daemon off;', 'daemon off;\nmaster_process off;').replace('worker_processes auto;', 'worker_processes 1;')
    .replace(/"\/tmp\/(nginx\.pid|client_body|proxy|fastcgi|uwsgi|scgi)"/g, (_match, name) => `"${root}/${name}"`)
    .replaceAll('/dev/stdout', `${root}/access.log`).replaceAll('/dev/stderr', `${root}/error.log`)
  const file = `${root}/nginx.conf`
  writeFileSync(file, nativeConfig(deployment))
  await Promise.all([tlsReservation, redirectReservation].map((server) => new Promise((resolve) => server.close(resolve))))
  const validate = spawnSync(process.env.PLATFORM_TEST_NGINX, ['-t', '-p', `${root}/`, '-c', file], { encoding: 'utf8', timeout: 5000, windowsHide: true })
  assert.equal(validate.status, 0, validate.stderr)
  const nginx = spawn(process.env.PLATFORM_TEST_NGINX, ['-p', `${root}/`, '-c', file], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
  children.push(nginx)
  let diagnostics = ''
  nginx.stderr.on('data', (chunk) => { diagnostics += chunk })
  const request = (target, { method = 'GET', headers = {} } = {}) => new Promise((resolve, reject) => {
    // Trust only the throwaway local fixture certificate for this test request.
    const req = https.request({ host: '127.0.0.1', port: tlsPort, path: target, method, agent: false, rejectUnauthorized: false, headers: { Host: `localhost:${tlsPort}`, ...headers } }, (res) => {
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
      assert.equal(nginx.exitCode, null, diagnostics)
      if (Date.now() >= deadline) throw error
      await new Promise((resolve) => setTimeout(resolve, 25))
    }
  }
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
      const context = await browser.newContext({ ignoreHTTPSErrors: true })
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
  platformRequests.length = 0
  for (const [target, expected] of [['/oauth-playground-api', '/api'], ['/oauth-playground-api/', '/api/'], ['/oauth-playground-api/config?return=%2Fmicro%2Foauth&next=1', '/api/config?return=%2Fmicro%2Foauth&next=1']]) {
    const result = await request(target, { headers: { Authorization: 'Bearer platform-fixture', Cookie: 'platform_session=private; oauth_playground=dev; platform_refresh=private; __Host-oauth_playground=secure; other=private', Origin: origin, Upgrade: 'websocket', Connection: 'upgrade' } })
    assert.equal(result.status, 200)
    const forwarded = apiRequests.at(-1)
    assert.equal(forwarded.url, expected)
    assert.equal(forwarded.headers.cookie, 'oauth_playground=dev; __Host-oauth_playground=secure')
    assert.equal(forwarded.headers.authorization, undefined)
    assert.equal(forwarded.headers.origin, origin)
    assert.equal(forwarded.headers.upgrade, undefined)
    assert.equal(forwarded.headers.connection, undefined)
    assert.deepEqual(result.headers['set-cookie'], cookies)
  }
  for (const [cookie, expected] of [['platform_session=private', undefined], ['oauth_playground=only', 'oauth_playground=only'], ['__Host-oauth_playground=only', '__Host-oauth_playground=only'], ['other_oauth_playground=private; OAUTH_PLAYGROUND=private; __Host-oauth_playground_extra=private', undefined], ['', undefined]]) {
    await request('/oauth-playground-api/config', { headers: { Cookie: cookie } })
    assert.equal(apiRequests.at(-1).headers.cookie, expected)
  }
  for (const target of ['/oauth-playground-api/internal', '/oauth-playground-api/internal/private', '/oauth-playground-api/metrics', '/oauth-playground-api/metrics/private']) {
    const count = apiRequests.length
    assert.equal((await request(target)).status, 404)
    assert.equal(apiRequests.length, count)
  }
  await request('/oauth-playground-api-other/config')
  assert.equal(platformRequests.at(-1).url, '/oauth-playground-api-other/config', 'similar names belong to host routing, never the playground API')
  const callback = await request('/callback?code=fixture-code&state=fixture-state', { headers: { Cookie: 'platform_session=private; __Host-oauth_playground=secure', Authorization: 'Bearer platform-fixture' } })
  assert.equal(callback.status, 200)
  assert.equal(callback.headers['referrer-policy'], 'no-referrer')
  assert.equal(staticRequests.at(-1).url, '/callback.html?code=fixture-code&state=fixture-state')
  assert.equal(staticRequests.at(-1).headers.cookie, undefined)
  assert.equal(staticRequests.at(-1).headers.authorization, undefined)
  assert.equal((await request('/callback', { method: 'HEAD' })).status, 200)
  assert.equal(staticRequests.at(-1).method, 'HEAD')
  assert.equal((await request('/sub/oauth-playground/assets/callback.js?v=1')).status, 200)
  assert.equal(staticRequests.at(-1).url, '/assets/callback.js?v=1')
  const redirect = await request('/sub/oauth-playground?from=fixture')
  assert.equal(redirect.status, 308)
  assert.equal(redirect.headers.location, `${origin}/sub/oauth-playground/?from=fixture`)
  for (const target of ['/callback', '/sub/oauth-playground', '/sub/oauth-playground/assets/callback.js']) {
    const count = staticRequests.length
    const denied = await request(target, { method: 'POST' })
    assert.equal(denied.status, 405)
    assert.equal(denied.headers.allow, 'GET, HEAD')
    assert.equal(staticRequests.length, count)
  }
  assert.ok(!readFileSync(`${root}/access.log`, 'utf8').includes('fixture-code'), 'gateway logging omits callback query strings')
  // The disabled config is tested through a real second Nginx start, so a broad
  // /oauth or host SPA location cannot accidentally take over these namespaces.
  nginx.kill()
  await once(nginx, 'exit')
  writeFileSync(file, nativeConfig({ ...deployment, oauthPlayground: null, oauthPlaygroundStatic: null }))
  const disabledNginx = spawn(process.env.PLATFORM_TEST_NGINX, ['-p', `${root}/`, '-c', file], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
  children.push(disabledNginx)
  disabledNginx.stderr.on('data', (chunk) => { diagnostics += chunk })
  const disabledDeadline = Date.now() + 5000
  while (true) {
    try { await request('/'); break } catch (error) {
      assert.equal(disabledNginx.exitCode, null, diagnostics)
      if (Date.now() >= disabledDeadline) throw error
      await new Promise((resolve) => setTimeout(resolve, 25))
    }
  }
  const counts = [apiRequests.length, staticRequests.length]
  for (const target of ['/oauth-playground-api', '/oauth-playground-api/config', '/callback?code=fixture-disabled', '/sub/oauth-playground', '/sub/oauth-playground/assets/callback.js']) {
    assert.equal((await request(target)).status, 404)
  }
  assert.deepEqual([apiRequests.length, staticRequests.length], counts)
})
