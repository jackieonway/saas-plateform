import { spawn, spawnSync } from 'node:child_process'
import assert from 'node:assert/strict'
import { randomBytes, createHash } from 'node:crypto'
import { cpSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, realpathSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { sourceRoots, sourceFiles, excluded, createSourceManifest, validatePublicSource } from './provenance.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const labelKey = 'saas.ci.owner'
const browserToolCapabilities = Object.freeze(['CHOWN', 'SETUID', 'SETGID'])
export function validateToolContainerSecurity(args, { gate } = {}) {
  assert.ok(['fast', 'postgres', 'browser', 'delivery'].includes(gate), 'Tool security policy requires a declared gate')
  const values = option => args.flatMap((value, index) => value === option ? [args[index + 1]] : value.startsWith(option + '=') ? [value.slice(option.length + 1)] : [])
  assert.deepEqual(values('--cap-drop'), ['ALL'], 'Owned tools must drop every default capability')
  assert.deepEqual(values('--cap-add'), gate === 'browser' ? browserToolCapabilities : [], 'Only the browser Nginx tool may regain CHOWN, SETUID and SETGID')
  assert.deepEqual(values('--security-opt'), ['no-new-privileges'], 'Owned tools must retain no-new-privileges')
  assert.ok(!args.some(value => /^--privileged(?:=|$)/.test(value)), 'Owned tools must never be privileged')
}
const contains = (parent, child) => { const relative = path.relative(parent, child); return relative === '' || (relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative)) }
function physicalPath(value) {
  let current = path.resolve(value); const tail = []
  while (!existsSync(current)) { const parent = path.dirname(current); if (parent === current) throw new Error('Output path has no accessible ancestor'); tail.unshift(path.basename(current)); current = parent }
  return path.join(realpathSync(current), ...tail)
}
export function validateOutputLocation(outDir, sourceRoot = root) {
  if (!path.isAbsolute(outDir || '')) throw new Error('Linux runner output must be an absolute directory')
  const destination = physicalPath(outDir), source = physicalPath(sourceRoot)
  if (contains(destination, source) || sourceRoots.some(relative => contains(path.join(source, relative), destination))) {
    throw new Error('Linux runner output must not contain the repository or be inside an included source directory')
  }
}
export function snapshotFilter(source, sourceRoot = root) {
  const relative = path.relative(sourceRoot, source).replaceAll('\\', '/')
  if (excluded(relative)) return false
  const stat = lstatSync(source)
  if (stat.isSymbolicLink()) throw new Error(`Source symlink refused: ${relative}`)
  if (stat.isFile()) validatePublicSource(relative, readFileSync(source))
  return true
}
function hashFile(file) { return createHash('sha256').update(readFileSync(file)).digest('hex') }
function manifest(directory) {
  const files = []
  function walk(current) { for (const entry of readdirSync(current, { withFileTypes: true })) { const absolute = path.join(current, entry.name); if (entry.isDirectory()) walk(absolute); else if (entry.isFile()) files.push({ path: path.relative(directory, absolute).replaceAll('\\', '/'), sha256: hashFile(absolute) }); else throw new Error('Clean CI snapshot does not accept symbolic links') } }
  walk(directory); files.sort((a,b) => a.path.localeCompare(b.path)); return { schemaVersion: 1, files, sha256: createHash('sha256').update(JSON.stringify(files)).digest('hex') }
}
export function createSnapshot(destination, sourceRoot = root) {
  validateOutputLocation(destination, sourceRoot)
  if (existsSync(destination)) throw new Error('Refusing an existing snapshot directory')
  mkdirSync(destination, { recursive: true })
  for (const relative of [...sourceRoots, ...sourceFiles]) {
    const source = path.join(sourceRoot, relative)
    if (existsSync(source)) cpSync(source, path.join(destination, relative), { recursive: true, dereference: false, filter: file => snapshotFilter(file, sourceRoot) })
  }
  return manifest(destination)
}
const linuxPath = value => process.platform === 'win32' ? `/mnt/${path.resolve(value)[0].toLowerCase()}/${path.resolve(value).slice(3).replaceAll('\\', '/')}` : path.resolve(value)
function command(program, args, { timeout = 30 * 60_000, allowFailure = false } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(program, args, { cwd: root, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
    let output = '', timedOut = false
    const timer = setTimeout(() => { timedOut = true; child.kill('SIGTERM') }, timeout)
    child.stdout.on('data', bytes => { output += bytes }); child.stderr.on('data', bytes => { output += bytes })
    child.on('error', error => { clearTimeout(timer); reject(error) })
    child.on('close', code => { clearTimeout(timer); if (!allowFailure && (code !== 0 || timedOut)) reject(new Error(`${program} command failed (exit=${code}, timeout=${timedOut}): ${output.slice(-5000)}`)); else resolve(output.trim()) })
  })
}
export function createDocker({ distro = process.env.CI_WSL_DISTRO || 'Ubuntu' } = {}) {
  const prefix = process.platform === 'win32' ? ['-d', distro, '--exec', 'docker', '--host', 'unix:///var/run/docker.sock'] : ['--host', 'unix:///var/run/docker.sock']
  return (args, options) => command(process.platform === 'win32' ? 'wsl.exe' : (process.env.DOCKER_BIN || 'docker'), [...prefix, ...args], options)
}
export function validateGateSummary(summary, { gate, sourceRevision, inputsSha256 }) {
  if (summary.status !== 'PASS' || summary.gate !== gate || summary.sourceRevision !== sourceRevision || summary.inputsSha256 !== inputsSha256 || summary.mandatoryFailureCount !== 0 || summary.mandatorySkipCount !== 0 || !Array.isArray(summary.steps) || !summary.steps.length || summary.steps.some(step => step.mandatory && step.status !== 'PASS')) {
    throw new Error('Mandatory child gate report failed, skipped work or did not match the source snapshot')
  }
}
export function onceCleanup(action) {
  let pending
  return () => pending ??= Promise.resolve().then(action)
}
async function ownedInspect(docker, kind, name, owner) {
  const output = await docker([kind, 'inspect', name], { allowFailure: true })
  if (/No such (?:container|network|object)/i.test(output)) return null
  let state
  try { state = JSON.parse(output)[0] } catch { throw new Error(`Unable to verify ownership for ${kind} ${name}`) }
  if ((kind === 'container' ? state.Config.Labels : state.Labels)?.[labelKey] !== owner) throw new Error(`Refusing to remove an unowned ${kind}`)
  return state
}
export async function startPostgres({ docker, owner, network, checkCancelled = () => {} }) {
  if (!/^ci-[a-z0-9-]{8,80}$/.test(owner)) throw new Error('Invalid CI resource owner')
  if (network !== `${owner}-network`) throw new Error('PostgreSQL must join the private network owned by this run')
  const container = `${owner}-postgres`, password = randomBytes(24).toString('hex')
  // Register the deterministic name before docker run: startup failures can
  // still leave a created container that must be removed by the owner.
  const cleanup = async () => { if (await ownedInspect(docker, 'container', container, owner)) await docker(['rm', '--force', container]) }
  try {
    checkCancelled()
    const image = JSON.parse(await docker(['image', 'inspect', 'postgres:17.6-bookworm']))[0]
    checkCancelled()
    // Real platform integration imports the complete bundled region catalogue.
    // Its data, indexes and migration WAL exceed the former 768 MiB fixture.
    await docker(['run', '--detach', '--name', container, '--label', `${labelKey}=${owner}`, '--network', network, '--tmpfs', '/var/lib/postgresql/data:rw,nosuid,size=2g', '--shm-size', '128m', '--env', 'POSTGRES_USER=ci', '--env', `POSTGRES_PASSWORD=${password}`, '--env', 'POSTGRES_DB=postgres', image.Id, '-c', 'max_connections=40'])
    checkCancelled()
    const until = Date.now() + 60_000; let ready = false
    do { checkCancelled(); const result = await docker(['exec', container, 'pg_isready', '--username', 'ci', '--dbname', 'postgres'], { allowFailure: true, timeout: 10_000 }); checkCancelled(); if (result.includes('accepting connections')) { ready = true; break } await new Promise(resolve => setTimeout(resolve, 250)) } while (Date.now() < until)
    if (!ready) throw new Error('Owned temporary PostgreSQL failed readiness')
    const state = await ownedInspect(docker, 'container', container, owner)
    checkCancelled()
    if (Object.values(state.NetworkSettings.Ports || {}).some(value => value !== null)) throw new Error('Temporary PostgreSQL must not publish host ports')
    const version = await docker(['exec', container, 'postgres', '--version'])
    checkCancelled()
    return { container, urlForNetwork: `postgresql://ci:${password}@${container}:5432/postgres`, cleanup, evidence: { container, image: state.Image, localRepoDigests: image.RepoDigests, version, hostPorts: [], storage: 'owned tmpfs; removed with container' } }
  } catch (error) { await cleanup(); throw error }
}

