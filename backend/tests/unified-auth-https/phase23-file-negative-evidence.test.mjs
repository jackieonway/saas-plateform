import assert from 'node:assert/strict';
import { test } from 'node:test';
import { validateFileNegativeEvidence } from './phase23-file-negative-evidence.mjs';

const INVALID = 'PHASE23_FILE_NEGATIVE_EVIDENCE_INVALID';
const CANARY = 'PRIVATE_FILE_NEGATIVE_EVIDENCE_CANARY';
const names = ['core.py', 'inventory.py', 'witness.py', 'reconcile.py', 'recovery.py', 'negative_probe.py'];
const cases = [
  ['ciphertext-tamper', 'RECOVERY_ARCHIVE_DIGEST_MISMATCH'],
  ['manifest-tamper', 'RECOVERY_AUTHENTICATION_FAILED'],
  ['wrong-encryption-key', 'RECOVERY_ARCHIVE_AUTHENTICATION_FAILED'],
  ['mixed-source-binding', 'RECOVERY_CHECKPOINT_BINDING_MISMATCH'],
  ['signed-old-floor', 'RECOVERY_EXTERNAL_FLOOR_ROLLBACK'],
  ['signed-old-current', 'RECOVERY_REJECTED'],
  ['signed-old-record', 'RECOVERY_REJECTED'],
  ['missing-chain', 'RECOVERY_REJECTED'],
  ['extra-chain', 'RECOVERY_REJECTED'],
];
function fixture() {
  const inputs = Object.fromEntries(names.map((name, index) => [
    `saas-plateform/backend/ops/identity-recovery/${name}`, (index + 1).toString(16).repeat(64),
  ]));
  return {
    value: {
      contractVersion: 'phase23-file-negative-evidence/2', status: 'PASS',
      sourceSnapshot: 'phase23-tools-r18', sourceArtifactName: 'phase-23-file-negative-probe-r21.json',
      sourceArtifactSha256: 'a'.repeat(64), inputs,
      result: {
        format: 'identity-recovery-negative-probe/1', status: 'PASS', sequence: 2,
        validationLayer: 'REAL_AUTHENTICATED_FILE_LOADERS_AND_READONLY_PG_PREFLIGHT',
        restoreSqlExecuted: false, databaseWrites: false,
        cases: cases.map(([name, code]) => ({
          case: name, status: 'PASS', code, originalBytesRestored: true,
          preflightBefore: { sourceConnectionCount: 0, targetDatabaseCount: 0 },
          preflightAfter: { sourceConnectionCount: 0, targetDatabaseCount: 0 },
        })),
      },
    },
    expectedInputs: { ...inputs, 'saas-identity/src/lib.rs': 'b'.repeat(64) },
  };
}
function rejects(value, expectedInputs) {
  assert.throws(() => validateFileNegativeEvidence(value, expectedInputs), error => {
    assert.equal(error.code, INVALID);
    assert.equal(error.message, INVALID);
    assert.equal(error.cause, undefined);
    assert.equal(JSON.stringify(error).includes(CANARY), false);
    assert.equal(String(error.stack).includes(CANARY), false);
    return true;
  });
}

test('current input digests bind all nine separately executed file cases and safe metadata', () => {
  const { value, expectedInputs } = fixture(), before = structuredClone(value);
  const result = validateFileNegativeEvidence(value, expectedInputs);
  assert.deepEqual(result, { ...before, separatelyExecuted: true, inputDigestsVerified: true });
  assert.deepEqual(value, before);
  assert.equal(Object.keys(result.inputs).length, 6);
  assert.equal(Object.hasOwn(result.inputs, 'saas-identity/src/lib.rs'), false);
});

