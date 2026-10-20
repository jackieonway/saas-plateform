import assert from 'node:assert/strict'
import {randomUUID,createHash} from 'node:crypto'
const sha=b=>createHash('sha256').update(b).digest('hex')
async function wait(fn){const end=Date.now()+30000;do{if(await fn())return;await new Promise(r=>setTimeout(r,100))}while(Date.now()<end);throw Error('node reload did not converge')}
import {readFileSync,writeFileSync} from 'node:fs'
import path from 'node:path'
export async function acceptance(c){
 const {owner,out,tools,report,secretValues,docker,exec,privateFile,copyToTools,pass,streamToFile,origin,request,nodeRequest,nodeNames,policy,tokens,config,sql,api,admin,token,app,releases,jobs,upload,intent,job,terminal,current,r1,created}=c
 const platformExec=args=>docker(['exec','--user','10001:10001',tools,...args])
 const target='/data/sub-apps/stores/versions/1.0.0/app.js'
 const original=(await nodeRequest(1,'/sub-apps/stores/versions/1.0.0/app.js')).text
 await docker(['exec','--user','1000:1000',nodeNames[1],'sh','-c','printf "%s" corrupted > "$1"','phase35-corrupt',target])
 await api(releases+'/'+r1.id+'/activate',{},token,409);assert.equal((await current())[0].activeReleaseId,r1.id)
 await api(jobs+'/'+created.id+'/retry',{},token);assert.equal((await terminal(created.id)).state,'FAILED')
 assert.equal((await nodeRequest(1,'/sub-apps/stores/versions/1.0.0/app.js')).text,'corrupted')
 // The controller repairs only its injected fault. The installer refuses overwriting it.
 await docker(['exec','--user','1000:1000',nodeNames[1],'sh','-c','printf "%s" "$2" > "$1"','phase35-restore',target,original]);await api(jobs+'/'+created.id+'/retry',{},token);assert.equal((await terminal(created.id)).state,'READY')
 await docker(['exec','--user','1000:1000',nodeNames[1],'mv',target,target+'.saved']);await api(releases+'/'+r1.id+'/preflight',{},token,409);await docker(['exec','--user','1000:1000',nodeNames[1],'mv',target+'.saved',target]);await api(jobs+'/'+created.id+'/retry',{},token);assert.equal((await terminal(created.id)).state,'READY')
 await platformExec(['node','-e',"require('fs').writeFileSync('/runtime/node2.token','f'.repeat(80))"])
 await api(releases+'/'+r1.id+'/activate',{},token,409);assert.equal((await job(created.id)).nodes.find(n=>n.id==='node2').errorCode,'NODE_AUTH_FAILED');assert.equal((await current())[0].activeReleaseId,r1.id)
 await platformExec(['node','-e',`require('fs').writeFileSync('/runtime/node2.token',${JSON.stringify(tokens[1])})`]);await api(jobs+'/'+created.id+'/retry',{},token);assert.equal((await terminal(created.id)).state,'READY')
 pass('live-file-verification',{previousPreflightPassed:true,corruptActivation:409,missingPreflight:409,activeUnchanged:true,retryDidNotOverwriteCorruption:true,platformCredentialFailureBlocksActivation:true})
 const stored=JSON.parse(await sql(`SELECT json_build_object('manifest',manifest,'attempt',attempt,'nonce',nonce,'revision',policy_revision) FROM zip_distribution_jobs WHERE id='${created.id}'`))
 const work={schemaVersion:1,jobId:created.id,attempt:stored.attempt,nonce:randomUUID(),groupId:policy.id,policyRevision:stored.revision,nodeId:'node1',applicationCode:'stores',version:'1.0.0',artifactSha256:stored.manifest.artifactSha256}
 const verify={work,manifest:stored.manifest}
 assert.equal((await nodeRequest(0,'/agent/verify',{method:'POST',token:tokens[0],body:{...verify,command:['sh']}})).status,400)
 assert.equal((await nodeRequest(1,'/agent/verify',{method:'POST',token:tokens[1],body:verify})).status,400)
 const payload={manifest:stored.manifest,files:[]};for(const f of stored.manifest.files){const r=await nodeRequest(0,'/sub-apps/stores/versions/1.0.0/'+f.path);payload.files.push({path:f.path,dataBase64:r.bytes.toString('base64')})}
 const canonicalPayload=structuredClone(payload)
 payload.files[0].dataBase64=Buffer.from('different').toString('base64')
 assert.equal((await nodeRequest(0,'/agent/install',{method:'POST',token:tokens[0],body:{work,payload}})).status,409)
 const changed=Buffer.from('valid but different artifact');payload.files[0].dataBase64=changed.toString('base64');payload.manifest.files[0].size=changed.length;payload.manifest.files[0].sha256=sha(changed)
 const identity={schemaVersion:1,entryType:'HTML',entry:'index.html',files:payload.manifest.files.map(f=>({path:f.path,size:f.size,sha256:f.sha256}))};payload.manifest.artifactSha256=sha(JSON.stringify(identity))
 assert.equal((await nodeRequest(0,'/agent/install',{method:'POST',token:tokens[0],body:{work:{...work,artifactSha256:payload.manifest.artifactSha256},payload}})).status,409)
 assert.equal((await nodeRequest(0,'/sub-apps/stores/versions/1.0.0/app.js')).text,original)
 pass('immutable-conflict',{unknownFields:400,wrongNode:400,wrongDigest:409,validConflictingArtifact:409,existingFilesUnchanged:true})
 // Truncated transfer is rejected before any directory can be published.
 const interrupted={work:{...work,version:'interrupted'},payload:canonicalPayload}
 assert.equal((await nodeRequest(0,'/agent/install',{method:'POST',token:tokens[0],body:Buffer.from(JSON.stringify(interrupted).slice(0,-20))})).status,400)
 assert.equal((await nodeRequest(0,'/sub-apps/stores/versions/interrupted/index.html')).status,404)
 await docker(['exec','--user','1000:1000',nodeNames[0],'sh','-c','mkdir -p /data/sub-apps/.staging/controller-partial; printf partial > /data/sub-apps/.staging/controller-partial/index.html'])
 assert.equal((await nodeRequest(0,'/sub-apps/.staging/controller-partial/index.html')).status,404)
 const installed=await nodeRequest(0,'/agent/install',{method:'POST',token:tokens[0],body:{work:{...work,version:'stage-check'},payload:canonicalPayload}});assert.equal(installed.status,200)
 assert.equal(await docker(['exec','--user','1000:1000',nodeNames[0],'cat','/data/sub-apps/.staging/controller-partial/index.html']),'partial')
 pass('private-staging-and-partial-transfer',{truncatedInstall:400,unpublishedVersion:404,privateStage:404,independentStageNotRemoved:true,completeInstall:200})
 const nginxOriginal=(await c.linux(['cat',c.privateRoot+'/node1/nginx.conf'])).output
 const validReceipt=JSON.parse(await sql(`SELECT receipt FROM zip_distribution_nodes WHERE job_id='${created.id}' AND node_id='node1'`))
 for(const kind of ['stale-nonce','foreign-node']){
  const fake=structuredClone(validReceipt);if(kind==='stale-nonce')fake.work.nonce=randomUUID();else fake.work.nodeId='node2'
  const location=`location = /agent/verify {default_type application/json; return 200 '${JSON.stringify(fake)}';} location /agent/`
  await privateFile('node1/nginx.conf',nginxOriginal.replace('location /agent/',location),'644');await docker(['exec','--user','1000:1000',nodeNames[0],'nginx','-c','/runtime/nginx.conf','-s','reload'])
  await wait(async()=>{const r=await nodeRequest(0,'/agent/verify',{method:'POST',token:tokens[0],body:{work,manifest:canonicalPayload.manifest}});return r.value?.work?.nonce===fake.work.nonce&&r.value?.work?.nodeId===fake.work.nodeId});await api(releases+'/'+r1.id+'/activate',{},token,409);assert.equal((await job(created.id)).nodes.find(n=>n.id==='node1').errorCode,'UNTRUSTED_RECEIPT')
  await privateFile('node1/nginx.conf',nginxOriginal,'644');await docker(['exec','--user','1000:1000',nodeNames[0],'nginx','-c','/runtime/nginx.conf','-s','reload']);await wait(async()=>{const r=await nodeRequest(0,'/agent/verify',{method:'POST',token:tokens[0],body:{work,manifest:canonicalPayload.manifest}});return r.value?.work?.nonce===work.nonce&&r.value?.work?.nodeId==='node1'});await api(jobs+'/'+created.id+'/retry',{},token);assert.equal((await terminal(created.id)).state,'READY')
 }
 pass('stale-and-foreign-receipts',{actualHttpsProxyFaults:2,receiptError:'UNTRUSTED_RECEIPT',activation:409,activeUnchanged:(await current())[0].activeReleaseId===r1.id})
 const r2=await upload('2.0.0'),two=await api(jobs,intent(r2),token);assert.equal((await terminal(two.id)).state,'READY');assert.equal((await api(releases+'/'+r2.id+'/preflight',{},token)).validationStatus,'PASSED');await api(releases+'/'+r2.id+'/activate',{},token)
 assert.equal((await current())[0].activeReleaseId,r2.id)
 await api('/api/applications/'+app.id+'/deployments/development/rollback',{},token)
 assert.equal((await current())[0].activeReleaseId,r1.id);for(const v of ['1.0.0','2.0.0'])assert.equal((await request('/sub-apps/stores/versions/'+v+'/app.js')).status,200)
 pass('activate-and-rollback',{v2Activated:true,v1Restored:true,bothOldResourceUrls:200})
 const foreign=await api('/api/applications',{code:'alias-stores',name:'隔离别名归属检查',entryType:'HTML',entryUrl:origin+'/sub-apps/stores/versions/1.0.0/index.html',basePath:'/micro/alias-stores',runtimeMode:'IFRAME',status:'ENABLED',autoCreateMenu:false},token)
 const aliasReleases='/api/applications/'+foreign.id+'/releases'
 const aliases=[]
 for(const [environment,entryUrl]of [['relative-alias','/sub-apps/stores/versions/1.0.0/index.html'],['encoded-alias',origin+'/sub-apps/%73tores/versions/1.0.0/index.html'],['dot-alias',origin+'/sub-apps/stores/versions/declared/../1.0.0/index.html']]){
  const r=await api(aliasReleases,{environment,version:'1.0.0',entryType:'HTML',entryUrl,resources:[]},token);await api(aliasReleases+'/'+r.id+'/preflight',{},token,409);await api(aliasReleases+'/'+r.id+'/activate',{},token,409);aliases.push(environment)
 }
 const empty={groups:[]};await platformExec(['node','-e',`require('fs').writeFileSync('/runtime/zip.json',JSON.stringify(${JSON.stringify(empty)}))`]);await api(releases+'/'+r1.id+'/activate',{},token,409);assert.equal((await request('/sub-apps/stores/versions/1.0.0/app.js')).status,200)
 await platformExec(['node','-e',`require('fs').writeFileSync('/runtime/zip.json',JSON.stringify(${JSON.stringify(config)}))`])
 pass('historical-alias-gates',{aliasesRejected:aliases,missingConfigurationCannotBypass:true,protectedHistoricalResource:200})
 const audited=await upload('audit'),body=intent(audited),before=await sql('SELECT count(*) FROM zip_distribution_jobs')
 await sql("CREATE FUNCTION phase35_reject_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='ZIP_DISTRIBUTION_CREATE' THEN RAISE EXCEPTION 'owned audit fault'; END IF; RETURN NEW; END $$; CREATE TRIGGER phase35_fault BEFORE INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION phase35_reject_audit();")
 await api(jobs,body,token,500);assert.equal(await sql('SELECT count(*) FROM zip_distribution_jobs'),before);assert.equal(await sql(`SELECT count(*) FROM zip_distribution_nodes n LEFT JOIN zip_distribution_jobs j ON j.id=n.job_id WHERE j.id IS NULL`),'0')
 await sql('DROP TRIGGER phase35_fault ON audit_logs; DROP FUNCTION phase35_reject_audit();');const auditJob=await api(jobs,body,token);assert.equal((await terminal(auditJob.id)).state,'READY')
 pass('audit-rollback',{failedAudit:500,noPartialRows:true,sameIntentRecovered:true})
 await sql(`UPDATE zip_distribution_jobs SET state='RUNNING',nonce=gen_random_uuid(),lease_until=now()-interval '1 second' WHERE id='${auditJob.id}'`);const recovered=await terminal(auditJob.id);assert.equal(recovered.state,'READY');assert.equal(recovered.attempt,2)
 const receipt=JSON.parse(await sql(`SELECT receipt FROM zip_distribution_nodes WHERE job_id='${auditJob.id}' ORDER BY node_id LIMIT 1`));assert.equal(receipt.work.attempt,2);assert.equal(receipt.operation,'VERIFY')
 pass('lease-recovery',{expiredLeaseReclaimed:true,attempt:2,existingNodesVerified:true})
 const third=await upload('3.0.0'),systemId=await sql("SELECT id FROM applications WHERE code='system'")
 await privateFile('browser.json',JSON.stringify({origin,session:admin.accessToken,refreshToken:admin.refreshToken,systemId,applicationId:app.id,applicationName:app.name,releaseId:third.id,groupId:policy.id,archiveBase64:c.archive('3.0.0').toString('base64')}));await copyToTools('browser.json','/runtime/browser.json')
 for(const database of ['/root/.pki/nssdb','/root/.local/share/pki/nssdb']){await exec(['mkdir','-p',database]);await exec(['sh','-c','test -f "$1/cert9.db" || certutil -N --empty-password -d "sql:$1"','phase35-nss',database]);await exec(['certutil','-A','-d','sql:'+database,'-n',owner,'-t','C,,','-i','/runtime/ca.pem'])}
 const runBrowser=async(mode,product='chromium')=>docker(['exec','--env','SAAS_TEST_BROWSER='+product,'--env','NODE_EXTRA_CA_CERTS=/runtime/ca.pem',tools,'node','tests/phase35-zip-browser.mjs',mode],{timeout:180000})
 await runBrowser('--submit');const submitted=JSON.parse(await exec(['cat','/workspace/target/phase35-browser-result.json']));assert.equal(submitted.draftUploadWaiting,true);assert.equal(submitted.requests.length,2);assert.equal(submitted.requests[0].requestId,submitted.requests[1].requestId)
 assert.equal(await sql(`SELECT count(*) FROM zip_distribution_jobs WHERE release_id='${third.id}'`),'1');const thirdJob=(await api(jobs,undefined,token)).jobs.find(j=>j.releaseId===third.id);assert.equal((await terminal(thirdJob.id)).state,'READY')
 const products=[];for(const product of ['chromium','chrome','msedge']){await runBrowser('--verify',product);const b=JSON.parse(await exec(['cat','/workspace/target/phase35-browser-result.json']));products.push({product,version:b.version,status:b.status})}
 report.browser={status:'PASS',ignoreTlsErrors:false,products,unknownCreateRecovered:true};pass('real-management-ui',{products,postRequests:2,sameRequestId:true,jobRows:1,draftUploadWaiting:submitted.draftUploadWaiting})
 for(const name of ['phase35-managed-ui.png','phase35-ready.png'])await streamToFile(['docker','--host','unix:///var/run/docker.sock','cp',tools+':/workspace/target/'+name,'-'],path.join(out,'screenshots',name+'.tar'))
 const log=await platformExec(['cat','/runtime/platform.log']);for(const s of secretValues)if(s)assert(!log.includes(s))
 report.database={migrations:JSON.parse(await sql("SELECT json_agg(json_build_object('version',version,'success',success) ORDER BY version) FROM _sqlx_migrations")),activeReleaseId:(await current())[0].activeReleaseId};assert(report.database.migrations.some(m=>Number(m.version)===6&&m.success))
}
