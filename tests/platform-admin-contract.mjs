// Documentation/consumer contracts only. Rust validates runtime inputs and HTTP
// behavior in tools/platform-admin tests; these assertions do not execute admin writes.
import assert from 'node:assert/strict'
import test from 'node:test'
import { existsSync, readFileSync } from 'node:fs'
import { routeView } from '../business-apps/work-orders/frontend/src/domain.mjs'

const read = relative => readFileSync(new URL('../' + relative, import.meta.url), 'utf8')
const json = relative => JSON.parse(read(relative))
const manifestPath = 'tools/platform-admin/manifests/'
const schemaPath = 'tools/platform-admin/schemas/'
const bootstrap = json(manifestPath + 'bootstrap.manifest.json')
const keys = ['root', 'ticket:read', 'ticket:create', 'ticket:update', 'ticket:delete', 'ticket:export', 'notification:publish']
const permissions = keys.slice(1)
const dialect = 'https://json-schema.org/draft/2020-12/schema'

function closed(schema, names, optional = []) {
  assert.equal(schema.type, 'object')
  assert.equal(schema.additionalProperties, false)
  assert.deepEqual(Object.keys(schema.properties).sort(), [...names].sort())
  assert.deepEqual([...schema.required].sort(), names.filter(name => !optional.includes(name)).sort())
}

test('Rust embedded bootstrap and retained public source share the exact seven-key schema contract', () => {
  assert.equal(read(manifestPath + 'bootstrap.manifest.json'), read('business-apps/work-orders/deploy/bootstrap.manifest.json'))
  const schema = json(schemaPath + 'bootstrap.schema.json')
  assert.equal(schema.$schema, dialect)
  closed(schema, ['schemaVersion', 'application', 'oauthClient', 'menus', 'grants'])
  assert.equal(schema.properties.schemaVersion.const, bootstrap.schemaVersion)
  for (const name of ['application', 'oauthClient']) {
    const value = bootstrap[name], definition = schema.properties[name]
    closed(definition, Object.keys(value))
    for (const [key, property] of Object.entries(definition.properties)) {
      if (Object.hasOwn(property, 'const')) assert.deepEqual(property.const, value[key])
      else if (property.type === 'string') assert.equal(typeof value[key], 'string')
      else assert.equal(Number.isInteger(value[key]), true)
    }
  }
  assert.equal(schema.properties.oauthClient.properties.provisioning.minLength, 1)
  const menus = schema.properties.menus
  assert.equal(menus.minItems, 7)
  assert.equal(menus.maxItems, 7)
  assert.equal(menus.items, false)
  assert.deepEqual(bootstrap.menus.map(menu => menu.key), keys)
  assert.equal(menus.prefixItems.length, 7)
  for (const [index, menu] of bootstrap.menus.entries()) {
    const definition = menus.prefixItems[index]
    closed(definition, Object.keys(menu))
    for (const key of ['key', 'parentKey', 'type', 'path', 'permission', 'visible', 'linkTarget']) {
      assert.deepEqual(definition.properties[key].const, menu[key])
    }
    assert.equal(definition.properties.title.type, 'string')
    assert.equal(definition.properties.sortOrder.type, 'integer')
  }
  assert.deepEqual(bootstrap.grants, [])
  assert.equal(schema.properties.grants.maxItems, 0)
})

