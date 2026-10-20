#!/usr/bin/env node
// Candidate images only. This command never logs in, pushes or deploys a target.
import { spawn, spawnSync } from 'node:child_process'
import { createHash, randomBytes } from 'node:crypto'
import { appendFileSync, closeSync, createWriteStream, existsSync, mkdirSync, openSync, readFileSync, readSync, statSync, writeFileSync } from 'node:fs'
import { pipeline } from 'node:stream/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { buildEnvironment, createSourceSnapshot, sourceEntries } from '../../deploy/native/build.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const digestPattern = /^sha256:[a-f0-9]{64}$/
const slash = value => value.replaceAll('\\', '/')
const json = (file, value) => writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 })

export function parseReleaseArgs(args) {
  const options = { platform: 'linux/amd64' }
  const keys = new Map([['--out-dir', 'outDir'], ['--source-root', 'sourceRoot'], ['--platform', 'platform'], ['--build-proxy', 'buildProxy']])
  const seen = new Set()
  for (let index = 0; index < args.length; index++) {
    const flag = args[index]
    if (flag === '--smoke' && !seen.has(flag)) { seen.add(flag); options.smoke = true; continue }
    const key = keys.get(flag)
    if (!key || seen.has(flag) || !args[index + 1] || args[index + 1].startsWith('--')) throw new Error('Unknown, duplicate or missing release argument')
    seen.add(flag); options[key] = args[++index]
  }
  if (!options.outDir || !path.isAbsolute(options.outDir)) throw new Error('--out-dir must be a new absolute directory')
  if (options.sourceRoot && !path.isAbsolute(options.sourceRoot)) throw new Error('--source-root must be absolute')
  if (!/^linux\/(?:amd64|arm64)$/.test(options.platform)) throw new Error('Explicit supported target platforms are linux/amd64 and linux/arm64')
  if (options.buildProxy && !/^http:\/\/(?:127\.0\.0\.1|localhost):[0-9]{1,5}$/.test(options.buildProxy)) throw new Error('Build proxy must be a credential-free loopback HTTP URL')
  return options
}

