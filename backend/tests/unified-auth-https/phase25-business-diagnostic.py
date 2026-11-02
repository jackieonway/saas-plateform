"""Read-only current stores denial diagnostics. Raw credentials enter stdin only.

Prints fixed codes, response contract checks and counts; never response bodies,
identifiers, credentials, original tokens, SQL causes or private configuration.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import urlencode

TASK = Path('/tmp/rust-saas-unified-auth-20261028')
PREFIX = 'saas-plateform/backend/tests/unified-auth-https/'
SAFE_CODES = frozenset(('BAD_REQUEST', 'UNAUTHORIZED', 'FORBIDDEN', 'OPERATION_DENIED', 'UNAVAILABLE',
    'INTERNAL_ERROR', 'SERVER_ERROR', 'NOT_FOUND', 'RATE_LIMITED', 'PLATFORM_UNAVAILABLE',
    'SUBJECT_MODE_DENIED', 'INVALID_APP_TOKEN', 'INVALID_CONTEXT', 'DATA_ACCESS_DENIED'))


def need(value, code):
    if not value:
        raise RuntimeError('PHASE25_BUSINESS_DIAGNOSTIC_' + code)


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def fixed_code(value):
    return value if type(value) is str and value in SAFE_CODES else 'UNRECOGNIZED_OR_ABSENT'


def shape(status, value, *, authorization_context_version=None):
    """Return only allowlisted facts; no untrusted string can escape."""
    value = value if type(value) is dict else {}
    context = value.get('context')
    principal = value.get('principal')
    data = value.get('data')
    result = {'status': status, 'fixedCode': fixed_code(value.get('code')),
        'isResourceAuthorization': value.get('contractVersion') == 'resource-authorization/1',
        'isResourceDirectory': value.get('contractVersion') == 'resource-directory/1',
        'userSubject': value.get('subjectMode') == 'USER', 'appSessionUse': value.get('tokenUse') == 'app_session',
        'hasPrincipalObject': type(principal) is dict, 'hasContextObject': type(context) is dict}
    if type(context) is dict:
        result.update({'contextContractCurrent': context.get('contractVersion') == '1.0',
            'storeReadPermission': 'store:read' in context.get('permissions', []) if type(context.get('permissions')) is list else False,
            'storeReadDataGrantCount': sum(type(row) is dict and row.get('resource') == 'store' and row.get('action') == 'read'
                for row in context.get('dataGrants', [])) if type(context.get('dataGrants')) is list else 0})
    if type(data) is dict:
        records = data.get('records')
        result.update({'directoryOrganizations': value.get('directory') == 'organizations',
            'directoryDataContractCurrent': data.get('contractVersion') == '1.0',
            'directoryContextVersionMatchesAuthorization': type(authorization_context_version) is str
                and data.get('contextVersion') == authorization_context_version,
            'recordCount': len(records) if type(records) is list else -1,
            'everyRecordEnabledStore': type(records) is list and all(type(row) is dict
                and row.get('organizationType') == 'STORE' and row.get('status') == 'ENABLED' for row in records),
            'paginationCorrect': data.get('page') == 1 and data.get('size') == 50})
    return result


def request(task, ca, endpoint, *, form=None, basic=None, bearer=None):
    origin = 'https://console.saas.test:24444' if form is not None else 'https://console.saas.test:24447'
    config = ['header = "Accept: application/json"']
    if form is not None:
        config += ['request = "POST"', 'header = "Content-Type: application/x-www-form-urlencoded"',
            'data = ' + json.dumps(urlencode(form))]
    if basic:
        config.append('user = ' + json.dumps(basic[0] + ':' + basic[1]))
    if bearer:
        config.append('header = ' + json.dumps('Authorization: Bearer ' + bearer))
    result = subprocess.run(task.ns_command(['curl', '--noproxy', '*', '--silent', '--show-error', '--max-time', '8',
        '--cacert', ca, '--config', '-', '--write-out', '\n%{http_code}', origin + endpoint]),
        input=('\n'.join(config) + '\n').encode(), capture_output=True, timeout=10)
    if result.returncode != 0:
        return {'status': 0, 'value': None, 'transportFailed': True}
    body, code = result.stdout.rsplit(b'\n', 1)
    try:
        value = json.loads(body)
    except (ValueError, UnicodeDecodeError):
        value = None
    return {'status': int(code) if re.fullmatch(rb'[0-9]{3}', code) else 0,
        'value': value, 'transportFailed': False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--snapshot', required=True)
    args = parser.parse_args()
    report = {'contractVersion': 'unified-auth-phase25-business-diagnostic/1', 'result': 'FAIL',
        'diagnosticOnly': True, 'readOnly': True, 'rawResponseOrCredentialsPersisted': False, 'tlsVerificationDisabled': False}
    canaries = []
    try:
        need(re.fullmatch(r'phase25-tools-r[1-9][0-9]*', args.snapshot), 'SNAPSHOT_INVALID')
        raw = sys.stdin.buffer.read(20001)
        need(0 < len(raw) <= 20000, 'INPUT_BUDGET_INVALID')
        supplied = json.loads(raw)
        need(type(supplied) is dict and set(supplied) == {'token'} and type(supplied['token']) is str
             and 1 <= len(supplied['token']) <= 16384 and all(33 <= ord(char) <= 126 for char in supplied['token']), 'CARRIER_INVALID')
        token = supplied['token']
        canaries.append(token)
        base = load('phase25_diagnostic_base', 'phase23-environment.py')
        runtime = load('phase25_diagnostic_runtime', 'phase25-business-runtime.py')
        task = base.Task()
        need(task.state and task.state['mode'] == 'SOURCE' and task.state['snapshot'] == args.snapshot, 'CURRENT_SOURCE_REQUIRED')
        services, _ = runtime.configuration(task)
        manifest = task.json(TASK / ('snapshot-' + args.snapshot + '.json'))
        need(Path(__file__).resolve() == TASK / ('source-snapshot-' + args.snapshot) / PREFIX / Path(__file__).name
             and task.digest(Path(__file__)) == manifest['inputs'][PREFIX + Path(__file__).name], 'FROZEN_TOOL_REQUIRED')
        business = task.json(TASK / 'phase25-business-manifest.json')
        source = task.json(TASK / 'https-environment.json')
        task.guard_databases('SOURCE')
        digest = hashlib.sha256(token.encode()).hexdigest()
        active = task.pg('SELECT EXISTS(SELECT 1 FROM platform_app_capabilities WHERE token_hash=' + base.literal(digest)
            + ' AND application_id=' + base.literal(business['ids']['stores']['application'])
            + ' AND user_id=' + base.literal(source['ids']['employee']) + ' AND platform_app_capability_active(token_hash))', business['baseDatabase'])
        report['actualCurrentOwnedCarrier'] = active == 't'
        need(active == 't', 'CURRENT_OWNED_CARRIER_REQUIRED')
        settings = services['stores']['environment']
        credential = (settings['PLATFORM_CLIENT_ID'], settings['PLATFORM_CLIENT_SECRET'])
        canaries.extend(credential)
        ca = settings['PLATFORM_CA_CERT_FILE']
        authorization = request(task, ca, '/api/apps/stores/authorize',
            form={'token': token, 'resource': 'store', 'action': 'read'}, basic=credential)
        report['authorization'] = {**shape(authorization['status'], authorization['value']), 'transportFailed': authorization['transportFailed']}
        context = authorization['value'].get('context') if type(authorization.get('value')) is dict else None
        context_version = context.get('contextVersion') if type(context) is dict else None
        directory = request(task, ca, '/internal/platform/apps/stores/organizations',
            form={'token': token, 'resource': 'store', 'action': 'read', 'page': 1, 'size': 50, 'q': ''}, basic=credential)
        report['organizationDirectory'] = {**shape(directory['status'], directory['value'], authorization_context_version=context_version),
            'transportFailed': directory['transportFailed']}
        business_read = request(task, ca, '/stores-api/stores', bearer=token)
        report['directBusinessRead'] = {**shape(business_read['status'], business_read['value']), 'transportFailed': business_read['transportFailed']}
        path = TASK / 'service-platform.log'
        if path.exists():
            log = task.read(path, empty_ok=True)
            report['fixedPlatformFailureLabels'] = {label: label.encode() in log for label in
                ('AUTHORIZATION_SNAPSHOT_INVALID', 'PUBLIC_CONTRACT_INVALID')}
            report['fixedPlatformSqlstatesObserved'] = [state for state in ('42501', '42P01', '42703', '42883') if state.encode() in log]
        report['result'] = 'PASS'
    except BaseException as error:
        code = str(error)
        report['failureCode'] = code if re.fullmatch(r'PHASE25_BUSINESS_DIAGNOSTIC_[A-Z0-9_]+', code) else 'PHASE25_BUSINESS_DIAGNOSTIC_FAILED'
    rendered = json.dumps(report, sort_keys=True)
    need(not any(value in rendered for value in canaries if value), 'OUTPUT_CANARY_FOUND')
    print(rendered)
    return 0 if report['result'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