test('grants schema closes all fields and keeps permissions, explicit scopes and seven menu IDs separate', () => {
  const schema = json(schemaPath + 'grants.schema.json')
  assert.equal(schema.$schema, dialect)
  closed(schema, ['schemaVersion', 'applicationCode', 'applicationId', 'menuIds', 'targets'], ['targets'])
  assert.equal(schema.properties.schemaVersion.const, '1.0')
  assert.equal(schema.properties.applicationCode.const, 'work-orders')
  closed(schema.properties.menuIds, keys)
  for (const value of Object.values(schema.properties.menuIds.properties)) assert.equal(value.$ref, '#/$defs/uuid')
  assert.deepEqual(schema.properties.targets.default, [])
  const target = schema.properties.targets.items
  closed(target, ['tenantId', 'organizationId', 'roleId', 'userId', 'membershipId', 'permissions', 'dataGrants'])
  for (const name of ['tenantId', 'organizationId', 'roleId', 'userId', 'membershipId']) assert.equal(target.properties[name].$ref, '#/$defs/uuid')
  assert.deepEqual(target.properties.permissions.items.enum, permissions)
  assert.equal(target.properties.permissions.uniqueItems, true)
  assert.equal(Object.hasOwn(target.properties.permissions, 'default'), false)
  const grant = target.properties.dataGrants.items
  closed(grant, ['resource', 'action', 'scope'])
  assert.deepEqual(grant.properties.scope.enum, ['SELF', 'ORG', 'ORG_SUBTREE', 'TENANT'])
  assert.equal(Object.hasOwn(grant.properties.scope, 'default'), false)
  assert.deepEqual(grant.allOf[0].then.properties.action.enum, ['read', 'create', 'update', 'delete', 'export'])
  assert.equal(grant.allOf[0].else.properties.action.const, 'publish')
  const uuid = new RegExp(schema.$defs.uuid.pattern)
  assert.equal(uuid.test('11111111-1111-1111-1111-111111111111'), true)
  assert.equal(schema.$defs.uuid.not.const, '00000000-0000-0000-0000-000000000000')
  const example = json(manifestPath + 'grants.manifest.example.json')
  assert.equal(read(manifestPath + 'grants.manifest.example.json'), read('business-apps/work-orders/deploy/grants.manifest.example.json'))
  assert.deepEqual(Object.keys(example.menuIds), keys)
  assert.deepEqual(example.targets, [])
  assert.equal(uuid.test(example.applicationId), false, 'unreviewed placeholder example must not be executable')
  for (const id of Object.values(example.menuIds)) assert.equal(uuid.test(id), false)
})

test('release schema publishes only existing URL registration fields and fixed resource kinds', () => {
  const schema = json(schemaPath + 'release.schema.json')
  assert.equal(schema.$schema, dialect)
  closed(schema, ['environment', 'version', 'entryType', 'entryUrl', 'healthUrl', 'resources'], ['healthUrl'])
  assert.equal(schema.properties.entryType.const, 'HTML')
  assert.equal(new RegExp(schema.properties.environment.pattern).test('production'), true)
  assert.equal(new RegExp(schema.properties.environment.pattern).test('../production'), false)
  assert.equal(new RegExp(schema.properties.version.pattern).test('1.0.0'), true)
  assert.equal(new RegExp(schema.properties.version.not.pattern).test('1..0'), true)
  assert.equal(schema.properties.healthUrl.anyOf[1].type, 'null')
  assert.equal(schema.properties.resources.maxItems, 48)
  closed(schema.properties.resources.items, ['url', 'kind'])
  assert.deepEqual(schema.properties.resources.items.properties.kind.enum, ['SCRIPT', 'STYLE', 'ASSET'])
})

test('artifact declaration schema is separate, closed and has source-specific immutable identities', () => {
  const schema = json(schemaPath + 'artifact.schema.json')
  assert.equal(schema.$schema, dialect)
  closed(schema, ['schemaVersion', 'applicationCode', 'publicOrigin', 'source', 'digest', 'commit', 'buildId', 'inputsSha256'])
  assert.equal(schema.properties.schemaVersion.const, 1)
  assert.deepEqual(schema.properties.applicationCode.enum, ['work-orders', 'stores'])
  const [image, external] = schema.properties.source.oneOf
  closed(image, ['kind', 'identity', 'targetPlatform'])
  closed(external, ['kind', 'identity'])
  assert.equal(image.properties.kind.const, 'OCI_IMAGE')
  assert.equal(external.properties.kind.const, 'STATIC_ARTIFACT')
  assert.deepEqual(image.properties.targetPlatform.enum, ['linux/amd64', 'linux/arm64'])
  assert.equal(new RegExp(schema.properties.digest.pattern).test('sha256:' + 'a'.repeat(64)), true)
  assert.equal(new RegExp(schema.properties.digest.pattern).test('sha256:' + 'A'.repeat(64)), false)
  const example = json(manifestPath + 'artifact.manifest.example.json')
  assert.equal(new RegExp(schema.properties.digest.pattern).test(example.digest), false, 'unreviewed artifact example must not be executable')
  const release = json(schemaPath + 'release.schema.json')
  assert.equal(Object.hasOwn(release.properties, 'source'), false)
  assert.equal(Object.hasOwn(release.properties, 'digest'), false)
})

