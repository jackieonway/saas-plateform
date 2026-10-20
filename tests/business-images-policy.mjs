import assert from 'node:assert/strict'
import { lstatSync, mkdtempSync, rmSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { checkBusinessHealth, classifyDaemonImageIdentity, fixedDigestPull, forgetOwnedBusinessImages, parseBusinessImageArgs, parseRegistryDescriptor, promoteBusinessRoutes, validateBusinessCandidate, validateBusinessPublication, businessImageKeys, businessOwnerLabel } from '../scripts/ci/business-images.mjs'
import { candidateArtifactName, validateCandidateRun } from '../scripts/ci/business-images-publish.mjs'

const digest = 'sha256:' + 'a'.repeat(64), sourceRevision = 'a'.repeat(40), inputsSha256 = 'b'.repeat(64)
const imageState = health => JSON.stringify([{ Image: digest, Config: { Image: 'localhost:5000/business/stores-web@' + digest }, State: { Running: true, Health: { Status: health } } }])
const services = [{ container: 'phase28-test-stores-v1', imageId: digest, imageRef: 'localhost:5000/business/stores-web@' + digest }]

test('build CLI requires explicit source identity and rejects accidental repeated or unsafe inputs', () => {
  const args = ['--out-dir', process.platform === 'win32' ? 'C:\\temp\\phase28-new' : '/tmp/phase28-new', '--source-revision', sourceRevision]
  assert.equal(parseBusinessImageArgs(args).targetPlatform, 'linux/amd64')
  assert.throws(() => parseBusinessImageArgs([...args, '--platform', 'linux/amd64', '--platform', 'linux/arm64']))
  assert.throws(() => parseBusinessImageArgs([...args, '--build-proxy', 'http://user:secret@localhost:1234']))
  assert.throws(() => parseBusinessImageArgs([...args, '--stores-api-base', 'https://api.test:8443/path?secret=bad']))
})

test('Docker health is bounded and records fixed reference and config identity', async () => {
  let reads = 0, time = 0
  const result = await checkBusinessHealth({ docker: async () => imageState(reads++ ? 'healthy' : 'starting'), services, now: () => time, sleep: async ms => { time += ms }, timeoutMs: 1000, pollMs: 10 })
  assert.equal(result.layer, 'CI_DOCKER_HEALTH'); assert.equal(result.checks[0].health, 'healthy'); assert.equal(reads, 2)
  await assert.rejects(checkBusinessHealth({ docker: async () => imageState('starting'), services, now: () => time, sleep: async ms => { time += ms }, timeoutMs: 10, pollMs: 10 }), /TIMEOUT/)
  await assert.rejects(checkBusinessHealth({ docker: async () => imageState('healthy'), services: [{ ...services[0], imageId: 'sha256:' + 'b'.repeat(64) }] }), /image ID/)
})

test('actual unhealthy Docker target blocks all binding, route and management callbacks', async () => {
  const calls = [], callback = name => async () => { calls.push(name) }
  await assert.rejects(promoteBusinessRoutes({ docker: async () => imageState('unhealthy'), services,
    bindArtifacts: callback('bind'), stageRoutes: callback('stage'), validateRoutes: callback('test'), commitRoutes: callback('commit'), registerReleases: callback('register'), preflightReleases: callback('preflight'), activateReleases: callback('activate') }), /UNHEALTHY/)
  assert.deepEqual(calls, [])
})

test('promotion rechecks health before changing routes and preserves separate platform refusal', async () => {
  const calls = [], callback = name => async () => { calls.push(name); return name }
  const options = { docker: async () => imageState('healthy'), services,
    bindArtifacts: async () => { calls.push('bind'); return { status: 'PASS', layer: 'RUST_CLI_DURABLE_BINDING', bindings: [{ bind: { bindingState: 'BOUND' }, verify: { bindingState: 'BOUND' } }] } }, stageRoutes: callback('stage'), validateRoutes: callback('test'), commitRoutes: callback('commit'), registerReleases: callback('register'),
    preflightReleases: async () => [{ httpStatus: 200, validationStatus: 'FAILED' }], activateReleases: callback('activate') }
  await assert.rejects(promoteBusinessRoutes(options), /PLATFORM_PREFLIGHT_FAILED/)
  assert.deepEqual(calls, ['bind', 'stage', 'test', 'commit', 'register'])
  calls.length = 0
  const result = await promoteBusinessRoutes({ ...options, preflightReleases: async () => [{ validationStatus: 'PASSED' }] })
  assert.equal(result.platformStoresDigest, false); assert.equal(calls.at(-1), 'activate')
})

test('registry descriptor digest is independent of saved archive digest and binds image config', () => {
  const remote = parseRegistryDescriptor(JSON.stringify({ Descriptor: { digest, mediaType: 'application/vnd.oci.image.manifest.v1+json' }, SchemaV2Manifest: { config: { digest: 'sha256:' + 'b'.repeat(64) }, layers: [{ digest: 'sha256:' + 'c'.repeat(64) }] } }))
  assert.equal(remote.digest, digest); assert.notEqual(remote.digest, remote.configDigest)
  assert.throws(() => parseRegistryDescriptor(JSON.stringify({ Descriptor: { digest } })), /missing/)
})

test('candidate evidence cannot hide a missing service, image source mismatch or publication claim', () => {
  const report = candidateReport('OCI_MANIFEST_DIGEST')
  assert.equal(validateBusinessCandidate(report), report)
  assert.throws(() => validateBusinessCandidate({ ...report, images: report.images.slice(1) }), /six/)
  assert.throws(() => validateBusinessCandidate({ ...report, registryPublication: 'PASS' }), /required checks/)
  assert.throws(() => validateBusinessCandidate(report, { sourceRevision: 'b'.repeat(40) }), /source/)
})

function candidateReport(kind) {
  const manifest = 'sha256:' + 'c'.repeat(64), config = 'sha256:' + 'd'.repeat(64)
  return { schemaVersion: 1, kind: 'business-image-candidate', status: 'PASS', build: 'PASS', registryPublication: 'NOT_REQUESTED', targetDeployment: 'NOT_RUN', currentSourceMatch: true, owner: 'phase28-test', sourceRevision, inputsSha256, targetPlatform: 'linux/amd64', buildId: 'build-1', images: businessImageKeys.map(key => ({ key, role: key.endsWith('-api') ? 'api' : 'static', appCode: key.startsWith('stores') ? 'stores' : 'work-orders', archive: `images/${key}.tar`, imageId: kind === 'OCI_MANIFEST_DIGEST' ? manifest : config, daemonImageIdKind: kind, archiveSha256: 'a'.repeat(64), ociManifestDigest: manifest, ociConfigDigest: config, sourceRevision, inputsSha256, targetPlatform: 'linux/amd64', buildId: 'build-1', imageConfigVerified: true, registryAvailability: 'NOT_VERIFIED_NO_PUSH' })) }
}

test('modern manifest-ID and classic config-ID daemons keep three distinct image identities', () => {
  for (const kind of ['OCI_MANIFEST_DIGEST', 'OCI_CONFIG_DIGEST']) {
    const candidate = candidateReport(kind)
    assert.equal(validateBusinessCandidate(candidate), candidate)
    assert.equal(classifyDaemonImageIdentity(candidate.images[0]), kind)
    const registryDigest = 'sha256:' + 'e'.repeat(64)
    const publication = { schemaVersion: 1, kind: 'business-image-publication', status: 'PASS', images: candidate.images.map(image => ({ ...image,
      repository: `registry.example.test/business/${image.key}`, digest: registryDigest, imageRef: `registry.example.test/business/${image.key}@${registryDigest}`,
      registryConfigDigest: image.ociConfigDigest, registryAvailability: 'VERIFIED_PUSH_AND_REGISTRY_DESCRIPTOR' })) }
    assert.equal(validateBusinessPublication(publication), publication)
    const wrong = structuredClone(publication)
    wrong.images[0].registryConfigDigest = registryDigest
    assert.throws(() => validateBusinessPublication(wrong), /identity/)
  }
  const unknown = { ...candidateReport('OCI_MANIFEST_DIGEST').images[0], imageId: 'sha256:' + 'f'.repeat(64) }
  assert.throws(() => classifyDaemonImageIdentity(unknown), /neither/)
  // Push may change the manifest encoding. The target's actual ID is compared
  // with that registry manifest/config pair, not with the old saved manifest.
  const candidate = candidateReport('OCI_MANIFEST_DIGEST').images[0]
  const registryManifest = 'sha256:' + 'e'.repeat(64)
  assert.notEqual(candidate.imageId, registryManifest)
  assert.equal(classifyDaemonImageIdentity({ imageId: registryManifest, ociManifestDigest: registryManifest, ociConfigDigest: candidate.ociConfigDigest }), 'OCI_MANIFEST_DIGEST')
  assert.equal(classifyDaemonImageIdentity({ imageId: candidate.ociConfigDigest, ociManifestDigest: registryManifest, ociConfigDigest: candidate.ociConfigDigest }), 'OCI_CONFIG_DIGEST')
})

test('fixed digest pull keeps the actual target ID when the registry changes the manifest encoding', async t => {
  const directory = path.resolve(mkdtempSync(path.join(os.tmpdir(), 'phase28-image-identity-')))
  t.after(() => {
    assert.equal(path.dirname(directory), path.resolve(os.tmpdir()))
    assert.ok(path.basename(directory).startsWith('phase28-image-identity-'))
    assert.equal(lstatSync(directory).isSymbolicLink(), false)
    rmSync(directory, { recursive: true })
  })
  const candidate = candidateReport('OCI_MANIFEST_DIGEST'), registryDigest = 'sha256:' + 'e'.repeat(64)
  const publication = { schemaVersion: 1, kind: 'business-image-publication', status: 'PASS', candidateOwner: candidate.owner, sourceRevision, inputsSha256, targetPlatform: 'linux/amd64',
    images: candidate.images.map(image => ({ ...image, repository: `registry.example.test/business/${image.key}`, digest: registryDigest,
      imageRef: `registry.example.test/business/${image.key}@${registryDigest}`, registryConfigDigest: image.ociConfigDigest, registryAvailability: 'VERIFIED_PUSH_AND_REGISTRY_DESCRIPTOR' })) }
  for (const kind of ['OCI_MANIFEST_DIGEST', 'OCI_CONFIG_DIGEST']) {
    const actualId = kind === 'OCI_MANIFEST_DIGEST' ? registryDigest : candidate.images[0].ociConfigDigest
    const docker = async (args, options) => {
      if (options?.allowFailure) return 'Error response from daemon: No such image: absent'
      if (args[0] === 'pull') return 'Actual pull returned'
      return JSON.stringify([{ Id: actualId, RepoDigests: [args[2]], Os: 'linux', Architecture: 'amd64', Config: { Labels: {
        [businessOwnerLabel]: candidate.owner, 'org.opencontainers.image.revision': sourceRevision, 'saas.ci.inputs-sha256': inputsSha256,
      } } }])
    }
    const result = await fixedDigestPull({ docker, publication, outDir: path.join(directory, kind) })
    assert.equal(result.status, 'PASS')
    for (const image of result.images) {
      assert.equal(image.imageId, actualId); assert.equal(image.pulledImageId, actualId)
      assert.equal(image.candidateImageId, candidate.images[0].imageId); assert.equal(image.daemonImageIdKind, kind)
      assert.notEqual(image.imageId, image.candidateImageId)
    }
  }
})

function ownedReferenceModel({ unrelated = false, consumer = false } = {}) {
  const candidate = candidateReport('OCI_MANIFEST_DIGEST')
  const states = []
  candidate.images.forEach((image, index) => {
    image.imageId = image.ociManifestDigest = 'sha256:' + (index + 1).toString(16).padStart(64, '0')
    image.tag = `${candidate.owner}-${image.key}:candidate`
  })
  const publication = { schemaVersion: 1, kind: 'business-image-publication', status: 'PASS', images: candidate.images.map((image, index) => {
    const repository = `localhost:5000/business/${image.key}`, registryDigest = 'sha256:' + (index + 11).toString(16).padStart(64, '0')
    const published = { ...image, repository, digest: registryDigest, imageRef: `${repository}@${registryDigest}`, publicationTag: `${repository}:build-1`, registryConfigDigest: image.ociConfigDigest, registryAvailability: 'VERIFIED_PUSH_AND_REGISTRY_DESCRIPTOR' }
    const localDigest = image.tag.slice(0, image.tag.lastIndexOf(':')) + '@' + image.ociManifestDigest
    states.push({ Id: image.imageId, Config: { Labels: { [businessOwnerLabel]: candidate.owner } }, RepoTags: [image.tag, published.publicationTag, ...(unrelated && index === 0 ? ['another-owner:production'] : [])], RepoDigests: [localDigest, published.imageRef], present: true })
    return published
  }) }
  const calls = []
  const docker = async args => {
    calls.push(args)
    if (args[0] === 'container') return consumer ? 'container-still-using-image' : ''
    const reference = args[2], state = states.find(value => value.present && (value.Id === reference || value.RepoTags.includes(reference) || value.RepoDigests.includes(reference)))
    if (!state) return 'Error response from daemon: No such image: missing-reference'
    if (args[1] === 'inspect') return JSON.stringify([state])
    assert.equal(args[1], 'rm')
    state.RepoTags = state.RepoTags.filter(value => value !== reference)
    state.RepoDigests = state.RepoDigests.filter(value => value !== reference && !(reference.includes(':') && !reference.includes('@') && value.startsWith(reference.slice(0, reference.lastIndexOf(':')) + '@')))
    if (!state.RepoTags.length && !state.RepoDigests.length) state.present = false
    return 'Untagged owned reference'
  }
  return { candidate, publication, docker, calls, states }
}

test('owned image removal accepts only the exact local candidate digest and tolerates auto-removed digests', async () => {
  const model = ownedReferenceModel()
  await forgetOwnedBusinessImages(model)
  assert.ok(model.states.every(state => state.present === false))
  assert.equal(model.calls.filter(args => args[0] === 'image' && args[1] === 'rm').length, 12)
  for (const options of [{ unrelated: true }, { consumer: true }]) {
    const blocked = ownedReferenceModel(options)
    await assert.rejects(forgetOwnedBusinessImages(blocked), /unrelated reference|container consumer/)
    assert.equal(blocked.calls.some(args => args[0] === 'image' && args[1] === 'rm'), false)
  }
})

function githubCandidate() {
  const name = candidateArtifactName({ sourceRevision, runId: 123, runAttempt: 2 })
  return { repository: 'example/repository', repositoryId: 987, runId: 123, runAttempt: 2,
    run: { id: 123, run_attempt: 2, repository: { full_name: 'example/repository', id: 987 }, head_repository: { full_name: 'example/repository', id: 987 }, path: '.github/workflows/production-build.yml', head_branch: 'main', event: 'push', status: 'completed', conclusion: 'success', head_sha: sourceRevision },
    jobs: ['Required candidate gate', 'Business images, owned registry and versioned HTTPS routes'].map(name => ({ name, status: 'completed', conclusion: 'success', run_id: 123, run_attempt: 2 })),
    artifacts: [{ id: 456, name, expired: false, size_in_bytes: 100, workflow_run: { id: 123, repository_id: 987, head_repository_id: 987, head_sha: sourceRevision, head_branch: 'main' } }] }
}

test('protected publication selects only the exact successful managed main run and attempt', () => {
  const candidate = githubCandidate()
  assert.equal(validateCandidateRun(candidate).artifactId, '456')
  for (const mutation of [{ event: 'pull_request' }, { head_branch: 'feature' }, { run_attempt: 1 }, { conclusion: 'skipped' }, { path: '.github/workflows/other.yml' }, { head_repository: { full_name: 'fork/repository' } }]) assert.throws(() => validateCandidateRun({ ...candidate, run: { ...candidate.run, ...mutation } }))
  assert.throws(() => validateCandidateRun({ ...candidate, jobs: candidate.jobs.map(job => ({ ...job, conclusion: 'skipped' })) }))
  assert.throws(() => validateCandidateRun({ ...candidate, artifacts: [{ ...candidate.artifacts[0], expired: true }] }))
  assert.throws(() => validateCandidateRun({ ...candidate, artifacts: [...candidate.artifacts, ...candidate.artifacts] }))
})
