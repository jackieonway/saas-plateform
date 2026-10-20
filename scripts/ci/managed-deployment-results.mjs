import assert from 'node:assert/strict'
export const managedDeploymentChecks = Object.freeze([
  'closed-intents-and-trusted-repositories', 'independent-root-permissions',
  'immutable-binding-and-create-idempotency', 'verified-tls-and-agent-authentication',
  'platform-has-no-docker-authority', 'actual-digest-pull-and-constrained-runtime',
  'receipt-required-before-activation', 'stale-and-foreign-receipts-refused',
  'public-preflight-explicit-activation', 'bad-image-retains-active',
  'unhealthy-retains-active', 'route-failure-restores-configuration',
  'retry-reuses-identical-runtime', 'rollback-preserves-old-resources',
  'reference-safe-cleanup', 'orphan-cleanup-preserves-unowned-resources',
  'real-management-ui', 'transactional-audit-failure',
])
export function validateManagedDeploymentReport(r) {
  assert.equal(r.schemaVersion, 1); assert.equal(r.kind, 'managed-image-deployment'); assert.equal(r.status, 'PASS')
  assert.equal(r.adoptable, true); assert.equal(r.scope, 'isolated-local'); assert.equal(r.sourceUnchanged, true); assert.equal(r.hostSourceUnchanged, true)
  assert.match(r.inputsSha256, /^[a-f0-9]{64}$/); assert.match(r.sourceRevision, /^[a-f0-9]{40}$/)
  assert.equal(r.checks.length, managedDeploymentChecks.length)
  for (const id of managedDeploymentChecks) { const matches = r.checks.filter(c => c.id === id); assert.equal(matches.length, 1); assert.equal(matches[0].status, 'PASS'); assert(matches[0].proof) }
  assert.equal(r.productionOperations, false); assert.equal(r.registry.scope, 'owned-loopback-tls'); assert.equal(r.registry.normalTls, true)
  assert(r.originalBinaries.some(b => b.name === 'platform' && b.elfMachine === 62)); assert(r.originalBinaries.some(b => b.name === 'agent' && b.elfMachine === 62))
  assert.equal(r.browser.status, 'PASS'); assert.equal(r.browser.ignoreTlsErrors, false)
  for(const id of ['current-source-fast','existing-release-regression'])assert.equal(r.steps.filter(s=>s.id===id&&s.status==='PASS'&&s.exitCode===0).length,1)
  assert(r.steps.length >= 6 && r.steps.every(s => s.exitCode === 0 && s.status === 'PASS'))
  assert.equal(r.cleanup.errors.length, 0)
  for (const key of ['containersRemoved', 'networksRemoved', 'candidateImagesRemoved', 'privateDirectoryRemoved', 'registryTrustRemoved']) assert.equal(r.cleanup[key], true)
  return r
}
