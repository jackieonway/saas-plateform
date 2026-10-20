import assert from 'node:assert/strict'
import test from 'node:test'
import { createServer } from 'node:net'
import { createServicePlan, parseArgs, supervise } from '../scripts/dev.mjs'

async function freePort() {
  const server = createServer()
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '::1', resolve)
  })
  const port = server.address().port
  await new Promise((resolve) => server.close(resolve))
  return port
}

test('supervisor starts four real Vite projects, routes custom proxy ports and stops owned listeners', { timeout: 60000 }, async (t) => {
  const names = ['host', 'vue', 'jquery', 'websocket']
  const ports = Object.fromEntries(await Promise.all(names.map(async (name) => [name, await freePort()])))
  assert.equal(new Set(Object.values(ports)).size, names.length)
  const options = parseArgs(['--apps', names.join(','), '--host', '::1',
    ...names.flatMap((name) => [`--${name}-port`, String(ports[name])])])
  const plan = createServicePlan(options)
  assert.deepEqual(plan.map(({ name }) => name), names)
  const controller = new AbortController()
  const logs = []
  let ready
  const readiness = new Promise((resolve) => { ready = resolve })
  const result = supervise(plan, { signal: controller.signal, timeoutMs: 30000, log: (line) => logs.push(line), onReady: ready })
  t.after(async () => { controller.abort(); await result })
  await Promise.race([readiness, result.then((code) => { throw new Error(`Vite startup exited ${code}: ${logs.join('\n')}`) })])
  const origin = `http://[::1]:${ports.host}`
  const home = await fetch(origin)
  assert.equal(home.status, 200)
  await home.text()
  for (const [alias, vendor] of [['vue', 'vue.global.prod.js'], ['jquery', 'jquery.min.js']]) {
    const entry = await fetch(`${origin}/sub/${alias}/`)
    assert.equal(entry.status, 200)
    const html = await entry.text()
    assert.doesNotMatch(html, /\/@vite\/client|type="module"/)
    for (const script of [...html.matchAll(/<script\s+src="([^"]+)"/g)].map((match) => match[1])) {
      assert.ok(!script.startsWith('/'))
      const asset = await fetch(new URL(script, `${origin}/sub/${alias}/`))
      assert.equal(asset.status, 200)
      assert.match(asset.headers.get('content-type'), /javascript/)
      // Drain bodies before killing the fixture servers: large vendor responses
      // otherwise leave Undici paused while their owned sockets are torn down.
      assert.ok((await asset.arrayBuffer()).byteLength > 0)
    }
    assert.ok(html.includes(`vendor/${vendor}`))
    const missing = await fetch(`${origin}/sub/${alias}/missing.js`, { headers: { Accept: 'text/html,*/*' } })
    assert.equal(missing.status, 404)
    await missing.text()
  }
  const chat = await fetch(`http://[::1]:${ports.websocket}`)
  assert.equal(chat.status, 200)
  assert.match(await chat.text(), /WebSocket Demo/)
  controller.abort()
  assert.equal(await result, 0)
  for (const port of Object.values(ports)) {
    const server = createServer()
    await new Promise((resolve, reject) => {
      server.once('error', reject)
      server.listen(port, '::1', resolve)
    })
    await new Promise((resolve) => server.close(resolve))
  }
})
