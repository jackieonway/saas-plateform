/** Real Phase19 HTTPS execution, only on a newly prepared --include-phase19 fixture.
 * Preparation/compilation of this file never executes it. The operator owns startup,
 * the CA, private configuration, and final environment/database cleanup.
 * No HTTP fixtures, TLS bypass, traces, screenshots, storage-state or secret exports.
 * Enrollment seeds/recovery codes and normal browser/BFF credentials remain in memory.
 */
import assert from 'node:assert/strict';
import path from 'node:path';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { createHmac, randomBytes } from 'node:crypto';
import { loadOwnedBrowserInputs, validateBrowserReport, saveBrowserReport } from './phase25-browser-binding.mjs';

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const options = {};
const args = process.argv.slice(2);
for (let index = 0; index < args.length; index += 2) {
  assert(['--environment', '--output', '--proxy', '--browser', '--phase', '--snapshot'].includes(args[index]) && args[index + 1], 'INVALID_ARGUMENTS');
  assert(!Object.hasOwn(options, args[index]), 'DUPLICATE_ARGUMENT');
  options[args[index]] = args[index + 1];
}
assert(options['--environment'] && options['--output'], 'ENVIRONMENT_AND_OUTPUT_REQUIRED');
const executionPhase = Number(options['--phase'] ?? 19);
assert([19, 25].includes(executionPhase), 'EXECUTION_PHASE_INVALID');
const environmentFile = options['--environment'];
assert(/^\/tmp\/rust-saas-unified-auth-20261028\/(?:[A-Za-z0-9_.-]+\/)*https-environment\.json$/.test(environmentFile)
  && !environmentFile.split('/').includes('..'), 'OWNED_ENVIRONMENT_REQUIRED');
const outputFile = path.resolve(options['--output']);
assert(outputFile.endsWith('.json') && !/^https-(?:environment|database|.*keys|signing)\.json$/.test(path.basename(outputFile)), 'SAFE_JSON_REPORT_REQUIRED');
validateBrowserReport({ outputFile, executionPhase, legacyPhase: 19 });
const proxy = options['--proxy'] || 'http://127.0.0.1:24445';
assert(proxy === 'http://127.0.0.1:24445', 'OWNED_CONNECT_PROXY_REQUIRED');
const product = options['--browser'] || 'chromium';
assert(['chromium', 'chrome', 'msedge'].includes(product), 'UNSUPPORTED_BROWSER');
const CONSOLE = 'https://console.saas.test';
const AUTH = 'https://auth.saas.test';
const ENTRY = '/auth/step-up?return=/system/oauth/clients';
const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const ACTORS = ['applicant', 'reviewer', 'publisher'];

const ownedInputs = loadOwnedBrowserInputs({ environmentFile, executionPhase, legacyPhase: 19,
  snapshot: options['--snapshot'], entrypoint: fileURLToPath(import.meta.url) });
