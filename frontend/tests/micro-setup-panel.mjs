import assert from 'node:assert/strict'
import test from 'node:test'
import { componentFixture, success, pending, failure, elements, button } from './helpers/component-fixture.mjs'

const application = { id: 'app-a', code: 'crm', name: 'CRM', entryType: 'HTML', runtimeMode: 'TRUSTED', status: 'ENABLED' }
const props = overrides => ({ application, environment: 'development', step: 'overview', onEnvironmentChange() {}, onStepChange() {}, onClose() {}, onEdit() {}, ...overrides })
const status = changes => ({ applicationId: 'app-a', environment: 'development', entryType: 'HTML', runtimeMode: 'TRUSTED', canConfigureAccess: true, entryMenuCount: 0, ...changes })
function fixture() {
  const calls = [], statuses = []
  const f = componentFixture('src/pages/system/applications/ApplicationSetupPanel.tsx', ['ApplicationSetupPanel'], {
    '@/api/application-setup': { applicationSetupApi: { status() {} } },
    './ApplicationAccessSetup': { ApplicationAccessSetup: 'ApplicationAccessSetup' },
    './ApplicationReleasesPanel': { ApplicationReleasesPanel: 'ApplicationReleasesPanel' },
    './application-setup-policy': { isQiankunApp: app => app.entryType === 'HTML' && app.runtimeMode === 'TRUSTED',
      applicationSetupChecks: (_app, value) => { statuses.push(value); return [{ id: 'release', title: '应用入口', complete: value.releaseValidated && value.entryType === 'HTML' && value.runtimeMode === 'TRUSTED', description: 'check' }] } },
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

test('setup rejects status from another application', () => {
  for (const wrong of [{ applicationId: 'app-b' }]) {
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
  assert.equal(elements(tree, node => node.type === 'Select').length,0)
  for (const nav of elements(tree, node => node.type === 'button' && node.props['aria-current'] !== undefined)) nav.props.onClick()
  assert.equal(button(tree, '稍后继续').props.disabled, true)
  assert.deepEqual([steps, environments, closes], [[], [], []])
})

test('legacy entry modes cannot pass readiness and use the same reconfiguration panel', () => {
  for (const legacy of [{ entryType: 'ESM', runtimeMode: 'TRUSTED' }, { entryType: 'HTML', runtimeMode: 'IFRAME' }]) {
    const { f } = fixture()
    f.queries.set('application-setup', success(status({ ...legacy, releaseValidated: true })))
    let tree = f.render('ApplicationSetupPanel', props({ application: { ...application, ...legacy }, step: 'access' }))
    assert.equal(elements(tree, node => node.type === 'ApplicationAccessSetup')[0].props.entryReady, false)
    tree = f.render('ApplicationSetupPanel', props({ application: { ...application, ...legacy }, step: 'release' }))
    assert.equal(elements(tree, node => node.type === 'ApplicationReleasesPanel').length, 1)
    assert.match(JSON.stringify(tree), /需重新配置 qiankun/)
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

test('setup derives the environment from the server instead of an old page parameter', () => {
  const {f}=fixture()
  f.queries.set('application-setup',success(status({environment:'production'})))
  const tree=f.render('ApplicationSetupPanel',props({environment:'development',step:'release'}))
  assert.equal(elements(tree,n=>n.type==='ApplicationReleasesPanel')[0].props.environment,'production')
  assert.equal(elements(tree,n=>n.type==='Select').length,0)
})
