/** Task-owned current services: type/audience and permission contractions.
 * Invoke after the ordinary business suite and before its fixture cleanup/MFA.
 * No response is forged except the deliberately held public RP callback page;
 * the authorization code is exchanged at the real public token endpoint.
 */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { createHash, createPublicKey, randomBytes, verify } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { runResourceCases } from './phase25-resource-cases.mjs';
import { fetchBusinessJson, closeBusinessBrowser } from './phase25-business-http.mjs';

const TASK = '/tmp/rust-saas-unified-auth-20261028';
const AUTH = 'https://auth.saas.test', CONSOLE = 'https://console.saas.test';
const BUSINESS = `${CONSOLE}:24447`;
const here = path.dirname(fileURLToPath(import.meta.url));
const repository = path.resolve(here, '../../..');
const args = {};
for (let index = 2; index < process.argv.length; index += 2) {
  assert(['--snapshot', '--output', '--capture-recovery'].includes(process.argv[index]) && process.argv[index + 1]
    && !Object.hasOwn(args, process.argv[index]), 'PHASE25_RESOURCE_ARGUMENTS_INVALID');
  args[process.argv[index]] = process.argv[index + 1];
}
const label = args['--snapshot'];
const captureOnly = args['--capture-recovery'] === 'true';
assert(args['--capture-recovery'] === undefined || captureOnly, 'PHASE25_RESOURCE_CAPTURE_MODE_INVALID');
assert(/^phase25-tools-r[1-9][0-9]*$/.test(label ?? ''), 'PHASE25_RESOURCE_FROZEN_TOOLS_REQUIRED');
const output = path.resolve(args['--output'] ?? '');
assert(path.dirname(output) === path.join(repository, 'backend/docs/designs/unified-auth/validation')
  && /^phase-25-(?:resource|business-recovery-capture)-[a-z0-9-]+\.json$/.test(path.basename(output)), 'PHASE25_RESOURCE_SAFE_OUTPUT_REQUIRED');
assert(!fs.lstatSync(path.dirname(output)).isSymbolicLink(), 'PHASE25_RESOURCE_OUTPUT_LINK_REJECTED');
const fd = fs.openSync(output, 'wx', 0o600);
const hash = value => createHash('sha256').update(value).digest('hex');
const canonical = value => JSON.stringify(value, (_key, item) => item && typeof item === 'object' && !Array.isArray(item)
  ? Object.fromEntries(Object.keys(item).sort().map(key => [key, item[key]])) : item);
const canaryValues = new Set();
const contexts = [], observations = new Set(), tls = new Set(), actors = new Map();
const report = { contractVersion: 'unified-auth-phase25-real-resources/1', result: 'FAIL',
  startedAtUtc: new Date().toISOString(), sourceSnapshot: label, checks: [], tlsErrorsIgnored: false,
  databaseMutationScope: 'Owned fixture employee member, stores status/generation and exact store-read data grant only',
  rawCarriersPersisted: false, observedEndpoints: {}, policyCleanup: null };
let browser, environment, verified, removedGrant, pendingGrantRestore = false, captureAttempted = false, stage = 'PREFLIGHT';

