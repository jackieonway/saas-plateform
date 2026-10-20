import assert from 'node:assert/strict'
import test from 'node:test'
import { renderBusinessStatic, validateStaticInput } from '../render-static.mjs'
import { createStaticBuildPlan } from '../build-static.mjs'
import { renderGateway } from '../../gateway/render.mjs'

const hostOrigin = 'https://host.example.test:8443'
const input = { appCode: 'stores', version: '2.0.0', hostOrigin, apiBase: hostOrigin + '/stores-api' }

test('external static frontend explicitly permits its HTTPS API and host ancestor', () => {
  const config = renderBusinessStatic(input)
  assert.match(config, /connect-src 'self' https:\/\/host\.example\.test:8443;/)
  assert.match(config, /frame-ancestors https:\/\/host\.example\.test:8443/)
  assert.match(config, /Access-Control-Allow-Origin "\*" always/)
  assert.doesNotMatch(config, /X-Frame-Options|unsafe-eval|Access-Control-Allow-Credentials/)
})

test('the actual build-stage plan passes only the original four public fields to the renderer', () => {
  for (const appCode of ['work-orders', 'stores']) for (const apiBase of [`/${appCode}-api`, `${hostOrigin}/${appCode}-api`]) {
    const raw = { ...input, appCode, apiBase }
    const plan = createStaticBuildPlan(raw)
    assert.deepEqual(plan.args, ['scripts/build.mjs', '--version', raw.version, '--allowed-host-origin', raw.hostOrigin, '--api-base', raw.apiBase])
    assert.equal(plan.nginx, renderBusinessStatic(raw))
    assert.throws(() => createStaticBuildPlan({ ...raw, apiOrigin: hostOrigin }), /unknown configuration field: apiOrigin/)
  }
})

test('static service serves real version bytes with strict misses and writable state only in tmp', () => {
  const config = renderBusinessStatic(input)
  assert.match(config, /root \/usr\/share\/nginx\/html/)
  assert.match(config, /try_files \$uri \$uri\/index\.html =404/)
  assert.doesNotMatch(config, /try_files[^;]* \/index\.html/)
  assert.match(config, /pid "\/tmp\/nginx\.pid"/)
  assert.match(config, /application\/javascript mjs cjs/)
  assert.match(config, /default "no-store"/)
  assert.match(config, /max-age=31536000, immutable/)
  assert.match(config, /~\*\\\.html\$ "no-cache"/)
})

test('both applications and relative API versions share the same narrow contract', () => {
  for (const appCode of ['work-orders', 'stores']) {
    assert.equal(validateStaticInput({ ...input, appCode, version: '1.0.0', apiBase: `/${appCode}-api` }).apiOrigin, hostOrigin)
    const gateway = renderGateway({ publicOrigin: hostOrigin, allowedServices: ['backend', 'frontend-web', `${appCode}-v1`], versionRoutes: [{ app: appCode, version: '1.0.0', upstream: `${appCode}-v1:8080` }] })
    assert.ok(gateway.includes(`location ^~ /sub-apps/${appCode}/versions/1.0.0/`))
    assert.ok(gateway.includes('proxy_pass http://$micro_version_0;'))
    assert.ok(gateway.includes('location /sub-apps/ { return 404; }'))
  }
})

test('untrusted public inputs cannot inject origins, paths or Nginx directives', () => {
  for (const mutation of [
    { appCode: '../stores' }, { version: '../1' }, { version: 'a..b' }, { hostOrigin: 'http://host.example.test' },
    { hostOrigin: 'https://host.example.test/' }, { hostOrigin: 'https://host.example.test;return 200' },
    { apiBase: '//untrusted.test/api' }, { apiBase: 'http://untrusted.test/api' }, { apiBase: 'https://user:pass@api.test/api' },
    { apiBase: '/a/../api' }, { apiBase: '/a/%2e%2e/api' }, { apiBase: '/api?secret=x' }, { apiBase: '/api#x' }, { apiBase: '/api;return 200' },
  ]) assert.throws(() => validateStaticInput({ ...input, ...mutation }))
  assert.throws(() => validateStaticInput({ ...input, secret: 'must-not-be-read' }))
})
