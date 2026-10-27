import assert from 'node:assert/strict'
import test from 'node:test'
import { componentFixture, elements, success, pending, button } from './helpers/component-fixture.mjs'

function deferred() {
  let resolve, reject
  const promise = new Promise((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

const client = { id: '01962345-6789-7abc-8def-000000000001', clientId: 'public-a', clientName: 'Public A', clientType: 'PUBLIC', requirePkce: false,
  subjectMode: 'USER', allowedMemberTypes: ['EMPLOYEE'], accessPolicyVersion: 1,
  grantTypes: ['authorization_code'], scopes: ['profile'], redirectUris: ['https://client.test/callback'] }

function fixture(api, clients = []) {
  let values
  const messages = []
  const form = {
    control: {},
    reset: next => { values = { ...next } },
    watch: key => values?.[key],
    setValue: (key, value) => { values[key] = value },
    handleSubmit: callback => () => callback({ ...values }),
  }
  const f = componentFixture('src/pages/system/oauth/ClientsPage.tsx', ['OAuthClientsPage'], {
    'react-hook-form': { useForm: options => { values ??= { ...options.defaultValues }; return form } },
    '@/lib/form': { zResolver: () => undefined },
    '@/lib/format': { GRANT_TYPE_LABELS: {}, formatNumber: String },
    '@/api/oauth': { oauthApi: api },
    './OidcClientsPanel': { OidcClientsPanel: 'OidcClientsPanel' },
    sonner: { toast: { success: message => messages.push(message), error: message => messages.push(message) } },
  })
  f.queries.set('oauth-clients', success({ records: clients, total: clients.length, page: 1, size: 10 }))
  f.queries.set('applications', success([]))
  return { ...f, messages, values: () => values, field(tree, name) {
    const field = elements(tree, item => item.type === 'FormField' && item.props.name === name)[0]
    return field.props.render({ field: { value: values[name], onChange: value => { values[name] = value } } })
  } }
}

function newClient(f) {
  const tree = f.render('OAuthClientsPage')
  const actions = elements(tree, item => item.type === 'PageHeader')[0].props.actions
  button(actions, '新建客户端').props.onClick()
  return f.render('OAuthClientsPage')
}

test('new clients use type-specific PKCE defaults and submit explicit migration overrides', async () => {
  const writes = []
  const f = fixture({ createClient: async payload => { writes.push(payload); return {} } })
  let tree = newClient(f)
  assert.equal(f.values().requirePkce, false)
  elements(f.field(tree, 'clientType'), item => item.type === 'Select')[0].props.onValueChange('PUBLIC')
  assert.equal(f.values().requirePkce, true)
  tree = f.render('OAuthClientsPage')
  elements(f.field(tree, 'requirePkce'), item => item.type === 'Checkbox')[0].props.onCheckedChange(false)
  elements(tree, item => item.type === 'form')[0].props.onSubmit()
  await f.mutations[0].promise
  assert.equal(writes[0].clientType, 'PUBLIC')
  assert.equal(writes[0].requirePkce, false)
  assert.equal(writes[0].subjectMode, 'USER')
  assert.deepEqual(writes[0].allowedMemberTypes, ['EMPLOYEE'])
  assert.equal(f.messages.at(-1), '客户端创建成功')
})

test('editing a legacy PUBLIC client preserves compatibility until the administrator enables S256', async () => {
  const writes = []
  const f = fixture({ updateClient: async (...args) => writes.push(args) }, [client])
  let tree = f.render('OAuthClientsPage')
  elements(tree, item => item.type === 'Button' && item.props['aria-label'] === '编辑')[0].props.onClick()
  tree = f.render('OAuthClientsPage')
  assert.equal(f.values().requirePkce, false)
  const policy = f.field(tree, 'requirePkce')
  assert.match(JSON.stringify(policy), /已有接入方请先完成 S256 配置再开启/)
  elements(policy, item => item.type === 'Checkbox')[0].props.onCheckedChange(true)
  elements(tree, item => item.type === 'form')[0].props.onSubmit()
  await f.mutations[0].promise
  assert.equal(writes[0][0], client.id)
  assert.equal(writes[0][1].requirePkce, true)
  assert.equal('allowedMemberTypes' in writes[0][1], false, 'Ordinary client editing must not submit an unversioned admission change')
  assert.equal('subjectMode' in writes[0][1], false, 'PKCE compatibility editing preserves the registered subject purpose')
})

test('client records show whether authorization-code requests require S256', () => {
  const f = fixture({}, [client, { ...client, id: '01962345-6789-7abc-8def-000000000002', requirePkce: true }])
  const tree = f.render('OAuthClientsPage')
  assert.match(JSON.stringify(tree), /PKCE · 兼容模式/)
  assert.match(JSON.stringify(tree), /PKCE · 强制 S256/)
})

test('secret regeneration rejects same-frame duplicate rotations and ignores unmounted results', async () => {
  const response = deferred(), writes = []
  const f = fixture({ regenerateSecret: id => { writes.push(id); return response.promise } }, [client])
  const tree = f.render('OAuthClientsPage')
  const rotate = elements(tree, item => item.type === 'Button' && item.props['aria-label'] === '重新生成 Secret')[0].props.onClick
  rotate(); rotate()
  await Promise.resolve()
  assert.deepEqual(writes, [client.id])
  f.unmount()
  response.resolve({ clientSecret: 'retired-secret' })
  await f.mutations[1].promise
  assert.deepEqual(f.messages, [])
})

test('a pending client save cannot close or replace its dialog and failed saves can retry', async () => {
  const response = deferred()
  let attempts = 0
  const f = fixture({ createClient: () => { attempts++; return response.promise } })
  let tree = newClient(f)
  const submit = elements(tree, item => item.type === 'form')[0].props.onSubmit
  submit(); submit()
  tree = f.render('OAuthClientsPage')
  elements(tree, item => item.type === 'Dialog')[0].props.onOpenChange(false)
  const actions = elements(tree, item => item.type === 'PageHeader')[0].props.actions
  button(actions, '新建客户端').props.onClick()
  tree = f.render('OAuthClientsPage')
  assert.equal(elements(tree, item => item.type === 'Dialog')[0].props.open, true)
  await Promise.resolve()
  assert.equal(attempts, 1)
  response.reject(new Error('temporary failure'))
  await f.mutations[0].promise
  tree = f.render('OAuthClientsPage')
  assert.equal(elements(tree, item => item.type === 'Dialog')[0].props.open, true)
  assert.equal(button(tree, '创建并生成密钥').props.disabled, false)
})

test('client paging requests server pages, resets size, and recovers a deleted last page without resetting during loading', () => {
  const f = fixture({})
  f.queries.set('oauth-clients', success({ records: [client], total: 25, page: 1, size: 10 }))
  let tree = f.render('OAuthClientsPage')
  let pagination = elements(tree, item => item.type === 'DataPagination')[0]
  assert.equal(pagination.props.total, 25)
  pagination.props.onPageChange(3)
  tree = f.render('OAuthClientsPage')
  assert.deepEqual(f.queryOptions.get('oauth-clients').queryKey, ['oauth-clients', 3, 10])
  elements(tree, item => item.type === 'DataPagination')[0].props.onSizeChange(20)
  tree = f.render('OAuthClientsPage')
  assert.deepEqual(f.queryOptions.get('oauth-clients').queryKey, ['oauth-clients', 1, 20])
  elements(tree, item => item.type === 'DataPagination')[0].props.onPageChange(2)
  f.queries.set('oauth-clients', pending())
  f.render('OAuthClientsPage')
  assert.deepEqual(f.queryOptions.get('oauth-clients').queryKey, ['oauth-clients', 2, 20])
  f.queries.set('oauth-clients', success({ records: [], total: 20, page: 2, size: 20 }))
  f.render('OAuthClientsPage')
  assert.deepEqual(f.queryOptions.get('oauth-clients').queryKey, ['oauth-clients', 1, 20])
})
