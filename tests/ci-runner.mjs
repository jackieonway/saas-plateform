import assert from 'node:assert/strict'
import test from 'node:test'
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { evaluate, redact, commandEnvironment } from '../scripts/ci/report.mjs'
import { createSourceManifest } from '../scripts/ci/provenance.mjs'
import {parseArgs} from '../scripts/ci/run.mjs'
import {contractTests,postgresSuites} from '../scripts/ci/manifest.mjs'
import {mandatorySuites} from './browser/manifest.mjs'
import { dataGrantUiChecks, parseDataGrantUiOutput } from '../scripts/ci/data-grant-ui-results.mjs'
import { liveUiEnvironment } from '../scripts/ci/data-grant-ui.mjs'

test('mandatory TAP requires executed tests and rejects skip, todo, failure and cancellation even with exit zero', () => {
  const tap = '# tests 3\n# pass 3\n# fail 0\n# cancelled 0\n# skipped 0\n# todo 0\n'
  assert.equal(evaluate({ format: 'tap', exitCode: 0, output: tap }).status, 'PASS')
  for (const output of ['', tap.replace('tests 3', 'tests 0'), tap.replace('skipped 0', 'skipped 1'), tap.replace('todo 0', 'todo 1'), tap.replace('fail 0', 'fail 1'), tap.replace('cancelled 0', 'cancelled 1')]) assert.equal(evaluate({ format: 'tap', exitCode: 0, output }).status, 'FAIL')
  assert.equal(evaluate({ format: 'tap', exitCode: 1, output: tap }).status, 'FAIL')
})

