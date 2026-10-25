import assert from 'node:assert/strict'
import test from 'node:test'
import { fixture, identity, contextResponse, json } from './helpers/session-fixture.mjs'

async function setup() {
  const f = await fixture(async () => json(200, {}))
  f.state().setProfile(contextResponse('ignored', 'ignored', identity()), f.state().contextEpoch)
  return { ...f, ...f.load('src/lib/recent-searches.ts') }
}

test('search history never imports an unbound global key and keeps only the current member history', async () => {
  const f = await setup(), key = f.state().contextKey
  f.storage.set('platform-recent-searches', JSON.stringify(['old private term']))
  assert.deepEqual(Array.from(f.readRecentSearches(key)), [])
  f.pushRecentSearch(' employee-only search ', key)
  f.pushRecentSearch('another search', key)
  f.pushRecentSearch('employee-only search', key)
  assert.deepEqual(Array.from(f.readRecentSearches(key)), ['employee-only search', 'another search'])
  for (let i = 0; i < 8; i++) f.pushRecentSearch(`query ${i}`, key)
  assert.equal(f.readRecentSearches(key).length, 6)
})

test('retained search callbacks cannot read or write another member generation in the same organization', async () => {
  const f = await setup(), oldKey = f.state().contextKey
  f.pushRecentSearch('prior generation', oldKey)
  const replacement = identity({ identityVersion: '2' })
  await f.state().setSession('new-access', 'psr_new', undefined, replacement)
  f.state().setProfile(contextResponse('ignored', 'ignored', replacement), f.state().contextEpoch)
  const nextKey = f.state().contextKey
  f.pushRecentSearch('stale retained callback', oldKey)
  assert.deepEqual(Array.from(f.readRecentSearches(oldKey)), [])
  assert.deepEqual(Array.from(f.readRecentSearches(nextKey)), [])
  f.pushRecentSearch('current generation', nextKey)
  assert.deepEqual(Array.from(f.readRecentSearches(nextKey)), ['current generation'])
  f.state().beginContextSwitch()
  f.pushRecentSearch('while switching', nextKey)
  assert.deepEqual(Array.from(f.readRecentSearches(nextKey)), [])
  assert.deepEqual(JSON.parse(f.storage.get(`platform-recent-searches-v2:${nextKey}`)), ['current generation'])
})
