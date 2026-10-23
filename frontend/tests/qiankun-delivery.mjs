import assert from 'node:assert/strict'
import test from 'node:test'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import { assertMicroHtml, verifyQiankunDelivery } from '../scripts/check-qiankun-delivery.mjs'

function fixture(t) {
  const root = mkdtempSync(path.join(os.tmpdir(), 'saas-qiankun-delivery-'))
  t.after(() => {
    assert.equal(path.dirname(root), path.resolve(os.tmpdir()))
    assert(path.basename(root).startsWith('saas-qiankun-delivery-'))
    rmSync(root, { recursive: true, force: true })
  })
  const write = (name, text) => { const file = path.join(root, name); mkdirSync(path.dirname(file), { recursive: true }); writeFileSync(file, text) }
  write('vendor/micro-app-sdk/package.json', '{"name":"@saas-platform/micro-app-sdk","version":"2.0.0"}')
  write('src/main.tsx', 'export const host = true')
  write('dist/index.html', '<script type="module" src="/assets/host.js"></script>')
  write('dist/assets/host.js', 'console.log("qiankun host")')
  return { root, write, dist: path.join(root, 'dist') }
}

test('the active checkout is qiankun-only while the host ESM entry remains allowed', () => {
  assert(verifyQiankunDelivery().checked > 20)
})

test('retired bridge code is rejected in both active source and bundled output', t => {
  for (const name of ['src/micro.ts', 'dist/assets/old.js']) {
    const f = fixture(t)
    f.write(name, 'window.MicroIframe = { connect() {} }')
    assert.throws(() => verifyQiankunDelivery(f), /Retired micro application runtime/)
  }
})

test('independent applications cannot silently return as public or built-in static copies', t => {
  for (const name of ['public/sub-apps/vue/index.html', 'dist/sub-apps/vue/index.html']) {
    const f = fixture(t)
    f.write(name, '<script src="./app.js"></script>')
    assert.throws(() => verifyQiankunDelivery(f), /independent repository|embedded micro application copies/)
  }
})

test('module child HTML is rejected regardless of quote or case while classic UMD and host ESM are accepted', t => {
  const f = fixture(t)
  assert.equal(verifyQiankunDelivery(f).buildChecked, true)
  assertMicroHtml('<script data-type="module" data-note="type=module" src="./vendor.js"></script><script entry src="./app.umd.js"></script>', 'micro.html')
  for (const type of ['type="module"', "TYPE='MoDuLe'", 'type=module', 'data-note=">" type="m&#111;dule"']) {
    f.write('dist/child.html', '<script ' + type + ' src="./app.js"></script>')
    assert.throws(() => verifyQiankunDelivery(f), /classic\/UMD/)
  }
  f.write('src/child.html', '<script type="module" src="./app.js"></script>')
  assert.throws(() => verifyQiankunDelivery({ root: f.root }), /classic\/UMD/)
})

test('a comment opener inside raw script text cannot hide the following real module element', t => {
  const html = '<script>const note="<!--";</script><script type=module src="./app.js"></script><!-- -->'
  assert.throws(() => assertMicroHtml(html, 'micro.html'), /classic\/UMD/)
  for (const name of ['src/child.html', 'dist/child.html']) {
    const f = fixture(t)
    f.write(name, html)
    assert.throws(() => verifyQiankunDelivery(f), /classic\/UMD/)
  }
})

test('script-like strings, comments and raw text do not become module elements', () => {
  for (const html of [
    '<script>const example=\'<script type="module" src="./fake.js">\';</script>',
    '<!-- <script type="module" src="./fake.js"></script> --><script src="./app.js"></script>',
    '<textarea><script type="module" src="./fake.js"></script></textarea>',
    '<style>/* <script type="module" src="./fake.js"></script> */</style>',
  ]) assert.doesNotThrow(() => assertMicroHtml(html, 'micro.html'))
})

test('decoded type attributes follow exact browser module matching rather than whitespace normalization', () => {
  assert.throws(() => assertMicroHtml('<script type="m&#111;dule" src="./app.js"></script>', 'micro.html'), /classic\/UMD/)
  // These decoded values are not exactly "module" and browsers treat them as data blocks.
  for (const type of ['&Tab;module&NewLine;', '&#32;module&#9;', 'module&amp;']) {
    assert.doesNotThrow(() => assertMicroHtml('<script type="' + type + '" src="./app.js"></script>', 'micro.html'))
  }
})

test('a historical SDK archive remains untouched and excluded from active delivery checks', t => {
  const f = fixture(t)
  f.write('vendor/micro-app-sdk/releases/1.0.0/iframe-bridge.js', 'window.MicroIframe = { archived: true }')
  assert(verifyQiankunDelivery(f).checked > 0)
  assert.match(readFileSync(path.join(f.root, 'vendor/micro-app-sdk/releases/1.0.0/iframe-bridge.js'), 'utf8'), /archived/)
})

test('an active SDK 1 package cannot replace the fixed SDK 2 contract', t => {
  const f = fixture(t)
  f.write('vendor/micro-app-sdk/package.json', '{"name":"@saas-platform/micro-app-sdk","version":"1.0.0"}')
  assert.throws(() => verifyQiankunDelivery(f), /Active SDK must match/)
})
