import assert from 'node:assert/strict'
import test from 'node:test'
import { componentFixture } from './helpers/component-fixture.mjs'

const uuid = '01962345-6789-7abc-8def-000000000001'
const uuid2 = '01962345-6789-7abc-8def-000000000002'
const uuid3 = '01962345-6789-7abc-8def-000000000003'
const isUuidV7 = value => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value)
const { partnerSchema, applicationSchema, manifestSchema, buildManifest, applicant, allowedDecisions, needsSync, directoryAllowed, safeReauthenticateUrl } = componentFixture(
  'src/pages/system/oauth/PartnerIntakePanel.tsx', ['partnerSchema', 'applicationSchema', 'manifestSchema', 'buildManifest', 'applicant', 'allowedDecisions', 'needsSync', 'directoryAllowed', 'safeReauthenticateUrl'], { '@/lib/uuid': { isUuidV7 } },
).exports
const manifest = () => ({ displayName: 'Owned partner login', purpose: 'Controlled account login', privacyUrl: 'https://partner.example.invalid/privacy', domains: ['partner.example.invalid'], redirectUris: ['https://partner.example.invalid/callback'], postLogoutRedirectUris: [], scopes: ['openid', 'profile'], contactReference: uuid, domainEvidenceReference: uuid2, logoutMode: 'SHORT_SESSION_ONLINE_CHECK' })

test('platform reauthentication accepts only its same-origin controlled entry and single fixed return target', () => {
  const origin = 'https://platform.example.invalid', entry = '/auth/step-up?return=/system/oauth/clients'
  for (const raw of [entry, `${origin}${entry}`, '/auth/step-up?return=%2Fsystem%2Foauth%2Fclients']) assert.equal(safeReauthenticateUrl(raw, origin), entry)
  for (const raw of [undefined, '', '/auth/login?return=/system/oauth/clients', '/auth/step-up/?return=/system/oauth/clients', '/auth/step-up', '/auth/step-up?return=/system/users', '/auth/step-up?return=https://evil.example.invalid', '/auth/step-up?return=//evil.example.invalid', `${entry}&return=/system/oauth/clients`, `${entry}&prompt=login`, `${entry}#`, `${entry}#token`, ` ${entry}`, `${entry}\n`, `https://evil.example.invalid${entry}`, `http://platform.example.invalid${entry}`, `https://user:password@platform.example.invalid${entry}`, `\\\\platform.example.invalid${entry}`, 'javascript:alert(1)']) {
    assert.equal(safeReauthenticateUrl(raw, origin), undefined, String(raw))
  }
})

test('manual partner intake requires bounded identity/contact evidence and rejects caller authority fields', () => {
  const value = { name: 'Owned partner', contactName: 'Owned fixture contact', contactChannel: 'verified-channel:owned', subjectReference: uuid }
  assert.equal(partnerSchema.safeParse(value).success, true)
  for (const patch of [{ name: '' }, { contactChannel: 'bad\nchannel' }, { subjectReference: 'v4-or-missing' }, { approvedBy: uuid2 }, { tenantId: uuid2 }]) assert.equal(partnerSchema.safeParse({ ...value, ...patch }).success, false)
  assert.equal(applicationSchema.safeParse({ partnerId: uuid, displayName: 'Owned app' }).success, true)
  assert.equal(applicationSchema.safeParse({ partnerId: uuid, displayName: 'Owned app', productionEnabled: true }).success, false)
  assert.equal(directoryAllowed(new Set(['system:oauth:register']), false), false)
  assert.equal(directoryAllowed(new Set(['system:oauth:register']), true), true)
  assert.equal(directoryAllowed(new Set(['system:oauth:review']), true), false)
  assert.equal(directoryAllowed(new Set(['system:oauth:register', 'system:oauth:assign']), false), true)
})

