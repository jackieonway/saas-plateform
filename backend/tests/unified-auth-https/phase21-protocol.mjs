/** Test driver using the delivered public client and real issuer HTTPS only.
 * Input/output pipes are private to the browser harness; bearer credentials
 * remain in this process. It has no database URL, SQL driver or private RPC.
 */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import { createInterface } from 'node:readline';
import { randomBytes } from 'node:crypto';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const task = process.argv[2];
assert(task === '/tmp/rust-saas-unified-auth-20261028' && fs.existsSync(path.join(task, 'OWNER')), 'OWNED_TASK_REQUIRED');
const source = path.join(task, 'phase21-rp-source');
const root = pathToFileURL(`${source}/`);
const execFileAsync = promisify(execFile);
const runtime = await import(new URL('src/oidc.mjs', root));
const { loadConfig, loadMachineConfig } = await import(new URL('src/config.mjs', root));
const { secureFetch } = await import(new URL('src/transport.mjs', root));
const require = createRequire(new URL('package.json', root));
const client = await import(pathToFileURL(require.resolve('openid-client')));
const publicConfig = JSON.parse(fs.readFileSync(path.join(task, 'phase21-public.json'), 'utf8'));
assert.deepEqual(Object.keys(publicConfig).sort(), ['machine', 'resource', 'rp']);
const machineEnvironment = {
  MACHINE_ISSUER: publicConfig.machine.issuer, MACHINE_CLIENT_ID: publicConfig.machine.clientId,
  MACHINE_CLIENT_SECRET_FILE: publicConfig.machine.secretFile, MACHINE_SCOPE: publicConfig.machine.scope,
  MACHINE_RESOURCE_URL: publicConfig.machine.resource,
};
const configurations = {
  third: loadConfig(publicConfig.rp.third), offline: loadConfig(publicConfig.rp.offline),
  machine: loadMachineConfig(machineEnvironment),
  resource: loadMachineConfig({ ...machineEnvironment, MACHINE_CLIENT_ID: publicConfig.resource.clientId,
    MACHINE_CLIENT_SECRET_FILE: publicConfig.resource.secretFile }),
};
const oidc = Object.fromEntries(await Promise.all(Object.entries(configurations).map(async ([name, config]) => [name, await runtime.discover(config)])));
const flows = new Map(), sessions = new Map();
const id = () => randomBytes(24).toString('base64url');
const expectedAudience = publicConfig.machine.resource;
const resource = http.createServer(async (request, response) => {
  const reply = (status, body) => { response.writeHead(status, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' }); response.end(JSON.stringify(body)); };
  if (request.method !== 'GET' || request.url !== '/demo-resource') return reply(404, { code: 'NOT_FOUND' });
  const token = /^Bearer ([\x21-\x7e]{1,16384})$/.exec(request.headers.authorization ?? '')?.[1];
  if (!token) return reply(401, { code: 'TOKEN_REQUIRED' });
  try {
    const result = await client.tokenIntrospection(oidc.resource.configuration, token, { resource: expectedAudience });
    const audiences = Array.isArray(result.aud) ? result.aud : [result.aud];
    if (!result.active || !audiences.includes(expectedAudience) || result.subject_mode !== 'MACHINE'
        || result.sub !== undefined || !(result.scope ?? '').split(' ').includes('demo.read')) return reply(401, { code: 'RESOURCE_PERMISSION_REQUIRED' });
    reply(200, { state: 'APPROVED_MACHINE_RESOURCE', subjectMode: 'MACHINE', scope: result.scope });
  } catch { reply(503, { code: 'AUTHORITY_UNAVAILABLE' }); }
});
resource.requestTimeout = 5000;
await new Promise((resolve, reject) => { resource.once('error', reject); resource.listen(23103, '127.0.0.1', resolve); });

async function execute(message) {
  switch (message.command) {
    case 'start': {
      assert(['third', 'offline'].includes(message.profile));
      const config = { ...configurations[message.profile], ...(message.scope ? { scope: message.scope } : {}) };
      const flow = await runtime.beginLogin(oidc[message.profile], config, message.prompt ?? 'consent');
      const key = id(); flows.set(key, { ...flow, profile: message.profile });
      return { key, authorizationUrl: flow.authorizationUrl };
    }
    case 'finish': {
      const flow = flows.get(message.key); assert(flow); flows.delete(message.key);
      const raw = await runtime.finishLogin(oidc[flow.profile], new URL(message.callback), flow);
      const token = runtime.tokenData(raw), profile = await runtime.userInfo(oidc[flow.profile], token);
      assert.equal(profile.sub, token.subject);
      const key = id(); sessions.set(key, { profile: flow.profile, token, raw, stale: [] });
      return { key, subject: token.subject, sid: token.sid, hasRefreshToken: Boolean(token.refreshToken), userInfoMatches: true };
    }
    case 'refresh': {
      const session = sessions.get(message.key); assert(session);
      const previous = session.token;
      const raw = await client.refreshTokenGrant(oidc[session.profile].configuration, previous.refreshToken);
      const next = runtime.tokenData(raw, previous);
      session.stale.push(previous.refreshToken); session.token = next; session.raw = raw;
      return { state: 'REFRESHED', rotated: next.refreshToken !== previous.refreshToken, sourcePreserved: next.subject === previous.subject && next.sid === previous.sid };
    }
    case 'replay': {
      const session = sessions.get(message.key); assert(session?.stale.length);
      await client.refreshTokenGrant(oidc[session.profile].configuration, session.stale[0]);
      return { state: 'UNEXPECTED_REPLAY_ACCEPTED' };
    }
    case 'userinfo': {
      const session = sessions.get(message.key); assert(session);
      const info = await runtime.userInfo(oidc[session.profile], session.token);
      return { matches: info.sub === session.token.subject };
    }
    case 'revoke': {
      const session = sessions.get(message.key); assert(session);
      await runtime.revoke(oidc[session.profile], session.token);
      return { state: 'REVOKED' };
    }
    case 'logout': {
      const session = sessions.get(message.key); assert(session);
      const token = session.token;
      const pending = { operationId: runtime.uuidV7(), scope: message.scope, token: token.refreshToken ?? token.accessToken,
        sid: token.sid, subject: token.subject };
      return runtime.logoutCommand(oidc[session.profile], configurations[session.profile], pending);
    }
    case 'machine': {
      const raw = await client.clientCredentialsGrant(oidc.machine.configuration, { scope: message.scope ?? configurations.machine.scope });
      assert(!raw.id_token && !raw.refresh_token);
      const key = id(); sessions.set(key, { profile: 'machine', raw, token: { accessToken: raw.access_token } });
      return { key, state: 'MACHINE_OBTAINED', scope: raw.scope, hasPersonIdentity: false };
    }
    case 'user-client-credentials': {
      await client.clientCredentialsGrant(oidc.third.configuration, { scope: 'openid' });
      return { state: 'UNEXPECTED_USER_MACHINE_GRANT' };
    }
    case 'machine-userinfo': {
      const session = sessions.get(message.key); assert(session?.profile === 'machine');
      await client.fetchUserInfo(oidc.machine.configuration, session.raw.access_token, 'no-person-subject');
      return { state: 'UNEXPECTED_MACHINE_USERINFO' };
    }
    case 'resource': {
      const session = sessions.get(message.key); assert(session);
      const target = message.business ? 'https://console.saas.test/api/external/menus' : expectedAudience;
      const token = message.idToken ? session.raw.id_token : session.token.accessToken;
      assert(typeof token === 'string');
      const response = await secureFetch({ origin: new URL(target).origin })(target, { headers: { Authorization: `Bearer ${token}` } });
      return { status: response.status };
    }
    case 'machine-cli': {
      const result = await execFileAsync(process.execPath, [path.join(source, 'machine.mjs')], {
        env: { ...machineEnvironment, NODE_EXTRA_CA_CERTS: process.env.NODE_EXTRA_CA_CERTS, PATH: path.dirname(process.execPath) + ':/usr/bin:/bin' },
        encoding: 'utf8', timeout: 15000, maxBuffer: 4096,
      });
      return JSON.parse(result.stdout);
    }
    default: throw new Error('UNKNOWN_COMMAND');
  }
}

console.log(JSON.stringify({ ready: true }));
let queue = Promise.resolve();
const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
input.on('line', line => {
  queue = queue.then(async () => {
    let message;
    try {
      message = JSON.parse(line); const result = await execute(message);
      process.stdout.write(JSON.stringify({ id: message.id, ok: true, result }) + '\n');
    } catch (error) {
      const code = typeof error?.code === 'string' && /^[A-Z_0-9]{1,80}$/.test(error.code) ? error.code : 'PROTOCOL_REQUEST_REJECTED';
      process.stdout.write(JSON.stringify({ id: message?.id, ok: false, code }) + '\n');
    }
  });
});
input.on('close', () => { queue.finally(() => { sessions.clear(); flows.clear(); resource.close(); resource.closeAllConnections(); }); });
