import fs from 'node:fs'
import vm from 'node:vm'
import path from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
export const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r }); return { promise, resolve } }
export const json = (status, body) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })

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
      setTimeout: settings.controlDeadline ? (callback, ms) => { const handle = ++id; timers.set(handle, { callback, ms }); return handle } : setTimeout,
      clearTimeout: settings.controlDeadline ? handle => timers.delete(handle) : clearTimeout,
      crypto: { randomUUID: () => `test-session-${++id}` },
      navigator: settings.noLocks ? {} : { locks }, localStorage: storageView, window,
      document: { visibilityState: 'visible', addEventListener: add, removeEventListener: remove },
      fetch: async (url, config) => { calls.push({ url, options: config }); return handler(url, config) },
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
      event: kind => listeners.get(kind)?.forEach(fn => fn({ key: 'platform-session-v1' })),
      record: () => JSON.parse(storage.get('platform-session-v1')),
      denyWrites: () => { denyWrites = true },
    }
  }
  return { storage, writes, notifications, page }
}

export async function fixture(handler, settings = {}) {
  const shared = sharedOrigin(settings)
  const p = await shared.page(handler, settings)
  if (settings.session !== false) await p.state().setSession('expired-access', 'psr_test-refresh')
  return { ...p, shared }
}
