/** Injected Phase 23 key-case orchestration; importing this file starts nothing.
 *
 * runKeyCases(ports) owns no sockets, process, database, file or wall clock.
 * The owner wires every port to its frozen, real environment:
 *   keyCommand({method,path,body?,operation?}) -> {status,body,operation}
 *     Paths are platform /api/identity/keys/*; operations are owner-saved UUIDv7.
 *   lookupOperation(operation) -> {status,body:{operationId,state,result}}
 *   authorize(profile='third') -> protocol finish result {key,kid}; on failure
 *     returns {status} or throws an error with a numeric safe status.
 *   rpc(command,params) -> unwrapped safe protocol result; rejects on failure.
 *   jwksKids() -> current authoritative public kid array (never cached).
 *   keyWindow(kid) -> {active,retired,revoked,lastSignedExp,dbNowEpoch}
 *     Timestamps are finite epoch seconds from the authority database.
 *   profile(ttl) -> resolves only after the actual owned service restart.
 *   signLogoutWithCollector() -> {kid,delivered:true}, using a separate grant.
 *   wait(ms) -> actual owner wait; checkpointProgress(fixedData) -> persisted.
 *
 * Return {cases,privateCarriers}; only cases may enter a formal report.
 * The private carrier handles stay in the same retained protocol process.
 * C is deliberately still published/nonactive at return; post-checkpoint K07
 * belongs to the owner, including C revoke and original-token verification.
 * Pure mock-port tests below prove orchestration, never actual elapsed time,
 * HTTPS, service restart, receipt cardinality, SQL or key cryptography.
 */

import { safeProtocolCause } from './phase23-protocol-diagnostics.mjs';

const KIDS = Object.freeze({ a: 'owned-https', b: 'phase23-b', c: 'phase23-c', d: 'phase23-d' });
const FUNCTIONS = ['keyCommand', 'lookupOperation', 'authorize', 'rpc', 'jwksKids', 'keyWindow',
  'profile', 'signLogoutWithCollector', 'refreshAdminMfa', 'wait', 'checkpointProgress'];
const MAX_POLLS = 120, MAX_REQUESTED_WAIT_MS = 900000, MAX_WAIT_MS = 10000;
const PREPUBLICATION_SECONDS = 61;
const record = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const operationId = value => typeof value === 'string'
  && /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value);
const handle = value => typeof value === 'string' && /^[A-Za-z0-9_-]{1,160}$/.test(value);

export class KeyCaseFailure extends Error {
  constructor(code) { super(code); this.name = 'KeyCaseFailure'; this.code = code; }
}
function need(condition, code) { if (!condition) throw new KeyCaseFailure(code); }
async function call(action, code) {
  try { return await action(); } catch (error) {
    const failure = new KeyCaseFailure(code), cause = safeProtocolCause(error?.protocolCause);
    if (cause) failure.protocolCause = cause;
    throw failure;
  }
}

