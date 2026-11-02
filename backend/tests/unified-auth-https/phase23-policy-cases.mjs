/** Injected current-API policy recovery cases; imports start no environment.
 *
 * Exports runBeforeCheckpoint(ports), runAfterCheckpoint(ports,privateCarriers),
 * runAfterRecovery(ports,privateCarriers). Return {facts,privateCarriers} in the
 * first two stages and {facts} after recovery. Only facts belong in reports.
 * The extra client secrets/tokens stay in independent caller control memory;
 * never put them in shared environment, source files, subprocess args or logs.
 *
 * ports.bindings = {applicationId,retireMemberId,retireUserId,
 *   protectedClientIds,protectedMembershipIds,protectedUserIds}.
 * applicationId must be a dedicated APPROVED fixture projection approving both
 * existing ACTIVE p21-demo scopes demo.read/demo.admin before this module runs.
 * Original Phase21 machineApp approves only demo.read: it is insufficient for
 * a real contraction while retaining the mandatory nonempty resource scope.
 * No application, original client, service or root member is mutated here.
 *
 * command({method,path,body}) -> {status,body}: real canonical platform API,
 *   including current employee/MFA/CSRF/intent and original-result recovery.
 * machine({clientId,clientSecret,scope}) -> {status,body}: independent external
 *   real client_credentials request; body is OAuth JSON, never emitted here.
 * resource({accessToken}) -> {status}: real approved demo resource request.
 * introspect({accessToken}) -> {status,active}: real resource-side credential,
 *   using its current rearmed credential after recovery, not the extra secret.
 * readFacts({clientId,memberId,userId}) -> canonical current SQL observations:
 *   {client:{id,clientId,applicationId,status,version,policyVersion,permissions:
 *      [{resourceKey,scopes}],credentials:[{id,credentialVersion,revoked,validNow}],
 *      liveCredentialCount},member:{id,userId,exists,status?,identityVersion?},
 *    user:{id,status,sessionVersion}}.
 * recoveryGate({clientId,memberId,userId}) -> authenticated recovery constraints
 *   {credentialFloor,memberIdentityFloor,userSessionFloor,
 *    rearmedClient:false,rearmedUser:false,rearmedMember:false}.
 * credentialFloor comes from real requirements.credential_floors: a credential
 * created only after checkpoint need not have a physical restored row. Never
 * synthesize a physical max credential version to stand in for this constraint.
 *
 * Source order: create extra, issue first credential, MACHINE/resource200;
 * after checkpoint rotate (ordinary rotation permits <=24h overlap), revoke all,
 * prove old secrets401/403 and same old opaque token inactive BEFORE disabling;
 * then shrink scopes and disable only extra, delete only the nominated member.
 * Recovery must keep extra/user/member quarantined, preserve floors and scope
 * contraction and reject original secrets/token. No rearm is requested here.
 * Pure tests are mock orchestration, not actual API/SQL/recovery evidence.
 */

const ROOT = '/api/identity/clients';
const RESOURCE = 'p21-demo', REQUIRED = 'demo.read', OPTIONAL = 'demo.admin';
const REASON = 'PHASE23_OWNED_RECOVERY_RETIREMENT';
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const record = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const id = value => typeof value === 'string' && UUID.test(value);
const version = value => Number.isSafeInteger(value) && value >= 1;
const secret = value => typeof value === 'string' && /^[0-9a-f]{64}$/.test(value);
const opaque = value => typeof value === 'string' && /^[\x21-\x7e]{1,16384}$/.test(value);

