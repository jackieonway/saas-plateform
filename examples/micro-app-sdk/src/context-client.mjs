import { SDK_VERSION, MICRO_PROPS_CONTRACT_VERSION, assertSupportedSdkVersion } from '../generated/protocol.mjs'
import { contextAbortError, protocolError, requestWithContext } from './http.mjs'
import { assertBrowserCapabilities } from './capabilities.mjs'
import { authorizationContextSignature } from './context-updates.mjs'

/** Browser display/request helper. Business services still authenticate and authorize every request. */
export function createContextClient(initialProps, fetchImpl = globalThis.fetch) {
  assertBrowserCapabilities('client')
  let props, controller, unbind, hostSignal, authorizationKey, generation = 0
  const usable = () => props && !controller.signal.aborted &&
    Date.parse(props.expiresAt) > Date.now() && Date.parse(props.tokenExpiresAt) > Date.now()
  const stop = () => { generation++; controller?.abort(); unbind?.(); unbind = null }
  const update = (next) => {
    try { if (next?.sdkVersion !== undefined) assertSupportedSdkVersion(next.sdkVersion) }
    catch { stop(); throw protocolError('UNSUPPORTED_SDK_VERSION') }
    if (next?.contractVersion !== MICRO_PROPS_CONTRACT_VERSION || typeof next.token !== 'string' || !next.token ||
        !next.signal || typeof next.signal.addEventListener !== 'function' ||
        !Number.isSafeInteger(next.contextEpoch) || next.contextEpoch <= 0 || !next.contextVersion ||
        !next.user?.id || !next.tenant?.id || !next.organization?.id) {
      stop(); throw protocolError('UNSUPPORTED_MICRO_PROPS')
    }
    const nextKey = authorizationContextSignature(next)
    if (!usable() || nextKey === null || nextKey !== authorizationKey || hostSignal !== next.signal) {
      stop()
      const scope = new AbortController()
      controller = scope
      const abort = () => scope.abort()
      hostSignal = next.signal
      const source = hostSignal
      source.addEventListener('abort', abort, { once: true })
      unbind = () => source.removeEventListener('abort', abort)
      if (source.aborted) abort()
    }
    authorizationKey = nextKey
    props = next
  }
  update(initialProps)
  return {
    sdkVersion: SDK_VERSION,
    update,
    dispose: stop,
    current: () => usable() ? props : null,
    async requestJson(url, options = {}) {
      if (!usable()) throw contextAbortError()
      const requestGeneration = generation, snapshot = props, lifetime = controller.signal
      const check = () => {
        if (requestGeneration !== generation || lifetime.aborted || !usable()) throw contextAbortError()
      }
      // Combine the SDK-owned lifetime with the caller signal; props.signal alone may not change during token renewal.
      const scope = new AbortController(), abort = () => scope.abort()
      lifetime.addEventListener('abort', abort, { once: true })
      options.signal?.addEventListener('abort', abort, { once: true })
      if (lifetime.aborted || options.signal?.aborted) abort()
      try { return await requestWithContext({ props: snapshot, url, options: { ...options, signal: scope.signal }, fetchImpl, check }) }
      finally { lifetime.removeEventListener('abort', abort); options.signal?.removeEventListener('abort', abort) }
    },
  }
}
