import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync, lstatSync, readFileSync, readdirSync } from 'node:fs'
import path from 'node:path'

export const sourceRoots = ['backend','frontend','scripts','tests','deploy','.github','tools/platform-admin','mini-services/micro-vue','mini-services/micro-jquery','mini-services/notification-ws','mini-services/oauth-playground','examples/websocket','examples/app-backend','examples/micro-app-sdk','business-apps/work-orders/frontend','business-apps/work-orders/backend','business-apps/stores/backend','business-apps/stores/frontend']
sourceRoots.push('crates/operational-telemetry')
sourceRoots.push('crates/deployment-contract','tools/deployment-agent','tools/artifact-agent')
export const sourceFiles = ['package.json','package-lock.json','.nvmrc','.node-version','.npmrc','rust-toolchain.toml','.gitignore','.dockerignore','docker-compose.yml',
  ...['backend/server.mjs','backend/auth.mjs','backend/errors.mjs','backend/observability.mjs','backend/tests/cors.test.mjs',
    'deploy/bootstrap.manifest.json','deploy/grants.manifest.example.json','tests/platform-e2e.mjs']
    .map(file=>`business-apps/work-orders/${file}`),
  ...['bootstrap.manifest.json','bootstrap.schema.json','grants.manifest.example.json','grants.schema.json'].map(file=>`business-apps/stores/deploy/${file}`)]
const sha = bytes => createHash('sha256').update(bytes).digest('hex')
const publicNpmrc = new Set(['.npmrc','frontend/.npmrc','scripts/ci/.npmrc','tests/browser/.npmrc','mini-services/micro-vue/.npmrc','mini-services/micro-jquery/.npmrc','mini-services/oauth-playground/frontend/.npmrc','business-apps/work-orders/frontend/.npmrc','business-apps/stores/frontend/.npmrc'])
export function excluded(relative) {
  const parts = relative.replaceAll('\\','/').split('/')
  const envExamples = ['deploy/compose.env.example','deploy/compose.legacy.env.example','deploy/compose.notification.env.example','deploy/compose.business.env.example','mini-services/notification-ws/config.env.example']
  return parts.some(part => ['.git','.cargo','node_modules','target','dist','.browsers','.cache','.vite','coverage','test-results','playwright-report','secrets','private'].includes(part) || part.startsWith('.env') || /\.(?:pem|key|p12|pfx|log|pid|tsbuildinfo)$/i.test(part) || /^(?:runtime|deployment|credentials|secrets?)\.(?:json|ya?ml|toml|conf|config|txt|ini)$/i.test(part))
    || (parts.at(-1) === '.npmrc' && !publicNpmrc.has(relative))
    || (!envExamples.includes(relative) && parts.some(part => /\.env(?:\.|$)/i.test(part)))
    || (relative.startsWith('deploy/static/history/') && relative !== 'deploy/static/history/.gitkeep')
    || relative.startsWith('frontend/public/sub-apps/')
}
export function validatePublicSource(relative, bytes) {
  if (publicNpmrc.has(relative) && bytes.toString('utf8').split(/\r?\n/).some(line => line.trim() && line.trim() !== 'engine-strict=true')) throw new Error(`Only public engine-strict configuration may enter a source snapshot: ${relative}`)
}
export function createSourceManifest(root, { sourceRevision } = {}) {
  root = path.resolve(root)
  const git = existsSync(path.join(root, '.git')) ? spawnSync('git', ['-c', `safe.directory=${root.replaceAll('\\','/')}`, '-C', root, 'rev-parse', 'HEAD'], { encoding: 'utf8', windowsHide: true, timeout: 10_000 }) : null
  const head = git?.status === 0 ? git.stdout.trim() : undefined
  if (head && sourceRevision && head !== sourceRevision) throw new Error('Explicit source revision does not match checkout HEAD')
  sourceRevision ||= head
  if (!/^[a-f0-9]{40}$/.test(sourceRevision || '')) throw new Error('A full source revision is required for a clean non-Git snapshot')
  const files = [], seen = new Set()
  function walk(relative) {
    relative = relative.replaceAll('\\','/')
    if (excluded(relative) || seen.has(relative)) return
    const file = path.join(root, relative)
    if (!existsSync(file)) return
    const stat = lstatSync(file)
    if (stat.isSymbolicLink()) throw new Error(`Source symlink refused: ${relative}`)
    if (stat.isDirectory()) for (const name of readdirSync(file).sort()) walk(`${relative}/${name}`)
    else if (stat.isFile()) { const bytes = readFileSync(file); validatePublicSource(relative,bytes); files.push({ path: relative, bytes: bytes.length, sha256: sha(bytes) }); seen.add(relative) }
    else throw new Error('Unsupported source input')
  }
  for (const entry of [...sourceRoots, ...sourceFiles]) walk(entry)
  files.sort((a,b) => a.path.localeCompare(b.path, 'en'))
  if (!files.length) throw new Error('No source inputs found')
  return { schemaVersion: 1, sourceRevision, inputsSha256: sha(JSON.stringify(files)), files,
    revisionMeaning: 'sourceRevision identifies Git HEAD; inputsSha256 and files identify actual working-tree bytes, including uncommitted changes' }
}
