#!/bin/sh
# Production-target transport uses Compose, never a Node process or Docker API
# exposed to the platform. This script does not modify routing or register URLs.
set -eu
if [ "$#" -ne 3 ]; then
  printf '%s\n' 'Usage: pull-start.sh /absolute/checkout /secure/platform.env /secure/business.env' >&2
  exit 2
fi
checkout=$1
platform_env=$2
business_env=$3
case "$checkout:$platform_env:$business_env" in /*:/*:/*) ;; *) printf '%s\n' 'All paths must be absolute' >&2; exit 2 ;; esac
compose() {
  docker compose --project-directory "$checkout" --env-file "$platform_env" --env-file "$business_env" \
    -f "$checkout/docker-compose.yml" -f "$checkout/deploy/compose.business.yml" "$@"
}
compose config --quiet
compose config --images | while IFS= read -r image; do
  if ! printf '%s\n' "$image" | LC_ALL=C grep -Eq '^[a-z0-9][a-z0-9./:_-]*@sha256:[a-f0-9]{64}$'; then
    printf '%s\n' 'Every Compose image must use an immutable registry digest' >&2
    exit 1
  fi
done
compose pull work-orders-api stores-api work-orders-v1 work-orders-v2 stores-v1 stores-v2
compose up --detach --no-build --pull never --wait --wait-timeout 180 \
  work-orders-api stores-api work-orders-v1 work-orders-v2 stores-v1 stores-v2
# Recheck current health after up; a previously healthy dependency can degrade.
for service in work-orders-api stores-api work-orders-v1 work-orders-v2 stores-v1 stores-v2; do
  container=$(compose ps --quiet "$service")
  [ -n "$container" ]
  health=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}missing{{end}}' "$container")
  [ "$health" = healthy ] || { printf '%s\n' "Unhealthy service: $service" >&2; exit 1; }
done
printf '%s\n' 'Fixed images pulled and healthy. Routing and platform registration require their separate controlled steps.'
