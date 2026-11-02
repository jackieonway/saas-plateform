/** Bounded, task-owned real TLS/fault/capacity validation. Reports are secret-free. */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import https from 'node:https';
import { execFileSync, spawn } from 'node:child_process';
import { randomBytes, createHash } from 'node:crypto';
import { performance } from 'node:perf_hooks';
import { fileURLToPath } from 'node:url';
import { runLoad } from './phase22-load.mjs';
import { createAlertPipeline, startCollector } from './phase22-alerts.mjs';

const TASK = '/tmp/rust-saas-unified-auth-20261028';
function runtimeOptions(argv) {
  const [output, flag] = argv;
  assert(argv.length >= 1 && argv.length <= 2);
  assert(flag === undefined || flag === '--faults-only' || flag === '--phase25');
  const phase = flag === '--phase25' ? 25 : 22;
  const expected = phase === 25 ? 'phase25-capacity-runtime.json' : 'phase22-runtime.json';
  assert.equal(path.dirname(output), TASK);
  assert.equal(path.basename(output), expected);
  return { output, phase, faultsOnly: flag === '--faults-only' };
}
const { output, phase, faultsOnly } = runtimeOptions(process.argv.slice(2));
assert.equal(fs.lstatSync(output, { throwIfNoEntry: false }), undefined, 'FRESH_RUNTIME_OUTPUT_REQUIRED');
const tools = path.dirname(fileURLToPath(import.meta.url));
// The root-created binding checks inode, owner, mode and the task marker. Every
// subsequent SQL command also proves the actual cluster and data directory.
const binding = JSON.parse(execFileSync('python3', ['-B', path.join(tools, 'phase23_cluster_binding.py'),
  'show', '--task-dir', TASK], { encoding: 'utf8', timeout: 10000, stdio: ['ignore', 'pipe', 'pipe'] }));
const read = name => JSON.parse(fs.readFileSync(path.join(TASK, name), 'utf8'));
const env = read('https-environment.json'), manifest = read('https-database.json');
const fixture = read('phase22-fixture.json');
assert.equal(manifest.clusterId, binding.systemIdentifier);
assert(/^unified_auth_https_[0-9a-f]{32}$/.test(manifest.database));
assert.equal(manifest.rolePrefix, 'uah_' + manifest.database.slice(-12));
assert.equal(fixture.binding.clusterId, binding.systemIdentifier);
const ca = fs.readFileSync(env.identity.CONTEXT_PLATFORM_CA_FILE);
const agent = new https.Agent({ ca, maxSockets: 48, keepAlive: true, rejectUnauthorized: true,
  lookup: (_host, _options, callback) => callback(null, [{ address: '127.0.0.2', family: 4 }]) });
const AUTH = 'https://auth.saas.test', CONSOLE = 'https://console.saas.test';
const jar = new Map();
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const report = { contractVersion: 'unified-auth-phase22-real-runtime/1', date: new Date().toISOString(), result: 'FAILED',
  phase, freshCapacityRequired: phase === 25, clusterId: binding.systemIdentifier,
  tlsVerification: true, nativeIdentityInstances: 1, nativePlatformInstances: 1, dockerContainers: 0,
  checks: [], capacity: {}, q9ProductionTargetsConfirmed: false, thirtyMinuteSoakClaimed: false };
