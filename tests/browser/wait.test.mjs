import assert from 'node:assert/strict';
import test from 'node:test';
import { waitState } from './wait.mjs';

test('an observed ready state is not sampled again during a subsequent UI transition', async () => {
  let samples = 0;
  await waitState(async () => ++samples === 1, 'the already observed ready state must pass');
  assert.equal(samples, 1);
});
test('asynchronous readiness is awaited across controlled polling intervals', async () => {
  let samples = 0, time = 0;
  await waitState(async () => ++samples === 3, 'must become ready', 100,
    { now: () => time, pause: async (ms) => { time += ms; } });
  assert.equal(samples, 3);
  assert.equal(time, 50);
});
test('a state that never becomes ready fails with the caller diagnostic', async () => {
  let time = 0;
  await assert.rejects(waitState(async () => false, 'required UI state did not become ready', 100,
    { now: () => time, pause: async (ms) => { time += ms; } }), /required UI state did not become ready/);
});
