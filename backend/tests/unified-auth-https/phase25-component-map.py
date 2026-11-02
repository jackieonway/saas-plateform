"""Link new component execution identities without promoting runtime scenarios.

The source mapping is a navigation aid. A matching passed test does not imply
that every business observation or another evidence layer has been satisfied.
"""
import datetime
import hashlib
import json
from pathlib import Path
import re


WORKSPACE = Path(__file__).resolve().parents[4]
VALIDATION = WORKSPACE / 'saas-plateform/backend/docs/designs/unified-auth/validation'


def main():
    def digest(value):
        return hashlib.sha256(value).hexdigest()
    matrix_raw = (VALIDATION / 'phase-25-matrix.json').read_bytes()
    matrix = json.loads(matrix_raw)
    evidence = {}
    for service in ('identity', 'platform', 'portal', 'stores', 'orders', 'notification'):
        preferred = VALIDATION / ('phase-25-' + service + '-final-components.json')
        path = preferred if preferred.exists() else VALIDATION / ('phase-25-' + service + '-components.json')
        if not path.exists():
            continue
        raw = path.read_bytes()
        value = json.loads(raw)
        evidence[service] = {'file': path.name, 'sha256': digest(raw), 'value': value}
    refs = {}
    for key, reference in matrix['sourceRefs'].items():
        selector = re.search(r'(?:async )?fn (\w+)', reference.get('selector', ''))
        if not selector:
            continue
        matches = []
        for service, item in evidence.items():
            value = item['value']
            if value.get('status') != 'PASS':
                continue
            source = reference['path']
            if value.get('sourceInputs', {}).get(source) != reference['sha256']:
                continue
            for run in value.get('runs', []):
                for case in run.get('cases', []):
                    if case['result'] == 'OK' and case['name'].split('::')[-1] == selector.group(1):
                        matches.append({'evidenceFile': item['file'], 'evidenceSha256': item['sha256'],
                                        'testName': case['name'], 'layer': reference['layer'],
                                        'sourcePath': source, 'sourceSha256': reference['sha256'],
                                        'snapshot': value['snapshot'], 'snapshotSha256': value['snapshotSha256']})
        if matches:
            refs[key] = matches
    scenarios = []
    for scenario in matrix['scenarios']:
        requested = scenario.get('componentRefIds', [])
        scenarios.append({'scenarioId': scenario['id'], 'categoryId': scenario['categoryId'],
                          'matchedComponentRefIds': [key for key in requested if key in refs],
                          'componentRefIdsWithoutMatchingNewRustEvidence': [key for key in requested if key not in refs],
                          'runtimeScenarioResult': 'NOT_DETERMINED_BY_COMPONENT_MAP'})
    output = {'contractVersion': 'unified-auth-phase25-component-execution-map/1', 'phase': 25,
              'generatedAtUtc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'matrixSha256': digest(matrix_raw), 'sourceReferenceExecutionMatches': refs,
              'evidence': {key: {name: value for name, value in item.items() if name != 'value'}
                           for key, item in evidence.items()},
              'scenarioSlots': scenarios,
              'summary': {'scenarioSlots': len(scenarios), 'matchedSourceReferences': len(refs),
                          'scenariosWithAtLeastOneMatchingComponentExecution': sum(bool(item['matchedComponentRefIds']) for item in scenarios)},
              'limits': ['Test execution identity and source hashes only; assertion coverage still needs review.',
                         'SMALL_SCHEMA_PG, canonical PG, LOGIN ACL, mock port and HTTP fixture retain distinct layers.',
                         'No runtime/browser/standard SDK/recovery or whole Phase25 completion conclusion.',
                         'No existing matrix result was modified.']}
    path = VALIDATION / 'phase-25-component-execution-map.json'
    path.write_text(json.dumps(output, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
    print(json.dumps(output['summary']))


if __name__ == '__main__':
    main()
