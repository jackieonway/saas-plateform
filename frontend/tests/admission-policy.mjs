import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const id = value => `01962345-6789-7abc-8def-${value.toString(16).padStart(12, '0')}`
const target = { kind: 'application', id: id(1), name: 'Customer portal' }
const policy = (patch = {}) => ({ id: target.id, status: 'ENABLED', allowedMemberTypes: ['EMPLOYEE'], accessPolicyVersion: 3, ...patch })
const plain = value => JSON.parse(JSON.stringify(value))
class ApiError extends Error { constructor(status) { super('concurrent policy update'); this.status = status } }

function moduleFrom(relative, dependencies) {
  const source = fs.readFileSync(path.join(root, relative), 'utf8')
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX } }).outputText
  const exports = {}
  const sandbox = vm.createContext({ DOMException, AbortController, Promise, Set, Map, console })
  new vm.Script(`(function(require,exports){${compiled}\n})`, { filename: relative }).runInContext(sandbox)(dependencies, exports)
  return exports
}

function apiFixture() {
  let response = policy(), put = async () => response
  const reads = [], writes = []
  const module = moduleFrom('src/api/admission-policies.ts', name => {
    if (name === '@/lib/uuid') return { isUuidV7: value => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value) }
    if (name === '@/lib/api') return { api: {
      get: async (url, options) => { reads.push({ url, options }); return response },
      put: async (url, body) => { writes.push({ url, body }); return put() },
    } }
    throw new Error(`unexpected API dependency ${name}`)
  })
  return { ...module, reads, writes, response: value => { response = value }, put: handler => { put = handler } }
}

test('application policy reads bind the exact owner and preserve cancellation', async () => {
  const f = apiFixture(), controller = new AbortController()
  await f.admissionPoliciesApi.read(target, controller.signal)
  assert.equal(f.reads[0].url, `/api/applications/${target.id}/access-policy`)
  assert.equal(f.reads[0].options.signal, controller.signal)
  await assert.rejects(f.admissionPoliciesApi.read({ ...target, id: 'old-owner' }))
  await assert.rejects(f.admissionPoliciesApi.read({ ...target, kind: 'client' }))
  assert.equal(f.reads.length, 1)
})

test('policy response rejects unknown identities, incoherent purposes, wrong owners and unsafe versions', () => {
  const f = apiFixture()
  for (const patch of [{ id: id(2) }, { status: 'UNKNOWN' }, { subjectMode: 'UNKNOWN' },
    { allowedMemberTypes: [] }, { allowedMemberTypes: ['EMPLOYEE', 'STAFF'] }, { allowedMemberTypes: ['EMPLOYEE', 'EMPLOYEE'] },
    { subjectMode: 'MACHINE' }, { accessPolicyVersion: 0 }, { accessPolicyVersion: -1 }, { accessPolicyVersion: '3' },
    { accessPolicyVersion: 1.5 }, { accessPolicyVersion: Number.MAX_SAFE_INTEGER + 1 }]) {
    assert.throws(() => f.parseAdmissionPolicy(policy(patch), target))
  }

})

