#!/usr/bin/env node
/** Host-only Docker control plane for owned, real business image delivery. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createHash, randomBytes } from 'node:crypto'
import { chmodSync, copyFileSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, realpathSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { createDocker, createSnapshot, validateOutputLocation } from './linux-runner.mjs'
import { createSourceManifest } from './provenance.mjs'
import { buildBusinessImages, publishBusinessImages, forgetOwnedBusinessImages, fixedDigestPull, checkBusinessHealth, promoteBusinessRoutes, bindBusinessArtifacts } from './business-images.mjs'
import { businessBrowserProducts, businessBrowserChecks, businessDeliveryChecks, validateBusinessDeliveryReport, parseBusinessBrowserOutput } from './business-delivery-results.mjs'
import { renderGateway } from '../../deploy/gateway/render.mjs'
import { startWslLease } from '../../tests/zip-release-e2e.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const label = 'saas.ci.owner'
const toolsImage = process.env.CI_TOOLCHAIN_IMAGE || 'sha256:1912f7302185ed2c1af4df155d0d2784886c87984266ed981325575e415d71dc'
const origin = 'https://host.phase28.test:8443', externalOrigin = 'https://external.phase28.test:8443'
const appCodes = ['work-orders', 'stores']
const sha = value => createHash('sha256').update(value).digest('hex')
const linux = value => process.platform === 'win32' ? '/mnt/' + path.resolve(value)[0].toLowerCase() + '/' + path.resolve(value).slice(3).replaceAll('\\', '/') : path.resolve(value)
const json = (file, value) => writeFileSync(file, JSON.stringify(value, null, 2) + '\n', { mode: 0o600 })
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
function staticTree(directory) {
  const files = []
  function visit(relative = '') {
    for (const name of readdirSync(path.join(directory, relative)).sort()) {
      const child = path.join(relative, name), file = path.join(directory, child), stat = lstatSync(file)
      assert(!stat.isSymbolicLink(), 'Static evidence must not contain links')
      if (stat.isDirectory()) visit(child)
      else { assert(stat.isFile(), 'Static evidence must be regular files'); const bytes = readFileSync(file); files.push({ path: child.replaceAll('\\', '/'), bytes: bytes.length, sha256: sha(bytes) }) }
    }
  }
  visit(); return files
}
/** Unlike the shared string adapter, this boundary preserves actual exit codes
 * for expected rejections; a negative probe is never reported as exit zero. */
