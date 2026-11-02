import assert from 'node:assert/strict';
import { createDecipheriv } from 'node:crypto';

// Decode only the task-owned server session; the caller must keep the result private.
export function openSessionEnvelope(kind, session, ring, purpose, envelope) {
  assert(['employee', 'customer'].includes(kind), 'SESSION_KIND_INVALID');
  assert(['oidc-access', 'pending-selection'].includes(purpose), 'SESSION_PURPOSE_INVALID');
  const parts = envelope.split('.');
  assert.equal(parts.length, 3, 'SESSION_ENVELOPE_INVALID');
  assert.equal(parts[0], 'v1', 'SESSION_ENVELOPE_INVALID');
  const material = ring.keys.filter(key => key.kid === parts[1]);
  assert.equal(material.length, 1, 'SESSION_KEY_INVALID');
  const key = Buffer.from(material[0].key, 'base64url'), payload = Buffer.from(parts[2], 'base64url');
  assert.equal(key.length, 32, 'SESSION_KEY_INVALID');
  assert(payload.length > 28, 'SESSION_ENVELOPE_INVALID');
  const domain = kind === 'employee' ? 'platform-bff-aead/1' : 'customer-portal-bff-aead/1';
  const cipher = createDecipheriv('aes-256-gcm', key, payload.subarray(0, 12));
  cipher.setAAD(Buffer.from(`${domain}\0${session.id}\0${session.issuer}\0${session.clientId}\0${purpose}`));
  cipher.setAuthTag(payload.subarray(-16));
  try { return Buffer.concat([cipher.update(payload.subarray(12, -16)), cipher.final()]).toString('utf8'); }
  finally { key.fill(0); }
}
export function openSessionAccess(kind, session, ring) {
  return openSessionEnvelope(kind, session, ring, 'oidc-access', session.accessCiphertext);
}

export function serviceAccessCases(environment, employee, customer) {
  const definitions = [
    ['source', 'platform', 'CONTEXT_SOURCE_SERVICE_CLIENT', '/internal/identity/auth/account-proof', 'platform-account-proof-request/1', employee, 403],
    ['portalSource', 'portal', 'CUSTOMER_SOURCE_SERVICE_CLIENT', '/internal/identity/auth/customer-account-proof', 'customer-account-proof-request/1', customer, 403],
    ['customerAuthority', 'platform', 'CUSTOMER_AUTHORITY_SERVICE_CLIENT', '/internal/identity/customer-access-proof', 'customer-access-proof-request/1', customer, 403],
    ['profile', 'portal', 'CUSTOMER_RESOURCE_CLIENT', '/internal/customer-portal/memberships', null, customer, 401],
  ];
  return definitions.map(([name, group, prefix, path, contract, session, denied]) => {
    const client = environment[group][`${prefix}_ID`], secret = environment[group][`${prefix}_SECRET`];
    assert(typeof client === 'string' && client && typeof secret === 'string' && secret, 'SERVICE_BINDING_MISSING');
    assert(session && typeof session.accessToken === 'string' && session.accessToken && session.clientId, 'FRESH_ACCESS_REQUIRED');
    const body = contract ? {contractVersion: contract, oidcAccessToken: session.accessToken, expectedClientId: session.clientId} : {oidcAccessToken: session.accessToken};
    return {name, client, secret, path, body, denied};
  });
}

export async function checkServiceAccessControls({ source, target, employee, customer, request }) {
  const old = serviceAccessCases(source, employee, customer), current = target && serviceAccessCases(target, employee, customer);
  const results = [];
  for (let i = 0; i < old.length; i++) {
    const previous = old[i], fresh = current?.[i];
    if (!fresh) assert.equal(await request(previous), 200, 'SOURCE_SERVICE_ACCESS_CONTROL_FAILED');
    else {
      assert.equal(previous.client, fresh.client, 'SERVICE_CLIENT_CHANGED');
      assert.notEqual(previous.secret, fresh.secret, 'SERVICE_SECRET_NOT_ROTATED');
      assert.equal(await request(fresh), 200, 'NEW_SERVICE_ACCESS_CONTROL_FAILED');
      assert.equal(await request({...fresh, secret: previous.secret}), fresh.denied, 'OLD_SERVICE_ACCESS_NOT_REJECTED');
    }
    results.push({name: previous.name, source: !fresh, target: Boolean(fresh)});
  }
  return results;
}
