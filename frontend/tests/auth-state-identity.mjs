import assert from 'node:assert/strict'
import test from 'node:test'
import { contextIdentityKey } from '../src/lib/context-identity.ts'
import { fixture, identity, json } from './helpers/session-fixture.mjs'

test('SSO state exposes no browser credential replacement or refresh pair', async t => {
  const f = await fixture(async () => json(200, {}))
  t.after(() => f.load('src/lib/bff-session.ts').bffSessionCoordinator.dispose())
  assert.equal(f.state().setSession, undefined)
  assert.equal(f.state().setToken, undefined)
  assert.equal('refreshToken' in f.state(), false)
  assert.equal(f.storage.has('platform-session-v2'), false)
  assert.equal(f.storage.has('platform-refresh-token'), false)
  assert.deepEqual(JSON.parse(JSON.stringify(f.state().identity)), identity())
})

test('SSO account replacement retires member preferences and aborts old requests', async t => {
  const f = await fixture(async () => json(200, {}))
  t.after(() => f.load('src/lib/bff-session.ts').bffSessionCoordinator.dispose())
  const key = `platform-current-app-v2:${contextIdentityKey(identity())}`
  f.state().setCurrentApplication('employee-application')
  assert.equal(f.storage.get(key), 'employee-application')
  const signal = f.captureSessionSignal()
  await f.authenticate('new-context-bearer', undefined, undefined, identity({ identityVersion: '2' }))
  assert.equal(signal.aborted, true)
  assert.equal(f.state().currentApplicationId, null)
  assert.equal(f.storage.has(key), false)
  assert.equal(f.state().identity.identityVersion, '2')
})

test('retiring authorization preserves SSO identity but fences responses', async t => {
  const f = await fixture(async () => json(200, {}))
  t.after(() => f.load('src/lib/bff-session.ts').bffSessionCoordinator.dispose())
  const token = f.state().token, before = f.state().identity
  const signal = f.captureSessionSignal(), epoch = f.state().contextEpoch
  assert.equal(f.state().retireAuthorizationProjection(f.captureSessionFence()), true)
  assert.equal(signal.aborted, true)
  assert.equal(f.state().contextEpoch, epoch + 1)
  assert.equal(f.state().token, token)
  assert.equal(f.state().identity, before)
  assert.equal(f.state().profile, null)
})
