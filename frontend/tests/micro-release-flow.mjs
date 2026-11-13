import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { elements, success } from './helpers/component-fixture.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const jsx = (type, props, key) => ({ type, props, key })
const symbols = new Proxy({}, { get: (_object, name) => name === '__esModule' ? true : String(name) })
const app = { id: 'app-a', code: 'sample', name: '示例应用', status: 'ENABLED', entryType: 'HTML', runtimeMode: 'TRUSTED' }
const release = { id: 'release-a', applicationId: app.id, environment: 'development', version: '1.0.0',
  entryType: 'HTML', entryUrl: '/sub-apps/sample/versions/1.0.0/index.html', resources: [],
  validationStatus: 'PASSED', checkedAt: '2026-10-22T00:00:00Z', createdAt: '2026-10-22T00:00:00Z', active: false }
const props = (changes = {}) => ({ application: app, environment: 'development', open: true, onOpenChange() {}, ...changes })
const text = value => typeof value === 'string' || typeof value === 'number' ? String(value)
  : Array.isArray(value) ? value.map(text).join('') : value && typeof value === 'object' ? text(value.props?.children) : ''
const findButton = (tree, label) => elements(tree, item => item.type === 'Button' && text(item).includes(label))[0]
const byLabel = (tree, label) => elements(tree, item => item.props?.['aria-label'] === label)

/** Exercise real release callbacks with stable refs and explicit query snapshots. */
function fixture(relative = 'ApplicationReleasesPanel.tsx', names = ['ApplicationReleasesPanelContent'], apiOverrides = {}) {
  const state = [], refs = [], dependencies = [], effects = [], forms = [], mutations = [], calls = [], toasts = []
  let stateIndex = 0, refIndex = 0, effectIndex = 0, formIndex = 0, mutationIndex = 0, dirty = false
  const queries = new Map([['application-releases', success([release])], ['application-deployments', success([])], ['application-url-entry', success(null)]])
  const react = {
    useState(initial) {
      const index = stateIndex++
      if (!(index in state)) state[index] = typeof initial === 'function' ? initial() : initial
      return [state[index], value => {
        const next = typeof value === 'function' ? value(state[index]) : value
        if (!Object.is(next, state[index])) { state[index] = next; dirty = true }
      }]
    },
    useRef(initial) { return refs[refIndex++] ??= { current: initial } },
    useMemo: callback => callback(),
    useCallback: callback => callback,
    useEffect(callback, values) {
      const index = effectIndex++, previous = dependencies[index]
      if (!previous || values.some((value, i) => !Object.is(value, previous[i]))) {
        dependencies[index] = values
        effects.push(callback)
      }
    },
  }
  const client = { invalidateQueries() { return Promise.resolve() }, setQueryData() {} }
  const api = new Proxy({}, { get: (_object, method) => async (...args) => {
    calls.push({ method, args })
    if (Object.hasOwn(apiOverrides, method)) return apiOverrides[method](...args)
    return method === 'activateRelease' || method === 'rollbackDeployment'
      ? { ...release, applicationId: app.id, activeReleaseId: release.id } : release
  } })
  const deps = {
    react,
    'react/jsx-runtime': { jsx, jsxs: jsx },
    '@tanstack/react-query': {
      useQuery: options => queries.get(options.queryKey[0]) ?? success([]),
      useQueryClient: () => client,
      useMutation(options) {
        const entry = mutations[mutationIndex++] ??= { isPending: false }
        entry.options = options
        entry.mutate = variables => {
          entry.variables = variables
          entry.isPending = true
          entry.promise = Promise.resolve().then(() => options.mutationFn(variables)).then(async result => {
            entry.isPending = false
            await options.onSuccess?.(result, variables)
            await options.onSettled?.(result, null, variables)
          })
        }
        return entry
      },
    },
    'react-hook-form': {
      useForm(options) {
        return forms[formIndex++] ??= { values: { ...options.defaultValues }, control: {}, formState: { errors: {} },
          handleSubmit(callback) { return values => callback(values ?? this.values) },
          reset(values) { this.values = values },
        }
      },
      useFieldArray: () => ({ fields: [], append() {}, remove() {} }),
    },
    '@/api/applications': { applicationsApi: api },
    '@/lib/form': { zResolver: () => ({}) },
    '@/lib/utils': { cn: (...values) => values.filter(Boolean).join(' ') },
    '@/hooks/use-auth-data': { usePermissionsQuery: () => queries.get('permissions') ?? success({ permissions: ['system:distribution:read', 'system:deployment:read'] }) },
    sonner: { toast: { success: (...values) => toasts.push(values), error: (...values) => toasts.push(values) } },
    './ExternalReleaseForm': { ExternalReleaseForm: 'ExternalReleaseForm' },
    './ApplicationUrlEntryForm': { ApplicationUrlEntryForm: 'ApplicationUrlEntryForm' },
    './application-setup-policy': { isQiankunApp: app => app.entryType === 'HTML' && app.runtimeMode === 'TRUSTED' },
    './ZipDistributionPanel': { ZipDistributionPanel: 'ZipDistributionPanel' },
    './ManagedDeploymentPanel': { ManagedDeploymentPanel: 'ManagedDeploymentPanel' },
    './external-release-policy': { externalReleaseSchema: {}, externalReleasePayload: (_environment, value) => value },
  }
  const source = readFileSync(path.join(root, 'src/pages/system/applications', relative), 'utf8') + `\nexport { ${names.join(', ')} };`
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS,
    target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX } }).outputText
  const exports = {}
  vm.runInThisContext(`(function(require,exports){${compiled}\n})`)(name => {
    if (name in deps) return deps[name]
    if (name.startsWith('@/') || name === 'lucide-react') return symbols
    return require(name)
  }, exports)
  return { queries, calls, toasts, forms, mutations,
    render(values = props(), name = names[0]) {
      let tree
      for (let pass = 0; pass < 10; pass++) {
        stateIndex = 0; refIndex = 0; effectIndex = 0; formIndex = 0; mutationIndex = 0; dirty = false
        tree = exports[name](values)
        while (effects.length) effects.shift()()
        if (!dirty) return tree
      }
      throw new Error('Release component did not settle')
    },
  }
}

