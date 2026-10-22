import assert from 'node:assert/strict'
import test from 'node:test'
import { componentFixture, success, pending, elements, button } from './helpers/component-fixture.mjs'

function roleFixture(search = 'tenantId=tenant-b&organizationId=org-b') {
  let params = new URLSearchParams(search)
  const resets = []
  const flatten = nodes => nodes.flatMap(node => [node, ...flatten(node.children ?? [])])
  const f = componentFixture('src/pages/system/roles/RolesPage.tsx', ['RolesPage'], {
    'react-router-dom': { useSearchParams: () => [params] },
    'react-hook-form': { useForm: () => ({ reset: value => resets.push(value), control: {}, handleSubmit: callback => callback }) },
    '@/lib/form': { zResolver: () => undefined },
    '@/lib/format': { useDebounced: value => value },
    '@/hooks/use-synced-keyword': { useSyncedKeyword: () => ['', () => {}] },
    '@/hooks/use-page-recovery': { usePageRecovery() {} },
    '@/hooks/use-table-density': { useTableDensity: () => ({ density: 'normal' }), DENSITY_BODY_CLASS: {} },
    '@/hooks/use-column-visibility': { useColumnVisibility: () => ({ visibleSet: new Set(), isVisible: () => false }) },
    '@/components/EntityTreePicker': { EntityTreePicker: 'EntityTreePicker', flattenPicker: flatten },
  })
  f.queries.set('tenants', success([{ id: 'tenant-a', name: 'Tenant A' }, { id: 'tenant-b', name: 'Tenant B' }]))
  f.queries.set('org-tree', success([{ id: 'org-b', name: 'Org B' }]))
  return { ...f, resets, route: value => { params = new URLSearchParams(value) } }
}

test('the application access link selects the requested tenant and organization for role creation', () => {
  const f = roleFixture()
  const tree = f.render('RolesPage')
  assert.deepEqual(f.queryOptions.get('org-tree').queryKey, ['org-tree', 'tenant-b'])
  assert.deepEqual(f.queryOptions.get('roles').queryKey.slice(-2), ['tenant-b', 'org-b'])
  const actions = elements(tree, item => item.type === 'PageHeader')[0].props.actions
  button(actions, '新建角色').props.onClick()
  assert.equal(f.resets.at(-1).organizationId, 'org-b')
})

test('role deep links wait for organization candidates and discard targets outside the selected tenant', () => {
  const f = roleFixture()
  f.queries.set('org-tree', pending())
  let tree = f.render('RolesPage')
  button(elements(tree, item => item.type === 'PageHeader')[0].props.actions, '新建角色').props.onClick()
  assert.equal(f.resets.at(-1).organizationId, '')
  f.queries.set('org-tree', success([{ id: 'other-org', name: 'Other' }]))
  tree = f.render('RolesPage')
  assert.equal(f.queryOptions.get('roles').queryKey.at(-1), null)
  button(elements(tree, item => item.type === 'PageHeader')[0].props.actions, '新建角色').props.onClick()
  assert.equal(f.resets.at(-1).organizationId, '')
})

test('role filters follow a new application access link when the same route stays mounted', () => {
  const f = roleFixture()
  f.render('RolesPage')
  f.route('tenantId=tenant-a&organizationId=org-a')
  f.queries.set('org-tree', pending())
  f.render('RolesPage')
  assert.deepEqual(f.queryOptions.get('org-tree').queryKey, ['org-tree', 'tenant-a'])
  f.queries.set('org-tree', success([{ id: 'org-a', name: 'Org A' }]))
  const tree = f.render('RolesPage')
  button(elements(tree, item => item.type === 'PageHeader')[0].props.actions, '新建角色').props.onClick()
  assert.equal(f.resets.at(-1).organizationId, 'org-a')
})

function deferred() {
  let resolve, reject
  const promise = new Promise((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

function regionFixture(api) {
  const f = componentFixture('src/pages/system/regions/RegionsPage.tsx', ['RegionsPage'], {
    '@/api/regions': { regionsApi: api },
    '@/hooks/use-page-recovery': { usePageRecovery() {} },
  })
  f.queries.set('region-versions', success([{ version: '2023', label: '2023版', count: 51 }]))
  f.queries.set('regions', success({ records: [], total: 51 }))
  return f
}

test('region saves ignore a second click before React renders the pending state', async () => {
  const response = deferred()
  const writes = []
  const f = regionFixture({ create: payload => { writes.push(payload); return response.promise } })
  let tree = f.render('RegionsPage')
  button(tree, '新增地区').props.onClick()
  tree = f.render('RegionsPage')
  const save = button(tree, '保存').props.onClick
  const first = save()
  const second = save()
  assert.equal(writes.length, 1)
  response.resolve({})
  await Promise.all([first, second])
  assert.equal(button(f.render('RegionsPage'), '保存'), undefined)
})

test('a failed region save releases the write lock and preserves the draft for retry', async () => {
  let attempts = 0
  const f = regionFixture({ create: async () => { if (++attempts === 1) throw new Error('unavailable') } })
  let tree = f.render('RegionsPage')
  button(tree, '新增地区').props.onClick()
  tree = f.render('RegionsPage')
  await button(tree, '保存').props.onClick()
  tree = f.render('RegionsPage')
  assert.equal(button(tree, '保存').props.disabled, false)
  await button(tree, '保存').props.onClick()
  assert.equal(attempts, 2)
})

test('creating a region version submits once and resets an old page and search', async () => {
  const response = deferred(), writes = []
  const f = regionFixture({ createVersion: payload => { writes.push(payload); return response.promise } })
  let tree = f.render('RegionsPage')
  button(tree, '下一页').props.onClick()
  elements(tree, item => item.type === 'Input' && item.props.placeholder === '搜索编码、名称或拼音')[0].props.onChange({ target: { value: 'Old search' } })
  tree = f.render('RegionsPage')
  elements(tree, item => item.type === 'form')[0].props.onSubmit({ preventDefault() {} })
  tree = f.render('RegionsPage')
  button(tree, '下一页').props.onClick()
  button(elements(tree, item => item.type === 'PageHeader')[0].props.actions, '新建版本').props.onClick()
  tree = f.render('RegionsPage')
  elements(tree, item => item.type === 'Input' && item.props.placeholder === '版本标识，如 2024')[0].props.onChange({ target: { value: ' 2024 ' } })
  tree = f.render('RegionsPage')
  const create = button(tree, '创建').props.onClick
  const first = create(), second = create()
  assert.equal(writes.length, 1)
  assert.equal(writes[0].version, '2024')
  f.queries.set('region-versions', success([{ version: '2023', count: 51 }, { version: '2024', count: 0 }]))
  response.resolve({})
  await Promise.all([first, second])
  f.render('RegionsPage')
  assert.deepEqual(f.queryOptions.get('regions').queryKey, ['regions', '2024', undefined, '', 1])
})