function canaries(value, field = '') {
  if (Array.isArray(value)) for (const child of value) canaries(child, field);
  else if (value && typeof value === 'object') for (const [key, child] of Object.entries(value)) canaries(child, key);
  else if (typeof value === 'string' && value.length >= 8 && /secret|password|token|cookie|csrf|nonce|verifier|username/i.test(field)) canaryValues.add(value);
}
function persist() {
  const text = JSON.stringify(report, null, 2) + '\n';
  assert(!/"(?:access_token|id_token|refresh_token|password|clientSecret|cookie|nonce|verifier)"\s*:/i.test(text), 'PHASE25_RESOURCE_SECRET_FIELD');
  for (const value of canaryValues) assert(!text.includes(value), 'PHASE25_RESOURCE_SECRET_VALUE');
  const bytes = Buffer.from(text); fs.ftruncateSync(fd, 0); fs.writeSync(fd, bytes, 0, bytes.length, 0); fs.fsyncSync(fd);
}
function control(action, grant, carriers) {
  const script = `${TASK}/source-snapshot-${label}/saas-plateform/backend/tests/unified-auth-https/phase25-resource-control.py`;
  try {
    const raw = execFileSync('wsl.exe', ['-d', 'Ubuntu', '-u', 'jackie', '--exec', 'python3', '-B', script], {
      input: JSON.stringify({ action, ...(grant ? { grant } : {}), ...(carriers ? { carriers } : {}) }), encoding: 'utf8', windowsHide: true,
      stdio: ['pipe', 'pipe', 'pipe'], timeout: 45000, maxBuffer: 8 * 1024 * 1024,
    });
    const value = JSON.parse(raw); canaries(value); return value;
  } catch { throw new Error('PHASE25_RESOURCE_OWNED_CONTROL_FAILED'); }
}
async function request(page, endpoint, options = {}) {
  const result = await page.evaluate(fetchBusinessJson, { endpoint, method: 'GET', ...options });
  canaries(result.body);
  const key = (endpoint.startsWith(BUSINESS) ? 'business:' : 'console:') + new URL(endpoint, CONSOLE).pathname + ':' + result.status;
  report.observedEndpoints[key] = (report.observedEndpoints[key] ?? 0) + 1;
  return result;
}
async function newPage() {
  const context = await browser.newContext({ viewport: { width: 1440, height: 960 }, serviceWorkers: 'block' });
  contexts.push(context);
  const page = await context.newPage(); page.setDefaultTimeout(30000); page.setDefaultNavigationTimeout(40000);
  page.on('response', response => {
    const observation = response.securityDetails().then(details => { if (details?.protocol) tls.add(details.protocol); }).catch(() => {});
    observations.add(observation); observation.finally(() => observations.delete(observation));
  });
  return page;
}
async function finishIdentity(page, account, destination, callback = false) {
  await page.waitForFunction(() => location.origin !== 'https://auth.saas.test'
    || !!document.querySelector('#login-form:not([hidden])') || !!document.querySelector('#consent-panel:not([hidden])'));
  if (new URL(page.url()).origin === AUTH && await page.locator('#login-form:not([hidden])').count()) {
    await page.locator('#username').fill(account.username); await page.locator('#password').fill(account.password);
    const response = page.waitForResponse(value => value.url() === `${AUTH}/api/sso/login` && value.request().method() === 'POST');
    await page.locator('#login-submit').click();
    assert.equal((await response).status(), 200, 'PHASE25_RESOURCE_REAL_LOGIN_FAILED');
  }
  for (let index = 0; index < 10; index++) {
    const state = await page.waitForFunction(({ origin, callback }) => {
      if (location.origin === origin && (callback ? location.pathname === '/auth/callback' : !location.pathname.startsWith('/auth/'))) return 'done';
      if (document.querySelector('#error-message:not([hidden])')) return 'error';
      if (document.querySelector('#auth-content')?.dataset.screen === 'CONSENT_REQUIRED'
        && !document.querySelector('#consent-panel')?.hidden && !document.querySelector('#consent-allow')?.disabled) return 'consent';
      return false;
    }, { origin: destination, callback });
    const value = await state.jsonValue(); await state.dispose();
    assert.notEqual(value, 'error', 'PHASE25_RESOURCE_IDENTITY_REJECTED');
    if (value === 'done') return;
    await page.locator('#consent-allow').click();
  }
  assert.fail('PHASE25_RESOURCE_IDENTITY_TRANSITIONS_EXCEEDED');
}
async function login(actor, force = false) {
  if (actors.has(actor) && !force) return actors.get(actor);
  const page = await newPage();
  await page.goto(`${CONSOLE}/auth/login?return=/`, { waitUntil: 'domcontentloaded' });
  await finishIdentity(page, environment.accounts[actor], CONSOLE);
  await page.goto(`${CONSOLE}/auth/config`, { waitUntil: 'domcontentloaded' });
  const session = await request(page, '/auth/session');
  assert.equal(session.status, 200, 'PHASE25_RESOURCE_SESSION_FAILED');
  assert.equal(session.body.account?.id, environment.ids[actor], 'PHASE25_RESOURCE_SESSION_ACCOUNT_CHANGED');
  assert.equal(session.body.context?.membershipId, environment.ids[`${actor}Member`], 'PHASE25_RESOURCE_SESSION_MEMBER_CHANGED');
  const capability = await request(page, '/auth/context-token', { method: 'POST',
    body: { expectedVersion: session.body.version }, csrf: session.body.csrfToken });
  assert.equal(capability.status, 200, 'PHASE25_RESOURCE_PLATFORM_CAPABILITY_FAILED');
  assert.equal(typeof capability.body.accessToken, 'string', 'PHASE25_RESOURCE_PLATFORM_CAPABILITY_MISSING');
  const record = { page, platformToken: capability.body.accessToken };
  actors.set(actor, record); return record;
}
async function mint(actor, code, options) {
  const source = await login(actor, options?.newLogin === true);
  const response = await request(source.page, '/api/apps/session-token', { method: 'POST',
    bearer: source.platformToken, body: { appCode: code } });
  assert.equal(response.status, 201, 'PHASE25_RESOURCE_APP_MINT_FAILED');
  assert.equal(response.body.audience, code, 'PHASE25_RESOURCE_APP_MINT_AUDIENCE_CHANGED');
  return response.body.accessToken;
}
async function tokenRequest(page, client, secret, form) {
  const result = await page.evaluate(async input => {
    const response = await fetch('/oauth/token', { method: 'POST', credentials: 'omit', redirect: 'error', cache: 'no-store',
      headers: { Authorization: `Basic ${btoa(input.client + ':' + input.secret)}`, 'Content-Type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams(input.form), signal: AbortSignal.timeout(20000) });
    let body; try { body = await response.json(); } catch { body = null; } return { status: response.status, body };
  }, { client, secret, form });
  canaries(result.body); return result;
}
async function types() {
  const page = await newPage(), config = environment.phase21.rp.third;
  const verifier = randomBytes(32).toString('base64url'), nonce = randomBytes(32).toString('base64url'), state = randomBytes(32).toString('base64url');
  for (const value of [verifier, nonce, state]) canaryValues.add(value);
  const callbackTarget = `${config.RP_ORIGIN}/auth/callback`; let callback;
  await page.route(callbackTarget + '?*', async route => {
    callback = route.request().url(); await route.fulfill({ status: 200, contentType: 'text/plain', body: 'Owned callback held in memory.' });
  });
  const authorize = new URL('/oauth/authorize', AUTH);
  authorize.search = new URLSearchParams({ response_type: 'code', client_id: config.OIDC_CLIENT_ID,
    redirect_uri: callbackTarget, scope: 'openid profile', state, nonce, code_challenge_method: 'S256',
    code_challenge: createHash('sha256').update(verifier).digest('base64url') });
  await page.goto(authorize.href, { waitUntil: 'domcontentloaded' });
  await finishIdentity(page, environment.phase21.accounts.user, config.RP_ORIGIN, true);
  await page.unroute(callbackTarget + '?*');
  assert(callback && new URL(callback).searchParams.get('state') === state, 'PHASE25_RESOURCE_STANDARD_CALLBACK_SOURCE_FAILED');
  const code = new URL(callback).searchParams.get('code'); canaries(code, 'token');
  assert(code && !new URL(callback).searchParams.has('error'), 'PHASE25_RESOURCE_STANDARD_CODE_REQUIRED');
  await page.goto(`${AUTH}/oauth/jwks`, { waitUntil: 'domcontentloaded' });
  const jwks = await page.evaluate(() => JSON.parse(document.body.textContent));
  const person = await tokenRequest(page, config.OIDC_CLIENT_ID, verified.secrets.third,
    { grant_type: 'authorization_code', redirect_uri: callbackTarget, code, code_verifier: verifier });
  assert.equal(person.status, 200, 'PHASE25_RESOURCE_REAL_ID_TOKEN_NOT_ISSUED');
  assert.equal(typeof person.body.id_token, 'string', 'PHASE25_RESOURCE_ID_TOKEN_MISSING');
  const parts = person.body.id_token.split('.'), header = JSON.parse(Buffer.from(parts[0], 'base64url')),
    claims = JSON.parse(Buffer.from(parts[1], 'base64url'));
  const publicJwk = jwks.keys.find(key => key.kid === header.kid);
  assert(header.alg === 'RS256' && publicJwk && verify('RSA-SHA256', Buffer.from(parts.slice(0, 2).join('.')),
    createPublicKey({ key: publicJwk, format: 'jwk' }), Buffer.from(parts[2], 'base64url')), 'PHASE25_RESOURCE_ID_SIGNATURE_INVALID');
  assert(claims.iss === AUTH && claims.aud === config.OIDC_CLIENT_ID && claims.nonce === nonce
    && claims.exp > Date.now() / 1000 && typeof claims.sub === 'string', 'PHASE25_RESOURCE_ID_SOURCE_INVALID');
  const machine = await tokenRequest(page, environment.phase21.machine.clientId, verified.secrets.machine,
    { grant_type: 'client_credentials', scope: environment.phase21.machine.scope });
  assert.equal(machine.status, 200, 'PHASE25_RESOURCE_MACHINE_NOT_ISSUED');
  assert(typeof machine.body.access_token === 'string' && !machine.body.id_token && !machine.body.refresh_token
    && !['sub', 'userId', 'membershipId', 'auth_time'].some(key => Object.hasOwn(machine.body, key)), 'PHASE25_RESOURCE_MACHINE_PERSON_SHAPE');
  return { idToken: person.body.id_token, machineToken: machine.body.access_token,
    platformToken: (await login('employee')).platformToken, idTokenVerified: true, machineHasPersonFields: false };
}
function change(action) {
  if (action === 'remove-employee-store-read') {
    // Record intent before mutation. A failed response never permits claiming
    // cleanup: the caller retains the failure and the root fixture is dropped.
    pendingGrantRestore = true; removedGrant = control(action); return;
  }
  if (action === 'restore-employee-store-read') {
    assert(removedGrant, 'PHASE25_RESOURCE_GRANT_RESULT_REQUIRED');
    control(action, removedGrant); pendingGrantRestore = false; return;
  }
  return control(action);
}
async function restore() {
  if (pendingGrantRestore) change('restore-employee-store-read');
  control('enable-stores'); control('enable-employee');
  const result = control('inspect');
  result.generationsNeverDecreased = result.memberGeneration >= verified.initialPolicy.memberGeneration
    && result.applicationGeneration >= verified.initialPolicy.applicationGeneration;
  result.policyRestored = result.memberEnabled && result.applicationEnabled && result.dataGrantRestored;
  report.policyCleanup = { ...result };
  return result;
}

try {
  verified = control('preflight'); environment = verified.environment;
  for (const value of Object.values(verified.secrets)) canaryValues.add(value);
  assert.equal(verified.sourceSnapshot, label, 'PHASE25_RESOURCE_TOOL_SELECTION_CHANGED');
  for (const name of ['phase25-resource-browser.mjs', 'phase25-resource-cases.mjs', 'phase25-business-http.mjs']) {
    assert.equal(hash(fs.readFileSync(path.join(here, name))), verified.scriptSha256[name], 'PHASE25_RESOURCE_LIVE_TOOL_NOT_FROZEN');
  }
  report.snapshotSha256 = verified.snapshotSha256; report.currentFixtureGuardVerified = true;
  process.env.PLAYWRIGHT_BROWSERS_PATH = path.join(repository, 'frontend/tests/browser/.browsers');
  const { chromium } = createRequire(path.join(repository, 'frontend/tests/browser/package.json'))('playwright');
  browser = await chromium.launch({ headless: true, proxy: { server: 'http://127.0.0.1:24445' } });
  if (captureOnly) {
    stage = 'FRESH_RECOVERY_CARRIER_CAPTURE';
    const employee = await login('employee', true), carriers = {};
    for (const [kind, code, endpoint] of [['stores', 'stores', '/stores-api/stores'], ['orders', 'work-orders', '/work-orders-api/tickets']]) {
      const token = await mint('employee', code);
      const source = await request(employee.page, BUSINESS + endpoint, { bearer: token });
      assert.equal(source.status, 200, 'PHASE25_RESOURCE_RECOVERY_SOURCE_READ_FAILED');
      carriers[kind] = { value: token, sourceResponseSha256: hash(canonical(source.body)) };
    }
    captureAttempted = true;
    const captured = control('capture-recovery', undefined, carriers);
    report.checks.push({ caseId: stage, status: 'PASS', ...captured, sourceAuthenticatedReads: 2 });
    report.policyCleanup = { policyRestored: true, mutationsPerformed: false };
    report.captureModeOnly = true; report.rawCarriersPersisted = true;
    report.formalReportContainsRawCredentials = false;
    report.rawCarrierStorage = 'Owned task mode-0600 handoff, removed by recovery finally';
    report.result = 'PASS';
  } else {
    stage = 'RESOURCE_MATRIX';
    const result = await runResourceCases({ fixtureGuardVerified: true, mint, types, change, restore,
    read: async (code, bearer) => request((await login('admin')).page,
      BUSINESS + (code === 'stores' ? '/stores-api/stores' : '/work-orders-api/tickets'), { bearer }),
    checkpoint: value => { report.checks.push(value); persist(); console.log(`PHASE25_RESOURCE_CHECK ${value.caseId} PASS`); } });
    report.scopeLimitations = result.limitations; report.result = 'PASS';
  }
} catch (error) {
  report.failedStep = stage;
  const code = String(error?.message ?? '').split(/\r?\n/, 1)[0].trim();
  report.failureCode = /^PHASE25_RESOURCE_[A-Z0-9_]+$/.test(code) ? code
    : error?.name === 'TimeoutError' ? 'PHASE25_RESOURCE_BROWSER_TIMEOUT' : 'PHASE25_RESOURCE_STEP_FAILED';
  process.exitCode = 1;
} finally {
  // The reusable cases restore in their finally. Also handle failures before
  // they take control; an unreadable or changed journal is not reported clean.
  if (verified && !captureOnly && !report.policyCleanup) {
    try { await restore(); } catch { report.policyCleanup = { policyRestored: false }; report.result = 'FAIL'; process.exitCode = 1; }
  }
  await Promise.allSettled([...observations]);
  report.cleanup = await closeBusinessBrowser(browser, contexts);
  report.tlsProtocols = [...tls].sort(); report.finishedAtUtc = new Date().toISOString();
  if (report.result === 'PASS' && (!report.cleanup.browserClosed || !report.cleanup.contextsClosed
    || report.cleanup.failures || !report.policyCleanup?.policyRestored || !report.tlsProtocols.includes('TLS 1.3'))) {
    report.result = 'FAIL'; report.failedStep = 'FINAL_TLS_AND_CLEANUP';
    report.failureCode = 'PHASE25_RESOURCE_TLS_OR_CLEANUP_FAILED'; process.exitCode = 1;
  }
  if (captureAttempted && report.result !== 'PASS') {
    try { report.failedCaptureCleanup = control('clear-recovery-capture'); }
    catch { report.failedCaptureCleanup = { remainingCarrierFiles: 'UNKNOWN' }; process.exitCode = 1; }
  }
  persist(); fs.closeSync(fd);
  console.log(`PHASE25_RESOURCE_RESULT ${JSON.stringify({ result: report.result, checks: report.checks.length, failureCode: report.failureCode })}`);
}
