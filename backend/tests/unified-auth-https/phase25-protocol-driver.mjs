/** Linux standard-client driver; private stdin/stdout only, no database client. */
import fs from 'node:fs';
import path from 'node:path';
import dns from 'node:dns/promises';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { verifySource, boundedLines } from './phase23-protocol.mjs';
import { safeSdkCode } from './phase23-protocol-diagnostics.mjs';
import { createProtocolCases } from './phase25-protocol-cases.mjs';

const TASK = '/tmp/rust-saas-unified-auth-20261028', AUTH = 'https://auth.saas.test';
const RP = 'https://customer.saas.test:24446';
const need = (value, code) => { if (!value) throw new Error('PHASE25_PROTOCOL_' + code); };
function privateRead(filename, maximum = 8 * 1024 * 1024, uid = process.getuid(), mode = 0o600) {
  need(filename.startsWith(TASK + '/') && fs.realpathSync(filename) === filename, 'PRIVATE_PATH');
  const fd = fs.openSync(filename, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW | fs.constants.O_NONBLOCK);
  try {
    const before = fs.fstatSync(fd);
    need(before.isFile() && before.uid === uid && (before.mode & 0o777) === mode
      && before.nlink === 1 && before.size > 0 && before.size <= maximum, 'PRIVATE_OWNER');
    const bytes = fs.readFileSync(fd), after = fs.fstatSync(fd);
    need(bytes.length === before.size && before.ino === after.ino && before.dev === after.dev
      && before.mtimeMs === after.mtimeMs && before.ctimeMs === after.ctimeMs && before.size === after.size, 'PRIVATE_CHANGED');
    return bytes;
  } finally { fs.closeSync(fd); }
}
const json = filename => JSON.parse(privateRead(filename));
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
async function setup(label) {
  need(process.platform === 'linux' && process.versions.node.split('.')[0] === '24'
    && /^phase25-tools-r[1-9][0-9]*$/.test(label), 'RUNTIME_OR_LABEL');
  const info = fs.lstatSync(TASK);
  need(info.isDirectory() && !info.isSymbolicLink() && info.uid === process.getuid() && (info.mode & 0o777) === 0o700
    && privateRead(TASK + '/OWNER', 256).toString() === 'rust-saas-unified-auth-20261028-pg55494\n', 'TASK_OWNER');
  const binding = JSON.parse(privateRead(TASK + '/phase23-cluster-binding.json', 4096, 0, 0o644));
  need(binding.taskUid === info.uid && binding.taskInode === info.ino && binding.taskDevice === info.dev
    && binding.taskDir === TASK && binding.dataDirectory === TASK + '/data', 'TASK_BINDING');
  need(fs.readFileSync(TASK + '/namespace.identity', 'utf8').trim() === fs.readlinkSync('/proc/self/ns/mnt'), 'NAMESPACE');
  const snapshot = TASK + '/source-snapshot-' + label, manifest = json(TASK + '/snapshot-' + label + '.json');
  const here = snapshot + '/saas-plateform/backend/tests/unified-auth-https/';
  need(fileURLToPath(import.meta.url) === here + 'phase25-protocol-driver.mjs', 'FROZEN_DRIVER_PATH');
  for (const name of ['phase25-protocol-driver.mjs', 'phase25-protocol-cases.mjs', 'phase23-protocol.mjs', 'phase23-protocol-diagnostics.mjs']) {
    need(hash(privateRead(here + name)) === manifest.inputs['saas-plateform/backend/tests/unified-auth-https/' + name], 'FROZEN_DRIVER_BYTES');
  }
  const source = verifySource(snapshot + '/saas-examples/oidc-rp');
  const [runtime, client, jose, transport] = await Promise.all([
    import(pathToFileURL(source.sourcePath + '/src/oidc.mjs')), import(source.packages['openid-client']),
    import(source.packages.jose), import(pathToFileURL(source.sourcePath + '/src/transport.mjs')),
  ]);
  const ca = privateRead(TASK + '/phase23-ca.pem');
  need(ca.includes(Buffer.from('-----BEGIN CERTIFICATE-----')) && !ca.includes(Buffer.from('PRIVATE KEY')), 'CA');
  const addresses = await dns.lookup('auth.saas.test', { all: true });
  need(addresses.length > 0 && addresses.every(({ address }) => address === '::1' || /^127(?:\.[0-9]{1,3}){3}$/.test(address)), 'LOOPBACK_ISSUER');
  const outbound = transport.secureFetch({ origin: AUTH, ca, timeoutSeconds: 5 });
  const fetch = (input, init) => {
    const url = new URL(input instanceof Request ? input.url : input);
    need(url.origin === AUTH && !url.search && ['/.well-known/openid-configuration', '/oauth/token', '/oauth/userinfo', '/oauth/jwks'].includes(url.pathname), 'PUBLIC_ENDPOINT');
    return outbound(input, init);
  };
  const publicConfig = json(TASK + '/phase23-public.json');
  const configurations = {};
  for (const profile of ['third', 'offline']) {
    const value = publicConfig.rp[profile];
    need(value.OIDC_ISSUER === AUTH && value.RP_ORIGIN === RP && typeof value.OIDC_CLIENT_ID === 'string'
      && typeof value.OIDC_SCOPES === 'string' && value.OIDC_SCOPES.split(' ').every(scope => ['openid', 'profile', 'offline_access'].includes(scope)), 'PUBLIC_CONFIG');
    configurations[profile] = { issuer: AUTH, origin: RP, redirectUri: RP + '/auth/callback', clientId: value.OIDC_CLIENT_ID,
      clientSecret: privateRead(value.OIDC_CLIENT_SECRET_FILE, 4096).toString().trim(), scope: value.OIDC_SCOPES, timeoutSeconds: 5 };
  }
  need(publicConfig.machine.issuer === AUTH && typeof publicConfig.machine.clientId === 'string'
    && publicConfig.machine.scope === 'demo.read', 'MACHINE_CONFIG');
  configurations.machine = { issuer: AUTH, clientId: publicConfig.machine.clientId,
    clientSecret: privateRead(publicConfig.machine.secretFile, 4096).toString().trim(), scope: 'demo.read', timeoutSeconds: 5 };
  need(typeof publicConfig.resource.clientId === 'string', 'RESOURCE_CONFIG');
  configurations.resource = { ...configurations.machine, clientId: publicConfig.resource.clientId,
    clientSecret: privateRead(publicConfig.resource.secretFile, 4096).toString().trim() };
  const oidc = Object.fromEntries(await Promise.all(Object.entries(configurations).map(async ([name, config]) => [name, await runtime.discover(config, fetch)])));
  return { runtime, client, jose, configurations, oidc, fetch, sourceSha256: source.snapshotSha256 };
}

