import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { componentFixture, elements } from './helpers/component-fixture.mjs'

const require = createRequire(import.meta.url)
const ts = require('typescript')
const moduleEntries = [{ code: 'report-center', entryUrl: '/src/micro-apps/report-center.tsx' }]
const record = { id: 'application-id', name: '客户管理', code: 'crm', basePath: '/micro/crm', entryType: 'HTML', runtimeMode: 'IFRAME' }

function editorFixture() {
  const writes = [], created = [], errors = {}
  let values
  const form = {
    control: {},
    formState: { errors: {} },
    watch: key => values[key],
    reset: next => { values = { ...next } },
    setValue: (key, value) => { values[key] = value },
    setError: (key, error) => { errors[key] = error.message },
    clearErrors: key => { delete errors[key] },
    handleSubmit: callback => event => { event?.preventDefault?.(); return callback({ ...values }) },
  }
  const f = componentFixture('src/pages/system/applications/use-application-editor.ts', ['useApplicationEditor', 'isMicroApp', 'isHostEsm'], {
    'react-hook-form': { useForm: options => { values ??= { ...options.defaultValues }; return form } },
    '@/lib/form': { zResolver: () => undefined },
    '@/micro-apps/registry': {
      listHostEsmModules: () => moduleEntries,
      hostEsmEntryForCode: code => moduleEntries.find(item => item.code === code)?.entryUrl ?? null,
    },
    '@/api/applications': { applicationsApi: {
      create: async payload => { writes.push(['create', payload]); return { ...record, ...payload } },
      update: async (...args) => { writes.push(['update', ...args]) },
    } },
  })
  const onCreated = application => { created.push(application) }
  return { ...f, writes, created, errors, form, values: () => values, editor: () => f.render('useApplicationEditor', onCreated) }
}

test('simple application creation keeps safe HTML defaults and continues with the created record', async () => {
  const f = editorFixture()
  let editor = f.editor()
  editor.openCreate()
  editor = f.editor()
  f.form.setValue('name', '客户管理')
  f.form.setValue('code', 'crm')
  assert.equal(editor.quickSetup, true)
  assert.equal(f.values().entryType, 'HTML')
  editor.submit({ preventDefault() {} })
  await f.mutations[0].promise
  assert.equal(f.writes[0][0], 'create')
  assert.deepEqual(f.writes[0][1], {
    name: '客户管理', code: 'crm', icon: 'LayoutGrid', basePath: '/micro/crm', description: '',
    sortOrder: 0, status: 'ENABLED', entryType: 'HTML', runtimeMode: 'IFRAME',
    entryUrl: '/sub-apps/crm/unreleased/index.html', autoCreateMenu: true,
  })
  assert.equal(f.created.length, 1)
  assert.equal(f.created[0].id, record.id)
  assert.equal(f.editor().dialogOpen, false)
})

test('built host module selection determines the code and continues through the same onboarding callback', async () => {
  const f = editorFixture()
  let editor = f.editor()
  f.form.setValue('name', '报表中心')
  f.form.setValue('entryType', 'ESM')
  editor = f.editor()
  editor.selectHostEsmModule('report-center')
  assert.equal(f.values().code, 'report-center')
  assert.equal(f.values().entryUrl, '/src/micro-apps/report-center.tsx')
  editor = f.editor()
  editor.submit({ preventDefault() {} })
  await f.mutations[0].promise
  assert.equal(f.writes[0][1].runtimeMode, 'TRUSTED')
  assert.equal(f.writes[0][1].basePath, '/micro/report-center')
  assert.equal(f.writes[0][1].autoCreateMenu, false)
  assert.equal(f.created[0].entryType, 'ESM')
})

test('unbuilt modules cannot be selected or sent even when the mutation is invoked directly', async () => {
  const f = editorFixture()
  let editor = f.editor()
  editor.selectHostEsmModule('not-in-build')
  assert.match(f.errors.code, /已构建/)
  assert.equal(f.values().code, '')
  f.form.setValue('entryType', 'ESM')
  f.form.setValue('code', 'not-in-build')
  editor = f.editor()
  editor.submit({ preventDefault() {} })
  assert.equal(f.writes.length, 0)
  await assert.rejects(f.mutations[0].options.mutationFn(f.values()), /已构建/)
  assert.equal(f.writes.length, 0)
})

