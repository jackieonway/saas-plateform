/** Final audit of current bytes, protected evidence, actual reports and owned cleanup. */
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFileSync, writeFileSync, existsSync } from 'node:fs'
import { createSourceManifest } from '../scripts/ci/provenance.mjs'
const read = p => JSON.parse(readFileSync(p))
const sha = bytes => createHash('sha256').update(bytes).digest('hex')
const source = createSourceManifest(process.cwd())
const prior = read('target/phase-29/prior-evidence.json')
for (const item of prior) assert.equal(sha(readFileSync('docs/validation/' + item.name)), item.sha256, item.name)
const productReports = {}
const after = read('target/phase-29/after/build.json')
assert.equal(sha(readFileSync('frontend/src/App.tsx')),after.sourceAppSha256)
assert.equal(sha(readFileSync('target/phase-29/source-before/App.tsx')),read('target/phase-29/before/build.json').sourceAppSha256)
const runtimeInventory = report => report.inventory.filter(item => /\.(?:js|css)$/.test(item.file)).map(item => [item.file, item.sha256]).sort((a,b) => a[0].localeCompare(b[0]))
for (const product of ['chromium', 'chrome', 'msedge']) {
  const report = read(`target/phase-29/final-host-${product}/host-browser-${product}.json`)
  assert.equal(report.count, 9); assert.equal(report.checks.length, 9); assert(report.checks.every(c => c.status === 'PASS'))
  const gate = read(`target/phase-29/gate-${product}/build.json`)
  assert.equal(gate.sourceAppSha256, after.sourceAppSha256)
  assert.deepEqual(runtimeInventory(gate), runtimeInventory(after), 'All production JS/CSS must match the measured artifact')
  const original = read(`target/phase-29/full-${product}-r1.json`)
  const retained = original.suites.filter(s => s.id !== 'phase29-host')
  assert.equal(retained.length, 10); assert.equal(retained.reduce((sum,s) => sum+s.pass,0), 200)
  assert(retained.every(s => s.exitCode === 0 && s.fail === 0 && s.skip === 0))
  productReports[product] = { newProductionChecks: report, retainedRegressionSuites: retained,
    originalWholeRunStatus: 'FAIL', originalWholeRunTotals: original.totals,
    replacement: 'Original phase29 test selector failure retained; current nine-check rerun replaces only that suite. Whole Chromium gate additionally rerun from scratch.' }
}
const full = read('target/phase-29/full-chromium-r2.json')
assert.deepEqual(full.totals, { pass: 209, fail: 0, skip: 0 }); assert.equal(full.suites.length, 11)
const linux = read('target/phase-29/linux-contracts-r3/result.json')
assert.equal(linux.exitCode, 0); assert.equal(linux.cleanupQueryExitCode,0); assert.equal(linux.remainingOwnedContainers, '')
const linuxSource = read('target/phase-29/linux-contracts-r3/source-manifest.json')
const frozen = new Map(linuxSource.files.map(file => [file.path,file.sha256]))
const differences = source.files.filter(file => frozen.get(file.path) !== file.sha256).map(file=>file.path)
const auditOnly = file => /^tests\/phase29-[^/]+\.mjs$/.test(file) || ['tests/browser/README.md','scripts/ci/README.md','deploy/static/README.md'].includes(file)
assert(differences.every(auditOnly), 'Executed Linux product/contracts must match current bytes: '+differences.join(', '))
const currentPaths = new Set(source.files.map(file=>file.path))
assert(linuxSource.files.every(file=>currentPaths.has(file.path)), 'Frozen source files must not disappear')
const linuxLog = readFileSync('target/phase-29/linux-contracts-r3/raw.log','utf8')
assert.match(linuxLog, /# pass 391\b/); assert.match(linuxLog, /# fail 0\b/); assert.match(linuxLog, /# skipped 0\b/)
const nginx = read('target/phase-29/nginx-r3/result.json'); assert.equal(nginx.cleanup,true); assert.equal(nginx.records.length,7)
const policy = readFileSync('target/phase-29/policies-final-r3.log','utf8')
assert.match(policy,/pass 55\b/); assert.match(policy,/fail 0\b/); assert.match(policy,/skipped 0\b/)
const lintLog = readFileSync('target/phase-29/lint-final.log','utf8')
const lintSummary = lintLog.match(/(\d+) problems \((\d+) errors, (\d+) warnings\)/)
assert(lintSummary); assert.equal(Number(lintSummary[2]),0)
const lint = { status:'PASS',errors:0,warnings:Number(lintSummary[3]),
  note:'ESLint exit 0 permits existing warnings; not a zero-warning result. No warning is in the Phase29 changed production files.' }
assert(!/src\\(?:components\\(?:RouteRecoveryBoundary|layout\\(?:AppLayout|CommandPalette))|pages\\system\\applications)\b/.test(lintLog))
assert.equal(readFileSync('target/phase-29/typecheck-final.log','utf8').trim(),'')
const perf = read('docs/validation/phase-29-performance.json')
assert.equal(perf.raw.before.samples.length,5);assert.equal(perf.raw.after.samples.length,5)
const input = { ...source, protectedPriorEvidenceFiles: prior.length, protectedPriorEvidenceVerified: true,
  baseline: { sourceFiles: ['App.tsx','AppLayout.tsx','CommandPalette.tsx','ApplicationsPage.tsx'].map(file => ({ file,
    sha256: sha(readFileSync('target/phase-29/source-before/' + file)) })),
    compiled: 'target/phase-29/before/dist', build: 'target/phase-29/before/build.json',
    meaning: 'Before production bytes and four edited source snapshots retained; no claim that final working-tree hash identifies baseline source.' } }
writeFileSync('docs/validation/phase-29-inputs.json', JSON.stringify(input,null,2)+'\n')
const logs = ['before/browser.log','after/browser-final.log','after-build-final.log','lint-final.log','typecheck-final.log','policies-final-r3.log',
  'full-chromium-r1.log','full-chrome-r1.log','full-msedge-r1.log','full-chromium-r2.log',
  'final-host-chromium.log','final-host-chrome.log','final-host-msedge.log','linux-contracts-r3/raw.log','nginx-launch-r3.log']
for (const file of logs) assert(existsSync('target/phase-29/'+file),file)
const result = { status:'PHASE29_VERIFIED', sourceRevision:source.sourceRevision,inputsSha256:source.inputsSha256,
  tools:{node:process.version,frontend:Object.fromEntries(['typescript','vite','react','react-dom','react-router-dom','eslint'].map(name=>[name,read(`frontend/node_modules/${name}/package.json`).version]))},
  fullChromium:full,products:productReports,linuxContracts: { ...linux, passed:391,failed:0,skipped:0,
    differenceFromFinalInput: differences, unchangedProductAndContractBytes:true }, nginx,
  staticPolicies:{passed:55,failed:0,skipped:0},lint,typescriptBuild:'PASS',protectedPriorEvidence:prior.length,
  performance:{ reductions:perf.reductions,budget:perf.budget },
  logs:logs.map(file=>({path:'target/phase-29/'+file,sha256:sha(readFileSync('target/phase-29/'+file))})),
  limits:['Browser APIs/notifications are isolated fixtures; no database or backend changes.',
    'Timing is controlled local Chromium HTTP/gzip, not production HTTPS or field INP.',
    'Chrome and Edge acceptance is the retained 200 groups plus final nine-check rerun; their original whole-run failures are not relabeled PASS.',
    'Rust/PG, full Linux fast/delivery, remote CI and production deployment not rerun or inherited.'] }
writeFileSync('docs/validation/phase-29-runtime.json',JSON.stringify(result,null,2)+'\n')
console.log(JSON.stringify({status:result.status,inputsSha256:source.inputsSha256,files:source.files.length,protectedPriorEvidence:prior.length}))
