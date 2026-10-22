import assert from 'node:assert/strict'
import test from 'node:test'
import { z } from 'zod'
import { componentFixture, elements, success, button } from './helpers/component-fixture.mjs'

function deferred() {
  let resolve, reject
  const promise = new Promise((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

function fixture(api) {
  let values
  const messages = []
  const state = { contextEpoch: 1, contextSwitching: false, profile: { tenant: { id: 'tenant-a' }, organization: { id: 'org-a', name: 'Org A' } } }
  const form = { control: {}, reset: next => { values = { ...next } }, handleSubmit: callback => () => callback({ ...values }) }
  const f = componentFixture('src/pages/system/departments/DepartmentsPage.tsx', ['DepartmentsPage'], {
    'react-hook-form': { useForm: options => { values ??= { ...options.defaultValues }; return form } },
    '@/lib/form': { zResolver: () => undefined },
    '@/stores/auth': { useAuthStore: selector => selector(state) },
    '@/api/departments': { departmentsApi: api },
    sonner: { toast: { success: value => messages.push(value), error: value => messages.push(value) } },
  })
  f.queries.set('department-organizations', success([{ id: 'org-a', name: 'Org A', children: [{ id: 'org-b', name: 'Org B', children: [] }] }]))
  f.queries.set('dept-tree', success([]))
  return { ...f, state, messages, select(id) {
    const tree = f.render('DepartmentsPage')
    const actions = elements(tree, item => item.type === 'PageHeader')[0].props.actions
    elements(actions, item => item.type === 'EntityTreePicker')[0].props.onSelect(id)
    return f.render('DepartmentsPage')
  }, create() {
    const tree = f.render('DepartmentsPage')
    button(tree, '新增根部门').props.onClick()
    return f.render('DepartmentsPage')
  } }
}

test('department create captures its organization and blocks duplicate submissions before rerender', async () => {
  const response = deferred(), writes = []
  const f = fixture({ create: payload => { writes.push(payload); return response.promise } })
  const tree = f.create()
  const submit = elements(tree, item => item.type === 'form')[0].props.onSubmit
  submit(); submit()
  await Promise.resolve()
  assert.equal(writes.length, 1)
  assert.equal(writes[0].organizationId, 'org-a')
  response.resolve({})
  await f.mutations[0].promise
})

test('a retained department form callback cannot write after selecting another organization', async () => {
  const writes = []
  const f = fixture({ create: async payload => writes.push(payload) })
  const tree = f.create()
  const submit = elements(tree, item => item.type === 'form')[0].props.onSubmit
  f.select('org-b')
  submit()
  await Promise.resolve()
  assert.deepEqual(writes, [])
})

test('a late department save does not close a new organization draft', async () => {
  const response = deferred()
  const f = fixture({ create: () => response.promise })
  elements(f.create(), item => item.type === 'form')[0].props.onSubmit()
  await Promise.resolve()
  f.select('org-b')
  f.create()
  response.resolve({})
  await f.mutations[0].promise
  const tree = f.render('DepartmentsPage')
  assert.equal(elements(tree, item => item.type === 'Dialog')[0].props.open, true)
  assert.deepEqual(f.messages, [])
})

test('returning to the original organization does not revive its retired save callback', async () => {
  const response = deferred()
  const f = fixture({ create: () => response.promise })
  elements(f.create(), item => item.type === 'form')[0].props.onSubmit()
  await Promise.resolve()
  f.select('org-b'); f.select('org-a'); f.create()
  response.resolve({})
  await f.mutations[0].promise
  assert.equal(elements(f.render('DepartmentsPage'), item => item.type === 'Dialog')[0].props.open, true)
  assert.deepEqual(f.messages, [])
})

test('a delayed department error after unmount does not emit a toast for the new page', async () => {
  const response = deferred()
  const f = fixture({ create: () => response.promise })
  elements(f.create(), item => item.type === 'form')[0].props.onSubmit()
  await Promise.resolve()
  f.unmount()
  response.reject(new Error('old organization error'))
  await f.mutations[0].promise
  assert.deepEqual(f.messages, [])
})

function organizationFixture(api = {}, configuration = {}) {
  const forms = new Map(), messages = []
  const f = componentFixture('src/pages/system/organizations/OrganizationsPage.tsx', ['OrganizationsPage', 'SelectRow'], {
    'react-router-dom': { useSearchParams: () => [new URLSearchParams()] },
    'react-hook-form': { useForm: options => {
      const key = 'organizationType' in options.defaultValues ? 'organization' : 'department'
      if (!forms.has(key)) {
        let values = { ...options.defaultValues }
        forms.set(key, { control: {}, reset: next => { values = { ...next } }, watch: name => name ? values[name] : values, setValue: (name, value) => { values[name] = value }, handleSubmit: callback => () => callback({ ...values }) })
      }
      return forms.get(key)
    } },
    '@/lib/form': { zResolver: () => undefined },
    '@/lib/contact': { contactFields: {}, requireContact() {} },
    '@/lib/entity-types': { organizationTypes: ['COMPANY'], organizationTypeLabels: { COMPANY: 'Company' } },
    '@/lib/location': { coordinateField: () => z.string(), hasCoordinatePair: () => true, coordinateOrNull: () => null, coordinatePoint: () => null, locationText: () => '' },
    '@/lib/expiry': { expiryBadge: () => null, isoToLocalInput: value => value ?? '' },
    '@/api/organizations': { organizationsApi: api.organizations ?? {} },
    '@/api/departments': { departmentsApi: api.departments ?? {} },
    sonner: { toast: { success: value => messages.push(value), error: value => messages.push(value) } },
  })
  const root = tenant => ({ id: `org-${tenant.at(-1)}`, tenantId: tenant, parentId: null, name: `Organization ${tenant}`, code: 'root', expiresAt: '2024-01-01T00:00:00Z', children: configuration.child ? [{ id: 'child', tenantId: tenant, parentId: `org-${tenant.at(-1)}`, name: 'Child', code: 'child', children: [] }] : [], status: 'ENABLED' })
  f.queries.set('tenants', success([{ id: 'tenant-a', name: 'Tenant A', isPlatform: configuration.platform === true }, { id: 'tenant-b', name: 'Tenant B' }]))
  f.queries.set('org-tree', success([root('tenant-a')]))
  f.queries.set('dept-tree', success([]))
  return { ...f, forms, messages, createOrganization() {
    const tree = f.render('OrganizationsPage')
    button(tree, '新增根组织').props.onClick()
    return f.render('OrganizationsPage')
  }, tenant(id) {
    const tree = f.render('OrganizationsPage')
    const actions = elements(tree, item => item.type === 'PageHeader')[0].props.actions
    elements(actions, item => item.type === 'Select')[0].props.onValueChange(id)
    f.queries.set('org-tree', success([root(id)]))
    return f.render('OrganizationsPage')
  } }
}

test('organization creation binds its tenant, prevents duplicates and retires old administrator receipts', async () => {
  const response = deferred(), writes = []
  const f = organizationFixture({ organizations: { create: payload => { writes.push(payload); return response.promise } } })
  const tree = f.createOrganization()
  const submit = elements(tree, item => item.type === 'form' && item.props.id === 'org-form')[0].props.onSubmit
  submit(); submit()
  await Promise.resolve()
  assert.equal(writes.length, 1)
  assert.equal(writes[0].tenantId, 'tenant-a')
  f.tenant('tenant-b')
  f.createOrganization()
  response.resolve({ administrator: { initialPassword: 'old-tenant-password' } })
  await f.mutations[0].promise
  const current = f.render('OrganizationsPage')
  assert.equal(elements(current, item => item.type === 'Dialog')[0].props.open, true)
  assert.equal(elements(current, item => item.type === 'ContactAdministratorDialog')[0].props.administrator, null)
  assert.deepEqual(f.messages, [])
})

test('an organization form retained across a tenant change cannot create in either tenant', async () => {
  const writes = []
  const f = organizationFixture({ organizations: { create: async payload => { writes.push(payload); return {} } } })
  const tree = f.createOrganization()
  const submit = elements(tree, item => item.type === 'form' && item.props.id === 'org-form')[0].props.onSubmit
  f.tenant('tenant-b')
  submit()
  await Promise.resolve()
  assert.deepEqual(writes, [])
})

test('administrator initialization submits once and never reveals a result after its entity unmounts', async () => {
  const response = deferred(), receipts = []
  let attempts = 0
  const f = componentFixture('src/components/InitializeAdministratorButton.tsx', ['InitializeAdministratorButton'])
  const props = { name: 'Tenant A', initialize: () => { attempts++; return response.promise }, onSuccess: value => receipts.push(value) }
  let tree = f.render('InitializeAdministratorButton', props)
  elements(tree, item => item.type === 'Button' && item.props['aria-label'] === '初始化首位管理员')[0].props.onClick()
  tree = f.render('InitializeAdministratorButton', props)
  const initialize = button(tree, '确认初始化').props.onClick
  initialize(); initialize()
  await Promise.resolve()
  assert.equal(attempts, 1)
  f.unmount()
  response.resolve({ administrator: { initialPassword: 'retired-password' } })
  await f.mutations[0].promise
  assert.deepEqual(receipts, [])
})

for (const scenario of [
  { name: 'platform root', platform: true, child: false, protected: true },
  { name: 'platform child', platform: true, child: true, protected: false },
  { name: 'ordinary tenant root', platform: false, child: false, protected: false },
]) {
  test(`${scenario.name} keeps the correct expiry and delete policy`, async () => {
    const writes = []
    const f = organizationFixture({ organizations: { update: async (...args) => writes.push(args) } }, scenario)
    let tree = f.render('OrganizationsPage')
    const editor = elements(tree, item => item.type?.name === 'SelectableTree')[0]
    const node = scenario.child ? editor.props.nodes[0].children[0] : editor.props.nodes[0]
    assert.equal(node.platformRoot, scenario.protected)
    editor.props.onDelete(node)
    tree = f.render('OrganizationsPage')
    assert.equal(elements(tree, item => item.type === 'ConfirmDialog')[0].props.open, !scenario.protected)
    editor.props.onEdit(node)
    tree = f.render('OrganizationsPage')
    const expiry = elements(tree, item => item.type === 'FormField' && item.props.name === 'expiresAt')[0]
    const field = expiry.props.render({ field: { value: f.forms.get('organization').watch('expiresAt') } })
    assert.equal(elements(field, item => item.type === 'Input')[0].props.disabled, scenario.protected)
    if (scenario.protected) assert.match(JSON.stringify(field), /平台根组织永久有效/)
    f.forms.get('organization').setValue('expiresAt', '2030-05-04T00:00:00Z')
    elements(tree, item => item.type === 'form' && item.props.id === 'org-form')[0].props.onSubmit()
    await f.mutations[0].promise
    assert.equal(writes[0][1].expiresAt, scenario.protected ? null : '2030-05-04T00:00:00.000Z')
    const rowFixture = organizationFixture({}, scenario)
    const row = rowFixture.render('SelectRow', { node, depth: 0, selectedId: node.id, onSelect() {}, onAdd() {}, onEdit() {}, onDelete() {}, onAdministrator() {} })
    assert.equal(elements(row, item => item.type?.name === 'IconBtn' && item.props.label === '删除').length, scenario.protected ? 0 : 1)
  })
}

for (const isPlatform of [true, false]) {
  test(`${isPlatform ? 'platform' : 'ordinary'} tenant expiry is rendered and submitted consistently`, async () => {
    let values
    const writes = []
    const tenant = { id: 'tenant', name: 'Tenant', code: 'tenant', tenantType: 'COMPANY', isPlatform, expiresAt: '2024-01-01T00:00:00Z' }
    const form = { control: {}, reset: next => { values = { ...next } }, watch: name => name ? values[name] : values, handleSubmit: callback => () => callback({ ...values }) }
    const f = componentFixture('src/pages/system/tenants/TenantsPage.tsx', ['TenantsPage'], {
      'react-hook-form': { useForm: options => { values ??= { ...options.defaultValues }; return form } },
      '@/lib/form': { zResolver: () => undefined },
      '@/lib/contact': { contactFields: {}, requireContact() {} },
      '@/lib/entity-types': { tenantTypes: ['COMPANY'], tenantTypeLabels: { COMPANY: 'Company' } },
      '@/lib/location': { coordinateField: () => z.string(), hasCoordinatePair: () => true, coordinateOrNull: () => null, coordinatePoint: () => null, locationText: () => '' },
      '@/lib/expiry': { isoToLocalInput: value => value ?? '' },
      '@/lib/format': { useDebounced: value => value, formatDateTime: value => value },
      '@/lib/platform-context': { hasPlatformContext: () => true },
      '@/stores/auth': { useAuthStore: selector => selector({ profile: {} }) },
      '@/hooks/use-synced-keyword': { useSyncedKeyword: () => ['', () => {}] },
      '@/hooks/use-page-recovery': { usePageRecovery() {} },
      '@/api/tenants': { tenantsApi: { update: async (...args) => writes.push(args) } },
    })
    f.queries.set('tenants', success({ records: [tenant], total: 1 }))
    let tree = f.render('TenantsPage')
    elements(tree, item => item.type === 'Button' && item.props['aria-label'] === '编辑 Tenant')[0].props.onClick()
    tree = f.render('TenantsPage')
    assert.equal(values.expiresAt, isPlatform ? '' : tenant.expiresAt)
    const expiry = elements(tree, item => item.type === 'FormField' && item.props.name === 'expiresAt')[0]
    const field = expiry.props.render({ field: { value: values.expiresAt } })
    assert.equal(elements(field, item => item.type === 'Input')[0].props.disabled, isPlatform)
    if (isPlatform) assert.match(JSON.stringify(field), /平台租户永久有效/)
    values.expiresAt = '2030-05-04T00:00:00Z'
    elements(tree, item => item.type === 'form' && item.props.id === 'tenant-form')[0].props.onSubmit()
    await f.mutations[0].promise
    assert.equal(writes[0][1].expiresAt, isPlatform ? null : '2030-05-04T00:00:00.000Z')
  })
}
