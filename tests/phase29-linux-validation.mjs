/** Run contracts from public source bytes in a disposable Linux filesystem. */
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdirSync, copyFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { randomUUID } from 'node:crypto'
import { createSourceManifest } from '../scripts/ci/provenance.mjs'
const root = process.cwd(), out = path.resolve(process.argv[2] || 'target/phase-29/linux-contracts')
mkdirSync(out, { recursive: true })
const source = createSourceManifest(root), snapshot = path.join(out, 'source')
for (const file of source.files) {
  const to = path.join(snapshot, file.path); mkdirSync(path.dirname(to), { recursive: true }); copyFileSync(path.join(root, file.path), to)
}
writeFileSync(path.join(out, 'source-manifest.json'), JSON.stringify(source, null, 2) + '\n')
const owner = 'phase29-contracts-' + randomUUID()
const image = 'sha256:1912f7302185ed2c1af4df155d0d2784886c87984266ed981325575e415d71dc'
const linuxPath = process.platform === 'win32' ? '/mnt/' + snapshot[0].toLowerCase() + snapshot.slice(2).replaceAll('\\', '/') : snapshot
const command = process.platform === 'win32' ? 'wsl.exe' : 'docker'
const prefix = process.platform === 'win32' ? ['-d', 'Ubuntu', '--', 'docker'] : []
const script = `set -eu
cp -R /input/. /work/
cd /work
export NPM_CONFIG_USERCONFIG=/tmp/phase29-npmrc NPM_CONFIG_GLOBALCONFIG=/tmp/phase29-global NPM_CONFIG_CACHE=/tmp/npm-cache
printf 'engine-strict=true\\n' > "$NPM_CONFIG_USERCONFIG"
touch "$NPM_CONFIG_GLOBALCONFIG"
node --version
npm --version
npm ci --prefix frontend --ignore-scripts --no-audit --no-fund
npm ci --prefix business-apps/work-orders/frontend --ignore-scripts --no-audit --no-fund
# Smoke contracts compare the generated compatibility mirror. Recreate it from
# the two selected public source trees; it is excluded from the source snapshot.
mkdir -p frontend/public/sub-apps/vue frontend/public/sub-apps/jquery
cp -R mini-services/micro-vue/public/. frontend/public/sub-apps/vue/
cp -R mini-services/micro-jquery/public/. frontend/public/sub-apps/jquery/
node --input-type=module -e "import {contractTests} from './scripts/ci/manifest.mjs'; import {spawnSync} from 'node:child_process';const r=spawnSync(process.execPath,['--test','--test-reporter=tap',...contractTests],{stdio:'inherit'});process.exitCode=r.status??1"
`
const result = spawnSync(command, [...prefix, 'run', '-i', '--rm', '--init', '--name', owner, '--label', `saas.phase29.owner=${owner}`,
  '--mount', `type=bind,src=${linuxPath},dst=/input,readonly`, '--tmpfs', '/work:rw,exec,size=1536m', '--tmpfs', '/tmp:rw,exec,size=768m',
  image, 'sh', '-s'], { input: script, encoding: 'utf8', windowsHide: true, timeout: 600000, maxBuffer: 20 * 1024 * 1024 })
writeFileSync(path.join(out, 'raw.log'), (result.stdout || '') + (result.stderr || ''))
const inspect = spawnSync(command, [...prefix, 'ps', '-aq', '--filter', `label=saas.phase29.owner=${owner}`], { encoding: 'utf8', windowsHide: true, timeout: 30000 })
const report = { owner, image, sourceRevision: source.sourceRevision, inputsSha256: source.inputsSha256, exitCode: result.status,
  error: result.error?.message, remainingOwnedContainers: inspect.stdout?.trim(), cleanupQueryExitCode: inspect.status }
writeFileSync(path.join(out, 'result.json'), JSON.stringify(report, null, 2) + '\n')
console.log(JSON.stringify(report))
assert.equal(result.status, 0, 'Linux contracts failed; see raw.log')
assert.equal(inspect.status, 0); assert.equal(inspect.stdout.trim(), '')
assert.match(result.stdout, /# fail 0\b/); assert.match(result.stdout, /# skipped 0\b/)
