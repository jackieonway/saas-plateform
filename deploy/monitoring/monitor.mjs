import { appendFile, readFile } from 'node:fs/promises'
import { pathToFileURL } from 'node:url'

const TYPES = new Set(['health', 'prometheus', 'asset'])
function integer(value, fallback, min, max) {
  const result = value ?? fallback
  if (!Number.isInteger(result) || result < min || result > max) throw new Error('Monitor integer configuration is out of bounds')
  return result
}
export function validateConfig(input, env = process.env) {
  if (env.NODE_TLS_REJECT_UNAUTHORIZED === '0') throw new Error('Monitor cannot disable TLS verification')
  if (!input || typeof input !== 'object' || !['production', 'development'].includes(input.environment) ||
      !Array.isArray(input.targets) || !input.targets.length || input.targets.length > 20 ||
      typeof input.eventLog !== 'string' || !input.eventLog.startsWith('/')) throw new Error('Monitor requires explicit environment, absolute eventLog and 1–20 targets')
  const names = new Set()
  const targets = input.targets.map((target) => {
    if (!target || !/^[a-z][a-z0-9-]{0,47}$/.test(target.name) || names.has(target.name) || !TYPES.has(target.kind)) throw new Error('Monitor target name/kind is invalid')
    names.add(target.name)
    let url
    try { url = new URL(target.url) } catch { throw new Error('Monitor target URL is invalid') }
    const loopback = ['127.0.0.1', '[::1]'].includes(url.hostname)
    const localHttp = url.protocol === 'http:' && loopback && (input.environment === 'development' || target.private === true)
    if ((url.protocol !== 'https:' && !localHttp) || url.username || url.password || url.search || url.hash ||
        (target.private === true && (!loopback || target.kind === 'asset'))) throw new Error('Monitor targets require exact HTTPS URLs; explicit private health/metrics targets permit literal loopback HTTP')
    let token = null
    if (target.kind === 'prometheus' || target.tokenEnv != null) {
      if (!/^[A-Z][A-Z0-9_]{0,63}$/.test(target.tokenEnv || '')) throw new Error('Protected monitor target requires tokenEnv')
      token = env[target.tokenEnv]
      if (typeof token !== 'string' || token.length < 32 || token.length > 256 || !/^[A-Za-z0-9_-]+$/.test(token)) throw new Error('Monitor target token must be configured securely in the process environment')
    }
    if (target.kind === 'asset' && (typeof target.contains !== 'string' || !target.contains || target.contains.length > 256)) throw new Error('Asset probe requires a bounded explicit version marker')
    return Object.freeze({ name: target.name, kind: target.kind, url: url.toString(), token, contains: target.contains })
  })
  return Object.freeze({ environment: input.environment, eventLog: input.eventLog, targets,
    intervalMs: integer(input.intervalMs, 5000, 1000, 300000), timeoutMs: integer(input.timeoutMs, 2000, 100, 10000),
    downThreshold: integer(input.downThreshold, 2, 1, 20), recoveryThreshold: integer(input.recoveryThreshold, 2, 1, 20),
    maxResponseBytes: integer(input.maxResponseBytes, 1048576, 1024, 4194304) })
}

async function limitedBody(response, limit, signal) {
  if (Number(response.headers.get('content-length')) > limit) { await response.body?.cancel(); throw new Error('BODY_LIMIT') }
  const reader = response.body?.getReader()
  if (!reader) throw new Error('CONTRACT')
  const parts = []; let bytes = 0
  try {
    for (;;) {
      signal.throwIfAborted()
      const { done, value } = await reader.read()
      if (done) break
      bytes += value.byteLength
      if (bytes > limit) { await reader.cancel(); throw new Error('BODY_LIMIT') }
      parts.push(value)
    }
    return new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(parts))
  } finally { reader.releaseLock() }
}

