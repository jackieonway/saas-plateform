import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import { assertWorkflowPolicy } from '../scripts/ci/workflow-policy.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'scripts/ci/package.json'))
const { parseDocument } = require('yaml')

function parseYaml(file) {
  const document = parseDocument(readFileSync(path.join(root, file), 'utf8'), { uniqueKeys: true, version: '1.2' })
  assert.deepEqual(document.errors, [], `${file} must be valid YAML without duplicate keys`)
  return document.toJS()
}

const workflow = parseYaml('.github/workflows/production-build.yml')
const tools = parseYaml('.github/actions/ci-tools/action.yml')

test('actual parsed workflow keeps PR credentials private and every mandatory candidate layer connected', () => {
  assert.equal(assertWorkflowPolicy(workflow, tools), true)
  assert.equal(readFileSync(path.join(root, '.nvmrc'), 'utf8').trim(), '24.18.0')
  assert.match(readFileSync(path.join(root, 'rust-toolchain.toml'), 'utf8'), /channel = "1\.97\.0"/)
})

test('policy rejects privilege escalation, deployment, hidden failures and lost browser or smoke dependencies', () => {
  const mutations = [
    (copy) => { copy.on.pull_request_target = {} },
    (copy) => { copy.permissions.contents = 'write' },
    (copy) => { copy.jobs.fast.permissions = { packages: 'write' } },
    (copy) => { copy.jobs.fast.env = { TOKEN: '${{ secrets.PRODUCTION_TOKEN }}' } },
    (copy) => { copy.jobs.fast.steps.push({ run: 'docker push registry.invalid/app:latest' }) },
    (copy) => { copy.jobs.fast.steps[0].with['persist-credentials'] = true },
    (copy) => { copy.jobs.browser.strategy.matrix.browser = ['chromium'] },
    (copy) => { copy.jobs.browser.steps = copy.jobs.browser.steps.filter(step => step.name !== 'Build isolated browser tools and fetch approved PostgreSQL') },
    (copy) => { copy.jobs.browser.steps.find(step => step.name === 'Run required browser checks').run = 'node scripts/ci/run.mjs --gate browser --out-dir "$RUNNER_TEMP/ci/browser"' },
    (copy) => { const step = copy.jobs.browser.steps.find(step => step.name === 'Save sanitized browser report'); step.with.path = step.with.path.split('\n').filter(value => !value.endsWith('/data-grant-ui/result.json')).join('\n') },
    (copy) => { copy.jobs.browser.steps.find(step => step.name === 'Save sanitized browser report').with.path += '${{ runner.temp }}/ci/browser/reports/browser/data-grant-ui/fixture.json\n' },
    (copy) => { const step = copy.jobs.browser.steps.find(step => step.name === 'Save sanitized browser report'); step.with.path = step.with.path.split('\n').filter(value => !value.endsWith('/work-orders/result.json')).join('\n') },
    (copy) => { copy.jobs.browser.steps.find(step => step.name === 'Save sanitized browser report').with.path += '${{ runner.temp }}/ci/browser/reports/browser/work-orders/work/logs/\n' },
    (copy) => { copy.jobs.browser.steps.find(step => step.name === 'Save sanitized browser report').with.path += '${{ runner.temp }}/ci/browser/reports/browser/work-orders/work/tls/ca.key\n' },
    (copy) => { const step = copy.jobs.browser.steps.find(step => step.name === 'Save sanitized browser report'); step.with.path = step.with.path.split('\n').filter(value => !value.endsWith('/stores/result.json')).join('\n') },
    (copy) => { copy.jobs.browser.steps.find(step => step.name === 'Save sanitized browser report').with.path += '${{ runner.temp }}/ci/browser/reports/browser/stores/work/logs/\n' },
    (copy) => { copy.jobs.browser.steps.find(step => step.name === 'Save sanitized browser report').with.path += '${{ runner.temp }}/ci/browser/reports/browser/stores/work/tls/ca.key\n' },
    (copy) => { delete copy.jobs['extended-browser'].steps.find(step => step.name === 'Run optional engine suite').env.WORK_ORDERS_ALLOWED_HOST_ORIGINS },
    (copy) => { const step = copy.jobs['extended-browser'].steps.find(step => step.name === 'Run optional engine suite'); step.run = step.run.replace('npm ci --prefix business-apps/work-orders/frontend', '') },
    (copy) => { delete copy.jobs['extended-browser'].steps.find(step => step.name === 'Run optional engine suite').env.STORES_ALLOWED_HOST_ORIGINS },
    (copy) => { const step = copy.jobs['extended-browser'].steps.find(step => step.name === 'Run optional engine suite'); step.run = step.run.replace('npm ci --prefix business-apps/stores/frontend', '') },
    (copy) => { copy.jobs.postgres['continue-on-error'] = true },
    (copy) => { const step = copy.jobs.postgres.steps.find(step => step.name === 'Save sanitized PostgreSQL report'); step.with.path = step.with.path.split('\n').filter(value => !value.endsWith('/stores-live-postgres/postgres-result.json')).join('\n') },
    (copy) => { copy.jobs.postgres.steps.find(step => step.name === 'Save sanitized PostgreSQL report').with.path += '${{ runner.temp }}/ci/postgres/reports/postgres/stores-postgres/private/*.log\n' },
    (copy) => { copy.jobs.postgres.steps = copy.jobs.postgres.steps.filter((step) => step.name !== 'Build isolated test tools and fetch approved PostgreSQL') },
    (copy) => { const index = copy.jobs.postgres.steps.findIndex((step) => step.name === 'Build isolated test tools and fetch approved PostgreSQL'); copy.jobs.postgres.steps.push(copy.jobs.postgres.steps.splice(index, 1)[0]) },
    (copy) => { copy.jobs.postgres.steps.find((step) => step.name === 'Save sanitized PostgreSQL report').with.path = '${{ runner.temp }}/ci/postgres/summary.json' },
    (copy) => { copy.jobs.delivery.needs = ['fast'] },
    (copy) => { copy.jobs['candidate-gate'].needs = ['fast', 'postgres', 'browser'] },
    (copy) => { copy.jobs['candidate-gate'].if = "github.event_name != 'pull_request'" },
    (copy) => { copy.jobs.delivery.steps.find((step) => step.name === 'Save verified candidate images').if = 'always()' },
    (copy) => { const step = copy.jobs.delivery.steps.find((step) => step.name === 'Save sanitized delivery report'); step.with.path = step.with.path.split('\n').filter((value) => !value.includes('/zip-release/result.json')).join('\n') },
    (copy) => { const step = copy.jobs.delivery.steps.find((step) => step.name === 'Save sanitized delivery report'); step.with.path = step.with.path.replace('/zip-release/logs/', '/zip-release/work/logs/') },
    (copy) => { copy.jobs.delivery.steps.find((step) => step.name === 'Save sanitized delivery report').with.path += '${{ runner.temp }}/ci/delivery/zip-release/source/logs/\n' },
    (copy) => { copy.jobs.fast.steps.find((step) => step.uses?.startsWith('actions/upload-artifact@')).with.path = '${{ runner.temp }}/ci/fast/' },
    (copy) => { copy.jobs.browser.steps.find(step => step.name === 'Save sanitized browser report').with.path += '${{ runner.temp }}/ci/browser/reports/browser/notification/source/logs/\n' },
    (copy) => { copy.jobs.delivery.steps.find(step => step.name === 'Save sanitized delivery report').with.path += '${{ runner.temp }}/ci/delivery/notification-image/work/logs/\n' },
    (copy) => { const step = copy.jobs.delivery.steps.find(step => step.name === 'Save sanitized delivery report'); step.with.path = step.with.path.split('\n').filter(value => !value.endsWith('/notification-image/inputs.json')).join('\n') },
    (copy) => { copy.jobs.fast.steps.push({ run: 'cargo test --workspace' }) },
  ]
  for (const mutate of mutations) {
    const copy = structuredClone(workflow)
    mutate(copy)
    assert.throws(() => assertWorkflowPolicy(copy, tools))
  }
})

test('actual final gate program rejects a failed, cancelled or skipped required job', () => {
  for (const name of ['pr-gate', 'candidate-gate']) {
    const step = workflow.jobs[name].steps[0]
    const match = /^node --input-type=module -e '([^']+)'\s*$/.exec(step.run)
    assert.ok(match, 'the final gate must remain an inspectable standalone Node program')
    const layers = workflow.jobs[name].needs
    const successful = Object.fromEntries(layers.map((layer) => [layer, { result: 'success' }]))
    const run = (results) => spawnSync(process.execPath, ['--input-type=module', '-e', match[1]], {
      env: { ...process.env, REQUIRED_RESULTS: JSON.stringify(results) }, encoding: 'utf8', timeout: 10000,
    })
    assert.equal(run(successful).status, 0)
    for (const layer of layers) {
      for (const result of ['failure', 'cancelled', 'skipped']) {
        const rejected = run({ ...successful, [layer]: { result } })
        assert.equal(rejected.status, 1, `${name} must reject ${layer}: ${result}`)
        assert.match(rejected.stderr, new RegExp(`${layer}: ${result}`))
      }
    }
  }
})
