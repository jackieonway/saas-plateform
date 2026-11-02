/** Private bounded RPC harness over the frozen delivered RP and standard SDK.
 * No database/owner credential, session-state export, or raw-token output.
 * node phase23-protocol.mjs --control /owned/phase23-protocol-control.json
 * node --test phase23-protocol.mjs
 *
 * Control (exact 0600/current-uid JSON under TASK):
 * {contractVersion:'identity-phase23-protocol-control/1', sourcePath, caFile}.
 * sourcePath = TASK/source-snapshot-LABEL/saas-examples/oidc-rp; manifest is
 * TASK/snapshot-LABEL.json. node_modules must be an explicit symlink to the
 * already verified TASK/phase21-rp-source/node_modules with identical lockfile.
 * Public config is TASK/phase23-public.json, exact Phase21 rp/machine/resource
 * shape, issuer AUTH443, RP customer24446, native RP23102, demo resource23103.
 *
 * RPC: {id:positiveInteger,command,...} -> {id,ok,result} or fixed code/status.
 * begin(profile,scope?,prompt?) -> key+authorizationUrl (PRIVATE PIPE ONLY);
 * finish(key,callback) -> key+kid+hasRefreshToken+tokenIdentityPort. The optional
 * port is gateway response metadata for this exact successful code exchange;
 * it never identifies a process without the separate supervisor evidence.
 * Keys are opaque local handles.
 * reload replaces SDK configs/secrets atomically and never clears old carriers.
 * userinfo/resource/revoke accept version=current|original (default current).
 * same-subject(leftKey,rightKey) compares retained USER subjects; boolean only.
 * introspect is MACHINE-only, default original, using current resource creds.
 * refresh rotates current only; replay deliberately submits initial refresh.
 * old-credential(profile,operation,key?) uses initial secret and returns status.
 * verify-old-id-session separates cached-key signature math (ignores expiry),
 * current fresh-JWKS JWT validation, and real old opaque-token userinfo status.
 * It never claims key revocation automatically revokes an opaque grant.
 * carrier-window exposes retained access deadlines and SHA256 digests, so
 * the owner can bind exact database deadlines without exporting raw tokens.
 * Expired carriers cannot be counted as recovery revocation.
 * scan-task-logs compares retained
 * tokens/client secrets against the fixed owned logs and returns counts only.
 *
 * 64 retained flows + 64 sessions; stale refresh list <=8; no automatic retry.
 * One RPC at a time, line <=64KiB, execution <=20s, network <=10s/256KiB,
 * four transport slots, lifetime <=30m. Unknown refresh consumption retires the
 * current chain. Do not log this private pipe: begin URLs contain state/nonce.
 * Credentials/tokens/claims/paths are never returned. No database command or
 * caller-selected filesystem path RPC exists.
 */
import fs from 'node:fs';
import path from 'node:path';
import dns from 'node:dns';
import http from 'node:http';
import https from 'node:https';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import { AsyncLocalStorage } from 'node:async_hooks';
import { createHash, createPublicKey, randomBytes, verify } from 'node:crypto';
import { safeSdkCode, protocolFailureFact } from './phase23-protocol-diagnostics.mjs';

const TASK = '/tmp/rust-saas-unified-auth-20261028';
const AUTH = 'https://auth.saas.test', RP = 'https://customer.saas.test:24446';
const RESOURCE = `${RP}/demo-resource`;
const LIMIT = { flows: 64, sessions: 64, line: 65536, body: 262144,
  requestMs: 20000, networkMs: 10000, lifetimeMs: 1800000, sockets: 4, stale: 8 };
