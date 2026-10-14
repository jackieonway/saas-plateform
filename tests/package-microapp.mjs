import assert from 'node:assert/strict'
import test from 'node:test'
import { spawnSync } from 'node:child_process'
import {
  existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync,
  readdirSync, realpathSync, rmSync, symlinkSync, writeFileSync,
} from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const cli = path.join(workspace, 'scripts', 'package-microapp.mjs')
const tempBase = realpathSync(os.tmpdir())
const tempPrefix = 'platform-package-microapp-test-'
const ownedTempRoots = new Set()

function fixture(t) {
  const root = realpathSync(mkdtempSync(path.join(tempBase, tempPrefix)))
  ownedTempRoots.add(root)
  t.after(() => {
    const absolute = path.resolve(root)
    const relative = path.relative(tempBase, absolute)
    assert.ok(ownedTempRoots.has(absolute), 'cleanup must target a temp root owned by this test')
    assert.ok(relative && !relative.startsWith('..') && !path.isAbsolute(relative), 'cleanup must remain within os.tmpdir')
    assert.equal(path.dirname(absolute), tempBase, 'cleanup root must be an immediate child of the known temp directory')
    assert.ok(path.basename(absolute).startsWith(tempPrefix), 'cleanup must retain the test-specific temp prefix')
    assert.ok(!lstatSync(absolute).isSymbolicLink(), 'cleanup root must not be replaced by a link')
    rmSync(absolute, { recursive: true, force: true })
    ownedTempRoots.delete(absolute)
  })
  const input = path.join(root, 'dist')
  const outputRoot = path.join(root, 'packages')
  mkdirSync(input)
  return { root, input, outputRoot, app: 'example', environment: 'development', version: '1.0.0' }
}

function writeTree(root, files) {
  for (const [name, bytes] of Object.entries(files)) {
    const file = path.join(root, name)
    mkdirSync(path.dirname(file), { recursive: true })
    writeFileSync(file, bytes)
  }
}

function runCli(base, overrides = {}, extra = []) {
  const options = { ...base, ...overrides }
  const args = [
    '--app', options.app, '--environment', options.environment, '--version', options.version,
    '--input', options.input, '--output-root', options.outputRoot,
  ]
  for (const [name, value] of [
    ['--entry-type', options.entryType], ['--entry', options.entry], ['--health-url', options.healthUrl],
  ]) if (value !== undefined) args.push(name, value)
  const result = spawnSync(process.execPath, [cli, ...args, ...extra], {
    cwd: workspace, encoding: 'utf8', timeout: 10_000, windowsHide: true,
  })
  assert.ifError(result.error)
  assert.equal(result.signal, null, `CLI must exit normally: ${result.stderr}`)
  return result
}

function assertAccepted(result) {
  assert.equal(result.status, 0, `CLI should accept valid package:\n${result.stderr}\n${result.stdout}`)
}

function assertRejected(result) {
  assert.notEqual(result.status, 0, `CLI should reject this package:\n${result.stdout}`)
  assert.ok(`${result.stderr}${result.stdout}`.trim(), 'rejection should provide a diagnostic')
}

function versionDirectory(base) {
  return path.join(base.outputRoot, base.app, 'versions', base.version)
}

function listFiles(root, prefix = '') {
  return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
    const relative = prefix ? `${prefix}/${entry.name}` : entry.name
    return entry.isDirectory() ? listFiles(path.join(root, entry.name), relative) : [relative]
  }).sort()
}

