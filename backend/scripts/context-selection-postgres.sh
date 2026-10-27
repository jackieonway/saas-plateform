#!/usr/bin/env bash
# Native, task-owned PostgreSQL for context-selection contract tests. No Docker.
# Waits for <printed-directory>/finish, then removes only its own data and logs.
# EXIT/signal cleanup and a bounded lease also cover failed/interrupted tests.
set -euo pipefail

port="${1:-55437}"
lease_seconds="${2:-1800}"
[[ "$lease_seconds" =~ ^[0-9]{3,4}$ ]] && (( lease_seconds >= 300 && lease_seconds <= 7200 ))
[[ "$port" =~ ^[0-9]{4,5}$ ]] && (( port >= 1024 && port <= 65535 ))
pg_bin=/usr/lib/postgresql/18/bin
test -x "$pg_bin/initdb"
task_dir=$(mktemp -d /tmp/rust-saas-context-selection.XXXXXX)

cleanup() {
    if "$pg_bin/pg_ctl" -D "$task_dir/data" status >/dev/null 2>&1; then
        "$pg_bin/pg_ctl" -D "$task_dir/data" -m immediate -w stop || return 1
    fi
    case "$task_dir" in
        /tmp/rust-saas-context-selection.??????)
            test "$(realpath "$task_dir")" = "$task_dir" || return 1
            rm -rf -- "$task_dir"
            ;;
        *) return 1 ;;
    esac
    echo TASK_POSTGRES_CLEANED
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

"$pg_bin/initdb" -D "$task_dir/data" --auth=trust -U postgres \
    --no-locale --encoding=UTF8 >"$task_dir/init.log"
"$pg_bin/pg_ctl" -D "$task_dir/data" -l "$task_dir/postgresql.log" \
    -o "-h 127.0.0.1 -p $port -k $task_dir" -w start
echo "TASK_POSTGRES_READY=$task_dir"
for ((attempt=0; attempt<lease_seconds; attempt++)); do
    test ! -e "$task_dir/finish" || exit 0
    sleep 1
done
exit 1
