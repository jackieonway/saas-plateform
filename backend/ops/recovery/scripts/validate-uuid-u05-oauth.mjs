#!/usr/bin/env node
/**
 * Real, operator-invoked UUID-U05 OAuth BFF acceptance. No fixture or SQL path.
 *
 * Linux, Node 24:
 *   node validate-uuid-u05-oauth.mjs --config /private/oauth-check.json
 *   node validate-uuid-u05-oauth.mjs --config /private/oauth-check.json --execute
 *
 * Owned mode-0600 config, outside the repository:
 * { publicOrigin, handoffFile, outputFile, credentialsFile? }
 * publicOrigin is an exact explicit-port HTTP loopback origin. handoffFile is
 * owned mode 0600 and contains login:{username,password} and applications
 * with {code:"oauth-playground",context:{userId,tenantId,organizationId}}.
 * The top-level handoff context belongs to HQ and is never selected here.
 * credentialsFile is an optional
 * legacy handoff reference: only its privacy is checked; its shared sessions
 * and refresh tokens are never read. Execution logs in once and uses its own
 * ticket to select the handoff business context, without switching a shared
 * session. BFF refresh/revoke affect only the newly granted BFF OAuth flow.
 *
 * Default mode validates inputs and writes a new private static report. It
 * makes no network request. --execute creates real login/OAuth protocol state.
 * Every redirect is inspected in memory, never followed. No request/response
 * body, URL, query, cookie, code, state, token, identity or credential is logged.
 * Reports contain only fixed check identifiers, statuses and aggregate counts.
 */
import fs from 'node:fs/promises';
import { constants } from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { randomBytes } from 'node:crypto';
import { fileURLToPath } from 'node:url';

const MAX_FILE = 8 * 1024 * 1024;
const MAX_RESPONSE = 256 * 1024;
const FLOW = 'authorization_code';
const V7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const PRIVATE_KEYS = new Set(['access_token', 'refreshtoken', 'refresh_token', 'accesstoken',
  'id_token', 'client_secret', 'password', 'code_verifier', 'device_code', 'authorization']);

class Rejected extends Error {
  constructor(code) { super('OAuth acceptance check rejected'); this.code = code; }
}
function requireCheck(condition, code) { if (!condition) throw new Rejected(code); }
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const token = value => typeof value === 'string' && value.length > 0 && value.length <= 16384 && !/[\s\x00-\x1f\x7f]/u.test(value);

/** The sole credentials adapter; never reads or returns a refresh token. */
export function extractCredentials(value) {
  requireCheck(object(value), 'CREDENTIALS_OBJECT_REQUIRED');
  for (const name of ['businessSession', 'platformSession', 'session']) {
    if (value[name] !== undefined) {
      requireCheck(object(value[name]) && token(value[name].accessToken), 'SESSION_ACCESS_TOKEN_REQUIRED');
      return { accessToken: value[name].accessToken };
    }
  }
  throw new Rejected('SESSION_ACCESS_TOKEN_REQUIRED');
}

