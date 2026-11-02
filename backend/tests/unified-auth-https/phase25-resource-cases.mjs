/** Current-service resource contractions. Only returned facts belong in reports.
 * All original credentials stay in the caller's memory. In particular a test
 * must replay the original carrier after re-enabling, before issuing a new one.
 */
import assert from 'node:assert/strict';

const CODES = ['stores', 'work-orders'];
const denied = response => [401, 403].includes(response?.status);
const need = (value, code) => assert(value, `PHASE25_RESOURCE_${code}`);
const carrier = value => typeof value === 'string' && value.length > 0 && value.length <= 16384;

export async function runResourceCases(ports) {
  need(ports?.fixtureGuardVerified === true && ['mint', 'read', 'types', 'change', 'restore'].every(
    name => typeof ports[name] === 'function'), 'PORTS_REQUIRED');
  const facts = [], original = {};
  const fact = async value => { facts.push(value); await ports.checkpoint?.(value); };
  const read = async (code, token, positive, label) => {
    need(carrier(token), 'ORIGINAL_CARRIER_REQUIRED');
    const response = await ports.read(code, token);
    need(positive ? response?.status === 200 : denied(response), label);
    return response.status;
  };
  let restored = false;
  try {
    for (const actor of ['employee', 'admin']) {
      original[actor] = {};
      for (const code of CODES) {
        original[actor][code] = await ports.mint(actor, code);
        await read(code, original[actor][code], true, 'BASELINE_RESOURCE_DENIED');
      }
    }
    need(new Set(Object.values(original).flatMap(Object.values)).size === 4, 'BASELINE_CARRIERS_NOT_DISTINCT');
    await fact({ caseId: 'RESOURCE_TWO_MEMBERS_TWO_APPS_BASELINE', status: 'PASS', actualResourceReads: 4 });

    const types = await ports.types();
    need(carrier(types?.idToken) && carrier(types?.machineToken) && carrier(types?.platformToken)
      && types.idTokenVerified === true && types.machineHasPersonFields === false, 'REAL_TYPES_REQUIRED');
    for (const code of CODES) {
      await read(code, types.idToken, false, 'ID_TOKEN_ACCEPTED_AS_RESOURCE_CARRIER');
      await read(code, types.machineToken, false, 'WRONG_RESOURCE_MACHINE_ACCEPTED');
      await read(code, types.platformToken, false, 'PLATFORM_CARRIER_ACCEPTED_AS_APP_CARRIER');
      await read(code, original.employee[CODES.find(other => other !== code)], false, 'CROSS_AUDIENCE_ACCEPTED');
    }
    await fact({ caseId: 'RESOURCE_REAL_ID_TOKEN_MACHINE_AND_AUDIENCE_BOUNDARIES', status: 'PASS',
      realIdTokenSignatureAndSourceVerified: true, idTokenDenials: 2, wrongResourceMachineDenials: 2,
      platformCarrierDenials: 2, crossAudienceDenials: 2, machinePersonFields: false,
      approvedBusinessMachineDelegationTested: false, canonicalBusinessScopeContractionTested: false });

    await ports.change('remove-employee-store-read');
    await read('stores', original.employee.stores, false, 'REMOVED_DATA_GRANT_STILL_AUTHORIZES');
    await read('stores', original.admin.stores, true, 'OTHER_MEMBER_DATA_GRANT_CHANGED');
    await read('work-orders', original.employee['work-orders'], true, 'OTHER_APPLICATION_DATA_GRANT_CHANGED');
    await fact({ caseId: 'RESOURCE_CURRENT_DATA_GRANT_REMOVAL', status: 'PASS',
      sameOriginalCarrierReplayed: true, freshPermissionAndDataGrantRequired: true,
      otherMemberPreserved: true, otherApplicationPreserved: true });
    await ports.change('restore-employee-store-read');
    // Data grants are checked afresh on every request. Their ordinary restore
    // may authorize an otherwise live source; permanent revocation is tested
    // separately below and must never be inferred from a removed grant.
    await read('stores', original.employee.stores, true, 'DATA_GRANT_RESTORATION_FAILED');

    await ports.change('disable-stores');
    for (const actor of ['employee', 'admin']) {
      await read('stores', original[actor].stores, false, 'DISABLED_APPLICATION_ACCEPTED');
      await read('work-orders', original[actor]['work-orders'], true, 'UNRELATED_APPLICATION_REVOKED');
    }
    await ports.change('enable-stores');
    for (const actor of ['employee', 'admin']) {
      await read('stores', original[actor].stores, false, 'REENABLED_APPLICATION_REVIVED_OLD_CARRIER');
      const replacement = await ports.mint(actor, 'stores');
      need(replacement !== original[actor].stores, 'APPLICATION_REPLACEMENT_NOT_NEW');
      await read('stores', replacement, true, 'REENABLED_APPLICATION_FRESH_CARRIER_DENIED');
    }
    await fact({ caseId: 'RESOURCE_APPLICATION_CONTRACTION_AND_REENABLE', status: 'PASS',
      originalCarriersRejectedWhileDisabled: 2, originalCarriersRejectedAfterReenable: 2,
      newCarriersAccepted: 2, unrelatedApplicationControls: 2 });

    const beforeMember = {};
    for (const code of CODES) beforeMember[code] = await ports.mint('employee', code);
    await ports.change('disable-employee');
    for (const code of CODES) {
      await read(code, beforeMember[code], false, 'DISABLED_MEMBER_ACCEPTED');
      const control = await ports.mint('admin', code);
      await read(code, control, true, 'UNRELATED_MEMBER_REVOKED');
    }
    await ports.change('enable-employee');
    for (const code of CODES) {
      await read(code, beforeMember[code], false, 'REENABLED_MEMBER_REVIVED_OLD_CARRIER');
      const replacement = await ports.mint('employee', code, { newLogin: true });
      need(replacement !== beforeMember[code], 'MEMBER_REPLACEMENT_NOT_NEW');
      await read(code, replacement, true, 'REENABLED_MEMBER_FRESH_CARRIER_DENIED');
    }
    await fact({ caseId: 'RESOURCE_MEMBER_CONTRACTION_AND_REENABLE', status: 'PASS',
      originalCarriersRejectedWhileDisabled: 2, originalCarriersRejectedAfterReenable: 2,
      newLoginAndNewCarriersAccepted: 2, independentMemberControls: 2 });
  } finally {
    const result = await ports.restore();
    need(result?.policyRestored === true && result?.memberEnabled === true && result?.applicationEnabled === true
      && result?.dataGrantRestored === true && result?.generationsNeverDecreased === true, 'RESTORATION_FAILED');
    restored = true;
  }
  return { facts, cleanup: { fixturePolicyRestored: restored, authorizationGenerationsRewound: false },
    limitations: ['No approved business MACHINE delegation or canonical business scope contraction positive is claimed.',
      'Notification subscription contraction and SDK context switching are covered by separate suites.'] };
}
