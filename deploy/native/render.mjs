import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import {
  absolutePath, apiLocations, commonHttp, configuration, exactOrigin, knownFields, listener,
  origins, privateLocations, quote, staticHeaders, staticMaps, upstreamPolicy, zipLocations,
} from '../gateway/policy.mjs'

/** Retain the original native fixture interface while using the same security
 * policy as the independent Docker edge. This produces Nginx only. Native
 * upstreams/listener default to loopback; private IPs require an allowlist. */
export function render(input) {
  knownFields(input, ['publicOrigin', 'listener', 'instanceRoot', 'hostRoot', 'microRoot', 'certificate', 'certificateKey', 'mimeTypes', 'backend', 'notification', 'workOrders', 'stores', 'applicationOrigins', 'imageOrigins', 'trustedHtmlEval', 'allowedUpstreamIps'])
  const origin = exactOrigin(input.publicOrigin)
  origins(input.applicationOrigins)
  const instanceRoot = absolutePath(input.instanceRoot, 'instanceRoot')
  const hostRoot = absolutePath(input.hostRoot, 'hostRoot')
  const upstream = upstreamPolicy({ allowedServices: [], allowedUpstreamIps: input.allowedUpstreamIps })
  const backend = upstream(input.backend), notification = input.notification == null ? null : upstream(input.notification), workOrders = input.workOrders == null ? null : upstream(input.workOrders)
  const stores = input.stores == null ? null : upstream(input.stores)
  const hostHeaders = staticHeaders({ origin, applicationOrigins: input.applicationOrigins, imageOrigins: input.imageOrigins, trustedHtmlEval: input.trustedHtmlEval })
  // Native trustedHtmlEval selects the host's audited TRUSTED loader only.
  // Serving a version never silently upgrades an opaque iframe's script policy.
  const microHeaders = staticHeaders({ origin, applicationOrigins: input.applicationOrigins, imageOrigins: input.imageOrigins, micro: true, cache: '$host_response_cache' })
  return { 'nginx.conf': configuration(commonHttp({ mimeTypes: input.mimeTypes, temporaryRoot: `${instanceRoot}/nginx-temp`, accessLog: `${instanceRoot}/nginx-access.log` }) + staticMaps() + `    server {
        listen ${listener(input.listener, { loopback: true })} ssl;
        server_name ${origin.hostname};
        ssl_certificate ${quote(absolutePath(input.certificate, 'certificate'))};
        ssl_certificate_key ${quote(absolutePath(input.certificateKey, 'certificateKey'))};
        ssl_protocols TLSv1.2 TLSv1.3;
        ssl_session_tickets off;
        root ${quote(hostRoot)};
        index index.html;
        autoindex off;
${hostHeaders}${privateLocations()}${apiLocations(origin, { backend: { name: 'platform', address: backend }, notification: notification && { name: 'notification', address: notification }, workOrders: workOrders && { name: 'work_orders', address: workOrders }, stores: stores && { name: 'stores', address: stores } })}${zipLocations({ origin, microRoot: input.microRoot, applicationOrigins: input.applicationOrigins, imageOrigins: input.imageOrigins })}        location ~ ^/sub-apps/(?:vue|jquery)/ {
            try_files $uri $uri/ =404;
            disable_symlinks on;
${microHeaders}        }
        location /assets/ {
            try_files $uri =404;
${hostHeaders}        }
        location / { try_files $uri $uri/ @host_spa; }
        location @host_spa {
            if ($request_method !~ ^(?:GET|HEAD)$) { return 404; }
            if ($host_accept_html = 0) { return 404; }
            if ($uri ~ \\.) { return 404; }
            if ($uri !~ ^/(?:dashboard|profile|login|system|athena|hermes|micro|tenant)(?:/|$)) { return 404; }
            try_files /index.html =404;
${hostHeaders}        }
    }
`, { pid: `${instanceRoot}/nginx.pid`, errorLog: `${instanceRoot}/nginx-error.log` }) }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const args = process.argv.slice(2)
  if (args.length !== 4 || args[0] !== '--config' || args[2] !== '--out-dir') throw new Error('usage: node deploy/native/render.mjs --config <deployment.json> --out-dir <output-directory>')
  const result = render(JSON.parse(readFileSync(args[1], 'utf8')))
  const destination = path.resolve(args[3])
  mkdirSync(destination, { recursive: true, mode: 0o700 })
  writeFileSync(path.join(destination, 'nginx.conf'), result['nginx.conf'], { mode: 0o600 })
}
