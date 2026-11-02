#!/usr/bin/env bash
# A private mount namespace changes only this task's /etc/hosts view. The
# existing hosts file, Windows HTTPS listener and application services stay put.
set -euo pipefail
tool_dir=${TASK_TOOL_DIR:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)}
gateway_script=${TASK_GATEWAY_SCRIPT:-gateway.py}
case "$gateway_script" in gateway.py|phase23-gateway.py) ;; *) exit 2 ;; esac
command_name=${1:-}
shift || true
task_dir=
workspace=
static_workspace=
service=
while (($#)); do
  case "$1" in
    --task-dir) task_dir=$2; shift 2 ;;
    --workspace) workspace=$2; shift 2 ;;
    --static-workspace) static_workspace=$2; shift 2 ;;
    --service) service=$2; shift 2 ;;
    *) printf 'Unknown environment argument\n' >&2; exit 2 ;;
  esac
done
if [[ ! $task_dir =~ ^/tmp/rust-saas-unified-auth-[A-Za-z0-9_-]+$ ]] ||
   [[ $(realpath -e -- "$task_dir") != "$task_dir" ]] ||
   [[ ! -f $task_dir/OWNER ]] ||
   [[ $(cat "$task_dir/OWNER") != "$(basename "$task_dir")"-pg* ]]; then
  printf 'Task directory ownership check failed\n' >&2
  exit 2
fi
if [[ $(id -u) != 0 ]]; then
  printf 'Run the namespace controller as the WSL root user\n' >&2
  exit 2
fi

namespace_pid() {
  python3 - "$task_dir" <<'PY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1]);record=json.loads((root/'namespace.pid').read_text())
pid=record['pid'];current=pathlib.Path('/proc/'+str(pid))
start=current.joinpath('stat').read_text().rsplit(')',1)[1].split()[19]
if start!=record['startTime'] or current.joinpath('ns/mnt').readlink().as_posix()!=record['namespace']:
 raise SystemExit('Task namespace process no longer matches its ownership record')
if str(root).encode() not in current.joinpath('cmdline').read_bytes():
 raise SystemExit('Task namespace command does not match')
print(pid)
PY
}

case "$command_name" in
  start)
    if [[ -e $task_dir/namespace.pid || -e $task_dir/gateway.pid ]]; then
      printf 'Refusing to replace an existing namespace ownership record\n' >&2
      exit 2
    fi
    workspace=$(realpath -e -- "$workspace")
    static_workspace=$(realpath -e -- "${static_workspace:-$workspace}")
    [[ -d $static_workspace/saas-plateform/frontend/dist && -f $workspace/.local-run/server.pem ]]
    [[ $static_workspace == "$workspace" || $static_workspace == "$task_dir/"* ]]
    awk '!/auth[.]saas[.]test|console[.]saas[.]test|customer[.]saas[.]test/' /etc/hosts > "$task_dir/hosts"
    printf '\n127.0.0.2 auth.saas.test console.saas.test customer.saas.test\n' >> "$task_dir/hosts"
    # The bind mount is read as /etc/hosts by the unprivileged service user.
    # Its contents are public host mappings, while the task directory is private.
    chmod 644 "$task_dir/hosts"
    rm -f -- "$task_dir/namespace.stop"
    # Bash reads later commands from its script after the long-running loop.
    # Freeze this controller so editing the reusable source cannot change that
    # file offset or turn a normal shutdown into a parser error.
    cp -- "$tool_dir/environment.sh" "$task_dir/environment-controller.sh"
    chmod 600 "$task_dir/environment-controller.sh"
    exec env TASK_TOOL_DIR="$tool_dir" unshare --mount --propagation private \
      bash "$task_dir/environment-controller.sh" inside \
      --task-dir "$task_dir" --workspace "$workspace" --static-workspace "$static_workspace"
    ;;
  inside)
    workspace=$(realpath -e -- "$workspace")
    mount --bind "$task_dir/hosts" /etc/hosts
    export TASK_NAMESPACE_PID=$$
    python3 - "$task_dir" <<'PY'
