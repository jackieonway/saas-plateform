import assert from 'node:assert/strict'
import { readFileSync, readdirSync, lstatSync } from 'node:fs'
import { createHash } from 'node:crypto'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
const defaultRoot = fileURLToPath(new URL('../', import.meta.url))
function checkedPath(root, relative) {
 assert.ok(relative && !path.isAbsolute(relative) && !relative.includes('\\') &&
   relative.split('/').every(part => part && part !== '.' && part !== '..'), 'Invalid SDK path')
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
 assert.equal(source.repository, 'saas-micro-app-sdk')
 assert.match(source.revision, /^[a-f0-9]{40}$/)
 for (const [name, hash] of Object.entries(source.files)) {
  assert.ok(!path.isAbsolute(name) && !name.split('/').includes('..'), 'Invalid SDK path')
  assert.equal(createHash('sha256').update(readFileSync(checkedPath(sdk, name))).digest('hex'), hash, 'SDK artifact drift: ' + name)
 }
 const manifest = JSON.parse(readFileSync(checkedPath(sdk, 'artifact-manifest.json')))
 const pkg = JSON.parse(readFileSync(checkedPath(sdk, 'package.json')))
 assert.equal(manifest.schemaVersion, 1)
 assert.equal(pkg.name, '@saas-platform/micro-app-sdk')
 assert.equal(pkg.version, manifest.sdkVersion)
 assert.equal(source.sdkVersion, manifest.sdkVersion)
 assert.match(manifest.sdkVersion, /^[1-9]\d*\.\d+\.\d+$/)
 const expected = ['artifact-manifest.json', ...manifest.files.map(file => file.path)].sort()
 assert.deepEqual(Object.keys(source.files).sort(), expected)
 assert.deepEqual(readdirSync(sdk).sort(), ['SOURCE.json', ...expected].sort())
 for (const file of manifest.files) {
  const bytes = readFileSync(checkedPath(sdk, file.path))
  assert.equal(bytes.length, file.bytes)
  assert.equal(createHash('sha256').update(bytes).digest('hex'), file.sha256)
 }
 return manifest
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
 verifySdk(); console.log('Pinned SDK artifacts verified')
}
