/** Explicit server identities for isolated browser fixtures. These utilities do
 * not run in the product and never decode credentials or upgrade legacy storage. */
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { contextIdentityFromResponse, contextIdentityKey, isContextIdentity } from '../../src/lib/context-identity.ts'

export const SESSION_KEY = 'platform-session-v2'
const profileHints = new WeakMap()
const installed = new WeakMap()
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
  profileHints.set(proof, profile)
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

/** HTTP-only SSO fixtures: no credential is restored from browser storage. */
export async function installSsoAuthMode(context) {
  if (installed.has(context)) return installed.get(context)
  const state = { token: null, proof: null, profile: null, selected: true, version: 1, sessionId: fixtureUuid('sso-browser-session') }
  installed.set(context, state)
  const reply = (route, body, status = 200) => route.fulfill({ status, contentType: 'application/json', headers: { 'cache-control': 'no-store' }, body: JSON.stringify(body) })
  const membership = (proof, profile) => ({ ...proof, tenantName: profile?.tenant?.name ?? 'Fixture tenant',
    organizationName: profile?.organization?.name ?? 'Fixture organization', organizationPath: profile?.organization?.path ?? '/Fixture',
    status: 'ENABLED', consoleSelectable: true, selectionDisabledReason: null })
  await context.route('**/auth/**', async route => {
    const path = new URL(route.request().url()).pathname
    if (path === '/auth/config') return reply(route, { mode: 'BFF' })
    if (path === '/auth/session') {
      if (!state.proof) return reply(route, {}, 401)
      const profile = state.profile, proof = state.proof
      const members = profile?.memberships?.filter(m => m.memberType === 'EMPLOYEE' && m.status === 'ENABLED') ?? [membership(proof, profile)]
      if (!members.some(m => m.membershipId === proof.membershipId)) members.push(membership(proof, profile))
      return reply(route, { sessionId: state.sessionId, version: String(state.version), csrfToken: 'a'.repeat(64),
        ...(state.centralizedContextSelection ? { centralizedContextSelection: true } : {}),
        account: { id: proof.userId, username: profile?.user?.username ?? 'sso-fixture', realName: profile?.user?.realName ?? 'SSO fixture', avatar: '' },
        memberships: members, context: state.selected ? proof : null, expiresAt: new Date(Date.now()+3600000).toISOString() })
    }
    if (path === '/auth/context-token' && state.proof) {
      const body = route.request().postDataJSON()
      if (body.membershipId) {
        const member = state.profile?.memberships?.find(m => m.membershipId === body.membershipId)
        if (member && member.memberType === 'EMPLOYEE') state.proof = { userId: state.proof.userId, tenantId: member.tenantId,
          organizationId: member.organizationId, membershipId: member.membershipId, memberType: 'EMPLOYEE',
          identityVersion: member.identityVersion, accountVersion: state.proof.accountVersion }
        else if (body.membershipId !== state.proof.membershipId) return reply(route, {}, 409)
        state.selected = true; state.version++
      }
      const profile = state.profile, proof = state.proof
      return reply(route, { ...proof, sessionId: state.sessionId, version: String(state.version), accessToken: state.token, expiresIn: 300,
        user: { id: proof.userId, username: profile?.user?.username ?? 'sso-fixture', realName: profile?.user?.realName ?? 'SSO fixture', email: '', phone: '', avatar: '', gender: '', isSuperAdmin: false },
        tenant: { id: proof.tenantId, name: profile?.tenant?.name ?? 'Fixture tenant', code: profile?.tenant?.code ?? 'fixture' },
        organization: { id: proof.organizationId, name: profile?.organization?.name ?? 'Fixture organization', path: profile?.organization?.path ?? '/Fixture' } })
    }
    if (path === '/auth/logout') { state.proof = null; state.token = null; return reply(route, { redirectTo: '/login?signedOut=1' }) }
    return route.fallback()
  })
  await context.route('**/__sso-fixture/session', async route => {
    const body = route.request().postDataJSON()
    state.token = body.token; state.proof = body.proof; state.profile = body.profile ?? null
    state.sessionId = fixtureUuid('sso-browser-session:' + body.token); state.version = 1
    return reply(route, { ok: true })
  })
  await context.addInitScript(() => {
    window.__replaceFixtureSession = async (token, proof, profile) => {
      await fetch('/__sso-fixture/session', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ token, proof, profile }) })
      const { bffSessionCoordinator } = await import('/src/lib/bff-session.ts')
      await bffSessionCoordinator.reconcile()
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))
    }
  })
  return state
}

export async function installSession(context, token, proof) {
  const state = await installSsoAuthMode(context)
  state.token = token; state.proof = proof; state.profile = profileHints.get(proof) ?? null
  state.sessionId = fixtureUuid('sso-browser-session:' + token)
  return state
}
