import assert from 'node:assert/strict'

const requiredLayers = ['fast', 'postgres', 'browser']
const allCandidateLayers = [...requiredLayers, 'delivery', 'business-delivery']
const businessPublicEvidenceFiles = [
  'summary.json', 'source-manifest.json', 'logs/', 'business-delivery/result.json',
  'business-delivery/source-manifest.json', 'business-delivery/logs/*.log', 'business-delivery/screenshots/*.png',
  ...['chromium', 'chrome', 'msedge'].map(product => 'business-delivery/browser-' + product + '/fixture-result.json'),
  ...['platform', 'stores', 'work-orders', 'admin'].map(name => 'business-delivery/artifacts/original-binaries/' + name),
  ...['work-orders-v1', 'work-orders-v2', 'stores-v1', 'stores-v2'].map(key => 'business-delivery/artifacts/static/' + key + '/**'),
  'business-delivery/artifacts/artifact-inventory/' + '[0-9]'.repeat(20) + '.json',
]
const businessCandidateFiles = ['candidate-result.json', 'source-manifest.json', 'images/*.tar'].map(file => 'business-delivery/candidate/' + file)

function values(value) {
  if (Array.isArray(value)) return value.flatMap(values)
  if (value && typeof value === 'object') return Object.values(value).flatMap(values)
  return [String(value)]
}

function needs(job) {
  return typeof job.needs === 'string' ? [job.needs] : [...(job.needs ?? [])]
}

function readOnlyPermissions(permissions, where) {
  assert.ok(permissions && typeof permissions === 'object', `${where} needs explicit read-only permissions`)
  for (const [name, mode] of Object.entries(permissions)) {
    assert.equal(name, 'contents', `${where} must not request unrelated token capabilities`)
    assert.equal(mode, 'read', `${where} must not receive a write token`)
  }
}