export class PolicyCaseFailure extends Error {
  constructor(code) { super(code); this.name = 'PolicyCaseFailure'; this.code = code; }
}
function need(condition, code) { if (!condition) throw new PolicyCaseFailure(code); }
async function call(action, code) {
  try { return await action(); } catch { throw new PolicyCaseFailure(code); }
}
function checkPorts(ports, names) {
  need(record(ports) && names.every(name => typeof ports[name] === 'function'), 'POLICY_PORTS_INVALID');
}
function bindings(ports) {
  const value = ports.bindings;
  need(record(value) && ['applicationId', 'retireMemberId', 'retireUserId'].every(key => id(value[key]))
    && ['protectedClientIds', 'protectedMembershipIds', 'protectedUserIds'].every(key => Array.isArray(value[key])
      && value[key].length > 0 && value[key].every(id) && new Set(value[key]).size === value[key].length)
    && !value.protectedMembershipIds.includes(value.retireMemberId)
    && !value.protectedUserIds.includes(value.retireUserId), 'POLICY_BINDING_INVALID');
  return structuredClone(value);
}
function registration(applicationId, expectedVersion, narrowed = false) {
  return { expectedVersion, applicationId, environment: 'test', displayName: 'Phase 23 owned recovery extra',
    clientType: 'CONFIDENTIAL', runtimeKind: 'SERVICE', subjectMode: 'MACHINE', contextMode: 'NONE',
    allowedMemberTypes: [], grantTypes: ['client_credentials'], tokenEndpointAuthMethod: 'client_secret_basic',
    redirectUris: [], postLogoutRedirectUris: [], logoutMode: 'SHORT_SESSION_ONLINE_CHECK',
    backchannelLogoutUri: null, backchannelTargetId: null, status: narrowed ? 'DISABLED' : 'ENABLED',
    consentGroupId: null, permissions: [{ resourceKey: RESOURCE, scopes: narrowed ? [REQUIRED] : [REQUIRED, OPTIONAL] }] };
}
function scopes(value, narrowed) {
  const expected = narrowed ? [REQUIRED] : [REQUIRED, OPTIONAL];
  return Array.isArray(value) && value.length === 1 && record(value[0]) && value[0].resourceKey === RESOURCE
    && Array.isArray(value[0].scopes) && value[0].scopes.length === expected.length
    && new Set(value[0].scopes).size === expected.length && expected.every(scope => value[0].scopes.includes(scope));
}
async function command(ports, method, path, body) {
  const value = await call(() => ports.command({ method, path, body }), 'POLICY_COMMAND_FAILED');
  need(record(value) && value.status === 200 && record(value.body), 'POLICY_COMMAND_REJECTED');
  return value.body;
}
function clientDto(value, binding, clientId, narrowed) {
  need(record(value) && id(value.id) && value.clientId === value.id && value.applicationId === binding.applicationId
    && (clientId === undefined || value.id === clientId) && !binding.protectedClientIds.includes(value.id)
    && value.status === (narrowed ? 'DISABLED' : 'ENABLED') && version(value.version) && version(value.policyVersion)
    && scopes(value.permissions, narrowed), 'POLICY_CLIENT_RESULT_MISMATCH');
}
function credential(value, clientId, expectedClientVersion, minimumCredentialVersion) {
  need(record(value) && value.clientId === clientId && id(value.credentialId)
    && value.clientVersion === expectedClientVersion + 1 && version(value.credentialVersion)
    && value.credentialVersion > minimumCredentialVersion && secret(value.clientSecret), 'POLICY_CREDENTIAL_RESULT_MISMATCH');
  return { id: value.credentialId, credentialVersion: value.credentialVersion, clientSecret: value.clientSecret };
}
function query(carriers) {
  return { clientId: carriers.clientId, memberId: carriers.binding.retireMemberId, userId: carriers.binding.retireUserId };
}
function checkFacts(value, current, { narrowed, memberExists, accountStatus }) {
  need(record(value) && record(value.client) && record(value.member) && record(value.user), 'POLICY_FACTS_INVALID');
  const { client, member, user } = value;
  clientDto(client, current.binding, current.clientId, narrowed);
  need(Array.isArray(client.credentials) && client.credentials.every(row => record(row) && id(row.id)
    && version(row.credentialVersion) && typeof row.revoked === 'boolean' && typeof row.validNow === 'boolean')
    && new Set(client.credentials.map(row => row.id)).size === client.credentials.length
    && Number.isSafeInteger(client.liveCredentialCount) && client.liveCredentialCount >= 0
    && client.liveCredentialCount === client.credentials.filter(row => row.validNow).length
    && member.id === current.binding.retireMemberId && member.userId === current.binding.retireUserId
    && member.exists === memberExists && (!memberExists || (version(member.identityVersion)
      && member.status === (accountStatus === 'DISABLED' ? 'DISABLED' : 'ENABLED')))
    && user.id === current.binding.retireUserId && user.status === accountStatus && version(user.sessionVersion), 'POLICY_FACTS_INVALID');
  return structuredClone(value);
}
function allRevoked(facts, known, requireAllRows) {
  const rows = facts.client.credentials;
  need(facts.client.liveCredentialCount === 0 && rows.every(row => row.revoked && !row.validNow)
    && known.every((source, index) => {
      const match = rows.find(row => row.id === source.id);
      return match ? match.credentialVersion === source.credentialVersion && match.revoked : !requireAllRows && index > 0;
    }), 'POLICY_CREDENTIALS_RESTORED_LIVE');
}
function privateState(value, phase) {
  need(record(value) && value.contractVersion === 'identity-phase23-policy-carriers/1' && value.phase === phase
    && record(value.binding) && id(value.clientId) && value.publicClientId === value.clientId
    && opaque(value.accessToken) && Array.isArray(value.credentials) && value.credentials.length === (phase === 'BEFORE_CHECKPOINT' ? 1 : 2)
    && value.credentials.every(row => record(row) && id(row.id) && version(row.credentialVersion) && secret(row.clientSecret))
    && record(value.highwater) && version(value.highwater.clientVersion) && version(value.highwater.policyVersion)
    && version(value.highwater.credentialVersion) && version(value.highwater.memberIdentityVersion)
    && version(value.highwater.userSessionVersion), 'POLICY_CARRIERS_INVALID');
  bindings({ bindings: value.binding });
  need(!value.binding.protectedClientIds.includes(value.clientId), 'POLICY_PROTECTED_CLIENT_SELECTED');
  return structuredClone(value);
}
async function rejectOld(ports, current) {
  const statuses = [];
  for (const source of current.credentials) {
    const value = await call(() => ports.machine({ clientId: current.publicClientId,
      clientSecret: source.clientSecret, scope: REQUIRED }), 'POLICY_OLD_CREDENTIAL_REQUEST_FAILED');
    need(record(value) && [401, 403].includes(value.status), 'POLICY_OLD_CREDENTIAL_NOT_REJECTED');
    statuses.push(value.status);
  }
  const value = await call(() => ports.introspect({ accessToken: current.accessToken }), 'POLICY_OLD_TOKEN_INTROSPECTION_FAILED');
  need(record(value) && value.status === 200 && value.active === false, 'POLICY_OLD_TOKEN_RESTORED_ACTIVE');
  return statuses;
}

