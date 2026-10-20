import {test} from 'node:test'
import assert from 'node:assert/strict'
import {managedDeploymentChecks,validateManagedDeploymentReport} from './managed-deployment-results.mjs'
const valid=()=>({schemaVersion:1,kind:'managed-image-deployment',status:'PASS',adoptable:true,scope:'isolated-local',sourceUnchanged:true,hostSourceUnchanged:true,inputsSha256:'a'.repeat(64),sourceRevision:'b'.repeat(40),checks:managedDeploymentChecks.map(id=>({id,status:'PASS',proof:{actual:true}})),productionOperations:false,registry:{scope:'owned-loopback-tls',normalTls:true},originalBinaries:[{name:'platform',elfMachine:62},{name:'agent',elfMachine:62}],browser:{status:'PASS',ignoreTlsErrors:false},steps:['current-source-fast','existing-release-regression','build','fmt','tests','clippy'].map(id=>({id,exitCode:0,status:'PASS'})),cleanup:{errors:[],containersRemoved:true,networksRemoved:true,candidateImagesRemoved:true,privateDirectoryRemoved:true,registryTrustRemoved:true}})
test('managed deployment evidence requires every closed check and no duplicate or skipped claim',()=>{
 assert.equal(validateManagedDeploymentReport(valid()).status,'PASS')
 for(const mutate of [r=>r.checks.pop(),r=>r.checks[1]=r.checks[0],r=>r.checks[0].status='SKIP',r=>delete r.checks[0].proof]){const r=valid();mutate(r);assert.throws(()=>validateManagedDeploymentReport(r))}
})
test('build-only, fixture-only, altered-source and insecure TLS reports cannot certify deployment',()=>{
 for(const mutate of [r=>r.adoptable=false,r=>r.sourceUnchanged=false,r=>r.hostSourceUnchanged=false,r=>r.browser.ignoreTlsErrors=true,r=>r.registry.normalTls=false,r=>r.originalBinaries.pop(),r=>r.productionOperations=true]){const r=valid();mutate(r);assert.throws(()=>validateManagedDeploymentReport(r))}
})
test('cleanup and actual failed process exit remain mandatory',()=>{
 for(const mutate of [r=>r.steps[0].exitCode=1,r=>r.cleanup.errors.push('owned resource remains'),r=>delete r.cleanup.registryTrustRemoved]){const r=valid();mutate(r);assert.throws(()=>validateManagedDeploymentReport(r))}
})
