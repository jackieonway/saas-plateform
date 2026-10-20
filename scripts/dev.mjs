#!/usr/bin/env node
import { spawn, spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { request } from 'node:http'
import { createServer, isIP } from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { setTimeout as delay } from 'node:timers/promises'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const guard = path.join(root, 'scripts/dev-child.mjs')
const frontendDirectories = { host: 'frontend', vue: 'mini-services/micro-vue', jquery: 'mini-services/micro-jquery', websocket: 'examples/websocket', 'work-orders': 'business-apps/work-orders/frontend', stores: 'business-apps/stores/frontend' }
const defaults = { host: 3000, vue: 3005, jquery: 3006, websocket: 3007, 'work-orders': 3008, stores: 3009, backend: 3001, notifications: 3003, oauth: 3010, 'oauth-api': 3011 }
const help = `Usage: node scripts/dev.mjs [options]

  --apps host,vue,jquery   Static frontends to start (default: host,vue,jquery).
                          Add websocket, work-orders or stores explicitly; use none for services only.
  --host ADDRESS          Bind IP or localhost (default: 127.0.0.1).
  --host-port PORT        Host Vite port (default: 3000).
  --vue-port PORT         Vue Vite port (default: 3005).
  --jquery-port PORT      jQuery Vite port (default: 3006).
  --websocket-port PORT   WebSocket frontend Vite port (default: 3007).
  --work-orders-port PORT Work-orders Vite port (default: 3008).
  --stores-port PORT      Stores Vite port (default: 3009).
  --timeout-ms NUMBER     Total startup deadline (default: 30000; max: 600000).
  --with-backend          Also run Rust platform; may migrate configured database.
  --backend-port PORT     Explicit platform bind port (default: 3001).
  --with-notifications    Also run the Rust notification service (default: 3003).
  --notifications-port PORT  Notification bind port.
  --with-oauth            Run the independent Rust OAuth API and Vite page.
  --oauth-port PORT       OAuth Vite port (default: 3010).
  --oauth-api-port PORT   OAuth Rust API port (default: 3011).
  --help                 Show this help.

Install each selected frontend with npm --prefix <directory> ci beforehand.
This command never installs dependencies or starts PostgreSQL. Services remain
in the foreground; Ctrl+C stops only the process trees started by this command.
The notification service requires explicit PostgreSQL/platform configuration. OAuth requires
explicit private Rust configuration; see mini-services/oauth-playground/README.md.
Work orders also requires explicit WORK_ORDERS_ALLOWED_HOST_ORIGINS. Its Rust API
is started separately with npm run dev:work-orders:backend and private config.
Stores requires explicit STORES_ALLOWED_HOST_ORIGINS; start its independent Rust
API separately with npm run dev:stores:backend and private configuration.
`

function integer(value, option, maximum) {
  if (!/^\d+$/.test(value || '') || Number(value) < 1 || Number(value) > maximum) {
    throw new Error(`${option} must be an integer from 1 to ${maximum}`)
  }
  return Number(value)
}

export function parseArgs(args) {
  const options = { apps: ['host', 'vue', 'jquery'], host: '127.0.0.1', ports: { ...defaults }, timeoutMs: 30000 }
  for (let n = 0; n < args.length; n++) {
    const option = args[n]
    if (option === '--help') { options.help = true; continue }
    if (['--with-backend', '--with-notifications', '--with-oauth'].includes(option)) {
      options[option.slice(7)] = true
      continue
    }
    if (option === '--apps') {
      const value = args[++n]
      if (!value) throw new Error('--apps requires a comma-separated list')
      options.apps = value === 'none' ? [] : value.split(',')
      if (new Set(options.apps).size !== options.apps.length || options.apps.some((app) => !Object.hasOwn(frontendDirectories, app))) {
        throw new Error('--apps accepts each of host,vue,jquery,websocket,work-orders,stores once, or none')
      }
    } else if (option === '--host') {
      options.host = args[++n]
      if (!isIP(options.host || '') && options.host !== 'localhost') throw new Error('--host must be an IP address or localhost')
    } else if (option === '--timeout-ms') options.timeoutMs = integer(args[++n], option, 600000)
    else if (/^--(host|vue|jquery|websocket|work-orders|stores|backend|notifications|oauth|oauth-api)-port$/.test(option)) {
      options.ports[option.slice(2, -5)] = integer(args[++n], option, 65535)
    } else throw new Error('Unknown option; use --help for supported arguments')
  }
  if (options.help) return options
  const selected = [...options.apps, ...['backend', 'notifications'].filter((name) => options[name]), ...(options.oauth ? ['oauth', 'oauth-api'] : [])]
  if (!selected.length) throw new Error('Select at least one frontend or an explicit --with-* service')
  if (new Set(selected.map((name) => options.ports[name])).size !== selected.length) {
    throw new Error('Selected services must use distinct ports')
  }
  return options
}

function address(host) { return isIP(host) === 6 ? `[${host}]` : host }
function connectHost(host) { return host === '0.0.0.0' ? '127.0.0.1' : host === '::' ? '::1' : host }
const origin = (host, port) => `http://${address(connectHost(host))}:${port}`

export function createServicePlan(options, directory = root) {
  const services = options.apps.map((name) => {
    const cwd = path.join(directory, frontendDirectories[name])
    const vite = path.join(cwd, 'node_modules/vite/bin/vite.js')
    if (!existsSync(path.join(cwd, 'package-lock.json')) || !existsSync(vite)) {
      throw new Error(`${name}: locked Vite dependencies are missing; run npm --prefix ${frontendDirectories[name]} ci`)
    }
    return {
      name, command: process.execPath, args: [vite, '--host', options.host, '--port', String(options.ports[name]), '--strictPort'],
      cwd, host: options.host, port: options.ports[name], readyPath: '/',
      env: name === 'host' ? {
        MICRO_VUE_DEV_PROXY_TARGET: origin(options.host, options.ports.vue),
        MICRO_JQUERY_DEV_PROXY_TARGET: origin(options.host, options.ports.jquery),
        ...(options.backend ? { BACKEND_PROXY_TARGET: origin(options.host, options.ports.backend) } : {}),
        ...(options.notifications ? { NOTIFICATION_DEV_PROXY_TARGET: origin(options.host, options.ports.notifications) } : {}),
      } : {},
    }
  })
  if (options.backend) services.push({
    name: 'backend', command: 'cargo', args: ['run', '--locked'], cwd: path.join(directory, 'backend'),
    host: options.host, port: options.ports.backend, readyPath: null,
    env: { BIND_ADDR: `${address(options.host === 'localhost' ? '127.0.0.1' : options.host)}:${options.ports.backend}` },
  })
  if (options.notifications) {
    if (!existsSync(path.join(directory, 'mini-services/notification-ws/Cargo.lock'))) {
      throw new Error('notifications: locked Rust service sources are missing')
    }
    services.push({
      name: 'notifications', command: 'cargo', args: ['run', '--locked', '--manifest-path', 'mini-services/notification-ws/Cargo.toml', '--target-dir', 'target/dev-notification'], cwd: directory,
      host: options.host, port: options.ports.notifications, readyPath: '/health/ready',
      env: { NOTIFICATION_HOST: options.host, NOTIFICATION_PORT: String(options.ports.notifications) },
    })
  }
  if (options.oauth) {
    const cwd = path.join(directory, 'mini-services/oauth-playground/frontend')
    const vite = path.join(cwd, 'node_modules/vite/bin/vite.js')
    if (!existsSync(path.join(cwd, 'package-lock.json')) || !existsSync(vite)) {
      throw new Error('oauth: locked Vite dependencies are missing; run npm --prefix mini-services/oauth-playground/frontend ci')
    }
    services.push({
      name: 'oauth-api', command: 'cargo', args: ['run', '--locked'], cwd: path.join(directory, 'mini-services/oauth-playground/backend'),
      host: options.host, port: options.ports['oauth-api'], readyPath: '/health',
      env: {
        OAUTH_BIND_ADDR: `${address(options.host === 'localhost' ? '127.0.0.1' : options.host)}:${options.ports['oauth-api']}`,
        OAUTH_PUBLIC_ORIGIN: origin(options.host, options.ports.oauth),
      },
    }, {
      name: 'oauth', command: process.execPath,
      args: [vite, '--host', options.host, '--port', String(options.ports.oauth), '--strictPort'],
      cwd, host: options.host, port: options.ports.oauth, readyPath: '/',
      env: { OAUTH_PLAYGROUND_API_TARGET: origin(options.host, options.ports['oauth-api']) },
    })
  }
  return services
}

export function redactLog(value, env = process.env) {
  let text = String(value)
  for (const [key, secret] of Object.entries(env)) {
    if (/(?:secret|token|password|passwd|database_url|session)/i.test(key) && secret && secret.length >= 4) {
      text = text.replaceAll(secret, '[redacted]')
    }
  }
  return text
    .replace(/([a-z][a-z0-9+.-]*:\/\/)[^/\s?#]*@/gi, '$1[redacted]@')
    .replace(/([?&](?:access_token|refresh_token|token|client_secret|password|session|api_key)=)[^&\s]*/gi, '$1[redacted]')
}

function available(service) {
  return new Promise((resolve, reject) => {
    const server = createServer()
    server.once('error', (error) => reject(new Error(`${service.name}: ${error.code === 'EADDRINUSE' ? 'port already in use' : 'cannot bind port'} ${service.port}; no existing process was stopped`)))
    server.listen({ host: service.host, port: service.port, exclusive: true }, () => server.close(resolve))
  })
}

function probe(service, timeoutMs) {
  if (service.readyPath === null) {
    return import('node:net').then(({ createConnection }) => new Promise((resolve) => {
      const socket = createConnection({ host: connectHost(service.host), port: service.port })
      const finish = (result) => { socket.destroy(); resolve(result) }
      socket.once('connect', () => finish(true)).once('error', () => finish(false))
      socket.setTimeout(timeoutMs, () => finish(false))
    }))
  }
  return new Promise((resolve) => {
    let finished = false
    const finish = (result) => {
      if (finished) return
      finished = true
      clearTimeout(deadline)
      attempt.destroy()
      resolve(result)
    }
    const attempt = request({ host: connectHost(service.host), port: service.port, path: service.readyPath, method: 'GET' }, (response) => {
      response.resume()
      finish(response.statusCode >= 200 && response.statusCode < 300)
    })
    // Socket idle timeouts reset on every byte. An absolute timer also bounds
    // peers that continually drip an incomplete response header.
    const deadline = setTimeout(() => finish(false), timeoutMs)
    attempt.once('error', () => finish(false))
    attempt.setTimeout(timeoutMs, () => finish(false))
    attempt.end()
  })
}

// The guard stays alive after a service failure. Windows taskkill can therefore
// still find the owned root PID and its descendants instead of losing the tree.
function stopTree(pid) {
  if (!pid) return
  if (process.platform === 'win32') {
    spawnSync('taskkill.exe', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore', windowsHide: true, timeout: 5000 })
  } else {
    try { process.kill(-pid, 'SIGTERM') } catch { /* Already exited. */ }
  }
}

async function waitForGroupExit(pid, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  while (true) {
    try { process.kill(-pid, 0) } catch (error) {
      if (error.code === 'ESRCH') return true
      throw error
    }
    if (Date.now() >= deadline) return false
    await delay(Math.min(20, deadline - Date.now()))
  }
}

function pipeLog(stream, name, env, log) {
  stream.setEncoding('utf8')
  let pending = ''
  let discarding = false
  stream.on('data', (chunk) => {
    if (discarding) {
      const newline = chunk.indexOf('\n')
      if (newline === -1) return
      chunk = chunk.slice(newline + 1)
      discarding = false
    }
    pending += chunk
    const lines = pending.split(/\r?\n/)
    pending = lines.pop()
    for (const line of lines) log(`[${name}] ${redactLog(line, env)}`)
    // Bound a misbehaving child's partial-line buffer without leaking a secret
    // split across chunks: oversized unterminated output is discarded.
    if (pending.length > 65536) {
      log(`[${name}] [oversized log line omitted]`)
      pending = ''
      discarding = true
    }
  })
  stream.on('end', () => { if (pending) log(`[${name}] ${redactLog(pending, env)}`) })
}

export async function supervise(services, { timeoutMs = 30000, signal, log = console.log, onReady = () => {} } = {}) {
  const children = []
  let failure
  let stopping = false
  let resolveFailure
  const failed = new Promise((resolve) => { resolveFailure = resolve })
  const fail = (message) => {
    if (!stopping && !failure) { failure = message; resolveFailure() }
  }
  let resolveAbort
  const aborted = new Promise((resolve) => { resolveAbort = resolve })
  const abort = () => resolveAbort()
  signal?.addEventListener('abort', abort, { once: true })
  if (signal?.aborted) abort()
  const cleanupSync = () => {
    for (const child of children) if (child.exitCode === null && child.signalCode === null) stopTree(child.pid)
  }
  process.once('exit', cleanupSync)
  const started = Date.now()
  try {
    for (const service of services) await available(service)
    for (const service of services) {
      if (signal?.aborted || failure) break
      const env = { ...process.env, ...service.env, DEV_SERVICE_HOST: service.host, DEV_SERVICE_PORT: String(service.port) }
      if (service.name === 'notifications') {
        delete env.JWT_SECRET
        if (process.env.NOTIFICATION_DATABASE_URL) env.DATABASE_URL = process.env.NOTIFICATION_DATABASE_URL
        if (process.env.NOTIFICATION_METRICS_TOKEN) env.METRICS_TOKEN = process.env.NOTIFICATION_METRICS_TOKEN
      }
      log(`[dev] Starting ${service.name} on port ${service.port}`)
      const child = spawn(process.execPath, [guard], {
        cwd: root, env, detached: process.platform !== 'win32', windowsHide: true,
        stdio: ['ignore', 'pipe', 'pipe', 'ipc'],
      })
      children.push(child)
      pipeLog(child.stdout, service.name, env, log)
      pipeLog(child.stderr, service.name, env, log)
      child.on('error', () => fail(`${service.name}: process guard could not start`))
      child.on('exit', () => fail(`${service.name}: process guard exited`))
      child.on('message', (message) => {
        if (message?.type === 'exit') fail(`${service.name}: service exited (code ${Number.isInteger(message.code) ? message.code : 'signal'})`)
        if (message?.type === 'error') fail(`${service.name}: command could not start; verify its runtime is installed`)
      })
      child.send({ command: service.command, args: service.args, cwd: service.cwd }, (error) => {
        if (error) fail(`${service.name}: process guard could not receive its configuration`)
      })
      while (!signal?.aborted && !failure) {
        const remaining = timeoutMs - (Date.now() - started)
        if (remaining <= 0) { fail(`${service.name}: readiness deadline exceeded (${timeoutMs} ms)`); break }
        const ready = await Promise.race([probe(service, Math.min(500, remaining)), failed, aborted])
        if (ready && !failure && !signal?.aborted) {
          log(`[dev] ${service.name} ready at ${origin(service.host, service.port)}`)
          break
        }
        await Promise.race([delay(Math.min(100, remaining)), failed, aborted])
      }
    }
    if (!failure && !signal?.aborted) {
      log('[dev] All selected services are ready; press Ctrl+C to stop.')
      onReady(children.map((child) => child.pid))
      await Promise.race([failed, aborted])
    }
  } catch (error) { fail(error.message) }
  finally {
    stopping = true
    signal?.removeEventListener('abort', abort)
    for (const child of children) if (child.connected) child.send({ type: 'stop' }, () => {})
    // Guard exit can precede its descendants' kernel cleanup after a group
    // signal. Wait for the owned group itself before reporting shutdown, so
    // callers can immediately reuse listeners without racing the descendants.
    await Promise.all(children.map(async (child) => {
      const exited = child.exitCode !== null || child.signalCode !== null
        ? Promise.resolve()
        : new Promise((resolve) => child.once('exit', resolve))
      const ended = await Promise.race([exited.then(() => true), delay(1500).then(() => false)])
      if (!ended) {
        stopTree(child.pid)
        if (process.platform !== 'win32') {
          await Promise.race([exited, delay(300)])
          try { process.kill(-child.pid, 'SIGKILL') } catch { /* Already exited. */ }
        }
      }
      if (process.platform !== 'win32' && child.pid && !await waitForGroupExit(child.pid, 1500)) {
        // The leader may already have exited while an owned descendant
        // ignored TERM. Never use a port lookup to choose a process to kill.
        try { process.kill(-child.pid, 'SIGKILL') } catch { /* Already exited. */ }
        if (!await waitForGroupExit(child.pid, 1500)) failure ||= 'owned process group did not stop before the cleanup deadline'
      }
    }))
    process.removeListener('exit', cleanupSync)
  }
  if (failure) log(`[dev] ${failure}`)
  log('[dev] Stopped owned services.')
  return failure ? 1 : 0
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) { console.log(help); return 0 }
  const services = createServicePlan(options)
  if (options.backend) console.log('[dev] Explicit backend startup can run SQLx migrations against the configured database.')
  if (options.notifications) console.log('[dev] Rust notification startup uses the explicitly configured platform and read-only database identity.')
  if (options.oauth) console.log('[dev] OAuth requires private OAUTH_* configuration; its Rust API can contact the configured platform.')
  const controller = new AbortController()
  const abort = () => controller.abort()
  for (const name of ['SIGINT', 'SIGTERM', 'SIGHUP']) process.on(name, abort)
  process.once('disconnect', abort)
  try { return await supervise(services, { timeoutMs: options.timeoutMs, signal: controller.signal }) }
  finally {
    for (const name of ['SIGINT', 'SIGTERM', 'SIGHUP']) process.removeListener(name, abort)
    process.removeListener('disconnect', abort)
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().then((code) => { process.exitCode = code }).catch((error) => {
    console.error(`[dev] ${redactLog(error.message)}`)
    process.exitCode = 1
  })
}