export async function runBeforeCheckpoint(ports) {
  checkPorts(ports, ['command', 'machine', 'resource', 'readFacts']);
  const binding = bindings(ports);
  const created = await command(ports, 'POST', ROOT, registration(binding.applicationId, null));
  clientDto(created, binding, undefined, false);
  const issued = credential(await command(ports, 'POST', `${ROOT}/${created.id}/credentials`,
    { expectedVersion: created.version }), created.id, created.version, 0);
  const machine = await call(() => ports.machine({ clientId: created.clientId,
    clientSecret: issued.clientSecret, scope: REQUIRED }), 'POLICY_MACHINE_REQUEST_FAILED');
  need(record(machine) && machine.status === 200 && record(machine.body) && opaque(machine.body.access_token)
    && machine.body.token_type?.toLowerCase() === 'bearer' && Number.isFinite(machine.body.expires_in)
    && machine.body.expires_in > 0 && !machine.body.id_token && !machine.body.refresh_token, 'POLICY_INITIAL_MACHINE_REJECTED');
  const resource = await call(() => ports.resource({ accessToken: machine.body.access_token }), 'POLICY_RESOURCE_REQUEST_FAILED');
  need(record(resource) && resource.status === 200, 'POLICY_INITIAL_RESOURCE_REJECTED');
  const current = { contractVersion: 'identity-phase23-policy-carriers/1', phase: 'BEFORE_CHECKPOINT', binding,
    clientId: created.id, publicClientId: created.clientId, credentials: [issued], accessToken: machine.body.access_token };
  const observed = checkFacts(await call(() => ports.readFacts(query(current)), 'POLICY_INITIAL_FACTS_FAILED'), current,
    { narrowed: false, memberExists: true, accountStatus: 'ENABLED' });
  need(observed.client.version === created.version + 1 && observed.client.policyVersion === created.policyVersion
    && observed.client.liveCredentialCount === 1 && observed.client.credentials.some(row => row.id === issued.id
      && row.credentialVersion === issued.credentialVersion && !row.revoked && row.validNow), 'POLICY_INITIAL_CREDENTIAL_FACTS_MISMATCH');
  current.highwater = { clientVersion: observed.client.version, policyVersion: observed.client.policyVersion,
    credentialVersion: Math.max(...observed.client.credentials.map(row => row.credentialVersion)),
    memberIdentityVersion: observed.member.identityVersion, userSessionVersion: observed.user.sessionVersion };
  return { facts: [{ caseId: 'DR_POLICY_CHECKPOINT', status: 'PASS', dedicatedClients: 1,
    credentialVersions: 1, machineStatus: 200, resourceStatus: 200, retainedRequiredScope: REQUIRED,
    optionalScopeBeforeContraction: OPTIONAL }], privateCarriers: current };
}

