import assert from 'node:assert/strict';
import { test } from 'node:test';
import { checkDualIdentityIssuance } from './phase23-dual-identity-cases.mjs';

const canary = 'PRIVATE_DUAL_IDENTITY_CANARY';
function fixture(ports = [23441, 23442], changes = {}) {
  let attempts = 0;
  return { get attempts() { return attempts; },
    ports: { authorize: async () => ({ status: 200, key: canary + '-' + ++attempts,
      kid: 'phase23-b', tokenIdentityPort: ports[(attempts - 1) % ports.length], ...changes.session }),
    verify: async () => ({ kid: 'phase23-b', cachedKeySignatureValid: true,
      currentAuthorityJwtValid: true, onlineStatus: 200, ...changes.verification }),
    sameSubject: async () => ({ matches: !changes.subjectChanged }) } };
}
test('two attributed code exchanges require current JWKS verification and stable opaque subject', async () => {
  const source = fixture();
  const result = await checkDualIdentityIssuance(source.ports, 'phase23-b');
  assert.equal(source.attempts, 2);
  assert.deepEqual(result.evidence.observedTokenUpstreamPorts, [23441, 23442]);
  assert.equal(result.evidence.freshAuthorityJwtVerifiedForEach, true);
  assert.equal(JSON.stringify(result).includes(canary), false);
});
test('intervening gateway turns are bounded and one signer alone never satisfies dual issuance', async () => {
  const interleaved = fixture([23441, 23441, 23442]);
  assert.equal((await checkDualIdentityIssuance(interleaved.ports, 'phase23-b')).evidence.successfulCodeExchanges, 3);
  const single = fixture([23441]);
  await assert.rejects(checkDualIdentityIssuance(single.ports, 'phase23-b'), /DISTINCT_SIGNERS_NOT_OBSERVED/);
  assert.equal(single.attempts, 4);
});
test('missing, spoof-shaped, failed or unverified issuer metadata cannot satisfy the gate', async () => {
  for (const port of [undefined, null, '23442', 5432, '23441, 23442']) {
    await assert.rejects(checkDualIdentityIssuance(fixture([port]).ports, 'phase23-b'), /UPSTREAM_EVIDENCE_MISSING/);
  }
  for (const [changes, reason] of [
    [{ session: { status: 400 } }, /AUTHORIZATION_FAILED/],
    [{ session: { kid: 'owned-https' } }, /WRONG_SIGNING_KID/],
    [{ verification: { currentAuthorityJwtValid: false } }, /CURRENT_JWKS_VERIFICATION_FAILED/],
    [{ verification: { cachedKeySignatureValid: false } }, /CURRENT_JWKS_VERIFICATION_FAILED/],
    [{ verification: { onlineStatus: 401 } }, /CURRENT_JWKS_VERIFICATION_FAILED/],
    [{ subjectChanged: true }, /SUBJECT_CHANGED/],
  ]) await assert.rejects(checkDualIdentityIssuance(fixture(undefined, changes).ports, 'phase23-b'), reason);
});
