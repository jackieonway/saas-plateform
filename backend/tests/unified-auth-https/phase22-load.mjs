/** Bounded, issuer-only HTTPS capacity driver. No service or container is started.
 * Run pure tests: node --test phase22-load.mjs
 * Run fixture: node phase22-load.mjs --input /owned/input.json --output /report.json
 */
import fs from 'node:fs';
import path from 'node:path';
import https from 'node:https';
import dns from 'node:dns';
import { performance } from 'node:perf_hooks';
import { setTimeout as delay } from 'node:timers/promises';
import { pathToFileURL } from 'node:url';

const LIMITS = Object.freeze({ rate: 1000, concurrency: 32, duration: 1800, timeout: 10000,
  wireBytes: 65536, sourceBytes: 32 * 1024 * 1024, sourceEntries: 100000 });
const KINDS = ['CODE', 'REFRESH', 'INTROSPECTION'];
const fail = code => { throw new Error(code); };
const own = (value, key) => Object.hasOwn(value, key);
const record = value => value !== null && typeof value === 'object' && !Array.isArray(value);
function exact(value, required, optional = []) {
  if (!record(value) || required.some(key => !own(value, key))
      || Object.keys(value).some(key => !required.includes(key) && !optional.includes(key))) fail('INPUT_SHAPE_REJECTED');
}
function number(value, minimum, maximum, integer = true) {
  if (!Number.isFinite(value) || value < minimum || value > maximum || (integer && !Number.isInteger(value))) fail('BUDGET_REJECTED');
  return value;
}
function opaque(value, maximum = 16384) {
  return typeof value === 'string' && value.length > 0 && value.length <= maximum && /^[\x21-\x7e]+$/.test(value);
}
function privatePath(value) {
  if (typeof value !== 'string' || !path.isAbsolute(value) || value.includes('\0')) fail('PRIVATE_FILE_REQUIRED');
  return value;
}
export function validateInput(value) {
  exact(value, ['contractVersion', 'kind', 'origins', 'ratePerSecond', 'concurrency', 'durationSeconds',
    'requestTimeoutMs', 'clientFile', 'caFile', 'sourceFile']);
  if (value.contractVersion !== 'identity-capacity-input/1' || !KINDS.includes(value.kind)) fail('CONTRACT_REJECTED');
  if (!Array.isArray(value.origins) || value.origins.length < 1 || value.origins.length > 2) fail('ORIGIN_REJECTED');
  const origins = value.origins.map(origin => {
    const match = typeof origin === 'string' && /^https:\/\/auth\.saas\.test(?::([1-9][0-9]{0,4}))?$/.exec(origin);
    if (!match || Number(match[1] ?? 443) > 65535) fail('ORIGIN_REJECTED');
    return { hostname: 'auth.saas.test', port: Number(match[1] ?? 443) };
  });
  if (new Set(origins.map(origin => origin.port)).size !== origins.length) fail('ORIGIN_REJECTED');
  return { kind: value.kind, origins,
    ratePerSecond: number(value.ratePerSecond, 1, LIMITS.rate),
    concurrency: number(value.concurrency, 1, LIMITS.concurrency),
    durationSeconds: number(value.durationSeconds, 0.001, LIMITS.duration, false),
    requestTimeoutMs: number(value.requestTimeoutMs, 1, LIMITS.timeout),
    clientFile: privatePath(value.clientFile), caFile: privatePath(value.caFile), sourceFile: privatePath(value.sourceFile) };
}
function readPrivate(filename, maximum) {
  // WSL/Linux is intentional: Windows ACLs cannot be represented by mode 0600.
  if (process.platform !== 'linux' || !fs.constants.O_NOFOLLOW) fail('PRIVATE_FILE_PLATFORM_REQUIRED');
  const fd = fs.openSync(privatePath(filename), fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
  try {
    const stat = fs.fstatSync(fd);
    if (!stat.isFile() || stat.uid !== process.getuid() || (stat.mode & 0o777) !== 0o600
        || stat.size < 1 || stat.size > maximum) fail('PRIVATE_FILE_REJECTED');
    const bytes = Buffer.alloc(stat.size + 1);
    let read = 0;
    while (read < bytes.length) {
      const size = fs.readSync(fd, bytes, read, bytes.length - read, null);
      if (!size) break;
      read += size;
    }
    if (read !== stat.size) fail('PRIVATE_FILE_CHANGED');
    return bytes.subarray(0, read);
  } finally { fs.closeSync(fd); }
}
function parseJson(bytes) {
  try { return JSON.parse(bytes.toString('utf8')); } catch { fail('PRIVATE_JSON_REJECTED'); }
}
export function validateSources(config, client, source) {
  exact(client, ['clientId', 'clientSecret']);
  if (!opaque(client.clientId, 256) || (client.clientSecret !== null && !opaque(client.clientSecret, 4096))
      || (config.kind !== 'CODE' && client.clientSecret === null)) fail('CLIENT_SHAPE_REJECTED');
  const key = { CODE: 'codes', REFRESH: 'refreshTokens', INTROSPECTION: 'tokens' }[config.kind];
  exact(source, [key]);
  const entries = source[key];
  if (!Array.isArray(entries) || entries.length < 1 || entries.length > LIMITS.sourceEntries) fail('SOURCE_SHAPE_REJECTED');
  if (config.kind === 'CODE') {
    const seen = new Set();
    for (const entry of entries) {
      exact(entry, ['code', 'verifier', 'redirectUri']);
      if (!opaque(entry.code) || typeof entry.verifier !== 'string' || !/^[A-Za-z0-9._~-]{43,128}$/.test(entry.verifier)
          || typeof entry.redirectUri !== 'string' || entry.redirectUri.length > 4096) fail('SOURCE_SHAPE_REJECTED');
      let redirect;
      try { redirect = new URL(entry.redirectUri); } catch { fail('SOURCE_SHAPE_REJECTED'); }
      if (redirect.protocol !== 'https:' || redirect.username || redirect.password || redirect.hash
          || seen.has(entry.code)) fail('SOURCE_SHAPE_REJECTED');
      seen.add(entry.code);
    }
  } else {
    if (entries.some(entry => !opaque(entry)) || new Set(entries).size !== entries.length
        || (config.kind === 'REFRESH' && entries.length !== config.concurrency)) fail('SOURCE_SHAPE_REJECTED');
  }
  // Validate every possible initial request before a socket can be opened.
  for (const entry of entries) makeRequest(config.kind, client, entry);
  return { client, entries };
}
function makeRequest(kind, client, entry) {
  const form = new URLSearchParams();
  if (kind === 'CODE') {
    form.set('grant_type', 'authorization_code'); form.set('code', entry.code);
    form.set('code_verifier', entry.verifier); form.set('redirect_uri', entry.redirectUri);
  } else if (kind === 'REFRESH') {
    form.set('grant_type', 'refresh_token'); form.set('refresh_token', entry);
  } else { form.set('token', entry); }
  const headers = { 'Content-Type': 'application/x-www-form-urlencoded', Accept: 'application/json' };
  if (client.clientSecret === null) form.set('client_id', client.clientId);
  else {
    const encoded = new URLSearchParams({ id: client.clientId, secret: client.clientSecret });
    const id = encoded.toString().split('&')[0].slice(3), secret = encoded.toString().split('&')[1].slice(7);
    headers.Authorization = `Basic ${Buffer.from(`${id}:${secret}`).toString('base64')}`;
  }
  const body = Buffer.from(form.toString());
  headers['Content-Length'] = String(body.length);
  const headerBytes = Object.entries(headers).reduce((sum, [key, value]) => sum + Buffer.byteLength(`${key}: ${value}\r\n`), 0);
  if (body.length + headerBytes + 512 > LIMITS.wireBytes) fail('REQUEST_LIMIT_REJECTED');
  return { path: kind === 'INTROSPECTION' ? '/oauth/introspect' : '/oauth/token', headers, body };
}
function restrictedLookup(hostname, options, callback, lookup = dns.lookup) {
  if (hostname !== 'auth.saas.test') return callback(Object.assign(new Error('TARGET_REJECTED'), { code: 'TARGET_REJECTED' }));
  lookup(hostname, { ...options, all: true }, (error, addresses) => {
    if (error) return callback(error);
    if (!addresses?.length || addresses.some(({ address }) => address !== '::1' && !/^127(?:\.[0-9]{1,3}){3}$/.test(address))) {
      return callback(Object.assign(new Error('TARGET_REJECTED'), { code: 'TARGET_REJECTED' }));
    }
    if (options.all) callback(null, addresses);
    else callback(null, addresses[0].address, addresses[0].family);
  });
}
export function createTransport(config, ca, implementation = https) {
  const agent = new implementation.Agent({ keepAlive: true, maxSockets: config.concurrency,
    maxTotalSockets: config.concurrency, maxFreeSockets: config.concurrency,
    timeout: config.requestTimeoutMs, maxCachedSessions: config.concurrency,
    ca, rejectUnauthorized: true, lookup: restrictedLookup });
  // No proxy/fetch/global dispatcher, redirects or retries. Node's default
  // checkServerIdentity validates the real issuer hostname against this CA.
  const send = (target, material, signal) => new Promise(resolve => {
    let settled = false, request, timer;
    const done = result => {
      if (settled) return;
      settled = true; clearTimeout(timer); signal?.removeEventListener('abort', abort); resolve(result);
    };
    const stop = code => { done({ transport: code }); request?.destroy(); };
    const abort = () => stop('cancelled');
    timer = setTimeout(() => stop('timeout'), config.requestTimeoutMs);
    if (signal?.aborted) return stop('cancelled');
    signal?.addEventListener('abort', abort, { once: true });
    try {
      request = implementation.request({ hostname: target.hostname, port: target.port, servername: target.hostname,
        method: 'POST', path: material.path, headers: material.headers, agent,
        maxHeaderSize: 8192, insecureHTTPParser: false }, response => {
        let size = response.rawHeaders.reduce((sum, value) => sum + Buffer.byteLength(value) + 4, 256);
        const chunks = [];
        if (size > LIMITS.wireBytes || (response.headers['content-length'] !== undefined
            && (!/^[0-9]+$/.test(response.headers['content-length'])
              || Number(response.headers['content-length']) + size > LIMITS.wireBytes))) return stop('response_limit');
        response.on('data', chunk => {
          if (settled) return;
          size += chunk.length;
          if (size > LIMITS.wireBytes) stop('response_limit');
          else chunks.push(chunk);
        });
        response.once('aborted', () => stop('network'));
        response.once('error', () => stop('network'));
        response.once('end', () => {
          if (!response.complete) return stop('network');
          let body = null;
          const json = /^application\/json(?:\s*;|$)/i.test(response.headers['content-type'] ?? '');
          if (json && !response.headers['content-encoding']) {
            try { body = JSON.parse(Buffer.concat(chunks).toString('utf8')); } catch { /* classified invalid */ }
          }
          done({ status: response.statusCode, body });
        });
      });
      request.once('error', () => stop('network'));
      request.end(material.body);
    } catch { stop('network'); }
  });
  return { send, close: () => agent.destroy() };
}
export function classify(kind, result, previous) {
  const consumable = kind !== 'INTROSPECTION';
  if (result.transport) return { category: 'network', retire: consumable, uncertain: consumable };
  if (result.status === 429) return { category: 'rateLimited', retire: false, uncertain: false };
  if (result.status === 503) return { category: 'unavailable', retire: consumable, uncertain: consumable };
  if (result.status !== 200) return { category: 'invalid', retire: kind === 'REFRESH'
    || (consumable && result.status >= 500), uncertain: consumable && result.status >= 500 };
  const body = result.body;
  if (kind === 'INTROSPECTION') {
    if (record(body) && body.active === true) return { category: 'effectiveSuccess', retire: false, uncertain: false };
    if (record(body) && body.active === false) return { category: 'denied', retire: false, uncertain: false };
  } else if (record(body) && opaque(body.access_token) && typeof body.token_type === 'string'
      && body.token_type.toLowerCase() === 'bearer' && Number.isFinite(body.expires_in) && body.expires_in > 0
      && (!own(body, 'refresh_token') || opaque(body.refresh_token))
      && (!own(body, 'id_token') || opaque(body.id_token))
      && (kind !== 'REFRESH' || (opaque(body.refresh_token) && body.refresh_token !== previous))) {
    return { category: 'effectiveSuccess', retire: false, uncertain: false, next: body.refresh_token };
  }
  return { category: 'invalid', retire: kind === 'REFRESH', uncertain: kind === 'REFRESH' };
}
function percentiles(samples, length) {
  if (!length) return { p50: null, p95: null, p99: null };
  const ordered = samples.subarray(0, length).sort();
  const value = fraction => Math.round(ordered[Math.max(0, Math.ceil(length * fraction) - 1)] * 1000) / 1000;
  return { p50: value(0.50), p95: value(0.95), p99: value(0.99) };
}
export async function runSchedule(config, material, transport, { now = () => performance.now(),
  sleep = (ms, signal) => delay(ms, undefined, { signal }), signal } = {}) {
  const maximumAttempts = Math.ceil(config.ratePerSecond * config.durationSeconds);
  const all = new Float64Array(maximumAttempts), success = new Float64Array(maximumAttempts);
  const workers = Array.from({ length: config.concurrency }, (_, index) => ({ busy: false, retired: false,
    token: config.kind === 'REFRESH' ? material.entries[index] : undefined }));
  const counts = { attempts: 0, effectiveSuccess: 0, denied: 0, invalid: 0, rateLimited: 0,
    unavailable: 0, network: 0, uncertainRetired: 0, workersRetired: 0 };
  const pending = new Set(), interval = 1000 / config.ratePerSecond;
  const began = now(), cpu = process.cpuUsage(), memoryStart = process.memoryUsage();
  const memoryPeak = { rss: memoryStart.rss, heapUsed: memoryStart.heapUsed, external: memoryStart.external };
  let slot = 0, nextWorker = 0, nextSource = 0, inflightPeak = 0, skippedBusy = 0, skippedLate = 0;
  let reason = 'duration_completed', schedulingEnded = began;
  function sampleMemory() {
    const sample = process.memoryUsage();
    for (const key of Object.keys(memoryPeak)) memoryPeak[key] = Math.max(memoryPeak[key], sample[key]);
  }
  const memoryTimer = setInterval(sampleMemory, 250);
  try {
    while (slot < maximumAttempts) {
      if (signal?.aborted) { reason = 'cancelled'; break; }
      const target = began + slot * interval, remaining = target - now();
      if (remaining > 0) {
        try { await sleep(remaining, signal); } catch { reason = 'cancelled'; break; }
        continue;
      }
      const current = now();
      if (current >= began + config.durationSeconds * 1000) break;
      const due = Math.floor((current - began) / interval);
      if (due > slot) { skippedLate += due - slot; slot = due; }
      if (slot >= maximumAttempts) break;
      slot += 1;
      if (pending.size >= config.concurrency) { skippedBusy += 1; continue; }
      let index = -1;
      for (let offset = 0; offset < workers.length; offset += 1) {
        const candidate = (nextWorker + offset) % workers.length;
        if (!workers[candidate].busy && !workers[candidate].retired) { index = candidate; break; }
      }
      if (index < 0) {
        if (workers.every(worker => worker.retired)) { reason = 'workers_retired'; break; }
        skippedBusy += 1; continue;
      }
      const worker = workers[index]; nextWorker = (index + 1) % workers.length;
      let entry;
      if (config.kind === 'CODE') {
        if (nextSource === material.entries.length) { reason = 'source_exhausted'; break; }
        entry = material.entries[nextSource++];
      } else if (config.kind === 'REFRESH') entry = worker.token;
      else entry = material.entries[nextSource++ % material.entries.length];
      const attemptIndex = counts.attempts++, started = now();
      worker.busy = true;
      const targetOrigin = config.origins[attemptIndex % config.origins.length];
      const operation = (async () => {
        let result;
        try { result = await transport.send(targetOrigin, makeRequest(config.kind, material.client, entry), signal); }
        catch { result = { transport: 'network' }; }
        const elapsed = Math.max(0, now() - started), outcome = classify(config.kind, result, entry);
        all[attemptIndex] = elapsed;
        if (outcome.category === 'effectiveSuccess') success[counts.effectiveSuccess] = elapsed;
        counts[outcome.category] += 1;
        if (outcome.retire) { worker.retired = true; counts.workersRetired += 1; }
        if (outcome.uncertain) counts.uncertainRetired += 1;
        if (config.kind === 'REFRESH' && outcome.category === 'effectiveSuccess') worker.token = outcome.next;
        worker.busy = false;
      })();
      pending.add(operation);
      operation.then(() => pending.delete(operation), () => pending.delete(operation));
      inflightPeak = Math.max(inflightPeak, pending.size);
      if (config.kind === 'CODE' && nextSource === material.entries.length) { reason = 'source_exhausted'; break; }
    }
    while (reason === 'duration_completed' && now() < began + config.durationSeconds * 1000) {
      try { await sleep(Math.max(1, Math.ceil(began + config.durationSeconds * 1000 - now())), signal); }
      catch { reason = 'cancelled'; }
    }
    schedulingEnded = now();
    await Promise.allSettled(pending);
    if (signal?.aborted) reason = 'cancelled';
  } finally {
    clearInterval(memoryTimer); transport.close();
    await Promise.allSettled(pending); sampleMemory();
    for (const worker of workers) worker.token = undefined;
  }
  const elapsedMs = Math.max(0.001, now() - began), cpuUsed = process.cpuUsage(cpu);
  return { contractVersion: 'identity-capacity-report/1', kind: config.kind,
    target: { hostname: 'auth.saas.test', ports: config.origins.map(origin => origin.port), transport: 'HTTPS',
      loopbackOnly: true, certificateVerification: true, redirects: false, implicitRetries: 0, proxy: false },
    budgets: { ratePerSecond: config.ratePerSecond, concurrency: config.concurrency,
      requestedDurationSeconds: config.durationSeconds, requestTimeoutMs: config.requestTimeoutMs,
      maximumSockets: config.concurrency, maximumPending: config.concurrency,
      maximumWireBytes: LIMITS.wireBytes, maximumAttempts },
    completion: { reason, sourceExhausted: config.kind === 'CODE' && nextSource === material.entries.length, requestedDurationCompleted:
      reason === 'duration_completed' && schedulingEnded - began >= config.durationSeconds * 1000,
    schedulingSeconds: (schedulingEnded - began) / 1000, elapsedSeconds: elapsedMs / 1000 },
    counts, throughput: { successRps: counts.effectiveSuccess * 1000 / elapsedMs,
      attemptsRps: counts.attempts * 1000 / elapsedMs },
    latencyMs: { effectiveSuccess: percentiles(success, counts.effectiveSuccess), all: percentiles(all, counts.attempts) },
    scheduler: { inflightPeak, pendingPeak: inflightPeak, skippedBusy, skippedLate },
    client: { cpuUserMs: cpuUsed.user / 1000, cpuSystemMs: cpuUsed.system / 1000,
      cpuOneCorePercent: (cpuUsed.user + cpuUsed.system) / (elapsedMs * 10),
      memoryStartBytes: { rss: memoryStart.rss, heapUsed: memoryStart.heapUsed, external: memoryStart.external },
      memorySampledPeakBytes: memoryPeak, processLifetimeMaxRssBytes: process.resourceUsage().maxRSS * 1024 },
    evidence: { slaEstablished: false, successfulProtocolResponsesOnly: true,
      bottleneckEstablished: false, requiresServerMetricsAndFixtureContext: true } };
}
/** Private-file entry point; returns the same safe object written to outputPath.
 * Errors are deliberately replaced with a constant to protect callers that log.
 */
export async function runLoad(inputPath, outputPath) {
  let controller;
  const abort = () => controller?.abort();
  try {
    const output = privatePath(outputPath);
    if (fs.existsSync(output)) fail('OUTPUT_EXISTS');
    const config = validateInput(parseJson(readPrivate(inputPath, LIMITS.wireBytes)));
    if ([inputPath, config.clientFile, config.caFile, config.sourceFile].some(filename => path.resolve(filename) === path.resolve(output))) fail('OUTPUT_CONFLICT');
    const material = validateSources(config, parseJson(readPrivate(config.clientFile, LIMITS.wireBytes)),
      parseJson(readPrivate(config.sourceFile, LIMITS.sourceBytes)));
    const ca = readPrivate(config.caFile, LIMITS.wireBytes);
    if (!ca.toString('ascii').includes('-----BEGIN CERTIFICATE-----') || ca.toString('ascii').includes('PRIVATE KEY')) fail('CA_REJECTED');
    controller = new AbortController();
    process.once('SIGINT', abort); process.once('SIGTERM', abort);
    const report = await runSchedule(config, material, createTransport(config, ca), { signal: controller.signal });
    fs.writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: 'wx', mode: 0o600 });
    return report;
  } catch {
    fail('IDENTITY_CAPACITY_DRIVER_FAILED');
  } finally { process.removeListener('SIGINT', abort); process.removeListener('SIGTERM', abort); }
}
async function cli() {
  try {
    const args = process.argv.slice(2);
    if (args.length !== 4 || args[0] !== '--input' || args[2] !== '--output') fail('CLI_SHAPE_REJECTED');
    const report = await runLoad(args[1], args[3]);
    if (report.completion.reason === 'cancelled') process.exitCode = 2;
  } catch {
    // Even input/parser/OpenSSL/URL/filesystem errors can carry secret values.
    // Intentionally emit only a constant diagnostic, never an exception object.
    process.stderr.write('IDENTITY_CAPACITY_DRIVER_FAILED\n'); process.exitCode = 1;
  }
}