test('release editor shows direct URL entry and optional version management without upload or environment controls', () => {
  const embedded = fixture().render(props({ embedded: true }))
  assert.equal(elements(embedded, item => item.type === 'Sheet').length, 0)
  assert.equal(elements(embedded, item => item.type === 'ApplicationUrlEntryForm').length, 1)
  assert.equal(byLabel(embedded, '上传新版本').length, 0)
  assert.equal(elements(embedded, item => item.type === 'Select').length, 0)
  const versions = byLabel(embedded, '版本管理（可选）')[0]
  assert.ok(versions)
  assert.ok(!versions.props.open)
  assert.equal(elements(versions, item => item.type === 'ExternalReleaseForm').length, 1)
  assert.equal(elements(fixture().render(), item => item.type === 'Sheet').length, 1)
})

test('a direct URL overrides the displayed deployment while the retained version can be reactivated', () => {
  const f = fixture()
  f.queries.set('application-url-entry', success({ applicationId: app.id, environment: 'development',
    entryUrl: 'http://localhost:3100/', proxyEnabled: true, browserEntryUrl: '/micro-proxy/sample/', updatedAt: '' }))
  f.queries.set('application-deployments', success([{ ...release, activeReleaseId: release.id }]))
  const tree = f.render()
  assert.equal(byLabel(tree, '当前环境部署').length, 0)
  assert.equal(findButton(tree, '激活').props.disabled, false)
  const direct = elements(tree, item => item.type === 'ApplicationUrlEntryForm')[0]
  assert.equal(direct.props.ready, true)
  assert.equal(direct.props.entry.proxyEnabled, true)
})

test('failed or mismatched direct entry reads cannot make an editable current entry', () => {
  for (const snapshot of [{ isSuccess: false, isError: true, error: new Error('unavailable') },
    success({ applicationId: app.id, environment: 'production', entryUrl: 'https://wrong.example/' })]) {
    const f = fixture()
    f.queries.set('application-url-entry', snapshot)
    const tree = f.render()
    const direct = elements(tree, item => item.type === 'ApplicationUrlEntryForm')[0]
    assert.equal(direct.props.ready, false)
    assert.equal(direct.props.entry, null)
  }
})

test('saving a direct URL blocks version operations in the enclosing release editor', () => {
  const f = fixture()
  let tree = f.render()
  elements(tree, item => item.type === 'ApplicationUrlEntryForm')[0].props.onBusyChange(true)
  tree = f.render()
  assert.equal(findButton(tree, '激活').props.disabled, true)
  assert.equal(elements(tree, item => item.type === 'ExternalReleaseForm')[0].props.busy, true)
})

test('publishing requires the explicit confirmation action before activating a validated release', async () => {
  const f = fixture()
  let tree = f.render()
  assert.equal(f.calls.length, 0)
  findButton(tree, '激活').props.onClick()
  tree = f.render()
  assert.equal(f.calls.length, 0)
  const confirm = elements(tree, item => item.type === 'AlertDialogAction' && text(item).includes('确认激活'))[0]
  confirm.props.onClick({ preventDefault() {} })
  await f.mutations[2].promise
  assert.deepEqual(f.calls, [{ method: 'activateRelease', args: [app.id, release.id] }])
})