test('Rust admin TLS gate requires one passing TAP file and its complete nine-check cleanup and binary report', () => {
  const driver=readFileSync(new URL('./platform-admin-tls.mjs',import.meta.url),'utf8')
  const ids=[...driver.matchAll(/check\('([^']+)'\)/g)].map(match=>match[1])
  assert.equal(ids.length,9)
  assert.equal(new Set(ids).size,9)
  const output='# tests 1\n# pass 1\n# fail 0\n# cancelled 0\n# skipped 0\n# todo 0\n'
  const document={schemaVersion:1,status:'PASS',checks:ids.map(id=>({id,status:'PASS'})),passed:9,failed:0,skipped:0,privateTlsRemoved:true,binarySha256:'a'.repeat(64)}
  const verdict=(value,extra={})=>evaluate({format:'platform-admin-tls',exitCode:0,output,document:value,...extra})
  assert.equal(verdict(document).status,'PASS')
  assert.equal(verdict(document).counts.pass,1)
  for(const invalid of [
    undefined,null,{}, {...document,schemaVersion:undefined}, {...document,schemaVersion:'1'},
    {...document,status:'FAIL'}, {...document,checks:undefined}, {...document,checks:{}},
    {...document,checks:[]}, {...document,checks:document.checks.slice(1)},
    {...document,checks:[...document.checks.slice(1),document.checks[1]]},
    {...document,checks:[...document.checks,{id:'unexpected',status:'PASS'}]},
    {...document,checks:document.checks.map((check,index)=>index?check:{...check,id:'unexpected'})},
    ...['SKIP','NOT_RUN','FAIL'].map(status=>({...document,checks:document.checks.map((check,index)=>index?check:{...check,status})})),
    {...document,checks:[null,...document.checks.slice(1)]},
    {...document,passed:8}, {...document,passed:'9'}, {...document,failed:1}, {...document,skipped:1},
    {...document,privateTlsRemoved:false}, {...document,privateTlsRemoved:undefined},
    {...document,binarySha256:undefined}, {...document,binarySha256:['a'.repeat(64)]}, {...document,binarySha256:'a'.repeat(63)}, {...document,binarySha256:'g'.repeat(64)},
  ]) assert.equal(verdict(invalid).status,'FAIL')
  for(const malformed of ['',output.replace('tests 1','tests 0'),output.replace('pass 1','pass 0'),output.replace('tests 1','tests 2').replace('pass 1','pass 2'),output.replace('skipped 0','skipped 1'),output.replace('todo 0','todo 1'),output.replace('cancelled 0','cancelled 1'),output.replace('fail 0','fail 1')]) {
    assert.equal(verdict(document,{output:malformed}).status,'FAIL')
  }
  assert.equal(verdict(document,{exitCode:1}).status,'FAIL')
  assert.equal(verdict(document,{timedOut:true}).status,'FAIL')
})
test('Cargo selection requires tests, rejects ignored, and records filtered cases separately', () => {
  const output = 'test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 81 filtered out; finished in 0.1s\n'
  assert.deepEqual(evaluate({ format: 'cargo', exitCode: 0, output }).counts, { tests: 8, pass: 8, fail: 0, skip: 0, cancelled: 0, todo: 0, filtered: 81 })
  for (const replacement of ['0 passed', '1 ignored', '1 failed']) assert.equal(evaluate({ format: 'cargo', exitCode: 0, output: output.replace(replacement.includes('passed') ? '8 passed' : replacement.includes('ignored') ? '0 ignored' : '0 failed', replacement) }).status, 'FAIL')
})
test('structured browser/PG/release reports cannot pass missing suites or skipped mandatory tests', () => {
  assert.equal(evaluate({ format: 'browser', exitCode: 0, document: { totals: { pass: 2, fail: 0, skip: 0 }, browserVersion: '1', suites: [{ id: 'a', pass: 2, fail: 0, skip: 0, exitCode: 0 }] }, expectedSuites: ['a'] }).status, 'PASS')
  assert.equal(evaluate({ format: 'browser', exitCode: 0, document: { totals: { pass: 2, fail: 0, skip: 1 }, browserVersion: '1', suites: [] }, expectedSuites: ['a'] }).status, 'FAIL')
  assert.equal(evaluate({ format: 'release', exitCode: 0, document: { status: 'PASS', build: 'PASS', smoke: 'NOT_RUN', images: [] } }).status, 'FAIL')
})
test('command environment strips ambient runtime/build secrets and logs redact credentials', () => {
  const env = commandEnvironment({ PATH: '/bin', HOME: '/home/test', TEST_DATABASE_URL: 'private', JWT_SECRET: 'private', VITE_CLIENT_SECRET: 'private', NODE_OPTIONS: '--require private', NPM_TOKEN: 'private', CI_TEST_POSTGRES_OWNER: 'phase04-owned', PLAYWRIGHT_BROWSERS_PATH: '/opt/ci/browser/.browsers' })
  for (const key of ['TEST_DATABASE_URL', 'JWT_SECRET', 'VITE_CLIENT_SECRET', 'NODE_OPTIONS', 'NPM_TOKEN']) assert.equal(env[key], undefined)
  assert.equal(env.PLAYWRIGHT_BROWSERS_PATH, '/opt/ci/browser/.browsers')
  assert.equal(env.CARGO_BUILD_JOBS, '2')
  assert.doesNotMatch(redact('postgres://user:secret@db/name Bearer example-token JWT_SECRET=very-private'), /secret@|example-token|very-private/)
})

function liveBrowserOutput() {
  const requestIds = ['a', 'b', 'c', 'd'].map(value => `${value.repeat(8)}-${value.repeat(4)}-4${value.repeat(3)}-8${value.repeat(3)}-${value.repeat(12)}`)
  const browser = { schemaVersion: 1, product: 'chrome', browserVersion: '155.0.8059.84', playwrightVersion: '1.64.1',
    checks: dataGrantUiChecks.map(id => ({ id, status: 'PASS' })), passed: dataGrantUiChecks.length, failed: 0, skipped: 0, requestIds,
    workOrders: { selfCount: 1, orgCount: 2, revokedStatus: 403, sameToken: true, independentDatabase: true, outsideNeverReturned: true },
    screenshot: { path: '/reports/browser/data-grant-ui/management-preview.png', sha256: 'a'.repeat(64) } }
  const database = { auditRows: 4, requestIdsMatch: true, membershipUuidPreserved: true, grantRevoked: true, requestIds }
  return { browser, database, output: [
    `PHASE15_LIVE_RESULT ${JSON.stringify(browser)}`,
    `PHASE15_LIVE_DATABASE ${JSON.stringify(database)}`,
    'PHASE15_BUSINESS_CLEANUP assertions_ok=true database_removed=true pool_closed=true',
    'entity creation cleanup assertions_ok=true database_removed=true pool_closed=true',
    'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 182 filtered out',
  ].join('\n') }
}

test('live grant UI requires exactly one Rust test, every named browser check and matching database cleanup evidence', () => {
  const { browser, database, output } = liveBrowserOutput()
  const document = parseDataGrantUiOutput(output, { product: 'chrome' })
  assert.equal(evaluate({ format: 'data-grant-ui', exitCode: 0, document, expectedProduct: 'chrome' }).status, 'PASS')
  const invalid = [
    '', output + '\n' + output,
    output.replace('1 passed', '0 passed'), output.replace('0 ignored', '1 ignored'),
    output.replace('database_removed=true', 'database_removed=false'),
    output.replace(JSON.stringify(browser), JSON.stringify({ ...browser, product: 'chromium' })),
    output.replace(JSON.stringify(browser), JSON.stringify({ ...browser, checks: browser.checks.slice(1) })),
    output.replace(JSON.stringify(browser), JSON.stringify({ ...browser, checks: [...browser.checks.slice(1), browser.checks[1]] })),
    output.replace(JSON.stringify(browser), JSON.stringify({ ...browser, checks: browser.checks.map((check, index) => index ? check : { ...check, status: 'SKIP' }) })),
    output.replace(JSON.stringify(browser), JSON.stringify({ ...browser, browserVersion: '', playwrightVersion: 'unapproved' })),
    output.replace(JSON.stringify(database), JSON.stringify({ ...database, requestIds: [...database.requestIds].reverse() })),
    output.replace(JSON.stringify(database), JSON.stringify({ ...database, grantRevoked: false })),
    output.replace(JSON.stringify(database), JSON.stringify({ ...database, auditRows: 3 })),
    output.replace(JSON.stringify(browser), JSON.stringify({ ...browser, workOrders: undefined })),
    output.replace(JSON.stringify(browser), JSON.stringify({ ...browser, workOrders: { ...browser.workOrders, selfCount: 2 } })),
    output.replace(JSON.stringify(browser), JSON.stringify({ ...browser, workOrders: { ...browser.workOrders, orgCount: 3 } })),
    output.replace(JSON.stringify(browser), JSON.stringify({ ...browser, workOrders: { ...browser.workOrders, revokedStatus: 200 } })),
    output.replace(JSON.stringify(browser), JSON.stringify({ ...browser, workOrders: { ...browser.workOrders, sameToken: false } })),
    output.replace(JSON.stringify(browser), JSON.stringify({ ...browser, workOrders: { ...browser.workOrders, independentDatabase: false } })),
    output.replace(JSON.stringify(browser), JSON.stringify({ ...browser, workOrders: { ...browser.workOrders, outsideNeverReturned: false } })),
    output.replace('PHASE15_BUSINESS_CLEANUP assertions_ok=true database_removed=true pool_closed=true', ''),
    output.replace('PHASE15_BUSINESS_CLEANUP assertions_ok=true', 'PHASE15_BUSINESS_CLEANUP assertions_ok=false'),
    output.replace(JSON.stringify(browser), JSON.stringify({ ...browser, screenshot: undefined })),
    output.replace(JSON.stringify(browser), JSON.stringify({ ...browser, screenshot: { path: '/reports/../private/management-preview.png', sha256: 'a'.repeat(64) } })),
    output.replace(JSON.stringify(browser), JSON.stringify({ ...browser, screenshot: { ...browser.screenshot, sha256: 'unverified' } })),
  ]
  for (const malformed of invalid) assert.throws(() => parseDataGrantUiOutput(malformed, { product: 'chrome' }))
  assert.equal(evaluate({ format: 'data-grant-ui', exitCode: 0, document, expectedProduct: 'msedge' }).status, 'FAIL')
  assert.equal(evaluate({ format: 'data-grant-ui', exitCode: 1, document, expectedProduct: 'chrome' }).status, 'FAIL')
  assert.equal(evaluate({ format: 'data-grant-ui', exitCode: 0, document: { ...document, status: 'FAIL' }, expectedProduct: 'chrome' }).status, 'FAIL')
})

test('live UI alone receives the validated owned database while ambient production configuration is stripped', () => {
  const owned = { PATH: '/bin', TEST_DATABASE_URL: 'postgresql://ci:synthetic@ci-12345678-postgres:5432/postgres', CI_TEST_POSTGRES_OWNER: 'ci-12345678',
    DATABASE_URL: 'postgresql://private/production', JWT_SECRET: 'private', DATA_GRANT_POLICY: 'private', SAAS_TEST_BROWSER: 'msedge',
    NODE_OPTIONS: '--require private', SQLX_OFFLINE_DIR: 'private', PHASE15_SCREENSHOT_PATH: '/private/untrusted.png', PLAYWRIGHT_BROWSERS_PATH: '/opt/ci/browser/.browsers' }
  const env = liveUiEnvironment(owned, 'chrome', '/workspace')
  assert.equal(env.TEST_DATABASE_URL, owned.TEST_DATABASE_URL)
  assert.equal(env.CI_TEST_POSTGRES_OWNER, owned.CI_TEST_POSTGRES_OWNER)
  assert.equal(env.SAAS_TEST_BROWSER, 'chrome')
  for (const key of ['DATABASE_URL', 'JWT_SECRET', 'DATA_GRANT_POLICY', 'NODE_OPTIONS', 'SQLX_OFFLINE_DIR', 'PHASE15_SCREENSHOT_PATH']) assert.equal(env[key], undefined)
  for (const changed of [{ CI_TEST_POSTGRES_OWNER: undefined }, { TEST_DATABASE_URL: 'postgresql://ci:synthetic@production:5432/postgres' }]) assert.throws(() => liveUiEnvironment({ ...owned, ...changed }, 'chrome', '/workspace'))
  assert.throws(() => liveUiEnvironment(owned, 'unknown', '/workspace'))
  const runner = readFileSync(new URL('../scripts/ci/run.mjs', import.meta.url), 'utf8')
  const browser = runner.split("} else if (options.gate === 'browser') {")[1].split('} else if')[0]
  assert.ok(browser.indexOf("step('browser'") < browser.indexOf("const liveEnv"))
  assert.match(browser, /'data-grant-ui-live',[^\n]*format:'data-grant-ui'[^\n]*env:liveEnv/)
  assert.doesNotMatch(browser.split('const liveEnv')[0], /TEST_DATABASE_URL|CI_TEST_POSTGRES_OWNER/)
  assert.match(browser, /install\('business-apps\/work-orders\/backend'\)/)
})
test('ZIP delivery requires every real check and successful cleanup, rejecting missing or skipped work',()=>{
  const document={status:'PASS',checks:[{id:'upload',status:'PASS'},{id:'rollback',status:'PASS'}],cleanupErrors:[]}
  const evaluateZip=value=>evaluate({format:'zip-release',exitCode:0,document:value,expectedChecks:['upload','rollback']})
  assert.equal(evaluateZip(document).status,'PASS')
  for(const value of [{...document,checks:[]},{...document,checks:document.checks.slice(0,1)},{...document,checks:[document.checks[0],document.checks[0]]},{...document,checks:[document.checks[0],{id:'rollback',status:'SKIP'}]},{...document,status:'FAIL'},{...document,cleanupErrors:['owned network removal failed']},{...document,cleanupErrors:undefined}]) assert.equal(evaluateZip(value).status,'FAIL')
  assert.equal(evaluate({format:'zip-release',exitCode:0,document}).status,'FAIL')
})
test('PostgreSQL summary requires the complete named manifest including data-grant management',()=>{
  const document={status:'PASS',passed:postgresSuites.reduce((sum,suite)=>sum+suite.expected,0),failed:0,skipped:0,suites:postgresSuites.map(s=>({...s,status:'PASS',passed:s.expected,failed:0,skipped:0}))}
  assert.equal(evaluate({format:'postgres',exitCode:0,document}).status,'PASS')
  for(const value of [{...document,passed:21,suites:document.suites.slice(0,4)},{...document,passed:27,suites:document.suites.slice(0,5)},{...document,passed:37,suites:document.suites.slice(0,6)},{...document,passed:41,suites:document.suites.slice(0,7)},{...document,passed:46,suites:document.suites.slice(0,8)},{...document,passed:54,suites:document.suites.slice(0,9)},{...document,passed:64,suites:document.suites.slice(0,10)},{...document,passed:71,suites:document.suites.slice(0,11)},{...document,suites:[...document.suites.slice(0,4),document.suites[0]]},{...document,skipped:1}]) assert.equal(evaluate({format:'postgres',exitCode:0,document:value}).status,'FAIL')
})
test('optional engines are explicit browser runs; malformed or duplicate gate arguments fail',()=>{
  for(const browser of ['firefox','webkit']) assert.equal(parseArgs(['--gate','browser','--browser',browser,'--out-dir',path.join(os.tmpdir(),'new-ci-report')]).browser,browser)
  assert.throws(()=>parseArgs(['--gate','fast','--browser','webkit','--out-dir',path.join(os.tmpdir(),'new-ci-report')]))
  assert.throws(()=>parseArgs(['--gate','fast','--gate','browser','--out-dir',path.join(os.tmpdir(),'new-ci-report')]))
})
test('fast and browser check generated SDK drift before building; fast selects public protocol contracts',()=>{
  for(const file of ['tests/micro-context-runtime.mjs','tests/micro-sandbox-lifecycle.mjs','tests/micro-runtime-faults.mjs','tests/micro-iframe-protocol.mjs','tests/micro-sdk-client.mjs','tests/micro-sdk-generation.mjs','business-apps/work-orders/frontend/tests/client.mjs','business-apps/work-orders/frontend/tests/app.mjs','business-apps/work-orders/backend/tests/cors.test.mjs']) assert.ok(contractTests.includes(file), `mandatory contract missing: ${file}`)
  const frontend=JSON.parse(readFileSync(new URL('../frontend/package.json',import.meta.url),'utf8'))
  assert.ok(frontend.scripts['test:contracts'].split(/\s+/).includes('../tests/micro-runtime-faults.mjs'), 'local contracts must include the mandatory runtime-fault regression')
  for(const file of ['tests/micro-context-updates.mjs','tests/micro-iframe-protocol.mjs','tests/micro-sdk-client.mjs','business-apps/work-orders/frontend/tests/client.mjs','business-apps/work-orders/frontend/tests/app.mjs']) {
    assert.ok(contractTests.includes(file), `mandatory update contract missing: ${file}`)
    for(const script of ['test:contracts','test:micro']) assert.ok(frontend.scripts[script].split(/\s+/).includes(`../${file}`), `local ${script} must include update contract ${file}`)
  }
  const runner=readFileSync(new URL('../scripts/ci/run.mjs',import.meta.url),'utf8')
  for(const gate of ['fast','browser']) {
    const branch=runner.split(`if (options.gate === '${gate}') {`)[1].split('} else if')[0]
    assert.ok(branch.indexOf("'sdk-drift'") >= 0)
    assert.ok(branch.indexOf("'sdk-drift'") < branch.indexOf("'build-frontends'"))
    assert.match(branch, /'scripts\/sync-micro-sdk\.mjs','--check'\],\{dependency:true\}/)
  }
  assert.match(runner, /'rust-public-contract-units',[^\n]*'public_contract_tests'/)
  assert.match(runner, /'rust-oauth-issuance-units',[^\n]*'oauth_issuance::tests'/)
  assert.match(runner, /'rust-login-limit-units',[^\n]*'login_limits::tests'/)
  assert.match(runner, /'rust-password-units',[^\n]*'util::password::tests'/)
  assert.match(runner, /'rust-client-ip-units',[^\n]*'client_ip::tests'/)
})
test('source provenance uses actual bytes while excluding private and generated inputs', t => {
  const root = mkdtempSync(path.join(os.tmpdir(), 'saas-ci-provenance-'))
  t.after(() => { assert.equal(path.dirname(root), path.resolve(os.tmpdir())); assert.match(path.basename(root), /^saas-ci-provenance-/); rmSync(root, { recursive: true, force: true }) })
  mkdirSync(path.join(root, 'frontend/src'), { recursive: true }); mkdirSync(path.join(root, 'frontend/node_modules'), { recursive: true })
  writeFileSync(path.join(root, 'package.json'), '{}'); writeFileSync(path.join(root, 'frontend/src/app.ts'), 'export const a=1')
  writeFileSync(path.join(root, 'frontend/.env'), 'never-copy'); writeFileSync(path.join(root, 'frontend/node_modules/private'), 'never-copy')
  const a = createSourceManifest(root, { sourceRevision: 'a'.repeat(40) })
  assert.deepEqual(a.files.map(file => file.path), ['frontend/src/app.ts', 'package.json'])
  writeFileSync(path.join(root, 'frontend/src/app.ts'), 'export const a=2')
  const b = createSourceManifest(root, { sourceRevision: 'a'.repeat(40) })
  assert.notEqual(a.inputsSha256, b.inputsSha256); assert.equal(a.sourceRevision, b.sourceRevision)
  mkdirSync(path.join(root,'business-apps/work-orders/frontend/vendor'),{recursive:true})
  writeFileSync(path.join(root,'business-apps/work-orders/frontend/vendor/iframe-bridge.js'),'registered classic bridge')
  writeFileSync(path.join(root,'business-apps/work-orders/frontend/vendor/micro-sdk.mjs'),'registered esm client')
  mkdirSync(path.join(root,'business-apps/work-orders/frontend/private'),{recursive:true})
  writeFileSync(path.join(root,'business-apps/work-orders/frontend/private/token.txt'),'must not copy private work-orders files')
  mkdirSync(path.join(root,'business-apps/work-orders/frontend/src'),{recursive:true})
  mkdirSync(path.join(root,'business-apps/work-orders/backend'),{recursive:true})
  writeFileSync(path.join(root,'business-apps/work-orders/frontend/src/client.mjs'),'import registered SDK')
  writeFileSync(path.join(root,'business-apps/work-orders/backend/server.mjs'),'controlled trace and CORS implementation')
  writeFileSync(path.join(root,'business-apps/work-orders/backend/db.mjs'),'reviewed existing business database runtime for real UI acceptance')
  const sdk=createSourceManifest(root,{sourceRevision:'a'.repeat(40)})
  assert.deepEqual(sdk.files.filter(file=>file.path.startsWith('business-apps/')).map(file=>file.path),['business-apps/work-orders/backend/db.mjs','business-apps/work-orders/backend/server.mjs','business-apps/work-orders/frontend/src/client.mjs','business-apps/work-orders/frontend/vendor/iframe-bridge.js','business-apps/work-orders/frontend/vendor/micro-sdk.mjs'])
  writeFileSync(path.join(root,'.npmrc'),'//registry.npmjs.org/:_authToken=never-copy\n')
  assert.throws(()=>createSourceManifest(root,{sourceRevision:'a'.repeat(40)}),/Only public engine-strict/)
})

