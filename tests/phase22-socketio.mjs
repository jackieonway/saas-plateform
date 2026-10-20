import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import test from 'node:test'
import { isNotificationSocketContext, isRealtimeNotification } from '../frontend/src/lib/notification-protocol.ts'
import { createChatSocket } from '../examples/websocket/src/chat-client.ts'
import { allowedOrigin, event, observeSilence, startRustFixture, until } from './helpers/phase22-fixtures.mjs'

// Resolve the installed, lockfile-controlled product clients, rather than a synthetic protocol driver.
const frontendRequire = createRequire(new URL('../frontend/package.json', import.meta.url))
const { io } = frontendRequire('socket.io-client')
const WebSocket = frontendRequire('ws')
const identityA = { userId: 'user-a', tenantId: 'tenant-a', organizationId: 'org-a' }
const identityB = { userId: 'user-b', tenantId: 'tenant-b', organizationId: 'org-b' }
const sameUserOtherTenant = { userId: 'user-a', tenantId: 'tenant-b', organizationId: 'org-a' }
const sameUserOtherOrganization = { userId: 'user-a', tenantId: 'tenant-a', organizationId: 'org-b' }
const invalidSession = { code: 'SESSION_INVALID', message: '实时通知会话已失效' }

test('Phase 22: real notification Socket.IO client against the Rust candidate', { timeout: 360_000 }, async t => {
  const fixture = await startRustFixture({
    manifest: 'tests/fixtures/socketio-rust/Cargo.toml', binary: 'phase22-socketio-fixture',
    env: { PHASE22_PORT: '0', PHASE22_ORIGIN: allowedOrigin },
  })
  const sockets = new Set()
  const socket = (options = {}, namespace = '') => {
    const instance = io(fixture.base + namespace, {
      path: '/socket.io/', autoConnect: false, forceNew: true, withCredentials: false,
      transports: ['websocket', 'polling'], reconnection: false, reconnectionAttempts: 8, timeout: 1000,
      extraHeaders: { origin: allowedOrigin }, ...options,
    })
    sockets.add(instance)
    return instance
  }
  const closeSockets = () => {
    for (const instance of sockets) { instance.io.reconnection(false); instance.disconnect(); instance.io.disconnect(); instance.removeAllListeners() }
    sockets.clear()
  }
  t.afterEach(closeSockets)
  const connect = async instance => {
    const connected = event(instance, 'connect')
    instance.connect()
    await connected
  }
  const authenticate = async (instance, token = 'valid-a', identity = identityA) => {
    const authenticated = event(instance, 'authenticated')
    instance.emit('authenticate', { token })
    const [context] = await authenticated
    assert.equal(isNotificationSocketContext(context, identity), true, 'Current frontend accepts the exact authenticated context')
    return context
  }
  const send = async (recipient, id) => {
    const response = await fetch(fixture.base + '/__fixture__/notification', {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ recipient, id }),
      signal: AbortSignal.timeout(2000),
    })
    assert.equal(response.status, 204)
  }
  const rejectAuthentication = async (instance, payload, { emit = true } = {}) => {
    const unauthorized = event(instance, 'unauthorized'), disconnected = event(instance, 'disconnect')
    if (emit) instance.emit('authenticate', payload)
    assert.deepEqual((await unauthorized)[0], invalidSession)
    assert.equal((await disconnected)[0], 'io server disconnect')
    assert.equal(instance.active, false)
  }

  try {
    const health = await fetch(fixture.base + '/health/live', { signal: AbortSignal.timeout(2000) })
    assert.equal(health.status, 200)

    for (const mode of [
      { name: 'forced polling', transports: ['polling'], upgrade: false, transport: 'polling' },
      { name: 'forced WebSocket', transports: ['websocket'], transport: 'websocket' },
      { name: 'polling to WebSocket Upgrade', transports: ['polling', 'websocket'], transport: 'websocket' },
    ]) {
      await t.test(`${mode.name}: explicit authentication and contract 1.0 event`, { timeout: 5000 }, async () => {
        const instance = socket({ transports: mode.transports, upgrade: mode.upgrade ?? true })
        await connect(instance)
        await authenticate(instance)
        await until(() => instance.io.engine.transport.name === mode.transport, mode.name)
        const notification = event(instance, 'notification')
        await send('a', `transport-${mode.transport}-${mode.transports.length}`)
        const [payload] = await notification
        assert.equal(isRealtimeNotification(payload, identityA), true, 'Current frontend accepts the emitted notification')
        assert.equal(payload.id, `transport-${mode.transport}-${mode.transports.length}`)
        assert.equal(payload.title, 'Rust 通知 🦀')
        assert.equal(payload.applicationId, null)
        assert.equal(payload.publicationId, null)
        t.diagnostic(`${mode.name}: actual Engine.IO transport=${instance.io.engine.transport.name}`)
      })
    }

    await t.test('notification is unavailable before authenticate and isolated by user/tenant/organization', { timeout: 6000 }, async () => {
      const a = socket(), b = socket(), otherTenant = socket(), otherOrganization = socket(), silent = socket()
      await Promise.all([connect(a), connect(b), connect(otherTenant), connect(otherOrganization), connect(silent)])
      const silentEvents = []
      silent.on('notification', payload => silentEvents.push(payload))
      const beforeAuthentication = observeSilence(a, 'notification', 100)
      await send('a', 'before-authentication')
      await beforeAuthentication
      const peers = [
        { instance: a, token: 'valid-a', identity: identityA, recipient: 'a' },
        { instance: b, token: 'valid-b', identity: identityB, recipient: 'b' },
        { instance: otherTenant, token: 'same-user-tenant-b', identity: sameUserOtherTenant, recipient: 'tenant-b' },
        { instance: otherOrganization, token: 'same-user-org-b', identity: sameUserOtherOrganization, recipient: 'org-b' },
      ].map(peer => ({ ...peer, events: [] }))
      await Promise.all(peers.map(peer => authenticate(peer.instance, peer.token, peer.identity)))
      for (const peer of peers) peer.instance.on('notification', payload => peer.events.push(payload))
      for (const peer of peers) await send(peer.recipient, `private-${peer.recipient}`)
      await until(() => peers.every(peer => peer.events.length === 1), 'one event per exact user/tenant/organization identity')
      await observeSilence(silent, 'notification', 100)
      for (const peer of peers) {
        assert.deepEqual(peer.events.map(value => value.id), [`private-${peer.recipient}`])
        assert.equal(isRealtimeNotification(peer.events[0], peer.identity), true)
        for (const other of peers.filter(other => other !== peer)) assert.equal(isRealtimeNotification(peer.events[0], other.identity), false)
      }
      assert.deepEqual(silentEvents, [])
    })

    for (const [label, payload] of [
      ['null', null], ['scalar', 'valid-a'], ['array', ['valid-a']], ['missing token', {}],
      ['wrong token type', { token: 42 }], ['additional field', { token: 'valid-a', userId: 'user-b' }],
      ['blank token', { token: '  ' }], ['internal whitespace', { token: 'valid- a' }],
      ['control character', { token: 'valid-a\u007f' }],
      ['overlong token', { token: 'a'.repeat(5000) }], ['invalid token', { token: 'invalid' }],
      ['already expired token', { token: 'expired' }],
    ]) {
      await t.test(`${label}: unauthorized and server disconnect`, { timeout: 4000 }, async () => {
        const instance = socket()
        await connect(instance)
        await rejectAuthentication(instance, payload)
      })
    }

    await t.test('handshake auth does not replace the explicit authentication event; silent client times out', { timeout: 4000 }, async () => {
      const instance = socket({ auth: { token: 'valid-a' } })
      await connect(instance)
      await rejectAuthentication(instance, undefined, { emit: false })
    })

    await t.test('second authenticate rejects instead of replacing the established identity', { timeout: 4000 }, async () => {
      const instance = socket()
      await connect(instance)
      await authenticate(instance)
      await rejectAuthentication(instance, { token: 'valid-b' })
    })

    await t.test('concurrent authenticate events cannot replace identity or revive the disconnected session', { timeout: 4000 }, async () => {
      const instance = socket()
      await connect(instance)
      const contexts = []
      instance.on('authenticated', context => contexts.push(context))
      const unauthorized = event(instance, 'unauthorized'), disconnected = event(instance, 'disconnect')
      instance.emit('authenticate', { token: 'valid-a' })
      instance.emit('authenticate', { token: 'valid-b' })
      assert.deepEqual((await unauthorized)[0], invalidSession)
      assert.equal((await disconnected)[0], 'io server disconnect')
      assert.ok(contexts.every(context => isNotificationSocketContext(context, identityA)))
      assert.ok(contexts.length <= 1)
      const afterRejection = observeSilence(instance, 'notification')
      await send('a', 'after-concurrent-authentication')
      await afterRejection
    })

    for (const transport of ['websocket', 'polling']) {
      await t.test(`${transport}: authenticated expiresAt closes the session with the current unauthorized contract`, { timeout: 5000 }, async () => {
        const instance = socket({ transports: [transport] })
        await connect(instance)
        const context = await authenticate(instance, 'expiring')
        const expires = Date.parse(context.expiresAt)
        assert.ok(expires - Date.now() <= 1500)
        const unauthorized = event(instance, 'unauthorized'), disconnected = event(instance, 'disconnect')
        assert.deepEqual((await unauthorized)[0], invalidSession)
        assert.ok(Date.now() >= expires - 30, 'Expiry disconnect cannot precede the acknowledged lifetime')
        assert.equal((await disconnected)[0], 'io server disconnect')
        const afterExpiry = observeSilence(instance, 'notification')
        await send('a', 'after-expiry')
        await afterExpiry
      })

      await t.test(`${transport}: Engine.IO heartbeat remains live across multiple ping/pong exchanges`, { timeout: 5000 }, async () => {
        const instance = socket({ transports: [transport] })
        await connect(instance)
        const receivedPings = [], sentPongs = []
        instance.io.engine.on('packet', packet => { if (packet.type === 'ping') receivedPings.push(packet) })
        instance.io.engine.on('packetCreate', packet => { if (packet.type === 'pong') sentPongs.push(packet) })
        await authenticate(instance)
        await until(() => receivedPings.length >= 3 && sentPongs.length >= 3, 'at least three real heartbeat exchanges')
        assert.equal(instance.connected, true)
        const notification = event(instance, 'notification')
        await send('a', 'after-heartbeats')
        assert.equal((await notification)[0].id, 'after-heartbeats')
        t.diagnostic(`Engine.IO heartbeat: ${receivedPings.length} ping / ${sentPongs.length} pong packets`)
      })

      await t.test(`${transport}: transport loss reconnects and reauthenticates a new connection`, { timeout: 6000 }, async () => {
        const instance = socket({ transports: [transport], reconnection: true, reconnectionDelay: 50, reconnectionDelayMax: 100, randomizationFactor: 0 })
        const contexts = [], reasons = []
        instance.on('connect', () => instance.emit('authenticate', { token: 'valid-a' }))
        instance.on('authenticated', context => contexts.push(context))
        instance.on('disconnect', reason => reasons.push(reason))
        await connect(instance)
        await until(() => contexts.length === 1, 'initial authenticated connection')
        const original = instance.id
        instance.io.engine.close()
        await until(() => instance.connected && instance.id !== original && contexts.length === 2, 'reconnected and authenticated socket')
        assert.ok(reasons.some(reason => reason === 'forced close' || reason === 'transport close'))
        assert.ok(contexts.every(context => isNotificationSocketContext(context, identityA)))
        const notification = event(instance, 'notification')
        await send('a', 'after-reconnect')
        assert.equal((await notification)[0].id, 'after-reconnect')
      })

      await t.test(`${transport}: unauthorized server disconnect disables automatic reconnection even when reconnect is enabled`, { timeout: 4000 }, async () => {
        const instance = socket({ transports: [transport], reconnection: true, reconnectionDelay: 50, reconnectionDelayMax: 100, randomizationFactor: 0 })
        await connect(instance)
        await rejectAuthentication(instance, { token: 'invalid' })
        await Promise.all([observeSilence(instance, 'connect', 350), observeSilence(instance.io, 'reconnect_attempt', 350)])
        assert.equal(instance.active, false)
      })
    }

    for (const origin of [undefined, 'http://foreign.example', 'null', allowedOrigin + '.evil', 'http://127.0.0.1:3001', 'https://127.0.0.1:3000']) {
      await t.test(`reject ${origin ?? 'missing'} Origin for polling and direct WebSocket handshakes`, { timeout: 5000 }, async () => {
        const headers = origin === undefined ? {} : { origin }
        const polling = await fetch(fixture.base + '/socket.io/?EIO=4&transport=polling', { headers, signal: AbortSignal.timeout(2000) })
        assert.equal(polling.status, 403)
        const instance = socket({ transports: ['websocket'], extraHeaders: headers })
        const failed = event(instance, 'connect_error')
        instance.connect()
        const [error] = await failed
        assert.match(error.message, /websocket|transport|403/i)
        assert.equal(instance.connected, false)
      })
    }

    await t.test('a valid polling session cannot Upgrade using a foreign Origin', { timeout: 5000 }, async () => {
      const instance = socket({ transports: ['polling'], upgrade: false })
      await connect(instance)
      await authenticate(instance)
      const sid = instance.io.engine.id
      assert.equal(typeof sid, 'string')
      const raw = new WebSocket(fixture.base.replace('http:', 'ws:') + `/socket.io/?EIO=4&transport=websocket&sid=${encodeURIComponent(sid)}`, {
        headers: { origin: 'http://foreign.example' }, handshakeTimeout: 2000,
      })
      try {
        const rejected = await new Promise((resolve, reject) => {
          raw.on('open', () => reject(new Error('Foreign Origin Upgrade was accepted')))
          raw.on('unexpected-response', (_request, response) => { response.resume(); resolve(response.statusCode) })
          raw.on('error', reject)
        })
        assert.equal(rejected, 403)
      } finally { raw.terminate() }
      assert.equal(instance.connected, true)
      assert.equal(instance.io.engine.transport.name, 'polling')
      const notification = event(instance, 'notification')
      await send('a', 'after-rejected-upgrade')
      assert.equal((await notification)[0].id, 'after-rejected-upgrade')
    })

    await t.test('unsupported Socket.IO namespace returns connect_error', { timeout: 4000 }, async () => {
      const instance = socket({}, '/unsupported')
      const failed = event(instance, 'connect_error')
      instance.connect()
      assert.match((await failed)[0].message, /namespace/i)
      assert.equal(instance.connected, false)
    })
  } finally {
    closeSockets()
    await fixture.stop()
  }
})