function assertPackage(base, files, entry, entryType, healthUrl) {
  const destination = versionDirectory(base)
  assert.deepEqual(listFiles(destination), [...Object.keys(files), 'release.json'].sort())
  for (const [relative, bytes] of Object.entries(files)) {
    assert.deepEqual(readFileSync(path.join(destination, relative)), Buffer.from(bytes), `copy must preserve ${relative} byte for byte`)
  }
  const manifest = JSON.parse(readFileSync(path.join(destination, 'release.json'), 'utf8'))
  assert.deepEqual(Object.keys(manifest).sort(), ['environment', 'version', 'entryType', 'entryUrl', 'resources', ...(healthUrl ? ['healthUrl'] : [])].sort())
  assert.equal(manifest.environment, base.environment)
  assert.equal(manifest.version, base.version)
  assert.equal(manifest.entryType, entryType)
  const entryUrl = `/sub-apps/${base.app}/versions/${base.version}/${entry}`
  assert.equal(manifest.entryUrl, entryUrl)
  if (healthUrl) assert.equal(manifest.healthUrl, healthUrl)
  assert.ok(Array.isArray(manifest.resources))
  const expected = Object.keys(files).filter((file) => file !== entry).map((file) => ({
    pathname: `/sub-apps/${base.app}/versions/${base.version}/${file.split('/').map(encodeURIComponent).join('/')}`,
    kind: /\.(?:js|mjs|cjs)$/i.test(file) ? 'SCRIPT' : /\.css$/i.test(file) ? 'STYLE' : 'ASSET',
  })).sort((a, b) => a.pathname.localeCompare(b.pathname))
  const actual = manifest.resources.map((resource) => {
    assert.deepEqual(Object.keys(resource).sort(), ['kind', 'url'])
    assert.equal(typeof resource.url, 'string')
    assert.ok(!/^(?:[a-z][a-z\d+.-]*:|\/)/i.test(resource.url), 'manifest resource URLs must be relative to the entry directory')
    const resolved = new URL(resource.url, `https://package.test${entryUrl}`)
    assert.equal(resolved.origin, 'https://package.test')
    assert.equal(resolved.search, '')
    assert.equal(resolved.hash, '', 'resource filename punctuation must be URL encoded')
    return { pathname: resolved.pathname, kind: resource.kind }
  }).sort((a, b) => a.pathname.localeCompare(b.pathname))
  assert.deepEqual(actual, expected, 'every non-entry source file must occur once with the correct resource kind and resolved URL')
  return manifest
}

function linkForTest(t, target, link, directory = true) {
  try {
    symlinkSync(target, link, directory && process.platform === 'win32' ? 'junction' : directory ? 'dir' : 'file')
    assert.ok(lstatSync(link).isSymbolicLink())
    return true
  } catch (error) {
    if (process.platform === 'win32' && ['EPERM', 'EACCES', 'ENOTSUP'].includes(error.code)) {
      t.skip(`Windows cannot create this ${directory ? 'junction' : 'file symlink'}: ${error.code}`)
      return false
    }
    throw error
  }
}

test('HTML default entry produces an exact copy and an API-ready manifest without authorization metadata', (t) => {
  const base = fixture(t)
  const files = {
    'index.html': '<!doctype html><script src="./assets/main.a1.js"></script><link rel="stylesheet" href="./assets/main.b2.css">',
    'assets/main.a1.js': 'console.log("release 1.0.0")\n',
    'assets/main.b2.css': '.app { color: #123456; }\r\n',
    'assets/lazy.c3.mjs': 'export const ready = true\n',
    'assets/icon #一.bin': Buffer.from([0, 255, 1, 128, 10]),
    'nested/readme.txt': 'build output\n',
  }
  writeTree(base.input, files)
  const healthUrl = '/sub-apps/example/health.json'
  assertAccepted(runCli(base, { healthUrl }))
  assertPackage(base, files, 'index.html', 'HTML', healthUrl)
})

test('ESM entry default is entry.mjs and scripts and styles remain in the manifest', (t) => {
  const base = fixture(t)
  const files = { 'entry.mjs': 'export default {}\n', 'chunks/vendor.js': 'export const vendor = 1\n', 'assets/site.css': 'body{}' }
  writeTree(base.input, files)
  assertAccepted(runCli(base, { entryType: 'ESM' }))
  assertPackage(base, files, 'entry.mjs', 'ESM')
})

