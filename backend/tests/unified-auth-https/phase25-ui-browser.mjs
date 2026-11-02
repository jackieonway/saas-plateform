/** Current-version UI checks against the owned, real HTTPS services.
 * Run before Phase23's MFA enrollment/recovery. No response interception, TLS
 * bypass, database mutation, trace, storage-state export or secret screenshot.
 */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { selectionSnapshot, selectionFacts } from './phase23-identity-driver.mjs';
import { withCookiesBlocked, checkBlockedCookieLogin } from './phase25-cookie-policy.mjs';

const TASK = '/tmp/rust-saas-unified-auth-20261028';
const AUTH = 'https://auth.saas.test';
const CONSOLE = 'https://console.saas.test';
const CUSTOMER = 'https://customer.saas.test';
const XSS_NAME = '<img src=x onerror=window.__phase25_xss=1>';
const here = path.dirname(fileURLToPath(import.meta.url));
const repository = path.resolve(here, '../../..');
const argumentsMap = {};
for (let index = 2; index < process.argv.length; index += 2) {
  const name = process.argv[index], value = process.argv[index + 1];
  assert(['--snapshot', '--output'].includes(name) && value && !Object.hasOwn(argumentsMap, name), 'PHASE25_UI_ARGUMENTS_INVALID');
  argumentsMap[name] = value;
}
const label = argumentsMap['--snapshot'];
assert(/^phase25-tools-r[1-9][0-9]*$/.test(label ?? ''), 'PHASE25_UI_FROZEN_TOOLS_REQUIRED');
const output = path.resolve(argumentsMap['--output'] ?? '');
assert(path.dirname(output) === path.join(repository, 'backend/docs/designs/unified-auth/validation')
  && /^phase-25-ui-[a-z0-9-]+\.json$/.test(path.basename(output)), 'PHASE25_UI_SAFE_OUTPUT_REQUIRED');
assert(!fs.lstatSync(path.dirname(output)).isSymbolicLink(), 'PHASE25_UI_OUTPUT_LINK_REJECTED');
const frozen = `${TASK}/source-snapshot-${label}`;
const hash = value => createHash('sha256').update(value).digest('hex');
const privateCanaries = new Set();
const contexts = [], pendingObservations = new Set(), tlsProtocols = new Set(), observedPages = new WeakSet();
const selectionObservations = new WeakMap();
const report = {
  contractVersion: 'unified-auth-phase25-real-ui/1', startedAtUtc: new Date().toISOString(),
  result: 'FAILED', sourceSnapshot: label, tlsErrorsIgnored: false, responseInterception: false,
  databaseMutations: false, screenshotsSaved: false, tracesSaved: false, storageStateExported: false,
  checks: [], endpoints: {}, cleanup: { browserClosed: false, contextsClosed: false },
  pageErrors: 0, pageErrorStages: {},
  scope: 'Task-owned source environment and current real HTTPS UI, including one explicit Chromium BLOCK_ALL cookie-policy probe. Other browsers and formal deployment remain outside this result.',
};
let browser, policyContext, environment, manifest, activeStep = 'OWNED_PREFLIGHT', interrupted = false;
const outputDescriptor = fs.openSync(output, 'wx', 0o600);

function canaries(value, field = '') {
  if (Array.isArray(value)) for (const child of value) canaries(child, field);
  else if (value && typeof value === 'object') for (const [name, child] of Object.entries(value)) canaries(child, name);
  else if (typeof value === 'string' && value.length >= 8
    && /password|secret|csrf|token|cookie|verifier|nonce|privatekey|username|^value$/i.test(field)) privateCanaries.add(value);
}
function persist() {
  const serialized = JSON.stringify(report, null, 2) + '\n';
  assert(!/"(?:access_token|refresh_token|id_token|logout_token|password|clientSecret|cookie|verifier)"\s*:/i.test(serialized), 'PHASE25_UI_REPORT_SECRET_FIELD');
  for (const value of privateCanaries) assert(!serialized.includes(value), 'PHASE25_UI_REPORT_SECRET_VALUE');
  const bytes = Buffer.from(serialized);
  fs.ftruncateSync(outputDescriptor, 0); fs.writeSync(outputDescriptor, bytes, 0, bytes.length, 0); fs.fsyncSync(outputDescriptor);
}
function pass(detail = {}) {
  report.checks.push({ scenario: activeStep, result: 'PASS', ...detail }); persist();
  console.log(`PHASE25_UI_CHECK ${activeStep} PASS`);
}
function pending(detail) { report.checks.push({ scenario: activeStep, result: 'PENDING', reason: detail }); persist(); }
function safeFailure(error) {
  if (interrupted) return 'PHASE25_UI_INTERRUPTED';
  const label = String(error?.message ?? '').split(/\r?\n/, 1)[0].trim();
  if (/^PHASE25_UI_[A-Z0-9_]{1,100}$/.test(label)) return label;
  if (error?.name === 'TimeoutError') return 'PHASE25_UI_BROWSER_TIMEOUT';
  return 'PHASE25_UI_STEP_FAILED';
}
function wsl(command, input) {
  try {
    return execFileSync('wsl.exe', ['-d', 'Ubuntu', '-u', 'jackie', '--exec', ...command], {
      input, encoding: 'utf8', windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'], timeout: 45000, maxBuffer: 8 * 1024 * 1024,
    });
  } catch { throw new Error('PHASE25_UI_PRIVATE_PREFLIGHT_FAILED'); }
}