let stage = 'preflight', pipeline, collector, lock, platformPaused = false, dbDisabled = false;
const pass = details => { report.checks.push({ scenario: stage, result: 'PASS', ...details }); console.log(`PHASE22_CHECK ${stage} PASS`); };
async function request(target, options = {}) {
  const url = new URL(target, AUTH), origin = url.origin;
  assert(['auth.saas.test', 'console.saas.test'].includes(url.hostname));
  assert(['443', '24444', ''].includes(url.port));
  const headers = { Origin: url.hostname === 'console.saas.test' ? CONSOLE : AUTH, Accept: 'application/json', ...(options.jar === false || !jar.get(origin) ? {} : { Cookie: jar.get(origin) }), ...options.headers };
  let body = options.body;
  if (body && typeof body !== 'string') { body = JSON.stringify(body); headers['Content-Type'] = 'application/json'; }
  if (body !== undefined) headers['Content-Length'] = Buffer.byteLength(body);
  const started = performance.now();
  return new Promise((resolve, reject) => {
    let timer;
    const fail = () => { clearTimeout(timer); reject(new Error('RUNTIME_TRANSPORT_UNAVAILABLE')); };
    const req = https.request(url, { method: options.method ?? 'GET', headers, agent }, response => {
      const chunks = []; let size = 0;
      response.on('data', chunk => { size += chunk.length; if (size > 1024 * 1024) req.destroy(new Error('BOUNDED_RESPONSE')); else chunks.push(chunk); });
      response.on('end', () => {
        clearTimeout(timer);
        const cookies = response.headers['set-cookie'];
        if (cookies && options.jar !== false) {
          const current = new Map((jar.get(origin) ?? '').split('; ').filter(Boolean).map(v => v.split(/=(.*)/s).slice(0, 2)));
          for (const cookie of cookies) {
            const [name, value] = cookie.split(';', 1)[0].split(/=(.*)/s);
            const maxAge = /(?:^|;)\s*max-age=(-?\d+)(?:;|$)/i.exec(cookie);
            if (maxAge && Number(maxAge[1]) <= 0) current.delete(name);
            else current.set(name, value);
          }
          jar.set(origin, [...current].map(([k, v]) => `${k}=${v}`).join('; '));
        }
        const text = Buffer.concat(chunks).toString('utf8'); let body;
        try { body = JSON.parse(text); } catch { body = null; }
        resolve({ status: response.statusCode, headers: response.headers, body, text, durationMs: performance.now() - started });
      });
      response.once('aborted', fail); response.once('error', fail);
    });
    timer = setTimeout(() => req.destroy(new Error('BOUNDED_DEADLINE')), 10000);
    req.once('error', fail);
    req.end(body);
  });
}
const literal = value => `'${String(value).replaceAll("'", "''")}'`;
const database = `postgres://postgres@127.0.0.1:55494/${manifest.database}`;
const guard = `DO $$ BEGIN IF current_database()<>${literal(manifest.database)} OR (SELECT system_identifier::text FROM pg_control_system())<>${literal(binding.systemIdentifier)} OR current_setting('data_directory')<>${literal(binding.dataDirectory)} OR (SELECT shobj_description(oid,'pg_database') FROM pg_database WHERE datname=current_database()) IS DISTINCT FROM ${literal('identity-deployment/1/' + env.ids.deployment)} THEN RAISE EXCEPTION 'owned phase22 guard failed'; END IF; END $$;`;
function sql(statement) {
  try { return execFileSync('psql', ['-X', '-q', '-v', 'ON_ERROR_STOP=1', '-v', 'VERBOSITY=sqlstate', '-At', database], { input: guard + '\n' + statement, encoding: 'utf8', timeout: 15000, stdio: ['pipe', 'pipe', 'pipe'] }).trim(); }
  catch { throw new Error('OWNED_PHASE22_SQL_FAILED'); }
}
const runtimeRole = manifest.rolePrefix + '_identity_runtime';
assert(/^uah_[0-9a-f]{12}_identity_runtime$/.test(runtimeRole));
function roleLogin(enabled) {
  sql(`DO $$ BEGIN IF NOT EXISTS(SELECT 1 FROM identity_deployment_state WHERE singleton AND identity_runtime=${literal(runtimeRole)}::name AND deployment_id=${literal(env.ids.deployment)}::uuid) THEN RAISE EXCEPTION 'owned role guard failed'; END IF; END $$; ALTER ROLE "${runtimeRole}" ${enabled ? 'LOGIN' : 'NOLOGIN'};`);
  dbDisabled = !enabled;
  if (!enabled) sql(`SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=current_database() AND usename=${literal(runtimeRole)} AND pid<>pg_backend_pid();`);
}
function ownedProcess(service) {
  const record = read(`service-${service}.pid`), proc = `/proc/${record.pid}`;
  assert.equal(fs.readFileSync(`${proc}/stat`, 'utf8').split(')').at(-1).trim().split(/\s+/)[19], record.startTime);
  assert.equal(fs.readlinkSync(`${proc}/exe`), `${TASK}/bin/${service === 'identity' ? 'saas-identity' : 'saas-platform-backend'}`);
  assert.equal(fs.readlinkSync(`${proc}/ns/mnt`), record.namespace);
  return record.pid;
}
const noSourceClearing = response => assert(!response.headers['set-cookie'] && !response.headers['www-authenticate']);
async function until(check, code, milliseconds = 10000) {
  const end = Date.now() + milliseconds;
  while (!(await check())) { assert(Date.now() < end, code); await delay(50); }
}
async function login(account) {
  const pre = await request('/api/sso/preauth'); assert.equal(pre.status, 200);
  const result = await request('/api/sso/login', { method: 'POST', body: account, headers: { 'X-CSRF-Token': pre.body.csrfToken } });
  report.normalLogin = { status: result.status, durationMs: Math.round(result.durationMs) };
  assert.equal(result.status, 200, 'REAL_SSO_LOGIN_REQUIRED'); return result.body;
}
function basic(client) { return 'Basic ' + Buffer.from(`${encodeURIComponent(client.clientId)}:${encodeURIComponent(client.clientSecret)}`).toString('base64'); }
function save(name, value) { const target = path.join(TASK, name); fs.writeFileSync(target, JSON.stringify(value), { mode: 0o600 }); fs.chmodSync(target, 0o600); return target; }
function fixtureFile(key, fallback) { const candidate = fixture.files?.[key] ?? (typeof fixture[key] === 'string' ? fixture[key] : fallback); assert(typeof candidate === 'string' && candidate.startsWith(TASK + '/')); return candidate; }
let lastAlertPoll = 0;
async function pollAlert() {
  await delay(Math.max(0, 110 - (Date.now() - lastAlertPoll)));
  const result = await pipeline.poll(); lastAlertPoll = Date.now();
  assert.equal(result.status, 'OK'); return result;
}
function alertReceived(condition, state) {
  return collector.snapshot().retainedEvents.some(event => event.condition === condition && event.state === state);
}

