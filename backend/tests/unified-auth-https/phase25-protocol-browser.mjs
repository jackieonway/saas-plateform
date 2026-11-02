/** Current-cluster protocol matrix. Captures callback navigation only; issuer responses are real. */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { execFileSync, spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { closeBusinessBrowser } from './phase25-business-http.mjs';
import { completedProtocolCheck, rewriteProtocolReport } from './phase25-protocol-report.mjs';

const TASK = '/tmp/rust-saas-unified-auth-20261028', AUTH = 'https://auth.saas.test', RP = 'https://customer.saas.test:24446';
const here = path.dirname(fileURLToPath(import.meta.url)), repository = path.resolve(here, '../../..');
const args = {};
for (let index = 2; index < process.argv.length; index += 2) {
  assert(['--snapshot', '--output'].includes(process.argv[index]) && process.argv[index + 1]
    && !Object.hasOwn(args, process.argv[index]), 'PHASE25_PROTOCOL_ARGUMENTS'); args[process.argv[index]] = process.argv[index + 1];
}
const label = args['--snapshot']; assert(/^phase25-tools-r[1-9][0-9]*$/.test(label ?? ''), 'PHASE25_PROTOCOL_LABEL');
const output = path.resolve(args['--output'] ?? '');
assert(path.dirname(output) === path.join(repository, 'backend/docs/designs/unified-auth/validation')
  && /^phase-25-protocol-[a-z0-9-]+\.json$/.test(path.basename(output)), 'PHASE25_PROTOCOL_OUTPUT');
const fd = fs.openSync(output, 'wx', 0o600), canaries = new Set(), contexts = [], tls = new Set(), observations = new Set();
const report = { contractVersion: 'unified-auth-phase25-current-protocol/1', startedAtUtc: new Date().toISOString(), result: 'FAIL',
  snapshot: label, checks: [], tlsErrorsIgnored: false, issuerResponsesIntercepted: false,
  callbackNavigationCapturedBeforeRP: true, rawCarriersPersisted: false, driverHasDatabaseAccess: false,
  coverageLimitations: [
    'Scenario IDs identify related evidence, not automatic whole-row completion. Other matrix assertions remain separately required.',
    'Wrong ID Token audience uses the actual signed identity response with a deliberately different expected audience in jose; no issuer response is forged or intercepted.',
    'Subject stability repeats the same client. Same-sector different-client and different-partner/environment subject comparisons are not claimed.',
    'Retired-entry checks cover two actual identity /oidc routes. Other retired registry/API paths require their own evidence.',
    'Callback navigation is captured before the RP handler only. Authorization, token, JWKS and UserInfo responses come from the actual current services.',
  ],
  unverifiedScenarios: ['PUBLIC_CLIENT_NO_SECRET', 'PUBLIC_CLIENT_TOKEN_AUTH_NEGATIVES', 'ACCOUNT_RENAME_AND_RETIREMENT',
    'TEST_PRODUCTION_SUBJECT_ISOLATION', 'CLIENT_CREDENTIAL_ROTATION_AND_EMERGENCY_REVOCATION', 'REFRESH_ABSOLUTE_EXPIRY'] };
let stage = 'OWNED_PREFLIGHT', browser, child, ready = false, driverFailed = false, sequence = 0, account, authWindow, interrupted = false;
const pending = new Map();
function interrupt() { interrupted = true; child?.stdin.end(); void browser?.close().catch(() => {}); }
process.once('SIGINT', interrupt); process.once('SIGTERM', interrupt);
const hash = value => createHash('sha256').update(value).digest('hex');
function remember(value) {
  if (typeof value === 'string' && value.length >= 8) canaries.add(value);
}
function persist() {
  const bytes = JSON.stringify(report, null, 2) + '\n';
  assert(!/"(?:access_token|refresh_token|id_token|clientSecret|password|cookie|verifier|nonce)"\s*:/.test(bytes), 'PHASE25_PROTOCOL_REPORT_FIELDS');
  for (const value of canaries) assert(!bytes.includes(value), 'PHASE25_PROTOCOL_REPORT_VALUES');
  rewriteProtocolReport(fd, bytes);
}
function pass(scenarioIds, facts) { report.checks.push(completedProtocolCheck(stage, scenarioIds, facts)); persist(); console.log('PHASE25_PROTOCOL_CHECK ' + stage + ' PASS'); }
function wsl(program) {
  try { return execFileSync('wsl.exe', ['-d', 'Ubuntu', '-u', 'root', '--exec', 'python3', '-B', '-c', program, TASK, label], {
    encoding: 'utf8', windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'], timeout: 45000, maxBuffer: 8 * 1024 * 1024,
  }); } catch { throw new Error('PHASE25_PROTOCOL_OWNED_PREFLIGHT_FAILED'); }
}
const preflight = String.raw`
import hashlib,json,pathlib,subprocess,sys,os,stat
t=pathlib.Path(sys.argv[1]);label=sys.argv[2];s=t/('source-snapshot-'+label);sys.path.insert(0,str(s/'saas-plateform/backend/tests/unified-auth-https'))
import phase23_cluster_binding as b
bound=b.load_binding(t);uid=bound['taskUid']
def read(p):return b.protected_read(p,uid=uid,mode=0o600,maximum=8*1024*1024)[0]
m=json.loads(read(t/('snapshot-'+label+'.json')));assert m['includeAllProjects'] and m['completeFrontendOutputs']
assert m['snapshotSha256']==hashlib.sha256(json.dumps(m['inputs'],sort_keys=True).encode()).hexdigest()
files=['phase25-protocol-browser.mjs','phase25-protocol-driver.mjs','phase25-protocol-cases.mjs','phase25-protocol-report.mjs','phase25-business-http.mjs','phase23-protocol.mjs','phase23-protocol-diagnostics.mjs']
digests={}
for name in files:
 relative='saas-plateform/backend/tests/unified-auth-https/'+name;digest=m['inputs'][relative];assert hashlib.sha256(read(s/relative)).hexdigest()==digest;digests[name]=digest
e=json.loads(read(t/'https-environment.json'));d=json.loads(read(t/'https-database.json'));state=json.loads(read(t/'phase23-environment-state.json'))
assert state['mode']=='SOURCE' and state['prepared'] and not state['sourceQuarantined'] and state['snapshot']==label
assert d['clusterId']==bound['systemIdentifier'];ids=e['phase21']['ids']
literal=lambda x:"'"+str(x).replace("'","''")+"'"
query="SELECT "+b.sql_guard(bound)+" AND (SELECT shobj_description(oid,'pg_database') FROM pg_database WHERE datname=current_database())="+literal('identity-deployment/1/'+e['ids']['deployment'])
query+=" AND NOT EXISTS(SELECT 1 FROM memberships WHERE user_id="+literal(ids['user'])+"::uuid)"
query+=" AND (SELECT count(*) FROM auth_clients WHERE id=ANY(ARRAY["+','.join(literal(ids[x])+'::uuid' for x in ['thirdClient','offlineClient'])+"]))=2"
query+=" AND (SELECT count(DISTINCT application_id) FROM auth_clients WHERE id=ANY(ARRAY["+','.join(literal(ids[x])+'::uuid' for x in ['machineClient','resourceClient'])+"]))=1;"
r=subprocess.run(['/usr/lib/postgresql/18/bin/psql','-X','-qAt','-v','ON_ERROR_STOP=1','-h','127.0.0.1','-p','55494','-U','postgres','-d',d['database']],input=query.encode(),capture_output=True,timeout=15,env={'PATH':'/usr/bin:/bin','PGOPTIONS':'-c log_statement=none -c log_min_error_statement=panic -c default_transaction_read_only=on'})
assert r.returncode==0 and r.stdout.strip()==b't'
n=t/'namespace.pid';details=n.lstat();assert stat.S_ISREG(details.st_mode) and details.st_uid==0 and details.st_nlink==1
ns=json.loads(n.read_text());p=pathlib.Path('/proc')/str(ns['pid']);assert p.stat().st_uid==0 and p.joinpath('stat').read_text().rsplit(')',1)[1].split()[19]==ns['startTime'] and os.readlink(p/'ns/mnt')==ns['namespace']==t.joinpath('namespace.identity').read_text().strip()
print(json.dumps({'account':e['phase21']['accounts']['user'],'namespacePid':ns['pid'],'uid':bound['taskUid'],'gid':bound['taskGid'],'snapshotSha256':m['snapshotSha256'],'digests':digests,'zeroMemberships':True,'sharedCanonicalRegistry':True,'machineClientsSameApplication':True}))
`;
const wait = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
async function until(test, failure, maximum = 20000) {
  const deadline = Date.now() + maximum;
  while (!test()) { assert(Date.now() < deadline, failure); await wait(80); }
}
function rpc(command, values = {}) {
  return new Promise((resolve, reject) => {
    const id = ++sequence, timer = setTimeout(() => { pending.delete(id); reject(new Error('PHASE25_PROTOCOL_RPC_TIMEOUT')); }, 35000);
    pending.set(id, { resolve: value => { clearTimeout(timer); if (!value.ok) {
      const error = new Error(/^PHASE25_PROTOCOL_[A-Z_]+$/.test(value.code ?? '') ? value.code : 'PHASE25_PROTOCOL_DRIVER_REJECTED');
      if (/^[A-Z_0-9]{1,80}$/.test(value.sdkCode ?? '')) error.sdkCode = value.sdkCode; reject(error);
    } else resolve(value.result); }, reject: error => { clearTimeout(timer); reject(error); } });
    child.stdin.write(JSON.stringify({ id, command, ...values }) + '\n');
  });
}
async function authorize(page, mutation) {
  const start = await rpc('begin', { profile: 'third', ...(mutation ? { mutation } : {}) }); remember(start.key); remember(start.authorizationUrl);
  let callback;
  const pattern = RP + '/auth/callback?*';
  await page.route(pattern, async route => {
    callback = route.request().url(); remember(callback);
    const url = new URL(callback); for (const value of url.searchParams.values()) remember(value);
    await route.fulfill({ status: 200, contentType: 'text/plain', body: 'Owned standard-client callback captured.' });
  });
  try {
    const response = await page.goto(start.authorizationUrl, { waitUntil: 'domcontentloaded' });
    if (mutation) {
      assert.equal(response.status(), 400, 'PHASE25_PROTOCOL_AUTHORIZE_NEGATIVE_STATUS');
      assert(!callback && new URL(page.url()).origin === AUTH, 'PHASE25_PROTOCOL_AUTHORIZE_NEGATIVE_CALLBACK');
      return { denied: true, httpStatus: 400 };
    }
    await page.waitForFunction(dest => location.origin === dest || !!document.querySelector('#login-form:not([hidden])')
      || document.querySelector('#auth-content')?.dataset.screen === 'CONSENT_REQUIRED'
      || !!document.querySelector('#error-message:not([hidden])'), RP);
    if (new URL(page.url()).origin === AUTH && await page.locator('#login-form:not([hidden])').isVisible()) {
      const before = Date.now(); await page.locator('#username').fill(account.username); await page.locator('#password').fill(account.password);
      const accepted = page.waitForResponse(value => value.url() === AUTH + '/api/sso/login' && value.request().method() === 'POST');
      await page.locator('#login-submit').click(); assert.equal((await accepted).status(), 200, 'PHASE25_PROTOCOL_PRIMARY_LOGIN');
      authWindow = [before, Date.now()];
    }
    for (let count = 0; count < 8; count++) {
      const state = await page.waitForFunction(dest => {
        if (location.origin === dest) return 'callback';
        if (document.querySelector('#error-message:not([hidden])')) return 'error';
        if (document.querySelector('#auth-content')?.dataset.screen === 'CONTEXT_SELECTION_REQUIRED') return 'context';
        if (document.querySelector('#auth-content')?.dataset.screen === 'CONSENT_REQUIRED'
          && !document.querySelector('#consent-panel')?.hidden && !document.querySelector('#consent-allow')?.disabled) return 'consent';
        return false;
      }, RP);
      const next = await state.jsonValue(); await state.dispose();
      assert(!['error', 'context'].includes(next), 'PHASE25_PROTOCOL_ACCOUNT_ONLY_REJECTED_OR_CONTEXT');
      if (next === 'callback') break; await page.locator('#consent-allow').click();
    }
    await page.waitForURL(url => url.origin === RP); assert(callback && authWindow, 'PHASE25_PROTOCOL_CALLBACK_OR_PRIMARY_WINDOW');
    return { key: start.key, callback, authWindow };
  } finally { await page.unroute(pattern); }
}

try {
  const checked = JSON.parse(wsl(preflight)); account = checked.account; remember(account.username); remember(account.password);
  for (const name of ['phase25-protocol-browser.mjs', 'phase25-business-http.mjs', 'phase25-protocol-report.mjs']) {
    assert.equal(hash(fs.readFileSync(path.join(here, name))), checked.digests[name], 'PHASE25_PROTOCOL_LIVE_INPUT_CHANGED');
  }
  report.snapshotSha256 = checked.snapshotSha256; report.sourceSha256 = checked.digests;
  pass([], { sourceClusterBound: true, zeroMemberships: true, sharedCanonicalRegistry: true, machineClientsSameApplication: true });
  const tools = TASK + '/source-snapshot-' + label + '/saas-plateform/backend/tests/unified-auth-https';
  child = spawn('wsl.exe', ['-d', 'Ubuntu', '-u', 'root', '--exec', 'nsenter', '--mount', '--target', String(checked.namespacePid),
    'setpriv', '--reuid=' + checked.uid, '--regid=' + checked.gid, '--clear-groups', '--pdeathsig', 'TERM',
    TASK + '/phase21-node-runtime/bin/node', tools + '/phase25-protocol-driver.mjs', '--snapshot', label], {
    windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'],
  });
  child.once('error', () => { driverFailed = true; }); child.stderr.on('data', () => { driverFailed = true; });
  child.once('exit', () => { driverFailed = true; for (const operation of pending.values()) operation.reject(new Error('PHASE25_PROTOCOL_DRIVER_EXIT')); pending.clear(); });
  createInterface({ input: child.stdout }).on('line', line => {
    try {
      assert(line.length <= 65536); const value = JSON.parse(line);
      if (value.ready) { report.standardClient = value.standardClient; report.jwtLibrary = value.jwtLibrary; ready = true; }
      else { const operation = pending.get(value.id); if (operation) { pending.delete(value.id); operation.resolve(value); } }
    } catch { driverFailed = true; }
  });
  await until(() => ready || driverFailed, 'PHASE25_PROTOCOL_DRIVER_START'); assert(!driverFailed, 'PHASE25_PROTOCOL_DRIVER_START');
  process.env.PLAYWRIGHT_BROWSERS_PATH = path.join(repository, 'frontend/tests/browser/.browsers');
  const { chromium } = createRequire(path.join(repository, 'frontend/tests/browser/package.json'))('playwright');
  browser = await chromium.launch({ headless: true, proxy: { server: 'http://127.0.0.1:24445' } });
  report.browserVersion = browser.version(); const context = await browser.newContext({ serviceWorkers: 'block' }); contexts.push(context);
  const page = await context.newPage(); page.setDefaultTimeout(30000); page.setDefaultNavigationTimeout(40000);
  page.on('response', response => {
    const operation = response.securityDetails().then(value => { if (value?.protocol) tls.add(value.protocol); }).catch(() => {});
    observations.add(operation); operation.finally(() => observations.delete(operation));
  });
  stage = 'CANONICAL_DISCOVERY_AND_RETIRED_ENTRYPOINTS';
  const discoveryResponse = await page.goto(AUTH + '/.well-known/openid-configuration');
  assert.equal(discoveryResponse.status(), 200); const discovery = await page.evaluate(() => JSON.parse(document.body.textContent));
  assert.equal(discovery.issuer, AUTH); assert.deepEqual(discovery.code_challenge_methods_supported, ['S256']);
  for (const route of ['/oidc/authorize', '/oidc/token']) assert.equal((await page.goto(AUTH + route)).status(), 404, 'PHASE25_PROTOCOL_OLD_ENTRYPOINT');
  pass(['P25-C01-S04'], { exactIssuer: true, s256Only: true, oldIdentityRoutes404: 2 });
  stage = 'ACTUAL_STANDARD_CLAIMS_PRIMARY_AUTH_TIME_AND_ZERO_MEMBERSHIP';
  const initial = await rpc('finish', await authorize(page)); remember(initial.key);
  const initialFacts = { ...initial }; delete initialFacts.key;
  pass(['P25-C02-S01', 'P25-C02-S02', 'P25-C03-S01', 'P25-C03-S02', 'P25-C03-S03', 'P25-C03-S04', 'P25-C03-S05', 'P25-C03-S06', 'P25-C03-S10'], initialFacts);
  stage = 'ACTUAL_CODE_REPLAY_DENIED'; pass(['P25-C05-S05'], await rpc('code-replay', { key: initial.key }));
  stage = 'SAME_SECTOR_STABLE_SUBJECT';
  const repeated = await rpc('finish', await authorize(page)); remember(repeated.key);
  assert((await rpc('subjects', { left: initial.key, right: repeated.key })).sameSubject, 'PHASE25_PROTOCOL_SUBJECT_CHANGED');
  pass(['P25-C04-S01'], { repeatedSameClientSubjectStable: true, crossClientSectorNotClaimed: true });
  for (const [mode, scenarioIds] of [['state', []], ['issuer', ['P25-C03-S08']], ['nonce', ['P25-C03-S09']],
    ['verifier', ['P25-C05-S04']], ['client', ['P25-C03-S07']], ['redirect', ['P25-C05-S01']]]) {
    stage = 'STANDARD_NEGATIVE_' + mode.toUpperCase(); pass(scenarioIds, await rpc('finish', { ...await authorize(page), mode }));
  }
  stage = 'AUTH_TIME_STALE_STANDARD_SDK_DENIAL';
  await until(() => Date.now() > authWindow[1] + 2100, 'PHASE25_PROTOCOL_AUTH_TIME_BOUNDARY', 4000);
  pass(['P25-C03-S05'], await rpc('finish', { ...await authorize(page), mode: 'stale-auth-time' }));
  for (const mutation of ['plain', 'missing', 'short', 'long', 'syntax', 'redirect', 'scope']) {
    stage = 'AUTHORIZE_NEGATIVE_' + mutation.toUpperCase();
    pass(mutation === 'plain' ? ['P25-C05-S02'] : ['missing', 'short', 'long', 'syntax'].includes(mutation) ? ['P25-C05-S03']
      : mutation === 'redirect' ? ['P25-C05-S01'] : [], await authorize(page, mutation));
  }
  stage = 'TWO_INDEPENDENT_PROCESS_CODE_EXCHANGE';
  pass(['P25-C05-S06'], await rpc('finish', { ...await authorize(page), mode: 'concurrent' }));
  stage = 'REFRESH_ROTATION_WITH_SOURCE_AND_SCOPE_PRESERVED';
  const refreshable = await rpc('finish', await authorize(page)); remember(refreshable.key);
  const refreshed = await rpc('refresh', { key: refreshable.key });
  assert(refreshed.rotated && refreshed.subjectAndSidPreserved && refreshed.scopeNotExpanded, 'PHASE25_PROTOCOL_REFRESH_ROTATION');
  pass(['P25-C06-S01'], refreshed);
  stage = 'REFRESH_REUSE_CLOSES_CURRENT_FAMILY';
  const replay = await rpc('refresh-replay', { key: refreshable.key }); const closed = await rpc('refresh-denied', { key: refreshable.key });
  pass(['P25-C06-S03'], { originalRejected: replay.rejected, rotatedCurrentRejected: closed.rejected });
  stage = 'REFRESH_SCOPE_SUBSET_AND_ORIGINAL_GRANT_CEILING';
  const shrinkable = await rpc('finish', await authorize(page)); remember(shrinkable.key);
  pass(['P25-C06-S06', 'P25-C06-S07'], await rpc('refresh-scope', { key: shrinkable.key }));
  stage = 'CONFIDENTIAL_WRONG_SECRET'; pass(['P25-C07-S02'], await rpc('wrong-secret'));
  stage = 'SAME_APPLICATION_INDEPENDENT_MACHINE_CLIENTS'; pass(['P25-C01-S02'], await rpc('client-separation'));
  await Promise.allSettled([...observations]); report.tlsProtocols = [...tls];
  assert(tls.has('TLS 1.3'), 'PHASE25_PROTOCOL_REAL_TLS'); report.result = 'PASS';
} catch (error) {
  report.failedStep = stage;
  report.failureCode = interrupted ? 'PHASE25_PROTOCOL_INTERRUPTED' : /^PHASE25_PROTOCOL_[A-Z_]+$/.test(error?.message ?? '') ? error.message
    : error?.name === 'TimeoutError' ? 'PHASE25_PROTOCOL_BROWSER_TIMEOUT' : 'PHASE25_PROTOCOL_STEP_FAILED';
  if (/^[A-Z_0-9]{1,80}$/.test(error?.sdkCode ?? '')) report.sdkCode = error.sdkCode;
  process.exitCode = 1;
} finally {
  for (const operation of pending.values()) operation.reject(new Error('PHASE25_PROTOCOL_SHUTDOWN')); pending.clear();
  if (child) {
    child.stdin.end();
    if (child.exitCode === null && child.signalCode === null) {
      await Promise.race([new Promise(resolve => child.once('exit', resolve)), wait(3000)]);
      if (child.exitCode === null && child.signalCode === null) {
        child.kill();
        await Promise.race([new Promise(resolve => child.once('exit', resolve)), wait(2000)]);
      }
    }
    report.driverClosed = child.exitCode !== null || child.signalCode !== null;
  } else report.driverClosed = true;
  report.cleanup = await closeBusinessBrowser(browser, contexts);
  if (!report.driverClosed || !report.cleanup.browserClosed || !report.cleanup.contextsClosed || report.cleanup.failures) {
    report.result = 'FAIL'; report.failureCode = 'PHASE25_PROTOCOL_CLEANUP_FAILED'; process.exitCode = 1;
  }
  report.finishedAtUtc = new Date().toISOString(); persist(); fs.closeSync(fd);
  process.removeListener('SIGINT', interrupt); process.removeListener('SIGTERM', interrupt);
  console.log('PHASE25_PROTOCOL_RESULT ' + JSON.stringify({ result: report.result, checks: report.checks.length, failedStep: report.failedStep, failureCode: report.failureCode }));
}
