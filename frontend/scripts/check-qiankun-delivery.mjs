import assert from 'node:assert/strict'
import { existsSync, lstatSync, readFileSync, readdirSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { parse } from 'parse5'

const frontend = fileURLToPath(new URL('../', import.meta.url))
const retiredRuntime = /\b(?:__MICRO_APP__|MicroIframe|MICRO_CONNECT|MICRO_READY|IFRAME_PROTOCOL_VERSION)\b|(?:legacy-iframe|iframe-bridge)\.(?:m?js|ts)\b/
const sourceExtensions = /\.(?:[cm]?js|[jt]sx?|html|json)$/i

function files(root, { archiveBoundary = false } = {}, prefix = '') {
  if (!existsSync(root)) return []
  assert(!lstatSync(root).isSymbolicLink(), 'Delivery roots must not be symlinks: ' + root)
  return readdirSync(root, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name)).flatMap(entry => {
    // Archived SDK releases are not active host dependencies or build inputs.
    if (archiveBoundary && entry.name === 'releases') return []
    const absolute = path.join(root, entry.name), relative = prefix + entry.name
    assert(!entry.isSymbolicLink(), 'Delivery files must not be symlinks: ' + relative)
    return entry.isDirectory() ? files(absolute, { archiveBoundary }, relative + '/') : [relative]
  })
}

export function assertMicroHtml(html, name) {
  // HTML raw-script and comment states depend on their surrounding tokens.
  // Parse real elements and decoded attributes; JavaScript strings are text.
  const nodes = [parse(html, { scriptingEnabled: true })]
  while (nodes.length) {
    const node = nodes.pop()
    if (node.tagName === 'script') {
      const type = node.attrs.find(attribute => attribute.name === 'type')?.value ?? ''
      // Browsers match "module" case-insensitively without trimming whitespace.
      assert(type.toLowerCase() !== 'module',
        'Micro application HTML must use classic/UMD scripts: ' + name)
    }
    nodes.push(...(node.childNodes ?? []))
    if (node.content) nodes.push(node.content) // Template elements have a separate fragment.
  }
}

export function verifyQiankunDelivery({ root = frontend, dist } = {}) {
  const sdk = JSON.parse(readFileSync(path.join(root, 'vendor/micro-app-sdk/package.json'), 'utf8'))
  assert.equal(sdk.name, '@saas-platform/micro-app-sdk')
  assert.equal(sdk.version, '2.0.0', 'Active SDK must match the host contract')
  const sections = ['src', 'public', 'vendor/micro-app-sdk']
  let checked = 0
  for (const section of sections) {
    const directory = path.join(root, section)
    for (const name of files(directory, { archiveBoundary: section === 'vendor/micro-app-sdk' })) {
      assert(section !== 'public' || !name.startsWith('sub-apps/'),
        'Micro applications must be built in their independent repository, not host public/sub-apps: ' + name)
      if (!sourceExtensions.test(name)) continue
      const text = readFileSync(path.join(directory, name), 'utf8')
      assert(!retiredRuntime.test(text), 'Retired micro application runtime in active delivery: ' + section + '/' + name)
      if (/\.html$/i.test(name)) assertMicroHtml(text, section + '/' + name)
      checked++
    }
  }
  if (dist) {
    assert(existsSync(path.join(dist, 'index.html')), 'Build output index.html is required')
    for (const name of files(dist)) {
      assert(!name.startsWith('sub-apps/'), 'Host build must not contain embedded micro application copies: ' + name)
      if (!sourceExtensions.test(name)) continue
      const text = readFileSync(path.join(dist, name), 'utf8')
      assert(!retiredRuntime.test(text), 'Retired micro application runtime in built delivery: ' + name)
      // The host itself is a Vite ESM application; only child HTML entries are UMD.
      if (/\.html$/i.test(name) && name !== 'index.html') assertMicroHtml(text, name)
      checked++
    }
  }
  return { checked, sdkVersion: sdk.version, buildChecked: !!dist }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2)
  assert(args.length === 0 || (args.length === 2 && args[0] === '--dist'),
    'Usage: node scripts/check-qiankun-delivery.mjs [--dist dist]')
  const result = verifyQiankunDelivery({ dist: args.length ? path.resolve(args[1]) : undefined })
  console.log('Qiankun delivery verified: ' + JSON.stringify(result))
}
