import assert from 'node:assert/strict'
import test from 'node:test'
import { componentFixture, elements, button } from './helpers/component-fixture.mjs'

const record = { id: 'application-id', name: '客户管理', code: 'crm', basePath: '/micro/crm', entryType: 'HTML', runtimeMode: 'TRUSTED' }

function editorFixture() {
  const writes = [], created = []
  let values
  const form = {
    control: {}, formState: { errors: {} }, watch: key => values[key],
    reset: next => { values = { ...next } }, setValue: (key, value) => { values[key] = value },
    handleSubmit: callback => event => { event?.preventDefault?.(); return callback({ ...values }) },
  }
  const f = componentFixture('src/pages/system/applications/use-application-editor.ts', ['useApplicationEditor', 'isMicroApp'], {
    'react-hook-form': { useForm: options => { values ??= { ...options.defaultValues }; return form } },
    '@/lib/form': { zResolver: () => undefined },
    '@/api/applications': { applicationsApi: {
      create: async payload => { writes.push(['create', payload]); return { ...record, ...payload } },
      update: async (...args) => { writes.push(['update', ...args]) },
    } },
  })
  const onCreated = application => created.push(application)
  const dialog = editor => componentFixture('src/pages/system/applications/ApplicationEditorDialog.tsx', ['ApplicationEditorDialog'], {
    './use-application-editor': f.exports,
  }).render('ApplicationEditorDialog', { editor })
  return { ...f, writes, created, form, values: () => values, dialog, editor: () => f.render('useApplicationEditor', onCreated) }
}

test('application creation always sends qiankun HTML and continues with the created record', async () => {
  const f = editorFixture()
  f.editor().openCreate()
  f.form.setValue('name', '客户管理')
  f.form.setValue('code', 'crm')
  f.form.setValue('entryType', 'ESM')
  f.form.setValue('runtimeMode', 'IFRAME')
  f.form.setValue('entryUrl', '/src/obsolete.tsx')
  f.editor().submit({ preventDefault() {} })
  await f.mutations[0].promise
  assert.deepEqual(f.writes[0], ['create', {
    name: '客户管理', code: 'crm', icon: 'LayoutGrid', basePath: '/micro/crm', description: '',
    sortOrder: 0, status: 'ENABLED', entryType: 'HTML', runtimeMode: 'TRUSTED',
    entryUrl: '/sub-apps/crm/unreleased/index.html', autoCreateMenu: true,
  }])
  assert.equal(f.created[0].id, record.id)
  assert.equal(f.editor().dialogOpen, false)
})

test('metadata editing preserves the code and never changes entry or execution mode', async () => {
  const f = editorFixture()
  f.editor().openEdit({ ...record, entryUrl: '/existing/version/index.html' })
  f.form.setValue('name', '新名称')
  f.form.setValue('code', 'attempted-change')
  f.form.setValue('entryType', 'ESM')
  f.form.setValue('runtimeMode', 'IFRAME')
  f.form.setValue('entryUrl', '/src/obsolete.tsx')
  f.editor().submit({ preventDefault() {} })
  await f.mutations[0].promise
  const payload = f.writes[0][2]
  assert.equal(payload.code, record.code)
  assert.equal(payload.name, '新名称')
  for (const key of ['entryType', 'entryUrl', 'runtimeMode', 'autoCreateMenu']) assert.equal(key in payload, false)
  assert.equal(f.created.length, 0)
})

test('built-in React pages use the same HTML metadata and preserve their empty external entry', async () => {
  for (const code of ['system', 'hermes']) {
    const f = editorFixture()
    f.editor().openEdit({ ...record, code, basePath: `/${code}`, entryUrl: '' })
    f.form.setValue('name', `${code} updated`)
    f.editor().submit({ preventDefault() {} })
    await f.mutations[0].promise
    assert.equal(f.writes[0][2].basePath, `/${code}`)
    for (const key of ['entryType', 'runtimeMode', 'entryUrl']) assert.equal(key in f.writes[0][2], false)
    assert.doesNotMatch(JSON.stringify(f.dialog(f.editor())), /iframe|ESM|宿主模块|选择第一方模块/)
  }
})

test('creation offers metadata only and does not expose legacy loader or module choices', () => {
  const f = editorFixture()
  f.editor().openCreate()
  const editor = f.editor(), tree = f.dialog(editor)
  const details = elements(tree, item => item.type === 'details')[0]
  assert.equal(Boolean(details.props.open), false)
  const optionalNames = elements(details, item => item.type === 'FormField').map(item => item.props.name)
  assert.deepEqual(optionalNames, ['basePath', 'icon', 'description', 'sortOrder', 'status'])
  assert.deepEqual(elements(tree, item => item.type === 'FormField').map(item => item.props.name).filter(name => !optionalNames.includes(name)), ['name', 'code'])
  assert.match(JSON.stringify(tree), /qiankun/)
  assert.doesNotMatch(JSON.stringify(tree), /iframe|ESM|选择第一方模块|接入方式|加载方式/)
  editor.saveMutation.isPending = true
  tree.props.onOpenChange(false)
  assert.equal(f.editor().dialogOpen, true)
})

test('legacy incompatible entries require explicit reconfiguration before metadata saving or enabling', () => {
  for (const legacy of [{ entryType: 'ESM', runtimeMode: 'TRUSTED' }, { entryType: 'HTML', runtimeMode: 'IFRAME' }]) {
    const f = editorFixture()
    f.editor().openEdit({ ...record, ...legacy, status: 'DISABLED' })
    const tree = f.dialog(f.editor())
    assert.equal(button(tree, '保存更改').props.disabled, true)
    assert.match(JSON.stringify(tree), /先在应用入口中明确重新配置为 qiankun/)
  }
})
