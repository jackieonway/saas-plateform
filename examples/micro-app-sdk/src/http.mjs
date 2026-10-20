import { MICRO_PROPS_CONTRACT_VERSION, DEFAULT_MAX_JSON_RESPONSE_BYTES, MAX_HTTP_RESPONSE_BYTES,
  assertSupportedSdkVersion } from '../generated/protocol.mjs'

/** Bounded HTTP transport. Error objects deliberately retain no URL, body, token or cause. */
export const DEFAULT_MAX_RESPONSE_BYTES = DEFAULT_MAX_JSON_RESPONSE_BYTES
export const MAX_RESPONSE_BYTES = MAX_HTTP_RESPONSE_BYTES

const messages = Object.freeze({
  UNAUTHORIZED: 'Application authentication is no longer valid',
  FORBIDDEN: 'Application operation is forbidden',
  CONFLICT: 'Application operation conflicts with current state',
  NETWORK: 'Application service is unavailable',
  PROTOCOL: 'Application response does not match the supported protocol',
  HTTP: 'Application request failed',
})
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
export class MicroSdkError extends Error {
  constructor(kind, { status = 0, requestId, code } = {}) {
    super(messages[kind] || messages.PROTOCOL)
    this.name = 'MicroSdkError'
    this.kind = Object.hasOwn(messages, kind) ? kind : 'PROTOCOL'
    this.status = Number.isInteger(status) && status >= 0 && status <= 599 ? status : 0
    this.code = typeof code === 'string' && /^[A-Z][A-Z0-9_]{0,63}$/.test(code) ? code : this.kind
    if (typeof requestId === 'string' && uuid.test(requestId)) this.requestId = requestId
  }
  toJSON() {
    return { kind: this.kind, status: this.status, code: this.code, message: this.message,
      ...(this.requestId === undefined ? {} : { requestId: this.requestId }) }
  }
}
export const contextAbortError = () => new DOMException('Application context cancelled or expired', 'AbortError')
export function protocolError(code = 'INVALID_PROTOCOL') { return new MicroSdkError('PROTOCOL', { code }) }
function statusKind(status) {
  return status === 401 ? 'UNAUTHORIZED' : status === 403 ? 'FORBIDDEN' : status === 409 ? 'CONFLICT' : 'HTTP'
}

function abortable(promise, signal) {
  return new Promise((resolve, reject) => {
    const abort = () => { cleanup(); reject(contextAbortError()) }
    const cleanup = () => signal.removeEventListener('abort', abort)
    signal.addEventListener('abort', abort, { once: true })
    Promise.resolve(promise).then(value => { cleanup(); resolve(value) }, error => { cleanup(); reject(error) })
    if (signal.aborted) abort()
  })
}

async function readLimited(response, check, maximum, preserveBom, requestId, signal) {
  const fail = () => new MicroSdkError('PROTOCOL', { status: 413, requestId, code: 'RESPONSE_TOO_LARGE' })
  const declared = response.headers?.get('Content-Length')
  if (declared !== null && declared !== undefined && /^\d+$/.test(declared) && Number(declared) > maximum) {
    response.body?.cancel?.().catch(() => {})
    check()
    throw fail()
  }
  if (response.body === null) return ''
  // Supported browsers expose streaming bodies. Falling back to text()/json() would allocate an unbounded response first.
  if (typeof response.body?.getReader !== 'function') throw new MicroSdkError('PROTOCOL', { requestId, code: 'INVALID_RESPONSE' })
  const reader = response.body.getReader()
  const decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: preserveBom })
  const parts = []
  let bytes = 0, completed = false
  try {
    for (;;) {
      const next = await abortable(reader.read(), signal)
      check()
      if (next.done) { completed = true; break }
      if (!(next.value instanceof Uint8Array)) throw new MicroSdkError('PROTOCOL', { requestId, code: 'INVALID_RESPONSE' })
      bytes += next.value.byteLength
      if (bytes > maximum) { reader.cancel().catch(() => {}); check(); throw fail() }
      try { parts.push(decoder.decode(next.value, { stream: true })) }
      catch { throw new MicroSdkError('PROTOCOL', { requestId, code: 'INVALID_UTF8' }) }
    }
    try { parts.push(decoder.decode()) }
    catch { throw new MicroSdkError('PROTOCOL', { requestId, code: 'INVALID_UTF8' }) }
    return parts.join('')
  } finally {
    if (!completed) reader.cancel().catch(() => {})
    reader.releaseLock()
  }
}