test('embedded create and detail MENU paths agree with actual work-orders route behavior', () => {
  const root = bootstrap.menus.find(menu => menu.key === 'root')
  const detail = bootstrap.menus.find(menu => menu.key === 'ticket:read')
  const create = bootstrap.menus.find(menu => menu.key === 'ticket:create')
  assert.deepEqual([root.type, root.path, root.permission], ['MENU', '/micro/work-orders', ''])
  assert.deepEqual([detail.type, detail.path, detail.visible, detail.permission], ['MENU', '/micro/work-orders/tickets/:id', false, 'ticket:read'])
  assert.deepEqual([create.type, create.path, create.visible, create.permission], ['MENU', '/micro/work-orders/ticket-create', true, 'ticket:create'])
  assert.deepEqual(routeView(create.path), { kind: 'create' })
  assert.deepEqual(routeView(detail.path.replace(':id', 'ticket-123')), { kind: 'detail', id: 'ticket-123' })
  assert.deepEqual(routeView('/micro/work-orders/tickets/new'), { kind: 'detail', id: 'new' })
  assert.equal(create.path.startsWith('/micro/work-orders/tickets/'), false)
  for (const menu of bootstrap.menus.slice(3)) assert.deepEqual([menu.type, menu.path, menu.visible, menu.linkTarget], ['BUTTON', '', false, 'BLANK'])
})

test('retired Node management implementation and suites have no executable fallback', () => {
  for (const name of ['platform-admin.mjs', 'platform-admin-core.mjs', 'grants.mjs', 'tests/platform-admin.mjs', 'tests/grants.mjs']) {
    assert.equal(existsSync(new URL('../business-apps/work-orders/deploy/' + name, import.meta.url)), false, name)
  }
  const fixture = read('business-apps/work-orders/tests/platform-e2e.mjs')
  assert.ok(fixture.includes('process.env.PLATFORM_ADMIN_BIN ?? process.env.PHASE25_CLI_BIN'))
  assert.equal(fixture.includes("import('../deploy/platform-admin-core.mjs')"), false)
  assert.equal(existsSync(new URL('../scripts/package-microapp.mjs', import.meta.url)), true)
})

const storesPath = 'business-apps/stores/deploy/'
const stores = json(storesPath + 'bootstrap.manifest.json')
const storesMenus = [
  ['root', 'MENU', '/micro/stores', '', true],
  ['store:read', 'MENU', '/micro/stores/stores/:id', 'store:read', false],
  ['store:create', 'MENU', '/micro/stores/store-create', 'store:create', true],
  ['store:update', 'BUTTON', '', 'store:update', false],
  ['store-staff:read', 'MENU', '/micro/stores/staff', 'store-staff:read', true],
  ['store-staff:assign', 'MENU', '/micro/stores/staff-assign', 'store-staff:assign', true],
  ['store-staff:unassign', 'BUTTON', '', 'store-staff:unassign', false],
  ['store-daily-report:read', 'MENU', '/micro/stores/reports', 'store-daily-report:read', true],
  ['store-daily-report:detail', 'MENU', '/micro/stores/daily-reports/:id', 'store-daily-report:read', false],
  ['store-daily-report:create', 'MENU', '/micro/stores/daily-report-create', 'store-daily-report:create', true],
  ['store-daily-report:update', 'BUTTON', '', 'store-daily-report:update', false],
]
const storesPermissions = [...new Set(storesMenus.map(menu => menu[3]).filter(Boolean))]

