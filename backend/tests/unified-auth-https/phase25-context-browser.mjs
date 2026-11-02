/** Actual frozen host, qiankun, two tabs and native notification context changes.
 * Every UI/API/socket response comes from the current services. No route mocks,
 * injected SDK, forged BroadcastChannel messages or exported storage state.
 */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { fetchBusinessJson, closeBusinessBrowser } from './phase25-business-http.mjs';

const TASK = '/tmp/rust-saas-unified-auth-20261028';
const AUTH = 'https://auth.saas.test', CONSOLE = 'https://console.saas.test';
const here = path.dirname(fileURLToPath(import.meta.url));
const repository = path.resolve(here, '../../..');
const options = {};
for (let index = 2; index < process.argv.length; index += 2) {
  assert(['--snapshot', '--output'].includes(process.argv[index]) && process.argv[index + 1]
    && !Object.hasOwn(options, process.argv[index]), 'PHASE25_CONTEXT_ARGUMENTS_INVALID');
  options[process.argv[index]] = process.argv[index + 1];
}
const label = options['--snapshot'];
assert(/^phase25-tools-r[1-9][0-9]*$/.test(label ?? ''), 'PHASE25_CONTEXT_FROZEN_TOOLS_REQUIRED');
const output = path.resolve(options['--output'] ?? '');
assert(path.dirname(output) === path.join(repository, 'backend/docs/designs/unified-auth/validation')
  && /^phase-25-context-[a-z0-9-]+\.json$/.test(path.basename(output))
  && !fs.lstatSync(path.dirname(output)).isSymbolicLink(), 'PHASE25_CONTEXT_SAFE_OUTPUT_REQUIRED');
const fd = fs.openSync(output, 'wx', 0o600);
const hash = value => createHash('sha256').update(value).digest('hex');
const canaryValues = new Set(), contexts = [], observations = new Set(), tls = new Set();
const pageFacts = new WeakMap();
const report = { contractVersion: 'unified-auth-phase25-real-host-context/1', result: 'FAIL',
  startedAtUtc: new Date().toISOString(), sourceSnapshot: label, checks: [], observedEndpoints: {},
  responseInterception: false, injectedSdk: false, forgedBroadcastMessages: false,
  tlsErrorsIgnored: false, rawCarriersPersisted: false, cleanup: null, fixtureCleanup: null };
let browser, verified, environment, fixture, stage = 'PREFLIGHT', cleanupNeeded = false;

