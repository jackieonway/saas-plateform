import assert from 'node:assert/strict'
import {randomUUID,createHash} from 'node:crypto'
const sha=b=>createHash('sha256').update(b).digest('hex')

export async function remaining(c){
 const {owner,privateRoot,tools,network,report,containers,docker,inspect,exec,linux,privateFile,copyToTools,pass,origin,policy,admin,adminToken,app,application,jobsPath,releasesPath,sql,request,api,newRelease,intent,job,current,agent,good1,good2,release1,release2,firstJob,secondJob,badImageRelease,badImage,badHealth,unhealthyRelease,oldChunk}=c
 await api(jobsPath+'/'+firstJob.id+'/cleanup',{},adminToken,409)
 await api(jobsPath+'/'+secondJob.id+'/cleanup',{},adminToken,409)
 const staging=await newRelease('bad-image','staging');assert(staging.id!==badImageRelease.id)
 assert.equal(await sql(`SELECT ever_ready FROM managed_image_artifacts WHERE application_id='${app}' AND version='bad-image'`),'f')
 await api(jobsPath+'/'+badImage.id+'/cleanup',{},adminToken,409)
 await api(jobsPath+'/'+badHealth.id+'/retry',{},adminToken)
 await api(jobsPath+'/'+badHealth.id+'/cleanup',{},adminToken,409)
 assert.equal((await agent()).errorCode,'UNHEALTHY')
 const sentinel=owner+'-sentinel';containers.add(sentinel)
 await docker(['create','--name',sentinel,'--label','saas.ci.owner='+owner,'--network',network,'--entrypoint','nginx',report.baseImages.nginx,'-g','daemon off;']);await docker(['start',sentinel])
 const sentinelId=(await inspect('container',sentinel)).Id
 const unhealthyName=(await docker(['ps','--all','--filter','label=saas.deployment.owner='+owner,'--filter','label=saas.deployment.version=unhealthy','--format','{{.Names}}'])).trim();assert(unhealthyName)
 const cleanup=await api(jobsPath+'/'+badHealth.id+'/cleanup',{},adminToken)
 const leased=await api('/api/deployment-agent/'+policy.id+'/claim',{},c.agentToken)
 assert.equal(leased.operation,'CLEANUP');assert.equal(leased.jobId,cleanup.id)
 await api(releasesPath,{environment:'staging',version:'unhealthy',entryType:'HTML',entryUrl:origin+'/sub-apps/stores/versions/unhealthy/',healthUrl:origin+'/sub-apps/stores/versions/unhealthy/healthz',resources:[{kind:'SCRIPT',url:origin+'/sub-apps/stores/versions/unhealthy/app.js'}]},adminToken,409)
 for(const entryUrl of ['/sub-apps/stores/versions/unhealthy/',origin+'/sub-apps/%73tores/versions/unhealthy/'])await api(releasesPath,{environment:'staging',version:'unhealthy',entryType:'HTML',entryUrl,resources:[]},adminToken,409)
 assert.equal(await sql(`SELECT count(*) FROM application_releases WHERE application_id='${app}' AND version='unhealthy' AND environment='staging'`),'0')
 await sql(`UPDATE managed_deployment_jobs SET lease_until=now()-interval '1 second' WHERE id='${cleanup.id}'`)
 assert.equal((await agent()).state,'CLEANED');assert.equal((await job(cleanup.id)).state,'CLEANED')
 assert.equal(await docker(['ps','--all','--filter','name=^'+unhealthyName+'$','--format','{{.Names}}']),'')
 assert.equal((await inspect('container',sentinel)).Id,sentinelId)
 assert.equal((await request('/sub-apps/stores/versions/1.0.0/app.js')).text,oldChunk)
 const collisionRelease=await newRelease('collision'),collision=await api(jobsPath,intent(collisionRelease,good1),adminToken)
 const name='sd-'+policy.id+'-stores-'+sha(policy.id+'\nstores\ncollision\n'+good1).slice(0,20);containers.add(name)
 await docker(['create','--name',name,'--label','saas.ci.owner='+owner,'--label','saas.deployment.owner=foreign-'+owner,'--network',network,'--entrypoint','nginx',report.baseImages.nginx,'-g','daemon off;'])
 const foreignId=(await inspect('container',name)).Id
 assert.equal((await agent()).errorCode,'OWNERSHIP_CONFLICT')
 const refused=await api(jobsPath+'/'+collision.id+'/cleanup',{},adminToken);assert(refused.id)
 assert.equal((await agent()).errorCode,'OWNERSHIP_CONFLICT');assert.equal((await inspect('container',name)).Id,foreignId)
 pass('orphan-cleanup-preserves-unowned-resources',{orphanCleanupJobId:cleanup.id,orphanContainerRemoved:unhealthyName,sentinelIdUnchanged:sentinelId,collidingForeignContainerUnchanged:foreignId,foreignCleanupError:'OWNERSHIP_CONFLICT',inFlightNewEnvironmentRegistration:409,relativeAndEncodedReferenceRegistration:409,newReferenceRows:0,oldActiveChunkStillOnline:true})
 // An actual browser submits the API request twice after losing its first response.
 const third=await newRelease('3.0.0'),systemId=await sql("SELECT id FROM applications WHERE code='system'")
 await privateFile('browser.json',JSON.stringify({origin,session:admin.accessToken,refreshToken:admin.refreshToken,systemId,applicationId:app,applicationName:application.name,releaseId:third.id,imageRef:good2,targetId:policy.id,templateId:'static-nginx'}))
 await copyToTools('browser.json','/runtime/browser.json')
 for(const database of ['/root/.pki/nssdb','/root/.local/share/pki/nssdb']){
  await exec(['mkdir','-p',database]);await exec(['sh','-c','test -f "$1/cert9.db" || certutil -N --empty-password -d "sql:$1"','phase36-nss',database]);await exec(['certutil','-A','-d','sql:'+database,'-n',owner,'-t','C,,','-i','/runtime/ca.pem'])
 }
 const runBrowser=async(mode,product='chromium')=>docker(['exec','--env','SAAS_TEST_BROWSER='+product,'--env','NODE_EXTRA_CA_CERTS=/runtime/ca.pem',tools,'node','tests/phase36-managed-browser.mjs',mode],{timeout:180000})
 await runBrowser('--submit')
 const submitted=JSON.parse(await exec(['cat','/workspace/target/phase36-browser-result.json']));assert.equal(submitted.status,'PASS');assert.equal(submitted.requests.length,2);assert.equal(submitted.requests[0].requestId,submitted.requests[1].requestId)
 assert.equal(await sql(`SELECT count(*) FROM managed_deployment_jobs WHERE release_id='${third.id}'`),'1')
 const thirdJob=(await api(jobsPath,undefined,adminToken)).find(j=>j.releaseId===third.id);assert(thirdJob);assert.equal((await agent()).state,'READY')
 await api(jobsPath+'/'+thirdJob.id+'/cleanup',{},adminToken,409)
 const products=[]
 for(const product of ['chromium','chrome','msedge']){await runBrowser('--verify',product);const r=JSON.parse(await exec(['cat','/workspace/target/phase36-browser-result.json']));assert.equal(r.status,'PASS');assert.equal(r.product,product);assert.equal(r.ignoreTlsErrors,false);products.push({product,version:r.version,status:r.status})}
 report.browser={status:'PASS',ignoreTlsErrors:false,products,unknownCreateRecovered:true,duplicateRequestId:submitted.requests[0].requestId,realJobId:thirdJob.id}
 await exec(['node','-e',`require('node:fs').writeFileSync('/workspace/target/phase36-browser-result.json',JSON.stringify(${JSON.stringify(report.browser)}))`])
 pass('real-management-ui',{products,jobId:thirdJob.id,actualPostRequests:2,sameRequestId:true,actualJobRows:1,readyObservedFromServer:true})
 pass('reference-safe-cleanup',{active:409,previous:409,otherEnvironment:409,inFlight:409,retentionWindow:409,crossEnvironmentReleaseId:staging.id,retentionReleaseId:third.id,activeAndPreviousStillOnline:true})
 // Fail the critical audit write and prove no intent/artifact/binding is committed.
 const audited=await newRelease('audit'),auditIntent=intent(audited,good1)
 const counts=()=>sql(`SELECT json_build_object('jobs',(SELECT count(*) FROM managed_deployment_jobs),'artifacts',(SELECT count(*) FROM managed_image_artifacts),'bindings',(SELECT count(*) FROM managed_deployment_bindings))`)
 const before=JSON.parse(await counts())
 await sql("CREATE FUNCTION phase36_reject_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='MANAGED_DEPLOYMENT_CREATE' THEN RAISE EXCEPTION 'owned audit fault'; END IF; RETURN NEW; END $$; CREATE TRIGGER phase36_audit_fault BEFORE INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION phase36_reject_audit();")
 await api(jobsPath,auditIntent,adminToken,500)
 assert.deepEqual(JSON.parse(await counts()),before)
 await sql('DROP TRIGGER phase36_audit_fault ON audit_logs; DROP FUNCTION phase36_reject_audit();')
 const after=await api(jobsPath,auditIntent,adminToken);assert(after.id);assert.equal((await agent()).state,'READY')
 const auditCount=Number(await sql("SELECT count(*) FROM audit_logs WHERE action IN ('MANAGED_DEPLOYMENT_CREATE','MANAGED_DEPLOYMENT_CLAIM','MANAGED_DEPLOYMENT_RECEIPT','MANAGED_DEPLOYMENT_RETRY','MANAGED_DEPLOYMENT_CLEANUP')"));assert(auditCount>=20)
 pass('transactional-audit-failure',{failedAuditStatus:500,noPartialJobsOrArtifactsOrBindings:true,sameRequestRecovered:true,auditRows:auditCount})
 const final=(await current())[0];assert.equal(final.activeReleaseId,release1.id);assert.equal(final.previousReleaseId,release2.id)
 assert.equal((await request('/sub-apps/stores/versions/1.0.0/app.js')).text,oldChunk)
 const platformLog=await docker(['exec','--user','10001:10001',tools,'cat','/runtime/platform.log']);for(const secret of c.secretValues)if(secret)assert(!platformLog.includes(secret),'Private value leaked in platform log')
 report.database={migrations:JSON.parse(await sql('SELECT json_agg(json_build_object(\'version\',version,\'success\',success) ORDER BY version) FROM _sqlx_migrations')),finalActiveReleaseId:final.activeReleaseId,finalPreviousReleaseId:final.previousReleaseId,auditRows:auditCount}
 assert(report.database.migrations.some(m=>Number(m.version)===5&&m.success))
}
