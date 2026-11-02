import { readFileSync, writeFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'
import {
  absolutePath, apiLocations, commonHttp, configuration, contentSecurityPolicy, exactOrigin, knownFields, listener,
  origins, privateLocations, proxyHeaders, quote, staticHeaders, staticMaps, token,
  upstreamPolicy, validateTrustedProxyIps, version, zipLocations,
} from './policy.mjs'

export { validateTrustedProxyIps } from './policy.mjs'

/** Render a single independent TLS edge. All HTTP targets are private Docker DNS
 * services or exact private IPs selected by the operator. External arbitrary URL
 * proxying is deliberately unavailable. Cross-host/untrusted transport needs a
 * separately reviewed HTTPS-upstream deployment rather than disabling TLS. */
export function renderGateway(input) {
  knownFields(input, ['publicOrigin', 'listener', 'httpListener', 'certificate', 'certificateKey', 'mimeTypes', 'backend', 'frontend', 'notification', 'workOrders', 'stores', 'zipStatic', 'allowedServices', 'allowedUpstreamIps', 'trustedProxyIps', 'versionRoutes', 'applicationOrigins', 'imageOrigins', 'trustedHtmlEval'])
  const origin = exactOrigin(input.publicOrigin)
  const upstream = upstreamPolicy(input)
  const backend = upstream(input.backend ?? 'backend:3001')
  const frontend = upstream(input.frontend ?? 'frontend-web:8080')
  const notification = input.notification == null ? null : upstream(input.notification)
  const workOrders = input.workOrders == null ? null : upstream(input.workOrders)
  const stores = input.stores == null ? null : upstream(input.stores)
  const zipStatic = input.zipStatic == null ? null : upstream(input.zipStatic)
  const trusted = validateTrustedProxyIps(input.trustedProxyIps)
  const hostCsp = contentSecurityPolicy({ origin, applicationOrigins: input.applicationOrigins, imageOrigins: input.imageOrigins, trustedHtmlEval: input.trustedHtmlEval })
  const routes = input.versionRoutes ?? []
  if (!Array.isArray(routes)) throw new Error('versionRoutes must be an array')
  const seen = new Set()
  const versionLocations = routes.map((route, index) => {
    knownFields(route, ['app', 'version', 'upstream'])
    const app = token(route.app, 'app'), release = version(route.version), target = upstream(route.upstream)
    const prefix = `/sub-apps/${app}/versions/${release}/`
    if (seen.has(prefix)) throw new Error('duplicate immutable version route')
    seen.add(prefix)
    return `        location ^~ ${prefix} {
            set $micro_version_${index} ${quote(target)};
            rewrite ^${prefix.replaceAll('.', '\\.')}(.*)$ /$1 break;
            proxy_http_version 1.1;
${proxyHeaders(origin)}            proxy_pass http://$micro_version_${index};
            proxy_hide_header X-Content-Type-Options;
            add_header X-Content-Type-Options "nosniff" always;
        }
`
  }).join('')
  const zip = zipStatic ? `        location /sub-apps/ {
            set $zip_static ${quote(zipStatic)};
${proxyHeaders(origin)}            proxy_pass http://$zip_static$request_uri;
            proxy_hide_header X-Content-Type-Options;
            add_header X-Content-Type-Options "nosniff" always;
        }
` : '        location /sub-apps/ { return 404; }\n'
  return configuration(commonHttp(input) + `    # Variables force fresh Docker service lookup after the resolver TTL.
    resolver 127.0.0.11 valid=5s ipv6=off;
    resolver_timeout 3s;
    server {
        listen ${listener(input.httpListener ?? '0.0.0.0:8080')};
        server_name ${origin.hostname};
        return 308 ${origin.origin}$request_uri;
    }
    server {
        listen ${listener(input.listener ?? '0.0.0.0:8443')} ssl;
        http2 on;
        server_name ${origin.hostname};
        ssl_certificate ${quote(absolutePath(input.certificate ?? '/etc/nginx/tls/fullchain.pem', 'certificate'))};
        ssl_certificate_key ${quote(absolutePath(input.certificateKey ?? '/etc/nginx/tls/privkey.pem', 'certificateKey'))};
        ssl_protocols TLSv1.2 TLSv1.3;
        ssl_session_cache shared:TLS:10m;
        ssl_session_tickets off;
        add_header Cache-Control "no-store" always;
        add_header X-Content-Type-Options "nosniff" always;
        if ($host != ${quote(origin.hostname)}) { return 421; }
${trusted.map((ip) => `        set_real_ip_from ${ip};\n`).join('')}${trusted.length ? '        real_ip_header X-Forwarded-For;\n        real_ip_recursive off;\n' : ''}
${privateLocations()}${apiLocations(origin, { backend: { name: 'platform', address: backend }, notification: notification && { name: 'notification', address: notification }, workOrders: workOrders && { name: 'work_orders', address: workOrders }, stores: stores && { name: 'stores', address: stores } })}        location = /sub-apps { return 404; }
${versionLocations}${zip}        # URL applications use the backend's allowlisted static proxy, never host SPA fallback.
        location = /micro-proxy { return 404; }
        location ^~ /micro-proxy/ {
            set $micro_proxy_backend ${quote(backend)};
            proxy_http_version 1.1;
${proxyHeaders(origin)}            proxy_set_header Upgrade "";
            proxy_set_header Connection "";
            proxy_pass http://$micro_proxy_backend$request_uri;
        }
        location / {
            set $host_static ${quote(frontend)};
            proxy_http_version 1.1;
${proxyHeaders(origin)}            proxy_pass http://$host_static$request_uri;
            proxy_hide_header Content-Security-Policy;
            proxy_hide_header X-Frame-Options;
            proxy_hide_header X-Content-Type-Options;
            add_header Content-Security-Policy ${quote(hostCsp)} always;
            add_header X-Frame-Options "DENY" always;
            add_header X-Content-Type-Options "nosniff" always;
        }
    }
`)
}

/** Only a ZIP compatibility static service mounts the release volume read-only.
 * Staging, dot files, unknown versions and arbitrary host navigations stay 404. */
export function renderMicroStatic(input) {
  knownFields(input, ['publicOrigin', 'listener', 'microRoot', 'mimeTypes', 'applicationOrigins', 'imageOrigins', 'trustedHtmlEval'])
  const origin = exactOrigin(input.publicOrigin)
  origins(input.applicationOrigins)
  const headers = staticHeaders({ origin, applicationOrigins: input.applicationOrigins, imageOrigins: input.imageOrigins, trustedHtmlEval: input.trustedHtmlEval, micro: true })
  return configuration(commonHttp(input) + staticMaps() + `    server {
        listen ${listener(input.listener ?? '0.0.0.0:8080')};
        server_name _;
        autoindex off;
${headers}        location = /healthz { add_header Cache-Control "no-store" always; return 200 "ok\\n"; }
${zipLocations({ origin, microRoot: input.microRoot ?? '/micro-apps', applicationOrigins: input.applicationOrigins, imageOrigins: input.imageOrigins, trustedHtmlEval: input.trustedHtmlEval })}        location / { return 404; }
    }
`)
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [source, destination] = process.argv.slice(2)
  if (!source || !destination || process.argv.length !== 4) throw new Error('usage: node backend/nginx/render.mjs <deployment.json> <nginx.conf>')
  const { kind = 'gateway', ...input } = JSON.parse(readFileSync(source, 'utf8'))
  const rendered = kind === 'gateway' ? renderGateway(input) : kind === 'micro-static' ? renderMicroStatic(input) : null
  if (!rendered) throw new Error('kind must be gateway or micro-static')
  writeFileSync(destination, rendered, { mode: 0o600 })
}
