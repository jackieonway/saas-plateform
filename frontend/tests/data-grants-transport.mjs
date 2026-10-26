import assert from 'node:assert/strict'
import test from 'node:test'
import path from 'node:path'
import { fixture as sessionFixture, deferred, identity, json, root } from './helpers/session-fixture.mjs'

const sourceRoot = process.env.DATA_GRANT_SOURCE_ROOT ? path.resolve(root, process.env.DATA_GRANT_SOURCE_ROOT) : root
const fixture = handler => sessionFixture(handler, { sourceRoot })
const requestId = 'b7307be0-6a17-701a-9be5-a57545376afb'
const id = number => `20000000-0000-7000-8000-${number.toString().padStart(12, '0')}`
const target = () => ({ tenantId: id(1), organizationId: id(2), userId: id(3), membershipId: id(4), applicationId: id(5) })
const grant = (scope = 'ORG') => ({ id: id(6), resource: 'ticket', action: 'read', scope, effectiveScope: scope, organizationIds: [id(2)] })
const snapshot = (scope = 'ORG') => ({ target: target(), version: 'b'.repeat(64), grants: [grant(scope)], capabilities: [{ resource: 'ticket', action: 'read', scopes: ['ORG', 'SELF'] }] })
const change = () => ({ target: target(), expectedVersion: 'a'.repeat(64), upsert: [{ resource: 'ticket', action: 'read', scope: 'ORG' }], remove: [] })
const preview = () => ({ target: target(), version: 'a'.repeat(64), before: [grant('SELF')], after: [grant()], changes: { added: [], updated: [grant()], removed: [] } })
const response = (status, value, receipt = requestId) => {
  const result = json(status, value)
  if (receipt !== undefined) result.headers.set('x-request-id', receipt)
  return result
}
const api = f => f.load('src/api/data-grants.ts').dataGrantApi
const plain = value => JSON.parse(JSON.stringify(value))

test('grant PATCH uses shared session transport and returns only fenced requestId metadata', async () => {
  const f = await fixture(async () => response(200, snapshot()))
  const receipt = await api(f).change(change())
  assert.equal(f.calls.length, 1)
  assert.equal(f.calls[0].url, '/api/data-grants')
  assert.equal(f.calls[0].options.method, 'PATCH')
  assert.equal(f.calls[0].options.headers.Authorization, 'Bearer expired-access')
  assert.deepEqual(JSON.parse(f.calls[0].options.body), change())
  assert.deepEqual(plain(receipt), { snapshot: snapshot(), requestId })
  assert.deepEqual(Object.keys(receipt).sort(), ['requestId', 'snapshot'])
  assert.equal(JSON.stringify(receipt).includes('expired-access'), false)
})

test('successful metadata callback exposes no response headers or credentials', async () => {
  const callbacks = [], f = await fixture(async () => response(200, { ok: true }))
  await f.api.api.patch('/api/test', { approved: true }, { onResponse: meta => callbacks.push(meta) })
  assert.equal(callbacks.length, 1)
  assert.deepEqual(plain(callbacks[0]), { requestId })
  assert.deepEqual(Object.keys(callbacks[0]), ['requestId'])
})

test('successful metadata waits for body parsing and rejects a context changed while reading it', async () => {
  const reading = deferred(), release = deferred(), callbacks = []
  const f = await fixture(async () => ({ ok: true, status: 200, headers: new Headers({ 'x-request-id': requestId }),
    text: async () => { reading.resolve(); await release.promise; return JSON.stringify({ ok: true }) },
  }))
  const pending = assert.rejects(f.api.api.patch('/api/test', {}, { onResponse: meta => callbacks.push(meta) }), error => error.name === 'AbortError')
  await reading.promise
  assert.equal(callbacks.length, 0)
  f.state().beginContextSwitch()
  release.resolve()
  await pending
  assert.equal(callbacks.length, 0)
})

test('grant response from a retired logical session cannot return a successful save receipt', async () => {
  const started = deferred(), release = deferred()
  const f = await fixture(async () => { started.resolve(); await release.promise; return response(200, snapshot()) })
  const pending = assert.rejects(api(f).change(change()), error => error.name === 'AbortError')
  await started.promise
  await f.authenticate('replacement-platform-access', 'psr_replacement', undefined, identity())
  release.resolve()
  await pending
  assert.equal(f.state().token, 'replacement-platform-access')
})

test('invalid or missing success requestId never becomes trusted receipt metadata', async () => {
  for (const receipt of [undefined, 'untrusted-request-id', 'secret:Bearer-private-value', '',
    '019c0000-0000-4000-8000-000000000001', // Intentional old-format correlation.
    '019c0000-0000-7000-c000-000000000001']) {
    const f = await fixture(async () => receipt === undefined ? json(200, snapshot()) : response(200, snapshot(), receipt))
    const result = await api(f).change(change())
    assert.equal(result.requestId, undefined)
    assert.equal(JSON.stringify(result).includes(receipt || 'never-present'), false)
  }
})

