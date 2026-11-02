/** Actual full-version BFF -> capability -> business and notification checks.
 * Secrets remain in process memory. Every HTTP response is produced by the
 * current native service; no routes or claims are mocked/intercepted here.
 */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { fetchBusinessJson, navigateBusinessReadiness, closeBusinessBrowser } from './phase25-business-http.mjs';

const TASK = '/tmp/rust-saas-unified-auth-20261028';
const AUTH = 'https://auth.saas.test', CONSOLE = 'https://console.saas.test';
const BUSINESS = `${CONSOLE}:24447`;
const here = path.dirname(fileURLToPath(import.meta.url));
const repository = path.resolve(here, '../../..');
const args = {};
for (let index = 2; index < process.argv.length; index += 2) {
  assert(['--snapshot', '--output'].includes(process.argv[index]) && process.argv[index + 1]
    && !Object.hasOwn(args, process.argv[index]), 'PHASE25_BUSINESS_ARGUMENTS_INVALID');
  args[process.argv[index]] = process.argv[index + 1];
}
const label = args['--snapshot'];
assert(/^phase25-tools-r[1-9][0-9]*$/.test(label ?? ''), 'PHASE25_BUSINESS_FROZEN_TOOLS_REQUIRED');
const output = path.resolve(args['--output'] ?? '');
assert(path.dirname(output) === path.join(repository, 'backend/docs/designs/unified-auth/validation')
  && /^phase-25-business-[a-z0-9-]+\.json$/.test(path.basename(output)), 'PHASE25_BUSINESS_SAFE_OUTPUT_REQUIRED');
const fd = fs.openSync(output, 'wx', 0o600);
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const privateCanaries = new Set();
const contexts = [], observations = new Set(), tls = new Set();
const report = { contractVersion: 'unified-auth-phase25-real-business/1', result: 'FAIL',
  startedAt: new Date().toISOString(), sourceSnapshot: label, checks: [], tlsErrorsIgnored: false,
  responseInterception: false, rawCarriersPersisted: false, databaseMutationsByBrowserRunner: false };