// Docker 29 saves an OCI layout. Hash each archive member without extracting
// paths or loading an entire image into RAM, then verify every referenced blob.
export function inspectOciArchive(file, targetPlatform) {
  const descriptor = openSync(file, 'r'), archiveHash = createHash('sha256')
  const files = new Map(), captured = new Map(), buffer = Buffer.alloc(1024 * 1024)
  function bytes(length) {
    const result = Buffer.alloc(length); let offset = 0
    while (offset < length) { const count = readSync(descriptor, result, offset, length - offset, null); if (!count) throw new Error('Truncated OCI archive'); offset += count }
    archiveHash.update(result); return result
  }
  try {
    while (true) {
      const header = bytes(512)
      if (header.every(value => value === 0)) {
        let count
        while ((count = readSync(descriptor, buffer, 0, buffer.length, null))) { const tail = buffer.subarray(0, count); archiveHash.update(tail); if (!tail.every(value => value === 0)) throw new Error('Unexpected data after OCI tar terminator') }
        break
      }
      const text = (start, length) => header.subarray(start, start + length).toString('utf8').replace(/\0.*$/s, '')
      let checksum = 0
      for (let index = 0; index < header.length; index++) checksum += index >= 148 && index < 156 ? 32 : header[index]
      if (parseInt(text(148, 8).trim(), 8) !== checksum) throw new Error('Invalid OCI tar header checksum')
      const prefix = text(345, 155), name = `${prefix ? `${prefix}/` : ''}${text(0, 100)}`.replace(/\/$/, '')
      if (!name || name.startsWith('/') || name.split('/').some(part => !part || part === '.' || part === '..') || /[\\\x00-\x1f]/.test(name)) throw new Error('Unsafe OCI tar member path')
      const type = String.fromCharCode(header[156]), sizeText = text(124, 12).trim()
      if (!/^[0-7]+$/.test(sizeText)) throw new Error('Unsupported OCI tar member size')
      const size = parseInt(sizeText, 8)
      if (!Number.isSafeInteger(size) || size < 0 || !['\0', '0', '5'].includes(type)) throw new Error('Unsupported OCI tar member type or size')
      if (type === '5') { if (size !== 0) throw new Error('OCI tar directory has content'); continue }
      if (files.has(name)) throw new Error('Duplicate OCI archive member')
      const memberHash = createHash('sha256'), chunks = [], capture = size <= 4 * 1024 * 1024
      let remaining = size
      while (remaining) { const chunk = bytes(Math.min(remaining, buffer.length)); memberHash.update(chunk); if (capture) chunks.push(chunk); remaining -= chunk.length }
      files.set(name, { bytes: size, sha256: memberHash.digest('hex') })
      if (capture) captured.set(name, Buffer.concat(chunks))
      if (size % 512) bytes(512 - size % 512)
    }
  } finally { closeSync(descriptor) }
  const parse = name => { const content = captured.get(name); if (!content) throw new Error(`Missing OCI metadata: ${name}; Docker 29 OCI save support is required`); return JSON.parse(content) }
  const layout = parse('oci-layout'), index = parse('index.json')
  if (layout.imageLayoutVersion !== '1.0.0' || index.schemaVersion !== 2 || index.manifests?.length !== 1) throw new Error('A single-image OCI layout is required')
  function blob(record) {
    if (!record || !digestPattern.test(record.digest)) throw new Error('Invalid OCI blob descriptor')
    const name = `blobs/sha256/${record.digest.slice(7)}`, actual = files.get(name)
    if (!actual || actual.sha256 !== record.digest.slice(7) || actual.bytes !== record.size) throw new Error(`OCI blob digest/size mismatch: ${record.digest}`)
    return name
  }
  const manifestDescriptor = index.manifests[0]
  if (manifestDescriptor.mediaType !== 'application/vnd.oci.image.manifest.v1+json') throw new Error('OCI image manifest required')
  const manifest = parse(blob(manifestDescriptor))
  if (manifest.schemaVersion !== 2 || !Array.isArray(manifest.layers) || !manifest.layers.length) throw new Error('Invalid OCI image manifest')
  const config = parse(blob(manifest.config))
  for (const layer of manifest.layers) blob(layer)
  if (`${config.os}/${config.architecture}` !== targetPlatform) throw new Error('OCI target platform differs from requested platform')
  return { archiveSha256: archiveHash.digest('hex'), archiveBytes: statSync(file).size, ociManifestDigest: manifestDescriptor.digest,
    ociConfigDigest: manifest.config.digest, targetPlatform, verifiedLayerCount: manifest.layers.length, labels: config.config?.Labels || {} }
}

export const platformProbe = 'printf "UID="; id -u; printf "BINARY_SHA256="; sha256sum /usr/local/bin/saas-platform-backend; printf "ELF_HEADER="; od -An -tx1 -N20 /usr/local/bin/saas-platform-backend | tr -d "\\n"; printf "\\n"; ldd /usr/local/bin/saas-platform-backend; for binary in node npm bun caddy; do if command -v "$binary"; then exit 1; fi; done'

export function parsePlatformProbe(output, targetPlatform) {
  const uid = /^UID=(\d+)$/m.exec(output)?.[1], binarySha256 = /^BINARY_SHA256=([a-f0-9]{64})\s+\/usr\/local\/bin\/saas-platform-backend$/m.exec(output)?.[1]
  const header = /^ELF_HEADER=([^\r\n]+)$/m.exec(output)?.[1]?.trim().split(/\s+/).map(byte => parseInt(byte, 16))
  if (uid !== '10001' || !binarySha256 || !header || header.length !== 20 || header.some(byte => !Number.isInteger(byte) || byte < 0 || byte > 255)
    || Buffer.from(header.slice(0, 4)).toString('hex') !== '7f454c46' || header[4] !== 2 || header[5] !== 1) throw new Error('Invalid nonroot platform ELF binary probe')
  const machine = header[18] | (header[19] << 8)
  if (machine !== (targetPlatform === 'linux/amd64' ? 62 : 183)) throw new Error('Platform ELF machine differs from requested target architecture')
  if (/not found|not a dynamic executable|statically linked/i.test(output) || !/libc\.so/.test(output)) throw new Error('Platform binary runtime shared libraries were not resolved')
  return { binarySha256, elfMachine: machine, uid: Number(uid), targetPlatform, sharedLibrariesResolved: true, noServiceStarted: true, network: 'none', readOnlyRootfs: true }
}