const { environment, manifest } = ownedInputs;
assert(environment.phase19?.enabled === true && environment.phase19.contractVersion === 'unified-auth-phase19/1', 'FRESH_PHASE19_FIXTURE_REQUIRED');
assert(/^unified_auth_https_[0-9a-f]{32}$/.test(manifest.database) && /^[0-9]{10,24}$/.test(manifest.clusterId), 'OWNED_DATABASE_REQUIRED');
const ids = environment.ids;
for (const key of [...ACTORS, ...ACTORS.map(actor => `${actor}Member`), 'deployment', 'organization', 'tenant']) assert(uuidPattern.test(ids[key]), 'PHASE19_ACTOR_ID_REQUIRED');
assert(new Set(ACTORS.map(actor => ids[actor])).size === 3, 'DISTINCT_PHASE19_ACTORS_REQUIRED');
assert(ACTORS.every(actor => !['admin', 'employee', 'customer'].some(base => ids[base] === ids[actor])), 'PHASE19_BASE_ACCOUNT_BOUNDARY_OVERLAP');
for (const actor of ACTORS) assert(typeof environment.accounts[actor]?.username === 'string' && typeof environment.accounts[actor]?.password === 'string', 'PRIVATE_PHASE19_ACCOUNT_REQUIRED');
let bound;
try { bound = new URL(environment.platform.DATABASE_URL); }
catch { throw new Error('EXACT_DATABASE_BINDING_INVALID'); }
assert(bound.hostname === '127.0.0.1' && bound.port === '55494' && bound.pathname === `/${manifest.database}`, 'EXACT_DATABASE_BINDING_REQUIRED');
const databaseUrl = `postgres://postgres@127.0.0.1:55494/${manifest.database}`;
const marker = `identity-deployment/1/${ids.deployment}`;
const literal = value => `'${String(value).replaceAll("'", "''")}'`;
function sql(statement) {
  const input = `BEGIN; DO $$ BEGIN IF current_database()<>${literal(manifest.database)}
    OR (SELECT system_identifier::text FROM pg_control_system())<>${literal(manifest.clusterId)}
    OR current_setting('data_directory')<>${literal(ownedInputs.dataDirectory)}
    OR (SELECT shobj_description(oid,'pg_database') FROM pg_database WHERE datname=current_database()) IS DISTINCT FROM ${literal(marker)}
    OR (SELECT count(*) FROM users subject JOIN memberships member ON member.user_id=subject.id
        WHERE subject.id=ANY(ARRAY[${ACTORS.map(actor => `${literal(ids[actor])}::uuid`).join(',')}])
          AND NOT subject.is_super_admin AND subject.status='ENABLED' AND member.member_type='EMPLOYEE'
          AND member.organization_id=${literal(ids.organization)}::uuid AND member.status='ENABLED')<>3
    THEN RAISE EXCEPTION 'owned phase19 fixture guard failed'; END IF; END $$;
    ${statement}; COMMIT;`;
  try {
    return execFileSync(process.platform === 'linux' ? 'psql' : 'wsl.exe',
      [...(process.platform === 'linux' ? [] : ['-u', 'root', '--', 'psql']), '-X', '-q', '-v', 'ON_ERROR_STOP=1', '-v', 'VERBOSITY=sqlstate', '-At', databaseUrl],
      { input, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true, timeout: 15_000 }).trim();
  } catch (failure) {
    const error = new Error('OWNED_SQL_FAILED');
    const state = /ERROR:\s+([0-9A-Z]{5})\b/.exec(String(failure.stderr || ''))?.[1];
    error.code = state ? `SQLSTATE_${state}` : 'OWNED_SQL_TRANSPORT_FAILED';
    throw error;
  }
}
function sqlJson(statement) { try { return JSON.parse(sql(statement)); } catch (error) { if (error.code) throw error; throw new Error('OWNED_SQL_SHAPE_INVALID'); } }
function uuid7() {
  const bytes = randomBytes(16); bytes.writeUIntBE(Date.now(), 0, 6);
  bytes[6] = (bytes[6] & 15) | 112; bytes[8] = (bytes[8] & 63) | 128;
  const hex = bytes.toString('hex'); return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
function secretFree(value) {
  const forbidden = /^(?:secret|seed|otpauthUri|recoveryCodes|password|token|invitationToken|deliveryMaterial|clientSecret|access_token|refresh_token|id_token|accessToken|refreshToken|idToken)$/;
  return !value || typeof value !== 'object' || Object.entries(value).every(([key, child]) => !forbidden.test(key) && secretFree(child));
}
function base32(raw) {
  assert(/^[A-Z2-7]{16,128}$/.test(raw), 'ONE_TIME_ENROLLMENT_FORMAT_INVALID');
  let accumulator = 0, bits = 0; const bytes = [];
  for (const character of raw) {
    accumulator = (accumulator << 5) | 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'.indexOf(character); bits += 5;
    if (bits >= 8) { bits -= 8; bytes.push((accumulator >>> bits) & 255); }
  }
  return Buffer.from(bytes);
}
function totp(key, counter) {
  const message = Buffer.alloc(8); message.writeBigUInt64BE(BigInt(counter));
  const digest = createHmac('sha1', key).update(message).digest();
  const offset = digest[digest.length - 1] & 15;
  const code = ((digest.readUInt32BE(offset) & 0x7fffffff) % 1_000_000).toString().padStart(6, '0');
  digest.fill(0); message.fill(0); return code;
}
const factors = new Map();
async function nextCode(actor) {
  const factor = factors.get(actor); assert(factor, 'IN_MEMORY_FACTOR_REQUIRED');
  for (;;) {
    const seconds = Number(sql('SELECT extract(epoch FROM clock_timestamp())'));
    assert(Number.isFinite(seconds), 'OWNED_CLOCK_UNAVAILABLE');
    const counter = Math.floor(seconds / 30);
    // Enrollment and login consume a step. Wait for the actual next clock
    // boundary rather than changing last_accepted_step or forging MFA facts.
    if (counter > factor.lastStep && seconds % 30 < 27) { factor.lastStep = counter; return totp(factor.key, counter); }
    await delay(Math.min(30_000, Math.max(250, (30 - seconds % 30) * 1000 + 150)));
  }
}

process.env.PLAYWRIGHT_BROWSERS_PATH = path.join(repository, 'frontend/tests/browser/.browsers');
const require = createRequire(path.join(repository, 'frontend/tests/browser/package.json'));
const { chromium } = require('playwright');
const report = { contractVersion: 'unified-auth-phase19-real-https/1', date: new Date().toISOString(),
  browser: product, playwright: require('playwright/package.json').version, tlsErrorsIgnored: false, httpFixtures: false,
  realEnrollment: false, positiveMfaSeededBySql: false, staleBoundaryInjected: false, fixtureProductionBindingApplied: false,
  productionResponsibility: 'isolated fixture only; does not establish real Q3 production responsibility',
  manualProofReferences: 'isolated fixture references; not real partner/domain verification',
  invitationDelivery: false, checks: [], endpoints: {}, result: 'FAILED' };
let browser, step = 'owned-phase19-preflight', substep = 'preflight', activePage, interrupted = false, originalBinding, mfaCleanupEligible = false;
const contexts = [], observations = new Set(), protocols = new Set(), seen = new Set(), boundaryRestores = new Map(), auditBaseline = new Map();
function interrupt() { interrupted = true; process.exitCode = 1; if (browser) void browser.close().catch(() => {}); }
process.on('SIGINT', interrupt); process.on('SIGTERM', interrupt);
function pass(name, facts = {}) { report.checks.push({ name, result: 'PASS', ...facts }); }
function observe(page) {
  page.on('response', response => {
    const url = new URL(response.url()); if (![CONSOLE, AUTH].includes(url.origin)) return;
    const pathname = url.pathname.replace(/\/[0-9a-f-]{36}(?=\/|$)/g, '/:id'); seen.add(pathname);
    if (/^\/(?:auth|oauth|api\/sso|api\/identity)\//.test(pathname)) {
      const key = `${pathname} ${response.status()}`; report.endpoints[key] = (report.endpoints[key] || 0) + 1;
    }
    if (response.request().isNavigationRequest()) {
      const item = response.securityDetails().then(details => { if (details?.protocol) protocols.add(details.protocol); })
        .catch(() => {}).finally(() => observations.delete(item)); observations.add(item);
    }
  });
}
async function newPage() {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, serviceWorkers: 'block' }); contexts.push(context);
  const page = await context.newPage(); activePage = page; page.setDefaultTimeout(25_000); page.setDefaultNavigationTimeout(40_000); observe(page); return page;
}
async function browserJson(page, endpoint, method = 'GET', body, extra = {}) {
  assert(new URL(page.url()).origin === CONSOLE || new URL(page.url()).origin === AUTH, 'CONTROLLED_BROWSER_ORIGIN_REQUIRED');
  assert(endpoint.startsWith('/') && !endpoint.startsWith('//') && !endpoint.includes('?') && !endpoint.includes('#'), 'FIXED_ENDPOINT_REQUIRED');
  return page.evaluate(async ({ endpoint, method, body, extra }) => {
    const response = await fetch(endpoint, { method, credentials: 'same-origin', redirect: 'error', cache: 'no-store',
      headers: { Accept: 'application/json', ...(body === undefined ? {} : { 'Content-Type': 'application/json' }), ...extra },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }) });
    const text = await response.text(); let value = null; try { value = text ? JSON.parse(text) : null; } catch { /* fixed assertion at caller */ }
    return { status: response.status, noStore: response.headers.get('cache-control')?.includes('no-store') === true, body: value };
  }, { endpoint, method, body, extra });
}
async function session(page, actor) {
  const response = await browserJson(page, '/auth/session');
  assert(response.status === 200 && response.noStore && secretFree(response.body), 'REAL_BFF_SESSION_FAILED');
  const value = response.body;
  assert(value.account?.id === ids[actor] && uuidPattern.test(value.sessionId) && /^[1-9][0-9]*$/.test(value.version), 'REAL_BFF_ACTOR_MISMATCH');
  assert(value.context?.memberType === 'EMPLOYEE' && value.context.membershipId === ids[`${actor}Member`], 'REAL_EMPLOYEE_CONTEXT_MISMATCH');
  return value;
}
async function source(page, actor) {
  const view = await session(page, actor);
  const proof = sqlJson(`SELECT row_to_json(facts) FROM (SELECT b.id,b.global_session_id,b.selected_session_id,s.mfa_method,s.mfa_verified_at
    FROM bff_sessions b JOIN sso_sessions s ON s.id=b.global_session_id WHERE b.id=${literal(view.sessionId)}::uuid AND b.user_id=${literal(ids[actor])}::uuid) facts`);
  assert(uuidPattern.test(proof.id) && uuidPattern.test(proof.global_session_id) && uuidPattern.test(proof.selected_session_id), 'REAL_SOURCE_BINDING_MISSING'); return proof;
}
async function businessToken(page, actor) {
  const view = await session(page, actor);
  const response = await browserJson(page, '/auth/context-token', 'POST', { expectedVersion: view.version }, { 'X-CSRF-Token': view.csrfToken });
  assert(response.status === 200 && typeof response.body?.accessToken === 'string', 'REAL_CONTEXT_TOKEN_FAILED'); return response.body.accessToken;
}
function allowedApi(endpoint) {
  return ['/api/identity/review/security', '/api/identity/review/partners', '/api/identity/review/applications', '/api/identity/invitations'].includes(endpoint)
    || /^\/api\/identity\/review\/(?:applications\/[0-9a-f-]{36}\/(?:requests|operators)|requests\/[0-9a-f-]{36}\/(?:decisions|sync)|operations\/[0-9a-f-]{36})$/.test(endpoint)
    || /^\/api\/identity\/(?:invitations\/[0-9a-f-]{36}\/revoke|invitation-operations\/[0-9a-f-]{36})$/.test(endpoint);
}
async function api(page, actor, endpoint, method = 'GET', body, operation) {
  assert(allowedApi(endpoint) && new URL(page.url()).origin === CONSOLE, 'PHASE19_FIXED_API_REQUIRED');
  const bearer = await businessToken(page, actor);
  return browserJson(page, endpoint, method, body, { Authorization: `Bearer ${bearer}`, ...(operation ? { 'X-Identity-Operation': operation } : {}) });
}
async function registryPage(page, actor) {
  assert(new URL(page.url()).origin === CONSOLE, 'REGISTRY_ORIGIN_REQUIRED');
  const bearer = await businessToken(page, actor);
  return page.evaluate(async authorization => {
    const response = await fetch('/api/identity/clients?limit=10&offset=0', {
      credentials: 'same-origin', redirect: 'error', cache: 'no-store',
      headers: { Accept: 'application/json', Authorization: authorization },
    });
    let body = null; try { body = await response.json(); } catch { /* status still proves rejection */ }
    return { status: response.status, body };
  }, `Bearer ${bearer}`);
}
function registryObservation(response) {
  const body = response?.body, object = body && typeof body === 'object' && !Array.isArray(body);
  const page = object && body.page && typeof body.page === 'object' && !Array.isArray(body.page) ? body.page : null;
  const pageValues = Object.fromEntries(['limit', 'offset', 'total'].map(key => [key,
    Number.isSafeInteger(page?.[key]) && page[key] >= 0 && page[key] <= 1_000_000_000 ? page[key] : null]));
  return { status: Number.isSafeInteger(response?.status) ? response.status : null,
    bodyKind: object ? 'OBJECT' : Array.isArray(body) ? 'ARRAY' : body === null ? 'NULL' : 'OTHER',
    hasRecordsArray: Array.isArray(body?.records), recordCount: Array.isArray(body?.records) ? body.records.length : null,
    page: pageValues, fixedErrorCode: object && ['UNAUTHORIZED', 'FORBIDDEN', 'OPERATION_DENIED', 'NOT_FOUND',
      'BAD_REQUEST', 'CONFLICT', 'SERVICE_UNAVAILABLE', 'TOO_MANY_REQUESTS', 'SERVER_ERROR',
      'OIDC_REMOTE_RESULT_UNKNOWN'].includes(body.code) ? body.code : null,
    separatePublicClientIdentifiers: Array.isArray(body?.records)
      ? body.records.filter(value => typeof value?.id === 'string' && typeof value?.clientId === 'string' && value.id !== value.clientId).length : null };
}
async function command(page, actor, endpoint, body, kind = 'review') {
  const operation = uuid7(); const response = await api(page, actor, endpoint, 'POST', body, operation);
  assert(secretFree(response.body), 'MANAGEMENT_RESPONSE_CONTAINS_SECRET');
  if (response.status === 200) { if (response.body?.operationId) assert(response.body.operationId === operation, 'OPERATION_RECEIPT_MISMATCH'); return response.body; }
  assert(response.status === 503 && response.body?.code === 'OIDC_REMOTE_RESULT_UNKNOWN' && response.body.operationId === operation, 'REAL_COMMAND_FAILED');
  for (let attempt = 0; attempt < 12; attempt += 1) {
    const lookup = await api(page, actor, `/api/identity/${kind === 'invite' ? 'invitation-' : 'review/'}operations/${operation}`);
    assert(secretFree(lookup.body), 'OPERATION_LOOKUP_CONTAINS_SECRET');
    if (lookup.status === 200 && lookup.body?.state === 'COMPLETED') { assert(lookup.body.operationId === operation, 'ORIGINAL_OPERATION_MISMATCH'); return lookup.body.result; }
    assert(lookup.status === 503 && lookup.body?.code === 'OIDC_REMOTE_RESULT_UNKNOWN', 'ORIGINAL_OPERATION_LOOKUP_FAILED');
    await delay(750);
  }
  assert.fail('ORIGINAL_OPERATION_STILL_PENDING');
}
async function apiDenied(page, actor, endpoint, body, code = 'FORBIDDEN') {
  const response = await api(page, actor, endpoint, 'POST', body, uuid7());
  assert(response.status === (code === 'BAD_REQUEST' ? 400 : 403) && response.body?.code === code, 'EXPECTED_MANAGEMENT_DENIAL_MISSING');
}
async function finishFlow(page, actor) {
  activePage = page;
  substep = 'identity-transition';
  const outcome = await page.waitForFunction(() => {
    if (location.origin === 'https://console.saas.test' && !location.pathname.startsWith('/auth/')) return 'console';
    if (document.querySelector('#error-message:not([hidden])')) return 'error';
    if (document.querySelector('#auth-content')?.dataset.screen === 'CONSENT_REQUIRED'
      && !document.querySelector('#consent-panel')?.hidden
      && document.querySelector('#consent-allow')?.disabled === false) return 'consent';
    return false;
  });
  const status = await outcome.jsonValue(); await outcome.dispose(); assert(status !== 'error', 'REAL_IDENTITY_FLOW_REJECTED');
  if (status === 'consent') {
    // Each fixture actor has one eligible member. Submit the real consent once
    // and wait for its callback; a disabled in-flight button is not a new step.
    substep = 'consent-submit';
    await page.locator('#consent-allow').click();
  }
  substep = 'console-return';
  await page.waitForURL(url => url.origin === CONSOLE && !url.pathname.startsWith('/auth/'), { waitUntil: 'domcontentloaded' });
  substep = 'initial-console-session';
  await session(page, actor);
  // Stop the actual host refresh manager before becoming the sole context-token
  // consumer. This is a real same-origin JSON page, never an intercepted response.
  // The management host polls its live session, so networkidle is not a valid
  // completion condition. Navigation ends that refresh driver after the real
  // callback/session proof above has succeeded.
  substep = 'single-driver-config-navigation';
  await page.goto(`${CONSOLE}/auth/config`, { waitUntil: 'domcontentloaded' });
  const cookies = await page.context().cookies([CONSOLE, AUTH]);
  for (const [name, domain] of [['__Host-saas_console', 'console.saas.test'], ['__Host-saas_sso', 'auth.saas.test']]) {
    const cookie = cookies.find(row => row.name === name); assert(cookie?.secure && cookie.httpOnly && cookie.sameSite === 'Lax' && cookie.domain === domain && cookie.path === '/', 'HOST_COOKIE_CONTRACT_FAILED');
  }
  return source(page, actor);
}
async function login(page, actor, mode = 'initial') {
  activePage = page; substep = 'primary-login-navigation';
  const prior = mode === 'initial' ? null : await source(page, actor);
  await page.goto(`${CONSOLE}${mode === 'initial' ? '/auth/login?return=/system/oauth/clients' : ENTRY}`, { waitUntil: 'domcontentloaded' });
  substep = 'primary-login-form'; await page.locator('#login-form:not([hidden])').waitFor();
  const credentials = environment.accounts[actor];
  await page.locator('#username').fill(credentials.username); await page.locator('#password').fill(credentials.password);
  const primary = page.waitForResponse(response => response.url() === `${AUTH}/api/sso/login` && response.request().method() === 'POST');
  await page.locator('#login-submit').click(); const first = await primary;
  if (mode !== 'initial') {
    assert(first.status() === 428, 'FRESH_PRIMARY_DID_NOT_REQUIRE_MFA'); await page.locator('#mfa-field:not([hidden])').waitFor();
    await page.locator('#password').fill(credentials.password);
    const factor = factors.get(actor);
    const code = mode === 'recovery' ? factor?.recovery.shift() : await nextCode(actor);
    assert(typeof code === 'string' && code.length >= 6, 'ONE_TIME_CODE_UNAVAILABLE');
    await page.locator('#mfa-code').fill(code);
    const accepted = page.waitForResponse(response => response.url() === `${AUTH}/api/sso/login` && response.request().method() === 'POST');
    await page.locator('#login-submit').click(); assert((await accepted).status() === 200, 'REAL_PASSWORD_MFA_LOGIN_FAILED');
  } else assert(first.status() === 200, 'INITIAL_PASSWORD_LOGIN_FAILED');
  const current = await finishFlow(page, actor);
  if (prior) {
    assert(current.id !== prior.id && current.global_session_id !== prior.global_session_id && current.selected_session_id !== prior.selected_session_id, 'STEP_UP_DID_NOT_REESTABLISH_ALL_REAL_SESSIONS');
    assert(sql(`SELECT expires_at>clock_timestamp() AND NOT platform_session_active(id) FROM platform_sessions WHERE id=${literal(prior.selected_session_id)}::uuid AND user_id=${literal(ids[actor])}::uuid`) === 't', 'OLD_UNEXPIRED_PLATFORM_GENERATION_NOT_FENCED');
  }
  assert(mode === 'initial' ? current.mfa_method === null : current.mfa_method === (mode === 'recovery' ? 'recovery_code' : 'totp'), 'REAL_SOURCE_MFA_METHOD_MISMATCH');
  return current;
}
async function enroll(page, actor) {
  activePage = page; substep = 'security-navigation';
  await page.goto(`${AUTH}/sso/security`, { waitUntil: 'domcontentloaded' });
  substep = 'security-panel'; await page.locator('#security-panel:not([hidden])').waitFor(); await page.locator('#start-enrollment:not([hidden])').click();
  await page.locator('#enrollment-panel:not([hidden])').waitFor();
  const seed = (await page.locator('#secret').textContent()).trim();
  const uri = new URL((await page.locator('#otpauth-uri').textContent()).trim());
  assert(uri.protocol === 'otpauth:' && uri.hostname === 'totp' && uri.searchParams.get('secret') === seed
    && uri.searchParams.get('algorithm') === 'SHA1' && uri.searchParams.get('digits') === '6' && uri.searchParams.get('period') === '30', 'REAL_ENROLLMENT_CONTRACT_FAILED');
  factors.set(actor, { key: base32(seed), lastStep: -1, recovery: [] });
  await page.locator('#confirm-code').fill(await nextCode(actor));
  await page.locator('#confirm-form button[type=submit]').click(); await page.locator('#recovery-panel:not([hidden])').waitFor();
  const recovery = await page.locator('#recovery-codes code').allTextContents();
  assert(recovery.length === 10 && recovery.every(code => /^[0-9a-f -]{32,40}$/.test(code)), 'REAL_RECOVERY_OUTPUT_INVALID');
  factors.get(actor).recovery = recovery;
  await page.locator('#recovery-done').click();
  // Remove one-time enrollment text after capture; never screenshot/export it.
  await page.evaluate(() => { document.querySelector('#secret').textContent = ''; document.querySelector('#otpauth-uri').textContent = ''; });
  await page.goto(`${CONSOLE}/auth/config`, { waitUntil: 'domcontentloaded' });
  // Enrollment may have upgraded the very same SSO source as the old BFF.
  // Always exercise the controlled fresh primary flow, without assuming old=false.
  const before = await source(page, actor); const after = await login(page, actor, 'totp');
  assert(before.id !== after.id && after.mfa_method === 'totp', 'REAL_ENROLLMENT_STEP_UP_FAILED');
  const assurance = await api(page, actor, '/api/identity/review/security');
  assert(assurance.status === 200 && assurance.body.totpEnabled === true && assurance.body.recent === true && assurance.body.reauthenticateUrl === ENTRY, 'REAL_TOTP_ASSURANCE_NOT_VISIBLE_TO_PLATFORM');
}
function applicationManifest() { return { displayName: 'Owned HTTPS Phase19 partner', purpose: 'Isolated fixture account login', privacyUrl: 'https://partner.example.invalid/privacy', domains: ['partner.example.invalid'], redirectUris: ['https://partner.example.invalid/callback'], postLogoutRedirectUris: [], scopes: ['openid', 'profile'], contactReference: uuid7(), domainEvidenceReference: uuid7(), logoutMode: 'SHORT_SESSION_ONLINE_CHECK' }; }
function decision(input, version, status) { return { expectedVersion: version, status, reason: 'Owned HTTPS fixture independent evidence review', verifiedContactReference: input.contactReference, verifiedDomainReference: input.domainEvidenceReference }; }
function invitationMetadata(value) {
  const fields = ['id', 'recipientRef', 'purpose', 'status', 'version', 'createdAt', 'expiresAt', 'revokedAt', 'consumedAt'];
  assert(value && Object.keys(value).every(key => fields.includes(key)) && fields.every(key => Object.hasOwn(value, key))
    && uuidPattern.test(value.id) && value.purpose === 'ACCOUNT_ONBOARDING' && secretFree(value), 'INVITATION_METADATA_CONTRACT_FAILED');
}
function restoreBoundary(sid, record) {
  assert(uuidPattern.test(sid) && ACTORS.some(actor => ids[actor] === record.user), 'OWNED_BOUNDARY_RESTORE_REQUIRED');
  sql(`UPDATE sso_sessions SET mfa_verified_at=${record.at === null ? 'NULL' : `${literal(record.at)}::timestamptz`},mfa_method=${record.method === null ? 'NULL' : literal(record.method)} WHERE id=${literal(sid)}::uuid AND user_id=${literal(record.user)}::uuid`);
  boundaryRestores.delete(sid);
}
function restoreReleaseBinding() {
  if (!originalBinding) return;
  const original = originalBinding;
  sql(`UPDATE platform_auth_release_bindings SET enabled=original.enabled,reviewer_user_ids=original.reviewer_user_ids,publisher_user_ids=original.publisher_user_ids,policy_reference=original.policy_reference,operations_reference=original.operations_reference,verified_at=original.verified_at FROM jsonb_populate_record(NULL::platform_auth_release_bindings,${literal(JSON.stringify(original))}::jsonb) original WHERE platform_auth_release_bindings.singleton`);
  originalBinding = null;
}
function enrollmentAudit(actor) {
  return sqlJson(`SELECT json_build_object('started',count(*) FILTER(WHERE action='MFA_ENROLLMENT_STARTED'),'enabled',count(*) FILTER(WHERE action='MFA_ENABLED')) FROM identity_audit_logs WHERE user_id=${literal(ids[actor])}::uuid`);
}
function resetOwnedMfa() {
  const actorIds = ACTORS.map(actor => `${literal(ids[actor])}::uuid`).join(',');
  // The manifest names three newly created fixture accounts, never the existing
  // admin, employee or customer. Preserve their audit; fence sources first.
  sql(`UPDATE sso_sessions SET revoked_at=clock_timestamp() WHERE user_id=ANY(ARRAY[${actorIds}]) AND revoked_at IS NULL;
    DELETE FROM auth_totp_recovery_codes WHERE user_id=ANY(ARRAY[${actorIds}]);
    DELETE FROM auth_totp_enrollments WHERE user_id=ANY(ARRAY[${actorIds}]);
    DELETE FROM auth_totp_factors WHERE user_id=ANY(ARRAY[${actorIds}])`);
  const remaining = sqlJson(`SELECT json_build_object(
    'factors',(SELECT count(*) FROM auth_totp_factors WHERE user_id=ANY(ARRAY[${actorIds}])),
    'enrollments',(SELECT count(*) FROM auth_totp_enrollments WHERE user_id=ANY(ARRAY[${actorIds}])),
    'recoveryRows',(SELECT count(*) FROM auth_totp_recovery_codes WHERE user_id=ANY(ARRAY[${actorIds}])),
    'unrevokedSso',(SELECT count(*) FROM sso_sessions WHERE user_id=ANY(ARRAY[${actorIds}]) AND revoked_at IS NULL))`);
  assert(Object.values(remaining).every(count => count === 0), 'OWNED_MFA_CLEANUP_INCOMPLETE');
  report.ownedMfaResidualRows = remaining;
}

