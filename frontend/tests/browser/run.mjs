import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const args = process.argv.slice(2)
assert(args.length === 0 || (args.length === 2 && args[0] === '--browser'),
  'Usage: node tests/browser/run.mjs [--browser chromium|chrome|msedge|firefox|webkit]')
const browser = args[1] || 'chromium'
assert(['chromium', 'chrome', 'msedge', 'firefox', 'webkit'].includes(browser), 'Unknown browser')

const suites = [
  'notification-frontend', 'platform-settings', 'phase9-navigation', 'region-location',
  'entity-types', 'department-context', 'route-permissions',
  'micro-qiankun',
]
const environment = { ...process.env, SAAS_TEST_BROWSER: browser }
// Diagnostic selectors must not silently reduce the full suite.
for (const name of ['NOTIFICATION_BROWSER_SCENARIO', 'ROUTE_PERMISSIONS_FILTER',
  'PHASE9_BROWSER_FILTER', 'PHASE9_VITE_CONFIG']) delete environment[name]
for (const suite of suites) {
  const filename = fileURLToPath(new URL(`../${suite}-browser.mjs`, import.meta.url))
  console.log(`Browser suite: ${suite} (${browser})`)
  const result = spawnSync(process.execPath, [filename], {
    stdio: 'inherit', windowsHide: true, env: environment, timeout: 300_000,
  })
  if (result.error) throw result.error
  assert.equal(result.status, 0, `${suite} failed${result.signal ? ` (${result.signal})` : ''}`)
}
