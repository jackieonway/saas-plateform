// Actual jQuery lifecycle handlers with deterministic DOM/events and delayed fetch completions.
const fs = require('node:fs')
const path = require('node:path')
const vm = require('node:vm')
const assert = require('node:assert/strict')
const { test } = require('node:test')
const code = fs.readFileSync(path.join(__dirname, 'public/app.js'), 'utf8')
function deferred() { let resolve; const promise = new Promise(r => { resolve = r }); return { promise, resolve } }
function trackedSignal() {
  const controller = new AbortController(), signal = controller.signal, listeners = new Set()
  const add = signal.addEventListener.bind(signal), remove = signal.removeEventListener.bind(signal)
  signal.addEventListener = (type, fn, opts) => { if (type === 'abort') listeners.add(fn); add(type, fn, opts) }
  signal.removeEventListener = (type, fn, opts) => { listeners.delete(fn); remove(type, fn, opts) }
  return { controller, signal, listeners }
}
function element() {
  const attributes = {}
  return { innerHTML: '', textContent: '', style: {}, className: '', children: [],
    setAttribute(k, v) { attributes[k] = v }, removeAttribute(k) { delete attributes[k] }, getAttribute(k) { return attributes[k] },
    appendChild(child) { this.children.push(child) },
  }
}
function setup() {
  let clockNow = Date.now(), id = 0, markup = ''
  class Clock extends Date { static now() { return clockNow } }
  const timers = new Map(), requests = [], notifications = [], events = new Map(), signal = trackedSignal(), nodes = new Map()
  const container = {
    ...element(), get innerHTML() { return markup },
    set innerHTML(value) {
      markup = value; nodes.clear()
      for (const match of value.matchAll(/id="([^"]+)"/g)) nodes.set('#' + match[1], element())
      for (const cls of ['mf-jq-upd-badge', 'mf-jq-route', 'mf-jq-header', 'mf-jq-tokenbox', 'mf-jq-sub']) nodes.set('.' + cls, element())
    },
    querySelector(selector) { return nodes.get(selector) || null },
  }
  const document = {
    getElementById: id => container.querySelector('#' + id),
    querySelector: selector => container.querySelector(selector),
    querySelectorAll: () => [], createElement: () => element(),
  }
  function jquery(target) {
    return {
      jquery: '3.7-test',
      off(namespace) { for (const [key, entry] of events) if (entry.event.includes(namespace)) events.delete(key); return this },
      on(eventList, selector, handler) { for (const event of eventList.split(' ')) events.set(event + selector, { event, selector, handler }); return this },
      data(key) { return target && target.dataset && target.dataset[key] },
      val() { return target && target.value },
    }
  }
  jquery.fn = { jquery: '3.7-test' }
  const window = { jQuery: jquery,
    setInterval(fn) { timers.set(++id, fn); return id }, clearInterval(timerId) { timers.delete(timerId) },
  }
  vm.runInNewContext(code, {
    window, document, AbortController, Date: Clock, console,
    fetch(url, options) { const result = deferred(); requests.push({ url, options, ...result }); return result.promise },
  })
  const props = {
    contractVersion: '2.0', appCode: 'legacy-jquery', contextVersion: 'context-a', contextEpoch: 1,
    expiresAt: new Date(clockNow + 30000).toISOString(), tokenExpiresAt: new Date(clockNow + 300000).toISOString(),
    basePath: '/micro/legacy-jquery', routePath: '/micro/legacy-jquery', token: 'application-token-a',
    user: { id: 'user-1', username: 'admin', displayName: 'Admin' }, tenant: { id: 'tenant-1', code: 'alpha', name: 'Alpha' },
    organization: { id: 'org-a', name: 'Organization A', path: '/a' },
    roles: [{ id: 'role-1', name: 'Viewer', code: 'viewer' }], permissions: ['ticket:read'],
    dataGrants: [{ resource: 'ticket', action: 'read', scope: 'ORG', subjectUserId: 'user-1', tenantId: 'tenant-1', organizationIds: ['org-a'] }],
    theme: 'light', menus: [{ id: 'menu-1', type: 'MENU', title: 'Ticket overview' }], signal: signal.signal,
    onNotify: (...args) => notifications.push(args), onNavigate() {},
  }
  return {
    lifecycle: window.__DANTE_MICRO_APP__, props, requests, signal, timers, events, notifications, container,
    find: selector => container.querySelector(selector),
    click(selector) { return events.get('click.mfjq' + selector).handler.call(element()) },
    change(selector, value) { return events.get('change.mfjq' + selector).handler.call({ value }) },
    advance(ms) { clockNow += ms; for (const tick of timers.values()) tick() },
  }
}
function response(body, status = 200) { return { ok: status < 400, status, json: async () => body } }
const settle = () => new Promise(resolve => setImmediate(resolve))