function canaries(value, field = '') {
  if (Array.isArray(value)) for (const child of value) canaries(child, field);
  else if (value && typeof value === 'object') for (const [key, child] of Object.entries(value)) canaries(child, key);
  else if (typeof value === 'string' && value.length >= 8 && /secret|password|token|cookie|csrf|nonce|verifier|username/i.test(field)) canaryValues.add(value);
}
function persist() {
  const text = JSON.stringify(report, null, 2) + '\n';
  assert(!/"(?:access_token|id_token|refresh_token|password|clientSecret|cookie|nonce|verifier)"\s*:/i.test(text), 'PHASE25_CONTEXT_SECRET_FIELD');
  for (const value of canaryValues) assert(!text.includes(value), 'PHASE25_CONTEXT_SECRET_VALUE');
  const bytes = Buffer.from(text); fs.ftruncateSync(fd, 0); fs.writeSync(fd, bytes, 0, bytes.length, 0); fs.fsyncSync(fd);
}
function pass(facts = {}) { report.checks.push({ caseId: stage, status: 'PASS', ...facts }); delete report.stageDetail; persist(); console.log(`PHASE25_CONTEXT_CHECK ${stage} PASS`); }
function checkpoint(value) { report.stageDetail = value; persist(); }
function control(action) {
  const script = `${TASK}/source-snapshot-${label}/saas-plateform/backend/tests/unified-auth-https/phase25-context-control.py`;
  try {
    const raw = execFileSync('wsl.exe', ['-d', 'Ubuntu', '-u', 'jackie', '--exec', 'python3', '-B', script], {
      input: JSON.stringify({ action }), encoding: 'utf8', windowsHide: true,
      stdio: ['pipe', 'pipe', 'pipe'], timeout: 45000, maxBuffer: 8 * 1024 * 1024,
    });
    const value = JSON.parse(raw); canaries(value); return value;
  } catch { throw new Error('PHASE25_CONTEXT_OWNED_CONTROL_FAILED'); }
}
async function request(page, endpoint, input = {}) {
  const result = await page.evaluate(fetchBusinessJson, { endpoint, method: 'GET', ...input });
  canaries(result.body);
  const key = new URL(endpoint, CONSOLE).pathname + ':' + result.status;
  report.observedEndpoints[key] = (report.observedEndpoints[key] ?? 0) + 1;
  return result;
}
function observe(page) {
  const facts = { sockets: [], businessRequests: 0, pageErrors: 0, contextResponses: 0, appContext: null };
  pageFacts.set(page, facts);
  page.on('pageerror', () => { facts.pageErrors++; });
  page.on('request', req => { if (new URL(req.url()).pathname.startsWith('/work-orders-api/')) facts.businessRequests++; });
  page.on('response', response => {
    const pending = (async () => {
      const details = await response.securityDetails(); if (details?.protocol) tls.add(details.protocol);
      if (new URL(response.url()).pathname === '/api/apps/work-orders/context' && response.status() === 200) {
        const body = await response.json(); canaries(body); facts.appContext = body; facts.contextResponses++;
      }
    })().catch(() => {});
    observations.add(pending); pending.finally(() => observations.delete(pending));
  });
  page.on('websocket', socket => {
    const url = new URL(socket.url());
    if (url.origin !== 'wss://console.saas.test' || url.pathname !== '/socket.io/' || url.searchParams.get('transport') !== 'websocket') return;
    const item = { closed: false, authenticated: [], notifications: [] }; facts.sockets.push(item);
    socket.on('close', () => { item.closed = true; });
    socket.on('framesent', event => {
      try { if (typeof event.payload === 'string' && event.payload.startsWith('42')) canaries(JSON.parse(event.payload.slice(2))); } catch { /* Not a JSON event. */ }
    });
    socket.on('framereceived', event => {
      try {
        if (typeof event.payload !== 'string' || !event.payload.startsWith('42')) return;
        const value = JSON.parse(event.payload.slice(2));
        if (value[0] === 'authenticated') item.authenticated.push(value[1]);
        if (value[0] === 'notification') item.notifications.push(value[1]);
      } catch { /* Engine.IO control packets are not application events. */ }
    });
  });
  page.setDefaultTimeout(30000); page.setDefaultNavigationTimeout(40000);
  return page;
}
async function newContext() {
  const context = await browser.newContext({ viewport: { width: 1440, height: 960 }, serviceWorkers: 'block' });
  contexts.push(context); return context;
}
async function identity(page, actor, membershipId) {
  const account = environment.accounts[actor];
  await page.waitForFunction(() => location.origin !== 'https://auth.saas.test'
    || !!document.querySelector('#login-form:not([hidden])')
    || ['CONSENT_REQUIRED', 'CONTEXT_SELECTION_REQUIRED'].includes(document.querySelector('#auth-content')?.dataset.screen));
  if (new URL(page.url()).origin === AUTH && await page.locator('#login-form:not([hidden])').isVisible()) {
    await page.locator('#username').fill(account.username); await page.locator('#password').fill(account.password);
    const response = page.waitForResponse(value => value.url() === `${AUTH}/api/sso/login` && value.request().method() === 'POST');
    await page.locator('#login-submit').click(); assert.equal((await response).status(), 200, 'PHASE25_CONTEXT_PRIMARY_LOGIN_FAILED');
  }
  for (let count = 0; count < 12; count++) {
    const observed = await page.waitForFunction(() => {
      if (location.origin === 'https://console.saas.test' && !location.pathname.startsWith('/auth/')) return 'done';
      if (document.querySelector('#error-message:not([hidden])')) return 'error';
      if (document.querySelector('#auth-content')?.dataset.screen === 'CONTEXT_SELECTION_REQUIRED'
        && document.querySelector('#organization-choices')?.hidden === false) return 'selection';
      if (document.querySelector('#auth-content')?.dataset.screen === 'CONSENT_REQUIRED'
        && !document.querySelector('#consent-panel')?.hidden && !document.querySelector('#consent-allow')?.disabled) return 'consent';
      return false;
    });
    const state = await observed.jsonValue(); await observed.dispose();
    assert.notEqual(state, 'error', 'PHASE25_CONTEXT_IDENTITY_REJECTED');
    if (state === 'done') return;
    if (state === 'selection') {
      const choice = page.locator(`input[name="context-membership"][value="${membershipId}"]`);
      await choice.check(); assert(await choice.isChecked(), 'PHASE25_CONTEXT_EXACT_MEMBER_NOT_SELECTED');
    }
    await page.locator('#consent-allow').click();
    await page.waitForFunction(previous => location.origin !== 'https://auth.saas.test'
      || document.querySelector('#auth-content')?.dataset.screen !== previous
      || !!document.querySelector('#error-message:not([hidden])'), state === 'selection' ? 'CONTEXT_SELECTION_REQUIRED' : 'CONSENT_REQUIRED');
  }
  assert.fail('PHASE25_CONTEXT_IDENTITY_TRANSITIONS_EXCEEDED');
}
async function login(page, actor, membershipId, returnPath = '/micro/work-orders') {
  await page.goto(`${CONSOLE}/auth/login?return=${encodeURIComponent(returnPath)}`, { waitUntil: 'domcontentloaded' });
  await identity(page, actor, membershipId);
  const view = await session(page, actor, membershipId);
  return view;
}
async function session(page, actor, membershipId) {
  const value = await request(page, '/auth/session');
  assert.equal(value.status, 200, 'PHASE25_CONTEXT_SESSION_FAILED');
  assert.equal(value.body.account?.id, environment.ids[actor], 'PHASE25_CONTEXT_ACCOUNT_CHANGED');
  assert(value.body.context?.memberType === 'EMPLOYEE' && value.body.context.membershipId === membershipId,
    'PHASE25_CONTEXT_MEMBER_CHANGED');
  return value.body;
}
async function carriers(page) {
  const current = await request(page, '/auth/session');
  const root = await request(page, '/auth/context-token', { method: 'POST', csrf: current.body.csrfToken,
    body: { expectedVersion: current.body.version } });
  assert.equal(root.status, 200, 'PHASE25_CONTEXT_ROOT_CAPABILITY_FAILED');
  const app = await request(page, '/api/apps/session-token', { method: 'POST', bearer: root.body.accessToken,
    body: { appCode: 'work-orders' } });
  assert.equal(app.status, 201, 'PHASE25_CONTEXT_APP_CAPABILITY_FAILED');
  return { root: root.body.accessToken, app: app.body.accessToken };
}
async function waitFor(predicate, code, timeout = 30000) {
  const until = Date.now() + timeout;
  while (!await predicate()) {
    assert(Date.now() < until, code);
    await new Promise(resolve => setTimeout(resolve, 35));
  }
}
async function mounted(page, organization) {
  const node = page.getByTestId('work-orders-app');
  await node.waitFor({ state: 'visible' });
  await page.waitForFunction(expected => {
    const node = document.querySelector('[data-testid="work-orders-app"]');
    return node?.getAttribute('data-org-id') === expected && node.getAttribute('data-context-active') === 'true';
  }, organization);
  await waitFor(() => pageFacts.get(page).appContext?.organization?.id === organization,
    'PHASE25_CONTEXT_ACTUAL_APP_CONTEXT_MISSING');
  const context = pageFacts.get(page).appContext;
  assert(context.user?.id === environment.ids.employee && context.tenant?.id === environment.ids.tenant
    && context.appCode === 'work-orders', 'PHASE25_CONTEXT_APP_TUPLE_CHANGED');
}
async function liveSocket(page, organization) {
  await page.locator('[aria-label="实时推送已连接"]').waitFor({ state: 'visible' });
  await waitFor(() => pageFacts.get(page).sockets.some(socket => !socket.closed && socket.authenticated.some(value =>
    value.userId === environment.ids.employee && value.tenantId === environment.ids.tenant && value.organizationId === organization)),
  'PHASE25_CONTEXT_REAL_SOCKET_CONTEXT_MISSING');
}
async function ticket(page, bearer, title, key) {
  const value = await request(page, '/work-orders-api/tickets', { method: 'POST', bearer, idempotencyKey: key,
    body: { title, description: 'Owned Phase25 host context validation.' } });
  assert.equal(value.status, 201, 'PHASE25_CONTEXT_TICKET_CREATE_FAILED');
  assert.equal(typeof value.body.id, 'string', 'PHASE25_CONTEXT_TICKET_ID_MISSING');
  return value.body.id;
}
async function publish(page, bearer, organization, title, key) {
  const value = await request(page, '/api/apps/work-orders/notifications', { method: 'POST', bearer,
    body: { idempotencyKey: key, target: { kind: 'USER', userId: environment.ids.employee, organizationId: organization },
      type: 'INFO', title, content: 'Owned actual source and recipient.', link: '/micro/work-orders' } });
  assert([200, 201].includes(value.status), 'PHASE25_CONTEXT_NOTIFICATION_PUBLISH_FAILED');
}
function delivered(page, title, organization) {
  return pageFacts.get(page).sockets.some(socket => socket.notifications.some(value => value.title === title
    && value.userId === environment.ids.employee && value.tenantId === environment.ids.tenant && value.organizationId === organization));
}
async function safeDiagnostics() {
  const output = [];
  for (const [contextIndex, context] of contexts.entries()) {
    for (const [pageIndex, page] of context.pages().entries()) {
      const fact = pageFacts.get(page);
      const url = new URL(page.url());
      const route = [AUTH, CONSOLE].includes(url.origin)
        ? url.pathname.replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, ':owned-id') : 'other-origin';
      const value = { contextIndex, pageIndex, origin: [AUTH, CONSOLE].includes(url.origin) ? url.origin : 'other-origin',
        pathWithoutQuery: route, pageErrors: fact?.pageErrors ?? 0, actualBusinessRequests: fact?.businessRequests ?? 0,
        contextResponses: fact?.contextResponses ?? 0, liveSockets: fact?.sockets.filter(socket => !socket.closed).length ?? 0 };
      try {
        value.dom = await page.evaluate(() => {
          const count = selector => document.querySelectorAll(selector).length;
          const root = document.querySelector('[data-testid="work-orders-app"]');
          const fault = document.querySelector('[data-testid="trusted-runtime-fault"]');
          return { microRoots: count('[data-work-orders-root]'), workOrdersApps: count('[data-testid="work-orders-app"]'),
            listViews: count('[data-testid="ticket-list"]'), detailViews: count('[data-testid="ticket-detail"]'),
            openTicketButtons: count('[data-testid="open-ticket"]'), backToListButtons: count('[data-testid="back-to-list"]'),
            runtimeFaultPresent: !!fault, microContextActive: root?.getAttribute('data-context-active') === 'true',
            dialogs: count('[role="dialog"]'), alertNodes: count('[role="alert"]') };
        });
      } catch { value.domUnavailable = true; }
      output.push(value);
    }
  }
  return output;
}