export async function runKeyCases(ports) {
  need(record(ports) && FUNCTIONS.every(name => typeof ports[name] === 'function'), 'KEY_PORTS_INVALID');
  const cases = [], carriers = {};
  const progress = data => call(() => ports.checkpointProgress(structuredClone(data)), 'KEY_PROGRESS_FAILED');
  const finishCase = async (caseId, evidence, status = 'PASS') => {
    const value = { caseId, status, evidence };
    await progress(value); cases.push(value);
  };
  async function command(method, suffix, body, expectedStatus, expectedState, kid) {
    const input = { method, path: '/api/identity/keys/' + suffix };
    if (body !== undefined) input.body = body;
    const response = await call(() => ports.keyCommand(input), 'KEY_COMMAND_FAILED');
    need(record(response) && response.status === expectedStatus && operationId(response.operation), 'KEY_COMMAND_REJECTED');
    if (expectedState) need(record(response.body) && response.body.kid === kid
      && response.body.status === expectedState, 'KEY_COMMAND_RESULT_MISMATCH');
    return response;
  }
  const publish = kid => command('POST', 'publish', { kid }, 200, 'PUBLISHED', kid);
  const activate = kid => command('POST', 'activate', { kid }, 200, 'ACTIVE', kid);
  const retire = (kid, status) => command('DELETE', kid, undefined, status, status === 200 ? 'RETIRED' : undefined, kid);
  const revoke = kid => command('POST', kid + '/revoke', undefined, 200, 'REVOKED', kid);
  async function authorize(kid) {
    const value = await call(() => ports.authorize('third'), 'KEY_AUTHORIZATION_FAILED');
    need(record(value) && handle(value.key) && value.kid === kid, 'KEY_SIGNER_MISMATCH');
    return value.key;
  }
  async function keys(expected) {
    const value = await call(() => ports.jwksKids(), 'KEY_JWKS_FAILED');
    need(Array.isArray(value) && value.length === expected.length && new Set(value).size === value.length
      && expected.every(kid => value.includes(kid)), 'KEY_JWKS_MISMATCH');
  }
  async function window(kid) {
    const value = await call(() => ports.keyWindow(kid), 'KEY_WINDOW_FAILED');
    need(record(value) && ['active', 'retired', 'revoked'].every(name => typeof value[name] === 'boolean')
      && Number.isFinite(value.dbNowEpoch) && value.dbNowEpoch > 0
      && (value.lastSignedExp === null || (Number.isFinite(value.lastSignedExp) && value.lastSignedExp > 0)), 'KEY_WINDOW_INVALID');
    return value;
  }
  async function onlyActive(known, active) {
    for (const kid of known) {
      const value = await window(kid);
      need(value.active === (kid === active) && (kid !== active || (!value.retired && !value.revoked)), 'KEY_ACTIVE_SET_MISMATCH');
    }
  }
  async function same(leftKey, rightKey) {
    const result = await call(() => ports.rpc('same-subject', { leftKey, rightKey }), 'KEY_SUBJECT_COMPARISON_FAILED');
    need(record(result) && result.matches === true, 'KEY_SUBJECT_CHANGED');
  }
  async function cache() {
    const value = await call(() => ports.rpc('jwks-cache', { profile: 'third' }), 'KEY_SDK_CACHE_OBSERVATION_FAILED');
    need(record(value) && value.observed === true && Array.isArray(value.cachedKids)
      && value.cachedKids.every(kid => Object.values(KIDS).includes(kid))
      && Number.isInteger(value.cacheAgeSeconds) && value.cacheAgeSeconds >= 0
      && Number.isInteger(value.cacheUpdatedAtEpoch) && value.cacheUpdatedAtEpoch > 0
      && value.unknownKidRefetchAfterSeconds === 60 && value.cacheMutationRequested === false,
    'KEY_SDK_CACHE_OBSERVATION_INVALID');
    return value;
  }
  async function prepublicationWindow(kid, caseId) {
    const start = await window(kid);let latest = start, sdk = await cache(), polls = 0, requestedWaitMs = 0;
    need(!start.active && !start.retired && !start.revoked, 'KEY_PREPUBLICATION_STATE_INVALID');
    while (latest.dbNowEpoch - start.dbNowEpoch < PREPUBLICATION_SECONDS
      || (!sdk.cachedKids.includes(kid) && sdk.cacheAgeSeconds < PREPUBLICATION_SECONDS)) {
      need(polls < MAX_POLLS, 'KEY_PREPUBLICATION_POLL_LIMIT');
      const seconds = Math.max(PREPUBLICATION_SECONDS - (latest.dbNowEpoch - start.dbNowEpoch),
        sdk.cachedKids.includes(kid) ? 0 : PREPUBLICATION_SECONDS - sdk.cacheAgeSeconds);
      const milliseconds = Math.min(MAX_WAIT_MS, Math.max(1, Math.ceil(seconds * 1000)));
      need(requestedWaitMs + milliseconds <= MAX_REQUESTED_WAIT_MS, 'KEY_PREPUBLICATION_WAIT_BUDGET');
      await progress({ caseId, status: 'WAITING', kid, poll: polls + 1, waitMs: milliseconds,
        observedPublishSeconds: latest.dbNowEpoch-start.dbNowEpoch, observedSdkCacheAgeSeconds: sdk.cacheAgeSeconds });
      await call(() => ports.wait(milliseconds), 'KEY_PREPUBLICATION_WAIT_FAILED');
      requestedWaitMs += milliseconds;polls += 1;
      const observed = await window(kid);
      need(!observed.active && !observed.retired && !observed.revoked
        && observed.dbNowEpoch >= latest.dbNowEpoch, 'KEY_PREPUBLICATION_STATE_OR_CLOCK_CHANGED');
      latest = observed;sdk = await cache();
    }
    return { publishedKid: kid, minimumPublishSeconds: PREPUBLICATION_SECONDS,
      observedPublishStartDbEpoch: start.dbNowEpoch, observedGateDbEpoch: latest.dbNowEpoch,
      observedPublishSeconds: latest.dbNowEpoch-start.dbNowEpoch,
      sdkCacheAgeBeforeActivationSeconds: sdk.cacheAgeSeconds, sdkCacheUpdatedAtEpoch: sdk.cacheUpdatedAtEpoch,
      sdkCachedKidsBeforeActivation: sdk.cachedKids, unknownKidRefetchAfterSeconds: 60,
      cacheMutationRequested: false, sameSdkProcessRetained: true, requestedWaitMs, polls };
  }

  carriers.a = await authorize(KIDS.a);
  await retire(KIDS.a, 409);
  await command('POST', 'activate', { kid: KIDS.b }, 409);
  await publish(KIDS.b);
  await keys([KIDS.a, KIDS.b]);
  const bPrepublication = await prepublicationWindow(KIDS.b, 'K02_PREPUBLISH_CACHE_WINDOW');
  // Prepublication consumes part of the warm-up A token's real lifetime.
  // Retain an actual new A token before proving A/B coexistence.
  carriers.a = await authorize(KIDS.a);
  await finishCase('K02_PREPUBLISH', { unpublishedActivationStatus: 409, publishedStatus: 200,
    jwksAAndB: true, signingKid: KIDS.a, freshACarrierAfterPublishWait: true, prepublication: bPrepublication });

  // Intentionally discard the complete activation response except its original
  // operation. This simulates a lost application reply, not a network loss.
  const activation = await call(() => ports.keyCommand({ method: 'POST', path: '/api/identity/keys/activate',
    body: { kid: KIDS.b } }), 'KEY_ACTIVATION_DISPATCH_FAILED');
  need(record(activation) && operationId(activation.operation), 'KEY_ORIGINAL_OPERATION_MISSING');
  const original = activation.operation;
  const recovered = await call(() => ports.lookupOperation(original), 'KEY_ORIGINAL_LOOKUP_FAILED');
  need(record(recovered) && recovered.status === 200 && record(recovered.body)
    && recovered.body.operationId === original && recovered.body.state === 'COMPLETED'
    && record(recovered.body.result) && recovered.body.result.kid === KIDS.b
    && recovered.body.result.status === 'ACTIVE', 'KEY_ORIGINAL_RESULT_MISMATCH');
  await onlyActive([KIDS.a, KIDS.b], KIDS.b);
  carriers.b = await authorize(KIDS.b);
  await same(carriers.a, carriers.b);
  const oldA = await call(() => ports.rpc('verify-old-id-session', { key: carriers.a }), 'KEY_OLD_A_VERIFICATION_FAILED');
  need(record(oldA) && oldA.kid === KIDS.a && oldA.cachedKeySignatureValid === true
    && oldA.currentAuthorityJwtValid === true && oldA.onlineStatus === 200, 'KEY_OLD_A_NOT_LIVE');
  const logout = await call(() => ports.signLogoutWithCollector(), 'KEY_LOGOUT_DELIVERY_FAILED');
  need(record(logout) && logout.kid === KIDS.b && logout.delivered === true, 'KEY_LOGOUT_SIGNER_MISMATCH');
  await finishCase('K03_ACTIVE_ORIGINAL_RESULT', { applicationReplyIgnored: true, originalOperationCompleted: true,
    uniqueActiveKid: KIDS.b, idAndLogoutSigningKid: KIDS.b, oldAStillValid: true, stableSubject: true });

  const initial = await window(KIDS.a);
  need(!initial.active && !initial.retired && !initial.revoked && initial.lastSignedExp !== null, 'K06_SIGNED_HIGHWATER_REQUIRED');
  const retainUntil = initial.lastSignedExp + 60;
  need(initial.dbNowEpoch <= retainUntil, 'K06_LIVE_WINDOW_NOT_OBSERVED');
  await retire(KIDS.a, 409);
  await finishCase('K05_LIVE_WINDOW', { activeRetirementStatus: 409, liveWindowRetirementStatus: 409,
    highwaterObserved: true });
  let latest = initial, polls = 0, requestedWaitMs = 0;
  while (latest.dbNowEpoch <= retainUntil) {
    need(polls < MAX_POLLS, 'K06_WAIT_POLL_LIMIT');
    const milliseconds = Math.min(MAX_WAIT_MS, Math.max(1, Math.ceil((retainUntil - latest.dbNowEpoch) * 1000) + 1));
    need(requestedWaitMs + milliseconds <= MAX_REQUESTED_WAIT_MS, 'K06_WAIT_BUDGET');
    await progress({ caseId: 'K06_NORMAL_RETIREMENT', status: 'WAITING', poll: polls + 1, waitMs: milliseconds });
    await call(() => ports.wait(milliseconds), 'K06_WAIT_FAILED');
    requestedWaitMs += milliseconds; polls += 1;
    const observed = await window(KIDS.a);
    need(!observed.active && !observed.retired && !observed.revoked
      && observed.lastSignedExp === initial.lastSignedExp, 'K06_HIGHWATER_OR_LIFECYCLE_CHANGED');
    need(observed.dbNowEpoch >= latest.dbNowEpoch, 'K06_DATABASE_CLOCK_REVERSED');
    latest = observed;
  }
  await retire(KIDS.a, 200);
  const retired = await window(KIDS.a);
  need(!retired.active && retired.retired && !retired.revoked
    && retired.lastSignedExp === initial.lastSignedExp, 'K06_RETIRED_STATE_MISMATCH');
  await keys([KIDS.b]);
  await authorize(KIDS.b);
  await finishCase('K06_NORMAL_RETIREMENT', { retiredKid: KIDS.a, signingKid: KIDS.b,
    lastSignedExpEpoch: initial.lastSignedExp, retainUntilEpoch: retainUntil,
    observedDbStartEpoch: initial.dbNowEpoch, observedDbEndEpoch: latest.dbNowEpoch,
    polls, requestedWaitMs, retirementStatus: 200, oldKeyAbsentFromJwks: true });

  await command('POST', 'activate', { kid: KIDS.c }, 409);
  await publish(KIDS.c);await publish(KIDS.d);
  await keys([KIDS.b, KIDS.c, KIDS.d]);
  const dPrepublication = await prepublicationWindow(KIDS.d, 'K08_RECOVERY_PREPUBLISH_CACHE_WINDOW');
  const mfa = await call(() => ports.refreshAdminMfa(), 'KEY_SIGNER_GAP_REAL_STEPUP_FAILED');
  need(record(mfa) && mfa.realPasswordTotp === true && mfa.currentBrowserSourceVerified === true,
    'KEY_SIGNER_GAP_FRESH_ADMIN_SOURCE_REQUIRED');
  await revoke(KIDS.b);
  await keys([KIDS.c, KIDS.d]);
  await onlyActive(Object.values(KIDS), null);
  let denied;
  try { denied = await ports.authorize('third'); }
  catch (error) { denied = { status: error?.status }; }
  need(record(denied) && denied.status === 503 && denied.key === undefined, 'K08_ID_ISSUANCE_DID_NOT_FAIL_CLOSED');
  await activate(KIDS.d);
  await keys([KIDS.c, KIDS.d]);
  await onlyActive(Object.values(KIDS), KIDS.d);
  await authorize(KIDS.d);
  const recoveryCache = await cache();
  need(recoveryCache.cachedKids.includes(KIDS.c) && recoveryCache.cachedKids.includes(KIDS.d),
    'KEY_C_D_NOT_AVAILABLE_IN_STANDARD_SDK_CACHE');
  await finishCase('K08_NO_ACTIVE_SIGNER', { unpublishedActivationStatus: 409, revokedActiveKid: KIDS.b,
    userIdIssuanceStatus: 503, recoverySigningKid: KIDS.d, publishedKeysDuringSignerGap: [KIDS.c, KIDS.d],
    realStepUpBeforeSignerGap: true, prepublication: dPrepublication,
    sdkCachedKidsAfterRecovery: recoveryCache.cachedKids });

  // This is a real restart with the requested legacy configuration value.
  // Canonical Access/ID deadlines remain capped at 300s and source limits;
  // recovery must measure their actual expiresAt instead of inferring 600s.
  await call(() => ports.profile(600), 'KEY_LONG_PROFILE_RESTART_FAILED');
  await activate(KIDS.c);
  carriers.c = await authorize(KIDS.c);
  await activate(KIDS.d);
  carriers.d = await authorize(KIDS.d);
  await same(carriers.c, carriers.d);
  await same(carriers.a, carriers.d);
  await keys([KIDS.c, KIDS.d]);
  await onlyActive(Object.values(KIDS), KIDS.d);
  const retainedC = await window(KIDS.c);
  need(!retainedC.active && !retainedC.retired && !retainedC.revoked
    && retainedC.lastSignedExp !== null && retainedC.lastSignedExp > retainedC.dbNowEpoch, 'KEY_C_LIVE_CARRIER_REQUIRED');
  await finishCase('K07_CHECKPOINT_CARRIERS_READY', { statusScope: 'PRECONDITION_ONLY', requestedLegacyOidcTokenTtlSeconds: 600,
    legacySettingAppliedToCanonicalTokens: false, carrierDeadlinesRequireActualObservation: true,
    publishedNonactiveKid: KIDS.c, activeKid: KIDS.d, stableSubject: true, revocationDeferredUntilAfterCheckpoint: true }, 'READY');
  return { cases, privateCarriers: carriers };
}

