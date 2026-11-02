import test from 'node:test';
import assert from 'node:assert/strict';
import { checkServiceGetControls, serviceGetCases } from './phase23-service-get-cases.mjs';

const prefixes = ['IDENTITY_SERVICE_CLIENT', 'IDENTITY_REVIEW_SERVICE_CLIENT', 'IDENTITY_INVITATION_SERVICE_CLIENT', 'IDENTITY_AUTH_ADMIN_SERVICE_CLIENT', 'IDENTITY_KEY_ADMIN_SERVICE_CLIENT'];
function environment(secretPrefix) {
  const platform = {};
  for (const prefix of prefixes) {
    platform[`${prefix}_ID`] = `${prefix}-id`;
    platform[`${prefix}_SECRET`] = `${secretPrefix}-${prefix}`;
  }
  return { platform };
}
const ids = { admin: 'admin-id' }, applicationId = 'application-id';

test('source and target exercise each endpoint with matching ID and distinct secrets', async () => {
  const source = environment('old'), target = environment('new'), requests = [];
  const request = async input => { requests.push(input); return input.secret.startsWith('old-') ? 403 : input.expectedStatus; };
  const sourceResults = await checkServiceGetControls({ source, ids, applicationId, request: async input => { requests.push(input); return input.expectedStatus; } });
  const targetResults = await checkServiceGetControls({ source, target, ids, applicationId, request });
  assert.equal(sourceResults.length, 5);
  assert.equal(targetResults.length, 5);
  assert.equal(requests.length, 15);
  for (let i = 0; i < 5; i++) {
    assert.equal(requests[i].path, requests[5 + 2 * i].path);
    assert.equal(requests[5 + 2 * i].path, requests[6 + 2 * i].path);
    assert.equal(requests[5 + 2 * i].client, requests[6 + 2 * i].client);
  }
  assert(!JSON.stringify(targetResults).includes('old-'));
});

test('rejects failed positive controls, unrelated failures and unchanged secrets', async () => {
  const source = environment('old'), target = environment('new');
  await assert.rejects(checkServiceGetControls({source, ids, applicationId, request: async () => 503}), /SOURCE_SERVICE_SECRET_CONTROL_FAILED/);
  await assert.rejects(checkServiceGetControls({source, target, ids, applicationId, request: async () => 503}), /NEW_SERVICE_SECRET_CONTROL_FAILED/);
  await assert.rejects(checkServiceGetControls({source, target, ids, applicationId, request: async input => input.secret.startsWith('new-') ? 200 : 503}), /OLD_SERVICE_SECRET_NOT_REJECTED/);
  await assert.rejects(checkServiceGetControls({source, target: source, ids, applicationId, request: async () => 200}), /SERVICE_SECRET_NOT_ROTATED/);
  assert.throws(() => serviceGetCases({platform: {}}, ids, applicationId), /SERVICE_BINDING_MISSING/);
});