test('nested ESM entry resolves resources relative to the entry directory and preserves bytes', (t) => {
  const base = fixture(t)
  const files = {
    'modules/entry.mjs': 'export const mount = () => {}\n',
    'modules/chunks/lazy.js': 'export const lazy = 1\n',
    'assets/site.css': '.nested{display:block}\n',
    'images/icon.bin': Buffer.from([37, 0, 252, 13, 10]),
  }
  writeTree(base.input, files)
  assertAccepted(runCli(base, { entryType: 'ESM', entry: 'modules/entry.mjs' }))
  assertPackage(base, files, 'modules/entry.mjs', 'ESM')
})

test('production rejects an HTTP health check URL without publishing a version', (t) => {
  const base = { ...fixture(t), environment: 'production' }
  writeTree(base.input, { 'index.html': '<html>production release</html>' })
  assertRejected(runCli(base, { healthUrl: 'http://health.example.test/status' }))
  assert.ok(!existsSync(versionDirectory(base)))
  assert.ok(!existsSync(base.outputRoot), 'HTTP production health policy must fail before creating output directories')
})

test('production accepts an HTTPS health check URL and retains it in the API manifest', (t) => {
  const base = { ...fixture(t), environment: 'production' }
  const files = { 'index.html': '<html>production release</html>', 'assets/main.js': 'console.log("production")' }
  writeTree(base.input, files)
  const healthUrl = 'https://health.example.test/status'
  assertAccepted(runCli(base, { healthUrl }))
  assertPackage(base, files, 'index.html', 'HTML', healthUrl)
})

for (const targetType of ['directory', 'file']) {
  test(`existing version ${targetType} is rejected without changing existing bytes`, (t) => {
    const base = fixture(t)
    writeTree(base.input, { 'index.html': '<html>new</html>', 'main.js': 'new' })
    const destination = versionDirectory(base)
    mkdirSync(path.dirname(destination), { recursive: true })
    const sentinel = Buffer.from([0, 44, 255, 73])
    if (targetType === 'directory') writeTree(destination, { 'original.bin': sentinel })
    else writeFileSync(destination, sentinel)
    assertRejected(runCli(base))
    if (targetType === 'directory') {
      assert.deepEqual(listFiles(destination), ['original.bin'])
      assert.deepEqual(readFileSync(path.join(destination, 'original.bin')), sentinel)
    } else assert.deepEqual(readFileSync(destination), sentinel)
  })
}

test('48 resources fit the API limit and the 49th is rejected without a version directory', (t) => {
  const base = fixture(t)
  const files = { 'index.html': '<html>resource limit</html>' }
  for (let index = 0; index < 48; index++) files[`assets/chunk-${index}.js`] = `export const n = ${index}`
  writeTree(base.input, files)
  assertAccepted(runCli(base))
  assert.equal(assertPackage(base, files, 'index.html', 'HTML').resources.length, 48)
  writeTree(base.input, { 'assets/one-more.css': 'body{}' })
  const next = { ...base, version: '1.0.1' }
  assertRejected(runCli(next))
  assert.ok(!existsSync(versionDirectory(next)), 'over-limit release must not leave a publishable partial package')
})

for (const filename of ['.env', '.env.production', 'nested/.env.local']) {
  test(`sensitive ${filename} causes rejection before any version is published`, (t) => {
    const base = fixture(t)
    writeTree(base.input, { 'index.html': '<html>safe</html>', [filename]: 'FAKE_TEST_SECRET=do-not-package' })
    assertRejected(runCli(base))
    assert.ok(!existsSync(versionDirectory(base)))
    assert.equal(readFileSync(path.join(base.input, filename), 'utf8'), 'FAKE_TEST_SECRET=do-not-package')
  })
}

for (const [name, overrides] of [
  ['app traversal', { app: '../escape' }],
  ['version traversal', { version: '../escape' }],
  ['version dot traversal', { version: '1..0' }],
  ['entry traversal', { entry: '../outside.html' }],
  ['entry absolute path', { entry: '/outside.html' }],
  ['entry Windows traversal', { entry: '..\\outside.html' }],
  ['environment traversal', { environment: '../production' }],
  ['invalid environment', { environment: 'Production' }],
  ['invalid entry type', { entryType: 'IFRAME' }],
]) {
  test(`${name} is rejected and files outside the source remain untouched`, (t) => {
    const base = fixture(t)
    writeTree(base.input, { 'index.html': '<html>safe</html>' })
    const outside = path.join(base.root, 'outside.html')
    writeFileSync(outside, '<html>outside sentinel</html>')
    assertRejected(runCli(base, overrides))
    assert.equal(readFileSync(outside, 'utf8'), '<html>outside sentinel</html>')
    assert.ok(!existsSync(base.outputRoot), 'invalid CLI paths must be rejected before creating output directories')
  })
}

