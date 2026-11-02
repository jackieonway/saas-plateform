import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { loadOwnedBrowserInputs, readOwnedBrowserJson, validateBrowserReport, saveBrowserReport } from './phase25-browser-binding.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const validation = path.resolve(here, '../../docs/designs/unified-auth/validation');
const task = '/tmp/rust-saas-unified-auth-20261028';

test('Phase25 rejects unknown phase, legacy snapshot and unfrozen or foreign inputs before subprocess execution', () => {
  const options = { environmentFile: task + '/https-environment.json', entrypoint: path.join(here, 'phase19-browser.mjs'),
    executionPhase: 25, legacyPhase: 19, snapshot: 'phase25-tools-r2' };
  for (const override of [{ executionPhase: 26 }, { snapshot: undefined }, { snapshot: 'phase23-tools-r1' },
    { environmentFile: '/tmp/other/https-environment.json' }, { entrypoint: path.join(here, 'other.mjs') },
    { executionPhase: 19 }]) {
    assert.throws(() => loadOwnedBrowserInputs({ ...options, ...override }), { name: 'AssertionError' });
  }
  for (const filename of ['/tmp/other/private.json', task + '/../private.json', undefined]) {
    assert.throws(() => readOwnedBrowserJson(filename), { name: 'AssertionError' });
  }
});

test('Phase25 report type/path checks preserve existing result and reject history overwrite', () => {
  const outputFile = path.join(validation, 'phase-25-management-policy-test-' + randomUUID() + '.json');
  try {
    validateBrowserReport({ outputFile, executionPhase: 25, legacyPhase: 19 });
    assert(!fs.existsSync(outputFile));
    saveBrowserReport(outputFile, { result: 'PUBLIC_TEST_FIRST' }, 25);
    assert.throws(() => validateBrowserReport({ outputFile, executionPhase: 25, legacyPhase: 19 }), { name: 'AssertionError' });
    assert.throws(() => saveBrowserReport(outputFile, { result: 'PUBLIC_TEST_SECOND' }, 25), { code: 'EEXIST' });
    assert.equal(JSON.parse(fs.readFileSync(outputFile, 'utf8')).result, 'PUBLIC_TEST_FIRST');
    for (const foreign of [path.join(here, path.basename(outputFile)), path.join(validation, 'phase-19-original.json'),
      path.join(validation, 'phase-25-logout-wrong-suite.json')]) {
      assert.throws(() => validateBrowserReport({ outputFile: foreign, executionPhase: 25, legacyPhase: 19 }), { name: 'AssertionError' });
    }
  } finally {
    if (fs.existsSync(outputFile)) fs.unlinkSync(outputFile);
  }
});
