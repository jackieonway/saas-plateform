/** Fixed-condition, task-owned alert pipeline. This is not a PromQL engine or
 * Alertmanager. Import exports from a fixture driver; pure/local tests:
 * node --test phase22-alerts.mjs
 */
import fs from 'node:fs';
import path from 'node:path';
import https from 'node:https';
import http from 'node:http';
import dns from 'node:dns';
import { createHash, timingSafeEqual } from 'node:crypto';
import { performance } from 'node:perf_hooks';

const CONDITIONS = ['DATABASE_UNAVAILABLE', 'PASSWORD_QUEUE_PRESSURE', 'NEW_TERMINAL_DELIVERY'];
const STATES = ['FIRING', 'RESOLVED', 'ACKNOWLEDGED'];
const BUDGET = Object.freeze({ metricsBytes: 2 * 1024 * 1024, metricsLines: 40000,
  metricsTimeoutMs: 2000, deliveryTimeoutMs: 1000, pollMinimumMs: 100,
  lifetimeMs: 1800000, maximumPolls: 18000, backlog: 16, eventTtlMs: 60000,
  maximumAttempts: 5, retryBaseMs: 100, retryMaximumMs: 5000, batch: 4,
  collectorBodyBytes: 2048, collectorConnections: 4, collectorDedup: 128,
  collectorTtlMs: 120000, collectorEvents: 64 });
