import assert from 'node:assert/strict'
import test from 'node:test'
import { createMicroServices } from '../src/components/micro/services.ts'

function props(overrides = {}) {
  return { signal: new AbortController().signal,
    expiresAt: new Date(Date.now() + 60_000).toISOString(), tokenExpiresAt: new Date(Date.now() + 60_000).toISOString(),
    user: { id: 'u' }, tenant: { id: 't' }, permissions: ['ticket:export'],
    dataGrants: [{ resource: 'ticket', action: 'export', subjectUserId: 'u', tenantId: 't', scope: 'ORG' }], ...overrides }
}
const request = { resource: 'ticket', filename: '工单.csv', content: '编号,标题\n1,示例' }

test('host CSV capability checks current authorization on every invocation', () => {
  let current = props(), clock = 1000
  const saved = []
  const service = createMicroServices(() => current, { now: () => clock, saveCsv: (...args) => saved.push(args) })
  service.downloadCsv(request)
  assert.deepEqual(saved, [[request.filename, request.content]])
  assert.throws(() => service.downloadCsv(request), /CSV_DOWNLOAD_DENIED/)
  clock += 1000
  current = props({ permissions: [] })
  assert.throws(() => service.downloadCsv(request), /CSV_DOWNLOAD_DENIED/)
  current = props({ tenant: { id: 'other' } })
  assert.throws(() => service.downloadCsv(request), /CSV_DOWNLOAD_DENIED/)
  current = props({ user: { id: 'other' } })
  assert.throws(() => service.downloadCsv(request), /CSV_DOWNLOAD_DENIED/)
  current = props()
  service.downloadCsv(request)
  service.dispose()
  clock += 1000
  assert.throws(() => service.downloadCsv(request), /CSV_DOWNLOAD_DENIED/)
  assert.equal(saved.length, 2)
})

test('aborted, expired and absent contexts cannot use a retained host service', () => {
  for (const current of [null, props({ expiresAt: new Date(0).toISOString() }),
    props({ tokenExpiresAt: 'invalid' }), props({ signal: AbortSignal.abort() })]) {
    const service = createMicroServices(() => current, { saveCsv: () => assert.fail('must not save') })
    assert.throws(() => service.downloadCsv(request), /CSV_DOWNLOAD_DENIED/)
  }
})

test('host downloads reject unsafe names, mismatched grants and oversized UTF-8', () => {
  const service = createMicroServices(() => props(), { saveCsv: () => assert.fail('must not save') })
  for (const filename of ['../x.csv', 'x.html', '.x.csv', 'x/y.csv', 'x\u0000.csv']) {
    assert.throws(() => service.downloadCsv({ ...request, filename }), /CSV_DOWNLOAD_DENIED/)
  }
  assert.throws(() => service.downloadCsv({ ...request, resource: 'other' }), /CSV_DOWNLOAD_DENIED/)
  assert.throws(() => service.downloadCsv({ ...request, content: '中'.repeat(1_500_000) }), /CSV_DOWNLOAD_DENIED/)
  for (const grant of [{ resource: 'ticket', action: 'read', subjectUserId: 'u', tenantId: 't', scope: 'ORG' },
    { resource: 'ticket', action: 'export', subjectUserId: 'u', tenantId: 't', scope: 'UNKNOWN' }]) {
    const scoped = createMicroServices(() => props({ dataGrants: [grant] }), { saveCsv: () => assert.fail('must not save') })
    assert.throws(() => scoped.downloadCsv(request), /CSV_DOWNLOAD_DENIED/)
  }
})

test('host disposal revokes pending object URLs and removes temporary links', () => {
  const previousDocument = globalThis.document
  const createUrl = URL.createObjectURL, revokeUrl = URL.revokeObjectURL
  let clicked = 0, removed = 0
  const revoked = []
  globalThis.document = { createElement: () => ({ style: {}, click: () => clicked++, remove: () => removed++ }),
    body: { appendChild() {} } }
  URL.createObjectURL = () => 'blob:host-download'
  URL.revokeObjectURL = (value) => revoked.push(value)
  try {
    const service = createMicroServices(() => props())
    service.downloadCsv(request)
    assert.equal(clicked, 1)
    assert.equal(removed, 1)
    service.dispose()
    service.dispose()
    assert.deepEqual(revoked, ['blob:host-download'])
  } finally {
    globalThis.document = previousDocument
    URL.createObjectURL = createUrl
    URL.revokeObjectURL = revokeUrl
  }
})
