/** Injected canonical invitation recovery cases; importing starts no environment.
 *
 * Exports runBeforeCheckpoint(ports), runAfterCheckpoint(ports,privateCarriers),
 * runAfterRecovery(ports,privateCarriers). Each returns {facts}; the first two
 * additionally return privateCarriers. Only facts may enter formal reports.
 * Tokens, original request/password, usernames, IDs and receipt tuples remain
 * exclusively in parent-owned private control memory. This module owns no file,
 * database, browser, socket, credentials, clocks, randomness or service process.
 *
 * Owner ports adapt the actual Phase20 browser/collector/canonical SQL helpers:
 * issue() -> current invitation DTO {id,version,status,recipientRef,purpose,...}.
 * verify(id) -> real owner-verified verification UUIDv7.
 * peek(id) -> original collector message plus deliveryState:'DELIVERED'; owner
 *   must wait for the actual auth_delivery_jobs row to be DELIVERED first.
 * revoke({invitationId,expectedVersion}) -> actual invitation DTO.
 * accept(input) -> {status,body,request}. input is {invitationId,token,
 *   verificationRef,operationId?,username?,password?}; first call has only its
 *   first three fields. Owner obtains the actual form's operation and fills
 *   fresh credentials, returning the complete original private request. Replay
 *   sends all six fields unchanged to the public canonical accept endpoint.
 * readReceipt({invitationId,operationId}) -> normalized current canonical tuple:
 *   {operationId,invitationId,verificationId,userId,deliveryTargetId,
 *    deliveryTargetVersion,invitationStatus,activationOperationId,
 *    activatedAccountId,verificationRef,receiptCount,accountCount,
 *    accountStatus,isSuperAdmin,memberships,roles,sessions,grants}.
 *   Counts and authority facts must be actual database observations; do not
 *   synthesize them from a 200 response. AccountCount is for the original user.
 * deny({invitationId,token,verificationRef,previousOperationId?}) ->
 *   {status,operationId,createdAccounts}. Owner generates a fresh operation and
 *   fresh, previously unused candidate username/password, submits the real
 *   public request, and queries the absolute current account count for that
 *   candidate username. createdAccounts must be that count, not a row delta.
 *
 * No new positive account acceptance is performed after the checkpoint.
 * Recovery replay must preserve the canonical receipt while the unselected
 * restored user stays DISABLED. Pure mock tests prove only this orchestration,
 * not actual HTTP, delivery, account creation, backup, recovery or SQL behavior.
 */

const AUTH = 'https://auth.saas.test';
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const REQUEST = ['invitationId', 'operationId', 'token', 'verificationRef', 'username', 'password'];
const IMMUTABLE = ['operationId', 'invitationId', 'verificationId', 'userId', 'deliveryTargetId',
  'deliveryTargetVersion', 'invitationStatus', 'activationOperationId', 'activatedAccountId', 'verificationRef'];
const AUTHORITY = ['memberships', 'roles', 'sessions', 'grants'];
const record = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const id = value => typeof value === 'string' && UUID.test(value);
const version = value => Number.isSafeInteger(value) && value >= 1;
const text = (value, maximum) => typeof value === 'string' && value.length > 0 && value.length <= maximum;
const token = value => typeof value === 'string' && /^[A-Za-z0-9_-]{43}$/.test(value)
  && Buffer.from(value, 'base64url').length === 32 && Buffer.from(value, 'base64url').toString('base64url') === value;

