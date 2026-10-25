#!/usr/bin/env node
// User-authorized temporary SELF grant through the formal API; never writes SQL.
import { createRequire } from 'node:module'
import { randomBytes, createHash } from 'node:crypto'
import { readFile, writeFile, lstat, realpath, mkdir } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const require = createRequire(import.meta.url)
const io = require(fileURLToPath(new URL('../../../../frontend/node_modules/socket.io-client/build/cjs/index.js', import.meta.url))).io
const args = process.argv.slice(2), options = {}
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--execute') options.execute = true
  else if (['--credentials', '--handoff', '--business-evidence', '--scope-evidence', '--output-dir', '--origin'].includes(args[i]) && args[i + 1]) options[args[i].slice(2)] = args[++i]
  else throw new Error('INVALID_ARGUMENT')
}
const origin = new URL(options.origin || 'http://127.0.0.1:13000').origin
const ensure = (condition, code) => { if (!condition) throw new Error(code) }
ensure(new URL(origin).hostname === '127.0.0.1' && new URL(origin).protocol === 'http:', 'LOCAL_LOOPBACK_REQUIRED')
const v7 = value => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value)
const digest = value => createHash('sha256').update(value).digest('hex')
async function privateFile(filename) {
  ensure(path.isAbsolute(filename) && path.normalize(filename) === filename, 'PRIVATE_ABSOLUTE_PATH_REQUIRED')
  const s = await lstat(filename)
  ensure(s.isFile() && !s.isSymbolicLink() && s.nlink === 1 && s.uid === process.getuid() && (s.mode & 0o777) === 0o600 && s.size < 16 * 1024 * 1024, 'OWNED_PRIVATE_FILE_REQUIRED')
  ensure(await realpath(filename) === filename, 'PRIVATE_SYMLINK_REFUSED')
  return JSON.parse(await readFile(filename, 'utf8'))
}
const credentials = await privateFile(options.credentials)
const handoff = await privateFile(options.handoff)
const business = await privateFile(path.join(options['business-evidence'], 'identities.private.json'))
const businessReport = await privateFile(path.join(options['business-evidence'], 'report.private.json'))
ensure(businessReport.checks.some(c => c.name === 'work_order_create_read_owner_and_idempotency' && c.status === 'PASS'), 'SUCCESSFUL_PRIOR_TICKET_EVIDENCE_REQUIRED')
const scopeEvidence = await privateFile(options['scope-evidence'])
ensure(scopeEvidence && credentials.session?.accessToken && handoff.login?.password, 'PRIVATE_HANDOFF_INVALID')
const app = handoff.applications.filter(a => a.code === 'work-orders')
ensure(app.length === 1 && v7(app[0].applicationId) && v7(app[0].context?.organizationId), 'WORK_ORDER_HANDOFF_INVALID')
const expected = app[0].context
ensure(business.entities.ticket.ownerUserId === expected.userId && business.entities.ticket.organizationId === expected.organizationId, 'PRIOR_TICKET_OWNER_MISMATCH')
if (!options.execute) {
  console.log(JSON.stringify({ status: 'READY_NO_HTTP_EXECUTED', temporaryGrantScope: 'SELF', targetMemberships: 1, existingTicketReused: true, platformRestartPerformed: false }))
  process.exit(0)
}
const output = options['output-dir']
ensure(output && path.isAbsolute(output), 'PRIVATE_OUTPUT_REQUIRED')
const parent = await lstat(path.dirname(output))
ensure(parent.isDirectory() && parent.uid === process.getuid() && (parent.mode & 0o777) === 0o700 && await realpath(path.dirname(output)) === path.dirname(output), 'OWNED_PRIVATE_OUTPUT_PARENT_REQUIRED')
await mkdir(output, { mode: 0o700 })
const report = { format: 'saas-uuid-u05-authorized-notification/1', status: 'RUNNING', startedAt: new Date().toISOString(), checks: [], http: [], directDatabaseWrites: false, roleOrDirectoryChanges: false, targetMemberships: 1, grantScope: 'SELF', sharedSessionChanged: false, sharedRefreshConsumed: false, rootMustRestoreOriginalPolicyAfterGrantRemoval: true }
const privateEvidence = { entities: {}, grantTarget: null }
let stage = 'prepare', target, before, temporaryGrantPresent = false, cleanupVerified = false, session, tokenBeforeGrant, tokenDuringGrant, key, ticket, payload
const sockets = []
const hq = credentials.session.accessToken
let lastTokenRequestAt = 0
async function save() {
  await writeFile(path.join(output, 'report.private.json'), JSON.stringify(report, null, 2) + '\n', { mode: 0o600 })
  await writeFile(path.join(output, 'identities.private.json'), JSON.stringify(privateEvidence, null, 2) + '\n', { mode: 0o600 })
}
async function req(name, route, { token = hq, method = 'GET', body, statuses = [200] } = {}) {
  if (route === '/api/oauth/session-token') {
    const pause = Math.max(0, 3100 - (Date.now() - lastTokenRequestAt))
    if (pause) await new Promise(resolve => setTimeout(resolve, pause))
    lastTokenRequestAt = Date.now()
  }
  const url = new URL(route, origin); ensure(url.origin === origin, 'HTTP_ORIGIN_ESCAPE_REFUSED')
  const response = await fetch(url, { method, redirect: 'manual', headers: { Origin: origin, Authorization: `Bearer ${token}`, ...(body === undefined ? {} : { 'Content-Type': 'application/json' }) }, ...(body === undefined ? {} : { body: JSON.stringify(body) }), signal: AbortSignal.timeout(20000) })
  const raw = await response.text(); ensure(raw.length < 2 * 1024 * 1024, 'HTTP_RESPONSE_LIMIT')
  let value; try { value = raw ? JSON.parse(raw) : null } catch { throw new Error('HTTP_JSON_REQUIRED') }
  const code = typeof value?.code === 'string' && /^[A-Z_0-9]{1,64}$/.test(value.code) ? value.code : undefined
  report.http.push({ name, method, status: response.status, ...(code ? { code } : {}) })
  ensure(statuses.includes(response.status), `${name.toUpperCase().replaceAll('-', '_')}_HTTP_${response.status}${code ? `_${code}` : ''}`)
  if (route.startsWith('/work-orders-api/')) ensure(v7(response.headers.get('x-request-id')), 'REQUEST_ID_NOT_V7')
  return value
}
async function check(name, action) { stage = name; const detail = await action(); report.checks.push({ name, result: 'PASS', ...(detail || {}) }); await save() }
const rows = value => Array.isArray(value) ? value : value?.records || value?.items || []
const query = t => new URLSearchParams(t).toString()
const isNotification = g => g.resource === 'notification' && g.action === 'publish'
const grantSignature = grants => JSON.stringify([...grants].sort((a, b) => `${a.resource}:${a.action}`.localeCompare(`${b.resource}:${b.action}`)))
async function snapshot(name) { return req(name, `/api/data-grants?${query(target)}`) }
function wait(socket, event, predicate = () => true, milliseconds = 20000) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { socket.off(event, listener); reject(new Error(`SOCKET_${event.toUpperCase().replaceAll('-', '_')}_TIMEOUT`)) }, milliseconds)
    function listener(value) { if (predicate(value)) { clearTimeout(timer); socket.off(event, listener); resolve(value) } }
    socket.on(event, listener)
  })
}
async function removeTemporaryGrant() {
  ensure(target && before, 'GRANT_CLEANUP_BASELINE_REQUIRED')
  const current = await snapshot('grant-cleanup-read')
  const grant = current.grants.filter(isNotification)
  if (grant.length) {
    ensure(grant.length === 1 && grant[0].scope === 'SELF', 'UNEXPECTED_NOTIFICATION_GRANT_DURING_CLEANUP')
    const body = { target, expectedVersion: current.version, upsert: [], remove: [{ resource: 'notification', action: 'publish' }] }
    const preview = await req('grant-cleanup-preview', '/api/data-grants/preview', { method: 'POST', body })
    ensure(preview.changes.added.length === 0 && preview.changes.updated.length === 0 && preview.changes.removed.length === 1 && isNotification(preview.changes.removed[0]), 'GRANT_CLEANUP_PREVIEW_NOT_EXACT')
    const removed = await req('grant-cleanup-remove', '/api/data-grants', { method: 'PATCH', body })
    ensure(!removed.grants.some(isNotification), 'TEMPORARY_GRANT_NOT_REMOVED')
  }
  const after = await snapshot('grant-cleanup-readback')
  ensure(!after.grants.some(isNotification) && grantSignature(after.grants) === grantSignature(before.grants), 'BASELINE_GRANTS_NOT_RESTORED')
  cleanupVerified = true; temporaryGrantPresent = false
  report.temporaryGrantRemovedViaFormalApi = true
  report.retainedGrantIdsScopesAndAssociationsUnchanged = true
  await save()
}
try {
  await save()
  await check('independent_actor_session_and_pregrant_live_token', async () => {
    const login = await req('own-login', '/api/auth/login', { token: '', method: 'POST', body: handoff.login })
    session = await req('own-business-context', '/api/auth/context', { token: login.ticket, method: 'POST', body: { organizationId: expected.organizationId, userId: expected.userId } })
    ensure(session.accessToken && session.organization?.id === expected.organizationId && session.user?.id === expected.userId, 'OWN_SESSION_CONTEXT_MISMATCH')
    const token = await req('issue-live-token-before-grant', '/api/oauth/session-token', { token: session.accessToken, method: 'POST', body: { appCode: 'work-orders' }, statuses: [201] })
    tokenBeforeGrant = token.accessToken
    ticket = await req('read-existing-test-ticket', `/work-orders-api/tickets/${business.entities.ticket.id}`, { token: tokenBeforeGrant })
    ensure(ticket.ownerUserId === expected.userId && ticket.organizationId === expected.organizationId && v7(ticket.id), 'EXISTING_TICKET_MISMATCH')
    privateEvidence.entities.ticket = { id: ticket.id }
    return { existingWorkOrdersReused: 1, newWorkOrdersCreated: 0, sharedSessionChanged: false }
  })
  await check('exact_single_membership_grant_preview_and_create', async () => {
    const options = await req('discover-exact-membership', `/api/data-grants/options?${query({ applicationId: app[0].applicationId, userId: expected.userId, page: '1', size: '50' })}`)
    const matches = options.records.filter(row => row.target?.applicationId === app[0].applicationId && row.target?.organizationId === expected.organizationId && row.target?.tenantId === expected.tenantId && row.target?.userId === expected.userId)
    ensure(matches.length === 1 && matches[0].canWrite === true, 'EXACT_MANAGEABLE_MEMBERSHIP_NOT_UNIQUE')
    target = matches[0].target
    ensure(Object.values(target).every(v7), 'GRANT_TARGET_UUID_INVALID')
    privateEvidence.grantTarget = target
    before = await snapshot('grant-before')
    ensure(!before.grants.some(isNotification), 'PREEXISTING_NOTIFICATION_GRANT_REFUSED')
    const capability = before.capabilities.filter(isNotification)
    ensure(capability.length === 1 && JSON.stringify(capability[0].scopes) === '["SELF"]', 'EXACT_SELF_RUNTIME_POLICY_REQUIRED')
    privateEvidence.grantsBefore = before.grants
    await save()
    const change = { target, expectedVersion: before.version, upsert: [{ resource: 'notification', action: 'publish', scope: 'SELF' }], remove: [] }
    const preview = await req('grant-preview', '/api/data-grants/preview', { method: 'POST', body: change })
    ensure(preview.changes.added.length === 1 && isNotification(preview.changes.added[0]) && preview.changes.added[0].scope === 'SELF' && preview.changes.updated.length === 0 && preview.changes.removed.length === 0, 'GRANT_PREVIEW_NOT_EXACT_SINGLE_SELF_INSERT')
    const after = await req('grant-create', '/api/data-grants', { method: 'PATCH', body: change })
    temporaryGrantPresent = true
    const inserted = after.grants.filter(isNotification)
    ensure(inserted.length === 1 && inserted[0].scope === 'SELF' && v7(inserted[0].id), 'CREATED_GRANT_INVALID')
    ensure(grantSignature(after.grants.filter(g => !isNotification(g))) === grantSignature(before.grants), 'UNRELATED_GRANTS_CHANGED')
    privateEvidence.temporaryGrant = { id: inserted[0].id }
    return { addedGrants: 1, targetMemberships: 1, scope: 'SELF', otherGrantChanges: 0 }
  })
  await check('live_token_observes_self_grant_without_rebinding_identity', async () => {
    const oldProjection = await req('pregrant-live-token-current-context', '/api/apps/work-orders/context', { token: tokenBeforeGrant })
    const oldContext = oldProjection.context || oldProjection
    ensure(oldContext.dataGrants.filter(isNotification).length === 1 && oldContext.dataGrants.find(isNotification).scope === 'SELF', 'LIVE_TOKEN_DID_NOT_OBSERVE_GRANT')
    const minted = await req('issue-live-token-during-grant', '/api/oauth/session-token', { token: session.accessToken, method: 'POST', body: { appCode: 'work-orders' }, statuses: [201] })
    tokenDuringGrant = minted.accessToken
    return { independentlyIssuedLiveTokens: 2, userOrOrganizationChanged: false }
  })
  await check('real_notification_publication_socket_inbox_deep_link_and_idempotency', async () => {
    const context = await req('notification-authoritative-context', '/api/notifications/socket-context', { token: session.accessToken })
    ensure(['userId', 'tenantId', 'organizationId', 'sessionId'].every(k => v7(context[k])) && context.userId === expected.userId && context.organizationId === expected.organizationId, 'NOTIFICATION_CONTEXT_INVALID')
    const socket = io(origin, { path: '/socket.io/', transports: ['websocket'], extraHeaders: { Origin: origin }, forceNew: true, reconnection: false, autoConnect: false, timeout: 10000 })
    sockets.push(socket)
    const connected = wait(socket, 'connect'); socket.connect(); await connected
    const events = []; socket.on('notification', event => events.push(event))
    const authenticated = wait(socket, 'authenticated'); socket.emit('authenticate', { token: session.accessToken }); const ack = await authenticated
    ensure(ack.contractVersion === '1.0' && ack.userId === context.userId && ack.tenantId === context.tenantId && ack.organizationId === context.organizationId && !('sessionId' in ack), 'SOCKET_AUTHENTICATED_CONTEXT_MISMATCH')
    const inboxBefore = await req('inbox-before', '/api/notifications?page=1&size=100', { token: session.accessToken })
    key = `${business.run}-self-${randomBytes(4).toString('hex')}`
    const publication = await req('real-test-ticket-reminder', `/work-orders-api/tickets/${ticket.id}/remind`, { token: tokenDuringGrant, method: 'POST', body: { idempotencyKey: key } })
    ensure(v7(publication.publicationId) && publication.recipientCount === 1 && publication.deliveryCount === 1 && publication.duplicate === false, 'PUBLICATION_RECEIPT_INVALID')
    privateEvidence.entities.publication = { id: publication.publicationId }; await save()
    const deadline = Date.now() + 25000
    while (!events.some(e => e.publicationId === publication.publicationId) && Date.now() < deadline) await new Promise(resolve => setTimeout(resolve, 25))
    const event = events.find(e => e.publicationId === publication.publicationId)
    ensure(event && ['id', 'userId', 'tenantId', 'organizationId', 'applicationId', 'publicationId'].every(k => v7(event[k])), 'REAL_NOTIFICATION_EVENT_INVALID_OR_MISSING')
    ensure(event.applicationId === app[0].applicationId && event.userId === expected.userId && event.tenantId === expected.tenantId && event.organizationId === expected.organizationId, 'REAL_EVENT_IDENTITY_ASSOCIATION_MISMATCH')
    ensure(event.link === `/micro/work-orders/tickets/${ticket.id}` && event.contractVersion === '1.0' && !('sessionId' in event) && !('sourceKind' in event) && !('isRead' in event), 'REAL_EVENT_LINK_OR_PUBLIC_CONTRACT_MISMATCH')
    const inboxAfter = await req('inbox-after', '/api/notifications?page=1&size=100', { token: session.accessToken })
    const delivered = rows(inboxAfter).filter(n => n.id === event.id && n.publicationId === publication.publicationId)
    ensure(delivered.length === 1 && delivered[0].link === event.link && delivered[0].applicationId === app[0].applicationId, 'HTTP_INBOX_EVENT_ASSOCIATION_MISMATCH')
    ensure(rows(inboxAfter).length === rows(inboxBefore).length + 1, 'UNEXPECTED_NOTIFICATION_DELIVERY_COUNT')
    await req('notification-specific-visibility', `/api/notifications/socket-context?notificationId=${event.id}`, { token: session.accessToken })
    const duplicate = await req('same-reminder-idempotent-replay', `/work-orders-api/tickets/${ticket.id}/remind`, { token: tokenDuringGrant, method: 'POST', body: { idempotencyKey: key } })
    ensure(duplicate.publicationId === publication.publicationId && duplicate.duplicate === true, 'REMINDER_REPLAY_NOT_SAME_PUBLICATION')
    payload = { idempotencyKey: key, target: { kind: 'USER', userId: expected.userId, organizationId: expected.organizationId }, type: 'INFO', title: `工单提醒 ${ticket.number}`, content: `请查看工单 ${ticket.number} 的详情并处理。`, link: event.link }
    const direct = await req('direct-notification-http-idempotent-replay', '/api/apps/work-orders/notifications', { token: tokenBeforeGrant, method: 'POST', body: payload })
    ensure(direct.publicationId === publication.publicationId && direct.duplicate === true, 'DIRECT_PUBLICATION_REPLAY_MISMATCH')
    const inboxFinal = await req('inbox-after-replays', '/api/notifications?page=1&size=100', { token: session.accessToken })
    ensure(rows(inboxFinal).length === rows(inboxAfter).length, 'IDEMPOTENT_REPLAY_CREATED_NEW_DELIVERY')
    privateEvidence.entities.notification = { id: event.id, publicationId: event.publicationId }
    return { createdPublications: 1, deliveries: 1, websocketEventsCorrelated: 1, inboxRowsCorrelated: 1, v7ReferenceFieldsChecked: 6, deepLinkMatchesExistingTestTicket: true, duplicateReceipts: 2 }
  })
  await check('self_scope_rejects_organization_and_tenant_broadcast', async () => {
    for (const [kind, targetValue] of [['organization', { kind: 'ORGANIZATION', organizationId: expected.organizationId }], ['tenant', { kind: 'TENANT' }]]) {
      await req(`reject-${kind}-broadcast`, '/api/apps/work-orders/notifications', { token: tokenDuringGrant, method: 'POST', body: { ...payload, idempotencyKey: `${key}-${kind}`, target: targetValue }, statuses: [403] })
    }
    return { rejectedBroadcasts: 2, grantScope: 'SELF' }
  })
  await check('formal_grant_removal_preserves_original_grants', async () => { await removeTemporaryGrant(); return { removedGrants: 1, originalGrantSnapshotRestored: true } })
  await check('live_tokens_cannot_publish_after_grant_removal', async () => {
    for (const [name, token] of [['pregrant-live-token', tokenBeforeGrant], ['during-grant-live-token', tokenDuringGrant]]) {
      const context = await req(`${name}-still-valid-context`, '/api/apps/work-orders/context', { token })
      // The formal context endpoint itself accepts only a currently active app
      // token; its successful 200 distinguishes grant denial from token expiry.
      ensure(!(context.context || context).dataGrants.some(isNotification), 'TOKEN_NOT_ACTIVE_OR_GRANT_REMAINS')
      await req(`${name}-publication-rejected`, '/api/apps/work-orders/notifications', { token, method: 'POST', body: payload, statuses: [403] })
    }
    const minted = await req('issue-fresh-token-after-removal', '/api/oauth/session-token', { token: session.accessToken, method: 'POST', body: { appCode: 'work-orders' }, statuses: [201] })
    await req('fresh-postremoval-token-reminder-rejected', `/work-orders-api/tickets/${ticket.id}/remind`, { token: minted.accessToken, method: 'POST', body: { idempotencyKey: key }, statuses: [403] })
    return { stillActiveIssuedTokensRejected: 2, newlyIssuedTokenRejected: 1, denialWasNotTokenExpiry: true }
  })
  report.status = 'PASS'
} catch (error) {
  report.status = 'FAIL'; report.failureStage = stage
  report.failureCode = /^[A-Z_0-9]{1,180}$/.test(error?.message || '') ? error.message : 'VALIDATION_OPERATION_FAILED'
  process.exitCode = 1
} finally {
  if (target && before && !cleanupVerified) {
    try { await removeTemporaryGrant() } catch { report.grantCleanupFailure = true; report.status = 'FAIL'; process.exitCode = 1 }
  }
  for (const socket of sockets) socket.disconnect()
  if (session?.accessToken) {
    try { await req('own-session-logout', '/api/auth/logout', { token: session.accessToken, method: 'POST', body: {} }); report.ownSessionLoggedOut = true } catch { report.ownSessionLoggedOut = false; report.status = 'FAIL'; process.exitCode = 1 }
  }
  report.finishedAt = new Date().toISOString(); report.scriptSha256 = digest(await readFile(fileURLToPath(import.meta.url)))
  report.temporaryGrantMayRemain = temporaryGrantPresent || Boolean(report.grantCleanupFailure)
  await save()
  console.log(JSON.stringify({ status: report.status, checksPassed: report.checks.length, httpRequests: report.http.length, temporaryGrantRemoved: cleanupVerified, sharedSessionChanged: false, rootMustRestoreOriginalPolicy: true, ...(report.failureStage ? { failureStage: report.failureStage, failureCode: report.failureCode } : {}) }))
}