test('third-party application forms enforce exact approved callback ownership and minimal account scopes', () => {
  assert.equal(manifestSchema.safeParse(manifest()).success, true)
  for (const scope of ['email', 'phone', 'offline_access', 'stores.read']) assert.equal(manifestSchema.safeParse({ ...manifest(), scopes: ['openid', scope] }).success, false)
  for (const uri of ['http://partner.example.invalid/callback', 'https://other.example.invalid/callback', 'https://partner.example.invalid/callback?code=stale', 'https://*.example.invalid/callback']) assert.equal(manifestSchema.safeParse({ ...manifest(), redirectUris: [uri] }).success, false)
  assert.equal(manifestSchema.safeParse({ ...manifest(), domains: ['partner.example.invalid', 'partner.example.invalid'] }).success, false)
  assert.equal(manifestSchema.safeParse({ ...manifest(), requesterId: uuid3 }).success, false)
  assert.equal(manifestSchema.safeParse({ ...manifest(), purpose: 'PLATFORM_CONSOLE' }).success, false)
  assert.equal(manifestSchema.safeParse({ ...manifest(), logoutMode: 'BACK_CHANNEL', backchannelLogoutUri: 'https://partner.example.invalid/auth/backchannel-logout' }).success, true)
  for (const uri of ['https://other.example.invalid/logout', 'https://partner.example.invalid/logout?mode=all', 'not-a-url']) assert.equal(manifestSchema.safeParse({ ...manifest(), logoutMode: 'BACK_CHANNEL', backchannelLogoutUri: uri }).success, false)
  assert.equal(manifestSchema.safeParse({ ...manifest(), logoutMode: 'BACK_CHANNEL' }).success, false)
  assert.equal(manifestSchema.safeParse({ ...manifest(), backchannelLogoutUri: 'https://partner.example.invalid/logout' }).success, false)
  const { scopes, ...draft } = manifest()
  const built = buildManifest({ ...draft, domains: draft.domains.join('\n'), redirectUris: draft.redirectUris.join('\n'), postLogoutRedirectUris: '' })
  assert.deepEqual([...built.scopes], scopes)
})

test('workflow presentation never makes applicant approval or unapproved production an implicit transition', () => {
  const request = { requestedBy: uuid, submittedBy: uuid2, status: 'SUBMITTED', environment: 'test', version: 2, syncedVersion: 0 }
  const app = { createdBy: uuid3 }
  for (const actor of [uuid, uuid2, uuid3]) assert.equal(applicant(request, app, actor), true)
  assert.equal(applicant(request, app, '01962345-6789-7abc-8def-000000000004'), false)
  assert.deepEqual([...allowedDecisions(request)], ['RETURNED', 'TEST_APPROVED'])
  assert.deepEqual([...allowedDecisions({ ...request, environment: 'production' })], ['RETURNED', 'PRODUCTION_APPROVED'])
  assert.deepEqual([...allowedDecisions({ ...request, status: 'PRODUCTION_APPROVED', environment: 'production' })], ['ENABLED', 'SUSPENDED'])
  assert.equal(needsSync(request), false)
  assert.equal(needsSync({ ...request, status: 'PRODUCTION_APPROVED' }), false)
  assert.equal(needsSync({ ...request, status: 'TEST_APPROVED' }), true)
})

class FixtureApiError extends Error { constructor(message, code, status) { super(message); Object.assign(this, { code, status }) } }
const invitation = () => ({ id: uuid, recipientRef: 'verified-recipient:owned', purpose: 'ACCOUNT_ONBOARDING', status: 'ACTIVE', version: 1, createdAt: '2026-10-28T00:00:00Z', expiresAt: '2026-10-29T00:00:00Z', revokedAt: null, consumedAt: null })
function apiFixture(response = invitation()) {
  const calls = []
  const { identityReviewApi, checkedInvitation } = componentFixture('src/api/identity-review.ts', ['identityReviewApi', 'checkedInvitation'], {
    '@/lib/uuid': { isUuidV7 },
    '@/lib/api': { ApiError: FixtureApiError, api: { get: async () => ({ invitations: [response] }), post: async (path, body, options) => { calls.push({ path, body, options }); return response } } },
    '@/api/identity-clients': { identityMutation: async (action, label, run) => { calls.push({ action, label }); return run(uuid3) } },
  }).exports
  return { calls, identityReviewApi, checkedInvitation }
}

test('invitation UI transport exposes only metadata and preserves the shared pending operation', async () => {
  const fixture = apiFixture()
  await fixture.identityReviewApi.invite('verified-recipient:owned')
  assert.deepEqual(fixture.calls, [{ action: 'invite', label: '受控账号邀请' }, { path: '/api/identity/invitations', body: { recipientRef: 'verified-recipient:owned', purpose: 'ACCOUNT_ONBOARDING' }, options: { operationId: uuid3 } }])
  for (const field of ['token', 'clientSecret', 'password', 'deliveryMaterial']) {
    const bad = apiFixture({ ...invitation(), [field]: 'forbidden-private-value' })
    assert.throws(() => bad.checkedInvitation({ ...invitation(), [field]: 'forbidden-private-value' }), error => error.code === 'INVITATION_RESPONSE_INVALID')
    await assert.rejects(bad.identityReviewApi.invitations(), error => error.code === 'INVITATION_RESPONSE_INVALID')
    await assert.rejects(bad.identityReviewApi.invite('owned'), error => error.code === 'INVITATION_RESPONSE_INVALID')
  }
})