// Every private file is read through the current cluster's descriptor-checked
// ownership binding. The fixed psql query is read-only and emits no identifiers
// or credentials except the nonsecret snapshot digest returned to the report.
const preflightProgram = String.raw`
import hashlib,json,pathlib,re,subprocess,sys
root=pathlib.Path(sys.argv[1]);label=sys.argv[2];snapshot=root/('source-snapshot-'+label)
assert re.fullmatch(r'phase25-tools-r[1-9][0-9]*',label) and snapshot.resolve(strict=True)==snapshot
tools=snapshot/'saas-plateform/backend/tests/unified-auth-https'
sys.path.insert(0,str(tools));import phase23_cluster_binding as binding
bound=binding.load_binding(root);uid=bound['taskUid']
def private(p,maximum=8*1024*1024):
 assert p.resolve(strict=True)==p and not p.is_symlink()
 return binding.protected_read(p,uid=uid,mode=0o600,maximum=maximum)[0]
source=json.loads(private(root/('snapshot-'+label+'.json')));inputs=source['inputs']
assert type(inputs) is dict and source['snapshotSha256']==hashlib.sha256(json.dumps(inputs,sort_keys=True).encode()).hexdigest()
for relative in ['saas-plateform/backend/tests/unified-auth-https/phase25-ui-browser.mjs','saas-plateform/backend/tests/unified-auth-https/phase25-cookie-policy.mjs','saas-plateform/backend/tests/unified-auth-https/phase23-identity-driver.mjs','saas-plateform/backend/tests/unified-auth-https/phase23_cluster_binding.py']:
 assert hashlib.sha256(private(snapshot/relative)).hexdigest()==inputs[relative]
env=json.loads(private(root/'https-environment.json'));manifest=json.loads(private(root/'https-database.json'))
name=manifest['database'];deployment=env['ids']['deployment']
assert re.fullmatch(r'unified_auth_https_[0-9a-f]{32}',name) and re.fullmatch(r'[a-f0-9]{8}-[a-f0-9]{4}-7[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}',deployment)
assert manifest['clusterId']==bound['systemIdentifier']
query="SELECT current_database()='"+name+"' AND "+binding.sql_guard(bound)+" AND (SELECT shobj_description(oid,'pg_database') FROM pg_database WHERE datname=current_database())='identity-deployment/1/"+deployment+"';"
command=['/usr/lib/postgresql/18/bin/psql','-X','-qAt','-v','ON_ERROR_STOP=1','-h','127.0.0.1','-p','55494','-U','postgres','-d',name]
result=subprocess.run(command,input=query.encode(),capture_output=True,timeout=15,env={'PATH':'/usr/bin:/bin','PGCONNECT_TIMEOUT':'5','PGOPTIONS':'-c default_transaction_read_only=on -c log_statement=none -c log_min_error_statement=panic'})
assert result.returncode==0 and result.stdout.strip()==b't'
print(json.dumps({'environment':env,'manifest':manifest,'snapshotSha256':source['snapshotSha256'],'inputs':{name:inputs[name] for name in inputs if name.endswith('/phase25-ui-browser.mjs') or name.endswith('/phase23-identity-driver.mjs') or name.endswith('/phase25-cookie-policy.mjs')},'currentClusterVerified':True}))
`;

