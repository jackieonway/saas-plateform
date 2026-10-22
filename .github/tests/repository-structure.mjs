import test from 'node:test'
import assert from 'node:assert/strict'
import { existsSync, readFileSync, readdirSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
const root = fileURLToPath(new URL('../../', import.meta.url))

test('application checkout exposes only frontend and backend beside Git automation', () => {
 const visible = readdirSync(root, { withFileTypes: true }).filter(entry => entry.isDirectory() && !entry.name.startsWith('.')).map(entry => entry.name).sort()
 assert.deepEqual(visible, ['backend', 'frontend'])
 assert.equal(existsSync(path.join(root, 'package.json')), false)
})

test('each Dockerfile reads source only from its own frontend or backend context', () => {
 for (const directory of ['frontend', 'backend']) {
  const context = path.join(root, directory)
  for (const line of readFileSync(path.join(context, 'Dockerfile'), 'utf8').split(/\r?\n/)) {
   if (!line.startsWith('COPY ') || line.includes('--from=')) continue
   const words = line.split(/\s+/).slice(1).filter(word => !word.startsWith('--'))
   for (const name of words.slice(0, -1)) {
    const file = path.resolve(context, name)
    assert.ok(file === context || file.startsWith(context + path.sep), name)
    assert.ok(existsSync(file), directory + ': missing COPY input ' + name)
   }
  }
 }
})

test('backend local Cargo dependencies remain inside the backend directory', () => {
 const backend = path.join(root, 'backend')
 for (const match of readFileSync(path.join(backend, 'Cargo.toml'), 'utf8').matchAll(/path\s*=\s*"([^"]+)"/g)) {
  const dependency = path.resolve(backend, match[1])
  assert.ok(dependency.startsWith(backend + path.sep), match[1])
  assert.ok(existsSync(path.join(dependency, 'Cargo.toml')), match[1])
 }
})

test('current build entry points never request cross-repository source snapshots', () => {
 const scripts = JSON.parse(readFileSync(path.join(root, 'frontend/package.json'))).scripts
 assert.doesNotMatch(JSON.stringify(scripts), /\.integration|business-sources|prepare-business|tasks\.mjs/)
 assert.doesNotMatch(JSON.stringify(scripts), /\.\.\/backend\//)
 assert.equal(existsSync(path.join(root, 'backend/acceptance')), false)
 assert.doesNotMatch(readFileSync(path.join(root, '.github/workflows/ci.yml'), 'utf8'), /business-sources|\.integration|backend\/scripts/)
 for (const tool of ['monitoring', 'recovery']) assert.ok(existsSync(path.join(root, 'backend/ops', tool, 'Cargo.toml')))
})
