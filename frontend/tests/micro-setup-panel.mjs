import assert from 'node:assert/strict'
import test from 'node:test'
import { componentFixture, success, pending, failure, elements, button } from './helpers/component-fixture.mjs'

const application = { id: 'app-a', code: 'crm', name: 'CRM', entryType: 'HTML', status: 'ENABLED' }
const props = overrides => ({ application, environment: 'development', step: 'overview', onEnvironmentChange() {}, onStepChange() {}, onClose() {}, onEdit() {}, ...overrides })
const status = changes => ({ applicationId: 'app-a', environment: 'development', canConfigureAccess: true, entryMenuCount: 0, ...changes })
function fixture() {
  const calls = [], statuses = []
  const f = componentFixture('src/pages/system/applications/ApplicationSetupPanel.tsx', ['ApplicationSetupPanel'], {
    '@/api/application-setup': { applicationSetupApi: { status() {} } },
    '@/micro-apps/registry': { hostEsmEntryForCode: code => code === 'report-center' ? '/src/micro-apps/report-center.tsx' : null },
    './ApplicationAccessSetup': { ApplicationAccessSetup: 'ApplicationAccessSetup' },
    './ApplicationReleasesPanel': { ApplicationReleasesPanel: 'ApplicationReleasesPanel' },
    './application-setup-policy': { applicationSetupChecks: (_app, value, available) => { calls.push(available); statuses.push(value); return [{ id: 'release', title: '发布版本', complete: value.releaseValidated && (value.entryType !== 'ESM' || available), description: 'check' }] } },
    'react-router-dom': { useNavigate: () => () => {} },
  })
  return { f, calls, statuses }
}

test('setup does not present cached results as current after a failed read', () => {
  const { f } = fixture()
  f.queries.set('application-setup', { ...failure(), data: status({ releaseValidated: true }) })
  const tree = f.render('ApplicationSetupPanel', props())
  assert.equal(elements(tree, node => node.type === 'ErrorState').length, 1)
  assert.equal(elements(tree, node => node.props?.children === 'check').length, 0)
})

test('setup rejects status from another application or environment', () => {
  for (const wrong of [{ applicationId: 'app-b' }, { environment: 'production' }]) {
    const { f } = fixture()
    f.queries.set('application-setup', success(status(wrong)))
    const tree = f.render('ApplicationSetupPanel', props({ step: 'access' }))
    assert.equal(elements(tree, node => node.type === 'ApplicationAccessSetup').length, 0)
    assert.equal(elements(tree, node => node.props?.role === 'status').length, 1)
  }
})

test('busy publication blocks parent close, steps and environment while keeping the embedded panel mounted', () => {
  const { f } = fixture(), steps = [], environments = [], closes = []
  f.queries.set('application-setup', success(status({})))
  const p = props({ step: 'release', onStepChange: value => steps.push(value), onEnvironmentChange: value => environments.push(value), onClose: () => closes.push(true) })
  let tree = f.render('ApplicationSetupPanel', p)
  const panel = elements(tree, node => node.type === 'ApplicationReleasesPanel')[0]
  assert.equal(panel.props.embedded, true)
  assert.equal(panel.props.environment, 'development')
  panel.props.onBusyChange(true)
  tree = f.render('ApplicationSetupPanel', p)
  elements(tree, node => node.type === 'Sheet')[0].props.onOpenChange(false)
  elements(tree, node => node.type === 'Select')[0].props.onValueChange('production')
  for (const nav of elements(tree, node => node.type === 'button' && node.props['aria-current'] !== undefined)) nav.props.onClick()
  assert.equal(button(tree, '稍后继续').props.disabled, true)
  assert.deepEqual([steps, environments, closes], [[], [], []])
})

test('a code existing in the host does not validate a different or missing ESM entry URL', () => {
  for (const entryUrl of ['/src/micro-apps/missing.tsx', '', '/src/micro-apps/report-center.tsx']) {
    const { f, calls } = fixture()
    f.queries.set('application-setup', success(status({ entryType: 'ESM', releaseValidated: true })))
    const tree = f.render('ApplicationSetupPanel', props({ application: { ...application, code: 'report-center', entryType: 'ESM', entryUrl }, step: 'access' }))
    const expected = entryUrl === '/src/micro-apps/report-center.tsx'
    assert.equal(calls.at(-1), expected)
    assert.equal(elements(tree, node => node.type === 'ApplicationAccessSetup')[0].props.entryReady, expected)
  }
})

test('pending checks never mount access configuration with guessed permissions', () => {
  const { f } = fixture()
  f.queries.set('application-setup', pending())
  const tree = f.render('ApplicationSetupPanel', props({ step: 'access' }))
  assert.equal(elements(tree, node => node.type === 'ApplicationAccessSetup').length, 0)
})

test('a selected audience survives step changes and feeds the actual overview check', () => {
  const { f, statuses } = fixture()
  const target = { tenantId: 'tenant-a', organizationId: 'org-a', roleId: 'role-a' }
  f.queries.set('application-setup', success(status({ releaseValidated: true })))
  let tree = f.render('ApplicationSetupPanel', props({ step: 'access' }))
  elements(tree, node => node.type === 'ApplicationAccessSetup')[0].props.onTargetChange(target)
  f.queries.set('application-setup-target', success(status({ access: { ...target, tenantAuthorized: true, organizationAuthorized: true, roleAuthorized: true, assignedUserCount: 2 } })))
  tree = f.render('ApplicationSetupPanel', props({ step: 'overview' }))
  assert.equal(statuses.at(-1).access.roleId, 'role-a')
  tree = f.render('ApplicationSetupPanel', props({ step: 'access' }))
  const access = elements(tree, node => node.type === 'ApplicationAccessSetup')[0]
  assert.deepEqual(access.props.initialTarget, target)
  access.props.onTargetChange(null)
  f.render('ApplicationSetupPanel', props({ step: 'overview' }))
  assert.equal(statuses.at(-1).access, null)
})

test('changing an audience does not unmount its editor while its new check is pending', () => {
  const { f } = fixture()
  f.queries.set('application-setup', success(status({})))
  let tree = f.render('ApplicationSetupPanel', props({ step: 'access' }))
  const access = elements(tree, node => node.type === 'ApplicationAccessSetup')[0]
  access.props.onTargetChange({ tenantId: 'tenant-a', organizationId: 'org-a', roleId: 'role-a' })
  f.queries.set('application-setup-target', pending())
  tree = f.render('ApplicationSetupPanel', props({ step: 'access' }))
  assert.equal(elements(tree, node => node.type === 'ApplicationAccessSetup').length, 1)
})

test('losing access management permission cannot reuse a cached audience check', () => {
  const { f, statuses } = fixture()
  const target = { tenantId: 'tenant-a', organizationId: 'org-a', roleId: 'role-a' }
  f.queries.set('application-setup', success(status({})))
  const tree = f.render('ApplicationSetupPanel', props({ step: 'access' }))
  elements(tree, node => node.type === 'ApplicationAccessSetup')[0].props.onTargetChange(target)
  f.queries.set('application-setup-target', success(status({ access: { ...target, tenantAuthorized: true, organizationAuthorized: true, roleAuthorized: true, assignedUserCount: 2 } })))
  f.queries.set('application-setup', success(status({ canConfigureAccess: false })))
  f.render('ApplicationSetupPanel', props({ step: 'overview' }))
  assert.equal(statuses.at(-1).access, null)
})