const context = new AsyncLocalStorage();
class Failure extends Error {
  constructor(code, status = 0) { super(code); this.code = code; this.status = status; }
}
const fail = (code = 'PROTOCOL_REQUEST_REJECTED', status = 0) => { throw new Failure(code, status); };
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
function alive() { if (context.getStore()?.signal.aborted) fail('REQUEST_TIMEOUT'); }
const opaque = (value, max = 16384) => typeof value === 'string' && /^[\x21-\x7e]+$/.test(value) && value.length <= max;
function shape(value, required, optional = []) {
  if (!object(value) || required.some(key => !Object.hasOwn(value, key))
      || Object.keys(value).some(key => !required.includes(key) && !optional.includes(key))) fail('INPUT_REJECTED');
}
function privateRead(filename, maximum = 65536) {
  if (process.platform !== 'linux' || typeof filename !== 'string' || !filename.startsWith(`${TASK}/`)
      || path.resolve(filename) !== filename || fs.realpathSync(filename) !== filename) fail('PRIVATE_INPUT_REJECTED');
  const descriptor = fs.openSync(filename, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
  try {
    const info = fs.fstatSync(descriptor);
    if (!info.isFile() || info.uid !== process.getuid() || (info.mode & 0o777) !== 0o600 || info.size < 1 || info.size > maximum) fail('PRIVATE_INPUT_REJECTED');
    const bytes = Buffer.alloc(info.size + 1); let count = 0;
    while (count < bytes.length) {
      const size = fs.readSync(descriptor, bytes, count, bytes.length - count, null);
      if (!size) break;
      count += size;
    }
    if (count !== info.size) fail('PRIVATE_INPUT_REJECTED');
    return bytes.subarray(0, count);
  } finally { fs.closeSync(descriptor); }
}
function secret(filename) {
  const value = privateRead(filename, 4098).toString('utf8').replace(/\r?\n$/, '');
  if (!opaque(value, 4096)) fail('PRIVATE_INPUT_REJECTED');
  return value;
}
function asciiJson(value) {
  return JSON.stringify(value).replace(/[\u007f-\uffff]/g, character => `\\u${character.charCodeAt(0).toString(16).padStart(4, '0')}`);
}
function sourceHash(inputs) {
  const order = (left, right) => {
    const a = Array.from(left, character => character.codePointAt(0)), b = Array.from(right, character => character.codePointAt(0));
    for (let index = 0; index < Math.min(a.length, b.length); index += 1) if (a[index] !== b[index]) return a[index] - b[index];
    return a.length - b.length;
  };
  return hash(`{${Object.keys(inputs).sort(order).map(key => `${asciiJson(key)}: ${asciiJson(inputs[key])}`).join(', ')}}`);
}
function publicSource(filename, root, expected) {
  if (fs.realpathSync(filename) !== filename || !filename.startsWith(`${root}/`)) fail('FROZEN_SOURCE_REJECTED');
  const info = fs.lstatSync(filename);
  if (!info.isFile() || info.size > 8 * 1024 * 1024 || hash(fs.readFileSync(filename)) !== expected) fail('FROZEN_SOURCE_REJECTED');
}
export function verifySource(sourcePath) {
  const match = typeof sourcePath === 'string' && new RegExp(`^${TASK}/source-snapshot-([A-Za-z0-9_-]+)/saas-examples/oidc-rp$`).exec(sourcePath);
  if (!match || fs.realpathSync(sourcePath) !== sourcePath) fail('FROZEN_SOURCE_REJECTED');
  const snapshot = `${TASK}/source-snapshot-${match[1]}`;
  const manifest = JSON.parse(privateRead(`${TASK}/snapshot-${match[1]}.json`, 8 * 1024 * 1024));
  if (!object(manifest.inputs) || !/^[0-9a-f]{64}$/.test(manifest.snapshotSha256)
      || sourceHash(manifest.inputs) !== manifest.snapshotSha256) fail('FROZEN_SOURCE_REJECTED');
  const prefix = 'saas-examples/oidc-rp/';
  const required = ['package.json', 'package-lock.json', 'server.mjs', 'src/config.mjs', 'src/oidc.mjs', 'src/transport.mjs', 'src/app.mjs'];
  const inputs = Object.entries(manifest.inputs).filter(([relative]) => relative.startsWith(prefix));
  if (!inputs.length || required.some(relative => !Object.hasOwn(manifest.inputs, prefix + relative))
      || !inputs.some(([relative]) => relative.startsWith(prefix + 'public/'))) fail('FROZEN_SOURCE_REJECTED');
  for (const [relative, expected] of inputs) {
    if (path.posix.normalize(relative) !== relative || relative.includes('\\') || relative.startsWith('/')
        || relative.split('/').includes('..') || !/^[0-9a-f]{64}$/.test(expected)) fail('FROZEN_SOURCE_REJECTED');
    publicSource(path.join(snapshot, relative), snapshot, expected);
  }
  const actual = [];
  function walk(directory) {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      if (entry.name === 'node_modules' && directory === sourcePath) continue;
      if (entry.isSymbolicLink()) fail('FROZEN_SOURCE_REJECTED');
      const filename = path.join(directory, entry.name);
      if (entry.isDirectory()) walk(filename);
      else if (entry.isFile()) actual.push(path.relative(sourcePath, filename).replaceAll('\\', '/'));
      else fail('FROZEN_SOURCE_REJECTED');
    }
  }
  walk(sourcePath);
  if (actual.length !== inputs.length || actual.some(relative => !Object.hasOwn(manifest.inputs, prefix + relative))) fail('FROZEN_SOURCE_REJECTED');
  const dependencyRoot = `${TASK}/phase21-rp-source/node_modules`;
  const mounted = path.join(sourcePath, 'node_modules');
  if (!fs.lstatSync(mounted).isSymbolicLink() || fs.realpathSync(mounted) !== fs.realpathSync(dependencyRoot)
      || fs.realpathSync(dependencyRoot) !== dependencyRoot
      || hash(fs.readFileSync(path.join(sourcePath, 'package-lock.json')))
        !== hash(fs.readFileSync(`${TASK}/phase21-rp-source/package-lock.json`))) fail('DEPENDENCIES_REJECTED');
  const require = createRequire(pathToFileURL(path.join(sourcePath, 'package.json')));
  const packages = {};
  for (const [name, version] of [['openid-client', '6.8.8'], ['jose', '6.2.13']]) {
    const entry = fs.realpathSync(require.resolve(name));
    const packageRoot = path.join(dependencyRoot, name);
    const metadata = JSON.parse(fs.readFileSync(path.join(packageRoot, 'package.json')));
    if (!entry.startsWith(`${packageRoot}/`) || metadata.name !== name || metadata.version !== version) fail('DEPENDENCIES_REJECTED');
    packages[name] = pathToFileURL(entry).href;
  }
  return { sourcePath, snapshotSha256: manifest.snapshotSha256, packages };
}
function safeLookup(hostname, options, callback) {
  if (!['auth.saas.test', 'customer.saas.test'].includes(hostname)) return callback(new Error('TARGET_REJECTED'));
  dns.lookup(hostname, { ...options, all: true }, (error, addresses) => {
    if (error || !addresses?.length || addresses.some(({ address }) => address !== '::1' && !/^127(?:\.[0-9]{1,3}){3}$/.test(address))) return callback(new Error('TARGET_REJECTED'));
    if (options.all) callback(null, addresses);
    else callback(null, addresses[0].address, addresses[0].family);
  });
}
function allowedUrl(value) {
  const url = new URL(value instanceof Request ? value.url : value);
  if (url.username || url.password || url.hash || url.search || url.protocol !== 'https:') fail('TARGET_REJECTED');
  const paths = ['/.well-known/openid-configuration', '/oauth/token', '/oauth/userinfo', '/oauth/introspect', '/oauth/revoke', '/oauth/jwks', '/oauth/logout'];
  if (!(url.origin === AUTH && (paths.includes(url.pathname) || /^\/oauth\/logout\/operations\/[0-9a-f-]{36}$/.test(url.pathname)))
      && !(url.origin === RP && url.pathname === '/demo-resource')) fail('TARGET_REJECTED');
  return url;
}
export function recordIdentityTokenResponse(url, method, status, headers, scope) {
  if (!Array.isArray(scope?.tokenIdentityPorts) || url.origin !== AUTH
      || url.pathname !== '/oauth/token' || url.search || method !== 'POST') return;
  scope.tokenResponseCount = Math.min(16, (scope.tokenResponseCount ?? 0) + 1);
  scope.lastTokenStatus = Number.isInteger(status) && status >= 0 && status <= 599 ? status : 0;
  if (status !== 200) return;
  const value = headers['x-task-identity-port'];
  scope.tokenIdentityPorts.push(value === '23441' || value === '23442' ? Number(value) : null);
}
export function taskLogName(name) {
  return ['gateway.log', 'postgres.log'].includes(name)
    || /^(?:service-|phase23-)[A-Za-z0-9_.-]*\.log$/.test(name);
}
export function requestBody(input, init = {}) {
  // Fetch represents a bodyless UserInfo GET with null. Native https.request
  // receives undefined for that same request and adds no entity framing.
  let body = init.body === null ? undefined : init.body;
  if (body instanceof URLSearchParams) body = body.toString();
  if (input instanceof Request && input.body && body === undefined) fail('INPUT_REJECTED');
  if (body !== undefined && !Buffer.isBuffer(body) && typeof body !== 'string') fail('INPUT_REJECTED');
  if (body && Buffer.byteLength(body) > LIMIT.line) fail('INPUT_REJECTED');
  return body;
}
function transport(ca) {
  const active = new Set();
  const agent = new https.Agent({ ca, rejectUnauthorized: true, keepAlive: true, maxSockets: 4,
    maxTotalSockets: 4, maxFreeSockets: 4, timeout: LIMIT.networkMs, lookup: safeLookup });
  let closed = false;
  const fetch = async (input, init = {}) => {
    if (closed) fail('DRIVER_CLOSED');
    if (active.size >= LIMIT.sockets) fail('TRANSPORT_CAPACITY');
    const url = allowedUrl(input), ctx = context.getStore();
    const method = init.method ?? (input instanceof Request ? input.method : 'GET');
    const signals = [ctx?.signal, init.signal, input instanceof Request ? input.signal : undefined].filter(Boolean);
    const signal = signals.length ? AbortSignal.any(signals) : undefined;
    const body = requestBody(input, init);
    const headers = new Headers(init.headers ?? (input instanceof Request ? input.headers : undefined));
    if (headers.has('cookie') || headers.has('transfer-encoding')) fail('INPUT_REJECTED');
    if (body !== undefined) headers.set('Content-Length', String(Buffer.byteLength(body)));
    return new Promise((resolve, reject) => {
      let request, settled = false;
      const done = (error, value) => {
        if (settled) return;
        settled = true; clearTimeout(timer); signal?.removeEventListener('abort', abort);
        if (request) active.delete(request);
        if (error) reject(new Failure('TRANSPORT_UNAVAILABLE'));
        else resolve(value);
      };
      const abort = () => { done(true); request?.destroy(); };
      const timer = setTimeout(abort, LIMIT.networkMs);
      if (signal?.aborted) return abort();
      signal?.addEventListener('abort', abort, { once: true });
      try {
        request = https.request(url, { method, headers: Object.fromEntries(headers), agent,
          maxHeaderSize: 8192 }, response => {
          let size = 0; const chunks = [];
          if (ctx) ctx.status = response.statusCode;
          if (response.headers['content-encoding']) return abort();
          response.on('data', chunk => {
            if (settled) return;
            size += chunk.length;
            if (size > LIMIT.body) abort();
            else chunks.push(chunk);
          });
          response.once('error', abort); response.once('aborted', abort);
          response.once('end', () => {
            if (!response.complete) return abort();
            try {
              recordIdentityTokenResponse(url, method, response.statusCode, response.headers, ctx);
              const responseHeaders = new Headers();
              for (const [key, value] of Object.entries(response.headers)) if (value !== undefined && key !== 'set-cookie') responseHeaders.set(key, Array.isArray(value) ? value.join(', ') : value);
              done(false, new Response([204, 205, 304].includes(response.statusCode) ? null : Buffer.concat(chunks), { status: response.statusCode, headers: responseHeaders }));
            } catch { abort(); }
          });
        });
        active.add(request); request.once('error', abort); request.end(body);
      } catch { abort(); }
    });
  };
  return { fetch, close() { closed = true; for (const request of active) request.destroy(); agent.destroy(); } };
}
function readPublic() {
  const value = JSON.parse(privateRead(`${TASK}/phase23-public.json`));
  shape(value, ['rp', 'machine', 'resource']); shape(value.rp, ['third', 'offline']);
  const allowed = ['OIDC_ISSUER', 'OIDC_CLIENT_ID', 'OIDC_CLIENT_SECRET_FILE', 'RP_ORIGIN', 'RP_BIND_HOST',
    'RP_BIND_PORT', 'NODE_EXTRA_CA_CERTS', 'OIDC_SCOPES', 'RP_SESSION_SECONDS', 'RP_STATE_SECONDS', 'RP_MAX_ENTRIES', 'RP_HTTP_TIMEOUT_SECONDS'];
  const configs = {};
  for (const profile of ['third', 'offline']) {
    const env = value.rp[profile];
    shape(env, ['OIDC_ISSUER', 'OIDC_CLIENT_ID', 'OIDC_CLIENT_SECRET_FILE', 'RP_ORIGIN', 'OIDC_SCOPES'], allowed);
    if (env.OIDC_ISSUER !== AUTH || env.RP_ORIGIN !== RP || (env.RP_BIND_HOST ?? '127.0.0.1') !== '127.0.0.1'
        || String(env.RP_BIND_PORT ?? '23102') !== '23102' || !opaque(env.OIDC_CLIENT_ID, 256)
        || typeof env.OIDC_SCOPES !== 'string' || !env.OIDC_SCOPES.split(' ').includes('openid')
        || env.OIDC_SCOPES.split(' ').length !== new Set(env.OIDC_SCOPES.split(' ')).size
        || env.OIDC_SCOPES.split(' ').some(scope => !['openid', 'profile', 'offline_access'].includes(scope))) fail('PUBLIC_CONFIG_REJECTED');
    configs[profile] = { issuer: AUTH, origin: RP, clientId: env.OIDC_CLIENT_ID,
      clientSecret: secret(env.OIDC_CLIENT_SECRET_FILE), redirectUri: `${RP}/auth/callback`,
      scope: env.OIDC_SCOPES, timeoutSeconds: 5 };
  }
  shape(value.machine, ['issuer', 'clientId', 'secretFile', 'scope', 'resource']);
  shape(value.resource, ['clientId', 'secretFile']);
  if (value.machine.issuer !== AUTH || value.machine.resource !== RESOURCE
      || !opaque(value.machine.clientId, 256) || !opaque(value.resource.clientId, 256)
      || typeof value.machine.scope !== 'string' || value.machine.scope.length > 256
      || !/^[A-Za-z0-9:._/-]+(?: [A-Za-z0-9:._/-]+)*$/.test(value.machine.scope)
      || value.machine.scope.split(' ').some(scope => ['openid', 'profile', 'offline_access'].includes(scope))
      || value.machine.scope.split(' ').length !== new Set(value.machine.scope.split(' ')).size) fail('PUBLIC_CONFIG_REJECTED');
  for (const profile of ['machine', 'resource']) configs[profile] = {
    issuer: AUTH, clientId: value[profile].clientId, clientSecret: secret(value[profile].secretFile),
    scope: value.machine.scope, resourceUrl: new URL(RESOURCE), timeoutSeconds: 5 };
  return configs;
}
function header(token) {
  if (!opaque(token) || token.split('.').length !== 3) fail('TOKEN_SHAPE_REJECTED');
  const value = JSON.parse(Buffer.from(token.split('.')[0], 'base64url').toString('utf8'));
  if (value.alg !== 'RS256' || typeof value.kid !== 'string' || !/^[A-Za-z0-9._-]{1,80}$/.test(value.kid)) fail('TOKEN_SHAPE_REJECTED');
  return value;
}
function jwks(value) {
  shape(value, ['keys']);
  if (!Array.isArray(value.keys) || value.keys.length > 64 || value.keys.some(key => !object(key) || key.kty !== 'RSA'
      || !/^[A-Za-z0-9._-]{1,80}$/.test(key.kid) || ['d', 'p', 'q', 'dp', 'dq', 'qi'].some(name => Object.hasOwn(key, name)))
      || new Set(value.keys.map(key => key.kid)).size !== value.keys.length) fail('JWKS_REJECTED');
  return value;
}
export function sdkJwksCacheView(client, configuration, nowMs = Date.now()) {
  if (typeof client.getJwksCache !== 'function') fail('JWKS_CACHE_UNAVAILABLE');
  const cache = client.getJwksCache(configuration), now = Math.floor(nowMs / 1000);
  if (!cache) return { observed: false, cachedKids: [], cacheUpdatedAtEpoch: null, cacheAgeSeconds: null,
    unknownKidRefetchAfterSeconds: 60, cacheMutationRequested: false };
  if (!Number.isInteger(cache.uat) || cache.uat < 1 || !Number.isInteger(now) || cache.uat > now) {
    fail('JWKS_CACHE_OBSERVATION_REJECTED');
  }
  const keys = jwks(cache.jwks).keys;
  // getJwksCache is only observed. Never call setJwksCache, alter uat/keys or
  // restart the SDK process to make a new signing kid appear in its cache.
  return { observed: true, cachedKids: keys.map(key => key.kid).sort(), cacheUpdatedAtEpoch: cache.uat,
    cacheAgeSeconds: now - cache.uat, unknownKidRefetchAfterSeconds: 60, cacheMutationRequested: false };
}
function mathematical(token, key) {
  try {
    header(token); const parts = token.split('.');
    return verify('RSA-SHA256', Buffer.from(parts.slice(0, 2).join('.')), key, Buffer.from(parts[2], 'base64url'));
  } catch { return false; }
}
function basic(config) {
  const encode = value => new URLSearchParams({ v: value }).toString().slice(2);
  return `Basic ${Buffer.from(`${encode(config.clientId)}:${encode(config.clientSecret)}`).toString('base64')}`;
}
function tokenVersion(session, version) {
  if (version !== undefined && !['current', 'original'].includes(version)) fail('INPUT_REJECTED');
  return version === 'original' ? { raw: session.originalRaw, token: session.originalToken } : session;
}
const commandFields = {
  begin: [['profile'], ['scope', 'prompt']], finish: [['key', 'callback'], []], reload: [[], []],
  userinfo: [['key'], ['version']], refresh: [['key'], []], replay: [['key'], []],
  revoke: [['key'], ['version']], logout: [['key', 'scope'], ['recover']], machine: [[], []],
  resource: [['key'], ['version', 'idToken']], introspect: [['key'], ['version']],
  'old-credential': [['profile', 'operation'], ['key']], 'verify-old-id-session': [['key'], []],
  'same-subject': [['leftKey', 'rightKey'], []],
  'carrier-window': [['key'], []],
  'scan-task-logs': [[], []],
  'jwks-cache': [['profile'], []],
};
export function createDriver({ runtime, client, jose, configurations, oidc, fetch, reload, readLogs = () => [],
  key = () => randomBytes(24).toString('base64url') }) {
  const flows = new Map(), sessions = new Map();
  const originalCredentials = Object.fromEntries(Object.entries(configurations).map(([profile, config]) => [profile, { ...config }]));
  const current = { configurations, oidc };
  const getSession = handle => { const session = sessions.get(handle); if (!session) fail('HANDLE_UNKNOWN'); return session; };
  const store = session => { alive(); if (sessions.size >= LIMIT.sessions) fail('SESSION_CAPACITY'); const handle = key(); sessions.set(handle, session); return handle; };
  async function freshKeys(profile) {
    const response = await fetch(current.oidc[profile].metadata.jwks_uri, { headers: { Accept: 'application/json', 'Cache-Control': 'no-cache' } });
    if (response.status !== 200) fail('JWKS_UNAVAILABLE', response.status);
    return jwks(await response.json());
  }
  async function execute(message) {
    const fields = commandFields[message?.command]; if (!fields) fail('COMMAND_REJECTED');
    shape(message, ['id', 'command', ...fields[0]], fields[1]);
    if (!Number.isSafeInteger(message.id) || message.id < 1 || message.id > 1000000000) fail('INPUT_REJECTED');
    for (const field of ['recover', 'idToken']) if (Object.hasOwn(message, field) && typeof message[field] !== 'boolean') fail('INPUT_REJECTED');
    switch (message.command) {
      case 'begin': {
        if (!['third', 'offline'].includes(message.profile)) fail('INPUT_REJECTED');
        if (flows.size >= LIMIT.flows) fail('FLOW_CAPACITY');
        const config = current.configurations[message.profile], scope = message.scope ?? config.scope;
        if (typeof scope !== 'string' || scope.split(' ').some(value => !config.scope.split(' ').includes(value))
            || !scope.split(' ').includes('openid') || !['consent', 'login', 'none', undefined].includes(message.prompt)) fail('INPUT_REJECTED');
        const flow = await runtime.beginLogin(current.oidc[message.profile], { ...config, scope }, message.prompt ?? 'consent');
        const url = new URL(flow.authorizationUrl);
        if (url.origin !== AUTH || url.pathname !== '/oauth/authorize' || url.hash || url.username || url.password) fail('TARGET_REJECTED');
        alive(); const handle = key(); flows.set(handle, { ...flow, profile: message.profile, scope, attempts: 0 });
        return { key: handle, authorizationUrl: flow.authorizationUrl };
      }
      case 'finish': {
        const flow = flows.get(message.key); if (!flow || ++flow.attempts > 4) fail('FLOW_UNKNOWN_OR_RETIRED');
        if (typeof message.callback !== 'string' || message.callback.length > LIMIT.line) fail('INPUT_REJECTED');
        const callback = new URL(message.callback);
        if (callback.origin !== RP || callback.pathname !== '/auth/callback' || callback.hash || callback.username || callback.password) fail('TARGET_REJECTED');
        if (sessions.size >= LIMIT.sessions) fail('SESSION_CAPACITY');
        const observed = context.getStore();
        if (observed) Object.assign(observed, { tokenIdentityPorts: [], tokenResponseCount: 0, lastTokenStatus: 0,
          callbackCodePresent: callback.searchParams.has('code'), callbackErrorPresent: callback.searchParams.has('error') });
        async function finishStep(stage, action) {
          if (observed) observed.finishStage = stage;
          try { return await action(); }
          catch (error) { if (observed) observed.finishSdkCode = safeSdkCode(error); throw error; }
        }
        const raw = await finishStep('finishLogin', () => runtime.finishLogin(current.oidc[flow.profile], callback, flow));
        const tokenIdentityPort = observed?.tokenIdentityPorts?.length === 1 ? observed.tokenIdentityPorts[0] : null;
        const token = await finishStep('tokenData', () => runtime.tokenData(raw, undefined, flow.scope));
        const info = await finishStep('userInfo', () => runtime.userInfo(current.oidc[flow.profile], token));
        if (info.sub !== token.subject) fail('SOURCE_MISMATCH');
        const kid = await finishStep('math', () => header(raw.id_token).kid);
        const keys = await finishStep('freshKeys', () => freshKeys(flow.profile));
        const publicKey = await finishStep('math', () => {
          const publicJwk = keys.keys.find(key => key.kid === kid); if (!publicJwk) fail('JWKS_REJECTED');
          const key = createPublicKey({ key: publicJwk, format: 'jwk' });
          if (!mathematical(raw.id_token, key)) fail('JWT_REJECTED');
          return key;
        });
        const handle = store({ profile: flow.profile, raw, token, originalRaw: raw, originalToken: token,
          originalNonce: flow.nonce, cachedKey: publicKey, kid, stale: [] });
        return { status: 200, key: handle, kid, hasRefreshToken: Boolean(token.refreshToken), tokenIdentityPort };
      }
      case 'reload': {
        const next = await reload();
        for (const profile of ['third', 'offline', 'machine', 'resource']) {
          for (const name of ['issuer', 'clientId', 'scope']) if (next.configurations[profile][name] !== current.configurations[profile][name]) fail('CONFIG_BINDING_CHANGED');
        }
        alive(); current.configurations = next.configurations; current.oidc = next.oidc;
        return { status: 200, reloaded: true, flows: flows.size, sessions: sessions.size };
      }
      case 'userinfo': {
        const session = getSession(message.key), selected = tokenVersion(session, message.version);
        const info = await runtime.userInfo(current.oidc[session.profile], selected.token);
        return { status: 200, matches: info.sub === selected.token.subject };
      }
      case 'same-subject': {
        const left = getSession(message.leftKey), right = getSession(message.rightKey);
        for (const session of [left, right]) {
          if (!['third', 'offline'].includes(session.profile)
              || typeof session.token.subject !== 'string' || session.token.subject.length === 0) fail('USER_SESSION_REQUIRED');
        }
        return { matches: left.token.subject === right.token.subject };
      }
      case 'jwks-cache': {
        if (!['third', 'offline'].includes(message.profile)) fail('INPUT_REJECTED');
        return sdkJwksCacheView(client, current.oidc[message.profile].configuration);
      }
      case 'carrier-window': {
        const session = getSession(message.key), now = Date.now();
        const original = session.originalToken.expiresAt, currentExpiry = session.token.expiresAt;
        if (!Number.isFinite(original) || !Number.isFinite(currentExpiry)) fail('CARRIER_DEADLINE_UNAVAILABLE');
        return { profile: session.profile, originalAccessExpiresAtMs: original,
          currentAccessExpiresAtMs: currentExpiry, nowMs: now,
          originalAccessSha256: hash(session.originalToken.accessToken),
          currentAccessSha256: hash(session.token.accessToken),
          originalAccessUnexpired: original > now, currentAccessUnexpired: currentExpiry > now };
      }
      case 'scan-task-logs': {
        const candidates = new Set();
        function collect(value, field = '') {
          if (Array.isArray(value)) { for (const item of value) collect(item, field); }
          else if (object(value)) { for (const [name, item] of Object.entries(value)) collect(item, name); }
          else if (typeof value === 'string' && value.length >= 16
              && /token|secret|nonce|state|verifier|authorizationUrl|stale/i.test(field)) candidates.add(value);
        }
        collect(originalCredentials); collect(current.configurations);
        for (const config of [...Object.values(originalCredentials), ...Object.values(current.configurations)]) {
          if (typeof config.clientId === 'string' && typeof config.clientSecret === 'string') {
            candidates.add(basic(config)); candidates.add(Buffer.from(config.clientId+':'+config.clientSecret).toString('base64'));
          }
        }
        for (const flow of flows.values()) collect(flow);
        for (const session of sessions.values()) { collect(session.raw); collect(session.originalRaw); collect(session.token);
          collect(session.originalToken); collect(session.stale, 'stale'); collect(session.logout); }
        const logs = await readLogs();
        if (!Array.isArray(logs) || logs.length > 32 || logs.some(value => !Buffer.isBuffer(value))
            || logs.reduce((total, value) => total + value.length, 0) > 64 * 1024 * 1024) fail('LOG_SCAN_BUDGET');
        return { logFiles: logs.length, candidatesCompared: candidates.size,
          candidateFindings: [...candidates].filter(value => logs.some(log => log.includes(Buffer.from(value)))).length };
      }
      case 'refresh': {
        const session = getSession(message.key); if (!session.token.refreshToken || session.uncertain) fail('REFRESH_RETIRED');
        let raw;
        try { raw = await client.refreshTokenGrant(current.oidc[session.profile].configuration, session.token.refreshToken); }
        catch (error) {
          const status = context.getStore()?.status ?? 0;
          if (status === 0 || status === 200 || status >= 500) session.uncertain = true;
          throw error;
        }
        let token;
        try { token = runtime.tokenData(raw, session.token); }
        catch (error) { session.uncertain = true; throw error; }
        alive(); session.stale.push(session.token.refreshToken); if (session.stale.length > LIMIT.stale) session.stale.shift();
        session.token = token; session.raw = raw;
        return { status: 200, rotated: true, sourcePreserved: true };
      }
      case 'replay': {
        const session = getSession(message.key), original = session.originalToken.refreshToken;
        if (!original) fail('REFRESH_UNAVAILABLE');
        await client.refreshTokenGrant(current.oidc[session.profile].configuration, original);
        return { status: 200, code: 'REPLAY_ACCEPTED' };
      }
      case 'revoke': {
        const session = getSession(message.key), selected = tokenVersion(session, message.version);
        await runtime.revoke(current.oidc[session.profile], selected.token);
        return { status: 200, code: 'REVOCATION_COMMAND_ACCEPTED' };
      }
      case 'logout': {
        const session = getSession(message.key);
        if (!['CURRENT_APP', 'GLOBAL'].includes(message.scope)) fail('INPUT_REJECTED');
        if (session.logout && session.logout.scope !== message.scope) fail('INPUT_REJECTED');
        session.logout ??= { operationId: runtime.uuidV7(), scope: message.scope,
          token: session.originalToken.refreshToken ?? session.originalToken.accessToken,
          sid: session.originalToken.sid, subject: session.originalToken.subject };
        await runtime.logoutCommand(current.oidc[session.profile], current.configurations[session.profile], session.logout, message.recover === true);
        return { status: 200, code: 'LOGOUT_COMPLETED' };
      }
      case 'machine': {
        if (sessions.size >= LIMIT.sessions) fail('SESSION_CAPACITY');
        const raw = await client.clientCredentialsGrant(current.oidc.machine.configuration, { scope: current.configurations.machine.scope });
        if (!opaque(raw.access_token) || raw.id_token || raw.refresh_token || raw.token_type?.toLowerCase() !== 'bearer'
            || !Number.isFinite(raw.expires_in) || raw.expires_in <= 0) fail('TOKEN_SHAPE_REJECTED');
        const token = { accessToken: raw.access_token, expiresAt: Date.now() + raw.expires_in * 1000 };
        return { status: 200, key: store({ profile: 'machine', raw, token, originalRaw: raw, originalToken: token }), hasRefreshToken: false };
      }
      case 'resource': {
        const session = getSession(message.key), selected = tokenVersion(session, message.version);
        const token = message.idToken === true ? selected.raw.id_token : selected.token.accessToken;
        if (!opaque(token)) fail('TOKEN_SHAPE_REJECTED');
        const response = await fetch(RESOURCE, { headers: { Authorization: `Bearer ${token}` } });
        return { status: response.status };
      }
      case 'introspect': {
        const session = getSession(message.key), selected = tokenVersion(session, message.version ?? 'original');
        if (session.profile !== 'machine') fail('MACHINE_SESSION_REQUIRED');
        const result = await client.tokenIntrospection(current.oidc.resource.configuration, selected.token.accessToken, { resource: RESOURCE });
        if (typeof result.active !== 'boolean') fail('TOKEN_SHAPE_REJECTED');
        return { status: 200, active: result.active };
      }
      case 'old-credential': {
        const config = originalCredentials[message.profile];
        if (!config || !['machine', 'introspect', 'refresh'].includes(message.operation)) fail('INPUT_REJECTED');
        if ((message.operation === 'machine' && message.profile !== 'machine')
            || (message.operation === 'introspect' && message.profile !== 'resource')
            || (message.operation === 'refresh' && !['third', 'offline'].includes(message.profile))) fail('INPUT_REJECTED');
        const form = new URLSearchParams();
        let target = '/oauth/token';
        if (message.operation === 'machine') { form.set('grant_type', 'client_credentials'); form.set('scope', config.scope); }
        else {
          const session = getSession(message.key);
          if ((message.operation === 'introspect' && session.profile !== 'machine')
              || (message.operation === 'refresh' && session.profile !== message.profile)) fail('INPUT_REJECTED');
          if (message.operation === 'introspect') { target = '/oauth/introspect'; form.set('token', session.originalToken.accessToken); form.set('resource', RESOURCE); }
          else { if (!session.originalToken.refreshToken) fail('REFRESH_UNAVAILABLE'); form.set('grant_type', 'refresh_token'); form.set('refresh_token', session.originalToken.refreshToken); }
        }
        const response = await fetch(AUTH + target, { method: 'POST', headers: { Authorization: basic(config), 'Content-Type': 'application/x-www-form-urlencoded' }, body: form });
        return { status: response.status };
      }
      case 'verify-old-id-session': {
        const session = getSession(message.key);
        if (!session.cachedKey || !session.originalRaw.id_token) fail('ID_TOKEN_UNAVAILABLE');
        const raw = session.originalRaw.id_token, keys = await freshKeys(session.profile);
        let currentAuthorityJwtValid = false, code = 'JWT_REJECTED';
        try {
          const { payload } = await jose.jwtVerify(raw, jose.createLocalJWKSet(keys), { issuer: AUTH,
            audience: current.configurations[session.profile].clientId, algorithms: ['RS256'], clockTolerance: 0,
            requiredClaims: ['iss', 'aud', 'sub', 'sid', 'iat', 'exp'] });
          if (payload.sub !== session.originalToken.subject || payload.sid !== session.originalToken.sid
              || payload.nonce !== session.originalNonce || !Number.isInteger(payload.iat) || payload.iat > Math.floor(Date.now() / 1000) + 5) fail('JWT_REJECTED');
          currentAuthorityJwtValid = true; code = 'JWT_VERIFIED';
        } catch (error) { if (error?.code === 'ERR_JWT_EXPIRED') code = 'JWT_EXPIRED'; else if (!keys.keys.some(key => key.kid === session.kid)) code = 'KEY_NOT_CURRENT'; }
        let onlineStatus = 0;
        try { await runtime.userInfo(current.oidc[session.profile], session.originalToken); onlineStatus = 200; }
        catch { onlineStatus = context.getStore()?.status >= 400 ? context.getStore().status : 0; }
        return { kid: session.kid, cachedKeySignatureValid: mathematical(raw, session.cachedKey),
          currentAuthorityJwtValid, currentVerificationCode: code, onlineStatus };
      }
    }
  }
  return { execute,
    async resourceStatus(token) {
      const value = await client.tokenIntrospection(current.oidc.resource.configuration, token, { resource: RESOURCE });
      const audiences = Array.isArray(value.aud) ? value.aud : [value.aud];
      return value.active === true && audiences.includes(RESOURCE) && value.subject_mode === 'MACHINE'
        && value.sub === undefined && typeof value.scope === 'string'
        && current.configurations.machine.scope.split(' ').every(scope => value.scope.split(' ').includes(scope)) ? 200 : 401;
    },
    counts: () => ({ flows: flows.size, sessions: sessions.size }), clear() { flows.clear(); sessions.clear(); } };
}
export async function dispatch(driver, message, timeoutMs = LIMIT.requestMs) {
  const id = Number.isSafeInteger(message?.id) && message.id >= 1 && message.id <= 1000000000 ? message.id : null;
  const controller = new AbortController(), ctx = { signal: controller.signal, status: 0 };
  let timer;
  try {
    const result = await context.run(ctx, () => Promise.race([driver.execute(message), new Promise((_resolve, reject) => {
      timer = setTimeout(() => { controller.abort(); reject(new Failure('REQUEST_TIMEOUT')); }, timeoutMs);
    })]));
    return { id, ok: true, result };
  } catch (error) {
    const failure = controller.signal.aborted ? { ok: false, status: 0, code: 'REQUEST_TIMEOUT' }
      : { ok: false, status: error instanceof Failure ? error.status : ctx.status >= 400 ? ctx.status : 0,
        code: error instanceof Failure ? error.code : 'SDK_REJECTED' };
    const fact = protocolFailureFact(message?.command, { ...failure, stage: ctx.finishStage, sdkCode: ctx.finishSdkCode,
      callbackCodePresent: ctx.callbackCodePresent, callbackErrorPresent: ctx.callbackErrorPresent,
      tokenResponseCount: ctx.tokenResponseCount, lastTokenStatus: ctx.lastTokenStatus });
    return { id, ...failure, ...(fact ? Object.fromEntries(Object.entries(fact).filter(([key]) => key !== 'command')) : {}) };
  } finally { clearTimeout(timer); controller.abort(); }
}
export async function* boundedLines(stream) {
  const buffer = Buffer.alloc(LIMIT.line); let size = 0;
  for await (const chunk of stream) {
    for (let start = 0; start < chunk.length;) {
      const newline = chunk.indexOf(10, start), end = newline < 0 ? chunk.length : newline;
      if (size + end - start > buffer.length) fail('LINE_LIMIT');
      chunk.copy(buffer, size, start, end); size += end - start; start = end + 1;
      if (newline >= 0) { yield new TextDecoder('utf8', { fatal: true }).decode(buffer.subarray(0, size)); size = 0; }
    }
  }
  if (size) fail('INCOMPLETE_LINE');
}
async function main() {
  let wire, resource, driver;
  const expires = Date.now() + LIMIT.lifetimeMs;
  let shutdown = false;
  let resourceRequests = 0;
  try {
    if (process.platform !== 'linux' || process.versions.node.split('.')[0] !== '24') fail('RUNTIME_REJECTED');
    const taskInfo = fs.lstatSync(TASK), marker = path.join(TASK, 'OWNER');
    if (!taskInfo.isDirectory() || taskInfo.isSymbolicLink() || taskInfo.uid !== process.getuid()
        || (taskInfo.mode & 0o777) !== 0o700 || fs.realpathSync(marker) !== marker
        || fs.statSync(marker).size > 256 || fs.readFileSync(marker, 'utf8').trim() !== path.basename(TASK) + '-pg55494') fail('TASK_REJECTED');
    const namespaceFile = path.join(TASK, 'namespace.identity'), namespace = fs.readFileSync(namespaceFile, 'utf8');
    if (fs.realpathSync(namespaceFile) !== namespaceFile || namespace.trim() !== fs.readlinkSync('/proc/self/ns/mnt')) fail('TASK_REJECTED');
    const args = process.argv.slice(2);
    if (args.length !== 2 || args[0] !== '--control') fail('CONTROL_REJECTED');
    const control = JSON.parse(privateRead(args[1])); shape(control, ['contractVersion', 'sourcePath', 'caFile']);
    if (control.contractVersion !== 'identity-phase23-protocol-control/1') fail('CONTROL_REJECTED');
    const frozen = verifySource(control.sourcePath), base = pathToFileURL(`${frozen.sourcePath}/`);
    const [runtime, client, jose] = await Promise.all([import(new URL('src/oidc.mjs', base)),
      import(frozen.packages['openid-client']), import(frozen.packages.jose)]);
    const ca = privateRead(control.caFile);
    if (!ca.includes(Buffer.from('-----BEGIN CERTIFICATE-----')) || ca.includes(Buffer.from('PRIVATE KEY'))) fail('CONTROL_REJECTED');
    wire = transport(ca);
    async function rebuild() {
      verifySource(control.sourcePath);
      const configurations = readPublic();
      const oidc = Object.fromEntries(await Promise.all(Object.entries(configurations).map(async ([name, config]) => [name, await runtime.discover(config, wire.fetch)])));
      for (const entry of Object.values(oidc)) if (entry.metadata.issuer !== AUTH || entry.metadata.introspection_endpoint !== `${AUTH}/oauth/introspect`) fail('DISCOVERY_REJECTED');
      return { configurations, oidc };
    }
    const initial = await rebuild();
    driver = createDriver({ runtime, client, jose, ...initial, fetch: wire.fetch, reload: rebuild,
      readLogs: () => fs.readdirSync(TASK).filter(taskLogName).map(name => {
        const filename = path.join(TASK, name);
        if (fs.realpathSync(filename) !== filename) fail('LOG_SCAN_INPUT');
        const descriptor = fs.openSync(filename, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW | fs.constants.O_NONBLOCK);
        try {
          const info = fs.fstatSync(descriptor);
          if (!info.isFile() || info.nlink !== 1 || info.uid !== process.getuid() || (info.mode & 0o777) !== 0o600
              || info.size > 32 * 1024 * 1024) fail('LOG_SCAN_INPUT');
          const bytes = Buffer.alloc(info.size); let count = 0;
          // Snapshot the existing prefix. The shared PG may append checkpoint
          // messages; truncation or an unreadable prefix remains a hard failure.
          while (count < bytes.length) { const read = fs.readSync(descriptor, bytes, count, bytes.length - count, null);
            if (!read) fail('LOG_SCAN_INPUT'); count += read; }
          return bytes;
        } finally { fs.closeSync(descriptor); }
      }) });
    resource = http.createServer({ maxHeaderSize: 8192, connectionsCheckingInterval: 250 }, (request, response) => {
      const reply = status => { if (!response.destroyed && !response.headersSent) { response.writeHead(status, { 'Content-Length': '0', 'Cache-Control': 'no-store' }); response.end(); } };
      response.on('error', () => {});
      const credentials = request.rawHeaders.filter((value, index) => index % 2 === 0 && value.toLowerCase() === 'authorization').length;
      if (request.method !== 'GET' || request.url !== '/demo-resource' || request.headers.cookie || credentials !== 1) return reply(404);
      const token = /^Bearer ([\x21-\x7e]{1,16384})$/.exec(request.headers.authorization ?? '')?.[1];
      if (!token) return reply(401);
      if (resourceRequests >= LIMIT.sockets) { reply(503); request.destroy(); return; }
      resourceRequests += 1;
      const timer = setTimeout(() => { request.destroy(); response.destroy(); }, LIMIT.requestMs);
      // Use the refreshed resource client credentials, never a session-local verdict.
      context.run({ signal: AbortSignal.timeout(LIMIT.requestMs), status: 0 }, async () => {
        try {
          reply(await driver.resourceStatus(token));
        } catch { reply(503); }
        finally { resourceRequests -= 1; clearTimeout(timer); }
      });
    });
    resource.maxConnections = 4; resource.requestTimeout = 20000; resource.headersTimeout = 5000; resource.keepAliveTimeout = 500;
    resource.maxRequestsPerSocket = 8;
    resource.setTimeout(LIMIT.requestMs, socket => socket.destroy());
    resource.on('clientError', (_error, socket) => socket.destroy());
    await new Promise((resolve, reject) => { resource.once('error', reject); resource.listen(23103, '127.0.0.1', resolve); });
    const stop = () => { shutdown = true; process.stdin.destroy(); process.stdout.destroy(); wire.close(); resource.close(); resource.closeAllConnections(); };
    process.once('SIGTERM', stop); process.once('SIGINT', stop);
    process.stdout.once('error', stop);
    const lifetime = setTimeout(stop, Math.max(1, expires - Date.now()));
    const lease = setInterval(() => {
      try {
        if (!fs.existsSync(path.join(TASK, 'namespace.pid')) || fs.readFileSync(namespaceFile, 'utf8') !== namespace) stop();
      } catch { stop(); }
    }, 250);
    async function output(value) {
      if (shutdown) fail('DRIVER_CLOSED');
      const line = JSON.stringify(value) + '\n'; if (Buffer.byteLength(line) > LIMIT.line) fail('OUTPUT_LIMIT');
      if (!process.stdout.write(line)) await new Promise((resolve, reject) => {
        const timer = setTimeout(() => { process.stdout.destroy(); reject(new Failure('OUTPUT_TIMEOUT')); }, 5000);
        process.stdout.once('drain', () => { clearTimeout(timer); resolve(); });
      });
    }
    try {
      await output({ ready: true });
      for await (const line of boundedLines(process.stdin)) {
        let message;
        try { message = JSON.parse(line); }
        catch { await output({ id: null, ok: false, status: 0, code: 'INPUT_REJECTED' }); continue; }
        await output(await dispatch(driver, message));
      }
    } finally {
      clearTimeout(lifetime); clearInterval(lease);
      process.removeListener('SIGTERM', stop); process.removeListener('SIGINT', stop); process.stdout.removeListener('error', stop);
    }
  } catch {
    if (!shutdown) { process.stderr.write('PHASE23_PROTOCOL_FAILED\n'); process.exitCode = 1; }
  } finally {
    wire?.close(); driver?.clear();
    if (resource) await new Promise(resolve => { resource.close(resolve); resource.closeAllConnections(); });
  }
}

