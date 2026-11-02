/** Pure CLI contract from the real driver; no environment, network or process I/O. */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import vm from 'node:vm';

const source = fs.readFileSync(new URL('./phase22-runtime.mjs', import.meta.url), 'utf8');
const start = source.indexOf('function runtimeOptions(argv) {');
const end = source.indexOf('\nconst { output, phase, faultsOnly }', start);
assert(start > 0 && end > start);
const TASK = '/tmp/rust-saas-unified-auth-20261028';
const context = vm.createContext({ assert, path, TASK });
vm.runInContext(source.slice(start, end), context);
const options = argv => JSON.parse(JSON.stringify(context.runtimeOptions(argv)));

test('Phase25 capacity selects fresh mode and its own report filename', () => {
  assert.deepEqual(options([TASK + '/phase25-capacity-runtime.json', '--phase25']), {
    output: TASK + '/phase25-capacity-runtime.json', phase: 25, faultsOnly: false });
});

test('Phase22 defaults and explicit component reuse remain available', () => {
  assert.equal(options([TASK + '/phase22-runtime.json']).phase, 22);
  assert.equal(options([TASK + '/phase22-runtime.json', '--faults-only']).faultsOnly, true);
});

test('Mode, report path and argument confusion reject before any runtime input', () => {
  for (const argv of [[], [TASK + '/phase22-runtime.json', '--phase25'],
    [TASK + '/phase25-capacity-runtime.json', '--faults-only'],
    ['/tmp/phase25-capacity-runtime.json', '--phase25'],
    [TASK + '/nested/phase25-capacity-runtime.json', '--phase25'],
    [TASK + '/phase25-capacity-runtime.json', '--phase25', '--faults-only'],
    [TASK + '/phase25-capacity-runtime.json', '--unknown']]) {
    assert.throws(() => options(argv));
  }
});
