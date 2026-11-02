/** Bounded, private-data-free decisions for the real identity browser driver. */
import assert from 'node:assert/strict';

export function selectionSnapshot(value) {
  if (value?.status !== 'CONTEXT_SELECTION_REQUIRED') return null;
  const members = value.contextSelection?.members;
  if (!Array.isArray(members) || members.length > 256 || typeof value.account?.id !== 'string') {
    return { valid: false, members: [] };
  }
  return { valid: true, accountId: value.account.id,
    members: members.map(member => ({ membershipId: member?.membershipId,
      userId: member?.userId, tenantId: member?.tenantId, memberType: member?.memberType })) };
}

export function selectionFacts(snapshot, expected) {
  const members = snapshot?.members ?? [];
  const valid = snapshot?.valid === true && members.every(member =>
    ['membershipId', 'userId', 'tenantId'].every(key => typeof member[key] === 'string')
      && ['EMPLOYEE', 'CUSTOMER'].includes(member.memberType));
  return { valid, candidateCount: members.length,
    tenantCount: new Set(members.map(member => member.tenantId)).size,
    memberTypes: [...new Set(members.map(member => ['EMPLOYEE', 'CUSTOMER'].includes(member.memberType)
      ? member.memberType : 'UNKNOWN'))].sort(),
    accountMatches: valid && snapshot.accountId === expected?.accountId,
    memberAccountsMatch: valid && members.every(member => member.userId === snapshot.accountId),
    expectedMembershipMatches: members.filter(member => member.membershipId === expected?.membershipId).length,
    expectedMemberTypeOnly: valid && members.every(member => member.memberType === expected?.memberType) };
}

export function selectionAction(facts, allowMultiple) {
  assert(facts.valid && facts.accountMatches && facts.memberAccountsMatch
    && facts.expectedMemberTypeOnly, 'REAL_SELECTION_CANDIDATE_BINDING_FAILED');
  assert(facts.candidateCount > 0, 'REAL_CONTEXT_SELECTION_EMPTY');
  assert.equal(facts.expectedMembershipMatches, 1, 'REAL_EXPECTED_MEMBERSHIP_MISSING');
  assert.equal(facts.tenantCount, 1, 'UNEXPECTED_MULTI_TENANT_FLOW');
  if (facts.candidateCount === 1) return 'AWAIT_AUTOMATIC_SELECTION';
  assert(allowMultiple, 'UNEXPECTED_MULTI_MEMBER_FLOW');
  return 'CHOOSE_EXPECTED_MEMBERSHIP';
}

export function verifyCurrentTotpSource(session, proof, expected) {
  assert(typeof expected?.userId === 'string' && expected.userId.length > 0
    && typeof expected?.cookieDigest === 'string' && expected.cookieDigest.length > 0
    && session?.account?.id === expected.userId && typeof session?.sessionId === 'string' && session.sessionId.length > 0
    && proof?.bffSessionId === session.sessionId && proof.bffUserId === expected.userId
    && proof.bffTokenDigest === expected.cookieDigest
    && proof.browserCurrentSessionId === session.sessionId, 'REAL_ADMIN_CURRENT_BFF_SOURCE_REQUIRED');
  assert(typeof proof.globalSessionId === 'string' && proof.globalSessionId.length > 0
    && proof.sourceSessionId === proof.globalSessionId && proof.grantSourceSessionId === proof.globalSessionId
    && proof.sourceUserId === expected.userId, 'REAL_ADMIN_GLOBAL_SOURCE_BINDING_REQUIRED');
  assert(proof.bffLive === true && proof.sourceLive === true && proof.canonicalProofActive === true
    && proof.totpEnabled === true && proof.mfaMethod === 'totp' && proof.recentTotp === true,
  'REAL_ADMIN_TOTP_SOURCE_REQUIRED');
  return { currentBffBound: true, browserCookieBound: true, exactGlobalSourceBound: true,
    liveCanonicalSource: true, mfaMethod: 'totp', verifiedWithinFiveMinutes: true };
}

export function safeFailureCode(error, interrupted = false) {
  if (interrupted) return 'BROWSER_INTERRUPTED';
  if (/^(?:RECOVERY|PHASE23)_[A-Z_0-9]+$/.test(error?.code ?? '')) return error.code;
  if (['KeyCaseFailure', 'InvitationCaseFailure', 'PolicyCaseFailure'].includes(error?.name)
      && /^[A-Z][A-Z0-9_]{2,100}$/.test(error?.code ?? '')) return error.code;
  // Node strictEqual appends a diff to an explicit custom message. Only the
  // first line of our assertion label is reportable; never include that diff.
  const label = String(error?.message ?? '').split(/\r?\n/, 1)[0].trim();
  if ((error?.code === 'ERR_ASSERTION' || error?.name === 'AssertionError')
      && /^[A-Z][A-Z0-9_]{2,100}$/.test(label)) return label;
  if (['OWNED_SUBPROCESS_FAILED', 'STANDARD_PROTOCOL_REJECTED', 'PROTOCOL_DRIVER_TIMEOUT',
    'PROTOCOL_DRIVER_START_FAILED', 'PROTOCOL_DRIVER_EXITED'].includes(error?.message)) return error.message;
  if (error?.name === 'TimeoutError') return 'BROWSER_STEP_TIMEOUT';
  return 'PHASE23_STEP_FAILED';
}

/** Append only fixed interaction labels, never a flow, URL or response body. */
export function recordIdentityInteraction(events, input) {
  assert(Array.isArray(events), 'PHASE23_INTERACTION_DIAGNOSTICS_INVALID');
  if (events.length >= 64 || !['admin', 'employee', 'customer', 'standard'].includes(input?.actor)
      || !['SOURCE', 'RESTORE'].includes(input.mode)
      || !['DECISION_POST', 'FOLLOWUP_GET'].includes(input.phase)
      || !Number.isInteger(input.status) || input.status < 100 || input.status > 599) return false;
  const screens = ['LOGIN_REQUIRED', 'CONSENT_REQUIRED', 'CONTEXT_SELECTION_REQUIRED',
    'RESTART_REQUIRED', 'AUTO_REDIRECT'];
  const actions = ['consent', 'approve', 'deny', 'restart'];
  events.push({ actor: input.actor, mode: input.mode, phase: input.phase,
    method: input.phase === 'DECISION_POST' ? 'POST' : 'GET', status: input.status,
    screen: screens.includes(input.screen) ? input.screen : 'UNKNOWN',
    action: input.phase === 'DECISION_POST' && actions.includes(input.action) ? input.action : null });
  return true;
}