test('returned cases, preflights and inputs are rebuilt without caller-owned references', () => {
  const { value, expectedInputs } = fixture();
  const result = validateFileNegativeEvidence(value, expectedInputs);
  value.inputs[Object.keys(value.inputs)[0]] = CANARY;
  value.result.cases[0].code = CANARY;
  value.result.cases[0].preflightBefore.sourceConnectionCount = 99;
  value.result.cases.push({ secret: CANARY });
  assert.equal(JSON.stringify(result).includes(CANARY), false);
  assert.equal(result.result.cases.length, 9);
  assert.equal(result.result.cases[0].preflightBefore.sourceConnectionCount, 0);
});

test('all six stale or malformed evidence digests fail against the current manifest', () => {
  for (const name of names) {
    const { value, expectedInputs } = fixture();
    const path = `saas-plateform/backend/ops/identity-recovery/${name}`;
    value.inputs[path] = 'f'.repeat(64);
    rejects(value, expectedInputs);
    value.inputs[path] = CANARY;
    expectedInputs[path] = CANARY;
    rejects(value, expectedInputs);
  }
});

test('missing or extra evidence inputs and missing current-manifest inputs fail', () => {
  for (const mutate of [
    ({ value }) => { delete value.inputs[Object.keys(value.inputs)[0]]; },
    ({ value }) => { value.inputs['ops/identity-recovery/extra.py'] = 'f'.repeat(64); },
    ({ expectedInputs }) => { delete expectedInputs[Object.keys(expectedInputs)[0]]; },
    ({ expectedInputs }) => { expectedInputs[Object.keys(expectedInputs)[0]] = 'x'.repeat(64); },
  ]) { const f = fixture(); mutate(f); rejects(f.value, f.expectedInputs); }
});

test('missing duplicate or unknown case cannot replace one of the fixed nine', () => {
  for (const mutate of [
    rows => rows.pop(),
    rows => { rows[8] = structuredClone(rows[0]); },
    rows => { rows[0].case = CANARY; },
    rows => rows.push(structuredClone(rows[0])),
  ]) { const f = fixture(); mutate(f.value.result.cases); rejects(f.value, f.expectedInputs); }
});

test('case order is canonicalized without changing fixed names and rejection codes', () => {
  const { value, expectedInputs } = fixture();
  value.result.cases.reverse();
  const result = validateFileNegativeEvidence(value, expectedInputs);
  assert.deepEqual(result.result.cases.map(row => [row.case, row.code]), cases);
});

test('failed cases, wrong rejection categories or unrestored original bytes fail', () => {
  for (const [field, bad] of [['status', 'FAIL'], ['code', 'RECOVERY_REJECTED'],
    ['code', CANARY], ['originalBytesRestored', false], ['originalBytesRestored', 'true']]) {
    const f = fixture(); f.value.result.cases[0][field] = bad; rejects(f.value, f.expectedInputs);
  }
});

test('every before and after preflight requires exact zero source connections and target databases', () => {
  for (let index = 0; index < cases.length; index += 1) {
    for (const stage of ['preflightBefore', 'preflightAfter']) {
      for (const field of ['sourceConnectionCount', 'targetDatabaseCount']) {
        for (const bad of [1, -1, '0', false, null]) {
          const f = fixture(); f.value.result.cases[index][stage][field] = bad;
          rejects(f.value, f.expectedInputs);
        }
      }
    }
  }
});

test('only the actual file-loader result contract and read-only boundaries are accepted', () => {
  for (const [field, bad] of [['format', 'fake/1'], ['status', 'FAIL'], ['validationLayer', 'MOCK'],
    ['restoreSqlExecuted', true], ['restoreSqlExecuted', 0], ['databaseWrites', true], ['databaseWrites', 0],
    ['sequence', 1], ['sequence', 2.5], ['sequence', '2'], ['sequence', Number.MAX_SAFE_INTEGER + 1]]) {
    const f = fixture(); f.value.result[field] = bad; rejects(f.value, f.expectedInputs);
  }
});