export async function runAfterCheckpoint(ports, privateCarriers) {
  checkPorts(ports, ['command', 'machine', 'introspect', 'readFacts']);
  const current = privateState(privateCarriers, 'BEFORE_CHECKPOINT');
  const next = credential(await command(ports, 'POST', `${ROOT}/${current.clientId}/credentials`,
    { expectedVersion: current.highwater.clientVersion }), current.clientId,
  current.highwater.clientVersion, current.credentials[0].credentialVersion);
  need(next.id !== current.credentials[0].id && next.clientSecret !== current.credentials[0].clientSecret,
    'POLICY_ROTATED_CREDENTIAL_NOT_NEW');
  current.credentials.push(next);
  const revoked = await command(ports, 'POST', `${ROOT}/${current.clientId}/credentials/revoke`,
    { expectedVersion: current.highwater.clientVersion + 1 });
  need(revoked.clientId === current.clientId && Number.isSafeInteger(revoked.revokedCredentials)
    && revoked.revokedCredentials >= 2 && revoked.version === current.highwater.clientVersion + 2
    && revoked.policyVersion === current.highwater.policyVersion + 1, 'POLICY_REVOKE_RESULT_MISMATCH');
  // Prove emergency revocation while client is still enabled, before the later
  // disabling status could independently cause old-credential rejection.
  const rejectionStatuses = await rejectOld(ports, current);
  const enabled = checkFacts(await call(() => ports.readFacts(query(current)), 'POLICY_REVOKED_FACTS_FAILED'), current,
    { narrowed: false, memberExists: true, accountStatus: 'ENABLED' });
  allRevoked(enabled, current.credentials, true);
  need(enabled.client.version === revoked.version && enabled.client.policyVersion === revoked.policyVersion,
    'POLICY_REVOKED_VERSION_MISMATCH');
  const narrowed = await command(ports, 'PATCH', `${ROOT}/${current.clientId}`,
    registration(current.binding.applicationId, revoked.version, true));
  clientDto(narrowed, current.binding, current.clientId, true);
  need(narrowed.version === revoked.version + 1 && narrowed.policyVersion === revoked.policyVersion + 1,
    'POLICY_CONTRACTION_VERSION_MISMATCH');
  const removed = await command(ports, 'DELETE', `/api/memberships/${current.binding.retireMemberId}`,
    { expectedIdentityVersion: current.highwater.memberIdentityVersion, reason: REASON });
  need(removed.id === current.binding.retireMemberId && removed.deleted === true
    && removed.identityVersion === current.highwater.memberIdentityVersion + 1, 'POLICY_MEMBER_RETIRE_RESULT_MISMATCH');
  const observed = checkFacts(await call(() => ports.readFacts(query(current)), 'POLICY_POSTCHECKPOINT_FACTS_FAILED'), current,
    { narrowed: true, memberExists: false, accountStatus: 'ENABLED' });
  allRevoked(observed, current.credentials, true);
  need(observed.client.version === narrowed.version && observed.client.policyVersion === narrowed.policyVersion
    && observed.user.sessionVersion >= current.highwater.userSessionVersion, 'POLICY_POSTCHECKPOINT_VERSION_MISMATCH');
  current.highwater = { clientVersion: observed.client.version, policyVersion: observed.client.policyVersion,
    credentialVersion: Math.max(current.highwater.credentialVersion, ...observed.client.credentials.map(row => row.credentialVersion)),
    memberIdentityVersion: removed.identityVersion, userSessionVersion: observed.user.sessionVersion };
  current.phase = 'AFTER_CHECKPOINT';
  return { facts: [{ caseId: 'DR_POLICY_POSTCHECKPOINT', status: 'PASS', rotatedCredentialVersions: 2,
    revokedCredentials: revoked.revokedCredentials, oldCredentialStatuses: rejectionStatuses,
    sameOldTokenActive: false, rejectionProvedBeforeDisable: true, retainedScope: REQUIRED, removedScope: OPTIONAL,
    extraClientDisabled: true, retiredMemberships: 1, retiredMemberIdentityVersion: removed.identityVersion }],
  privateCarriers: current };
}

