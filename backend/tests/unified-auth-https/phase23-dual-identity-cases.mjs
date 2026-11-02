/** Require real code exchanges from both supervised identity instances.
 * Handles and claims remain in private pipes; returned evidence has no tokens.
 */
import assert from 'node:assert/strict';

export async function checkDualIdentityIssuance({ authorize, verify, sameSubject }, expectedKid) {
  assert([authorize, verify, sameSubject].every(value => typeof value === 'function'), 'DUAL_IDENTITY_PORTS_REQUIRED');
  assert(expectedKid === 'phase23-b', 'DUAL_IDENTITY_ROTATED_KID_REQUIRED');
  const sessions = [], observed = [];
  // A background RP refresh can consume one gateway turn. Keep this bounded,
  // but require two actual, independently attributed successful exchanges.
  for (let attempt = 0; attempt < 4 && new Set(observed).size < 2; attempt++) {
    const session = await authorize('third');
    assert(session?.status === 200 && typeof session.key === 'string' && session.key.length > 0,
      'DUAL_IDENTITY_AUTHORIZATION_FAILED');
    assert(session.kid === expectedKid, 'DUAL_IDENTITY_WRONG_SIGNING_KID');
    assert([23441, 23442].includes(session.tokenIdentityPort), 'DUAL_IDENTITY_UPSTREAM_EVIDENCE_MISSING');
    const verification = await verify(session.key);
    assert(verification?.kid === expectedKid && verification.cachedKeySignatureValid === true
      && verification.currentAuthorityJwtValid === true && verification.onlineStatus === 200,
    'DUAL_IDENTITY_CURRENT_JWKS_VERIFICATION_FAILED');
    if (sessions.length) assert((await sameSubject(sessions[0].key, session.key))?.matches === true,
      'DUAL_IDENTITY_SUBJECT_CHANGED');
    sessions.push(session); observed.push(session.tokenIdentityPort);
  }
  assert(new Set(observed).size === 2, 'DUAL_IDENTITY_DISTINCT_SIGNERS_NOT_OBSERVED');
  return { caseId: 'K03_DUAL_IDENTITY_SIGNING', status: 'PASS',
    evidence: { signingKid: expectedKid, observedTokenUpstreamPorts: [...new Set(observed)].sort(),
      successfulCodeExchanges: sessions.length, freshAuthorityJwtVerifiedForEach: true,
      cachedSignatureVerifiedForEach: true, userinfoStatusForEach: 200, stableSubject: true,
      gatewayMetadataRequiresSeparateProcessOwnershipProof: true } };
}
