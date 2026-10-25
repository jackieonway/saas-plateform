#!/usr/bin/env node
// Real U05 services only. No database access, migration, fixture server or token logging.
import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { randomBytes, createHash } from 'node:crypto'
import { readFile, writeFile, lstat, realpath, mkdir } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const require = createRequire(import.meta.url)
const argv = process.argv.slice(2)
const options = {}
for (let i = 0; i < argv.length; i++) {
  const arg = argv[i]
  if (arg === '--execute') options.execute = true
  else if (['--credentials', '--handoff', '--output-dir', '--origin', '--chat-origin', '--resume-evidence'].includes(arg) && argv[i + 1]) options[arg.slice(2)] = argv[++i]
  else throw new Error('INVALID_ARGUMENT')
}
const origin = new URL(options.origin || 'http://127.0.0.1:13000').origin
const chatOrigin = new URL(options['chat-origin'] || 'http://127.0.0.1:13008').origin
for (const url of [origin, chatOrigin]) {
  assert.equal(new URL(url).hostname, '127.0.0.1', 'LOOPBACK_REQUIRED')
  assert.equal(new URL(url).protocol, 'http:', 'LOCAL_HTTP_REQUIRED')
}
const uuid7 = value => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value)
const ensure = (condition, code) => { if (!condition) throw new Error(code) }
async function privateJson(filename) {
  ensure(filename && path.isAbsolute(filename) && path.normalize(filename) === filename, 'PRIVATE_ABSOLUTE_PATH_REQUIRED')
  const metadata = await lstat(filename)
  ensure(metadata.isFile() && !metadata.isSymbolicLink() && metadata.nlink === 1 && (metadata.mode & 0o777) === 0o600, 'PRIVATE_FILE_REQUIRED')
  ensure(typeof process.getuid === 'function' && metadata.uid === process.getuid(), 'PRIVATE_FILE_OWNER_REQUIRED')
  ensure(await realpath(filename) === filename && metadata.size <= 16 * 1024 * 1024, 'PRIVATE_PATH_OR_LIMIT_INVALID')
  return JSON.parse(await readFile(filename, 'utf8'))
}
const io = require(fileURLToPath(new URL('../../../../frontend/node_modules/socket.io-client/build/cjs/index.js', import.meta.url))).io
const credentials = await privateJson(options.credentials)
const handoff = await privateJson(options.handoff)
const resumed = options['resume-evidence'] ? { identities: await privateJson(path.join(options['resume-evidence'], 'identities.private.json')), report: await privateJson(path.join(options['resume-evidence'], 'report.private.json')) } : null
if (resumed) for (const name of ['store_read_safe_versioned_update_and_platform_association', 'work_order_create_read_owner_and_idempotency']) ensure(resumed.report.checks.some(check => check.name === name && check.status === 'PASS'), 'RESUME_REQUIRES_PRIOR_SUCCESSFUL_BUSINESS_EVIDENCE')
ensure(credentials.session?.accessToken && handoff.login?.username && handoff.login?.password && uuid7(handoff.context?.organizationId) && uuid7(handoff.context?.userId), 'PRIVATE_CREDENTIAL_STRUCTURE_INVALID')
if (!options.execute) {
  console.log(JSON.stringify({ status: 'READY_NO_HTTP_EXECUTED', serviceContracts: ['stores', 'work-orders', 'notification', 'chat'], sharedRefreshConsumed: false }))
  process.exit(0)
}
ensure(options['output-dir'] && path.isAbsolute(options['output-dir']), 'PRIVATE_OUTPUT_REQUIRED')
const output = options['output-dir']
const parent = await lstat(path.dirname(output))
ensure(parent.isDirectory() && (parent.mode & 0o777) === 0o700 && parent.uid === process.getuid(), 'PRIVATE_OUTPUT_PARENT_REQUIRED')
ensure(await realpath(path.dirname(output)) === path.dirname(output), 'PRIVATE_OUTPUT_SYMLINK_REFUSED')
await mkdir(output, { mode: 0o700 })
const run = resumed?.identities.run || `u05-${Date.now()}-${randomBytes(4).toString('hex')}`
const report = { format: 'saas-uuid-u05-business-validation/1', status: 'RUNNING', startedAt: new Date().toISOString(), checks: [], http: [], counts: {}, limits: [], sharedRefreshConsumed: false, fixtureServices: false, ...(resumed ? { reusedPriorBusinessEvidenceSha256: createHash('sha256').update(JSON.stringify(resumed.report)).digest('hex') } : {}) }
const identities = resumed ? structuredClone(resumed.identities) : { run, entities: {} }
const sockets = []
let stage = 'initialize'
let privateSession
let privateSessionEnded = false
const ownSessions = []
async function save() {
  await writeFile(path.join(output, 'report.private.json'), JSON.stringify(report, null, 2) + '\n', { mode: 0o600 })
  await writeFile(path.join(output, 'identities.private.json'), JSON.stringify(identities, null, 2) + '\n', { mode: 0o600 })
}
async function check(name, operation) {
  stage = name
  const details = await operation()
  report.checks.push({ name, status: 'PASS', ...(details || {}) })
  await save()
}
async function request(label, pathname, { method = 'GET', token, body, headers = {}, statuses = [200], target = origin } = {}) {
  const url = new URL(pathname, target)
  ensure(url.origin === target, 'HTTP_ORIGIN_ESCAPE_REFUSED')
  const response = await fetch(url, { method, redirect: 'manual', headers: { Origin: origin, ...(body === undefined ? {} : { 'Content-Type': 'application/json' }), ...(token ? { Authorization: `Bearer ${token}` } : {}), ...headers }, ...(body === undefined ? {} : { body: JSON.stringify(body) }), signal: AbortSignal.timeout(20000) })
  const text = await response.text()
  ensure(text.length <= 2 * 1024 * 1024, 'HTTP_RESPONSE_LIMIT')
  let value
  try { value = text ? JSON.parse(text) : null } catch { throw new Error('HTTP_JSON_REQUIRED') }
  const code = typeof value?.code === 'string' && /^[A-Z_0-9]{1,64}$/.test(value.code) ? value.code : undefined
  const requestId = response.headers.get('x-request-id')
  report.http.push({ name: label, method, status: response.status, ...(code ? { code } : {}), ...(requestId ? { requestIdV7: uuid7(requestId) } : {}) })
  ensure(statuses.includes(response.status), `${label.toUpperCase().replaceAll('-', '_')}_HTTP_${response.status}${code ? `_${code}` : ''}`)
  if (pathname.startsWith('/stores-api/') || pathname.startsWith('/work-orders-api/')) ensure(uuid7(requestId), 'BUSINESS_REQUEST_ID_NOT_V7')
  return value
}
function rows(value) { return Array.isArray(value) ? value : value?.records || value?.items || value?.data || [] }
function listen(socket, event, predicate = () => true, timeout = 15000) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { socket.off(event, handler); reject(new Error(`SOCKET_${event.toUpperCase().replaceAll('-', '_')}_TIMEOUT`)) }, timeout)
    function handler(value) { if (predicate(value)) { clearTimeout(timer); socket.off(event, handler); resolve(value) } }
    socket.on(event, handler)
  })
}
async function connect(target, socketPath) {
  const socket = io(target, { path: socketPath, transports: ['websocket'], extraHeaders: { Origin: origin }, forceNew: true, reconnection: false, timeout: 10000, autoConnect: false, withCredentials: false })
  sockets.push(socket)
  const connected = listen(socket, 'connect', () => true, 12000)
  socket.connect()
  await connected
  return socket
}
try {
  await save()
  let storesToken, ordersToken, storeContext, orderContext
  await check('independent_business_session_and_tokens', async () => {
    const login = await request('independent-login', '/api/auth/login', { method: 'POST', body: handoff.login })
    ensure(login.ticket, 'LOGIN_TICKET_MISSING')
    for (const code of ['stores', 'work-orders']) {
      const expected = handoff.applications.find(app => app.code === code)?.context
      ensure(uuid7(expected?.organizationId) && uuid7(expected?.userId), 'APPLICATION_HANDOFF_CONTEXT_REQUIRED')
      const session = await request(`business-context-${code}`, '/api/auth/context', { method: 'POST', token: login.ticket, body: { organizationId: expected.organizationId, userId: expected.userId } })
      ensure(session.accessToken && session.organization?.id === expected.organizationId && session.user?.id === expected.userId, 'BUSINESS_CONTEXT_MISMATCH')
      ownSessions.push(session)
      if (code === 'work-orders') privateSession = session
      const issued = await request(`issue-${code}`, '/api/oauth/session-token', { method: 'POST', token: session.accessToken, body: { appCode: code }, statuses: [201] })
      ensure(issued.accessToken, 'APP_TOKEN_MISSING')
      const projection = await request(`context-${code}`, `/api/apps/${code}/context`, { token: issued.accessToken })
      const context = projection.context || projection
      ensure(uuid7(context.tenant?.id) && uuid7(context.organization?.id) && uuid7(context.user?.id), 'APP_CONTEXT_ID_INVALID')
      ensure(context.user.id === expected.userId && context.organization.id === expected.organizationId, 'APP_CONTEXT_MISMATCH')
      if (code === 'stores') { storesToken = issued.accessToken; storeContext = context } else { ordersToken = issued.accessToken; orderContext = context }
    }
    return { independentlyCreatedSessions: 2, independentlyIssuedAppTokens: 2, sharedSessionChanged: false }
  })
  await check('store_read_safe_versioned_update_and_platform_association', async () => {
    const list = await request('stores-list', '/stores-api/stores?page=1&size=50', { token: storesToken })
    const candidates = rows(list)
    ensure(Array.isArray(candidates) && candidates.length > 0, 'EXISTING_VISIBLE_STORE_REQUIRED')
    const store = await request('store-detail', `/stores-api/stores/${resumed?.identities.entities.store.id || candidates[0].id}`, { token: storesToken })
    for (const key of ['id', 'tenantId', 'organizationId', 'ownerUserId']) ensure(uuid7(store[key]), 'STORE_ID_INVALID')
    ensure(store.tenantId === storeContext.tenant.id && store.organization?.id === store.organizationId && store.organization?.name === store.name, 'STORE_PLATFORM_ASSOCIATION_INVALID')
    if (resumed) {
      ensure(store.organizationId === resumed.identities.entities.store.organizationId && store.ownerUserId === resumed.identities.entities.store.ownerUserId && store.version >= resumed.identities.entities.store.beforeVersion + 1, 'RESUMED_STORE_IDENTITY_MISMATCH')
    } else {
      identities.entities.store = { id: store.id, organizationId: store.organizationId, ownerUserId: store.ownerUserId, beforeVersion: store.version }
      await save()
      // Preserve the business value; the real versioned API still records one safe update.
      const updated = await request('store-safe-update', `/stores-api/stores/${store.id}`, { method: 'PATCH', token: storesToken, body: { version: store.version, businessContact: store.businessContact } })
      ensure(updated.id === store.id && updated.version === store.version + 1 && updated.businessContact === store.businessContact && updated.organizationId === store.organizationId && updated.ownerUserId === store.ownerUserId, 'STORE_UPDATE_CHANGED_IDENTITY_OR_VALUE')
      const reread = await request('store-reread', `/stores-api/stores/${store.id}`, { token: storesToken })
      ensure(reread.version === updated.version && reread.businessContact === store.businessContact, 'STORE_REREAD_MISMATCH')
    }
    const staff = await request('store-platform-members', `/stores-api/stores/${store.id}/staff?page=1&size=50`, { token: storesToken })
    ensure(Array.isArray(rows(staff)), 'PLATFORM_MEMBER_PROJECTION_INVALID')
    report.counts.platformStoreMembers = rows(staff).length
    return { storesRead: 1, safeVersionedUpdates: resumed ? 0 : 1, ...(resumed ? { reusedSuccessfulPriorUpdate: true } : {}), businessContactPreserved: true, creationSkippedBecauseExistingStorePreserved: true }
  })
  let ticket
  const ticketBody = { title: `UUID v7 acceptance ${run}`, description: 'Local U05 cross-service validation', priority: 'NORMAL' }
  await check('work_order_create_read_owner_and_idempotency', async () => {
    ticket = resumed ? await request('existing-validation-ticket', `/work-orders-api/tickets/${resumed.identities.entities.ticket.id}`, { token: ordersToken }) : await request('ticket-create', '/work-orders-api/tickets', { method: 'POST', token: ordersToken, headers: { 'Idempotency-Key': `${run}-ticket` }, body: ticketBody, statuses: [201] })
    for (const key of ['id', 'tenantId', 'organizationId', 'ownerUserId']) ensure(uuid7(ticket[key]), 'TICKET_ID_INVALID')
    ensure(ticket.ownerUserId === orderContext.user.id && ticket.tenantId === orderContext.tenant.id && ticket.organizationId === orderContext.organization.id, 'TICKET_OWNERSHIP_MISMATCH')
    ensure(ticket.status === 'OPEN' && ticket.version === 1, 'TICKET_INITIAL_STATE_INVALID')
    identities.entities.ticket = { id: ticket.id, tenantId: ticket.tenantId, organizationId: ticket.organizationId, ownerUserId: ticket.ownerUserId, version: ticket.version }
    await save()
    const detail = await request('ticket-detail', `/work-orders-api/tickets/${ticket.id}`, { token: ordersToken })
    ensure(detail.id === ticket.id && detail.title === ticketBody.title && detail.ownerUserId === orderContext.user.id, 'TICKET_REREAD_MISMATCH')
    const listed = await request('ticket-list', `/work-orders-api/tickets?page=1&size=100&q=${encodeURIComponent(run)}`, { token: ordersToken })
    ensure(rows(listed).some(row => row.id === ticket.id), 'TICKET_LIST_MISSING_NEW_ROW')
    if (!resumed) {
      const replay = await request('ticket-replay', '/work-orders-api/tickets', { method: 'POST', token: ordersToken, headers: { 'Idempotency-Key': `${run}-ticket` }, body: ticketBody, statuses: [201] })
      ensure(replay.id === ticket.id && replay.number === ticket.number, 'TICKET_REPLAY_CREATED_DIFFERENT_ROW')
      await request('ticket-idempotency-conflict', '/work-orders-api/tickets', { method: 'POST', token: ordersToken, headers: { 'Idempotency-Key': `${run}-ticket` }, body: { ...ticketBody, title: `${ticketBody.title} conflict` }, statuses: [409] })
    }
    return resumed ? { createdTickets: 0, existingTicketReused: true, priorCreationAndIdempotencyEvidencePreserved: true } : { createdTickets: 1, idempotentReplays: 1, conflictingReplaysRejected: 1 }
  })
  await check('notification_real_reminder_http_socket_and_inbox', async () => {
    const context = await request('notification-context', '/api/notifications/socket-context', { token: privateSession.accessToken })
    for (const key of ['userId', 'tenantId', 'organizationId', 'sessionId']) ensure(uuid7(context[key]), 'NOTIFICATION_CONTEXT_ID_INVALID')
    ensure(context.userId === orderContext.user.id && context.organizationId === orderContext.organization.id, 'NOTIFICATION_CONTEXT_MISMATCH')
    const socket = await connect(origin, '/socket.io/')
    const events = []
    socket.on('notification', event => events.push(event))
    const authentication = listen(socket, 'authenticated')
    socket.emit('authenticate', { token: privateSession.accessToken })
    const ack = await authentication
    ensure(ack.contractVersion === '1.0' && ack.userId === context.userId && ack.tenantId === context.tenantId && ack.organizationId === context.organizationId && !('sessionId' in ack), 'NOTIFICATION_AUTHENTICATED_MISMATCH')
    const key = `${run}-reminder`
    if (!orderContext.dataGrants.some(grant => grant.resource === 'notification' && grant.action === 'publish')) {
      const before = await request('notification-inbox-before-denied-publish', '/api/notifications?page=1&size=100', { token: privateSession.accessToken })
      const denied = await request('ticket-reminder-grant-denied', `/work-orders-api/tickets/${ticket.id}/remind`, { method: 'POST', token: ordersToken, body: { idempotencyKey: key }, statuses: [403] })
      ensure(denied.code === 'DATA_DENIED', 'EXPECTED_NOTIFICATION_GRANT_DENIAL_MISSING')
      const hqContext = await request('hq-notification-context', '/api/notifications/socket-context', { token: credentials.session.accessToken })
      await request('hq-narrow-user-publish-without-grant', '/api/notifications', { method: 'POST', token: credentials.session.accessToken, body: { applicationId: orderContext.application.id, idempotencyKey: `${run}-hq-probe`, target: { kind: 'USER', userId: hqContext.userId, organizationId: hqContext.organizationId }, type: 'INFO', title: 'U05 authorization boundary probe', content: '', link: '/micro/work-orders' }, statuses: [403] })
      const after = await request('notification-inbox-after-denied-publish', '/api/notifications?page=1&size=100', { token: privateSession.accessToken })
      ensure(JSON.stringify(rows(before).map(row => row.id).sort()) === JSON.stringify(rows(after).map(row => row.id).sort()), 'DENIED_NOTIFICATION_CHANGED_INBOX')
      for (const row of rows(after)) for (const field of ['id', 'publicationId', 'applicationId']) if (row[field] !== null && row[field] !== undefined) ensure(uuid7(row[field]), 'EXISTING_NOTIFICATION_ID_INVALID')
      const oldState = await privateJson(path.join(path.dirname(options.handoff), 'old-state.private.json'))
      const oldToken = oldState.businessSession?.accessToken || oldState.platformSession?.accessToken
      ensure(typeof oldToken === 'string', 'OLD_SESSION_EVIDENCE_MISSING')
      await request('old-session-notification-http-rejected', '/api/notifications/socket-context', { token: oldToken, statuses: [401] })
      const oldSocket = await connect(origin, '/socket.io/')
      const rejected = listen(oldSocket, 'unauthorized')
      oldSocket.emit('authenticate', { token: oldToken })
      const rejection = await rejected
      ensure(rejection.code === 'SESSION_INVALID', 'OLD_SESSION_SOCKET_NOT_REJECTED')
      oldSocket.disconnect(); socket.disconnect()
      report.limits.push('Notification create/event delivery is not covered: frozen policy declares no notification/publish grant, and neither business nor HQ publication is authorized. No privilege or policy was changed.')
      report.counts.existingNotificationRows = rows(after).length
      return { publicationAuthorized: false, actualPublicationsCreated: 0, positiveDeliveryCovered: false, authenticatedNewV7Subject: true, businessGrantDenialVerified: true, hqGrantDenialVerified: true, oldSessionHttpAndSocketRejected: true, inboxUnchanged: true }
    }
    const publication = await request('ticket-reminder-create', `/work-orders-api/tickets/${ticket.id}/remind`, { method: 'POST', token: ordersToken, body: { idempotencyKey: key } })
    ensure(uuid7(publication.publicationId) && publication.recipientCount === 1 && publication.deliveryCount === 1 && publication.duplicate === false, 'PUBLICATION_RECEIPT_INVALID')
    identities.entities.publication = { id: publication.publicationId }
    await save()
    const deadline = Date.now() + 20000
    while (!events.some(event => event.publicationId === publication.publicationId) && Date.now() < deadline) await new Promise(resolve => setTimeout(resolve, 25))
    const event = events.find(item => item.publicationId === publication.publicationId)
    ensure(event && uuid7(event.id) && uuid7(event.applicationId) && event.contractVersion === '1.0', 'NOTIFICATION_EVENT_MISSING_OR_INVALID')
    ensure(event.userId === context.userId && event.tenantId === context.tenantId && event.organizationId === context.organizationId, 'NOTIFICATION_EVENT_RECIPIENT_MISMATCH')
    ensure(!('sourceKind' in event) && !('sessionId' in event) && !('isRead' in event), 'NOTIFICATION_EVENT_PRIVATE_FIELDS')
    identities.entities.notification = { id: event.id, publicationId: event.publicationId }
    const inbox = await request('notification-inbox', '/api/notifications?page=1&size=100', { token: privateSession.accessToken })
    ensure(rows(inbox).some(row => row.id === event.id && row.publicationId === publication.publicationId), 'NOTIFICATION_INBOX_MISMATCH')
    await request('notification-row-context', `/api/notifications/socket-context?notificationId=${event.id}`, { token: privateSession.accessToken })
    const again = await request('ticket-reminder-replay', `/work-orders-api/tickets/${ticket.id}/remind`, { method: 'POST', token: ordersToken, body: { idempotencyKey: key } })
    ensure(again.publicationId === publication.publicationId && again.duplicate === true, 'REMINDER_REPLAY_NOT_DUPLICATE')
    const direct = await request('notification-direct-http-replay', '/api/apps/work-orders/notifications', { method: 'POST', token: ordersToken, body: { idempotencyKey: key, target: { kind: 'USER', userId: ticket.ownerUserId, organizationId: ticket.organizationId }, type: 'INFO', title: `工单提醒 ${ticket.number}`, content: `请查看工单 ${ticket.number} 的详情并处理。`, link: `/micro/work-orders/tickets/${ticket.id}` } })
    ensure(direct.publicationId === publication.publicationId && direct.duplicate === true, 'DIRECT_HTTP_REPLAY_MISMATCH')
    socket.disconnect()
    return { createdPublications: 1, recipients: 1, socketEventsCorrelated: 1, httpInboxRowsCorrelated: 1, duplicateReceipts: 2 }
  })
  await check('typed_v4_identity_input_rejected', async () => {
    const oldFixture = '11111111-1111-4111-8111-111111111111'
    await request('stores-reject-v4', `/stores-api/stores/${oldFixture}`, { token: storesToken, statuses: [400] })
    await request('orders-reject-v4', `/work-orders-api/tickets/${oldFixture}`, { token: ordersToken, statuses: [400] })
    return { rejectedV4Inputs: 2 }
  })
  await check('chat_two_real_clients_message_disconnect_and_rejoin', async () => {
    const a = await connect(chatOrigin, '/')
    const beforeJoin = listen(a, 'error', value => value?.code === 'not_joined')
    a.emit('message', { username: `${run}-a`, content: 'before join' })
    await beforeJoin
    const aUsers = listen(a, 'users-list', value => value?.users?.some(user => user.username === `${run}-a`))
    a.emit('join', { username: `${run}-a` }); await aUsers
    const b = await connect(chatOrigin, '/')
    const bUsers = listen(b, 'users-list', value => value?.users?.some(user => user.username === `${run}-b`))
    b.emit('join', { username: `${run}-b` }); await bUsers
    const message = `real cross-client ${run}`
    const onA = listen(a, 'message', value => value?.content === message)
    const onB = listen(b, 'message', value => value?.content === message)
    a.emit('message', { username: `${run}-a`, content: message })
    const [ma, mb] = await Promise.all([onA, onB])
    ensure(ma.id === mb.id && /^chat-[0-9]+$/.test(ma.id) && ma.type === 'user' && ma.username === `${run}-a`, 'CHAT_MESSAGE_CONTRACT_INVALID')
    const left = listen(a, 'user-left', value => value?.user?.username === `${run}-b`)
    b.disconnect(); await left
    const b2 = await connect(chatOrigin, '/')
    const users2 = listen(b2, 'users-list', value => value?.users?.some(user => user.username === `${run}-b`))
    b2.emit('join', { username: `${run}-b` }); await users2
    for (const client of [a, b2]) client.disconnect()
    return { realClients: 2, messagesSent: 1, matchingRecipients: 2, disconnectObserved: true, rejoinObserved: true, messageIdContract: 'opaque-chat-sequence-not-persistent-uuid' }
  })
  await check('independent_session_logout', async () => {
    for (const [index, session] of ownSessions.entries()) await request(`own-session-logout-${index}`, '/api/auth/logout', { method: 'POST', token: session.accessToken, body: {} })
    privateSessionEnded = true
    return { sharedSessionChanged: false }
  })
  report.status = report.limits.length ? 'PASS_WITH_AUTHORIZATION_LIMIT' : 'PASS'
} catch (error) {
  report.status = 'FAIL'
  report.failureStage = stage
  report.failureCode = /^[A-Z_0-9]{1,180}$/.test(error?.message || '') ? error.message : 'VALIDATION_OPERATION_FAILED'
  process.exitCode = 1
} finally {
  for (const socket of sockets) socket.disconnect()
  if (ownSessions.length && !privateSessionEnded) {
    try { for (const [index, session] of ownSessions.entries()) await request(`own-session-failure-cleanup-${index}`, '/api/auth/logout', { method: 'POST', token: session.accessToken, body: {} }); report.failureCleanupSessionLogout = true } catch { report.failureCleanupSessionLogout = false }
  }
  report.finishedAt = new Date().toISOString()
  report.scriptSha256 = createHash('sha256').update(await readFile(fileURLToPath(import.meta.url))).digest('hex')
  await save()
  console.log(JSON.stringify({ status: report.status, checksPassed: report.checks.length, httpRequests: report.http.length, ...(report.failureStage ? { failureStage: report.failureStage, failureCode: report.failureCode } : {}), sharedRefreshConsumed: false, fixtureServices: false }))
}
