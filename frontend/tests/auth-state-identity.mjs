import assert from 'node:assert/strict'
import test from 'node:test'
import { contextIdentityKey } from '../src/lib/context-identity.ts'
import { fixture, identity, contextResponse, json, sharedOrigin } from './helpers/session-fixture.mjs'

test('a naked bearer cannot replace the current authenticated member proof', async () => {
  const f = await fixture(async () => json(200, {})), before = f.record()
  await assert.rejects(f.state().setSession('unproven-access', 'psr_unproven'), /成员证明/)
  await assert.rejects(f.state().setToken('unproven-access'), /成员证明/)
  assert.deepEqual(f.record(), before)
})

test('application selection has a concrete-member namespace and is discarded on replacement', async () => {
  const f = await fixture(async () => json(200, {})), original = identity()
  f.storage.set('platform-current-app', 'legacy-owner-application')
  assert.equal(f.state().currentApplicationId, null)
  f.state().setCurrentApplication('employee-application')
  const originalKey = `platform-current-app-v2:${contextIdentityKey(original)}`
  assert.equal(f.storage.get(originalKey), 'employee-application')
  const replacement = identity({ membershipId: '019d0000-0000-7000-8000-000000000006', identityVersion: '2' })
  const oldSignal = f.captureSessionSignal()
  await f.state().setSession('replacement-access', 'psr_replacement', undefined, replacement)
  assert.equal(oldSignal.aborted, true)
  assert.equal(f.state().currentApplicationId, null)
  assert.equal(f.storage.has(originalKey), false)
  assert.equal(f.storage.has('platform-current-app'), false)
  f.state().setCurrentApplication('replacement-application')
  assert.equal(f.storage.get(`platform-current-app-v2:${contextIdentityKey(replacement)}`), 'replacement-application')
})

test('reload restores only the preference in the complete proven member namespace', async () => {
  const origin = sharedOrigin(), first = await origin.page(async () => json(200, {}))
  await first.state().setSession('current-access', 'psr_current', undefined, identity())
  first.state().setCurrentApplication('current-member-app')
  origin.storage.set('platform-current-app', 'legacy-owner-app')
  const reloaded = await origin.page(async () => json(200, {}))
  assert.equal(reloaded.state().currentApplicationId, 'current-member-app')
  await reloaded.state().setSession('next-member', 'psr_next', undefined, identity({ identityVersion: '2' }))
  assert.equal(reloaded.state().currentApplicationId, null)
})

test('a mismatched profile cannot install applications and retires the bound local session', async () => {
  const f = await fixture(async () => json(200, {}))
  const correct = contextResponse('ignored', 'ignored', identity())
  correct.applications = [{ id: 'allowed' }]
  f.state().setProfile(correct, f.state().contextEpoch)
  assert.equal(f.state().profile.applications[0].id, 'allowed')
  const changed = { ...correct, membershipId: '019d0000-0000-7000-8000-000000000006', applications: [{ id: 'other-member' }] }
  f.state().setProfile(changed, f.state().contextEpoch)
  await f.state().reconcileSession()
  assert.equal(f.record().status, 'anonymous')
  assert.equal(f.state().profile, null)
  assert.equal(f.state().identity, null)
  assert.equal(f.state().token, null)
})

test('admission policy saves immediately retire authorization and requests without changing proven credentials', async () => {
  const f = await fixture(async () => json(200, {}))
  const profile = contextResponse('ignored', 'ignored', identity())
  profile.applications = [{ id: 'active-app' }]
  f.state().setProfile(profile, f.state().contextEpoch)
  f.state().setCurrentApplication('active-app')
  const fence = f.captureSessionFence(), signal = f.captureSessionSignal(), before = f.record(), epoch = f.state().contextEpoch
  assert.equal(f.state().retireAuthorizationProjection(fence), true)
  assert.equal(signal.aborted, true)
  assert.equal(f.state().profile, null)
  assert.equal(f.state().currentApplicationId, null)
  assert.equal(f.state().contextEpoch, epoch + 1)
  assert.deepEqual(f.record(), before, 'Projection retirement does not invent a new member or rotate credentials')
  await f.state().setSession('replacement', 'psr_replacement', undefined, identity({ identityVersion: '2' }))
  const replacementEpoch = f.state().contextEpoch
  assert.equal(f.state().retireAuthorizationProjection(fence), false)
  assert.equal(f.state().contextEpoch, replacementEpoch, 'A stale policy callback cannot retire another identity')
})
