import assert from 'node:assert/strict'
import { chownSync, lstatSync, realpathSync } from 'node:fs'
import path from 'node:path'
import { createSourceManifest } from './provenance.mjs'
import { validateOwnedPostgres } from './postgres.mjs'

// This core accepts an explicit operation for policy tests. The managed entry
// below supplies only the real current process identity and the copied snapshot.
export function normalizePublicManifestOwnership({ root, manifest, uid, gid, chown = chownSync }) {
  assert.ok(Number.isSafeInteger(uid) && uid >= 0 && Number.isSafeInteger(gid) && gid >= 0, 'Actual process UID and GID are required')
  root = path.resolve(root)
  assert.equal(realpathSync(root), root, 'Source root cannot have symbolic-link ancestors')
  const before = createSourceManifest(root, { sourceRevision: manifest.sourceRevision })
  assert.deepEqual(before.files, manifest.files, 'Only the exact current public source manifest may be normalized')
  assert.equal(before.inputsSha256, manifest.inputsSha256)
  const directories = new Set([root]), files = []
  for (const file of before.files) {
    assert.ok(file.path && !path.isAbsolute(file.path) && !file.path.includes('\\') && file.path.split('/').every(part => part && part !== '.' && part !== '..'), 'Public source path must remain inside the snapshot')
    const absolute = path.join(root, file.path)
    assert.ok(absolute.startsWith(root + path.sep), 'Public source path must remain inside the snapshot')
    for (let directory = path.dirname(absolute); directory !== root; directory = path.dirname(directory)) {
      assert.ok(directory.startsWith(root + path.sep), 'Source ancestor must remain inside the snapshot')
      directories.add(directory)
    }
    files.push(absolute)
  }
  const targets = [...[...directories].sort((a, b) => a.length - b.length || a.localeCompare(b)).map(file => ({ file, directory: true })), ...files.map(file => ({ file, directory: false }))]
  // Validate every target before changing any metadata. No recursive chown,
  // chmod, generated output, dependency cache or unlisted file is included.
  for (const target of targets) {
    const stat = lstatSync(target.file)
    assert.ok(!stat.isSymbolicLink() && (target.directory ? stat.isDirectory() : stat.isFile()), 'Ownership target must be an ordinary source file or directory')
    target.mode = stat.mode; target.changed = stat.uid !== uid || stat.gid !== gid
  }
  for (const target of targets) {
    const current = lstatSync(target.file)
    assert.ok(!current.isSymbolicLink() && (target.directory ? current.isDirectory() : current.isFile()), 'Ownership target type changed')
    chown(target.file, uid, gid)
    const after = lstatSync(target.file)
    assert.ok(!after.isSymbolicLink() && after.uid === uid && after.gid === gid, 'Public source ownership normalization did not take effect')
    assert.equal(after.mode, target.mode, 'Source permission modes must remain unchanged')
  }
  const after = createSourceManifest(root, { sourceRevision: before.sourceRevision })
  assert.deepEqual(after.files, before.files, 'Ownership normalization cannot change source bytes')
  assert.equal(after.inputsSha256, before.inputsSha256, 'Ownership normalization cannot change source inputs')
  return { schemaVersion: 1, kind: 'owned-browser-public-source-ownership', status: 'PASS', sourceRoot: root,
    sourceRevision: before.sourceRevision, inputsBefore: before.inputsSha256, inputsAfter: after.inputsSha256,
    uid, gid, files: files.length, directories: directories.size,
    changedFiles: targets.filter(target => !target.directory && target.changed).length,
    changedDirectories: targets.filter(target => target.directory && target.changed).length,
    sourceUnchanged: true, ownershipVerified: true, modeUnchanged: true }
}

export function normalizeBrowserSourceOwnership({ root = '/workspace', sourceRevision, expectedInputsSha256, env = process.env } = {}) {
  assert.equal(process.platform, 'linux', 'Ownership normalization is restricted to the managed Linux browser snapshot')
  assert.equal(env.CI, 'true'); assert.equal(env.CI_MANAGED_BROWSER_SOURCE_ROOT, '/workspace', 'An explicit managed browser snapshot is required')
  root = path.resolve(root)
  assert.equal(root, '/workspace', 'Ownership normalization is confined to the copied /workspace snapshot')
  const ownership = validateOwnedPostgres(env)
  assert.equal(new URL(env.TEST_DATABASE_URL).hash, '')
  const manifest = createSourceManifest(root, { sourceRevision: sourceRevision || env.CI_SOURCE_REVISION })
  assert.match(expectedInputsSha256 || '', /^[a-f0-9]{64}$/, 'Expected frozen source inputs are required')
  assert.equal(manifest.inputsSha256, expectedInputsSha256, 'Managed browser source must match its frozen inputs')
  return { ...normalizePublicManifestOwnership({ root, manifest, uid: process.getuid(), gid: process.getgid() }), owner: ownership.owner }
}
