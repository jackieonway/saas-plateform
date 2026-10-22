import assert from 'node:assert/strict';

/** Return on the observed successful sample; an async predicate may change immediately afterwards. */
export async function waitState(predicate, message, timeout = 12_000, {
  now = Date.now,
  pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
} = {}) {
  const deadline = now() + timeout;
  for (;;) {
    if (await predicate()) return;
    if (now() >= deadline) assert.fail(message);
    await pause(25);
  }
}
