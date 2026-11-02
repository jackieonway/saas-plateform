/** Real TLS/browser verification against prepare.py's owned disposable database.
 * No HTTP fixture, intercepted response, ignored TLS error, trace or storage-state
 * export is used. Credentials, Cookies, CSRF and response tokens stay in memory.
 */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { randomBytes } from 'node:crypto';

const here = path.dirname(fileURLToPath(import.meta.url));
const repository = path.resolve(here, '../../..');
const args = process.argv.slice(2);
const options = {};
for (let i = 0; i < args.length; i += 2) {
  assert(['--environment', '--output', '--proxy', '--browser'].includes(args[i]) && args[i + 1], 'INVALID_ARGUMENTS');
  assert(!Object.hasOwn(options, args[i]), 'DUPLICATE_ARGUMENT');
  options[args[i]] = args[i + 1];
}
assert(options['--environment'] && options['--output'], 'ENVIRONMENT_AND_OUTPUT_REQUIRED');
const environmentFile = options['--environment'];
assert(/^\/tmp\/rust-saas-unified-auth-20261028\/(?:[a-zA-Z0-9_.-]+\/)*https-environment\.json$/.test(environmentFile)
  && !environmentFile.split('/').includes('..'), 'OWNED_ENVIRONMENT_PATH_REQUIRED');
const outputFile = path.resolve(options['--output']);
assert(outputFile.endsWith('.json') && outputFile !== environmentFile, 'JSON_OUTPUT_REQUIRED');
const proxy = options['--proxy'] || 'http://127.0.0.1:24445';
assert(proxy === 'http://127.0.0.1:24445', 'OWNED_CONNECT_PROXY_REQUIRED');
const product = options['--browser'] || 'chromium';
assert(['chromium', 'chrome', 'msedge'].includes(product), 'UNSUPPORTED_BROWSER');

function linuxFile(filename) {
  if (process.platform === 'linux') return fs.readFileSync(filename, 'utf8');
  return execFileSync('wsl.exe', ['-u', 'root', '--', 'cat', filename],
    { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true, timeout: 10_000 });
}
function privateJson(filename) {
  try { return JSON.parse(linuxFile(filename)); }
  catch { throw new Error('OWNED_PRIVATE_JSON_UNAVAILABLE'); }
}
const environment = privateJson(environmentFile);
const manifest = privateJson(environmentFile.replace(/https-environment\.json$/, 'https-database.json'));
assert(/^unified_auth_https_[0-9a-f]{32}$/.test(manifest.database), 'OWNED_DATABASE_REQUIRED');
assert(/^[0-9]{10,24}$/.test(manifest.clusterId), 'OWNED_CLUSTER_REQUIRED');
const uuid7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const ids = environment.ids;
for (const id of Object.values(ids)) assert(uuid7.test(id), 'FIXTURE_UUID_REQUIRED');
let boundDatabase;
try { boundDatabase = new URL(environment.platform.DATABASE_URL).pathname; }
catch { throw new Error('OWNED_DATABASE_BINDING_INVALID'); }
assert(boundDatabase === `/${manifest.database}`, 'DATABASE_BINDING_REQUIRED');
const databaseUrl = `postgres://postgres@127.0.0.1:55494/${manifest.database}`;
const marker = `identity-deployment/1/${ids.deployment}`;

function sql(statement) {
  // The guard and mutation run in one transaction, on the exact fresh fixture.
  const input = `BEGIN; DO $$ BEGIN IF current_database() <> '${manifest.database}'
    OR (SELECT system_identifier::text FROM pg_control_system()) <> '${manifest.clusterId}'
    OR (SELECT shobj_description(oid,'pg_database') FROM pg_database WHERE datname=current_database()) IS DISTINCT FROM '${marker}'
    THEN RAISE EXCEPTION 'owned HTTPS database guard failed'; END IF; END $$;
    ${statement}; COMMIT;`;
  try {
    const command = process.platform === 'linux' ? 'psql' : 'wsl.exe';
    const prefix = process.platform === 'linux' ? [] : ['-u', 'root', '--', 'psql'];
    return execFileSync(command, [...prefix, '-X', '-q', '-v', 'ON_ERROR_STOP=1', '-v', 'VERBOSITY=sqlstate', '-At', databaseUrl],
      { input, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true, timeout: 15_000 }).trim();
  } catch (failure) {
    const error = new Error('OWNED_SQL_FAILED');
    const sqlState = /ERROR:\s+([0-9A-Z]{5})\b/.exec(String(failure.stderr || ''))?.[1];
    error.code = sqlState ? `SQLSTATE_${sqlState}` : 'OWNED_SQL_TRANSPORT_FAILED';
    throw error;
  }
}