/** The caller supplies the captured host context and a freshness check; no session/storage access occurs. */
export async function requestWithContext({ props, url, options = {}, fetchImpl = globalThis.fetch,
  check = () => {}, responseType = 'json', maxBytes = DEFAULT_MAX_RESPONSE_BYTES }) {
  if (!Number.isSafeInteger(maxBytes) || maxBytes <= 0 || maxBytes > MAX_RESPONSE_BYTES ||
      !['json', 'text'].includes(responseType)) throw protocolError('INVALID_RESPONSE_LIMIT')
  try { if (props?.sdkVersion !== undefined) assertSupportedSdkVersion(props.sdkVersion) }
  catch { throw protocolError('UNSUPPORTED_SDK_VERSION') }
  if (props?.contractVersion !== MICRO_PROPS_CONTRACT_VERSION || typeof props.token !== 'string' || !props.token ||
      !props.signal || typeof props.signal.addEventListener !== 'function') throw protocolError('UNSUPPORTED_MICRO_PROPS')
  const request = new AbortController()
  const abort = () => request.abort()
  const signals = [...new Set([props.signal, options.signal].filter(Boolean))]
  signals.forEach(signal => signal.addEventListener('abort', abort, { once: true }))
  if (signals.some(signal => signal.aborted)) abort()
  const verify = () => {
    check()
    if (request.signal.aborted || !(Date.parse(props.expiresAt) > Date.now()) || !(Date.parse(props.tokenExpiresAt) > Date.now())) throw contextAbortError()
  }
  let requestId
  try {
    verify()
    const headers = new Headers(options.headers)
    const supplied = headers.get('X-Request-Id')
    requestId = supplied && uuid.test(supplied) ? supplied : globalThis.crypto.randomUUID()
    headers.set('X-Request-Id', requestId)
    headers.set('Authorization', 'Bearer ' + props.token)
    let response
    try {
      // Calling an injected native fetch through a local binding preserves its global receiver semantics.
      const fetchRequest = fetchImpl
      response = await abortable(fetchRequest(url, { ...options, headers, signal: request.signal, credentials: 'omit' }), request.signal)
    } catch (error) {
      verify()
      if (error?.name === 'AbortError') throw contextAbortError()
      throw new MicroSdkError('NETWORK', { requestId })
    }
    verify()
    if (!response || !Number.isInteger(response.status) || response.status < 100 || response.status > 599 ||
        typeof response.ok !== 'boolean' || !response.headers || typeof response.headers.get !== 'function') {
      throw new MicroSdkError('PROTOCOL', { requestId, code: 'INVALID_RESPONSE' })
    }
    const echoed = response.headers.get('X-Request-Id')
    if (echoed && uuid.test(echoed)) requestId = echoed
    if (!response.ok) {
      // Remote error text can contain credentials; never parse, retain or display it.
      response.body?.cancel?.().catch(() => {})
      verify()
      throw new MicroSdkError(statusKind(response.status), { status: response.status, requestId })
    }
    let text
    try { text = await readLimited(response, verify, maxBytes, responseType === 'text', requestId, request.signal) }
    catch (error) {
      verify()
      if (error instanceof MicroSdkError) throw error
      if (error?.name === 'AbortError') throw contextAbortError()
      throw new MicroSdkError('NETWORK', { requestId })
    }
    verify()
    if (responseType === 'text') return text
    try { return JSON.parse(text) }
    catch { throw new MicroSdkError('PROTOCOL', { status: response.status, requestId, code: 'INVALID_JSON' }) }
  } catch (error) {
    if (error instanceof MicroSdkError) throw error
    if (request.signal.aborted || error?.name === 'AbortError') throw contextAbortError()
    throw new MicroSdkError('PROTOCOL', { requestId, code: 'INVALID_REQUEST' })
  } finally { signals.forEach(signal => signal.removeEventListener('abort', abort)) }
}
