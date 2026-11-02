import assert from 'node:assert/strict';

// Only read endpoints with stable, independent positive controls belong here.
export function serviceGetCases(environment, ids, applicationId) {
  const platform = environment.platform;
  const definitions = [
    ['account', 'IDENTITY_SERVICE_CLIENT', `/internal/identity/accounts/${ids.admin}`],
    ['review', 'IDENTITY_REVIEW_SERVICE_CLIENT', `/internal/identity/auth/review/applications/${applicationId}/test`],
    ['invitation', 'IDENTITY_INVITATION_SERVICE_CLIENT', '/internal/identity/auth/invitations'],
    ['authAdmin', 'IDENTITY_AUTH_ADMIN_SERVICE_CLIENT', '/internal/identity/auth/clients'],
    // This deliberately absent operation verifies authentication before lookup.
    ['keyAdmin', 'IDENTITY_KEY_ADMIN_SERVICE_CLIENT', `/internal/identity/auth/key-operations/${ids.admin}`, 404],
  ];
  return definitions.map(([name, prefix, path, expectedStatus = 200]) => {
    const client = platform[`${prefix}_ID`], secret = platform[`${prefix}_SECRET`];
    assert(typeof client === 'string' && client.length > 0 && typeof secret === 'string' && secret.length > 0, 'SERVICE_BINDING_MISSING');
    return { name, path, client, secret, expectedStatus };
  });
}

export async function checkServiceGetControls({ source, target, ids, applicationId, request }) {
  const before = serviceGetCases(source, ids, applicationId);
  const after = target && serviceGetCases(target, ids, applicationId);
  const results = [];
  for (let i = 0; i < before.length; i++) {
    const old = before[i], current = after?.[i];
    if (current) {
      assert.equal(old.client, current.client, 'SERVICE_CLIENT_CHANGED');
      assert.notEqual(old.secret, current.secret, 'SERVICE_SECRET_NOT_ROTATED');
      assert.equal(await request(current), current.expectedStatus, 'NEW_SERVICE_SECRET_CONTROL_FAILED');
      assert.equal(await request({ ...current, secret: old.secret }), 403, 'OLD_SERVICE_SECRET_NOT_REJECTED');
    } else {
      assert.equal(await request(old), old.expectedStatus, 'SOURCE_SERVICE_SECRET_CONTROL_FAILED');
    }
    results.push({ name: old.name, source: !current, target: Boolean(current) });
  }
  return results;
}
