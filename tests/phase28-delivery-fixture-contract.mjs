import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { readPhase28Runtime } from './helpers/phase28-delivery-services.mjs'

function owned(t) {
  const parent = realpathSync(os.tmpdir()), root = realpathSync(mkdtempSync(path.join(parent, 'phase28-fixture-contract-')))
  t.after(() => {
    assert.equal(path.dirname(realpathSync(root)), parent)
    assert.match(path.basename(root), /^phase28-fixture-contract-/)
    rmSync(root, { recursive: true, force: true })
  })
  writeFileSync(path.join(root, 'ca.pem'), 'public fixture CA')
  writeFileSync(path.join(root, 'admin'), 'selected fixture CLI')
  const candidate = { release: { environment: 'production', version: '1.0.0', entryType: 'HTML', entryUrl: '/sub-apps/stores/versions/1.0.0/index.html', healthUrl: null, resources: [] },
    artifact: { schemaVersion: 1, digest: 'sha256:' + 'a'.repeat(64) }, buildInfo: { entry: 'assets/main.js', dynamicChunks: ['assets/lazy.js'] } }
  const runtime = { schemaVersion: 1, owner: 'ci-phase28-contract-01234567', sourceRevision: 'a'.repeat(40), inputsSha256: 'b'.repeat(64),
    origin: 'https://host.phase28.test:8443', externalOrigin: 'https://external.phase28.test:8443', caFile: path.join(root, 'ca.pem'), adminBinary: path.join(root, 'admin'),
    inventoryDir: path.join(root, 'inventory'), stateFile: path.join(root, 'state.json'), policyFile: path.join(root, 'policy.json'), reportDir: path.join(root, 'report'),
    admin: { username: 'fixture-admin', password: 'synthetic-unpublished-password' },
    databaseUrls: Object.fromEntries(['platform','stores','work-orders'].map((code, index) => [code, `postgresql://fixture_${index}:synthetic@db:5432/fixture_${index}`])),
    clients: Object.fromEntries(['stores','work-orders'].map(code => [code, { clientId: code + '-fixture', clientSecret: 'synthetic-' + code }])),
    releases: Object.fromEntries(['stores','work-orders'].map(code => [code, Object.fromEntries(['v1','v2','external'].map(kind => [kind, structuredClone(candidate)]))])),
    deliveryProof: { registryPushPull: true, fixedDigestCompose: true, unhealthyBlocked: { routes: 0, register: 0, activate: 0 } } }
  const config = path.join(root, 'config.json')
  const save = (value = runtime) => writeFileSync(config, JSON.stringify(value))
  save(); return { root, config, runtime, save }
}

test('private fixture inputs are stable across prepare/browser and never silently rewritten', t => {
  const fixture = owned(t), first = readPhase28Runtime(fixture.config)
  const firstBytes = readFileSync(first.releases.stores.v1.releaseFile, 'utf8')
  const second = readPhase28Runtime(fixture.config)
  assert.equal(readFileSync(second.releases.stores.v1.releaseFile, 'utf8'), firstBytes)
  fixture.runtime.releases.stores.v1.release.entryUrl = '/sub-apps/stores/versions/1.0.0/replaced.html'; fixture.save()
  assert.throws(() => readPhase28Runtime(fixture.config), /immutable fixture inputs changed/)
  assert.equal(readFileSync(first.releases.stores.v1.releaseFile, 'utf8'), firstBytes)
})

test('private runtime rejects insecure origins, shared external origin and unrelated resource ownership', t => {
  const fixture = owned(t)
  for (const mutate of [runtime => { runtime.origin = 'http://host.phase28.test:8443' }, runtime => { runtime.externalOrigin = runtime.origin },
    runtime => { runtime.origin += '/path' }, runtime => { runtime.owner = 'an-unrelated-production-owner' }]) {
    const changed = structuredClone(fixture.runtime); mutate(changed); fixture.save(changed)
    assert.throws(() => readPhase28Runtime(fixture.config))
  }
})