test('policy body keeps the old numeric version, trims real reason and canonicalizes only supported types', () => {
  const f = apiFixture()
  assert.deepEqual(plain(f.admissionPolicyBody({ allowedMemberTypes: ['CUSTOMER', 'EMPLOYEE', 'CUSTOMER'], accessPolicyVersion: 3, reason: '  Open the registered customer entry  ' })), {
    allowedMemberTypes: ['EMPLOYEE', 'CUSTOMER'], accessPolicyVersion: 3, reason: 'Open the registered customer entry',
  })
  for (const accessPolicyVersion of ['3', 0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
    assert.throws(() => f.admissionPolicyBody({ allowedMemberTypes: ['EMPLOYEE'], accessPolicyVersion, reason: 'Reviewed change' }))
  }
  for (const reason of ['', '   ', 'x'.repeat(501), 'embedded\ncontrol', 'embedded\u0000control', 'embedded\u007fcontrol', 'embedded\u0085control']) {
    assert.equal(f.isAdmissionReasonValid(reason), false)
    assert.throws(() => f.admissionPolicyBody({ allowedMemberTypes: ['EMPLOYEE'], accessPolicyVersion: 3, reason }))
  }
  assert.equal(f.isAdmissionReasonValid('😀'.repeat(500)), true)
  assert.throws(() => f.admissionPolicyBody({ allowedMemberTypes: ['MACHINE'], accessPolicyVersion: 3, reason: 'Reviewed change' }))
})

test('a policy 409 is surfaced after one PUT without refreshing or rewriting its old version', async () => {
  const f = apiFixture()
  f.put(async () => { throw new ApiError(409) })
  await assert.rejects(f.admissionPoliciesApi.save(target, { allowedMemberTypes: ['CUSTOMER'], accessPolicyVersion: 3, reason: 'Reviewed customer entry' }), error => error.status === 409)
  assert.equal(f.writes.length, 1)
  assert.equal(f.reads.length, 0)
  assert.deepEqual(plain(f.writes[0]), { url: `/api/applications/${target.id}/access-policy`, body: {
    allowedMemberTypes: ['CUSTOMER'], accessPolicyVersion: 3, reason: 'Reviewed customer entry',
  } })
})

function dialogFixture(initial = policy()) {
  const api = apiFixture(); api.response(initial)
  let next = 0, effects = [], current = true, mutation, queryOptions, pending = Promise.resolve(), isPending = false
  const slots = [], events = []
  const session = { token: 'platform-session', namespace: 'complete-member-proof', epoch: 7, profileReady: true }
  const fence = { sessionId: id(9), version: 3, contextKey: session.namespace }
  const useSlot = initialize => {
    const index = next++
    if (!Object.hasOwn(slots, index)) slots[index] = initialize()
    return [slots[index], value => { slots[index] = typeof value === 'function' ? value(slots[index]) : value }]
  }
  const hooks = {
    useState: initial => useSlot(() => typeof initial === 'function' ? initial() : initial),
    useRef: initial => useSlot(() => ({ current: initial }))[0],
    useCallback: callback => callback,
    useEffect: callback => { effects.push(callback) },
  }
  const jsx = (type, props) => ({ type, props })
  const query = { data: initial, isFetching: false, isError: false,
    refetch: async () => ({ data: await queryOptions.queryFn({ signal: new AbortController().signal }) }) }
  const module = moduleFrom('src/components/admission-policy-dialog.tsx', name => {
    if (name === 'react') return hooks
    if (name === 'react/jsx-runtime') return { jsx, jsxs: jsx, Fragment: 'Fragment' }
    if (name === '@tanstack/react-query') return {
      useQuery: options => { queryOptions = options; return query },
      useQueryClient: () => ({ setQueryData: (...args) => events.push({ kind: 'cache', args }) }),
      useMutation: options => {
        mutation = options
        return { isPending, mutate: change => {
          isPending = true
          pending = (async () => {
            try { await options.onSuccess(await options.mutationFn(change)) }
            catch (error) { await options.onError(error) }
            finally { isPending = false; options.onSettled() }
          })()
        } }
      },
    }
    if (name === '@/api/admission-policies') return api
    if (name === '@/lib/api') return { ApiError }
    if (name === '@/lib/auth-queries') return { captureAuthSession: () => session, isCurrentAuthSession: () => current }
    if (name === '@/stores/auth') return { captureSessionFence: () => fence, useAuthStore: selector => selector({ contextKey: session.namespace, contextEpoch: session.epoch }) }
    if (name === 'sonner') return { toast: { success: () => events.push({ kind: 'toast' }) } }
    if (name === 'lucide-react' || name.startsWith('@/components/ui/')) return new Proxy({}, { get: (_, key) => key })
    throw new Error(`unexpected dialog dependency ${name}`)
  })
  const render = () => {
    next = 0; effects = []
    const tree = module.AdmissionPolicyDialog({ target, onClose: () => events.push({ kind: 'close' }), onSaved: value => events.push({ kind: 'retire', fence: value }) })
    for (const effect of effects) effect()
    return tree
  }
  const nodes = (tree, type) => {
    if (Array.isArray(tree)) return tree.flatMap(node => nodes(node, type))
    if (!tree || typeof tree !== 'object') return []
    return [...(tree.type === type ? [tree] : []), ...nodes(tree.props?.children, type)]
  }
  const text = tree => Array.isArray(tree) ? tree.map(text).join('') : tree && typeof tree === 'object' ? text(tree.props?.children) : typeof tree === 'string' ? tree : ''
  const edit = (tree, reason) => {
    nodes(tree, 'Checkbox').find(node => node.props['aria-label'] === '允许客户身份').props.onCheckedChange(true)
    nodes(tree, 'Textarea')[0].props.onChange({ target: { value: reason } })
    nodes(tree, 'Checkbox').find(node => !node.props['aria-label']).props.onCheckedChange(true)
  }
  const saveButton = tree => nodes(tree, 'Button').find(node => text(node).includes('保存准入策略'))
  render()
  return { api, events, fence, render, nodes, text, edit, saveButton, pending: () => pending, mutation: () => mutation, current: value => { current = value } }
}

test('dialog 409 reloads latest values and discards old reason and impact confirmation before any retry', async () => {
  const f = dialogFixture()
  let tree = f.render()
  f.edit(tree, 'Enable the reviewed customer portal')
  tree = f.render(); assert.equal(f.saveButton(tree).props.disabled, false)
  f.api.response(policy({ accessPolicyVersion: 4 }))
  f.api.put(async () => { throw new ApiError(409) })
  f.saveButton(tree).props.onClick(); await f.pending()
  tree = f.render()
  assert.equal(f.api.writes.length, 1)
  assert.equal(f.api.writes[0].body.accessPolicyVersion, 3)
  assert.equal(f.api.reads.length, 1)
  assert.ok(f.text(tree).includes('v4'))
  assert.equal(f.nodes(tree, 'Textarea')[0].props.value, '')
  assert.equal(f.nodes(tree, 'Checkbox').find(node => !node.props['aria-label']).props.checked, false)
  assert.equal(f.saveButton(tree).props.disabled, true)
  assert.equal(f.events.some(event => event.kind === 'retire' || event.kind === 'close'), false)
  assert.equal(f.mutation().retry, false)
})

test('successful policy save closes its reviewed view then retires projections using the captured fence', async () => {
  const f = dialogFixture()
  f.edit(f.render(), 'Enable the reviewed customer portal')
  const tree = f.render()
  f.api.put(async () => policy({ allowedMemberTypes: ['EMPLOYEE', 'CUSTOMER'], accessPolicyVersion: 4 }))
  f.saveButton(tree).props.onClick(); await f.pending()
  assert.deepEqual(f.events.filter(event => event.kind !== 'cache').map(event => event.kind), ['toast', 'close', 'retire'])
  assert.equal(f.events.find(event => event.kind === 'retire').fence, f.fence)
  assert.equal(f.api.writes.length, 1)
})
