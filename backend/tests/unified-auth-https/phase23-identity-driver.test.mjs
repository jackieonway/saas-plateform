import assert from 'node:assert/strict';
import { test } from 'node:test';
import { selectionSnapshot, selectionFacts, selectionAction, verifyCurrentTotpSource, safeFailureCode, recordIdentityInteraction } from './phase23-identity-driver.mjs';

const expected = { accountId: 'private-account', membershipId: 'private-membership', memberType: 'EMPLOYEE' };
const member = { membershipId: expected.membershipId, userId: expected.accountId,
  tenantId: 'private-tenant', memberType: 'EMPLOYEE' };
function value(members = [member]) {
  return { status: 'CONTEXT_SELECTION_REQUIRED', account: { id: expected.accountId, username: 'PRIVATE_ACTOR_CANARY' },
    csrfToken: 'PRIVATE_CSRF_CANARY', contextSelection: { members } };
}
test('the actual single server candidate always waits for product automatic selection', () => {
  const snapshot = selectionSnapshot(value()), facts = selectionFacts(snapshot, expected);
  assert.equal(selectionAction(facts, false), 'AWAIT_AUTOMATIC_SELECTION');
  assert.equal(facts.candidateCount, 1); assert.equal(facts.expectedMembershipMatches, 1);
  assert.equal(JSON.stringify(snapshot).includes('PRIVATE_ACTOR_CANARY'), false);
  assert.equal(JSON.stringify(snapshot).includes('PRIVATE_CSRF_CANARY'), false);
  assert.equal(JSON.stringify(facts).includes('private-'), false);
});
test('admin multiple candidates remain a failure and only explicit customer multiple selection may proceed', () => {
  const facts = selectionFacts(selectionSnapshot(value([member, { ...member, membershipId: 'private-second' }])), expected);
  assert.throws(() => selectionAction(facts, false), /UNEXPECTED_MULTI_MEMBER_FLOW/);
  assert.equal(selectionAction(facts, true), 'CHOOSE_EXPECTED_MEMBERSHIP');
  const multiTenant = selectionFacts(selectionSnapshot(value([member,
    { ...member, membershipId: 'private-second', tenantId: 'private-another-tenant' }])), expected);
  assert.throws(() => selectionAction(multiTenant, true), /UNEXPECTED_MULTI_TENANT_FLOW/);
});
test('server candidate bindings cannot be replaced by filtered visible count or wrong member type', () => {
  for (const members of [[{ ...member, userId: 'wrong-account' }],
    [{ ...member, memberType: 'CUSTOMER' }], [{ ...member, memberType: 'PRIVATE_VALUE_CANARY' }],
    [{ ...member, membershipId: 'wrong-membership' }], [member, member]]) {
    const facts = selectionFacts(selectionSnapshot(value(members)), expected);
    assert.throws(() => selectionAction(facts, false));
    assert.equal(JSON.stringify(facts).includes('PRIVATE_VALUE_CANARY'), false);
  }
  assert.throws(() => selectionAction(selectionFacts(selectionSnapshot(value([])), expected), false), /SELECTION_EMPTY/);
  assert.equal(selectionSnapshot({ status: 'CONSENT_REQUIRED' }), null);
  assert.equal(selectionSnapshot(value(Array(257).fill(member))).valid, false);
});
test('custom assertion code survives a Node diff without exposing compared private values', () => {
  let failure;
  try { assert.equal('PRIVATE_LEFT_CANARY', 'PRIVATE_RIGHT_CANARY', 'UNEXPECTED_MULTI_MEMBER_FLOW'); }
  catch (error) { failure = error; }
  assert.equal(safeFailureCode(failure), 'UNEXPECTED_MULTI_MEMBER_FLOW');
  assert.equal(safeFailureCode({ name: 'AssertionError', message: 'REAL_BOUNDARY_FAILED\r\n\r\nPRIVATE_CANARY' }), 'REAL_BOUNDARY_FAILED');
  assert.equal(safeFailureCode(new Error('PRIVATE_CANARY')), 'PHASE23_STEP_FAILED');
  assert.equal(safeFailureCode({ code: 'RECOVERY_DATABASE_ACL_CHANGED' }), 'RECOVERY_DATABASE_ACL_CHANGED');
  assert.equal(safeFailureCode({ name: 'KeyCaseFailure', code: 'K06_WAIT_FAILED', message: 'PRIVATE_CANARY' }), 'K06_WAIT_FAILED');
  assert.equal(safeFailureCode(new Error('STANDARD_PROTOCOL_REJECTED')), 'STANDARD_PROTOCOL_REJECTED');
  assert.equal(safeFailureCode({ name: 'TimeoutError', message: 'PRIVATE_CANARY' }), 'BROWSER_STEP_TIMEOUT');
  assert.equal(safeFailureCode(failure, true), 'BROWSER_INTERRUPTED');
});
function currentSource() {
  return { session: { sessionId: 'private-current-bff', account: { id: 'private-admin' } },
    expected: { userId: 'private-admin', cookieDigest: 'private-cookie-digest' },
    proof: { bffSessionId: 'private-current-bff', bffUserId: 'private-admin',
      bffTokenDigest: 'private-cookie-digest', browserCurrentSessionId: 'private-current-bff',
      globalSessionId: 'private-current-source', sourceSessionId: 'private-current-source',
      grantSourceSessionId: 'private-current-source', sourceUserId: 'private-admin',
      bffLive: true, sourceLive: true, canonicalProofActive: true,
      totpEnabled: true, mfaMethod: 'totp', recentTotp: true } };
}
test('valid current BFF TOTP proof succeeds regardless of other live enrollment or step-up sessions', () => {
  const { session, expected, proof } = currentSource();
  // The current source is sufficient; a proof of a different valid session
  // may coexist, but cannot be substituted for the selected browser source.
  for (const other of ['private-enrollment-source', 'private-newer-other-source']) {
    const otherProof = { ...proof, bffSessionId: 'private-other-bff',
      globalSessionId: other, sourceSessionId: other, grantSourceSessionId: other };
    assert.throws(() => verifyCurrentTotpSource(session, otherProof, expected), /CURRENT_BFF_SOURCE_REQUIRED/);
  }
  const evidence = verifyCurrentTotpSource(session, proof, expected);
  assert.equal(evidence.exactGlobalSourceBound, true);
  assert.equal(evidence.verifiedWithinFiveMinutes, true);
  assert.equal(JSON.stringify(evidence).includes('private-'), false);
});
test('other old or fresh TOTP sources cannot replace the current cookie and BFF global source', () => {
  const { session, expected, proof } = currentSource();
  for (const changes of [{ bffSessionId: 'private-other-bff' }, { bffUserId: 'private-other-admin' },
    { bffTokenDigest: 'private-other-cookie' }, { browserCurrentSessionId: 'private-other-bff' },
    { sourceSessionId: 'private-enrollment-source' }, { sourceSessionId: 'private-newer-other-source' },
    { grantSourceSessionId: 'private-other-source' }, { sourceUserId: 'private-other-admin' },
    { bffLive: false }, { sourceLive: false }, { canonicalProofActive: false },
    { totpEnabled: false }, { mfaMethod: 'recovery_code' }, { recentTotp: false }]) {
    assert.throws(() => verifyCurrentTotpSource(session, { ...proof, ...changes }, expected));
  }
  assert.throws(() => verifyCurrentTotpSource(session, null, expected), /CURRENT_BFF_SOURCE_REQUIRED/);
});

