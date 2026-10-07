#!/bin/sh
# Caller supplies the paths listed by tests/production-native-e2e.mjs.
# Test credentials are loaded only into the Node harness; individual services
# receive explicit credentials and the browser receives runtime variables only.
set -eu
task_native_node="${NODE_BIN:-node}"
task_test_env="${TEST_ENV_FILE:?Set the absolute isolated test environment file}"
task_acceptance_ca="${PRODUCTION_CA_FILE:?Set the dedicated acceptance CA file}"
case "$task_test_env" in /*) ;; *) echo 'TEST_ENV_FILE must be absolute' >&2; exit 1;; esac
case "$task_acceptance_ca" in /*) ;; *) echo 'PRODUCTION_CA_FILE must be absolute' >&2; exit 1;; esac
if [ "${NODE_TLS_REJECT_UNAUTHORIZED:-}" = 0 ]; then
  echo 'TLS verification cannot be disabled' >&2
  exit 1
fi
export NODE_EXTRA_CA_CERTS="$task_acceptance_ca"
exec "$task_native_node" --env-file="$task_test_env" --test tests/production-native-e2e.mjs