test('metadata editing preserves the original code and never rewrites entry or runtime trust', async () => {
  const f = editorFixture()
  let editor = f.editor()
  editor.openEdit({ ...record, entryType: 'HTML', runtimeMode: undefined, entryUrl: '/existing/version/index.html' })
  editor = f.editor()
  f.form.setValue('name', '新名称')
  f.form.setValue('code', 'attempted-change')
  f.form.setValue('entryType', 'ESM')
  f.form.setValue('entryUrl', '/src/micro-apps/report-center.tsx')
  editor.submit({ preventDefault() {} })
  await f.mutations[0].promise
  assert.equal(f.writes[0][0], 'update')
  const payload = f.writes[0][2]
  assert.equal(payload.code, record.code)
  assert.equal(payload.name, '新名称')
  for (const key of ['entryType', 'entryUrl', 'runtimeMode', 'autoCreateMenu']) assert.equal(key in payload, false)
  assert.equal(f.created.length, 0)
})

test('simple dialog folds optional settings and exposes only name and code before them', () => {
  const f = editorFixture()
  f.editor().openCreate()
  const editor = f.editor()
  const ui = componentFixture('src/pages/system/applications/ApplicationEditorDialog.tsx', ['ApplicationEditorDialog'], {
    './use-application-editor': f.exports,
  })
  const tree = ui.render('ApplicationEditorDialog', { editor })
  const details = elements(tree, item => item.type === 'details')[0]
  assert.ok(details)
  assert.equal(Boolean(details.props.open), false)
  const optionalNames = elements(details, item => item.type === 'FormField').map(item => item.props.name)
  assert.deepEqual(optionalNames, ['entryType', 'basePath', 'icon', 'description', 'sortOrder', 'status'])
  const primaryNames = elements(tree, item => item.type === 'FormField').map(item => item.props.name).filter(name => !optionalNames.includes(name))
  assert.deepEqual(primaryNames, ['name', 'code'])
  assert.match(JSON.stringify(tree), /\/micro\/\{code\}/)
  editor.saveMutation.isPending = true
  tree.props.onOpenChange(false)
  assert.equal(f.editor().dialogOpen, true)
})

test('ESM dialog selects a built module and keeps the derived code read-only', () => {
  const f = editorFixture()
  f.editor()
  f.form.setValue('entryType', 'ESM')
  const editor = f.editor()
  const ui = componentFixture('src/pages/system/applications/ApplicationEditorDialog.tsx', ['ApplicationEditorDialog'], {
    './use-application-editor': f.exports,
  })
  const tree = ui.render('ApplicationEditorDialog', { editor })
  const picker = elements(tree, item => item.type === 'Select')[0]
  picker.props.onValueChange('report-center')
  assert.equal(f.values().code, 'report-center')
  const codeField = elements(tree, item => item.type === 'FormField' && item.props.name === 'code')[0]
  const codeControl = codeField.props.render({ field: { value: f.values().code } })
  assert.equal(elements(codeControl, item => item.type === 'Input')[0].props.readOnly, true)
  assert.match(JSON.stringify(tree), /共享运行环境/)
})

test('host registry lists only valid built modules in stable code order', () => {
  const source = fs.readFileSync(new URL('../src/micro-apps/registry.ts', import.meta.url), 'utf8')
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
    .replace('import.meta.glob("./*.tsx")', '__modules')
  const exported = {}
  vm.runInThisContext(`(function(exports,__modules){${compiled}\n})`)(exported, {
    './z-module.tsx': () => {}, './report-center.tsx': () => {}, './Bad_Module.tsx': () => {}, './nested/module.tsx': () => {},
  })
  assert.deepEqual(exported.listHostEsmModules(), [
    { code: 'report-center', entryUrl: '/src/micro-apps/report-center.tsx' },
    { code: 'z-module', entryUrl: '/src/micro-apps/z-module.tsx' },
  ])
  assert.equal(exported.hostEsmEntryForCode('../outside'), null)
})
