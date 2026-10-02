/**
 * 持久通知发布辅助层。client 可为 createContextClient 或 iframe bridge。
 * 调用方为同一业务事件保留同一个 idempotencyKey；服务端执行权限与目标校验。
 */
export function createNotificationPublisher(client) {
  if (!client || typeof client.current !== 'function' || typeof client.requestJson !== 'function') {
    throw new TypeError('An active micro application context client is required')
  }
  return {
    async publish(body) {
      const current = client.current()
      if (!current?.appCode) throw new DOMException('Context expired', 'AbortError')
      if (typeof body?.idempotencyKey !== 'string' || !body.idempotencyKey.trim()) {
        throw new TypeError('A stable idempotencyKey is required')
      }
      return client.requestJson(`/api/apps/${encodeURIComponent(current.appCode)}/notifications`, {
        method: 'POST',
        credentials: 'omit',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      })
    },
  }
}
