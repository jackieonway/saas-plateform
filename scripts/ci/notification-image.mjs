#!/usr/bin/env node
// Build and verify a local candidate only. No registry push or production run.
import assert from 'node:assert/strict'
import { createHash, randomBytes } from 'node:crypto'
import { existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createDocker, validateOutputLocation } from './linux-runner.mjs'
import { startWslLease } from '../../tests/zip-release-e2e.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const label = 'saas.phase23.image.owner'
const linuxPath = value => process.platform === 'win32' ? `/mnt/${path.resolve(value)[0].toLowerCase()}/${path.resolve(value).slice(3).replaceAll('\\', '/')}` : path.resolve(value)

export function parseNotificationImageArgs(args) {
  const options = {}, seen = new Set()
  const keys = { '--out-dir': 'outDir', '--source-revision': 'sourceRevision', '--build-proxy': 'buildProxy' }
  for (let index = 0; index < args.length; index += 2) {
    const key = keys[args[index]]
    if (!key || seen.has(key) || !args[index + 1]) throw new Error('Unknown, repeated or missing notification image argument')
    seen.add(key); options[key] = args[index + 1]
  }
  if (!path.isAbsolute(options.outDir || '') || !/^[a-f0-9]{40}$/.test(options.sourceRevision || '')) throw new Error('An absolute fresh --out-dir and full --source-revision are required')
  if (options.buildProxy && !/^http:\/\/(?:127\.0\.0\.1|localhost):[1-9][0-9]{0,4}$/.test(options.buildProxy)) throw new Error('Build proxy must be a credential-free loopback HTTP URL')
  return options
}

