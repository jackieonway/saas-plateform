import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import path from 'node:path'

/** Include tracked files even when ignored, plus new non-ignored source files. */
export function repositoryDeliveryFiles(root) {
  return [...new Set(execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard', '-z'], {
    cwd: root, encoding: 'utf8', windowsHide: true,
  }).split('\0').filter(Boolean))].filter(name => existsSync(path.join(root, name)))
}

export function assertApplicationStructure(root) {
  const files = repositoryDeliveryFiles(root)
  const directories = [...new Set(files.filter(name => name.includes('/')).map(name => name.split('/')[0]))]
    .filter(name => !name.startsWith('.')).sort()
  assert.deepEqual(directories, ['backend', 'frontend'], 'Only frontend and backend may ship as application source directories')
  assert(!files.includes('package.json'), 'The application repository must not ship a root npm package')
  return files
}
