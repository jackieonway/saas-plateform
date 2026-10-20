import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { resolveRehearsalImages } from './rehearsal-images.mjs'

const toolsId = 'sha256:' + 'a'.repeat(64), postgresId = 'sha256:' + 'b'.repeat(64)
const postgres = 'postgres@sha256:45cd22f8d32e189d245403954882f88e7a8714301fda80dab6da90f1265b25a3'
const image = Id => ({ Id, Os: 'linux', Architecture: 'amd64', RepoDigests: [] })

test('clean-host rehearsals resolve provisioned references and use their actual immutable IDs', async () => {
  const calls = [], states = [image(toolsId), image(postgresId)]
  const resolved = await resolveRehearsalImages(async args => { calls.push(args); return JSON.stringify([states[calls.length - 1]]) }, {}, {})
  assert.deepEqual(calls, [['image', 'inspect', 'saas-ci:phase04'], ['image', 'inspect', postgres]])
  assert.equal(resolved.toolsImage, toolsId)
  assert.equal(resolved.postgresImage, postgresId)
  assert.equal(resolved.evidence.tools.requested, 'saas-ci:phase04')
  // Both real entry points consume the resolver rather than machine-specific IDs.
  for (const name of ['managed-deployment', 'zip-distribution']) {
    const source = readFileSync(new URL(`./${name}.mjs`, import.meta.url), 'utf8')
    assert.match(source, /await resolveRehearsalImages\(docker, options\)/)
    assert.doesNotMatch(source, /const (?:toolsImage|postgresImage)\s*=\s*'sha256:/)
  }
  const zipFixture = readFileSync(new URL('../../tests/phase35-zip-distribution.mjs', import.meta.url), 'utf8')
  assert.match(zipFixture, /'--entrypoint','sh',toolsImage,'-c'/)
  assert.doesNotMatch(zipFixture, /sha256:1912f730/)
})

test('explicit image selection overrides environment and is recorded with the resolved identity', async () => {
  const environment = { CI_TOOLCHAIN_IMAGE: 'env-tools:approved', CI_POSTGRES_IMAGE: 'env-postgres:approved' }
  const calls = []
  const docker = async args => { calls.push(args); return JSON.stringify([image(calls.length % 2 ? toolsId : postgresId)]) }
  const env = await resolveRehearsalImages(docker, {}, environment)
  assert.equal(env.evidence.tools.requested, 'env-tools:approved')
  assert.equal(env.evidence.postgres.requested, 'env-postgres:approved')
  const explicit = await resolveRehearsalImages(docker, { toolchainImage: 'operator-tools@' + toolsId, postgresImage: postgres }, environment)
  assert.equal(explicit.evidence.tools.requested, 'operator-tools@' + toolsId)
  assert.equal(explicit.evidence.postgres.requested, postgres)
})

test('image resolution refuses unavailable, ambiguous or incompatible images before resource creation', async () => {
  for (const states of [[], [image(toolsId), image(toolsId)], [image('sha256:short')], [{ ...image(toolsId), Os: 'windows' }], [{ ...image(toolsId), Architecture: 'arm64' }]]) {
    await assert.rejects(resolveRehearsalImages(async () => JSON.stringify(states)), /Linux amd64 immutable identity/)
  }
  await assert.rejects(resolveRehearsalImages(async () => { throw Error('No such image') }), /No such image/)
  for (const toolchainImage of ['--malformed', 'user:password@registry/image', 'registry/image@sha256:short']) {
    let called = false
    await assert.rejects(resolveRehearsalImages(async () => { called = true }, { toolchainImage }), /literal Docker image reference/)
    assert.equal(called, false)
  }
})