export async function probeTarget(target, config, { fetchImpl = fetch, signal: parentSignal } = {}) {
  const timeout = AbortSignal.timeout(config.timeoutMs)
  const signal = parentSignal ? AbortSignal.any([parentSignal, timeout]) : timeout
  try {
    const response = await fetchImpl(target.url, { method: 'GET', redirect: 'error', signal, credentials: 'omit',
      headers: { Accept: target.kind === 'health' ? 'application/json' : 'text/plain', ...(target.token ? { Authorization: 'Bearer ' + target.token } : {}) } })
    if (response.status !== 200) { await response.body?.cancel(); return { up: false, reason: 'STATUS' } }
    const body = await limitedBody(response, config.maxResponseBytes, signal)
    if (target.kind === 'health') {
      if (!/^application\/json(?:;|$)/i.test(response.headers.get('content-type') || '') || JSON.parse(body)?.status !== 'UP') return { up: false, reason: 'CONTRACT' }
    } else if (target.kind === 'prometheus') {
      if (!/^(?:text\/plain|application\/openmetrics-text)(?:;|$)/i.test(response.headers.get('content-type') || '') ||
          !/^[a-zA-Z_:][a-zA-Z0-9_:]*(?:\{[^\r\n]*\})?\s+[-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?(?:\s|$)/m.test(body)) return { up: false, reason: 'CONTRACT' }
    } else if (!body.includes(target.contains)) return { up: false, reason: 'VERSION' }
    return { up: true, reason: 'UP' }
  } catch (error) {
    const reason = timeout.aborted ? 'TIMEOUT' : ['BODY_LIMIT', 'CONTRACT'].includes(error.message) ? error.message : 'UNAVAILABLE'
    return { up: false, reason }
  }
}

export function createMonitor(config, { fetchImpl = fetch, emit = async (event) => {
  const line = JSON.stringify(event) + '\n'
  await appendFile(config.eventLog, line, { mode: 0o600 })
  process.stdout.write(line)
} } = {}) {
  const states = new Map(config.targets.map((target) => [target.name, { down: 0, up: 0, alerted: false }]))
  let running = false
  async function check(signal) {
    if (running) throw new Error('Monitor does not overlap checks')
    running = true
    try {
      const results = await Promise.all(config.targets.map(async (target) => ({ target, result: await probeTarget(target, config, { fetchImpl, signal }) })))
      if (signal?.aborted) return results.map(({ target, result }) => ({ target: target.name, ...result }))
      for (const { target, result } of results) {
        const state = states.get(target.name)
        if (result.up) { state.up++; state.down = 0 } else { state.down++; state.up = 0 }
        let event
        if (!state.alerted && state.down >= config.downThreshold) { state.alerted = true; event = 'DOWN' }
        else if (state.alerted && state.up >= config.recoveryThreshold) { state.alerted = false; event = 'RECOVERED' }
        if (event) await emit({ timestamp: new Date().toISOString(), event, target: target.name, reason: result.reason,
          consecutive: result.up ? state.up : state.down })
      }
      return results.map(({ target, result }) => ({ target: target.name, ...result }))
    } finally { running = false }
  }
  return { check }
}

export async function main(args = process.argv.slice(2), env = process.env) {
  if (args.some((arg) => arg !== '--once') || !env.MONITOR_CONFIG) throw new Error('Set MONITOR_CONFIG; only --once is supported')
  const raw = await readFile(env.MONITOR_CONFIG)
  if (raw.length > 65536) throw new Error('Monitor configuration exceeds 64 KiB')
  const config = validateConfig(JSON.parse(raw.toString('utf8')), env), monitor = createMonitor(config)
  const lifetime = new AbortController()
  const shutdown = () => lifetime.abort()
  process.once('SIGINT', shutdown); process.once('SIGTERM', shutdown)
  try {
    do {
      const results = await monitor.check(lifetime.signal)
      if (args.includes('--once')) { if (results.some((result) => !result.up)) process.exitCode = 1; break }
      await new Promise((resolve) => {
        const done = () => { clearTimeout(timer); lifetime.signal.removeEventListener('abort', done); resolve() }
        const timer = setTimeout(done, config.intervalMs)
        lifetime.signal.addEventListener('abort', done, { once: true })
        if (lifetime.signal.aborted) done()
      })
    } while (!lifetime.signal.aborted)
  } finally { process.removeListener('SIGINT', shutdown); process.removeListener('SIGTERM', shutdown) }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main().catch(() => {
  process.stderr.write(JSON.stringify({ event: 'MONITOR_FAILED', reason: 'invalid configuration or local probe/log failure' }) + '\n')
  process.exitCode = 1
})