function observe(page) {
  if (observedPages.has(page)) return page;
  observedPages.add(page);
  // Error messages can contain response or credential material. Preserve only
  // a count and the fixed current test stage, and fail the final browser gate.
  page.on('pageerror', () => {
    report.pageErrors++;
    report.pageErrorStages[activeStep] = (report.pageErrorStages[activeStep] ?? 0) + 1;
  });
  page.on('dialog', dialog => { report.unexpectedScriptDialogs = (report.unexpectedScriptDialogs ?? 0) + 1; void dialog.dismiss(); });
  page.on('response', response => {
    const operation = (async () => {
      const url = new URL(response.url());
      if (![AUTH, CONSOLE, CUSTOMER].includes(url.origin)) return;
      const safePath = url.pathname.replace(/\/[a-f0-9-]{36}(?=\/|$)/g, '/:id');
      if (/^\/(?:auth|oauth|api\/sso)\//.test(safePath)) {
        const endpoint = `${response.request().method()} ${url.origin}${safePath} ${response.status()}`;
        report.endpoints[endpoint] = (report.endpoints[endpoint] ?? 0) + 1;
      }
      if (url.origin === AUTH && response.request().method() === 'GET' && response.status() === 200
        && /^\/oauth\/interaction\/[a-f0-9-]{36}$/.test(url.pathname)) {
        const value = await response.json().catch(() => null); canaries(value);
        const snapshot = selectionSnapshot(value);
        if (snapshot) selectionObservations.set(page, { flow: url.pathname.split('/').at(-1), snapshot });
      }
      const security = await response.securityDetails().catch(() => null); if (security?.protocol) tlsProtocols.add(security.protocol);
    })();
    pendingObservations.add(operation); operation.finally(() => pendingObservations.delete(operation)).catch(() => {});
  });
  page.setDefaultTimeout(25000); page.setDefaultNavigationTimeout(40000);
  return page;
}
async function newPage({ mobile = false, context } = {}) {
  if (!context) {
    context = await browser.newContext({ viewport: mobile ? { width: 390, height: 844 } : { width: 1440, height: 960 }, serviceWorkers: 'block', reducedMotion: 'reduce' });
    contexts.push(context);
  }
  return observe(await context.newPage());
}
async function browserJson(page, endpoint, { method = 'GET', body, csrf, omit = false } = {}) {
  assert(['/auth/session', '/auth/login-bootstrap', '/auth/login', '/api/profile', '/api/sso/session'].includes(endpoint), 'PHASE25_UI_FIXED_ENDPOINT_REQUIRED');
  const result = await page.evaluate(async input => {
    const response = await fetch(input.endpoint, { method: input.method, cache: 'no-store', redirect: 'error',
      credentials: input.omit ? 'omit' : 'same-origin', headers: { Accept: 'application/json',
        ...(input.body === undefined ? {} : { 'Content-Type': 'application/json' }), ...(input.csrf ? { 'X-CSRF-Token': input.csrf } : {}) },
      ...(input.body === undefined ? {} : { body: JSON.stringify(input.body) }) });
    let body = null; try { body = await response.json(); } catch {}
    return { status: response.status, body, noStore: response.headers.get('cache-control')?.includes('no-store') === true };
  }, { endpoint, method, body, csrf, omit });
  canaries(result.body); return result;
}
async function session(page, actor, memberType, membershipId) {
  const result = await browserJson(page, '/auth/session');
  assert.equal(result.status, 200, 'PHASE25_UI_SESSION_STATUS_FAILED');
  assert(result.noStore && result.body.account?.id === environment.ids[actor]
    && result.body.context?.memberType === memberType && result.body.context.membershipId === membershipId,
  'PHASE25_UI_SESSION_SOURCE_FAILED');
  assert(!/"(?:accessToken|refreshToken|idToken|access_token|refresh_token|id_token)"\s*:/.test(JSON.stringify(result.body)), 'PHASE25_UI_SESSION_CREDENTIAL_EXPOSED');
  return result.body;
}
async function noOverflow(page) {
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), 'PHASE25_UI_HORIZONTAL_OVERFLOW');
}
const xssLayers = new Set();
async function checkXss(page, layer) {
  if (environment.phase25Ui?.hostileName !== XSS_NAME) return;
  assert(await page.locator('body').textContent().then(value => value.includes(XSS_NAME)), 'PHASE25_UI_HOSTILE_NAME_NOT_RENDERED');
  assert.equal(await page.locator('img[src="x"]').count(), 0, 'PHASE25_UI_HOSTILE_MARKUP_CREATED');
  assert(await page.evaluate(() => window.__phase25_xss === undefined), 'PHASE25_UI_HOSTILE_HANDLER_EXECUTED');
  assert.equal(report.unexpectedScriptDialogs ?? 0, 0, 'PHASE25_UI_UNEXPECTED_SCRIPT_DIALOG');
  xssLayers.add(layer);
}
async function identity(page, actor, destination, membershipId, { keyboard = false, expectSso = false } = {}) {
  const account = environment.accounts[actor];
  assert(account && typeof account.username === 'string' && typeof account.password === 'string', 'PHASE25_UI_ACTOR_REQUIRED');
  await page.waitForFunction(dest => location.origin === dest || !!document.querySelector('#login-form:not([hidden])')
    || ['CONSENT_REQUIRED', 'CONTEXT_SELECTION_REQUIRED'].includes(document.querySelector('#auth-content')?.dataset.screen)
    || !!document.querySelector('#error-message:not([hidden])'), destination);
  if (new URL(page.url()).origin === AUTH && await page.locator('#login-form:not([hidden])').isVisible()) {
    assert(!expectSso, 'PHASE25_UI_EXISTING_SSO_NOT_REUSED');
    await noOverflow(page);
    if (keyboard) {
      await page.locator('#login-submit').focus(); await page.keyboard.press('Enter');
      assert(await page.locator('#username').evaluate(element => element === document.activeElement && element.getAttribute('aria-invalid') === 'true'), 'PHASE25_UI_INVALID_INPUT_FOCUS_FAILED');
      await page.locator('#username').fill(account.username); await page.keyboard.press('Tab');
      assert(await page.locator('#password').evaluate(element => element === document.activeElement), 'PHASE25_UI_LOGIN_TAB_ORDER_FAILED');
    } else await page.locator('#username').fill(account.username);
    await page.locator('#password').fill(account.password);
    const accepted = page.waitForResponse(response => response.url() === `${AUTH}/api/sso/login` && response.request().method() === 'POST');
    if (keyboard) await page.keyboard.press('Enter'); else await page.locator('#login-submit').click();
    assert.equal((await accepted).status(), 200, 'PHASE25_UI_PRIMARY_LOGIN_OR_PRE_MFA_ORDER_FAILED');
  }
  for (let transition = 0; transition < 10; transition++) {
    const handle = await page.waitForFunction(dest => {
      if (location.origin === dest && !location.pathname.startsWith('/auth/')) return 'destination';
      if (document.querySelector('#error-message:not([hidden])')) return 'error';
      if (document.querySelector('#auth-content')?.dataset.screen === 'CONTEXT_SELECTION_REQUIRED' && document.querySelector('#context-selection')?.hidden === false) return 'selection';
      if (document.querySelector('#auth-content')?.dataset.screen === 'CONSENT_REQUIRED' && document.querySelector('#consent-panel')?.hidden === false && document.querySelector('#consent-allow')?.disabled === false) return 'consent';
      return false;
    }, destination);
    const outcome = await handle.jsonValue(); await handle.dispose();
    assert.notEqual(outcome, 'error', 'PHASE25_UI_IDENTITY_REJECTED');
    if (outcome === 'destination') return;
    if (outcome === 'selection') {
      const flow = new URL(page.url()).searchParams.get('flow');
      await page.waitForFunction(() => document.querySelector('#organization-choices')?.hidden === false || location.origin !== 'https://auth.saas.test');
      if (new URL(page.url()).origin !== AUTH) continue;
      const deadline = Date.now() + 10000;
      while (selectionObservations.get(page)?.flow !== flow) {
        assert(Date.now() < deadline, 'PHASE25_UI_SELECTION_RESPONSE_NOT_OBSERVED'); await new Promise(resolve => setTimeout(resolve, 50));
      }
      const facts = selectionFacts(selectionObservations.get(page).snapshot, {
        accountId: environment.ids[actor], membershipId, memberType: destination === CUSTOMER ? 'CUSTOMER' : 'EMPLOYEE',
      });
      assert(facts.valid && facts.accountMatches && facts.memberAccountsMatch && facts.expectedMemberTypeOnly
        && facts.expectedMembershipMatches === 1 && facts.tenantCount === 1, 'PHASE25_UI_SELECTION_SOURCE_FAILED');
      if (facts.candidateCount === 1) {
        await page.waitForFunction(() => location.origin !== 'https://auth.saas.test'
          || document.querySelector('#auth-content')?.dataset.screen !== 'CONTEXT_SELECTION_REQUIRED'
          || !!document.querySelector('#error-message:not([hidden])'));
        continue;
      }
      await noOverflow(page); await checkXss(page, 'identity-context-selection');
      const choice = page.locator(`input[name="context-membership"][value="${membershipId}"]`);
      await choice.focus(); await page.keyboard.press('Space');
      assert(await choice.isChecked(), 'PHASE25_UI_KEYBOARD_MEMBER_SELECTION_FAILED');
    }
    await page.locator('#consent-allow').focus(); await page.keyboard.press('Enter');
    await page.waitForFunction(previous => location.origin !== 'https://auth.saas.test'
      || document.querySelector('#auth-content')?.dataset.screen !== previous
      || !!document.querySelector('#error-message:not([hidden])'), outcome === 'selection' ? 'CONTEXT_SELECTION_REQUIRED' : 'CONSENT_REQUIRED');
  }
  assert.fail('PHASE25_UI_IDENTITY_TRANSITION_LIMIT');
}
async function consoleLogin(page, actor, keyboard = false, returnPath = '/system/oauth/clients') {
  assert(['/system/oauth/clients', '/'].includes(returnPath), 'PHASE25_UI_FIXED_LOGIN_RETURN_REQUIRED');
  await page.goto(`${CONSOLE}/auth/login?${new URLSearchParams({ return: returnPath })}`, { waitUntil: 'domcontentloaded' });
  await identity(page, actor, CONSOLE, environment.ids[`${actor}Member`], { keyboard });
  return session(page, actor, 'EMPLOYEE', environment.ids[`${actor}Member`]);
}
async function portalLogin(page, actor, membershipId, expectSso = false) {
  await page.goto(CUSTOMER, { waitUntil: 'domcontentloaded' });
  await page.getByTestId('portal-login').waitFor();
  await Promise.all([page.waitForURL(url => url.origin === AUTH, { waitUntil: 'domcontentloaded' }), page.getByTestId('portal-login').click()]);
  await identity(page, actor, CUSTOMER, membershipId, { expectSso });
  await page.getByTestId('portal-profile').waitFor();
  return session(page, actor, 'CUSTOMER', membershipId);
}
async function dialogChecks(page) {
  await page.setViewportSize({ width: 390, height: 844 }); await noOverflow(page);
  const opener = page.getByRole('button', { name: '登记客户端', exact: true }); await opener.click();
  const dialog = page.getByRole('dialog'); await dialog.getByRole('heading', { name: '登记新客户端', exact: true }).waitFor();
  const dialogHandle = await dialog.elementHandle();
  try { await page.waitForFunction(element => element.contains(document.activeElement), dialogHandle); }
  finally { await dialogHandle?.dispose(); }
  assert(await dialog.evaluate(element => element.contains(document.activeElement)), 'PHASE25_UI_DIALOG_INITIAL_FOCUS_OUTSIDE');
  const bounds = await dialog.boundingBox();
  assert(bounds && bounds.x >= -1 && bounds.y >= -1 && bounds.x + bounds.width <= 391 && bounds.y + bounds.height <= 845, 'PHASE25_UI_DIALOG_VIEWPORT_FAILED');
  const scroll = dialog.locator('form .overflow-y-auto').first();
  const geometry = await scroll.evaluate(element => ({ client: element.clientHeight, content: element.scrollHeight }));
  assert(geometry.content > geometry.client && geometry.client > 0, 'PHASE25_UI_DIALOG_INNER_SCROLL_REQUIRED');
  await scroll.evaluate(element => { element.scrollTop = element.scrollHeight; });
  assert(await scroll.evaluate(element => element.scrollTop > 0), 'PHASE25_UI_DIALOG_SCROLL_DID_NOT_MOVE');
  const actionBounds = await dialog.getByRole('button', { name: '创建客户端', exact: true }).boundingBox();
  assert(actionBounds && actionBounds.y >= 0 && actionBounds.y + actionBounds.height <= 844, 'PHASE25_UI_DIALOG_ACTION_BAR_HIDDEN');
  const focusableCount = await dialog.locator('button:not([disabled]),input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex="0"]').count();
  assert(focusableCount > 2 && focusableCount < 100, 'PHASE25_UI_DIALOG_FOCUSABLE_SET_INVALID');
  for (const direction of ['Tab', 'Shift+Tab']) {
    for (let index = 0; index < focusableCount + 2; index++) {
      await page.keyboard.press(direction);
      assert(await dialog.evaluate(element => element.contains(document.activeElement)), 'PHASE25_UI_DIALOG_FOCUS_ESCAPED');
    }
  }
  await page.keyboard.press('Escape'); await dialog.waitFor({ state: 'hidden' });
  const openerHandle = await opener.elementHandle();
  try { await page.waitForFunction(element => element === document.activeElement, openerHandle); }
  finally { await openerHandle?.dispose(); }
  assert(await opener.evaluate(element => element === document.activeElement), 'PHASE25_UI_DIALOG_FOCUS_NOT_RESTORED');
  await noOverflow(page);
}
function interrupt() { interrupted = true; void browser?.close().catch(() => {}); void policyContext?.close().catch(() => {}); }
process.on('SIGINT', interrupt); process.on('SIGTERM', interrupt);