try {
  stage = 'real-https-health-and-private-metrics';
  assert.equal((await request('/healthz')).status, 200);
  const metricsToken = env.identity.IDENTITY_METRICS_TOKEN;
  assert(metricsToken?.length >= 32);
  const metricHeaders = { Authorization: `Bearer ${metricsToken}` };
  assert.equal((await request('/internal/identity/metrics', { headers: metricHeaders, jar: false })).status, 404);
  assert.equal((await request(`${AUTH}:24444/internal/identity/metrics`, { jar: false })).status, 401);
  const metrics = await request(`${AUTH}:24444/internal/identity/metrics`, { headers: metricHeaders, jar: false });
  assert.equal(metrics.status, 200); assert(metrics.text.includes('identity_database_up 1'));
  assert.equal(metrics.headers['cache-control'], 'no-store'); pass({ publicStatus: 404, privateMissingToken: 401, privateAuthorized: 200 });

  stage = 'independent-local-alert-pipeline';
  const caFile = path.join(TASK, 'phase22-ca.pem'); fs.writeFileSync(caFile, ca, { mode: 0o600 });
  const metricsTokenFile = path.join(TASK, 'phase22-metrics-token'); fs.writeFileSync(metricsTokenFile, metricsToken, { mode: 0o600 });
  const collectorTokenFile = path.join(TASK, 'phase22-collector-token'); fs.writeFileSync(collectorTokenFile, randomBytes(32).toString('hex'), { mode: 0o600 });
  collector = await startCollector({ tokenFile: collectorTokenFile, port: 0 });
  pipeline = await createAlertPipeline(save('phase22-alert-input.json', { contractVersion: 'identity-fixed-alert-input/1', metricsOrigin: `${AUTH}:24444`, caFile, metricsTokenFile, collectorOrigin: collector.origin, collectorTokenFile }));
  await pollAlert(); pass({ implementation: 'bounded fixed-condition evaluator', promqlOrAlertmanagerClaimed: false });

  stage = 'bounded-successful-code-refresh-introspection-capacity';
  if (faultsOnly) {
    const baseline = read('phase22-capacity-baseline.json');
    assert.equal(baseline.result, 'PASS');
    assert.equal(baseline.identityBinarySha256, createHash('sha256').update(fs.readFileSync(path.join(TASK, 'bin/saas-identity'))).digest('hex'));
    for (const kind of ['CODE', 'REFRESH', 'INTROSPECTION']) assert.equal(baseline.capacity[kind].completion.requestedDurationCompleted, true);
    report.capacity = baseline.capacity; report.serverCapacityObservation = baseline.serverCapacityObservation;
    report.capacityEvidence = { reusedCompletedComponentEvidence: true, sourceRuntimeSha256: baseline.sourceRuntimeSha256,
      date: baseline.date, sameIdentityBinaryVerified: true, separateOwnedFixture: true };
    pass({ previouslyCompletedComponentEvidence: true, newLoadClaimed: false });
  } else {
  // Codes are freshly prepared only after the running instance is healthy.
  execFileSync('python3', ['-B', path.join(tools, 'phase22-fixture.py'), '--task-dir', TASK, '--admin-url', 'postgres://postgres@127.0.0.1:55494/postgres', '--stage', 'codes'], { timeout: 45000, stdio: 'ignore' });
  const userClient = fixtureFile('userClient', `${TASK}/phase22-user-client.json`);
  const machineClient = fixtureFile('machineClient', `${TASK}/phase22-machine-client.json`);
  const machine = JSON.parse(fs.readFileSync(machineClient, 'utf8'));
  const mint = await request('/oauth/token', { method: 'POST', jar: false, headers: { Authorization: basic(machine), 'Content-Type': 'application/x-www-form-urlencoded' }, body: new URLSearchParams({ grant_type: 'client_credentials', scope: fixture.machineScope ?? 'phase22.read' }).toString() });
  assert.equal(mint.status, 200); assert(mint.body.access_token && !mint.body.id_token && !mint.body.refresh_token);
  const introspectionSource = save('phase22-live-introspection-source.json', { tokens: [mint.body.access_token] });
  const sources = { CODE: fixtureFile('codes', `${TASK}/phase22-code-sources.json`), REFRESH: fixtureFile('refresh', `${TASK}/phase22-refresh-sources.json`), INTROSPECTION: introspectionSource };
  const loads = [['CODE',25,8,30,userClient],['REFRESH',25,8,120,userClient],['INTROSPECTION',200,16,120,machineClient]];
  const pid = ownedProcess('identity'), cpuTicks = Number(execFileSync('getconf', ['CLK_TCK'], { encoding: 'utf8' }).trim());
  const sample = () => {
    const fields = fs.readFileSync(`/proc/${pid}/stat`, 'utf8').split(')').at(-1).trim().split(/\s+/);
    const memory = fs.readFileSync(`/proc/${pid}/statm`, 'utf8').trim().split(/\s+/);
    return { ticks: Number(fields[11]) + Number(fields[12]), rss: Number(memory[1]) * 4096 };
  };
  const began = performance.now(), initial = sample(); let peakRss = initial.rss, samples = 0, scrapeFailures = 0, poolIdleMinimum = 4, poolSizeMaximum = 0, sampling = false;
  const sampler = setInterval(async () => {
    if (sampling) return; sampling = true;
    try {
      const result = await request(`${AUTH}:24444/internal/identity/metrics`, { headers: metricHeaders, jar: false });
      if (result.status !== 200) { scrapeFailures++; return; }
      poolIdleMinimum = Math.min(poolIdleMinimum, Number(/^identity_db_pool_idle (\d+)$/m.exec(result.text)?.[1] ?? 0));
      poolSizeMaximum = Math.max(poolSizeMaximum, Number(/^identity_db_pool_size (\d+)$/m.exec(result.text)?.[1] ?? 0));
      peakRss = Math.max(peakRss, sample().rss); samples++;
    } catch { scrapeFailures++; } finally { sampling = false; }
  }, 1000);
  let loadResults;
  try { loadResults = await Promise.allSettled(loads.map(async ([kind, ratePerSecond, concurrency, durationSeconds, clientFile]) => {
    const reportPath = path.join(TASK, `phase22-capacity-${kind.toLowerCase()}.json`);
    await runLoad(save(`phase22-load-${kind.toLowerCase()}.json`, { contractVersion: 'identity-capacity-input/1', kind, origins: [AUTH], ratePerSecond, concurrency, durationSeconds, requestTimeoutMs: 10000, clientFile, caFile, sourceFile: sources[kind] }), reportPath);
    report.capacity[kind] = JSON.parse(fs.readFileSync(reportPath, 'utf8'));
    assert(report.capacity[kind].counts.effectiveSuccess > 0);
    assert.equal(report.capacity[kind].completion.requestedDurationCompleted, true);
  })); } finally { clearInterval(sampler); }
  await until(async () => !sampling, 'CAPACITY_SAMPLER_DID_NOT_DRAIN');
  const final = sample(), elapsed = performance.now() - began;
  report.serverCapacityObservation = { samples, scrapeFailures, identityCpuOneCorePercent: ((final.ticks-initial.ticks)/cpuTicks)/(elapsed/1000)*100,
    identityRssStartBytes: initial.rss, identityRssSampledPeakBytes: peakRss, databasePoolSizeMaximum: poolSizeMaximum, databasePoolIdleMinimum: poolIdleMinimum,
    boundedConstraints: { databasePool: 4, authRequests: 32, identityTcp: 64, totalLoadWorkers: 32 },
    buildProfile: 'dev', singleSharedDriverProcess: true, stableProductionBottleneckEstablished: false };
  for (const result of loadResults) assert.equal(result.status, 'fulfilled');
  pass({ requestedDurationSeconds: { CODE: 30, REFRESH: 120, INTROSPECTION: 120 }, machineTokenObtainedThroughRealTokenEndpoint: true });
  }

  stage = 'canonical-database-outage-and-recovery';
  const sso = await login(env.accounts.employee);
  const before = jar.get(AUTH);
  try {
    roleLogin(false);
    const session = await request('/api/sso/session'); assert.equal(session.status, 503); noSourceClearing(session);
    assert.equal(jar.get(AUTH), before);
    assert.equal((await request('/healthz')).status, 200);
    assert.equal((await request('/ready')).status, 503);
    await pollAlert(); await pollAlert();
    assert(alertReceived('DATABASE_UNAVAILABLE', 'FIRING'));
    report.databaseOutage = { sessionStatus: session.status, livenessStatus: 200, readinessStatus: 503 };
  } finally { roleLogin(true); }
  await until(async () => (await request('/api/sso/session')).status === 200, 'DATABASE_RECOVERY_FAILED');
  assert.equal(jar.get(AUTH), before); await pollAlert();
  assert(alertReceived('DATABASE_UNAVAILABLE', 'RESOLVED'));
  pass({ originalCookiePreserved: true, originalAuthorityRecovered: true });

  stage = 'private-platform-response-timeout-and-recovery';
  // A genuine console login registers the canonical context transaction.
  const begin = await request(`${CONSOLE}/auth/login?return=/`);
  assert([302,303].includes(begin.status));
  const authorization = await request(begin.headers.location);
  assert([302,303].includes(authorization.status));
  const flow = new URL(authorization.headers.location, AUTH).searchParams.get('flow'); assert(flow);
  const interaction = `/oauth/interaction/${flow}`;
  let state = await request(interaction); assert.equal(state.status, 200);
  if (state.body.status === 'CONSENT_REQUIRED') {
    const consent = await request(interaction, { method: 'POST', headers: { 'X-CSRF-Token': state.body.csrfToken }, body: { action: 'consent' } }); assert.equal(consent.status, 200);
  }
  const platformPid = ownedProcess('platform');
  try {
    process.kill(platformPid, 'SIGSTOP'); platformPaused = true;
    const failed = await request(interaction); assert.equal(failed.status, 503); noSourceClearing(failed);
    assert(failed.durationMs >= 2500 && failed.durationMs < 5000);
    report.privateCallTimeoutMs = Math.round(failed.durationMs);
  } finally { process.kill(platformPid, 'SIGCONT'); platformPaused = false; }
  let recoveredContext;
  await until(async () => { recoveredContext = await request(interaction); return recoveredContext.status === 200; }, 'PRIVATE_PLATFORM_RECOVERY_FAILED');
  assert.equal(recoveredContext.body.status, 'CONTEXT_SELECTION_REQUIRED');
  assert(recoveredContext.body.contextSelection.members.some(member => member.membershipId === env.ids.employeeMember));
  pass({ fixedClientDeadlineSeconds: 3, originalFlowRecovered: true, realIdentityAndPlatform: true });

  stage = 'identity-unavailable-and-recovery-with-original-cookie';
  const beforeIdentity = jar.get(AUTH);
  process.kill(ownedProcess('identity'), 'SIGTERM');
  await until(async () => !fs.existsSync(path.join(TASK, 'service-identity.pid')), 'OWNED_IDENTITY_DID_NOT_STOP');
  const unavailable = await request('/api/sso/session');
  assert.equal(unavailable.status, 502); noSourceClearing(unavailable);
  assert.equal(jar.get(AUTH), beforeIdentity);
  const pressureEnvironment = read('https-environment.json');
  pressureEnvironment.identity.PASSWORD_EXECUTION_TIMEOUT_MS = '100';
  pressureEnvironment.identity.PASSWORD_QUEUE_TIMEOUT_MS = '500';
  fs.writeFileSync(path.join(TASK, 'https-environment.json'), JSON.stringify(pressureEnvironment), { mode: 0o600 });
  report.passwordPressureProfile = { normalExecutionDeadlineMs: 10000, pressureExecutionDeadlineMs: 100,
    pressureQueueDeadlineMs: 500, changedAtAlreadyRequiredOwnedRestart: true, productionConfigurationChanged: false };
  const restarted = spawn('python3', ['-B', path.join(tools, 'run-service.py'), '--task-dir', TASK, 'identity'], { stdio: 'ignore' });
  restarted.unref();
  await until(async () => (await request('/api/sso/session')).status === 200, 'IDENTITY_RECOVERY_FAILED');
  assert.equal(jar.get(AUTH), beforeIdentity);
  pass({ publicGatewayStatus: 502, application503ClaimedForConnectionRefusal: false, originalCookiePreserved: true });

  stage = 'password-admission-queue-deadline-and-recovery';
  const probe = fixture.passwordProbe; assert(probe?.username && probe?.password);
  const challenges = await Promise.all(Array.from({ length: 8 }, async () => {
    const r = await request('/api/sso/preauth', { jar: false }); assert.equal(r.status, 200);
    return { csrf: r.body.csrfToken, cookie: r.headers['set-cookie'][0].split(';')[0] };
  }));
  const pending = challenges.map(c => request('/api/sso/login', { method: 'POST', jar: false, headers: { Cookie: c.cookie, 'X-CSRF-Token': c.csrf }, body: { username: probe.username, password: probe.password } }));
  await delay(20);
  for (let i = 0; i < 3; i++) { await pollAlert(); }
  const responses = await Promise.all(pending);
  assert(responses.some(r => r.status === 429));
  for (const r of responses.filter(r => r.status === 429)) { assert.equal(r.headers['retry-after'], '1'); noSourceClearing(r); }
  await delay(1000); await pollAlert();
  assert(alertReceived('PASSWORD_QUEUE_PRESSURE', 'FIRING'));
  assert(alertReceived('PASSWORD_QUEUE_PRESSURE', 'RESOLVED'));
  pass({ requests: 8, statuses: responses.reduce((acc, r) => ({ ...acc, [r.status]: (acc[r.status] ?? 0) + 1 }), {}), permitHoldingVerifiedByComponentTests: true });

  stage = 'terminal-failure-fires-and-is-acknowledged-without-reset';
  execFileSync('python3', ['-B', path.join(tools, 'phase22-fixture.py'), '--task-dir', TASK, '--admin-url', 'postgres://postgres@127.0.0.1:55494/postgres', '--stage', 'terminal'], { timeout: 15000, stdio: 'ignore' });
  await pollAlert(); await pollAlert();
  assert.equal((await pipeline.acknowledgeTerminal()).accepted, true); await pollAlert();
  assert(alertReceived('NEW_TERMINAL_DELIVERY', 'FIRING'));
  assert(alertReceived('NEW_TERMINAL_DELIVERY', 'ACKNOWLEDGED'));
  assert(!alertReceived('NEW_TERMINAL_DELIVERY', 'RESOLVED'));
  report.alerts = pipeline.report(); report.collector = collector.snapshot();
  assert(sql("SELECT count(*) FROM auth_delivery_jobs WHERE state='TERMINAL'") !== '0');
  pass({ terminalFactSource: 'explicit owned canonical fixture', terminalRowsRetained: true });
  report.result = 'PASS';
} catch (error) {
  report.failure = { scenario: stage, code: error instanceof assert.AssertionError ? 'RUNTIME_ASSERTION_FAILED' : 'RUNTIME_VALIDATION_FAILED' };
  const location = /phase22-runtime\.mjs:(\d+):(\d+)/.exec(error?.stack ?? '');
  if (location) report.failure.driverLine = Number(location[1]);
  console.error(`PHASE22_CHECK ${stage} FAIL`);
} finally {
  if (platformPaused) { try { process.kill(ownedProcess('platform'), 'SIGCONT'); } catch {} }
  if (dbDisabled) { try { roleLogin(true); } catch { report.cleanupFault = 'OWNED_ROLE_LOGIN_RESTORE_FAILED'; } }
  lock?.kill('SIGTERM');
  if (pipeline) { report.alerts = pipeline.report(); await pipeline.close(); }
  if (collector) { report.collector = collector.snapshot(); await collector.close(); }
  agent.destroy();
  const reportFd = fs.openSync(output, 'wx', 0o600);
  try { fs.writeFileSync(reportFd, JSON.stringify(report, null, 2)); }
  finally { fs.closeSync(reportFd); }
}
if (report.result !== 'PASS') process.exitCode = 1;