test('OAuth split source includes both deployable units without private configuration or built artifacts', t => {
  const root = mkdtempSync(path.join(os.tmpdir(), 'saas-ci-oauth-'))
  t.after(() => { assert.equal(path.dirname(root), path.resolve(os.tmpdir())); assert.match(path.basename(root), /^saas-ci-oauth-/); rmSync(root, { recursive: true, force: true }) })
  const app = 'mini-services/oauth-playground'
  for (const directory of ['backend/src', 'frontend/src', 'backend/target', 'frontend/dist', 'private']) mkdirSync(path.join(root, app, directory), { recursive: true })
  for (const [file, value] of Object.entries({ 'backend/Cargo.toml': '[package]', 'backend/src/lib.rs': 'pub fn app() {}', 'frontend/package-lock.json': '{}', 'frontend/src/main.js': 'render()', 'config.env.example': 'public example', 'backend/.env': 'private secret', 'backend/target/bin': 'compiled', 'frontend/dist/index.html': 'built', 'private/client.txt': 'private secret' })) writeFileSync(path.join(root, app, file), value)
  const manifest = createSourceManifest(root, { sourceRevision: 'a'.repeat(40) })
  assert.deepEqual(manifest.files.map(file => file.path), ['backend/Cargo.toml', 'backend/src/lib.rs', 'frontend/package-lock.json', 'frontend/src/main.js'].map(file => `${app}/${file}`))
})

