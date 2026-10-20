import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { mkdtempSync, mkdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import { gates, ciUnitTests, contractTests, postgresSuites } from '../scripts/ci/manifest.mjs'
import { createSourceManifest, excluded } from '../scripts/ci/provenance.mjs'
import { assertBusinessPublishWorkflowPolicy, assertWorkflowPolicy } from '../scripts/ci/workflow-policy.mjs'
import { evaluate } from '../scripts/ci/report.mjs'
import { parseArgs } from '../scripts/ci/run.mjs'
import { businessBrowserChecks, businessBrowserProducts, businessDeliveryChecks } from '../scripts/ci/business-delivery-results.mjs'
import { candidateArtifactName, validateCandidateRun } from '../scripts/ci/business-images-publish.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'scripts/ci/package.json'))
const { parseDocument } = require('yaml')
const revision = 'a'.repeat(40)
function yaml(file) {
  const document = parseDocument(readFileSync(path.join(root, file), 'utf8'), { uniqueKeys: true, version: '1.2' })
  assert.deepEqual(document.errors, [])
  return document.toJS()
}
function temporary(t) {
  const parent = realpathSync(os.tmpdir()), directory = mkdtempSync(path.join(parent, 'saas-phase28-ci-'))
  t.after(() => {
    const resolved = realpathSync(directory)
    assert.equal(path.dirname(resolved), parent)
    assert.match(path.basename(resolved), /^saas-phase28-ci-[A-Za-z0-9]{6}$/)
    rmSync(resolved, { recursive: true })
  })
  return directory
}
function put(directory, file, bytes = 'reviewed source') {
  const target = path.join(directory, file)
  mkdirSync(path.dirname(target), { recursive: true })
  writeFileSync(target, bytes)
}

test('business image delivery is mandatory for candidates and preserves the existing owned fixture boundary', () => {
  assert.equal(gates.filter(gate => gate === 'business-delivery').length, 1)
  assert.ok(ciUnitTests.includes('tests/phase28-ci-policy.mjs'))
  assert.ok(contractTests.includes('tests/application-release-policy.mjs'))
  const runner = readFileSync(path.join(root, 'scripts/ci/run.mjs'), 'utf8')
  const branch = runner.split("} else if (options.gate === 'business-delivery') {")[1].split('} else if')[0]
  assert.match(branch, /'scripts\/ci\/business-delivery\.mjs'/)
  assert.match(branch, /format:'business-delivery'/)
  assert.match(branch, /expectedSourceRevision:source\.sourceRevision,expectedInputsSha256:source\.inputsSha256/)
  assert.match(branch, /CI_TOOLCHAIN_IMAGE:process\.env\.CI_TOOLCHAIN_IMAGE/)
  assert.match(branch, /env:businessEnv/)
  assert.doesNotMatch(branch, /TEST_DATABASE_URL|CI_TEST_POSTGRES_OWNER|liveEnv|pgEnv/)
  const browser = runner.split("} else if (options.gate === 'browser') {")[1].split('} else if')[0]
  assert.doesNotMatch(browser.split('const liveEnv')[0], /TEST_DATABASE_URL|CI_TEST_POSTGRES_OWNER/)
  for (const id of ['browser','notification-browser-live','data-grant-ui-live','work-orders-browser-live','stores-browser-live']) {
    assert.equal((browser.match(new RegExp("step\\('" + id + "'", 'g')) || []).length, 1)
  }
  assert.equal(postgresSuites.length, 12)
  assert.equal(postgresSuites.reduce((sum, suite) => sum + suite.expected, 0), 76)
  const args = ['--gate','business-delivery','--out-dir',path.join(os.tmpdir(),'new-phase28-evidence')]
  assert.equal(parseArgs(args).platform, 'linux/amd64')
  assert.throws(() => parseArgs([...args,'--platform','linux/arm64']), /requires the verified linux\/amd64/)
})

test('candidate workflow rejects bypassed business dependencies, private uploads and publication credentials', () => {
  const workflow = yaml('.github/workflows/production-build.yml'), tools = yaml('.github/actions/ci-tools/action.yml')
  assert.equal(assertWorkflowPolicy(workflow, tools), true)
  for (const change of [
    copy => { delete copy.jobs['business-delivery'] },
    copy => { copy.jobs['business-delivery'].needs = ['fast'] },
    copy => { copy.jobs['business-delivery'].if = 'always()' },
    copy => { copy.jobs['candidate-gate'].needs = copy.jobs['candidate-gate'].needs.filter(name => name !== 'business-delivery') },
    copy => { copy.jobs['business-delivery']['continue-on-error'] = true },
    copy => { copy.jobs['business-delivery'].steps = copy.jobs['business-delivery'].steps.filter(step => step.name !== 'Prepare isolated business browser tools and declared base images') },
    copy => { copy.jobs['business-delivery'].steps.find(step => step.name === 'Prepare isolated business browser tools and declared base images').run = 'docker pull registry:latest' },
    copy => { delete copy.jobs['business-delivery'].steps.find(step => step.name === 'Verify business candidates with owned registry and real versioned routes').env.CI_TOOLCHAIN_IMAGE },
    copy => { copy.jobs['business-delivery'].steps.find(step => step.name === 'Save verified business candidate images').if = 'always()' },
    copy => { copy.jobs['business-delivery'].steps.find(step => step.name === 'Save verified business candidate images').with.name = 'latest-business-candidate' },
    copy => { copy.jobs['business-delivery'].steps.find(step => step.name === 'Save sanitized business delivery report').with.path += '${{ runner.temp }}/ci/business-delivery/business-delivery/private/ca.key\n' },
    copy => { copy.jobs['business-delivery'].steps.find(step => step.name === 'Save sanitized business delivery report').with.path += '${{ runner.temp }}/ci/business-delivery/business-delivery/work/logs/\n' },
    copy => { const step = copy.jobs['business-delivery'].steps.find(step => step.name === 'Save sanitized business delivery report'); step.with.path = step.with.path.replace('${{ runner.temp }}/ci/business-delivery/business-delivery/browser-chrome/fixture-result.json\n', '') },
    copy => { copy.jobs['business-delivery'].steps.find(step => step.name === 'Save sanitized business delivery report').with.path += '${{ runner.temp }}/ci/business-delivery/business-delivery/browser-chrome/\n' },
    copy => { copy.jobs['business-delivery'].steps.find(step => step.name === 'Save sanitized business delivery report').with.path += '${{ runner.temp }}/ci/business-delivery/business-delivery/artifacts/original-binaries/*\n' },
    copy => { copy.jobs['business-delivery'].steps.find(step => step.name === 'Save sanitized business delivery report').with.path += '${{ runner.temp }}/ci/business-delivery/business-delivery/artifacts/static/**\n' },
    copy => { copy.jobs['business-delivery'].steps.find(step => step.name === 'Save sanitized business delivery report').with.path += '${{ runner.temp }}/ci/business-delivery/business-delivery/artifacts/artifact-inventory/*\n' },
    copy => { copy.jobs['business-delivery'].steps.find(step => step.name === 'Save verified business candidate images').with.path += '${{ runner.temp }}/ci/business-delivery/business-delivery/candidate/source/\n' },
    copy => { copy.jobs['business-delivery'].steps.push({ run: 'docker push registry.invalid/app:latest' }) },
    copy => { copy.jobs['business-delivery'].steps.push({ uses: 'docker/login-action@v3' }) },
    copy => { copy.jobs['business-delivery'].env = { REGISTRY_PASSWORD: '${{ secrets.REGISTRY_PASSWORD }}' } },
    copy => { copy.jobs['business-delivery'].permissions = { contents: 'read', packages: 'write' } },
  ]) {
    const copy = structuredClone(workflow)
    change(copy)
    assert.throws(() => assertWorkflowPolicy(copy, tools))
  }
})

test('business source provenance includes both Rust Dockerfiles and only the explicitly public environment example', t => {
  const directory = temporary(t)
  const selected = [
    'business-apps/work-orders/backend/Dockerfile', 'business-apps/stores/backend/Dockerfile',
    'deploy/microapps/Dockerfile', 'deploy/microapps/gateway.example.json',
    'deploy/compose.business.yml', 'deploy/compose.business-ca.yml', 'deploy/compose.business.env.example',
    '.github/workflows/business-publish.yml', 'scripts/ci/business-delivery.mjs', 'scripts/ci/business-images.mjs',
  ]
  for (const file of selected) put(directory, file)
  for (const file of [
    'deploy/.env', 'deploy/compose.business.env', 'deploy/compose.business.env.production',
    'deploy/another.env.example', 'deploy/microapps/private/credentials.json', 'deploy/microapps/runtime.json',
    'business-apps/stores/backend/.env', 'business-apps/work-orders/backend/target/release/api',
    'business-apps/stores/deploy/unreviewed.json', 'deploy/microapps/tls/ca.key',
  ]) put(directory, file, 'must remain private')
  assert.deepEqual(createSourceManifest(directory, { sourceRevision: revision }).files.map(file => file.path).sort(), [...selected].sort())
  assert.equal(excluded('deploy/compose.business.env.example'), false)
  for (const file of ['deploy/compose.business.env', 'deploy/other.env.example', 'deploy/.env.example']) assert.equal(excluded(file), true)
})

function deliveryReport() {
  const inputsSha256 = 'b'.repeat(64), digest = 'sha256:' + 'c'.repeat(64)
  return {
    schemaVersion: 1, kind: 'business-delivery', status: 'PASS', sourceRevision: revision, inputsSha256,
    targetPlatform: 'linux/amd64', sourceUnchanged: true,
    checks: businessDeliveryChecks.map(id => ({ id, status: 'PASS' })),
    browserProducts: businessBrowserProducts.map(product => ({ product, status: 'PASS', sourceRevision: revision, inputsSha256, normalTLS: true,
      checks: businessBrowserChecks.map(id => ({ id, status: 'PASS' })), cleanup: { databaseConnectionsClosed: true, browserClosed: true } })),
    registry: { scope: 'isolated', push: 'PASS', pull: 'PASS', candidateAbsentBeforePull: true, registryRequestsObserved: true, productionPublication: false },
    images: ['work-orders-v1', 'work-orders-v2', 'stores-v1', 'stores-v2', 'work-orders-api', 'stores-api'].map(key => ({ key, imageId: digest,
      reference: 'localhost:5000/' + key + '@' + digest, digest, sourceRevision: revision, inputsSha256, buildId: 'phase28-test-build' })),
    unhealthy: { status: 'BLOCKED', calls: { routes: 0, register: 0, activate: 0 } },
    cleanup: { ownedContainersRemoved: true, ownedNetworksRemoved: true, ownedVolumesRemoved: true, privateWorkRemoved: true, errors: [] },
  }
}

test('central business result evaluation rejects stale, incomplete or skipped real delivery evidence even after exit zero', () => {
  const valid = deliveryReport()
  const verdict = (document, extra = {}) => evaluate({ format: 'business-delivery', exitCode: 0, document,
    expectedSourceRevision: revision, expectedInputsSha256: valid.inputsSha256, ...extra })
  assert.equal(verdict(valid).status, 'PASS')
  assert.equal(verdict(valid).counts.pass, businessDeliveryChecks.length)
  for (const change of [
    copy => { copy.sourceRevision = 'd'.repeat(40) }, copy => { copy.inputsSha256 = 'e'.repeat(64) },
    copy => { copy.checks = [] }, copy => { copy.checks[0].status = 'SKIP' },
    copy => { copy.checks[0].status = 'NOT_RUN' }, copy => { copy.browserProducts.pop() },
    copy => { copy.cleanup.privateWorkRemoved = false }, copy => { copy.registry.pull = 'NOT_RUN' },
    copy => { delete copy.cleanup.ownedVolumesRemoved }, copy => { copy.cleanup.ownedVolumesRemoved = false },
    copy => { copy.unhealthy.calls.activate = 1 },
    copy => { copy.images[0].key = 'unexpected-image' },
    copy => { copy.images[0].sourceRevision = 'd'.repeat(40) },
    copy => { copy.images[0].digest = 'sha256:' + 'd'.repeat(64) },
  ]) {
    const copy = structuredClone(valid)
    change(copy)
    assert.equal(verdict(copy).status, 'FAIL')
  }
  for (const document of [undefined, null, {}, { ...valid, checks: null }]) assert.equal(verdict(document).status, 'FAIL')
  assert.equal(verdict(valid, { exitCode: 1 }).status, 'FAIL')
  assert.equal(verdict(valid, { timedOut: true }).status, 'FAIL')
})

test('actual publication workflow is manual main-only, environment-protected and verifies immutable archives before login', () => {
  const workflow = yaml('.github/workflows/business-publish.yml')
  assert.equal(assertBusinessPublishWorkflowPolicy(workflow), true)
  for (const change of [
    copy => { copy.on.push = {} }, copy => { copy.on.workflow_run = {} },
    copy => { copy.jobs.verify.if = "github.ref == 'refs/heads/feature'" },
    copy => { copy.jobs.publish.if = 'always()' }, copy => { delete copy.jobs.publish.environment },
    copy => { copy.jobs.publish.environment = '${{ inputs.environment }}' },
    copy => { copy.permissions.actions = 'write' },
    copy => { copy.jobs.verify.env = { TOKEN: '${{ secrets.REGISTRY_TOKEN }}' } },
    copy => { copy.jobs.publish.env = { TOKEN: '${{ secrets.REGISTRY_TOKEN }}' } },
    copy => { copy.jobs.extra = { ...structuredClone(copy.jobs.publish), environment: undefined } },
    copy => { copy.jobs.publish.needs = [] },
    copy => { copy.jobs.verify.outputs.artifact_id = '${{ inputs.artifact_id }}' },
    copy => { copy.jobs.verify.steps.find(step => step.id === 'candidate').if = 'false' },
    copy => { copy.jobs.verify.steps.find(step => step.id === 'candidate').run += '${{ inputs.candidate_run_id }}' },
    copy => { copy.jobs.publish.steps.find(step => step.uses?.startsWith('actions/checkout@')).with.ref = '${{ inputs.source_sha }}' },
    copy => { copy.jobs.publish.steps.find(step => step.uses?.startsWith('actions/checkout@')).with['persist-credentials'] = true },
    copy => { copy.jobs.publish.steps.find(step => step.uses?.startsWith('actions/download-artifact@')).with['run-id'] = '${{ inputs.candidate_run_id }}' },
    copy => { copy.jobs.publish.steps.find(step => step.uses?.startsWith('actions/download-artifact@')).with['artifact-ids'] = '${{ inputs.artifact_id }}' },
    copy => { copy.jobs.publish.steps = copy.jobs.publish.steps.filter(step => !step.run?.includes('--verify-candidate')) },
    copy => { const index = copy.jobs.publish.steps.findIndex(step => step.run?.includes('--verify-candidate')); copy.jobs.publish.steps.push(copy.jobs.publish.steps.splice(index, 1)[0]) },
    copy => { copy.jobs.publish.steps.find(step => step.name === 'Docker Login approved registry').if = 'always()' },
    copy => { copy.jobs.publish.steps.find(step => step.run?.includes('--publish'))['continue-on-error'] = true },
    copy => { copy.jobs.publish.steps.push({ run: 'docker push registry.invalid/unchecked:latest' }) },
    copy => { copy.jobs.publish.steps.find(step => step.uses?.startsWith('actions/upload-artifact@')).with.path += '${{ runner.temp }}/business-publication/work/logs/\n' },
  ]) {
    const copy = structuredClone(workflow)
    change(copy)
    assert.throws(() => assertBusinessPublishWorkflowPolicy(copy))
  }
})

function trustedCandidateRun() {
  const repository = 'example/rust-saas', repositoryId = 100, runId = 42, runAttempt = 2
  const artifactName = candidateArtifactName({ sourceRevision: revision, runId, runAttempt })
  return { repository, repositoryId, runId, runAttempt,
    run: { id: runId, run_attempt: runAttempt, repository: { id: repositoryId, full_name: repository }, head_repository: { id: repositoryId, full_name: repository },
      path: '.github/workflows/production-build.yml', head_branch: 'main', event: 'push', status: 'completed', conclusion: 'success', head_sha: revision },
    jobs: ['Required candidate gate', 'Business images, owned registry and versioned HTTPS routes'].map(name => ({ name, run_id: runId, run_attempt: runAttempt, status: 'completed', conclusion: 'success' })),
    artifacts: [{ id: 99, name: artifactName, size_in_bytes: 123, expired: false,
      workflow_run: { id: runId, repository_id: repositoryId, head_repository_id: repositoryId, head_sha: revision, head_branch: 'main' } }],
  }
}

test('actual candidate API verifier rejects forks, wrong workflow/source/attempt/jobs and artifact substitutions', () => {
  const valid = trustedCandidateRun()
  assert.equal(validateCandidateRun(valid).sourceRevision, revision)
  assert.equal(validateCandidateRun(valid).artifactId, '99')
  for (const change of [
    copy => { copy.repositoryId = 101 }, copy => { delete copy.repositoryId },
    copy => { copy.run.repository.id = 101 }, copy => { copy.run.head_repository.id = 101 },
    copy => { copy.run.repository.full_name = 'other/repo' }, copy => { copy.run.head_repository.full_name = 'fork/repo' },
    copy => { copy.run.event = 'pull_request' }, copy => { copy.run.head_branch = 'feature' },
    copy => { copy.run.path = '.github/workflows/other.yml' }, copy => { copy.run.run_attempt = 1 },
    copy => { copy.run.id = 41 }, copy => { copy.run.head_sha = 'tag-latest' },
    copy => { copy.run.status = 'in_progress' }, copy => { copy.run.conclusion = 'failure' },
    copy => { copy.jobs.pop() }, copy => { copy.jobs[0].run_id = 41 },
    copy => { copy.jobs[0].run_attempt = 1 }, copy => { copy.jobs[1].conclusion = 'skipped' },
    copy => { copy.artifacts.push(copy.artifacts[0]) }, copy => { copy.artifacts[0].expired = true },
    copy => { copy.artifacts[0].size_in_bytes = 0 }, copy => { copy.artifacts[0].name = 'latest-business-candidate' },
    copy => { delete copy.artifacts[0].workflow_run }, copy => { copy.artifacts[0].workflow_run.id = 41 },
    copy => { copy.artifacts[0].workflow_run.head_sha = 'd'.repeat(40) },
    copy => { copy.artifacts[0].workflow_run.repository_id = 101 },
    copy => { copy.artifacts[0].workflow_run.head_repository_id = 101 },
  ]) {
    const copy = structuredClone(valid)
    change(copy)
    assert.throws(() => validateCandidateRun(copy))
  }
})