if (process.env.NODE_TEST_CONTEXT) {
  const { test } = await import('node:test');
  const assert = (await import('node:assert/strict')).default;
  const CANARY = 'PRIVATE_KEY_CASE_CANARY';
  function fixture(options = {}) {
    let now = 1000, sequence = 0, count = 0, lookupCount = 0, active = KIDS.a;
    const canonicalIdTtlSeconds = 300;
    let sdkCache, retirementWaits = 0;
    const state = new Map([[KIDS.a, { active: true, retired: false, revoked: false, lastSignedExp: null }]]);
    const operations = new Map(), calls = [], waits = [], progress = [], handles = new Map();
    const id = () => `00000000-0000-7000-8000-${String(++sequence).padStart(12, '0')}`;
    const ports = {
      keyCommand: async request => {
        calls.push(['command', { ...request }]);
        const operation = id(), kid = request.body?.kid ?? request.path.split('/')[4];
        const row = state.get(kid);
        let status = 200, result;
        if (request.path.endsWith('/publish')) {
          if (row) status = 409;
          else { state.set(kid, { active: false, retired: false, revoked: false, lastSignedExp: null }); result = { kid, status: 'PUBLISHED' }; }
        } else if (request.path.endsWith('/activate')) {
          if (!row || row.retired || row.revoked) status = 409;
          else { for (const value of state.values()) value.active = false; row.active = true; active = kid; result = { kid, status: 'ACTIVE' }; }
        } else if (request.path.endsWith('/revoke')) {
          row.active = false; row.revoked = true; active = null; result = { kid, status: 'REVOKED' };
        } else if (request.method === 'DELETE') {
          if (row.active || (row.lastSignedExp !== null && now <= row.lastSignedExp + 60)) status = 409;
          else { row.retired = true; result = { kid, status: 'RETIRED' }; }
        }
        if (result) operations.set(operation, { ...result });
        if (options.poisonActivationReply && result?.status === 'ACTIVE' && kid === KIDS.b)
          return { operation, status: 599, body: { raw: CANARY } };
        return { operation, status, body: result ?? { error: CANARY } };
      },
      lookupOperation: async operation => {
        lookupCount += 1; calls.push(['lookup', operation]);
        return { status: 200, body: { operationId: options.wrongOperation ? id() : operation,
          state: options.notCompleted ? 'NOT_APPLIED' : 'COMPLETED', result: operations.get(operation) } };
      },
      authorize: async profile => {
        calls.push(['authorize', profile]);
        if (!active) {
          if (options.wrongDenial) return { status: 400 };
          if (options.throwDenial) throw Object.assign(new Error(CANARY), { status: 503 });
          return { status: 503 };
        }
        const age = sdkCache ? Math.floor(now) - sdkCache.uat : 0;
        if (!sdkCache || age >= 300 || (!sdkCache.kids.includes(active) && age >= 60)) {
          sdkCache = { uat: Math.floor(now), kids: [...state].filter(([, row]) => !row.retired && !row.revoked).map(([kid]) => kid) };
        }
        if (!sdkCache.kids.includes(active)) throw new Error('FIXTURE_STANDARD_SDK_UNKNOWN_KID');
        const row = state.get(active); row.lastSignedExp = Math.max(row.lastSignedExp ?? 0, now + canonicalIdTtlSeconds);
        const key = 'retained-' + ++count; handles.set(key, { kid: active, issuedAt: now });
        return { key, kid: active };
      },
      rpc: async (command, params) => {
        calls.push(['rpc', command, params]);
        if (options.rpcThrows && command !== 'jwks-cache') throw new Error(CANARY);
        if (command === 'jwks-cache') return { observed: true, cachedKids: [...sdkCache.kids],
          cacheUpdatedAtEpoch: sdkCache.uat, cacheAgeSeconds: Math.floor(now)-sdkCache.uat,
          unknownKidRefetchAfterSeconds: 60, cacheMutationRequested: false };
        if (command === 'same-subject') return { matches: !options.subjectChanges };
        return { kid: handles.get(params.key).kid, cachedKeySignatureValid: true,
          currentAuthorityJwtValid: !options.oldExpired, onlineStatus: 200 };
      },
      jwksKids: async () => [...state].filter(([, row]) => !row.retired && !row.revoked).map(([kid]) => kid),
      keyWindow: async kid => {
        const row = state.get(kid);
        const result = { ...row, dbNowEpoch: now };
        if (kid === KIDS.a && !row.active) {
          if (options.missingHighwater) result.lastSignedExp = null;
          if (options.decreaseHighwater && retirementWaits) result.lastSignedExp -= 1;
        }
        if (options.extraActive && kid === KIDS.a && !row.active && !row.retired) result.active = true;
        return result;
      },
      profile: async value => { calls.push(['profile', value]); },
      signLogoutWithCollector: async () => { calls.push(['logout']);
        state.get(active).lastSignedExp = Math.max(state.get(active).lastSignedExp ?? 0, now + 120);
        return { kid: options.wrongLogoutKid ? KIDS.a : active, delivered: !options.noDelivery }; },
      refreshAdminMfa: async () => { calls.push(['real-step-up']);return { realPasswordTotp: true, currentBrowserSourceVerified: !options.invalidMfaSource }; },
      wait: async milliseconds => { calls.push(['wait', milliseconds]); waits.push(milliseconds);
        if (active === KIDS.b && !state.has(KIDS.c)) retirementWaits += 1;
        if ((!options.stalledClock || active === KIDS.a) && !options.stalledPublishClock) now += milliseconds / 1000; },
      checkpointProgress: async value => { progress.push(structuredClone(value)); },
    };
    return { ports, state, calls, waits, progress, handles, get lookupCount() { return lookupCount; } };
  }
  test('mock ports run exact key boundaries and leave live C nonactive for the owner checkpoint', async () => {
    const mock = fixture({ poisonActivationReply: true });
    const result = await runKeyCases(mock.ports);
    assert.equal(mock.lookupCount, 1);
    assert.equal(result.cases.length, 6);
    assert.equal(result.cases.at(-1).status, 'READY');
    assert.deepEqual(Object.keys(result.privateCarriers), ['a', 'b', 'c', 'd']);
    assert.equal(mock.state.get(KIDS.c).revoked, false);
    assert.equal(mock.state.get(KIDS.c).active, false);
    assert.equal(mock.state.get(KIDS.d).active, true);
    assert.equal(mock.calls.filter(call => call[0] === 'profile').length, 1);
    assert.ok(mock.waits.every(value => value > 0 && value <= 10000));
    const k06 = result.cases.find(value => value.caseId === 'K06_NORMAL_RETIREMENT').evidence;
    assert.ok(k06.observedDbEndEpoch > k06.retainUntilEpoch);
    assert.equal(k06.lastSignedExpEpoch, 1361);
    assert.equal(k06.requestedWaitMs, 360001);
    assert.equal(JSON.stringify({ cases: result.cases, progress: mock.progress }).includes(CANARY), false);
    assert.equal(JSON.stringify(result.cases).includes('retained-'), false);
    assert.equal(JSON.stringify(mock.progress).includes('retained-'), false);
    assert.ok(mock.calls.filter(call => call[0] === 'authorize').every(call => call[1] === 'third'));
  });
  test('strict exp plus sixty comparison still waits when authority time equals the boundary', async () => {
    const mock = fixture();
    const result = await runKeyCases(mock.ports);
    const k06 = result.cases.find(value => value.caseId === 'K06_NORMAL_RETIREMENT').evidence;
    assert.ok(k06.observedDbEndEpoch > 1090);
    const retirementWaits = mock.progress.filter(value => value.caseId==='K06_NORMAL_RETIREMENT'&&value.status==='WAITING');
    assert.equal(retirementWaits.at(-2).waitMs, 10000); assert.equal(retirementWaits.at(-1).waitMs, 1);
    assert.ok(mock.waits.every(value => value <= 10000));
  });
  test('missing actual highwater fails before any retirement wait', async () => {
    const mock = fixture({ missingHighwater: true });
    await assert.rejects(runKeyCases(mock.ports), { code: 'K06_SIGNED_HIGHWATER_REQUIRED' });
    assert.equal(mock.progress.filter(value=>value.caseId==='K06_NORMAL_RETIREMENT'&&value.status==='WAITING').length, 0);
  });
  test('highwater changes, a stalled clock and excessive wait are bounded failures', async () => {
    for (const [option, code] of [['decreaseHighwater', 'K06_HIGHWATER_OR_LIFECYCLE_CHANGED'], ['stalledClock', 'K06_WAIT_BUDGET']]) {
      const mock = fixture({ [option]: true });
      await assert.rejects(runKeyCases(mock.ports), { code });
      const retirementWaits=mock.progress.filter(value=>value.caseId==='K06_NORMAL_RETIREMENT'&&value.status==='WAITING');
      assert.ok(retirementWaits.length <= MAX_POLLS);
      assert.ok(retirementWaits.reduce((sum, value) => sum + value.waitMs, 0) <= MAX_REQUESTED_WAIT_MS);
    }
  });
  test('original operation and completed result must match without a second activation', async () => {
    for (const option of ['wrongOperation', 'notCompleted']) {
      const mock = fixture({ [option]: true });
      await assert.rejects(runKeyCases(mock.ports), { code: 'KEY_ORIGINAL_RESULT_MISMATCH' });
      assert.equal(mock.calls.filter(call => call[0] === 'command' && call[1].path.endsWith('/activate')
        && call[1].body.kid === KIDS.b).length, 2); // one unpublished denial, one actual activation.
    }
  });
  test('old-token authority, subject, unique signer and real logout delivery are required', async () => {
    for (const [option, code] of [['oldExpired', 'KEY_OLD_A_NOT_LIVE'], ['subjectChanges', 'KEY_SUBJECT_CHANGED'],
      ['extraActive', 'KEY_ACTIVE_SET_MISMATCH'], ['wrongLogoutKid', 'KEY_LOGOUT_SIGNER_MISMATCH'], ['noDelivery', 'KEY_LOGOUT_SIGNER_MISMATCH']]) {
      const mock = fixture({ [option]: true });
      await assert.rejects(runKeyCases(mock.ports), { code });
    }
  });
  test('only actual USER issuance 503 satisfies no-active-key denial; safe thrown status is supported', async () => {
    await assert.rejects(runKeyCases(fixture({ wrongDenial: true }).ports), { code: 'K08_ID_ISSUANCE_DID_NOT_FAIL_CLOSED' });
    assert.equal((await runKeyCases(fixture({ throwDenial: true }).ports)).cases.length, 6);
  });
  test('port exception material and invalid ports never become diagnostic output', async () => {
    await assert.rejects(runKeyCases({}), { code: 'KEY_PORTS_INVALID' });
    try { await runKeyCases(fixture({ rpcThrows: true }).ports); assert.fail('must reject'); }
    catch (error) { assert.equal(error.code, 'KEY_SUBJECT_COMPARISON_FAILED'); assert.equal(String(error).includes(CANARY), false); }
  });
  test('outer key failure retains only sanitized inner RPC stage and fixed cause labels', async () => {
    const mock = fixture();
    mock.ports.authorize = async () => { throw Object.assign(new Error(CANARY), {
      protocolCause: { command: 'finish', code: 'SDK_REJECTED', status: 0, stage: 'finishLogin',
        sdkCode: 'OAUTH_JWT_CLAIM_COMPARISON_FAILED', callback: CANARY, key: CANARY } }); };
    await assert.rejects(runKeyCases(mock.ports), error => {
      assert.equal(error.code, 'KEY_AUTHORIZATION_FAILED');
      assert.deepEqual(error.protocolCause, { command: 'finish', code: 'SDK_REJECTED', status: 0,
        stage: 'finishLogin', sdkCode: 'OAUTH_JWT_CLAIM_COMPARISON_FAILED' });
      assert.equal(JSON.stringify(error).includes(CANARY), false); return true;
    });
  });
  test('prepublication keeps standard cache rules, refreshes retained A and precaches both C/D', async () => {
    const mock=fixture(),result=await runKeyCases(mock.ports);
    const b=result.cases.find(value=>value.caseId==='K02_PREPUBLISH').evidence.prepublication;
    const d=result.cases.find(value=>value.caseId==='K08_NO_ACTIVE_SIGNER').evidence;
    for(const window of [b,d.prepublication]){
      assert(window.observedPublishSeconds>=61);assert(window.requestedWaitMs>=61000);
      assert(window.sdkCacheAgeBeforeActivationSeconds>=61);assert.equal(window.cacheMutationRequested,false);
    }
    assert(mock.handles.get(result.privateCarriers.a).issuedAt>=b.observedGateDbEpoch);
    assert.deepEqual(d.publishedKeysDuringSignerGap,[KIDS.c,KIDS.d]);
    assert.deepEqual(d.sdkCachedKidsAfterRecovery,[KIDS.c,KIDS.d]);
    const ready=result.cases.at(-1).evidence;
    assert.equal(ready.requestedLegacyOidcTokenTtlSeconds,600);
    assert.equal(ready.legacySettingAppliedToCanonicalTokens,false);
    assert.equal(Object.hasOwn(ready,'profileTtlSeconds'),false);
    const canonicalC=await mock.ports.keyWindow(KIDS.c);
    assert.equal(canonicalC.lastSignedExp-canonicalC.dbNowEpoch,300);
    const stepUp=mock.calls.findIndex(call=>call[0]==='real-step-up');
    const revokeB=mock.calls.findIndex(call=>call[0]==='command'&&call[1].path.endsWith(KIDS.b+'/revoke'));
    const activateD=mock.calls.findIndex(call=>call[0]==='command'&&call[1].path.endsWith('/activate')&&call[1].body.kid===KIDS.d);
    assert(stepUp>=0&&stepUp<revokeB&&revokeB<activateD);
  });
  test('stalled prepublication is bounded and signer gap requires actual current-source MFA', async () => {
    const stalled=fixture({stalledPublishClock:true});
    await assert.rejects(runKeyCases(stalled.ports),{code:'KEY_PREPUBLICATION_WAIT_BUDGET'});
    assert(stalled.waits.every(value=>value>0&&value<=10000));
    assert(stalled.waits.reduce((sum,value)=>sum+value,0)<=MAX_REQUESTED_WAIT_MS);
    const badMfa=fixture({invalidMfaSource:true});
    await assert.rejects(runKeyCases(badMfa.ports),{code:'KEY_SIGNER_GAP_FRESH_ADMIN_SOURCE_REQUIRED'});
    assert.equal(badMfa.state.get(KIDS.b).active,true);
    assert.equal(badMfa.state.get(KIDS.b).revoked,false);
  });
  test('progress callbacks cannot inject private material into returned fixed case metadata', async () => {
    const mock = fixture();
    mock.ports.checkpointProgress = async value => { value.raw = CANARY; if (value.evidence) value.evidence.raw = CANARY; };
    const result = await runKeyCases(mock.ports);
    assert.equal(JSON.stringify(result.cases).includes(CANARY), false);
  });
}
