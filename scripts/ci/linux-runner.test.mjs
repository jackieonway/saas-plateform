import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync, rmSync } from 'node:fs'
import { EventEmitter } from 'node:events'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { createSnapshot, startPostgres, validateGateSummary, validateToolContainerSecurity, runLinux, validateOutputLocation, onceCleanup } from './linux-runner.mjs'

test('clean runner snapshot excludes developer environment and dependency caches', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-snapshot-'))
  try {
    const source = path.join(directory, 'source'); mkdirSync(path.join(source, 'frontend', 'node_modules'), { recursive: true })
    writeFileSync(path.join(source, 'frontend', 'package.json'), '{}')
    writeFileSync(path.join(source, 'frontend', '.env'), 'private')
    writeFileSync(path.join(source, 'frontend', 'node_modules', 'cache'), 'private')
    const vendor = path.join(source, 'business-apps/work-orders/frontend/vendor'); mkdirSync(vendor, { recursive: true })
    writeFileSync(path.join(vendor, 'iframe-bridge.js'), 'registered classic SDK')
    writeFileSync(path.join(vendor, 'micro-sdk.mjs'), 'registered ESM SDK')
    writeFileSync(path.join(vendor, 'unrelated.js'), 'public frontend dependency retained in the source snapshot')
    const destination = path.join(directory, 'snapshot'), result = createSnapshot(destination, source)
    assert.deepEqual(result.files.map(file => file.path), ['business-apps/work-orders/frontend/vendor/iframe-bridge.js', 'business-apps/work-orders/frontend/vendor/micro-sdk.mjs', 'business-apps/work-orders/frontend/vendor/unrelated.js', 'frontend/package.json'])
    assert.equal(existsSync(path.join(destination, 'frontend', '.env')), false)
    assert.equal(existsSync(path.join(destination, 'frontend', 'node_modules')), false)
    assert.throws(() => createSnapshot(destination, source), /existing/)
  } finally { rmSync(directory, { recursive: true, force: true }) }
})

test('Linux output rejects source ancestors and recursive source destinations before creating directories', async () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-output-'))
  try {
    const sourceRoot = path.join(directory, 'source'); mkdirSync(sourceRoot)
    for (const outDir of [sourceRoot, directory, path.join(sourceRoot, 'scripts', 'ci', 'new-output'), path.join(sourceRoot, 'frontend', 'new-output')]) {
      const existed = existsSync(outDir)
      await assert.rejects(runLinux({ gate: 'fast', outDir, sourceRoot }), /must not contain the repository|included source directory/)
      assert.equal(existsSync(outDir), existed)
    }
    validateOutputLocation(path.join(sourceRoot, 'target', 'new-output'), sourceRoot)
    validateOutputLocation(path.join(directory, 'outside-project'), sourceRoot)
    assert.throws(() => createSnapshot(path.join(sourceRoot, 'scripts', 'snapshot'), sourceRoot), /included source directory/)
    assert.equal(existsSync(path.join(sourceRoot, 'scripts')), false)
  } finally { rmSync(directory, { recursive: true, force: true }) }
})

test('PostgreSQL startup failure removes only the label-verified partially created container', async () => {
  const owner = 'ci-12345678', calls = []
  const docker = async args => {
    calls.push(args)
    if (args[0] === 'image') return JSON.stringify([{ Id: 'sha256:synthetic' }])
    if (args[0] === 'run') throw new Error('controlled startup failure')
    if (args[0] === 'container') return JSON.stringify([{ Config: { Labels: { 'saas.ci.owner': owner } } }])
    if (args[0] === 'rm') return ''
    throw new Error('unexpected command')
  }
  await assert.rejects(startPostgres({ docker, owner, network: `${owner}-network` }), /startup failure/)
  assert.deepEqual(calls.at(-1), ['rm', '--force', `${owner}-postgres`])
})

test('public npm configuration is validated before it can enter a snapshot', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-npmrc-'))
  try {
    const source = path.join(directory, 'source'); mkdirSync(source)
    writeFileSync(path.join(source, '.npmrc'), '//registry.npmjs.org/:_authToken=synthetic-test-value\n')
    const destination = path.join(directory, 'snapshot')
    assert.throws(() => createSnapshot(destination, source), /Only public engine-strict/)
    assert.equal(existsSync(path.join(destination, '.npmrc')), false)
  } finally { rmSync(directory, { recursive: true, force: true }) }
})

test('PostgreSQL startup cleanup refuses a container with another owner', async () => {
  const calls = [], owner = 'ci-12345678'
  const docker = async args => {
    calls.push(args)
    if (args[0] === 'image') return JSON.stringify([{ Id: 'sha256:synthetic' }])
    if (args[0] === 'run') throw new Error('controlled startup failure')
    return JSON.stringify([{ Config: { Labels: { 'saas.ci.owner': 'another-owner' } } }])
  }
  await assert.rejects(startPostgres({ docker, owner, network: `${owner}-network` }), /unowned/)
  assert.equal(calls.some(args => args[0] === 'rm'), false)
})

