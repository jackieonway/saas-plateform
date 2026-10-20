#!/usr/bin/env node
// Real ZIP compatibility deployment: Rust API + temporary PostgreSQL + Nginx.
// The host and gateway images are explicit immutable inputs and are never built.
import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { createHash, randomBytes } from 'node:crypto'
import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { request } from 'node:https'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { crc32, deflateRawSync } from 'node:zlib'
import { createSourceSnapshot } from '../deploy/native/build.mjs'
import { renderGateway, renderMicroStatic } from '../deploy/gateway/render.mjs'
import { createSourceManifest, sourceRoots } from '../scripts/ci/provenance.mjs'
import { commandEnvironment, redact as redactReport } from '../scripts/ci/report.mjs'
import { validateOutputLocation } from '../scripts/ci/linux-runner.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sha = bytes => createHash('sha256').update(bytes).digest('hex')
const immutable = /^sha256:[a-f0-9]{64}$/
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds))
export const requiredCheckIds = Object.freeze(['production-boundaries', 'response-recovery', 'v1-preflight-activate', 'parallel-idempotent', 'cross-environment-immutable', 'activate-rollback', 'parallel-conflict', 'interrupted-private-staging', 'private-metadata'])

export function parseArgs(args) {
  const options = {}, seen = new Set()
  const keys = { '--out-dir': 'outDir', '--frontend-image': 'frontendImage', '--edge-image': 'edgeImage', '--release-report': 'releaseReport', '--build-proxy': 'buildProxy' }
  for (let index = 0; index < args.length; index += 2) {
    const key = keys[args[index]]
    if (!key || seen.has(key) || !args[index + 1] || args[index + 1].startsWith('--')) throw new Error('Unknown, duplicate or missing ZIP acceptance argument')
    seen.add(key); options[key] = args[index + 1]
  }
  if (!path.isAbsolute(options.outDir || '')) throw new Error('Require a new absolute --out-dir')
  if (options.releaseReport) {
    if (!path.isAbsolute(options.releaseReport) || options.frontendImage || options.edgeImage) throw new Error('--release-report must be absolute and is mutually exclusive with manual image arguments')
  } else if (!immutable.test(options.frontendImage || '') || !immutable.test(options.edgeImage || '')) throw new Error('Require immutable sha256 --frontend-image / --edge-image IDs')
  if (options.buildProxy && !/^http:\/\/(?:127\.0\.0\.1|localhost):[0-9]{1,5}$/.test(options.buildProxy)) throw new Error('Build proxy must be a credential-free loopback HTTP URL')
  return options
}

export function validateReleaseCandidate(candidate, { sourceRevision, inputsSha256 } = {}) {
  const hex = /^[a-f0-9]{64}$/
  if (!candidate || candidate.status !== 'PASS' || candidate.build !== 'PASS' || candidate.smoke !== 'PASS' || candidate.cleanupError || candidate.imageExport && candidate.imageExport.cleanup !== 'PASS') throw new Error('ZIP gate requires a completely successful candidate build/smoke/cleanup')
  if (!/^[a-f0-9]{40}$/.test(candidate.commit || '') || candidate.commit !== sourceRevision || !hex.test(candidate.inputsSha256 || '') || inputsSha256 && candidate.inputsSha256 !== inputsSha256) throw new Error('Candidate source revision or current production snapshot hash does not match')
  if (candidate.targetPlatform !== 'linux/amd64' || !candidate.buildId || !Array.isArray(candidate.images) || candidate.images.length !== 3 || new Set(candidate.images.map(image => image?.name)).size !== 3) throw new Error('Candidate requires exactly frontend, edge and platform Linux image metadata')
  for (const name of ['frontend', 'edge', 'platform']) {
    const image = candidate.images.find(image => image?.name === name)
    if (!image || image.status && image.status !== 'PASS' || !immutable.test(image.imageId || '') || !immutable.test(image.ociManifestDigest || '') || !immutable.test(image.ociConfigDigest || '')
      || image.commit !== candidate.commit || image.inputsSha256 !== candidate.inputsSha256 || image.buildId !== candidate.buildId || image.targetPlatform !== candidate.targetPlatform
      || image.archive !== `images/${name}.tar` || !hex.test(image.archiveSha256 || '') || !Number.isSafeInteger(image.archiveBytes) || image.archiveBytes <= 0) throw new Error(`Missing, failed or inconsistent immutable ${name} candidate image metadata`)
  }
  const probe = candidate.images.find(image => image.name === 'platform').runtimeVerification
  if (!probe || !hex.test(probe.binarySha256 || '') || probe.uid !== 10001 || probe.elfMachine !== 62 || probe.targetPlatform !== 'linux/amd64' || probe.sharedLibrariesResolved !== true || probe.noServiceStarted !== true || probe.network !== 'none' || probe.readOnlyRootfs !== true) throw new Error('Candidate platform requires the successful actual ELF/runtime probe')
  return candidate
}

/** Keep only this WSL test session alive while Windows performs HTTPS requests.
 * EOF releases the lease on normal cleanup or when the Windows parent dies. */
export async function startWslLease({ distro = 'Ubuntu', owner, env, spawnProcess = spawn, onUnexpectedExit = () => {} }) {
  const marker = `ZIP_LEASE_READY:${owner}`
  const child = spawnProcess('wsl.exe', ['-d',distro,'--exec','python3','-I','-u','-c','import sys\nprint(sys.argv[1],flush=True)\nsys.stdin.buffer.read()',marker], { env, windowsHide: true, stdio: ['pipe','pipe','pipe'] })
  const evidence = { mode: 'owned WSL foreground stdin lease; no global configuration changes', ready: false, stopped: false, exitCode: null }
  let expectedExit = false, readinessResolve, readinessReject, output = '', stopping
  const readiness = new Promise((resolve, reject) => { readinessResolve = resolve; readinessReject = reject })
  child.stdin.on('error', () => {})
  child.stdout.on('data', bytes => { output = (output + bytes).slice(-4096); if (output.includes(marker)) { evidence.ready = true; readinessResolve() } })
  child.stderr.on('data', () => {})
  const exited = new Promise(resolve => {
    child.once('error', error => { readinessReject(error); resolve({ code: null, error: error.message }) })
    child.once('close', code => { evidence.exitCode = code; if (!evidence.ready) readinessReject(new Error('WSL lease exited before readiness')); if (!expectedExit) onUnexpectedExit(); resolve({ code }) })
  })
  const stop = () => stopping ??= (async () => {
    expectedExit = true; child.stdin.end()
    const timeout = setTimeout(() => child.kill(), 15_000)
    try { const result = await exited; evidence.stopped = true; if (result.code !== 0) throw new Error('Owned WSL foreground lease did not exit cleanly') }
    finally { clearTimeout(timeout) }
  })()
  const readyTimeout = setTimeout(() => readinessReject(new Error('WSL foreground lease readiness timed out')), 30_000)
  try { await readiness; return { evidence, stop } }
  catch (error) { try { await stop() } catch {} throw error }
  finally { clearTimeout(readyTimeout) }
}