try {
  const factorCount = sql('SELECT count(*) FROM auth_totp_factors WHERE user_id=ANY(ARRAY[' + ACTORS.map(actor => `${literal(ids[actor])}::uuid`).join(',') + '])');
  mfaCleanupEligible = true;
  assert(factorCount === '0', 'FRESH_UNENROLLED_ACTORS_REQUIRED');
  for (const actor of ACTORS) auditBaseline.set(actor, enrollmentAudit(actor));
  originalBinding = sqlJson('SELECT row_to_json(binding) FROM platform_auth_release_bindings binding WHERE singleton');
  assert(originalBinding.enabled === false, 'PRODUCTION_MUST_START_CLOSED');
  browser = await chromium.launch({ headless: true, proxy: { server: proxy }, ...(product === 'chromium' ? {} : { channel: product }) });
  report.browserVersion = browser.version(); assert(!interrupted, 'BROWSER_INTERRUPTED');
  const applicant = await newPage();
  step = 'real-unenrolled-primary-and-sensitive-denial'; await login(applicant, 'applicant');
  const missing = await api(applicant, 'applicant', '/api/identity/review/security');
  assert(missing.status === 200 && missing.body.totpEnabled === false && missing.body.recent === false, 'MISSING_FACTOR_BOOTSTRAP_FAILED');
  const partnerBody = { name: 'Owned HTTPS partner fixture', contactName: 'Fixture responsible contact', contactChannel: 'verified-channel:owned-https-fixture', subjectReference: uuid7() };
  await apiDenied(applicant, 'applicant', '/api/identity/review/partners', partnerBody); pass(step);

  step = 'real-applicant-enrollment-and-fresh-primary-totp-new-bff'; await enroll(applicant, 'applicant'); report.realEnrollment = true; pass(step, { enrollmentThroughRealUi: true, freshPrimaryAndTotp: true, newBffAndSource: true });
  step = 'canonical-registry-pagination-through-real-private-guard';
  const registry = await registryPage(applicant, 'applicant');
  substep = 'registry-response-validation';
  report.registryObservation = registryObservation(registry);
  assert(registry.status === 200 && Array.isArray(registry.body?.records)
    && registry.body.page?.limit === 10 && registry.body.page?.offset === 0 && secretFree(registry.body), 'CANONICAL_REGISTRY_PAGINATION_FAILED');
  pass(step, { exactGetQuery: true, privateIdentityGuardExecuted: true });
  step = 'real-partner-and-application-submission';
  const partner = await command(applicant, 'applicant', '/api/identity/review/partners', partnerBody); assert(uuidPattern.test(partner.id), 'REAL_PARTNER_ID_MISSING');
  const app = await command(applicant, 'applicant', '/api/identity/review/applications', { partnerId: partner.id, displayName: 'Owned HTTPS partner fixture application' }); assert(uuidPattern.test(app.applicationId), 'REAL_APPLICATION_ID_MISSING');
  const appPath = `/api/identity/review/applications/${app.applicationId}`;
  const input = applicationManifest();
  await apiDenied(applicant, 'applicant', `${appPath}/requests`, { environment: 'test', expectedVersion: null, manifest: { ...input, scopes: ['openid', 'email'] } }, 'BAD_REQUEST');
  const test = await command(applicant, 'applicant', `${appPath}/requests`, { environment: 'test', expectedVersion: null, manifest: input });
  assert(test.status === 'SUBMITTED' && test.version === 1 && test.requestedBy === ids.applicant, 'REAL_SUBMISSION_FAILED'); pass(step);

  const reviewer = await newPage(); step = 'real-reviewer-enrollment-and-new-bff'; await login(reviewer, 'reviewer'); await enroll(reviewer, 'reviewer'); pass(step);
  const testPath = `/api/identity/review/requests/${test.id}/decisions`;
  step = 'real-independent-test-approval-and-identity-admission';
  const approved = await command(reviewer, 'reviewer', testPath, decision(input, 1, 'TEST_APPROVED'));
  assert(approved.status === 'TEST_APPROVED' && approved.version === 2 && approved.syncedVersion === 2 && approved.syncState === 'COMPLETED' && approved.reviewedBy === ids.reviewer, 'REAL_TEST_ADMISSION_NOT_CONFIRMED');
  const accepted = await command(reviewer, 'reviewer', testPath, { ...decision(input, 2, 'TEST_ACCEPTED'), testEvidenceReference: uuid7() });
  assert(accepted.status === 'TEST_ACCEPTED' && accepted.version === 3 && accepted.syncedVersion === 2, 'REAL_TEST_ACCEPTANCE_FAILED'); pass(step);

  step = 'real-invitation-metadata-only-issue-and-revoke';
  const invitation = await command(reviewer, 'reviewer', '/api/identity/invitations', { recipientRef: `owned-fixture-recipient:${uuid7()}`, purpose: 'ACCOUNT_ONBOARDING' }, 'invite'); invitationMetadata(invitation);
  assert(invitation.status === 'ACTIVE' && Math.abs(Date.parse(invitation.expiresAt) - Date.parse(invitation.createdAt) - 86_400_000) < 2000, 'REAL_INVITATION_EXPIRY_FAILED');
  const revoked = await command(reviewer, 'reviewer', `/api/identity/invitations/${invitation.id}/revoke`, { expectedVersion: invitation.version }, 'invite'); invitationMetadata(revoked);
  assert(revoked.id === invitation.id && revoked.status === 'REVOKED' && revoked.version === invitation.version + 1, 'REAL_INVITATION_REVOCATION_FAILED'); pass(step, { deliveryAttempted: false });

  step = 'real-totp-source-with-stale-boundary-denies-sensitive-write';
  const currentSource = await source(reviewer, 'reviewer');
  const original = { user: ids.reviewer, at: currentSource.mfa_verified_at, method: currentSource.mfa_method };
  boundaryRestores.set(currentSource.global_session_id, original);
  sql(`UPDATE sso_sessions SET mfa_verified_at=clock_timestamp()-interval '6 minutes' WHERE id=${literal(currentSource.global_session_id)}::uuid AND user_id=${literal(ids.reviewer)}::uuid`);
  report.staleBoundaryInjected = true;
  const stale = await api(reviewer, 'reviewer', '/api/identity/review/security'); assert(stale.status === 200 && stale.body.recent === false, 'STALE_BOUNDARY_REPORTED_RECENT');
  await apiDenied(reviewer, 'reviewer', '/api/identity/invitations', { recipientRef: 'owned-fixture-stale-denial', purpose: 'ACCOUNT_ONBOARDING' });
  restoreBoundary(currentSource.global_session_id, original); pass(step, { timestampBoundaryInjected: true, restoredOriginalRealTimestamp: true });

  step = 'real-recovery-login-cannot-authorize-totp-management';
  await login(reviewer, 'reviewer', 'recovery');
  const recovery = await api(reviewer, 'reviewer', '/api/identity/review/security'); assert(recovery.status === 200 && recovery.body.totpEnabled === true && recovery.body.recent === false, 'RECOVERY_SOURCE_REPORTED_AS_TOTP');
  await apiDenied(reviewer, 'reviewer', '/api/identity/invitations', { recipientRef: 'owned-fixture-recovery-denial', purpose: 'ACCOUNT_ONBOARDING' });
  await login(reviewer, 'reviewer', 'totp'); pass(step, { actualRecoveryLogin: true, sensitiveWriteDenied: true, restoredByRealPrimaryTotp: true });

  step = 'production-default-closed-before-fixture-binding';
  // Renew the applicant by a real password/TOTP flow after the other actors'
  // enrollment waits, never by copying or advancing its MFA timestamp.
  await login(applicant, 'applicant', 'totp');
  const production = await command(applicant, 'applicant', `${appPath}/requests`, { environment: 'production', expectedVersion: null, manifest: input });
  const productionPath = `/api/identity/review/requests/${production.id}/decisions`;
  await apiDenied(reviewer, 'reviewer', productionPath, decision(input, 1, 'PRODUCTION_APPROVED')); pass(step);

  const publisher = await newPage(); step = 'real-publisher-enrollment-and-explicit-assignment'; await login(publisher, 'publisher'); await enroll(publisher, 'publisher');
  await command(applicant, 'applicant', `${appPath}/operators`, { userId: ids.publisher }); pass(step);
  step = 'fixture-bound-production-real-review-publish-admission-suspend';
  // This is a deployment fixture boundary only; real Q3 ownership remains open.
  sql(`UPDATE platform_auth_release_bindings SET enabled=true,reviewer_user_ids=ARRAY[${literal(ids.reviewer)}::uuid],publisher_user_ids=ARRAY[${literal(ids.publisher)}::uuid],policy_reference='fixture-phase19-policy-not-production',operations_reference='fixture-phase19-operations-not-production',verified_at=clock_timestamp() WHERE singleton`);
  report.fixtureProductionBindingApplied = true;
  const productionApproved = await command(reviewer, 'reviewer', productionPath, decision(input, 1, 'PRODUCTION_APPROVED')); assert(productionApproved.version === 2 && productionApproved.status === 'PRODUCTION_APPROVED', 'REAL_FIXTURE_PRODUCTION_REVIEW_FAILED');
  const enabled = await command(publisher, 'publisher', productionPath, decision(input, 2, 'ENABLED'));
  assert(enabled.status === 'ENABLED' && enabled.version === 3 && enabled.syncedVersion === 3 && enabled.syncState === 'COMPLETED' && enabled.publishedBy === ids.publisher, 'REAL_FIXTURE_PRODUCTION_ADMISSION_FAILED');
  const suspended = await command(publisher, 'publisher', productionPath, decision(input, 3, 'SUSPENDED'));
  assert(suspended.status === 'SUSPENDED' && suspended.version === 4 && suspended.syncedVersion === 4 && suspended.syncState === 'COMPLETED', 'REAL_FIXTURE_PRODUCTION_SUSPENSION_FAILED');
  restoreReleaseBinding(); pass(step, { responsibilityBinding: 'fixture only', identityAdmissionOverRealHttps: true, releaseBindingRestored: true });

  step = 'redacted-durable-evidence-and-tls';
  assert(sql(`SELECT count(*) FROM auth_totp_factors WHERE user_id=ANY(ARRAY[${ACTORS.map(actor => `${literal(ids[actor])}::uuid`).join(',')}])`) === '3', 'REAL_FACTOR_COUNT_MISMATCH');
  assert(sql("SELECT NOT enabled FROM platform_auth_release_bindings WHERE singleton") === 't', 'PRODUCTION_BINDING_NOT_RESTORED');
  const actorIds = ACTORS.map(actor => `${literal(ids[actor])}::uuid`).join(',');
  for (const actor of ACTORS) {
    const baseline = auditBaseline.get(actor), actual = enrollmentAudit(actor);
    assert(actual.started - baseline.started === 1 && actual.enabled - baseline.enabled === 1, 'REAL_ENROLLMENT_AUDIT_INCREMENT_MISSING');
  }
  report.enrollmentAuditAdded = { started: 3, enabled: 3 };
  assert(sql(`SELECT NOT EXISTS(SELECT 1 FROM identity_audit_logs WHERE user_id=ANY(ARRAY[${actorIds}]) AND action IN ('MFA_ENABLED','MFA_ENROLLMENT_STARTED','MFA_STEP_UP','SSO_LOGIN') AND detail::jsonb ?| ARRAY['secret','seed','recoveryCodes','password','code','token'])`) === 't', 'SECRET_BEARING_SECURITY_AUDIT');
  await Promise.allSettled([...observations]); assert(seen.has('/auth/step-up') && seen.has('/api/sso/mfa/enrollment/confirm') && seen.has('/auth/callback'), 'REAL_AUTHENTICATION_PATH_MISSING');
  assert(!seen.has('/oauth/token') && !seen.has('/oauth/introspect'), 'SERVER_TOKEN_ENDPOINT_REACHED_BY_BROWSER');
  assert(protocols.size > 0 && [...protocols].every(protocol => /^TLS [12]\./.test(protocol)), 'REAL_TLS_EVIDENCE_MISSING');
  pass(step); assert(!interrupted, 'BROWSER_INTERRUPTED'); report.result = 'PASS';
} catch (error) {
  report.failedStep = step;
  report.failedSubstep = substep;
  try {
    if (activePage && !activePage.isClosed()) {
      const current = new URL(activePage.url());
      report.pageState = { origin: [AUTH, CONSOLE].includes(current.origin) ? current.origin : 'OTHER',
        pathname: current.pathname.replace(/\/[0-9a-f-]{36}(?=\/|$)/g, '/:id'),
        view: await activePage.evaluate(() => ({ ready: document.readyState,
          authScreen: /^[A-Z_]{1,40}$/.test(document.querySelector('#auth-content')?.dataset.screen || '')
            ? document.querySelector('#auth-content').dataset.screen : null,
          loginVisible: !!document.querySelector('#login-form:not([hidden])'),
          securityVisible: !!document.querySelector('#security-panel:not([hidden])'),
          enrollmentVisible: !!document.querySelector('#enrollment-panel:not([hidden])'),
          recoveryVisible: !!document.querySelector('#recovery-panel:not([hidden])') })) };
    }
  } catch { report.pageState = { diagnostic: 'SAFE_STATE_UNAVAILABLE' }; }
  const network = /net::(ERR_[A-Z0-9_]+)/.exec(error?.message || '')?.[1];
  report.failureCode = interrupted ? 'BROWSER_INTERRUPTED' : /^SQLSTATE_[0-9A-Z]{5}$/.test(error?.code || '') ? error.code
    : error?.code === 'ERR_ASSERTION' && /^[A-Z0-9_]+$/.test(error.message) ? error.message : network || (error?.name === 'TimeoutError' ? 'BROWSER_STEP_TIMEOUT' : 'BROWSER_STEP_FAILED');
  process.exitCode = 1;
} finally {
  await Promise.allSettled([...observations]);
  for (const context of contexts) await context.close().catch(() => {}); if (browser) await browser.close().catch(() => {});
  report.browserCleanup = { contextsClosed: contexts.every(context => context.pages().length === 0), browserClosed: !browser || !browser.isConnected() };
  if (!report.browserCleanup.contextsClosed || !report.browserCleanup.browserClosed) {
    report.result = 'FAILED'; report.failureCode = 'OWNED_BROWSER_CLEANUP_FAILED'; process.exitCode = 1;
  }
  const boundaryFailures = [];
  for (const [sid, record] of [...boundaryRestores]) {
    try { restoreBoundary(sid, record); }
    catch (error) { boundaryFailures.push(/^SQLSTATE_[0-9A-Z]{5}$/.test(error?.code || '') ? error.code : 'OWNED_MFA_TIMESTAMP_RESET_FAILED'); }
  }
  try { restoreReleaseBinding(); }
  catch (error) { boundaryFailures.push(/^SQLSTATE_[0-9A-Z]{5}$/.test(error?.code || '') ? error.code : 'OWNED_RELEASE_BINDING_RESET_FAILED'); }
  report.fixtureBoundaryReset = boundaryFailures.length === 0;
  if (boundaryFailures.length) {
    report.fixtureBoundaryResetFailures = boundaryFailures; report.result = 'FAILED'; process.exitCode = 1;
  }
  try {
    if (mfaCleanupEligible) { resetOwnedMfa(); report.ownedMfaBoundaryReset = true; }
    else report.ownedMfaBoundaryReset = false;
  } catch (error) {
    report.ownedMfaBoundaryReset = false; report.ownedMfaBoundaryResetFailure = /^SQLSTATE_[0-9A-Z]{5}$/.test(error?.code || '') ? error.code : 'OWNED_MFA_RESET_FAILED';
    report.result = 'FAILED'; process.exitCode = 1;
  }
  for (const factor of factors.values()) { factor.key.fill(0); factor.recovery.fill(''); } factors.clear();
  for (const account of Object.values(environment.accounts)) delete account.password;
  if (interrupted) { report.result = 'FAILED'; report.failureCode = 'BROWSER_INTERRUPTED'; }
  report.tlsProtocols = [...protocols];
  assert(secretFree(report), 'REPORT_SECRET_GUARD_FAILED');
  report.executionPhase = executionPhase; report.snapshotSha256 = ownedInputs.snapshotSha256;
  saveBrowserReport(outputFile, report, executionPhase);
  console.log(`PHASE19_HTTPS_BROWSER_RESULT ${JSON.stringify({ result: report.result, checks: report.checks.length, ...(report.failedStep ? { failedStep: report.failedStep, failureCode: report.failureCode } : {}) })}`);
  process.removeListener('SIGINT', interrupt); process.removeListener('SIGTERM', interrupt);
}
