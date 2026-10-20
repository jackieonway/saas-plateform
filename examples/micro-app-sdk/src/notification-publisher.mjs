import { SDK_VERSION, assertSupportedSdkVersion } from '../generated/protocol.mjs'
import { contextAbortError, protocolError } from './http.mjs'

/** Reuse one idempotency key for one event. The service independently validates recipients and grants. */
export function createNotificationPublisher(client) {
  if (!client || typeof client.current !== 'function' || typeof client.requestJson !== 'function') {
    throw new TypeError('An active micro application context client is required')
  }
  try { if (client.sdkVersion !== undefined) assertSupportedSdkVersion(client.sdkVersion) }
  catch { throw protocolError('UNSUPPORTED_SDK_VERSION') }
  return {
    sdkVersion: SDK_VERSION,
    async publish(body) {
      const current = client.current()
      if (!current?.appCode) throw contextAbortError()
      if (typeof body?.idempotencyKey !== 'string' || !body.idempotencyKey.trim()) {
        throw protocolError('INVALID_NOTIFICATION')
      }
      let payload
      try { payload = JSON.stringify(body) } catch { throw protocolError('INVALID_NOTIFICATION') }
      if (new TextEncoder().encode(payload).byteLength > 64 * 1024) throw protocolError('NOTIFICATION_TOO_LARGE')
      return client.requestJson(`/api/apps/${encodeURIComponent(current.appCode)}/notifications`, {
        method: 'POST', credentials: 'omit', headers: { 'Content-Type': 'application/json' }, body: payload,
      })
    },
  }
}