const error = () => { throw new Error('IDENTITY_FIXED_ALERT_FAILED'); };
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const digest = value => createHash('sha256').update(value).digest();
const isToken = value => typeof value === 'string' && /^[\x21-\x7e]{16,256}$/.test(value);
function shape(value, keys) {
  if (!object(value) || keys.length !== Object.keys(value).length || keys.some(key => !Object.hasOwn(value, key))) error();
}
function readPrivate(filename, maximum) {
  if (process.platform !== 'linux' || typeof filename !== 'string' || !path.isAbsolute(filename)
      || filename.includes('\0') || !fs.constants.O_NOFOLLOW) error();
  const fd = fs.openSync(filename, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
  try {
    const stat = fs.fstatSync(fd);
    if (!stat.isFile() || stat.uid !== process.getuid() || (stat.mode & 0o777) !== 0o600
        || stat.size < 1 || stat.size > maximum) error();
    const bytes = Buffer.alloc(stat.size + 1); let count = 0;
    while (count < bytes.length) {
      const size = fs.readSync(fd, bytes, count, bytes.length - count, null);
      if (!size) break;
      count += size;
    }
    if (count !== stat.size) error();
    return bytes.subarray(0, count);
  } finally { fs.closeSync(fd); }
}
function tokenFile(filename) {
  const value = readPrivate(filename, 258).toString('utf8').replace(/\r?\n$/, '');
  if (!isToken(value)) error();
  return value;
}
function input(value) {
  shape(value, ['contractVersion', 'metricsOrigin', 'caFile', 'metricsTokenFile', 'collectorOrigin', 'collectorTokenFile']);
  if (value.contractVersion !== 'identity-fixed-alert-input/1') error();
  const metrics = typeof value.metricsOrigin === 'string'
    && /^https:\/\/auth\.saas\.test(?::([1-9][0-9]{0,4}))?$/.exec(value.metricsOrigin);
  const collector = typeof value.collectorOrigin === 'string'
    && /^http:\/\/127\.0\.0\.1:([1-9][0-9]{0,4})$/.exec(value.collectorOrigin);
  if (!metrics || !collector || Number(metrics[1] ?? 443) > 65535 || Number(collector[1]) > 65535) error();
  return { ...value, metricsPort: Number(metrics[1] ?? 443), collectorPort: Number(collector[1]) };
}
function loopbackLookup(hostname, options, done) {
  if (hostname !== 'auth.saas.test') return done(new Error('TARGET_REJECTED'));
  dns.lookup(hostname, { ...options, all: true }, (failure, addresses) => {
    if (failure) return done(new Error('TARGET_UNAVAILABLE'));
    if (!addresses?.length || addresses.some(({ address }) => address !== '::1' && !/^127(?:\.[0-9]{1,3}){3}$/.test(address))) {
      return done(new Error('TARGET_REJECTED'));
    }
    if (options.all) done(null, addresses);
    else done(null, addresses[0].address, addresses[0].family);
  });
}
function boundedRequest(implementation, options, body, maximum, deadline, signal) {
  return new Promise(resolve => {
    let settled = false, request;
    const finish = result => {
      if (settled) return;
      settled = true; clearTimeout(timer); signal?.removeEventListener('abort', abort); resolve(result);
    };
    const stop = () => { finish({ ok: false }); request?.destroy(); };
    const abort = () => stop();
    const timer = setTimeout(stop, deadline);
    if (signal?.aborted) return stop();
    signal?.addEventListener('abort', abort, { once: true });
    try {
      request = implementation.request({ ...options, maxHeaderSize: 8192 }, response => {
        let bytes = 0; const chunks = [];
        if (response.headers['content-encoding'] || (response.headers['content-length'] !== undefined
            && (!/^[0-9]+$/.test(response.headers['content-length']) || Number(response.headers['content-length']) > maximum))) return stop();
        response.on('data', chunk => {
          if (settled) return;
          bytes += chunk.length;
          if (bytes > maximum) stop();
          else chunks.push(chunk);
        });
        response.once('error', stop); response.once('aborted', stop);
        response.once('end', () => {
          if (!response.complete) return stop();
          finish({ ok: true, status: response.statusCode, type: response.headers['content-type'] ?? '',
            bytes: Buffer.concat(chunks) });
        });
      });
      request.once('error', stop); request.end(body);
    } catch { stop(); }
  });
}
function scraper(config, ca, token, implementation = https) {
  const agent = new implementation.Agent({ keepAlive: true, maxSockets: 1, maxTotalSockets: 1,
    maxFreeSockets: 1, timeout: BUDGET.metricsTimeoutMs, ca, rejectUnauthorized: true, lookup: loopbackLookup });
  return { async scrape(signal) {
    const result = await boundedRequest(implementation, { hostname: 'auth.saas.test', servername: 'auth.saas.test',
      port: config.metricsPort, method: 'GET', path: '/internal/identity/metrics', agent,
      headers: { Authorization: `Bearer ${token}`, Accept: 'text/plain', 'Cache-Control': 'no-store' } },
    undefined, BUDGET.metricsBytes, BUDGET.metricsTimeoutMs, signal);
    if (!result.ok || result.status !== 200 || !/^text\/plain(?:\s*;|$)/i.test(result.type)) return null;
    try { return parseMetrics(result.bytes.toString('utf8')); } catch { return null; }
  }, close: () => agent.destroy() };
}
function sender(port, token) {
  const agent = new http.Agent({ keepAlive: true, maxSockets: 1, maxTotalSockets: 1,
    maxFreeSockets: 1, timeout: BUDGET.deliveryTimeoutMs });
  return { async send(payload, signal) {
    const body = Buffer.from(JSON.stringify(payload));
    const result = await boundedRequest(http, { hostname: '127.0.0.1', port, method: 'POST',
      path: '/internal/identity/alerts', agent, headers: { Authorization: `Bearer ${token}`,
        'Content-Type': 'application/json', 'Content-Length': String(body.length) } },
    body, BUDGET.collectorBodyBytes, BUDGET.deliveryTimeoutMs, signal);
    return result.ok && result.status >= 200 && result.status < 300;
  }, close: () => agent.destroy() };
}
/** Consume only three fixed gauges and three fixed TERMINAL job series.
 * Unknown series are ignored, never promoted to dynamic alert labels.
 */
export function parseMetrics(text) {
  if (typeof text !== 'string' || Buffer.byteLength(text) > BUDGET.metricsBytes) error();
  const lines = text.split('\n'); if (lines.length > BUDGET.metricsLines) error();
  const plain = new Map(), terminal = new Map();
  const gauges = ['identity_database_up', 'identity_password_queued', 'identity_delivery_observation_up'];
  for (const line of lines) {
    if (line.length > 8192) error();
    if (!line || line.startsWith('#')) continue;
    const gauge = gauges.find(name => line === name || line.startsWith(`${name} `) || line.startsWith(`${name}{`));
    if (gauge) {
      const match = /^([a-z_]+) ([0-9]+)$/.exec(line);
      if (!match || plain.has(gauge) || !Number.isSafeInteger(Number(match[2]))) error();
      plain.set(gauge, Number(match[2]));
    }
    if (line.startsWith('identity_delivery_jobs{')) {
      const match = /^identity_delivery_jobs\{kind="(LOGOUT|INVITATION|SECURITY)",state="(PENDING|LEASED|DELIVERED|TERMINAL|DEGRADED)"\} ([0-9]+)$/.exec(line);
      if (!match || !Number.isSafeInteger(Number(match[3]))) error();
      if (match[2] === 'TERMINAL') {
        if (terminal.has(match[1])) error();
        terminal.set(match[1], Number(match[3]));
      }
    }
  }
  const databaseUp = plain.get(gauges[0]), passwordQueued = plain.get(gauges[1]), deliveryUp = plain.get(gauges[2]);
  if (![0, 1].includes(databaseUp) || ![0, 1].includes(deliveryUp) || passwordQueued === undefined) error();
  const total = [...terminal.values()].reduce((sum, count) => sum + count, 0);
  if (deliveryUp === 1 && (terminal.size !== 3 || !Number.isSafeInteger(total))) error();
  return { databaseUp, passwordQueued, terminalCount: deliveryUp === 1 ? total : null };
}
function eventShape(payload) {
  shape(payload, ['contractVersion', 'sequence', 'condition', 'state']);
  if (payload.contractVersion !== 'identity-fixed-alert/1' || !Number.isSafeInteger(payload.sequence) || payload.sequence < 1
      || !CONDITIONS.includes(payload.condition) || !STATES.includes(payload.state)
      || (payload.condition === CONDITIONS[2] ? payload.state === 'RESOLVED' : payload.state === 'ACKNOWLEDGED')) error();
  return { contractVersion: payload.contractVersion, sequence: payload.sequence, condition: payload.condition, state: payload.state };
}
function collectorCore(now = () => performance.now()) {
  const seen = new Map(), retained = [];
  const counters = { accepted: 0, duplicates: 0, conflicts: 0, rejectedFull: 0, evictedEvents: 0 };
  return { accept(raw) {
    let payload;
    try { payload = eventShape(raw); } catch { return 400; }
    for (const [key, entry] of seen) if (now() - entry.at >= BUDGET.collectorTtlMs) seen.delete(key);
    const fingerprint = digest(JSON.stringify(payload)), previous = seen.get(payload.sequence);
    if (previous) {
      if (!timingSafeEqual(previous.fingerprint, fingerprint)) { counters.conflicts += 1; return 409; }
      counters.duplicates += 1; return 204;
    }
    if (seen.size >= BUDGET.collectorDedup) { counters.rejectedFull += 1; return 503; }
    seen.set(payload.sequence, { fingerprint, at: now() });
    counters.accepted += 1;
    if (retained.length >= BUDGET.collectorEvents) { retained.shift(); counters.evictedEvents += 1; }
    retained.push(payload); return 204;
  }, snapshot() {
    return { contractVersion: 'identity-fixed-collector-report/1', counters: { ...counters },
      dedupEntries: seen.size, retainedEvents: retained.map(event => ({ ...event })),
      semantics: 'IN_MEMORY_FIXED_CONDITIONS', durableReception: false };
  } };
}
/** Starts only the explicitly requested task-owned loopback collector. */
export async function startCollector({ tokenFile: filename, port = 0 }) {
  let server;
  try {
    if (!Number.isInteger(port) || port < 0 || port > 65535) error();
    const expected = digest(tokenFile(filename)), core = collectorCore();
    server = http.createServer({ maxHeaderSize: 8192, connectionsCheckingInterval: 250 }, (request, response) => {
      const deadline = setTimeout(() => { request.destroy(); response.destroy(); }, 2000);
      response.once('close', () => clearTimeout(deadline));
      const reply = status => { response.writeHead(status, { 'Cache-Control': 'no-store' }); response.end(); };
      const authCount = request.rawHeaders.filter((_, index) => index % 2 === 0 && request.rawHeaders[index].toLowerCase() === 'authorization').length;
      const supplied = /^Bearer ([\x21-\x7e]{16,256})$/.exec(request.headers.authorization ?? '')?.[1];
      if (request.method !== 'POST' || request.url !== '/internal/identity/alerts' || request.headers.cookie
          || request.headers['content-encoding'] || authCount !== 1 || !supplied || !timingSafeEqual(expected, digest(supplied))) {
        request.resume(); return reply(401);
      }
      if (!/^application\/json(?:\s*;|$)/i.test(request.headers['content-type'] ?? '')) { request.resume(); return reply(415); }
      let size = 0, finished = false; const chunks = [];
      request.on('data', chunk => {
        if (finished) return;
        size += chunk.length;
        if (size > BUDGET.collectorBodyBytes) { finished = true; reply(413); request.destroy(); }
        else chunks.push(chunk);
      });
      request.once('error', () => { if (!finished) { finished = true; response.destroy(); } });
      request.once('end', () => {
        if (finished) return;
        finished = true;
        try { reply(core.accept(JSON.parse(Buffer.concat(chunks).toString('utf8')))); }
        catch { reply(400); }
      });
    });
    server.maxConnections = BUDGET.collectorConnections;
    server.requestTimeout = 2000; server.headersTimeout = 2000; server.keepAliveTimeout = 1000;
    server.setTimeout(2000, socket => socket.destroy());
    server.on('clientError', (_failure, socket) => socket.destroy());
    await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); });
    let closed = false;
    return { origin: `http://127.0.0.1:${server.address().port}`, snapshot: () => core.snapshot(),
      async close() {
        if (closed) return;
        closed = true;
        await new Promise(resolve => { server.close(resolve); server.closeAllConnections(); });
      } };
  } catch {
    if (server) { server.close(); server.closeAllConnections(); }
    error();
  }
}
function pipelineCore(source, delivery, { now = () => performance.now(), metricsPort = 443, collectorPort = 0 } = {}) {
  const queue = [], began = now(), controller = new AbortController();
  const observed = [false, false], lastFiring = [0, 0], lastFiringAck = [0, 0];
  const deliveredStates = ['NONE', 'NONE'];
  const terminal = { baseline: 0, observed: null, firingCount: 0, state: 'IDLE', firingSequence: 0, decreases: 0 };
  const counters = { polls: 0, scrapeSuccess: 0, scrapeFailure: 0, busy: 0, throttled: 0,
    enqueued: 0, delivered: 0, deliveryAttempts: 0, deliveryFailures: 0, deduplicatedTransitions: 0,
    backlogRejected: 0, expired: 0, retryExhausted: 0, dependencyFailures: 0, abandonedOnClose: 0 };
  let busy = false, closed = false, lastPoll = -Infinity, sequence = 0, backlogPeak = 0;
  function report() {
    return { contractVersion: 'identity-fixed-alert-report/1', pipeline: 'FIXED_CONDITIONS',
      provesPromql: false, provesAlertmanager: false, durableDelivery: false,
      target: { metricsPort, collectorPort, strictMetricsTls: true, loopbackOnly: true },
      budgets: { ...BUDGET }, counters: { ...counters }, backlog: queue.length, backlogPeak,
      conditions: CONDITIONS.slice(0, 2).map((condition, index) => ({ condition, observedFiring: observed[index], deliveredState: deliveredStates[index] })),
      terminal: { baseline: terminal.baseline, observed: terminal.observed, firingCount: terminal.firingCount,
        state: terminal.state, decreases: terminal.decreases, databaseMutated: false }, closed };
  }
  function enqueue(condition, state, count = null, dependsOn = 0) {
    if (queue.length >= BUDGET.backlog) { counters.backlogRejected += 1; return null; }
    const payload = { contractVersion: 'identity-fixed-alert/1', sequence: ++sequence, condition, state };
    queue.push({ payload, count, dependsOn, created: now(), next: now(), attempts: 0 });
    counters.enqueued += 1; backlogPeak = Math.max(backlogPeak, queue.length); return payload.sequence;
  }
  function dropped(entry) {
    if (entry.payload.condition === CONDITIONS[2]) terminal.state = entry.payload.state === 'FIRING' ? 'FIRING_UNDELIVERED' : 'ACK_UNDELIVERED';
  }
  function expire() {
    for (let index = 0; index < queue.length;) {
      if (now() - queue[index].created >= BUDGET.eventTtlMs) {
        const [entry] = queue.splice(index, 1); counters.expired += 1; dropped(entry);
      } else index += 1;
    }
  }
  async function flush() {
    expire();
    for (let sent = 0; queue.length && sent < BUDGET.batch && !closed; sent += 1) {
      const entry = queue[0]; if (entry.next > now()) break;
      const index = CONDITIONS.indexOf(entry.payload.condition);
      if (entry.payload.state === 'RESOLVED' && lastFiringAck[index] !== entry.dependsOn) {
        queue.shift(); counters.dependencyFailures += 1; continue;
      }
      entry.attempts += 1; counters.deliveryAttempts += 1;
      let accepted = false;
      try { accepted = await delivery.send(entry.payload, controller.signal); } catch { /* safe failure only */ }
      if (closed) return;
      if (accepted === true) {
        queue.shift(); counters.delivered += 1;
        if (index < 2) {
          deliveredStates[index] = entry.payload.state;
          if (entry.payload.state === 'FIRING') lastFiringAck[index] = entry.payload.sequence;
        } else if (entry.payload.state === 'FIRING') terminal.state = 'FIRING_DELIVERED';
        else { terminal.baseline = entry.count; terminal.state = 'IDLE'; }
      } else {
        counters.deliveryFailures += 1;
        if (entry.attempts >= BUDGET.maximumAttempts) { queue.shift(); counters.retryExhausted += 1; dropped(entry); }
        else entry.next = now() + Math.min(BUDGET.retryMaximumMs, BUDGET.retryBaseMs * 2 ** (entry.attempts - 1));
        break;
      }
      expire();
    }
  }
  function evaluate(sample) {
    const active = [sample.databaseUp === 0, sample.passwordQueued > 0];
    for (let index = 0; index < 2; index += 1) {
      if (active[index] === observed[index]) { counters.deduplicatedTransitions += 1; continue; }
      const event = enqueue(CONDITIONS[index], active[index] ? 'FIRING' : 'RESOLVED', null, lastFiring[index]);
      if (event !== null) { observed[index] = active[index]; if (active[index]) lastFiring[index] = event; }
    }
    if (sample.terminalCount === null) return;
    if ((terminal.observed !== null && sample.terminalCount < terminal.observed) || sample.terminalCount < terminal.baseline) terminal.decreases += 1;
    terminal.observed = sample.terminalCount;
    if (sample.terminalCount > terminal.baseline && terminal.state === 'IDLE') {
      const event = enqueue(CONDITIONS[2], 'FIRING', sample.terminalCount);
      if (event !== null) { terminal.firingCount = sample.terminalCount; terminal.firingSequence = event; terminal.state = 'FIRING_QUEUED'; }
    }
  }
  async function poll() {
    if (closed) return { status: 'CLOSED', report: report() };
    if (busy) { counters.busy += 1; return { status: 'BUSY', report: report() }; }
    if (now() - began >= BUDGET.lifetimeMs || counters.polls >= BUDGET.maximumPolls) {
      close(); return { status: 'EXPIRED', report: report() };
    }
    if (now() - lastPoll < BUDGET.pollMinimumMs) { counters.throttled += 1; return { status: 'THROTTLED', report: report() }; }
    busy = true; lastPoll = now(); counters.polls += 1;
    try {
      let sample = null;
      try { sample = await source.scrape(controller.signal); } catch { /* hold last state */ }
      if (closed) return { status: 'CLOSED', report: report() };
      if (sample === null) counters.scrapeFailure += 1;
      else { counters.scrapeSuccess += 1; evaluate(sample); }
      await flush();
      return { status: sample === null ? 'SCRAPE_FAILED' : 'OK', report: report() };
    } catch { error(); }
    finally { busy = false; }
  }
  async function acknowledgeTerminal() {
    if (now() - began >= BUDGET.lifetimeMs) close();
    if (closed || busy || terminal.state !== 'FIRING_DELIVERED') return { accepted: false, report: report() };
    busy = true;
    try {
      if (enqueue(CONDITIONS[2], 'ACKNOWLEDGED', terminal.firingCount) === null) return { accepted: false, report: report() };
      terminal.state = 'ACK_QUEUED'; await flush();
      return { accepted: true, report: report() };
    } catch { error(); }
    finally { busy = false; }
  }
  function close() {
    if (closed) return;
    closed = true; controller.abort(); source.close(); delivery.close();
    counters.abandonedOnClose += queue.length;
    for (const entry of queue) dropped(entry);
    queue.length = 0;
  }
  return { poll, acknowledgeTerminal, report, close };
}
/** Load only task-private credentials; neither the config nor errors are logged. */
export function createAlertPipeline(inputPath) {
  let source, delivery;
  try {
    const config = input(JSON.parse(readPrivate(inputPath, 65536).toString('utf8')));
    const metricsToken = tokenFile(config.metricsTokenFile), collectorToken = tokenFile(config.collectorTokenFile);
    if (timingSafeEqual(digest(metricsToken), digest(collectorToken))) error();
    const ca = readPrivate(config.caFile, 65536);
    if (!ca.toString('ascii').includes('-----BEGIN CERTIFICATE-----') || ca.toString('ascii').includes('PRIVATE KEY')) error();
    source = scraper(config, ca, metricsToken); delivery = sender(config.collectorPort, collectorToken);
    return pipelineCore(source, delivery, { metricsPort: config.metricsPort, collectorPort: config.collectorPort });
  } catch { source?.close(); delivery?.close(); error(); }
}