export async function runLinux({ gate, outDir, browser = 'chromium', image = process.env.CI_TOOLCHAIN_IMAGE || 'saas-ci:phase04', sourceRoot = root, sourceRevision, docker: dockerAdapter, signalEmitter = process } = {}) {
  if (!['fast', 'postgres', 'browser'].includes(gate)) throw new Error('Linux runner supports fast, postgres or browser gates')
  if (!['chromium', 'chrome', 'msedge'].includes(browser)) throw new Error('Unknown required browser')
  validateOutputLocation(outDir, sourceRoot)
  if (existsSync(outDir)) throw new Error('Linux runner output must be a fresh absolute directory')
  mkdirSync(outDir, { recursive: true })
  const owner = `ci-${Date.now()}-${randomBytes(4).toString('hex')}`, network = `${owner}-network`, container = `${owner}-runner`, docker = dockerAdapter || createDocker()
  const report = { schemaVersion: 1, owner, gate, browser: gate === 'browser' ? browser : undefined, startedAt: new Date().toISOString(), status: 'FAIL', cleanupErrors: [] }
  const save = () => writeFileSync(path.join(outDir, 'linux-runner-result.json'), `${JSON.stringify(report, null, 2)}\n`)
  save()
  let cancelled = false
  const checkCancelled = () => { if (cancelled) throw new Error('Linux gate cancelled by signal') }
  let postgres, toolCreated = false, networkCreated = false, attaching = false, cancellationStop
  const cleanup = onceCleanup(async () => {
    if (toolCreated) try { if (await ownedInspect(docker, 'container', container, owner)) await docker(['rm', '--force', container]) } catch (error) { report.cleanupErrors.push(error.message) }
    if (postgres) try { await postgres.cleanup() } catch (error) { report.cleanupErrors.push(error.message) }
    if (networkCreated) try { if (await ownedInspect(docker, 'network', network, owner)) await docker(['network', 'rm', network]) } catch (error) { report.cleanupErrors.push(error.message) }
    report.finishedAt = new Date().toISOString(); if (report.cleanupErrors.length) report.status = 'FAIL'; save()
  })
  const onSignal = () => {
    cancelled = true; report.cancelled = true; report.status = 'FAIL'; report.error = 'Linux gate cancelled by signal'; save()
    if (attaching && !cancellationStop) cancellationStop = (async () => {
      const state = await ownedInspect(docker, 'container', container, owner)
      if (state) await docker(['rm', '--force', container], { timeout: 20_000 })
    })().catch(error => { report.cancellationStopError = error.message })
  }
  signalEmitter.once('SIGINT', onSignal); signalEmitter.once('SIGTERM', onSignal)
  try {
    report.source = createSnapshot(path.join(outDir, 'source'), sourceRoot)
    writeFileSync(path.join(outDir, 'snapshot-manifest.json'), `${JSON.stringify(report.source, null, 2)}\n`)
    delete report.source.files
    sourceRevision ??= spawnSync('git', ['-c', `safe.directory=${sourceRoot.replaceAll('\\','/')}`, 'rev-parse', 'HEAD'], { cwd: sourceRoot, encoding: 'utf8', windowsHide: true }).stdout?.trim()
    if (!/^[0-9a-f]{40}$/.test(sourceRevision || '')) throw new Error('Provide a full source revision for clean CI snapshots')
    report.sourceRevision = sourceRevision
    report.inputsSha256 = createSourceManifest(path.join(outDir, 'source'), { sourceRevision }).inputsSha256
    checkCancelled()
    report.dockerVersion = await docker(['version', '--format', '{{.Server.Version}}'])
    checkCancelled()
    const imageState = JSON.parse(await docker(['image', 'inspect', image]))[0]
    report.toolchainImage = { requested: image, id: imageState.Id, localRepoDigests: imageState.RepoDigests, architecture: imageState.Architecture, os: imageState.Os }
    checkCancelled()
    networkCreated = true
    await docker(['network', 'create', '--label', `${labelKey}=${owner}`, network])
    checkCancelled()
    if (gate === 'postgres' || gate === 'browser') { postgres = await startPostgres({ docker, owner, network, checkCancelled }); report.postgres = postgres.evidence }
    checkCancelled()
    const env = ['--env', 'CI=true', '--env', 'PLAYWRIGHT_BROWSERS_PATH=/opt/ci/browser/.browsers', '--env', `CI_SOURCE_REVISION=${sourceRevision}`]
    if (gate === 'browser') env.push('--env', 'CI_MANAGED_BROWSER_SOURCE_ROOT=/workspace')
    if (postgres) env.push('--env', `TEST_DATABASE_URL=${postgres.urlForNetwork}`, '--env', `CI_TEST_POSTGRES_OWNER=${owner}`)
    const args = ['node', 'scripts/ci/run.mjs', '--gate', gate, '--out-dir', `/reports/${gate}`, '--source-revision', sourceRevision]
    if (gate === 'browser') args.push('--browser', browser)
    // The official Nginx master needs these three operations to initialize its
    // owned temp files and drop worker identity. Other gates retain cap-drop ALL.
    const security = ['--cap-drop', 'ALL', ...(gate === 'browser' ? browserToolCapabilities.flatMap(capability => ['--cap-add', capability]) : []), '--security-opt', 'no-new-privileges']
    validateToolContainerSecurity(security, { gate })
    report.requestedToolSecurity = { capDrop: ['ALL'], capAdd: gate === 'browser' ? [...browserToolCapabilities] : [], securityOpt: ['no-new-privileges'] }
    toolCreated = true
    await docker(['create', '--name', container, '--label', `${labelKey}=${owner}`, '--network', network, '--workdir', '/workspace', '--init', '--shm-size', '1g', ...security, ...env, imageState.Id, ...args])
    checkCancelled()
    await docker(['cp', `${linuxPath(path.join(outDir, 'source'))}/.`, `${container}:/workspace`])
    checkCancelled()
    save()
    let output
    attaching = true
    try { output = await docker(['start', '--attach', container], { allowFailure: true, timeout: (gate === 'browser' ? 95 : gate === 'postgres' ? 50 : 45) * 60_000 }) }
    finally { attaching = false }
    writeFileSync(path.join(outDir, 'container.log'), output)
    checkCancelled()
    const state = await ownedInspect(docker, 'container', container, owner)
    checkCancelled()
    report.exitCode = state.State.ExitCode
    await docker(['cp', `${container}:/reports/.`, linuxPath(path.join(outDir, 'reports'))], { allowFailure: true })
    checkCancelled()
    if (state.State.Running) throw new Error('Tool container did not finish within the gate timeout')
    if (state.State.ExitCode !== 0) throw new Error(`Clean Linux ${gate} gate failed (exit=${state.State.ExitCode}); see container.log and reports`)
    if (!existsSync(path.join(outDir, 'reports', gate, 'summary.json'))) throw new Error('Mandatory CI summary is missing')
    validateGateSummary(JSON.parse(readFileSync(path.join(outDir, 'reports', gate, 'summary.json'), 'utf8')), { gate, sourceRevision, inputsSha256: report.inputsSha256 })
    report.status = 'PASS'
  } catch (error) { report.error = error.message }
  finally { if (cancellationStop) await cancellationStop; await cleanup(); signalEmitter.removeListener('SIGINT', onSignal); signalEmitter.removeListener('SIGTERM', onSignal) }
  if (report.status !== 'PASS') throw new Error(report.error || 'Linux runner cleanup failed')
  return report
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const input = {}; const args = process.argv.slice(2)
  try { for (let index=0; index<args.length; index+=2) { const key = { '--gate':'gate', '--out-dir':'outDir', '--browser':'browser', '--image':'image', '--source-revision':'sourceRevision' }[args[index]]; if (!key || !args[index+1]) throw new Error('Usage: node scripts/ci/linux-runner.mjs --gate fast|postgres|browser --out-dir <fresh absolute directory> [--browser chromium|chrome|msedge] [--image <owned toolchain image>]'); input[key] = args[index+1] } await runLinux(input) } catch (error) { console.error(error.message); process.exitCode = 1 }
}
