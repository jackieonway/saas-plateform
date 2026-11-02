import assert from 'node:assert/strict';
import { test } from 'node:test';
import { prepareRecoveryKeyCarriers } from './phase23-recovery-key-cases.mjs';

const A = 'owned-https', C = 'phase23-c', D = 'phase23-d';
const CANARY = 'PRIVATE_RECOVERY_KEY_CARRIER_CANARY';
function fixture(options = {}) {
  let now = 1000, sequence = 0, active = A, sdkCache;
  const states = new Map([[A, { active: true, retired: false, revoked: false, lastSignedExp: null }]]);
  const handles = new Map(), calls = [], waits = [], progress = [];
  const id = () => `00000000-0000-7000-8000-${String(++sequence).padStart(12, '0')}`;
  const ports = {
    profile: async ttl => { calls.push(['profile', ttl]); },
    keyCommand: async request => {
      calls.push(['command', structuredClone(request)]);
      const kid = request.body.kid;
      assert.equal(request.method, 'POST');
      if (request.path.endsWith('/publish')) {
        assert(!states.has(kid));
        states.set(kid, { active: false, retired: false, revoked: false, lastSignedExp: null });
        return { status: options.rejectedPublish ? 409 : 200, operation: id(), body: { kid, status: 'PUBLISHED' } };
      }
      assert(request.path.endsWith('/activate'));
      assert(states.has(kid));
      for (const row of states.values()) row.active = false;
      states.get(kid).active = true;active = kid;
      return { status: 200, operation: options.badOperation ? CANARY : id(),
        body: { kid, status: options.badActivationBody ? 'PUBLISHED' : 'ACTIVE' } };
    },
    authorize: async profile => {
      calls.push(['authorize', profile, active]);assert.equal(profile, 'third');
      if (!sdkCache || (!sdkCache.kids.includes(active) && Math.floor(now) - sdkCache.uat >= 60)) {
        sdkCache = { uat: Math.floor(now), kids: [...states.keys()] };
      }
      assert(sdkCache.kids.includes(active), 'real SDK unknown-kid refresh window would reject');
      const key = CANARY + '-' + handles.size;
      handles.set(key, active);states.get(active).lastSignedExp = now + 300;
      return { status: options.badAuthorization ? 503 : 200, key,
        kid: options.wrongSigner && active === C ? D : active };
    },
    rpc: async (command, parameters) => {
      calls.push(['rpc', command, structuredClone(parameters)]);
      if (command === 'jwks-cache') return { observed: !options.noCache, cachedKids: [...sdkCache.kids],
        cacheUpdatedAtEpoch: sdkCache.uat, cacheAgeSeconds: Math.floor(now) - sdkCache.uat,
        unknownKidRefetchAfterSeconds: 60, cacheMutationRequested: options.cacheMutation === true };
      if (command === 'same-subject') return { matches: !options.subjectChanges };
      assert.equal(command, 'verify-old-id-session');
      return { kid: handles.get(parameters.key), cachedKeySignatureValid: !options.badSignature,
        currentAuthorityJwtValid: !options.expiredC, onlineStatus: options.offlineC ? 401 : 200 };
    },
    jwksKids: async () => {
      calls.push(['jwks']);return options.badJwks ? [...states.keys(), 'unrelated'] : [...states.keys()];
    },
    keyWindow: async kid => {
      calls.push(['window', kid]);
      const row = { ...states.get(kid), dbNowEpoch: now };
      if (options.preactivated && kid === C) row.active = true;
      if (options.rollback && waits.length > 0) row.dbNowEpoch = 999;
      if (options.badWindow) row.dbNowEpoch = NaN;
      return row;
    },
    wait: async ms => {
      calls.push(['wait', ms]);waits.push(ms);
      if (!options.stalledClock) now += ms / 1000;
    },
    checkpointProgress: async value => { progress.push(structuredClone(value));value.privateData = CANARY; },
  };
  return { ports, calls, waits, progress, states, handles };
}

test('only C/D preparation waits on real port observations before two standard SDK issuances', async () => {
  const mock = fixture(), result = await prepareRecoveryKeyCarriers(mock.ports);
  assert.deepEqual(Object.keys(result), ['cases', 'privateCarriers']);
  assert.deepEqual(Object.keys(result.privateCarriers), ['c', 'd']);
  assert.equal(mock.handles.get(result.privateCarriers.c), C);
  assert.equal(mock.handles.get(result.privateCarriers.d), D);
  assert.deepEqual(mock.waits, [10000, 10000, 10000, 10000, 10000, 10000, 1000]);
  assert.deepEqual(mock.calls.filter(call => call[0] === 'profile'), [['profile', 600]]);
  assert.deepEqual(mock.calls.filter(call => call[0] === 'authorize').map(call => call[2]), [A, C, D]);
  const commands = mock.calls.filter(call => call[0] === 'command').map(call => call[1]);
  assert.deepEqual(commands.map(value => [value.path.split('/').at(-1), value.body.kid]),
    [['publish', C], ['publish', D], ['activate', C], ['activate', D]]);
  assert(commands.every(value => value.method === 'POST'));
  const activate = mock.calls.findIndex(call => call[0] === 'command' && call[1].path.endsWith('/activate'));
  assert(mock.calls.slice(0, activate).filter(call => call[0] === 'wait').length === 7);
  assert.equal(mock.states.get(A).active, false);assert.equal(mock.states.get(A).retired, false);
  assert.equal(mock.states.get(C).active, false);assert.equal(mock.states.get(C).revoked, false);
  assert.equal(mock.states.get(D).active, true);
  assert.equal(result.cases.length, 2);
  assert(result.cases.every(value => value.statusScope === 'RECOVERY_PREPARATION_ONLY'));
  assert.equal(result.cases[0].evidence.observedPublishSeconds, 61);
  assert.equal(result.cases[1].evidence.fullKeyLifecycleExecuted, false);
  assert.equal(result.cases[1].evidence.legacySettingAppliedToCanonicalTokens, false);
  assert.equal(JSON.stringify(result.cases).includes(CANARY), false);
  assert.equal(JSON.stringify(mock.progress).includes(CANARY), false);
});

