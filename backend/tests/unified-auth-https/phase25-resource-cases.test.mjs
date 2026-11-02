import assert from 'node:assert/strict';
import test from 'node:test';
import { runResourceCases } from './phase25-resource-cases.mjs';

function fixture({ reviveApplication = false, reviveMember = false, ignoreGrant = false, badCleanup = false } = {}) {
  const log = [], carriers = new Map(), state = { app: true, member: true, grant: true, appGeneration: 1, memberGeneration: 1 };
  let sequence = 0;
  const ports = {
    fixtureGuardVerified: true,
    mint(actor, code, options) {
      log.push({ operation: 'mint', actor, code, options, appGeneration: state.appGeneration, memberGeneration: state.memberGeneration });
      const token = `private-test-carrier-${++sequence}`;
      carriers.set(token, { actor, code, app: state.appGeneration, member: state.memberGeneration }); return token;
    },
    types() { return { idToken: 'private-id-token', machineToken: 'private-machine-token',
      platformToken: 'private-platform-token', idTokenVerified: true, machineHasPersonFields: false }; },
    read(code, token) {
      const source = carriers.get(token); log.push({ operation: 'read', code, token });
      const allowed = source && source.code === code && (code !== 'stores' || (state.app
        && (reviveApplication || source.app === state.appGeneration)
        && (source.actor !== 'employee' || state.grant || ignoreGrant)))
        && (source.actor !== 'employee' || (state.member && (reviveMember || source.member === state.memberGeneration)));
      return { status: allowed ? 200 : 403 };
    },
    change(action) {
      log.push({ operation: 'change', action });
      if (action === 'remove-employee-store-read') state.grant = false;
      if (action === 'restore-employee-store-read') state.grant = true;
      if (action === 'disable-stores') { state.app = false; state.appGeneration++; }
      if (action === 'enable-stores') { state.app = true; state.appGeneration++; }
      if (action === 'disable-employee') { state.member = false; state.memberGeneration++; }
      if (action === 'enable-employee') { state.member = true; state.memberGeneration++; }
    },
    restore() {
      log.push({ operation: 'restore' }); state.app = state.member = state.grant = true;
      return { policyRestored: !badCleanup, memberEnabled: true, applicationEnabled: true,
        dataGrantRestored: true, generationsNeverDecreased: true };
    },
  };
  return { ports, log, state };
}

test('current resource orchestration checks original carriers before replacement, and only returns facts', async () => {
  const value = fixture(), result = await runResourceCases(value.ports);
  assert.equal(result.facts.length, 5);
  assert(result.facts.every(fact => fact.status === 'PASS'));
  assert.equal(result.cleanup.fixturePolicyRestored, true);
  assert(!JSON.stringify(result).includes('private-'));
  const enabled = value.log.findIndex(item => item.operation === 'change' && item.action === 'enable-stores');
  const replay = value.log[enabled + 1], replacement = value.log[enabled + 2];
  assert.equal(replay.operation, 'read'); assert.equal(replay.token, 'private-test-carrier-1');
  assert.equal(replacement.operation, 'mint');
  const memberEnabled = value.log.findIndex(item => item.operation === 'change' && item.action === 'enable-employee');
  assert.equal(value.log[memberEnabled + 1].operation, 'read');
  assert.equal(value.log[memberEnabled + 2].operation, 'mint');
  assert.equal(value.log[memberEnabled + 2].options.newLogin, true);
  assert.equal(value.log.at(-1).operation, 'restore');
});

test('a removed data grant that still permits the original request is a hard failure with cleanup', async () => {
  const value = fixture({ ignoreGrant: true });
  await assert.rejects(runResourceCases(value.ports), /PHASE25_RESOURCE_REMOVED_DATA_GRANT_STILL_AUTHORIZES/);
  assert.equal(value.log.at(-1).operation, 'restore'); assert.equal(value.state.grant, true);
});

test('re-enabling application cannot turn a stale original carrier into a passing result', async () => {
  const value = fixture({ reviveApplication: true });
  await assert.rejects(runResourceCases(value.ports), /PHASE25_RESOURCE_REENABLED_APPLICATION_REVIVED_OLD_CARRIER/);
  assert.equal(value.log.at(-1).operation, 'restore');
});

test('re-enabling membership cannot turn a stale original carrier into a passing result', async () => {
  const value = fixture({ reviveMember: true });
  await assert.rejects(runResourceCases(value.ports), /PHASE25_RESOURCE_REENABLED_MEMBER_REVIVED_OLD_CARRIER/);
  assert.equal(value.log.at(-1).operation, 'restore');
});

test('cleanup failure never produces PASS', async () => {
  const value = fixture({ badCleanup: true });
  await assert.rejects(runResourceCases(value.ports), /PHASE25_RESOURCE_RESTORATION_FAILED/);
});

test('unverified fixture or missing real carrier type fails before contraction', async () => {
  const unbound = fixture(); unbound.ports.fixtureGuardVerified = false;
  await assert.rejects(runResourceCases(unbound.ports), /PHASE25_RESOURCE_PORTS_REQUIRED/);
  assert.equal(unbound.log.length, 0);
  const missing = fixture(); missing.ports.types = () => ({ idToken: 'fixture-is-not-a-real-token' });
  await assert.rejects(runResourceCases(missing.ports), /PHASE25_RESOURCE_REAL_TYPES_REQUIRED/);
  assert(!missing.log.some(item => item.operation === 'change'));
  assert.equal(missing.log.at(-1).operation, 'restore');
});