// Existing tool/browser versions and cache belong to this repository.
process.env.PLAYWRIGHT_BROWSERS_PATH = path.join(repository, 'frontend/tests/browser/.browsers');
const require = createRequire(path.join(repository, 'frontend/tests/browser/package.json'));
const { chromium } = require('playwright');
const playwrightVersion = require('playwright/package.json').version;
const CONSOLE = 'https://console.saas.test';
const AUTH = 'https://auth.saas.test';
const report = { contractVersion: 'unified-auth-real-https-browser/1', date: new Date().toISOString(),
  browser: product, playwright: playwrightVersion, tlsErrorsIgnored: false, httpFixtures: false,
  sqlBoundarySetup: true, checks: [], endpoints: {}, result: 'FAILED' };
let step = 'owned-database-preflight';
let browser;
let interrupted = false;
const contexts = [];
const pendingObservations = new Set();
const seenPaths = new Set();
const tlsProtocols = new Set();
const interrupt = () => {
  interrupted = true;
  process.exitCode = 1;
  report.result = 'FAILED';
  report.failedStep = step;
  report.failureCode = 'BROWSER_INTERRUPTED';
  // Closing the live transport rejects an in-flight Playwright wait and enters
  // the same finally cleanup as a failed assertion, instead of exiting early.
  if (browser) void browser.close().catch(() => {});
};
process.on('SIGINT', interrupt);
process.on('SIGTERM', interrupt);

