// Render non-secret native deployment configuration. Secrets stay in service
// environment files; this program never reads or copies their contents.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { isIP } from 'node:net'
import { fileURLToPath } from 'node:url'

const ownRoot = path.dirname(fileURLToPath(import.meta.url))
function absolute(value, name) {
  if (typeof value !== 'string' || !value.startsWith('/') || /[\s\0"'$`\\]/.test(value) || value.split('/').includes('..')) throw new Error(`${name} must be an absolute POSIX path without whitespace or configuration metacharacters`)
  return path.posix.normalize(value)
}
function origin(value, name) {
  try { const u = new URL(value); if (u.protocol !== 'https:' || u.origin !== value || u.username || u.password) throw new Error(); return u } catch { throw new Error(`${name} must be an exact HTTPS origin`) }
}
function address(value, name) {
  if (typeof value !== 'string' || !/^(?:127\.0\.0\.1|\[::1\]):[1-9][0-9]{0,4}$/.test(value) || Number(value.slice(value.lastIndexOf(':') + 1)) > 65535) throw new Error(`${name} must be a loopback host:port`)
  return value
}
function listenerAddress(value) {
  const match=typeof value==='string'?value.match(/^(\[[0-9a-fA-F:]+\]|[0-9.]+):([1-9][0-9]{0,4})$/):null
  if(!match || !isIP(match[1].replace(/^\[|\]$/g,'')) || Number(match[2])>65535)throw new Error('listener must be an explicit IP address and valid port')
  return value
}
export function render(input) {
  const gateway = origin(input.publicOrigin, 'publicOrigin')
  const root = absolute(input.instanceRoot, 'instanceRoot')
  const hostRoot = absolute(input.hostRoot, 'hostRoot'), microRoot = absolute(input.microRoot, 'microRoot')
  const certificate = absolute(input.certificate, 'certificate'), certificateKey = absolute(input.certificateKey, 'certificateKey')
  const mimeTypes = absolute(input.mimeTypes, 'mimeTypes')
  const backend = address(input.backend, 'backend'), notification = address(input.notification, 'notification'), workOrders = address(input.workOrders, 'workOrders')
  const externalOrigins = [...new Set((input.applicationOrigins ?? []).map((item) => origin(item, 'applicationOrigins').origin))]
  const trustedScriptOrigins=[...new Set((input.trustedScriptOrigins??[]).map((item)=>origin(item,'trustedScriptOrigins').origin))]
  const frameSources = [gateway.origin, ...externalOrigins].join(' ')
  const connectSources = [gateway.origin, gateway.origin.replace(/^https:/, 'wss:'), ...externalOrigins].join(' ')
  const scriptPolicy = input.trustedHtmlEval === true ? "'self' 'unsafe-eval'" : "'self'"
  const csp = `default-src 'self'; script-src ${scriptPolicy} ${[gateway.origin,...trustedScriptOrigins].join(' ')}; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; connect-src 'self' ${connectSources}; frame-src 'self' ${frameSources}; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'self'`
  const values = { ROOT: root, HOST_ROOT: hostRoot, MICRO_ROOT: microRoot, CERTIFICATE: certificate, CERTIFICATE_KEY: certificateKey,
    MIME_TYPES: mimeTypes, ORIGIN: gateway.origin, HOST: gateway.hostname.replace(/^\[|\]$/g, ''), LISTENER: listenerAddress(input.listener ?? `127.0.0.1:${gateway.port || 443}`),
    LISTEN_HOST: listenerAddress(input.listener ?? `127.0.0.1:${gateway.port || 443}`).replace(/:[0-9]+$/,'').replace(/^\[|\]$/g,''),
    BACKEND: backend, NOTIFICATION: notification, WORK_ORDERS: workOrders, CSP: csp,
    NODE: absolute(input.node ?? '/usr/bin/node', 'node'), BACKEND_BINARY: absolute(input.backendBinary ?? `${root}/current/backend/dante-cloud-backend`, 'backendBinary'),
    NOTIFICATION_ENTRY: absolute(input.notificationEntry ?? `${root}/current/notification-ws/index.ts`, 'notificationEntry'), WORK_ORDERS_ENTRY: absolute(input.workOrdersEntry ?? `${root}/current/work-orders-api/index.mjs`, 'workOrdersEntry'),
    NGINX: absolute(input.nginx ?? '/usr/sbin/nginx', 'nginx') }
  const files = {}
  for (const [template, output] of [['nginx.conf.tpl', 'nginx.conf'], ['Caddyfile.tpl', 'Caddyfile'], ['services/backend.service.tpl', 'services/dante-backend.service'],
    ['services/notification.service.tpl', 'services/dante-notification.service'], ['services/work-orders.service.tpl', 'services/dante-work-orders.service'], ['services/gateway.service.tpl', 'services/dante-gateway.service']]) {
    files[output] = readFileSync(path.join(ownRoot, template), 'utf8').replace(/@@([A-Z_]+)@@/g, (_, key) => { if (!(key in values)) throw new Error(`Unknown template field ${key}`); return values[key] })
  }
  files['deployment.json'] = JSON.stringify({ schemaVersion: '1.0', publicOrigin: gateway.origin, instanceRoot: root, hostRoot, microRoot,
    trustedHtmlEval: input.trustedHtmlEval === true, applicationOrigins: externalOrigins,trustedScriptOrigins }, null, 2) + '\n'
  return files
}
export function writeConfiguration(input, destination) {
  absolute(destination, 'output directory')
  for (const [name, body] of Object.entries(render(input))) { const output = path.join(destination, name); mkdirSync(path.dirname(output), { recursive: true, mode: 0o700 }); writeFileSync(output, body, { mode: 0o600 }) }
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const args = process.argv.slice(2)
    if (args.length !== 4 || args[0] !== '--config' || args[2] !== '--out-dir') throw new Error('Usage: node deploy/native/render.mjs --config /absolute/deployment.json --out-dir /absolute/configuration')
    const input = JSON.parse(readFileSync(args[1], 'utf8'))
    writeConfiguration(input, args[3]); console.log(JSON.stringify({ ok: true, publicOrigin: input.publicOrigin, files: Object.keys(render(input)) }))
  } catch (error) { console.error(error.message); process.exitCode = 1 }
}
