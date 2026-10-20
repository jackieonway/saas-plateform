import { spawn } from 'node:child_process'
import { writeFile } from 'node:fs/promises'
import { createServer } from 'node:http'
import { createServer as createTcpServer } from 'node:net'

const mode = process.argv[2]
if (mode === 'exit') process.exit(23)
if (mode === 'descendant-listener') {
  const server = createServer((_, response) => response.end('owned-descendant'))
  server.listen(0, process.env.DEV_SERVICE_HOST, () => process.send({ pid: process.pid, port: server.address().port }))
}
else if (mode === 'hang' || mode === 'descendant-child') setInterval(() => {}, 1000)
else if (mode === 'slow-headers') {
  createTcpServer((socket) => {
    socket.on('data', () => {})
    socket.on('error', () => {})
    socket.write('HTTP/1.1 200 OK\r\nX-Slow: ')
    const drip = setInterval(() => {
      socket.write('x')
      console.log('slow-header drip')
    }, 40)
    socket.once('close', () => clearInterval(drip))
  }).listen(Number(process.env.DEV_SERVICE_PORT), process.env.DEV_SERVICE_HOST)
}
else {
  if (mode === 'descendant-listeners') {
    const listeners = await Promise.all(Array.from({ length: 12 }, () => new Promise((resolve, reject) => {
      const child = spawn(process.execPath, [process.argv[1], 'descendant-listener'], { stdio: ['ignore', 'ignore', 'ignore', 'ipc'] })
      child.once('message', resolve)
      child.once('error', reject)
      child.once('exit', () => reject(new Error('owned descendant exited before readiness')))
    })))
    await writeFile(process.env.FIXTURE_PID_FILE, JSON.stringify(listeners))
  }
  if (mode === 'descendant') {
    const child = spawn(process.execPath, [process.argv[1], 'descendant-child'], { stdio: 'ignore' })
    await writeFile(process.env.FIXTURE_PID_FILE, String(child.pid))
  }
  if (mode === 'log-secrets') {
    console.log(process.env.DATABASE_URL)
    console.log('postgresql://user:pass@127.0.0.1/fixture')
    console.log('http://localhost/callback?access_token=query-secret&ok=1')
  }
  createServer((_, response) => response.end('ready')).listen(Number(process.env.DEV_SERVICE_PORT), process.env.DEV_SERVICE_HOST)
  if (mode === 'later-exit') setTimeout(() => process.exit(24), 700)
}