export async function runAfterRecovery(ports, privateCarriers) {
  checkPorts(ports, ['machine', 'introspect', 'readFacts', 'recoveryGate']);
  const current = privateState(privateCarriers, 'AFTER_CHECKPOINT');
  const gate = await call(() => ports.recoveryGate(query(current)), 'POLICY_RECOVERY_GATE_FAILED');
  const credentialFloor = Math.max(current.highwater.credentialVersion, ...current.credentials.map(row => row.credentialVersion));
  need(record(gate) && version(gate.credentialFloor) && gate.credentialFloor >= credentialFloor
    && version(gate.memberIdentityFloor) && gate.memberIdentityFloor >= current.highwater.memberIdentityVersion
    && version(gate.userSessionFloor) && gate.userSessionFloor > current.highwater.userSessionVersion
    && gate.rearmedClient === false && gate.rearmedUser === false && gate.rearmedMember === false, 'POLICY_RECOVERY_GATE_MISMATCH');
  const observed = checkFacts(await call(() => ports.readFacts(query(current)), 'POLICY_RESTORED_FACTS_FAILED'), current,
    { narrowed: true, memberExists: true, accountStatus: 'DISABLED' });
  allRevoked(observed, current.credentials, false);
  need(observed.client.version > current.highwater.clientVersion
    && observed.client.policyVersion > current.highwater.policyVersion
    && observed.member.identityVersion >= gate.memberIdentityFloor
    && observed.user.sessionVersion >= gate.userSessionFloor, 'POLICY_RESTORED_HIGHWATER_LOST');
  const rejectionStatuses = await rejectOld(ports, current);
  return { facts: [{ caseId: 'DR_POLICY_RESTORED_QUARANTINE', status: 'PASS', extraClientDisabled: true,
    retiredUserDisabled: true, retiredMembershipDisabled: true, rearmedExtraAuthorities: 0,
    retainedScope: REQUIRED, removedScope: OPTIONAL, oldCredentialStatuses: rejectionStatuses,
    sameOldTokenActive: false, credentialHighwater: gate.credentialFloor,
    memberIdentityHighwater: observed.member.identityVersion, userSessionHighwater: observed.user.sessionVersion }] };
}