test('failed validation cannot enable activation', () => {
  const f = fixture()
  f.queries.set('application-releases', success([{ ...release, validationStatus: 'FAILED' }]))
  const tree = f.render()
  assert.equal(findButton(tree, '激活').props.disabled, true)
  assert.equal(f.calls.length, 0)
})

test('successful current deployment notifies the wizard but a result from another environment cannot', async () => {
  const f = fixture()
  let changes = 0
  const values = props({ onDeploymentChanged() { changes++ } })
  let tree = f.render(values)
  const request = { kind: 'activate', target: { applicationId: app.id, environment: 'development' }, release }
  await f.mutations[2].options.onSuccess({ version: release.version, environment: 'development' }, request)
  assert.equal(changes, 1)
  tree = f.render({ ...values, environment: 'production' })
  assert.equal(elements(tree, item => item.type === 'ApplicationUrlEntryForm')[0].props.environment, 'production')
  await f.mutations[2].options.onSuccess({ version: release.version, environment: 'development' }, request)
  assert.equal(changes, 1)
})

test('parent-controlled environment resets drafts and fences an old activation confirmation', () => {
  const f = fixture()
  let tree = f.render(props({ embedded: true, environment: 'development' }))
  assert.equal(byLabel(tree, '发布环境').length, 0)
  tree = f.render(props({ embedded: true, environment: 'development' }))
  assert.equal(elements(tree, item => item.type === 'ExternalReleaseForm').length, 1)
  findButton(tree, '激活').props.onClick()
  tree = f.render(props({ embedded: true, environment: 'development' }))
  const oldConfirmation = elements(tree, item => item.type === 'AlertDialogAction' && text(item).includes('确认激活'))[0]
  assert.equal(elements(tree, item => item.type === 'AlertDialog')[1].props.open, true)
  tree = f.render(props({ embedded: true, environment: 'production' }))
  assert.equal(elements(tree, item => item.type === 'AlertDialog')[1].props.open, false)
  assert.equal(elements(tree, item => item.type === 'ExternalReleaseForm')[0].props.environment, 'production')
  oldConfirmation.props.onClick({ preventDefault() {} })
  assert.equal(f.mutations[2].variables, undefined)
  assert.equal(f.calls.length, 0)
})

test('closed release panel ignores a late deployment result and next step is an explicit callback', async () => {
  const f = fixture()
  let changes = 0, continues = 0
  const values = props({ onDeploymentChanged() { changes++ }, onContinue() { continues++ } })
  let tree = f.render(values)
  findButton(tree, '查看接入检查').props.onClick()
  assert.equal(continues, 1)
  assert.equal(f.calls.length, 0)
  tree = f.render({ ...values, open: false })
  await f.mutations[2].options.onSuccess({ version: release.version, environment: 'development' }, {
    kind: 'activate', target: { applicationId: app.id, environment: 'development' }, release,
  })
  assert.equal(changes, 0)
})

test('external entry keeps optional diagnostics folded until they need correction', () => {
  const f = fixture('ExternalReleaseForm.tsx', ['ExternalReleaseForm'])
  const values = { environment: 'development', busy: false, pending: false, onSubmit() {} }
  let tree = f.render(values)
  assert.deepEqual(f.forms[0].values.resources, [])
  assert.ok(findButton(tree, '登记并检查'))
  let advanced = elements(tree, item => item.type === 'details' && text(item).includes('补充检查项（可选）'))[0]
  assert.ok(advanced)
  assert.ok(!advanced.props.open)
  f.forms[0].formState.errors = { healthUrl: { message: '无效的健康地址' } }
  tree = f.render(values)
  advanced = elements(tree, item => item.type === 'details' && text(item).includes('补充检查项（可选）'))[0]
  assert.equal(advanced.props.open, true)
})

test('delivery operations stay folded and disappear without successful permission evidence', () => {
  const f = fixture()
  let tree = f.render()
  let operations = elements(tree, item => item.type === 'details' && text(item).includes('交付与运维'))[0]
  assert.ok(operations)
  assert.ok(!operations.props.open)
  f.queries.set('permissions', success({ permissions: [] }))
  tree = f.render()
  assert.equal(elements(tree, item => item.type === 'details' && text(item).includes('交付与运维')).length, 0)
  f.queries.set('permissions', { ...success({ permissions: ['system:deployment:read'] }), isSuccess: false, isError: true })
  tree = f.render()
  assert.equal(elements(tree, item => item.type === 'details' && text(item).includes('交付与运维')).length, 0)
})

const urlRequest = () => ({ target: { applicationId: app.id, environment: 'development' }, payload: {
  environment: 'development', version: '2.0.0', entryType: 'HTML', entryUrl: '/sub-apps/sample/versions/2.0.0/index.html',
  healthUrl: null, resources: [],
} })