test('Phase 9 authorization, navigation and creation regressions remain mandatory in formal and local contracts',()=>{
  const frontend=JSON.parse(readFileSync(new URL('../frontend/package.json',import.meta.url),'utf8'))
  for(const file of ['tests/auth-queries.mjs','tests/route-policy.mjs','tests/route-policy-guard.mjs','tests/application-navigation.mjs','business-apps/work-orders/frontend/tests/client.mjs','business-apps/work-orders/frontend/tests/app.mjs','tests/platform-admin-contract.mjs']) {
    assert.equal(contractTests.filter(selected=>selected===file).length,1,`mandatory Phase 9 contract missing or duplicated: ${file}`)
    for(const script of ['test:contracts','test:auth']) assert.equal(frontend.scripts[script]?.split(/\s+/).filter(selected=>selected===`../${file}`).length,1,`local ${script} must select Phase 9 contract once: ${file}`)
  }
  for(const [id,count] of [['micro-host',47],['notifications',10],['platform-settings',11],['regions',12],['entities',13],['route-permissions',64],['departments',11],['chat',1]]) {
    assert.equal(mandatorySuites.filter(suite=>suite.id===id&&suite.expectedCheckGroups===count).length,1,`existing browser suite must remain mandatory: ${id}`)
  }
  const added=mandatorySuites.filter(suite=>suite.id==='phase9-navigation')
  assert.equal(added.length,1);assert.equal(added[0].script,'tests/phase9-navigation-browser.mjs')
  assert.equal(added[0].expectedCheckGroups,14)
  assert.equal(frontend.scripts['test:auth:navigation:browser'],'node ../tests/phase9-navigation-browser.mjs')
})

