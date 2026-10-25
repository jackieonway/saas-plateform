import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'
import { Script } from 'node:vm'
import ts from '../node_modules/typescript/lib/typescript.js'
import * as runtime from '../src/components/micro/runtime.ts'

const deferred = () => {
  let resolve, reject
  const promise = new Promise((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
const flush = async () => { for (let i = 0; i < 40; i++) await Promise.resolve() }
function clock() {
  let now = 0, next = 0
  const timers = new Map()
  return {
    timers,
    setTimer(callback, delay) { const id = ++next; timers.set(id, { at: now + delay, callback }); return id },
    clearTimer(id) { timers.delete(id) },
    async advance(ms) {
      const end = now + ms
      await flush()
      for (;;) {
        const timer = [...timers].filter(([, value]) => value.at <= end).sort((a, b) => a[1].at - b[1].at)[0]
        if (!timer) break
        timers.delete(timer[0]); now = timer[1].at; timer[1].callback(); await flush()
      }
      now = end; await flush()
    },
  }
}
function props(controller = new AbortController(), epoch = 1) {
  return { contractVersion: '2.0', sdkVersion: '3.0.0', contextVersion: 'context-' + epoch,
    contextEpoch: epoch, expiresAt: new Date(Date.now() + 60_000).toISOString(),
    tokenExpiresAt: new Date(Date.now() + 60_000).toISOString(), token: 'application-token',
    signal: controller.signal, appCode: 'probe', basePath: '/micro/probe', routePath: '/micro/probe',
    user: { id: 'user' }, tenant: { id: 'tenant' }, organization: { id: 'organization' },
    roles: [], permissions: [], menus: [], dataGrants: [], organizationScopeIds: ['organization'],
    theme: 'light', onNotify() {}, onNavigate() {} }
}
function fixture(options = {}) {
  const time = options.time || clock(), domain = options.domain || new runtime.TrustedMicroRuntimeDomain()
  const events = [], faults = [], authorization = new AbortController()
  const session = new runtime.MicroLifecycleSession(() => events.push('cleanup'), undefined, {
    domain, deadlinesMs: { load: 10, mount: 10, update: 10, unmount: 10 }, ...time,
    abort: () => authorization.abort(), onFault: fault => faults.push(fault), ...options,
  })
  return { time, domain, events, faults, authorization, session,
    lifecycle: { name: 'probe', mount: (_, value) => events.push(['mount', value]),
      update: value => events.push(['update', value]), unmount: () => events.push('unmount') } }
}
const rejected = promise => promise.then(() => null, error => error)
function fault(error, domain, stage, reason = 'TIMEOUT') {
  assert.ok(error instanceof runtime.MicroRuntimeFault)
  assert.equal(error.code, 'TRUSTED_RUNTIME_FAULT')
  assert.equal(error.stage, stage)
  assert.equal(error.reason, reason)
  assert.equal(domain.fault, error)
}

test('a never completing mount reaches its deadline and cancels authorization instead of hanging', async () => {
  const time = clock(), pending = deferred(), authorization = new AbortController()
  // The fallback permits this regression to exercise the old session: it must
  // fail for the missing deadline behavior, rather than for a missing export.
  const domain = runtime.TrustedMicroRuntimeDomain ? new runtime.TrustedMicroRuntimeDomain() : undefined
  let outcome = 'pending'
  const session = new runtime.MicroLifecycleSession(() => {}, undefined, {
    domain, ...time, deadlinesMs: { mount: 10 }, abort: () => authorization.abort(),
  })
  const mounted = session.mount(async () => ({ name: 'probe', mount: () => pending.promise, unmount() {} }), {}, props(authorization))
  mounted.then(() => { outcome = 'resolved' }, () => { outcome = 'rejected' })
  await flush(); await time.advance(10)
  assert.equal(outcome, 'rejected', 'A pending trusted mount must reject at its deadline.')
  assert.ok(authorization.signal.aborted)
  fault(await rejected(mounted), domain, 'mount')
  await rejected(session.stop())
})

test('each permanently pending stage poisons the domain, cancels captured signals and cleans once', async () => {
  for (const stage of ['load', 'mount', 'update', 'unmount']) {
    const s = fixture(), gate = deferred()
    const lifecycle = { ...s.lifecycle, ...(stage === 'load' ? {} : { [stage]: () => gate.promise }) }
    let task
    if (stage === 'load' || stage === 'mount') {
      task = s.session.mount(() => stage === 'load' ? gate.promise : Promise.resolve(lifecycle), {}, props(s.authorization))
    } else {
      await s.session.mount(async () => lifecycle, {}, props(s.authorization))
      task = stage === 'update' ? s.session.update(props(s.authorization, 2)) : s.session.stop()
    }
    const outcome = rejected(task)
    await s.time.advance(10)
    fault(await outcome, s.domain, stage)
    assert.ok(s.authorization.signal.aborted)
    assert.ok(s.session.signal.aborted)
    assert.equal(s.faults.length, 1)
    await rejected(s.session.stop())
    assert.equal(s.events.filter(value => value === 'cleanup').length, 1)
    assert.equal(s.time.timers.size, 0)
  }
})

test('a timed out load cannot late-mount or unmount after it finally supplies a lifecycle', async () => {
  const s = fixture(), gate = deferred(), mounted = rejected(s.session.mount(() => gate.promise, {}, props()))
  await s.time.advance(10); fault(await mounted, s.domain, 'load')
  gate.resolve(s.lifecycle); await flush()
  await rejected(s.session.stop())
  assert.deepEqual(s.events, ['cleanup'])
  assert.equal(s.faults.length, 1)
})

test('a trusted load thenable remains tracked after abort until its deadline or actual settlement', async () => {
  const time = clock(), scope = new AbortController(), gate = deferred()
  let assimilated = false
  const exported = { then(resolve, reject) { assimilated = true; gate.promise.then(resolve, reject) } }
  const s = fixture({ time, signal: scope.signal })
  try {
    let outcome = 'pending'
    const loading = s.session.mount(() => Promise.resolve(exported), {}, props())
    loading.then(() => { outcome = 'resolved' }, () => { outcome = 'rejected' })
    await flush(); assert.ok(assimilated, 'The real load promise must have begun assimilating the thenable.')
    scope.abort(); await flush()
    assert.equal(outcome, 'pending', 'Abort is not proof that already executing trusted code has settled.')
    await time.advance(10)
    fault(await rejected(loading), s.domain, 'load')
    gate.reject(new Error('late trusted failure')); await flush()
    await rejected(s.session.stop())
    assert.deepEqual(s.events, ['cleanup'])
  } finally {
    gate.resolve()
  }
})

test('queueing behind normal trusted disposal does not spend the next load deadline', async () => {
  const time = clock(), domain = new runtime.TrustedMicroRuntimeDomain(), gate = deferred()
  const first = fixture({ time, domain }), second = fixture({ time, domain })
  await first.session.mount(async () => ({ ...first.lifecycle, unmount: () => gate.promise }), {}, props())
  const stopping = first.session.stop()
  const mounting = second.session.mount(async () => second.lifecycle, {}, props())
  await time.advance(9)
  assert.equal(second.events.length, 0)
  assert.equal(domain.fault, null)
  gate.resolve(); await Promise.all([stopping, mounting])
  assert.equal(second.events[0][0], 'mount')
  await second.session.stop()
  assert.equal(domain.fault, null)
})

test('load and mount receive separate execution budgets after their operations actually begin', async () => {
  const s = fixture(), load = deferred(), mount = deferred()
  const mounted = rejected(s.session.mount(() => load.promise, {}, props()))
  await s.time.advance(8)
  load.resolve({ ...s.lifecycle, mount: () => mount.promise }); await flush()
  await s.time.advance(9)
  assert.equal(s.domain.fault, null)
  await s.time.advance(1)
  fault(await mounted, s.domain, 'mount')
  await rejected(s.session.stop())
})

test('queued and newly requested trusted sessions cannot bypass the page fault', async () => {
  const s = fixture(), second = fixture({ time: s.time, domain: s.domain }), gate = deferred()
  const mounting = rejected(s.session.mount(async () => ({ ...s.lifecycle, mount: () => gate.promise }), {}, props()))
  const queued = rejected(second.session.mount(async () => second.lifecycle, {}, props()))
  await s.time.advance(10)
  const firstFault = await mounting
  assert.equal(await queued, firstFault)
  const third = fixture({ time: s.time, domain: s.domain })
  assert.equal(await rejected(third.session.mount(async () => third.lifecycle, {}, props())), firstFault)
  assert.deepEqual(second.events.filter(Array.isArray), [])
  assert.deepEqual(third.events.filter(Array.isArray), [])
  gate.resolve(); await flush()
  assert.equal(s.domain.fault, firstFault)
  await Promise.all([s, second, third].map(value => rejected(value.session.stop())))
})

test('stop before a pending mount deadline still records the permanent fault and settles disposal', async () => {
  const s = fixture(), gate = deferred()
  const mounted = rejected(s.session.mount(async () => ({ ...s.lifecycle, mount: () => gate.promise }), {}, props()))
  await flush()
  const stopping = rejected(s.session.stop())
  assert.ok(s.session.signal.aborted)
  await s.time.advance(10)
  fault(await mounted, s.domain, 'mount')
  assert.equal(await stopping, s.domain.fault)
  assert.equal(s.events.includes('unmount'), false)
  assert.deepEqual(s.events, ['cleanup'])
})

test('external scope abort does not erase an unresolved update deadline after navigation', async () => {
  const scope = new AbortController(), s = fixture({ signal: scope.signal }), gate = deferred()
  await s.session.mount(async () => ({ ...s.lifecycle, update: () => gate.promise }), {}, props())
  const updating = rejected(s.session.update(props()))
  await flush(); scope.abort(); await s.time.advance(10)
  fault(await updating, s.domain, 'update')
  await rejected(s.session.stop())
  assert.equal(s.faults.length, 1)
  assert.equal(s.events.includes('unmount'), false)
})

test('late resolve and late reject never revive a poisoned session or create unhandled rejections', async () => {
  const unhandled = [], listener = error => unhandled.push(error)
  process.on('unhandledRejection', listener)
  try {
    for (const rejectLate of [false, true]) {
      for (const stage of ['load', 'mount', 'update', 'unmount']) {
        const s = fixture(), gate = deferred(), lifecycle = { ...s.lifecycle,
          ...(stage === 'load' ? {} : { [stage]: () => gate.promise }) }
        let work
        if (stage === 'load' || stage === 'mount') work = s.session.mount(() => stage === 'load' ? gate.promise : Promise.resolve(lifecycle), {}, props())
        else {
          await s.session.mount(async () => lifecycle, {}, props())
          work = stage === 'update' ? s.session.update(props()) : s.session.stop()
        }
        const outcome = rejected(work)
        await s.time.advance(10)
        const current = await outcome
        if (rejectLate) gate.reject(new Error('application-token-secret'))
        else gate.resolve(stage === 'load' ? lifecycle : undefined)
        await flush(); await rejected(s.session.stop())
        assert.equal(s.domain.fault, current)
        assert.equal(s.faults.length, 1)
        assert.equal(s.events.filter(value => value === 'cleanup').length, 1)
        assert.equal(s.events.includes('unmount'), false)
        assert.equal(current.message.includes('secret'), false)
      }
    }
    await new Promise(resolve => setImmediate(resolve))
    assert.deepEqual(unhandled, [])
  } finally { process.removeListener('unhandledRejection', listener) }
})

test('normal final unloading waits for a late mount that settles within its deadline', async () => {
  const s = fixture(), gate = deferred()
  const mounted = s.session.mount(async () => ({ ...s.lifecycle, mount: async (_, value) => {
    s.events.push(['mount', value]); await gate.promise; s.events.push('mount-end') } }), {}, props())
  await flush(); const stopping = s.session.stop()
  gate.resolve(); await Promise.all([mounted, stopping])
  assert.equal(s.domain.fault, null)
  assert.deepEqual(s.events.map(value => Array.isArray(value) ? value[0] : value), ['mount', 'mount-end', 'unmount', 'cleanup'])
  assert.equal(s.time.timers.size, 0)
})

test('trusted operation order preserves an old final unmount before the next application load', async () => {
  const time = clock(), domain = new runtime.TrustedMicroRuntimeDomain(), gate = deferred(), order = []
  const a = fixture({ time, domain }), b = fixture({ time, domain })
  await a.session.mount(async () => ({ name: 'a', mount() {}, update: () => gate.promise, unmount: () => order.push('a-unmount') }), {}, props())
  const update = a.session.update(props()); await flush()
  const stop = a.session.stop()
  const next = b.session.mount(async () => { order.push('b-load'); return b.lifecycle }, {}, props())
  gate.resolve(); await Promise.all([update, stop, next])
  assert.deepEqual(order, ['a-unmount', 'b-load'])
  await b.session.stop()
})

test('stop and captured authorization cancellation at microtask boundaries never begin a stale lifecycle callback', async () => {
  for (const operation of ['mount', 'update']) {
    for (const cancellation of ['stop', 'context']) {
      for (let offset = 0; offset <= 18; offset++) {
        const s = fixture(), authorization = new AbortController(), events = []
        let cancelled = false, stopTask
        const lifecycle = { name: 'probe', mount() { events.push(cancelled ? 'mount-after-cancel' : 'mount') },
          update() { events.push(cancelled ? 'update-after-cancel' : 'update') }, unmount() { events.push('unmount') } }
        if (operation === 'update') await s.session.mount(async () => lifecycle, {}, props())
        const work = rejected(operation === 'mount'
          ? s.session.mount(async () => { events.push(cancelled ? 'load-after-cancel' : 'load'); return lifecycle }, {}, props(authorization))
          : s.session.update(props(authorization)))
        let cancel = () => {
          cancelled = true
          authorization.abort()
          if (cancellation === 'stop') stopTask = s.session.stop()
        }
        for (let n = 0; n < offset; n++) { const next = cancel; cancel = () => queueMicrotask(next) }
        queueMicrotask(cancel)
        await work; await new Promise(resolve => setImmediate(resolve))
        await (stopTask ?? s.session.stop())
        assert.equal(events.includes(operation + '-after-cancel'), false,
          `${operation} must recheck ${cancellation} cancellation immediately before execution (offset ${offset}).`)
        if (cancellation === 'stop') assert.equal(events.includes('load-after-cancel'), false,
          `load must not begin after stop (offset ${offset}).`)
        assert.equal(s.domain.fault, null)
      }
    }
  }
})

test('normal stop remains idempotent and releases all timers and domain listeners', async () => {
  const s = fixture()
  await s.session.mount(async () => s.lifecycle, {}, props())
  const first = s.session.stop(), second = s.session.stop()
  assert.equal(first, second)
  await first; await s.session.stop()
  assert.equal(s.events.filter(value => value === 'unmount').length, 1)
  assert.equal(s.events.filter(value => value === 'cleanup').length, 1)
  assert.equal(s.time.timers.size, 0)
  assert.equal(s.domain.fault, null)
})

test('a settled mount error retains the bounded final-unmount path before domain reuse', async () => {
  const s = fixture()
  await assert.rejects(s.session.mount(async () => ({ ...s.lifecycle, mount() { throw new Error('known failure') } }), {}, props()))
  await s.session.stop()
  assert.deepEqual(s.events, ['unmount', 'cleanup'])
  assert.equal(s.domain.fault, null)
  const next = fixture({ time: s.time, domain: s.domain })
  await next.session.mount(async () => next.lifecycle, {}, props()); await next.session.stop()
})

test('an unmount rejection poisons the domain because successful reclamation is unproven', async () => {
  const s = fixture()
  await s.session.mount(async () => ({ ...s.lifecycle, unmount: async () => { throw new Error('secret') } }), {}, props())
  const error = await rejected(s.session.stop())
  fault(error, s.domain, 'unmount', 'OPERATION_FAILED')
  assert.equal(error.message.includes('secret'), false)
  assert.equal(s.events.filter(value => value === 'cleanup').length, 1)
})

test('a host cleanup exception conservatively poisons the domain without exposing its input', async () => {
  const time = clock(), domain = new runtime.TrustedMicroRuntimeDomain()
  const session = new runtime.MicroLifecycleSession(() => { throw new Error('platform-token-secret') }, undefined,
    { domain, ...time, deadlinesMs: { unmount: 10 } })
  await session.mount(async () => ({ name: 'probe', mount() {}, unmount() {} }), {}, props())
  const error = await rejected(session.stop())
  fault(error, domain, 'unmount', 'OPERATION_FAILED')
  assert.equal(error.message.includes('secret'), false)
})

test('aborted load that settles before its deadline permits cleanup and the next trusted application', async () => {
  const s = fixture(), gate = deferred()
  const mounting = rejected(s.session.mount(() => gate.promise, {}, props()))
  await flush(); const stopping = s.session.stop()
  gate.reject(new DOMException('Cancelled', 'AbortError'))
  assert.equal((await mounting).name, 'AbortError')
  await stopping
  assert.equal(s.domain.fault, null)
  assert.deepEqual(s.events, ['cleanup'])
  const next = fixture({ time: s.time, domain: s.domain })
  await next.session.mount(async () => next.lifecycle, {}, props()); await next.session.stop()
})

test('fault notification is stable even when listeners throw, unsubscribe or inspect a latched domain', () => {
  const domain = new runtime.TrustedMicroRuntimeDomain(), received = []
  domain.subscribe(() => { throw new Error('ignored listener error') })
  const unsubscribe = domain.subscribe(value => received.push(value)); unsubscribe()
  domain.subscribe(value => received.push(value))
  const first = domain.poison(new runtime.MicroRuntimeFault('mount', 'TIMEOUT'))
  const second = domain.poison(new runtime.MicroRuntimeFault('unmount', 'TIMEOUT'))
  assert.equal(first, second)
  assert.deepEqual(received, [first])
  assert.ok(domain.signal.aborted)
  assert.throws(() => domain.assertHealthy(), error => error === first)
})

test('same-page module replacement preserves the active trusted queue and permanent fault until a fresh realm', async () => {
  const first = await import('../src/components/micro/runtime.ts?phase7-page-before')
  const time = clock(), gate = deferred(), events = [], domain = first.trustedMicroRuntimeDomain
  const oldSession = new first.MicroLifecycleSession(() => events.push('old-cleanup'), undefined,
    { domain, ...time, deadlinesMs: { mount: 10 } })
  const oldMount = rejected(oldSession.mount(async () => ({ name: 'old', mount: () => gate.promise, unmount() { events.push('old-unmount') } }), {}, props()))
  await flush()
  const second = await import('../src/components/micro/runtime.ts?phase7-page-after')
  assert.equal(second.trustedMicroRuntimeDomain, domain,
    'Replacing the module must retain the healthy domain and its active operation queue.')
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, Symbol.for('saas.micro.trusted-runtime-domain.v1'))
  assert.equal(descriptor.value, domain)
  assert.equal(descriptor.enumerable, false)
  assert.equal(descriptor.writable, false)
  assert.equal(descriptor.configurable, false)
  const newSession = new second.MicroLifecycleSession(() => events.push('new-cleanup'), undefined,
    { domain: second.trustedMicroRuntimeDomain, ...time, deadlinesMs: { mount: 10 } })
  const nextMount = rejected(newSession.mount(async () => ({ name: 'new', mount: () => events.push('new-mount'), unmount() {} }), {}, props()))
  await flush()
  assert.equal(events.includes('new-mount'), false)
  await time.advance(10)
  const oldFault = await oldMount
  assert.equal(await nextMount, oldFault)
  assert.ok(second.isMicroRuntimeFault(oldFault), 'An error from the previous module identity must remain recognizable.')
  assert.equal(oldFault instanceof second.MicroRuntimeFault, false)
  const third = await import('../src/components/micro/runtime.ts?phase7-page-poisoned')
  assert.equal(third.trustedMicroRuntimeDomain, domain)
  assert.equal(third.trustedMicroRuntimeDomain.fault, oldFault)
  assert.ok(third.isMicroRuntimeFault(oldFault))
  gate.reject(new Error('late previous-module failure')); await flush()
  await Promise.all([rejected(oldSession.stop()), rejected(newSession.stop())])
  assert.equal(events.includes('new-mount'), false)
  assert.equal(events.includes('old-unmount'), false)
  const isolated = {}
  const compiled = ts.transpileModule(readFileSync(new URL('../src/components/micro/runtime.ts', import.meta.url), 'utf8'),
    { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText
  new Script(compiled).runInNewContext({ exports: isolated, AbortController, AbortSignal, DOMException,
    Error, Promise, Symbol, setTimeout, clearTimeout, require: () => ({ CONTEXT_CONTRACT_VERSION: '1.0' }) })
  assert.notEqual(isolated.trustedMicroRuntimeDomain, domain)
  assert.equal(isolated.trustedMicroRuntimeDomain.fault, null)
  assert.equal(isolated.trustedMicroRuntimeDomain.signal.aborted, false)
  const invalidSlot = {}, invalidRealm = { exports: {}, AbortController, AbortSignal, DOMException,
    Error, Promise, Symbol, setTimeout, clearTimeout, require: () => ({ CONTEXT_CONTRACT_VERSION: '1.0' }) }
  Object.defineProperty(invalidRealm, Symbol.for('saas.micro.trusted-runtime-domain.v1'), { value: invalidSlot })
  assert.throws(() => new Script(compiled).runInNewContext(invalidRealm),
    error => runtime.isMicroRuntimeFault(error) && error.reason === 'OPERATION_FAILED')
  assert.equal(invalidRealm[Symbol.for('saas.micro.trusted-runtime-domain.v1')], invalidSlot,
    'An invalid occupied page slot must fail closed rather than install a healthy replacement.')
})