function copyRustInputs(destination) {
  const component = path.join(root, 'mini-services/notification-ws'), files = []
  function copy(relative) {
    const source = path.join(component, relative), stat = lstatSync(source)
    if (stat.isSymbolicLink()) throw new Error('Notification image source must not contain symlinks')
    if (stat.isDirectory()) {
      if (destination) mkdirSync(path.join(destination, relative), { recursive: true })
      for (const name of readdirSync(source).sort()) copy(path.join(relative, name))
    } else if (stat.isFile()) {
      if (relative.startsWith(`src${path.sep}`) && !relative.endsWith('.rs')) throw new Error('Only Rust source files may enter notification source snapshot')
      const bytes = readFileSync(source)
      if (destination) writeFileSync(path.join(destination, relative), bytes)
      files.push({ path: relative.replaceAll('\\', '/'), bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') })
    } else throw new Error('Unsupported notification image source')
  }
  if (destination) mkdirSync(destination)
  for (const relative of ['Cargo.toml', 'Cargo.lock', 'Dockerfile', '.dockerignore', 'src']) copy(relative)
  files.sort((left, right) => left.path.localeCompare(right.path, 'en'))
  return { files, inputsSha256: createHash('sha256').update(JSON.stringify(files)).digest('hex') }
}

export async function runNotificationImage(options) {
  const outDir = path.resolve(options.outDir)
  validateOutputLocation(outDir, root)
  if (existsSync(outDir)) throw new Error('Notification image output already exists; use a fresh directory')
  mkdirSync(path.join(outDir, 'logs'), { recursive: true, mode: 0o700 })
  const owner = `phase23-image-${Date.now()}-${randomBytes(4).toString('hex')}`
  const token = randomBytes(32).toString('hex'), password = randomBytes(24).toString('hex')
  const report = { schemaVersion: 1, status: 'RUNNING', owner, sourceRevision: options.sourceRevision,
    startedAt: new Date().toISOString(), publication: 'NOT_REQUESTED', productionDeployment: 'NOT_REQUESTED',
    checks: [], commands: [], images: [], cleanupErrors: [] }
  const save = () => writeFileSync(path.join(outDir, 'result.json'), JSON.stringify(report, null, 2) + '\n', { mode: 0o600 })
  const redact = text => String(text).replaceAll(token, '[redacted]').replaceAll(password, '[redacted]')
  const adapter = createDocker()
  let lease, created = false, cancelled = false
  const cancel = () => { cancelled = true; report.cancelled = true; save() }
  process.once('SIGINT', cancel); process.once('SIGTERM', cancel)
  async function docker(name, args, settings) {
    if (cancelled) throw new Error('Notification image verification was cancelled')
    const log = `logs/${String(report.commands.length + 1).padStart(3, '0')}-${name}.log`
    const record = { name, executable: process.platform === 'win32' ? 'wsl.exe' : 'docker', args: args.map(redact), log, exitCode: null }
    report.commands.push(record); save()
    try {
      const output = await adapter(args, settings)
      record.exitCode = 0; writeFileSync(path.join(outDir, log), redact(output) + '\n', { mode: 0o600 }); save(); return output
    } catch (error) {
      record.exitCode = Number(/exit=(\d+)/.exec(error.message)?.[1] || 1)
      writeFileSync(path.join(outDir, log), redact(error.message) + '\n', { mode: 0o600 }); save()
      throw new Error(`${name} failed; see ${log}`, { cause: error })
    }
  }
  async function base(reference) {
    const state = JSON.parse(await docker('base-image', ['image', 'inspect', reference]))[0]
    report.images.push({ role: 'base', requested: reference, imageId: state.Id, repoDigests: state.RepoDigests || [] })
    save(); return state.RepoDigests?.[0] || reference
  }
  try {
    if (process.platform === 'win32') {
      lease = await startWslLease({ owner, env: process.env, distro: process.env.CI_WSL_DISTRO || 'Ubuntu', onUnexpectedExit: cancel })
      report.wslLease = lease.evidence
    }
    report.dockerVersion = await docker('docker-version', ['version', '--format', '{{.Server.Version}}'])
    const rust = await base('rust:1.97.0-slim-bookworm'), runtime = await base('debian:bookworm-slim')
    const source = path.join(outDir, 'source'), inputs = copyRustInputs(source)
    report.inputsSha256 = inputs.inputsSha256
    writeFileSync(path.join(outDir, 'inputs.json'), JSON.stringify({ sourceRevision: options.sourceRevision, ...inputs }, null, 2) + '\n')
    const tag = `${owner}:candidate`
    const proxy = options.buildProxy ? ['--network', 'host', ...['HTTP_PROXY', 'HTTPS_PROXY', 'http_proxy', 'https_proxy'].flatMap(key => ['--build-arg', `${key}=${options.buildProxy}`])] : []
    await docker('build', ['build', '--force-rm', '--platform', 'linux/amd64', ...proxy, '--label', `${label}=${owner}`,
      '--label', `saas.ci.inputs-sha256=${inputs.inputsSha256}`, '--build-arg', `SOURCE_REVISION=${options.sourceRevision}`,
      '--build-arg', `RUST_IMAGE=${rust}`, '--build-arg', `RUNTIME_IMAGE=${runtime}`, '--tag', tag, linuxPath(source)], { timeout: 45 * 60_000 })
    const image = JSON.parse(await docker('candidate-image', ['image', 'inspect', tag]))[0]
    assert.equal(image.Config.Labels[label], owner)
    assert.equal(image.Config.Labels['org.opencontainers.image.revision'], options.sourceRevision)
    assert.equal(image.Config.Labels['saas.ci.inputs-sha256'], inputs.inputsSha256)
    assert.equal(image.Config.User, '10003:10003')
    assert.deepEqual(image.Config.Entrypoint, ['saas-notification-ws'])
    assert.equal(image.Os, 'linux')
    assert.equal(image.Architecture, 'amd64')
    report.images.push({ role: 'candidate', tag, imageId: image.Id, labels: image.Config.Labels, identityMeaning: 'local image ID; no registry publication' })
    report.checks.push('clean-Rust-source-locked-release-build-and-image-identity')
    // Deliberately unavailable dependencies: this validates honest readiness,
    // not the separate real-platform/PostgreSQL functional acceptance suite.
    created = true
    await docker('run', ['run', '--detach', '--name', owner, '--label', `${label}=${owner}`, '--network', 'none',
      '--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges:true', '--tmpfs', '/tmp:rw,noexec,nosuid,size=16m',
      '--env', 'NOTIFICATION_ENV=production', '--env', 'NOTIFICATION_HOST=0.0.0.0', '--env', 'NOTIFICATION_PORT=3003',
      '--env', `DATABASE_URL=postgresql://notification_ws:${password}@127.0.0.1:5432/unavailable`,
      '--env', 'PLATFORM_API_BASE_URL=https://127.0.0.1', '--env', 'NOTIFICATION_ALLOWED_ORIGINS=https://platform.example.test',
      '--env', `METRICS_TOKEN=${token}`, image.Id])
    await docker('runtime-probe', ['exec', owner, 'sh', '-ec',
      'test "$(id -u)" = 10003; test "$(id -g)" = 10003; for binary in node npm bun; do if command -v "$binary"; then exit 1; fi; done; ' +
      'for attempt in 1 2 3 4 5 6 7 8 9 10; do if curl --fail --silent --max-time 2 http://127.0.0.1:3003/health/live >/dev/null; then exit 0; fi; sleep 1; done; exit 1'])
    for (const [route, expected] of [['/health/live', '200'], ['/health/ready', '503'], ['/health', '503'], ['/metrics', '401']]) {
      const status = await docker(`probe-${route.replaceAll('/', '-')}`, ['exec', owner, 'curl', '--silent', '--max-time', '3', '--output', '/dev/null', '--write-out', '%{http_code}', `http://127.0.0.1:3003${route}`])
      assert.equal(status, expected, `${route} status`)
    }
    const metrics = await docker('metrics-authorized', ['exec', owner, 'curl', '--fail', '--silent', '--max-time', '3', '--header', `Authorization: Bearer ${token}`, 'http://127.0.0.1:3003/metrics'])
    assert.match(metrics, /saas_notification_/)
    report.checks.push('nonroot-no-Node-runtime', 'default-closed-stdin-keeps-production-service-running', 'liveness-UP-readiness-DOWN-with-dependencies-unavailable', 'metrics-require-independent-Bearer')
    const state = JSON.parse(await docker('runtime-inspect', ['container', 'inspect', owner]))[0]
    assert.equal(state.Config.User, '10003:10003')
    assert.equal(state.HostConfig.ReadonlyRootfs, true)
    assert.ok(state.HostConfig.CapDrop.includes('ALL'))
    assert.ok(state.HostConfig.SecurityOpt.some(value => value.startsWith('no-new-privileges')))
    assert.ok(!Object.keys(state.HostConfig.PortBindings || {}).length)
    assert.equal(state.Config.OpenStdin, false)
    assert.ok(!state.Mounts.some(mount => mount.Type === 'bind' || mount.Destination.includes('docker.sock')))
    report.runtime = { user: state.Config.User, readOnly: true, capabilities: [], noNewPrivileges: true, hostPorts: [], sourceOrSocketMounts: [], openStdin: false }
    report.checks.push('read-only-capability-free-private-container-without-host-mounts')
    const started = Date.now()
    await docker('sigterm-stop', ['stop', '--time', '15', owner], { timeout: 25_000 })
    const stopped = JSON.parse(await docker('stopped-inspect', ['container', 'inspect', owner]))[0]
    assert.equal(stopped.State.Running, false)
    assert.equal(stopped.State.ExitCode, 0, 'SIGTERM must finish without forced kill')
    const logs = await docker('runtime-logs', ['logs', owner])
    assert.ok(!logs.includes(token) && !logs.includes(password))
    report.shutdown = { signal: 'SIGTERM', exitCode: stopped.State.ExitCode, elapsedMs: Date.now() - started }
    report.checks.push('SIGTERM-graceful-zero-exit', 'logs-exclude-private-credentials')
    assert.equal(copyRustInputs().inputsSha256, inputs.inputsSha256, 'Notification image inputs changed during verification; rebuild a fresh candidate')
    report.currentSourceMatch = true
    report.checks.push('candidate-inputs-match-current-Rust-source')
    report.status = 'PASS'
  } catch (error) {
    report.status = 'FAIL'; report.error = redact(error.message)
    if (created && !cancelled) {
      try { await docker('failure-runtime-logs', ['logs', owner]) } catch { /* Preserve the original failure. */ }
      try { await docker('failure-runtime-state', ['container', 'inspect', owner]) } catch { /* Preserve the original failure. */ }
    }
    throw error
  }
  finally {
    process.removeListener('SIGINT', cancel); process.removeListener('SIGTERM', cancel)
    cancelled = false
    if (created) try {
      const actual = await docker('cleanup-owner', ['container', 'inspect', '--format', `{{index .Config.Labels "${label}"}}`, owner])
      if (actual !== owner) throw new Error('Refusing cleanup of an unowned notification container')
      await docker('cleanup-container', ['rm', '--force', owner])
    } catch (error) { report.cleanupErrors.push(redact(error.message)) }
    if (lease) try { await lease.stop() } catch (error) { report.cleanupErrors.push(redact(error.message)) }
    if (report.cleanupErrors.length) report.status = 'FAIL'
    report.finishedAt = new Date().toISOString(); save()
  }
  if (report.status !== 'PASS') throw new Error('Notification image verification cleanup failed')
  return report
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const options = parseNotificationImageArgs(process.argv.slice(2)), result = await runNotificationImage(options)
    console.log(JSON.stringify({ status: result.status, owner: result.owner, checks: result.checks.length, output: options.outDir }))
  } catch (error) { console.error(error.message); process.exitCode = 1 }
}