/** Standards-only ZIP construction; both storage methods contain identical files. */
export function zipArchive(files, compressed = false) {
  const members = [], central = []; let offset = 0
  for (const [name, value] of Object.entries(files).sort(([a], [b]) => a.localeCompare(b))) {
    const filename = Buffer.from(name), bytes = Buffer.from(value), data = compressed ? deflateRawSync(bytes) : bytes
    const local = Buffer.alloc(30), entry = Buffer.alloc(46), checksum = crc32(bytes)
    local.writeUInt32LE(0x04034b50); local.writeUInt16LE(20, 4); local.writeUInt16LE(0x800, 6); local.writeUInt16LE(compressed ? 8 : 0, 8); local.writeUInt16LE(33, 12)
    local.writeUInt32LE(checksum, 14); local.writeUInt32LE(data.length, 18); local.writeUInt32LE(bytes.length, 22); local.writeUInt16LE(filename.length, 26)
    entry.writeUInt32LE(0x02014b50); entry.writeUInt16LE(0x314, 4); entry.writeUInt16LE(20, 6); entry.writeUInt16LE(0x800, 8); entry.writeUInt16LE(compressed ? 8 : 0, 10); entry.writeUInt16LE(33, 14)
    entry.writeUInt32LE(checksum, 16); entry.writeUInt32LE(data.length, 20); entry.writeUInt32LE(bytes.length, 24); entry.writeUInt16LE(filename.length, 28); entry.writeUInt32LE(0o100644 << 16 >>> 0, 38); entry.writeUInt32LE(offset, 42)
    members.push(local, filename, data); central.push(entry, filename); offset += local.length + filename.length + data.length
  }
  const directory = Buffer.concat(central), end = Buffer.alloc(22), count = Object.keys(files).length
  end.writeUInt32LE(0x06054b50); end.writeUInt16LE(count, 8); end.writeUInt16LE(count, 10); end.writeUInt32LE(directory.length, 12); end.writeUInt32LE(offset, 16)
  return Buffer.concat([...members, directory, end])
}

function versionFiles(marker) {
  return {
    'index.html': `<!doctype html><html><head><link rel="stylesheet" href="./assets/style.css"></head><body>${marker}<script type="module" src="./assets/app.mjs"></script></body></html>\n`,
    'assets/app.mjs': `export const version = ${JSON.stringify(marker)}; document.body.dataset.release = version;\n`,
    'assets/style.css': `body::after { content: ${JSON.stringify(marker)}; }\n`,
  }
}