export class InvitationCaseFailure extends Error {
  constructor(code) { super(code); this.name = 'InvitationCaseFailure'; this.code = code; }
}
function need(condition, code) { if (!condition) throw new InvitationCaseFailure(code); }
async function call(action, code) {
  try { return await action(); } catch { throw new InvitationCaseFailure(code); }
}
function portsPresent(ports, names) {
  need(record(ports) && names.every(name => typeof ports[name] === 'function'), 'INVITATION_PORTS_INVALID');
}
function exact(value, keys) {
  return record(value) && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));
}
function request(value) {
  need(exact(value, REQUEST) && id(value.invitationId) && id(value.operationId) && id(value.verificationRef)
    && token(value.token) && text(value.username, 400) && text(value.password, 1024), 'INVITATION_REQUEST_INVALID');
  return Object.fromEntries(REQUEST.map(key => [key, value[key]]));
}
function accepted(response, expectedRequest) {
  need(record(response) && response.status === 200 && exact(response.body, ['operationId', 'invitationId', 'status'])
    && response.body.status === 'ACCOUNT_CREATED', 'INVITATION_ACCEPTANCE_FAILED');
  const original = request(response.request);
  need(response.body.operationId === original.operationId && response.body.invitationId === original.invitationId,
    'INVITATION_ACCEPTANCE_BINDING_MISMATCH');
  need(Object.keys(expectedRequest).every(key => original[key] === expectedRequest[key]), 'INVITATION_REQUEST_CHANGED');
  return original;
}
function receipt(value, original, expectedStatus) {
  need(record(value) && IMMUTABLE.filter(key => key !== 'deliveryTargetVersion' && key !== 'invitationStatus')
    .every(key => id(value[key])) && version(value.deliveryTargetVersion)
    && value.invitationStatus === 'CONSUMED' && value.operationId === original.operationId
    && value.activationOperationId === original.operationId && value.invitationId === original.invitationId
    && value.verificationId === original.verificationRef && value.verificationRef === original.verificationRef
    && value.userId === value.activatedAccountId && value.receiptCount === 1 && value.accountCount === 1
    && value.accountStatus === expectedStatus && value.isSuperAdmin === false
    && AUTHORITY.every(key => value[key] === 0), 'INVITATION_CANONICAL_RECEIPT_MISMATCH');
  return Object.fromEntries([...IMMUTABLE, 'receiptCount', 'accountCount', 'accountStatus', 'isSuperAdmin', ...AUTHORITY]
    .map(key => [key, value[key]]));
}
function carriers(value, phase) {
  need(record(value) && value.contractVersion === 'identity-phase23-invitation-carriers/1' && value.phase === phase
    && record(value.consumed) && record(value.pending), 'INVITATION_CARRIERS_INVALID');
  const consumed = request(value.consumed.request);
  receipt(value.consumed.receipt, consumed, 'ENABLED');
  need(record(value.pending.invitation) && id(value.pending.invitation.id) && version(value.pending.invitation.version)
    && value.pending.invitation.status === 'ACTIVE' && id(value.pending.verificationRef) && token(value.pending.token)
    && value.pending.invitation.id !== consumed.invitationId, 'INVITATION_CARRIERS_INVALID');
  if (phase === 'AFTER_CHECKPOINT') need(version(value.pending.revokedVersion)
    && value.pending.revokedVersion === value.pending.invitation.version + 1
    && id(value.pending.deniedOperationId) && value.pending.deniedOperationId !== consumed.operationId,
  'INVITATION_POSTCHECKPOINT_BINDING_MISSING');
  return structuredClone(value);
}
async function delivered(ports) {
  const invitation = await call(() => ports.issue(), 'INVITATION_ISSUE_FAILED');
  need(record(invitation) && id(invitation.id) && version(invitation.version) && invitation.status === 'ACTIVE'
    && invitation.purpose === 'ACCOUNT_ONBOARDING' && text(invitation.recipientRef, 256)
    && !Object.hasOwn(invitation, 'token'), 'INVITATION_ISSUE_RESULT_INVALID');
  const verificationRef = await call(() => ports.verify(invitation.id), 'INVITATION_VERIFICATION_FAILED');
  need(id(verificationRef), 'INVITATION_VERIFICATION_INVALID');
  const material = await call(() => ports.peek(invitation.id), 'INVITATION_DELIVERY_FAILED');
  need(record(material) && material.deliveryState === 'DELIVERED' && material.invitationId === invitation.id
    && material.verificationRef === verificationRef && material.recipientRef === invitation.recipientRef
    && material.acceptUrl === `${AUTH}/sso/invitations/${invitation.id}` && token(material.token),
  'INVITATION_DELIVERY_BINDING_MISMATCH');
  return { invitation: { id: invitation.id, version: invitation.version, status: 'ACTIVE' },
    verificationRef, token: material.token };
}
async function denied(ports, source, previousOperationId, used) {
  const input = { invitationId: source.invitationId, token: source.token, verificationRef: source.verificationRef };
  if (previousOperationId !== undefined) input.previousOperationId = previousOperationId;
  const result = await call(() => ports.deny(input), 'INVITATION_DENIAL_FAILED');
  need(record(result) && result.status === 409 && id(result.operationId) && !used.has(result.operationId)
    && result.operationId !== previousOperationId && result.createdAccounts === 0, 'INVITATION_OLD_TOKEN_NOT_DENIED');
  used.add(result.operationId); return result.operationId;
}