test('Phase 22: current standalone chat client against the Rust reference', { timeout: 360_000 }, async t => {
  const fixture = await startRustFixture({
    manifest: 'examples/websocket/backend/Cargo.toml', binary: 'saas-chat-reference',
    env: { CHAT_BIND_ADDR: '127.0.0.1:0', CHAT_ALLOWED_ORIGINS: allowedOrigin }, target: 'phase22-chat',
  })
  const sockets = new Set()
  const socket = (origin = allowedOrigin) => {
    const instance = createChatSocket(fixture.base)
    // Node has no browser Origin. Supply only that browser header; retain the product factory's options.
    instance.io.opts.extraHeaders = { origin }
    sockets.add(instance)
    return instance
  }
  const closeSockets = () => {
    for (const instance of sockets) { instance.io.reconnection(false); instance.disconnect(); instance.io.disconnect(); instance.removeAllListeners() }
    sockets.clear()
  }
  t.afterEach(closeSockets)
  const connect = async instance => {
    const connected = event(instance, 'connect')
    instance.connect()
    await connected
  }
  const join = async (instance, username) => {
    const joined = event(instance, 'user-joined'), listed = event(instance, 'users-list')
    instance.emit('join', { username })
    const [membership] = await joined, [list] = await listed
    assert.deepEqual(membership.user, { id: instance.id, username })
    assert.equal(membership.message.type, 'system')
    assert.equal(membership.message.username, 'System')
    assert.ok(Number.isFinite(Date.parse(membership.message.timestamp)))
    assert.ok(list.users.some(user => user.id === instance.id && user.username === username))
    return list.users
  }
  const errorFor = async (instance, name, payload, expectedCode) => {
    const rejected = event(instance, 'error')
    instance.emit(name, payload)
    const [error] = await rejected
    assert.equal(error.code, expectedCode)
    assert.equal(typeof error.message, 'string')
    assert.equal(instance.connected, true)
  }
  try {
    await t.test('real factory connects on /; join/list/broadcast/leave preserve the declared chat events', { timeout: 7000 }, async () => {
      const a = socket(), b = socket()
      assert.equal(a.active, false)
      await Promise.all([connect(a), connect(b)])
      assert.equal(a.io.engine.transport.name, 'websocket')
      const aJoinAtB = event(b, 'user-joined')
      await join(a, 'Alice')
      assert.equal((await aJoinAtB)[0].user.id, a.id)
      const bJoinAtA = event(a, 'user-joined')
      const users = await join(b, '小明 🦀')
      assert.equal((await bJoinAtA)[0].user.id, b.id)
      assert.ok(users.some(user => user.id === a.id && user.username === 'Alice'))

      const atA = event(a, 'message'), atB = event(b, 'message')
      a.emit('message', { username: 'Alice', content: 'Rust 中文与 emoji 🦀 <b>plain content</b>' })
      const [messageA] = await atA, [messageB] = await atB
      assert.deepEqual(messageA, messageB)
      assert.equal(messageA.username, 'Alice')
      assert.equal(messageA.content, 'Rust 中文与 emoji 🦀 <b>plain content</b>')
      assert.equal(messageA.type, 'user')
      assert.equal(typeof messageA.id, 'string')
      assert.ok(Number.isFinite(Date.parse(messageA.timestamp)))

      const oldId = b.id, departed = event(a, 'user-left')
      b.disconnect()
      const [left] = await departed
      assert.deepEqual(left.user, { id: oldId, username: '小明 🦀' })
      assert.equal(left.message.type, 'system')
      assert.equal(left.message.username, 'System')
    })

    await t.test('joined connection owns the username and duplicate join is idempotent', { timeout: 7000 }, async () => {
      const a = socket(), observer = socket()
      await Promise.all([connect(a), connect(observer)])
      const beforeJoin = observeSilence(observer, 'message')
      await errorFor(a, 'message', { username: 'Alice', content: 'before join' }, 'not_joined')
      await beforeJoin
      const joinedAtObserver = event(observer, 'user-joined')
      await join(a, 'Alice')
      await joinedAtObserver
      const listed = event(a, 'users-list'), noDuplicate = observeSilence(observer, 'user-joined')
      a.emit('join', { username: 'Alice' })
      const [list] = await listed
      assert.equal(list.users.filter(user => user.id === a.id).length, 1)
      await noDuplicate
      const forgedMessage = observeSilence(observer, 'message')
      await errorFor(a, 'join', { username: 'Bob' }, 'username_mismatch')
      await errorFor(a, 'message', { username: 'Bob', content: 'forged username' }, 'username_mismatch')
      await forgedMessage
      const sent = event(observer, 'message')
      a.emit('message', { username: 'Alice', content: 'still the original identity' })
      assert.equal((await sent)[0].username, 'Alice')
    })

    await t.test('malformed and bounded chat inputs return explicit errors without breaking the connection', { timeout: 7000 }, async () => {
      const instance = socket()
      await connect(instance)
      for (const payload of [null, ['Alice'], { username: 42 }, { username: 'Alice', userId: 'forged' }]) {
        await errorFor(instance, 'join', payload, 'invalid_payload')
      }
      for (const username of ['', ' Alice ', '字'.repeat(65), 'A\nB']) {
        await errorFor(instance, 'join', { username }, 'invalid_username')
      }
      await join(instance, 'Alice')
      await errorFor(instance, 'message', ['Alice', 'hello'], 'invalid_payload')
      await errorFor(instance, 'message', { username: 'Alice', content: 42 }, 'invalid_payload')
      await errorFor(instance, 'message', { username: 'Alice', content: '字'.repeat(4097) }, 'invalid_message')
      await errorFor(instance, 'message', { username: 'Alice', content: '  ' }, 'invalid_message')
      const message = event(instance, 'message')
      instance.emit('message', { username: 'Alice', content: 'valid after rejection' })
      assert.equal((await message)[0].content, 'valid after rejection')
    })

    await t.test('transport reconnect creates a new chat membership that must join again', { timeout: 10_000 }, async () => {
      const instance = socket()
      await connect(instance)
      await join(instance, 'Alice')
      const original = instance.id
      instance.io.engine.close()
      await until(() => instance.connected && instance.id !== original, 'chat factory automatic reconnect', 6000)
      await errorFor(instance, 'message', { username: 'Alice', content: 'old membership is gone' }, 'not_joined')
      const users = await join(instance, 'Alice')
      assert.equal(users.some(user => user.id === original), false)
      const message = event(instance, 'message')
      instance.emit('message', { username: 'Alice', content: 'joined after reconnect' })
      assert.equal((await message)[0].content, 'joined after reconnect')
    })

    await t.test('declared legacy test extension preserves a JSON object and ISO timestamp', { timeout: 4000 }, async () => {
      const instance = socket()
      await connect(instance)
      const response = event(instance, 'test-response')
      instance.emit('test', { nested: ['中文', 42, false], message: 'opaque extension payload' })
      const [payload] = await response
      assert.equal(payload.message, 'Server received test message')
      assert.deepEqual(payload.data, { nested: ['中文', 42, false], message: 'opaque extension payload' })
      assert.ok(Number.isFinite(Date.parse(payload.timestamp)))
    })

    for (const origin of ['http://foreign.example', 'null']) {
      await t.test(`chat browser Origin ${origin} is rejected`, { timeout: 4000 }, async () => {
        const instance = socket(origin)
        // Keep the factory's transport order, while bounding rejection to the first attempt.
        instance.io.reconnection(false)
        const failed = event(instance, 'connect_error')
        instance.connect()
        assert.match((await failed)[0].message, /websocket|transport|403/i)
        assert.equal(instance.connected, false)
      })
    }
  } finally {
    closeSockets()
    await fixture.stop()
  }
})
