import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourceRoot = process.env.DATA_GRANT_SOURCE_ROOT ? path.resolve(root, process.env.DATA_GRANT_SOURCE_ROOT) : root
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const source = fs.readFileSync(path.join(sourceRoot, 'src/lib/data-grants.ts'), 'utf8')
const output = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
const exports = {}
vm.runInNewContext(`(function(exports){${output}\n})`, {})(exports)
const { grantKey, createGrantDraft, buildGrantChange, canonicalGrantChange, changeFingerprint,
  assertPreviewMatches, parseDataGrantSnapshot, parseDataGrantPreview, parseDataGrantOptions } = exports
const id = number => `10000000-0000-4000-8000-${number.toString().padStart(12, '0')}`
const target = () => ({ tenantId: id(1), organizationId: id(2), userId: id(3), membershipId: id(4), applicationId: id(5) })
const row = (resource, action, scope, number = 6) => ({ id: id(number), resource, action, scope, effectiveScope: scope === 'TENANT' ? 'ORG_SUBTREE' : scope, organizationIds: [id(2)] })
const snapshot = () => ({ target: target(), version: 'a'.repeat(64),
  grants: [row('ticket', 'read', 'ORG'), row('legacy', 'read', 'TENANT', 7), row('ticket', 'write', 'TENANT', 8)],
  capabilities: [
    { resource: 'ticket', action: 'read', scopes: ['SELF', 'ORG'] },
    { resource: 'ticket', action: 'write', scopes: ['ORG'] },
    { resource: 'ticket', action: 'export', scopes: ['SELF'] },
  ],
})
const plain = value => JSON.parse(JSON.stringify(value))
const key = (resource, action) => grantKey({ resource, action })
const invalid = operation => assert.throws(operation, error => error.name === 'DataGrantValidationError')

function proposed() {
  const base = snapshot(), draft = createGrantDraft(base)
  draft[key('ticket', 'read')] = 'SELF'
  draft[key('ticket', 'export')] = 'SELF'
  const change = buildGrantChange(base, draft)
  const read = { ...base.grants[0], scope: 'SELF', effectiveScope: 'SELF' }
  const added = { ...row('ticket', 'export', 'SELF'), id: null }
  const preview = { target: target(), version: base.version, before: base.grants,
    after: [base.grants[2], added, base.grants[1], read],
    changes: { added: [added], updated: [read], removed: [] },
  }
  return { base, draft, change, preview }
}

test('grant draft leaves absent capabilities empty and historical pairs read-only', () => {
  const base = snapshot(), draft = createGrantDraft(base)
  assert.deepEqual(plain(draft), { [key('ticket', 'read')]: 'ORG', [key('ticket', 'export')]: null })
  assert.deepEqual(plain(buildGrantChange(base, draft)), { target: target(), expectedVersion: base.version, upsert: [], remove: [] })
  assert.equal(base.grants[1].id, id(7))
  assert.equal(base.grants[2].id, id(8))
  const tenantOnly = { ...base, grants: [], capabilities: [{ resource: 'ticket', action: 'read', scopes: ['TENANT'] }] }
  assert.equal(createGrantDraft(tenantOnly)[key('ticket', 'read')], null, 'a capability never selects its first or broadest scope')
})

test('delta contains only explicit registered edits and preserves every untouched legacy key', () => {
  const { base, change } = proposed()
  assert.deepEqual(plain(change.upsert), [
    { resource: 'ticket', action: 'export', scope: 'SELF' }, { resource: 'ticket', action: 'read', scope: 'SELF' },
  ])
  assert.deepEqual(plain(change.remove), [])
  assert.equal(JSON.stringify(change).includes('legacy'), false)
  assert.equal(JSON.stringify(change).includes('"id"'), false)
  assert.equal(base.grants[0].scope, 'ORG', 'building a delta cannot mutate the loaded snapshot')
})

test('only explicit null removes an existing grant, and omitted draft fields stay unchanged', () => {
  const base = snapshot()
  const removed = buildGrantChange(base, { [key('ticket', 'read')]: null })
  assert.deepEqual(plain(removed.remove), [{ resource: 'ticket', action: 'read' }])
  assert.deepEqual(plain(removed.upsert), [])
  assert.equal(buildGrantChange(base, {}).remove.length, 0)
  assert.equal(buildGrantChange(base, { [key('ticket', 'export')]: null }).remove.length, 0)
})

test('unknown keys, unknown scopes, out-of-policy scopes and legacy edits fail closed', () => {
  const base = snapshot()
  for (const value of ['ADMIN', 'org', 'TENANT', undefined, false, 1]) {
    invalid(() => buildGrantChange(base, { [key('ticket', 'read')]: value }))
  }
  for (const resource of ['unregistered', 'legacy']) invalid(() => buildGrantChange(base, { [key(resource, 'read')]: null }))
  invalid(() => buildGrantChange(base, { [key('ticket', 'write')]: 'ORG' }))
})

test('canonical request identity binds all five target IDs, version and exact delta', () => {
  const { change } = proposed(), reversed = plain(change)
  reversed.upsert.reverse()
  assert.equal(changeFingerprint(change), changeFingerprint(reversed))
  for (const field of Object.keys(target())) {
    const changed = plain(change)
    changed.target[field] = id(99)
    assert.notEqual(changeFingerprint(change), changeFingerprint(changed), `target ${field} was not bound`)
  }
  assert.notEqual(changeFingerprint(change), changeFingerprint({ ...change, expectedVersion: 'b'.repeat(64) }))
  const changed = plain(change); changed.upsert[1].scope = 'ORG'
  assert.notEqual(changeFingerprint(change), changeFingerprint(changed))
  assert.notEqual(key('ticket:read', 'all'), key('ticket', 'read:all'))
})

