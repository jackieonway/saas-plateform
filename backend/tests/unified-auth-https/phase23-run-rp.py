"""Run only the frozen standard third-party RP, in the owned namespace as owner.

The supervisor reads protected configuration, but the Node child gets only the
exact public RP variables. It has no database or owner/service environment.
Restart is explicit; local session loss is never evidence of remote revocation.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

spec = importlib.util.spec_from_file_location('phase23_environment', Path(__file__).with_name('phase23-environment.py'))
control = importlib.util.module_from_spec(spec)
spec.loader.exec_module(control)
TASK = control.TASK
ALLOWED = {'OIDC_ISSUER', 'OIDC_CLIENT_ID', 'OIDC_CLIENT_SECRET_FILE', 'RP_ORIGIN',
           'RP_BIND_HOST', 'RP_BIND_PORT', 'NODE_EXTRA_CA_CERTS', 'OIDC_SCOPES'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--task-dir', type=Path, required=True)
    args = parser.parse_args()
    control.need(args.task_dir == TASK, 'RP_TASK_ARGUMENT_INVALID')
    task = control.Task(require_root=False)
    control.need(task.state and task.state['prepared'], 'RP_PREPARED_TASK_REQUIRED')
    expected_namespace = task.read(TASK / 'namespace.identity', private=False).decode().strip()
    control.need(os.readlink('/proc/self/ns/mnt') == expected_namespace, 'RP_NAMESPACE_REQUIRED')
    namespace = task.json(TASK / 'namespace.pid', private=False)
    control.need(namespace['namespace'] == expected_namespace
                 and control.start_time(namespace['pid']) == namespace['startTime']
                 and str(TASK / 'environment-controller.sh').encode() in Path('/proc', str(namespace['pid']), 'cmdline').read_bytes(), 'RP_LEASE_CHANGED')
    snapshot, manifest, _attestation = task.snapshot()
    source = TASK / 'phase23-rp-source'
    control.need(not source.is_symlink(), 'RP_SOURCE_LINK_REJECTED')
    if not source.exists():
        source.mkdir(mode=0o700)
        for name in control.RP_FILES:
            destination = source / name
            destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            task.write(destination, task.read(snapshot / control.RP_PREFIX / name, private=False))
    for name in control.RP_FILES:
        control.need(task.digest(source / name) == manifest['inputs'][control.RP_PREFIX + name], 'RP_FROZEN_INPUT_CHANGED')
    modules = TASK / 'phase21-rp-source/node_modules'
    control.need(modules.is_dir() and not modules.is_symlink()
                 and task.digest(TASK / 'phase21-rp-source/package-lock.json') == manifest['inputs'][control.RP_PREFIX + 'package-lock.json'], 'RP_REUSABLE_DEPENDENCIES_INVALID')
    link = source / 'node_modules'
    if link.exists() or link.is_symlink():
        control.need(link.is_symlink() and link.resolve(strict=True) == modules.resolve(strict=True), 'RP_DEPENDENCIES_LINK_CHANGED')
    else:
        link.symlink_to(modules, target_is_directory=True)
    environment = task.json(TASK / 'https-environment.json')
    task.environment_binding(environment, task.state['mode'])
    phase = environment['phase21']
    control.need(phase['contractVersion'] == 'unified-auth-phase21/1', 'RP_CONFIG_CONTRACT_INVALID')
    public = phase['rp']['third']
    control.need(set(public) == ALLOWED and all(isinstance(value, str) for value in public.values())
                 and public['OIDC_ISSUER'] == 'https://auth.saas.test'
                 and public['RP_ORIGIN'] == 'https://customer.saas.test:24446'
                 and public['RP_BIND_HOST'] == '127.0.0.1' and public['RP_BIND_PORT'] == '23102'
                 and public['OIDC_SCOPES'] == 'openid profile', 'RP_PUBLIC_CONFIG_INVALID')
    task.checked(Path(public['OIDC_CLIENT_SECRET_FILE']))
    task.checked(Path(public['NODE_EXTRA_CA_CERTS']))
    node = TASK / 'phase21-node-runtime/bin/node'
    task.checked(node, private=False, maximum=256 * 1024 * 1024)
    child_environment = {**public, 'PATH': str(node.parent) + ':/usr/bin:/bin', 'HOME': str(TASK), 'LANG': 'C.UTF-8'}
    control.need(not any('DATABASE' in name or 'SERVICE' in name or 'OWNER' in name for name in child_environment), 'RP_PRIVATE_ENV_REJECTED')
    record = TASK / 'phase23-rp.pid'
    control.need(not record.exists(), 'RP_RECORD_EXISTS')
    original = None
    child = None
    def stop(*_args):
        raise SystemExit(0)
    for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(signum, stop)
    log_path = TASK / 'phase23-rp-child.log'
    descriptor = task.open_log(log_path)
    try:
        # A caught termination signal must wait until the Node reference and
        # native lease are assigned, so finally can always reap this child.
        with control.spawn_signals(defer=True):
            child = subprocess.Popen([str(node), str(source / 'server.mjs')], cwd=source,
                                     env=child_environment, stdout=descriptor, stderr=subprocess.STDOUT)
            original = {'contractVersion': 'unified-auth-phase23-rp-process/1', 'supervisor': os.getpid(),
                'pid': child.pid, 'startTime': control.start_time(child.pid), 'namespace': expected_namespace,
                'script': str(source / 'server.mjs'), 'binary': str(node),
                'snapshotSha256': manifest['snapshotSha256'], 'rpInputCount': 14}
            task.write(record, original)
        print('PHASE23_FROZEN_STANDARD_RP_STARTED inputs=14', flush=True)
        while child.poll() is None:
            if not (TASK / 'namespace.pid').exists():
                break
            current = task.json(TASK / 'namespace.pid', private=False)
            control.need(current == namespace, 'RP_NAMESPACE_LEASE_CHANGED')
            time.sleep(.25)
        if child.poll() not in (None, 0):
            raise RuntimeError('PHASE23_RP_CHILD_FAILED')
    finally:
        with control.spawn_signals(defer=True):
            os.close(descriptor)
            if child is not None and child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=3)
            if record.exists():
                control.need(original is not None and task.json(record) == original, 'RP_RECORD_CHANGED')
                record.unlink()
            print('PHASE23_FROZEN_STANDARD_RP_STOPPED', flush=True)


if __name__ == '__main__':
    try:
        main()
    except Exception:
        print('PHASE23_STANDARD_RP_FAILED', file=sys.stderr)
        raise SystemExit(1) from None