test('the original C is checked both before and after D becomes active', async () => {
  const mock = fixture(), result = await prepareRecoveryKeyCarriers(mock.ports);
  const checks = mock.calls.filter(call => call[0] === 'rpc' && call[1] === 'verify-old-id-session');
  assert.deepEqual(checks.map(call => call[2].key),
    [result.privateCarriers.c, result.privateCarriers.c, result.privateCarriers.d]);
  const compared = mock.calls.find(call => call[0] === 'rpc' && call[1] === 'same-subject');
  assert.deepEqual(compared[2], { leftKey: result.privateCarriers.c, rightKey: result.privateCarriers.d });
});

test('a stalled publication clock has bounded waits and never activates a new signer', async () => {
  const mock = fixture({ stalledClock: true });
  await assert.rejects(prepareRecoveryKeyCarriers(mock.ports), { code: 'PHASE23_RECOVERY_KEYS_PREPUBLICATION_WAIT_BUDGET' });
  assert(mock.waits.every(value => value > 0 && value <= 10000));
  assert(mock.waits.reduce((sum, value) => sum + value, 0) <= 900000);
  assert.equal(mock.calls.filter(call => call[0] === 'command' && call[1].path.endsWith('/activate')).length, 0);
});

test('rollback, premature activation and malformed cache or clock fail closed', async () => {
  for (const [option, code] of [['rollback', 'PREPUBLICATION_CLOCK_ROLLED_BACK'],
    ['preactivated', 'PREPUBLICATION_STATE_CHANGED'], ['badWindow', 'WINDOW_INVALID'],
    ['noCache', 'SDK_CACHE_INVALID'], ['cacheMutation', 'SDK_CACHE_INVALID']]) {
    await assert.rejects(prepareRecoveryKeyCarriers(fixture({ [option]: true }).ports),
      { code: 'PHASE23_RECOVERY_KEYS_' + code });
  }
});

test('publication, activation and standard callback results retain their real contracts', async () => {
  for (const [option, code] of [['rejectedPublish', 'COMMAND_REJECTED'], ['badOperation', 'COMMAND_REJECTED'],
    ['badActivationBody', 'COMMAND_REJECTED'], ['wrongSigner', 'SIGNER_MISMATCH'],
    ['badAuthorization', 'SIGNER_MISMATCH'], ['badJwks', 'JWKS_MISMATCH']]) {
    await assert.rejects(prepareRecoveryKeyCarriers(fixture({ [option]: true }).ports),
      { code: 'PHASE23_RECOVERY_KEYS_' + code });
  }
});

test('stable subject, live original C and actual signature and online checks are mandatory', async () => {
  for (const [option, code] of [['subjectChanges', 'SUBJECT_CHANGED'], ['expiredC', 'CARRIER_NOT_LIVE'],
    ['badSignature', 'CARRIER_NOT_LIVE'], ['offlineC', 'CARRIER_NOT_LIVE']]) {
    await assert.rejects(prepareRecoveryKeyCarriers(fixture({ [option]: true }).ports),
      { code: 'PHASE23_RECOVERY_KEYS_' + code });
  }
});

test('missing ports and private port exceptions yield only fixed safe diagnostics', async () => {
  await assert.rejects(prepareRecoveryKeyCarriers({}), { code: 'PHASE23_RECOVERY_KEYS_PORTS_INVALID' });
  const mock = fixture();
  mock.ports.authorize = async () => { throw Object.assign(new Error(CANARY), {
    protocolCause: { command: 'finish', code: 'SDK_REJECTED', status: 0, stage: 'finishLogin',
      sdkCode: 'OAUTH_KEY_SELECTION_FAILED', callback: CANARY, secret: CANARY } }); };
  await assert.rejects(prepareRecoveryKeyCarriers(mock.ports), error => {
    assert.equal(error.code, 'PHASE23_RECOVERY_KEYS_AUTHORIZATION_FAILED');
    assert.deepEqual(error.protocolCause, { command: 'finish', code: 'SDK_REJECTED', status: 0,
      stage: 'finishLogin', sdkCode: 'OAUTH_KEY_SELECTION_FAILED' });
    assert.equal(JSON.stringify(error).includes(CANARY), false);
    assert.equal(String(error).includes(CANARY), false);
    return true;
  });
});