test('interaction diagnostics retain an actual POST followed by its failing GET as fixed labels', () => {
  const events = [];
  assert(recordIdentityInteraction(events, { actor: 'employee', mode: 'RESTORE',
    phase: 'DECISION_POST', status: 200, screen: 'CONSENT_REQUIRED', action: 'consent' }));
  assert(recordIdentityInteraction(events, { actor: 'employee', mode: 'RESTORE',
    phase: 'FOLLOWUP_GET', status: 401, screen: 'CONSENT_REQUIRED' }));
  assert.deepEqual(events, [
    { actor: 'employee', mode: 'RESTORE', phase: 'DECISION_POST', method: 'POST', status: 200,
      screen: 'CONSENT_REQUIRED', action: 'consent' },
    { actor: 'employee', mode: 'RESTORE', phase: 'FOLLOWUP_GET', method: 'GET', status: 401,
      screen: 'CONSENT_REQUIRED', action: null },
  ]);
});

test('interaction diagnostics cap all actors and requests at 64 and reject invalid labels', () => {
  const input = { actor: 'standard', mode: 'SOURCE', phase: 'DECISION_POST', status: 200,
    screen: 'CONSENT_REQUIRED', action: 'approve' };
  const events = [];
  for (let index = 0; index < 80; index += 1) {
    assert.equal(recordIdentityInteraction(events, input), index < 64);
  }
  assert.equal(events.length, 64);
  for (const change of [{ actor: 'private-user' }, { mode: 'UNKNOWN' }, { phase: 'private-url' },
    { status: 0 }, { status: '401' }, { status: 600 }, { status: NaN }]) {
    const rejected = [];
    assert.equal(recordIdentityInteraction(rejected, { ...input, ...change }), false);
    assert.deepEqual(rejected, []);
  }
});

test('interaction diagnostics discard private fields and unknown response or action values', () => {
  const canary = 'PRIVATE_INTERACTION_CANARY';
  const events = [];
  recordIdentityInteraction(events, { actor: 'customer', mode: 'RESTORE', phase: 'DECISION_POST',
    status: 200, screen: canary, action: canary, url: canary, flow: canary, csrfToken: canary,
    response: { secret: canary } });
  assert.equal(events[0].screen, 'UNKNOWN');assert.equal(events[0].action, null);
  assert.equal(JSON.stringify(events).includes(canary), false);
  assert.deepEqual(Object.keys(events[0]), ['actor', 'mode', 'phase', 'method', 'status', 'screen', 'action']);
});
