/** Real issuer + independent standard-library RP. No secrets enter the report. */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { execFileSync, spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { createHash, randomBytes, sign } from 'node:crypto';

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const argumentsMap = Object.fromEntries(process.argv.slice(2).map((value, index, all) => value.startsWith('--') ? [value, all[index + 1]] : []).filter(Boolean));
const TASK = '/tmp/rust-saas-unified-auth-20261028';
const ENV = `${TASK}/https-environment.json`;
const output = path.resolve(argumentsMap['--output'] ?? '');
assert(output.endsWith('.json') && path.basename(output).startsWith('phase-21-'), 'SAFE_OUTPUT_REQUIRED');
const artifactPrefix = argumentsMap['--artifact-prefix'] ?? 'phase-21-rp';
assert(/^phase-21-[a-z0-9-]+$/.test(artifactPrefix), 'SAFE_ARTIFACT_PREFIX_REQUIRED');
const privateJson = file => JSON.parse(execFileSync('wsl.exe', ['-d', 'Ubuntu', '--exec', 'cat', file], { encoding: 'utf8', windowsHide: true }));
const environment = privateJson(ENV), manifest = privateJson(`${TASK}/https-database.json`);
const phase = environment.phase21, ids = phase.ids;
assert(phase.contractVersion === 'unified-auth-phase21/1' && manifest.clusterId === '7701548153213487841' && /^unified_auth_https_[a-f0-9]{32}$/.test(manifest.database), 'EXACT_OWNED_FIXTURE_REQUIRED');
const AUTH = 'https://auth.saas.test', RP = 'https://customer.saas.test';
const lit = value => `'${String(value).replaceAll("'", "''")}'`;
const uuid = /^[a-f0-9]{8}-[a-f0-9]{4}-7[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/;
for (const value of Object.values(ids)) assert(uuid.test(value), 'FIXTURE_ID_REQUIRED');
function ownerSql(statement) {
  const guard = `IF current_database()<>${lit(manifest.database)} OR (SELECT system_identifier::text FROM pg_control_system())<>${lit(manifest.clusterId)} OR (SELECT shobj_description(oid,'pg_database') FROM pg_database WHERE datname=current_database()) IS DISTINCT FROM ${lit('identity-deployment/1/' + environment.ids.deployment)} OR (SELECT count(*) FROM users WHERE id=ANY(ARRAY[${lit(ids.user)}::uuid,${lit(ids.otherUser)}::uuid]) AND username=ANY(ARRAY['phase21-user','phase21-otheruser']) AND NOT is_super_admin)<>2 OR EXISTS(SELECT 1 FROM memberships WHERE user_id=ANY(ARRAY[${lit(ids.user)}::uuid,${lit(ids.otherUser)}::uuid])) THEN RAISE EXCEPTION 'owned phase21 guard failed'; END IF;`;
  const input = `BEGIN; DO $$ BEGIN ${guard} END $$; ${statement}; COMMIT;`;
  try { return execFileSync('wsl.exe', ['-d', 'Ubuntu', '-u', 'root', '--exec', 'psql', '-X', '-q', '-v', 'ON_ERROR_STOP=1', '-v', 'VERBOSITY=sqlstate', '-At', `postgres://postgres@127.0.0.1:55494/${manifest.database}`], { input, encoding: 'utf8', windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'], timeout: 15000 }).trim(); }
  catch (error) { const safe = new Error('OWNED_SQL_FAILED'); safe.code = /ERROR:\s+([A-Z0-9]{5})\b/.exec(String(error.stderr ?? ''))?.[1]; throw safe; }
}
const queryRow = statement => JSON.parse(ownerSql(`SELECT row_to_json(facts) FROM (${statement}) facts`));
const latestGrant = client => queryRow(`SELECT rp.grant_id,rp.public_sid,rp.public_sub FROM auth_rp_sessions rp JOIN auth_grants g ON g.id=rp.grant_id WHERE rp.user_id=${lit(ids.user)}::uuid AND rp.client_id=${lit(client)}::uuid AND g.revoked_at IS NULL ORDER BY rp.created_at DESC,rp.grant_id DESC LIMIT 1`);
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
async function until(predicate, code, timeout = 15000) { const end = Date.now() + timeout; while (!(await predicate())) { assert(Date.now() < end, code); await delay(120); } }
const namespace = privateJson(`${TASK}/namespace.pid`);
const node = `${TASK}/phase21-node-runtime/node-v24.22.1-linux-x64/bin/node`;
const protocol = spawn('wsl.exe', ['-d', 'Ubuntu', '-u', 'root', '--exec', 'nsenter', '--mount', '--target', String(namespace.pid), 'env', `NODE_EXTRA_CA_CERTS=${environment.identity.CONTEXT_PLATFORM_CA_FILE}`, node, `${TASK}/phase21-tools/phase21-protocol.mjs`, TASK], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
let sequence = 0, protocolReady = false, protocolFailure = false;
const pending = new Map();
createInterface({ input: protocol.stdout }).on('line', line => {
  try { const value = JSON.parse(line); if (value.ready) protocolReady = true; else { const operation = pending.get(value.id); if (operation) { pending.delete(value.id); operation.resolve(value); } } }
  catch { protocolFailure = true; }
});
protocol.stderr.on('data', () => { protocolFailure = true; });
protocol.on('exit', () => { for (const operation of pending.values()) operation.reject(new Error('PROTOCOL_DRIVER_EXITED')); pending.clear(); });
function rpc(command, values = {}) {
  return new Promise((resolve, reject) => {
    const id = ++sequence; const timer = setTimeout(() => { pending.delete(id); reject(new Error('PROTOCOL_DRIVER_TIMEOUT')); }, 20000);
    pending.set(id, { resolve: value => { clearTimeout(timer); resolve(value); }, reject: error => { clearTimeout(timer); reject(error); } });
    protocol.stdin.write(JSON.stringify({ command, ...values, id }) + '\n');
  });
}
async function successful(command, values) { const result = await rpc(command, values); assert(result.ok, `STANDARD_${command.toUpperCase().replaceAll('-', '_')}_FAILED_${result.code ?? 'UNKNOWN'}`); return result.result; }
async function rejected(command, values) { const result = await rpc(command, values); assert(!result.ok, `STANDARD_${command.toUpperCase().replaceAll('-', '_')}_NOT_REJECTED`); return result; }
process.env.PLAYWRIGHT_BROWSERS_PATH = path.join(repository, 'frontend/tests/browser/.browsers');
const require = createRequire(path.join(repository, 'frontend/tests/browser/package.json'));
const { chromium } = require('playwright');
const report = { contractVersion: 'unified-auth-phase21-real-https/1', date: new Date().toISOString(), result: 'FAILED',
  standardClient: 'openid-client@6.8.8', tlsErrorsIgnored: false, httpIssuerFixtures: false, clientReadsDatabase: false,
  ownerFixtureFacts: true, fixtureSignedReceiverNegatives: true, checks: [], endpoints: {}, tlsProtocols: [] };
const contexts = [], tls = new Set(), observations = new Set();
let browser, activePage, step = 'preflight', interrupted = false;
const interrupt = () => { interrupted = true; browser?.close().catch(() => {}); protocol.stdin.end(); };
process.on('SIGINT', interrupt); process.on('SIGTERM', interrupt);
function pass(details = {}) { report.checks.push({ scenario: step, result: 'PASS', ...details }); console.log(`PHASE21_CHECK ${step} PASS`); }
async function contextPage() {
  const context = await browser.newContext({ viewport: { width: 1440, height: 960 } }); contexts.push(context);
  const page = await context.newPage(); activePage = page;
  page.on('response', response => {
    const operation = (async () => { const url = new URL(response.url()); if (![AUTH, RP, 'https://console.saas.test'].includes(url.origin)) return; const key = `${response.request().method()} ${url.origin}${url.pathname.replace(/\/[a-f0-9-]{36}(?=\/|$)/g, '/:id')} ${response.status()}`; report.endpoints[key] = (report.endpoints[key] ?? 0) + 1; const security = await response.securityDetails().catch(() => null); if (security?.protocol) tls.add(security.protocol); })(); observations.add(operation); operation.finally(() => observations.delete(operation));
  });
  return { context, page };
}
async function json(page, endpoint, method = 'GET', body, csrf) {
  return page.evaluate(async args => { const response = await fetch(args.endpoint, { method: args.method, credentials: 'same-origin', redirect: 'error', headers: { Accept: 'application/json', ...(args.body === undefined ? {} : { 'Content-Type': 'application/json' }), ...(args.csrf ? { 'X-CSRF-Token': args.csrf } : {}) }, ...(args.body === undefined ? {} : { body: JSON.stringify(args.body) }) }); let body = null; try { body = await response.json(); } catch {} return { status: response.status, body, noStore: response.headers.get('cache-control')?.includes('no-store') }; }, { endpoint, method, body, csrf });
}
async function session(page) { const result = await json(page, '/api/session'); assert.equal(result.status, 200); assert(result.noStore); return result.body; }
async function authorize(page, authorizationUrl, { capture = false, callbackMutation } = {}) {
  let callback;
  const pattern = `${RP}/auth/callback?*`;
  if (capture || callbackMutation) await page.route(pattern, async route => {
    const url = new URL(route.request().url());
    if (capture) { callback = url.href; await route.fulfill({ status: 200, contentType: 'text/plain', body: 'Private standard-client callback captured.' }); }
    else { callbackMutation(url); await route.continue({ url: url.href }); }
  });
  await page.goto(authorizationUrl, { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(destination => location.origin === destination || !!document.querySelector('#login-form:not([hidden])') || (document.querySelector('#auth-content')?.dataset.screen === 'CONSENT_REQUIRED' && !document.querySelector('#consent-panel')?.hidden && !document.querySelector('#consent-allow')?.disabled) || !!document.querySelector('#error-message:not([hidden])'), RP);
  if (new URL(page.url()).origin === AUTH && await page.locator('#login-form:not([hidden])').isVisible()) {
    await page.locator('#username').fill(phase.accounts.user.username); await page.locator('#password').fill(phase.accounts.user.password);
    const accepted = page.waitForResponse(response => response.url() === `${AUTH}/api/sso/login` && response.request().method() === 'POST');
    await page.locator('#login-submit').click(); assert.equal((await accepted).status(), 200, 'REAL_PURE_ACCOUNT_LOGIN_FAILED');
  }
  const outcome = await page.waitForFunction(destination => {
    if (location.origin === destination) return 'destination';
    if (document.querySelector('#error-message:not([hidden])')) return 'error';
    if (document.querySelector('#auth-content')?.dataset.screen === 'CONSENT_REQUIRED' && !document.querySelector('#consent-panel')?.hidden && document.querySelector('#consent-allow')?.disabled === false) return 'consent';
    return false;
  }, RP);
  const state = await outcome.jsonValue(); await outcome.dispose();
  assert.notEqual(state, 'error', 'REAL_AUTHORIZATION_REJECTED_BEFORE_CALLBACK');
  if (state === 'consent') await page.locator('#consent-allow').click();
  await page.waitForURL(url => url.origin === RP, { waitUntil: 'domcontentloaded' });
  if (capture || callbackMutation) await page.unroute(pattern);
  return callback;
}
async function rpLogin(page, mutation, callbackMutation) {
  await page.goto(`${RP}/api/session`, { waitUntil: 'domcontentloaded' });
  const before = await session(page);
  const start = await json(page, '/auth/login', 'POST', { prompt: 'consent' }, before.csrfToken); assert.equal(start.status, 200);
  const authorizationUrl = new URL(start.body.authorizationUrl); if (mutation) mutation(authorizationUrl);
  await authorize(page, authorizationUrl.href, { callbackMutation });
  if (mutation || callbackMutation) {
    assert(new URL(page.url()).pathname === '/auth/callback', 'INVALID_CALLBACK_UNEXPECTEDLY_ACCEPTED');
    const result = await page.evaluate(() => JSON.parse(document.body.textContent)); assert.equal(typeof result.code, 'string');
    await page.goto(`${RP}/api/session`, { waitUntil: 'domcontentloaded' }); assert(!(await session(page)).authenticated, 'FAILED_LOGIN_ESTABLISHED_SESSION');
    return;
  }
  assert.equal(new URL(page.url()).pathname, '/');
  const result = await session(page); assert(result.authenticated && result.identity.issuer === AUTH && !uuid.test(result.identity.subject), 'OPAQUE_PURE_ACCOUNT_IDENTITY_REQUIRED');
  return result;
}
async function protocolLogin(profile = 'third', values = {}) {
  const { page } = await contextPage(); const start = await successful('start', { profile, ...values });
  const callback = await authorize(page, start.authorizationUrl, { capture: true }); assert(callback, 'STANDARD_CALLBACK_REQUIRED');
  const finish = await successful('finish', { key: start.key, callback }); assert(finish.userInfoMatches && !uuid.test(finish.subject)); return finish;
}
function signedLogout(claims, headerOverrides = {}) {
  const keyset = privateJson(`${TASK}/https-signing.json`), key = keyset.keys[0];
  const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url');
  const input = `${encode({ alg: 'RS256', typ: 'logout+jwt', kid: key.kid, ...headerOverrides })}.${encode(claims)}`;
  return `${input}.${sign('RSA-SHA256', Buffer.from(input), key.privateKeyPem).toString('base64url')}`;
}
const newId = () => { const value = randomBytes(16); value.writeUIntBE(Date.now(), 0, 6); value[6] = (value[6] & 15) | 112; value[8] = (value[8] & 63) | 128; const hex = value.toString('hex'); return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`; };
try {
  assert.equal(ownerSql(`SELECT count(*) FROM memberships WHERE user_id=ANY(ARRAY[${lit(ids.user)}::uuid,${lit(ids.otherUser)}::uuid])`), '0');
  await until(() => protocolReady || protocolFailure, 'STANDARD_PROTOCOL_DRIVER_NOT_READY'); assert(!protocolFailure, 'STANDARD_PROTOCOL_DRIVER_FAILED');
  browser = await chromium.launch({ headless: true, proxy: { server: 'http://127.0.0.1:24445' } }); report.browserVersion = browser.version();
  step = 'standalone-desktop-mobile-and-exact-discovery';
  const initial = await contextPage(); await initial.page.goto(RP, { waitUntil: 'networkidle' });
  await initial.page.screenshot({ path: path.join(path.dirname(output), `${artifactPrefix}-desktop.png`) });
  await initial.page.setViewportSize({ width: 390, height: 844 }); await initial.page.screenshot({ path: path.join(path.dirname(output), `${artifactPrefix}-mobile.png`) });
  assert(await initial.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'MOBILE_HORIZONTAL_OVERFLOW');
  await initial.page.goto(`${AUTH}/.well-known/openid-configuration`, { waitUntil: 'domcontentloaded' });
  const discovery = await initial.page.evaluate(() => JSON.parse(document.body.textContent));
  assert(discovery.issuer === AUTH && !discovery.end_session_endpoint && discovery.rp_logout_command_endpoint === `${AUTH}/oauth/logout` && discovery.code_challenge_methods_supported.includes('S256'));
  pass({ standalone: true, standardRpInitiatedLogoutAdvertised: false, logoutExtensionExplicit: true });

  step = 'no-membership-account-standard-login-id-token-userinfo-and-refresh';
  await protocolLogin();
  const first = await contextPage(); let firstSession = await rpLogin(first.page); const firstGrant = latestGrant(ids.thirdClient);
  const info = await json(first.page, '/api/userinfo'); assert.equal(info.status, 200); assert.equal(info.body.subject, firstSession.identity.subject);
  const cookie = (await first.context.cookies(RP)).find(item => item.name === '__Host-oidc_rp'); assert(cookie?.secure && cookie.httpOnly && cookie.sameSite === 'Lax' && cookie.path === '/');
  const refreshed = await json(first.page, '/auth/refresh', 'POST', {}, firstSession.csrfToken); assert.equal(refreshed.status, 200); assert.equal((await session(first.page)).identity.subject, firstSession.identity.subject);
  pass({ membershipCount: 0, opaqueSubject: true, userInfoMatches: true, refreshRotated: true });

  for (const kind of ['state', 'issuer', 'nonce', 'pkce']) {
    step = `real-issuer-${kind}-tampering-rejected`;
    const { page } = await contextPage();
    const mutation = kind === 'nonce' ? url => url.searchParams.set('nonce', randomBytes(24).toString('hex')) : kind === 'pkce' ? url => url.searchParams.set('code_challenge', createHash('sha256').update(randomBytes(32)).digest('base64url')) : undefined;
    const callbackMutation = kind === 'state' ? url => url.searchParams.set('state', randomBytes(24).toString('hex')) : kind === 'issuer' ? url => url.searchParams.set('iss', 'https://wrong-issuer.example') : undefined;
    await rpLogin(page, mutation, callbackMutation); pass();
  }

  step = 'explicit-dual-control-linking-and-csrf';
  firstSession = await session(first.page);
  const denied = await json(first.page, '/api/account/link', 'POST', { email: 'same@example.test' }, firstSession.csrfToken); assert.equal(denied.status, 409); assert.equal(denied.body.code, 'EXPLICIT_DUAL_CONTROL_LINK_REQUIRED');
  const noCsrf = await json(first.page, '/auth/refresh', 'POST', {}); assert.equal(noCsrf.status, 403); pass({ automaticEmailMerge: false });

  step = 'third-party-offline-and-unapproved-resource-scope-rejected';
  for (const scope of ['openid profile offline_access', 'openid profile demo.read']) {
    const { page } = await contextPage(); const start = await successful('start', { profile: 'third', scope });
    const denial = await page.goto(start.authorizationUrl, { waitUntil: 'domcontentloaded' });
    assert.equal(denial.status(), 400, 'THIRD_PARTY_SCOPE_ESCALATION_ACCEPTED');
    assert.equal(new URL(page.url()).origin, AUTH);
  }
  await rejected('user-client-credentials'); pass({ defaultScopes: 'openid profile', thirdPartyMachineDenied: true });

  step = 'standard-resource-audience-id-token-and-unapproved-business-denied';
  const external = await protocolLogin();
  assert.equal((await successful('resource', { key: external.key })).status, 401);
  assert.equal((await successful('resource', { key: external.key, idToken: true })).status, 401);
  assert([401, 403].includes((await successful('resource', { key: external.key, business: true })).status));
  pass({ userInfoAudienceCannotCallDemo: true, idTokenCannotCallApi: true, platformBusinessApiDenied: true });

  step = 'standard-rfc7009-revocation-and-refresh-replay';
  assert((await successful('refresh', { key: external.key })).rotated);
  await rejected('replay', { key: external.key }); await rejected('refresh', { key: external.key });
  const revocable = await protocolLogin(); await successful('revoke', { key: revocable.key }); await rejected('userinfo', { key: revocable.key }); await rejected('refresh', { key: revocable.key }); pass();

  step = 'standard-machine-cli-approved-resource-scope-and-person-separation';
  const machine = await successful('machine'); assert.equal((await successful('resource', { key: machine.key })).status, 200);
  await rejected('machine', { scope: 'demo.admin' }); await rejected('machine-userinfo', { key: machine.key });
  const cli = await successful('machine-cli'); assert.equal(cli.state, 'MACHINE_TOKEN_OBTAINED'); assert.equal(cli.resourceStatus, 200);
  pass({ separateApprovedServiceClient: true, thirdPartyDefaultMachineEnabled: false, personIdentity: false });

  step = 'current-app-extension-preserves-other-grant-and-consent';
  const second = await contextPage(); const secondSession = await rpLogin(second.page); const secondGrant = latestGrant(ids.thirdClient);
  const beforeConsent = ownerSql(`SELECT count(*) FROM auth_consents WHERE user_id=${lit(ids.user)}::uuid AND group_id=${lit(ids.thirdGroup)}::uuid AND status='ACTIVE'`);
  firstSession = await session(first.page); assert(firstSession.authenticated);
  const logout = await json(first.page, '/auth/logout', 'POST', { scope: 'CURRENT_APP' }, firstSession.csrfToken); assert.equal(logout.status, 200); assert.equal(logout.body.scope, 'CURRENT_APP');
  assert(!(await session(first.page)).authenticated); assert((await session(second.page)).authenticated); assert.equal((await json(second.page, '/api/userinfo')).status, 200);
  assert.equal(ownerSql(`SELECT revoked_at IS NOT NULL FROM auth_grants WHERE id=${lit(firstGrant.grant_id)}::uuid`), 't');
  assert.equal(ownerSql(`SELECT revoked_at IS NULL FROM auth_grants WHERE id=${lit(secondGrant.grant_id)}::uuid`), 't');
  assert.equal(ownerSql(`SELECT count(*) FROM auth_consents WHERE user_id=${lit(ids.user)}::uuid AND group_id=${lit(ids.thirdGroup)}::uuid AND status='ACTIVE'`), beforeConsent);
  await until(() => ownerSql(`SELECT state FROM auth_delivery_jobs WHERE grant_id=${lit(firstGrant.grant_id)}::uuid AND kind='LOGOUT'`) === 'DELIVERED', 'CURRENT_APP_BACKCHANNEL_NOT_DELIVERED');
  pass({ otherGrantPreserved: true, consentPreserved: true });

  step = 'signed-backchannel-wrong-audience-signature-and-replay';
  const now = Math.floor(Date.now() / 1000), claims = { iss: AUTH, aud: phase.publicClients.third, sid: randomBytes(32).toString('hex'), sub: 'owned-unknown-public-subject', jti: newId(), iat: now, exp: now + 120, events: { 'http://schemas.openid.net/event/backchannel-logout': {} } };
  const receiver = async token => second.page.evaluate(async value => { const response = await fetch('/auth/backchannel-logout', { method: 'POST', headers: { 'Content-Type': 'application/x-www-form-urlencoded' }, body: new URLSearchParams({ logout_token: value }) }); return { status: response.status, body: await response.json() }; }, token);
  const valid = signedLogout(claims); assert.equal((await receiver(valid)).status, 200); const replay = await receiver(valid); assert.equal(replay.status, 409); assert.equal(replay.body.code, 'BACKCHANNEL_REPLAY');
  assert.equal((await receiver(signedLogout({ ...claims, jti: newId(), aud: 'wrong-client' }))).status, 400);
  const pieces = signedLogout({ ...claims, jti: newId() }).split('.'); pieces[2] = (pieces[2][0] === 'A' ? 'B' : 'A') + pieces[2].slice(1); assert.equal((await receiver(pieces.join('.'))).status, 400); pass();

  step = 'approved-offline-absolute-idle-and-natural-sso-expiry';
  const offline = await protocolLogin('offline');
  const timing = queryRow(`SELECT f.absolute_expires_at-g.created_at BETWEEN interval '6 days 23 hours' AND interval '7 days 1 hour' AS absolute_ok,f.idle_expires_at-f.created_at BETWEEN interval '23 hours' AND interval '25 hours' AS idle_ok FROM auth_refresh_families f JOIN auth_grants g ON g.id=f.grant_id WHERE g.user_id=${lit(ids.user)}::uuid AND g.client_id=${lit(ids.offlineClient)}::uuid ORDER BY g.created_at DESC LIMIT 1`);
  assert(timing.absolute_ok && timing.idle_ok);
  // Expire at this statement's clock so a newly created SSO still satisfies
  // expires_at > created_at; the next actual HTTPS request observes it elapsed.
  ownerSql(`UPDATE sso_sessions SET expires_at=clock_timestamp() WHERE id=(SELECT source_session_id FROM auth_grants WHERE user_id=${lit(ids.user)}::uuid AND client_id=${lit(ids.offlineClient)}::uuid ORDER BY created_at DESC LIMIT 1)`);
  assert((await successful('refresh', { key: offline.key })).sourcePreserved);
  ownerSql(`UPDATE auth_refresh_families SET created_at=clock_timestamp()-interval '2 days',idle_expires_at=clock_timestamp()-interval '1 second' WHERE grant_id=(SELECT id FROM auth_grants WHERE user_id=${lit(ids.user)}::uuid AND client_id=${lit(ids.offlineClient)}::uuid ORDER BY created_at DESC LIMIT 1)`);
  await rejected('refresh', { key: offline.key });
  const absolute = await protocolLogin('offline');
  ownerSql(`UPDATE auth_refresh_families SET created_at=clock_timestamp()-interval '2 days',absolute_expires_at=clock_timestamp()-interval '1 second',idle_expires_at=clock_timestamp()-interval '2 seconds' WHERE grant_id=(SELECT id FROM auth_grants WHERE user_id=${lit(ids.user)}::uuid AND client_id=${lit(ids.offlineClient)}::uuid ORDER BY created_at DESC LIMIT 1)`);
  await rejected('refresh', { key: absolute.key }); pass({ approvedOfflineProfileSeparate: true, configuredAbsoluteDays: 7, configuredIdleHours: 24, expiredDeadlines: 'owner shrunk only owned fixture deadlines; real HTTPS refresh rejected' });

  step = 'global-extension-actual-worker-closes-independent-rp-and-retains-consent';
  // A fresh local session is less than 60 seconds old, so /api/session cannot
  // hide a missing back-channel behind its periodic online authority check.
  const globalRp = await contextPage(); await rpLogin(globalRp.page); const target = latestGrant(ids.thirdClient);
  const command = await protocolLogin(); assert((await session(globalRp.page)).authenticated);
  await successful('logout', { key: command.key, scope: 'GLOBAL' });
  await until(async () => !(await session(globalRp.page)).authenticated, 'ACTUAL_BACKCHANNEL_DID_NOT_CLOSE_RP');
  await until(() => ownerSql(`SELECT state FROM auth_delivery_jobs WHERE grant_id=${lit(target.grant_id)}::uuid AND kind='LOGOUT'`) === 'DELIVERED', 'GLOBAL_BACKCHANNEL_NOT_DELIVERED');
  assert.equal(ownerSql(`SELECT count(*) FROM auth_consents WHERE user_id=${lit(ids.user)}::uuid AND group_id=${lit(ids.thirdGroup)}::uuid AND status='ACTIVE'`), beforeConsent);
  await rejected('refresh', { key: command.key }); pass({ serverToServerBackchannel: true, localRevocationDoesNotDependOnPagePolling: true, consentPreserved: true });
  await Promise.allSettled([...observations]); assert(tls.size && [...tls].every(value => /^TLS [12]\./.test(value)), 'ACTUAL_VERIFIED_TLS_REQUIRED');
  assert(!interrupted); report.result = 'PASS';
} catch (error) {
  report.failedStep = step;
  report.failureCode = interrupted ? 'BROWSER_INTERRUPTED' : /^[A-Z0-9]{5}$/.test(error.code ?? '') ? `SQLSTATE_${error.code}` : /^[A-Z][A-Z0-9_]+$/.test(error.message ?? '') ? error.message : error.name === 'TimeoutError' ? 'BROWSER_STEP_TIMEOUT' : 'BROWSER_STEP_FAILED';
  if (activePage && !activePage.isClosed()) { const url = new URL(activePage.url()); report.page = { origin: [AUTH, RP].includes(url.origin) ? url.origin : 'OTHER', pathname: url.pathname.replace(/\/[a-f0-9-]{36}(?=\/|$)/g, '/:id'), screen: await activePage.evaluate(() => document.querySelector('#auth-content')?.dataset.screen ?? null).catch(() => null), errorText: await activePage.locator('#error-message').textContent({ timeout: 1000 }).catch(() => null), rpErrorCode: await activePage.evaluate(() => { try { const v = JSON.parse(document.body.textContent); return /^[A-Z_0-9]{1,80}$/.test(v.code ?? '') ? v.code : null; } catch { return null; } }).catch(() => null) }; }
  process.exitCode = 1;
} finally {
  protocol.stdin.end();
  await Promise.allSettled([...observations]); for (const context of contexts) await context.close().catch(() => {}); await browser?.close().catch(() => {});
  await Promise.race([new Promise(resolve => protocol.once('exit', resolve)), delay(3000)]); if (protocol.exitCode === null) protocol.kill();
  for (const account of Object.values(phase.accounts)) delete account.password;
  report.tlsProtocols = [...tls];
  assert(!JSON.stringify(report).match(/"(?:access_token|refresh_token|id_token|logout_token|password|clientSecret|cookie|verifier)"\s*:/i), 'REPORT_SECRET_GUARD_FAILED');
  fs.mkdirSync(path.dirname(output), { recursive: true }); fs.writeFileSync(output, JSON.stringify(report, null, 2) + '\n');
  console.log(`PHASE21_HTTPS_RESULT ${JSON.stringify({ result: report.result, checks: report.checks.length, failedStep: report.failedStep, failureCode: report.failureCode })}`);
  process.removeListener('SIGINT', interrupt); process.removeListener('SIGTERM', interrupt);
}