test('Linux launcher refuses successful process exit with an invalid mandatory summary', () => {
  const expected = { gate: 'postgres', sourceRevision: 'a'.repeat(40), inputsSha256: 'b'.repeat(64) }
  const valid = { ...expected, status: 'PASS', mandatoryFailureCount: 0, mandatorySkipCount: 0, steps: [{ mandatory: true, status: 'PASS' }] }
  validateGateSummary(valid, expected)
  for (const changed of [{ status: 'FAIL' }, { gate: 'fast' }, { sourceRevision: 'c'.repeat(40) }, { inputsSha256: 'd'.repeat(64) }, { mandatoryFailureCount: 1 }, { mandatorySkipCount: 1 }, { steps: [] }, { steps: [{ mandatory: true, status: 'SKIP' }] }]) assert.throws(() => validateGateSummary({ ...valid, ...changed }, expected))
})

test('browser startup provisions only its owned private PostgreSQL and cleans both containers on failure', async () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-browser-pg-'))
  try {
    const sourceRoot = path.join(directory, 'source'); mkdirSync(sourceRoot); writeFileSync(path.join(sourceRoot, 'package.json'), '{}')
    const containers = new Map(), networks = new Map(), calls = []
    let owner, runnerCreate
    const labels = () => ({ 'saas.ci.owner': owner })
    const docker = async args => {
      calls.push(args)
      if (args[0] === 'version') return '29.1.3'
      if (args[0] === 'image') return JSON.stringify([{ Id: 'sha256:synthetic', Architecture: 'amd64', Os: 'linux' }])
      if (args[0] === 'network' && args[1] === 'create') { owner = args[args.indexOf('--label') + 1].split('=')[1]; networks.set(args.at(-1), { Labels: labels() }); return 'owned-network' }
      if (args[0] === 'run' || args[0] === 'create') {
        containers.set(args[args.indexOf('--name') + 1], { Config: { Labels: labels() }, NetworkSettings: { Ports: { '5432/tcp': null } } })
        if (args[0] === 'create') { runnerCreate = args; throw new Error('controlled browser startup failure') }
        return 'owned-postgres'
      }
      if (args[0] === 'exec') return args.includes('pg_isready') ? 'accepting connections' : 'postgres (PostgreSQL) 17.6'
      if (args[0] === 'container') return containers.has(args[2]) ? JSON.stringify([containers.get(args[2])]) : 'No such container'
      if (args[0] === 'network' && args[1] === 'inspect') return JSON.stringify([networks.get(args[2])])
      if (args[0] === 'rm') { containers.delete(args.at(-1)); return '' }
      if (args[0] === 'network' && args[1] === 'rm') { networks.delete(args.at(-1)); return '' }
      throw new Error('unexpected browser startup operation')
    }
    await assert.rejects(runLinux({ gate: 'browser', browser: 'chrome', outDir: path.join(directory, 'output'), sourceRoot, sourceRevision: 'a'.repeat(40), docker }), /controlled browser startup failure/)
    assert.ok(runnerCreate.includes(`CI_TEST_POSTGRES_OWNER=${owner}`))
    assert.ok(runnerCreate.includes('CI_MANAGED_BROWSER_SOURCE_ROOT=/workspace'))
    assert.deepEqual(runnerCreate.flatMap((value, index) => value === '--cap-add' ? [runnerCreate[index + 1]] : []), ['CHOWN', 'SETUID', 'SETGID'])
    assert.equal(runnerCreate[runnerCreate.indexOf('--cap-drop') + 1], 'ALL')
    assert.equal(runnerCreate[runnerCreate.indexOf('--security-opt') + 1], 'no-new-privileges')
    validateToolContainerSecurity(runnerCreate, { gate: 'browser' })
    assert.ok(runnerCreate.some(value => value.startsWith(`TEST_DATABASE_URL=postgresql://ci:`) && value.includes(`@${owner}-postgres:5432/postgres`)))
    const postgres = calls.find(args => args[0] === 'run')
    assert.equal(postgres[postgres.indexOf('--network') + 1], `${owner}-network`)
    assert.ok(postgres.includes('--tmpfs'))
    assert.equal(calls.some(args => args.includes('--publish') || args.includes('-p')), false)
    assert.equal(containers.size, 0); assert.equal(networks.size, 0)
  } finally { rmSync(directory, { recursive: true, force: true }) }
})

