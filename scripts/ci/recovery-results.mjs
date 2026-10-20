import assert from 'node:assert/strict'
export const recoveryChecks=Object.freeze(['live-credentials-and-business-before-backup','coordinated-encrypted-remote-set','six-negative-refusals','actual-backup-alert-events',
  'remote-only-restore-with-separate-key','all-old-credential-families-refused','restored-login-authorization-crud-notifications','four-restored-frontend-versions','measured-pilot-rpo-rto','sources-retained-and-no-overwrite'])
/** Refuse incomplete or skipped recovery evidence; generic CLI exit codes do not suffice. */
export function validateRecovery(result){
  assert.equal(result.status,'PASS');assert.equal(result.checks.length,recoveryChecks.length)
  const proof=id=>{const matches=result.checks.filter(c=>c.id===id);assert.equal(matches.length,1);assert.equal(matches[0].status,'PASS');return matches[0].proof}
  for(const id of recoveryChecks)proof(id)
  const negatives=proof('six-negative-refusals').negatives
  for(const name of ['empty','corrupt','wrong-key','missing-artifact','same-target','existing-target'])assert(negatives.some(n=>n.name===name&&n.exit!==0&&n.noTargetCreated===true))
  const restored=proof('remote-only-restore-with-separate-key');assert(restored.localBundleAbsent&&restored.credentialReset)
  assert.equal(restored.productionPitr,'NOT_ACCEPTED');assert.equal(restored.recoveredImages.status,'PASS');assert.equal(restored.recoveredImages.images.length,4);assert(restored.recoveredImages.images.every(i=>i.loadedAndStarted))
  const creds=proof('all-old-credential-families-refused');assert(creds.sameJwtKey);assert.equal(creds.oldPersistentRowsRevoked,4);assert.equal(creds.recoveryAudits,1)
  const pages=proof('four-restored-frontend-versions').pages;assert.equal(pages.length,4)
  for(const app of ['stores','work-orders'])for(const version of ['v1','v2'])assert.equal(pages.filter(p=>p.app===app&&p.version===version&&p.restoredDataVisible&&p.errors===0).length,1)
  const measured=proof('measured-pilot-rpo-rto');assert.equal(measured.profile,'pilot');assert.equal(measured.production15min1h,'NOT_ACCEPTED');assert.equal(measured.physicalRemoteHost,'NOT_ACCEPTED')
  for(const key of ['drainSeconds','backupSeconds','scheduledBackupSeconds','keyRetrievalSeconds','restoreSeconds','readinessSeconds','businessAcceptanceSeconds','rtoSeconds','rpoSeconds','totalQuiescenceSeconds'])assert(Number.isFinite(result.segments[key])&&result.segments[key]>=0)
  assert(result.segments.rpoSeconds<=86400&&result.segments.rtoSeconds<=14400)
  assert(Number.isFinite(result.segments.latestRecoveryPointUnixSeconds)&&Number.isFinite(result.segments.failureAtUnixMs))
  assert(Math.abs(result.segments.rpoSeconds-(result.segments.failureAtUnixMs/1000-result.segments.latestRecoveryPointUnixSeconds))<0.001)
  const events=proof('actual-backup-alert-events');assert.equal(events.externalDelivery,false);assert.equal(events.timerInstalled,false)
  for(const component of ['backup_run','backup_verify','backup_age'])for(const event of ['DOWN','RECOVERED'])assert(events.events.some(e=>e.component===component&&e.event===event))
  assert(result.cleanup&&Object.values(result.cleanup).every(v=>v===true))
}
