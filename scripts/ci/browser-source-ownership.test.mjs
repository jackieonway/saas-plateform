import assert from 'node:assert/strict'
import { lstatSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { normalizeBrowserSourceOwnership, normalizePublicManifestOwnership } from './browser-source-ownership.mjs'
import { createSourceManifest } from './provenance.mjs'

const revision = 'a'.repeat(40)
function fixture(t) {
  const parent = realpathSync(os.tmpdir()), directory = realpathSync(mkdtempSync(path.join(parent, 'saas-browser-ownership-'))), root = path.join(directory, 'source')
  t.after(() => { assert.equal(path.dirname(realpathSync(directory)), parent); assert.match(path.basename(directory), /^saas-browser-ownership-/); assert(!lstatSync(directory).isSymbolicLink()); rmSync(directory, { recursive: true, force: true }) })
  const put = (file, contents) => { const absolute = path.join(root, file); mkdirSync(path.dirname(absolute), { recursive: true }); writeFileSync(absolute, contents) }
  put('package.json', '{}'); put('frontend/public/logo.svg', '<svg/>'); put('scripts/ci/reviewed.mjs', 'reviewed source')
  put('frontend/.env', 'private'); put('frontend/node_modules/dependency.js', 'generated'); put('frontend/dist/output.js', 'generated'); put('private/secret.txt', 'private')
  const manifest = createSourceManifest(root, { sourceRevision: revision }), stat = lstatSync(root)
  return { directory, root, put, manifest, uid: stat.uid, gid: stat.gid }
}

test('ownership normalization visits only exact public files and all owned ancestors while preserving bytes and modes', t => {
  const f = fixture(t), calls = []
  const proof = normalizePublicManifestOwnership({ ...f, chown: (file, uid, gid) => calls.push({ file, uid, gid }) })
  assert.deepEqual(new Set(calls.map(item => path.relative(f.root, item.file).replaceAll('\\', '/'))), new Set(['', 'frontend', 'frontend/public', 'scripts', 'scripts/ci', 'package.json', 'frontend/public/logo.svg', 'scripts/ci/reviewed.mjs']))
  assert.ok(calls.every(item => item.uid === f.uid && item.gid === f.gid))
  assert.ok(calls.every(item => item.file === f.root || item.file.startsWith(f.root + path.sep)))
  assert.equal(proof.files, 3); assert.equal(proof.directories, 5)
  assert.equal(proof.inputsBefore, f.manifest.inputsSha256); assert.equal(proof.inputsAfter, f.manifest.inputsSha256)
  assert.equal(proof.sourceUnchanged, true); assert.equal(proof.ownershipVerified, true); assert.equal(proof.modeUnchanged, true)
  assert.equal(readFileSync(path.join(f.root, 'frontend/.env'), 'utf8'), 'private')
})

test('stale or extended manifests cannot cause metadata changes outside the exact public source selection', t => {
  const f = fixture(t)
  for (const manifest of [
    { ...f.manifest, inputsSha256: 'b'.repeat(64) },
    { ...f.manifest, files: [...f.manifest.files, { path: '../outside', bytes: 1, sha256: 'b'.repeat(64) }] },
    { ...f.manifest, files: [...f.manifest.files, { path: 'frontend/.env', bytes: 7, sha256: 'b'.repeat(64) }] },
  ]) assert.throws(() => normalizePublicManifestOwnership({ ...f, manifest, chown: () => assert.fail('manifest must be checked before changing ownership') }))
  f.put('package.json', '{"changed":true}')
  assert.throws(() => normalizePublicManifestOwnership({ ...f, chown: () => assert.fail('stale inputs must not be changed') }), /exact current public source/)
})

test('a symlinked public source ancestor is rejected before any ownership operation', t => {
  const f = fixture(t), outside = path.join(f.directory, 'outside')
  mkdirSync(outside); writeFileSync(path.join(outside, 'source.mjs'), 'outside source')
  symlinkSync(outside, path.join(f.root, 'frontend/src'), 'junction')
  assert.throws(() => normalizePublicManifestOwnership({ ...f, chown: () => assert.fail('symlink must be rejected before ownership changes') }), /symlink refused/)
  assert.equal(readFileSync(path.join(outside, 'source.mjs'), 'utf8'), 'outside source')
})

test('a byte mutation or failed metadata operation prevents a successful ownership proof', t => {
  const f = fixture(t)
  assert.throws(() => normalizePublicManifestOwnership({ ...f, chown: () => { throw new Error('controlled ownership failure') } }), /controlled ownership failure/)
  let changed = false
  assert.throws(() => normalizePublicManifestOwnership({ ...f, chown: () => {
    if (!changed) { changed = true; f.put('package.json', '{"mutation":true}') }
  } }), /cannot change source bytes/)
})

test('the managed entry refuses ordinary workspaces and runs in browser before dependency installation', t => {
  const f = fixture(t), env = { CI: 'true', CI_MANAGED_BROWSER_SOURCE_ROOT: '/workspace',
    CI_TEST_POSTGRES_OWNER: 'ci-12345678', TEST_DATABASE_URL: 'postgresql://ci:synthetic@ci-12345678-postgres:5432/postgres' }
  assert.throws(() => normalizeBrowserSourceOwnership({ root: f.root, sourceRevision: revision, expectedInputsSha256: f.manifest.inputsSha256, env }), /managed Linux|confined/)
  const runner = readFileSync(new URL('./run.mjs', import.meta.url), 'utf8')
  assert.match(runner, /if \(options\.gate === 'browser'\) \{\s+summary\.sourceOwnership = normalizeBrowserSourceOwnership/)
  assert.ok(runner.indexOf('summary.sourceOwnership = normalizeBrowserSourceOwnership') < runner.indexOf('const env = commandEnvironment()'))
  assert.equal((runner.match(/summary\.sourceOwnership = normalizeBrowserSourceOwnership/g) || []).length, 1)
})