test('stores public and embedded definitions freeze exactly eleven ordered menus and the existing confidential client protocol', () => {
  for (const [directory, embedded, published] of [
    [manifestPath, 'stores-bootstrap.manifest.json', 'bootstrap.manifest.json'],
    [schemaPath, 'stores-bootstrap.schema.json', 'bootstrap.schema.json'],
    [manifestPath, 'stores-grants.manifest.example.json', 'grants.manifest.example.json'],
    [schemaPath, 'stores-grants.schema.json', 'grants.schema.json'],
  ]) assert.equal(read(directory + embedded), read(storesPath + published))
  assert.deepEqual(stores.menus.map(menu => [menu.key, menu.type, menu.path, menu.permission, menu.visible]), storesMenus)
  assert.equal(stores.application.code, 'stores')
  assert.equal(stores.application.basePath, '/micro/stores')
  assert.equal(stores.application.entryUrl, '/sub-apps/stores/unreleased/index.html')
  assert.equal(stores.application.autoCreateMenu, false)
  assert.deepEqual([stores.application.entryType, stores.application.runtimeMode], ['HTML', 'IFRAME'])
  assert.deepEqual(stores.grants, [])
  assert.equal(stores.oauthClient.clientName, 'stores-client')
  assert.equal(stores.oauthClient.clientType, 'CONFIDENTIAL')
  assert.deepEqual(stores.oauthClient.grantTypes, ['client_credentials'])
  assert.deepEqual(stores.oauthClient.scopes, ['openid'])
  assert.deepEqual(stores.oauthClient.redirectUris, [])
  assert.deepEqual([stores.oauthClient.accessTokenValidity, stores.oauthClient.refreshTokenValidity], [300, 300])
  const schema = json(storesPath + 'bootstrap.schema.json')
  assert.equal(schema.$schema, dialect)
  closed(schema, Object.keys(stores))
  assert.deepEqual([schema.properties.menus.minItems, schema.properties.menus.maxItems], [11, 11])
  assert.equal(schema.properties.menus.items, false)
  stores.menus.forEach((menu, index) => {
    assert.equal(menu.parentKey, index === 0 ? null : 'root')
    assert.equal(menu.sortOrder, index)
    assert.equal(menu.linkTarget, menu.type === 'MENU' ? 'MICRO' : 'BLANK')
    assert.equal(menu.linkUrl, '')
    assert.equal(menu.status, 'ENABLED')
    const definition = schema.properties.menus.prefixItems[index]
    closed(definition, Object.keys(menu))
    for (const [key, value] of Object.entries(menu)) if (key !== 'title') assert.deepEqual(definition.properties[key].const, value)
  })
})

test('stores grants schema and backend policy publish exactly nine capabilities, eleven menu IDs and three member purposes', () => {
  const schema = json(storesPath + 'grants.schema.json')
  assert.equal(schema.properties.applicationCode.const, 'stores')
  closed(schema, ['schemaVersion', 'applicationCode', 'applicationId', 'menuIds', 'targets'], ['targets'])
  closed(schema.properties.menuIds, storesMenus.map(menu => menu[0]))
  const target = schema.properties.targets.items
  closed(target, ['tenantId', 'organizationId', 'roleId', 'userId', 'membershipId', 'permissions', 'dataGrants'])
  assert.deepEqual(target.properties.permissions.items.enum, storesPermissions)
  assert.equal(storesPermissions.length, 9)
  assert.equal(target.properties.permissions.uniqueItems, true)
  const grant = target.properties.dataGrants.items
  closed(grant, ['resource', 'action', 'scope'])
  assert.deepEqual(grant.properties.scope.enum, ['SELF', 'ORG', 'ORG_SUBTREE', 'TENANT'])
  const pairs = grant.oneOf.flatMap(variant => variant.properties.action.enum.map(action => variant.properties.resource.const + ':' + action))
  assert.deepEqual(pairs, storesPermissions)
  const policy = json('business-apps/stores/backend/policy.example.json').applications[0]
  assert.deepEqual(policy.capabilities.map(capability => capability.resource + ':' + capability.action), storesPermissions)
  assert.deepEqual(policy.memberLookups.map(pair => pair.resource + ':' + pair.action), ['store:create', 'store:update', 'store-staff:assign'])
  const example = json(storesPath + 'grants.manifest.example.json')
  assert.deepEqual(example.targets, [])
  assert.deepEqual(Object.keys(example.menuIds), storesMenus.map(menu => menu[0]))
  const uuid = new RegExp(schema.$defs.uuid.pattern)
  for (const id of [example.applicationId, ...Object.values(example.menuIds), policy.applicationId]) assert.equal(uuid.test(id), false)
  for (const value of [stores, example]) assert.doesNotMatch(JSON.stringify(value), /"(?:clientId|clientSecret|secret|token)"\s*:/)
})
