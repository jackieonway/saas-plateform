/** Bind separately executed file negatives to the current frozen recovery inputs.
 * This pure validator opens no files or connections and never substitutes for
 * restore-new's authenticated precreation loaders or the actual restore/gate.
 */
const INVALID = 'PHASE23_FILE_NEGATIVE_EVIDENCE_INVALID';
const CONTRACT = 'phase23-file-negative-evidence/2';
const FORMAT = 'identity-recovery-negative-probe/1';
const LAYER = 'REAL_AUTHENTICATED_FILE_LOADERS_AND_READONLY_PG_PREFLIGHT';
const SHA256 = /^[0-9a-f]{64}$/;
const INPUTS = ['core.py', 'inventory.py', 'witness.py', 'reconcile.py', 'recovery.py', 'negative_probe.py']
  .map(name => `saas-plateform/backend/ops/identity-recovery/${name}`);
const CASES = [
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

function invalid() {
  const error = new Error(INVALID);
  error.code = INVALID;
  return error;
}
function need(condition) { if (!condition) throw invalid(); }
function record(value) {
  need(value !== null && typeof value === 'object' && !Array.isArray(value));
  const prototype = Object.getPrototypeOf(value);
  need(prototype === Object.prototype || prototype === null);
}
function own(value, name) {
  const descriptor = Object.getOwnPropertyDescriptor(value, name);
  need(descriptor && Object.hasOwn(descriptor, 'value') && descriptor.enumerable);
  return descriptor.value;
}
function exact(value, names) {
  record(value);
  const keys = Reflect.ownKeys(value);
  need(keys.length === names.length && keys.every(key => typeof key === 'string' && names.includes(key)));
  return Object.fromEntries(names.map(name => [name, own(value, name)]));
}
function preflight(value) {
  value = exact(value, ['sourceConnectionCount', 'targetDatabaseCount']);
  need(value.sourceConnectionCount === 0 && value.targetDatabaseCount === 0);
  return { sourceConnectionCount: 0, targetDatabaseCount: 0 };
}
function caseRows(value) {
  need(Array.isArray(value) && Object.getPrototypeOf(value) === Array.prototype);
  const length = Object.getOwnPropertyDescriptor(value, 'length');
  need(length && length.value === CASES.length);
  const keys = Reflect.ownKeys(value);
  need(keys.length === CASES.length + 1 && keys.every(key => key === 'length'
    || typeof key === 'string' && /^[0-8]$/.test(key)));
  return CASES.map((_, index) => own(value, String(index)));
}

export function validateFileNegativeEvidence(value, expectedInputs) {
  try {
    value = exact(value, ['contractVersion', 'status', 'sourceSnapshot', 'sourceArtifactName', 'sourceArtifactSha256', 'inputs', 'result']);
    need(value.contractVersion === CONTRACT && value.status === 'PASS');
    need(typeof value.sourceSnapshot === 'string' && value.sourceSnapshot.length <= 64
      && /^phase23-tools-r\d+$/.test(value.sourceSnapshot));
    need(typeof value.sourceArtifactName === 'string' && value.sourceArtifactName.length <= 160
      && value.sourceArtifactName === value.sourceArtifactName.trim()
      && /^phase-23-[a-z0-9-]+\.json$/.test(value.sourceArtifactName));
    need(typeof value.sourceArtifactSha256 === 'string' && SHA256.test(value.sourceArtifactSha256));
    const evidenceInputs = exact(value.inputs, INPUTS);
    record(expectedInputs);
    const inputs = {};
    for (const name of INPUTS) {
      const expected = own(expectedInputs, name), actual = evidenceInputs[name];
      need(typeof expected === 'string' && SHA256.test(expected) && actual === expected);
      inputs[name] = actual;
    }

    const result = exact(value.result,
      ['format', 'status', 'sequence', 'validationLayer', 'restoreSqlExecuted', 'databaseWrites', 'cases']);
    need(result.format === FORMAT && result.status === 'PASS' && result.validationLayer === LAYER);
    need(result.restoreSqlExecuted === false && result.databaseWrites === false);
    need(Number.isSafeInteger(result.sequence) && result.sequence >= 2);
    const observed = new Map();
    for (const row of caseRows(result.cases)) {
      const item = exact(row, ['case', 'status', 'code', 'preflightBefore', 'preflightAfter', 'originalBytesRestored']);
      const expected = CASES.find(([name]) => name === item.case);
      need(expected && !observed.has(item.case) && item.code === expected[1]);
      need(item.status === 'PASS' && item.originalBytesRestored === true);
      observed.set(item.case, {
        case: expected[0], status: 'PASS', code: expected[1],
        preflightBefore: preflight(item.preflightBefore),
        preflightAfter: preflight(item.preflightAfter), originalBytesRestored: true,
      });
    }
    return {
      contractVersion: CONTRACT, status: 'PASS', sourceSnapshot: value.sourceSnapshot,
      sourceArtifactName: value.sourceArtifactName, sourceArtifactSha256: value.sourceArtifactSha256, inputs,
      result: {
        format: FORMAT, status: 'PASS', sequence: result.sequence, validationLayer: LAYER,
        restoreSqlExecuted: false, databaseWrites: false,
        cases: CASES.map(([name]) => observed.get(name)),
      },
      separatelyExecuted: true, inputDigestsVerified: true,
    };
  } catch {
    // Even getters/proxies or hostile exceptions cannot echo evidence contents.
    throw invalid();
  }
}
