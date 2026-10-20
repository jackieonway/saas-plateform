import assert from 'node:assert/strict';
import test from 'node:test';
import { classifyBrowserResult } from './results.mjs';

test('complete mandatory check groups pass only with a successful process', () => {
  assert.deepEqual(classifyBrowserResult({ output: 'PASS first\nPASS second\n', exitCode: 0, expectedCheckGroups: 2 }),
    { pass: 2, fail: 0, skip: 0, reportedFailures: 0 });
});
test('a browser failure remains failed after earlier successful groups', () => {
  assert.equal(classifyBrowserResult({ output: 'PASS earlier\n', exitCode: 1, expectedCheckGroups: 1 }).fail, 1);
});
test('an explicitly skipped mandatory group fails the gate even with exit zero', () => {
  assert.deepEqual(classifyBrowserResult({ output: 'PASS earlier\nSKIP required environment missing\n', exitCode: 0, expectedCheckGroups: 1 }),
    { pass: 1, fail: 1, skip: 1, reportedFailures: 0 });
});
test('an omitted mandatory group fails instead of shrinking the tested manifest', () => {
  assert.equal(classifyBrowserResult({ output: 'PASS earlier\n', exitCode: 0, expectedCheckGroups: 2 }).fail, 1);
});
test('a reported failure or spawn error cannot be hidden by a zero exit status', () => {
  assert.equal(classifyBrowserResult({ output: 'PASS earlier\nFAIL later\n', exitCode: 0, expectedCheckGroups: 1 }).fail, 1);
  assert.equal(classifyBrowserResult({ output: 'PASS earlier\n', exitCode: 0, expectedCheckGroups: 1, error: 'Browser process could not start' }).fail, 1);
});