export async function runBeforeCheckpoint(ports) {
  portsPresent(ports, ['issue', 'verify', 'peek', 'accept', 'readReceipt']);
  const consumed = await delivered(ports);
  const input = { invitationId: consumed.invitation.id, token: consumed.token, verificationRef: consumed.verificationRef };
  const response = await call(() => ports.accept({ ...input }), 'INVITATION_INITIAL_ACCEPT_FAILED');
  const original = accepted(response, input);
  const observed = receipt(await call(() => ports.readReceipt({ invitationId: original.invitationId,
    operationId: original.operationId }), 'INVITATION_INITIAL_RECEIPT_FAILED'), original, 'ENABLED');
  const pending = await delivered(ports);
  need(pending.invitation.id !== original.invitationId, 'INVITATION_DISTINCT_PENDING_REQUIRED');
  return { facts: [{ caseId: 'DR_INVITATION_CHECKPOINT', status: 'PASS', consumedInvitations: 1,
    pendingDeliveredInvitations: 1, activationReceipts: 1, accountsCreated: 1, businessAuthoritiesCreated: 0 }],
  privateCarriers: { contractVersion: 'identity-phase23-invitation-carriers/1', phase: 'BEFORE_CHECKPOINT',
    consumed: { request: original, receipt: observed }, pending } };
}

export async function runAfterCheckpoint(ports, privateCarriers) {
  portsPresent(ports, ['revoke', 'deny']);
  const current = carriers(privateCarriers, 'BEFORE_CHECKPOINT'), pending = current.pending;
  const response = await call(() => ports.revoke({ invitationId: pending.invitation.id,
    expectedVersion: pending.invitation.version }), 'INVITATION_POSTCHECKPOINT_REVOKE_FAILED');
  need(record(response) && response.id === pending.invitation.id && response.status === 'REVOKED'
    && response.version === pending.invitation.version + 1, 'INVITATION_REVOKE_RESULT_MISMATCH');
  pending.deniedOperationId = await denied(ports, { invitationId: pending.invitation.id, token: pending.token,
    verificationRef: pending.verificationRef }, undefined, new Set([current.consumed.request.operationId]));
  pending.revokedVersion = response.version; current.phase = 'AFTER_CHECKPOINT';
  return { facts: [{ caseId: 'DR_INVITATION_POSTCHECKPOINT_REVOKE', status: 'PASS', revokedInvitations: 1,
    oldTokenDenied: 1, newAccounts: 0 }], privateCarriers: current };
}

export async function runAfterRecovery(ports, privateCarriers) {
  portsPresent(ports, ['accept', 'readReceipt', 'deny']);
  const current = carriers(privateCarriers, 'AFTER_CHECKPOINT'), original = current.consumed.request;
  const response = await call(() => ports.accept({ ...original }), 'INVITATION_ORIGINAL_REPLAY_FAILED');
  accepted(response, original);
  const restored = receipt(await call(() => ports.readReceipt({ invitationId: original.invitationId,
    operationId: original.operationId }), 'INVITATION_RESTORED_RECEIPT_FAILED'), original, 'DISABLED');
  need(IMMUTABLE.every(key => restored[key] === current.consumed.receipt[key]), 'INVITATION_RESTORED_TUPLE_CHANGED');
  const used = new Set([original.operationId, current.pending.deniedOperationId]);
  await denied(ports, original, original.operationId, used);
  await denied(ports, { invitationId: current.pending.invitation.id, token: current.pending.token,
    verificationRef: current.pending.verificationRef }, current.pending.deniedOperationId, used);
  return { facts: [{ caseId: 'DR_INVITATION_ORIGINAL_RECEIPT', status: 'PASS', sameOperationStatus: 200,
    immutableTuplePreserved: true, receiptCount: 1, accountCount: 1, restoredAccountDisabled: true,
    businessAuthoritiesCreated: 0 }, { caseId: 'DR_INVITATION_OLD_TOKENS', status: 'PASS', freshOperations: 2,
    consumedStatus: 409, revokedStatus: 409, newAccounts: 0 }] };
}

