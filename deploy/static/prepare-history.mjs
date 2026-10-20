import { spawnSync } from 'node:child_process'
import { cpSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { argumentsOf, immutableImage, localImageId, readHistory, sha256, validateInventory } from './finalize-build.mjs'

// This is an operator-side export tool. No Docker socket or CLI is installed
// in either the Rust API or the production static image.
export function exportHistory({ image, imageId, output, docker = 'docker', run = spawnSync }) {
  if ((image && imageId) || (!image && !imageId)) throw new Error('Specify exactly one of --image or --image-id')
  if (image && !immutableImage.test(image)) throw new Error('--image must be a registry/repository@sha256:<64 hex> reference')
  if (imageId && !localImageId.test(imageId)) throw new Error('--image-id must be a full sha256:<64 hex> Docker image ID')
  const reference = image || imageId
  const identityKind = image ? 'registry-digest' : 'docker-image-id'
  const destination = path.resolve(output)
  if (existsSync(destination)) {
    const stat = lstatSync(destination)
    if (stat.isSymbolicLink() || !stat.isDirectory()) throw new Error('Output must be a regular directory')
    if (existsSync(path.join(destination, 'assets')) || existsSync(path.join(destination, 'manifest.json'))) {
      throw new Error('Output already contains history; use a new directory or archive it explicitly')
    }
  }
  const temporary = mkdtempSync(path.join(os.tmpdir(), 'platform-host-history-'))
  let container
  function dockerCommand(args) {
    const result = run(docker, args, { encoding: 'utf8', windowsHide: true, timeout: 120_000, maxBuffer: 1024 * 1024 })
    if (result.error) throw result.error
    if (result.status !== 0) throw new Error(`Docker ${args[0]} failed: ${String(result.stderr || '').trim()}`)
    return String(result.stdout || '').trim()
  }
  try {
    // Offline inspection verifies the exact immutable local config identity;
    // it must never trigger a registry pull or resolve a mutable local tag.
    if (imageId) {
      const inspected = dockerCommand(['image', 'inspect', '--format', '{{.Id}}', imageId])
      if (inspected !== imageId) throw new Error('Inspected local Docker image ID does not match --image-id')
    } else dockerCommand(['pull', image])
    container = dockerCommand(['create', '--network', 'none', '--entrypoint', '/bin/false', reference])
    if (!/^[a-f0-9]{12,64}$/.test(container)) throw new Error('Docker returned an invalid container identity')
    dockerCommand(['cp', `${container}:/usr/share/nginx/html/assets`, temporary])
    dockerCommand(['cp', `${container}:/usr/share/nginx/html/release-metadata.json`, temporary])
    const metadataFile = path.join(temporary, 'release-metadata.json')
    if (lstatSync(metadataFile).isSymbolicLink()) throw new Error('Release metadata cannot be a symlink')
    const metadataBytes = readFileSync(metadataFile)
    const metadata = JSON.parse(metadataBytes)
    if (metadata.schemaVersion !== 1 || !Array.isArray(metadata.retainedReleases)) throw new Error('Previous image has no supported release metadata')
    validateInventory(temporary, metadata.assets)
    const retainedReleases = [...metadata.retainedReleases, {
      image: reference, identityKind, buildId: metadata.buildId, sourceRevision: metadata.sourceRevision,
      sourceSha256: metadata.sourceSha256, metadataSha256: sha256(metadataBytes),
    }]
    const manifest = { schemaVersion: 1, retainedReleases, assets: metadata.assets }
    writeFileSync(path.join(temporary, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`)
    readHistory(temporary)
    mkdirSync(destination, { recursive: true })
    // Copy works when the temporary directory and the workspace are on
    // different volumes (common on Windows). Write the manifest last; an
    // interrupted export cannot be accepted by the image build.
    cpSync(path.join(temporary, 'assets'), path.join(destination, 'assets'), { recursive: true, errorOnExist: true, force: false })
    writeFileSync(path.join(destination, 'manifest.json'), readFileSync(path.join(temporary, 'manifest.json')), { flag: 'wx' })
    return manifest
  } finally {
    if (container && /^[a-f0-9]{12,64}$/.test(container)) {
      const cleanup = run(docker, ['rm', container], { encoding: 'utf8', windowsHide: true, timeout: 30_000 })
      if (cleanup.error || cleanup.status !== 0) process.stderr.write(`Could not remove export-only container ${container}; remove it explicitly.\n`)
    }
    // Only delete the fixed, newly created temporary root after validating its
    // parent and prefix. Never recursively remove the operator's output path.
    if (path.dirname(temporary) !== path.resolve(os.tmpdir()) || !path.basename(temporary).startsWith('platform-host-history-') ||
        lstatSync(temporary).isSymbolicLink()) throw new Error('Refusing unsafe temporary cleanup')
    rmSync(temporary, { recursive: true, force: true })
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const args = argumentsOf(process.argv.slice(2))
    if ((!args.image && !args['image-id']) || !args.output) throw new Error('Usage: node prepare-history.mjs (--image registry/repository@sha256:<digest> | --image-id sha256:<full-id>) --output deploy/static/history')
    const manifest = exportHistory({ image: args.image, imageId: args['image-id'], output: args.output, docker: args.docker || 'docker' })
    process.stdout.write(`Verified ${manifest.assets.length} retained assets from ${manifest.retainedReleases.length} releases\n`)
  } catch (error) { process.stderr.write(`${error.message}\n`); process.exitCode = 1 }
}