export async function executeBusinessDocker(args, { timeout = 30 * 60_000, signal } = {}) {
  const program = process.platform === 'win32' ? 'wsl.exe' : 'docker'
  const prefix = process.platform === 'win32' ? ['-d', process.env.CI_WSL_DISTRO || 'Ubuntu', '--exec', 'docker'] : []
  const child = spawn(program, [...prefix, '--host', 'unix:///var/run/docker.sock', ...args], { cwd: root, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
  let output = '', timedOut = false, aborted = false, outputLimitExceeded = false
  for (const stream of [child.stdout, child.stderr]) stream.on('data', bytes => { output += bytes; if (output.length > 32 * 1024 * 1024) { outputLimitExceeded = true; child.kill('SIGTERM') } })
  const abort = () => { aborted = true; child.kill('SIGTERM') }
  signal?.addEventListener('abort', abort, { once: true }); if (signal?.aborted) abort()
  const timer = setTimeout(() => { timedOut = true; child.kill('SIGTERM') }, timeout)
  try {
    const exitCode = await new Promise((resolve, reject) => { child.once('error', reject); child.once('close', resolve) })
    return { output: output.trim(), exitCode, timedOut, aborted, outputLimitExceeded }
  } finally { clearTimeout(timer); signal?.removeEventListener('abort', abort) }
}
export function parseBusinessDeliveryArgs(args) {
  const options = { targetPlatform: 'linux/amd64' }, seen = new Set(), keys = { '--out-dir': 'outDir', '--source-revision': 'sourceRevision', '--platform': 'targetPlatform', '--build-proxy': 'buildProxy' }
  for (let index = 0; index < args.length; index += 2) {
    const key = keys[args[index]], value = args[index + 1]
    if (!key || !value || seen.has(key)) throw new Error('Unknown, repeated or missing business delivery argument')
    seen.add(key)
    options[key] = value
  }
  if (!path.isAbsolute(options.outDir || '') || !/^[a-f0-9]{40}$/.test(options.sourceRevision || '') || options.targetPlatform !== 'linux/amd64') throw new Error('Fresh absolute output, full revision and linux/amd64 are required')
  if (options.buildProxy && !/^http:\/\/(?:127\.0\.0\.1|localhost):[1-9][0-9]{0,4}$/.test(options.buildProxy)) throw new Error('Build proxy must be a credential-free loopback HTTP URL')
  return options
}

export async function runBusinessDelivery(options) {
  const sourceRoot = path.resolve(options.sourceRoot || root), outDir = path.resolve(options.outDir)
  validateOutputLocation(outDir, sourceRoot); assert(!existsSync(outDir), 'Fresh delivery output required')
  for (const directory of ['logs', 'screenshots', 'private', 'artifacts/original-binaries', 'artifacts/static']) mkdirSync(path.join(outDir, directory), { recursive: true, mode: 0o700 })
  const privateRoot = realpathSync(path.join(outDir, 'private'))
  const owner = 'ci-phase28-' + Date.now() + '-' + randomBytes(4).toString('hex')
  const report = { schemaVersion: 1, kind: 'business-delivery', status: 'FAIL', owner, sourceRevision: options.sourceRevision, targetPlatform: options.targetPlatform || 'linux/amd64',
    startedAt: new Date().toISOString(), launcherCommand: [process.execPath, ...process.argv.slice(1)], checks: businessDeliveryChecks.map(id => ({ id, status: 'NOT_RUN' })), browserProducts: [], images: [], commands: [], runtimeProofs: [],
    registry: { scope: 'isolated', push: 'NOT_RUN', pull: 'NOT_RUN', productionPublication: false }, toolLeases: [], originalBinaries: [], staticArtifacts: [], cleanup: { errors: [] } }
  const save = () => json(path.join(outDir, 'result.json'), report)
  const secretValues = new Set(), adapter = options.docker || createDocker(), containers = new Set(), networks = new Set(), ephemeralVolumes = new Set()
  let lease, builder, cache, inventoryVolume, cancelled = false, cleanupMode = false
  const cancellation = new AbortController()
  const toolAttachments = new Map()
  const clean = value => {
    let text = String(value).replace(/Bearer\s+[^\s"']+/gi, 'Bearer [redacted]').replace(/postgres(?:ql)?:\/\/[^\s"']+/gi, '[database-url-redacted]')
    for (const secret of secretValues) if (secret) text = text.replaceAll(secret, '[redacted]')
    return text
  }
  const cancel = reason => { cancelled = true; report.cancelled = true; report.status = 'FAIL'; process.exitCode = 1; report.cancellationReason = reason; cancellation.abort(); save() }
  const interrupt = () => cancel('SIGINT'), terminate = () => cancel('SIGTERM')
  process.once('SIGINT', interrupt); process.once('SIGTERM', terminate)
  async function docker(args, settings = {}) {
    if (cancelled && !cleanupMode) throw new Error('Delivery foreground lease ended')
    const index = report.commands.length + 1, record = { args: args.map(clean), log: 'logs/' + String(index).padStart(4, '0') + '-docker.log', exitCode: null }
    report.commands.push(record); save()
    try {
      const result = options.docker ? { output: await adapter(args, settings), exitCode: settings.allowFailure ? null : 0, timedOut: false } : await executeBusinessDocker(args, { ...settings, signal: cleanupMode ? undefined : cancellation.signal })
      record.exitCode = result.exitCode; record.timedOut = result.timedOut; record.aborted = result.aborted || false; record.outputLimitExceeded = result.outputLimitExceeded || false
      if (result.exitCode === null) record.exitStatus = 'NOT_EXPOSED_BY_INJECTED_ADAPTER'
      writeFileSync(path.join(outDir, record.log), clean(result.output) + '\n'); save()
      if (result.timedOut || result.aborted || result.outputLimitExceeded || result.exitCode !== 0 && !settings.allowFailure && result.exitCode !== null) throw new Error('Docker operation failed; see ' + record.log)
      return result.output
    }
    catch (error) { if (!existsSync(path.join(outDir, record.log))) { record.exitStatus = 'SPAWN_OR_ADAPTER_ERROR'; writeFileSync(path.join(outDir, record.log), clean(error.message) + '\n') } record.error = clean(error.message); save(); throw new Error('Docker operation failed; see ' + record.log, { cause: error }) }
  }
  async function inspect(kind, name) { return JSON.parse(await docker([kind, 'inspect', name]))[0] }
  async function owned(kind, name) {
    const output = await adapter([kind, 'inspect', name], { allowFailure: true })
    if (/No such (?:container|network|volume|object)|not found/i.test(output)) return null
    const actual = JSON.parse(output)[0]
    assert.equal(actual.Config?.Labels?.[label] || actual.Labels?.[label], owner, 'Refuse nonowned runtime resource')
    return actual
  }
  async function makeNetwork(name) { networks.add(name); await docker(['network', 'create', '--label', label + '=' + owner, name]); return name }
  async function create(name, args) {
    containers.add(name)
    await docker(['create', '--name', name, '--label', label + '=' + owner, ...args])
    return name
  }
  async function start(name) { await docker(['start', name]); return name }
  async function cpTo(file, name, destination) {
    await docker(['cp', linux(file), name + ':' + destination])
    // Docker's archive copy can retain DrvFS uid 1000. These root tool
    // containers have CHOWN but deliberately lack FOWNER/DAC_OVERRIDE; adopt
    // this exact input file before chmod or reading mode-0600 runtime JSON.
    await exec(name, ['chown', '0:0', destination])
  }
  async function cpFrom(name, source, destination) { await docker(['cp', name + ':' + source, linux(destination)]) }
  async function exec(name, args, settings) { return docker(['exec', name, ...args], settings) }
  async function eventually(action, message, timeout = 120_000) {
    const deadline = Date.now() + timeout
    let error
    do { try { return await action() } catch (caught) { error = caught; await pause(200) } } while (Date.now() < deadline)
    throw new Error(message, { cause: error })
  }
  const hardening = ['--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', '--tmpfs', '/tmp:rw,nosuid,noexec,size=64m']
  const toolSecurity = ['--cap-drop', 'ALL', '--cap-add', 'CHOWN', '--cap-add', 'SETUID', '--cap-add', 'SETGID', '--security-opt', 'no-new-privileges']
  async function sourceTo(name) {
    // The numeric owner is intentional: browser ownership normalization must
    // prove its source inputs without adopting DrvFS's synthetic uid 1000.
    const program = process.platform === 'win32' ? 'wsl.exe' : 'bash'
    const prefix = process.platform === 'win32' ? ['-d', process.env.CI_WSL_DISTRO || 'Ubuntu', '--exec', 'bash'] : []
    const args = [...prefix, '-c', 'set -euo pipefail; tar --format=posix --numeric-owner --owner=0 --group=0 -C "$1" -cf - . | docker --host unix:///var/run/docker.sock cp - "$2:/workspace"', 'phase28-source-transfer', linux(path.join(outDir, 'source')), name]
    const child = spawn(program, args, { cwd: root, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] }); let output = ''
    child.stdout.on('data', bytes => { output += bytes }); child.stderr.on('data', bytes => { output += bytes })
    assert.equal(await new Promise(resolve => { child.once('error', () => resolve(-1)); child.once('close', resolve) }), 0, 'Owned source transfer failed: ' + clean(output).slice(-500))
  }
  function passed(id, evidence = {}) { Object.assign(report.checks.find(check => check.id === id), { status: 'PASS', evidence }); save() }
  async function tool(name, network, extra = []) {
    await create(name, ['--interactive', '--network', network, '--workdir', '/workspace', '--init', '--shm-size', '1g', ...toolSecurity,
      '--env', 'CI=true', '--env', 'CI_SOURCE_REVISION=' + report.sourceRevision, '--env', 'CARGO_TARGET_DIR=/owned-cache/target', '--env', 'PLAYWRIGHT_BROWSERS_PATH=/opt/ci/browser/.browsers',
      '--mount', 'type=volume,source=' + cache.name + ',target=/owned-cache', '--mount', 'type=volume,source=' + inventoryVolume + ',target=/runtime/artifact-inventory', ...extra, toolsImage, 'node', '-e', 'console.log("PHASE28_TOOL_READY");process.stdin.resume();process.stdin.on("end",()=>{console.log("PHASE28_TOOL_EOF");process.exit(0)})'])
    const program = process.platform === 'win32' ? 'wsl.exe' : 'docker', prefix = process.platform === 'win32' ? ['-d', process.env.CI_WSL_DISTRO || 'Ubuntu', '--exec', 'docker'] : []
    const argv = [...prefix, '--host', 'unix:///var/run/docker.sock', 'start', '--attach', '--interactive', name]
    const child = spawn(program, argv, { cwd: root, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] })
    const proof = { container: name, launcher: [program, ...argv], launcherPid: child.pid, ready: false, stopped: false, eofObserved: false, unexpectedExit: false, exitCode: null }
    let expected = false, output = ''
    const exited = new Promise(resolve => { child.once('error', () => { proof.unexpectedExit = !expected; if (!expected) cancel('tool-lease-error'); resolve(-1) }); child.once('close', code => { proof.exitCode = code; proof.stopped = expected; proof.unexpectedExit = !expected; if (!expected) cancel('tool-lease-exit'); resolve(code) }) })
    const consume = bytes => { output += bytes; proof.ready ||= output.includes('PHASE28_TOOL_READY'); proof.eofObserved ||= output.includes('PHASE28_TOOL_EOF') }
    child.stdout.on('data', consume); child.stderr.on('data', consume); child.stdin.on('error', () => {})
    toolAttachments.set(name, { child, proof, exited, get output() { return output }, expectExit() { expected = true } }); report.toolLeases.push(proof)
    await eventually(async () => assert(proof.ready && !proof.unexpectedExit), 'Owned foreground tool lease did not start', 30_000)
    const actual = await inspect('container', name)
    assert.equal(actual.HostConfig.Privileged, false); assert(!actual.Mounts.some(mount => /docker\.sock/.test(mount.Source)))
    await sourceTo(name); await exec(name, ['mkdir', '-p', '/runtime', '/reports/business-browser'])
    return name
  }
  async function closeTool(name) {
    const attachment = toolAttachments.get(name)
    if (!attachment) return
    attachment.expectExit(); attachment.child.stdin.end()
    const timeout = setTimeout(() => attachment.child.kill(), 15_000)
    try {
      assert.equal(await attachment.exited, 0); assert(attachment.proof.eofObserved, 'Tool EOF marker missing'); attachment.proof.stopped = true
      const bytes = Buffer.from(clean(attachment.output)); attachment.proof.log = 'logs/' + name + '-lease.log'; attachment.proof.sha256 = sha(bytes); writeFileSync(path.join(outDir, attachment.proof.log), bytes)
      const state = await inspect('container', name); attachment.proof.containerExitCode = state.State.ExitCode; attachment.proof.containerRunningAfterEOF = state.State.Running; assert.equal(state.State.Running, false); assert.equal(state.State.ExitCode, 0)
    }
    finally { clearTimeout(timeout); toolAttachments.delete(name) }
  }
  async function dockerBuild(tag, context, dockerfile = 'Dockerfile') {
    const proxy = options.buildProxy ? ['--network', 'host', ...['HTTP_PROXY', 'HTTPS_PROXY'].flatMap(key => ['--build-arg', key + '=' + options.buildProxy])] : []
    await docker(['build', '--force-rm', '--platform', 'linux/amd64', ...proxy, '--label', label + '=' + owner, '--label', 'org.opencontainers.image.revision=' + report.sourceRevision,
      '--label', 'saas.ci.inputs-sha256=' + report.inputsSha256, '-t', tag, '-f', linux(path.join(context, dockerfile)), linux(context)], { timeout: 60 * 60_000 })
    return (await inspect('image', tag)).Id
  }
  async function privateRead(container, file) {
    // Do not call the logging adapter: fixture state and policy are private.
    const bytes = await adapter(['exec', container, 'node', '-e', 'process.stdout.write(require("node:fs").readFileSync(process.argv[1]))', file])
    return JSON.parse(bytes)
  }
  async function writeRuntime(container, value, suffix = 'config.json') {
    const file = path.join(privateRoot, container + '-' + suffix); json(file, value); await cpTo(file, container, '/runtime/' + suffix)
  }
  try {
    if (process.platform === 'win32') lease = await (options.startLease || startWslLease)({ owner, distro: process.env.CI_WSL_DISTRO || 'Ubuntu', onUnexpectedExit() { cancel('wsl-lease-exit') } })
    report.toolchainImage = (await inspect('image', toolsImage)).Id
    const snapshot = path.join(outDir, 'source'); createSnapshot(snapshot, sourceRoot)
    const source = createSourceManifest(snapshot, { sourceRevision: options.sourceRevision }); report.inputsSha256 = source.inputsSha256
    report.sourceBefore = { sourceRevision: source.sourceRevision, inputsSha256: source.inputsSha256, fileCount: source.files.length }
    const hostSourceRoot = process.env.BUSINESS_DELIVERY_HOST_SOURCE_ROOT ? path.resolve(process.env.BUSINESS_DELIVERY_HOST_SOURCE_ROOT) : sourceRoot
    report.sourceRoot = sourceRoot; report.hostSourceRoot = hostSourceRoot
    const hostBefore = createSourceManifest(hostSourceRoot, { sourceRevision: options.sourceRevision }); report.hostSourceBefore = { sourceRevision: hostBefore.sourceRevision, inputsSha256: hostBefore.inputsSha256, fileCount: hostBefore.files.length }
    assert.equal(hostBefore.inputsSha256, source.inputsSha256, 'Host and frozen candidate source differ before execution')
    report.driverArtifacts = ['scripts/ci/business-delivery.mjs', 'scripts/ci/business-delivery-results.mjs', 'scripts/ci/business-images.mjs', 'tests/helpers/phase28-delivery-services.mjs', 'tests/phase28-delivery-browser.mjs'].map(file => source.files.find(item => item.path === file))
    assert(report.driverArtifacts.every(Boolean), 'Frozen driver inputs missing')
    json(path.join(outDir, 'source-manifest.json'), source); save()
    const cacheFile = process.env.BUSINESS_DELIVERY_CACHE_FILE || path.join(root, 'target/phase-28/validation-runner/cache-owner.json')
    if (existsSync(cacheFile)) {
      cache = JSON.parse(readFileSync(cacheFile, 'utf8')); assert.match(cache.owner, /^ci-phase28-cache-/)
      assert.equal((await inspect('volume', cache.name)).Labels[label], cache.owner)
    } else {
      cache = { owner, name: owner + '-cargo' }; ephemeralVolumes.add(cache.name); await docker(['volume', 'create', '--label', label + '=' + owner, cache.name])
    }
    report.cache = { name: cache.name, owner: cache.owner, retainedForPhase: !ephemeralVolumes.has(cache.name) }
    inventoryVolume = owner + '-artifact-inventory'; ephemeralVolumes.add(inventoryVolume); await docker(['volume', 'create', '--label', label + '=' + owner, inventoryVolume])
    report.artifactInventory = { name: inventoryVolume, owner, lifecycle: 'owned persistent Docker volume; survives tool container recreation; removed at final cleanup', bindingLayer: 'Rust CLI local journal; platform does not store image digest' }
    const buildNetwork = await makeNetwork(owner + '-build-network')
    builder = await tool(owner + '-builder', buildNetwork)
    report.tools = { node: await exec(builder, ['node', '--version']), npm: await exec(builder, ['npm', '--version']), rust: await exec(builder, ['rustc', '--version']), nginx: await exec(builder, ['nginx', '-v']) }
    assert.equal(report.tools.node, 'v24.18.0'); assert.equal(report.tools.npm, '11.16.0'); assert.match(report.tools.rust, /^rustc 1\.97\.0\b/); assert.match(report.tools.nginx, /nginx\/1\.28\.0\b/)
    const packages = ['frontend', 'tests/browser', 'mini-services/notification-ws']
    for (const directory of packages) await exec(builder, ['npm', '--prefix', directory, 'ci', '--no-audit', '--no-fund'], { timeout: 8 * 60_000 })
    for (const [id, manifest, binary] of [['platform', 'backend/Cargo.toml', 'saas-platform-backend'], ['stores', 'business-apps/stores/backend/Cargo.toml', 'stores-api'],
      ['work-orders', 'business-apps/work-orders/backend/Cargo.toml', 'work-orders-api'], ['admin', 'tools/platform-admin/Cargo.toml', 'saas-platform-admin']]) {
      console.log('BUSINESS_DELIVERY build ' + id)
      await exec(builder, ['cargo', 'build', '--locked', '--manifest-path', manifest, '--bin', binary, '--target-dir', '/owned-cache/target', '-j', '2'], { timeout: 40 * 60_000 })
      await cpFrom(builder, '/owned-cache/target/debug/' + binary, path.join(privateRoot, binary)); chmodSync(path.join(privateRoot, binary), 0o755)
      const destination = path.join(outDir, 'artifacts/original-binaries', id); copyFileSync(path.join(privateRoot, binary), destination)
      const bytes = readFileSync(destination); report.originalBinaries.push({ name: id, path: 'artifacts/original-binaries/' + id, bytes: bytes.length, sha256: sha(bytes), elfMachine: bytes.readUInt16LE(18) })
    }
    await exec(builder, ['npm', '--prefix', 'frontend', 'run', 'build'], { timeout: 8 * 60_000 })
    await cpFrom(builder, '/workspace/frontend/dist', path.join(privateRoot, 'host-dist'))
    const nginx = await inspect('image', 'nginx:1.28.0-alpine'), node = await inspect('image', 'node:24.18.0-alpine'), runtimeBase = await inspect('image', 'debian:bookworm-slim')
    const prebuiltBinaries = Object.fromEntries(appCodes.map(code => [code, { path: path.join(privateRoot, code + '-api'), sha256: sha(readFileSync(path.join(privateRoot, code + '-api'))),
      sourceRevision: source.sourceRevision, inputsSha256: source.inputsSha256, targetPlatform: 'linux/amd64' }]))
    console.log('BUSINESS_DELIVERY six independent image builds')
    const candidate = await buildBusinessImages({ docker, owner: owner.replace(/^ci-/, ''), sourceRoot: snapshot, outDir: path.join(outDir, 'candidate'), sourceRevision: source.sourceRevision, inputsSha256: source.inputsSha256,
      hostOrigin: origin, versions: ['1.0.0', '2.0.0'], versionApiBases: Object.fromEntries(appCodes.map(code => [code, { '1.0.0': '/' + code + '-api', '2.0.0': origin + '/' + code + '-api' }])),
      baseImages: { node: node.Id, static: nginx.Id, runtime: runtimeBase.Id }, prebuiltBinaries, buildProxy: options.buildProxy })
    report.candidate = { file: 'candidate/candidate-result.json', sha256: sha(readFileSync(path.join(outDir, 'candidate/candidate-result.json'))) }
    passed('independent-static-image-builds', { staticCount: 4, rustApiCount: 2, rustBuild: 'locked toolchain build; verified ELF packaging', imageArchiveCount: candidate.images.length })
    const registryReference = process.env.BUSINESS_REGISTRY_IMAGE || 'registry:3.0.0', registryImage = await inspect('image', registryReference)
    const registryName = owner + '-registry'; await create(registryName, ['--network', buildNetwork, '--publish', '127.0.0.1::5000', '--env', 'REGISTRY_STORAGE_DELETE_ENABLED=true', registryImage.Id]); await start(registryName)
    const registryState = await inspect('container', registryName), port = registryState.NetworkSettings.Ports['5000/tcp'][0].HostPort, registryAuthority = 'localhost:' + port
    report.registry = { ...report.registry, requestedReference: registryReference, registryImage: registryImage.Id, registryRepoDigests: registryImage.RepoDigests || [], authority: registryAuthority, transport: 'http-loopback-owned', baseLayersMayRemainCached: true }
    await eventually(async () => { const output = await docker(['logs', registryName]); assert(/listening on/.test(output)); return true }, 'Registry did not start')
    const publication = await publishBusinessImages({ docker, candidate, candidateDir: path.join(outDir, 'candidate'), registry: registryAuthority, repositoryPrefix: owner.replace(/^ci-/, ''), scope: 'isolated', insecureLoopback: true, outDir: path.join(outDir, 'publication') })
    report.registry.push = 'PASS'; save()
    await forgetOwnedBusinessImages({ docker, candidate, publication })
    const pull = await fixedDigestPull({ docker, publication, outDir: path.join(outDir, 'pull'), requireCandidateAbsent: true })
    const registryLog = await docker(['logs', registryName]); report.registry.log = report.commands.at(-1).log; report.registry.logSha256 = sha(Buffer.from(clean(registryLog) + '\n')); report.registry.registryRequestsObserved = /http.request.method=GET/.test(registryLog) && /manifests\//.test(registryLog) && /blobs\//.test(registryLog)
    assert(report.registry.registryRequestsObserved, 'Registry must observe actual manifest and blob GETs')
    report.registry.pull = 'PASS'; report.registry.candidateAbsentBeforePull = pull.images.every(image => image.candidateImageAbsentBeforePull)
    const targetImages = publication.images.map(image => { const target = pull.images.find(item => item.key === image.key); assert(target); return { ...image, candidateImageId: image.imageId, imageId: target.imageId, daemonImageIdKind: target.daemonImageIdKind } })
    report.images = targetImages.map(image => ({ key: image.key, imageId: image.imageId, candidateImageId: image.candidateImageId, reference: image.imageRef, digest: image.digest, ociConfigDigest: image.ociConfigDigest, daemonImageIdKind: image.daemonImageIdKind, sourceRevision: image.sourceRevision, inputsSha256: image.inputsSha256, buildId: image.buildId }))
    passed('isolated-registry-push-pull', { proof: 'same daemon; owned candidate image IDs and references absent before pull', count: report.images.length })
    const imageBy = Object.fromEntries(targetImages.map(image => [image.key, image]))
    const hostContext = path.join(privateRoot, 'host-image'); mkdirSync(hostContext); await cpFrom(builder, '/workspace/frontend/dist', path.join(hostContext, 'dist'))
    copyFileSync(path.join(snapshot, 'deploy/static/nginx.conf'), path.join(hostContext, 'nginx.conf'))
    writeFileSync(path.join(hostContext, 'Dockerfile'), `FROM ${nginx.Id}\nCOPY --chown=101:101 dist /usr/share/nginx/html\nCOPY nginx.conf /etc/nginx/nginx.conf\nUSER 101:101\nHEALTHCHECK --interval=2s --timeout=2s --retries=10 CMD wget -q -O /dev/null http://127.0.0.1:8080/healthz || exit 1\nENTRYPOINT ["nginx"]\nCMD ["-c","/etc/nginx/nginx.conf","-g","daemon off;"]\n`)
    const hostImage = await dockerBuild(owner + '-host:candidate', hostContext)
    const platformContext = path.join(privateRoot, 'platform-image'); mkdirSync(platformContext); copyFileSync(path.join(privateRoot, 'saas-platform-backend'), path.join(platformContext, 'platform'))
    writeFileSync(path.join(platformContext, 'Dockerfile'), `FROM ${imageBy['stores-api'].imageId}\nCOPY --chown=10001:10001 platform /usr/local/bin/platform\nENV API_HEALTH_URL=http://127.0.0.1:3001/api/health\nENTRYPOINT ["/usr/local/bin/platform"]\n`)
    const platformImage = await dockerBuild(owner + '-platform:candidate', platformContext)
    // TLS material stays in owned private work and the disposable runtime.
    await exec(builder, ['mkdir', '-p', '/runtime/tls'])
    await exec(builder, ['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-sha256', '-days', '1', '-subj', '/CN=Phase28-Owned-Test-CA', '-keyout', '/runtime/tls/ca.key', '-out', '/runtime/tls/ca.pem', '-addext', 'basicConstraints=critical,CA:TRUE', '-addext', 'keyUsage=critical,keyCertSign,cRLSign'])
    await exec(builder, ['openssl', 'req', '-new', '-newkey', 'rsa:2048', '-nodes', '-sha256', '-subj', '/CN=host.phase28.test', '-keyout', '/runtime/tls/server.key', '-out', '/runtime/tls/server.csr'])
    const extensions = path.join(privateRoot, 'server.ext'); writeFileSync(extensions, 'basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:host.phase28.test,DNS:external.phase28.test\n'); await cpTo(extensions, builder, '/runtime/tls/server.ext')
    await exec(builder, ['openssl', 'x509', '-req', '-in', '/runtime/tls/server.csr', '-CA', '/runtime/tls/ca.pem', '-CAkey', '/runtime/tls/ca.key', '-CAcreateserial', '-out', '/runtime/tls/server.pem', '-days', '1', '-sha256', '-extfile', '/runtime/tls/server.ext'])
    await exec(builder, ['openssl', 'verify', '-CAfile', '/runtime/tls/ca.pem', '-verify_hostname', 'host.phase28.test', '/runtime/tls/server.pem'])
    for (const name of ['ca.pem', 'server.pem', 'server.key']) await cpFrom(builder, '/runtime/tls/' + name, path.join(privateRoot, name))
    const manifests = {}
    for (const image of targetImages.filter(image => image.role === 'static')) {
      const probe = owner + '-manifest-' + image.key; await create(probe, ['--entrypoint', '/bin/true', image.imageRef])
      manifests[image.key] = {}
      for (const [file, key] of [['asset-manifest.json', 'manifest'], ['build-info.json', 'buildInfo']]) {
        const destination = path.join(privateRoot, image.key + '-' + file); await cpFrom(probe, '/usr/share/nginx/html/' + file, destination); manifests[image.key][key] = JSON.parse(readFileSync(destination))
      }
      const artifactDirectory = path.join(outDir, 'artifacts/static', image.key); mkdirSync(artifactDirectory); await cpFrom(probe, '/usr/share/nginx/html/.', artifactDirectory)
      report.staticArtifacts.push({ key: image.key, path: 'artifacts/static/' + image.key, buildInfoSha256: sha(readFileSync(path.join(artifactDirectory, 'build-info.json'))), manifestSha256: sha(readFileSync(path.join(artifactDirectory, 'asset-manifest.json'))), imageId: image.imageId, imageRef: image.imageRef, files: staticTree(artifactDirectory) })
    }
    const releases = {}
    for (const code of appCodes) {
      releases[code] = {}
      for (const [key, version, publicOrigin] of [['v1', '1.0.0', origin], ['v2', '2.0.0', externalOrigin], ['badEntry', 'fail-entry', origin], ['badResource', 'fail-resource', origin], ['badHealth', 'fail-health', origin]]) {
        const image = imageBy[code + (key === 'v2' ? '-v2' : '-v1')], metadata = manifests[image.key], prefix = publicOrigin + '/sub-apps/' + code + '/versions/' + version + '/'
        const resourceFiles = new Set([metadata.buildInfo.entry, ...(metadata.buildInfo.dynamicChunks || []), 'assets/config.js', ...(code === 'work-orders' ? ['assets/iframe-bridge.js'] : [])])
        for (const row of Object.values(metadata.manifest)) { if (row.file) resourceFiles.add(row.file); for (const file of row.css || []) resourceFiles.add(file) }
        const resources = [...resourceFiles].filter(Boolean).sort().map(file => ({ url: new URL(file, prefix).href, kind: /\.css$/.test(file) ? 'STYLE' : 'SCRIPT' }))
        const release = { environment: 'production', version, entryType: 'HTML', entryUrl: prefix + (key === 'badEntry' ? 'missing.html' : 'index.html'), healthUrl: prefix + (key === 'badHealth' ? 'missing-health' : 'healthz'), resources: key === 'badResource' ? [{ url: prefix + 'missing-chunk.js', kind: 'SCRIPT' }] : resources }
        releases[code][key] = { release, artifact: { schemaVersion: 1, applicationCode: code, publicOrigin, source: { kind: 'OCI_IMAGE', identity: image.imageRef, targetPlatform: 'linux/amd64' }, digest: image.digest,
          commit: report.sourceRevision, buildId: image.buildId, inputsSha256: report.inputsSha256 }, ...metadata }
      }
      releases[code].external = releases[code].v2
    }
    const unhealthy = owner + '-unhealthy', unhealthyComposeFile = path.join(privateRoot, 'unhealthy-compose.json'); containers.add(unhealthy)
    json(unhealthyComposeFile, { services: { probe: { container_name: unhealthy, image: imageBy['stores-v1'].imageRef, labels: { [label]: owner }, read_only: true,
      cap_drop: ['ALL'], security_opt: ['no-new-privileges:true'], tmpfs: ['/tmp:rw,nosuid,noexec,size=32m'], healthcheck: { test: ['CMD', '/bin/false'], interval: '1s', timeout: '1s', retries: 1, start_period: '0s' }, networks: ['owned'] } }, networks: { owned: { external: true, name: buildNetwork } } })
    const unhealthyComposeOutput = await docker(['compose', '--project-name', owner + '-unhealthy', '--file', linux(unhealthyComposeFile), 'up', '--detach', '--wait', '--wait-timeout', '20', '--pull', 'never'], { allowFailure: true, timeout: 30_000 })
    const unhealthyComposeLog = report.commands.at(-1).log; assert.match(unhealthyComposeOutput, /unhealthy/i, 'Actual Compose wait must refuse its unhealthy service')
    await eventually(async () => assert.equal((await inspect('container', unhealthy)).State.Health.Status, 'unhealthy'), 'Expected unhealthy test container')
    const calls = { routes: 0, register: 0, activate: 0 }
    const routeGuard = path.join(privateRoot, 'route-publication-state.json'), gatewayGuard = path.join(privateRoot, 'gateway-publication-state.conf')
    json(routeGuard, { publicVersionRoutes: [] }); writeFileSync(gatewayGuard, renderGateway({ publicOrigin: origin, versionRoutes: [] }))
    const guardBefore = sha(readFileSync(routeGuard)), gatewayBefore = sha(readFileSync(gatewayGuard))
    let blocked = false
    try { await promoteBusinessRoutes({ docker, services: [{ container: unhealthy, imageId: imageBy['stores-v1'].imageId, imageRef: imageBy['stores-v1'].imageRef }], bindArtifacts: async () => {},
      stageRoutes: async () => { calls.routes++; return {} }, validateRoutes: async () => {}, commitRoutes: async () => {}, registerReleases: async () => { calls.register++ }, preflightReleases: async () => [], activateReleases: async () => { calls.activate++ } }) }
    catch (error) { blocked = /BUSINESS_CONTAINER_UNHEALTHY/.test(error.message) }
    assert(blocked); assert.deepEqual(calls, { routes: 0, register: 0, activate: 0 }); report.unhealthy = { status: 'BLOCKED', calls, layer: 'CI_DOCKER_HEALTH', composeWaitRejected: true, composeLog: unhealthyComposeLog, routeStateBefore: guardBefore, routeStateAfter: sha(readFileSync(routeGuard)), gatewayConfigBefore: gatewayBefore, gatewayConfigAfter: sha(readFileSync(gatewayGuard)), runtimePlatformsCreated: 0 }; passed('unhealthy-blocks-promotion', report.unhealthy)
    const bindings = []
    await cpTo(path.join(privateRoot, 'saas-platform-admin'), builder, '/runtime/saas-platform-admin'); await exec(builder, ['chmod', '755', '/runtime/saas-platform-admin'])
    for (const code of appCodes) for (const key of ['v1', 'v2']) {
      const value = releases[code][key]
      await writeRuntime(builder, value.release, code + '-' + key + '-release.json'); await writeRuntime(builder, value.artifact, code + '-' + key + '-artifact.json')
      bindings.push({ appCode: code, environment: 'production', releaseFile: '/runtime/' + code + '-' + key + '-release.json', artifactFile: '/runtime/' + code + '-' + key + '-artifact.json' })
    }
    for (const product of businessBrowserProducts) {
      console.log('BUSINESS_DELIVERY real browser ' + product)
      const runtimeOwner = owner + '-' + product, network = await makeNetwork(runtimeOwner + '-network')
      const postgres = runtimeOwner + '-postgres', password = randomBytes(24).toString('hex'); secretValues.add(password)
      const postgresImage = await inspect('image', 'postgres:17.6-bookworm')
      await create(postgres, ['--network', network, '--tmpfs', '/var/lib/postgresql/data:rw,nosuid,size=2g', '--shm-size', '128m', '--env', 'POSTGRES_USER=ci', '--env', 'POSTGRES_PASSWORD=' + password,
        '--env', 'POSTGRES_DB=postgres', postgresImage.Id, '-c', 'max_connections=70']); await start(postgres)
      await eventually(async () => assert.match(await exec(postgres, ['pg_isready', '-U', 'ci', '-d', 'postgres']), /accepting connections/), 'Owned PostgreSQL readiness')
      const databaseUrls = {}
      for (const [index, code] of ['platform', 'stores', 'work-orders'].entries()) {
        const name = 'p28_' + index, role = 'p28_role_' + index, rolePassword = randomBytes(24).toString('hex'); secretValues.add(rolePassword)
        await exec(postgres, ['psql', '-U', 'ci', '-d', 'postgres', '-v', 'ON_ERROR_STOP=1', '-c', `CREATE ROLE ${role} LOGIN PASSWORD '${rolePassword}' NOSUPERUSER NOCREATEDB NOCREATEROLE;`])
        await exec(postgres, ['createdb', '-U', 'ci', '-O', role, name]); databaseUrls[code] = `postgresql://${role}:${rolePassword}@${postgres}:5432/${name}`; secretValues.add(databaseUrls[code])
      }
      const adminPassword = 'Phase28-Owned-' + randomBytes(16).toString('hex'), jwt = randomBytes(48).toString('hex'); secretValues.add(adminPassword); secretValues.add(jwt)
      const clients = Object.fromEntries(appCodes.map(code => [code, { clientId: 'p28-' + randomBytes(12).toString('hex'), clientSecret: randomBytes(32).toString('hex') }]))
      for (const value of Object.values(clients)) secretValues.add(value.clientSecret)
      // Functional cold-bootstrap profile; default request budgets are measured separately.
      const platform = runtimeOwner + '-platform', platformEnv = { DATABASE_URL: databaseUrls.platform, JWT_SECRET: jwt, METRICS_TOKEN: randomBytes(40).toString('hex'), DB_STATEMENT_TIMEOUT_MS: '60000', BIND_ADDR: '0.0.0.0:3001', PUBLIC_APP_URL: origin,
        MICRO_GATEWAY_ORIGIN: origin, MICRO_ALLOWED_ORIGINS: origin + ',' + externalOrigin, CORS_ALLOWED_ORIGINS: origin, APP_ENV: 'production', APPLICATION_ENVIRONMENT: 'production', AUTO_CREATE_DATABASE: 'false',
        BOOTSTRAP_ADMIN_USERNAME: 'phase28-admin', BOOTSTRAP_ADMIN_PASSWORD: adminPassword, STATIC_DIR: '', MICRO_APPS_DIR: '', MICRO_CA_CERT_FILE: '/run/phase28-ca.pem', RUST_LOG: 'warn',
        OAUTH_INTROSPECTION_CLIENT_REQUESTS_PER_MINUTE: '12000', OAUTH_INTROSPECTION_ATTEMPTS_PER_MINUTE: '24000', DATA_GRANT_POLICY: JSON.stringify({ applications: [], managementScopes: [] }) }
      secretValues.add(platformEnv.METRICS_TOKEN)
      const launchPlatform = async () => {
        await create(platform, ['--network', network, ...hardening, '--mount', 'type=bind,source=' + linux(path.join(privateRoot, 'ca.pem')) + ',target=/run/phase28-ca.pem,readonly', ...Object.entries(platformEnv).flatMap(([key, value]) => ['--env', key + '=' + value]), platformImage])
        await start(platform)
        try { await checkBusinessHealth({ docker, services: [{ container: platform, imageId: platformImage }], timeoutMs: 180_000 }) }
        catch (error) { await docker(['logs', platform], { allowFailure: true }); throw error }
      }
      await launchPlatform()
      const composeDirectory = path.join(privateRoot, product + '-compose'); mkdirSync(composeDirectory)
      const composeFile = path.join(composeDirectory, 'compose.json'), services = {}
      for (const image of targetImages) {
        const name = runtimeOwner + '-' + image.key
        services[image.key] = { image: image.imageRef, container_name: name, read_only: true, cap_drop: ['ALL'], security_opt: ['no-new-privileges:true'], tmpfs: ['/tmp:rw,nosuid,noexec,size=64m'], networks: { owned: { aliases: [image.key] } }, labels: { [label]: owner } }
        containers.add(name)
        if (image.role === 'api') {
          const code = image.appCode, upper = code.replaceAll('-', '_').toUpperCase()
          services[image.key].environment = { [upper + '_DATABASE_URL']: databaseUrls[code], [upper + '_HOST']: '0.0.0.0', [upper + '_ALLOWED_ORIGINS']: 'null', PLATFORM_API_ORIGIN: origin,
            PLATFORM_CLIENT_ID: clients[code].clientId, PLATFORM_CLIENT_SECRET: clients[code].clientSecret, PLATFORM_CA_CERT_FILE: '/run/phase28-ca.pem', NODE_ENV: 'production', RUST_LOG: 'warn' }
          services[image.key].volumes = [{ type: 'bind', source: linux(path.join(privateRoot, 'ca.pem')), target: '/run/phase28-ca.pem', read_only: true }]
        }
      }
      json(composeFile, { services, networks: { owned: { external: true, name: network } } })
      const compose = ['compose', '--project-name', runtimeOwner, '--file', linux(composeFile), '--project-directory', linux(composeDirectory)]
      const composePullCommandIndex = report.commands.length
      await docker([...compose, 'pull']); await docker([...compose, 'up', '--detach', '--wait', '--wait-timeout', '180', '--pull', 'never', ...targetImages.filter(image => image.role === 'static').map(image => image.key)], { timeout: 4 * 60_000 })
      const staticHealth = await checkBusinessHealth({ docker, services: targetImages.filter(image => image.role === 'static').map(image => ({ service: image.key, container: runtimeOwner + '-' + image.key, imageId: image.imageId, imageRef: image.imageRef })) })
      const runtimeProof = { product, owner: runtimeOwner, network, postgresImage: postgresImage.Id, composePull: { commandIndex: composePullCommandIndex, fixedImages: Object.fromEntries(targetImages.map(image => [image.key, image.imageRef])) }, staticHealth, gatewayConfigs: [], managementGatewayConfigs: [], artifactHistoryVerifiedAfterToolRecreation: false }
      report.runtimeProofs.push(runtimeProof)
      const host = runtimeOwner + '-host'; await create(host, ['--network', network, ...hardening, hostImage]); await start(host); await checkBusinessHealth({ docker, services: [{ container: host, imageId: hostImage }] })
      const versions = ['1.0.0', '2.0.0', 'fail-entry', 'fail-resource', 'fail-health']
      const versionRoutes = appCodes.flatMap(code => versions.map(version => ({ app: code, version, upstream: runtimeOwner + '-' + code + (version === '2.0.0' ? '-v2' : '-v1') + ':8080' })))
      const runtimeGateways = []
      const gatewaySource = (publicOrigin, routes) => renderGateway({ publicOrigin, backend: platform + ':3001', frontend: host + ':8080', stores: runtimeOwner + '-stores-api:3031', workOrders: runtimeOwner + '-work-orders-api:3004',
        certificate: '/etc/nginx/tls/server.pem', certificateKey: '/etc/nginx/tls/server.key', versionRoutes: routes, allowedServices: [platform, host, ...targetImages.map(image => runtimeOwner + '-' + image.key)], applicationOrigins: [origin, externalOrigin] })
      for (const [suffix, publicOrigin, alias] of [['edge', origin, 'host.phase28.test'], ['external-edge', externalOrigin, 'external.phase28.test']]) {
        const edge = runtimeOwner + '-' + suffix, configDirectory = path.join(privateRoot, edge + '-configuration'); mkdirSync(configDirectory)
        const configPath = path.join(configDirectory, 'nginx.conf')
        const config = gatewaySource(publicOrigin, [])
        writeFileSync(configPath + '.pending', config); renameSync(configPath + '.pending', configPath)
        const publicConfigLog = 'logs/' + edge + '-management-nginx.conf.log'; writeFileSync(path.join(outDir, publicConfigLog), config)
        runtimeProof.managementGatewayConfigs.push({ origin: publicOrigin, path: publicConfigLog, sha256: sha(Buffer.from(config)), publicVersionRouteCount: 0 })
        runtimeGateways.push({ edge, publicOrigin, configPath, configDirectory })
        const tlsDirectory = path.join(privateRoot, edge + '-tls'); mkdirSync(tlsDirectory); for (const file of ['server.pem', 'server.key']) { copyFileSync(path.join(privateRoot, file), path.join(tlsDirectory, file)); chmodSync(path.join(tlsDirectory, file), 0o644) }
        await create(edge, ['--network', network, '--network-alias', alias, '--user', '101:101', ...hardening,
          '--mount', 'type=bind,source=' + linux(configDirectory) + ',target=/run/phase28-nginx,readonly', '--mount', 'type=bind,source=' + linux(tlsDirectory) + ',target=/etc/nginx/tls,readonly',
          '--entrypoint', '/bin/sh', nginx.Id, '-c', 'nginx -t -c /run/phase28-nginx/nginx.conf && exec nginx -c /run/phase28-nginx/nginx.conf'])
        await start(edge)
      }
      const browserTool = await tool(runtimeOwner + '-tools', network, ['--env', 'SAAS_TEST_BROWSER=' + product, '--env', 'PHASE28_RUNTIME_CONFIG=/runtime/config.json', '--env', 'NODE_EXTRA_CA_CERTS=/runtime/ca.pem', '--env', 'SSL_CERT_FILE=/runtime/ca.pem',
        '--env', 'HOME=/runtime/home', '--env', 'XDG_DATA_HOME=/runtime/xdg-data', '--env', 'XDG_CONFIG_HOME=/runtime/xdg-config', '--env', 'XDG_CACHE_HOME=/runtime/xdg-cache'])
      await cpTo(path.join(privateRoot, 'ca.pem'), browserTool, '/runtime/ca.pem'); await cpTo(path.join(privateRoot, 'saas-platform-admin'), browserTool, '/runtime/saas-platform-admin'); await exec(browserTool, ['chmod', '755', '/runtime/saas-platform-admin'])
      for (const directory of ['tests/browser', 'mini-services/notification-ws']) await exec(browserTool, ['npm', '--prefix', directory, 'ci', '--no-audit', '--no-fund'], { timeout: 8 * 60_000 })
      for (const database of ['/runtime/home/.pki/nssdb', '/runtime/xdg-data/pki/nssdb']) {
        await exec(browserTool, ['mkdir', '-p', database]); await exec(browserTool, ['certutil', '-N', '--empty-password', '-d', 'sql:' + database]); await exec(browserTool, ['certutil', '-A', '-d', 'sql:' + database, '-n', 'phase28-owned-ca', '-t', 'C,,', '-i', '/runtime/ca.pem'])
      }
      for (const binding of bindings) {
        const code = binding.appCode, key = binding.releaseFile.includes('-v1-') ? 'v1' : 'v2'
        await writeRuntime(browserTool, releases[code][key].release, code + '-' + key + '-release.json'); await writeRuntime(browserTool, releases[code][key].artifact, code + '-' + key + '-artifact.json')
        if (report.browserProducts.length) {
          const verify = JSON.parse(await exec(browserTool, ['/runtime/saas-platform-admin', 'artifact-verify', '--app-code', code, '--environment', 'production', '--release-file', binding.releaseFile, '--artifact-file', binding.artifactFile, '--inventory-dir', '/runtime/artifact-inventory']))
          assert.equal(verify.bindingState, 'BOUND')
        }
      }
      runtimeProof.artifactHistoryVerifiedAfterToolRecreation = report.browserProducts.length > 0
      const config = { schemaVersion: 1, owner: runtimeOwner, sourceRevision: report.sourceRevision, inputsSha256: report.inputsSha256, origin, externalOrigin,
        caFile: '/runtime/ca.pem', adminBinary: '/runtime/saas-platform-admin', inventoryDir: '/runtime/artifact-inventory', stateFile: '/runtime/state.json', policyFile: '/runtime/policy.json', reportDir: '/reports/business-browser', screenshotsDir: '/reports/business-browser/screenshots',
        admin: { username: 'phase28-admin', password: adminPassword }, databaseUrls, clients, releases, deliveryProof: { registryPushPull: true, fixedDigestCompose: true, unhealthyBlocked: calls, sourceRevision: report.sourceRevision, inputsSha256: report.inputsSha256 },
        faultSignals: { request: '/runtime/recheck-request.json', ready: '/runtime/recheck-ready.json', restore: '/runtime/recheck-restore.json', restored: '/runtime/recheck-restored.json' } }
      await writeRuntime(browserTool, config)
      await exec(browserTool, ['node', 'tests/phase28-delivery-browser.mjs', '--prepare'], { timeout: 5 * 60_000 })
      const policy = await privateRead(browserTool, '/runtime/policy.json'); platformEnv.DATA_GRANT_POLICY = JSON.stringify(policy)
      await docker(['rm', '--force', platform]); await launchPlatform()
      await docker([...compose, 'up', '--detach', '--wait', '--wait-timeout', '180', '--pull', 'never', ...targetImages.filter(image => image.role === 'api').map(image => image.key)], { timeout: 4 * 60_000 })
      runtimeProof.apiHealth = await checkBusinessHealth({ docker, services: targetImages.filter(image => image.role === 'api').map(image => ({ container: runtimeOwner + '-' + image.key, imageId: image.imageId, imageRef: image.imageRef })) })
      runtimeProof.apiBinaryHashes = []
      for (const image of targetImages.filter(image => image.role === 'api')) {
        const output = await exec(runtimeOwner + '-' + image.key, ['sha256sum', '/usr/local/bin/business-api'])
        const actual = /^([a-f0-9]{64})\s+\/usr\/local\/bin\/business-api\s*$/.exec(output)?.[1]
        assert.equal(actual, image.prebuilt.sha256, 'Actual running Rust API binary differs from the verified candidate ELF')
        runtimeProof.apiBinaryHashes.push({ key: image.key, sha256: actual, log: report.commands.at(-1).log })
      }
      const requiredServices = targetImages.map(image => ({ container: runtimeOwner + '-' + image.key, imageId: image.imageId, imageRef: image.imageRef }))
      runtimeProof.requiredHealthBeforeBinding = await checkBusinessHealth({ docker, services: requiredServices }); runtimeProof.requiredHealthCompletedAt = new Date().toISOString()
      runtimeProof.artifactBindings = await bindBusinessArtifacts({ binary: '/runtime/saas-platform-admin', bindings, inventoryDir: '/runtime/artifact-inventory', execute: async (binary, args) => exec(browserTool, [binary, ...args]) })
      Object.assign(runtimeProof.artifactBindings, { completedAt: new Date().toISOString(), afterHealthCompletedAt: runtimeProof.requiredHealthCompletedAt, beforePublicVersionRoutes: true })
      report.artifactBindings ||= runtimeProof.artifactBindings
      if (!report.artifactConflict) {
        const before = await exec(postgres, ['psql', '-U', 'ci', '-d', 'p28_0', '-tAc', 'SELECT COUNT(*) FROM application_releases'])
        assert.equal(before, '0')
        const actualGatewayBefore = runtimeGateways.map(gateway => ({ origin: gateway.publicOrigin, sha256: sha(readFileSync(gateway.configPath)) }))
        const conflicting = structuredClone(releases.stores.v1.artifact); conflicting.digest = 'sha256:' + 'e'.repeat(64); conflicting.source.identity = conflicting.source.identity.replace(/sha256:[a-f0-9]{64}$/, conflicting.digest)
        await writeRuntime(browserTool, conflicting, 'conflicting-artifact.json')
        const refused = await exec(browserTool, ['/runtime/saas-platform-admin', 'artifact-bind', '--app-code', 'stores', '--environment', 'production', '--release-file', '/runtime/stores-v1-release.json', '--artifact-file', '/runtime/conflicting-artifact.json', '--inventory-dir', '/runtime/artifact-inventory', '--apply'], { allowFailure: true })
        assert.match(refused, /ARTIFACT_CONFLICT/); assert.equal(sha(readFileSync(routeGuard)), guardBefore)
        const after = await exec(postgres, ['psql', '-U', 'ci', '-d', 'p28_0', '-tAc', 'SELECT COUNT(*) FROM application_releases'])
        assert.equal(after, before)
        const actualGatewayAfter = runtimeGateways.map(gateway => ({ origin: gateway.publicOrigin, sha256: sha(readFileSync(gateway.configPath)) })); assert.deepEqual(actualGatewayAfter, actualGatewayBefore)
        report.artifactConflict = { category: 'ARTIFACT_CONFLICT', actualCLI: true, routeStateBefore: guardBefore, routeStateAfter: sha(readFileSync(routeGuard)), gatewayConfigsBefore: actualGatewayBefore, gatewayConfigsAfter: actualGatewayAfter, registeredReleasesBefore: Number(before), registeredReleasesAfter: Number(after), activatedReleases: 0, reason: 'actual offline rejection preserves management-only gateway files and actual platform release row count before version publication' }
      }
      await checkBusinessHealth({ docker, services: requiredServices })
      for (const gateway of runtimeGateways) {
        const config = gatewaySource(gateway.publicOrigin, versionRoutes)
        const pendingPath = path.join(gateway.configDirectory, 'candidate.conf'); writeFileSync(pendingPath, config)
        const tested = await exec(gateway.edge, ['nginx', '-t', '-c', '/run/phase28-nginx/candidate.conf']); assert.match(tested, /syntax is ok/); assert.match(tested, /test is successful/)
        const configTestLog = report.commands.at(-1).log
        renameSync(pendingPath, gateway.configPath)
        const publicConfigLog = 'logs/' + gateway.edge + '-nginx.conf.log'; writeFileSync(path.join(outDir, publicConfigLog), config)
        const proof = { origin: gateway.publicOrigin, path: publicConfigLog, sha256: sha(Buffer.from(config)), stagedAt: new Date().toISOString(), afterArtifactBindingsCompletedAt: runtimeProof.artifactBindings.completedAt }
        await exec(gateway.edge, ['nginx', '-s', 'reload', '-c', '/run/phase28-nginx/nginx.conf'])
        Object.assign(proof, { publishedAt: new Date().toISOString(), configTestLog, reloadLog: report.commands.at(-1).log, publicVersionRouteCount: versionRoutes.length }); runtimeProof.gatewayConfigs.push(proof)
      }
      let browserFinished = false, signalError
      const signals = (async () => {
        let stopped = false, restored = false
        while (!browserFinished) {
          try {
            const values = await privateRead(browserTool, '/runtime/' + (stopped ? 'recheck-restore.json' : 'recheck-request.json'))
            assert.deepEqual(values, { appCode: 'stores', action: stopped ? 'start' : 'stop' })
            const target = runtimeOwner + '-stores-v2'; assert(await owned('container', target))
            if (!stopped) { await docker(['stop', '--time', '5', target]); await writeRuntime(browserTool, { stopped: true }, 'recheck-ready.json'); stopped = true }
            else if (!restored) { await start(target); await checkBusinessHealth({ docker, services: [{ container: target, imageId: imageBy['stores-v2'].imageId, imageRef: imageBy['stores-v2'].imageRef }] }); await writeRuntime(browserTool, { healthy: true, restored: true }, 'recheck-restored.json'); restored = true; return }
          } catch (error) { if (!/ENOENT|no such file/i.test(error.message)) { signalError = error; return } }
          await pause(100)
        }
      })()
      let output, browserError, browserArtifactError
      try { output = await exec(browserTool, ['node', 'tests/phase28-delivery-browser.mjs', '--browser'], { timeout: 20 * 60_000 }) }
      catch (error) { browserError = error }
      finally {
        browserFinished = true; await signals
        // Keep the fixture's original JSON and PNG bytes even when its process
        // exits nonzero. Preserve the execution error if evidence copying also
        // fails; both actual command failures remain in the append-only log.
        const publicPath = 'browser-' + product, browserPublic = path.join(outDir, publicPath)
        report.browserArtifactCopies ||= []
        try {
          mkdirSync(browserPublic); await cpFrom(browserTool, '/reports/business-browser/.', browserPublic)
          report.browserArtifactCopies.push({ product, path: publicPath, status: 'PASS' })
        } catch (error) {
          browserArtifactError = error
          report.browserArtifactCopies.push({ product, path: publicPath, status: 'FAIL', error: clean(error.message) })
        }
        save()
      }
      if (browserError) throw browserError
      if (browserArtifactError) throw browserArtifactError
      if (signalError) throw signalError
      const result = parseBusinessBrowserOutput(output, { product, sourceRevision: report.sourceRevision, inputsSha256: report.inputsSha256, sourceSha256: source.files.find(file => file.path === 'tests/phase28-delivery-browser.mjs').sha256 })
      const browserResult = { ...result, rawLog: report.commands.findLast(command => command.args[0] === 'exec' && command.args[1] === browserTool && command.args.includes('--browser')).log }
      report.browserProducts.push(browserResult); save()
      report.screenshots ||= []
      for (const item of result.screenshots) {
        assert.equal(path.posix.dirname(item.path), '/reports/business-browser/screenshots'); assert(/^[A-Za-z0-9._-]+\.png$/.test(path.posix.basename(item.path)))
        const publicPath = 'screenshots/' + product + '-' + path.posix.basename(item.path), destination = path.join(outDir, publicPath)
        await cpFrom(browserTool, item.path, destination); const bytes = readFileSync(destination)
        assert.equal(bytes.subarray(0, 8).toString('hex'), '89504e470d0a1a0a'); assert.equal(sha(bytes), item.sha256); if (item.bytes !== undefined) assert.equal(bytes.length, item.bytes)
        report.screenshots.push({ product, path: publicPath, bytes: bytes.length, sha256: sha(bytes), originalPath: item.path })
      }
      await closeTool(browserTool)
      for (const name of [...containers].filter(name => name.startsWith(runtimeOwner + '-'))) { if (await owned('container', name)) await docker(['rm', '--force', name]); containers.delete(name) }
      if (await owned('network', network)) await docker(['network', 'rm', network]); networks.delete(network)
    }
    passed('fixed-digest-compose-start', { products: businessBrowserProducts, imagesPerProduct: 6, publicStaticRoutes: 4 })
    passed('normal-tls', { origins: [origin, externalOrigin], trust: 'owned CA; normal certificate and hostname verification; no ignore flags' })
    for (const id of businessBrowserChecks) { assert(report.browserProducts.every(product => product.checks.some(check => check.id === id && check.status === 'PASS'))); passed(id, { products: businessBrowserProducts }) }
    const sourceAfter = createSourceManifest(snapshot, { sourceRevision: report.sourceRevision }), hostAfter = createSourceManifest(hostSourceRoot, { sourceRevision: report.sourceRevision })
    report.sourceAfter = { sourceRevision: sourceAfter.sourceRevision, inputsSha256: sourceAfter.inputsSha256, fileCount: sourceAfter.files.length }
    report.hostSourceAfter = { sourceRevision: hostAfter.sourceRevision, inputsSha256: hostAfter.inputsSha256, fileCount: hostAfter.files.length }
    report.sourceUnchanged = sourceAfter.inputsSha256 === report.inputsSha256 && hostAfter.inputsSha256 === report.hostSourceBefore.inputsSha256
    assert(report.sourceUnchanged, 'Source changed during actual delivery run')
    report.status = 'PASS'
  } catch (error) { report.error = clean(error.message); report.status = 'FAIL'; console.error(report.error); process.exitCode = 1 }
  finally {
    cleanupMode = true
    for (const name of [...toolAttachments.keys()]) try { await closeTool(name) } catch (error) { report.cleanup.errors.push(clean(error.message)) }
    if (inventoryVolume && builder && containers.has(builder)) try { const inventoryOut = path.join(outDir, 'artifacts/artifact-inventory'); mkdirSync(inventoryOut); await cpFrom(builder, '/runtime/artifact-inventory/.', inventoryOut) } catch (error) { report.cleanup.errors.push(clean(error.message)) }
    for (const name of [...containers].reverse()) try { if (await owned('container', name)) await docker(['rm', '--force', name]) } catch (error) { report.cleanup.errors.push(clean(error.message)) }
    for (const name of [...networks].reverse()) try { if (await owned('network', name)) await docker(['network', 'rm', name]) } catch (error) { report.cleanup.errors.push(clean(error.message)) }
    for (const name of ephemeralVolumes) try { if (await owned('volume', name)) await docker(['volume', 'rm', name]) } catch (error) { report.cleanup.errors.push(clean(error.message)) }
    try {
      assert.equal(realpathSync(path.join(outDir, 'private')), privateRoot); assert.equal(path.dirname(privateRoot), realpathSync(outDir)); assert(!lstatSync(privateRoot).isSymbolicLink())
      rmSync(privateRoot, { recursive: true, force: true }); report.cleanup.privateWorkRemoved = !existsSync(privateRoot)
    } catch (error) { report.cleanup.errors.push(clean(error.message)) }
    for (const [kind, command, field] of [
      ['container', ['ps', '--all', '--filter', 'label=' + label + '=' + owner, '--format', '{{.Names}}'], 'ownedContainersRemoved'],
      ['network', ['network', 'ls', '--filter', 'label=' + label + '=' + owner, '--format', '{{.Name}}'], 'ownedNetworksRemoved'],
      ['volume', ['volume', 'ls', '--filter', 'label=' + label + '=' + owner, '--format', '{{.Name}}'], 'ownedVolumesRemoved'],
    ]) try { report.cleanup[field] = !(await docker(command)).trim(); report.cleanup[kind + 'AuditLog'] = report.commands.at(-1).log }
    catch (error) { report.cleanup[field] = false; report.cleanup.errors.push(clean(error.message)) }
    if (!report.cleanup.ownedContainersRemoved || !report.cleanup.ownedNetworksRemoved || !report.cleanup.ownedVolumesRemoved || !report.cleanup.privateWorkRemoved || report.cleanup.errors.length) { report.status = 'FAIL'; process.exitCode = 1 }
    if (lease) { try { await lease.stop(); report.wslLease = lease.evidence } catch (error) { report.cleanup.errors.push(clean(error.message)); report.status = 'FAIL'; process.exitCode = 1 } }
    process.removeListener('SIGINT', interrupt); process.removeListener('SIGTERM', terminate)
    report.finishedAt = new Date().toISOString()
    if (report.status === 'PASS') try { validateBusinessDeliveryReport(report) } catch (error) { report.error = error.message; report.status = 'FAIL'; process.exitCode = 1 }
    save(); console.log('BUSINESS_DELIVERY ' + report.status + ' ' + path.join(outDir, 'result.json'))
  }
  return report
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { await runBusinessDelivery(parseBusinessDeliveryArgs(process.argv.slice(2))) }
  catch (error) { console.error(error.message); process.exitCode = 1 }
}