test('HTTP refusal retains ApiError requestId and never invokes a success callback', async () => {
  const callbacks = [], f = await fixture(async () => response(403, { code: 'FORBIDDEN', message: 'grant boundary denied' }))
  await assert.rejects(f.api.api.patch('/api/data-grants', change(), { onResponse: meta => callbacks.push(meta) }), error => error.status === 403 && error.code === 'FORBIDDEN' && error.requestId === requestId)
  assert.equal(callbacks.length, 0)
})

test('read URI contains all five target identities and discovery uses encoded filters', async () => {
  const application = { id: id(5), code: 'tickets', name: 'Tickets' }
  const f = await fixture(async url => url.startsWith('/api/data-grants/options')
    ? response(200, { applications: [application], records: [], total: 0, page: 2, size: 20 })
    : response(200, snapshot()))
  const controller = new AbortController()
  await api(f).read(target(), controller.signal)
  const read = new URL(f.calls[0].url, 'https://owned.invalid')
  assert.deepEqual(Object.fromEntries(read.searchParams), target())
  assert.ok(f.calls[0].options.signal instanceof AbortSignal)
  const keyword = '成员 & A/B? #授权'
  await api(f).options({ applicationId: id(5), userId: id(3), keyword, page: 2, size: 20 }, controller.signal)
  const options = new URL(f.calls[1].url, 'https://owned.invalid')
  assert.equal(options.pathname, '/api/data-grants/options')
  assert.equal(options.searchParams.get('keyword'), keyword)
  assert.equal(options.searchParams.get('applicationId'), id(5))
  assert.equal(options.searchParams.get('userId'), id(3))
  assert.equal(options.searchParams.get('page'), '2')
  assert.equal(options.searchParams.get('size'), '20')
  assert.equal(options.searchParams.size, 5)
})

test('caller cancellation uses the shared lifetime and cannot deliver grant results', async () => {
  const started = deferred(), release = deferred(), controller = new AbortController()
  const f = await fixture(async () => { started.resolve(); await release.promise; return response(200, snapshot()) })
  const pending = assert.rejects(api(f).read(target(), controller.signal), error => error.name === 'AbortError')
  await started.promise
  controller.abort()
  assert.equal(f.calls[0].options.signal.aborted, true)
  release.resolve()
  await pending
})

test('preview captures a detached canonical request and validates its exact returned change', async () => {
  const started = deferred(), release = deferred()
  const f = await fixture(async () => { started.resolve(); await release.promise; return response(200, preview()) })
  const mutable = change(), pending = api(f).preview(mutable)
  await started.promise
  mutable.target.membershipId = id(99)
  mutable.upsert[0].scope = 'SELF'
  release.resolve()
  const result = await pending
  assert.deepEqual(JSON.parse(f.calls[0].options.body), change())
  assert.equal(result.target.membershipId, id(4))
  assert.equal(f.calls[0].options.method, 'POST')
})

test('unexpected target or preview delta becomes a recoverable protocol error with requestId', async () => {
  const foreign = snapshot(); foreign.target.membershipId = id(99)
  const f = await fixture(async () => response(200, foreign))
  await assert.rejects(api(f).read(target()), error => error.status === 502 && error.code === 'PROTOCOL_ERROR' && error.requestId === requestId)
  await assert.rejects(api(f).change(change()), error => error.status === 502 && error.requestId === requestId)
  const wrong = preview(); wrong.changes.updated = []
  const p = await fixture(async () => response(200, wrong))
  await assert.rejects(api(p).preview(change()), error => error.status === 502 && error.code === 'PROTOCOL_ERROR' && error.requestId === requestId)
})

test('empty deltas are rejected before HTTP and discovery cannot return members without an app', async () => {
  const f = await fixture(async () => response(200, { applications: [], records: [{ target: target(), username: 'member', displayName: '', tenantName: 'Tenant', organizationName: 'Org', canWrite: false }], total: 1, page: 1, size: 20 }))
  const empty = { ...change(), upsert: [] }
  await assert.rejects(api(f).preview(empty), error => error.name === 'DataGrantValidationError')
  await assert.rejects(api(f).change(empty), error => error.name === 'DataGrantValidationError')
  assert.equal(f.calls.length, 0)
  await assert.rejects(api(f).options(), error => error.status === 502 && error.code === 'PROTOCOL_ERROR')
  const emptyOptions = { applications: [], records: [], total: 0, page: 1, size: 20 }
  const defaults = await fixture(async () => response(200, emptyOptions))
  assert.deepEqual(plain(await api(defaults).options()), emptyOptions)
  for (const [query, pagination] of [
    [{}, { page: 2, size: 20 }],
    [{}, { page: 1, size: 10 }],
    [{ page: 2, size: 10 }, { page: 1, size: 10 }],
    [{ page: 2, size: 10 }, { page: 2, size: 20 }],
  ]) {
    const wrongPage = await fixture(async () => response(200, { ...emptyOptions, ...pagination }))
    await assert.rejects(api(wrongPage).options(query), error => error.status === 502 && error.code === 'PROTOCOL_ERROR' && error.requestId === requestId)
  }
})
