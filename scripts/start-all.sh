#!/usr/bin/env sh
# All default static frontends, supervised in the foreground.
# Backend/notification/OAuth require explicit --with-* flags; no database startup.
set -eu
cd "$(dirname "$0")/.."
exec node scripts/dev.mjs "$@"
