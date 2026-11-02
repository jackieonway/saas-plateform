import assert from 'node:assert/strict';
import { test } from 'node:test';
import { safeSdkCode, protocolFailureFact, safeProtocolCause } from './phase23-protocol-diagnostics.mjs';
const canary = 'PRIVATE_DIAGNOSTIC_CANARY';
test('only fixed SDK codes can be reported and exception text or nested cause is ignored', () => {
  assert.equal(safeSdkCode({ code: 'OAUTH_JWT_CLAIM_COMPARISON_FAILED', message: canary,
    cause: { token: canary } }), 'OAUTH_JWT_CLAIM_COMPARISON_FAILED');
  for (const value of [canary, 'OAUTH_'+canary, undefined]) assert.equal(safeSdkCode({ code: value }), undefined);
});
test('finish diagnostic preserves fixed stage and bounded observations without private carrier fields', () => {
  const value = protocolFailureFact('finish', { ok: false, code: 'SDK_REJECTED', status: 0,
    stage: 'finishLogin', sdkCode: 'OAUTH_INVALID_RESPONSE', callbackCodePresent: true, callbackErrorPresent: false,
    tokenResponseCount: 1, lastTokenStatus: 200, key: canary, callback: canary, message: canary, cause: { body: canary } });
  assert.deepEqual(value, { command: 'finish', code: 'SDK_REJECTED', status: 0, stage: 'finishLogin',
    sdkCode: 'OAUTH_INVALID_RESPONSE', callbackCodePresent: true, callbackErrorPresent: false,
    tokenResponseCount: 1, lastTokenStatus: 200 });
  assert.equal(JSON.stringify(value).includes(canary), false);
  assert.deepEqual(safeProtocolCause({ ...value, token: canary }), value);
});
test('unrecognized command labels, arbitrary codes, stages and excessive counters never become diagnostics', () => {
  assert.equal(protocolFailureFact(canary, { ok: false }), null);
  assert.equal(protocolFailureFact('finish', { ok: true }), null);
  const value = protocolFailureFact('finish', { ok: false, code: canary, status: 900, stage: canary,
    sdkCode: canary, callbackCodePresent: canary, tokenResponseCount: 17, lastTokenStatus: -1 });
  assert.deepEqual(value, { command: 'finish', code: 'PROTOCOL_DIAGNOSTIC_REJECTED', status: 0 });
  assert.equal(JSON.stringify(value).includes(canary), false);
});