test('request validation rejects duplicate/conflicting keys and empty network mutations', () => {
  const { change } = proposed()
  invalid(() => canonicalGrantChange({ ...change, upsert: [...change.upsert, change.upsert[0]] }))
  invalid(() => canonicalGrantChange({ ...change, remove: [{ resource: 'ticket', action: 'read' }] }))
  invalid(() => canonicalGrantChange({ ...change, expectedVersion: 'A'.repeat(64) }))
  invalid(() => canonicalGrantChange({ ...change, expectedVersion: 'a'.repeat(64) + '\n' }))
  invalid(() => canonicalGrantChange({ ...change, target: { ...target(), userId: id(3) + '\n' } }))
  invalid(() => canonicalGrantChange({ ...change, upsert: [{ resource: 'ticket', action: 'read\n', scope: 'ORG' }] }))
  invalid(() => canonicalGrantChange({ ...change, target: { ...target(), membershipId: '00000000-0000-0000-0000-000000000000' } }))
  invalid(() => canonicalGrantChange({ ...change, upsert: [], remove: [] }, true))
  invalid(() => canonicalGrantChange({ ...change, upsert: Array.from({ length: 129 }, (_, n) => ({ resource: `r${n}`, action: 'read', scope: 'SELF' })) }))
})

test('preview binds the requested target/version/delta and retains existing IDs with nullable new IDs', () => {
  const { change, preview } = proposed()
  assert.doesNotThrow(() => assertPreviewMatches(change, preview))
  const parsed = parseDataGrantPreview(preview)
  assert.equal(parsed.changes.added[0].id, null)
  assert.equal(parsed.changes.updated[0].id, id(6))
  assert.equal(parsed.after.find(item => item.resource === 'legacy').id, id(7))
})

test('preview mismatches cannot authorize confirmation of a different or incomplete change', () => {
  const { change, preview } = proposed()
  const corruptions = [
    value => { value.target.membershipId = id(99) },
    value => { value.version = 'b'.repeat(64) },
    value => { value.after.find(item => item.action === 'read' && item.resource === 'ticket').id = id(99) },
    value => { value.after.find(item => item.action === 'export').id = id(99) },
    value => { value.after = value.after.filter(item => item.resource !== 'legacy') },
    value => { value.after.find(item => item.resource === 'legacy').effectiveScope = 'TENANT' },
    value => { value.changes.updated = [] },
    value => { value.changes.added[0].scope = 'ORG' },
    value => { value.after.push(row('extra', 'read', 'SELF', 99)) },
  ]
  for (const corrupt of corruptions) {
    const value = plain(preview); corrupt(value)
    invalid(() => assertPreviewMatches(change, value))
  }
})

test('snapshot DTO rejects invalid identities, duplicate rows and malformed closed scope sets', () => {
  const corruptions = [
    value => { value.target.userId = 'user-a' },
    value => { value.version = 'unversioned' },
    value => { value.grants[0].id = null },
    value => { value.grants[0].effectiveScope = 'ALL' },
    value => { value.grants.push(value.grants[0]) },
    value => { value.grants[1].id = value.grants[0].id },
    value => { value.capabilities[0].scopes = [] },
    value => { value.capabilities[0].scopes = ['ORG', 'ORG'] },
    value => { value.capabilities[0].scopes = ['ALL'] },
  ]
  for (const corrupt of corruptions) {
    const value = snapshot(); corrupt(value)
    invalid(() => parseDataGrantSnapshot(value))
  }
})

test('discovery DTO uses complete nested direct-member targets and explicit canWrite', () => {
  const options = { applications: [{ id: id(5), code: 'tickets', name: 'Tickets' }], records: [{
    target: target(), username: 'member', displayName: 'Member', tenantName: 'Tenant', organizationName: 'Organization', canWrite: false,
  }], total: 1, page: 1, size: 20 }
  assert.deepEqual(plain(parseDataGrantOptions(options)), options)
  invalid(() => parseDataGrantOptions({ ...options, records: [{ ...options.records[0], target: undefined, ...target() }] }))
  invalid(() => parseDataGrantOptions({ ...options, records: [{ ...options.records[0], canWrite: 'true' }] }))
  invalid(() => parseDataGrantOptions({ ...options, total: 0 }))
  const records = Array.from({ length: 50 }, (_, index) => ({
    ...options.records[0], target: { ...target(), membershipId: id(100 + index) },
  }))
  assert.equal(parseDataGrantOptions({ ...options, records, total: 50, size: 50 }).records.length, 50)
  assert.equal(parseDataGrantOptions({ ...options, size: 1 }).size, 1)
  assert.equal(parseDataGrantOptions({ ...options, records: [], total: 0, page: 1_000_000 }).page, 1_000_000)
  invalid(() => parseDataGrantOptions({ ...options, records: [...records, { ...records[0], target: { ...target(), membershipId: id(150) } }], total: 51, size: 50 }))
  invalid(() => parseDataGrantOptions({ ...options, records: records.slice(0, 2), total: 2, size: 1 }))
  for (const page of [0, 1.5, 1_000_001]) invalid(() => parseDataGrantOptions({ ...options, page }))
  for (const size of [0, 1.5, 51, 1000]) invalid(() => parseDataGrantOptions({ ...options, size }))
})
