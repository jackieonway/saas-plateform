# Independent Nginx edge and ZIP compatibility service

This directory contains real Nginx configuration renderers, not an application
server. Node is used to validate deployment input and write a configuration file
before deployment. Runtime images execute Nginx as UID/GID 101 and do not run a
configuration shell, Node, or environment substitution.

Copy `config.example.json` to an operator-owned file outside the checkout, replace
the explicit example origin, and render before starting Compose:

```sh
node deploy/gateway/render.mjs /secure/platform-gateway.json /secure/nginx.conf
```

Mount that file read-only at `/etc/nginx/nginx.conf`, the certificate chain at
`/etc/nginx/tls/fullchain.pem`, and its key at
`/etc/nginx/tls/privkey.pem`. UID 101 must be able to read these mounted files;
grant only the required ACL/group access. A read-only root filesystem needs a
writable `/tmp` tmpfs for the PID and request/proxy temporary files. Configuration
contains `daemon off`, so invoke `nginx -c /etc/nginx/nginx.conf` without another
`-g 'daemon off;'`. Container ports 8080/8443 map to the public 80/443 only on the
edge. Static and Rust services must not publish host ports.

The exact `publicOrigin` must use HTTPS, without credentials, a path, query,
fragment, or trailing slash. It controls canonical redirects and forwarded
Host/Proto; incoming Host cannot redirect users to an arbitrary origin. The
certificate must be valid for this hostname and clients must verify it normally.
This renderer supports controlled same-host private HTTP upstreams only. Moving
across an untrusted network requires a separately reviewed HTTPS upstream with
verified CA/SNI; do not disable verification or expose the private services.

`backend` and `frontend` default to `backend:3001` and `frontend-web:8080`.
`notification`, `workOrders`, `stores`, and `zipStatic` default to null: absent services
return 404 rather than routing into the host. Set the optional targets only when
the corresponding service is enabled. Each target is a literal `service:port`,
not an arbitrary URL. `allowedServices` can explicitly replace the default Docker
service-name allowlist. Exact private IPs additionally require
`allowedUpstreamIps`; public/link-local addresses, URL syntax, directives and
commands are rejected. Service variables with Docker resolver `127.0.0.11`
re-resolve after a five-second cache TTL, allowing an upstream container to be
recreated with a different IP. This recovery is a required runtime acceptance
check, not a guarantee inferred from the config text.

The edge overwrites forwarded client headers. `trustedProxyIps` is empty by
default. Only if a controlled load balancer is actually in front of Nginx should
its exact private IP be included; CIDRs, public IPs, hostnames, and wildcard trust
are refused. The Rust platform's `TRUSTED_PROXY_IPS` is a separate setting: it must
contain the actual connecting edge-container IP. It must not contain frontend
container IPs or the public origin. With the default direct edge, the client's
supplied X-Forwarded-For never becomes the forwarded user IP.

`/api` and `/oauth` preserve their prefixes; business and Socket.IO routes also
preserve prefix/query/body/Authorization/Origin. Socket.IO supports polling POST
and WebSocket Upgrade at the transport layer. Rust owns business CORS. The edge
does not append business CORS, cache successful API responses, mask upstream 404s, or
globally loosen management API origins. Private metrics/internal and Vite dev
paths are denied. Release operations have a separate 65-second proxy budget.

The optional `stores` target proxies `/stores-api` to its independent Rust
service, with the original prefix and business CORS headers preserved. Its
`/stores-api/metrics` path remains private. Native rendering accepts the same
optional target with its existing exact private-IP policy. This route supports
Phase 27 integration; stores image/version URL publication is a later delivery
step and is not configured automatically.

Phase32 also denies `/work-orders-api/internal/`, `/stores-api/internal/`,
`/api/internal/`, `/internal/` and notification metrics before public proxy
locations. An accepted private credential must still receive 404 at the edge.

Immutable micro-frontend images are registered by explicit `versionRoutes`, for
example:

```json
{
  "app": "orders",
  "version": "1.2.0",
  "upstream": "orders-v120:8080"
}
```

Also allow `orders-v120` in `allowedServices`. The service must itself serve
anonymous static bytes at `/` with the required MIME/cache/iframe CSP headers.
Keep old route/container/digest combinations reachable. This is an operator
deployment mapping; registering a URL in the platform does not create it.
Unknown `/sub-apps` versions return 404. The edge owns the full host CSP: it
replaces the internal host service's basic CSP/X-Frame-Options on the host route,
then emits one full CSP and `X-Frame-Options: DENY`. `applicationOrigins` explicitly
adds approved HTTPS frame/connect targets; `trustedHtmlEval` allows eval only on
this host policy for an audited trusted runtime. Both default to the opaque-safe
policy. Version/static routes retain the upstream's own iframe CSP and CORS and
do not inherit the host CSP. Their cache headers are not duplicated by the edge.

Branding logos/avatars hosted elsewhere require an explicit `imageOrigins` array
of approved exact HTTPS origins, for example `["https://images.example.com"]`.
The default image policy allows only self/data/blob sources; it does not permit
every HTTPS host. Image origins do not expand connect/frame/script sources.
This field is also available in native and ZIP static configurations, so an
approved branding image can remain usable without weakening application policy.

The recommended image deployment has no release-volume mount. To enable the ZIP
compatibility profile, render a second configuration:

```sh
node deploy/gateway/render.mjs /secure/micro-static.json /secure/micro-static.conf
```

Start from `micro-static.example.json`, keep `kind: "micro-static"`, and set its
same exact public origin. `Dockerfile.micro-static` is a separate non-root static
service. Mount its configuration read-only and the persistent release directory
read-only at `/micro-apps`; only the Rust platform receives the writable mount.
Set the edge `zipStatic` to `micro-static:8080`. The edge itself never mounts the
release directory. Only `/sub-apps/<app>/versions/<version>/...` is served;
dotfiles, staging/incoming/tmp paths, symlinks and unknown files remain 404.
Recreate the static container and verify the persisted version bytes survive.

ZIP static responses use anonymous `Access-Control-Allow-Origin: *`, correct
JavaScript types including mjs/cjs, HTML `no-cache`, version resources immutable,
and errors `no-store`. They do not send credentialed CORS or a frame-denial
header. CSP permits framing only by `publicOrigin`; `applicationOrigins` can add
exact HTTPS origins to resource connections/frames. The default opaque iframe
policy does not permit eval. `trustedHtmlEval: true` is an explicit exceptional
deployment policy for audited trusted HTML; never enable it as an opaque-iframe
workaround. Confirm headers and modules in real HTTPS/browser tests.

`node --test tests/nginx-render-policy.mjs tests/production-native-config.mjs`
validates configuration input/security policy. Real `nginx -t`, non-root
read-only containers, TLS, DNS recreation and HTTP transport checks are separate
required gates; absence of Docker/Nginx is not a passing runtime result.
