import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import vm from 'node:vm'
import path from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'package.json')), ts = require('typescript')
const id = number => `01962345-6789-7abc-8def-${String(number).padStart(12, '0')}`
const identity = (patch = {}) => ({ userId: id(1), tenantId: id(2), organizationId: id(3), membershipId: id(4),
  memberType: 'EMPLOYEE', identityVersion: '1', accountVersion: '0', ...patch })

function fixture() {
  const storage = new Map([['platform-recent-visits', JSON.stringify([{ path: '/legacy', title: 'Old owner', icon: 'Page' }])]])
  const state = { token: 'server-session', contextSwitching: false, identity: null, contextKey: null, profile: null }
  const cleanups = [], modules = new Map()
  const store = selector => selector(state)
  store.getState = () => state
  store.subscribe = () => () => {}
  const sandbox = vm.createContext({ Event, localStorage: { getItem: key => storage.get(key) ?? null,
    setItem: (key, value) => storage.set(key, value) }, window: { dispatchEvent() {}, addEventListener() {}, removeEventListener() {} } })
  function load(relative) {
    if (modules.has(relative)) return modules.get(relative)
    const compiled = ts.transpileModule(fs.readFileSync(path.join(root, relative), 'utf8'), {
      compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
    }).outputText
    const exports = {}; modules.set(relative, exports)
    new vm.Script(`(function(require,exports){${compiled}\n})`).runInContext(sandbox)(name => {
      if (name === 'react') return { useState: factory => [factory(), () => {}], useEffect: callback => cleanups.push(callback()) }
      if (name === '@/stores/auth') return { useAuthStore: store, reconcileSessionSync() {} }
      if (name.startsWith('@/')) return load(`src/${name.slice(2)}.ts`)
      return require(name)
    }, exports)
    return exports
  }
  const helpers = load('src/lib/context-identity.ts'), visits = load('src/hooks/use-recent-visits.ts')
  function select(proof) {
    state.identity = proof; state.contextKey = helpers.contextIdentityKey(proof)
    state.profile = { user: { id: proof.userId }, tenant: { id: proof.tenantId }, organization: { id: proof.organizationId },
      membershipId: proof.membershipId, memberType: proof.memberType, identityVersion: proof.identityVersion, accountVersion: proof.accountVersion }
  }
  return { state, storage, visits, select, dispose: () => cleanups.forEach(cleanup => cleanup?.()) }
}

test('recent pages never import old global preferences and isolate same-organization identities and generations', () => {
  const f = fixture(); f.select(identity())
  assert.equal(f.visits.useRecentVisits().length, 0)
  f.visits.pushRecentVisit({ path: '/employee', title: 'Employee page' })
  assert.equal(f.visits.useRecentVisits()[0].path, '/employee')
  for (const patch of [{ membershipId: id(5), memberType: 'CUSTOMER' }, { identityVersion: '2' }, { accountVersion: '1' }]) {
    f.select(identity(patch))
    assert.equal(f.visits.useRecentVisits().length, 0)
    f.visits.pushRecentVisit({ path: '/separate', title: 'Separate scope' })
    assert.equal(f.visits.useRecentVisits()[0].path, '/separate')
  }
  f.select(identity())
  assert.equal(f.visits.useRecentVisits()[0].path, '/employee')
  assert.equal(f.storage.size, 5)
  f.dispose()
})

test('missing or mismatched member proof and switching state cannot read or write recent pages', () => {
  const f = fixture(); f.select(identity())
  f.visits.pushRecentVisit({ path: '/allowed', title: 'Allowed page' })
  for (const patch of [{ contextSwitching: true }, { profile: { ...f.state.profile, membershipId: id(5) } }, { identity: null }]) {
    const prior = { ...f.state }; Object.assign(f.state, patch)
    const before = [...f.storage.entries()]
    f.visits.pushRecentVisit({ path: '/blocked', title: 'Must not persist' })
    assert.equal(f.visits.useRecentVisits().length, 0)
    assert.deepEqual([...f.storage.entries()], before)
    Object.assign(f.state, prior)
  }
  f.dispose()
})