try {
  cleanupNeeded = true;
  verified = control('prepare'); environment = verified.environment; fixture = verified.fixture;
  for (const [name, expected] of Object.entries(verified.scriptSha256)) {
    assert.equal(hash(fs.readFileSync(path.join(here, name))), expected, 'PHASE25_CONTEXT_LIVE_TOOL_CHANGED');
  }
  report.snapshotSha256 = verified.snapshotSha256;
  report.fixtureScope = 'One new employee membership, second-organization work-orders grants, and one explicit read-only detail MENU with exact links';
  report.currentClusterAndSourceVerified = true; pass();
  process.env.PLAYWRIGHT_BROWSERS_PATH = path.join(repository, 'frontend/tests/browser/.browsers');
  const { chromium } = createRequire(path.join(repository, 'frontend/tests/browser/package.json'))('playwright');
  browser = await chromium.launch({ headless: true, proxy: { server: 'http://127.0.0.1:24445' } });
  const shared = await newContext(), pageA = observe(await shared.newPage()), pageB = observe(await shared.newPage());
  const suffix = fixture.fixtureId.slice(-12), firstTitle = `Owned host first ${suffix}`, secondTitle = `Owned host second ${suffix}`;

  stage = 'CURRENT_HOST_QIANKUN_AND_SAME_COOKIE_TABS';
  const before = await login(pageA, 'employee', environment.ids.employeeMember);
  await mounted(pageA, environment.ids.organization); await liveSocket(pageA, environment.ids.organization);
  const old = await carriers(pageA);
  const firstTicket = await ticket(pageA, old.app, firstTitle, `p25-host-first-${suffix}`);
  await pageA.reload({ waitUntil: 'domcontentloaded' }); await mounted(pageA, environment.ids.organization);
  await pageB.goto(`${CONSOLE}/micro/work-orders`, { waitUntil: 'domcontentloaded' });
  await mounted(pageB, environment.ids.organization); await liveSocket(pageB, environment.ids.organization);
  const sharedView = await session(pageB, 'employee', environment.ids.employeeMember);
  assert.equal(sharedView.sessionId, before.sessionId, 'PHASE25_CONTEXT_TABS_NOT_SHARED_COOKIE');
  await pageB.getByTestId('work-orders-app').getByText(firstTitle, { exact: true }).waitFor();
  pass({ scenarioIds: ['P25-C13-S06'], actualFrozenHostAndMicro: true, actualNativeBusinessApi: true,
    sameCookieTwoPages: true, actualSameOriginWebsocket: true });

  stage = 'REAL_HOST_ROUTE_DELIVERS_LIFECYCLE_UPDATE';
  checkpoint('capture-real-micro-root');
  const rootNode = await pageB.locator('[data-work-orders-root]').elementHandle();
  assert(rootNode, 'PHASE25_CONTEXT_MICRO_ROOT_MISSING');
  checkpoint('click-owned-ticket-detail');
  await pageB.locator(`[data-ticket-id="${firstTicket}"]`).getByTestId('open-ticket').click();
  checkpoint('wait-host-detail-route');
  await pageB.waitForURL(url => url.pathname === `/micro/work-orders/tickets/${firstTicket}`);
  checkpoint('wait-real-micro-detail-view');
  await pageB.getByTestId('ticket-detail').waitFor({ state: 'visible' });
  checkpoint('verify-same-mounted-root');
  assert(await pageB.locator('[data-work-orders-root]').evaluate((node, original) => node === original, rootNode),
    'PHASE25_CONTEXT_ROUTE_REMOUNTED_INSTEAD_OF_UPDATE');
  checkpoint('click-return-to-list');
  await pageB.getByTestId('back-to-list').click();
  checkpoint('wait-host-list-route');
  await pageB.waitForURL(url => url.pathname === '/micro/work-orders');
  checkpoint('wait-real-micro-list-view');
  await pageB.getByTestId('ticket-list').waitFor({ state: 'visible' });
  await rootNode.dispose();
  pass({ scenarioIds: ['P25-C13-S06'], actualRouteChanged: true, actualDetailRead: true,
    sameMountedMicroRoot: true, productionQiankunUpdatePath: true });

  const publisherContext = await newContext(), publisher = observe(await publisherContext.newPage());
  await login(publisher, 'admin', environment.ids.adminMember, '/');
  const sender = await carriers(publisher);

  stage = 'REAL_SWITCH_FREEZES_OTHER_TAB_AND_RETIRES_OLD_SOCKET';
  const oldSockets = [...pageFacts.get(pageB).sockets].filter(socket => !socket.closed);
  await pageA.getByRole('button', { name: '前往认证中心切换租户和组织', exact: true }).click();
  await pageA.waitForFunction(() => location.origin === 'https://auth.saas.test'
    && document.querySelector('#auth-content')?.dataset.screen === 'CONTEXT_SELECTION_REQUIRED'
    && document.querySelector('#organization-choices')?.hidden === false);
  await pageB.getByText('正在切换组织...', { exact: true }).waitFor({ state: 'visible' });
  assert.equal(await pageB.getByTestId('work-orders-app').count(), 0, 'PHASE25_CONTEXT_OLD_MICRO_NOT_FROZEN');
  await waitFor(() => oldSockets.length > 0 && oldSockets.every(socket => socket.closed), 'PHASE25_CONTEXT_OLD_SOCKET_NOT_RETIRED');
  const frozenBusinessRequests = pageFacts.get(pageB).businessRequests;
  assert.equal(await pageB.getByTestId('create-ticket').count(), 0, 'PHASE25_CONTEXT_OLD_WRITE_STILL_EXPOSED');
  // Exercise a real unrelated publication while the authority still holds
  // the old selection. The background tab must remain frozen through it;
  // no artificial delay, response substitution or forged channel message.
  const frozenNotice = `Owned frozen recipient ${suffix}`;
  await publish(publisher, sender.app, environment.ids.organization, frozenNotice, `p25-notify-frozen-${suffix}`);
  assert.equal(pageFacts.get(pageB).businessRequests, frozenBusinessRequests, 'PHASE25_CONTEXT_FROZEN_TAB_STARTED_BUSINESS');
  assert.equal(await pageB.getByText(frozenNotice, { exact: true }).count(), 0, 'PHASE25_CONTEXT_FROZEN_TAB_RENDERED_OLD_NOTIFICATION');
  pass({ scenarioIds: ['P25-C13-S03', 'P25-C13-S05', 'P25-C13-S07'], actualCentralSelectionPaused: true,
    backgroundTabFrozenBeforeCommit: true, oldMicroRemoved: true, oldSocketClosed: true });

  stage = 'REAL_MEMBER_SWITCH_UPDATES_BOTH_TABS_AND_INVALIDATES_OLD_APP';
  await identity(pageA, 'employee', fixture.member);
  await mounted(pageA, fixture.organization); await liveSocket(pageA, fixture.organization);
  await mounted(pageB, fixture.organization); await liveSocket(pageB, fixture.organization);
  const afterA = await session(pageA, 'employee', fixture.member), afterB = await session(pageB, 'employee', fixture.member);
  assert(afterA.version === afterB.version && afterA.sessionId === afterB.sessionId
    && (afterA.version !== before.version || afterA.sessionId !== before.sessionId), 'PHASE25_CONTEXT_NEW_COOKIE_SOURCE_NOT_SHARED');
  const denied = await request(pageB, '/work-orders-api/tickets', { bearer: old.app });
  assert([401, 403].includes(denied.status), 'PHASE25_CONTEXT_OLD_APP_SURVIVED_SWITCH');
  const next = await carriers(pageA);
  assert.notEqual(next.app, old.app, 'PHASE25_CONTEXT_NEW_APP_CARRIER_NOT_DISTINCT');
  await ticket(pageA, next.app, secondTitle, `p25-host-second-${suffix}`);
  // A normal same-route reload reads the actual new-org database records.
  await pageA.reload({ waitUntil: 'domcontentloaded' }); await mounted(pageA, fixture.organization); await liveSocket(pageA, fixture.organization);
  await pageA.getByTestId('work-orders-app').getByText(secondTitle, { exact: true }).waitFor();
  assert.equal(await pageA.getByTestId('work-orders-app').getByText(firstTitle, { exact: true }).count(), 0,
    'PHASE25_CONTEXT_OLD_ORG_CONTENT_VISIBLE');
  pass({ scenarioIds: ['P25-C13-S03', 'P25-C13-S05', 'P25-C13-S06'], actualSecondMembership: true,
    bothTabsShareNewAuthority: true, oldAppCarrierDenied: true, actualNewMicroContext: true,
    oldOrganizationContentRemoved: true, newOrganizationContentVisible: true,
    organizationSwitchUsesProductionUnmountAndRemount: true });

  stage = 'REAL_NOTIFICATION_SWITCH_DELIVERS_ONLY_NEW_RECIPIENT';
  const retiredTitle = `Owned retired recipient ${suffix}`, currentTitle = `Owned current recipient ${suffix}`;
  await publish(publisher, sender.app, environment.ids.organization, retiredTitle, `p25-notify-old-${suffix}`);
  await publish(publisher, sender.app, fixture.organization, currentTitle, `p25-notify-new-${suffix}`);
  for (const page of [pageA, pageB]) {
    await waitFor(() => delivered(page, currentTitle, fixture.organization), 'PHASE25_CONTEXT_NEW_NOTIFICATION_NOT_DELIVERED');
    assert(!delivered(page, retiredTitle, environment.ids.organization), 'PHASE25_CONTEXT_OLD_NOTIFICATION_CROSSED_SWITCH');
    await page.getByText(currentTitle, { exact: true }).first().waitFor({ state: 'visible' });
    assert.equal(await page.getByText(retiredTitle, { exact: true }).count(), 0, 'PHASE25_CONTEXT_OLD_NOTIFICATION_RENDERED');
  }
  pass({ scenarioIds: ['P25-C13-S07'], actualNativeNotification: true, actualPostgresPublicationAndHint: true,
    actualSameOriginWebsocket: true, accurateRecipientDeliveredToBothTabs: true, oldRecipientNotDeliveredOrRendered: true });

  stage = 'REAL_LOGOUT_RETIRES_OTHER_TAB_MICRO_AND_SOCKET';
  await pageA.getByRole('button', { name: '用户菜单', exact: true }).click();
  await pageA.getByRole('menuitem', { name: '退出本平台', exact: true }).click();
  await pageA.waitForURL(url => url.pathname === '/login' && url.searchParams.get('signedOut') === '1');
  await pageB.bringToFront();
  await pageB.waitForURL(url => url.pathname === '/login');
  assert.equal(await pageB.getByTestId('work-orders-app').count(), 0, 'PHASE25_CONTEXT_OLD_TAB_MICRO_SURVIVED_LOGOUT');
  await waitFor(() => pageFacts.get(pageB).sockets.every(socket => socket.closed), 'PHASE25_CONTEXT_OLD_TAB_SOCKET_SURVIVED_LOGOUT');
  const loggedOut = await request(pageB, '/auth/session');
  assert(loggedOut.status === 200 && loggedOut.body.account === null && loggedOut.body.context === null,
    'PHASE25_CONTEXT_OLD_TAB_REVIVED_SESSION');
  pass({ scenarioIds: ['P25-C13-S05', 'P25-C13-S07'], actualSharedCookieLogout: true,
    backgroundTabAnonymous: true, oldMicroAndSocketRetired: true });
  await Promise.allSettled([...observations]);
  assert(tls.has('TLS 1.3'), 'PHASE25_CONTEXT_VERIFIED_TLS_REQUIRED');
  assert([pageA, pageB].every(page => pageFacts.get(page).pageErrors === 0), 'PHASE25_CONTEXT_HOST_PAGE_ERROR');
  report.tlsProtocols = [...tls]; report.browserVersion = browser.version(); report.result = 'PASS';
} catch (error) {
  const code = String(error?.message ?? '').split(/\r?\n/, 1)[0];
  report.failedStep = stage;
  report.failureCode = /^PHASE25_CONTEXT_[A-Z0-9_]+$/.test(code) ? code
    : error?.name === 'TimeoutError' ? 'PHASE25_CONTEXT_BROWSER_TIMEOUT' : 'PHASE25_CONTEXT_STEP_FAILED';
  report.failureDiagnostics = await safeDiagnostics();
  process.exitCode = 1;
} finally {
  report.cleanup = await closeBusinessBrowser(browser, contexts);
  if (cleanupNeeded) {
    try { report.fixtureCleanup = control('cleanup'); }
    catch { report.fixtureCleanup = { fixtureRowsRemoved: false, failureCode: 'PHASE25_CONTEXT_FIXTURE_CLEANUP_FAILED' }; }
  }
  if (!report.cleanup.browserClosed || !report.cleanup.contextsClosed || report.cleanup.failures > 0
    || !report.fixtureCleanup?.fixtureRowsRemoved) { report.result = 'FAIL'; process.exitCode = 1; }
  report.finishedAtUtc = new Date().toISOString();
  try { persist(); } finally { fs.closeSync(fd); }
  console.log(JSON.stringify({ result: report.result, failedStep: report.failedStep,
    failureCode: report.failureCode, browserClosed: report.cleanup.browserClosed,
    fixtureRowsRemoved: report.fixtureCleanup?.fixtureRowsRemoved }));
}
