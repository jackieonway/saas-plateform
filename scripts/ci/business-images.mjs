#!/usr/bin/env node
// Build/test orchestration only. Production targets use Compose and Rust CLI.
import { spawn } from 'node:child_process'
import { createHash, randomBytes } from 'node:crypto'
import { chmodSync, copyFileSync, createWriteStream, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { pipeline } from 'node:stream/promises'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { applications, validateStaticInput } from '../../deploy/microapps/render-static.mjs'
import { createDocker, createSnapshot, validateOutputLocation } from './linux-runner.mjs'
import { createSourceManifest } from './provenance.mjs'
import { inspectOciArchive } from './release.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const digestPattern = /^sha256:[a-f0-9]{64}$/
const hashPattern = /^[a-f0-9]{64}$/
export const businessApps = applications
export const businessImageKeys = Object.freeze(['work-orders-v1', 'work-orders-v2', 'stores-v1', 'stores-v2', 'work-orders-api', 'stores-api'])
export const businessOwnerLabel = 'saas.phase28.owner'
const linuxPath = value => process.platform === 'win32' ? `/mnt/${path.resolve(value)[0].toLowerCase()}/${path.resolve(value).slice(3).replaceAll('\\', '/')}` : path.resolve(value)
const json = (file, value) => writeFileSync(file, JSON.stringify(value, null, 2) + '\n', { mode: 0o600 })
const fail = message => { throw new Error(message) }
const sha = bytes => createHash('sha256').update(bytes).digest('hex')
const parse = output => { try { return JSON.parse(output) } catch { return fail('Invalid Docker JSON response') } }
const first = output => { const values = parse(output); if (!Array.isArray(values) || values.length !== 1) fail('Exactly one Docker object is required'); return values[0] }
const imageMissing = output => {
  try { if (Array.isArray(JSON.parse(output))) return false } catch { /* Expected Docker CLI error text is not JSON. */ }
  return /No such image:|(?:Error|error)[^\r\n]*not found/.test(String(output))
}

export function staticImageSpec({ appCode, version, hostOrigin, apiBase, targetPlatform = 'linux/amd64' }) {
  const config = validateStaticInput({ appCode, version, hostOrigin, apiBase })
  if (!/^linux\/(?:amd64|arm64)$/.test(targetPlatform)) fail('Unsupported target platform')
  return { ...config, targetPlatform, publicVersionPath: `/sub-apps/${appCode}/versions/${version}/`, imageRoot: '/usr/share/nginx/html', port: 8080, user: '101:101' }
}

export function parseBusinessImageArgs(args) {
  const options = { targetPlatform: 'linux/amd64', hostOrigin: 'https://platform.example.test', apiBases: {} }, seen = new Set()
  const keys = { '--out-dir': 'outDir', '--source-root': 'sourceRoot', '--source-revision': 'sourceRevision', '--platform': 'targetPlatform', '--host-origin': 'hostOrigin', '--build-proxy': 'buildProxy' }
  for (let index = 0; index < args.length; index += 2) {
    const flag = args[index], value = args[index + 1]
    if (seen.has(flag) || !value || value.startsWith('--')) fail('Unknown, repeated or missing business image argument')
    seen.add(flag)
    if (flag === '--work-orders-api-base') options.apiBases['work-orders'] = value
    else if (flag === '--stores-api-base') options.apiBases.stores = value
    else if (keys[flag]) options[keys[flag]] = value
    else fail('Unknown business image argument')
  }
  if (!path.isAbsolute(options.outDir || '') || !/^[a-f0-9]{40}$/.test(options.sourceRevision || '')) fail('Fresh absolute output and full source revision required')
  if (options.sourceRoot && !path.isAbsolute(options.sourceRoot)) fail('Source root must be absolute')
  if (options.buildProxy && !/^http:\/\/(?:127\.0\.0\.1|localhost):[1-9][0-9]{0,4}$/.test(options.buildProxy)) fail('Build proxy must be a credential-free loopback URL')
  for (const appCode of businessApps) staticImageSpec({ appCode, version: '1.0.0', hostOrigin: options.hostOrigin, apiBase: options.apiBases[appCode] || `/${appCode}-api`, targetPlatform: options.targetPlatform })
  return options
}

function freshOutput(outDir, sourceRoot = root) {
  validateOutputLocation(outDir, sourceRoot)
  if (existsSync(outDir)) fail('Output already exists; use a fresh absolute directory')
  mkdirSync(path.join(outDir, 'logs'), { recursive: true, mode: 0o700 })
}

function loggedDocker(docker, report, outDir, save) {
  return async (name, args, options) => {
    const record = { name, args, log: `logs/${String(report.commands.length + 1).padStart(3, '0')}-${name}.log`, exitCode: null }
    report.commands.push(record); save()
    try {
      const output = await docker(args, options)
      writeFileSync(path.join(outDir, record.log), String(output) + '\n', { mode: 0o600 })
      record.exitCode = options?.allowFailure ? null : 0
      if (options?.allowFailure) record.completion = 'RETURNED_WITH_FAILURE_ALLOWED_EXIT_CODE_NOT_EXPOSED_BY_ADAPTER'
      save(); return output
    } catch (error) {
      record.exitCode = Number(/exit=(\d+)/.exec(error.message)?.[1] || 1); writeFileSync(path.join(outDir, record.log), String(error.message) + '\n', { mode: 0o600 }); save()
      throw new Error(`${name} failed; see ${record.log}`, { cause: error })
    }
  }
}

/** Optional Linux ext4 export avoids image-sized buffering and WSL DrvFS writes.
 * A caller can inject exportArchive({docker,image,outputPath}) for its owned IO. */
async function saveArchive({ docker, image, outputPath, owner }) {
  if (process.platform !== 'win32') return docker('save-' + image.key, ['image', 'save', '--output', outputPath, image.imageId])
  const distro = process.env.CI_WSL_DISTRO || 'Ubuntu', stage = `/var/tmp/${owner}-${image.key}`
  const command = args => new Promise((resolve, reject) => {
    const child = spawn('wsl.exe', ['-d', distro, '--exec', ...args], { cwd: root, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
    let output = ''; child.stderr.on('data', bytes => { output += bytes })
    child.on('error', reject); child.on('close', code => code === 0 ? resolve() : reject(new Error('Owned archive IO failed: ' + output.slice(-1000))))
  })
  await command(['mkdir', '--mode=700', stage])
  try {
    await docker('save-' + image.key, ['image', 'save', '--output', `${stage}/image.tar`, image.imageId])
    const child = spawn('wsl.exe', ['-d', distro, '--exec', 'cat', `${stage}/image.tar`], { cwd: root, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
    const complete = new Promise((resolve, reject) => { child.on('error', reject); child.on('close', code => code === 0 ? resolve() : reject(new Error('Archive stream failed'))) })
    try { await Promise.all([pipeline(child.stdout, createWriteStream(outputPath, { flags: 'wx', mode: 0o600 })), complete]) }
    catch (error) { child.kill(); throw error }
  } finally {
    await command(['rm', '--force', '--', `${stage}/image.tar`])
    await command(['rmdir', '--', stage])
  }
}

export function verifyPrebuiltBinary(binary, { sourceRevision, inputsSha256, targetPlatform }) {
  if (!binary || !path.isAbsolute(binary.path || '') || binary.sourceRevision !== sourceRevision || binary.inputsSha256 !== inputsSha256 || binary.targetPlatform !== targetPlatform || !hashPattern.test(binary.sha256 || '')) fail('Prebuilt ELF requires exact source, input, architecture and SHA-256 binding')
  const bytes = readFileSync(binary.path)
  const machine = bytes.length >= 20 ? bytes.readUInt16LE(18) : 0
  if (sha(bytes) !== binary.sha256 || bytes.subarray(0, 4).toString('hex') !== '7f454c46' || bytes[4] !== 2 || bytes[5] !== 1 || machine !== (targetPlatform === 'linux/amd64' ? 62 : 183)) fail('Prebuilt ELF digest or target architecture mismatch')
  return { sha256: binary.sha256, bytes: bytes.length, elfMachine: machine, sourceRevision, inputsSha256, targetPlatform }
}

/** Docker's classic image store exposes a config digest as .Id; the containerd
 * image store can expose the single-platform manifest digest. Record the actual
 * daemon identity without treating either as a registry publication receipt. */
export function classifyDaemonImageIdentity({ imageId, ociManifestDigest, ociConfigDigest }) {
  if (![imageId, ociManifestDigest, ociConfigDigest].every(value => digestPattern.test(value || ''))) fail('Invalid daemon/OCI image identity')
  if (imageId === ociManifestDigest) return 'OCI_MANIFEST_DIGEST'
  if (imageId === ociConfigDigest) return 'OCI_CONFIG_DIGEST'
  fail('Daemon image ID matches neither verified OCI manifest nor config')
}

export function validateBusinessCandidate(report, { sourceRevision, inputsSha256 } = {}) {
  if (report?.schemaVersion !== 1 || report.kind !== 'business-image-candidate' || report.status !== 'PASS' || report.build !== 'PASS' || report.registryPublication !== 'NOT_REQUESTED' || report.targetDeployment !== 'NOT_RUN') fail('Business image candidate build did not pass its own required checks')
  if (!/^[a-f0-9]{40}$/.test(report.sourceRevision || '') || !hashPattern.test(report.inputsSha256 || '') || !/^linux\/(?:amd64|arm64)$/.test(report.targetPlatform || '') || report.currentSourceMatch !== true || !/^phase28-[a-z0-9-]+$/.test(report.owner || '')) fail('Invalid business candidate provenance')
  if ((sourceRevision && report.sourceRevision !== sourceRevision) || (inputsSha256 && report.inputsSha256 !== inputsSha256)) fail('Business candidate source differs from required source')
  if (!Array.isArray(report.images) || report.images.length !== businessImageKeys.length || new Set(report.images.map(image => image.key)).size !== businessImageKeys.length) fail('All six independent business image records are required')
  for (const key of businessImageKeys) {
    const image = report.images.find(value => value.key === key)
    if (!image || image.archive !== `images/${key}.tar` || !digestPattern.test(image.imageId || '') || !hashPattern.test(image.archiveSha256 || '') || !digestPattern.test(image.ociManifestDigest || '') || !digestPattern.test(image.ociConfigDigest || '') || image.targetPlatform !== report.targetPlatform || image.sourceRevision !== report.sourceRevision || image.inputsSha256 !== report.inputsSha256 || image.buildId !== report.buildId || image.imageConfigVerified !== true || image.registryAvailability !== 'NOT_VERIFIED_NO_PUSH') fail('Incomplete image identity, archive or source metadata')
    if (image.daemonImageIdKind !== classifyDaemonImageIdentity(image)) fail('Daemon image ID meaning does not match verified OCI identities')
    if (image.role !== (key.endsWith('-api') ? 'api' : 'static') || !businessApps.includes(image.appCode) || !key.startsWith(image.appCode + '-')) fail('Image application/role mismatch')
  }
  return report
}

/** Build the four versioned frontends and two independent Rust API candidates.
 * It never logs in, pushes, changes routes, registers or starts a production API. */
export async function buildBusinessImages(options) {
  const sourceRoot = path.resolve(options.sourceRoot || root), outDir = path.resolve(options.outDir || '')
  freshOutput(outDir, sourceRoot)
  const source = createSourceManifest(sourceRoot, { sourceRevision: options.sourceRevision })
  if (options.inputsSha256 && source.inputsSha256 !== options.inputsSha256) fail('Business candidate source input mismatch')
  const targetPlatform = options.targetPlatform || options.platform || 'linux/amd64', versions = options.versions || ['1.0.0', '2.0.0']
  if (!Array.isArray(versions) || versions.length !== 2 || versions[0] === versions[1]) fail('Two distinct immutable frontend versions are required')
  const hostOrigin = options.hostOrigin || 'https://platform.example.test'
  const apiBases = Object.fromEntries(businessApps.map(appCode => [appCode, Object.fromEntries(versions.map(version => {
    const selected = options.versionApiBases?.[appCode]?.[version] || (typeof options.apiBases?.[appCode] === 'object' ? options.apiBases[appCode][version] : options.apiBases?.[appCode]) || `/${appCode}-api`
    return [version, selected]
  }))]))
  for (const appCode of businessApps) for (const version of versions) staticImageSpec({ appCode, version, hostOrigin, apiBase: apiBases[appCode][version], targetPlatform })
  const owner = options.owner || `phase28-${Date.now()}-${randomBytes(4).toString('hex')}`
  if (!/^phase28-[a-z0-9-]{8,100}$/.test(owner)) fail('Invalid candidate owner')
  const buildId = `${source.sourceRevision.slice(0, 12)}-${source.inputsSha256.slice(0, 16)}`
  const report = { schemaVersion: 1, kind: 'business-image-candidate', owner, status: 'RUNNING', build: 'NOT_RUN', registryPublication: 'NOT_REQUESTED', targetDeployment: 'NOT_RUN',
    sourceRevision: source.sourceRevision, inputsSha256: source.inputsSha256, targetPlatform, buildId, publicConfig: { hostOrigin, apiBases, versions }, startedAt: new Date().toISOString(), images: [], baseImages: [], commands: [] }
  const save = () => json(path.join(outDir, 'candidate-result.json'), report), docker = loggedDocker(options.docker || createDocker(), report, outDir, save)
  mkdirSync(path.join(outDir, 'images')); json(path.join(outDir, 'source-manifest.json'), source); save()
  try {
    const snapshot = path.join(outDir, 'source'); createSnapshot(snapshot, sourceRoot)
    const version = await docker('docker-version', ['version', '--format', '{{.Server.Version}}'])
    if (Number(version.split('.')[0]) < 29) fail('Docker 29+ OCI archive output is required')
    report.dockerVersion = version
    const bases = { node: 'node:24.18.0-alpine', static: 'nginxinc/nginx-unprivileged:1.28.0-alpine', rust: 'rust:1.97.0-slim-bookworm', runtime: 'debian:bookworm-slim', ...options.baseImages }
    for (const [role, reference] of Object.entries(bases)) {
      if (role === 'rust' && options.prebuiltBinaries) continue
      const image = first(await docker('base-' + role, ['image', 'inspect', reference]))
      if (!digestPattern.test(image.Id || '')) fail('Invalid base image identity')
      const immutable = image.RepoDigests?.[0] || image.Id
      report.baseImages.push({ role, requested: reference, imageId: image.Id, repoDigests: image.RepoDigests || [], selected: immutable }); bases[role] = immutable
    }
    const specs = businessApps.flatMap(appCode => versions.map((version, index) => ({ key: `${appCode}-v${index + 1}`, appCode, role: 'static', version })))
      .concat(businessApps.map(appCode => ({ key: `${appCode}-api`, appCode, role: 'api' })))
    for (const spec of specs) {
      const tag = `${owner}-${spec.key}:candidate`, image = { ...spec, tag, sourceRevision: source.sourceRevision, inputsSha256: source.inputsSha256, buildId, targetPlatform }
      let context = snapshot, dockerfile = path.join(snapshot, 'deploy/microapps/Dockerfile')
      const args = { SOURCE_REVISION: source.sourceRevision, INPUTS_SHA256: source.inputsSha256, BUILD_ID: buildId }
      if (spec.role === 'static') Object.assign(args, { APP_CODE: spec.appCode, APP_VERSION: spec.version, HOST_ORIGIN: hostOrigin, API_BASE: apiBases[spec.appCode][spec.version], NODE_IMAGE: bases.node, STATIC_IMAGE: bases.static })
      else {
        dockerfile = path.join(snapshot, 'business-apps', spec.appCode, 'backend', 'Dockerfile')
        Object.assign(args, { RUST_IMAGE: bases.rust, RUNTIME_IMAGE: bases.runtime })
        image.apiBuildMode = 'locked-Docker-source-build'
        if (options.prebuiltBinaries) {
          const binary = options.prebuiltBinaries[spec.appCode]
          image.prebuilt = verifyPrebuiltBinary(binary, { sourceRevision: source.sourceRevision, inputsSha256: source.inputsSha256, targetPlatform })
          context = path.join(outDir, 'work', spec.key); mkdirSync(context, { recursive: true })
          copyFileSync(binary.path, path.join(context, 'business-api')); chmodSync(path.join(context, 'business-api'), 0o755)
          dockerfile = path.join(snapshot, 'deploy/microapps/Dockerfile.api-runtime')
          copyFileSync(dockerfile, path.join(context, 'Dockerfile')); dockerfile = path.join(context, 'Dockerfile')
          Object.assign(args, { APP_CODE: spec.appCode, API_PORT: spec.appCode === 'stores' ? '3031' : '3004' }); image.apiBuildMode = 'verified-prebuilt-ELF'
        }
      }
      const proxy = options.buildProxy ? ['--network', 'host', ...['HTTP_PROXY', 'HTTPS_PROXY', 'http_proxy', 'https_proxy'].flatMap(key => ['--build-arg', `${key}=${options.buildProxy}`])] : []
      await docker('build-' + spec.key, ['build', '--force-rm', '--platform', targetPlatform, ...proxy, '--label', `${businessOwnerLabel}=${owner}`,
        ...Object.entries(args).filter(([, value]) => value !== undefined).flatMap(([key, value]) => ['--build-arg', `${key}=${value}`]), '--tag', tag, '--file', linuxPath(dockerfile), linuxPath(context)], { timeout: 60 * 60_000 })
      const actual = first(await docker('inspect-' + spec.key, ['image', 'inspect', tag]))
      const user = spec.role === 'static' ? '101:101' : '10001:10001', entrypoint = spec.role === 'static' ? ['nginx'] : [image.apiBuildMode === 'verified-prebuilt-ELF' ? '/usr/local/bin/business-api' : `${spec.appCode}-api`]
      if (`${actual.Os}/${actual.Architecture}` !== targetPlatform || !digestPattern.test(actual.Id || '') || actual.Config.User !== user || JSON.stringify(actual.Config.Entrypoint) !== JSON.stringify(entrypoint) || !actual.Config.Healthcheck?.Test?.length || actual.Config.Labels?.[businessOwnerLabel] !== owner || actual.Config.Labels?.['org.opencontainers.image.revision'] !== source.sourceRevision || actual.Config.Labels?.['saas.ci.inputs-sha256'] !== source.inputsSha256) fail('Candidate image runtime configuration or provenance differs from contract')
      image.imageId = actual.Id; image.imageConfigVerified = true; image.user = user; image.entrypoint = entrypoint; image.healthcheck = actual.Config.Healthcheck
      image.archive = `images/${spec.key}.tar`
      const outputPath = path.join(outDir, image.archive)
      if (options.exportArchive) await options.exportArchive({ docker: options.docker || createDocker(), image, outputPath, owner })
      else await saveArchive({ docker, image, outputPath, owner })
      Object.assign(image, inspectOciArchive(outputPath, targetPlatform))
      image.daemonImageIdKind = classifyDaemonImageIdentity(image)
      if (image.labels['org.opencontainers.image.revision'] !== source.sourceRevision || image.labels['saas.ci.inputs-sha256'] !== source.inputsSha256 || image.labels[businessOwnerLabel] !== owner) fail('Saved OCI archive differs from built candidate')
      image.registryAvailability = 'NOT_VERIFIED_NO_PUSH'; report.images.push(image); save()
    }
    if (createSourceManifest(sourceRoot, { sourceRevision: source.sourceRevision }).inputsSha256 !== source.inputsSha256) fail('Source changed during business image build; rebuild a fresh candidate')
    report.currentSourceMatch = true; report.build = 'PASS'; report.status = 'PASS'; validateBusinessCandidate(report)
    return report
  } catch (error) { report.status = 'FAIL'; report.build = 'FAIL'; report.error = error.message; throw error }
  finally { report.finishedAt = new Date().toISOString(); save() }
}

function registryAuthority(registry, scope, insecureLoopback) {
  if (typeof registry !== 'string' || !/^(?:localhost|[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?)(?::[1-9][0-9]{0,4})?$/.test(registry)) fail('Registry must be a literal lowercase authority without credentials or path')
  const local = /^(?:localhost|127\.0\.0\.1):[1-9][0-9]{0,4}$/.test(registry)
  if (!['isolated', 'protected-release'].includes(scope) || (scope === 'isolated' && !local) || (insecureLoopback && (scope !== 'isolated' || !local))) fail('Isolated registry publication is restricted to explicit loopback authority')
  if (registry.includes(':') && Number(registry.split(':').at(-1)) > 65535) fail('Invalid registry port')
  return registry
}

export function parseRegistryDescriptor(output) {
  let value = parse(output)
  if (Array.isArray(value)) { if (value.length !== 1) fail('Single-platform registry manifest required'); value = value[0] }
  const descriptor = value.Descriptor || value.descriptor, manifest = value.SchemaV2Manifest || value.OCIManifest || value.schemaV2Manifest || value.ociManifest
  if (!digestPattern.test(descriptor?.digest || '') || !digestPattern.test(manifest?.config?.digest || '') || !Array.isArray(manifest.layers) || !manifest.layers.length) fail('Registry descriptor/config digest evidence is missing')
  return { digest: descriptor.digest, configDigest: manifest.config.digest, mediaType: descriptor.mediaType, layerCount: manifest.layers.length }
}

/** Consume precisely the verified archives. Registry digest is learned from
 * actual push+registry inspection, never copied from the OCI archive identity. */
export async function publishBusinessImages({ docker: adapter = createDocker(), candidate, candidateDir, registry, repositoryPrefix = 'business', scope, insecureLoopback = false, allowProtectedPublication = false, outDir }) {
  validateBusinessCandidate(candidate); registryAuthority(registry, scope, insecureLoopback)
  if (scope === 'protected-release' && allowProtectedPublication !== true) fail('Protected publication requires its explicit release boundary')
  if (!/^[a-z0-9]+(?:[._/-][a-z0-9]+)*$/.test(repositoryPrefix)) fail('Invalid lowercase repository prefix')
  freshOutput(outDir)
  const report = { schemaVersion: 1, kind: 'business-image-publication', status: 'RUNNING', scope, registry, registryTransport: insecureLoopback ? 'http-loopback-isolated' : 'https-verified',
    sourceRevision: candidate.sourceRevision, inputsSha256: candidate.inputsSha256, buildId: candidate.buildId, targetPlatform: candidate.targetPlatform, candidateOwner: candidate.owner, productionDeployment: 'NOT_RUN', images: [], commands: [] }
  const save = () => json(path.join(outDir, 'publication-result.json'), report), docker = loggedDocker(adapter, report, outDir, save); save()
  try {
    for (const image of candidate.images) {
      const archive = path.join(candidateDir, image.archive), verified = inspectOciArchive(archive, candidate.targetPlatform)
      if (verified.archiveSha256 !== image.archiveSha256 || verified.ociConfigDigest !== image.ociConfigDigest || verified.ociManifestDigest !== image.ociManifestDigest) fail('Candidate archive changed before publication')
      await docker('load-' + image.key, ['image', 'load', '--input', linuxPath(archive)])
      const loaded = first(await docker('loaded-' + image.key, ['image', 'inspect', image.imageId]))
      if (loaded.Id !== image.imageId || loaded.Config.Labels?.[businessOwnerLabel] !== candidate.owner || loaded.Config.Labels?.['org.opencontainers.image.revision'] !== candidate.sourceRevision || loaded.Config.Labels?.['saas.ci.inputs-sha256'] !== candidate.inputsSha256) fail('Loaded candidate identity/provenance mismatch')
      const repository = `${registry}/${repositoryPrefix}/${image.appCode}-${image.role === 'static' ? 'web' : 'api'}`
      const tag = `${repository}:${candidate.buildId}-${image.role === 'static' ? image.version : 'api'}`
      await docker('tag-' + image.key, ['tag', image.imageId, tag])
      const push = await docker('push-' + image.key, ['push', tag], { timeout: 15 * 60_000 })
      const digests = [...push.matchAll(/digest:\s*(sha256:[a-f0-9]{64})/g)].map(match => match[1])
      if (digests.length !== 1) fail('Push did not report one actual registry digest')
      const remote = parseRegistryDescriptor(await docker('registry-' + image.key, ['manifest', 'inspect', '--verbose', ...(insecureLoopback ? ['--insecure'] : []), `${repository}@${digests[0]}`]))
      if (remote.digest !== digests[0] || remote.configDigest !== image.ociConfigDigest) fail('Registry content does not match the verified candidate config')
      report.images.push({ ...image, repository, publicationTag: tag, digest: remote.digest, imageRef: `${repository}@${remote.digest}`, registryConfigDigest: remote.configDigest, registryLayerCount: remote.layerCount, registryAvailability: 'VERIFIED_PUSH_AND_REGISTRY_DESCRIPTOR' }); save()
    }
    report.status = 'PASS'; return report
  } catch (error) { report.status = 'FAIL'; report.error = error.message; throw error }
  finally { report.finishedAt = new Date().toISOString(); save() }
}

export function validateBusinessPublication(report) {
  if (report?.schemaVersion !== 1 || report.kind !== 'business-image-publication' || report.status !== 'PASS' || !Array.isArray(report.images) || report.images.length !== businessImageKeys.length || new Set(report.images.map(image => image.key)).size !== businessImageKeys.length) fail('Complete actual publication evidence required')
  for (const key of businessImageKeys) {
    const image = report.images.find(value => value.key === key)
    if (!image || !digestPattern.test(image.digest || '') || image.imageRef !== `${image.repository}@${image.digest}` || !digestPattern.test(image.ociConfigDigest || '') || image.registryConfigDigest !== image.ociConfigDigest || image.daemonImageIdKind !== classifyDaemonImageIdentity(image) || image.registryAvailability !== 'VERIFIED_PUSH_AND_REGISTRY_DESCRIPTOR') fail('Invalid fixed registry image identity')
  }
  return report
}

/** Remove only this candidate's owned references before the registry pull test.
 * Cached layers/build cache can remain; this does not claim a clean daemon. */
export async function forgetOwnedBusinessImages({ docker = createDocker(), candidate, publication }) {
  validateBusinessCandidate(candidate); validateBusinessPublication(publication)
  for (const image of candidate.images) {
    const actual = first(await docker(['image', 'inspect', image.imageId]))
    if (actual.Config.Labels?.[businessOwnerLabel] !== candidate.owner) fail('Refusing to remove an unowned candidate image')
    const published = publication.images.find(value => value.key === image.key)
    if (image.tag !== `${candidate.owner}-${image.key}:candidate`) fail('Candidate tag does not belong to the declared owner and image')
    // The containerd image store can retain a local repository digest alongside
    // the source tag. It is the exact saved manifest identity for this owner,
    // separate from the registry digest learned by the actual publication.
    const candidateRepository = image.tag.slice(0, image.tag.lastIndexOf(':'))
    const localCandidateDigest = `${candidateRepository}@${image.ociManifestDigest}`
    const allowed = new Set([image.tag, localCandidateDigest, published.publicationTag, published.imageRef])
    const references = [...(actual.RepoTags || []), ...(actual.RepoDigests || [])]
    if (references.some(reference => !allowed.has(reference))) fail('Candidate has an unrelated reference; refusing image removal')
    const consumers = await docker(['container', 'ls', '--all', '--filter', `ancestor=${image.imageId}`, '--format', '{{.ID}}'])
    if (String(consumers).trim()) fail('Candidate image still has a container consumer; stop and remove owned targets first')
    for (const reference of references) {
      // Removing a tag can also remove its associated RepoDigest, while another
      // tag keeps the image itself alive. Reinspect this reference, not just Id.
      const state = await docker(['image', 'inspect', reference], { allowFailure: true })
      if (imageMissing(state)) continue
      const current = first(state)
      if (current.Id !== image.imageId || current.Config.Labels?.[businessOwnerLabel] !== candidate.owner) fail('Image reference ownership changed before removal')
      await docker(['image', 'rm', reference])
    }
    const missing = await docker(['image', 'inspect', image.imageId], { allowFailure: true })
    if (!imageMissing(missing)) fail('Candidate image ID must be absent before fixed digest pull')
  }
}

export async function fixedDigestPull({ docker: adapter = createDocker(), publication, outDir, requireCandidateAbsent = true }) {
  validateBusinessPublication(publication); freshOutput(outDir)
  const report = { schemaVersion: 1, kind: 'business-fixed-digest-pull', status: 'RUNNING', sourceRevision: publication.sourceRevision, inputsSha256: publication.inputsSha256,
    targetPlatform: publication.targetPlatform, isolation: requireCandidateAbsent ? 'same-daemon-candidate-images-absent' : 'same-daemon-explicit-registry-pull', layerCache: 'MAY_REMAIN_NOT_A_CLEAN_DAEMON', images: [], commands: [] }
  const save = () => json(path.join(outDir, 'pull-result.json'), report), docker = loggedDocker(adapter, report, outDir, save); save()
  try {
    for (const image of publication.images) {
      const beforeId = await docker('absent-id-' + image.key, ['image', 'inspect', image.imageId], { allowFailure: true })
      const beforeRef = await docker('absent-ref-' + image.key, ['image', 'inspect', image.imageRef], { allowFailure: true })
      const absent = imageMissing(beforeId) && imageMissing(beforeRef)
      if (requireCandidateAbsent && !absent) fail('The candidate image ID and fixed repository reference must both be absent before pull')
      await docker('pull-' + image.key, ['pull', '--platform', publication.targetPlatform, image.imageRef], { timeout: 15 * 60_000 })
      const actual = first(await docker('pulled-' + image.key, ['image', 'inspect', image.imageRef]))
      const daemonImageIdKind = classifyDaemonImageIdentity({ imageId: actual.Id, ociManifestDigest: image.digest, ociConfigDigest: image.ociConfigDigest })
      if (!actual.RepoDigests?.includes(image.imageRef) || `${actual.Os}/${actual.Architecture}` !== publication.targetPlatform || actual.Config.Labels?.[businessOwnerLabel] !== publication.candidateOwner || actual.Config.Labels?.['org.opencontainers.image.revision'] !== publication.sourceRevision || actual.Config.Labels?.['saas.ci.inputs-sha256'] !== publication.inputsSha256) fail('Pulled digest does not reproduce the published candidate')
      report.images.push({ key: image.key, imageId: actual.Id, pulledImageId: actual.Id, candidateImageId: image.imageId, daemonImageIdKind,
        registryManifestDigest: image.digest, ociConfigDigest: image.ociConfigDigest, imageRef: image.imageRef, candidateImageAbsentBeforePull: absent, repoDigestVerified: true }); save()
    }
    report.status = 'PASS'; return report
  } catch (error) { report.status = 'FAIL'; report.error = error.message; throw error }
  finally { report.finishedAt = new Date().toISOString(); save() }
}
export const pullBusinessImages = fixedDigestPull

export async function checkBusinessHealth({ docker = createDocker(), services, timeoutMs = 180_000, pollMs = 250, now = Date.now, sleep = ms => new Promise(resolve => setTimeout(resolve, ms)) }) {
  if (!Array.isArray(services) || !services.length || new Set(services.map(service => service.container)).size !== services.length) fail('Explicit distinct target containers are required')
  const deadline = now() + timeoutMs, checks = []
  for (const service of services) {
    if (!/^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}$/.test(service.container || '')) fail('Invalid literal target container name')
    let actual
    do {
      actual = first(await docker(['container', 'inspect', service.container]))
      if (service.imageId && actual.Image !== service.imageId) fail('Health target has a different image ID')
      if (service.imageRef && actual.Config.Image !== service.imageRef) fail('Health target was not started by its fixed image reference')
      if (!actual.State.Running || !actual.State.Health || actual.State.Health.Status === 'unhealthy') fail('BUSINESS_CONTAINER_UNHEALTHY')
      if (actual.State.Health.Status === 'healthy') break
      if (now() >= deadline) fail('BUSINESS_CONTAINER_HEALTH_TIMEOUT')
      await sleep(Math.min(pollMs, Math.max(1, deadline - now())))
    } while (now() <= deadline)
    if (actual.State.Health.Status !== 'healthy') fail('BUSINESS_CONTAINER_HEALTH_TIMEOUT')
    checks.push({ service: service.service || service.container, container: service.container, imageId: actual.Image, configuredImage: actual.Config.Image, health: 'healthy' })
  }
  return { status: 'PASS', layer: 'CI_DOCKER_HEALTH', checks }
}

function executeCli(binary, args) {
  return new Promise((resolve, reject) => {
    // Binding is offline. Do not inherit platform sessions, service credentials
    // or database URLs merely to update the local immutable journal.
    const env = Object.fromEntries(['PATH', 'Path', 'SystemRoot', 'SYSTEMROOT', 'WINDIR', 'TEMP', 'TMP'].filter(key => process.env[key]).map(key => [key, process.env[key]]))
    const child = spawn(binary, args, { cwd: root, env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = '', stderr = '', oversized = false, timedOut = false
    const timer = setTimeout(() => { timedOut = true; child.kill('SIGTERM') }, 30_000)
    const capture = (name, bytes) => {
      if (name === 'stdout') stdout += bytes; else stderr += bytes
      if (stdout.length + stderr.length > 1024 * 1024) { oversized = true; child.kill('SIGTERM') }
    }
    child.stdout.on('data', bytes => capture('stdout', bytes)); child.stderr.on('data', bytes => capture('stderr', bytes))
    child.on('error', error => { clearTimeout(timer); reject(error) })
    child.on('close', code => { clearTimeout(timer); if (code !== 0 || oversized || timedOut) reject(new Error('Rust artifact binding command failed; inspect its safe category before retry')); else resolve(stdout.trim()) })
  })
}

/** Real offline Rust CLI invocations. An injected executor can invoke the same
 * frozen CLI inside an owned Linux tool container; it must return raw stdout. */
export async function bindBusinessArtifacts({ binary, bindings, inventoryDir, execute = executeCli }) {
  if (!path.isAbsolute(binary || '') || !path.isAbsolute(inventoryDir || '') || !Array.isArray(bindings) || !bindings.length) fail('Explicit Rust CLI, durable inventory and release/artifact files are required')
  const results = []
  for (const binding of bindings) {
    if (!businessApps.includes(binding.appCode) || !/^[a-z][a-z0-9_-]{0,31}$/.test(binding.environment || '') || !path.isAbsolute(binding.releaseFile || '') || !path.isAbsolute(binding.artifactFile || '')) fail('Invalid artifact binding target')
    const args = ['--app-code', binding.appCode, '--environment', binding.environment, '--release-file', binding.releaseFile, '--artifact-file', binding.artifactFile, '--inventory-dir', inventoryDir]
    const bound = parse(await execute(binary, ['artifact-bind', ...args, '--apply']))
    const verified = parse(await execute(binary, ['artifact-verify', ...args]))
    if (bound.ok !== true || bound.mode !== 'APPLIED' || bound.bindingState !== 'BOUND' || verified.ok !== true || verified.mode !== 'VERIFIED' || verified.bindingState !== 'BOUND' || bound.externalWrites !== false || verified.externalWrites !== false || bound.remoteDigestStored !== false || verified.remoteDigestStored !== false || bound.sequence !== verified.sequence) fail('Rust CLI did not confirm the durable immutable artifact binding')
    results.push({ appCode: binding.appCode, environment: binding.environment, bind: bound, verify: verified })
  }
  return { status: 'PASS', layer: 'RUST_CLI_DURABLE_BINDING', bindings: results, platformStoresDigest: false }
}

/** A narrow controlled promotion sequence. Durable binding is supplied by the
 * Rust CLI; callbacks must stage separately, validate Nginx, and atomically
 * publish the staged config. No callback is invoked before real health passes. */
export async function promoteBusinessRoutes({ docker, services, bindArtifacts, stageRoutes, validateRoutes, commitRoutes, registerReleases, preflightReleases, activateReleases, healthOptions = {} }) {
  const operations = { bindArtifacts, stageRoutes, validateRoutes, commitRoutes, registerReleases, preflightReleases, activateReleases }
  if (Object.values(operations).some(operation => typeof operation !== 'function')) fail('Every controlled promotion operation must be explicit')
  const steps = [], health = () => checkBusinessHealth({ ...healthOptions, docker, services })
  await health(); steps.push('docker-health')
  const binding = await bindArtifacts()
  if (binding?.status !== 'PASS' || binding.layer !== 'RUST_CLI_DURABLE_BINDING' || !binding.bindings?.length || binding.bindings.some(value => value.bind?.bindingState !== 'BOUND' || value.verify?.bindingState !== 'BOUND')) fail('Durable Rust CLI binding/verification evidence is required before routing')
  steps.push('durable-artifact-bind')
  const staged = await stageRoutes(); steps.push('stage-routes')
  await validateRoutes(staged); steps.push('nginx-config-test')
  await health(); await commitRoutes(staged); steps.push('commit-routes')
  await health(); const registered = await registerReleases(); steps.push('register-draft')
  const preflight = await preflightReleases(registered)
  if (!Array.isArray(preflight) || !preflight.length || preflight.some(value => value.validationStatus !== 'PASSED')) fail('PLATFORM_PREFLIGHT_FAILED')
  steps.push('platform-preflight-passed')
  await health(); const activated = await activateReleases(registered); steps.push('activate')
  return { status: 'PASS', steps, activated, platformStoresDigest: false }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { const result = await buildBusinessImages(parseBusinessImageArgs(process.argv.slice(2))); console.log(JSON.stringify({ status: result.status, buildId: result.buildId, imageCount: result.images.length, registryPublication: result.registryPublication, targetDeployment: result.targetDeployment })) }
  catch (error) { console.error(error.message); process.exitCode = 1 }
}