function safePath(raw) {
  const url = new URL(raw);
  if (![CONSOLE, AUTH].includes(url.origin)) return null;
  return url.pathname.replace(/\/[0-9a-f-]{36}(?=\/|$)/g, '/:id');
}
function observe(page) {
  page.on('response', response => {
    const endpoint = safePath(response.url());
    if (!endpoint) return;
    seenPaths.add(endpoint);
    if (/^\/(auth|oauth|api\/sso)\//.test(endpoint) || /^\/api\/users(?:\/|$)/.test(endpoint)) {
      const key = `${endpoint} ${response.status()}`;
      report.endpoints[key] = (report.endpoints[key] || 0) + 1;
    }
    if (response.request().isNavigationRequest()) {
      const observation = response.securityDetails().then(details => {
        if (details?.protocol) tlsProtocols.add(details.protocol);
      }).catch(() => {}).finally(() => pendingObservations.delete(observation));
      pendingObservations.add(observation);
    }
  });
}
function pass(name, detail = {}) { report.checks.push({ name, result: 'PASS', ...detail }); }
function secretFree(value) {
  const forbidden = new Set(['access_token', 'refresh_token', 'id_token', 'accessToken', 'refreshToken', 'idToken']);
  if (!value || typeof value !== 'object') return true;
  return Object.entries(value).every(([key, child]) => !forbidden.has(key) && secretFree(child));
}
async function inBrowser(page, endpoint, method = 'GET', body, csrf) {
  assert(['/auth/session', '/auth/context-token', '/auth/context-switch'].includes(endpoint), 'FIXED_BROWSER_ENDPOINT_REQUIRED');
  return page.evaluate(async ({ endpoint, method, body, csrf }) => {
    const response = await fetch(endpoint, { method, credentials: 'same-origin', cache: 'no-store', redirect: 'error',
      headers: { Accept: 'application/json', ...(body ? { 'Content-Type': 'application/json', 'X-CSRF-Token': csrf } : {}) },
      ...(body ? { body: JSON.stringify(body) } : {}) });
    return { status: response.status, noStore: response.headers.get('cache-control')?.includes('no-store') === true,
      body: await response.json() };
  }, { endpoint, method, body, csrf });
}
async function session(page, account) {
  const response = await inBrowser(page, '/auth/session');
  assert(response.status === 200 && response.noStore, 'REAL_SESSION_READ_FAILED');
  const view = response.body;
  assert(secretFree(view) && view.account?.id === ids[account], 'SESSION_ACCOUNT_OR_SECRET_CONTRACT_FAILED');
  assert(uuid7.test(view.sessionId) && /^[1-9][0-9]*$/.test(view.version), 'SESSION_VERSION_CONTRACT_FAILED');
  assert(view.centralizedContextSelection === true && view.context?.memberType === 'EMPLOYEE', 'EMPLOYEE_CONTEXT_REQUIRED');
  assert(view.context.membershipId === ids[`${account}Member`]
    || (account === 'employee' && view.context.organizationId === ids.secondOrganization), 'UNEXPECTED_CONTEXT');
  return view;
}
async function employeeApi(page, endpoint, method, body, bearer) {
  assert(endpoint === '/api/users' || /^\/api\/users\/[0-9a-f-]{36}\/memberships$/.test(endpoint), 'FIXED_EMPLOYEE_API_REQUIRED');
  return page.evaluate(async ({ endpoint, method, body, bearer }) => {
    const response = await fetch(endpoint, { method, credentials: 'same-origin', cache: 'no-store', redirect: 'error',
      headers: { Accept: 'application/json', 'Content-Type': 'application/json', Authorization: `Bearer ${bearer}` },
      body: JSON.stringify(body) });
    return { status: response.status, body: await response.json() };
  }, { endpoint, method, body, bearer });
}
async function newPage() {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, serviceWorkers: 'block' });
  contexts.push(context);
  const page = await context.newPage();
  page.setDefaultTimeout(20_000);
  page.setDefaultNavigationTimeout(35_000);
  observe(page);
  return page;
}
async function beginLogin(page, account) {
  await page.goto(`${CONSOLE}/auth/login?return=%2F`, { waitUntil: 'domcontentloaded' });
  await page.locator('#login-form:not([hidden])').waitFor();
  await page.locator('#username').fill(environment.accounts[account].username);
  await page.locator('#password').fill(environment.accounts[account].password);
  await page.locator('#login-submit').click();
  // A fresh consent requires this real user action; a valid saved consent can
  // continue automatically on a repeated run of the same isolated environment.
  const outcome = await page.waitForFunction(() => {
    if (location.origin === 'https://console.saas.test') return 'console';
    const screen = document.querySelector('#auth-content')?.dataset.screen;
    if (screen === 'CONSENT_REQUIRED') return 'consent';
    if (screen === 'CONTEXT_SELECTION_REQUIRED' && document.querySelector('#selection-empty:not([hidden])')) return 'empty';
    if (document.querySelector('#error-message:not([hidden])')) return 'error';
    return false;
  });
  const state = await outcome.jsonValue();
  await outcome.dispose();
  assert(state !== 'error', 'REAL_LOGIN_UI_REJECTED');
  if (state === 'consent') await page.locator('#consent-allow').click();
}
async function finishLogin(page, account) {
  await beginLogin(page, account);
  await page.waitForURL(url => url.origin === CONSOLE && !url.pathname.startsWith('/auth/'), { waitUntil: 'domcontentloaded' });
  const view = await session(page, account);
  const cookies = await page.context().cookies([CONSOLE, AUTH]);
  for (const [name, host] of [['__Host-saas_console', 'console.saas.test'], ['__Host-saas_sso', 'auth.saas.test']]) {
    const cookie = cookies.find(row => row.name === name);
    assert(cookie?.secure && cookie.httpOnly && cookie.sameSite === 'Lax' && cookie.path === '/' && cookie.domain === host, 'HOST_COOKIE_CONTRACT_FAILED');
  }
  assert(seenPaths.has('/oauth/authorize') && seenPaths.has('/auth/callback'), 'CANONICAL_REDIRECT_CHAIN_MISSING');
  return view;
}

