import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { createServer } from 'node:http'
import { lstat, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import test from 'node:test'
import { createServicePlan, parseArgs, redactLog, supervise } from '../scripts/dev.mjs'

const fixture = path.resolve('tests/fixtures/dev-service.mjs')
const cleanupStacks = new WeakMap()
function cleanupAfter(t, callback) {
  if (!cleanupStacks.has(t)) {
    const callbacks = []
    cleanupStacks.set(t, callbacks)
    t.after(async () => {
      const errors = []
      for (const cleanup of callbacks.toReversed()) {
        try { await cleanup() } catch (error) { errors.push(error) }
      }
      if (errors.length) throw new AggregateError(errors, 'fixture cleanup failed')
    })
  }
  cleanupStacks.get(t).push(callback)
}
async function fixtureDirectory(t, prefix) {
  assert.ok(['saas-dev-plan-', 'saas-dev-descendant-', 'saas-dev-parent-exit-', 'saas-dev-group-drain-'].includes(prefix))
  const temporaryRoot = await realpath(tmpdir())
  const directory = await mkdtemp(path.join(temporaryRoot, prefix))
  cleanupAfter(t, async () => {
    const entry = await lstat(directory)
    assert.ok(entry.isDirectory() && !entry.isSymbolicLink(), 'temporary fixture must remain a real directory')
    const target = await realpath(directory)
    assert.ok(path.isAbsolute(target))
    assert.equal(target, path.resolve(directory), 'temporary fixture must not resolve through a link')
    assert.equal(path.dirname(target), temporaryRoot, 'temporary fixture must be a direct child of known os.tmpdir')
    assert.ok(path.basename(target).startsWith(prefix) && /^[A-Za-z0-9]{6}$/.test(path.basename(target).slice(prefix.length)))
    await rm(target, { recursive: true })
  })
  return directory
}
const listen = async (server) => {
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  return server.address().port
}
async function freePort() {
  const server = createServer()
  const port = await listen(server)
  await new Promise((resolve) => server.close(resolve))
  return port
}
async function service(name, mode = 'ready', extra = {}) {
  return {
    name, command: process.execPath, args: [fixture, mode], cwd: process.cwd(),
    host: '127.0.0.1', port: await freePort(), readyPath: '/', ...extra,
  }
}
function session(t, services, options = {}) {
  const controller = new AbortController()
  const output = []
  let ready
  const readiness = new Promise((resolve) => { ready = resolve })
  const result = supervise(services, {
    timeoutMs: 4000, signal: controller.signal,
    log: (line) => output.push(line), onReady: ready,
    ...options,
  })
  cleanupAfter(t, async () => { controller.abort(); await result })
  return { controller, output, readiness, result }
}
function ownChild(t, child) {
  const exited = new Promise((resolve) => child.once('exit', resolve))
  cleanupAfter(t, async () => {
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
    await exited
  })
  return exited
}
async function waitFile(file) {
  for (let n = 0; n < 100; n++) {
    try { return await readFile(file, 'utf8') } catch { await delay(30) }
  }
  throw new Error('fixture did not report its child PID')
}
async function gone(pid) {
  for (let n = 0; n < 100; n++) {
    try { process.kill(pid, 0) } catch { return }
    await delay(30)
  }
  assert.fail(`owned fixture process ${pid} is still running`)
}

test('CLI defaults select only the three static frontends; explicit options validate', () => {
  assert.deepEqual(parseArgs([]).apps, ['host', 'vue', 'jquery'])
  const options = parseArgs(['--apps', 'host,websocket', '--host', '::1', '--host-port', '4100', '--timeout-ms', '8000'])
  assert.equal(options.host, '::1')
  assert.equal(options.ports.host, 4100)
  assert.equal(options.timeoutMs, 8000)
  assert.deepEqual(parseArgs(['--apps', 'none', '--with-backend']).apps, [])
  const oauth = parseArgs(['--apps', 'none', '--with-oauth'])
  assert.equal(oauth.ports.oauth, 3010)
  assert.equal(oauth.ports['oauth-api'], 3011)
  for (const args of [
    ['--apps', 'unknown'], ['--apps', 'host,host'], ['--apps', 'none'],
    ['--host-port', '0'], ['--host-port', '65536'], ['--host-port', '1x'],
    ['--host', 'http://user:secret@host'], ['--timeout-ms', '-1'],
    ['--with-backend=secret'], ['--unknown'], ['--apps'],
    ['--host-port', '3005'],
    ['--apps', 'none', '--with-oauth', '--oauth-port', '3011'],
    ['--apps', 'host', '--with-oauth', '--oauth-api-port', '3000'],
  ]) assert.throws(() => parseArgs(args), undefined, args.join(' '))
})

test('OAuth starts an independent health-checked Rust API before its own Vite page with configured origins', async (t) => {
  const directory = await fixtureDirectory(t, 'saas-dev-plan-')
  const frontend = path.join(directory, 'mini-services/oauth-playground/frontend')
  const options = parseArgs(['--apps', 'none', '--with-oauth', '--host', '::1', '--oauth-port', '4310', '--oauth-api-port', '4311'])
  assert.throws(() => createServicePlan(options, directory), /oauth.*frontend ci/)
  await mkdir(path.join(frontend, 'node_modules/vite/bin'), { recursive: true })
  await writeFile(path.join(frontend, 'node_modules/vite/bin/vite.js'), '')
  await writeFile(path.join(frontend, 'package-lock.json'), '{}')
  const [api, page] = createServicePlan(options, directory)
  assert.equal(api.name, 'oauth-api')
  assert.equal(api.command, 'cargo')
  assert.deepEqual(api.args, ['run', '--locked'])
  assert.equal(api.cwd, path.join(directory, 'mini-services/oauth-playground/backend'))
  assert.equal(api.readyPath, '/health')
  assert.equal(api.env.OAUTH_BIND_ADDR, '[::1]:4311')
  assert.equal(api.env.OAUTH_PUBLIC_ORIGIN, 'http://[::1]:4310')
  assert.ok(!Object.hasOwn(api.env, 'OAUTH_ALLOW_INSECURE_LOOPBACK'), 'HTTP development must be explicitly opted into by private configuration')
  assert.equal(page.command, process.execPath)
  assert.equal(page.cwd, frontend)
  assert.ok(page.args.includes('--strictPort'))
  assert.equal(page.port, 4310)
  assert.equal(page.env.OAUTH_PLAYGROUND_API_TARGET, 'http://[::1]:4311')
  assert.deepEqual(Object.keys(page.env), ['OAUTH_PLAYGROUND_API_TARGET'])
  const wildcard = createServicePlan(parseArgs(['--apps', 'none', '--with-oauth', '--host', '0.0.0.0']), directory)
  assert.equal(wildcard[0].env.OAUTH_BIND_ADDR, '0.0.0.0:3011')
  assert.equal(wildcard[0].env.OAUTH_PUBLIC_ORIGIN, 'http://127.0.0.1:3010')
  assert.equal(wildcard[1].env.OAUTH_PLAYGROUND_API_TARGET, 'http://127.0.0.1:3011')
})

test('plans use each frontend own Vite installation and propagate configured proxy ports', async (t) => {
  const directory = await fixtureDirectory(t, 'saas-dev-plan-')
  for (const project of ['frontend', 'mini-services/micro-vue', 'mini-services/micro-jquery']) {
    const target = path.join(directory, project, 'node_modules/vite/bin')
    await mkdir(target, { recursive: true })
    await writeFile(path.join(target, 'vite.js'), '')
    await writeFile(path.join(directory, project, 'package-lock.json'), '{}')
  }
  const plan = createServicePlan(parseArgs(['--host', '::1', '--vue-port', '4105', '--jquery-port', '4106']), directory)
  assert.equal(plan[0].env.MICRO_VUE_DEV_PROXY_TARGET, 'http://[::1]:4105')
  assert.equal(plan[0].env.MICRO_JQUERY_DEV_PROXY_TARGET, 'http://[::1]:4106')
  for (const item of plan) {
    assert.equal(item.command, process.execPath)
    assert.equal(item.args[0], path.join(item.cwd, 'node_modules/vite/bin/vite.js'))
    assert.ok(item.args.includes('--strictPort'))
  }
  assert.throws(() => createServicePlan(parseArgs(['--apps', 'websocket']), directory), /npm.*ci/)
})

test('all selected children become HTTP ready and abort releases owned ports', async (t) => {
  const first = await service('first')
  const second = await service('second')
  const run = session(t, [first, second])
  const pids = await run.readiness
  assert.equal(pids.length, 2)
  assert.match(run.output.join('\n'), /All selected services are ready/)
  run.controller.abort()
  assert.equal(await run.result, 0)
  for (const pid of pids) await gone(pid)
  for (const item of [first, second]) {
    const server = createServer()
    server.listen(item.port, item.host)
    await once(server, 'listening')
    await new Promise((resolve) => server.close(resolve))
  }
})

test('a child exiting before readiness fails and cleans siblings plus descendant processes', async (t) => {
  const directory = await fixtureDirectory(t, 'saas-dev-descendant-')
  const pidFile = path.join(directory, 'child.pid')
  const ready = await service('owns-descendant', 'descendant', { env: { FIXTURE_PID_FILE: pidFile } })
  const failing = await service('early-exit', 'exit')
  const run = session(t, [ready, failing])
  const childPid = Number(await waitFile(pidFile))
  assert.equal(await run.result, 1)
  assert.match(run.output.join('\n'), /early-exit.*exited/)
  await gone(childPid)
})

test('POSIX shutdown drains the owned process group before returning reusable descendant listeners', { skip: process.platform === 'win32', timeout: 15000 }, async (t) => {
  const directory = await fixtureDirectory(t, 'saas-dev-group-drain-')
  const pidFile = path.join(directory, 'children.json')
  const item = await service('owns-listeners', 'descendant-listeners', { env: { FIXTURE_PID_FILE: pidFile } })
  const run = session(t, [item], { timeoutMs: 8000 })
  const pids = await Promise.race([run.readiness, run.result.then((code) => { throw new Error(`fixture startup exited ${code}: ${run.output.join('\n')}`) })])
  const descendants = JSON.parse(await waitFile(pidFile))
  assert.equal(descendants.length, 12)
  run.controller.abort()
  assert.equal(await run.result, 0)
  for (const pid of pids) assert.throws(() => process.kill(-pid, 0), { code: 'ESRCH' }, 'supervise must return only after its process group has exited')
  for (const { port } of descendants) {
    const server = createServer()
    server.listen(port, item.host)
    await once(server, 'listening')
    await new Promise((resolve) => server.close(resolve))
  }
})

test('readiness has a deadline and cleanup also owns a child that never listens', async (t) => {
  const item = await service('never-ready', 'hang')
  const run = session(t, [item], { timeoutMs: 400 })
  assert.equal(await run.result, 1)
  assert.match(run.output.join('\n'), /readiness deadline/)
})

test('continuous partial response headers cannot extend the absolute readiness deadline', { timeout: 6000 }, async (t) => {
  const item = await service('slow-headers', 'slow-headers')
  const started = Date.now()
  const run = session(t, [item], { timeoutMs: 1600 })
  const result = await Promise.race([
    run.result,
    delay(3600).then(() => { throw new Error('continuous headers exceeded startup plus bounded cleanup budget') }),
  ])
  assert.equal(result, 1)
  assert.ok(Date.now() - started < 3600)
  assert.match(run.output.join('\n'), /slow-header drip/)
  assert.match(run.output.join('\n'), /readiness deadline/)
  const server = createServer()
  server.listen(item.port, item.host)
  await once(server, 'listening')
  await new Promise((resolve) => server.close(resolve))
})

test('occupied ports fail without starting services or terminating the existing listener', async (t) => {
  const owner = createServer((_, response) => response.end('existing-owner'))
  const port = await listen(owner)
  try {
    const run = session(t, [await service('occupied', 'ready', { port })])
    assert.equal(await run.result, 1)
    assert.match(run.output.join('\n'), /occupied.*already in use/)
    assert.equal(await (await fetch(`http://127.0.0.1:${port}`)).text(), 'existing-owner')
    assert.doesNotMatch(run.output.join('\n'), /Starting occupied/)
  } finally { await new Promise((resolve) => owner.close(resolve)) }
})

test('post-readiness exit also fails and stops the other owned service', async (t) => {
  const persistent = await service('persistent')
  const crash = await service('later-exit', 'later-exit')
  const run = session(t, [persistent, crash])
  const pids = await run.readiness
  assert.equal(await run.result, 1)
  assert.match(run.output.join('\n'), /later-exit.*exited/)
  for (const pid of pids) await gone(pid)
})

test('child output hides environment secrets, URL userinfo and sensitive query values', async (t) => {
  const secret = 'fixture-database-password'
  const item = await service('redaction', 'log-secrets', { env: { DATABASE_URL: secret } })
  const run = session(t, [item])
  await run.readiness
  run.controller.abort()
  assert.equal(await run.result, 0)
  const output = run.output.join('\n')
  for (const value of [secret, 'user:pass', 'query-secret']) assert.ok(!output.includes(value))
  assert.match(output, /\[redacted\]/)
  assert.equal(redactLog('Bearer token-value', { ACCESS_TOKEN: 'token-value' }), 'Bearer [redacted]')
})

test('a missing executable fails promptly and cleans the already ready sibling', async (t) => {
  const first = await service('first')
  const failing = await service('missing', 'ready', { command: 'saas-dev-fixture-nonexistent-command' })
  const run = session(t, [first, failing])
  assert.equal(await run.result, 1)
  assert.match(run.output.join('\n'), /missing.*command could not start/)
  const server = createServer()
  server.listen(first.port, first.host)
  await once(server, 'listening')
  await new Promise((resolve) => server.close(resolve))
})

test('unexpected supervisor termination disconnects guards and removes owned descendants', async (t) => {
  const directory = await fixtureDirectory(t, 'saas-dev-parent-exit-')
  const pidFile = path.join(directory, 'child.pid')
  const parent = spawn(process.execPath, [path.resolve('tests/fixtures/dev-supervisor.mjs')], {
    env: { ...process.env, FIXTURE_PORT: String(await freePort()), FIXTURE_PID_FILE: pidFile },
    stdio: ['ignore', 'ignore', 'pipe', 'ipc'], windowsHide: true,
  })
  const exited = ownChild(t, parent)
  let output = ''
  parent.stderr.on('data', (chunk) => { output += chunk })
  const readiness = await Promise.race([
    once(parent, 'message'),
    once(parent, 'exit').then(() => { throw new Error(`fixture supervisor exited early: ${output}`) }),
    delay(5000).then(() => { throw new Error('fixture supervisor readiness timeout') }),
  ])
  const childPid = Number(await waitFile(pidFile))
  parent.kill('SIGKILL')
  await exited
  for (const pid of [...readiness[0].pids, childPid]) await gone(pid)
})

test('invalid CLI input exits nonzero and does not echo untrusted credential text', async (t) => {
  const child = spawn(process.execPath, [path.resolve('scripts/dev.mjs'), '--host', 'http://fixture:private@invalid'], {
    stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true,
  })
  ownChild(t, child)
  let output = ''
  child.stderr.on('data', (chunk) => { output += chunk })
  const [code] = await once(child, 'exit')
  assert.equal(code, 1)
  assert.match(output, /--host must be/)
  assert.doesNotMatch(output, /fixture:private/)
})
