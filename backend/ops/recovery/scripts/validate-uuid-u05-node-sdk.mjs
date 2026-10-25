#!/usr/bin/env node
// Read-only deployment checks and in-process SDK transport; no real credentials.
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../../..')
const publicOutput = process.argv[2]
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex')
const v7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i
const gateway = 'http://127.0.0.1:13000'

async function checkHistory(baseline) {
  assert.equal(baseline.files.length, 216, 'HISTORY_EXPECTED_216_FILES')
  for (const item of baseline.files) {
    assert(!path.isAbsolute(item.path) && !item.path.split(/[\\/]/).includes('..'), 'HISTORY_RELATIVE_PATH')
    const bytes = await readFile(path.join(root, item.path))
    assert.equal(bytes.length, item.bytes, 'HISTORY_LENGTH_UNCHANGED')
    assert.equal(sha256(bytes), item.sha256, 'HISTORY_HASH_UNCHANGED')
  }
  return baseline.files.length
}

async function readGateway(resource, expected) {
  const response = await fetch(gateway + resource, { redirect: 'manual', signal: AbortSignal.timeout(15000) })
  assert.equal(response.status, 200, 'GATEWAY_SDK_200_REQUIRED')
  assert.match(response.headers.get('content-type') || '', /javascript/, 'GATEWAY_JAVASCRIPT_REQUIRED')
  const bytes = Buffer.from(await response.arrayBuffer())
  assert.equal(bytes.length, expected.bytes, 'GATEWAY_RELEASE_LENGTH_MATCH')
  assert.equal(sha256(bytes), expected.sha256, 'GATEWAY_RELEASE_HASH_MATCH')
  return bytes
}

async function requestIds(sdk) {
  assert.equal(sdk.SDK_VERSION, '3.0.0', 'ACTUAL_SDK_VERSION')
  assert(sdk.isSupportedSdkVersion('3.0.0'), 'SDK_3_SUPPORTED')
  assert(!sdk.isSupportedSdkVersion('2.0.0'), 'SDK_2_NOT_ACTIVE')
  const ids = []
  const props = {
    sdkVersion: '3.0.0', contractVersion: '2.0', appCode: 'u05-node-probe',
    basePath: '/micro/u05-node-probe', contextVersion: 'u05-offline', contextEpoch: 1,
    token: 'in-process-fixture-only-never-sent-to-network', signal: new AbortController().signal,
    expiresAt: new Date(Date.now() + 60000).toISOString(),
    tokenExpiresAt: new Date(Date.now() + 60000).toISOString(),
    user: { id: '019a2200-0000-7000-8000-000000000001' },
    tenant: { id: '019a2200-0000-7000-8000-000000000002' },
    organization: { id: '019a2200-0000-7000-8000-000000000003' },
  }
  const localTransport = async (_url, options) => {
    ids.push(options.headers.get('X-Request-Id'))
    return new Response('{}', { status: 200, headers: { 'content-type': 'application/json' } })
  }
  for (let index = 0; index < 1000; index += 1) {
    assert.deepEqual(await sdk.requestWithContext({ props, url: '/in-process-fixture', fetchImpl: localTransport }), {})
  }
  assert.equal(ids.length, 1000, '1000_ACTUAL_SDK_REQUESTS')
  assert(ids.every(value => v7.test(value)), 'ALL_GENERATED_UUIDS_RFC_V7')
  assert.equal(new Set(ids).size, 1000, 'ALL_GENERATED_UUIDS_UNIQUE')
  return ids
}

function describeIds(ids) {
  return {
    generated: ids.length,
    version7: ids.filter(value => value[14] === '7').length,
    rfcVariant: ids.filter(value => '89ab'.includes(value[19].toLowerCase())).length,
    unique: new Set(ids).size,
    generatedIdsSha256: sha256(ids.join('\n')),
    transport: 'IN_PROCESS_CAPTURE_OF_REAL_SDK_X_REQUEST_ID_HEADER_NO_NETWORK',
  }
}

try {
  assert(publicOutput && path.isAbsolute(publicOutput), 'ABSOLUTE_NEW_PUBLIC_OUTPUT_REQUIRED')
  const frontendBytes = await readFile(path.join(root, 'saas-plateform/backend/docs/identity-sso/validation/uuid-u05-frontend-builds.json'))
  const history = JSON.parse(frontendBytes).frozen_history
  const before = await checkHistory(history)
  const releaseRoot = path.join(root, 'saas-micro-app-sdk/releases/3.0.0')
  const manifestBytes = await readFile(path.join(releaseRoot, 'artifact-manifest.json'))
  const manifest = JSON.parse(manifestBytes)
  assert.equal(manifest.sdkVersion, '3.0.0')
  const item = name => {
    const value = manifest.files.find(file => file.path === name)
    assert(value, 'MANIFEST_ENTRY_REQUIRED')
    return value
  }
  const classicBytes = await readGateway('/sub/vue/vendor/micro-sdk.js', item('sdk.js'))
  await readGateway('/sub/jquery/vendor/micro-sdk.js', item('sdk.js'))
  // The classic artifact intentionally exposes window.MicroSdk. Import the
  // exact served bytes without editing, recompiling, or appending exports.
  assert.equal(globalThis.window, undefined, 'CLEAN_NODE_GLOBAL_REQUIRED')
  globalThis.window = {}
  await import('data:text/javascript;base64,' + classicBytes.toString('base64'))
  const classicIds = await requestIds(window.MicroSdk)
  delete globalThis.window
  const esmBytes = await readFile(path.join(releaseRoot, 'index.mjs'))
  assert.equal(esmBytes.length, item('index.mjs').bytes, 'ESM_RELEASE_LENGTH_MATCH')
  assert.equal(sha256(esmBytes), item('index.mjs').sha256, 'ESM_RELEASE_HASH_MATCH')
  const esm = await import('data:text/javascript;base64,' + esmBytes.toString('base64'))
  const esmIds = await requestIds(esm)
  assert.equal(new Set([...classicIds, ...esmIds]).size, 2000, 'CROSS_CONSUMER_UUID_UNIQUENESS')
  const after = await checkHistory(history)
  const result = {
    schemaVersion: 1, status: 'PASS', scope: 'LOCAL_QUARANTINED_PREVIEW_SDK_NODE_RUNTIME',
    sdkVersion: '3.0.0', nodeVersion: process.version,
    manifestSha256: sha256(manifestBytes),
    gatewayArtifacts: [
      { path: '/sub/vue/vendor/micro-sdk.js', bytes: item('sdk.js').bytes, sha256: item('sdk.js').sha256 },
      { path: '/sub/jquery/vendor/micro-sdk.js', bytes: item('sdk.js').bytes, sha256: item('sdk.js').sha256 },
    ],
    actualGatewayClassicImportedInNode: describeIds(classicIds),
    frozenReleaseEsmImportedInNode: { ...describeIds(esmIds), bytes: item('index.mjs').bytes, sha256: item('index.mjs').sha256 },
    combinedUnique: 2000,
    historicalFiles: { checkedBefore: before, checkedAfter: after, unchanged: true, baselineSha256: sha256(frontendBytes) },
    limitations: ['Node request transport is in-process; real browser transport and business HTTP are validated separately'],
  }
  await writeFile(publicOutput, JSON.stringify(result, null, 2) + '\n', { flag: 'wx' })
  console.log(JSON.stringify(result))
} catch {
  // Do not echo parser/network/data-URL exception messages or fixture values.
  console.error(JSON.stringify({ status: 'FAIL', failureCode: 'NODE_SDK_ACCEPTANCE_FAILED' }))
  process.exitCode = 1
}