if (process.env.NODE_TEST_CONTEXT) {
  const { test } = await import('node:test');
  const assert = (await import('node:assert/strict')).default;
  const input = (kind = 'INTROSPECTION') => ({ contractVersion: 'identity-capacity-input/1', kind,
    origins: ['https://auth.saas.test:443'], ratePerSecond: 100, concurrency: 2, durationSeconds: 0.1,
    requestTimeoutMs: 100, clientFile: path.resolve('client.json'), caFile: path.resolve('ca.pem'), sourceFile: path.resolve('source.json') });
  const canary = 'SECRET_CANARY_NOT_IN_REPORT';
  const client = { clientId: canary, clientSecret: canary };
  test('input bounds, issuer-only standard or explicit port, and closed schemas', () => {
    assert.equal(validateInput(input()).origins[0].port, 443);
    assert.equal(validateInput({ ...input(), origins: ['https://auth.saas.test'] }).origins[0].port, 443);
    for (const patch of [{ ratePerSecond: 1001 }, { concurrency: 33 }, { durationSeconds: 1801 },
      { requestTimeoutMs: 10001 }, { origins: ['https://auth.saas.test:443', 'https://auth.saas.test'] },
      { origins: ['https://auth.saas.test:443/?secret=x'] }, { origins: ['https://outside.invalid:443'] },
      { origins: ['https://auth.saas.test:443@outside.invalid'] }, { extra: canary }]) {
      assert.throws(() => validateInput({ ...input(), ...patch }));
    }
  });
  test('source uniqueness, per-worker refresh isolation and wire limits', () => {
    const config = validateInput(input('REFRESH'));
    assert.throws(() => validateSources(config, client, { refreshTokens: ['same', 'same'] }));
    assert.throws(() => validateSources(config, client, { refreshTokens: ['one'] }));
    assert.equal(validateSources(config, client, { refreshTokens: ['one', 'two'] }).entries.length, 2);
    const code = { code: canary, verifier: 'x'.repeat(43), redirectUri: 'https://rp.saas.test/callback?private=canary' };
    assert.throws(() => validateSources(validateInput(input('CODE')), client, { codes: [code, code] }));
    assert.throws(() => makeRequest('REFRESH', client, '\uffff'.repeat(30000)));
  });
  test('classification requires effective protocol success and protects unknown refresh results', () => {
    const success = { status: 200, body: { access_token: 'access', token_type: 'Bearer', expires_in: 60, refresh_token: 'next' } };
    assert.equal(classify('REFRESH', success, 'original').next, 'next');
    assert.equal(classify('REFRESH', success, 'next').category, 'invalid');
    for (const result of [{ status: 200, body: {} }, { status: 503 }, { transport: 'timeout' }]) {
      assert.equal(classify('REFRESH', result, 'original').uncertain, true);
    }
    assert.equal(classify('REFRESH', { status: 429 }, 'original').retire, false);
    assert.equal(classify('INTROSPECTION', { status: 200, body: { active: false } }).category, 'denied');
    assert.equal(classify('INTROSPECTION', { status: 200, body: { active: true } }).category, 'effectiveSuccess');
    assert.equal(classify('INTROSPECTION', { status: 200, body: { active: 'true' } }).category, 'invalid');
    assert.deepEqual(percentiles(new Float64Array([100, 1, 2, 3]), 4), { p50: 2, p95: 100, p99: 100 });
    assert.deepEqual(percentiles(new Float64Array(0), 0), { p50: null, p95: null, p99: null });
  });
  test('DNS guard rejects external and mixed address sets before any socket', async () => {
    const lookup = (hostname, addresses, all = true) => new Promise(resolve => {
      restrictedLookup(hostname, { all }, (error, result, family) => resolve({ error, result, family }),
        (_host, _options, done) => done(null, addresses));
    });
    assert.equal((await lookup('outside.invalid', [{ address: '127.0.0.1', family: 4 }])).error.code, 'TARGET_REJECTED');
    assert.equal((await lookup('auth.saas.test', [{ address: '127.0.0.1', family: 4 },
      { address: '192.0.2.10', family: 4 }])).error.code, 'TARGET_REJECTED');
    assert.equal((await lookup('auth.saas.test', [{ address: '::1', family: 6 }], false)).result, '::1');
    assert.equal((await lookup('auth.saas.test', [{ address: '127.0.0.2', family: 4 }])).result.length, 1);
  });
  function fakeClock() {
    let value = 0;
    return { now: () => value, sleep: async ms => { value += ms; }, advance: ms => { value += ms; } };
  }
  test('fixed pace, bounded pending, valid-only throughput and secret-free report', async () => {
    const config = validateInput(input()), clock = fakeClock(); let closed = false, calls = 0;
    const report = await runSchedule(config, { client, entries: [canary] }, {
      send: async () => ({ status: ++calls % 2 ? 200 : 429, body: { active: true, secret: canary } }), close: () => { closed = true; },
    }, clock);
    assert.equal(report.counts.attempts, 10); assert.equal(report.counts.effectiveSuccess, 5);
    assert.equal(report.counts.rateLimited, 5); assert.ok(report.scheduler.pendingPeak <= config.concurrency);
    assert.equal(report.throughput.successRps, report.counts.effectiveSuccess / report.completion.elapsedSeconds);
    assert.equal(closed, true); assert.equal(JSON.stringify(report).includes(canary), false);
    assert.equal(JSON.stringify(report).includes(config.sourceFile), false);
    assert.equal(report.completion.elapsedSeconds, config.durationSeconds);
    assert.equal(report.completion.requestedDurationCompleted, true);
  });
  test('early final sleep wakeups still complete the full refresh and introspection duration', async () => {
    for (const kind of ['REFRESH', 'INTROSPECTION']) {
      const config = validateInput({ ...input(kind), ratePerSecond: 1, concurrency: 1, durationSeconds: 0.05 });
      let clock = 0, closed = 0; const sleeps = [];
      const report = await runSchedule(config, { client, entries: ['initial-refresh-or-token'] }, {
        send: async () => ({ status: 200, body: kind === 'REFRESH'
          ? { access_token: 'access', token_type: 'Bearer', expires_in: 60, refresh_token: 'next-refresh' }
          : { active: true } }),
        close: () => { closed += 1; },
      }, { now: () => clock, sleep: async ms => {
        sleeps.push(ms);
        assert.ok(sleeps.length <= 3, 'early wakeups must make bounded progress');
        // Reproduce sub-millisecond undersleep twice, then ordinary timer progress.
        clock += sleeps.length === 1 ? ms - 0.75 : sleeps.length === 2 ? 0.25 : ms;
      } });
      assert.deepEqual(sleeps, [50, 1, 1]);
      assert.equal(report.completion.reason, 'duration_completed');
      assert.equal(report.completion.requestedDurationCompleted, true);
      assert.ok(report.completion.schedulingSeconds >= config.durationSeconds);
      assert.ok(report.completion.schedulingSeconds < config.durationSeconds + 0.002);
      assert.equal(report.counts.attempts, 1); assert.equal(report.counts.effectiveSuccess, 1);
      assert.equal(closed, 1);
    }
  });
  test('cancellation during an early final wakeup stops waiting and closes transport once', async () => {
    const config = validateInput({ ...input(), ratePerSecond: 1, concurrency: 1, durationSeconds: 0.05 });
    const controller = new AbortController(); let clock = 0, sleeps = 0, closed = 0;
    const report = await runSchedule(config, { client, entries: [canary] }, {
      send: async () => ({ status: 200, body: { active: true } }), close: () => { closed += 1; },
    }, { now: () => clock, signal: controller.signal, sleep: async (ms, signal) => {
      sleeps += 1;
      assert.ok(sleeps <= 2, 'cancelled final wait must not loop');
      assert.equal(signal, controller.signal);
      if (sleeps === 1) clock += ms - 0.75;
      else { controller.abort(); throw new DOMException('Aborted', 'AbortError'); }
    } });
    assert.equal(sleeps, 2); assert.equal(closed, 1);
    assert.equal(report.completion.reason, 'cancelled');
    assert.equal(report.completion.requestedDurationCompleted, false);
    assert.ok(report.completion.schedulingSeconds < config.durationSeconds);
    assert.equal(report.counts.attempts, 1);
  });
  test('full elapsed duration with busy-worker under-delivery reports only actual successful work', async () => {
    const config = validateInput({ ...input(), concurrency: 1 });
    let clock = 0, finish, sleeps = 0, closed = 0;
    const report = await runSchedule(config, { client, entries: [canary] }, {
      send: () => new Promise(resolve => { finish = resolve; }), close: () => { closed += 1; },
    }, { now: () => clock, sleep: async ms => {
      sleeps += 1; assert.ok(sleeps <= 20, 'bounded pacing must not build a catch-up loop');
      clock += ms;
      if (clock >= config.durationSeconds * 1000) finish({ status: 200, body: { active: true } });
    } });
    assert.equal(report.completion.requestedDurationCompleted, true);
    assert.equal(report.completion.elapsedSeconds, config.durationSeconds);
    assert.equal(report.counts.attempts, 1); assert.equal(report.counts.effectiveSuccess, 1);
    assert.equal(report.scheduler.skippedBusy, 9); assert.equal(report.scheduler.pendingPeak, 1);
    assert.equal(report.throughput.successRps, 10);
    assert.ok(report.throughput.successRps < config.ratePerSecond);
    assert.equal(closed, 1);
  });
  test('CODE exhausts once-only source and reports actual duration', async () => {
    const config = validateInput(input('CODE')), clock = fakeClock(), seen = new Set();
    const entries = [0, 1, 2].map(index => ({ code: `${canary}${index}`, verifier: 'v'.repeat(43), redirectUri: 'https://rp.saas.test/cb' }));
    const report = await runSchedule(config, { client, entries }, {
      send: async (_target, request) => {
        const code = new URLSearchParams(request.body.toString()).get('code');
        assert.equal(seen.has(code), false); seen.add(code);
        return { status: 200, body: { access_token: 'a', token_type: 'Bearer', expires_in: 60 } };
      }, close() {},
    }, clock);
    assert.equal(report.counts.attempts, 3); assert.equal(report.completion.sourceExhausted, true);
    assert.equal(report.completion.requestedDurationCompleted, false);
    assert.ok(report.completion.elapsedSeconds < config.durationSeconds);
  });
  test('refresh chains rotate independently, keep 429 token, retire uncertain chain', async () => {
    const config = validateInput({ ...input('REFRESH'), durationSeconds: 0.2 }), clock = fakeClock();
    const seen = new Map(), retired = new Set();
    const report = await runSchedule(config, { client, entries: ['worker0:0', 'worker1:0'] }, {
      send: async (_target, request) => {
        const token = new URLSearchParams(request.body.toString()).get('refresh_token');
        const [worker, version] = token.split(':'); assert.equal(retired.has(worker), false);
        const attempt = (seen.get(worker) ?? 0) + 1; seen.set(worker, attempt);
        if (worker === 'worker0' && attempt === 1) return { status: 429 };
        if (worker === 'worker1' && attempt === 2) { retired.add(worker); return { transport: 'timeout' }; }
        if (worker === 'worker0' && attempt === 3) { retired.add(worker); return { status: 503 }; }
        assert.equal(Number(version), worker === 'worker0' ? attempt - 2 : attempt - 1);
        return { status: 200, body: { access_token: 'a', token_type: 'Bearer', expires_in: 60, refresh_token: `${worker}:${Number(version) + 1}` } };
      }, close() {},
    }, clock);
    assert.equal(report.counts.uncertainRetired, 2); assert.equal(report.counts.workersRetired, 2);
    assert.equal(report.counts.rateLimited, 1); assert.equal(report.counts.unavailable, 1);
    assert.equal(report.counts.network, 1); assert.equal(report.completion.reason, 'workers_retired');
  });
  test('slow workers never build an unbounded queue and cancellation closes transport', async () => {
    const config = validateInput(input()), clock = fakeClock(); let closed = false;
    const waiters = []; const controller = new AbortController();
    const report = await runSchedule(config, { client, entries: [canary] }, {
      send: () => new Promise(resolve => waiters.push(resolve)), close: () => { closed = true; waiters.forEach(resolve => resolve({ transport: 'cancelled' })); },
    }, { ...clock, signal: controller.signal, sleep: async ms => {
      clock.advance(ms);
      if (clock.now() >= 50) { controller.abort(); waiters.forEach(resolve => resolve({ transport: 'cancelled' })); }
    } });
    assert.equal(report.counts.attempts, 2); assert.equal(report.scheduler.pendingPeak, 2);
    assert.ok(report.scheduler.skippedBusy > 0); assert.equal(report.completion.reason, 'cancelled'); assert.equal(closed, true);
  });
  test('TLS transport keeps certificate defaults, bounds sockets and responses, and never retries', async () => {
    const { EventEmitter } = await import('node:events');
    let agentOptions, requestOptions, calls = 0, closed = false;
    class Agent {
      constructor(options) { agentOptions = options; }
      destroy() { closed = true; }
    }
    const implementation = { Agent, request(options, receive) {
      requestOptions = options; calls += 1;
      const request = new EventEmitter();
      request.destroy = () => {};
      request.end = () => queueMicrotask(() => {
        const response = new EventEmitter(); response.rawHeaders = ['Content-Type', 'application/json'];
        response.headers = { 'content-type': 'application/json' }; response.statusCode = 200;
        receive(response); response.emit('data', Buffer.alloc(65537));
      });
      return request;
    } };
    const config = validateInput(input());
    const transport = createTransport(config, Buffer.from('PUBLIC_TEST_CA'), implementation);
    const response = await transport.send(config.origins[0], makeRequest(config.kind, client, canary));
    transport.close();
    assert.equal(response.transport, 'response_limit'); assert.equal(calls, 1); assert.equal(closed, true);
    assert.equal(agentOptions.rejectUnauthorized, true); assert.equal(agentOptions.keepAlive, true);
    assert.equal(agentOptions.maxTotalSockets, 2); assert.equal(agentOptions.maxSockets, 2);
    assert.equal(own(agentOptions, 'checkServerIdentity'), false);
    assert.equal(requestOptions.servername, 'auth.saas.test'); assert.equal(requestOptions.maxHeaderSize, 8192);
  });
  test('TLS request deadline includes unresolved connection and cancellation is bounded', async () => {
    const { EventEmitter } = await import('node:events'); let destroyed = 0, calls = 0;
    const implementation = { Agent: class { destroy() {} }, request() {
      calls += 1; const request = new EventEmitter(); request.end = () => {};
      request.destroy = () => { destroyed += 1; }; return request;
    } };
    const config = validateInput({ ...input(), requestTimeoutMs: 5 });
    const transport = createTransport(config, Buffer.from('PUBLIC_TEST_CA'), implementation);
    try {
      const expired = await transport.send(config.origins[0], makeRequest(config.kind, client, canary));
      assert.equal(expired.transport, 'timeout');
      const controller = new AbortController();
      const pending = transport.send(config.origins[0], makeRequest(config.kind, client, canary), controller.signal);
      controller.abort(); assert.equal((await pending).transport, 'cancelled');
      assert.equal(calls, 2); assert.equal(destroyed, 2);
    } finally { transport.close(); }
  });
  test('private reads enforce Linux ownership, exact mode, regular file and no final symlink', { skip: process.platform !== 'linux' }, async () => {
    const os = await import('node:os');
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'identity-capacity-selftest-'));
    try {
      const filename = path.join(directory, 'private');
      fs.writeFileSync(filename, canary, { mode: 0o600 });
      assert.equal(readPrivate(filename, 65536).toString(), canary);
      assert.throws(() => readPrivate(filename, 1));
      fs.chmodSync(filename, 0o644); assert.throws(() => readPrivate(filename, 65536));
      fs.chmodSync(filename, 0o600);
      const link = path.join(directory, 'link'); fs.symlinkSync(filename, link);
      assert.throws(() => readPrivate(link, 65536)); assert.throws(() => readPrivate(directory, 65536));
    } finally { fs.rmSync(directory, { recursive: true, force: true }); }
  });
  test('private CLI failure emits only constant diagnostic and cannot publish a secret', async () => {
    const { execFile } = await import('node:child_process');
    const { promisify } = await import('node:util');
    const env = { ...process.env }; delete env.NODE_TEST_CONTEXT;
    try {
      await promisify(execFile)(process.execPath, [process.argv[1], '--input', canary, '--output', path.resolve('never-created-capacity-output.json')], { env });
      assert.fail('CLI should reject relative private input');
    } catch (error) {
      assert.equal(error.stdout, ''); assert.equal(error.stderr, 'IDENTITY_CAPACITY_DRIVER_FAILED\n');
    }
    await assert.rejects(runLoad(canary, path.resolve('never-created-capacity-output.json')),
      error => error.message === 'IDENTITY_CAPACITY_DRIVER_FAILED' && !error.stack.includes(canary));
  });
} else if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await cli();
}
