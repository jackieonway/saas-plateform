import assert from 'node:assert/strict'
import { readFileSync, readdirSync, lstatSync } from 'node:fs'
import { createHash } from 'node:crypto'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
const defaultRoot = fileURLToPath(new URL('../', import.meta.url))
function checkedPath(root, relative) {
 assert.ok(!path.isAbsolute(relative) && !relative.split('/').includes('..'), 'Invalid SDK path')
 let current = root
 for (const part of relative.split('/')) {
  current = path.join(current, part)
  assert.ok(!lstatSync(current).isSymbolicLink(), 'SDK consumer refuses symlinks')
 }
 return current
}
export function verifySdk({ root = defaultRoot } = {}) {
 const sdk = checkedPath(root, 'vendor/micro-app-sdk')
 const source = JSON.parse(readFileSync(checkedPath(sdk, 'SOURCE.json')))
 assert.match(source.revision, /^[a-f0-9]{40}$/)
 for (const [name, hash] of Object.entries(source.files)) {
  assert.ok(!path.isAbsolute(name) && !name.split('/').includes('..'), 'Invalid SDK path')
  assert.equal(createHash('sha256').update(readFileSync(checkedPath(sdk, name))).digest('hex'), hash, 'SDK artifact drift: ' + name)
 }
 const release = JSON.parse(readFileSync(checkedPath(sdk, 'release.json')))
 assert.equal(release.schemaVersion, 1)
 assert.equal(release.status, 'REPOSITORY_CANDIDATE')
 assert.match(release.sdkVersion, /^[1-9]\d*\.\d+\.\d+$/)
 assert.deepEqual(release.supportedSdkVersions, [release.sdkVersion])
 const directory = checkedPath(sdk, 'releases/' + release.sdkVersion)
 const manifest = JSON.parse(readFileSync(checkedPath(directory, 'artifact-manifest.json')))
 assert.deepEqual(readdirSync(directory).sort(), ['artifact-manifest.json', ...manifest.files.map(file => file.path)].sort())
 for (const file of manifest.files) {
  const bytes = readFileSync(checkedPath(directory, file.path))
  assert.equal(bytes.length, file.bytes)
  assert.equal(createHash('sha256').update(bytes).digest('hex'), file.sha256)
 }
 return release
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
 verifySdk(); console.log('Pinned SDK artifacts verified')
}
