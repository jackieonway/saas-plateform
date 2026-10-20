import assert from 'node:assert/strict'
export const zipDistributionChecks=Object.freeze(['independent-local-storage','closed-intents','root-permissions','tls-authentication','all-nodes-before-public','idempotent-retry','live-file-verification','immutable-conflict','private-staging-and-partial-transfer','stale-and-foreign-receipts','activate-and-rollback','historical-alias-gates','audit-rollback','lease-recovery','real-management-ui'])
export function validateZipDistributionReport(r){
 assert.equal(r.schemaVersion,1);assert.equal(r.kind,'zip-distribution');assert.equal(r.status,'PASS');assert.equal(r.adoptable,true)
 assert.equal(r.scope,'isolated-local');assert.equal(r.productionOperations,false);assert.equal(r.sourceUnchanged,true);assert.equal(r.hostSourceUnchanged,true)
 assert.match(r.inputsSha256,/^[a-f0-9]{64}$/);assert.match(r.sourceRevision,/^[a-f0-9]{40}$/)
 assert.equal(r.checks.length,zipDistributionChecks.length)
 for(const id of zipDistributionChecks){const check=r.checks.filter(c=>c.id===id);assert.equal(check.length,1);assert.equal(check[0].status,'PASS');assert(check[0].proof)}
 for(const id of ['current-source-fast','existing-release-regression'])assert.equal(r.steps.filter(s=>s.id===id&&s.status==='PASS'&&s.exitCode===0).length,1)
 for(const b of r.originalBinaries){assert.equal(b.elfMachine,62);assert.match(b.sha256,/^[a-f0-9]{64}$/)}
 assert(r.originalBinaries.some(b=>b.name==='platform'));assert(r.originalBinaries.some(b=>b.name==='agent'));
 assert.equal(r.originalBinaries.length,2);assert.equal(r.browser.status,'PASS');assert.equal(r.browser.ignoreTlsErrors,false)
 for(const product of ['chromium','chrome','msedge'])assert.equal(r.browser.products.filter(p=>p.product===product&&p.status==='PASS').length,1)
 assert.equal(r.cleanup.cacheRemoved,true)
 assert.equal(r.cleanup.errors.length,0)
 for(const key of ['containersRemoved','networksRemoved','candidateImagesRemoved','privateDirectoryRemoved','registryTrustRemoved'])assert.equal(r.cleanup[key],true)
 return r
}