test('MicroProps 2.0 displays tenant, organization, roles, operations and independent grants', async () => {
  const t = setup(); await t.lifecycle.mount(t.container, t.props)
  const output = t.find('#mf-jq-context').innerHTML
  for (const value of ['Alpha', 'Organization A', 'Viewer', 'ticket:read', 'ORG', 'org-a', 'Ticket overview']) assert.ok(output.includes(value), value)
  t.click('#mf-jq-btn-notify'); assert.deepEqual(t.notifications[0], ['来自 jQuery 遗留系统的消息', 'info'])
  t.change('#mf-jq-search', '数据库')
  const filtered = t.find('#mf-jq-tbody').innerHTML
  await t.lifecycle.update({ ...t.props, routePath: '/micro/legacy-jquery/other', theme: 'dark' })
  assert.equal(t.find('#mf-jq-tbody').innerHTML, filtered)
  assert.ok(t.find('#mf-jq-context').innerHTML.includes('dark'))
  assert.ok(t.find('.mf-jq-route').innerHTML.includes('/other')); await t.lifecycle.unmount()
})
test('renewed token aborts old request, refuses late response and sends new token', async () => {
  const t = setup(); await t.lifecycle.mount(t.container, t.props)
  const oldRequest = t.click('#mf-jq-btn-menus')
  assert.equal(t.requests[0].options.headers.Authorization, 'Bearer application-token-a')
  await t.lifecycle.update({ ...t.props, token: 'application-token-b', contextEpoch: 2 })
  assert.equal(t.requests[0].options.signal.aborted, true)
  t.requests[0].resolve(response({ menus: [{ title: 'old organization' }] })); await oldRequest
  assert.equal(t.find('#mf-jq-menu-list').innerHTML, '')
  const newRequest = t.click('#mf-jq-btn-menus')
  assert.equal(t.requests[1].options.headers.Authorization, 'Bearer application-token-b')
  t.requests[1].resolve(response({ menus: [{ title: 'fresh menu' }] })); await newRequest
  assert.ok(t.find('#mf-jq-menu-list').innerHTML.includes('fresh menu')); await t.lifecycle.unmount()
})
test('organization changes reject delayed JSON and replace independent permissions', async () => {
  const t = setup(); await t.lifecycle.mount(t.container, t.props)
  const pending = t.click('#mf-jq-btn-menus'), json = deferred()
  t.requests[0].resolve({ ok: true, status: 200, json: () => json.promise }); await settle()
  await t.lifecycle.update({ ...t.props, contextVersion: 'context-b', contextEpoch: 2,
    organization: { id: 'org-b', name: 'Organization B', path: '/b' }, roles: [], permissions: [], dataGrants: [], menus: [] })
  json.resolve({ menus: [{ title: 'old organization' }] }); await pending
  assert.equal(t.find('#mf-jq-menu-list').innerHTML, '')
  const output = t.find('#mf-jq-context').innerHTML
  assert.ok(output.includes('Organization B')); assert.ok(!output.includes('Organization A'))
  assert.ok(!output.includes('ticket:read')); assert.ok(!output.includes('Viewer')); await t.lifecycle.unmount()
})
test('host cancellation and expiry block requests and erase old authorized results', async () => {
  const t = setup(); await t.lifecycle.mount(t.container, t.props)
  const pending = t.click('#mf-jq-btn-menus'); t.signal.controller.abort()
  assert.equal(t.requests[0].options.signal.aborted, true)
  t.requests[0].resolve(response({ menus: [{ title: 'late' }] })); await pending
  assert.equal(t.find('#mf-jq-menu-list').innerHTML, '')
  assert.ok(t.find('#mf-jq-context').innerHTML.includes('上下文已失效'))
  await t.click('#mf-jq-btn-menus'); assert.equal(t.requests.length, 1)
  const fresh = trackedSignal(); await t.lifecycle.update({ ...t.props, signal: fresh.signal, contextEpoch: 2 })
  t.advance(31000); await t.click('#mf-jq-btn-menus'); assert.equal(t.requests.length, 1)
  assert.equal(t.find('#mf-jq-btn-menus').disabled, true); await t.lifecycle.unmount()
})
test('repeated mounting cleans document listeners, intervals and host listeners', async () => {
  const t = setup(); await t.lifecycle.mount(t.container, t.props)
  const eventCount = t.events.size; assert.ok(eventCount > 0); assert.equal(t.timers.size, 1)
  await t.lifecycle.mount(t.container, t.props)
  assert.equal(t.events.size, eventCount); assert.equal(t.timers.size, 1); assert.equal(t.signal.listeners.size, 1)
  const pending = t.click('#mf-jq-btn-menus'), detachedList = t.find('#mf-jq-menu-list')
  await t.lifecycle.unmount(); await t.lifecycle.unmount()
  assert.equal(t.events.size, 0); assert.equal(t.timers.size, 0); assert.equal(t.signal.listeners.size, 0)
  assert.equal(t.container.innerHTML, '')
  t.requests[0].resolve(response({ menus: [{ title: 'unmounted' }] })); await pending
  assert.equal(detachedList.innerHTML, '')
  await t.lifecycle.update(t.props); assert.equal(t.signal.listeners.size, 0)
  await t.lifecycle.mount(t.container, t.props); assert.equal(t.timers.size, 1); await t.lifecycle.unmount()
})
test('rejects legacy and malformed context; escapes values and keeps public mirror identical', async () => {
  const t = setup()
  await assert.rejects(t.lifecycle.mount(t.container, { ...t.props, contractVersion: '1.0' }), /MicroProps 2.0/)
  await t.lifecycle.mount(t.container, { ...t.props, tenant: { ...t.props.tenant, name: '<img onerror=alert(1)>' } })
  assert.ok(t.find('#mf-jq-context').innerHTML.includes('&lt;img'))
  assert.ok(!t.find('#mf-jq-context').innerHTML.includes('<img'))
  await assert.rejects(t.lifecycle.update({ ...t.props, signal: undefined }), /MicroProps 2.0/)
  assert.ok(t.find('#mf-jq-context').innerHTML.includes('上下文已失效')); await t.lifecycle.unmount()
  assert.equal(fs.readFileSync(path.join(__dirname, '../../frontend/public/sub-apps/jquery/app.js'), 'utf8'), code)
})
