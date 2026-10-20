/** RFC 8628 polling with one in-flight request and cancellation across restarts. */
export function createDevicePoller({
  request,
  onEvent,
  now = () => Date.now(),
  setTimer = (fn, delay) => setTimeout(fn, delay),
  clearTimer = id => clearTimeout(id),
}) {
  let generation = 0
  let pollTimer
  let expiryTimer
  let controller

  function cancel() {
    generation++
    clearTimer(pollTimer)
    clearTimer(expiryTimer)
    controller?.abort()
    controller = undefined
  }

  function start({ interval, expires_in: expiresIn }) {
    cancel()
    const activeGeneration = generation
    if (!Number.isFinite(interval) || interval <= 0 || !Number.isFinite(expiresIn) || expiresIn <= 0) {
      onEvent({ kind: 'error', error: 'invalid_device_response' })
      return
    }
    let delay = interval * 1000
    const expiresAt = now() + expiresIn * 1000
    const metadata = () => ({ interval: delay / 1000, expiresAt })
    const finish = event => { cancel(); onEvent(event) }
    expiryTimer = setTimer(() => finish({ kind: 'expired' }), expiresIn * 1000)

    async function poll() {
      if (generation !== activeGeneration) return
      if (now() >= expiresAt) { finish({ kind: 'expired' }); return }
      controller = new AbortController()
      try {
        const reply = await request(controller.signal)
        if (generation !== activeGeneration) return
        controller = undefined
        if (reply.ok && reply.data?.granted === true) {
          finish({ kind: 'granted', data: reply.data })
          return
        }
        const error = reply.data?.error || 'invalid_device_response'
        if (error !== 'authorization_pending' && error !== 'slow_down') {
          finish({ kind: 'error', error })
          return
        }
        if (error === 'slow_down') delay += 5000
        onEvent({ kind: error === 'slow_down' ? 'slow_down' : 'pending', ...metadata() })
        pollTimer = setTimer(poll, delay)
      } catch (error) {
        if (generation === activeGeneration) finish({ kind: 'error', error: error.name === 'TimeoutError' ? 'request_timeout' : 'network_error' })
      }
    }

    onEvent({ kind: 'waiting', ...metadata() })
    pollTimer = setTimer(poll, delay)
  }

  return { start, cancel }
}