try {
  const preflight = JSON.parse(wsl(['python3', '-B', '-c', preflightProgram, TASK, label]));
  environment = preflight.environment; manifest = preflight.manifest; canaries(environment);
  for (const [relative, digest] of Object.entries(preflight.inputs)) {
    const filename = path.join(here, path.posix.basename(relative));
    assert.equal(hash(fs.readFileSync(filename)), digest, 'PHASE25_UI_WINDOWS_RUNNER_NOT_FROZEN');
  }
  for (const kind of ['identity', 'platform', 'portal']) {
    const url = new URL(environment[kind].DATABASE_URL);
    assert(url.hostname === '127.0.0.1' && url.port === '55494'
      && url.pathname === `/${kind === 'portal' ? manifest.portal.database : manifest.database}`, 'PHASE25_UI_SOURCE_DATABASE_BINDING_FAILED');
  }
  assert(environment.phase19?.enabled === true && environment.ids.adminCustomerMember, 'PHASE25_UI_FULL_SOURCE_FIXTURE_REQUIRED');
  report.snapshotSha256 = preflight.snapshotSha256; report.currentOwnedClusterVerified = preflight.currentClusterVerified;
  report.runnerSha256 = hash(fs.readFileSync(fileURLToPath(import.meta.url))); pass();
  process.env.PLAYWRIGHT_BROWSERS_PATH = path.join(repository, 'frontend/tests/browser/.browsers');
  const require = createRequire(path.join(repository, 'frontend/tests/browser/package.json'));
  const { chromium } = require('playwright'); report.playwrightVersion = require('playwright/package.json').version;
  browser = await chromium.launch({ headless: true, proxy: { server: 'http://127.0.0.1:24445' } }); report.browserVersion = browser.version();

  activeStep = 'DESKTOP_IDENTITY_KEYBOARD_AND_PLATFORM';
  const adminPage = await newPage(); const consoleBefore = await consoleLogin(adminPage, 'admin', true);
  await adminPage.getByRole('button', { name: '登记客户端', exact: true }).waitFor(); await noOverflow(adminPage);
  await checkXss(adminPage, 'platform-context');
  const sharedContext = adminPage.context();
  const sourceCookies = await sharedContext.cookies([AUTH, CONSOLE]); canaries(sourceCookies);
  for (const [name, domain] of [['__Host-saas_sso', 'auth.saas.test'], ['__Host-saas_console', 'console.saas.test']]) {
    const cookie = sourceCookies.find(value => value.name === name);
    assert(cookie?.domain === domain && cookie.secure && cookie.httpOnly && cookie.sameSite === 'Lax' && cookie.path === '/', 'PHASE25_UI_HOST_COOKIE_CONTRACT_FAILED');
  }
  pass({ viewportWidth: 1440, invalidFieldFocus: true, tabOrder: true, enterSubmission: true, currentEmployeeSession: true });

  activeStep = 'MOBILE_PLATFORM_DIALOG_SCROLL_FOCUS_AND_ESCAPE'; await dialogChecks(adminPage);
  pass({ viewportWidth: 390, initialDialogFocusBeforeTab: true, internalFormScroll: true, actionBarReachable: true,
    keyboardFocusContained: true, escapeRestoresTrigger: true, writesSubmitted: 0 });

  activeStep = 'SHARED_SSO_WITH_INDEPENDENT_PORTAL_UI';
  const adminPortal = await newPage({ context: sharedContext });
  const portalBefore = await portalLogin(adminPortal, 'admin', environment.ids.adminCustomerMember, true); await noOverflow(adminPortal);
  await adminPortal.setViewportSize({ width: 390, height: 844 }); await noOverflow(adminPortal); await checkXss(adminPortal, 'portal-profile');
  const currentCookies = await sharedContext.cookies([AUTH, CONSOLE, CUSTOMER]); canaries(currentCookies);
  assert.equal(currentCookies.find(value => value.name === '__Host-saas_sso')?.value,
    sourceCookies.find(value => value.name === '__Host-saas_sso')?.value, 'PHASE25_UI_PORTAL_REPLACED_SSO');
  for (const name of ['__Host-saas_customer', '__Host-saas_customer_browser']) {
    const cookie = currentCookies.find(value => value.name === name);
    assert(cookie?.domain === 'customer.saas.test' && cookie.secure && cookie.httpOnly && cookie.path === '/', 'PHASE25_UI_PORTAL_HOST_COOKIE_FAILED');
  }
  pass({ reusedSso: true, employeeAndCustomerSessionsSeparate: true, desktopAnd390pxPortal: true });

  activeStep = 'MOBILE_IDENTITY_CONTEXT_AND_CUSTOMER_PROFILE';
  const customerPage = await newPage({ mobile: true });
  await portalLogin(customerPage, 'customer', environment.ids.customerMember); await noOverflow(customerPage); await checkXss(customerPage, 'portal-profile');
  pass({ viewportWidth: 390, canonicalIdentityFlow: true, customerMembershipChecked: true });

  activeStep = 'HOSTILE_ORGANIZATION_NAME_RENDERS_AS_TEXT';
  if (environment.phase25Ui?.contractVersion === 'unified-auth-phase25-ui-fixture/1'
    && environment.phase25Ui.hostileName === XSS_NAME
    && ['platform-context', 'portal-profile', 'identity-context-selection'].every(layer => xssLayers.has(layer))) {
    pass({ realServiceLayers: [...xssLayers].sort(), injectedElements: 0, scriptExecution: false });
  } else pending('OWNED_HOSTILE_ORGANIZATION_AND_MULTI_CUSTOMER_SELECTION_FIXTURE_REQUIRED');

  activeStep = 'REAL_BFF_OPEN_REDIRECT_INPUTS_REJECTED';
  const redirectPage = await newPage();
  const unsafe = ['https://phase25-redirect.invalid/', '//phase25-redirect.invalid/', '/\\phase25-redirect.invalid/',
    '/%252f%252fphase25-redirect.invalid/', '/auth/callback', '/#unsafe', 'javascript:window.__phase25_xss=1'];
  for (const value of unsafe) {
    const response = await redirectPage.goto(`${CONSOLE}/auth/login?${new URLSearchParams({ return: value })}`, { waitUntil: 'domcontentloaded' });
    assert.equal(response.status(), 400, 'PHASE25_UI_CONSOLE_OPEN_REDIRECT_ACCEPTED');
    assert.equal(new URL(redirectPage.url()).origin, CONSOLE, 'PHASE25_UI_CONSOLE_LEFT_OWNED_ORIGIN');
  }
  await redirectPage.goto(CUSTOMER, { waitUntil: 'domcontentloaded' });
  const bootstrap = await browserJson(redirectPage, '/auth/login-bootstrap');
  assert.equal(bootstrap.status, 200, 'PHASE25_UI_PORTAL_BOOTSTRAP_FAILED');
  for (const value of unsafe) {
    const result = await browserJson(redirectPage, '/auth/login', { method: 'POST', body: { returnTo: value }, csrf: bootstrap.body.csrfToken });
    assert.equal(result.status, 400, 'PHASE25_UI_PORTAL_OPEN_REDIRECT_ACCEPTED');
  }
  pass({ invalidConsoleReturns: unsafe.length, invalidPortalReturns: unsafe.length, externalNavigations: 0 });

  activeStep = 'MISSING_COOKIES_FAIL_CLOSED_WITH_INDEPENDENT_APP_PRESERVED';
  // Stop UI refresh owners before the explicit cookie checks; this also prevents
  // a host redirect from racing the fixed same-origin session request.
  await adminPage.goto(`${CONSOLE}/auth/config`, { waitUntil: 'domcontentloaded' });
  await adminPortal.goto(`${CUSTOMER}/auth/config`, { waitUntil: 'domcontentloaded' });
  const omittedConsole = await browserJson(adminPage, '/auth/session', { omit: true });
  assert.equal(omittedConsole.status, 401, 'PHASE25_UI_OMITTED_CONSOLE_COOKIE_ACCEPTED');
  const omittedPortal = await browserJson(adminPortal, '/auth/session', { omit: true });
  assert(omittedPortal.status === 200 && omittedPortal.body.account === null && omittedPortal.body.context === null, 'PHASE25_UI_OMITTED_PORTAL_COOKIE_AUTHENTICATED');
  assert.equal((await browserJson(adminPortal, '/api/profile', { omit: true })).status, 401, 'PHASE25_UI_OMITTED_PORTAL_PROFILE_COOKIE_ACCEPTED');
  await sharedContext.clearCookies({ name: '__Host-saas_console', domain: 'console.saas.test' });
  assert.equal((await browserJson(adminPage, '/auth/session')).status, 401, 'PHASE25_UI_CLEARED_CONSOLE_COOKIE_ACCEPTED');
  const unchangedPortal = await session(adminPortal, 'admin', 'CUSTOMER', environment.ids.adminCustomerMember);
  assert.equal(unchangedPortal.sessionId, portalBefore.sessionId, 'PHASE25_UI_OTHER_APP_SESSION_CHANGED');
  await consoleLogin(adminPage, 'admin');
  assert((await sharedContext.cookies(CONSOLE)).some(value => value.name === '__Host-saas_console'), 'PHASE25_UI_EXPLICIT_SSO_RECOVERY_FAILED');
  await session(adminPortal, 'admin', 'CUSTOMER', environment.ids.adminCustomerMember);
  pass({ consoleCookieOmittedStatus: 401, portalCookieOmittedAnonymousStatus: 200, portalProfileCookieOmittedStatus: 401,
    selectedConsoleCookieCleared: true, originalPortalSessionPreserved: true, explicitSsoRecovery: true,
    browserWideCookieBlockingTested: false, initialConsoleSessionObserved: typeof consoleBefore.sessionId === 'string' });

  activeStep = 'CHROMIUM_BLOCK_ALL_COOKIES_FAILS_CLOSED_WITH_BOUNDED_LOGIN';
  const cookieRestriction = await withCookiesBlocked(chromium,
    { proxy: { server: 'http://127.0.0.1:24445' } }, async context => {
      policyContext = context;
      return checkBlockedCookieLogin(context, { authOrigin: AUTH, consoleOrigin: CONSOLE,
        account: environment.accounts.employee, observePage: observe,
        progress: (stage, facts) => { report.cookiePolicyObservation = { stage, ...facts }; persist(); } });
    }, async context => {
      policyContext = context;
      const restoredPage = await newPage({ context });
      await consoleLogin(restoredPage, 'employee', false, '/');
      await restoredPage.goto(`${CONSOLE}/auth/config`, { waitUntil: 'domcontentloaded' });
      const restoredCookies = await context.cookies([AUTH, CONSOLE]); canaries(restoredCookies);
      assert(restoredCookies.some(cookie => cookie.name === '__Host-saas_sso')
        && restoredCookies.some(cookie => cookie.name === '__Host-saas_console'), 'PHASE25_UI_COOKIE_PERMISSION_RECOVERY_FAILED');
      return { explicitCookiePermission: 'ALLOW', freshPrimaryLoginSucceeded: true, employeeBffSessionEstablished: true };
    });
  policyContext = undefined;
  pass({ ...cookieRestriction, browserWideCookieBlockingTested: true, privateBrowserProfileRemoved: true });
  assert.equal(report.unexpectedScriptDialogs ?? 0, 0, 'PHASE25_UI_UNEXPECTED_SCRIPT_DIALOG');
  assert.equal(report.pageErrors, 0, 'PHASE25_UI_UNEXPECTED_PAGE_ERROR');
  report.result = report.checks.some(check => check.result === 'PENDING') ? 'PARTIAL' : 'PASS';
  if (report.result === 'PARTIAL') process.exitCode = 1;
} catch (error) {
  report.result = 'FAILED'; report.failedStep = activeStep; report.failureCode = safeFailure(error); process.exitCode = 1;
} finally {
  if (policyContext) {
    await policyContext.close().catch(() => {});
    report.cleanup.cookiePolicyContextClosed = policyContext.pages().length === 0;
    if (!report.cleanup.cookiePolicyContextClosed) { report.result = 'FAILED'; process.exitCode = 1; }
  }
  for (const context of contexts) await context.close().catch(() => {});
  report.cleanup.contextsClosed = contexts.every(context => context.pages().length === 0);
  if (browser) { await browser.close().catch(() => {}); report.cleanup.browserClosed = !browser.isConnected(); }
  await Promise.allSettled([...pendingObservations]);
  report.tlsProtocols = [...tlsProtocols].sort(); report.finishedAtUtc = new Date().toISOString();
  if (report.result === 'PASS' || report.result === 'PARTIAL') {
    if (!report.tlsProtocols.includes('TLS 1.3') || !report.cleanup.contextsClosed || !report.cleanup.browserClosed || report.pageErrors !== 0) {
      report.result = 'FAILED'; report.failedStep = 'FINAL_TLS_AND_CLEANUP'; process.exitCode = 1;
      report.failureCode = report.pageErrors !== 0 ? 'PHASE25_UI_UNEXPECTED_PAGE_ERROR'
        : !report.tlsProtocols.includes('TLS 1.3') ? 'PHASE25_UI_REAL_TLS_NOT_OBSERVED' : 'PHASE25_UI_BROWSER_CLEANUP_FAILED';
    }
  }
  persist(); fs.closeSync(outputDescriptor);
  process.removeListener('SIGINT', interrupt); process.removeListener('SIGTERM', interrupt);
  console.log(`PHASE25_UI_RESULT ${JSON.stringify({ result: report.result, checks: report.checks.length, failedStep: report.failedStep, failureCode: report.failureCode })}`);
}
