/** 浏览器微应用的展示/请求辅助层；业务后端仍必须独立校验权限与数据归属。 */
export function createContextClient(initialProps, fetchImpl = fetch) {
  let props, controller, unbind, generation = 0
  const usable = () => props && !controller.signal.aborted &&
    Date.parse(props.expiresAt) > Date.now() && Date.parse(props.tokenExpiresAt) > Date.now()
  const stop = () => {
    generation++
    controller?.abort()
    unbind?.()
    unbind = null
  }
  const update = (next) => {
    if (next?.contractVersion !== '2.0' || !next.token || !next.signal ||
        !next.user?.id || !next.tenant?.id || !next.organization?.id) throw new Error('MicroProps 2.0 required')
    if (!props || props.token !== next.token || props.contextEpoch !== next.contextEpoch ||
        props.contextVersion !== next.contextVersion || props.signal !== next.signal ||
        props.user.id !== next.user.id || props.tenant.id !== next.tenant.id || props.organization.id !== next.organization.id) {
      stop()
      const scope = new AbortController()
      controller = scope
      const abort = () => scope.abort()
      next.signal.addEventListener('abort', abort, { once: true })
      unbind = () => next.signal.removeEventListener('abort', abort)
      if (next.signal.aborted) abort()
    }
    props = next
  }
  update(initialProps)
  return {
    update,
    dispose: stop,
    current: () => usable() ? props : null,
    async requestJson(url, options = {}) {
      if (!usable()) throw new DOMException('Context expired', 'AbortError')
      const requestGeneration = generation
      const snapshot = props
      const lifetime = controller.signal
      const request = new AbortController()
      const abort = () => request.abort()
      lifetime.addEventListener('abort', abort, { once: true })
      options.signal?.addEventListener('abort', abort, { once: true })
      if (lifetime.aborted || options.signal?.aborted) abort()
      const check = () => {
        if (request.signal.aborted || requestGeneration !== generation || !usable()) {
          throw new DOMException('Context changed', 'AbortError')
        }
      }
      try {
        const headers = new Headers(options.headers)
        headers.set('Authorization', 'Bearer ' + snapshot.token)
        const response = await fetchImpl(url, { ...options, headers, signal: request.signal })
        check()
        const body = await response.json()
        check()
        if (!response.ok) throw new Error(body.message ?? 'Request failed: ' + response.status)
        return body
      } finally {
        lifetime.removeEventListener('abort', abort)
        options.signal?.removeEventListener('abort', abort)
      }
    },
  }
}