if (process.env.NODE_TEST_CONTEXT) {
  const { test } = await import('node:test');
  const assert = (await import('node:assert/strict')).default;
  const CANARY = 'PRIVATE_INVITATION_CANARY';
  function fixture(options = {}) {
    let sequence = 0, recovered = false, accountCount = 0;
    const newId = () => `00000000-0000-7000-8000-${String(++sequence).padStart(12, '0')}`;
    const target = newId(), user = newId(), recipientRef = CANARY + '-recipient';
    const state = new Map(), calls = [], requests = new Map();
    const ports = {
      issue: async () => {
        calls.push(['issue']); const invitation = { id: newId(), version: 1, status: 'ACTIVE',
          recipientRef, purpose: 'ACCOUNT_ONBOARDING' };
        state.set(invitation.id, { ...invitation, token: Buffer.alloc(32, state.size + 1).toString('base64url') });
        return { ...invitation };
      },
      verify: async invitationId => { calls.push(['verify', invitationId]); const proof = newId();
        state.get(invitationId).verificationRef = proof; return proof; },
      peek: async invitationId => {
        calls.push(['peek', invitationId]); const row = state.get(invitationId);
        return { invitationId, verificationRef: options.badProof ? newId() : row.verificationRef, recipientRef,
          acceptUrl: `${AUTH}/sso/invitations/${invitationId}`, token: row.token,
          deliveryState: options.notDelivered ? 'PENDING' : 'DELIVERED' };
      },
      accept: async input => {
        calls.push(['accept', structuredClone(input)]);
        const submitted = input.operationId ? { ...input } : { ...input, operationId: newId(),
          username: CANARY + '-username', password: CANARY + '-password' };
        const row = state.get(input.invitationId), prior = requests.get(submitted.operationId);
        if (!prior) {
          assert.equal(recovered, false, 'there must be no new positive target activation');
          assert.equal(row.status, 'ACTIVE'); row.status = 'CONSUMED'; accountCount += 1;
          requests.set(submitted.operationId, { ...submitted });
        } else assert.deepEqual(submitted, prior);
        return { status: options.replayDisabledFails && recovered ? 409 : 200,
          body: { operationId: options.wrongResponseOperation ? newId() : submitted.operationId,
            invitationId: input.invitationId, status: 'ACCOUNT_CREATED' }, request: submitted };
      },
      readReceipt: async input => {
        calls.push(['readReceipt', { ...input }]); const original = requests.get(input.operationId);
        return { operationId: original.operationId, invitationId: original.invitationId,
          verificationId: original.verificationRef, userId: options.changedUser && recovered ? newId() : user,
          deliveryTargetId: target, deliveryTargetVersion: 1, invitationStatus: 'CONSUMED',
          activationOperationId: original.operationId, activatedAccountId: options.changedUser && recovered ? '00000000-0000-7000-8000-000000000999' : user,
          verificationRef: original.verificationRef, receiptCount: options.doubleReceipt ? 2 : 1,
          accountCount, accountStatus: recovered && !options.targetEnabled ? 'DISABLED' : 'ENABLED',
          isSuperAdmin: false, memberships: 0, roles: options.businessRole ? 1 : 0, sessions: 0, grants: 0 };
      },
      revoke: async input => {
        calls.push(['revoke', { ...input }]); const row = state.get(input.invitationId);
        assert.equal(row.version, input.expectedVersion); row.status = 'REVOKED'; row.version += 1;
        return { id: row.id, version: row.version, status: options.revokeStaysActive ? 'ACTIVE' : row.status };
      },
      deny: async input => {
        calls.push(['deny', structuredClone(input)]); const row = state.get(input.invitationId);
        return { status: options.wrongDenial ? 400 : ['CONSUMED', 'REVOKED'].includes(row.status) ? 409 : 200,
          operationId: options.reusedOperation ? [...requests.keys()][0] : newId(),
          createdAccounts: options.createdByDenial ? 1 : 0 };
      },
    };
    return { ports, calls, state, recover() { recovered = true; }, get accountCount() { return accountCount; } };
  }
  async function beforeAndAfter(mock) {
    const before = await runBeforeCheckpoint(mock.ports);
    const after = await runAfterCheckpoint(mock.ports, before.privateCarriers);
    return { before, after };
  }
  test('mock lifecycle consumes one before checkpoint, revokes pending after, and preserves the original receipt', async () => {
    const mock = fixture(), { before, after } = await beforeAndAfter(mock);
    assert.equal(before.privateCarriers.phase, 'BEFORE_CHECKPOINT');
    assert.equal(after.privateCarriers.phase, 'AFTER_CHECKPOINT');
    assert.equal(before.privateCarriers.pending.deniedOperationId, undefined);
    mock.recover(); const target = await runAfterRecovery(mock.ports, after.privateCarriers);
    assert.equal(mock.accountCount, 1); assert.equal(mock.calls.filter(call => call[0] === 'issue').length, 2);
    const accepts = mock.calls.filter(call => call[0] === 'accept'); assert.equal(accepts.length, 2);
    assert.equal(accepts[0][1].operationId, undefined);
    assert.deepEqual(accepts[1][1], before.privateCarriers.consumed.request);
    assert.equal(mock.calls.filter(call => call[0] === 'deny').length, 3);
    assert.equal(JSON.stringify([...before.facts, ...after.facts, ...target.facts]).includes(CANARY), false);
    assert.equal(JSON.stringify(target).includes('privateCarriers'), false);
    assert.equal(target.facts[0].restoredAccountDisabled, true);
  });
  test('delivery binding and actual durable delivered state are required before consumption', async () => {
    for (const option of ['badProof', 'notDelivered']) {
      const mock = fixture({ [option]: true });
      await assert.rejects(runBeforeCheckpoint(mock.ports), { code: 'INVITATION_DELIVERY_BINDING_MISMATCH' });
      assert.equal(mock.accountCount, 0);
    }
  });
  test('receipt cardinality, returned operation and no business authority are mandatory', async () => {
    for (const [option, code] of [['doubleReceipt', 'INVITATION_CANONICAL_RECEIPT_MISMATCH'],
      ['businessRole', 'INVITATION_CANONICAL_RECEIPT_MISMATCH'], ['wrongResponseOperation', 'INVITATION_ACCEPTANCE_BINDING_MISMATCH']]) {
      await assert.rejects(runBeforeCheckpoint(fixture({ [option]: true }).ports), { code });
    }
  });
  test('post-checkpoint stage cannot run a new positive acceptance or mutate the original carrier', async () => {
    const mock = fixture(), before = await runBeforeCheckpoint(mock.ports), snapshot = structuredClone(before.privateCarriers);
    await runAfterCheckpoint(mock.ports, before.privateCarriers);
    assert.deepEqual(before.privateCarriers, snapshot);
    assert.equal(mock.calls.filter(call => call[0] === 'accept').length, 1);
    await assert.rejects(runAfterRecovery(mock.ports, before.privateCarriers), { code: 'INVITATION_CARRIERS_INVALID' });
  });
  test('revocation and fresh denial must reject with no candidate account rows', async () => {
    for (const [option, code] of [['revokeStaysActive', 'INVITATION_REVOKE_RESULT_MISMATCH'],
      ['wrongDenial', 'INVITATION_OLD_TOKEN_NOT_DENIED'], ['reusedOperation', 'INVITATION_OLD_TOKEN_NOT_DENIED'],
      ['createdByDenial', 'INVITATION_OLD_TOKEN_NOT_DENIED']]) {
      const mock = fixture({ [option]: true }), before = await runBeforeCheckpoint(mock.ports);
      await assert.rejects(runAfterCheckpoint(mock.ports, before.privateCarriers), { code });
    }
  });
  test('disabled target user still requires real canonical same-operation 200, with no weakened fallback', async () => {
    const mock = fixture({ replayDisabledFails: true }), { after } = await beforeAndAfter(mock); mock.recover();
    await assert.rejects(runAfterRecovery(mock.ports, after.privateCarriers), { code: 'INVITATION_ACCEPTANCE_FAILED' });
  });
  test('target re-enablement or changed original receipt tuple fails closed', async () => {
    for (const option of ['targetEnabled', 'changedUser']) {
      const mock = fixture({ [option]: true }), { after } = await beforeAndAfter(mock); mock.recover();
      await assert.rejects(runAfterRecovery(mock.ports, after.privateCarriers), { code: 'INVITATION_CANONICAL_RECEIPT_MISMATCH' });
    }
  });
  test('port exceptions never expose secret material and missing ports fail explicitly', async () => {
    await assert.rejects(runBeforeCheckpoint({}), { code: 'INVITATION_PORTS_INVALID' });
    const mock = fixture(); mock.ports.issue = async () => { throw new Error(CANARY); };
    try { await runBeforeCheckpoint(mock.ports); assert.fail('must reject'); }
    catch (error) { assert.equal(error.code, 'INVITATION_ISSUE_FAILED'); assert.equal(String(error).includes(CANARY), false); }
  });
}
