/** Prepare real C/D carriers for the separately reported recovery suite.
 *
 * Importing starts nothing. Ports have the runKeyCases contract: keyCommand,
 * authorize, rpc, jwksKids, keyWindow, profile, wait and checkpointProgress.
 * The owner supplies real standard-SDK authorizations, current public JWKS,
 * database clock observations and bounded real waits. A is used only to prime
 * the retained SDK cache; this module proves no A retirement, B lifecycle or
 * no-active-signer case. Pure port tests are never dynamic recovery evidence.
 */
import { safeProtocolCause } from './phase23-protocol-diagnostics.mjs';

const A = 'owned-https', C = 'phase23-c', D = 'phase23-d';
const SCOPE = 'RECOVERY_PREPARATION_ONLY';
const PORTS = ['keyCommand', 'authorize', 'rpc', 'jwksKids', 'keyWindow',
  'profile', 'wait', 'checkpointProgress'];
const MAX_POLLS = 120, MAX_WAIT_MS = 10000, MAX_TOTAL_WAIT_MS = 900000;
const PREPUBLICATION_SECONDS = 61;
const record = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const operation = value => typeof value === 'string'
  && /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value);
const handle = value => typeof value === 'string' && /^[A-Za-z0-9_-]{1,160}$/.test(value);

export class RecoveryKeyCaseFailure extends Error {
  constructor(code) { super(code); this.name = 'RecoveryKeyCaseFailure'; this.code = code; }
}
function need(condition, suffix) {
  if (!condition) throw new RecoveryKeyCaseFailure('PHASE23_RECOVERY_KEYS_' + suffix);
}
async function call(action, suffix) {
  try { return await action(); }
  catch (error) {
    const failure = new RecoveryKeyCaseFailure('PHASE23_RECOVERY_KEYS_' + suffix);
    const cause = safeProtocolCause(error?.protocolCause);
    if (cause) failure.protocolCause = cause;
    throw failure;
  }
}

