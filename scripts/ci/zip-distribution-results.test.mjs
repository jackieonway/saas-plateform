import test from 'node:test'
import assert from 'node:assert/strict'
import {validateZipDistributionReport,zipDistributionChecks} from './zip-distribution-results.mjs'
function complete(){return {schemaVersion:1,kind:'zip-distribution',status:'PASS',adoptable:true,scope:'isolated-local',productionOperations:false,sourceUnchanged:true,hostSourceUnchanged:true,inputsSha256:'a'.repeat(64),sourceRevision:'b'.repeat(40),checks:zipDistributionChecks.map(id=>({id,status:'PASS',proof:{actual:true}})),steps:['current-source-fast','existing-release-regression'].map(id=>({id,status:'PASS',exitCode:0})),originalBinaries:[{name:'platform',elfMachine:62,sha256:'a'.repeat(64)},{name:'agent',elfMachine:62,sha256:'b'.repeat(64)}],browser:{status:'PASS',ignoreTlsErrors:false,products:['chromium','chrome','msedge'].map(product=>({product,status:'PASS'}))},cleanup:{cacheRemoved:true,errors:[],containersRemoved:true,networksRemoved:true,candidateImagesRemoved:true,privateDirectoryRemoved:true,registryTrustRemoved:true}}}
test('ZIP acceptance refuses missing checks, untrusted TLS, stale source and incomplete cleanup',()=>{
 assert.equal(validateZipDistributionReport(complete()).status,'PASS')
 for(const mutate of [r=>r.checks.pop(),r=>r.checks[0].status='NOT_RUN',r=>r.checks[0]=r.checks[1],r=>r.browser.ignoreTlsErrors=true,r=>r.hostSourceUnchanged=false,r=>r.adoptable=false,r=>r.cleanup.errors.push('fault'),r=>r.cleanup.containersRemoved=false,r=>r.steps[0].exitCode=1,r=>r.inputsSha256='unknown']){const r=complete();mutate(r);assert.throws(()=>validateZipDistributionReport(r))}
})
