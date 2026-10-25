import assert from 'node:assert/strict'
import test from 'node:test'
import { isUuidV7, newUuid } from '../src/lib/uuid.ts'

test('browser identifier source produces unique RFC UUID v7 with a millisecond timestamp', () => {
  const earliest = Date.now(), values = Array.from({ length: 10_000 }, newUuid), latest = Date.now()
  assert.equal(new Set(values).size, values.length)
  for (const value of values) {
    assert.ok(isUuidV7(value))
    const time = Number.parseInt(value.replaceAll('-', '').slice(0, 12), 16)
    assert.ok(time >= earliest && time <= latest)
  }
})

test('typed identifiers reject intentional v4, nil, invalid variant and malformed samples', () => {
  for (const value of [
    '019c0000-0000-4000-8000-000000000001', // Intentional legacy v4.
    '00000000-0000-0000-0000-000000000000',
    '019c0000-0000-7000-c000-000000000001',
    '019c0000000070008000000000000001', 'user-a', null, undefined,
  ]) assert.equal(isUuidV7(value), false)
  assert.ok(isUuidV7('019C0000-0000-7000-A000-000000000001'))
})
