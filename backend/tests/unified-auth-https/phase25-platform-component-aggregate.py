"""Combine current frozen executions while preserving the original failed run.

This is a coverage reconciliation, never a claim that all tests were executed
in one invocation. The only accepted earlier failure is the exact TLS opt-in
fixture subsequently executed successfully against the same source closure.
"""
import datetime
import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[4]
VALIDATION = ROOT / 'saas-plateform/backend/docs/designs/unified-auth/validation'
TLS_CASE = 'customer_portal::remote_authority::tests::tls::phase12_customer_authority_real_tls_validates_ca_hostname_redirects_and_response_limits'


def main():
    def load(name):
        path = VALIDATION / name
        raw = path.read_bytes()
        return json.loads(raw), {'file': name, 'sha256': hashlib.sha256(raw).hexdigest()}
    base, base_reference = load('phase-25-platform-components.json')
    assert base['status'] == 'FAIL' and base['failureTestNames'] == [TLS_CASE]
    assert base['totals'] == {'passed': 408, 'failed': 1, 'ignored': 0, 'filtered': 0}
    assert all(not item['added'] and not item['removed'] for item in base['resourceObservation'].values())
    assert not base['sourceMismatchAfter']
    selected, selected_reference = load('phase-25-platform-selected-338b490efae8-components.json')
    assert selected['status'] == 'PASS' and selected['exactTestSelector'] == TLS_CASE
    assert selected['totals'] == {'passed': 1, 'failed': 0, 'ignored': 0, 'filtered': 431}
    binaries = lambda value: {item['pathRelativeToTarget']: item['sha256'] for run in value['runs']
                              for item in run.get('testExecutables', [])}
    original_binaries, selected_binaries = binaries(base), binaries(selected)
    assert original_binaries and all(selected_binaries.get(name) == digest
                                     for name, digest in original_binaries.items())
    records = [(base, base_reference), (selected, selected_reference)]
    for target in ('authorization_policy', 'context_selection', 'context_selection_database', 'migration_command', 'retention'):
        value, reference = load('phase-25-platform-integration-' + target + '-components.json')
        assert value['status'] == 'PASS' and value['integrationTestTarget'] == target
        assert value['totals']['ignored'] == value['totals']['failed'] == value['totals']['filtered'] == 0
        records.append((value, reference))
    for value, _reference in records:
        assert value['snapshotSha256'] == base['snapshotSha256']
        assert value['sourceInputs'] == base['sourceInputs']
        assert value['clusterSystemIdentifier'] == base['clusterSystemIdentifier']
        assert not value['sourceMismatchAfter']
    cases = {}
    execution_totals = {'passed': 0, 'failed': 0, 'ignored': 0, 'filtered': 0}
    runs = []
    for value, reference in records:
        for key in execution_totals:
            execution_totals[key] += value['totals'][key]
        for run in value['runs']:
            runs.append({**run, 'evidenceFile': reference['file'], 'evidenceSha256': reference['sha256']})
            for case in run['cases']:
                cases[case['name']] = case['result']
    assert len(cases) == 432
    assert all(result == 'OK' for result in cases.values())
    supplement, supplement_reference = load('phase-25-components-cross-input-supplement.json')
    assert supplement['status'] == 'PASS' and supplement['snapshotSha256'] == base['snapshotSha256']
    output = {'contractVersion': 'unified-auth-phase25-platform-component-coverage/1', 'phase': 25,
              'service': 'platform', 'status': 'PASS',
              'generatedAtUtc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'snapshot': base['snapshot'], 'snapshotSha256': base['snapshotSha256'],
              'manifestSha256': base['manifestSha256'], 'sourceInputs': base['sourceInputs'],
              'clusterSystemIdentifier': base['clusterSystemIdentifier'], 'runs': runs,
              'evidenceReferences': [reference for _value, reference in records],
              'crossProjectInputSupplement': supplement_reference,
              'originalAndExactTlsTestExecutablesIdentical': original_binaries,
              'coverage': {'uniquePassedTestNames': len(cases), 'unresolvedFailures': [],
                           'singleInvocationClaimed': False, 'testNames': sorted(cases)},
              'actualExecutionTotalsIncludingRetainedHistory': execution_totals,
              'resolvedHistoricalFailure': {'testName': TLS_CASE, 'originalResult': 'FAIL',
                  'reason': 'Runner omitted explicit IDENTITY_PHASE12_TLS_TESTS=TASK_OWNED. Fixture rejected before resource creation.',
                  'resolution': 'Explicit owner opt-in set; exact same existing TLS test passed in the recorded separate invocation.'},
              'sourceMismatchAfter': [],
              'limits': ['432 unique passed test names are assembled from separate current executions, not one green all-targets invocation.',
                         'Original 408 pass / 1 fail report and exact 1 pass / 431 filtered report remain unchanged.',
                         'No old Phase24 results are imported.',
                         'Unit, small-schema PG, canonical PG, LOGIN, mock authority ports and TLS fixture retain their original layers.',
                         'This does not prove the actual multi-service runtime or Phase25 completion.']}
    path = VALIDATION / 'phase-25-platform-final-components.json'
    path.write_text(json.dumps(output, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'status': output['status'], 'uniquePassed': len(cases),
                      'executions': execution_totals, 'output': str(path)}))


if __name__ == '__main__':
    main()
