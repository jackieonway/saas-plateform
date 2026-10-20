#!/usr/bin/env bash
# Review the complete writer inventory for the target host before installation.
set -euo pipefail
umask 077
units=(saas-platform.service saas-work-orders.service saas-stores.service saas-notification.service saas-jobs.service)
active=()
restart() {
  local status=$?
  trap - EXIT
  # Restart only units active when this window began. A failed restart makes the
  # unit fail too; SIGKILL/power loss still requires explicit operator recovery.
  for unit in "${active[@]}"; do systemctl start "$unit" || status=1; done
  exit "$status"
}
trap restart EXIT
for unit in "${units[@]}"; do
  if systemctl is-active --quiet "$unit"; then active+=("$unit"); fi
done
for unit in "${active[@]}"; do systemctl stop "$unit"; done
for unit in "${units[@]}"; do
  if systemctl is-active --quiet "$unit"; then echo 'writer failed to stop' >&2; exit 1; fi
done
# Operator inventory must also stop external writers, release jobs and direct DBA
# writes and drain their transactions. This is an assertion, not automatic discovery.
export BACKUP_WRITES_QUIESCED=true
/usr/local/bin/saas-recovery backup-new /etc/saas/recovery/config.json