test('tool capability policy rejects broad permissions and confines Nginx worker identity operations to browser', () => {
  const base = ['--cap-drop', 'ALL', '--security-opt', 'no-new-privileges']
  const browser = ['--cap-drop', 'ALL', '--cap-add', 'CHOWN', '--cap-add', 'SETUID', '--cap-add', 'SETGID', '--security-opt', 'no-new-privileges']
  validateToolContainerSecurity(browser, { gate: 'browser' })
  for (const gate of ['fast', 'postgres', 'delivery']) {
    validateToolContainerSecurity(base, { gate })
    assert.throws(() => validateToolContainerSecurity(browser, { gate }))
    for (const capability of ['CHOWN', 'SETUID', 'SETGID', 'ALL', 'SYS_ADMIN']) assert.throws(() => validateToolContainerSecurity([...base, '--cap-add', capability], { gate }))
  }
  for (const invalid of [
    base, browser.slice(2), browser.slice(0, -2), browser.filter(value => value !== 'SETGID'),
    [...browser, '--cap-add', 'NET_ADMIN'], [...browser, '--cap-add=ALL'], [...browser, '--privileged'], [...browser, '--privileged=true'],
    [...browser, '--security-opt', 'seccomp=unconfined'], browser.map(value => value === 'no-new-privileges' ? 'no-new-privileges=false' : value),
  ]) assert.throws(() => validateToolContainerSecurity(invalid, { gate: 'browser' }))
})

test('fast and PostgreSQL actual Docker create arguments never gain the browser Nginx capabilities', async () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-tool-caps-'))
  try {
    const sourceRoot = path.join(directory, 'source'); mkdirSync(sourceRoot); writeFileSync(path.join(sourceRoot, 'package.json'), '{}')
    for (const gate of ['fast', 'postgres']) {
      const containers = new Map(), networks = new Map()
      let owner, created
      const docker = async args => {
        if (args[0] === 'version') return '29.1.3'
        if (args[0] === 'image') return JSON.stringify([{ Id: 'sha256:synthetic', Architecture: 'amd64', Os: 'linux' }])
        const labels = () => ({ 'saas.ci.owner': owner })
        if (args[0] === 'network' && args[1] === 'create') { owner = args[args.indexOf('--label') + 1].split('=')[1]; networks.set(args.at(-1), { Labels: labels() }); return '' }
        if (args[0] === 'run' || args[0] === 'create') {
          containers.set(args[args.indexOf('--name') + 1], { Config: { Labels: labels() }, NetworkSettings: { Ports: { '5432/tcp': null } } })
          if (args[0] === 'create') { created = args; throw new Error('controlled tool startup failure') }
          return ''
        }
        if (args[0] === 'exec') return args.includes('pg_isready') ? 'accepting connections' : 'postgres (PostgreSQL) 17.6'
        if (args[0] === 'container') return JSON.stringify([containers.get(args[2])])
        if (args[0] === 'network' && args[1] === 'inspect') return JSON.stringify([networks.get(args[2])])
        if (args[0] === 'rm') { containers.delete(args.at(-1)); return '' }
        if (args[0] === 'network' && args[1] === 'rm') { networks.delete(args.at(-1)); return '' }
        throw new Error('unexpected tool policy operation')
      }
      await assert.rejects(runLinux({ gate, outDir: path.join(directory, gate), sourceRoot, sourceRevision: 'a'.repeat(40), docker }), /controlled tool startup failure/)
      validateToolContainerSecurity(created, { gate })
      assert.equal(created.includes('--cap-add'), false)
      assert.equal(created.includes('CI_MANAGED_BROWSER_SOURCE_ROOT=/workspace'), false)
      assert.equal(created[created.indexOf('--cap-drop') + 1], 'ALL')
      assert.equal(created[created.indexOf('--security-opt') + 1], 'no-new-privileges')
      const report = JSON.parse(readFileSync(path.join(directory, gate, 'linux-runner-result.json'), 'utf8'))
      assert.deepEqual(report.requestedToolSecurity, { capDrop: ['ALL'], capAdd: [], securityOpt: ['no-new-privileges'] })
      assert.equal(containers.size, 0); assert.equal(networks.size, 0)
    }
  } finally { assert.equal(path.dirname(directory), path.resolve(tmpdir())); assert.match(path.basename(directory), /^ci-tool-caps-/); rmSync(directory, { recursive: true, force: true }) }
})

test('concurrent cleanup callers await the same pending resource removal', async () => {
  let release, entered = 0, completed = 0
  const pendingRemoval = new Promise(resolve => { release = resolve })
  const cleanup = onceCleanup(async () => { entered++; await pendingRemoval; completed++ })
  const normalFinally = cleanup()
  await Promise.resolve()
  const signalHandler = cleanup()
  assert.equal(signalHandler, normalFinally)
  assert.equal(entered, 1); assert.equal(completed, 0)
  let signalCanExit = false
  signalHandler.then(() => { signalCanExit = true })
  await Promise.resolve()
  assert.equal(signalCanExit, false)
  release()
  await Promise.all([normalFinally, signalHandler])
  assert.equal(completed, 1); assert.equal(signalCanExit, true)
  assert.equal(cleanup(), normalFinally)
})

