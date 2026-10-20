#!/usr/bin/env sh
# Compatibility entry; npm run dev works directly on Windows and POSIX.
set -eu
cd "$(dirname "$0")/.."
exec node scripts/dev.mjs "$@"