for (const flags of [['--version', '2.0.0'], ['--unknown', 'value']]) {
  test(`${flags[0] === '--unknown' ? 'unknown' : 'duplicate'} CLI flag is rejected`, (t) => {
    const base = fixture(t)
    writeTree(base.input, { 'index.html': '<html>safe</html>' })
    assertRejected(runCli(base, {}, flags))
    assert.ok(!existsSync(base.outputRoot))
  })
}

test('missing entry is rejected without writing a partial release', (t) => {
  const base = fixture(t)
  writeTree(base.input, { 'main.js': 'console.log(1)' })
  assertRejected(runCli(base))
  assert.ok(!existsSync(versionDirectory(base)))
})

test('output within source is rejected without modifying the input tree', (t) => {
  const base = fixture(t)
  writeTree(base.input, { 'index.html': '<html>safe</html>' })
  assertRejected(runCli(base, { outputRoot: path.join(base.input, 'nested-output') }))
  assert.deepEqual(listFiles(base.input), ['index.html'])
})

test('source within output is rejected without modifying the source', (t) => {
  const base = fixture(t)
  const outputRoot = path.join(base.root, 'outer-output')
  const input = path.join(outputRoot, 'dist')
  writeTree(input, { 'index.html': '<html>safe</html>' })
  assertRejected(runCli(base, { input, outputRoot }))
  assert.deepEqual(listFiles(outputRoot), ['dist/index.html'])
})

test('input root symlink or junction is rejected', (t) => {
  const base = fixture(t)
  writeTree(base.input, { 'index.html': '<html>safe</html>' })
  const inputLink = path.join(base.root, 'linked-dist')
  if (!linkForTest(t, base.input, inputLink)) return
  assertRejected(runCli(base, { input: inputLink }))
  assert.ok(!existsSync(base.outputRoot))
})

test('nested input directory symlink or junction is rejected instead of copying external files', (t) => {
  const base = fixture(t)
  writeTree(base.input, { 'index.html': '<html>safe</html>' })
  const target = path.join(base.root, 'external')
  writeTree(target, { 'external.js': 'external sentinel' })
  if (!linkForTest(t, target, path.join(base.input, 'linked-assets'))) return
  assertRejected(runCli(base))
  assert.ok(!existsSync(versionDirectory(base)))
  assert.equal(readFileSync(path.join(target, 'external.js'), 'utf8'), 'external sentinel')
})

test('input file symlink is rejected when the platform permits creating one', (t) => {
  const base = fixture(t)
  writeTree(base.input, { 'index.html': '<html>safe</html>' })
  const target = path.join(base.root, 'external.js')
  writeFileSync(target, 'external sentinel')
  if (!linkForTest(t, target, path.join(base.input, 'linked.js'), false)) return
  assertRejected(runCli(base))
  assert.ok(!existsSync(versionDirectory(base)))
  assert.equal(readFileSync(target, 'utf8'), 'external sentinel')
})

test('symlink or junction in an output ancestor is rejected before writing through it', (t) => {
  const base = fixture(t)
  writeTree(base.input, { 'index.html': '<html>safe</html>' })
  const external = path.join(base.root, 'external-output')
  writeTree(external, { 'sentinel.bin': Buffer.from([0, 1, 255]) })
  const linkedParent = path.join(base.root, 'output-link')
  if (!linkForTest(t, external, linkedParent)) return
  assertRejected(runCli(base, { outputRoot: path.join(linkedParent, 'packages') }))
  assert.deepEqual(listFiles(external), ['sentinel.bin'])
  assert.deepEqual(readFileSync(path.join(external, 'sentinel.bin')), Buffer.from([0, 1, 255]))
})
