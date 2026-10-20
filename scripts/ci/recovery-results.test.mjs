import test from 'node:test'
import assert from 'node:assert/strict'
import {recoveryChecks,validateRecovery} from './recovery-results.mjs'
function evidence(){
  const checks=recoveryChecks.map(id=>({id,status:'PASS',proof:{}})),p=id=>checks.find(c=>c.id===id).proof
  p('six-negative-refusals').negatives=['empty','corrupt','wrong-key','missing-artifact','same-target','existing-target'].map(name=>({name,exit:1,noTargetCreated:true}))
  Object.assign(p('remote-only-restore-with-separate-key'),{localBundleAbsent:true,credentialReset:true,productionPitr:'NOT_ACCEPTED',recoveredImages:{status:'PASS',images:Array.from({length:4},()=>({loadedAndStarted:true}))}})
  Object.assign(p('all-old-credential-families-refused'),{sameJwtKey:true,oldPersistentRowsRevoked:4,recoveryAudits:1})
  p('four-restored-frontend-versions').pages=['stores','work-orders'].flatMap(app=>['v1','v2'].map(version=>({app,version,restoredDataVisible:true,errors:0})))
  Object.assign(p('measured-pilot-rpo-rto'),{profile:'pilot',production15min1h:'NOT_ACCEPTED',physicalRemoteHost:'NOT_ACCEPTED'})
  Object.assign(p('actual-backup-alert-events'),{externalDelivery:false,timerInstalled:false,events:['backup_run','backup_verify','backup_age'].flatMap(component=>['DOWN','RECOVERED'].map(event=>({component,event})))})
  return {status:'PASS',checks,cleanup:{owned:true},segments:{...Object.fromEntries(['drainSeconds','backupSeconds','scheduledBackupSeconds','keyRetrievalSeconds','restoreSeconds','readinessSeconds','businessAcceptanceSeconds','rtoSeconds','rpoSeconds','totalQuiescenceSeconds'].map(k=>[k,1])),failureAtUnixMs:2000,latestRecoveryPointUnixSeconds:1}}
}
test('recovery rejects exit-only, missing/duplicate groups, skipped negatives and failed cleanup',()=>{
  validateRecovery(evidence())
  for(const mutate of [r=>r.checks.pop(),r=>r.checks[0].status='SKIP',r=>r.checks[1].id=r.checks[0].id,r=>r.checks[2].proof.negatives[0].noTargetCreated=false,r=>r.cleanup.owned=false]){const r=evidence();mutate(r);assert.throws(()=>validateRecovery(r))}
  assert.throws(()=>validateRecovery({status:'PASS',exitCode:0}))
})
test('recovery refuses fictional production success and CLI-only image recovery',()=>{
  for(const mutate of [r=>r.checks[4].proof.recoveredImages.images[0].loadedAndStarted=false,r=>r.checks[5].proof.oldPersistentRowsRevoked=0,r=>r.checks[7].proof.pages.pop(),r=>r.checks[8].proof.production15min1h='PASS',r=>r.checks[8].proof.physicalRemoteHost='PASS',r=>r.segments.rtoSeconds=14401,r=>r.segments.restoreSeconds=NaN,r=>r.checks[3].proof.events.pop()]){const r=evidence();mutate(r);assert.throws(()=>validateRecovery(r))}
})