export async function prepareRecoveryKeyCarriers(ports) {
  need(record(ports) && PORTS.every(name => typeof ports[name] === 'function'), 'PORTS_INVALID');
  const cases = [];
  const progress = value => call(() => ports.checkpointProgress(structuredClone(value)), 'PROGRESS_FAILED');
  async function fact(caseId, evidence) {
    const value = { caseId, status: 'PASS', statusScope: SCOPE, evidence };
    await progress(value);cases.push(value);
  }
  async function keys(expected) {
    const value = await call(() => ports.jwksKids(), 'JWKS_FAILED');
    need(Array.isArray(value) && value.length === expected.length && new Set(value).size === value.length
      && expected.every(kid => value.includes(kid)), 'JWKS_MISMATCH');
  }
  async function command(action, kid, state) {
    const value = await call(() => ports.keyCommand({ method: 'POST', path: '/api/identity/keys/' + action,
      body: { kid } }), 'COMMAND_FAILED');
    need(record(value) && value.status === 200 && operation(value.operation)
      && record(value.body) && value.body.kid === kid && value.body.status === state, 'COMMAND_REJECTED');
  }
  async function authorize(kid) {
    const value = await call(() => ports.authorize('third'), 'AUTHORIZATION_FAILED');
    need(record(value) && (value.status === undefined || value.status === 200)
      && handle(value.key) && value.kid === kid, 'SIGNER_MISMATCH');
    return value.key;
  }
  async function window(kid) {
    const value = await call(() => ports.keyWindow(kid), 'WINDOW_FAILED');
    need(record(value) && ['active', 'retired', 'revoked'].every(name => typeof value[name] === 'boolean')
      && Number.isFinite(value.dbNowEpoch) && value.dbNowEpoch > 0
      && (value.lastSignedExp === null || (Number.isFinite(value.lastSignedExp) && value.lastSignedExp > 0)),
    'WINDOW_INVALID');
    return value;
  }
  async function cache() {
    const value = await call(() => ports.rpc('jwks-cache', { profile: 'third' }), 'SDK_CACHE_FAILED');
    need(record(value) && value.observed === true && Array.isArray(value.cachedKids)
      && value.cachedKids.length > 0 && new Set(value.cachedKids).size === value.cachedKids.length
      && value.cachedKids.every(kid => [A, C, D].includes(kid))
      && Number.isInteger(value.cacheAgeSeconds) && value.cacheAgeSeconds >= 0
      && Number.isInteger(value.cacheUpdatedAtEpoch) && value.cacheUpdatedAtEpoch > 0
      && value.unknownKidRefetchAfterSeconds === 60 && value.cacheMutationRequested === false,
    'SDK_CACHE_INVALID');
    return value;
  }
  function unpublished(value) {
    need(!value.active && !value.retired && !value.revoked, 'PREPUBLICATION_STATE_CHANGED');
  }
  async function verify(key, kid) {
    const value = await call(() => ports.rpc('verify-old-id-session', { key }), 'CARRIER_VERIFICATION_FAILED');
    need(record(value) && value.kid === kid && value.cachedKeySignatureValid === true
      && value.currentAuthorityJwtValid === true && value.onlineStatus === 200, 'CARRIER_NOT_LIVE');
  }

  await keys([A]);
  // This requests the existing legacy profile. Canonical deadlines still have
  // to be observed from the actual retained carriers and original database.
  await call(() => ports.profile(600), 'PROFILE_FAILED');
  await authorize(A);
  await command('publish', C, 'PUBLISHED');await command('publish', D, 'PUBLISHED');
  await keys([A, C, D]);
  unpublished(await window(C));
  const start = await window(D);unpublished(start);
  let latest = start, sdk = await cache(), polls = 0, requestedWaitMs = 0;
  while (latest.dbNowEpoch - start.dbNowEpoch < PREPUBLICATION_SECONDS
    || (![C, D].every(kid => sdk.cachedKids.includes(kid)) && sdk.cacheAgeSeconds < PREPUBLICATION_SECONDS)) {
    need(polls < MAX_POLLS, 'PREPUBLICATION_POLL_LIMIT');
    const seconds = Math.max(PREPUBLICATION_SECONDS - (latest.dbNowEpoch - start.dbNowEpoch),
      [C, D].every(kid => sdk.cachedKids.includes(kid)) ? 0 : PREPUBLICATION_SECONDS - sdk.cacheAgeSeconds);
    const milliseconds = Math.min(MAX_WAIT_MS, Math.max(1, Math.ceil(seconds * 1000)));
    need(requestedWaitMs + milliseconds <= MAX_TOTAL_WAIT_MS, 'PREPUBLICATION_WAIT_BUDGET');
    await progress({ caseId: 'RK01_C_D_PREPUBLICATION', status: 'WAITING', statusScope: SCOPE,
      poll: polls + 1, waitMs: milliseconds, observedPublishSeconds: latest.dbNowEpoch - start.dbNowEpoch,
      observedSdkCacheAgeSeconds: sdk.cacheAgeSeconds });
    await call(() => ports.wait(milliseconds), 'PREPUBLICATION_WAIT_FAILED');
    requestedWaitMs += milliseconds;polls += 1;
    unpublished(await window(C));
    const observed = await window(D);unpublished(observed);
    need(observed.dbNowEpoch >= latest.dbNowEpoch, 'PREPUBLICATION_CLOCK_ROLLED_BACK');
    latest = observed;sdk = await cache();
  }
  await fact('RK01_C_D_PREPUBLICATION', { minimumPublishSeconds: PREPUBLICATION_SECONDS,
    observedPublishSeconds: latest.dbNowEpoch - start.dbNowEpoch,
    observedPublishStartDbEpoch: start.dbNowEpoch, observedGateDbEpoch: latest.dbNowEpoch,
    sdkCacheAgeBeforeActivationSeconds: sdk.cacheAgeSeconds, unknownKidRefetchAfterSeconds: 60,
    cacheMutationRequested: false, requestedWaitMs, polls });

  await command('activate', C, 'ACTIVE');const c = await authorize(C);await verify(c, C);
  await command('activate', D, 'ACTIVE');const d = await authorize(D);
  const same = await call(() => ports.rpc('same-subject', { leftKey: c, rightKey: d }), 'SUBJECT_COMPARISON_FAILED');
  need(record(same) && same.matches === true, 'SUBJECT_CHANGED');
  await keys([A, C, D]);await verify(c, C);await verify(d, D);
  const retainedC = await window(C), activeD = await window(D);
  need(!retainedC.active && !retainedC.retired && !retainedC.revoked
    && retainedC.lastSignedExp !== null && retainedC.lastSignedExp > retainedC.dbNowEpoch
    && activeD.active && !activeD.retired && !activeD.revoked
    && activeD.lastSignedExp !== null && activeD.lastSignedExp > activeD.dbNowEpoch, 'FINAL_STATE_INVALID');
  const retainedCache = await cache();
  need([C, D].every(kid => retainedCache.cachedKids.includes(kid)), 'C_D_NOT_IN_SDK_CACHE');
  await fact('RK02_RECOVERY_CARRIERS_READY', { publishedNonactiveKid: C, activeKid: D,
    standardSdkIssuances: 2, stableSubject: true, originalCStillValid: true,
    requestedLegacyOidcTokenTtlSeconds: 600, legacySettingAppliedToCanonicalTokens: false,
    carrierDeadlinesRequireActualObservation: true, revocationDeferredUntilAfterCheckpoint: true,
    fullKeyLifecycleExecuted: false, sameSdkProcessRetained: true });
  return { cases, privateCarriers: { c, d } };
}
