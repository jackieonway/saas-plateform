// Exercise actual lifecycle and request handlers; fetch deliberately ignores AbortSignal.
const fs = require('node:fs')
const path = require('node:path')
const vm = require('node:vm')
const assert = require('node:assert/strict')
const { test } = require('node:test')
const code = fs.readFileSync(path.join(__dirname, 'public/app.js'), 'utf8')
const sdkCode = fs.readFileSync(path.join(__dirname, 'public/vendor/micro-sdk.js'), 'utf8')
function deferred() { let resolve; const promise = new Promise(r => { resolve = r }); return { promise, resolve } }
function trackedSignal() {
  const controller = new AbortController(), signal = controller.signal, listeners = new Set()
  const add = signal.addEventListener.bind(signal), remove = signal.removeEventListener.bind(signal)
  signal.addEventListener = (type, fn, opts) => { if (type === 'abort') listeners.add(fn); add(type, fn, opts) }
  signal.removeEventListener = (type, fn, opts) => { listeners.delete(fn); remove(type, fn, opts) }
  return { controller, signal, listeners }
}
function setup() {
  let clockNow = Date.now(), component, instance, id = 0
  class Clock extends Date { static now() { return clockNow } }
  const timers = new Map(), requests = [], notifications = [], mounted = [], signal = trackedSignal()
  const vue = {
    version: '3.5-test', h: (tag, props, children) => ({ tag, props, children }), reactive: o => o,
    createApp(definition) {
      component = definition
      let ownInstance
      return {
        mount() {
          ownInstance = definition.data()
          for (const [key, fn] of Object.entries(definition.methods)) ownInstance[key] = fn.bind(ownInstance)
          for (const [key, fn] of Object.entries(definition.computed)) Object.defineProperty(ownInstance, key, { get: () => fn.call(ownInstance) })
          instance = ownInstance; mounted.push(instance); definition.mounted.call(instance); return instance
        },
        unmount() { definition.beforeUnmount.call(ownInstance) },
      }
    },
  }
  const window = { Vue: vue, setInterval(fn) { timers.set(++id, fn); return id }, clearInterval(timerId) { timers.delete(timerId) } }
  vm.runInNewContext(sdkCode + '\n' + code, {
    window, Vue: vue, AbortController, Date: Clock, console, Headers, URL, TextEncoder, TextDecoder, DOMException, Uint8Array, Response, ReadableStream, crypto,
    fetch(url, options) { const result = deferred(); requests.push({ url, options, ...result }); return result.promise },
  })
  const props = {
    contractVersion: '2.0', appCode: 'vue-workshop', contextVersion: 'context-a', contextEpoch: 1,
    expiresAt: new Date(clockNow + 30000).toISOString(), tokenExpiresAt: new Date(clockNow + 300000).toISOString(),
    basePath: '/micro/vue-workshop', routePath: '/micro/vue-workshop', token: 'application-token-a',
    user: { id: 'user-1', username: 'admin', displayName: 'Admin' }, tenant: { id: 'tenant-1', code: 'alpha', name: 'Alpha' },
    organization: { id: 'org-a', name: 'Organization A', path: '/a' },
    roles: [{ id: 'role-1', name: 'Viewer', code: 'viewer' }], permissions: ['ticket:read'],
    dataGrants: [{ resource: 'ticket', action: 'read', scope: 'SELF', subjectUserId: 'user-1', tenantId: 'tenant-1', organizationIds: [] }],
    theme: 'light', menus: [{ id: 'menu-1', type: 'MENU', title: 'Ticket overview' }], signal: signal.signal,
    onNotify: (...args) => notifications.push(args), onNavigate() {},
  }
  return {
    lifecycle: window.__MICRO_APP__, props, requests, signal, timers, mounted, notifications, container: { innerHTML: '' },
    get instance() { return instance }, render() { return component.render.call(instance) },
    advance(ms) { clockNow += ms; for (const tick of timers.values()) tick() },
  }
}
function response(body, status = 200) { return new Response(JSON.stringify(body), { status }) }
function delayedResponse(promise) { return new Response(new ReadableStream({ async pull(controller) { const value = await promise; try { controller.enqueue(new TextEncoder().encode(JSON.stringify(value))); controller.close() } catch {} } })) }
const settle = () => new Promise(resolve => setImmediate(resolve))

