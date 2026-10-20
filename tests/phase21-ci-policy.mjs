import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { ciUnitTests, frontendPackages } from '../scripts/ci/manifest.mjs'
import { createSourceManifest } from '../scripts/ci/provenance.mjs'
import { createServicePlan, parseArgs } from '../scripts/dev.mjs'

const frontend = 'business-apps/work-orders/frontend'
const revision = 'a'.repeat(40)

function temporary(t) {
  const parent = realpathSync(os.tmpdir())
  const root = mkdtempSync(path.join(parent, 'saas-phase21-ci-'))
  t.after(() => {
    const resolved = realpathSync(root)
    assert.equal(path.dirname(resolved), parent)
    assert.match(path.basename(resolved), /^saas-phase21-ci-[A-Za-z0-9]{6}$/)
    rmSync(resolved, { recursive: true })
  })
  return root
}

function put(root, file, text = 'reviewed source') {
  mkdirSync(path.dirname(path.join(root, file)), { recursive: true })
  writeFileSync(path.join(root, file), text)
}

test('required frontend installation and the aggregate build include the independent work-orders package', () => {
  assert.deepEqual(new Set(frontendPackages), new Set(['frontend', 'mini-services/micro-vue', 'mini-services/micro-jquery',
    'examples/websocket', 'mini-services/oauth-playground/frontend', frontend, 'business-apps/stores/frontend']))
  assert.equal(frontendPackages.filter(file => file === frontend).length, 1)
  const pkg = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'))
  assert.equal(pkg.scripts['build:work-orders'], 'npm --prefix business-apps/work-orders/frontend run build')
  assert.match(pkg.scripts['build:examples'], /npm run build:work-orders/)
  assert.ok(!pkg.scripts['build:work-orders'].includes('example.com'), 'deployment origin must remain explicit public build input')
})

test('the immutable source selector retains Vite entry, lock, public npm config and the real browser helper without private inputs', t => {
  const root = temporary(t)
  const included = [`${frontend}/package.json`, `${frontend}/package-lock.json`, `${frontend}/.npmrc`, `${frontend}/vite.config.mjs`,
    `${frontend}/src/main.mjs`, `${frontend}/src/lifecycle.mjs`, `${frontend}/src/app.mjs`, 'tests/phase21-work-orders-browser.mjs', 'tests/helpers/phase21-work-orders-services.mjs', 'scripts/ci/browser-source-ownership.mjs']
  for (const file of included) put(root, file, file.endsWith('.npmrc') ? 'engine-strict=true\n' : 'reviewed source')
  for (const file of [`${frontend}/.env`, `${frontend}/private/token.txt`, `${frontend}/dist/assets/app.js`]) put(root, file, 'private or generated')
  const manifest = createSourceManifest(root, { sourceRevision: revision })
  assert.deepEqual(manifest.files.map(file => file.path).sort(), [...included].sort())
  put(root, `${frontend}/.npmrc`, 'engine-strict=true\n//registry.invalid/:_authToken=private\n')
  assert.throws(() => createSourceManifest(root, { sourceRevision: revision }), /Only public engine-strict/)
})

test('work-orders dev registration is explicit and starts only its own locked Vite process', t => {
  const root = temporary(t)
  put(root, `${frontend}/package-lock.json`, '{}')
  put(root, `${frontend}/node_modules/vite/bin/vite.js`, '')
  const options = parseArgs(['--apps', 'work-orders', '--work-orders-port', '4308'])
  const [service, ...other] = createServicePlan(options, root)
  assert.deepEqual(other, [])
  assert.equal(service.command, process.execPath)
  assert.equal(service.name, 'work-orders')
  assert.equal(service.cwd, path.join(root, frontend))
  assert.equal(service.port, 4308)
  assert.ok(service.args.includes('--strictPort'))
  assert.ok(service.args[0].startsWith(path.join(root, frontend, 'node_modules/vite')))
  assert.deepEqual(parseArgs([]).apps, ['host', 'vue', 'jquery'])
  assert.throws(() => parseArgs(['--apps', 'host,work-orders', '--work-orders-port', '3000']), /distinct ports/)
})

test('real work-order browser gate is independent of fixtures and preserves all historical mandatory gates', () => {
  const runner = readFileSync(new URL('../scripts/ci/run.mjs', import.meta.url), 'utf8')
  const browser = runner.split("} else if (options.gate === 'browser') {")[1].split('} else if')[0]
  for (const id of ['browser', 'notification-browser-live', 'data-grant-ui-live', 'work-orders-browser-live']) assert.equal((browser.match(new RegExp("step\\('" + id + "'", 'g')) || []).length, 1)
  assert.match(browser, /'work-orders-browser-live',[^\n]*format:'work-orders-browser'[^\n]*expectedProduct:options.browser,env:liveEnv/)
  for (const bin of ['saas-platform-backend', 'work-orders-api']) assert.match(browser, new RegExp("'cargo',\\['build','--locked'[^\\n]*'--bin','" + bin + "','--target-dir','target/ci-rust','-j','2'"))
  assert.ok(browser.indexOf("step('work-orders-platform-build'") < browser.indexOf("step('work-orders-browser-live'"))
  assert.ok(browser.indexOf("step('work-orders-business-build'") < browser.indexOf("step('work-orders-browser-live'"))
  for (const gate of ['fast', 'browser']) {
    const branch = runner.split(`if (options.gate === '${gate}') {`)[1].split('} else if')[0]
    assert.match(branch, /'build-frontends',[^\n]*WORK_ORDERS_ALLOWED_HOST_ORIGINS:'https:\/\/platform\.example\.com'/)
  }
  assert.ok(ciUnitTests.includes('scripts/ci/work-orders-browser.test.mjs'))
  assert.match(runner, /'work-orders-vite-contracts',\['business-apps\/work-orders\/frontend\/tests\/build\.mjs','business-apps\/work-orders\/frontend\/tests\/static-delivery\.mjs','business-apps\/work-orders\/frontend\/tests\/lifecycle\.mjs'\]/)
  assert.ok(!readFileSync(new URL('./browser/manifest.mjs', import.meta.url), 'utf8').includes('phase21-work-orders-browser'), 'owned database acceptance cannot use the fixture runner environment')
})

test('owned browser tools use the fixed official production-compatible Nginx binary with its account and runtime libraries', () => {
  const dockerfile = readFileSync(new URL('../deploy/ci/Dockerfile', import.meta.url), 'utf8')
  assert.match(dockerfile, /^FROM nginx:1\.28\.0-bookworm@sha256:3b03dfd827515a07ccf4946000f0bbdd2a4fb6aca7499249fcb6b1f9726823d6 AS nginx-tools$/m)
  assert.match(dockerfile, /^COPY --from=nginx-tools \/usr\/sbin\/nginx \/usr\/sbin\/nginx$/m)
  assert.match(dockerfile, /getent passwd nginx \|\| useradd --system --user-group --no-create-home/)
  assert.match(dockerfile, /nginx -V && ldd \/usr\/sbin\/nginx/)
  const cachePreparation = dockerfile.indexOf('mkdir -p /var/cache/nginx')
  assert.ok(cachePreparation >= 0 && cachePreparation < dockerfile.indexOf('nginx -t -c /tmp/phase21-nginx-probe.conf'))
  assert.match(dockerfile, /nginx -t -c \/tmp\/phase21-nginx-probe\.conf/)
  assert.match(dockerfile, /apt-get install --yes --no-install-recommends nginx openssl libnss3-tools/)
})
