import test from 'node:test'
import assert from 'node:assert/strict'
import { existsSync, readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { assertApplicationStructure } from '../scripts/repository-delivery.mjs'
const root = fileURLToPath(new URL('../../', import.meta.url))

test('the delivered application source exposes only frontend and backend beside Git automation', () => {
 assertApplicationStructure(root)
})

test('structure checks exclude ignored local tools but reject tracked and new unexpected source', t => {
 const directory = mkdtempSync(path.join(os.tmpdir(), 'saas-repository-structure-'))
 const git = (...args) => execFileSync('git', args, { cwd: directory, encoding: 'utf8', windowsHide: true })
 const write = (name, content = 'fixture') => {
  const file = path.join(directory, name)
  mkdirSync(path.dirname(file), { recursive: true }); writeFileSync(file, content)
 }
 t.after(() => {
  assert.equal(path.dirname(directory), path.resolve(os.tmpdir()))
  assert(path.basename(directory).startsWith('saas-repository-structure-'))
  rmSync(directory, { recursive: true, force: true })
 })
 git('init', '--quiet')
 write('.gitignore', 'local-*/\n')
 write('frontend/src/main.ts'); write('backend/src/main.rs')
 write('local-application-rebuild/package.json')
 git('add', '.gitignore', 'frontend', 'backend')
 assertApplicationStructure(directory)
 write('unexpected/source.rs')
 assert.throws(() => assertApplicationStructure(directory), /Only frontend and backend/)
 git('add', 'unexpected/source.rs')
 assert.throws(() => assertApplicationStructure(directory), /Only frontend and backend/)
 // Tracking a path explicitly overrides its ignored status: it becomes delivery.
 git('rm', '--cached', 'unexpected/source.rs')
 rmSync(path.join(directory, 'unexpected/source.rs'))
 git('add', '-f', 'local-application-rebuild/package.json')
 assert.throws(() => assertApplicationStructure(directory), /Only frontend and backend/)
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