if (process.env.NODE_TEST_CONTEXT) {
  const { test } = await import('node:test');
  const assert = (await import('node:assert/strict')).default;
  const CANARY = 'PRIVATE_POLICY_CANARY';
  function fixture(options = {}) {
    let sequence = 0, recovered = false, access;
    const newId = () => `00000000-0000-7000-8000-${String(++sequence).padStart(12, '0')}`;
    const binding = { applicationId: newId(), retireMemberId: newId(), retireUserId: newId(),
      protectedClientIds: [newId(), newId(), newId(), newId()], protectedMembershipIds: [newId()], protectedUserIds: [newId()] };
    const member = { id: binding.retireMemberId, userId: binding.retireUserId, exists: true, status: 'ENABLED', identityVersion: 1 };
    const user = { id: binding.retireUserId, status: 'ENABLED', sessionVersion: 1 };
    const calls = [], issued = []; let client;
    const ports = {
      bindings: binding,
      command: async input => {
        calls.push(['command', structuredClone(input)]);
        let body;
        if (input.path === ROOT) {
          const clientId = options.protectedClient ? binding.protectedClientIds[0] : newId();
          client = { id: clientId, clientId, applicationId: binding.applicationId, status: 'ENABLED', version: 1,
            policyVersion: 1, permissions: structuredClone(input.body.permissions), credentials: [], liveCredentialCount: 0 };
          body = { ...client };
        } else if (input.path.endsWith('/credentials')) {
          assert.equal(input.body.expectedVersion, client.version);
          const number = issued.length + 1, row = { id: newId(), credentialVersion: number, revoked: false, validNow: true };
          issued.push({ ...row, clientSecret: String(number).repeat(64) }); client.credentials.push(row);
          client.version += 1; client.liveCredentialCount += 1;
          body = { clientId: client.id, credentialId: row.id, credentialVersion: number,
            clientVersion: client.version, clientSecret: issued.at(-1).clientSecret };
        } else if (input.path.endsWith('/credentials/revoke')) {
          assert.equal(input.body.expectedVersion, client.version);
          for (const row of client.credentials) { row.revoked = true; row.validNow = false; }
          client.liveCredentialCount = 0; client.version += 1; client.policyVersion += 1;
          body = { clientId: client.id, revokedCredentials: 2, version: client.version, policyVersion: client.policyVersion };
        } else if (input.method === 'PATCH') {
          assert.equal(input.body.expectedVersion, client.version);
          client.permissions = structuredClone(input.body.permissions); client.status = input.body.status;
          client.version += 1; client.policyVersion += 1; body = { ...client };
        } else if (input.method === 'DELETE') {
          assert.equal(input.body.expectedIdentityVersion, member.identityVersion);
          assert.equal(input.body.reason, REASON); member.exists = false; member.identityVersion += 1;
          body = { id: member.id, deleted: true, identityVersion: member.identityVersion };
        }
        return { status: 200, body: structuredClone(body) };
      },
      machine: async input => {
        calls.push(['machine', { ...input, clientSecret: '[private]' }]);
        const known = issued.find(row => row.clientSecret === input.clientSecret);
        const live = client.credentials.find(row => row.id === known?.id && !row.revoked);
        if (!live || recovered) return { status: options.denialAs400 ? 400 : 401, body: { error: CANARY } };
        access = CANARY + '-opaque';
        return { status: 200, body: { access_token: access, token_type: 'Bearer', expires_in: 600 } };
      },
      resource: async input => { calls.push(['resource']); assert.equal(input.accessToken, access); return { status: 200 }; },
      introspect: async input => { calls.push(['introspect']); assert.equal(input.accessToken, access);
        return { status: 200, active: options.tokenRevives || client.liveCredentialCount > 0 }; },
      readFacts: async () => {
        calls.push(['readFacts']); const result = structuredClone({ client, member, user });
        if (options.observedHistoricalCredential && !recovered && client.credentials.length >= 2
            && client.liveCredentialCount === 0) result.client.credentials.push({
          id: '00000000-0000-7000-8000-000000001717', credentialVersion: 17, revoked: true, validNow: false });
        if (recovered && options.scopeRevives) result.client.permissions[0].scopes.push(OPTIONAL);
        if (recovered && options.memberRevives) result.member.status = 'ENABLED';
        if (recovered && options.userRevives) result.user.status = 'ENABLED';
        if (recovered && options.floorRollsBack) result.member.identityVersion = 1;
        return result;
      },
      recoveryGate: async () => ({ credentialFloor: options.credentialFloorLost ? 1 : 2,
        memberIdentityFloor: 2, userSessionFloor: 3, rearmedClient: options.rearmed === true,
        rearmedUser: false, rearmedMember: false }),
    };
    return { ports, calls, recover() {
      recovered = true; client.version += 1; client.policyVersion += 1;
      client.credentials = client.credentials.slice(0, 1); // v2 existed only after checkpoint.
      member.exists = true; member.status = 'DISABLED'; user.status = 'DISABLED'; user.sessionVersion = 3;
    } };
  }
  async function allStages(mock) {
    const before = await runBeforeCheckpoint(mock.ports), after = await runAfterCheckpoint(mock.ports, before.privateCarriers);
    mock.recover(); const target = await runAfterRecovery(mock.ports, after.privateCarriers);
    return { before, after, target };
  }
  test('mock staged recovery preserves scope contraction, old-secret rejection and independent credential floor', async () => {
    const mock = fixture(), result = await allStages(mock);
    assert.equal(result.target.facts[0].credentialHighwater, 2);
    assert.equal(result.target.facts[0].sameOldTokenActive, false);
    assert.equal(JSON.stringify([...result.before.facts, ...result.after.facts, ...result.target.facts]).includes(CANARY), false);
    assert.equal(JSON.stringify(result.target).includes('privateCarriers'), false);
    const requests = mock.calls.filter(row => row[0] === 'command').map(row => row[1]);
    assert.equal(Object.keys(requests[0].body).length, 19);
    assert.equal(requests[0].body.expectedVersion, null);
    assert.equal(requests[0].body.contextMode, 'NONE');
    assert.deepEqual(requests.find(row => row.method === 'PATCH').body.permissions, [{ resourceKey: RESOURCE, scopes: [REQUIRED] }]);
    const disableIndex = mock.calls.findIndex(row => row[0] === 'command' && row[1].method === 'PATCH');
    assert.equal(mock.calls.slice(0, disableIndex).filter(row => row[0] === 'machine').length, 3);
    assert.equal(mock.calls.slice(0, disableIndex).filter(row => row[0] === 'introspect').length, 1);
    assert.equal(requests.filter(row => row.path.includes('/api/memberships/')).length, 1);
    assert.ok(requests.every(row => !row.path.includes('/services')));
  });
  test('protected members/users and original clients cannot be selected', async () => {
    for (const field of ['retireMemberId', 'retireUserId']) {
      const mock = fixture(), protectedKey = field === 'retireMemberId' ? 'protectedMembershipIds' : 'protectedUserIds';
      mock.ports.bindings[field] = mock.ports.bindings[protectedKey][0];
      await assert.rejects(runBeforeCheckpoint(mock.ports), { code: 'POLICY_BINDING_INVALID' });
      assert.equal(mock.calls.length, 0);
    }
    const mock = fixture({ protectedClient: true });
    await assert.rejects(runBeforeCheckpoint(mock.ports), { code: 'POLICY_CLIENT_RESULT_MISMATCH' });
    assert.equal(mock.calls.filter(row => row[0] === 'command').length, 1);
  });
  test('400 is not credential rejection, and inactive false is required for the same original opaque token', async () => {
    for (const [option, code] of [['denialAs400', 'POLICY_OLD_CREDENTIAL_NOT_REJECTED'], ['tokenRevives', 'POLICY_OLD_TOKEN_RESTORED_ACTIVE']]) {
      const mock = fixture({ [option]: true }), before = await runBeforeCheckpoint(mock.ports);
      await assert.rejects(runAfterCheckpoint(mock.ports, before.privateCarriers), { code });
    }
  });
  test('recovery cannot rearm the extra authorities or forget an intermediate credential highwater', async () => {
    for (const option of ['credentialFloorLost', 'rearmed']) {
      const mock = fixture({ [option]: true }), before = await runBeforeCheckpoint(mock.ports);
      const after = await runAfterCheckpoint(mock.ports, before.privateCarriers); mock.recover();
      await assert.rejects(runAfterRecovery(mock.ports, after.privateCarriers), { code: 'POLICY_RECOVERY_GATE_MISMATCH' });
    }
  });
  test('all observed source credential generations contribute to the independent recovery floor', async () => {
    const mock = fixture({ observedHistoricalCredential: true }), before = await runBeforeCheckpoint(mock.ports);
    const after = await runAfterCheckpoint(mock.ports, before.privateCarriers);
    assert.equal(after.privateCarriers.highwater.credentialVersion, 17); mock.recover();
    await assert.rejects(runAfterRecovery(mock.ports, after.privateCarriers), { code: 'POLICY_RECOVERY_GATE_MISMATCH' });
  });
  test('restored scopes, member/user status and membership generation remain fenced', async () => {
    for (const [option, code] of [['scopeRevives', 'POLICY_CLIENT_RESULT_MISMATCH'], ['memberRevives', 'POLICY_FACTS_INVALID'],
      ['userRevives', 'POLICY_FACTS_INVALID'], ['floorRollsBack', 'POLICY_RESTORED_HIGHWATER_LOST']]) {
      const mock = fixture({ [option]: true }), before = await runBeforeCheckpoint(mock.ports);
      const after = await runAfterCheckpoint(mock.ports, before.privateCarriers); mock.recover();
      await assert.rejects(runAfterRecovery(mock.ports, after.privateCarriers), { code });
    }
  });
  test('source carrier remains immutable and recovery performs no mutation/rearm command', async () => {
    const mock = fixture(), before = await runBeforeCheckpoint(mock.ports), saved = structuredClone(before.privateCarriers);
    const after = await runAfterCheckpoint(mock.ports, before.privateCarriers);
    assert.deepEqual(before.privateCarriers, saved); mock.recover();
    const commands = mock.calls.filter(row => row[0] === 'command').length;
    await runAfterRecovery(mock.ports, after.privateCarriers);
    assert.equal(mock.calls.filter(row => row[0] === 'command').length, commands);
    await assert.rejects(runAfterRecovery(mock.ports, before.privateCarriers), { code: 'POLICY_CARRIERS_INVALID' });
  });
  test('exception bodies/secret canaries do not enter failure diagnostics', async () => {
    await assert.rejects(runBeforeCheckpoint({}), { code: 'POLICY_PORTS_INVALID' });
    const mock = fixture(); mock.ports.command = async () => { throw new Error(CANARY); };
    try { await runBeforeCheckpoint(mock.ports); assert.fail('must reject'); }
    catch (error) { assert.equal(error.code, 'POLICY_COMMAND_FAILED'); assert.equal(String(error).includes(CANARY), false); }
  });
}
