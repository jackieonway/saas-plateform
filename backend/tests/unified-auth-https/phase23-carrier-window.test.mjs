import assert from 'node:assert/strict';
import { test } from 'node:test';
import { whileUnexpired } from './phase23-carrier-window.mjs';

const observations = (...values) => {
  let index = 0;
  return () => {
    assert(index < values.length, 'unexpected clock observation');
    return values[index++];
  };
};

test('an expired or exact-boundary carrier never submits the request', async () => {
  for (const before of [1000, 1001]) {
    let calls = 0;
    await assert.rejects(whileUnexpired({ expiresAtMs: 1000, now: () => before,
      run: () => { calls += 1; } }), { code: 'PHASE23_OLD_CARRIER_EXPIRED_BEFORE_REQUEST' });
    assert.equal(calls, 0);
  }
});

test('a 401 produced across expiry cannot count as revocation evidence', async () => {
  for (const after of [1000, 1001]) {
    let calls = 0;
    await assert.rejects(whileUnexpired({ expiresAtMs: 1000, now: observations(999, after),
      run: async () => { calls += 1; return { status: 401 }; } }),
    { code: 'PHASE23_OLD_CARRIER_EXPIRED_DURING_REQUEST' });
    assert.equal(calls, 1);
  }
});

test('a response received before the same deadline retains its exact result', async () => {
  const response = Object.freeze({ status: 401, body: Object.freeze({ error: 'unauthorized' }) });
  const guarded = await whileUnexpired({ expiresAtMs: 1000, now: observations(900, 999),
    run: async () => response });
  assert.equal(guarded.result, response);
  assert.equal(guarded.remainingMs, 1);
});

test('equal successive clock observations are allowed; a rollback is rejected', async () => {
  assert.deepEqual(await whileUnexpired({ expiresAtMs: 1000, now: observations(900, 900),
    run: () => 401 }), { result: 401, remainingMs: 100 });
  await assert.rejects(whileUnexpired({ expiresAtMs: 1000, now: observations(900, 899),
    run: () => ({ active: false }) }), { code: 'PHASE23_CARRIER_CLOCK_ROLLED_BACK' });
});

test('invalid options, deadlines and initial clock values fail without submission', async () => {
  for (const input of [undefined, null, [], 'private-carrier', {},
    { expiresAtMs: NaN }, { expiresAtMs: Infinity }, { expiresAtMs: -Infinity },
    { expiresAtMs: '1000' }, { expiresAtMs: 1000, now: 900 },
    ...[NaN, Infinity, -Infinity, '900', null, undefined].map(value => ({ expiresAtMs: 1000, now: () => value }))]) {
    let calls = 0;
    const options = input && typeof input === 'object' && !Array.isArray(input)
      ? { ...input, run: () => { calls += 1; } } : input;
    await assert.rejects(whileUnexpired(options), { code: 'PHASE23_OLD_CARRIER_WINDOW_INVALID' });
    assert.equal(calls, 0);
  }
  await assert.rejects(whileUnexpired({ expiresAtMs: 1000, now: () => 900, run: 'private-carrier' }),
    { code: 'PHASE23_OLD_CARRIER_WINDOW_INVALID' });
});

test('invalid or failed observations after a request reject the evidence', async () => {
  for (const value of [NaN, Infinity, -Infinity, '901', null, undefined]) {
    let calls = 0;
    await assert.rejects(whileUnexpired({ expiresAtMs: 1000, now: observations(900, value),
      run: () => { calls += 1; return { status: 401 }; } }), { code: 'PHASE23_OLD_CARRIER_WINDOW_INVALID' });
    assert.equal(calls, 1);
  }
  let count = 0;
  await assert.rejects(whileUnexpired({ expiresAtMs: 1000, now: () => {
    if (count++ === 0) return 900;
    throw new Error('PRIVATE_CLOCK_VALUE');
  }, run: () => ({ status: 401 }) }), { code: 'PHASE23_OLD_CARRIER_WINDOW_INVALID' });
});

test('async authority observations are checked before and after the request', async () => {
  const events = [];
  let observation = 0;
  const result = await whileUnexpired({ expiresAtMs: 1000,
    now: async () => { events.push('observe'); return ++observation === 1 ? 900 : 950; },
    run: async () => { events.push('request'); return { status: 401 }; } });
  assert.deepEqual(events, ['observe', 'request', 'observe']);
  assert.equal(result.remainingMs, 50);
});

test('guard failures expose only a fixed code and never private input or deadline values', async () => {
  const canary = 'PRIVATE_CARRIER_CANARY';
  const scenarios = [
    { expiresAtMs: canary, now: () => 900, run: () => ({ token: canary }) },
    { expiresAtMs: 1760000000123, now: () => { throw new Error(canary); }, run: () => canary },
    { expiresAtMs: 1760000000123, now: observations(1760000000122, 1760000000123),
      run: () => ({ status: 401, token: canary }) },
  ];
  for (const options of scenarios) {
    await assert.rejects(whileUnexpired(options), error => {
      assert.match(error.code, /^(PHASE23_OLD_CARRIER_WINDOW_INVALID|PHASE23_OLD_CARRIER_EXPIRED_DURING_REQUEST)$/);
      assert.equal(error.message, error.code);
      for (const text of [String(error), JSON.stringify(error)]) {
        assert.equal(text.includes(canary), false);
        assert.equal(text.includes('1760000000123'), false);
      }
      assert.equal(Object.hasOwn(error, 'cause'), false);
      assert.equal(Object.hasOwn(error, 'result'), false);
      return true;
    });
  }
});

test('request exceptions and rejections propagate unchanged without another observation', async () => {
  for (const asynchronous of [false, true]) {
    const failure = new Error('ORIGINAL_REQUEST_FAILURE');
    let clockCalls = 0;
    await assert.rejects(whileUnexpired({ expiresAtMs: 1000, now: () => { clockCalls += 1; return 900; },
      run: asynchronous ? async () => { throw failure; } : () => { throw failure; } }),
    error => error === failure);
    assert.equal(clockCalls, 1);
  }
});