test('envelope versions snapshots status and artifact digests use exact bounded formats', () => {
  for (const [field, bad] of [['contractVersion', 'phase23-file-negative-evidence/1'], ['status', 'FAIL'],
    ['sourceSnapshot', 'phase23-apps-r18'], ['sourceSnapshot', 'phase23-tools-r18/extra'],
    ['sourceSnapshot', CANARY], ['sourceArtifactSha256', CANARY], ['sourceArtifactSha256', 'a'.repeat(63)],
    ['sourceArtifactSha256', 'g'.repeat(64)], ['sourceArtifactSha256', 1]]) {
    const f = fixture(); f.value[field] = bad; rejects(f.value, f.expectedInputs);
  }
});

test('source artifacts have an explicit safe basename independent of the tools revision', () => {
  const f = fixture();
  const result = validateFileNegativeEvidence(f.value, f.expectedInputs);
  assert.equal(result.sourceArtifactName, 'phase-23-file-negative-probe-r21.json');
  assert.equal(result.sourceSnapshot, 'phase23-tools-r18');
  for (const name of [undefined, null, 1, '', '../phase-23-r18-integration.json',
    'phase-23-a/phase-23-b.json', 'phase-23-a\\phase-23-b.json', 'phase-23-A.json',
    'phase-23-a.json?secret', 'phase-23-a.json\n', 'phase-23-' + 'a'.repeat(160) + '.json']) {
    const invalid = fixture();invalid.value.sourceArtifactName = name;
    rejects(invalid.value, invalid.expectedInputs);
  }
  const missing = fixture();delete missing.value.sourceArtifactName;
  rejects(missing.value, missing.expectedInputs);
});

test('opensTraffic and secret-bearing extra fields are rejected at every evidence layer', () => {
  for (const target of [f => f.value, f => f.value.result, f => f.value.result.cases[0],
    f => f.value.result.cases[0].preflightBefore, f => f.value.result.cases[0].preflightAfter]) {
    for (const name of ['opensTraffic', 'secret', '__proto__']) {
      const f = fixture(); Object.defineProperty(target(f), name, { value: CANARY, enumerable: true });
      rejects(f.value, f.expectedInputs);
    }
  }
});

test('case arrays cannot hide extra authority claims holes or index accessors', () => {
  for (const mutate of [
    rows => { rows.opensTraffic = true; },
    rows => { rows[Symbol(CANARY)] = CANARY; },
    rows => { delete rows[0]; },
    rows => { Object.defineProperty(rows, '0', { get() { throw new Error(CANARY); }, enumerable: true }); },
  ]) {
    const f = fixture(); mutate(f.value.result.cases); rejects(f.value, f.expectedInputs);
  }
});

test('descriptor snapshots prevent hostile proxy reads from changing a validated summary', () => {
  const f = fixture();
  const evidence = new Proxy(f.value, { get() { throw new Error(CANARY); } });
  const result = validateFileNegativeEvidence(evidence, f.expectedInputs);
  assert.equal(result.sourceSnapshot, 'phase23-tools-r18');
  assert.equal(JSON.stringify(result).includes(CANARY), false);
});

test('non-record missing hidden symbol inherited and accessor data never escape as diagnostics', () => {
  for (const value of [undefined, null, [], CANARY, 1, new Date(), Object.create({ secret: CANARY })]) {
    rejects(value, fixture().expectedInputs);
  }
  for (const mutate of [
    f => { delete f.value.status; },
    f => { f.value[Symbol(CANARY)] = CANARY; },
    f => { Object.defineProperty(f.value, 'secret', { value: CANARY }); },
    f => { Object.defineProperty(f.value, 'status', { get() { throw new Error(CANARY); }, enumerable: true }); },
    f => { Object.defineProperty(f.expectedInputs, Object.keys(f.expectedInputs)[0],
      { get() { throw new Error(CANARY); }, enumerable: true }); },
  ]) { const f = fixture(); mutate(f); rejects(f.value, f.expectedInputs); }
  const f = fixture();
  rejects(new Proxy(f.value, { ownKeys() { throw new Error(CANARY); } }), f.expectedInputs);
});