if (process.env.NODE_TEST_CONTEXT) {
  const { test } = await import('node:test');
  const assert = (await import('node:assert/strict')).default;
  const { Readable } = await import('node:stream');
  const canary = 'PRIVATE_PROTOCOL_CANARY';
  function fixture(logs = []) {
    const calls = [], configs = Object.fromEntries(['third', 'offline', 'machine', 'resource'].map(profile => [profile,
      { issuer: AUTH, clientId: 'client-' + profile, clientSecret: canary, scope: profile === 'machine' || profile === 'resource' ? 'demo.read' : 'openid profile' }]));
    let generation = 0;
    const oidcs = generation => Object.fromEntries(Object.keys(configs).map(profile => [profile,
      { configuration: { profile, generation }, metadata: { jwks_uri: `${AUTH}/oauth/jwks` } }]));
    const client = { clientCredentialsGrant: async config => { calls.push(['machine', config.generation]); return { access_token: canary, token_type: 'Bearer', expires_in: 60 }; },
      tokenIntrospection: async (config, token) => { calls.push(['introspect', config.generation, token]); return { active: generation === 0 }; },
      refreshTokenGrant: async (_config, token) => { calls.push(['refresh', token]); throw new Error(canary); } };
    const runtime = { beginLogin: async (_oidc, _config, _prompt) => ({ authorizationUrl: `${AUTH}/oauth/authorize?state=${canary}`, nonce: canary, state: canary, verifier: canary }),
      userInfo: async () => { throw new Error(canary); } };
    let next = 0;
    const driver = createDriver({ runtime, client, jose: {}, configurations: configs, oidc: oidcs(0),
      key: () => `private-handle-${++next}`, readLogs: () => logs, fetch: async (_url, options) => { calls.push(['authorization', options?.headers?.Authorization]); return new Response(null, { status: 401 }); },
      reload: async () => { generation += 1; return { configurations: Object.fromEntries(Object.entries(configs).map(([name, config]) => [name, { ...config, clientSecret: 'ROTATED_PRIVATE_SECRET' }])), oidc: oidcs(generation) }; } });
    return { driver, calls };
  }
  test('reload retains the exact original machine token and old credentials without exporting them', async () => {
    const { driver, calls } = fixture();
    const machine = await dispatch(driver, { id: 1, command: 'machine' });
    assert.equal(machine.ok, true); assert.equal(JSON.stringify(machine).includes(canary), false);
    const key = machine.result.key;
    await dispatch(driver, { id: 2, command: 'reload' });
    const introspect = await dispatch(driver, { id: 3, command: 'introspect', key });
    assert.equal(introspect.result.active, false); assert.deepEqual(calls.at(-1), ['introspect', 1, canary]);
    const old = await dispatch(driver, { id: 4, command: 'old-credential', profile: 'resource', operation: 'introspect', key });
    assert.equal(old.result.status, 401); assert.equal(JSON.stringify(old).includes(canary), false);
    assert.equal(calls.at(-1)[1], basic({ clientId: 'client-resource', clientSecret: canary }));
    driver.clear();
  });
  test('carrier deadline observation preserves the original finite expiry across reload and never exports a token', async () => {
    const { driver } = fixture();
    const created = await dispatch(driver, { id: 1, command: 'machine' });
    const first = await dispatch(driver, { id: 2, command: 'carrier-window', key: created.result.key });
    assert(first.ok && first.result.originalAccessUnexpired && first.result.currentAccessUnexpired);
    assert(first.result.originalAccessExpiresAtMs > first.result.nowMs);
    assert.equal(first.result.originalAccessSha256, hash(canary));
    assert.equal(first.result.currentAccessSha256, first.result.originalAccessSha256);
    await dispatch(driver, { id: 3, command: 'reload' });
    const second = await dispatch(driver, { id: 4, command: 'carrier-window', key: created.result.key });
    assert.equal(second.result.originalAccessExpiresAtMs, first.result.originalAccessExpiresAtMs);
    assert.equal(second.result.originalAccessSha256, first.result.originalAccessSha256);
    assert.equal(second.result.currentAccessSha256, first.result.currentAccessSha256);
    assert.equal(JSON.stringify(second).includes(canary), false);
    driver.clear();
  });
  test('private retained token and client secret log scan returns counts without revealing the canary', async () => {
    const { driver } = fixture([Buffer.from('prefix '+canary+' suffix')]);
    await dispatch(driver, { id: 1, command: 'machine' });
    const result = await dispatch(driver, { id: 2, command: 'scan-task-logs' });
    assert(result.ok && result.result.logFiles === 1 && result.result.candidateFindings === 1);
    assert.equal(JSON.stringify(result).includes(canary), false);driver.clear();
    const clean = fixture([Buffer.from('fixed safe diagnostic')]);
    assert.equal((await dispatch(clean.driver, { id: 1, command: 'scan-task-logs' })).result.candidateFindings,0);
    clean.driver.clear();
    const encoded = fixture([Buffer.from('encoded '+Buffer.from('client-machine:'+canary).toString('base64'))]);
    const wire = await dispatch(encoded.driver, { id: 1, command: 'scan-task-logs' });
    assert(wire.result.candidateFindings > 0);assert.equal(JSON.stringify(wire).includes(canary),false);encoded.driver.clear();
  });
  test('all native identity instance logs participate in retained-token canary scanning', async () => {
    const files = { 'service-identity.log': Buffer.from('safe primary diagnostic'),
      'service-identity-peer.log': Buffer.from('peer leaked '+canary),
      'https-environment.json': Buffer.from('excluded private config '+canary) };
    const names = Object.keys(files).filter(taskLogName);
    assert.deepEqual(names, ['service-identity.log', 'service-identity-peer.log']);
    for (const name of ['../service-identity-peer.log', 'service-identity-peer.log/elsewhere', '.log']) {
      assert.equal(taskLogName(name), false);
    }
    const { driver } = fixture(names.map(name => files[name]));
    const result = await dispatch(driver, { id: 1, command: 'scan-task-logs' });
    assert.equal(result.result.logFiles, 2); assert.equal(result.result.candidateFindings, 1);
    assert.equal(JSON.stringify(result).includes(canary), false); driver.clear();
  });
  test('flows survive reload and both handle stores have hard capacity', async () => {
    const { driver } = fixture();
    for (let id = 1; id <= 64; id += 1) assert.equal((await dispatch(driver, { id, command: 'begin', profile: 'third' })).ok, true);
    assert.equal((await dispatch(driver, { id: 65, command: 'begin', profile: 'third' })).code, 'FLOW_CAPACITY');
    await dispatch(driver, { id: 66, command: 'reload' }); assert.equal(driver.counts().flows, 64);
    for (let id = 67; id < 131; id += 1) assert.equal((await dispatch(driver, { id, command: 'machine' })).ok, true);
    assert.equal((await dispatch(driver, { id: 131, command: 'machine' })).code, 'SESSION_CAPACITY');
    assert.deepEqual(driver.counts(), { flows: 64, sessions: 64 }); driver.clear();
  });
  test('unsafe ids, unknown fields and SDK exception canaries never enter a reply', async () => {
    const { driver } = fixture();
    const result = await dispatch(driver, { id: canary, command: 'machine', secret: canary });
    assert.equal(result.id, null); assert.equal(JSON.stringify(result).includes(canary), false);
    const machine = await dispatch(driver, { id: 2, command: 'machine' });
    const failure = await dispatch(driver, { id: 3, command: 'userinfo', key: machine.result.key });
    assert.equal(failure.code, 'SDK_REJECTED'); assert.equal(JSON.stringify(failure).includes(canary), false);
  });
  test('bounded line parser handles split lines and rejects excess or incomplete input', async () => {
    const read = async chunks => { const result = []; for await (const line of boundedLines(Readable.from(chunks))) result.push(line); return result; };
    assert.deepEqual(await read([Buffer.from('{"id":'), Buffer.from('1}\n{"id":2}\n')]), ['{"id":1}', '{"id":2}']);
    await assert.rejects(read([Buffer.alloc(LIMIT.line + 1, 97)]));
    await assert.rejects(read([Buffer.from('{}')]));
  });
  test('RPC deadline aborts the in-flight context and emits only a fixed failure', async () => {
    let aborted = false;
    const driver = { execute: () => new Promise((_resolve, reject) => {
      context.getStore().signal.addEventListener('abort', () => { aborted = true; reject(new Error(canary)); });
    }) };
    const result = await dispatch(driver, { id: 1 }, 5);
    assert.equal(aborted, true); assert.equal(result.ok, false); assert.equal(result.code, 'REQUEST_TIMEOUT');
    assert.equal(JSON.stringify(result).includes(canary), false);
  });
  test('only fixed public HTTPS endpoints and RS256 headers are accepted', () => {
    assert.equal(allowedUrl(`${AUTH}/oauth/introspect`).origin, AUTH);
    for (const value of ['https://outside.invalid/oauth/token', `${AUTH}/internal/identity/auth/clients`, `${RESOURCE}?token=${canary}`, 'http://auth.saas.test/oauth/token']) assert.throws(() => allowedUrl(value));
    assert.throws(() => header(Buffer.from('{"alg":"none","kid":"key"}').toString('base64url') + '.e.e'));
    assert.equal(mathematical('invalid', {}), false);
  });
  test('real delivered runtime and standard SDK send bodyless null UserInfo GET without weakening subject checks', async () => {
    const runtime = await import(new URL('../../../../saas-examples/oidc-rp/src/oidc.mjs', import.meta.url));
    const calls = [], subject = canary + '-subject';
    let returnedSubject = subject;
    const metadata = { issuer: AUTH, authorization_endpoint: `${AUTH}/oauth/authorize`, token_endpoint: `${AUTH}/oauth/token`,
      userinfo_endpoint: `${AUTH}/oauth/userinfo`, jwks_uri: `${AUTH}/oauth/jwks`,
      revocation_endpoint: `${AUTH}/oauth/revoke`, code_challenge_methods_supported: ['S256'],
      id_token_signing_alg_values_supported: ['RS256'] };
    const sdk = await runtime.discover({ issuer: AUTH, clientId: 'owned-pure-userinfo', clientSecret: canary, timeoutSeconds: 5 },
      async (input, init = {}) => {
        const url = allowedUrl(input), normalized = requestBody(input, init);
        calls.push({ path: url.pathname, method: init.method, suppliedNull: init.body === null,
          hasEntityBody: normalized !== undefined });
        if (url.pathname === '/.well-known/openid-configuration') {
          return new Response(JSON.stringify(metadata), { status: 200, headers: { 'Content-Type': 'application/json' } });
        }
        assert.equal(url.pathname, '/oauth/userinfo'); assert.equal(init.method, 'GET');
        assert.equal(init.body, null); assert.equal(normalized, undefined);
        assert.equal(new Headers(init.headers).get('authorization'), 'Bearer '+canary);
        return new Response(JSON.stringify({ sub: returnedSubject }), { status: 200,
          headers: { 'Content-Type': 'application/json' } });
      });
    assert.equal((await runtime.userInfo(sdk, { accessToken: canary, subject })).sub, subject);
    assert.deepEqual(calls.at(-1), { path: '/oauth/userinfo', method: 'GET', suppliedNull: true, hasEntityBody: false });
    returnedSubject = canary + '-different-subject';
    await assert.rejects(runtime.userInfo(sdk, { accessToken: canary, subject }),
      error => error.code === 'OAUTH_JSON_ATTRIBUTE_COMPARISON_FAILED');
    assert.equal(requestBody(`${AUTH}/oauth/token`, { body: new URLSearchParams({ grant_type: 'authorization_code' }) }),
      'grant_type=authorization_code');
    for (const body of [{ unsupported: true }, new Uint8Array([1]), 'a'.repeat(LIMIT.line+1)]) {
      assert.throws(() => requestBody(`${AUTH}/oauth/token`, { body }), { code: 'INPUT_REJECTED' });
    }
    assert.throws(() => requestBody(new Request(`${AUTH}/oauth/token`, { method: 'POST', body: canary })),
      { code: 'INPUT_REJECTED' });
  });
  test('SDK cache observation reads public age and kids without altering cache or exposing material', () => {
    const cache=Object.freeze({uat:1000,jwks:Object.freeze({keys:Object.freeze([
      Object.freeze({kty:'RSA',kid:'phase23-b',n:canary}),Object.freeze({kty:'RSA',kid:'owned-https',n:canary})])})});
    let reads=0;
    const client={getJwksCache:()=>{reads++;return cache;},setJwksCache:()=>assert.fail('CACHE_MUTATION_FORBIDDEN')};
    const value=sdkJwksCacheView(client,{},1061000);
    assert.equal(reads,1);assert.equal(value.cacheAgeSeconds,61);
    assert.deepEqual(value.cachedKids,['owned-https','phase23-b']);
    assert.equal(value.cacheUpdatedAtEpoch,1000);assert.equal(value.cacheMutationRequested,false);
    assert.equal(cache.uat,1000);assert.equal(cache.jwks.keys[0].kid,'phase23-b');
    assert.equal(JSON.stringify(value).includes(canary),false);
    assert.equal(sdkJwksCacheView({getJwksCache:()=>undefined},{},1061000).observed,false);
    assert.throws(()=>sdkJwksCacheView(client,{},999000),{code:'JWKS_CACHE_OBSERVATION_REJECTED'});
  });
  test('token issuer evidence is scoped to one exact successful code-exchange response', () => {
    const valid = new URL(`${AUTH}/oauth/token`), scope = { tokenIdentityPorts: [] };
    recordIdentityTokenResponse(valid, 'POST', 200, { 'x-task-identity-port': '23441' }, scope);
    recordIdentityTokenResponse(valid, 'POST', 200, { 'x-task-identity-port': '23442' }, scope);
    assert.deepEqual(scope.tokenIdentityPorts, [23441, 23442]);
    const before = [...scope.tokenIdentityPorts];
    for (const [url, method, status] of [[`${AUTH}/oauth/userinfo`, 'POST', 200],
      [`${RP}/oauth/token`, 'POST', 200], [`${AUTH}/oauth/token?unrelated=1`, 'POST', 200],
      [`${AUTH}/oauth/token`, 'GET', 200], [`${AUTH}/oauth/token`, 'POST', 400]]) {
      recordIdentityTokenResponse(new URL(url), method, status, { 'x-task-identity-port': '23442' }, scope);
    }
    assert.deepEqual(scope.tokenIdentityPorts, before);
    for (const value of [undefined, null, '5432', '23441, 23442', ['23442'], canary]) {
      const invalid = { tokenIdentityPorts: [] };
      recordIdentityTokenResponse(valid, 'POST', 200, { 'x-task-identity-port': value }, invalid);
      assert.deepEqual(invalid.tokenIdentityPorts, [null]);
      assert.equal(JSON.stringify(invalid).includes(canary), false);
    }
    recordIdentityTokenResponse(valid, 'POST', 200, { 'x-task-identity-port': '23442' }, {});
    recordIdentityTokenResponse(valid, 'POST', 200, { 'x-task-identity-port': '23442' }, undefined);
  });
  test('each real finish step reports only a fixed stage and allowlisted SDK code', async () => {
    const { generateKeyPairSync, sign } = await import('node:crypto');
    const pair = generateKeyPairSync('rsa', { modulusLength: 2048 }), kid = 'diagnostic-public-kid';
    const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url');
    const input = `${encode({ alg: 'RS256', kid })}.${encode({ sub: canary })}`;
    const idToken = `${input}.${sign('RSA-SHA256', Buffer.from(input), pair.privateKey).toString('base64url')}`;
    const publicJwk = { ...pair.publicKey.export({ format: 'jwk' }), kid, alg: 'RS256', use: 'sig' };
    for (const stage of ['finishLogin', 'tokenData', 'userInfo', 'freshKeys', 'math']) {
      let next = 0;
      const failure = () => { throw Object.assign(new Error(canary), {
        code: 'OAUTH_JWT_CLAIM_COMPARISON_FAILED', cause: { token: canary, callback: canary } }); };
      const configurations = { third: { issuer: AUTH, clientId: 'client-third', clientSecret: canary, scope: 'openid profile' } };
      const runtime = {
        beginLogin: async () => ({ authorizationUrl: `${AUTH}/oauth/authorize`, nonce: canary }),
        finishLogin: async () => {
          recordIdentityTokenResponse(new URL(`${AUTH}/oauth/token`), 'POST', 200,
            { 'x-task-identity-port': '23441' }, context.getStore());
          if (stage === 'finishLogin') failure();
          return { id_token: stage === 'math' ? idToken.slice(0, -8) + 'AAAAAAAA' : idToken };
        },
        tokenData: () => { if (stage === 'tokenData') failure(); return { subject: canary, sid: canary }; },
        userInfo: async () => { if (stage === 'userInfo') failure(); return { sub: canary }; },
      };
      const driver = createDriver({ runtime, configurations, client: {}, jose: {}, key: () => `diagnostic-${++next}`,
        oidc: { third: { metadata: { jwks_uri: `${AUTH}/oauth/jwks` } } },
        fetch: async () => { if (stage === 'freshKeys') failure();
          return new Response(JSON.stringify({ keys: [publicJwk] }), { status: 200 }); },
        reload: async () => ({}) });
      const begin = await dispatch(driver, { id: 1, command: 'begin', profile: 'third' });
      const result = await dispatch(driver, { id: 2, command: 'finish', key: begin.result.key,
        callback: `${RP}/auth/callback?code=${canary}` });
      assert.equal(result.ok, false); assert.equal(result.stage, stage);
      assert.equal(result.code, stage === 'math' ? 'JWT_REJECTED' : 'SDK_REJECTED');
      assert.equal(result.sdkCode, stage === 'math' ? undefined : 'OAUTH_JWT_CLAIM_COMPARISON_FAILED');
      assert.equal(result.callbackCodePresent, true); assert.equal(result.callbackErrorPresent, false);
      assert.equal(result.tokenResponseCount, 1); assert.equal(result.lastTokenStatus, 200);
      assert.equal(JSON.stringify(result).includes(canary), false); driver.clear();
    }
  });
  test('same old ID and opaque token distinguish cached math, fresh authority and post-recovery online denial', async () => {
    const { generateKeyPairSync, sign } = await import('node:crypto');
    const pair = generateKeyPairSync('rsa', { modulusLength: 2048 });
    const kid = 'public-old-kid', nonce = canary + '-nonce', subject = canary + '-subject', sid = canary + '-sid';
    const payload = { iss: AUTH, aud: 'client-third', sub: subject, sid, nonce,
      iat: Math.floor(Date.now() / 1000), exp: Math.floor(Date.now() / 1000) + 300 };
    const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url');
    const signed = value => { const input = `${encode({ alg: 'RS256', typ: 'JWT', kid })}.${encode(value)}`;
      return `${input}.${sign('RSA-SHA256', Buffer.from(input), pair.privateKey).toString('base64url')}`; };
    const idToken = signed(payload), raw = { id_token: idToken, access_token: canary + '-access', refresh_token: canary + '-refresh' };
    const oldJwk = { ...pair.publicKey.export({ format: 'jwk' }), kid, alg: 'RS256', use: 'sig' };
    const configurations = Object.fromEntries(['third', 'offline', 'machine', 'resource'].map(profile => [profile,
      { issuer: AUTH, clientId: 'client-' + profile, clientSecret: canary, scope: 'openid profile' }]));
    const builds = generation => Object.fromEntries(Object.keys(configurations).map(profile => [profile,
      { configuration: { generation }, metadata: { jwks_uri: `${AUTH}/oauth/jwks` } }]));
    let generation = 0, next = 0; const seenAccess = [], seenFlows = [];
    const runtime = {
      beginLogin: async () => ({ authorizationUrl: `${AUTH}/oauth/authorize?state=${canary}`, nonce, verifier: canary }),
      finishLogin: async (_sdk, _callback, flow) => { seenFlows.push(flow.verifier);
        if (generation >= 2) { context.getStore().status = 400; throw new Error(canary); }
        recordIdentityTokenResponse(new URL(`${AUTH}/oauth/token`), 'POST', 200,
          { 'x-task-identity-port': '23442' }, context.getStore()); return raw; },
      tokenData: () => ({ subject, sid, accessToken: raw.access_token, refreshToken: raw.refresh_token, scope: 'openid profile' }),
      userInfo: async (_sdk, token) => { seenAccess.push(token.accessToken);
        if (generation >= 2) { context.getStore().status = 401; throw new Error(canary); } return { sub: subject }; },
    };
    const jose = { createLocalJWKSet: keys => keys,
      jwtVerify: async (_raw, keys) => { if (!keys.keys.length) throw new Error(canary); return { payload }; } };
    const driver = createDriver({ runtime, jose, client: {}, configurations, oidc: builds(0), key: () => `handle-${++next}`,
      fetch: async () => new Response(JSON.stringify({ keys: generation === 0 ? [oldJwk] : [] }), { status: 200 }),
      reload: async () => { generation += 1; return { configurations, oidc: builds(generation) }; } });
    const begin = await dispatch(driver, { id: 1, command: 'begin', profile: 'third' });
    const finish = await dispatch(driver, { id: 2, command: 'finish', key: begin.result.key, callback: `${RP}/auth/callback?code=${canary}` });
    assert.equal(finish.ok, true); assert.equal(finish.result.kid, kid); assert.equal(JSON.stringify(finish).includes(canary), false);
    assert.equal(finish.result.tokenIdentityPort, 23442);
    const key = finish.result.key;
    const original = await dispatch(driver, { id: 3, command: 'verify-old-id-session', key });
    assert.equal(original.result.currentAuthorityJwtValid, true);
    await dispatch(driver, { id: 4, command: 'reload' });
    const revoked = await dispatch(driver, { id: 5, command: 'verify-old-id-session', key });
    assert.equal(revoked.result.cachedKeySignatureValid, true); assert.equal(revoked.result.currentAuthorityJwtValid, false);
    assert.equal(revoked.result.currentVerificationCode, 'KEY_NOT_CURRENT'); assert.equal(revoked.result.onlineStatus, 200);
    await dispatch(driver, { id: 6, command: 'reload' });
    const restored = await dispatch(driver, { id: 7, command: 'verify-old-id-session', key });
    assert.equal(restored.result.onlineStatus, 401); assert.equal(restored.result.cachedKeySignatureValid, true);
    assert.equal(JSON.stringify(restored).includes(canary), false);
    assert.ok(seenAccess.every(value => value === raw.access_token));
    const oldFlow = await dispatch(driver, { id: 8, command: 'finish', key: begin.result.key, callback: `${RP}/auth/callback?code=${canary}` });
    assert.equal(oldFlow.ok, false); assert.equal(oldFlow.status, 400); assert.deepEqual(seenFlows, [canary, canary]);
    assert.equal(mathematical(signed({ ...payload, exp: 1 }), pair.publicKey), true);
    driver.clear();
  });
  test('same-subject compares retained USER TokenData and returns no subject or sid', async () => {
    const { generateKeyPairSync, sign } = await import('node:crypto');
    const pair = generateKeyPairSync('rsa', { modulusLength: 2048 }), kid = 'public-comparison-kid';
    const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url');
    const input = `${encode({ alg: 'RS256', kid })}.${encode({ sub: canary })}`;
    const raw = { id_token: `${input}.${sign('RSA-SHA256', Buffer.from(input), pair.privateKey).toString('base64url')}` };
    const publicJwk = { ...pair.publicKey.export({ format: 'jwk' }), kid, alg: 'RS256', use: 'sig' };
    const subjects = [canary + '-same', canary + '-same', canary + '-different', undefined, ''];
    const configurations = Object.fromEntries(['third', 'offline', 'machine', 'resource'].map(profile => [profile,
      { issuer: AUTH, clientId: 'client-' + profile, clientSecret: canary, scope: 'openid profile' }]));
    const oidc = Object.fromEntries(Object.keys(configurations).map(profile => [profile,
      { configuration: {}, metadata: { jwks_uri: `${AUTH}/oauth/jwks` } }]));
    let next = 0, id = 0;
    const runtime = { beginLogin: async () => ({ authorizationUrl: `${AUTH}/oauth/authorize`, nonce: canary }),
      finishLogin: async () => raw, tokenData: () => ({ subject: subjects.shift(), sid: canary }),
      userInfo: async (_sdk, token) => ({ sub: token.subject }) };
    const driver = createDriver({ runtime, configurations, oidc, jose: {}, key: () => `compare-${++next}`,
      client: { clientCredentialsGrant: async () => ({ access_token: canary, token_type: 'Bearer', expires_in: 60 }) },
      fetch: async () => new Response(JSON.stringify({ keys: [publicJwk] }), { status: 200 }),
      reload: async () => ({ configurations, oidc }) });
    const login = async profile => {
      const begin = await dispatch(driver, { id: ++id, command: 'begin', profile });
      const finish = await dispatch(driver, { id: ++id, command: 'finish', key: begin.result.key, callback: `${RP}/auth/callback` });
      assert.equal(finish.ok, true); return finish.result.key;
    };
    const keys = [await login('third'), await login('offline'), await login('third'), await login('third'), await login('third')];
    const machine = await dispatch(driver, { id: ++id, command: 'machine' });
    await dispatch(driver, { id: ++id, command: 'reload' });
    const compare = (leftKey, rightKey) => dispatch(driver, { id: ++id, command: 'same-subject', leftKey, rightKey });
    const same = await compare(keys[0], keys[1]), different = await compare(keys[0], keys[2]);
    assert.deepEqual(same.result, { matches: true }); assert.deepEqual(different.result, { matches: false });
    const replies = [same, different];
    for (const invalid of [...keys.slice(3), machine.result.key]) {
      for (const [left, right] of [[invalid, keys[0]], [keys[0], invalid]]) {
        const result = await compare(left, right); assert.equal(result.code, 'USER_SESSION_REQUIRED'); replies.push(result);
      }
    }
    const unknown = await compare('unknown', keys[0]); assert.equal(unknown.code, 'HANDLE_UNKNOWN'); replies.push(unknown);
    assert.equal(JSON.stringify(replies).includes(canary), false);
    driver.clear();
  });
  test('snapshot aggregate hash matches Python JSON spacing and ASCII encoding', () => {
    assert.equal(sourceHash({ b: '2', a: '1' }), hash('{"a": "1", "b": "2"}'));
    assert.equal(sourceHash({ '中': '1' }), hash('{"\\u4e2d": "1"}'));
  });
} else if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await main();
}
