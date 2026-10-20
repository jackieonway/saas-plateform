import assert from 'node:assert/strict'
import { copyFileSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import { assetInventory, finalizeBuild, immutableImage, readHistory, sha256, sourceHash, validateInventory } from '../finalize-build.mjs'
import { exportHistory } from '../prepare-history.mjs'

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const image = `registry.example.test/platform/host@sha256:${'a'.repeat(64)}`
const imageId = `sha256:${'f'.repeat(64)}`
const revision = 'b'.repeat(40)

function fixture(t) {
  const temporary = path.resolve(mkdtempSync(path.join(os.tmpdir(), 'platform-static-policy-')))
  t.after(() => {
    assert.equal(path.dirname(temporary), path.resolve(os.tmpdir()))
    assert.ok(path.basename(temporary).startsWith('platform-static-policy-'))
    assert.ok(!lstatSync(temporary).isSymbolicLink())
    rmSync(temporary, { recursive: true, force: true })
  })
  const source = path.join(temporary, 'source')
  const dist = path.join(temporary, 'dist')
  const history = path.join(temporary, 'history')
  write(source, 'package.json', '{"name":"fixture"}')
  write(source, 'src/main.ts', 'console.log("current");')
  write(dist, 'index.html', '<script src="/assets/current-12345678.js"></script>')
  write(dist, 'assets/current-12345678.js', 'console.log("current");')
  mkdirSync(history)
  return { temporary, source, dist, history }
}

function write(root, name, bytes) {
  const destination = path.join(root, name)
  mkdirSync(path.dirname(destination), { recursive: true })
  writeFileSync(destination, bytes)
}

function historyRelease(history, assets, overrides = {}) {
  for (const [name, bytes] of Object.entries(assets)) write(history, name, bytes)
  const manifest = { schemaVersion: 1, retainedReleases: [{ image, buildId: 'previous-01', sourceRevision: revision,
    sourceSha256: 'c'.repeat(64), metadataSha256: 'd'.repeat(64) }], assets: assetInventory(history), ...overrides }
  write(history, 'manifest.json', JSON.stringify(manifest))
  return manifest
}

test('first build records content provenance without claiming a missing Git revision', (t) => {
  const input = fixture(t)
  const metadata = finalizeBuild(input)
  assert.equal(metadata.sourceRevision, null)
  assert.match(metadata.sourceSha256, /^[a-f0-9]{64}$/)
  assert.equal(metadata.buildId, `host-${metadata.sourceSha256.slice(0, 20)}`)
  assert.equal(metadata.assets.length, 1)
  assert.deepEqual(metadata.currentAssets, metadata.assets)
  assert.deepEqual(metadata.retainedReleases, [])
  assert.deepEqual(JSON.parse(readFileSync(path.join(input.dist, 'release-metadata.json'))), metadata)
})

test('historical chunks remain at their original URL while current HTML stays current', (t) => {
  const input = fixture(t)
  const retained = historyRelease(input.history, { 'assets/old-abcdef12.js': 'export const old = true;', 'assets/old-faceabcd.woff2': Buffer.from([0, 255, 17]) })
  const metadata = finalizeBuild({ ...input, sourceRevision: revision, buildId: 'candidate-02' })
  assert.equal(readFileSync(path.join(input.dist, 'assets/old-abcdef12.js'), 'utf8'), 'export const old = true;')
  assert.deepEqual(readFileSync(path.join(input.dist, 'assets/old-faceabcd.woff2')), Buffer.from([0, 255, 17]))
  assert.match(readFileSync(path.join(input.dist, 'index.html'), 'utf8'), /current-12345678/)
  assert.equal(metadata.assets.length, 3)
  assert.deepEqual(metadata.currentAssets.map((asset) => asset.path), ['assets/current-12345678.js'])
  assert.ok(metadata.assets.some((asset) => asset.path === 'assets/old-abcdef12.js'))
  assert.ok(!metadata.currentAssets.some((asset) => asset.path === 'assets/old-abcdef12.js'))
  assert.equal(metadata.sourceRevision, revision)
  assert.deepEqual(metadata.retainedReleases, retained.retainedReleases)
})

test('an immutable URL collision fails before copying any retained file', (t) => {
  const input = fixture(t)
  historyRelease(input.history, { 'assets/a-old-aaaaaaaa.js': 'old', 'assets/current-12345678.js': 'changed bytes' })
  assert.throws(() => finalizeBuild(input), /Immutable asset collision/)
  assert.ok(!existsSync(path.join(input.dist, 'assets/a-old-aaaaaaaa.js')))
  assert.equal(readFileSync(path.join(input.dist, 'assets/current-12345678.js'), 'utf8'), 'console.log("current");')
})

test('identical asset URL and bytes can be retained without changing current files', (t) => {
  const input = fixture(t)
  historyRelease(input.history, { 'assets/current-12345678.js': 'console.log("current");' })
  assert.equal(finalizeBuild(input).assets.length, 1)
})

test('tampered historical bytes or extra files cannot enter the next image', (t) => {
  const input = fixture(t)
  historyRelease(input.history, { 'assets/old-abcdef12.js': 'original bytes' })
  write(input.history, 'assets/old-abcdef12.js', 'tampered bytes')
  assert.throws(() => finalizeBuild(input), /do not match the recorded inventory/)
  write(input.history, 'assets/old-abcdef12.js', 'original bytes')
  write(input.history, 'assets/unrecorded-abcdef12.js', 'unrecorded bytes')
  assert.throws(() => finalizeBuild(input), /do not match the recorded inventory/)
})

test('history assets without a manifest and mutable image references are rejected', (t) => {
  const input = fixture(t)
  write(input.history, 'assets/old-abcdef12.js', 'old')
  assert.throws(() => readHistory(input.history), /require manifest/)
  const manifest = historyRelease(input.history, {})
  manifest.retainedReleases[0].image = 'registry.example.test/platform/host:latest'
  write(input.history, 'manifest.json', JSON.stringify(manifest))
  assert.throws(() => readHistory(input.history), /immutable image/)
  assert.equal(immutableImage.test(image), true)
  assert.equal(immutableImage.test('host:latest'), false)
})

test('inventory rejects traversal and duplicate paths while ignoring JSON property order', (t) => {
  const input = fixture(t)
  const [asset] = assetInventory(input.dist)
  assert.doesNotThrow(() => validateInventory(input.dist, [{ bytes: asset.bytes, sha256: asset.sha256, path: asset.path }]))
  assert.throws(() => validateInventory(input.dist, [{ ...asset, path: 'assets/../../outside.js' }]), /Invalid/)
  assert.throws(() => validateInventory(input.dist, [{ ...asset, path: 'assets\\outside.js' }]), /Invalid/)
  assert.throws(() => validateInventory(input.dist, [asset, asset]), /duplicate/)
})

test('source provenance changes on content changes but is reproducible across build metadata', (t) => {
  const input = fixture(t)
  const initial = sourceHash(input.source)
  assert.equal(sourceHash(input.source), initial)
  write(input.source, 'src/main.ts', 'console.log("changed");')
  assert.notEqual(sourceHash(input.source), initial)
  assert.throws(() => finalizeBuild({ ...input, sourceRevision: 'unknown' }), /full Git revision/)
  assert.throws(() => finalizeBuild({ ...input, buildId: '../invalid' }), /invalid format/)
})

function fakeDocker(previous, calls, failAt) {
  return (_, args) => {
    calls.push(args)
    if (args[0] === failAt) return { status: 1, stdout: '', stderr: 'fixture failure' }
    if (args[0] === 'image' && args[1] === 'inspect') return { status: 0, stdout: `${imageId}\n`, stderr: '' }
    if (args[0] === 'create') return { status: 0, stdout: `${'e'.repeat(64)}\n`, stderr: '' }
    if (args[0] === 'cp') {
      const basename = args[1].split('/').at(-1)
      const target = args[2]
      if (basename === 'assets') {
        for (const asset of previous.assets) write(target, asset.path, readFileSync(path.join(previous.root, asset.path)))
      } else copyFileSync(path.join(previous.root, basename), path.join(target, basename))
    }
    return { status: 0, stdout: '', stderr: '' }
  }
}

test('operator export verifies assets and metadata, preserves the chain and never starts a container', (t) => {
  const input = fixture(t)
  const priorHistory = historyRelease(input.history, { 'assets/old-abcdef12.js': 'older' })
  const previous = finalizeBuild({ ...input, sourceRevision: revision, buildId: 'previous-02' })
  const calls = []
  const output = path.join(input.temporary, 'export')
  const manifest = exportHistory({ image, output, run: fakeDocker({ ...previous, root: input.dist }, calls) })
  assert.equal(manifest.retainedReleases.length, 2)
  assert.deepEqual(manifest.retainedReleases[0], priorHistory.retainedReleases[0])
  assert.equal(manifest.retainedReleases[1].image, image)
  assert.equal(manifest.retainedReleases[1].identityKind, 'registry-digest')
  assert.equal(manifest.retainedReleases[1].metadataSha256, sha256(readFileSync(path.join(input.dist, 'release-metadata.json'))))
  assert.deepEqual(readHistory(output), manifest)
  assert.deepEqual(calls[0], ['pull', image])
  assert.deepEqual(calls[1].slice(0, 5), ['create', '--network', 'none', '--entrypoint', '/bin/false'])
  assert.ok(calls.every((args) => !['run', 'start', 'exec'].includes(args[0])))
  assert.equal(calls.at(-1)[0], 'rm')
})

test('offline export verifies the full local image ID without pulling or resolving tags', (t) => {
  const input = fixture(t)
  const previous = finalizeBuild(input)
  const calls = []
  const output = path.join(input.temporary, 'offline-export')
  const manifest = exportHistory({ imageId, output, run: fakeDocker({ ...previous, root: input.dist }, calls) })
  assert.deepEqual(calls[0], ['image', 'inspect', '--format', '{{.Id}}', imageId])
  assert.equal(calls[1].at(-1), imageId)
  assert.ok(calls.every((args) => args[0] !== 'pull'))
  assert.equal(manifest.retainedReleases[0].identityKind, 'docker-image-id')
  assert.equal(manifest.retainedReleases[0].image, imageId)
  assert.deepEqual(readHistory(output), manifest)
  const candidate = path.join(input.temporary, 'candidate')
  write(candidate, 'index.html', 'candidate')
  write(candidate, 'assets/new-12345678.js', 'candidate asset')
  const combined = finalizeBuild({ ...input, dist: candidate, history: output })
  assert.equal(combined.assets.length, 2)
  assert.deepEqual(combined.currentAssets.map((asset) => asset.path), ['assets/new-12345678.js'])
  assert.equal(readFileSync(path.join(candidate, previous.assets[0].path), 'utf8'), 'console.log("current");')
})

test('export accepts schema-1 metadata created before currentAssets was introduced', (t) => {
  const input = fixture(t)
  const metadata = finalizeBuild(input)
  delete metadata.currentAssets
  write(input.dist, 'release-metadata.json', `${JSON.stringify(metadata, null, 2)}\n`)
  const calls = []
  const output = path.join(input.temporary, 'legacy-schema-export')
  const exported = exportHistory({ imageId, output, run: fakeDocker({ ...metadata, root: input.dist }, calls) })
  assert.deepEqual(exported.assets, metadata.assets)
  const candidate = path.join(input.temporary, 'legacy-schema-candidate')
  write(candidate, 'index.html', '<script src="/assets/new-12345678.js"></script>')
  write(candidate, 'assets/new-12345678.js', 'new entry')
  const next = finalizeBuild({ ...input, dist: candidate, history: output })
  assert.deepEqual(next.currentAssets.map((asset) => asset.path), ['assets/new-12345678.js'])
  assert.equal(next.assets.length, 2)
})

test('offline export rejects shortened IDs, tag identities and a mismatched Docker inspection', (t) => {
  const input = fixture(t)
  let calls = 0
  const run = () => { calls++; return { status: 0, stdout: `sha256:${'0'.repeat(64)}`, stderr: '' } }
  for (const invalid of ['sha256:1234', 'host:latest', 'f'.repeat(64)]) {
    assert.throws(() => exportHistory({ imageId: invalid, output: input.history, run }), /full sha256/)
  }
  assert.throws(() => exportHistory({ image, imageId, output: input.history, run }), /exactly one/)
  assert.equal(calls, 0)
  assert.throws(() => exportHistory({ imageId, output: input.history, run }), /does not match/)
  assert.equal(calls, 1)
  assert.ok(!existsSync(path.join(input.history, 'manifest.json')))
})

test('export refuses tags and existing history before invoking Docker', (t) => {
  const input = fixture(t)
  let calls = 0
  const run = () => { calls++; throw new Error('must not be called') }
  assert.throws(() => exportHistory({ image: 'registry.test/host:latest', output: input.history, run }), /must be a registry/)
  historyRelease(input.history, { 'assets/old-abcdef12.js': 'old' })
  assert.throws(() => exportHistory({ image, output: input.history, run }), /already contains history/)
  assert.equal(calls, 0)
})

test('failed extraction removes only its stopped export container and does not publish a history manifest', (t) => {
  const input = fixture(t)
  const previous = finalizeBuild(input)
  const calls = []
  const output = path.join(input.temporary, 'failed-export')
  assert.throws(() => exportHistory({ image, output, run: fakeDocker({ ...previous, root: input.dist }, calls, 'cp') }), /Docker cp failed/)
  assert.equal(calls.at(-1)[0], 'rm')
  assert.ok(!existsSync(path.join(output, 'manifest.json')))
})

test('corrupted previous image metadata is rejected before export output is published', (t) => {
  const input = fixture(t)
  const previous = finalizeBuild(input)
  write(input.dist, 'assets/current-12345678.js', 'corrupt previous image')
  const calls = []
  const output = path.join(input.temporary, 'corrupt-export')
  assert.throws(() => exportHistory({ image, output, run: fakeDocker({ ...previous, root: input.dist }, calls) }), /do not match/)
  assert.ok(!existsSync(output))
  assert.equal(calls.at(-1)[0], 'rm')
})

test('checked-in host runtime policy has only static Nginx and an explicit read-only layout (text check only)', () => {
  const dockerfile = readFileSync(path.join(repository, 'frontend/Dockerfile'), 'utf8')
  const runtime = dockerfile.split(' AS runtime')[1]
  assert.match(dockerfile, /FROM nginxinc\/nginx-unprivileged:1\.28\.0-alpine AS runtime/)
  assert.match(runtime, /USER 101:101/)
  assert.match(runtime, /EXPOSE 8080/)
  assert.match(runtime, /ENTRYPOINT \["nginx"\]/)
  assert.doesNotMatch(runtime, /(?:npm |node |bun |caddy|envsubst)/i)
  assert.match(dockerfile, /WORKDIR \/app\/frontend/)
  assert.match(dockerfile, /COPY examples\/micro-app-sdk\/ \/app\/examples\/micro-app-sdk\//)
  assert.match(dockerfile, /COPY scripts\/sync-micro-sdk\.mjs \/app\/scripts\/sync-micro-sdk\.mjs/)
  assert.match(dockerfile, /RUN node \/app\/scripts\/sync-micro-sdk\.mjs --check --scope sdk/)
  assert.ok(dockerfile.indexOf('npm ci --ignore-scripts') < dockerfile.indexOf('--check --scope sdk'))
  assert.ok(dockerfile.indexOf('--check --scope sdk') < dockerfile.indexOf('RUN npm run build'))
  assert.match(dockerfile, /--dist \/app\/frontend\/dist --source \/app\/frontend/)
  assert.match(runtime, /\/app\/frontend\/dist\//)
  const config = readFileSync(path.join(repository, 'deploy/static/nginx.conf'), 'utf8')
  assert.match(config, /pid \/tmp\/nginx\.pid;/)
  for (const kind of ['client_body', 'proxy', 'fastcgi', 'uwsgi', 'scgi']) assert.match(config, new RegExp(`${kind}_temp_path /tmp/`))
  assert.match(config, /location \^~ \/sub-apps\/ \{ return 404; \}/)
  assert.match(config, /location \^~ \/assets\/ \{\s*limit_except GET \{ deny all; \}\s*try_files \$uri =404;\s*error_page 404 = @missing_host_asset;\s*\}/)
  assert.match(config, /location @missing_host_asset \{\s*types \{ \}\s*default_type text\/plain;\s*return 404 "Static resource not found\\n";\s*\}/)
  assert.doesNotMatch(config, /error_page.*index\.html|Access-Control-Allow-Credentials|envsubst/)
})