test('MicroProps 2.0 displays separate roles, operations and independent data grants', async () => {
  const t = setup(); await t.lifecycle.mount(t.container, t.props)
  const output = JSON.stringify(t.render())
  for (const value of ['Alpha', 'Organization A', 'Viewer', 'ticket:read', 'SELF', 'Ticket overview']) assert.ok(output.includes(value), value)
  assert.equal(t.instance.p.dataGrants[0].scope, 'SELF')
  t.instance.sendNotify(); assert.deepEqual(t.notifications[0], ['来自 Vue 子应用的消息', 'success'])
  await t.lifecycle.update({ ...t.props, routePath: '/micro/vue-workshop/security', theme: 'dark' })
  assert.equal(t.instance.p.theme, 'dark'); assert.equal(t.mounted.length, 1)
  assert.ok(JSON.stringify(t.render()).includes('沙箱计时器')); await t.lifecycle.unmount()
})
test('renewed token aborts old work, refuses late fetch and uses new Bearer token', async () => {
  const t = setup(); await t.lifecycle.mount(t.container, t.props)
  const oldRequest = t.instance.loadMenus()
  assert.equal(t.requests[0].options.headers.get('Authorization'), 'Bearer application-token-a')
  assert.equal(t.requests[0].options.credentials, 'omit')
  assert.equal(t.lifecycle.sdkVersion, '1.0.0')
  await t.lifecycle.update({ ...t.props, token: 'application-token-b', contextEpoch: 2 })
  assert.equal(t.requests[0].options.signal.aborted, true); assert.equal(t.instance.menus.loading, false)
  t.requests[0].resolve(response({ menus: [{ title: 'old organization' }] })); await oldRequest
  assert.equal(t.instance.menus.data, null)
  const newRequest = t.instance.loadMenus()
  assert.equal(t.requests[1].options.headers.get('Authorization'), 'Bearer application-token-b')
  t.requests[1].resolve(response({ menus: [{ title: 'fresh menu' }] })); await newRequest
  assert.equal(t.instance.menus.data.menus[0].title, 'fresh menu'); await t.lifecycle.unmount()
})
test('organization update refuses delayed JSON and clears prior roles and permissions', async () => {
  const t = setup(); await t.lifecycle.mount(t.container, t.props)
  const pending = t.instance.loadRoles(), json = deferred()
  t.requests[0].resolve(delayedResponse(json.promise)); await settle()
  await t.lifecycle.update({ ...t.props, contextVersion: 'context-b', contextEpoch: 2,
    organization: { id: 'org-b', name: 'Organization B', path: '/b' }, roles: [], permissions: [], dataGrants: [], menus: [] })
  json.resolve({ organizations: [{ roles: [{ name: 'old privileged role' }] }] }); await pending
  assert.equal(t.instance.roles.data, null); assert.equal(t.instance.p.organization.id, 'org-b')
  assert.equal(t.instance.p.permissions.length, 0); assert.equal(t.instance.p.dataGrants.length, 0)
  assert.ok(!JSON.stringify(t.render()).includes('old privileged role')); await t.lifecycle.unmount()
})
test('host abort and context expiry block new requests and late responses', async () => {
  const t = setup(); await t.lifecycle.mount(t.container, t.props)
  const pending = t.instance.loadMenus(); t.signal.controller.abort()
  assert.equal(t.requests[0].options.signal.aborted, true)
  t.requests[0].resolve(response({ menus: [{ title: 'late' }] })); await pending
  assert.equal(t.instance.menus.data, null); assert.equal(t.instance.p.contextAvailable, false)
  await t.instance.loadMenus(); assert.equal(t.requests.length, 1)
  const fresh = trackedSignal(); await t.lifecycle.update({ ...t.props, signal: fresh.signal, contextEpoch: 2 })
  t.advance(31000); assert.equal(t.instance.p.contextAvailable, false)
  await t.instance.loadMenus(); assert.equal(t.requests.length, 1); await t.lifecycle.unmount()
})
test('repeated mount/unmount cleans timers and host listeners, ignores late unmounted work', async () => {
  const t = setup(); await t.lifecycle.mount(t.container, t.props); assert.equal(t.timers.size, 1)
  await t.lifecycle.mount(t.container, t.props); assert.equal(t.timers.size, 1); assert.equal(t.signal.listeners.size, 1)
  const instance = t.instance, pending = instance.loadMenus()
  await t.lifecycle.unmount(); await t.lifecycle.unmount()
  assert.equal(t.timers.size, 0); assert.equal(t.signal.listeners.size, 0); assert.equal(t.container.innerHTML, '')
  t.requests[0].resolve(response({ menus: [{ title: 'unmounted' }] })); await pending; assert.equal(instance.menus.data, null)
  await t.lifecycle.update(t.props); assert.equal(t.signal.listeners.size, 0)
  await t.lifecycle.mount(t.container, t.props); assert.equal(t.timers.size, 1); await t.lifecycle.unmount()
})
test('rejects legacy contract and malformed updates; public mirror stays identical', async () => {
  const t = setup()
  await assert.rejects(t.lifecycle.mount(t.container, { ...t.props, contractVersion: '1.0' }), /MicroProps 2.0/)
  await t.lifecycle.mount(t.container, t.props)
  await assert.rejects(t.lifecycle.update({ ...t.props, signal: undefined }), /MicroProps 2.0/)
  assert.equal(t.instance.p.contextAvailable, false); await t.lifecycle.unmount()
  assert.equal(fs.readFileSync(path.join(__dirname, '../../frontend/public/sub-apps/vue/app.js'), 'utf8'), code)
})

test('display updates preserve pending reads while same-hash grant replacement cancels and clears them', async () => {
  const t = setup(); await t.lifecycle.mount(t.container, t.props)
  const display = t.instance.loadMenus()
  await t.lifecycle.update({ ...t.props, theme: 'dark' })
  assert.equal(t.requests[0].options.signal.aborted, false)
  t.requests[0].resolve(response({ menus: [{ title: 'same authorization' }] })); await display
  assert.equal(t.instance.menus.data.menus[0].title, 'same authorization')
  const old = t.instance.loadMenus()
  await t.lifecycle.update({ ...t.props, theme: 'dark', permissions: [], dataGrants: [] })
  assert.equal(t.requests[1].options.signal.aborted, true)
  t.requests[1].resolve(response({ menus: [{ title: 'removed authorization' }] })); await old
  assert.equal(t.instance.menus.data, null)
  await t.lifecycle.unmount()
})
