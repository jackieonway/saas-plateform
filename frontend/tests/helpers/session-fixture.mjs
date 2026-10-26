import fs from 'node:fs'
import { webcrypto } from 'node:crypto'
import vm from 'node:vm'
import path from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
export const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r }); return { promise, resolve } }
export const json = (status, body) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })
export const SESSION_KEY = 'platform-session-v2'
export const identity = (patch = {}) => ({
  userId: '019d0000-0000-7000-8000-000000000001', tenantId: '019d0000-0000-7000-8000-000000000002',
  organizationId: '019d0000-0000-7000-8000-000000000003', membershipId: '019d0000-0000-7000-8000-000000000004',
  memberType: 'EMPLOYEE', identityVersion: '1', accountVersion: '0', ...patch,
})
export const targetIdentity = identity({ organizationId: '019d0000-0000-7000-8000-000000000005', membershipId: '019d0000-0000-7000-8000-000000000006' })
export const contextResponse = (accessToken, refreshToken, member = targetIdentity) => ({
  accessToken, ...(refreshToken === undefined ? {} : { refreshToken }),
  user: { id: member.userId, username: 'session-fixture', realName: 'Fixture', email: '', phone: '', avatar: '', gender: '', isSuperAdmin: false },
  tenant: { id: member.tenantId, name: 'Fixture tenant', code: 'fixture' },
  organization: { id: member.organizationId, name: 'Fixture organization', path: '/Fixture' }, applications: [],
  membershipId: member.membershipId, memberType: member.memberType,
  identityVersion: member.identityVersion, accountVersion: member.accountVersion,
})

/** A FIFO exclusive lock, shared durable storage, and independently evaluated
 * modules model separate pages without pretending that missing browser APIs work. */
export function sharedOrigin(options = {}) {
  const storage = new Map(options.initial ?? [])
  const pages = [], writes = [], notifications = [], tails = new Map()
  let id = 0
  const locks = { request(name, callback) {
    if (options.lockRejected) return Promise.reject(new Error('locks denied'))
    const previous = tails.get(name) ?? Promise.resolve()
    const work = previous.catch(() => {}).then(callback)
    tails.set(name, work.catch(() => {}))
    return work
  } }
  async function page(handler, settings = {}) {
    const redirects = [], calls = [], listeners = new Map(), timers = new Map()
    const p = { listeners, disposed: false }
    let denyWrites = false
    let proof = settings.session === false ? null : identity(), serial = 1, access = 'expired-access'
    const sid = () => `019d0000-0000-7000-8000-${String(serial).padStart(12, '0')}`
    const account = () => ({ id: proof?.userId ?? identity().userId, username: 'session-fixture', realName: 'Fixture', avatar: '' })
    const member = p => ({ ...p, status: 'ENABLED', consoleSelectable: true, selectionDisabledReason: null,
      tenantName: 'Fixture tenant', organizationName: 'Fixture organization', organizationPath: '/Fixture', departmentName: null })
    const session = () => ({ sessionId: sid(), version: '1', csrfToken: 'a'.repeat(64), account: account(),
      memberships: proof ? [member(proof), member(targetIdentity)] : [], context: proof, expiresAt: new Date(Date.now()+600000).toISOString() })

    const add = (kind, callback) => { if (!listeners.has(kind)) listeners.set(kind, new Set()); listeners.get(kind).add(callback) }
    const remove = (kind, callback) => listeners.get(kind)?.delete(callback)
    const storageView = {
      getItem: key => { if (settings.storageUnavailable) throw new Error('storage disabled'); return storage.get(key) ?? null },
      setItem: (key, value) => {
        if (settings.storageUnavailable || denyWrites) throw new Error('storage disabled')
        storage.set(key, value); writes.push({ key, value })
        if (options.events !== false) for (const other of pages) if (other !== p) queueMicrotask(() => other.listeners.get('storage')?.forEach(fn => fn({ key })))
      },
      removeItem: key => { if (settings.storageUnavailable) throw new Error('storage disabled'); storage.delete(key) },
    }
    const window = { localStorage: storageView, addEventListener: add, removeEventListener: remove,
      location: { pathname: '/system/users', search: '', assign: value => redirects.push(value) } }
    const sandbox = vm.createContext({ Response, URLSearchParams, DOMException, AbortController, AbortSignal, Promise, console,
      setTimeout: settings.controlDeadline ? (callback, ms) => { const handle = ++id; timers.set(handle, { callback, ms }); return handle } : (fn, ms) => { const handle = setTimeout(fn, ms); handle.unref?.(); return handle },
      clearTimeout: settings.controlDeadline ? handle => timers.delete(handle) : clearTimeout,
      crypto: webcrypto,
      navigator: settings.noLocks ? {} : { locks }, localStorage: storageView, window,
      document: { visibilityState: 'visible', addEventListener: add, removeEventListener: remove },
      fetch: async (url, config) => {
        // Legacy fixtures explicitly select their server mode without changing
        // the request counts and controlled handlers owned by each scenario.
        if (url === '/auth/config' && (config?.method ?? 'GET') === 'GET') return json(200, { mode: 'BFF' })
        if (!settings.authMode && url === '/auth/session') return json(200, session())
        if (!settings.authMode && url === '/auth/context-token') return json(200, { ...contextResponse(access, undefined, proof),
          sessionId: sid(), version: '1', expiresIn: 300 })
        calls.push({ url, options: config })
        return handler(url, config)
      },
    })
    const cache = new Map()
    function load(relative) {
      if (cache.has(relative)) return cache.get(relative)
      const source = fs.readFileSync(path.join(settings.sourceRoot ?? root, relative), 'utf8')
      const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
      const exports = {}; cache.set(relative, exports)
      const wrapper = new vm.Script(`(function(require,exports){${compiled}\n})`, { filename: relative }).runInContext(sandbox)
      wrapper(name => name.startsWith('@/') ? load(`src/${name.slice(2)}.ts`) : require(name), exports)
      return exports
    }
    pages.push(p)
    const auth = load('src/stores/auth.ts'), api = load('src/lib/api.ts')
    const state = () => auth.authStore.getState()
    if (settings.initialize !== false && state().initializeSessionCoordination) await state().initializeSessionCoordination()
    return { ...auth, api, state, storage, redirects, calls, load,
      fireDeadline: () => { for (const timer of [...timers.values()]) { if (timer.ms === 20_000) timer.callback() } },
      event: kind => listeners.get(kind)?.forEach(fn => fn({ key: SESSION_KEY })),
      record: () => load('src/lib/bff-session.ts').bffSessionCoordinator.current().record,
      authenticate: async (token, _ignored, _expected, next = identity()) => {
        proof = next; serial++; access = token; await load('src/lib/bff-session.ts').bffSessionCoordinator.reconcile(); return true
      },
      denyWrites: () => { denyWrites = true },
    }
  }
  return { storage, writes, notifications, page }
}

export async function fixture(handler, settings = {}) {
  const shared = sharedOrigin(settings)
  const p = await shared.page(handler, settings)
  return { ...p, shared }
}