test('work-orders snapshot retains reviewed deploy files and includes the existing business backend without private/generated data',t=>{
  const root=mkdtempSync(path.join(os.tmpdir(),'saas-ci-workorders-source-'))
  t.after(()=>{assert.equal(path.dirname(root),path.resolve(os.tmpdir()));assert.match(path.basename(root),/^saas-ci-workorders-source-/);rmSync(root,{recursive:true,force:true})})
  const reviewed=['bootstrap.manifest.json','grants.manifest.example.json']
  const relative='business-apps/work-orders/deploy/'
  const fixture='business-apps/work-orders/tests/platform-e2e.mjs'
  mkdirSync(path.dirname(path.join(root,fixture)),{recursive:true});writeFileSync(path.join(root,fixture),'reviewed fixture')
  writeFileSync(path.join(root,'business-apps/work-orders/tests/not-selected.mjs'),'excluded')
  for(const file of [...reviewed,'not-selected.mjs','.env','private/runtime.json']) {
    const absolute=path.join(root,relative,file);mkdirSync(path.dirname(absolute),{recursive:true});writeFileSync(absolute,'reviewed fixture')
  }
  const backendFiles = ['config.mjs', 'db.mjs', 'server.mjs', 'migrations/001-initial.sql', 'package.json', 'package-lock.json']
  for (const file of [...backendFiles, '.env', 'private/runtime.json', 'node_modules/cached.js']) {
    const absolute = path.join(root, 'business-apps/work-orders/backend', file)
    mkdirSync(path.dirname(absolute), { recursive: true });writeFileSync(absolute, 'reviewed fixture')
  }
  const manifest=createSourceManifest(root,{sourceRevision:'a'.repeat(40)})
  assert.deepEqual(manifest.files.map(file=>file.path),[fixture,...reviewed.map(file=>relative+file), ...backendFiles.map(file=>'business-apps/work-orders/backend/'+file)].sort((a,b)=>a.localeCompare(b,'en')))
})