async function checkedPath(value) {
  requireCheck(process.platform === 'linux' && typeof process.geteuid === 'function', 'LINUX_PRIVATE_FILES_REQUIRED');
  requireCheck(typeof value === 'string' && path.isAbsolute(value) && !value.includes('\0') &&
    !value.split('/').some(part => part === '.' || part === '..'), 'PRIVATE_ABSOLUTE_PATH_REQUIRED');
  const resolved = path.resolve(value);
  for (let current = resolved; ; current = path.dirname(current)) {
    const info = await fs.lstat(current);
    requireCheck(!info.isSymbolicLink(), 'PRIVATE_SYMLINK_REJECTED');
    if (current === path.dirname(current)) break;
  }
  return resolved;
}
async function privateFile(value) {
  const filename = await checkedPath(value);
  const info = await fs.lstat(filename);
  requireCheck(info.isFile() && info.nlink === 1, 'PRIVATE_REGULAR_FILE_REQUIRED');
  requireCheck(info.uid === process.geteuid() && (info.mode & 0o777) === 0o600, 'PRIVATE_OWNER_MODE_REQUIRED');
  requireCheck(info.size <= MAX_FILE, 'PRIVATE_FILE_TOO_LARGE');
  return filename;
}
async function readPrivateJson(value) {
  const filename = await privateFile(value);
  const handle = await fs.open(filename, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const info = await handle.stat();
    requireCheck(info.isFile() && info.nlink === 1 && info.uid === process.geteuid() &&
      (info.mode & 0o777) === 0o600 && info.size <= MAX_FILE, 'PRIVATE_OPEN_FILE_INVALID');
    const bytes = await handle.readFile();
    requireCheck(bytes.length <= MAX_FILE, 'PRIVATE_FILE_TOO_LARGE');
    try { return JSON.parse(bytes.toString('utf8').replace(/^\uFEFF/u, '')); }
    catch { throw new Rejected('PRIVATE_JSON_INVALID'); }
  } finally { await handle.close(); }
}
async function reserveOutput(value) {
  requireCheck(typeof value === 'string' && path.isAbsolute(value) &&
    !value.split('/').some(part => part === '.' || part === '..'), 'OUTPUT_ABSOLUTE_PATH_REQUIRED');
  const parent = await checkedPath(path.dirname(value));
  const info = await fs.lstat(parent);
  requireCheck(info.isDirectory() && info.uid === process.geteuid() && (info.mode & 0o777) === 0o700,
    'OUTPUT_PARENT_OWNER_MODE_REQUIRED');
  return fs.open(path.join(parent, path.basename(value)),
    constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600);
}
function origin(value) {
  let url;
  try { url = new URL(value); } catch { throw new Rejected('LOOPBACK_ORIGIN_REQUIRED'); }
  requireCheck(url.protocol === 'http:' && ['127.0.0.1', '[::1]'].includes(url.hostname) &&
    url.port && url.origin === value && !url.username && !url.password && !url.search && !url.hash,
  'LOOPBACK_ORIGIN_REQUIRED');
  return url.origin;
}
function uuid7() {
  const data = randomBytes(16);
  data.writeUIntBE(Date.now(), 0, 6);
  data[6] = (data[6] & 15) | 0x70;
  data[8] = (data[8] & 63) | 0x80;
  const hex = data.toString('hex');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
function noTokenMaterial(value) {
  if (Array.isArray(value)) return value.every(noTokenMaterial);
  if (!object(value)) return true;
  return Object.entries(value).every(([key, child]) => !PRIVATE_KEYS.has(key.toLowerCase()) && noTokenMaterial(child));
}
function tokenSummary(value) {
  return object(value) && Object.keys(value).sort().join(',') ===
    'expiresIn,flow,granted,hasRefreshToken,scope,tokenType' && value.granted === true &&
    value.flow === FLOW && typeof value.tokenType === 'string' && value.tokenType.toLowerCase() === 'bearer' &&
    Number.isSafeInteger(value.expiresIn) && value.expiresIn > 0 &&
    typeof value.scope === 'string' && value.scope.length <= 512 &&
    typeof value.hasRefreshToken === 'boolean' && noTokenMaterial(value);
}
function oneParameter(url, name) {
  const values = url.searchParams.getAll(name);
  requireCheck(values.length === 1 && values[0].length > 0, 'OAUTH_PARAMETER_INVALID');
  return values[0];
}
function sameOriginUrl(value, expectedOrigin, expectedPath) {
  let url;
  try { url = new URL(value, expectedOrigin); } catch { throw new Rejected('OAUTH_URL_INVALID'); }
  requireCheck(url.origin === expectedOrigin && url.pathname === expectedPath && !url.username &&
    !url.password && !url.hash, 'OAUTH_DESTINATION_REJECTED');
  return url;
}

class Runner {
  constructor(publicOrigin, report) { this.origin = publicOrigin; this.report = report; this.cookie = null; }
  pass(id) { this.report.checks.push({ check: id, status: 'PASS' }); }
  async step(stage, operation) { this.report.stage = stage; return operation(); }
  request(method, destination, { body, bearer, bff = false } = {}) {
    const url = new URL(destination, this.origin);
    requireCheck(url.origin === this.origin && !url.username && !url.password && !url.hash,
      'HTTP_DESTINATION_REJECTED');
    requireCheck(!bff || url.pathname.startsWith('/oauth-playground-api/'), 'BFF_PATH_REQUIRED');
    requireCheck(!bearer || !url.pathname.startsWith('/oauth-playground-api/'), 'BFF_BEARER_REJECTED');
    const headers = { Accept: 'application/json', Origin: this.origin, 'X-Request-Id': uuid7() };
    if (bearer) headers.Authorization = `Bearer ${bearer}`;
    if (bff && this.cookie) headers.Cookie = this.cookie;
    const encoded = body === undefined ? null : Buffer.from(JSON.stringify(body));
    if (encoded) { headers['Content-Type'] = 'application/json'; headers['Content-Length'] = encoded.length; }
    this.report.counts.requests += 1;
    if (method !== 'GET') this.report.counts.mutations += 1;
    return new Promise((resolve, reject) => {
      const request = http.request(url, { method, headers, agent: false,
        signal: AbortSignal.timeout(15000) }, response => {
        const chunks = []; let length = 0; let settled = false;
        const fail = code => { if (!settled) { settled = true; response.destroy(); reject(new Rejected(code)); } };
        response.on('data', chunk => {
          length += chunk.length;
          if (length > MAX_RESPONSE) fail('HTTP_RESPONSE_TOO_LARGE');
          else chunks.push(chunk);
        });
        response.on('aborted', () => fail('HTTP_RESPONSE_ABORTED'));
        response.on('error', () => fail('HTTP_RESPONSE_FAILED'));
        response.on('end', () => {
          if (settled) return;
          settled = true;
          const status = response.statusCode;
          this.report.counts.statuses[String(status)] = (this.report.counts.statuses[String(status)] || 0) + 1;
          let value = null;
          if (status < 300 || status >= 400) {
            if (!/^application\/json(?:\s*;|$)/iu.test(response.headers['content-type'] || '')) {
              reject(new Rejected('HTTP_JSON_REQUIRED')); return;
            }
            try { value = JSON.parse(Buffer.concat(chunks).toString('utf8')); }
            catch { reject(new Rejected('HTTP_JSON_INVALID')); return; }
          }
          if (bff && !/(?:^|,)\s*no-store(?:\s|,|$)/iu.test(response.headers['cache-control'] || '')) {
            reject(new Rejected('BFF_NO_STORE_REQUIRED')); return;
          }
          resolve({ status, value, location: response.headers.location, cookies: response.headers['set-cookie'] || [] });
        });
      });
      request.setTimeout(15000, () => request.destroy(new Rejected('HTTP_TIMEOUT')));
      request.on('error', error => reject(error instanceof Rejected ? error : new Rejected('HTTP_TRANSPORT_FAILED')));
      if (encoded) request.write(encoded);
      request.end();
    });
  }
  async ok(method, destination, options) {
    const response = await this.request(method, destination, options);
    requireCheck(response.status === 200, 'HTTP_SUCCESS_REQUIRED');
    return response;
  }
  async bff(action, body) {
    return (await this.ok('POST', `/oauth-playground-api/${action}`, { bff: true, body })).value;
  }
}

/** Injection is limited to the in-memory newly created session DTO adapter. */
export async function runValidation(configFile, { execute = false, credentialsExtractor = extractCredentials } = {}) {
  const report = { format: 'saas-uuid-u05-oauth-validation/1', status: 'FAIL', stage: 'INPUT_VALIDATION',
    checks: [], counts: { requests: 0, mutations: 0, statuses: {} } };
  let output;
  try {
    const config = await readPrivateJson(configFile);
    requireCheck(object(config) && ['publicOrigin', 'handoffFile', 'outputFile'].every(key => Object.hasOwn(config, key)) &&
      Object.keys(config).every(key => ['publicOrigin', 'handoffFile', 'outputFile', 'credentialsFile'].includes(key)),
    'CONFIG_FIELDS_INVALID');
    const publicOrigin = origin(config.publicOrigin);
    const handoff = await readPrivateJson(config.handoffFile);
    const application = object(handoff) && Array.isArray(handoff.applications)
      ? handoff.applications.filter(item => object(item) && item.code === 'oauth-playground') : [];
    const businessContext = application.length === 1 ? application[0].context : null;
    requireCheck(object(handoff) && object(handoff.login) && object(businessContext) &&
      typeof handoff.login.username === 'string' && handoff.login.username.length > 0 &&
      handoff.login.username.length <= 1024 && typeof handoff.login.password === 'string' &&
      handoff.login.password.length > 0 && handoff.login.password.length <= 1024 &&
      ['userId', 'tenantId', 'organizationId'].every(key => V7.test(businessContext[key])), 'HANDOFF_FIELDS_INVALID');
    if (config.credentialsFile !== undefined) await privateFile(config.credentialsFile);
    requireCheck(config.outputFile !== configFile && config.outputFile !== config.handoffFile &&
      config.outputFile !== config.credentialsFile, 'OUTPUT_INPUT_COLLISION');
    output = await reserveOutput(config.outputFile);
    const runner = new Runner(publicOrigin, report);
    runner.pass('private-inputs-and-new-output');
    if (!execute) {
      report.status = 'STATIC_ONLY'; report.stage = 'NETWORK_DISABLED';
      runner.pass('network-disabled-without-execute');
    } else {
      let sessionToken; let granted = false; let revoked = false; let primaryFailure; let failureStage;
      const expected = businessContext;
      try {
        await runner.step('INDEPENDENT_LOGIN', async () => {
          const login = (await runner.ok('POST', '/api/auth/login', {
            body: { username: handoff.login.username, password: handoff.login.password } })).value;
          requireCheck(object(login) && token(login.ticket) && Array.isArray(login.memberships) &&
            login.memberships.some(item => item.organizationId === expected.organizationId && item.userId === expected.userId),
          'LOGIN_TICKET_CONTEXT_REQUIRED');
          const selected = (await runner.ok('POST', '/api/auth/context', { bearer: login.ticket,
            body: { organizationId: expected.organizationId, userId: expected.userId } })).value;
          sessionToken = credentialsExtractor({ businessSession: selected }).accessToken;
          requireCheck(token(sessionToken), 'INDEPENDENT_SESSION_REQUIRED');
          const profile = (await runner.ok('GET', '/api/auth/profile', { bearer: sessionToken })).value;
          requireCheck(profile?.user?.id === expected.userId && profile?.tenant?.id === expected.tenantId &&
            profile?.organization?.id === expected.organizationId, 'BUSINESS_PROFILE_MISMATCH');
          runner.pass('independent-business-session');
        });
        await runner.step('BFF_CONFIGURATION', async () => {
          const configuration = await runner.ok('GET', '/oauth-playground-api/config', { bff: true });
          requireCheck(configuration.value?.publicOrigin === publicOrigin && configuration.value?.platformOrigin === publicOrigin &&
            configuration.value?.flows?.authorization_code?.enabled === true, 'BFF_CONFIGURATION_MISMATCH');
          requireCheck(configuration.cookies.length === 1, 'BFF_COOKIE_REQUIRED');
          const cookie = configuration.cookies[0];
          requireCheck(/^oauth_playground=[A-Za-z0-9_-]{43};/u.test(cookie) && /;\s*HttpOnly(?:;|$)/iu.test(cookie) &&
            /;\s*SameSite=Lax(?:;|$)/iu.test(cookie) && /;\s*Path=\/(?:;|$)/iu.test(cookie), 'BFF_COOKIE_ATTRIBUTES_INVALID');
          runner.cookie = cookie.split(';', 1)[0];
          runner.pass('bff-private-cookie-and-capability');
        });
        let state; let code;
        await runner.step('AUTHORIZATION_START', async () => {
          const started = await runner.bff('auth-start', {});
          requireCheck(object(started) && token(started.state) && started.state.length <= 128, 'BFF_STATE_REQUIRED');
          state = started.state;
          const authorization = sameOriginUrl(started.authorizeUrl, publicOrigin, '/oauth/authorize');
          requireCheck(oneParameter(authorization, 'state') === state && oneParameter(authorization, 'response_type') === 'code' &&
            oneParameter(authorization, 'code_challenge_method') === 'S256' &&
            /^[A-Za-z0-9_-]{43}$/u.test(oneParameter(authorization, 'code_challenge')) &&
            oneParameter(authorization, 'redirect_uri') === `${publicOrigin}/callback`, 'BFF_PKCE_REQUEST_INVALID');
          const response = await runner.request('GET', authorization);
          requireCheck(response.status === 302, 'AUTHORIZE_MANUAL_REDIRECT_REQUIRED');
          const consentUrl = sameOriginUrl(response.location, publicOrigin, '/consent');
          const requestId = oneParameter(consentUrl, 'request_id');
          requireCheck(V7.test(requestId), 'CONSENT_REQUEST_UUID7_REQUIRED');
          const consentPath = `/api/oauth/consent/${requestId}`;
          const consent = (await runner.ok('GET', consentPath, { bearer: sessionToken })).value;
          requireCheck(consent?.state === state && consent?.redirectUri === `${publicOrigin}/callback` &&
            consent?.client?.clientId === oneParameter(authorization, 'client_id') &&
            typeof consent.expiresAt === 'string' && Date.parse(consent.expiresAt) > Date.now(), 'CONSENT_CONTEXT_MISMATCH');
          const decision = (await runner.ok('POST', `${consentPath}/decision`,
            { bearer: sessionToken, body: { approve: true } })).value;
          const callback = sameOriginUrl(decision?.redirectTo, publicOrigin, '/callback');
          requireCheck(oneParameter(callback, 'state') === state && !callback.searchParams.has('error'), 'CALLBACK_STATE_MISMATCH');
          code = oneParameter(callback, 'code');
          requireCheck(token(code) && code.length <= 4096, 'AUTHORIZATION_CODE_REQUIRED');
          runner.pass('real-pkce-consent-and-v7-request');
        });
        let summary;
        await runner.step('BFF_CALLBACK', async () => {
          summary = await runner.bff('callback', { state, code });
          granted = true;
          requireCheck(tokenSummary(summary), 'BFF_GRANTED_SUMMARY_INVALID');
          runner.pass('bff-code-exchange-without-browser-token');
          const replay = await runner.request('POST', '/oauth-playground-api/callback', { bff: true, body: { state, code } });
          requireCheck(replay.status === 400 && replay.value?.error === 'invalid_state', 'CALLBACK_REPLAY_NOT_REJECTED');
          state = undefined; code = undefined;
          runner.pass('consumed-callback-state-rejected');
        });
        await runner.step('BFF_RESOURCES', async () => {
          const info = await runner.bff('userinfo', { flow: FLOW });
          requireCheck(noTokenMaterial(info) && info?.sub === expected.userId && info?.tenant?.id === expected.tenantId &&
            info?.organization?.id === expected.organizationId &&
            [info.sub, info.tenant.id, info.organization.id].every(value => V7.test(value)), 'USERINFO_UUID7_IDENTITY_MISMATCH');
          const introspection = await runner.bff('introspect', { flow: FLOW });
          requireCheck(noTokenMaterial(introspection) && introspection?.active === true &&
            introspection.sub === expected.userId && Number.isSafeInteger(introspection.exp) &&
            introspection.exp > Date.now() / 1000, 'INTROSPECTION_ACTIVE_IDENTITY_REQUIRED');
          runner.pass('userinfo-and-active-introspection');
          if (summary.hasRefreshToken) {
            requireCheck(tokenSummary(await runner.bff('refresh-token', { flow: FLOW })), 'BFF_REFRESH_SUMMARY_INVALID');
            const refreshed = await runner.bff('userinfo', { flow: FLOW });
            requireCheck(noTokenMaterial(refreshed) && refreshed?.sub === expected.userId &&
              refreshed?.tenant?.id === expected.tenantId && refreshed?.organization?.id === expected.organizationId,
            'REFRESHED_USERINFO_MISMATCH');
            runner.pass('bff-owned-refresh-and-resource');
          } else report.checks.push({ check: 'bff-owned-refresh-and-resource', status: 'NOT_AVAILABLE' });
        });
      } catch (error) {
        primaryFailure = error; failureStage = report.stage;
      } finally {
        const cleanup = async (stage, check, operation) => {
          try { await runner.step(stage, operation); }
          catch (error) {
            report.checks.push({ check, status: 'FAIL' });
            if (!primaryFailure) { primaryFailure = error; failureStage = stage; }
          }
        };
        if (granted) {
          await cleanup('BFF_REVOKE', 'bff-owned-token-cleanup', async () => {
            const result = await runner.bff('revoke', { flow: FLOW });
            requireCheck(result?.revoked === true && result.flow === FLOW, 'BFF_REVOKE_REQUIRED');
            revoked = true;
            const unavailable = await runner.request('POST', '/oauth-playground-api/userinfo', { bff: true, body: { flow: FLOW } });
            requireCheck(unavailable.status === 400 && unavailable.value?.error === 'no_token', 'BFF_REVOKED_LOCAL_TOKEN_RETAINED');
            runner.pass('bff-owned-token-revoked-and-removed');
          });
        }
        if (sessionToken) {
          await cleanup('INDEPENDENT_LOGOUT', 'independent-session-cleanup', async () => {
            const result = (await runner.ok('POST', '/api/auth/logout', { bearer: sessionToken })).value;
            requireCheck(result?.ok === true, 'INDEPENDENT_LOGOUT_REQUIRED');
            runner.pass('independent-session-logout');
          });
        }
        if (primaryFailure) report.stage = failureStage;
      }
      if (primaryFailure) throw primaryFailure;
      requireCheck(granted && revoked, 'OAUTH_COMPLETION_REQUIRED');
      report.status = 'PASS'; report.stage = 'COMPLETE';
    }
  } catch (error) {
    report.failure = error instanceof Rejected ? error.code : 'UNEXPECTED_OPERATION_FAILURE';
  } finally {
    if (output) {
      try {
        await output.writeFile(`${JSON.stringify(report, null, 2)}\n`, 'utf8');
        await output.sync();
      } finally { await output.close(); }
    }
  }
  return report;
}

async function main() {
  const args = process.argv.slice(2);
  if (args.length === 1 && args[0] === '--help') {
    process.stdout.write('Usage: node validate-uuid-u05-oauth.mjs --config <private-json> [--execute]\n');
    return;
  }
  requireCheck(args.length >= 2 && args[0] === '--config' && args[1] &&
    (args.length === 2 || (args.length === 3 && args[2] === '--execute')), 'CLI_ARGUMENTS_INVALID');
  const report = await runValidation(args[1], { execute: args[2] === '--execute' });
  process.stdout.write(`${JSON.stringify({ status: report.status, stage: report.stage,
    checks: report.checks.length, requests: report.counts.requests, mutations: report.counts.mutations,
    ...(report.failure ? { failure: report.failure } : {}) })}\n`);
  if (report.status === 'FAIL') process.exitCode = 1;
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(() => {
    process.stderr.write('OAuth acceptance rejected; no response or credential details are logged.\n');
    process.exitCode = 1;
  });
}
