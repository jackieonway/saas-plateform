import { spawn, spawnSync } from 'node:child_process'

// One IPC-connected guard per owned service. If the supervisor disappears,
// disconnect fires even on Windows, where console signals are insufficient.
let service
let stopping = false
function stop() {
  if (stopping) return
  stopping = true
  if (process.platform === 'win32') {
    // The root PID is this guard, never a PID discovered by scanning a port.
    spawnSync('taskkill.exe', ['/PID', String(process.pid), '/T', '/F'], { stdio: 'ignore', windowsHide: true, timeout: 5000 })
  } else {
    // The supervisor creates this guard as its own POSIX process-group leader.
    try { process.kill(-process.pid, 'SIGKILL') } catch { /* Already exited. */ }
  }
  if (service) { try { service.kill('SIGKILL') } catch { /* Already exited. */ } }
  process.exit(0)
}

function report(message) {
  if (process.connected) process.send(message, () => {})
  else stop()
}
process.once('disconnect', stop)
process.once('SIGTERM', stop)
process.once('SIGINT', stop)
process.on('message', (message) => {
  if (message?.type === 'stop') { stop(); return }
  if (service) return
  try {
    service = spawn(message.command, message.args, {
      cwd: message.cwd, env: process.env, stdio: ['ignore', 'inherit', 'inherit'], windowsHide: true,
    })
    service.once('error', () => report({ type: 'error' }))
    service.once('exit', (code) => report({ type: 'exit', code }))
  } catch { report({ type: 'error' }) }
})