test('Rust admin source snapshot retains its locked implementation and excludes runtime credentials and build output',t=>{
  const root=mkdtempSync(path.join(os.tmpdir(),'saas-ci-admin-source-'))
  t.after(()=>{assert.equal(path.dirname(root),path.resolve(os.tmpdir()));assert.match(path.basename(root),/^saas-ci-admin-source-/);rmSync(root,{recursive:true,force:true})})
  const base='tools/platform-admin/'
  const selected=['Cargo.toml','Cargo.lock','src/lib.rs','src/grants.rs','schemas/release.schema.json','manifests/bootstrap.manifest.json']
  for(const file of [...selected,'.env','private/session.json','target/debug/saas-platform-admin']){
    const absolute=path.join(root,base,file);mkdirSync(path.dirname(absolute),{recursive:true});writeFileSync(absolute,'source fixture')
  }
  const manifest=createSourceManifest(root,{sourceRevision:'a'.repeat(40)})
  assert.deepEqual(manifest.files.map(file=>file.path),selected.map(file=>base+file).sort((a,b)=>a.localeCompare(b,'en')))
})

test('fast builds the Rust admin binary before mandatory real TLS with isolated explicit environment and fresh reports',()=>{
  const runner=readFileSync(new URL('../scripts/ci/run.mjs',import.meta.url),'utf8')
  const fast=runner.split("if (options.gate === 'fast') {")[1].split('} else if')[0]
  assert.match(fast,/const adminTarget = 'target\/phase25-platform-admin'/)
  assert.match(fast,/step\('platform-admin-build','cargo',\['build','--locked','--manifest-path',adminManifest,'--bin','saas-platform-admin','--target-dir',adminTarget,'-j','2'\],\{dependency:true\}\)/)
  assert.ok(fast.indexOf("step('platform-admin-build'") < fast.indexOf("tap('platform-admin-tls'"))
  const tlsEnv=fast.split('const adminTlsEnv = ')[1].split('\n')[0]
  assert.match(tlsEnv,/\.\.\.env, PHASE25_CLI_BIN:path\.join\(root,adminTarget,'debug'/)
  assert.match(tlsEnv,/PHASE25_TLS_OUT:path\.join\(out,'platform-admin-tls'\)/)
  assert.doesNotMatch(tlsEnv,/process\.env|PLATFORM_ADMIN_BEARER|TEST_DATABASE_URL/)
  assert.match(fast,/tap\('platform-admin-tls',\['tests\/platform-admin-tls\.mjs'\],\{format:'platform-admin-tls',env:adminTlsEnv,report:path\.join\(out,'platform-admin-tls\/result\.json'\),timeout:2\*60_000\}\)/)
  assert.match(runner,/const tap = [^\n]*format:'tap',\.\.\.extra/)
  assert.match(runner,/env:item\.env\|\|env/)
})

test('production native acceptance requires the prebuilt Rust admin binary and normalizes the explicit CLI alias before fixture import',()=>{
  const source=readFileSync(new URL('./production-native-e2e.mjs',import.meta.url),'utf8')
  const alias="if(!process.env.PLATFORM_ADMIN_BIN && process.env.PHASE25_CLI_BIN)process.env.PLATFORM_ADMIN_BIN=process.env.PHASE25_CLI_BIN"
  assert.ok(source.indexOf(alias) >= 0)
  assert.ok(source.indexOf(alias) < source.indexOf("await import('../business-apps/work-orders/tests/platform-e2e.mjs')"))
  assert.match(source,/const required=\[[^\n]*'PLATFORM_ADMIN_BIN'/)
  const fixture=readFileSync(new URL('../business-apps/work-orders/tests/platform-e2e.mjs',import.meta.url),'utf8')
  assert.match(fixture,/process\.env\.PLATFORM_ADMIN_BIN \?\? process\.env\.PHASE25_CLI_BIN/)
  assert.match(fixture,/path\.isAbsolute\(adminBinaryArgument\)/)
  assert.doesNotMatch(fixture,/import\(['"]\.\.\/deploy\/platform-admin-core\.mjs['"]\)/)
})

test('Phase 10 shared-session regressions remain mandatory in formal and frontend-local entries',()=>{
  const frontend=JSON.parse(readFileSync(new URL('../frontend/package.json',import.meta.url),'utf8'))
  const sessions=['tests/session-refresh.mjs','tests/session-coordinator.mjs','tests/session-integration.mjs']
  for(const file of sessions){
    assert.equal(contractTests.filter(selected=>selected===file).length,1,`mandatory session contract missing or duplicated: ${file}`)
    for(const script of ['test:contracts','test:auth','test:auth:refresh']){
      assert.match(frontend.scripts[script],/^node --test /,`local ${script} must be a frontend-relative Node test entry`)
      assert.equal(frontend.scripts[script].split(/\s+/).filter(selected=>selected===`../${file}`).length,1,`local ${script} must select session contract once: ${file}`)
    }
  }
  const added=mandatorySuites.filter(suite=>suite.id==='phase10-session')
  assert.equal(added.length,1);assert.equal(added[0].script,'tests/phase10-session-browser.mjs')
  assert.equal(added[0].expectedCheckGroups,17)
  assert.equal(mandatorySuites.length,11)
  assert.equal(mandatorySuites.reduce((total,suite)=>total+suite.expectedCheckGroups,0),209)
  assert.deepEqual(mandatorySuites.find(suite=>suite.id==='phase29-host'),
    {id:'phase29-host',script:'tests/phase29-browser-gate.mjs',expectedCheckGroups:9})
  assert.equal(frontend.scripts['test:auth:session:browser'],'node ../tests/phase10-session-browser.mjs')
})