export async function runZipReleaseE2E(options) {
  const outDir = path.resolve(options.outDir)
  validateOutputLocation(outDir, root)
  if (existsSync(outDir) || root === outDir || root.startsWith(outDir + path.sep) || sourceRoots.some(relative => outDir === path.join(root, relative) || outDir.startsWith(path.join(root, relative) + path.sep))) throw new Error('ZIP acceptance output must be a fresh directory outside source inputs')
  mkdirSync(path.join(outDir, 'logs'), { recursive: true, mode: 0o700 })
  const owner = `phase05-${Date.now()}-${randomBytes(4).toString('hex')}`, label = 'saas.phase05.owner'
  const windows = process.platform === 'win32', distro = process.env.CI_WSL_DISTRO || 'Ubuntu'
  const dockerProgram = windows ? 'wsl.exe' : 'docker'
  const dockerPrefix = windows ? ['-d', distro, '--exec', 'env', 'BUILDX_NO_DEFAULT_ATTESTATIONS=1', 'docker', '--host', 'unix:///var/run/docker.sock'] : ['--host', 'unix:///var/run/docker.sock']
  const linuxPath = value => windows ? `/mnt/${path.resolve(value)[0].toLowerCase()}/${path.resolve(value).slice(3).replaceAll('\\', '/')}` : path.resolve(value)
  const secrets = new Set(), containers = new Set(), networks = new Set(), volumes = new Set()
  const secretRoot = `/var/tmp/saas-${owner}-secrets`
  let secretsCreated = false, cancelled = false, wslLease
  const redact = value => redactReport([...secrets].reduce((result, secret) => result.replaceAll(secret, '[REDACTED]'), String(value)))
  const childEnvironment = { ...commandEnvironment(), BUILDX_NO_DEFAULT_ATTESTATIONS: '1' }
  const report = { schemaVersion: 1, owner, kind: 'REAL_RUST_POSTGRES_NGINX_ZIP', status: 'FAIL', startedAt: new Date().toISOString(), checks: [], commands: [], cleanupErrors: [],
    registryPublication: 'NOT_REQUESTED', targetDeployment: 'NOT_RUN', secrets: { archived: false, location: 'owned Linux directory outside evidence/source', cleanup: 'NOT_RUN' } }
  const save = () => {
    report.pass = report.checks.filter(check => check.status === 'PASS').length
    report.fail = report.checks.filter(check => check.status === 'FAIL').length + (report.error && !report.checks.some(check => check.status === 'FAIL') ? 1 : 0) + (report.cleanupErrors.length ? 1 : 0)
    report.skip = 0; report.notRun = requiredCheckIds.length - report.checks.length; report.requiredCheckIds = requiredCheckIds
    writeFileSync(path.join(outDir, 'result.json'), `${JSON.stringify(report, null, 2)}\n`, { mode: 0o600 })
  }
  const write = (relative, bytes) => { const target = path.join(outDir, relative); mkdirSync(path.dirname(target), { recursive: true }); writeFileSync(target, bytes, { mode: 0o600 }); return target }
  const checkCancelled = () => { if (cancelled) throw new Error('ZIP acceptance cancelled') }
  const onSignal = () => { cancelled = true; report.cancelled = true }
  process.once('SIGINT', onSignal); process.once('SIGTERM', onSignal)
  async function command(name, program, args, { input, timeout = 45 * 60_000, allowFailure = false, recordOutput = true } = {}) {
    const logfile = `logs/${String(report.commands.length + 1).padStart(3, '0')}-${name}.log`
    const record = { name, executable: path.basename(program), args: args.map(redact), log: logfile, exitCode: null, ...(allowFailure ? { readinessOrOwnershipProbe: true } : {}) }; report.commands.push(record); save()
    return new Promise((resolve, reject) => {
      const child = spawn(program, args, { cwd: root, env: childEnvironment, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] }); let output = '', timedOut = false
      const timer = setTimeout(() => { timedOut = true; child.kill() }, timeout)
      child.stdout.on('data', bytes => { output += bytes }); child.stderr.on('data', bytes => { output += bytes })
      child.stdin.on('error', () => {}); child.stdin.end(input)
      child.on('error', error => { clearTimeout(timer); record.error = redact(error.message); save(); reject(error) })
      child.on('close', code => { clearTimeout(timer); record.exitCode = code; record.timedOut = timedOut; write(logfile, recordOutput ? redact(output) : '[output deliberately not archived]\n'); save()
        if (!allowFailure && (code !== 0 || timedOut)) reject(new Error(`${name} failed; inspect ${logfile}: ${redact(output).slice(-3000)}`)); else resolve(output.trim()) })
    })
  }
  const docker = (name, args, settings) => command(name, dockerProgram, [...dockerPrefix, ...args], settings)
  const linux = (name, program, args, settings) => command(name, windows ? 'wsl.exe' : program, windows ? ['-d', distro, '--exec', program, ...args] : args, settings)
  async function inspect(name, kind = 'container', { allowMissing = false } = {}) {
    const raw = await docker(`inspect-${kind}`, [kind, 'inspect', name], { allowFailure: allowMissing })
    if (allowMissing && /No such (?:container|object|network|volume)/i.test(raw)) return null
    return JSON.parse(raw)[0]
  }
  async function owned(name, kind) {
    const state = await inspect(name, kind, { allowMissing: true })
    if (state) assert.equal((kind === 'container' ? state.Config.Labels : state.Labels)?.[label], owner, `Refusing unowned ${kind}`)
    return state
  }
  async function run(name, image, args = [], { hardened = true } = {}) {
    checkCancelled(); containers.add(name)
    await docker('run-container', ['run', '--detach', '--name', name, '--label', `${label}=${owner}`, ...(hardened ? ['--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', '--tmpfs', '/tmp:rw,nosuid,noexec,size=32m'] : []), ...args, image])
  }
  async function remove(name) { assert(containers.has(name)); if (await owned(name, 'container')) await docker('remove-container', ['rm', '--force', name]); containers.delete(name) }
  async function check(id, name, action) {
    checkCancelled(); console.log(`ZIP acceptance: ${name}`)
    try { const details = await action(); report.checks.push({ id, name, status: 'PASS', details }); save(); return details }
    catch (error) { report.checks.push({ id, name, status: 'FAIL', error: redact(error.stack || error.message) }); save(); throw error }
  }
  async function eventually(action, timeout = 60_000) { let last; const until = Date.now() + timeout; do { checkCancelled(); try { return await action() } catch (error) { last = error; await delay(300) } } while (Date.now() < until); throw last }
  const publicName = 'zip-phase05.test', origin = `https://${publicName}:8443`
  let port, ca, sessionToken, frontendBefore
  const network = `${owner}-private`, ingress = `${owner}-ingress`, backend = `${owner}-backend`, frontend = `${owner}-frontend`, edge = `${owner}-edge`, micro = `${owner}-micro`, postgres = `${owner}-postgres`, volume = `${owner}-zip-store`
  const mount = (source, destination) => ['--mount', `type=bind,source=${source},target=${destination},readonly`]
  function https(url, { method = 'GET', body, token = sessionToken, headers = {}, loseBody = false } = {}) {
    checkCancelled()
    const bytes = body === undefined ? undefined : Buffer.isBuffer(body) ? body : Buffer.from(JSON.stringify(body))
    return new Promise((resolve, reject) => {
      let settled = false
      const req = request({ hostname: '127.0.0.1', port, servername: publicName, path: url, method, ca,
        headers: { host: `${publicName}:8443`, origin, ...(token ? { authorization: `Bearer ${token}` } : {}), ...(bytes ? { 'content-length': bytes.length, 'content-type': Buffer.isBuffer(body) ? 'application/zip' : 'application/json' } : {}), ...headers } }, response => {
        if (loseBody) { const status = response.statusCode; response.destroy(); settled = true; return resolve({ status, bodyDiscardedAfterHeaders: true }) }
        const parts = []; response.on('data', bytes => parts.push(bytes)); response.on('error', reject); response.on('end', () => {
          settled = true; const body = Buffer.concat(parts); let json
          if ((response.headers['content-type'] || '').includes('application/json')) try { json = JSON.parse(body) } catch {}
          resolve({ status: response.statusCode, headers: response.headers, body, json })
        })
      })
      req.on('error', error => { if (!settled) reject(error) }); req.setTimeout(45_000, () => req.destroy(new Error('ZIP HTTPS request timeout'))); req.end(bytes)
    })
  }
  async function api(url, options) {
    const response = await https(url, options)
    assert.equal(response.status, 200, `${options?.method || 'GET'} ${url}: ${response.body?.toString().slice(0, 2500)}`)
    assert(response.json, `${url} must return actual JSON`); return response.json
  }
  try {
    if (windows) {
      wslLease = await startWslLease({ distro, owner, env: childEnvironment, onUnexpectedExit() { cancelled = true; report.hostLeaseUnexpectedExit = true } })
      report.runtimeHostLease = wslLease.evidence; save()
    }
    const source = createSourceManifest(root); write('source-manifest.json', `${JSON.stringify(source, null, 2)}\n`)
    report.sourceRevision = source.sourceRevision; report.inputsSha256 = source.inputsSha256
    await eventually(async () => { report.docker = JSON.parse(await docker('docker-version', ['version', '--format', '{{json .}}'], { allowFailure: true, timeout: 15_000 })); assert(report.docker.Server?.Version, 'Owned local Docker server must be ready') })
    let releaseCandidate
    if (options.releaseReport) {
      releaseCandidate = validateReleaseCandidate(JSON.parse(readFileSync(options.releaseReport, 'utf8')), { sourceRevision: source.sourceRevision })
      options.frontendImage = releaseCandidate.images.find(image => image.name === 'frontend').imageId
      options.edgeImage = releaseCandidate.images.find(image => image.name === 'edge').imageId
      report.releaseCandidate = { path: options.releaseReport, sha256: sha(readFileSync(options.releaseReport)), buildId: releaseCandidate.buildId, inputsSha256: releaseCandidate.inputsSha256 }
    }
    for (const [name, id] of [['frontend', options.frontendImage], ['edge', options.edgeImage]]) {
      const image = await inspect(id, 'image'); assert.equal(image.Id, id); assert.equal(`${image.Os}/${image.Architecture}`, 'linux/amd64')
      if (releaseCandidate) { assert.equal(image.Config.Labels['saas.ci.inputs-sha256'], releaseCandidate.inputsSha256); assert.equal(image.Config.Labels['org.opencontainers.image.revision'], source.sourceRevision) }
      report[`${name}Image`] = { id, imageUser: image.Config.User, targetPlatform: 'linux/amd64' }
    }
    const snapshotRoot = path.join(outDir, 'source'), snapshot = createSourceSnapshot(root, snapshotRoot)
    write('platform-source-manifest.json', `${JSON.stringify(snapshot, null, 2)}\n`); report.platformSourceInputsSha256 = snapshot.inputsSha256
    const platformTag = `${owner}-platform:zip-acceptance`
    const proxy = options.buildProxy ? ['--network', 'host', ...['HTTP_PROXY','HTTPS_PROXY','http_proxy','https_proxy'].flatMap(key => ['--build-arg', `${key}=${options.buildProxy}`])] : []
    let platformImage
    if (releaseCandidate) {
      validateReleaseCandidate(releaseCandidate, { sourceRevision: source.sourceRevision, inputsSha256: snapshot.inputsSha256 })
      platformImage = await inspect(releaseCandidate.images.find(image => image.name === 'platform').imageId, 'image')
      assert.equal(platformImage.Config.Labels['saas.ci.inputs-sha256'], snapshot.inputsSha256)
      assert.equal(platformImage.Config.Labels['org.opencontainers.image.revision'], source.sourceRevision)
      report.platformBuild = 'REUSED_CURRENT_VERIFIED_CANDIDATE'
    } else {
      await docker('build-current-rust-platform', ['build', '--force-rm', '--platform', 'linux/amd64', ...proxy, '--label', `${label}=${owner}`, '--label', `saas.inputs-sha256=${snapshot.inputsSha256}`, '--label', `org.opencontainers.image.revision=${source.sourceRevision}`, '--file', linuxPath(path.join(snapshotRoot, 'backend/Dockerfile')), '--tag', platformTag, linuxPath(snapshotRoot)])
      platformImage = await inspect(platformTag, 'image'); report.platformBuild = 'BUILT_CURRENT_SOURCE'
    }
    assert.equal(`${platformImage.Os}/${platformImage.Architecture}`, 'linux/amd64')
    report.platformImage = { id: platformImage.Id, targetPlatform: `${platformImage.Os}/${platformImage.Architecture}`, sourceInputsSha256: snapshot.inputsSha256 }
    checkCancelled()
    await linux('create-private-material-directory', 'mkdir', ['--mode=700', secretRoot]); secretsCreated = true
    const putPrivate = async (name, content, mode = 0o600) => linux('write-private-runtime-input', 'python3', ['-I', '-c', 'import os,sys\np=sys.argv[1]\nf=os.open(p,os.O_CREAT|os.O_EXCL|os.O_WRONLY,int(sys.argv[2]))\nwith os.fdopen(f,"wb") as s:s.write(sys.stdin.buffer.read())', `${secretRoot}/${name}`, String(mode)], { input: content, recordOutput: false })
    await putPrivate('server.cnf', `[req]\ndistinguished_name=dn\nprompt=no\n[dn]\nCN=${publicName}\n[extensions]\nsubjectAltName=DNS:${publicName}\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n`)
    await linux('create-isolated-ca', 'openssl', ['req','-x509','-newkey','rsa:2048','-nodes','-days','1','-subj','/CN=Phase05 ZIP Isolated CA','-keyout',`${secretRoot}/ca.key`,'-out',`${secretRoot}/ca.pem`])
    await linux('create-server-csr', 'openssl', ['req','-newkey','rsa:2048','-nodes','-config',`${secretRoot}/server.cnf`,'-keyout',`${secretRoot}/server.key`,'-out',`${secretRoot}/server.csr`])
    await linux('sign-isolated-server-certificate', 'openssl', ['x509','-req','-days','1','-in',`${secretRoot}/server.csr`,'-CA',`${secretRoot}/ca.pem`,'-CAkey',`${secretRoot}/ca.key`,'-CAcreateserial','-extfile',`${secretRoot}/server.cnf`,'-extensions','extensions','-out',`${secretRoot}/server.pem`])
    await linux('certificate-mount-permissions', 'chmod', ['0444',`${secretRoot}/server.key`,`${secretRoot}/server.pem`,`${secretRoot}/ca.pem`])
    ca = await linux('load-public-test-ca', 'cat', [`${secretRoot}/ca.pem`], { recordOutput: false }); report.testCaSha256 = sha(ca)
    for (const [name, internal] of [[network, true], [ingress, false]]) { networks.add(name); await docker('create-owned-network', ['network','create',...(internal ? ['--internal'] : []),'--label',`${label}=${owner}`,name]) }
    volumes.add(volume); await docker('create-owned-zip-volume', ['volume','create','--label',`${label}=${owner}`,volume])
    const initializeVolume = `${owner}-volume-init`; containers.add(initializeVolume)
    await docker('initialize-owned-volume-permissions', ['run','--name',initializeVolume,'--label',`${label}=${owner}`,'--network','none','--read-only','--user','0:0','--mount',`type=volume,source=${volume},target=/micro-apps`,'--entrypoint','sh',options.edgeImage,'-ec','chown 10001:10001 /micro-apps; chmod 0755 /micro-apps'])
    await remove(initializeVolume)
    const pgPassword = randomBytes(24).toString('hex'), bootstrapPassword = randomBytes(24).toString('hex'), jwtSecret = randomBytes(32).toString('hex'), metricsToken = randomBytes(32).toString('hex')
    for (const secret of [pgPassword, bootstrapPassword, jwtSecret, metricsToken]) secrets.add(secret)
    await putPrivate('postgres.env', `POSTGRES_USER=ci\nPOSTGRES_PASSWORD=${pgPassword}\nPOSTGRES_DB=postgres\n`)
    // Production bootstrap imports the complete regional catalog (665k rows),
    // so include space for its indexes and WAL as well as the ZIP test rows.
    await run(postgres, 'postgres:17.6-bookworm', ['--network',network,'--tmpfs','/var/lib/postgresql/data:rw,nosuid,size=2g','--shm-size','128m','--env-file',`${secretRoot}/postgres.env`], { hardened: false })
    await eventually(async () => assert.match(await docker('postgres-readiness', ['exec',postgres,'pg_isready','--username','ci','--dbname','postgres'], { allowFailure: true, timeout: 10_000 }), /accepting connections/))
    const microConfig = write('micro-static.conf', renderMicroStatic({ publicOrigin: origin }))
    const edgeConfig = write('gateway.conf', renderGateway({ publicOrigin: origin, zipStatic: 'micro-static:8080' }))
    // Bound configuration files contain no secrets and must be readable by
    // Nginx UID 101 on a native Linux runner as well as Windows/DrvFS.
    for (const file of [microConfig, edgeConfig]) chmodSync(file, 0o644)
    await run(frontend, options.frontendImage, ['--network',network,'--network-alias','frontend-web'])
    await run(micro, options.edgeImage, ['--network',network,'--network-alias','micro-static',...mount(linuxPath(microConfig),'/etc/nginx/nginx.conf'),'--mount',`type=volume,source=${volume},target=/micro-apps,readonly`])
    await run(edge, options.edgeImage, ['--network',ingress,'--publish','127.0.0.1::8443',...mount(linuxPath(edgeConfig),'/etc/nginx/nginx.conf'),...mount(`${secretRoot}/server.pem`,'/etc/nginx/tls/fullchain.pem'),...mount(`${secretRoot}/server.key`,'/etc/nginx/tls/privkey.pem')])
    await docker('connect-edge-private-network', ['network','connect','--alias',publicName,network,edge])
    const edgeState = await inspect(edge), edgeIp = edgeState.NetworkSettings.Networks[network].IPAddress
    port = Number(edgeState.NetworkSettings.Ports['8443/tcp'][0].HostPort)
    const databaseUrl = `postgresql://ci:${pgPassword}@${postgres}:5432/postgres`; secrets.add(databaseUrl)
    await putPrivate('platform.env', `APP_ENV=production\nAPPLICATION_ENVIRONMENT=production\nDATABASE_URL=${databaseUrl}\nAUTO_CREATE_DATABASE=false\nBIND_ADDR=0.0.0.0:3001\nJWT_SECRET=${jwtSecret}\nMETRICS_TOKEN=${metricsToken}\nDB_STATEMENT_TIMEOUT_MS=60000\nBOOTSTRAP_ADMIN_USERNAME=zip-admin\nBOOTSTRAP_ADMIN_PASSWORD=${bootstrapPassword}\nPUBLIC_APP_URL=${origin}\nMICRO_GATEWAY_ORIGIN=${origin}\nMICRO_ALLOWED_ORIGINS=${origin}\nCORS_ALLOWED_ORIGINS=${origin}\nTRUSTED_PROXY_IPS=${edgeIp}\nMICRO_CA_CERT_FILE=/run/zip-test-ca.pem\nMICRO_APPS_DIR=/micro-apps\nSTATIC_DIR=\nRUST_LOG=info\n`)
    await run(backend, platformImage.Id, ['--network',network,'--network-alias','backend','--env-file',`${secretRoot}/platform.env`,...mount(`${secretRoot}/ca.pem`,'/run/zip-test-ca.pem'),'--mount',`type=volume,source=${volume},target=/micro-apps`])
    await eventually(async () => assert.equal((await https('/api/health', { token: null })).status, 200), 180_000)
    const login = await api('/api/auth/login', { method: 'POST', token: null, body: { username: 'zip-admin', password: bootstrapPassword } }); secrets.add(login.ticket)
    assert(login.memberships.length === 1)
    const context = await api('/api/auth/context', { method: 'POST', token: login.ticket, body: { organizationId: login.memberships[0].organizationId } })
    sessionToken = context.accessToken; secrets.add(sessionToken); if (context.refreshToken) secrets.add(context.refreshToken); assert.equal(context.platformContext, true)
    const code = 'zip-e2e', app = await api('/api/applications', { method: 'POST', body: { code, name: 'ZIP isolated acceptance', basePath: `/micro/${code}`, entryType: 'HTML', entryUrl: `/sub-apps/${code}/index.html`, runtimeMode: 'IFRAME', status: 'ENABLED' } })
    report.applicationId = app.id
    const releasesPath = `/api/applications/${app.id}/releases`, uploadPath = (version, environment = 'production') => `${releasesPath}/upload?environment=${environment}&version=${version}&entryType=HTML&format=zip`
    const list = (environment = 'production') => api(`${releasesPath}?environment=${environment}`)
    const publicPath = (version, file = 'index.html') => `/sub-apps/${code}/versions/${version}/${file}`
    const filesV1 = versionFiles('ZIP_V1'), filesV2 = versionFiles('ZIP_V2'), archiveV1 = zipArchive(filesV1), archiveV2 = zipArchive(filesV2)
    const validateRow = row => { assert.match(row.id, /^[a-f0-9-]{36}$/); assert.match(row.artifactDigest, /^[a-f0-9]{64}$/); return row }
    const verifyFiles = async (version, files) => { const result = []; for (const [file, bytes] of Object.entries(files)) { const response = await https(publicPath(version, file), { token: null }); assert.equal(response.status, 200, file); assert.equal(response.body.toString(), bytes, file); result.push({ path: publicPath(version, file), sha256: sha(response.body) }) } return result }
    await check('production-boundaries', 'production TLS bootstrap and private ZIP deployment boundaries', async () => {
      const maintenance = JSON.parse(await docker('offline-staging-cli-scan', ['exec',backend,'env','APP_ENV=invalid','DATABASE_URL=invalid','JWT_SECRET=invalid','/usr/local/bin/saas-platform-backend','--micro-staging','--root','/micro-apps']))
      assert.equal(maintenance.mode, 'SCAN'); assert.deepEqual(maintenance.removed, []); assert.equal(maintenance.installedVersionsUntouched, true)
      frontendBefore = await inspect(frontend)
      for (const name of [backend, frontend, micro, postgres]) { const state = await inspect(name); assert.equal(Object.values(state.NetworkSettings.Ports || {}).filter(Boolean).length, 0); if (name !== postgres) assert.equal(state.HostConfig.ReadonlyRootfs, true) }
      const microState = await inspect(micro), backendState = await inspect(backend)
      assert.equal(microState.Mounts.find(mount => mount.Destination === '/micro-apps').RW, false)
      assert.equal(backendState.Mounts.find(mount => mount.Destination === '/micro-apps').RW, true)
      const zipVolumeFilesystem = await docker('record-actual-zip-volume-filesystem', ['exec',backend,'stat','-f','-c','%T','/micro-apps'])
      assert.equal(await docker('verify-static-readonly', ['exec',micro,'sh','-ec','if touch /micro-apps/forbidden-static-write 2>/dev/null; then exit 1; fi; printf readonly']), 'readonly')
      return { trustedHttps: true, appEnv: 'production', login: 'real production bootstrap -> credential ticket -> platform context session', pgStorage: 'owned tmpfs', onlyEdgePublished: true, frontendImageId: frontendBefore.Image, frontendContainerId: frontendBefore.Id, microStaticReadOnly: true, platformVolumeReadWrite: true, zipVolumeFilesystem,
        offlineStagingCli: { mode: maintenance.mode, removed: maintenance.removed, installedVersionsUntouched: maintenance.installedVersionsUntouched, invalidConfigDatabaseAndJwtEnvironment: true, platformNotStartedByScan: true } }
    })
    let v1, v2
    await check('response-recovery', 'upload committed with response body lost, GET reconciliation and identical retry preserve ID/digest', async () => {
      const dropped = await https(uploadPath('1.0.0'), { method: 'POST', body: archiveV1, loseBody: true }); assert.equal(dropped.status, 200)
      const rows = await list(); v1 = validateRow(rows.find(row => row.version === '1.0.0')); assert.equal(rows.filter(row => row.version === '1.0.0').length, 1)
      const retry = validateRow(await api(uploadPath('1.0.0'), { method: 'POST', body: archiveV1 })); assert.equal(retry.id, v1.id); assert.equal(retry.artifactDigest, v1.artifactDigest)
      const storage = await api(`${releasesPath}/storage?version=1.0.0`); assert.equal(storage.status, 'REGISTERED'); assert.equal(storage.artifactDigest, v1.artifactDigest); assert.equal(storage.registeredArtifactDigest, v1.artifactDigest); assert(storage.references.some(row => row.id === v1.id))
      assert.equal((await https(`${releasesPath}/storage?version=1.0.0`, { token: null })).status, 401)
      return { responseBodyDiscardedAfterHeaders: true, reconciledBy: 'GET releases filtered by environment/version and protected GET storage', releaseId: v1.id, artifactDigest: v1.artifactDigest, retryStable: true, storageStatus: storage.status, protectedReconciliation: true }
    })
    await check('v1-preflight-activate', 'v1 explicit preflight and activation use actual Nginx HTML/module/CSS', async () => {
      const checked = await api(`${releasesPath}/${v1.id}/preflight`, { method: 'POST' }); assert.equal(checked.validationStatus, 'PASSED')
      const deployment = await api(`${releasesPath}/${v1.id}/activate`, { method: 'POST' }); assert.equal(deployment.activeReleaseId, v1.id)
      return { releaseId: v1.id, preflight: checked.validationStatus, activeReleaseId: deployment.activeReleaseId, files: await verifyFiles('1.0.0', filesV1) }
    })
    await check('parallel-idempotent', 'same-version simultaneous uploads and different ZIP compression preserve one immutable artifact', async () => {
      const uploads = await Promise.all(Array.from({ length: 3 }, () => api(uploadPath('2.0.0'), { method: 'POST', body: archiveV2 })))
      v2 = validateRow(uploads[0]); for (const row of uploads) { assert.equal(row.id, v2.id); assert.equal(row.artifactDigest, v2.artifactDigest) }
      const compressed = zipArchive(filesV2, true); assert.notEqual(sha(compressed), sha(archiveV2))
      const alternate = validateRow(await api(uploadPath('2.0.0'), { method: 'POST', body: compressed })); assert.equal(alternate.id, v2.id); assert.equal(alternate.artifactDigest, v2.artifactDigest)
      const rows = await list(); assert.equal(rows.filter(row => row.version === '2.0.0').length, 1)
      return { concurrentRequests: 3, stableReleaseId: v2.id, artifactDigest: v2.artifactDigest, storedZipSha256: sha(archiveV2), deflatedZipSha256: sha(compressed), alternateCompressionIdempotent: true, files: await verifyFiles('2.0.0', filesV2) }
    })
    await check('cross-environment-immutable', 'different bytes for a version conflict across environments without changing published files', async () => {
      const rejected = await https(uploadPath('2.0.0', 'staging'), { method: 'POST', body: zipArchive(versionFiles('CHANGED_V2')) }); assert.equal(rejected.status, 409)
      const urlRegistration = await https(releasesPath, { method: 'POST', body: { environment: 'qa', version: '2.0.0', entryType: 'HTML', entryUrl: publicPath('2.0.0','alternate.html'), healthUrl: null, resources: [] } }); assert.equal(urlRegistration.status, 409)
      assert.equal((await list('qa')).filter(row => row.version === '2.0.0').length, 0)
      const promoted = validateRow(await api(uploadPath('2.0.0', 'staging'), { method: 'POST', body: archiveV2 })); assert.notEqual(promoted.id, v2.id); assert.equal(promoted.artifactDigest, v2.artifactDigest)
      const retried = validateRow(await api(uploadPath('2.0.0'), { method: 'POST', body: archiveV2 })); assert.equal(retried.id, v2.id); assert.equal(retried.artifactDigest, v2.artifactDigest)
      const current = await api(`/api/applications/${app.id}/deployments`); assert.equal(current.find(row => row.environment === 'production').activeReleaseId, v1.id)
      const storage = await api(`${releasesPath}/storage?version=2.0.0`); assert.equal(storage.status, 'REGISTERED'); assert.equal(storage.artifactDigest, v2.artifactDigest); assert.equal(storage.references.length, 2); assert(storage.references.every(row => row.artifactDigest === v2.artifactDigest))
      return { differentDigestStatus: rejected.status, ordinaryUrlRegistrationStatus: urlRegistration.status, rejectedOrdinaryRegistrationEnvironment: 'qa', ordinaryRegistrationRowAbsent: true, identicalRetryStableAfterRejectedRegistration: true, promotionEnvironment: 'staging', promotedReleaseId: promoted.id, artifactDigest: promoted.artifactDigest, productionPointerUnchanged: true, references: storage.references.map(({ id, environment, artifactDigest }) => ({ id, environment, artifactDigest })), files: await verifyFiles('2.0.0', filesV2) }
    })
    await check('activate-rollback', 'v2 activation and real rollback retain both old version resource URLs without rebuilding host', async () => {
      assert.equal((await api(`${releasesPath}/${v2.id}/preflight`, { method: 'POST' })).validationStatus, 'PASSED')
      const activated = await api(`${releasesPath}/${v2.id}/activate`, { method: 'POST' }); assert.equal(activated.activeReleaseId, v2.id); assert.equal(activated.previousReleaseId, v1.id)
      const whileV2 = [...await verifyFiles('1.0.0', filesV1), ...await verifyFiles('2.0.0', filesV2)]
      const rolledBack = await api(`/api/applications/${app.id}/deployments/production/rollback`, { method: 'POST' }); assert.equal(rolledBack.activeReleaseId, v1.id); assert.equal(rolledBack.previousReleaseId, v2.id)
      const afterRollback = [...await verifyFiles('1.0.0', filesV1), ...await verifyFiles('2.0.0', filesV2)], after = await inspect(frontend)
      assert.equal(after.Id, frontendBefore.Id); assert.equal(after.Image, frontendBefore.Image)
      return { activeBeforeRollback: v2.id, activeAfterRollback: v1.id, frontendImageId: after.Image, frontendContainerId: after.Id, hostUnchanged: true, whileV2, afterRollback }
    })
    await check('parallel-conflict', 'different-content concurrent same-version race exposes only the complete winning bytes', async () => {
      const variants = ['RACE_A','RACE_B'].map(marker => ({ ...versionFiles(marker), 'assets/data.bin': marker.repeat(90_000) }))
      const observations = { missing: 0, complete: 0 }; let uploadsFinished = false, observationError
      const observe = async () => {
        const entry = await https(publicPath('race-1'), { token: null })
        if (entry.status === 404) { observations.missing++; return }
        assert.equal(entry.status, 200)
        const possible = variants.find(files => files['index.html'] === entry.body.toString()); assert(possible, 'Observed public HTML must belong to one complete candidate')
        await verifyFiles('race-1', possible); observations.complete++
      }
      const observing = (async () => { do { await observe(); if (!uploadsFinished) await delay(20) } while (!uploadsFinished) })().catch(error => { observationError = error })
      let results
      try { results = await Promise.all(variants.map(files => https(uploadPath('race-1'), { method: 'POST', body: zipArchive(files) }))) }
      finally { uploadsFinished = true; await observing }
      if (observationError) throw observationError
      await observe(); assert(observations.complete > 0)
      assert.deepEqual(results.map(result => result.status).sort(), [200, 409])
      const winnerIndex = results.findIndex(result => result.status === 200), winner = validateRow(results[winnerIndex].json)
      const rows = await list(); assert.equal(rows.filter(row => row.version === 'race-1').length, 1); assert.equal(rows.find(row => row.version === 'race-1').id, winner.id)
      return { statuses: results.map(result => result.status), winner: winnerIndex === 0 ? 'A' : 'B', releaseId: winner.id, artifactDigest: winner.artifactDigest, observations, observationRule: '404 before publication or a complete consistent candidate across HTML/script/style/asset', files: await verifyFiles('race-1', variants[winnerIndex]) }
    })
    await check('interrupted-private-staging', 'interrupted HTTP upload and private staging never become public static versions', async () => {
      const partial = zipArchive({ ...versionFiles('INTERRUPTED'), 'assets/payload.bin': randomBytes(1024 * 1024) })
      await new Promise((resolve, reject) => {
        const req = request({ hostname: '127.0.0.1', port, servername: publicName, ca, path: uploadPath('interrupted-1'), method: 'POST', headers: { host: `${publicName}:8443`, origin, authorization: `Bearer ${sessionToken}`, 'content-type': 'application/zip', 'content-length': partial.length } })
        req.on('error', error => { if (error.code === 'ECONNRESET') resolve(); else reject(error) }); req.on('response', response => { response.resume(); reject(new Error('Incomplete upload unexpectedly produced a response before cancellation')) })
        req.write(partial.subarray(0, 128 * 1024), async error => { if (error) return reject(error); await delay(100); req.destroy(); resolve() })
      })
      assert.equal((await https(publicPath('interrupted-1'), { token: null })).status, 404); assert.equal((await list()).some(row => row.version === 'interrupted-1'), false)
      await docker('seed-private-staging-probe', ['exec',backend,'sh','-ec','mkdir -p /micro-apps/.staging-e2e /micro-apps/zip-e2e/staging; printf PRIVATE > /micro-apps/.staging-e2e/private.txt; printf PRIVATE > /micro-apps/zip-e2e/staging/private.txt'])
      for (const url of ['/sub-apps/.staging-e2e/private.txt', '/sub-apps/zip-e2e/staging/private.txt', '/sub-apps/zip-e2e/versions/no-such/index.html']) assert.equal((await https(url, { token: null, headers: { accept: 'text/html' } })).status, 404)
      return { interruptedVersion: 'interrupted-1', publiclyReadable: false, databaseRowPresent: false, stagingDenied: true,
        interruptionScope: 'HTTP body interrupted before full extraction; filesystem write interruption is covered by the Rust installer regression tests', preservedV1: await verifyFiles('1.0.0', filesV1), preservedV2: await verifyFiles('2.0.0', filesV2) }
    })
    await check('private-metadata', 'real metadata files and encoded variants stay 404 for GET and HEAD', async () => {
      await docker('seed-actual-private-metadata-files', ['exec',backend,'sh','-ec','mkdir -p /micro-apps/leak-probe/versions/1.0.0/assets /micro-apps/leak-probe/versions/1.0.0/.locks /micro-apps/leak-probe/versions/1.0.0/.staging; for f in release.json RELEASE.JSON assets/Release.Json .release-artifact.json .locks/private .staging/private; do printf PRIVATE_METADATA > "/micro-apps/leak-probe/versions/1.0.0/$f"; done'])
      const prefix = '/sub-apps/leak-probe/versions/1.0.0/'
      const paths = ['release.json','RELEASE.JSON','assets/Release.Json','%72elease%2ejson','.release-artifact.json','%2erelease-artifact.json','.locks/private','.staging/private']
      for (const method of ['GET','HEAD']) for (const suffix of paths) { const response = await https(`${prefix}${suffix}`, { method, token: null }); assert.equal(response.status, 404, `${method} ${suffix}`); assert.equal(response.headers['cache-control'], 'no-store', suffix); assert.doesNotMatch(response.body.toString(), /PRIVATE_METADATA/) }
      await docker('verify-real-installed-private-metadata', ['exec',backend,'sh','-ec','test -f /micro-apps/zip-e2e/versions/1.0.0/.artifact-manifest.json; test -f /micro-apps/.staging/.maintenance.lock'])
      const installedPrivate = [publicPath('1.0.0','.artifact-manifest.json'), publicPath('1.0.0','%2eartifact-manifest.json'), '/sub-apps/.staging/.maintenance.lock']
      for (const method of ['GET','HEAD']) for (const url of installedPrivate) { const response = await https(url, { method, token: null }); assert.equal(response.status, 404, `${method} ${url}`); assert.equal(response.headers['cache-control'], 'no-store', url) }
      const harmlessQuery = await https(`${publicPath('1.0.0','assets/app.mjs')}?label=release.json`, { token: null }); assert.equal(harmlessQuery.status, 200)
      return { actualFilesSeeded: true, actualInstalledManifestAndMaintenanceLockExist: true, requests: (paths.length + installedPrivate.length) * 2, methods: ['GET','HEAD'], deniedPaths: paths, installedPrivate, status: 404, cacheControl: 'no-store', harmlessQueryAllowed: true }
    })
    report.status = report.checks.length === requiredCheckIds.length && report.checks.every(check => check.status === 'PASS') ? 'PASS' : 'FAIL'
  } catch (error) { report.error = redact(error.stack || error.message); report.status = 'FAIL'; throw error }
  finally {
    for (const name of containers) try { if (await owned(name, 'container')) { write(`logs/${name}-runtime.log`, redact(await docker('collect-owned-runtime-logs', ['logs',name], { allowFailure: true, timeout: 15_000 }))); await docker('cleanup-owned-container', ['rm','--force',name], { timeout: 20_000 }) } } catch (error) { report.cleanupErrors.push(redact(error.message)) }
    for (const name of volumes) try { if (await owned(name, 'volume')) await docker('cleanup-owned-volume', ['volume','rm',name], { timeout: 20_000 }) } catch (error) { report.cleanupErrors.push(redact(error.message)) }
    for (const name of networks) try { if (await owned(name, 'network')) await docker('cleanup-owned-network', ['network','rm',name], { timeout: 20_000 }) } catch (error) { report.cleanupErrors.push(redact(error.message)) }
    if (secretsCreated) try {
      await linux('cleanup-private-runtime-material', 'python3', ['-I','-c','import os,pathlib,sys\np=pathlib.Path(sys.argv[1])\nif p.parent!=pathlib.Path("/var/tmp") or p.name!=sys.argv[2] or p.is_symlink() or p.stat().st_uid!=os.getuid(): raise RuntimeError("unowned private material directory")\nfor f in p.iterdir():\n if not f.is_file() or f.is_symlink(): raise RuntimeError("unexpected private material entry")\n f.unlink()\np.rmdir()',secretRoot,path.posix.basename(secretRoot)], { timeout: 30_000 }); report.secrets.cleanup = 'PASS'
    } catch (error) { report.secrets.cleanup = 'FAIL'; report.cleanupErrors.push(redact(error.message)) }
    if (wslLease) try { await wslLease.stop() } catch (error) { report.cleanupErrors.push(redact(error.message)) }
    if (cancelled || report.cleanupErrors.length) report.status = 'FAIL'
    report.finishedAt = new Date().toISOString(); save(); process.off('SIGINT', onSignal); process.off('SIGTERM', onSignal)
    console.log(`ZIP actual acceptance ${report.status}: ${outDir}`)
    if (report.cleanupErrors.length) throw new Error('ZIP acceptance owned resource cleanup failed')
  }
  if (report.status !== 'PASS') throw new Error('ZIP acceptance failed')
  return report
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { await runZipReleaseE2E(parseArgs(process.argv.slice(2))) }
  catch (error) { console.error(redactReport(error.message)); process.exitCode = 1 }
}
