import { createHash } from 'node:crypto'
import { copyFileSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

export const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex')
const digest = /^[a-f0-9]{64}$/
export const immutableImage = /^(?:[a-zA-Z0-9._:-]+\/)+[a-zA-Z0-9._-]+@sha256:[a-f0-9]{64}$/
export const localImageId = /^sha256:[a-f0-9]{64}$/

export function regularFiles(root, prefix = '') {
  if (!existsSync(root)) return []
  if (!lstatSync(root).isDirectory() || lstatSync(root).isSymbolicLink()) throw new Error(`Not a regular directory: ${root}`)
  return readdirSync(root).sort().flatMap((name) => {
    if (name === '.' || name === '..' || /[\\\x00-\x1f]/.test(name)) throw new Error(`Unsafe file name: ${name}`)
    const absolute = path.join(root, name)
    const relative = `${prefix}${name}`
    const stat = lstatSync(absolute)
    if (stat.isSymbolicLink()) throw new Error(`Symlink is forbidden in static artifacts: ${relative}`)
    if (stat.isDirectory()) return regularFiles(absolute, `${relative}/`)
    if (!stat.isFile()) throw new Error(`Unsupported static artifact: ${relative}`)
    return [relative]
  })
}

export function assetInventory(root) {
  return regularFiles(path.join(root, 'assets'), 'assets/').map((name) => {
    const bytes = readFileSync(path.join(root, ...name.split('/')))
    return { path: name, sha256: sha256(bytes), bytes: bytes.length }
  })
}

export function validateInventory(root, expected) {
  if (!Array.isArray(expected)) throw new Error('Asset inventory must be an array')
  const names = new Set()
  for (const asset of expected) {
    if (!asset || typeof asset.path !== 'string' || !asset.path.startsWith('assets/') ||
        asset.path.split('/').some((part) => !part || part.startsWith('.') || /[\\\x00-\x1f]/.test(part)) ||
        !digest.test(asset.sha256) || !Number.isSafeInteger(asset.bytes) || asset.bytes < 0 || names.has(asset.path)) {
      throw new Error('Invalid or duplicate asset inventory entry')
    }
    names.add(asset.path)
  }
  const actual = assetInventory(root)
  const ordered = expected.map(({ path: name, sha256: checksum, bytes }) => ({ path: name, sha256: checksum, bytes }))
    .sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0)
  const actualOrdered = [...actual].sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0)
  if (JSON.stringify(actualOrdered) !== JSON.stringify(ordered)) throw new Error('Asset bytes or file set do not match the recorded inventory')
  return actual
}

function validateRelease(release) {
  // Registry digests identify distributable manifests. Docker image IDs refer
  // only to an immutable local config object and must be marked explicitly.
  const validIdentity = release && ((release.identityKind === 'docker-image-id' && localImageId.test(release.image)) ||
    ((release.identityKind === undefined || release.identityKind === 'registry-digest') && immutableImage.test(release.image)))
  if (!validIdentity || !/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(release.buildId) ||
      !digest.test(release.sourceSha256) || !digest.test(release.metadataSha256) ||
      (release.sourceRevision !== null && !/^[a-f0-9]{40,64}$/.test(release.sourceRevision))) {
    throw new Error('Retained release must identify an immutable image and verified source metadata')
  }
}

export function readHistory(history) {
  const manifestPath = path.join(history, 'manifest.json')
  if (!existsSync(manifestPath)) {
    if (assetInventory(history).length) throw new Error('History assets require manifest.json')
    return { schemaVersion: 1, retainedReleases: [], assets: [] }
  }
  if (lstatSync(manifestPath).isSymbolicLink()) throw new Error('History manifest cannot be a symlink')
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))
  if (manifest.schemaVersion !== 1 || !Array.isArray(manifest.retainedReleases) || manifest.retainedReleases.length === 0) throw new Error('Invalid history manifest')
  manifest.retainedReleases.forEach(validateRelease)
  validateInventory(history, manifest.assets)
  return manifest
}

export function sourceHash(source) {
  const inputs = ['src', 'public', 'package.json', 'package-lock.json', '.npmrc', 'index.html', 'vite.config.ts', 'tsconfig.json', 'Dockerfile']
  const hash = createHash('sha256')
  for (const input of inputs) {
    const absolute = path.join(source, input)
    if (!existsSync(absolute)) continue
    if (lstatSync(absolute).isSymbolicLink()) throw new Error(`Source input cannot be a symlink: ${input}`)
    const files = lstatSync(absolute).isDirectory() ? regularFiles(absolute, `${input}/`) : [input]
    for (const name of files) hash.update(`${name}\0${sha256(readFileSync(path.join(source, ...name.split('/'))))}\n`)
  }
  return hash.digest('hex')
}

export function finalizeBuild({ dist, source, history, sourceRevision = '', buildId = '' }) {
  if (!existsSync(path.join(dist, 'index.html'))) throw new Error('Built index.html is required')
  if (sourceRevision && !/^[a-f0-9]{40,64}$/.test(sourceRevision)) throw new Error('SOURCE_REVISION must be a full Git revision')
  const sourceSha256 = sourceHash(source)
  const resolvedBuildId = buildId || `host-${sourceSha256.slice(0, 20)}`
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(resolvedBuildId)) throw new Error('HOST_BUILD_ID has an invalid format')
  const retained = readHistory(history)
  const current = new Map(assetInventory(dist).map((asset) => [asset.path, asset]))
  // Record this before merging history so release verification can prove an
  // old entry URL is retained-only, rather than coincidentally built again.
  const currentAssets = [...current.values()]
  // Check every collision before copying anything. A hash-shaped URL must never
  // change its bytes, even if an upstream bundler accidentally reuses a name.
  for (const asset of retained.assets) {
    const existing = current.get(asset.path)
    if (existing && existing.sha256 !== asset.sha256) throw new Error(`Immutable asset collision: ${asset.path}`)
  }
  for (const asset of retained.assets) {
    if (current.has(asset.path)) continue
    const destination = path.join(dist, ...asset.path.split('/'))
    mkdirSync(path.dirname(destination), { recursive: true })
    copyFileSync(path.join(history, ...asset.path.split('/')), destination)
  }
  const metadata = {
    schemaVersion: 1, buildId: resolvedBuildId,
    sourceRevision: sourceRevision || null, sourceSha256,
    retainedReleases: retained.retainedReleases,
    currentAssets,
    assets: assetInventory(dist),
  }
  writeFileSync(path.join(dist, 'release-metadata.json'), `${JSON.stringify(metadata, null, 2)}\n`)
  return metadata
}

export function argumentsOf(argv) {
  const values = {}
  for (let i = 0; i < argv.length; i += 2) {
    if (!argv[i]?.startsWith('--') || argv[i + 1] === undefined) throw new Error('Arguments must be --name value pairs')
    values[argv[i].slice(2)] = argv[i + 1]
  }
  return values
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const args = argumentsOf(process.argv.slice(2))
    for (const name of ['dist', 'source', 'history']) if (!args[name]) throw new Error(`--${name} is required`)
    const metadata = finalizeBuild({ dist: path.resolve(args.dist), source: path.resolve(args.source), history: path.resolve(args.history),
      sourceRevision: args['source-revision'], buildId: args['build-id'] })
    process.stdout.write(`Host build ${metadata.buildId}: ${metadata.assets.length} assets; ${metadata.retainedReleases.length} retained releases\n`)
  } catch (error) { process.stderr.write(`${error.message}\n`); process.exitCode = 1 }
}
