// Generation tests live in the SDK repository; the host verifies its delivery.
import test from 'node:test'
import assert from 'node:assert/strict'
import { cpSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import { verifySdk } from '../../frontend/scripts/check-sdk.mjs'
const root = path.resolve(import.meta.dirname, '../..')
test('platform consumes a verified fixed SDK delivery and matching public Rust DTO', async () => {
  const release = verifySdk()
  assert.equal(release.sdkVersion, '1.0.0')
  assert.equal(readFileSync(path.join(root, 'backend/src/public_contract.rs'), 'utf8'), readFileSync(path.join(root, 'frontend/vendor/micro-app-sdk/releases/1.0.0/protocol.rs'), 'utf8'))
})
test('consumer verification rejects modified SDK bytes without regenerating them', async () => {
  const directory = mkdtempSync(path.join(os.tmpdir(), 'saas-sdk-consumer-'))
  cpSync(path.join(root, 'frontend/vendor'), path.join(directory, 'frontend/vendor'), { recursive: true })
  const file = path.join(directory, 'frontend/vendor/micro-app-sdk/releases/1.0.0/index.mjs')
  const changed = readFileSync(file, 'utf8') + '\n/* drift */\n'
  writeFileSync(file, changed)
  assert.throws(() => verifySdk({ root: path.join(directory, 'frontend') }), /artifact drift/)
  assert.equal(readFileSync(file, 'utf8'), changed)
})