async function concurrentExchange(label, payload) {
  const children = [], pending = [], closed = new WeakSet();
  async function closedWithin(child, milliseconds) {
    if (closed.has(child)) return true;
    return new Promise(resolve => {
      const done = result => { clearTimeout(timer); child.removeListener('close', onClose); resolve(result); };
      const onClose = () => done(true), timer = setTimeout(() => done(false), milliseconds);
      child.once('close', onClose);
      if (closed.has(child)) done(true);
    });
  }
  try {
    for (let index = 0; index < 2; index++) {
      const child = spawn('/usr/bin/setpriv', ['--pdeathsig', 'TERM', process.execPath,
        fileURLToPath(import.meta.url), '--snapshot', label, '--worker'], {
        stdio: ['pipe', 'pipe', 'pipe'], env: { PATH: '/usr/bin:/bin' },
      });
      children.push(child);
      let ready, complete;
      const readiness = new Promise((resolve, reject) => { ready = { resolve, reject }; });
      const result = new Promise((resolve, reject) => { complete = { resolve, reject }; });
      // Both promises are observed immediately, including a startup failure.
      result.catch(() => {}); readiness.catch(() => {});
      const timer = setTimeout(() => { ready.reject(new Error('PHASE25_PROTOCOL_CHILD_TIMEOUT')); complete.reject(new Error('PHASE25_PROTOCOL_CHILD_TIMEOUT')); }, 20000);
      child.once('error', () => { ready.reject(new Error('PHASE25_PROTOCOL_CHILD_FAILED')); complete.reject(new Error('PHASE25_PROTOCOL_CHILD_FAILED')); });
      child.stderr.on('data', () => { ready.reject(new Error('PHASE25_PROTOCOL_CHILD_FAILED')); complete.reject(new Error('PHASE25_PROTOCOL_CHILD_FAILED')); });
      createInterface({ input: child.stdout }).on('line', line => {
        try {
          need(line.length <= 4096, 'CHILD_OUTPUT_LIMIT'); const value = JSON.parse(line);
          if (value.workerReady === true) ready.resolve();
          else { need(Number.isInteger(value.status) && [200, 400, 401].includes(value.status), 'CHILD_RESULT'); complete.resolve(value); }
        } catch { complete.reject(new Error('PHASE25_PROTOCOL_CHILD_FAILED')); }
      });
      child.once('close', () => {
        closed.add(child);
        clearTimeout(timer); ready.reject(new Error('PHASE25_PROTOCOL_CHILD_EARLY_EXIT'));
        complete.reject(new Error('PHASE25_PROTOCOL_CHILD_EARLY_EXIT'));
      });
      pending.push({ readiness, result });
    }
    await Promise.all(pending.map(value => value.readiness));
    need(children[0].pid > 1 && children[1].pid > 1 && children[0].pid !== children[1].pid, 'CHILD_PIDS');
    for (const child of children) child.stdin.end(JSON.stringify(payload) + '\n');
    const results = await Promise.all(pending.map(value => value.result));
    return { processesDistinct: true, statuses: results.map(value => value.status), errors: results.map(value => value.error ?? null) };
  } finally {
    let cleanupFailed = false;
    for (const child of children) {
      try {
        child.stdin.end();
        if (!await closedWithin(child, 1000)) {
          child.kill('SIGTERM');
          if (!await closedWithin(child, 2000)) {
            child.kill('SIGKILL');
            if (!await closedWithin(child, 2000)) cleanupFailed = true;
          }
        }
      } catch { cleanupFailed = true; }
    }
    need(!cleanupFailed, 'CHILD_CLEANUP_FAILED');
  }
}

