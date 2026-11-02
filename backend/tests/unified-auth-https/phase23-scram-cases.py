"""Private inputs for the attested Rust password probe; no HBA or role changes.

--mode SOURCE|RESTORE --task-dir TASK --environment PRIVATE --output TASK/new.json
SOURCE: correct+wrong for three runtime roles (6 fresh connections).
RESTORE: old+new+wrong for the same roles and derived target databases (9).
This wrapper is code-only until the environment owner invokes the CLI after its
root-owned HBA/proxy setup. Pure tests inject launch and never connect to PG.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
from urllib.parse import unquote, urlsplit

spec = importlib.util.spec_from_file_location('phase23_environment', Path(__file__).with_name('phase23-environment.py'))
control = importlib.util.module_from_spec(spec)
spec.loader.exec_module(control)
TASK = control.TASK
LABELS = {'identity': 'IDENTITY', 'platform': 'PLATFORM', 'portal': 'PORTAL'}
OBSERVATIONS = {'ACCEPTED', 'PASSWORD_REJECTED', 'OTHER_DATABASE_ERROR', 'TIMEOUT',
                'UNAVAILABLE', 'PROOF_MISMATCH', 'CLOSE_FAILED'}
SQLSTATES = {None, '28P01', '28000', '3D000', 'CONNECTION_CLASS08', 'RESOURCE_CLASS53', 'OTHER'}


def password(task, path):
    raw = task.read(path, maximum=4096)
    value = raw.decode('ascii').removesuffix('\n')
    control.need(32 <= len(value) <= 1024 and all(33 <= ord(char) <= 126 for char in value), 'SCRAM_PASSWORD_FILE_INVALID')
    return value


def run_probe(task, binary, input_path, output_path):
    command = [str(binary), '--input', str(input_path), '--output', str(output_path)]
    if os.getuid() == 0:
        command = ['/usr/sbin/runuser', '-u', task.username, '--', *command]
    # Do not inherit PGOPTIONS from the privileged owner SQL helpers: runtime
    # LOGIN roles must not be asked to SET superuser-only logging parameters.
    environment = {'PATH': '/usr/local/bin:/usr/bin:/bin', 'HOME': str(TASK), 'LANG': 'C.UTF-8'}
    result = subprocess.run(command, env=environment, capture_output=True, timeout=195)
    control.need(result.returncode in (0, 1) and result.stdout in (b'DB_AUTH_PROBE_PASS\n', b'DB_AUTH_PROBE_FAIL\n')
                 and result.stderr == b'', 'SCRAM_PROBE_EXECUTION_FAILED')
    return result.returncode


def execute(task, args, launch=run_probe, reservation=None):
    output = Path(args.output)
    control.need(output.parent == TASK and re.fullmatch(r'phase23-[a-z0-9-]+\.json', output.name)
                 and not output.exists() and not output.is_symlink(), 'SCRAM_NEW_OUTPUT_REQUIRED')
    source_path = TASK / 'phase23-source-environment.json'
    source = task.json(source_path)
    task.environment_binding(source, 'SOURCE')
    selected = task.json(args.environment)
    task.environment_binding(selected, args.mode)
    resources = task.resources()
    prefix = resources['source']['rolePrefix']
    source_map = task.json(TASK / 'phase23-source-db-passwords.json')
    control.need(set(source_map) == set(LABELS), 'SCRAM_SOURCE_MAP_INVALID')
    old = {}
    for kind in LABELS:
        row = source_map[kind]
        control.need(set(row) == {'role', 'passwordFile'} and row['role'] == prefix + '_' + kind + '_runtime', 'SCRAM_SOURCE_MAP_INVALID')
        old[kind] = password(task, Path(row['passwordFile']))
        control.need(unquote(urlsplit(source[kind]['DATABASE_URL']).password or '') == old[kind], 'SCRAM_SOURCE_ENV_PASSWORD_MISMATCH')
    fresh = None
    if args.mode == 'RESTORE':
        raw = task.read(Path(selected['phase23']['runtimePasswordsFile']))
        control.need(control.sha(raw) == selected['phase23']['runtimePasswordsSha256'], 'SCRAM_TARGET_MAP_DIGEST_INVALID')
        fresh = json.loads(raw)
        control.need(set(fresh) == set(LABELS), 'SCRAM_TARGET_MAP_INVALID')
        seen = set(old.values())
        for kind in LABELS:
            row = fresh[kind]
            control.need(set(row) == {'role', 'password'} and row['role'] == prefix + '_' + kind + '_runtime'
                         and isinstance(row['password'], str) and 32 <= len(row['password']) <= 1024
                         and all(33 <= ord(char) <= 126 for char in row['password'])
                         and row['password'] not in seen
                         and unquote(urlsplit(selected[kind]['DATABASE_URL']).password or '') == row['password'], 'SCRAM_TARGET_MAP_INVALID')
            seen.add(row['password'])
    else:
        control.need(all(unquote(urlsplit(selected[kind]['DATABASE_URL']).password or '') == old[kind] for kind in LABELS), 'SCRAM_SOURCE_ENV_PASSWORD_MISMATCH')
    label = (task.state or {}).get('buildSnapshot', 'phase23-apps-r1')
    control.need(re.fullmatch(r'phase(?:23|25)-[a-z0-9-]+', label), 'SCRAM_BUILD_ATTESTATION_INVALID')
    attestation = task.json(TASK / ('build-attestation-' + label + '.json'), private=False)
    expected_binary = attestation['binaries']['phase23-db-auth-probe']
    binary = TASK / 'bin/phase23-db-auth-probe'
    control.need(re.fullmatch(r'[0-9a-f]{64}', expected_binary) and task.digest(binary) == expected_binary
                 and attestation['snapshotLabel'] == label, 'SCRAM_PROBE_ATTESTATION_INVALID')
    # Reserve the caller's new result before creating any probe inputs or socket.
    task.write(output, {'contractVersion': 'unified-auth-phase23-scram/1', 'result': 'RUNNING', 'mode': args.mode})
    if reservation is not None:
        info = output.lstat()
        reservation['identity'] = (info.st_dev, info.st_ino)
    base = output.stem
    probe_input = TASK / (base + '-input.json')
    probe_output = TASK / (base + '-probe.json')
    cases = []
    selected_pairs = task.pairs(args.mode)
    for kind, label_prefix in LABELS.items():
        target = selected_pairs['portal' if kind == 'portal' else 'platform']
        role = prefix + '_' + kind + '_runtime'
        if args.mode == 'SOURCE':
            materials = [('CORRECT', Path(source_map[kind]['passwordFile']), 'ACCEPT')]
        else:
            fresh_path = TASK / (base + '-' + kind + '-new-password')
            task.write(fresh_path, fresh[kind]['password'].encode())
            materials = [('OLD', Path(source_map[kind]['passwordFile']), 'REJECT_28P01'),
                         ('NEW', fresh_path, 'ACCEPT')]
        wrong = secrets.token_hex(32)
        control.need(wrong not in set(old.values()) | ({row['password'] for row in fresh.values()} if fresh else set()), 'SCRAM_RANDOM_COLLISION')
        wrong_path = TASK / (base + '-' + kind + '-wrong-password')
        task.write(wrong_path, wrong.encode())
        materials.append(('WRONG', wrong_path, 'REJECT_28P01'))
        for suffix, secret_file, expected in materials:
            cases.append({'label': label_prefix + '_' + suffix, 'database': target['database'], 'role': role,
                'passwordFile': str(secret_file), 'expected': expected, 'expectedMarker': target['marker']})
    binding = {'mode': args.mode, 'sharedDatabase': resources['source']['database'],
        'portalDatabase': resources['source']['portal']['database'], 'rolePrefix': prefix,
        'deploymentId': resources['deploymentId']}
    task.write(probe_input, {'contractVersion': 'identity-db-password-probe-input/1', 'binding': binding, 'cases': cases})
    exit_code = launch(task, binary, probe_input, probe_output)
    actual = task.json(probe_output)
    control.need(set(actual) == {'contractVersion', 'result', 'cases'}
                 and actual['contractVersion'] == 'identity-db-password-probe-report/1'
                 and actual['result'] in ('PASS', 'FAIL') and isinstance(actual['cases'], list)
                 and len(actual['cases']) == len(cases), 'SCRAM_PROBE_REPORT_INVALID')
    expected_cases = {row['label']: row['expected'] for row in cases}
    observed = {}
    for row in actual['cases']:
        control.need(set(row) == {'label', 'status', 'observation', 'sqlState'}
                     and row['label'] in expected_cases and row['label'] not in observed
                     and row['status'] in ('PASS', 'FAIL') and row['observation'] in OBSERVATIONS
                     and row['sqlState'] in SQLSTATES, 'SCRAM_PROBE_REPORT_INVALID')
        observed[row['label']] = row
    passed = all(row['status'] == 'PASS' and (
        row['observation'] == 'ACCEPTED' and row['sqlState'] is None if expected_cases[label] == 'ACCEPT'
        else row['observation'] == 'PASSWORD_REJECTED' and row['sqlState'] == '28P01') for label, row in observed.items())
    passed = passed and exit_code == 0 and actual['result'] == 'PASS'
    result = {'contractVersion': 'unified-auth-phase23-scram/1', 'result': 'PASS' if passed else 'FAIL',
        'mode': args.mode, 'caseCount': len(cases), 'probeBinarySha256': expected_binary,
        'freshConnectionsRequired': True, 'hbaChangedByWrapper': False, 'cases': list(observed.values())}
    task.write(output, result, replace=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=('SOURCE', 'RESTORE'), required=True)
    parser.add_argument('--task-dir', type=Path, required=True)
    parser.add_argument('--environment', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    control.need(args.task_dir == TASK, 'SCRAM_TASK_INVALID')
    task = control.Task(require_root=os.getuid() == 0)
    reservation = {}
    try:
        result = execute(task, args, reservation=reservation)
    except Exception:
        if args.output.parent == TASK and args.output.exists() and reservation.get('identity') == (
                args.output.lstat().st_dev, args.output.lstat().st_ino):
            task.write(args.output, {'contractVersion': 'unified-auth-phase23-scram/1', 'result': 'FAIL',
                'mode': args.mode, 'code': 'PHASE23_SCRAM_CASES_FAILED'}, replace=True)
        raise
    print(json.dumps(result, sort_keys=True))
    return 0 if result['result'] == 'PASS' else 1


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except Exception:
        print('PHASE23_SCRAM_CASES_FAILED', file=sys.stderr)
        raise SystemExit(1) from None