test('signal cancellation waits for in-flight Docker operations and then removes late-created owned resources', async t => {
  for (const stage of ['network', 'runner', 'postgres', 'report-copy', 'attach', 'attach-created']) await t.test(stage, { timeout: 2000 }, async () => {
    const directory = mkdtempSync(path.join(tmpdir(), 'ci-cancel-'))
    try {
      const sourceRoot = path.join(directory, 'source'); mkdirSync(sourceRoot); writeFileSync(path.join(sourceRoot, 'package.json'), '{}')
      const outDir = path.join(directory, 'report'), signals = new EventEmitter(), containers = new Map(), networks = new Map(), calls = []
      let release, reached, owner
      const barrier = new Promise(resolve => { release = resolve }), entered = new Promise(resolve => { reached = resolve })
      const pause = async current => { if (current === stage) { reached(); await barrier } }
      const owned = () => ({ 'saas.ci.owner': owner })
      const docker = async args => {
        calls.push(args)
        if (args[0] === 'version') return '29.1.3'
        if (args[0] === 'image') return JSON.stringify([{ Id: `sha256:${'a'.repeat(64)}`, Architecture: 'amd64', Os: 'linux', RepoDigests: [] }])
        if (args[0] === 'network' && args[1] === 'create') { owner = args[args.indexOf('--label')+1].split('=')[1]; await pause('network'); networks.set(args.at(-1), { Labels: owned() }); return 'network-id' }
        if (args[0] === 'create' || args[0] === 'run') { await pause(args[0] === 'run' ? 'postgres' : 'runner'); containers.set(args[args.indexOf('--name')+1], { Config: { Labels: owned() }, State: { Running: false, ExitCode: 0 }, NetworkSettings: { Ports: { '5432/tcp': null } } }); return 'container-id' }
        if (args[0] === 'container' && args[1] === 'inspect') return containers.has(args[2]) ? JSON.stringify([containers.get(args[2])]) : 'Error response from daemon: No such container'
        if (args[0] === 'network' && args[1] === 'inspect') return networks.has(args[2]) ? JSON.stringify([networks.get(args[2])]) : 'Error response from daemon: No such network'
        if (args[0] === 'rm') { containers.delete(args.at(-1)); if (stage.startsWith('attach')) release(); return '' }
        if (args[0] === 'network' && args[1] === 'rm') { networks.delete(args.at(-1)); return '' }
        if (args[0] === 'cp') { if (args[1].includes(':/reports/')) await pause('report-copy'); return '' }
        if (args[0] === 'start') { if (stage.startsWith('attach')) { containers.get(args.at(-1)).State.Running = stage === 'attach'; await pause(stage) } return 'controlled completed gate' }
        throw new Error(`Unexpected Docker execution: ${args[0]}`)
      }
      const running = runLinux({ gate: stage === 'postgres' ? 'postgres' : 'fast', outDir, sourceRoot, sourceRevision: 'a'.repeat(40), docker, signalEmitter: signals })
      const rejected = assert.rejects(running, /cancelled by signal/)
      await entered
      signals.emit('SIGTERM')
      await Promise.resolve()
      if (!stage.startsWith('attach')) assert.equal(calls.some(args => args[0] === 'rm' || (args[0] === 'network' && args[1] === 'rm')), false, 'cleanup must await the pending create/copy operation')
      if (!stage.startsWith('attach')) release()
      await rejected
      const report = JSON.parse(readFileSync(path.join(outDir, 'linux-runner-result.json'), 'utf8'))
      assert.equal(report.status, 'FAIL'); assert.equal(report.cancelled, true); assert.deepEqual(report.cleanupErrors, [])
      assert.equal(containers.size, 0); assert.equal(networks.size, 0)
      if (stage !== 'report-copy' && !stage.startsWith('attach')) assert.equal(calls.some(args => args[0] === 'start' || args[0] === 'exec' || args[0] === 'cp'), false, 'cancellation must prevent subsequent execution')
      if (stage.startsWith('attach')) {
        const removeIndex = calls.findIndex(args => args[0] === 'rm')
        assert(removeIndex > 0, 'cancellation must remove the owned runner without waiting for a pending start or natural completion')
        assert.deepEqual(calls[removeIndex-1], ['container', 'inspect', `${owner}-runner`])
        assert.deepEqual(calls[removeIndex], ['rm', '--force', `${owner}-runner`])
        assert.equal(calls.filter(args => args[0] === 'rm').length, 1)
      }
      assert.equal(signals.listenerCount('SIGINT'), 0); assert.equal(signals.listenerCount('SIGTERM'), 0)
    } finally { rmSync(directory, { recursive: true, force: true }) }
  })
})
