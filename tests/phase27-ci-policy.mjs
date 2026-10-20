import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { frontendPackages, postgresSuites } from '../scripts/ci/manifest.mjs'
import { createSourceManifest } from '../scripts/ci/provenance.mjs'
import { createServicePlan, parseArgs } from '../scripts/dev.mjs'
import { renderGateway } from '../deploy/gateway/render.mjs'
import { render } from '../deploy/native/render.mjs'

const frontend='business-apps/stores/frontend'
const deploy='business-apps/stores/deploy'
const revision='a'.repeat(40)
function temporary(t) {
  const parent=realpathSync(os.tmpdir()),root=mkdtempSync(path.join(parent,'saas-phase27-ci-'))
  t.after(()=>{const resolved=realpathSync(root);assert.equal(path.dirname(resolved),parent);assert.match(path.basename(resolved),/^saas-phase27-ci-[A-Za-z0-9]{6}$/);rmSync(resolved,{recursive:true})})
  return root
}
function put(root,file,value='reviewed source') {const target=path.join(root,file);mkdirSync(path.dirname(target),{recursive:true});writeFileSync(target,value)}

test('stores has a locked independent frontend install/build/test and preserves the Rust build alias',()=>{
  assert.equal(frontendPackages.filter(file=>file===frontend).length,1)
  const pkg=JSON.parse(readFileSync(new URL('../package.json',import.meta.url),'utf8'))
  assert.equal(pkg.scripts['build:stores'],'cargo build --locked --manifest-path business-apps/stores/backend/Cargo.toml --bin stores-api')
  assert.equal(pkg.scripts['build:stores:frontend'],`npm --prefix ${frontend} run build`)
  assert.equal(pkg.scripts['test:stores:frontend'],`npm --prefix ${frontend} test`)
  assert.match(pkg.scripts['build:examples'],/npm run build:stores:frontend/)
  const delivery=readFileSync(new URL('./vite-example-delivery.mjs',import.meta.url),'utf8')
  assert.match(delivery,/STORES_ALLOWED_HOST_ORIGINS: 'https:\/\/platform\.example\.com'/)
  assert.match(delivery,/'business-apps\/stores\/frontend'/)
  const runner=readFileSync(new URL('../scripts/ci/run.mjs',import.meta.url),'utf8')
  assert.match(runner,/step\('stores-frontend-contracts',process\.execPath,\[npm,'--prefix','business-apps\/stores\/frontend','test','--','--test-reporter=tap'\],\{format:'tap'/)
  for(const gate of ['fast','browser']) {
    const branch=runner.split(`if (options.gate === '${gate}') {`)[1].split('} else if')[0]
    assert.match(branch,/'build-frontends',[^\n]*STORES_ALLOWED_HOST_ORIGINS:'https:\/\/platform\.example\.com'/)
  }
})

test('stores source snapshot retains complete typed SDK, public npmrc and exact reviewed manifests without runtime secrets',t=>{
  const root=temporary(t)
  const selected=[`${frontend}/package.json`,`${frontend}/package-lock.json`,`${frontend}/.npmrc`,`${frontend}/vite.config.mts`,`${frontend}/src/main.ts`,
    ...['index.mjs','index.d.mts','types.d.ts','http.d.mts','capabilities.d.mts','protocol.mjs','protocol.d.mts','artifact-manifest.json'].map(file=>`${frontend}/vendor/micro-sdk/${file}`),
    ...['bootstrap.manifest.json','bootstrap.schema.json','grants.manifest.example.json','grants.schema.json'].map(file=>`${deploy}/${file}`),
    'tests/phase27-stores-browser.mjs','tests/helpers/phase27-stores-services.mjs']
  for(const file of selected)put(root,file,file.endsWith('.npmrc')?'engine-strict=true\n':'reviewed source')
  for(const file of [`${frontend}/.env`,`${frontend}/private/session.json`,`${frontend}/node_modules/cached.js`,`${frontend}/dist/assets/main.js`,`${frontend}/nested/.npmrc`,`${deploy}/runtime.json`,`${deploy}/other.json`,`${deploy}/.env`])put(root,file,'private or unreviewed')
  assert.deepEqual(createSourceManifest(root,{sourceRevision:revision}).files.map(file=>file.path).sort(),[...selected].sort())
  put(root,`${frontend}/.npmrc`,'engine-strict=true\n//registry.invalid/:_authToken=private\n')
  assert.throws(()=>createSourceManifest(root,{sourceRevision:revision}),/Only public engine-strict/)
})

test('stores dev is explicit, dependency local and never starts a database or backend implicitly',t=>{
  const root=temporary(t)
  put(root,`${frontend}/package-lock.json`,'{}');put(root,`${frontend}/node_modules/vite/bin/vite.js`,'')
  const services=createServicePlan(parseArgs(['--apps','stores','--stores-port','4309']),root)
  assert.equal(services.length,1)
  const [service]=services
  assert.equal(service.name,'stores');assert.equal(service.port,4309);assert.equal(service.cwd,path.join(root,frontend));assert.equal(service.command,process.execPath)
  assert.equal(service.args[0],path.join(root,frontend,'node_modules/vite/bin/vite.js'));assert.ok(service.args.includes('--strictPort'))
  assert.deepEqual(parseArgs([]).apps,['host','vue','jquery'])
  assert.throws(()=>parseArgs(['--apps','host,stores','--stores-port','3000']),/distinct ports/)
})

test('stores real browser uses a separate owned database gate and retains the original platform and fixture selections',()=>{
  const runner=readFileSync(new URL('../scripts/ci/run.mjs',import.meta.url),'utf8')
  const browser=runner.split("} else if (options.gate === 'browser') {")[1].split('} else if')[0]
  for(const id of ['browser','notification-browser-live','data-grant-ui-live','work-orders-browser-live','stores-browser-live'])assert.equal((browser.match(new RegExp("step\\('"+id+"'",'g'))||[]).length,1)
  assert.match(browser,/'stores-browser-live',[^\n]*format:'stores-browser'[^\n]*expectedProduct:options.browser,env:liveEnv/)
  assert.match(browser,/'stores-browser-live',[^\n]*expectedSourceRevision:source.sourceRevision/)
  assert.match(browser,/'stores-business-build','cargo',\['build','--locked','--manifest-path','business-apps\/stores\/backend\/Cargo.toml','--bin','stores-api','--target-dir','target\/ci-rust','-j','2'\]/)
  assert.match(browser,/'stores-platform-admin-build','cargo',\['build','--locked','--manifest-path','tools\/platform-admin\/Cargo.toml','--bin','saas-platform-admin','--target-dir','target\/phase25-platform-admin','-j','2'\]/)
  for(const id of ['work-orders-platform-build','work-orders-business-build','stores-business-build','stores-platform-admin-build'])assert.ok(browser.indexOf(`step('${id}'`)<browser.indexOf("step('stores-browser-live'"))
  const fixture=readFileSync(new URL('./browser/manifest.mjs',import.meta.url),'utf8')
  assert.ok(!fixture.includes('phase27-stores-browser'))
  assert.equal(postgresSuites.length,12);assert.equal(postgresSuites.reduce((total,suite)=>total+suite.expected,0),76)
  const workflow=readFileSync(new URL('../.github/workflows/production-build.yml',import.meta.url),'utf8')
  assert.match(workflow,/reports\/browser\/stores\/result\.json/)
  assert.match(workflow,/npm ci --prefix business-apps\/stores\/frontend/)
})

test('stores API proxy is private opt-in, preserves Rust CORS and blocks metrics before business routes',()=>{
  const gateway={publicOrigin:'https://platform.example.test'}
  const native={...gateway,listener:'127.0.0.1:8443',instanceRoot:'/srv/platform',hostRoot:'/srv/platform/frontend',microRoot:'/srv/platform/micro-apps',certificate:'/srv/platform/cert.pem',certificateKey:'/srv/platform/key.pem',backend:'127.0.0.1:3001'}
  for(const config of [renderGateway({...gateway,stores:'stores-api:3014'}),render({...native,stores:'127.0.0.1:3014'})['nginx.conf']]) {
    assert.match(config,/set \$stores "(?:stores-api|127\.0\.0\.1):3014";/)
    const start=config.indexOf('location = /stores-api {'),end=config.indexOf('location = /socket.io {',start),business=config.slice(start,end)
    assert.ok(start>=0&&end>start)
    assert.match(business,/proxy_pass http:\/\/\$stores\$request_uri;/)
    assert.doesNotMatch(business,/Access-Control|proxy_set_header (?:Origin|Authorization)/)
    assert.ok(config.indexOf('location = /stores-api/metrics { return 404; }')<start)
    assert.match(config,/application\/javascript mjs cjs;/)
  }
  assert.match(renderGateway(gateway),/location = \/stores-api \{ return 404; \}/)
  assert.throws(()=>renderGateway({...gateway,stores:'outside.invalid:3014'}),/allowlisted/)
  assert.throws(()=>render({...native,stores:'8.8.8.8:3014'})['nginx.conf'],/private/)
})