test('URL registration automatically checks the newly registered release without activating it', async () => {
  const registered = { ...release, id: 'registered-new-id', version: '2.0.0', validationStatus: 'DRAFT' }
  const checked = { ...registered, validationStatus: 'PASSED' }
  const f = fixture(undefined, undefined, { registerRelease: async () => registered, preflightRelease: async () => checked })
  f.render()
  const request = urlRequest()
  const result = await f.mutations[0].options.mutationFn(request)
  assert.equal(result.release, checked)
  assert.ok(!result.preflightError)
  assert.deepEqual(f.calls, [
    { method: 'registerRelease', args: [app.id, request.payload] },
    { method: 'preflightRelease', args: [app.id, registered.id] },
  ])
})

test('automatic check failure preserves successful registration and reports a retryable check result', async () => {
  const registered = { ...release, id: 'known-registered-id', validationStatus: 'DRAFT' }
  const f = fixture(undefined, undefined, { registerRelease: async () => registered,
    preflightRelease: async () => { throw new Error('检查连接中断') } })
  f.render()
  const request = urlRequest()
  const result = await f.mutations[0].options.mutationFn(request)
  assert.equal(result.release, registered)
  assert.equal(result.preflightError, '检查连接中断')
  f.mutations[0].options.onSuccess(result, request)
  assert.deepEqual(f.calls.map(call => call.method), ['registerRelease', 'preflightRelease'])
  assert.ok(!f.toasts.some(values => text(values).includes('URL 登记失败')))
  assert.ok(f.toasts.some(values => text(values).includes('已登记')))
})

test('uncertain registration failure never starts a check or silently repeats registration', async () => {
  const f = fixture(undefined, undefined, { registerRelease: async () => { throw new Error('登记结果未知') } })
  f.render()
  await assert.rejects(f.mutations[0].options.mutationFn(urlRequest()), /登记结果未知/)
  assert.deepEqual(f.calls.map(call => call.method), ['registerRelease'])
})

test('URL registration stays busy while registration and resource checking run in sequence', async () => {
  let completeRegistration, completeCheck
  const registration = new Promise(resolve => { completeRegistration = resolve })
  const check = new Promise(resolve => { completeCheck = resolve })
  const f = fixture(undefined, undefined, { registerRelease: () => registration, preflightRelease: () => check })
  let tree = f.render()
  f.render()
  f.mutations[0].mutate(urlRequest())
  await Promise.resolve()
  tree = f.render()
  assert.equal(elements(tree, item => item.type === 'ExternalReleaseForm')[0].props.busy, true)
  assert.deepEqual(f.calls.map(call => call.method), ['registerRelease'])
  completeRegistration({ ...release, id: 'registered-pending-check', validationStatus: 'DRAFT' })
  await new Promise(resolve => setImmediate(resolve))
  tree = f.render()
  assert.deepEqual(f.calls.map(call => call.method), ['registerRelease', 'preflightRelease'])
  assert.equal(elements(tree, item => item.type === 'ExternalReleaseForm')[0].props.busy, true)
  completeCheck(release)
  await f.mutations[0].promise
  tree = f.render()
  assert.equal(elements(tree, item => item.type === 'ExternalReleaseForm')[0].props.busy, false)
})

test('version records summarize automatically checked scripts and styles without requiring explicit resources', () => {
  const f = fixture()
  f.queries.set('application-releases', success([{ ...release, resources: [], validationResult: { checks: [
    { kind: 'ENTRY', passed: true, url: release.entryUrl, message: '入口可用' },
    { kind: 'SCRIPT', passed: true, url: '/assets/app.js', message: '脚本可用' },
    { kind: 'SCRIPT', passed: true, url: '/assets/chunk.js', message: '脚本可用' },
    { kind: 'STYLE', passed: true, url: '/assets/app.css', message: '样式可用' },
  ] } }]))
  const tree = f.render()
  assert.match(text(tree), /已检查 2 个脚本 · 1 个样式/)
  assert.ok(!text(tree).includes('0 项补充资源'))
})


test('release deletion requires confirmation and protects deployed and rollback versions', async () => {
  const f = fixture()
  let tree = f.render()
  byLabel(tree, `删除版本 ${release.version}`)[0].props.onClick()
  tree = f.render()
  assert.equal(f.calls.length, 0)
  elements(tree, item => item.type === 'AlertDialogAction' && text(item).includes('确认删除'))[0].props.onClick({ preventDefault() {} })
  await f.mutations[3].promise
  assert.deepEqual(f.calls, [{ method: 'removeRelease', args: [app.id, release.id] }])
  for (const field of ['activeReleaseId', 'previousReleaseId']) {
    const protectedFixture = fixture()
    protectedFixture.queries.set('application-deployments', success([{ environment: 'development', [field]: release.id }]))
    assert.equal(byLabel(protectedFixture.render(), `删除版本 ${release.version}`)[0].props.disabled, true)
  }
})