if (process.env.NODE_TEST_CONTEXT) {
  const { test } = await import('node:test');
  const assert = (await import('node:assert/strict')).default;
  const metrics = (database = 1, queued = 0, terminal = 0, delivery = 1) =>
    `identity_database_up ${database}\nidentity_password_queued ${queued}\nidentity_delivery_observation_up ${delivery}\n`
    + (delivery ? ['LOGOUT', 'INVITATION', 'SECURITY'].map((kind, index) => `identity_delivery_jobs{kind="${kind}",state="TERMINAL"} ${index ? 0 : terminal}`).join('\n') : '');
  function fake(source = metrics(), deliver = () => true) {
    let time = 0, text = source, sent = [], closed = false;
    const pipeline = pipelineCore({ scrape: async () => text === null ? null : parseMetrics(text), close() { closed = true; } },
      { send: async payload => { sent.push({ ...payload }); return deliver(payload); }, close() {} }, { now: () => time });
    return { pipeline, sent, set: value => { text = value; }, advance: (ms = 100) => { time += ms; }, closed: () => closed };
  }
  test('strict metrics parsing never treats missing or failed observation as zero', () => {
    assert.equal(parseMetrics(metrics()).terminalCount, 0);
    assert.equal(parseMetrics(metrics(0, 2, 0, 0)).terminalCount, null);
    for (const value of ['identity_database_up 1', `${metrics()}\nidentity_password_queued 0`,
      metrics().replace('identity_password_queued 0', 'identity_password_queued NaN'),
      metrics().replace('kind="LOGOUT"', 'kind="SECRET_DYNAMIC_LABEL"'),
      metrics().replace(/\nidentity_delivery_jobs[^\n]+$/, ''), 'x'.repeat(BUDGET.metricsBytes + 1)]) assert.throws(() => parseMetrics(value));
  });
  test('database and password conditions deliver FIRING then RESOLVED once', async () => {
    const fixture = fake();
    try {
      await fixture.pipeline.poll(); fixture.set(metrics(0, 1, 0, 0)); fixture.advance(); await fixture.pipeline.poll();
      fixture.advance(); await fixture.pipeline.poll();
      fixture.set(metrics()); fixture.advance(); await fixture.pipeline.poll();
      assert.deepEqual(fixture.sent.map(event => event.state), ['FIRING', 'FIRING', 'RESOLVED', 'RESOLVED']);
      assert.equal(fixture.pipeline.report().counters.delivered, 4);
    } finally { fixture.pipeline.close(); }
  });
  test('TERMINAL begins at baseline zero, requires delivered FIRING and ACK 2xx, and never resolves', async () => {
    let available = true; const fixture = fake(metrics(1, 0, 4), () => available);
    try {
      await fixture.pipeline.poll();
      assert.equal(fixture.pipeline.report().terminal.baseline, 0);
      assert.equal(fixture.pipeline.report().terminal.state, 'FIRING_DELIVERED');
      available = false; await fixture.pipeline.acknowledgeTerminal();
      assert.equal(fixture.pipeline.report().terminal.baseline, 0);
      available = true; fixture.advance(); await fixture.pipeline.poll();
      assert.equal(fixture.pipeline.report().terminal.baseline, 4);
      fixture.set(metrics(1, 0, 5)); fixture.advance(); await fixture.pipeline.poll();
      assert.equal(fixture.pipeline.report().terminal.state, 'FIRING_DELIVERED');
      assert.equal(fixture.sent.some(event => event.condition === CONDITIONS[2] && event.state === 'RESOLVED'), false);
      fixture.set(metrics(1, 0, 0)); fixture.advance(); await fixture.pipeline.poll();
      assert.equal(fixture.pipeline.report().terminal.baseline, 4); assert.ok(fixture.pipeline.report().terminal.decreases > 0);
    } finally { fixture.pipeline.close(); }
  });
  test('collector loss deduplicates repeated scrapes and retries the exact event before 2xx acknowledgement', async () => {
    let available = false; const fixture = fake(metrics(0, 0, 0, 0), () => available);
    try {
      await fixture.pipeline.poll(); assert.equal((await fixture.pipeline.acknowledgeTerminal()).accepted, false);
      for (let index = 0; index < 3; index += 1) { fixture.advance(10); await fixture.pipeline.poll(); }
      assert.equal(fixture.pipeline.report().backlog, 1); assert.equal(fixture.pipeline.report().counters.delivered, 0);
      available = true; fixture.advance(100); await fixture.pipeline.poll();
      assert.equal(fixture.sent[0].sequence, fixture.sent[1].sequence);
      assert.equal(fixture.pipeline.report().counters.delivered, 1); assert.equal(fixture.pipeline.report().backlog, 0);
    } finally { fixture.pipeline.close(); }
  });
  test('retry count, TTL, backlog and dependent recovery remain bounded', async () => {
    const fixture = fake(metrics(0, 0, 0, 0), () => false);
    try {
      await fixture.pipeline.poll();
      for (let index = 0; index < 4; index += 1) { fixture.advance(5000); await fixture.pipeline.poll(); }
      assert.equal(fixture.pipeline.report().counters.deliveryAttempts, BUDGET.maximumAttempts);
      assert.equal(fixture.pipeline.report().counters.retryExhausted, 1);
      fixture.set(metrics()); fixture.advance(); await fixture.pipeline.poll();
      assert.equal(fixture.pipeline.report().counters.dependencyFailures, 1);
      for (let index = 0; index < 30; index += 1) {
        fixture.set(metrics(index % 2, index % 2)); fixture.advance(); await fixture.pipeline.poll();
      }
      assert.ok(fixture.pipeline.report().backlog <= BUDGET.backlog);
      assert.ok(fixture.pipeline.report().counters.backlogRejected > 0);
      fixture.advance(BUDGET.eventTtlMs); await fixture.pipeline.poll();
      assert.ok(fixture.pipeline.report().counters.expired > 0);
    } finally { fixture.pipeline.close(); }
  });
  test('unknown scrape preserves observed incidents and reports failure without secrets', async () => {
    const canary = 'SECRET_FIXED_ALERT_CANARY'; const fixture = fake(metrics(0, 1, 2));
    try {
      fixture.set(`${metrics(0, 1, 2)}\nunknown_series{user="${canary}"} 1`);
      await fixture.pipeline.poll(); fixture.set(null); fixture.advance(); await fixture.pipeline.poll();
      const report = fixture.pipeline.report();
      assert.equal(report.counters.scrapeFailure, 1); assert.ok(report.conditions.every(condition => condition.observedFiring));
      assert.equal(report.terminal.baseline, 0);
      assert.equal(JSON.stringify(report).includes(canary), false);
      assert.equal(report.provesPromql, false); assert.equal(report.provesAlertmanager, false);
      assert.equal(fixture.sent.some(event => event.state === 'RESOLVED'), false);
      assert.throws(() => eventShape({ ...fixture.sent[0], secret: canary }));
    } finally { fixture.pipeline.close(); }
  });
  test('collector bounds dedup TTL, rejects conflicting sequence and retains only safe fixed payloads', () => {
    let clock = 0; const core = collectorCore(() => clock);
    const event = sequence => ({ contractVersion: 'identity-fixed-alert/1', sequence, condition: CONDITIONS[0], state: 'FIRING' });
    assert.equal(core.accept(event(1)), 204); assert.equal(core.accept(event(1)), 204);
    assert.equal(core.accept({ ...event(1), state: 'RESOLVED' }), 409);
    for (let index = 2; index <= BUDGET.collectorDedup; index += 1) assert.equal(core.accept(event(index)), 204);
    assert.equal(core.accept(event(129)), 503);
    assert.equal(core.snapshot().retainedEvents.length, BUDGET.collectorEvents);
    clock = BUDGET.collectorTtlMs; assert.equal(core.accept(event(129)), 204);
    assert.equal(core.accept({ ...event(130), user: 'SECRET' }), 400);
    assert.equal(core.accept({ ...event(130), condition: CONDITIONS[2], state: 'RESOLVED' }), 400);
  });
  test('actual owned loopback collector proves lost acknowledgement dedup and exact auth', { skip: process.platform !== 'linux' }, async () => {
    const os = await import('node:os');
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'identity-alert-selftest-'));
    let collector, transport, pipeline;
    try {
      const filename = path.join(directory, 'collector-token');
      const credential = 'LOCAL_COLLECTOR_SELFTEST_7f6bd934'; fs.writeFileSync(filename, credential, { mode: 0o600 });
      collector = await startCollector({ tokenFile: filename });
      const port = Number(new URL(collector.origin).port); transport = sender(port, credential);
      let clock = 0, loseFirstAck = true;
      pipeline = pipelineCore({ scrape: async () => parseMetrics(metrics(0, 0, 0, 0)), close() {} },
        { async send(payload, signal) {
          const accepted = await transport.send(payload, signal);
          if (loseFirstAck) { loseFirstAck = false; return false; }
          return accepted;
        }, close() {} }, { now: () => clock });
      await pipeline.poll();
      assert.equal(pipeline.report().counters.delivered, 0); assert.equal(pipeline.report().backlog, 1);
      clock += 100; await pipeline.poll();
      assert.equal(pipeline.report().counters.delivered, 1); assert.equal(pipeline.report().backlog, 0);
      const payload = { contractVersion: 'identity-fixed-alert/1', sequence: 1, condition: CONDITIONS[0], state: 'FIRING' };
      const wrong = sender(port, 'WRONG_COLLECTOR_SELFTEST_759ff164');
      try { assert.equal(await wrong.send(payload), false); } finally { wrong.close(); }
      assert.equal(collector.snapshot().counters.accepted, 1); assert.equal(collector.snapshot().counters.duplicates, 1);
      assert.equal(JSON.stringify(collector.snapshot()).includes(credential), false);
      await collector.close(); assert.equal(await transport.send(payload), false);
    } finally {
      pipeline?.close(); transport?.close(); await collector?.close(); fs.rmSync(directory, { recursive: true, force: true });
    }
  });
  test('private input and secret failures expose only constant errors', { skip: process.platform !== 'linux' }, async () => {
    const os = await import('node:os');
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'identity-alert-selftest-'));
    try {
      const config = path.join(directory, 'input.json'), secret = 'SECRET_PRIVATE_ALERT_CANARY';
      fs.writeFileSync(config, secret, { mode: 0o600 });
      assert.throws(() => createAlertPipeline(config), failure => failure.message === 'IDENTITY_FIXED_ALERT_FAILED'
        && !failure.stack.includes(secret) && !failure.stack.includes(directory));
      assert.throws(() => input({ contractVersion: 'identity-fixed-alert-input/1', metricsOrigin: 'https://outside.invalid',
        caFile: 'x', metricsTokenFile: 'x', collectorOrigin: 'http://127.0.0.1:443', collectorTokenFile: 'x' }));
      fs.chmodSync(config, 0o644); assert.throws(() => readPrivate(config, 65536));
      fs.chmodSync(config, 0o600); const link = path.join(directory, 'link'); fs.symlinkSync(config, link);
      assert.throws(() => readPrivate(link, 65536));
      const token = path.join(directory, 'token'); fs.writeFileSync(token, `${secret}\n`, { mode: 0o600 });
      assert.equal(tokenFile(token), secret);
    } finally { fs.rmSync(directory, { recursive: true, force: true }); }
  });
  test('scraper uses private TLS default hostname verification, independent Bearer and fixed path', async () => {
    const { EventEmitter } = await import('node:events'); let agentOptions, requestOptions, closed = false;
    const implementation = { Agent: class {
      constructor(options) { agentOptions = options; }
      destroy() { closed = true; }
    }, request(options, receive) {
      requestOptions = options; const request = new EventEmitter(); request.destroy = () => {};
      request.end = () => queueMicrotask(() => {
        const response = new EventEmitter(); response.headers = { 'content-type': 'text/plain; version=0.0.4' };
        response.statusCode = 200; response.complete = true;
        receive(response); response.emit('data', Buffer.from(metrics())); response.emit('end');
      }); return request;
    } };
    const source = scraper({ metricsPort: 24444 }, Buffer.from('PUBLIC_TEST_CA'), 'METRICS_BEARER_SECRET_TEST', implementation);
    try {
      assert.equal((await source.scrape()).databaseUp, 1);
      assert.equal(agentOptions.rejectUnauthorized, true); assert.equal(agentOptions.maxTotalSockets, 1);
      assert.equal(Object.hasOwn(agentOptions, 'checkServerIdentity'), false);
      assert.equal(requestOptions.servername, 'auth.saas.test'); assert.equal(requestOptions.port, 24444);
      assert.equal(requestOptions.path, '/internal/identity/metrics'); assert.equal(requestOptions.method, 'GET');
      assert.equal(Object.hasOwn(requestOptions.headers, 'Cookie'), false);
      assert.equal(requestOptions.headers.Authorization, 'Bearer METRICS_BEARER_SECRET_TEST');
    } finally { source.close(); }
    assert.equal(closed, true);
  });
  test('parallel poll is rejected without queueing and lifetime closes both transports', async () => {
    let release, clock = 0, closed = 0;
    const pipeline = pipelineCore({ scrape: () => new Promise(resolve => { release = resolve; }), close: () => { closed += 1; } },
      { send: async () => true, close: () => { closed += 1; } }, { now: () => clock });
    try {
      const first = pipeline.poll(); assert.equal((await pipeline.poll()).status, 'BUSY');
      release(parseMetrics(metrics())); await first;
      clock = BUDGET.lifetimeMs; assert.equal((await pipeline.poll()).status, 'EXPIRED');
      assert.equal(closed, 2); assert.equal(pipeline.report().closed, true);
    } finally { pipeline.close(); }
  });
}