export async function buildCandidate(options) {
  const outDir = path.resolve(options.outDir), sourceRoot = path.resolve(options.sourceRoot || root)
  if (existsSync(outDir)) throw new Error('Release output already exists; choose a new owned directory')
  if (sourceRoot === outDir || sourceRoot.startsWith(outDir + path.sep)) throw new Error('Release output may not contain source checkout')
  if (sourceEntries.some(entry => { const input = path.resolve(sourceRoot, entry); return input === outDir || outDir.startsWith(input + path.sep) })) throw new Error('Release output overlaps source inputs')
  mkdirSync(outDir, { recursive: true, mode: 0o700 }); mkdirSync(path.join(outDir, 'logs')); mkdirSync(path.join(outDir, 'images'))
  const owner = `phase04-${Date.now()}-${randomBytes(4).toString('hex')}`
  const result = { schemaVersion: 1, owner, status: 'BUILDING', build: 'NOT_RUN', smoke: 'NOT_RUN', registryPublication: 'NOT_REQUESTED', targetDeployment: 'NOT_RUN',
    targetPlatform: options.platform, startedAt: new Date().toISOString(), images: [], commands: [] }
  const save = () => json(path.join(outDir, 'release-result.json'), result)
  const windows = process.platform === 'win32'
  const linuxPath = value => windows ? `/mnt/${path.resolve(value)[0].toLowerCase()}/${slash(path.resolve(value).slice(3))}` : path.resolve(value)
  const dockerProgram = windows ? 'wsl.exe' : 'docker'
  const dockerPrefix = windows ? ['-d', process.env.PHASE03_WSL_DISTRO || 'Ubuntu', '--exec', 'env', 'BUILDX_NO_DEFAULT_ATTESTATIONS=1', 'docker', '--host', 'unix:///var/run/docker.sock'] : ['--host', 'unix:///var/run/docker.sock']
  const env = buildEnvironment(outDir)
  // Keep the saved candidate a single-platform image on BuildKit runners as
  // well as the local legacy builder. Provenance is the explicit source/image
  // manifest below; no unverified attestation image is silently discarded.
  env.BUILDX_NO_DEFAULT_ATTESTATIONS = '1'
  async function command(name, program, args, { timeout = 2_700_000, extraEnv = {} } = {}) {
    const log = `logs/${String(result.commands.length + 1).padStart(2, '0')}-${name}.log`, destination = path.join(outDir, log)
    const record = { name, executable: path.basename(program), args, log, exitCode: null }; result.commands.push(record); save()
    writeFileSync(destination, '', { mode: 0o600 })
    return new Promise((resolve, reject) => {
      const child = spawn(program, args, { cwd: sourceRoot, env: { ...env, ...extraEnv }, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] }); let output = '', timedOut = false
      const timer = setTimeout(() => { timedOut = true; child.kill() }, timeout)
      for (const stream of [child.stdout, child.stderr]) stream.on('data', chunk => { appendFileSync(destination, chunk); output = (output + chunk).slice(-8 * 1024 * 1024) })
      child.on('error', error => { clearTimeout(timer); record.error = error.message; save(); reject(error) })
      child.on('close', code => { clearTimeout(timer); record.exitCode = code; save(); if (timedOut || code !== 0) reject(new Error(`${name} ${timedOut ? 'timed out' : `exited ${code}`}; see ${log}`)); else resolve(output.trim()) })
    })
  }
  const docker = (name, args, settings) => command(name, dockerProgram, [...dockerPrefix, ...args], settings)
  const exportStage = `/var/tmp/saas-ci-${owner}-export`
  let exportStageCreated = false
  async function copyWslArchive(name, source, destination) {
    const args = ['-d', process.env.PHASE03_WSL_DISTRO || 'Ubuntu', '--exec', 'cat', source]
    const log = `logs/${String(result.commands.length + 1).padStart(2, '0')}-copy-${name}-archive.log`, logPath = path.join(outDir, log)
    const record = { name: `copy-${name}-archive`, executable: 'wsl.exe', args, log, exitCode: null, output: path.relative(outDir, destination) }
    result.commands.push(record); writeFileSync(logPath, '', { mode: 0o600 }); save()
    const child = spawn('wsl.exe', args, { env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
    child.stderr.on('data', chunk => appendFileSync(logPath, chunk))
    const timer = setTimeout(() => child.kill(), 120_000)
    try {
      await Promise.all([pipeline(child.stdout, createWriteStream(destination, { flags: 'wx', mode: 0o600 })), new Promise((resolve, reject) => {
        child.on('error', reject); child.on('close', code => { record.exitCode = code; code === 0 ? resolve() : reject(new Error(`WSL archive stream exited ${code}`)) })
      })])
    } catch (error) { child.kill(); record.error = error.message; throw error }
    finally { clearTimeout(timer); save() }
  }
  try {
    const head = spawnSync('git', ['-c', `safe.directory=${slash(sourceRoot)}`, '-C', sourceRoot, 'rev-parse', 'HEAD'], { encoding: 'utf8', windowsHide: true })
    if (head.status !== 0 || !/^[a-f0-9]{40}$/.test(head.stdout.trim())) throw new Error('Full Git commit required')
    result.commit = head.stdout.trim()
    const snapshot = path.join(outDir, 'source'), inputs = createSourceSnapshot(sourceRoot, snapshot)
    result.inputsSha256 = inputs.inputsSha256; result.buildId = `${result.commit.slice(0, 12)}-${inputs.inputsSha256.slice(0, 16)}`
    json(path.join(outDir, 'source-manifest.json'), { schemaVersion: 1, commit: result.commit, buildId: result.buildId, ...inputs,
      revisionMeaning: 'commit identifies HEAD; files/inputsSha256 identify actual working-tree bytes including uncommitted inputs' })
    result.tools = { node: process.version, docker: JSON.parse(await docker('docker-version', ['version', '--format', '{{json .}}'], { timeout: 30_000 })) }
    if (Number(result.tools.docker.Server.Version.split('.')[0]) < 29) throw new Error('Docker Engine 29+ is required for verified OCI archive output')
    if (windows) {
      // DrvFS writes can return ENOMEM under parallel browser/compile load.
      // Save on Linux ext4, then stream bytes through a bounded Node pipeline
      // into a native Windows file; no image-sized buffer or DrvFS write.
      await command('prepare-image-export', 'wsl.exe', ['-d', process.env.PHASE03_WSL_DISTRO || 'Ubuntu', '--exec', 'mkdir', '--mode=700', exportStage])
      exportStageCreated = true; result.imageExport = { mode: 'wsl-native-save-windows-stream', stagingRoot: exportStage, cleanup: 'NOT_RUN' }
    }
    for (const [name, dockerfile] of [['frontend', 'frontend/Dockerfile'], ['edge', 'deploy/gateway/Dockerfile'], ['platform', 'backend/Dockerfile']]) {
      const tag = `${owner}-${name}:candidate`
      console.log(`Building ${name} candidate for ${options.platform}`)
      const proxy = options.buildProxy ? ['--network', 'host', ...['HTTPS_PROXY', 'HTTP_PROXY', 'https_proxy', 'http_proxy'].flatMap(key => ['--build-arg', `${key}=${options.buildProxy}`])] : []
      await docker(`build-${name}`, ['build', '--force-rm', '--platform', options.platform, ...proxy,
        '--label', `saas.ci.owner=${owner}`, '--label', `org.opencontainers.image.revision=${result.commit}`,
        '--label', `org.opencontainers.image.version=${result.buildId}`, '--label', `saas.ci.inputs-sha256=${result.inputsSha256}`,
        '--build-arg', `SOURCE_REVISION=${result.commit}`, '--build-arg', `HOST_BUILD_ID=${result.buildId}`,
        '--file', linuxPath(path.join(snapshot, dockerfile)), '--tag', tag, linuxPath(snapshot)])
      const inspection = JSON.parse(await docker(`inspect-${name}`, ['image', 'inspect', tag]))[0]
      if (`${inspection.Os}/${inspection.Architecture}` !== options.platform || !digestPattern.test(inspection.Id)) throw new Error('Built image platform/identity mismatch')
      const archive = `images/${name}.tar`
      const savedArchive = windows ? `${exportStage}/${name}.tar` : linuxPath(path.join(outDir, archive))
      await docker(`save-${name}`, ['image', 'save', '--output', savedArchive, tag])
      if (windows) await copyWslArchive(name, savedArchive, path.join(outDir, archive))
      const archiveMetadata = inspectOciArchive(path.join(outDir, archive), options.platform)
      if (archiveMetadata.labels['org.opencontainers.image.revision'] !== result.commit || archiveMetadata.labels['saas.ci.inputs-sha256'] !== result.inputsSha256) throw new Error('OCI source provenance label mismatch')
      let runtimeVerification
      if (name === 'platform') runtimeVerification = parsePlatformProbe(await docker('probe-platform-binary', ['run', '--rm', '--name', `${owner}-platform-probe`, '--label', `saas.ci.owner=${owner}`,
        '--network', 'none', '--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', '--entrypoint', 'sh', inspection.Id, '-ec', platformProbe], { timeout: 30_000 }), options.platform)
      result.images.push({ name, tag, imageId: inspection.Id, identityKind: 'local-docker-image-id', commit: result.commit, buildId: result.buildId,
        inputsSha256: result.inputsSha256, archive, ...archiveMetadata, repoDigests: inspection.RepoDigests || [],
        runtimeVerification,
        registryAvailability: 'NOT_VERIFIED_NO_PUSH', distributionDigestMeaning: 'ociManifestDigest identifies the verified saved OCI manifest; it does not prove registry publication' }); save()
    }
    result.build = 'PASS'
    if (options.smoke) {
      result.smoke = 'RUNNING'; save()
      await command('delivery-smoke', process.execPath, ['--test', path.join(root, 'tests/micro-release-gateways.mjs')], { extraEnv: {
        PHASE03_WSL_DISTRO: process.env.PHASE03_WSL_DISTRO || 'Ubuntu', PHASE03_BUILD_PROXY: options.buildProxy || '',
        PHASE03_HOST_IMAGE: result.images.find(image => image.name === 'frontend').imageId,
        PHASE03_EDGE_IMAGE: result.images.find(image => image.name === 'edge').imageId,
        PHASE03_EVIDENCE_ROOT: path.join(outDir, 'smoke'), PHASE03_RELEASE_BUILD_ID: result.buildId,
      } })
      const smoke = JSON.parse(readFileSync(path.join(outDir, 'smoke/result.json'), 'utf8'))
      if (smoke.result !== 'PASS' || smoke.checks.length !== 7 || smoke.checks.some(check => check.result !== 'PASS') || smoke.cleanupErrors?.length) throw new Error('Delivery smoke result lacks all seven successful actual container checks')
      result.smoke = 'PASS'; result.smokeEvidence = 'smoke/result.json'
    }
    result.status = 'PASS'; return result
  } catch (error) { result.status = 'FAIL'; if (result.build !== 'PASS') result.build = 'FAIL'; else if (result.smoke === 'RUNNING') result.smoke = 'FAIL'; result.error = error.message; throw error }
  finally {
    let cleanupError
    if (exportStageCreated) try {
      const cleanup = 'import os,pathlib,sys\np=pathlib.Path(sys.argv[1])\nassert p.parent==pathlib.Path("/var/tmp") and p.name==sys.argv[2] and not p.is_symlink() and p.stat().st_uid==os.getuid()\nfor f in p.iterdir():\n assert f.is_file() and not f.is_symlink()\n f.unlink()\np.rmdir()'
      await command('cleanup-image-export', 'wsl.exe', ['-d', process.env.PHASE03_WSL_DISTRO || 'Ubuntu', '--exec', 'python3', '-c', cleanup, exportStage, path.posix.basename(exportStage)], { timeout: 30_000 })
      result.imageExport.cleanup = 'PASS'
    } catch (error) { cleanupError = error; result.imageExport.cleanup = 'FAIL'; result.status = 'FAIL'; result.cleanupError = error.message }
    result.finishedAt = new Date().toISOString(); save()
    if (cleanupError) throw cleanupError
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { const result = await buildCandidate(parseReleaseArgs(process.argv.slice(2))); console.log(JSON.stringify({ status: result.status, buildId: result.buildId, build: result.build, smoke: result.smoke, registryPublication: result.registryPublication, targetDeployment: result.targetDeployment })) }
  catch (error) { console.error(error.message); process.exitCode = 1 }
}
