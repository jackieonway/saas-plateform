import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { existsSync, lstatSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import { renderGateway, renderMicroStatic, validateTrustedProxyIps } from '../deploy/gateway/render.mjs'

const gateway = { publicOrigin: 'https://platform.example.test' }
const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

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
  assert.ok(!config.includes('unsafe-eval'))
  assert.ok(renderGateway({ ...gateway, trustedHtmlEval: true }).includes("script-src 'self' 'unsafe-eval'"))
  assert.match(config, /proxy_set_header Host "platform\.example\.test";/)
  assert.match(config, /return 308 https:\/\/platform\.example\.test\$request_uri;/)
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
  const rejected = spawnSync(process.execPath, [path.join(repository, 'deploy/gateway/render.mjs'), source, output], { encoding: 'utf8', timeout: 5000, windowsHide: true })
  assert.notEqual(rejected.status, 0)
  assert.equal(existsSync(output), false)
  writeFileSync(source, JSON.stringify(gateway))
  const accepted = spawnSync(process.execPath, [path.join(repository, 'deploy/gateway/render.mjs'), source, output], { encoding: 'utf8', timeout: 5000, windowsHide: true })
  assert.equal(accepted.status, 0, accepted.stderr)
  assert.equal(readFileSync(output, 'utf8'), renderGateway(gateway))
})

test('ZIP CLI selects a static-only configuration and rejects unknown deployment kinds', (t) => {
  const root = directory(t), source = path.join(root, 'input.json'), output = path.join(root, 'nginx.conf')
  const run = () => spawnSync(process.execPath, [path.join(repository, 'deploy/gateway/render.mjs'), source, output], { encoding: 'utf8', timeout: 5000, windowsHide: true })
  writeFileSync(source, JSON.stringify({ kind: 'shell', ...gateway }))
  assert.notEqual(run().status, 0)
  assert.equal(existsSync(output), false)
  writeFileSync(source, JSON.stringify({ kind: 'micro-static', ...gateway }))
  const accepted = run()
  assert.equal(accepted.status, 0, accepted.stderr)
  assert.equal(readFileSync(output, 'utf8'), renderMicroStatic(gateway))
  assert.ok(!readFileSync(output, 'utf8').includes('proxy_pass'))
})
