// Real Vite HTTP/build checks. Resolve Vite only from this independently installed project.
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const os = require('node:os')
const path = require('node:path')
const { pathToFileURL } = require('node:url')
const { test } = require('node:test')

const root = __dirname
const appName = path.basename(root)
const vendor = appName === 'micro-vue' ? 'vendor/vue.global.prod.js' : 'vendor/jquery.min.js'
const prefix = `/sub-apps/${appName}/versions/build-test/`

async function assertEntry(baseUrl, built) {
  const response = await fetch(new URL('index.html', baseUrl))
  assert.equal(response.status, 200)
  const html = await response.text()
  const scripts = [...html.matchAll(/<script\b([^>]*)\bsrc="([^"]+)"[^>]*>/g)]
  assert.equal(scripts.length, 3, 'only the registered SDK and classic lifecycle scripts are served')
  assert.ok(!html.includes('type="module"'), 'host HTML loaders must not evaluate Vite client modules')
  assert.deepEqual(scripts.map(match => match[2]), [vendor, 'vendor/micro-sdk.js', 'app.js'])
  for (const [, , src] of scripts) {
    assert.ok(!src.startsWith('/') && !src.includes('://'), 'asset URL remains relative to its version entry')
    const assetUrl = new URL(src, new URL('index.html', baseUrl))
    assert.ok(assetUrl.pathname.startsWith(new URL(baseUrl).pathname))
    const asset = await fetch(assetUrl)
    assert.equal(asset.status, 200, assetUrl.href)
    assert.match(asset.headers.get('content-type'), /javascript/)
    assert.equal(await asset.text(), await fs.readFile(path.join(root, 'public', src), 'utf8'))
  }
  for (const missing of ['missing.js', 'vendor/missing.js', 'missing.css', 'missing.png', 'missing.html', 'missing-route']) {
    const result = await fetch(new URL(missing, baseUrl), { headers: { Accept: 'text/html' } })
    assert.equal(result.status, 404, `${built ? 'preview' : 'dev'} ${missing} must not use an SPA fallback`)
    assert.ok(!(await result.text()).includes('micro-app-entry'))
  }
}

test('independent Vite dev and version-prefixed production preview keep classic assets and real 404s', { timeout: 120000 }, async () => {
  const localVite = path.join(root, 'node_modules/vite/dist/node/index.js')
  await fs.access(localVite)
  const { build, createServer, preview } = await import(pathToFileURL(localVite).href)
  const configFile = path.join(root, 'vite.config.mjs')
  const dev = await createServer({ configFile, server: { host: '127.0.0.1', port: 0, strictPort: true } })
  try {
    await dev.listen()
    await assertEntry(`http://127.0.0.1:${dev.httpServer.address().port}/`, false)
  } finally {
    await dev.close()
  }
  const tempBase = await fs.realpath(os.tmpdir())
  const temporaryRoot = await fs.realpath(await fs.mkdtemp(path.join(tempBase, `${appName}-vite-`)))
  const outDir = path.join(temporaryRoot, 'dist')
  assert.equal(path.dirname(path.resolve(outDir)), temporaryRoot)
  let server
  try {
    await build({ configFile, build: { outDir, emptyOutDir: true } })
    assert.equal(await fs.readFile(path.join(outDir, 'app.js'), 'utf8'), await fs.readFile(path.join(root, 'public/app.js'), 'utf8'))
    server = await preview({ configFile, base: prefix, build: { outDir }, preview: { host: '127.0.0.1', port: 0, strictPort: true } })
    await assertEntry(`http://127.0.0.1:${server.httpServer.address().port}${prefix}`, true)
  } finally {
    if (server) {
      server.httpServer.closeAllConnections()
      await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve()))
    }
    assert.equal(path.dirname(temporaryRoot), tempBase)
    assert.ok(path.basename(temporaryRoot).startsWith(`${appName}-vite-`))
    assert.ok(!(await fs.lstat(temporaryRoot)).isSymbolicLink())
    assert.equal(await fs.realpath(temporaryRoot), temporaryRoot)
    await fs.rm(temporaryRoot, { recursive: true, force: true })
  }
})
