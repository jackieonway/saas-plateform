import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export const repository = fileURLToPath(new URL('../../', import.meta.url))
export const allowedOrigin = 'http://127.0.0.1:3000'

export async function until(predicate, description, timeout = 3000) {
  const deadline = performance.now() + timeout
  while (performance.now() < deadline) {
    if (await predicate()) return
    await new Promise(resolve => setTimeout(resolve, 10))
  }
  assert.fail(`Timed out: ${description}`)
}

export function event(emitter, name, timeout = 3000) {
  return new Promise((resolve, reject) => {
    const onEvent = (...args) => { cleanup(); resolve(args) }
    const timer = setTimeout(() => { cleanup(); reject(new Error(`Timed out waiting for ${name}`)) }, timeout)
    const cleanup = () => { clearTimeout(timer); emitter.off(name, onEvent) }
    emitter.on(name, onEvent)
  })
}

// Absence assertions need an observation window; positive assertions always wait on events/state.
export async function observeSilence(emitter, name, duration = 150) {
  const received = []
  const listener = (...args) => received.push(args)
  emitter.on(name, listener)
  try {
    await new Promise(resolve => setTimeout(resolve, duration))
    assert.deepEqual(received, [], `Unexpected ${name} during ${duration}ms observation`)
  } finally {
    emitter.off(name, listener)
  }
}

async function build(manifest, targetDirectory) {
  const child = spawn('cargo', ['build', '--locked', '--manifest-path', manifest, '--target-dir', targetDirectory], {
    cwd: repository, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true,
  })
  let output = ''
  child.stdout.on('data', chunk => { output = (output + chunk).slice(-16000) })
  child.stderr.on('data', chunk => { output = (output + chunk).slice(-16000) })
  const timer = setTimeout(() => child.kill(), 300_000)
  try {
    const [code, signal] = await once(child, 'exit')
    assert.equal(code, 0, `Rust fixture build failed (${signal ?? code}):\n${output}`)
  } finally { clearTimeout(timer) }
}

export async function startRustFixture({ manifest, binary, env, target = 'phase22-socketio' }) {
  const targetDirectory = resolve(repository, 'target', target)
  await build(manifest, targetDirectory)
  const executable = resolve(targetDirectory, 'debug', binary + (process.platform === 'win32' ? '.exe' : ''))
  const child = spawn(executable, [], {
    cwd: repository, env: { ...process.env, ...env }, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true,
  })
  let output = '', pending = ''
  const exit = once(child, 'exit')
  let stopped = false
  const stop = async () => {
    if (stopped) return
    stopped = true
    child.stdin.end()
    const timer = setTimeout(() => child.kill(), 5000)
    try {
      const [code, signal] = await exit
      assert.equal(code, 0, `Rust fixture did not exit cleanly (${signal ?? code}):\n${output}`)
    } finally { clearTimeout(timer) }
  }
  try {
    const ready = await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`Rust fixture readiness timeout:\n${output}`)), 15_000)
      child.on('error', error => { clearTimeout(timer); reject(error) })
      child.on('exit', code => { clearTimeout(timer); reject(new Error(`Rust fixture exited before ready (${code}):\n${output}`)) })
      child.stderr.on('data', chunk => { output = (output + chunk).slice(-16000) })
      child.stdout.on('data', chunk => {
        output = (output + chunk).slice(-16000)
        pending += chunk
        while (pending.includes('\n')) {
          const end = pending.indexOf('\n'), line = pending.slice(0, end)
          pending = pending.slice(end + 1)
          try {
            const message = JSON.parse(line)
            if (message.event === 'ready' && Number.isInteger(message.port) && message.port > 0) {
              clearTimeout(timer); resolve(message)
            }
          } catch { /* Rust tracing output is diagnostic data, never a ready signal. */ }
        }
      })
    })
    return { child, port: ready.port, base: `http://127.0.0.1:${ready.port}`, stop }
  } catch (error) {
    await stop().catch(() => {})
    throw error
  }
}