/** Check the trust boundaries and mandatory job graph of the actual CI file. */
export function assertWorkflowPolicy(workflow, toolAction) {
  assert.ok(workflow.on?.pull_request !== undefined, 'ordinary pull requests must be checked')
  for (const event of ['pull_request_target', 'workflow_run']) {
    assert.equal(workflow.on?.[event], undefined, `${event} cannot execute an untrusted checkout with elevated context`)
  }
  readOnlyPermissions(workflow.permissions, 'workflow')
  const content = values([workflow, toolAction]).join('\n')
  assert.doesNotMatch(content, /\$\{\{\s*secrets\b|secrets\s*:\s*inherit/i, 'this workflow needs no external secrets')
  assert.doesNotMatch(content, /docker\/login-action|\bdocker\s+push\b|\b(?:ssh|scp)\s/i, 'candidate checks must not publish or deploy')
  assert.doesNotMatch(content, /cargo\s+test[^\n]*--workspace/, 'there is no root Cargo workspace')

  const jobs = workflow.jobs
  for (const name of [...allCandidateLayers, 'pr-gate', 'candidate-gate']) {
    assert.ok(jobs[name], `missing required job ${name}`)
    assert.equal(jobs[name]['continue-on-error'], undefined, `${name} must fail the check`)
    assert.equal(jobs[name]['runs-on'], 'ubuntu-24.04', `${name} must use an isolated GitHub Linux runner`)
  }
  for (const [name, job] of Object.entries(jobs)) {
    if (job.permissions !== undefined) readOnlyPermissions(job.permissions, name)
    for (const step of job.steps ?? []) {
      assert.equal(step['continue-on-error'], undefined, `${name} cannot hide failed steps`)
      if (step.uses?.startsWith('actions/checkout@')) {
        assert.equal(step.with?.['persist-credentials'], false, 'an untrusted checkout must not retain a token')
      }
      if (step.uses?.startsWith('actions/upload-artifact@')) {
        assert.equal(step.with?.['if-no-files-found'], 'error', 'missing evidence must fail')
        for (const entry of step.with.path.split('\n').map((value) => value.trim()).filter(Boolean)) {
          assert.match(entry, /^\$\{\{ runner\.temp \}\}\/ci\//, 'uploads must use owned CI evidence paths')
          if (entry.startsWith('${{ runner.temp }}/ci/business-delivery/')) {
            const files = [...businessPublicEvidenceFiles, ...businessCandidateFiles]
            assert.ok(files.some(file => entry === '${{ runner.temp }}/ci/business-delivery/' + file), 'business uploads must contain only exact public reports, direct sanitized logs, PNGs and verified candidate archives')
            assert.equal(name, 'business-delivery', 'business evidence must come from its owning mandatory job')
            continue
          }
          assert.match(entry, /(?:\/(?:summary|source-manifest|release-result|browser-result|linux-runner-result|snapshot-manifest|postgres-result|notification-result)\.json|\/logs\/|\/images\/\*\.tar|\/(?:postgres|work-orders-postgres|work-orders-read-postgres|work-orders-write-postgres|work-orders-member-postgres|work-orders-live-postgres|work-orders-write-live-postgres|work-orders-member-live-postgres|work-orders-reminder-postgres|work-orders-reminder-live-postgres|stores-postgres|stores-live-postgres|notification)\/\*\.log|\/delivery\/zip-release\/result\.json|\/delivery\/notification-image\/(?:result|inputs)\.json|\/notification\/service-logs\/\*\.log|\/notification\/browser-artifacts\/phase23-notification-\*\.png|\/browser\/reports\/browser\/data-grant-ui\/(?:result\.json|cargo\.log|management-preview\.png)|\/browser\/reports\/browser\/(?:work-orders|stores)\/(?:result\.json|logs\/|screenshots\/\*\.png))$/, 'do not upload private TLS, database or source scratch directories')
          if (entry.includes('/notification')) {
            assert.match(entry, /^(?:\$\{\{ runner\.temp \}\}\/ci\/(?:postgres\/reports\/postgres|browser\/reports\/browser)\/notification\/(?:notification-result\.json|\*\.log|service-logs\/\*\.log|browser-artifacts\/phase23-notification-\*\.png)|\$\{\{ runner\.temp \}\}\/ci\/delivery\/notification-image\/(?:result\.json|inputs\.json|logs\/))$/, 'notification uploads must only contain sanitized reports, logs and screenshots')
          }
          if (entry.includes('/zip-release/')) {
            assert.ok(['${{ runner.temp }}/ci/delivery/zip-release/result.json', '${{ runner.temp }}/ci/delivery/zip-release/logs/'].includes(entry), 'ZIP evidence uploads must not include private work or source trees')
          }
          if (entry.includes('/work-orders/')) {
            assert.match(entry, /^\$\{\{ runner\.temp \}\}\/ci\/browser\/reports\/browser\/work-orders\/(?:result\.json|logs\/|screenshots\/\*\.png)$/, 'work-order uploads may only contain the sanitized real result, controlled logs and PNG screenshots')
          }
          if (entry.includes('/stores/')) {
            assert.match(entry, /^\$\{\{ runner\.temp \}\}\/ci\/browser\/reports\/browser\/stores\/(?:result\.json|logs\/|screenshots\/\*\.png)$/, 'stores uploads may only contain the sanitized real result, controlled logs and PNG screenshots')
          }
          if (entry.includes('/stores-postgres/') || entry.includes('/stores-live-postgres/')) {
            assert.match(entry, /^\$\{\{ runner\.temp \}\}\/ci\/postgres\/reports\/postgres\/stores(?:-live)?-postgres\/(?:postgres-result\.json|\*\.log)$/, 'stores uploads must contain only sanitized reports and direct controlled logs')
          }
        }
      }
    }
  }

  assert.deepEqual(needs(jobs.postgres), ['fast'])
  const postgresSetup = jobs.postgres.steps.find((step) => step.name === 'Build isolated test tools and fetch approved PostgreSQL')
  assert.ok(postgresSetup, 'a clean runner must provision the private database test tools')
  assert.match(postgresSetup.run, /docker build --target toolchain --build-arg TARGETARCH=amd64 -f deploy\/ci\/Dockerfile -t saas-ci:phase04-tools \./)
  assert.match(postgresSetup.run, /docker pull postgres@sha256:45cd22f8d32e189d245403954882f88e7a8714301fda80dab6da90f1265b25a3/)
  assert.ok(jobs.postgres.steps.indexOf(postgresSetup) < jobs.postgres.steps.findIndex((step) => step.name === 'Run required database checks'), 'tools and PostgreSQL must be available before the clean launcher runs')
  assert.match(jobs.postgres.steps.find((step) => step.name === 'Run required database checks')?.run ?? '', /--image saas-ci:phase04-tools/)
  const postgresPaths = jobs.postgres.steps.find((step) => step.name === 'Save sanitized PostgreSQL report')?.with?.path ?? ''
  for (const entry of ['linux-runner-result.json', 'snapshot-manifest.json', 'reports/postgres/summary.json', 'reports/postgres/source-manifest.json', 'reports/postgres/postgres/postgres-result.json', 'reports/postgres/work-orders-postgres/postgres-result.json', 'reports/postgres/work-orders-postgres/*.log', 'reports/postgres/work-orders-read-postgres/postgres-result.json', 'reports/postgres/work-orders-read-postgres/*.log', 'reports/postgres/work-orders-write-postgres/postgres-result.json', 'reports/postgres/work-orders-write-postgres/*.log', 'reports/postgres/work-orders-member-postgres/postgres-result.json', 'reports/postgres/work-orders-member-postgres/*.log', 'reports/postgres/work-orders-live-postgres/postgres-result.json', 'reports/postgres/work-orders-live-postgres/*.log', 'reports/postgres/work-orders-write-live-postgres/postgres-result.json', 'reports/postgres/work-orders-write-live-postgres/*.log', 'reports/postgres/work-orders-member-live-postgres/postgres-result.json', 'reports/postgres/work-orders-member-live-postgres/*.log', 'reports/postgres/work-orders-reminder-postgres/postgres-result.json', 'reports/postgres/work-orders-reminder-postgres/*.log', 'reports/postgres/work-orders-reminder-live-postgres/postgres-result.json', 'reports/postgres/work-orders-reminder-live-postgres/*.log']) {
    assert.ok(postgresPaths.includes(`/ci/postgres/${entry}`), `missing actual Linux report path: ${entry}`)
  }
  assert.ok(postgresPaths.includes('/notification/notification-result.json'), 'real notification PostgreSQL evidence must be retained')
  const storesPaths = new Set(postgresPaths.split('\n').map(entry => entry.trim()).filter(Boolean))
  for (const gate of ['stores-postgres', 'stores-live-postgres']) for (const file of ['postgres-result.json', '*.log']) {
    assert.ok(storesPaths.has('${{ runner.temp }}/ci/postgres/reports/postgres/' + gate + '/' + file), 'actual stores SQL and live evidence must both be retained')
  }
  assert.deepEqual(needs(jobs.browser), ['fast'])
  assert.deepEqual(jobs.browser.strategy.matrix.browser, ['chromium', 'chrome', 'msedge'], 'all three actual products are mandatory')
  assert.equal(jobs.browser.strategy['fail-fast'], false, 'record every required browser result')
  const browserSetup = jobs.browser.steps.find(step => step.name === 'Build isolated browser tools and fetch approved PostgreSQL')
  assert.ok(browserSetup, 'browser acceptance must provision its owned PostgreSQL environment')
  assert.match(browserSetup.run, /docker build --target browsers --build-arg TARGETARCH=amd64 -f deploy\/ci\/Dockerfile -t saas-ci:phase15-browser \./)
  assert.match(browserSetup.run, /docker pull postgres@sha256:45cd22f8d32e189d245403954882f88e7a8714301fda80dab6da90f1265b25a3/)
  const browserRun = jobs.browser.steps.find(step => step.name === 'Run required browser checks')
  assert.ok(jobs.browser.steps.indexOf(browserSetup) < jobs.browser.steps.indexOf(browserRun))
  assert.match(browserRun?.run || '', /node scripts\/ci\/linux-runner\.mjs --gate browser/)
  assert.match(browserRun.run, /--image saas-ci:phase15-browser/)
  const browserPaths = new Set((jobs.browser.steps.find(step => step.name === 'Save sanitized browser report')?.with?.path || '').split('\n').map(value => value.trim()).filter(Boolean))
  for (const entry of ['linux-runner-result.json', 'snapshot-manifest.json', 'reports/browser/summary.json', 'reports/browser/source-manifest.json', 'reports/browser/browser-result.json', 'reports/browser/data-grant-ui/result.json', 'reports/browser/data-grant-ui/cargo.log', 'reports/browser/data-grant-ui/management-preview.png', 'reports/browser/work-orders/result.json', 'reports/browser/work-orders/logs/', 'reports/browser/work-orders/screenshots/*.png', 'reports/browser/stores/result.json', 'reports/browser/stores/logs/', 'reports/browser/stores/screenshots/*.png']) {
    assert.ok(browserPaths.has('${{ runner.temp }}/ci/browser/' + entry), `missing owned browser evidence: ${entry}`)
  }
  assert.ok(browserPaths.has('${{ runner.temp }}/ci/browser/reports/browser/notification/notification-result.json'), 'real notification browser evidence must be retained')
  assert.deepEqual(needs(jobs.delivery).sort(), [...requiredLayers].sort())
  assert.equal(jobs.delivery.if, "github.event_name != 'pull_request'")
  const engine = jobs.delivery.steps.find((step) => step.uses?.startsWith('docker/setup-docker-action@'))
  assert.equal(engine?.with?.version, 'v29.1.3', 'OCI archive verification requires the declared Docker engine')
  const deliveryReport = jobs.delivery.steps.find((step) => step.name === 'Save sanitized delivery report')
  const deliveryPaths = new Set((deliveryReport?.with?.path ?? '').split('\n').map((value) => value.trim()).filter(Boolean))
  for (const entry of ['${{ runner.temp }}/ci/delivery/zip-release/result.json', '${{ runner.temp }}/ci/delivery/zip-release/logs/']) {
    assert.ok(deliveryPaths.has(entry), 'the actual ZIP acceptance result and controlled logs must be uploaded')
  }
  for (const file of ['result.json', 'inputs.json', 'logs/']) {
    assert.ok(deliveryPaths.has('${{ runner.temp }}/ci/delivery/notification-image/' + file), 'actual notification image inputs and results must be retained')
  }
  const candidateUpload = jobs.delivery.steps.find((step) => step.name === 'Save verified candidate images')
  assert.ok(candidateUpload, 'candidate archives need a separate immutable artifact')
  assert.equal(candidateUpload.if, undefined, 'never upload a candidate after a failed smoke')
  assert.match(candidateUpload.with.path, /images\/\*\.tar/)

  const business = jobs['business-delivery']
  assert.deepEqual(needs(business).sort(), [...requiredLayers].sort())
  assert.equal(business.if, "github.event_name != 'pull_request'")
  assert.equal(business.steps.find(step => step.uses?.startsWith('docker/setup-docker-action@'))?.with?.version, 'v29.1.3')
  const businessRun = business.steps.find(step => step.name === 'Verify business candidates with owned registry and real versioned routes')
  const businessSetup = business.steps.find(step => step.name === 'Prepare isolated business browser tools and declared base images')
  assert.ok(businessSetup, 'a clean business runner must provision its owned tools and base images')
  assert.ok(business.steps.indexOf(businessSetup) < business.steps.indexOf(businessRun), 'declared tools must exist before business delivery starts')
  assert.deepEqual(businessSetup.run.trim().split(/\r?\n/), [
    'docker build --target browsers --build-arg TARGETARCH=amd64 -f deploy/ci/Dockerfile -t saas-ci:phase28-business-browser .',
    'docker pull registry:3.0.0', 'docker pull nginx:1.28.0-alpine', 'docker pull node:24.18.0-alpine', 'docker pull debian:bookworm-slim',
    'docker pull postgres@sha256:45cd22f8d32e189d245403954882f88e7a8714301fda80dab6da90f1265b25a3',
    'docker tag postgres@sha256:45cd22f8d32e189d245403954882f88e7a8714301fda80dab6da90f1265b25a3 postgres:17.6-bookworm',
  ])
  assert.equal(businessRun.env?.CI_TOOLCHAIN_IMAGE, 'saas-ci:phase28-business-browser')
  assert.equal(businessRun?.run, 'node scripts/ci/run.mjs --gate business-delivery --platform linux/amd64 --out-dir "$RUNNER_TEMP/ci/business-delivery"')
  const businessReport = business.steps.find(step => step.name === 'Save sanitized business delivery report')
  assert.equal(businessReport?.if, 'always()')
  assert.deepEqual(businessReport.with.path.trim().split(/\r?\n/).map(entry => entry.trim()), businessPublicEvidenceFiles.map(file => '${{ runner.temp }}/ci/business-delivery/' + file))
  const businessCandidate = business.steps.find(step => step.name === 'Save verified business candidate images')
  assert.ok(businessCandidate, 'business archives need their own immutable artifact')
  assert.equal(businessCandidate.if, undefined, 'business candidates may only be uploaded after all acceptance checks succeed')
  assert.equal(businessCandidate.with.name, 'business-linux-amd64-candidate-${{ github.sha }}-${{ github.run_id }}-${{ github.run_attempt }}')
  assert.deepEqual(businessCandidate.with.path.trim().split(/\r?\n/).map(entry => entry.trim()), businessCandidateFiles.map(file => '${{ runner.temp }}/ci/business-delivery/' + file))

  for (const [name, layers, event] of [
    ['pr-gate', requiredLayers, "github.event_name == 'pull_request'"],
    ['candidate-gate', allCandidateLayers, "github.event_name != 'pull_request'"],
  ]) {
    assert.deepEqual(needs(jobs[name]).sort(), [...layers].sort(), `${name} must depend on every required layer`)
    assert.equal(jobs[name].if, `always() && ${event}`, `${name} must run after failure or skip`)
    assert.equal(jobs[name].steps.length, 1)
    assert.equal(jobs[name].steps[0].env.REQUIRED_RESULTS, '${{ toJSON(needs) }}')
    assert.match(jobs[name].steps[0].run, /job\.result !== "success"/, 'failures, skips and cancellation must be rejected')
  }

  const optional = jobs['extended-browser']
  assert.equal(optional.if, "github.event_name == 'workflow_dispatch' && inputs.extended_browsers")
  assert.deepEqual(optional.strategy.matrix.browser, ['firefox', 'webkit'])
  for (const name of ['pr-gate', 'candidate-gate']) assert.ok(!needs(jobs[name]).includes('extended-browser'))
  assert.equal(workflow.on.workflow_dispatch.inputs.extended_browsers.default, false)
  const optionalRun = optional.steps.find(step => step.name === 'Run optional engine suite')
  assert.equal(optionalRun?.env?.WORK_ORDERS_ALLOWED_HOST_ORIGINS, 'https://platform.example.com', 'optional CI also needs an explicit fixture host origin')
  assert.equal(optionalRun?.env?.STORES_ALLOWED_HOST_ORIGINS, 'https://platform.example.com', 'optional CI also needs an explicit stores host origin')
  for (const directory of ['mini-services/oauth-playground/frontend', 'business-apps/work-orders/frontend', 'business-apps/stores/frontend']) assert.ok(optionalRun.run.includes('npm ci --prefix ' + directory), `optional aggregate builds must install ${directory}`)
  assert.equal(toolAction.runs.using, 'composite')
  assert.equal(toolAction.runs.steps.find((step) => step.uses?.startsWith('actions/setup-node@'))?.with?.['node-version-file'], '.nvmrc')
  assert.ok(values(toolAction).some((value) => value === 'npm install --global npm@11.16.0'))
  assert.ok(values(toolAction).some((value) => value.includes('rustup toolchain install 1.97.0')))
  return true
}

/** Publication is a separate manual trust boundary; candidate checks stay secret-free. */
export function assertBusinessPublishWorkflowPolicy(workflow) {
  assert.deepEqual(Object.keys(workflow.on ?? {}), ['workflow_dispatch'], 'external publication must only be explicitly dispatched')
  const inputs = workflow.on.workflow_dispatch?.inputs
  assert.deepEqual(Object.keys(inputs ?? {}).sort(), ['candidate_run_attempt', 'candidate_run_id'])
  for (const input of Object.values(inputs)) {
    assert.equal(input.type, 'string')
    assert.equal(input.required, true, 'a run and exact attempt must be selected explicitly')
  }
  assert.deepEqual(Object.keys(workflow.jobs ?? {}).sort(), ['publish', 'verify'], 'only the source verifier and protected publisher may execute')
  const permissions = (value, where) => {
    assert.ok(value && typeof value === 'object', `${where} needs explicit read-only permissions`)
    for (const [capability, mode] of Object.entries(value)) {
      assert.ok(['contents', 'actions'].includes(capability), `${where} must not acquire unrelated token capabilities`)
      assert.equal(mode, 'read', `${where} must not acquire a write token`)
    }
  }
  permissions(workflow.permissions, 'publish workflow')
  assert.doesNotMatch(values(workflow).join('\n'), /\b(?:ssh|scp)\s|\bdocker\s+push\b/i, 'publication may only load and publish verified archives through the controlled publisher')
  assert.doesNotMatch(values([workflow.env, workflow.on]).join('\n'), /\$\{\{\s*secrets\b|secrets\s*:\s*inherit/i, 'secrets must not enter workflow-wide or manual-input context')
  const { verify, publish } = workflow.jobs
  for (const [name, job] of Object.entries(workflow.jobs)) {
    assert.equal(job.if, "github.ref == 'refs/heads/main'", `${name} must execute trusted main workflow code`)
    assert.equal(job['runs-on'], 'ubuntu-24.04')
    assert.equal(job['continue-on-error'], undefined)
    if (job.permissions !== undefined) permissions(job.permissions, name)
    for (const step of job.steps ?? []) {
      assert.equal(step['continue-on-error'], undefined, 'publication cannot ignore verification or operation failures')
      if (step.uses?.startsWith('actions/checkout@')) assert.equal(step.with?.['persist-credentials'], false)
    }
  }
  assert.equal(verify.environment, undefined, 'metadata verification must not need external publication credentials')
  assert.doesNotMatch(values(verify).join('\n'), /\$\{\{\s*secrets\b|secrets\s*:\s*inherit|docker\/login-action|\bdocker\s+push\b/i, 'the source verifier must remain independent of registry credentials')
  assert.deepEqual(needs(publish), ['verify'])
  assert.equal(publish.environment, 'business-image-publish', 'external secrets require the fixed protected environment')
  assert.doesNotMatch(values(publish.env).join('\n'), /\$\{\{\s*secrets\b/i, 'publication secrets may only be read by the registry login step')
  const mainCheckout = verify.steps.find(step => step.uses?.startsWith('actions/checkout@'))
  assert.equal(mainCheckout?.with?.ref, '${{ github.sha }}', 'the verifier must come from the trusted dispatched main SHA')
  const trustedRun = verify.steps.find(step => /business-images-publish\.mjs[^\n]*--verify-run/.test(step.run ?? ''))
  assert.ok(trustedRun?.id, 'the API verifier must produce the trusted candidate identity')
  assert.equal(trustedRun.env?.GITHUB_TOKEN, '${{ github.token }}')
  assert.equal(trustedRun.env?.CANDIDATE_RUN_ID, '${{ inputs.candidate_run_id }}')
  assert.equal(trustedRun.env?.CANDIDATE_RUN_ATTEMPT, '${{ inputs.candidate_run_attempt }}')
  assert.doesNotMatch(trustedRun.run, /\$\{\{/, 'manual input must be passed as data through validated environment arguments')
  assert.equal(trustedRun.if, undefined, 'trusted run verification may never be skipped')
  for (const output of ['head_sha', 'artifact_id', 'artifact_name', 'run_id', 'run_attempt']) {
    assert.equal(verify.outputs?.[output], '${{ steps.' + trustedRun.id + '.outputs.' + output + ' }}')
  }
  const candidateCheckout = publish.steps.find(step => step.uses?.startsWith('actions/checkout@'))
  assert.equal(candidateCheckout?.with?.ref, '${{ needs.verify.outputs.head_sha }}', 'publish code must come from the verified candidate source')
  const download = publish.steps.find(step => step.uses?.startsWith('actions/download-artifact@'))
  assert.ok(download, 'the publisher must download the verified immutable candidate')
  assert.equal(download.with?.['artifact-ids'], '${{ needs.verify.outputs.artifact_id }}')
  assert.equal(download.with?.['run-id'], '${{ needs.verify.outputs.run_id }}')
  assert.equal(download.with?.['github-token'], '${{ github.token }}')
  assert.equal(download.with?.name, undefined, 'the downloaded artifact must be pinned by its verified ID')
  const archiveCheck = publish.steps.find(step => /business-images-publish\.mjs[^\n]*--verify-candidate/.test(step.run ?? ''))
  const login = publish.steps.find(step => step.uses?.startsWith('docker/login-action@') || /\bdocker\s+login\b/.test(step.run ?? ''))
  const operation = publish.steps.find(step => /business-images-publish\.mjs[^\n]*--publish/.test(step.run ?? ''))
  assert.ok(archiveCheck && login && operation, 'same-byte archive validation must precede protected login and publication')
  assert.ok(publish.steps.indexOf(candidateCheckout) < publish.steps.indexOf(download))
  assert.ok(publish.steps.indexOf(download) < publish.steps.indexOf(archiveCheck))
  assert.ok(publish.steps.indexOf(archiveCheck) < publish.steps.indexOf(login))
  assert.ok(publish.steps.indexOf(login) < publish.steps.indexOf(operation))
  assert.equal(archiveCheck.if, undefined, 'archive verification must never be conditional')
  assert.equal(login.if, undefined)
  assert.equal(operation.if, undefined)
  assert.equal(publish.steps.filter(step => /business-images-publish\.mjs[^\n]*--publish/.test(step.run ?? '')).length, 1)
  for (const step of publish.steps) {
    if (step !== login) assert.doesNotMatch(values(step).join('\n'), /\$\{\{\s*secrets\b/i, 'only protected registry login may consume external secrets')
    if (step.uses?.startsWith('actions/upload-artifact@')) {
      assert.equal(step.with?.['if-no-files-found'], 'error')
      for (const entry of step.with.path.trim().split(/\r?\n/).map(value => value.trim())) {
        assert.match(entry, /^\$\{\{ runner\.temp \}\}\/business-publication\/(?:publication-result\.json|logs\/\*\.log)$/, 'publication evidence excludes archives, source and login configuration')
      }
    }
  }
  return true
}