async function main() {
  let driver; const args = process.argv.slice(2), worker = args.length === 3 && args[2] === '--worker';
  try {
    need(args[0] === '--snapshot' && (args.length === 2 || worker), 'ARGUMENTS');
    const ports = await setup(args[1]);
    if (worker) {
      process.stdout.write('{"workerReady":true}\n');
      for await (const line of boundedLines(process.stdin)) {
        const payload = JSON.parse(line), config = ports.configurations[payload.profile];
        need(['third', 'offline'].includes(payload.profile) && typeof payload.code === 'string'
          && /^[\x21-\x7e]{1,4096}$/.test(payload.code) && typeof payload.verifier === 'string'
          && /^[A-Za-z0-9._~-]{43,128}$/.test(payload.verifier) && payload.redirectUri === RP + '/auth/callback', 'CHILD_INPUT');
        const encode = value => new URLSearchParams({ v: value }).toString().slice(2);
        const response = await ports.fetch(AUTH + '/oauth/token', { method: 'POST',
          headers: { Authorization: 'Basic ' + Buffer.from(encode(config.clientId) + ':' + encode(config.clientSecret)).toString('base64'),
            'Content-Type': 'application/x-www-form-urlencoded' },
          body: new URLSearchParams({ grant_type: 'authorization_code', code: payload.code, redirect_uri: payload.redirectUri, code_verifier: payload.verifier }) });
        const value = await response.json();
        need(response.status !== 200 || typeof value.access_token === 'string', 'CHILD_TOKEN_SHAPE');
        const result = { status: response.status };
        if (['invalid_grant', 'invalid_client'].includes(value.error)) result.error = value.error;
        process.stdout.write(JSON.stringify(result) + '\n'); break;
      }
      return;
    }
    driver = createProtocolCases({ ...ports, concurrent: payload => concurrentExchange(args[1], payload) });
    process.stdout.write(JSON.stringify({ ready: true, sourceSha256: ports.sourceSha256, standardClient: 'openid-client@6.8.8', jwtLibrary: 'jose@6.2.13' }) + '\n');
    const lifetime = setTimeout(() => process.stdin.destroy(), 30 * 60 * 1000);
    const stop = () => process.stdin.destroy(); process.once('SIGTERM', stop); process.once('SIGINT', stop);
    try {
      for await (const line of boundedLines(process.stdin)) {
        let input;
        try {
          input = JSON.parse(line); need(Number.isSafeInteger(input.id) && input.id > 0, 'RPC_ID');
          const result = await driver.execute(input); process.stdout.write(JSON.stringify({ id: input.id, ok: true, result }) + '\n');
        } catch (error) {
          const fixed = /^PHASE25_PROTOCOL_[A-Z_]+$/.test(error?.message ?? '') ? error.message : 'PHASE25_PROTOCOL_OPERATION_FAILED';
          process.stdout.write(JSON.stringify({ id: Number.isSafeInteger(input?.id) ? input.id : null, ok: false, code: fixed, sdkCode: safeSdkCode(error) }) + '\n');
        }
      }
    } finally { clearTimeout(lifetime); process.removeListener('SIGTERM', stop); process.removeListener('SIGINT', stop); }
  } catch { process.stderr.write('PHASE25_PROTOCOL_DRIVER_FAILED\n'); process.exitCode = 1; }
  finally { driver?.clear(); }
}
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) await main();