try {
  assert(sql(`SELECT count(*) FROM memberships WHERE user_id='${ids.employee}' AND organization_id='${ids.organization}' AND member_type='EMPLOYEE' AND status='ENABLED'`) === '1', 'BASE_EMPLOYEE_FIXTURE_REQUIRED');
  // Reuse this one environment on retries. A previously created extra test
  // membership is made inactive until the explicit switching scenario below.
  sql(`UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE user_id='${ids.employee}' AND organization_id='${ids.secondOrganization}' AND member_type='EMPLOYEE' AND status<>'DISABLED'`);
  // Match prepare.py's deliberately minimal administrator grant. Super-admin
  // status does not bypass the real application/tenant/organization menu caps.
  let adminMenu = sql(`SELECT id FROM menus WHERE application_id='${ids.application}' AND permission='system:user' AND path='/system/users'`);
  if (!adminMenu) {
    adminMenu = sql('SELECT uuidv7()::text');
    assert(uuid7.test(adminMenu), 'FIXTURE_ADMIN_MENU_ID_REQUIRED');
    sql(`INSERT INTO menus(id,application_id,type,title,path,permission) VALUES('${adminMenu}','${ids.application}','MENU','HTTPS users','/system/users','system:user')`);
  }
  assert(uuid7.test(adminMenu), 'FIXTURE_ADMIN_MENU_REQUIRED');
  sql(`INSERT INTO tenant_menus(tenant_id,menu_id) VALUES('${ids.tenant}','${adminMenu}') ON CONFLICT DO NOTHING;
    INSERT INTO organization_menus(organization_id,menu_id) VALUES('${ids.organization}','${adminMenu}'),('${ids.secondOrganization}','${adminMenu}') ON CONFLICT DO NOTHING`);
  report.fixtureAdminPermission = 'system:user';
  browser = await chromium.launch({ headless: true, proxy: { server: proxy },
    ...(product === 'chromium' ? {} : { channel: product }) });
  assert(!interrupted, 'BROWSER_INTERRUPTED');
  report.browserVersion = browser.version();

  step = 'administrator-real-login-single-employee';
  const adminPage = await newPage();
  let admin = await finishLogin(adminPage, 'admin');
  assert(admin.memberships.length === 1 && admin.context.membershipId === ids.adminMember, 'ADMIN_AUTOMATIC_SELECTION_FAILED');
  pass(step);

  step = 'new-account-created-through-real-private-command';
  // Finish the real host bootstrap, then use an actual same-origin JSON page
  // for API operations. Two independent context-token consumers would otherwise
  // race the host's refresh manager and retire each other's business proof.
  await adminPage.waitForLoadState('networkidle');
  await adminPage.goto(`${CONSOLE}/auth/config`, { waitUntil: 'domcontentloaded' });
  admin = await session(adminPage, 'admin');
  const business = await inBrowser(adminPage, '/auth/context-token', 'POST', { expectedVersion: admin.version }, admin.csrfToken);
  assert(business.status === 200 && typeof business.body.accessToken === 'string', 'ADMIN_BUSINESS_PROOF_FAILED');
  const freshCredentials = { username: `fresh_${randomBytes(8).toString('hex')}`,
    password: `Fresh-Https-${randomBytes(18).toString('base64url')}9!` };
  const created = await employeeApi(adminPage, '/api/users', 'POST', { ...freshCredentials,
    realName: 'Fresh canonical employee', email: '', phone: '', gender: 'UNKNOWN' }, business.body.accessToken);
  assert(created.status === 200 && uuid7.test(created.body.id), 'ACTUAL_ACCOUNT_COMMAND_CREATE_FAILED');
  ids.fresh = created.body.id;
  environment.accounts.fresh = freshCredentials;
  const memberships = await employeeApi(adminPage, `/api/users/${ids.fresh}/memberships`, 'PUT', {
    items: [{ organizationId: ids.organization, departmentId: null, position: 'Fresh employee' }],
  }, business.body.accessToken);
  assert(memberships.status === 200, 'ACTUAL_EMPLOYEE_MEMBERSHIP_CREATE_FAILED');
  ids.freshMember = sql(`SELECT id FROM memberships WHERE user_id='${ids.fresh}' AND organization_id='${ids.organization}' AND member_type='EMPLOYEE' AND status='ENABLED'`);
  assert(uuid7.test(ids.freshMember), 'ACTUAL_FRESH_MEMBER_REQUIRED');
  const sourceFacts = JSON.parse(sql(`SELECT json_build_object('accountVersion',u.session_version,'beforeVersion',r.before_version,'afterVersion',r.after_version) FROM users u JOIN identity_account_command_receipts r ON r.target_user_id=u.id WHERE u.id='${ids.fresh}' ORDER BY r.created_at LIMIT 1`));
  report.freshAccountSourceVersions = sourceFacts;
  assert(sourceFacts.beforeVersion === 0 && sourceFacts.afterVersion === sourceFacts.accountVersion, 'ACTUAL_ACCOUNT_COMMAND_RECEIPT_MISSING');
  const freshPage = await newPage();
  const fresh = await finishLogin(freshPage, 'fresh');
  assert(fresh.context.membershipId === ids.freshMember, 'FRESH_ACCOUNT_CANONICAL_LOGIN_FAILED');
  pass(step, { canonicalLoginSucceeded: true });
  await freshPage.context().close();

  step = 'employee-real-login-single-employee';
  const employeePage = await newPage();
  let employee = await finishLogin(employeePage, 'employee');
  assert(employee.memberships.length === 1 && employee.context.membershipId === ids.employeeMember, 'EMPLOYEE_AUTOMATIC_SELECTION_FAILED');
  assert(employee.sessionId !== admin.sessionId, 'INDEPENDENT_ACCOUNT_SESSIONS_REQUIRED');
  pass(step);

  step = 'cookie-session-page-reload';
  const beforeReload = employee.sessionId;
  await employeePage.reload({ waitUntil: 'domcontentloaded' });
  employee = await session(employeePage, 'employee');
  assert(employee.sessionId === beforeReload, 'COOKIE_RESTORE_CHANGED_SESSION');
  await employeePage.waitForLoadState('networkidle');
  const storageSafe = await employeePage.evaluate(() => [localStorage, sessionStorage].every(storage =>
    !Object.entries(storage).some(([key, value]) => /token|password|secret|verifier/i.test(key)
      || /(?:access_token|refresh_token|id_token)/.test(value))));
  assert(storageSafe, 'BROWSER_SECRET_PERSISTENCE_FOUND');
  pass(step);

  step = 'online-refresh-at-access-boundary';
  // The real host reload/storage checks above have completed. Keep one owner
  // of this deliberate expiry-boundary request; the product correctly rejects
  // a second refresh while its first caller holds the rotation lease.
  await employeePage.goto(`${CONSOLE}/auth/config`, { waitUntil: 'domcontentloaded' });
  employee = await session(employeePage, 'employee');
  assert(uuid7.test(employee.sessionId), 'SESSION_ID_REQUIRED');
  const beforeGeneration = Number(sql(`SELECT max(t.generation) FROM auth_refresh_tokens t JOIN auth_refresh_families f ON f.id=t.family_id JOIN bff_sessions b ON b.oidc_grant_id=f.grant_id WHERE b.id='${employee.sessionId}'`));
  sql(`UPDATE bff_sessions SET access_expires_at=LEAST(access_expires_at,clock_timestamp()+interval '20 seconds') WHERE id='${employee.sessionId}' AND user_id='${ids.employee}'`);
  employee = await session(employeePage, 'employee');
  const afterGeneration = Number(sql(`SELECT max(t.generation) FROM auth_refresh_tokens t JOIN auth_refresh_families f ON f.id=t.family_id JOIN bff_sessions b ON b.oidc_grant_id=f.grant_id WHERE b.id='${employee.sessionId}'`));
  assert(afterGeneration > beforeGeneration, 'ONLINE_REFRESH_NOT_ROTATED');
  pass(step, { refreshFamilyRotated: true });

  step = 'real-central-context-switch';
  const secondMember = sql('SELECT uuidv7()::text');
  assert(uuid7.test(secondMember), 'SECOND_MEMBER_ID_REQUIRED');
  sql(`INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES('${secondMember}','${ids.employee}','${ids.secondOrganization}','EMPLOYEE') ON CONFLICT(user_id,organization_id,member_type) DO UPDATE SET status='ENABLED',identity_version=memberships.identity_version+1 WHERE memberships.status<>'ENABLED'`);
  const actualSecondMember = sql(`SELECT id FROM memberships WHERE user_id='${ids.employee}' AND organization_id='${ids.secondOrganization}' AND member_type='EMPLOYEE' AND status='ENABLED'`);
  assert(uuid7.test(actualSecondMember), 'SECOND_MEMBER_REQUIRED');
  employee = await session(employeePage, 'employee');
  const switching = await inBrowser(employeePage, '/auth/context-switch', 'POST', { expectedVersion: employee.version, returnPath: '/' }, employee.csrfToken);
  assert(switching.status === 200 && typeof switching.body.redirectTo === 'string', 'SWITCH_START_FAILED');
  const target = new URL(switching.body.redirectTo, CONSOLE);
  assert(target.origin === AUTH && target.pathname === '/oauth/authorize', 'CANONICAL_SWITCH_URL_REQUIRED');
  await employeePage.goto(target.toString(), { waitUntil: 'domcontentloaded' });
  await employeePage.locator(`input[name="context-membership"][value="${actualSecondMember}"]`).check();
  await employeePage.locator('#consent-allow').click();
  await employeePage.waitForURL(url => url.origin === CONSOLE && !url.pathname.startsWith('/auth/'), { waitUntil: 'domcontentloaded' });
  employee = await session(employeePage, 'employee');
  assert(employee.context.membershipId === actualSecondMember && employee.context.organizationId === ids.secondOrganization, 'SWITCHED_CONTEXT_MISMATCH');
  pass(step);
  await employeePage.waitForLoadState('networkidle');
  await employeePage.goto(`${CONSOLE}/auth/config`, { waitUntil: 'domcontentloaded' });
  employee = await session(employeePage, 'employee');

  step = 'customer-cannot-establish-employee-session';
  const customerPage = await newPage();
  await beginLogin(customerPage, 'customer');
  await customerPage.locator('#auth-content[data-screen="CONTEXT_SELECTION_REQUIRED"] #selection-empty:not([hidden])').waitFor();
  assert(await customerPage.locator('input[name="context-membership"]').count() === 0, 'CUSTOMER_RECEIVED_EMPLOYEE_CHOICES');
  assert(!(await customerPage.context().cookies(CONSOLE)).some(cookie => cookie.name === '__Host-saas_console'), 'CUSTOMER_RECEIVED_BFF_COOKIE');
  assert(sql(`SELECT count(*) FROM bff_sessions WHERE user_id='${ids.customer}'`) === '0', 'CUSTOMER_BFF_SESSION_CREATED');
  pass(step);

  step = 'revoked-source-fails-closed';
  sql(`UPDATE sso_sessions SET revoked_at=clock_timestamp() WHERE id=(SELECT global_session_id FROM bff_sessions WHERE id='${employee.sessionId}' AND user_id='${ids.employee}')`);
  const rejected = await inBrowser(employeePage, '/auth/session');
  assert(rejected.status === 401, 'REVOKED_SOURCE_ACCEPTED');
  const adminUnaffected = await session(adminPage, 'admin');
  assert(adminUnaffected.sessionId === admin.sessionId, 'OTHER_ACCOUNT_SESSION_CHANGED');
  pass(step, { otherAccountUnaffected: true });

  await Promise.allSettled([...pendingObservations]);
  assert(!seenPaths.has('/oauth/token') && !seenPaths.has('/oauth/introspect'), 'UPSTREAM_TOKEN_ENDPOINT_USED_BY_BROWSER');
  assert(tlsProtocols.size > 0 && [...tlsProtocols].every(protocol => /^TLS [12]\./.test(protocol)), 'TLS_PROTOCOL_EVIDENCE_MISSING');
  report.tlsProtocols = [...tlsProtocols];
  assert(!interrupted, 'BROWSER_INTERRUPTED');
  report.result = 'PASS';
} catch (error) {
  // Playwright exception messages can include form values or callback queries.
  // Record only the fixed scenario and a known assertion code.
  report.failedStep = step;
  const networkFailure = /net::(ERR_[A-Z0-9_]+)/.exec(error?.message || '')?.[1];
  report.failureCode = interrupted ? 'BROWSER_INTERRUPTED' : /^SQLSTATE_[0-9A-Z]{5}$/.test(error?.code || '') ? error.code : error?.code === 'ERR_ASSERTION' && /^[A-Z0-9_]+$/.test(error.message)
    ? error.message : networkFailure || (error?.name === 'TimeoutError' ? 'BROWSER_STEP_TIMEOUT' : 'BROWSER_STEP_FAILED');
  process.exitCode = 1;
} finally {
  await Promise.allSettled([...pendingObservations]);
  report.tlsProtocols = [...tlsProtocols];
  for (const context of contexts) await context.close().catch(() => {});
  if (browser) await browser.close().catch(() => {});
  try {
    sql(`UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE user_id='${ids.employee}' AND organization_id='${ids.secondOrganization}' AND member_type='EMPLOYEE' AND status<>'DISABLED'`);
    report.fixtureBoundaryReset = true;
  } catch (error) {
    report.fixtureBoundaryReset = false;
    report.fixtureBoundaryResetFailure = /^SQLSTATE_[0-9A-Z]{5}$/.test(error?.code || '') ? error.code : 'OWNED_SQL_FAILED';
    report.result = 'FAILED';
    process.exitCode = 1;
  }
  if (interrupted) report.result = 'FAILED';
  fs.mkdirSync(path.dirname(outputFile), { recursive: true });
  fs.writeFileSync(outputFile, `${JSON.stringify(report, null, 2)}\n`);
  console.log(`HTTPS_BROWSER_RESULT ${JSON.stringify({ result: report.result, checks: report.checks.length,
    ...(report.failedStep ? { failedStep: report.failedStep, failureCode: report.failureCode } : {}) })}`);
  process.removeListener('SIGINT', interrupt);
  process.removeListener('SIGTERM', interrupt);
}
