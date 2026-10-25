/** Explicit server identities for isolated browser fixtures. These utilities do
 * not run in the product and never decode credentials or upgrade legacy storage. */
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { contextIdentityFromResponse, contextIdentityKey, isContextIdentity } from '../../src/lib/context-identity.ts'

export const SESSION_KEY = 'platform-session-v2'
export function fixtureUuid(label) {
  const hex = createHash('sha256').update(String(label)).digest('hex').slice(0, 32).split('')
  hex[12] = '7'; hex[16] = '8'
  const value = hex.join('')
  return `${value.slice(0, 8)}-${value.slice(8, 12)}-${value.slice(12, 16)}-${value.slice(16, 20)}-${value.slice(20)}`
}

export function identityForProfile(profile, patch = {}) {
  const proof = contextIdentityFromResponse({
    ...profile,
    membershipId: profile.membershipId ?? fixtureUuid(`membership:${profile.user.id}:${profile.organization.id}:EMPLOYEE`),
    memberType: profile.memberType ?? 'EMPLOYEE', identityVersion: profile.identityVersion ?? '1',
    accountVersion: profile.accountVersion ?? '0', ...patch,
  })
  assert(proof, 'Browser HTTP fixtures must provide v7 user/tenant/organization IDs and canonical string versions')
  return proof
}

export function profileWithIdentity(profile, patch = {}) {
  const proof = identityForProfile(profile, patch)
  const memberships = profile.memberships?.map(member => {
    const memberType = member.memberType ?? 'EMPLOYEE'
    const row = {
      userId: profile.user.id, ...member,
      membershipId: member.membershipId ?? fixtureUuid(`membership:${member.userId ?? profile.user.id}:${member.organizationId}:${memberType}`),
      memberType, identityVersion: member.identityVersion ?? '1', status: member.status ?? 'ENABLED',
      consoleSelectable: member.consoleSelectable ?? memberType === 'EMPLOYEE', selectionDisabledReason: member.selectionDisabledReason ?? null,
    }
    assert(isContextIdentity({ userId: row.userId, tenantId: row.tenantId, organizationId: row.organizationId,
      membershipId: row.membershipId, memberType: row.memberType, identityVersion: row.identityVersion, accountVersion: proof.accountVersion }))
    return row
  })
  return { ...profile, membershipId: proof.membershipId, memberType: proof.memberType,
    identityVersion: proof.identityVersion, accountVersion: proof.accountVersion,
    ...(memberships ? { memberships } : {}) }
}

export function sessionRecord(token, proof, refreshToken = null, sessionId = fixtureUuid(`browser-session:${token}`)) {
  assert(isContextIdentity(proof) && proof.memberType === 'EMPLOYEE', 'Console fixtures require a complete employee proof')
  return { schema: 2, version: 1, sessionId, status: 'active', token, refreshToken,
    identity: proof, contextKey: contextIdentityKey(proof) }
}

/** Explicit server configuration for retained legacy UI scenarios. */
export async function installLegacyAuthMode(context) {
  await context.route('**/auth/config', async route => {
    if (route.request().method() !== 'GET') return route.fallback()
    await route.fulfill({ status: 200, contentType: 'application/json',
      headers: { 'cache-control': 'no-store' }, body: JSON.stringify({ mode: 'LEGACY' }) })
  })
}

export async function installSession(context, token, proof, refreshToken = null) {
  await installLegacyAuthMode(context)
  const record = sessionRecord(token, proof, refreshToken)
  await context.addInitScript(({ key, record }) => {
    if (!localStorage.getItem(key)) localStorage.setItem(key, JSON.stringify(record))
  }, { key: SESSION_KEY, record })
}