let stage = 'PREFLIGHT', browser, environment, page, pollingUrl, platformToken;
function canaries(value, field = '') {
  if (Array.isArray(value)) for (const child of value) canaries(child, field);
  else if (value && typeof value === 'object') for (const [name, child] of Object.entries(value)) canaries(child, name);
  else if (typeof value === 'string' && value.length >= 8 && /password|secret|token|csrf|cookie|username|privatekey/i.test(field)) privateCanaries.add(value);
}
function persist() {
  const text = JSON.stringify(report, null, 2) + '\n';
  assert(!/"(?:access_token|refresh_token|id_token|logout_token|password|clientSecret|cookie|verifier)"\s*:/i.test(text), 'PHASE25_BUSINESS_REPORT_FIELD_LEAK');
  for (const value of privateCanaries) assert(!text.includes(value), 'PHASE25_BUSINESS_REPORT_VALUE_LEAK');
  const bytes = Buffer.from(text); fs.ftruncateSync(fd, 0); fs.writeSync(fd, bytes, 0, bytes.length, 0); fs.fsyncSync(fd);
}
function pass(facts = {}) { report.checks.push({ caseId: stage, status: 'PASS', ...facts }); persist(); console.log(`PHASE25_BUSINESS_CHECK ${stage} PASS`); }
function wsl(command) {
  try { return execFileSync('wsl.exe', ['-d', 'Ubuntu', '-u', 'jackie', '--exec', ...command], {
    encoding: 'utf8', windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'], timeout: 45000, maxBuffer: 8 * 1024 * 1024,
  }); } catch { throw new Error('PHASE25_BUSINESS_OWNED_PREFLIGHT_FAILED'); }
}
const preflight = String.raw`
import hashlib,json,pathlib,re,subprocess,sys
t=pathlib.Path(sys.argv[1]);label=sys.argv[2];snapshot=t/('source-snapshot-'+label)
assert re.fullmatch(r'phase25-tools-r[1-9][0-9]*',label) and snapshot.resolve(strict=True)==snapshot
sys.path.insert(0,str(snapshot/'saas-plateform/backend/tests/unified-auth-https'));import phase23_cluster_binding as b
bound=b.load_binding(t);uid=bound['taskUid']
def read(p):return b.protected_read(p,uid=uid,mode=0o600,maximum=8*1024*1024)[0]
m=json.loads(read(t/('snapshot-'+label+'.json')));inputs=m['inputs'];assert m['includeAllProjects'] and m['completeFrontendOutputs']
assert m['snapshotSha256']==hashlib.sha256(json.dumps(inputs,sort_keys=True).encode()).hexdigest()
relative='saas-plateform/backend/tests/unified-auth-https/phase25-business-browser.mjs';assert hashlib.sha256(read(snapshot/relative)).hexdigest()==inputs[relative]
helper='saas-plateform/backend/tests/unified-auth-https/phase25-business-http.mjs';assert hashlib.sha256(read(snapshot/helper)).hexdigest()==inputs[helper]
diagnostic='saas-plateform/backend/tests/unified-auth-https/phase25-business-diagnostic.py';assert hashlib.sha256(read(snapshot/diagnostic)).hexdigest()==inputs[diagnostic]
e=json.loads(read(t/'https-environment.json'));d=json.loads(read(t/'https-database.json'));business=json.loads(read(t/'phase25-business-manifest.json'))
assert d['clusterId']==bound['systemIdentifier'] and re.fullmatch(r'unified_auth_https_[0-9a-f]{32}',d['database'])
query="SELECT "+b.sql_guard(bound)+" AND (SELECT shobj_description(oid,'pg_database') FROM pg_database WHERE datname=current_database())='identity-deployment/1/"+e['ids']['deployment']+"';"
r=subprocess.run(['/usr/lib/postgresql/18/bin/psql','-X','-qAt','-v','ON_ERROR_STOP=1','-h','127.0.0.1','-p','55494','-U','postgres','-d',d['database']],input=query.encode(),capture_output=True,timeout=15,env={'PATH':'/usr/bin:/bin','PGOPTIONS':'-c log_statement=none -c log_min_error_statement=panic -c default_transaction_read_only=on'})
assert r.returncode==0 and r.stdout.strip()==b't'
print(json.dumps({'environment':e,'business':business,'snapshotSha256':m['snapshotSha256'],'scriptSha256':inputs[relative],'helperSha256':inputs[helper],'diagnosticSha256':inputs[diagnostic]}))
`;
async function json(endpoint, { method = 'GET', body, bearer, csrf, idempotencyKey } = {}) {
  const result = await page.evaluate(fetchBusinessJson, { endpoint, method, body, bearer, csrf, idempotencyKey });
  canaries(result.body); return result;
}
async function login() {
  await page.goto(`${CONSOLE}/auth/login?return=/`, { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => location.origin === 'https://console.saas.test' || !!document.querySelector('#login-form:not([hidden])'));
  if (new URL(page.url()).origin === AUTH) {
    const account = environment.accounts.employee;
    await page.locator('#username').fill(account.username); await page.locator('#password').fill(account.password);
    const response = page.waitForResponse(r => r.url() === `${AUTH}/api/sso/login` && r.request().method() === 'POST');
    await page.locator('#login-submit').click(); assert.equal((await response).status(), 200, 'PHASE25_BUSINESS_REAL_PRIMARY_LOGIN_FAILED');
  }
  for (let index = 0; index < 8; index++) {
    const state = await page.waitForFunction(() => {
      if (location.origin === 'https://console.saas.test' && !location.pathname.startsWith('/auth/')) return 'done';
      if (document.querySelector('#error-message:not([hidden])')) return 'error';
      if (document.querySelector('#auth-content')?.dataset.screen === 'CONSENT_REQUIRED'
          && !document.querySelector('#consent-panel')?.hidden && !document.querySelector('#consent-allow')?.disabled) return 'consent';
      return false;
    });
    const next = await state.jsonValue(); await state.dispose();
    assert.notEqual(next, 'error', 'PHASE25_BUSINESS_REAL_IDENTITY_REJECTED');
    if (next === 'done') break;
    await page.locator('#consent-allow').click();
  }
  await page.waitForURL(url => url.origin === CONSOLE && !url.pathname.startsWith('/auth/'));
  await page.goto(`${CONSOLE}/auth/config`);
  const session = await json('/auth/session');
  assert.equal(session.status, 200, 'PHASE25_BUSINESS_BFF_FAILED');
  assert.equal(session.body.account?.id, environment.ids.employee, 'PHASE25_BUSINESS_ACCOUNT_MISMATCH');
  assert.equal(session.body.context?.memberType, 'EMPLOYEE', 'PHASE25_BUSINESS_MEMBER_TYPE_MISMATCH');
  const capability = await json('/auth/context-token', { method: 'POST', body: { expectedVersion: session.body.version }, csrf: session.body.csrfToken });
  assert.equal(capability.status, 200, 'PHASE25_BUSINESS_PLATFORM_CAPABILITY_FAILED');
  assert.equal(typeof capability.body.accessToken, 'string', 'PHASE25_BUSINESS_PLATFORM_CARRIER_MISSING');
  platformToken = capability.body.accessToken;
}
async function appToken(code) {
  const result = await json('/api/apps/session-token', { method: 'POST', bearer: platformToken, body: { appCode: code } });
  assert.equal(result.status, 201, 'PHASE25_BUSINESS_APP_CAPABILITY_FAILED');
  assert.equal(result.body.audience, code, 'PHASE25_BUSINESS_APP_AUDIENCE_MISMATCH');
  return result.body.accessToken;
}
async function packet(method = 'GET', body) {
  return page.evaluate(async input => {
    const response = await fetch(input.url, { method: input.method, credentials: 'omit', cache: 'no-store', redirect: 'error',
      ...(input.body === undefined ? {} : { headers: { 'Content-Type': 'text/plain;charset=UTF-8' }, body: input.body }), signal: AbortSignal.timeout(24000) });
    return { status: response.status, text: await response.text() };
  }, { url: pollingUrl, method, body });
}
async function nextEvent(expected, maximum = 8) {
  for (let count = 0; count < maximum; count++) {
    const result = await packet(); assert.equal(result.status, 200, 'PHASE25_BUSINESS_NOTIFICATION_POLL_FAILED');
    for (const item of result.text.split('\x1e')) {
      if (item === '2') await packet('POST', '3');
      if (expected === 'namespace' && item.startsWith('40')) return {};
      if (item.startsWith('42')) { const value = JSON.parse(item.slice(2)); if (value[0] === expected) return value[1]; }
      assert(!['1', '41'].includes(item), 'PHASE25_BUSINESS_NOTIFICATION_EARLY_DISCONNECT');
    }
  }
  throw new Error('PHASE25_BUSINESS_NOTIFICATION_EVENT_MISSING');
}
try {
  const verified = JSON.parse(wsl(['python3', '-B', '-c', preflight, TASK, label]));
  environment = verified.environment; canaries(environment);
  assert.equal(hash(fs.readFileSync(fileURLToPath(import.meta.url))), verified.scriptSha256, 'PHASE25_BUSINESS_LIVE_RUNNER_NOT_FROZEN');
  assert.equal(hash(fs.readFileSync(path.join(here, 'phase25-business-http.mjs'))), verified.helperSha256, 'PHASE25_BUSINESS_LIVE_HELPER_NOT_FROZEN');
  assert.equal(hash(fs.readFileSync(path.join(here, 'phase25-business-diagnostic.py'))), verified.diagnosticSha256, 'PHASE25_BUSINESS_DIAGNOSTIC_NOT_FROZEN');
  report.helperSha256 = verified.helperSha256;
  report.snapshotSha256 = verified.snapshotSha256; report.currentClusterAndSourceVerified = true; pass();
  process.env.PLAYWRIGHT_BROWSERS_PATH = path.join(repository, 'frontend/tests/browser/.browsers');
  const { chromium } = createRequire(path.join(repository, 'frontend/tests/browser/package.json'))('playwright');
  browser = await chromium.launch({ headless: true, proxy: { server: 'http://127.0.0.1:24445' } });
  const context = await browser.newContext({ viewport: { width: 1440, height: 960 }, serviceWorkers: 'block' }); contexts.push(context);
  page = await context.newPage(); page.setDefaultTimeout(30000); page.setDefaultNavigationTimeout(40000);
  page.on('response', response => {
    const observation = response.securityDetails().then(details => { if (details?.protocol) tls.add(details.protocol); }).catch(() => {});
    observations.add(observation); observation.finally(() => observations.delete(observation));
  });
  stage = 'REAL_EMPLOYEE_BFF_AND_DERIVED_CAPABILITIES'; await login();
  const stores = await appToken('stores'), orders = await appToken('work-orders');
  assert.notEqual(stores, orders, 'PHASE25_BUSINESS_APP_CARRIERS_NOT_DISTINCT'); pass({ employeeBff: true, exactAudienceCapabilities: 2 });
  stage = 'FRESH_BUSINESS_READINESS_AND_AUTHENTICATION';
  for (const prefix of ['/stores-api', '/work-orders-api']) {
    assert.equal((await json(`${BUSINESS}${prefix}/health/ready`)).status, 200, 'PHASE25_BUSINESS_READY_FAILED');
  }
  const storesRead = await json(`${BUSINESS}/stores-api/stores`, { bearer: stores });
  if (storesRead.status !== 200) {
    report.failedStoresReadStatus = storesRead.status;
    try {
      report.privateBusinessDiagnostic = JSON.parse(execFileSync('wsl.exe', ['-d', 'Ubuntu', '-u', 'root', '--exec',
        'python3', '-B', `${TASK}/source-snapshot-${label}/saas-plateform/backend/tests/unified-auth-https/phase25-business-diagnostic.py`, '--snapshot', label], {
        input: JSON.stringify({ token: stores }), encoding: 'utf8', windowsHide: true,
        stdio: ['pipe', 'pipe', 'pipe'], timeout: 45000, maxBuffer: 8 * 1024 * 1024,
      }));
    } catch { report.privateBusinessDiagnostic = { result: 'FAIL', failureCode: 'PHASE25_BUSINESS_DIAGNOSTIC_SUBPROCESS_FAILED' }; }
    persist();
  }
  assert.equal(storesRead.status, 200, 'PHASE25_BUSINESS_STORES_READ_FAILED');
  assert.equal((await json(`${BUSINESS}/work-orders-api/tickets`, { bearer: orders })).status, 200, 'PHASE25_BUSINESS_ORDERS_READ_FAILED');
  for (const [endpoint, wrong] of [['/stores-api/stores', orders], ['/work-orders-api/tickets', stores]]) {
    assert([401, 403].includes((await json(BUSINESS + endpoint)).status), 'PHASE25_BUSINESS_MISSING_CARRIER_ACCEPTED');
    assert([401, 403].includes((await json(BUSINESS + endpoint, { bearer: wrong })).status), 'PHASE25_BUSINESS_WRONG_AUDIENCE_ACCEPTED');
    assert([401, 403].includes((await json(BUSINESS + endpoint, { bearer: 'invalid-public-fixture' })).status), 'PHASE25_BUSINESS_INVALID_CARRIER_ACCEPTED');
  }
  assert.equal((await json('/api/apps/stores/authorize', { method: 'POST', body: {} })).status, 404, 'PHASE25_BUSINESS_PUBLIC_AUTHORITY_EXPOSED');
  assert.equal((await json('/api/internal/notifications/context-proof', { bearer: platformToken })).status, 404, 'PHASE25_BUSINESS_PUBLIC_NOTIFICATION_PROOF_EXPOSED');
  pass({ actualCanonicalPrivateProofs: true, realBusinessServers: 2, wrongAudienceDenied: 2, missingAndInvalidDenied: 4, publicProofBoundariesClosed: 2 });
  stage = 'REAL_BUSINESS_WRITE_AND_DURABLE_RESULT';
  const ticketRequest = { method: 'POST', bearer: orders, idempotencyKey: 'owned-phase25-current-candidate-ticket-once',
    body: { title: 'Owned Phase25 current candidate', description: 'Current independent database and exact member proof.' } };
  const created = await json(`${BUSINESS}/work-orders-api/tickets`, ticketRequest);
  assert.equal(created.status, 201, 'PHASE25_BUSINESS_TICKET_CREATE_FAILED');
  assert.equal(typeof created.body?.id, 'string', 'PHASE25_BUSINESS_TICKET_ID_MISSING');
  const replayed = await json(`${BUSINESS}/work-orders-api/tickets`, ticketRequest);
  assert.equal(replayed.status, 201, 'PHASE25_BUSINESS_TICKET_REPLAY_FAILED');
  assert.deepEqual(replayed.body, created.body, 'PHASE25_BUSINESS_TICKET_REPLAY_CHANGED');
  const conflict = await json(`${BUSINESS}/work-orders-api/tickets`, {
    ...ticketRequest, body: { ...ticketRequest.body, title: 'Owned Phase25 conflicting retry' },
  });
  assert.equal(conflict.status, 409, 'PHASE25_BUSINESS_TICKET_DIFFERENT_RETRY_ACCEPTED');
  const list = await json(`${BUSINESS}/work-orders-api/tickets`, { bearer: orders });
  assert.equal(list.status, 200, 'PHASE25_BUSINESS_DURABLE_READ_FAILED');
  assert(Array.isArray(list.body?.records), 'PHASE25_BUSINESS_TICKET_LIST_INVALID');
  assert.equal(list.body.records.filter(ticket => ticket.id === created.body.id).length, 1, 'PHASE25_BUSINESS_TICKET_REPLAY_DUPLICATED');
  assert.equal(list.body.records.filter(ticket => ticket.title === ticketRequest.body.title).length, 1, 'PHASE25_BUSINESS_CREATED_TICKET_MISSING');
  assert.equal(list.body.records.filter(ticket => ticket.title === 'Owned Phase25 conflicting retry').length, 0, 'PHASE25_BUSINESS_CONFLICTING_TICKET_WRITTEN');
  pass({ realBusinessWrite: true, independentDatabaseReadAfterWrite: true, sameKeyReplayRestoredExactResult: true,
    differentBodySameKeyDenied: true, durableMatchingTickets: 1 });
  stage = 'REAL_NOTIFICATION_AUTHENTICATION_AND_DELIVERY';
  await navigateBusinessReadiness(page, `${BUSINESS}/health/ready`, `${CONSOLE}/auth/config`);
  pollingUrl = `${BUSINESS}/socket.io/?EIO=4&transport=polling`;
  const opened = await packet(); assert.equal(opened.status, 200, 'PHASE25_BUSINESS_ENGINE_OPEN_FAILED');
  assert(opened.text.startsWith('0'), 'PHASE25_BUSINESS_ENGINE_HANDSHAKE_FAILED');
  const sid = JSON.parse(opened.text.slice(1)).sid; canaries(sid, 'token');
  assert(typeof sid === 'string' && sid.length < 200, 'PHASE25_BUSINESS_ENGINE_SESSION_INVALID'); pollingUrl += '&sid=' + encodeURIComponent(sid);
  assert.equal((await packet('POST', '40')).status, 200, 'PHASE25_BUSINESS_NAMESPACE_CONNECT_FAILED'); await nextEvent('namespace');
  assert.equal((await packet('POST', '42' + JSON.stringify(['authenticate', { token: platformToken }]))).status, 200, 'PHASE25_BUSINESS_SOCKET_AUTH_SUBMIT_FAILED');
  const acknowledgement = await nextEvent('authenticated');
  assert(acknowledgement && typeof acknowledgement === 'object', 'PHASE25_BUSINESS_SOCKET_ACK_MISSING');
  const publication = await json('/api/apps/work-orders/notifications', { method: 'POST', bearer: orders, body: {
    idempotencyKey: 'owned-phase25-notification-once', target: { kind: 'USER', userId: environment.ids.employee, organizationId: environment.ids.organization },
    type: 'INFO', title: 'Owned Phase25 notification', content: 'Current real recipient source.', link: '/micro/work-orders',
  } });
  assert([200, 201].includes(publication.status), 'PHASE25_BUSINESS_NOTIFICATION_PUBLISH_FAILED');
  const delivered = await nextEvent('notification');
  assert(JSON.stringify(delivered).includes('Owned Phase25 notification'), 'PHASE25_BUSINESS_WRONG_NOTIFICATION_DELIVERED');
  pass({ nativeNotificationProcess: true, actualSocketIoPolling: true, canonicalContextProof: true, persistedPublicationAndPostgresHint: true, actualRecipientDelivery: true });
  assert.equal((await packet('POST', '1')).status, 200, 'PHASE25_BUSINESS_SOCKET_CLOSE_FAILED'); pollingUrl = null;
  stage = 'CURRENT_BUSINESS_TLS'; await Promise.allSettled([...observations]);
  assert(tls.size > 0 && [...tls].every(protocol => /^TLS [12]\./.test(protocol)), 'PHASE25_BUSINESS_VERIFIED_TLS_REQUIRED');
  report.tlsProtocols = [...tls]; report.browserVersion = browser.version(); pass(); report.result = 'PASS';
} catch (error) {
  const fixed = String(error?.message ?? '').split(/\r?\n/, 1)[0];
  report.failedStep = stage; report.failureCode = /^PHASE25_BUSINESS_[A-Z0-9_]+$/.test(fixed) ? fixed
    : error?.name === 'TimeoutError' ? 'PHASE25_BUSINESS_BROWSER_TIMEOUT' : 'PHASE25_BUSINESS_STEP_FAILED';
  process.exitCode = 1;
} finally {
  if (pollingUrl && page && !page.isClosed()) await packet('POST', '1').catch(() => {});
  await Promise.allSettled([...observations]);
  report.cleanup = await closeBusinessBrowser(browser, contexts);
  report.browserClosed = report.cleanup.browserClosed; report.endedAt = new Date().toISOString();
  if (!report.cleanup.browserClosed || !report.cleanup.contextsClosed || report.cleanup.failures > 0) {
    report.result = 'FAIL'; report.failedStep = 'BROWSER_CLEANUP';
    report.failureCode = 'PHASE25_BUSINESS_BROWSER_CLEANUP_FAILED'; process.exitCode = 1;
  }
  persist(); fs.closeSync(fd); console.log(`PHASE25_BUSINESS_RESULT ${JSON.stringify({ result: report.result, checks: report.checks.length, failedStep: report.failedStep, failureCode: report.failureCode })}`);
}