import json,os,pathlib,sys
root=pathlib.Path(sys.argv[1]);pid=int(os.environ['TASK_NAMESPACE_PID'])
namespace=os.readlink('/proc/'+str(pid)+'/ns/mnt')
start=pathlib.Path('/proc/'+str(pid)+'/stat').read_text().rsplit(')',1)[1].split()[19]
(root/'namespace.pid').write_text(json.dumps({'pid':pid,'startTime':start,'namespace':namespace}))
(root/'namespace.identity').write_text(namespace+'\n')
PY
    gateway_pid=
    cleanup() {
      trap - EXIT HUP INT TERM
      # Removing the lease asks task service supervisors and the Windows proxy
      # to close their own children and connections even on an abnormal exit.
      rm -f -- "$task_dir/namespace.pid" "$task_dir/namespace.identity"
      if [[ -n $gateway_pid ]] && kill -0 "$gateway_pid" 2>/dev/null; then
        kill "$gateway_pid" 2>/dev/null || true
        wait "$gateway_pid" 2>/dev/null || true
      fi
      rm -f -- "$task_dir/gateway.pid"
      printf 'TASK_NAMESPACE_CLEANUP_DONE\n'
    }
    trap 'cleanup; exit 0' HUP INT TERM
    trap cleanup EXIT
    python3 -u "$tool_dir/$gateway_script" --task-dir "$task_dir" --workspace "$workspace" \
      --static-workspace "$static_workspace" \
      > "$task_dir/gateway.log" 2>&1 &
    gateway_pid=$!
    printf '%s\n' "$gateway_pid" > "$task_dir/gateway.pid"
    while [[ ! -f $task_dir/namespace.stop && -f $task_dir/OWNER ]]; do
      if ! kill -0 "$gateway_pid" 2>/dev/null; then
        printf 'Task gateway exited; inspect its task-owned log\n' >&2
        exit 1
      fi
      sleep 1
    done
    ;;
  run)
    case "$service" in identity|identity-peer|platform|portal) ;; *) exit 2 ;; esac
    task_namespace=$(namespace_pid)
    task_uid=$(stat -c %u "$task_dir/OWNER")
    task_gid=$(stat -c %g "$task_dir/OWNER")
    [[ $task_uid != 0 ]]
    exec nsenter -t "$task_namespace" -m -- setpriv --reuid="$task_uid" \
      --regid="$task_gid" --init-groups python3 -u "$tool_dir/run-service.py" \
      --task-dir "$task_dir" "$service"
    ;;
  stop-service)
    case "$service" in identity|identity-peer|platform|portal) ;; *) exit 2 ;; esac
    namespace_pid >/dev/null
    python3 - "$task_dir" "$service" <<'PY'
import json,os,pathlib,signal,sys,time
root=pathlib.Path(sys.argv[1]);name=sys.argv[2];record=root/('service-'+name+'.pid')
if not record.exists():
 print('TASK_SERVICE_ALREADY_STOPPED '+name);raise SystemExit(0)
owned=json.loads(record.read_text());pid=owned['pid'];proc=pathlib.Path('/proc/'+str(pid))
expected=str(root/'bin'/{'identity':'saas-identity','identity-peer':'saas-identity','platform':'saas-platform-backend','portal':'saas-customer-portal-backend'}[name])
def matches():
 try:
  start=proc.joinpath('stat').read_text().rsplit(')',1)[1].split()[19]
  executable=os.readlink(proc/'exe')
  return start==owned['startTime'] and executable in (expected,expected+' (deleted)') and os.readlink(proc/'ns/mnt')==owned['namespace']
 except OSError:return False
if not matches():raise SystemExit('Refusing to stop an unmatched task service process')
os.kill(pid,signal.SIGTERM)
for _ in range(120):
 if not record.exists():break
 time.sleep(.1)
if record.exists() and matches():os.kill(pid,signal.SIGKILL)
for _ in range(40):
 if not record.exists():break
 time.sleep(.1)
if record.exists():raise SystemExit('Task service did not release its ownership record')
print('TASK_SERVICE_STOPPED '+name)
PY
    ;;
  stop)
    if [[ ! -f $task_dir/namespace.pid ]]; then
      printf 'TASK_NAMESPACE_ALREADY_STOPPED\n'
      exit 0
    fi
    task_namespace=$(namespace_pid)
    touch "$task_dir/namespace.stop"
    for _ in {1..20}; do
      [[ ! -f $task_dir/namespace.pid ]] && break
      sleep 1
    done
    if [[ -f $task_dir/namespace.pid ]]; then
      [[ $(namespace_pid) == "$task_namespace" ]]
      kill -TERM "$task_namespace"
    fi
    printf 'TASK_NAMESPACE_STOP_REQUESTED\n'
    ;;
  status)
    printf 'TASK_NAMESPACE_PID %s\n' "$(namespace_pid)"
    printf 'TASK_GATEWAY_PID %s\n' "$(cat "$task_dir/gateway.pid")"
    ;;
  *) printf 'Usage: environment.sh start|run|stop-service|stop|status --task-dir /tmp/rust-saas-unified-auth-NAME [--workspace ROOT] [--service identity|platform|portal]\n' >&2; exit 2 ;;
esac
