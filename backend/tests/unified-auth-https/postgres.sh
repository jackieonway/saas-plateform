#!/usr/bin/env bash
# Foreground task supervisor. Stop by creating TASK/cleanup.request; EXIT and
# signal traps stop the namespace and this cluster before removing owned files.
set -euo pipefail
tool_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
task_dir=
port=55494
pg_bin=/usr/lib/postgresql/18/bin
while (($#)); do
  case "$1" in
    --task-dir) task_dir=$2; shift 2 ;;
    --port) port=$2; shift 2 ;;
    --pg-bin) pg_bin=$2; shift 2 ;;
    *) printf 'Unknown PostgreSQL argument\n' >&2; exit 2 ;;
  esac
done
if [[ ! $task_dir =~ ^/tmp/rust-saas-unified-auth-[A-Za-z0-9_-]+$ ]] ||
   [[ ! $port =~ ^[0-9]+$ ]] || ((port < 1024 || port > 65535)) ||
   [[ $(id -u) == 0 ]] || [[ -e $task_dir ]]; then
  printf 'A new dedicated task path, unprivileged user and valid port are required\n' >&2
  exit 2
fi
if ss -ltn | awk '{print $4}' | grep -Eq "[:.]${port}$"; then
  printf 'Task PostgreSQL port is occupied\n' >&2
  exit 2
fi
pg_bin=$(realpath -e -- "$pg_bin")
[[ -x $pg_bin/initdb && -x $pg_bin/pg_ctl ]]
printf 'TASK_BASELINE_DISK\n'
df -B1 /tmp
if command -v docker >/dev/null; then
  printf 'TASK_DOCKER_CONTEXT\n'
  docker context show
  docker ps -a --format '{{.ID}} {{.Names}} {{.Status}}'
  docker volume ls
  docker network ls
  docker system df
fi
umask 077
mkdir -- "$task_dir"
marker="$(basename "$task_dir")-pg${port}"
printf '%s\n' "$marker" > "$task_dir/OWNER"
cleanup() {
  trap - EXIT HUP INT TERM
  if [[ $(realpath -e -- "$task_dir") != "$task_dir" ]] ||
     [[ $(cat "$task_dir/OWNER") != "$marker" ]]; then
    printf 'CLEANUP_REFUSED: task ownership mismatch\n' >&2
    exit 2
  fi
  # The root namespace controller normally stops before this sentinel. If an
  # abnormal supervisor exit happens first, the lease still releases children.
  if [[ -f $task_dir/namespace.pid ]]; then
    touch "$task_dir/namespace.stop"
    for _ in {1..25}; do
      [[ ! -f $task_dir/namespace.pid ]] && break
      sleep 1
    done
    if [[ -f $task_dir/namespace.pid ]]; then
      printf 'CLEANUP_BLOCKED: stop the owned mount namespace as root first\n' >&2
      exit 3
    fi
  fi
  for _ in {1..12}; do
    if ! find "$task_dir" -maxdepth 1 \( -name 'service-*.pid' -o -name 'browser-proxy.pid' \) -print -quit | grep -q .; then break; fi
    sleep 1
  done
  if find "$task_dir" -maxdepth 1 \( -name 'service-*.pid' -o -name 'browser-proxy.pid' \) -print -quit | grep -q .; then
    printf 'CLEANUP_BLOCKED: owned service supervisors have not released their records\n' >&2
    exit 3
  fi
  printf 'TASK_CLEANUP_BYTES '; du -sb -- "$task_dir" | cut -f1
  if command -v docker >/dev/null; then docker system df; fi
  if [[ -f $task_dir/data/postmaster.pid ]]; then
    "$pg_bin/pg_ctl" -D "$task_dir/data" -m fast -w stop
  fi
  [[ ! -f $task_dir/data/postmaster.pid ]]
  # Both the absolute target and exact marker were checked above. Only this
  # invocation's data/logs/keys/native build cache reside in this directory.
  rm -rf -- "$task_dir"
  printf 'TASK_CLEANUP_DONE\n'
  df -B1 /tmp
  if command -v docker >/dev/null; then docker system df; fi
}
trap 'cleanup; exit 0' HUP INT TERM
trap cleanup EXIT
"$pg_bin/initdb" -D "$task_dir/data" -U postgres --auth=trust --encoding=UTF8 \
  --locale=C > "$task_dir/initdb.log"
printf "listen_addresses = '127.0.0.1'\nport = %s\nunix_socket_directories = '%s'\nfsync = on\n" \
  "$port" "$task_dir" >> "$task_dir/data/postgresql.conf"
"$pg_bin/pg_ctl" -D "$task_dir/data" -l "$task_dir/postgres.log" -w start
printf 'postgres://postgres@127.0.0.1:%s/postgres\n' "$port" > "$task_dir/admin-url"
printf 'TASK_PG_READY port=%s\n' "$port"
while [[ ! -f $task_dir/cleanup.request ]]; do sleep 2; done
