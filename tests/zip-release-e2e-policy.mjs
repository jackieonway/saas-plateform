import assert from 'node:assert/strict'
import path from 'node:path'
import os from 'node:os'
import test from 'node:test'
import { spawn } from 'node:child_process'
import { parseArgs, requiredCheckIds, startWslLease, validateReleaseCandidate } from './zip-release-e2e.mjs'

const commit = 'a'.repeat(40), inputsSha256 = 'b'.repeat(64), id = `sha256:${'c'.repeat(64)}`
function candidate() {
  return { status: 'PASS', build: 'PASS', smoke: 'PASS', commit, inputsSha256, buildId: 'candidate-fixture', targetPlatform: 'linux/amd64',
    images: ['frontend','edge','platform'].map(name => ({ name, imageId: id, ociManifestDigest: id, ociConfigDigest: id, commit, inputsSha256, buildId: 'candidate-fixture', targetPlatform: 'linux/amd64', archive: `images/${name}.tar`, archiveSha256: 'd'.repeat(64), archiveBytes: 1024,
      ...(name === 'platform' ? { runtimeVerification: { binarySha256: 'e'.repeat(64), uid: 10001, elfMachine: 62, targetPlatform: 'linux/amd64', sharedLibrariesResolved: true, noServiceStarted: true, network: 'none', readOnlyRootfs: true } } : {}) })) }
}

test('ZIP gate accepts explicit manual immutable images or one candidate report, with mutually exclusive modes', () => {
  const output = path.resolve(os.tmpdir(), 'fresh-zip-evidence'), report = path.resolve(os.tmpdir(), 'release-result.json')
  assert.equal(parseArgs(['--out-dir',output,'--release-report',report]).releaseReport, report)
  assert.equal(parseArgs(['--out-dir',output,'--frontend-image',id,'--edge-image',id]).frontendImage, id)
  for (const args of [[], ['--out-dir','relative','--release-report',report], ['--out-dir',output,'--release-report','relative'],
    ['--out-dir',output,'--release-report',report,'--frontend-image',id], ['--out-dir',output,'--frontend-image','frontend:latest','--edge-image',id],
    ['--out-dir',output,'--frontend-image',id,'--edge-image',id,'--build-proxy','http://user:private@127.0.0.1:7897'],
    ['--out-dir',output,'--release-report',report,'--release-report',report]]) assert.throws(() => parseArgs(args))
})

test('ZIP candidate provenance refuses a different commit or production source hash', () => {
  assert.equal(validateReleaseCandidate(candidate(), { sourceRevision: commit, inputsSha256 }).status, 'PASS')
  assert.throws(() => validateReleaseCandidate(candidate(), { sourceRevision: 'f'.repeat(40), inputsSha256 }), /source revision/)
  assert.throws(() => validateReleaseCandidate(candidate(), { sourceRevision: commit, inputsSha256: 'f'.repeat(64) }), /snapshot hash/)
  const changed = candidate(); changed.images[0].inputsSha256 = 'f'.repeat(64)
  assert.throws(() => validateReleaseCandidate(changed, { sourceRevision: commit, inputsSha256 }), /inconsistent/)
})

test('ZIP candidate gate refuses failed or missing image, immutable identity, archive or actual ELF metadata', () => {
  const cases = [
    value => { value.status = 'FAIL' }, value => { value.smoke = 'NOT_RUN' }, value => { value.imageExport = { cleanup: 'FAIL' } },
    value => { value.images.pop() }, value => { value.images[1].name = 'frontend' }, value => { value.images[0].status = 'FAIL' },
    value => { value.images[0].imageId = 'frontend:latest' }, value => { value.images[1].ociManifestDigest = '' }, value => { delete value.images[0].archiveSha256 },
    value => { value.images[0].archive = '../operator.tar' }, value => { delete value.images[2].runtimeVerification }, value => { value.images[2].runtimeVerification.uid = 0 },
  ]
  for (const mutate of cases) { const value = candidate(); mutate(value); assert.throws(() => validateReleaseCandidate(value, { sourceRevision: commit, inputsSha256 })) }
})

test('ZIP acceptance exports a nonempty unique mandatory check list for candidate CI skip enforcement', () => {
  assert.equal(requiredCheckIds.length, 9); assert.equal(new Set(requiredCheckIds).size, requiredCheckIds.length)
  for (const id of ['response-recovery','parallel-idempotent','parallel-conflict','activate-rollback','private-metadata']) assert(requiredCheckIds.includes(id))
})

test('owned foreground lease releases on stdin EOF and cleanup is idempotent', async () => {
  let unexpected = false
  const lease = await startWslLease({ owner: 'unit-owned-lease', onUnexpectedExit: () => { unexpected = true }, spawnProcess(_program, args, options) {
    return spawn(process.execPath, ['-e','process.stdout.write(process.argv[1]+"\\n");process.stdin.resume();process.stdin.on("end",()=>process.exit(0))',args.at(-1)], options)
  } })
  assert.equal(lease.evidence.ready, true)
  await Promise.all([lease.stop(), lease.stop()])
  assert.equal(lease.evidence.stopped, true); assert.equal(lease.evidence.exitCode, 0); assert.equal(unexpected, false)
})
