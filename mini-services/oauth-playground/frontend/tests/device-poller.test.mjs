import test from 'node:test'
import assert from 'node:assert/strict'
import { createDevicePoller } from '../src/device-poller.js'

function clock() {
  let time = 0
  let nextId = 1
  const tasks = new Map()
  return {
    now: () => time,
    setTimer(fn, delay) { const id = nextId++; tasks.set(id, { at: time + delay, fn }); return id },
    clearTimer: id => tasks.delete(id),
    async advance(ms) {
      const end = time + ms
      for (;;) {
        const next = [...tasks.entries()].sort((a, b) => a[1].at - b[1].at)[0]
        if (!next || next[1].at > end) break
        time = next[1].at
        tasks.delete(next[0])
        await next[1].fn()
        await Promise.resolve()
      }
      time = end
    },
    count: () => tasks.size,
  }
}

test('pending waits for the stated interval; slow_down adds five seconds; success stops all timers', async () => {
  const timer = clock()
  const calls = []
  const events = []
  const replies = [
    { ok: false, data: { error: 'authorization_pending' } },
    { ok: false, data: { error: 'slow_down' } },
    { ok: true, data: { granted: true, flow: 'device' } },
  ]
  const poller = createDevicePoller({ ...timer, request: async () => { calls.push(timer.now()); return replies.shift() }, onEvent: e => events.push(e) })
  poller.start({ interval: 5, expires_in: 100 })
  await timer.advance(4999)
  assert.equal(calls.length, 0)
  await timer.advance(1)
  await timer.advance(5000)
  await timer.advance(9999)
  assert.deepEqual(calls, [5000, 10000])
  await timer.advance(1)
  assert.deepEqual(calls, [5000, 10000, 20000])
  assert.equal(events.at(-1).kind, 'granted')
  assert.equal(timer.count(), 0)
})

test('restarting cancels the old request and ignores its late success', async () => {
  const timer = clock()
  const events = []
  let complete
  let oldSignal
  const poller = createDevicePoller({ ...timer, request: signal => {
    oldSignal = signal
    return new Promise(resolve => { complete = resolve })
  }, onEvent: e => events.push(e) })
  poller.start({ interval: 1, expires_in: 30 })
  // Intentionally leave the request pending while the controllable timer moves.
  const advance = timer.advance(1000)
  await Promise.resolve()
  poller.start({ interval: 2, expires_in: 30 })
  assert.equal(oldSignal.aborted, true)
  complete({ ok: true, data: { granted: true } })
  await advance
  assert.equal(events.some(e => e.kind === 'granted'), false)
  poller.cancel()
  assert.equal(timer.count(), 0)
})

test('expiry aborts an in-flight request; cancellation cannot schedule another poll', async () => {
  const timer = clock()
  const events = []
  let signal
  let complete
  const poller = createDevicePoller({ ...timer, request: incoming => {
    signal = incoming
    return new Promise(resolve => { complete = resolve })
  }, onEvent: e => events.push(e) })
  poller.start({ interval: 1, expires_in: 2 })
  const advance = timer.advance(1000)
  await Promise.resolve()
  await timer.advance(1000)
  assert.equal(signal.aborted, true)
  assert.equal(events.at(-1).kind, 'expired')
  complete({ ok: false, data: { error: 'authorization_pending' } })
  await advance
  assert.equal(timer.count(), 0)
})

test('denial ends the flow and malformed timing never starts a request', async () => {
  const timer = clock()
  const events = []
  let calls = 0
  const poller = createDevicePoller({ ...timer, request: async () => { calls++; return { ok: false, data: { error: 'access_denied' } } }, onEvent: e => events.push(e) })
  poller.start({ interval: 1, expires_in: 20 })
  await timer.advance(1000)
  assert.equal(events.at(-1).kind, 'error')
  assert.equal(events.at(-1).error, 'access_denied')
  assert.equal(timer.count(), 0)
  poller.start({ interval: 0, expires_in: -1 })
  await timer.advance(2000)
  assert.equal(calls, 1)
  assert.equal(events.at(-1).error, 'invalid_device_response')
})
