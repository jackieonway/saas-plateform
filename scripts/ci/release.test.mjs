import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { buildCandidate, inspectOciArchive, parsePlatformProbe, parseReleaseArgs } from './release.mjs'

const hash = bytes => createHash('sha256').update(bytes).digest('hex')
function member(name, bytes, type = '0') {
  const content = Buffer.from(bytes), header = Buffer.alloc(512)
  header.write(name, 0, 100); header.write('0000644\0', 100); header.write('0000000\0', 108); header.write('0000000\0', 116)
  header.write(`${content.length.toString(8).padStart(11, '0')}\0`, 124); header.write('00000000000\0', 136); header.fill(32, 148, 156); header.write(type, 156)
  header.write('ustar\0', 257); header.write('00', 263)
  const checksum = [...header].reduce((sum, value) => sum + value, 0)
  header.write(`${checksum.toString(8).padStart(6, '0')}\0 `, 148)
  return Buffer.concat([header, content, Buffer.alloc((512 - content.length % 512) % 512)])
}
function fixture(t, { platform = 'linux/amd64', mutate, extra = [] } = {}) {
  const directory = mkdtempSync(path.join(os.tmpdir(), 'saas-oci-test-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const files = new Map(), add = (bytes, mediaType) => { const data = Buffer.from(bytes), digest = `sha256:${hash(data)}`; files.set(`blobs/sha256/${digest.slice(7)}`, data); return { mediaType, digest, size: data.length } }
  const [osName, architecture] = platform.split('/')
  const config = add(JSON.stringify({ os: osName, architecture, config: { Labels: { 'org.opencontainers.image.revision': 'a'.repeat(40) } } }), 'application/vnd.oci.image.config.v1+json')
  const layer = add('valid small layer bytes', 'application/vnd.oci.image.layer.v1.tar')
  const manifest = add(JSON.stringify({ schemaVersion: 2, config, layers: [layer] }), 'application/vnd.oci.image.manifest.v1+json')
  files.set('index.json', Buffer.from(JSON.stringify({ schemaVersion: 2, manifests: [manifest] })))
  files.set('oci-layout', Buffer.from('{"imageLayoutVersion":"1.0.0"}'))
  mutate?.(files, { config, layer, manifest })
  const file = path.join(directory, 'image.tar')
  writeFileSync(file, Buffer.concat([...files.entries()].map(([name, bytes]) => member(name, bytes)).concat(extra, [Buffer.alloc(1024)])))
  return { file, manifest, config, layer, directory }
}

test('release CLI refuses ambiguous platforms, output paths, duplicated switches and credentialled proxies', () => {
  const output = path.resolve(os.tmpdir(), 'new-release')
  assert.deepEqual(parseReleaseArgs(['--out-dir', output, '--platform', 'linux/arm64', '--smoke']), { outDir: output, platform: 'linux/arm64', smoke: true })
  for (const args of [[], ['--out-dir', 'relative'], ['--out-dir', output, '--platform', 'windows/amd64'], ['--out-dir', output, '--smoke', '--smoke'],
    ['--out-dir', output, '--platform', 'linux/amd64', '--platform', 'linux/arm64'], ['--out-dir', output, '--build-proxy', 'http://user:secret@127.0.0.1:7897']]) assert.throws(() => parseReleaseArgs(args))
})

test('verified OCI manifest and archive hashes bind complete layer bytes and exact platform', t => {
  const { file, manifest, config } = fixture(t)
  const actual = inspectOciArchive(file, 'linux/amd64')
  assert.equal(actual.ociManifestDigest, manifest.digest); assert.equal(actual.ociConfigDigest, config.digest)
  assert.equal(actual.archiveSha256, hash(readFileSync(file))); assert.equal(actual.verifiedLayerCount, 1)
  assert.equal(actual.labels['org.opencontainers.image.revision'], 'a'.repeat(40))
  assert.throws(() => inspectOciArchive(file, 'linux/arm64'), /platform/)
})

test('OCI verification rejects content tampering even with valid tar checksum', t => {
  const { file } = fixture(t, { mutate(files, { layer }) { files.set(`blobs/sha256/${layer.digest.slice(7)}`, Buffer.from('modified small layer byte')) } })
  assert.throws(() => inspectOciArchive(file, 'linux/amd64'), /digest\/size mismatch/)
})

test('OCI verification rejects traversal, links and duplicate members before trusting metadata', t => {
  for (const invalid of [member('../outside', 'x'), member('linked', '', '2'), member('index.json', '{}')]) {
    const { file } = fixture(t, { extra: [invalid] })
    assert.throws(() => inspectOciArchive(file, 'linux/amd64'), /Unsafe|Unsupported|Duplicate/)
  }
})

test('OCI verification fails a missing layer and legacy-only archive rather than inventing a distribution digest', t => {
  const missing = fixture(t, { mutate(files, { layer }) { files.delete(`blobs/sha256/${layer.digest.slice(7)}`) } })
  assert.throws(() => inspectOciArchive(missing.file, 'linux/amd64'), /digest\/size mismatch/)
  const old = path.join(missing.directory, 'legacy.tar'); writeFileSync(old, Buffer.concat([member('manifest.json', '[]'), Buffer.alloc(1024)]))
  assert.throws(() => inspectOciArchive(old, 'linux/amd64'), /Docker 29 OCI save support is required/)
})

test('candidate builder never overwrites existing evidence or nests output in build inputs', async t => {
  const directory = mkdtempSync(path.join(os.tmpdir(), 'saas-release-test-')); t.after(() => rmSync(directory, { recursive: true, force: true }))
  writeFileSync(path.join(directory, 'keep.txt'), 'keep')
  await assert.rejects(buildCandidate({ outDir: directory, platform: 'linux/amd64' }), /already exists/)
  assert.equal(readFileSync(path.join(directory, 'keep.txt'), 'utf8'), 'keep')
  await assert.rejects(buildCandidate({ outDir: path.join(directory, 'frontend/src/new-output'), sourceRoot: directory, platform: 'linux/amd64' }), /overlaps source inputs/)
})

test('platform probe checks actual ELF architecture, runtime identity and shared-library resolution', () => {
  const output = `UID=10001\nBINARY_SHA256=${'a'.repeat(64)}  /usr/local/bin/saas-platform-backend\nELF_HEADER= 7f 45 4c 46 02 01 01 00 00 00 00 00 00 00 00 00 03 00 3e 00\n libc.so.6 => /lib/x86_64-linux-gnu/libc.so.6\n`
  assert.equal(parsePlatformProbe(output, 'linux/amd64').elfMachine, 62)
  assert.throws(() => parsePlatformProbe(output, 'linux/arm64'), /ELF machine/)
  assert.throws(() => parsePlatformProbe(output.replace('UID=10001', 'UID=0'), 'linux/amd64'), /Invalid nonroot/)
  assert.throws(() => parsePlatformProbe(output.replace('libc.so.6 => /lib/x86_64-linux-gnu/libc.so.6', 'libc.so.6 => not found'), 'linux/amd64'), /shared libraries/)
})
