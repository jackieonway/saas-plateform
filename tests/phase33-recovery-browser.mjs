/** Real recovery pilot. Node is only the owned test driver; product recovery is Rust. */
import assert from 'node:assert/strict'
import {spawn} from 'node:child_process'
import {randomBytes,randomUUID,createHash,createCipheriv,createDecipheriv} from 'node:crypto'
import {readFileSync,writeFileSync,mkdirSync,cpSync,chmodSync,existsSync,rmSync} from 'node:fs'
import http from 'node:http'
import https from 'node:https'
import path from 'node:path'
import {fileURLToPath} from 'node:url'
import {startPhase27Services,wait,delay} from './helpers/phase27-stores-services.mjs'
import {launchBrowser} from './browser/runtime.mjs'
import {validateOwnedPostgres} from '../scripts/ci/postgres.mjs'
const sha=b=>createHash('sha256').update(b).digest('hex'),checks=[],segments={},commands=[]
let services,browser,cleanup,failure,monitor,monitorLog='',restoredDb,plan,config,privateRoot
const contexts=[];const remote=JSON.parse(readFileSync('/remote-private.json')),remoteCA=readFileSync('/remote-ca.pem')
const checkpoint=(id,proof)=>{checks.push({id,status:'PASS',proof});console.log('PHASE33_CHECK '+id+' PASS')}
async function run(program,args,{expected=0,env=process.env}={}){
  const start=performance.now(),child=spawn(program,args,{env,stdio:['ignore','pipe','pipe']});let output='';
  child.stdout.on('data',b=>output+=b);child.stderr.on('data',b=>output+=b)
  const timer=setTimeout(()=>child.kill('SIGTERM'),300000),exit=await new Promise(r=>{child.once('error',()=>r(-1));child.once('close',r)});clearTimeout(timer)
  assert(expected===0?exit===0:exit!==0,'Owned '+path.basename(program)+' failed: '+output.slice(-900));
  return {exit,output,seconds:(performance.now()-start)/1000}
}
function privateFile(file,bytes){writeFileSync(file,bytes,{mode:0o600});chmodSync(file,0o600);return file}
async function storage(method,object,body){
  return new Promise((resolve,reject)=>{const q=https.request(remote.origin+'/objects/'+object,{method,ca:remoteCA,headers:{authorization:'Bearer '+remote.token,...(method==='PUT'?{'if-none-match':'*'}:{})}},r=>{const data=[];r.on('data',b=>data.push(b));r.on('end',()=>resolve({status:r.statusCode,body:Buffer.concat(data)}))});q.on('error',reject);q.end(body)})
}
async function oauth(fields,endpoint='/oauth/token'){
  return new Promise((resolve,reject)=>{const body=new URLSearchParams(fields).toString();const q=http.request(services.recoveryControl.platformOrigin+endpoint,{method:'POST',headers:{'content-type':'application/x-www-form-urlencoded','content-length':Buffer.byteLength(body)}},r=>{let b='';r.on('data',s=>b+=s);r.on('end',()=>resolve({status:r.statusCode,data:JSON.parse(b)}))});q.on('error',reject);q.end(body)})
}
async function cli(op,overrides={},expected=0){
  privateFile(privateRoot+'/config.json',JSON.stringify({...config,...overrides}))
  const r=await run('/owned-cache/target/debug/saas-recovery',[op,privateRoot+'/config.json'],{expected,env:{...process.env,BACKUP_WRITES_QUIESCED:'true'}})
  commands.push({operation:op,expected:expected===0?'SUCCESS':'REFUSAL',exitCode:r.exit,seconds:r.seconds})
  if(expected!==0)return r
  return {...r,value:JSON.parse(r.output.trim().split(/\r?\n/).at(-1))}
}
const codec={
  open(bytes,key){const prefix=Buffer.from('SAAS-RECOVERY-1'),nonce=bytes.subarray(prefix.length,prefix.length+12),data=bytes.subarray(prefix.length+12);const d=createDecipheriv('aes-256-gcm',key,nonce);d.setAAD(prefix);d.setAuthTag(data.subarray(-16));return JSON.parse(Buffer.concat([d.update(data.subarray(0,-16)),d.final()]))},
  seal(b,key){const prefix=Buffer.from('SAAS-RECOVERY-1'),nonce=randomBytes(12),c=createCipheriv('aes-256-gcm',key,nonce);c.setAAD(prefix);return Buffer.concat([prefix,nonce,c.update(JSON.stringify(b)),c.final(),c.getAuthTag()])}
}
try{
  validateOwnedPostgres(process.env)
  const pgEndpoint=new URL(process.env.TEST_DATABASE_URL)
  await run('psql',['-X','-A','-t','-w','-c','SELECT 1'],{env:{...process.env,PGHOST:pgEndpoint.hostname,PGPORT:pgEndpoint.port,PGDATABASE:pgEndpoint.pathname.slice(1),PGUSER:pgEndpoint.username,PGPASSWORD:pgEndpoint.password}})
  services=await startPhase27Services({recovery:true});privateRoot=services.ownedRoot+'/recovery-private';mkdirSync(privateRoot,{mode:0o700})
  const actor=services.actors.ordinary;let oldStores=actor,oldWork=await services.authorize(actor,'A','work-orders')
  const call=async(a,p,options={},status=200)=>{const r=await services.businessCall(a,p,options);assert.equal(r.status,status, p+': '+r.text.slice(0,250));return r.data}
  const store=await call(oldStores,'/stores-api/stores',{method:'POST',headers:{'idempotency-key':randomUUID()},body:{name:'恢复前门店',businessStatus:'OPEN',businessContact:'Phase33'}},201)
  const ticket=await call(oldWork,'/work-orders-api/tickets',{method:'POST',headers:{'idempotency-key':randomUUID()},body:{title:'恢复前工单',description:'coordinated recovery',priority:'NORMAL',assigneeUserId:actor.userId}},201)
  await call(oldWork,'/work-orders-api/tickets/'+ticket.id+'/remind',{method:'POST',body:{idempotencyKey:'phase33-before-backup'}})
  const inboxBefore=await services.platformApi('/api/notifications',{token:actor.session})
  assert(inboxBefore.records.length>0)
  // Ordinary OAuth tokens are actually issued by Rust; pending one-time credentials
  // use the same database fixture seam as platform OAuth integration tests.
  const client=await services.adminApi('/api/oauth/clients','POST',{clientName:'Phase33 recovery OAuth',clientType:'CONFIDENTIAL',grantTypes:['authorization_code','refresh_token','urn:ietf:params:oauth:grant-type:device_code'],scopes:['openid'],redirectUris:['https://localhost/callback'],accessTokenValidity:3600,refreshTokenValidity:86400})
  const auth={client_id:client.clientId,client_secret:client.clientSecret}
  async function code(){const raw=randomBytes(32).toString('hex');await services.platformDb.query("INSERT INTO oauth_authorization_codes(code_hash,client_id,user_id,tenant_id,organization_id,redirect_uri,scope,expires_at) VALUES($1,$2,$3,$4,$5,'https://localhost/callback','openid',now()+interval '1 hour')",[sha(raw),client.clientId,actor.userId,actor.tenantId,actor.organizationId]);return raw}
  const issued=await oauth({...auth,grant_type:'authorization_code',code:await code(),redirect_uri:'https://localhost/callback'});assert.equal(issued.status,200);assert(issued.data.access_token&&issued.data.refresh_token)
  assert.equal((await oauth({...auth,token:issued.data.access_token},'/oauth/introspect')).data.active,true)
  const pendingCode=await code(),device=randomBytes(32).toString('hex')
  await services.platformDb.query("INSERT INTO oauth_device_codes(device_code_hash,user_code,client_id,user_id,tenant_id,organization_id,scope,status,interval_seconds,expires_at) VALUES($1,$2,$3,$4,$5,$6,'openid','APPROVED',0,now()+interval '1 hour')",[sha(device),randomBytes(8).toString('hex'),client.clientId,actor.userId,actor.tenantId,actor.organizationId])
  const requestId=(await services.platformDb.query("INSERT INTO oauth_authorization_requests(client_id,redirect_uri,scope,expires_at) VALUES($1,'https://localhost/callback','openid',now()+interval '1 hour') RETURNING id",[client.clientId])).rows[0].id
  assert.equal((await services.request(services.recoveryControl.platformOrigin,'/api/oauth/consent/'+requestId,{token:actor.session})).status,200)
  checkpoint('live-credentials-and-business-before-backup',{ordinaryOAuthIssued:true,pendingConsentValid:true,notificationRows:inboxBefore.records.length})
  // Second real builds, with immutable v2 entry URLs and their own build metadata.
  const releases=services.ownedRoot+'/releases',versionReleases={}
  for(const app of ['stores','work-orders']){
    const dist=services.ownedRoot+'/'+app+'-v2-app'
    await run(process.execPath,['business-apps/'+app+'/frontend/scripts/build.mjs','--out-dir',dist,'--version','v2','--allowed-host-origin',services.origin])
    await run(process.execPath,['scripts/package-microapp.mjs','--app',app,'--environment','production','--version','v2','--input',dist,'--output-root',releases])
    const manifest=JSON.parse(readFileSync(path.join(releases,app,'versions/v2/release.json')))
    const r=await services.adminApi('/api/applications/'+services.apps[app].application.id+'/releases','POST',manifest)
    await services.adminApi('/api/applications/'+services.apps[app].application.id+'/releases/'+r.id+'/preflight','POST')
    versionReleases[app]=r
    for(const[v,source]of [['v1',services.ownedRoot+'/'+app+'-app'],['v2',dist]]){const dest='/image-inputs/'+app+'/'+v;mkdirSync(path.dirname(dest),{recursive:true});cpSync(source,dest,{recursive:true})}
  }
  console.log('PHASE33_BUILD_IMAGES_READY')
  await wait(()=>existsSync('/image-archives/lock.json'),'Managed owner must create four actual image archives',600000)
  const imageLock=JSON.parse(readFileSync('/image-archives/lock.json'))
  assert.equal(imageLock.length,4)
  for(const image of imageLock){const archive='/image-archives/'+image.app+'-'+image.version+'.tar';assert.equal(sha(readFileSync(archive)),image.archiveSha256)}
  oldStores=await services.authorize(actor,'A','stores');oldWork=await services.authorize(actor,'A','work-orders')
  const stopStart=performance.now();plan=await services.recoveryControl.quiesce();segments.drainSeconds=(performance.now()-stopStart)/1000
  const artifacts=privateRoot+'/artifacts';mkdirSync(artifacts,{mode:0o700});cpSync('/image-archives',artifacts+'/images',{recursive:true})
  cpSync(plan.host,artifacts+'/host',{recursive:true});cpSync(plan.releases,artifacts+'/runtime/releases',{recursive:true})
  mkdirSync(artifacts+'/routes',{recursive:true});for(const f of plan.configs)cpSync(f,artifacts+'/routes/'+path.basename(path.dirname(f))+'.conf')
  cpSync(plan.configs[2],artifacts+'/routes/nginx.conf')
  for(const app of ['stores','work-orders'])for(const v of ['v1','v2'])cpSync('/image-inputs/'+app+'/'+v,artifacts+'/'+app+'/'+v,{recursive:true})
  mkdirSync(artifacts+'/runtime/bin',{recursive:true})
  const nativeImages=[]
  for(const[name,file]of Object.entries(plan.binaries)){const dest=artifacts+'/runtime/bin/'+name;cpSync(file,dest);await run('strip',['--strip-debug',dest]);chmodSync(dest,0o755);nativeImages.push({component:name,originalSha256:sha(readFileSync(file)),restorableSha256:sha(readFileSync(dest)),transform:'strip --strip-debug'})}
  privateFile(artifacts+'/runtime/native-lock.json',JSON.stringify(nativeImages))
  const serviceFile=privateRoot+'/pg-services.conf',service=(name,url)=>{const u=new URL(url);return '['+name+']\nhost='+u.hostname+'\nport='+u.port+'\ndbname='+u.pathname.slice(1)+'\nuser='+u.username+'\npassword='+u.password+'\n'}
  privateFile(serviceFile,service('admin',plan.adminUrl)+plan.databases.map(d=>service(d.kind,d.sourceUrl)).join(''))
  // A separate key escrow path is retrieved after the simulated loss; no key enters
  // the remote object store, public archive, argv or frontend artifact collection.
  const escrow=privateRoot+'/key-escrow';mkdirSync(escrow,{mode:0o700});const key=randomBytes(32);privateFile(escrow+'/master',key)
  privateFile(privateRoot+'/remote-token',remote.token);privateFile(privateRoot+'/monitor-token',remote.token)
  const monitorConfig={bind:'0.0.0.0:9098',interval_ms:1000,timeout_ms:500,failures:1,recoveries:1,metrics_token:randomBytes(32).toString('hex'),event_token:remote.token,targets:[{component:'platform',url:services.recoveryControl.platformOrigin+'/api/health'}]}
  privateFile(privateRoot+'/monitor.json',JSON.stringify(monitorConfig));monitor=spawn('/owned-cache/target/debug/saas-monitor',[privateRoot+'/monitor.json'],{stdio:['ignore','pipe','pipe']});monitor.stdout.on('data',b=>monitorLog+=b);monitor.stderr.on('data',b=>monitorLog+=b)
  await wait(async()=>{try{return(await fetch('http://127.0.0.1:9098/internal/metrics',{headers:{authorization:'Bearer '+monitorConfig.metrics_token}})).status===200}catch{return false}},'Actual Rust monitor readiness')
  config={profile:'pilot',service_file:serviceFile,admin_service:'admin',databases:plan.databases.map(d=>({kind:d.kind,source_service:d.kind,target:d.target,owner:d.owner})),key_file:escrow+'/master',scratch:'/scratch',local_bundle:privateRoot+'/set.enc',artifact_root:artifacts,restored_artifacts:privateRoot+'/restored',remote_url:remote.origin+'/objects/pilot',remote_token_file:privateRoot+'/remote-token',ca_file:'/remote-ca.pem',monitor_url:remote.origin+'/internal/events',monitor_token_file:privateRoot+'/monitor-token',sequence_file:privateRoot+'/sequence',timeout_seconds:180}
  const backed=await cli('backup');segments.backupSeconds=backed.seconds
  assert.equal(backed.value.remote_retrieval_authenticated,true)
  const encrypted=readFileSync(config.local_bundle);assert(!encrypted.includes(Buffer.from('恢复前门店'))&&!encrypted.includes(Buffer.from(plan.databases[0].sourceName)))
  const downloaded=await storage('GET','pilot');assert.equal(downloaded.status,200);assert.equal(sha(downloaded.body),sha(encrypted))
  checkpoint('coordinated-encrypted-remote-set',{remoteSha256:sha(downloaded.body),bytes:downloaded.body.length,recoveryPoint:backed.value.recovery_point,entries:backed.value.entries,imageLock,nativeImages})
  // Negatives run against authenticated remote objects, before creating any target.
  const negatives=[]
  async function bad(name,bytes,overrides={},operation='restore'){
    if(bytes!==null){assert.equal((await storage('PUT',name,bytes)).status,201);overrides.remote_url=remote.origin+'/objects/'+name}
    const before=(await services.platformDb.query('SELECT count(*)::int AS n FROM pg_database WHERE datname=ANY($1)',[plan.databases.map(d=>d.target)])).rows[0].n
    const r=await cli(operation,overrides,1)
    const after=(await services.platformDb.query('SELECT count(*)::int AS n FROM pg_database WHERE datname=ANY($1)',[plan.databases.map(d=>d.target)])).rows[0].n
    assert.equal(before,after);negatives.push({name,exit:r.exit,noTargetCreated:true})
  }
  await bad('empty',Buffer.alloc(0));const corrupted=Buffer.from(encrypted);corrupted[corrupted.length-1]^=1;await bad('corrupt',corrupted)
  privateFile(privateRoot+'/wrong-key',randomBytes(32));await bad('wrong-key',null,{key_file:privateRoot+'/wrong-key'})
  const missing=codec.open(encrypted,key);missing.entries=missing.entries.filter(e=>e.path!=='artifacts/images/stores-v2.tar');await bad('missing-artifact',codec.seal(missing,key))
  await bad('same-target',null,{databases:[config.databases[0],{...config.databases[1],target:config.databases[0].target},config.databases[2]]})
  const existing=plan.databases[0].target
  // Explicit fixture-owned existing target, retained by the product refusal.
  await services.platformDb.query('SELECT 1');
  const {createRequire}=await import('node:module');const pg=createRequire(new URL('../mini-services/notification-ws/package.json',import.meta.url))('pg');const control=new pg.Client({connectionString:plan.adminUrl});await control.connect()
  await control.query('CREATE DATABASE "'+existing+'"');await bad('existing-target',null)
  assert.equal((await control.query('SELECT count(*)::int AS n FROM pg_database WHERE datname=$1',[existing])).rows[0].n,1)
  await control.query('DROP DATABASE "'+existing+'"')
  const mismatched=codec.open(encrypted,key);mismatched.databases.find(d=>d.kind==='platform').migrations+='intentional-mismatch'
  assert.equal((await storage('PUT','migration-mismatch',codec.seal(mismatched,key))).status,201)
  const failedTargets=config.databases.map(d=>({...d,target:d.target+'_failed'}))
  await cli('restore',{databases:failedTargets,remote_url:remote.origin+'/objects/migration-mismatch'},1)
  const retained=(await control.query('SELECT datname FROM pg_database WHERE datname=ANY($1)',[failedTargets.map(d=>d.target)])).rows
  assert.equal(retained.length,1);assert.equal(retained[0].datname,failedTargets[0].target)
  // Only the test owner disposes this demonstrated failed/quarantined target.
  await control.query('DROP DATABASE "'+retained[0].datname+'" WITH (FORCE)');await control.end()
  checkpoint('six-negative-refusals',{negatives,partialRestoreFailureRetained:true,operatorTestCleanup:true})
  const stale=codec.open(encrypted,key);stale.recovery_point=Math.floor(Date.now()/1000)-86401;await bad('stale',codec.seal(stale,key),{},'age')
  await cli('age');await cli('verify');await cli('backup',{},1)
  assert(monitorLog.includes('"component":"backup_run","event":"DOWN"'))
  assert(monitorLog.includes('"component":"backup_verify","event":"DOWN"')&&monitorLog.includes('"component":"backup_verify","event":"RECOVERED"'))
  assert(monitorLog.includes('"component":"backup_age","event":"DOWN"')&&monitorLog.includes('"component":"backup_age","event":"RECOVERED"'))
  const scheduled=await cli('backup-new');segments.scheduledBackupSeconds=scheduled.seconds;segments.latestRecoveryPointUnixSeconds=scheduled.value.recovery_point;segments.remoteVerifiedAtUnixMs=Date.now()
  assert(monitorLog.includes('"component":"backup_run","event":"RECOVERED"'))
  const latest=JSON.parse(readFileSync(privateRoot+'/sequence.latest.json'))
  assert.notEqual(latest.remote_url,config.remote_url);assert.notEqual(latest.local_bundle,config.local_bundle)
  assert.equal((await storage('GET',new URL(latest.remote_url).pathname.split('/').at(-1))).status,200)
  checkpoint('actual-backup-alert-events',{events:monitorLog.trim().split(/\r?\n/).map(l=>{try{return JSON.parse(l)}catch{return {parseFailure:true}}}),externalDelivery:false,timerInstalled:false})
  // Simulate losing same-host encrypted cache and serving artifacts. Recovery now
  // depends on retrieving the remote object and the separately stored key.
  rmSync(config.local_bundle);rmSync(latest.local_bundle);config=latest
  rmSync(artifacts,{recursive:true});rmSync(plan.releases,{recursive:true});rmSync(plan.host,{recursive:true})
  segments.quiescenceBeforeRecoverySeconds=(performance.now()-stopStart)/1000
  const failureAt=Date.now(),rtoStart=performance.now(),keyStart=performance.now();segments.failureAtUnixMs=failureAt;segments.rtoStartMonotonicMs=rtoStart
  const retrievedKey=privateRoot+'/retrieved-key';privateFile(retrievedKey,readFileSync(escrow+'/master'));segments.keyRetrievalSeconds=(performance.now()-keyStart)/1000
  config.key_file=retrievedKey
  const restored=await cli('restore');segments.restoreSeconds=restored.seconds;assert.equal(restored.value.status,'QUARANTINED')
  assert.equal(restored.value.recovery_point,segments.latestRecoveryPointUnixSeconds)
  segments.rpoSeconds=Math.max(0,failureAt/1000-restored.value.recovery_point)
  for(const image of imageLock){const archive=config.restored_artifacts+'/images/'+image.app+'-'+image.version+'.tar';assert.equal(sha(readFileSync(archive)),image.archiveSha256);const listing=await run('tar',['-tf',archive]);assert(listing.output.includes('manifest.json'))}
  const imageLoadStart=performance.now();cpSync(config.restored_artifacts+'/images','/restored-image-archives',{recursive:true})
  console.log('PHASE33_RESTORED_IMAGES_READY')
  await wait(()=>existsSync('/restored-image-archives/accepted.json'),'Managed owner must load recovered archives and start all four actual images',600000)
  segments.imageLoadSeconds=(performance.now()-imageLoadStart)/1000;const recoveredImages=JSON.parse(readFileSync('/restored-image-archives/accepted.json'));assert.equal(recoveredImages.status,'PASS');assert.equal(recoveredImages.images.length,4)
  for(const native of JSON.parse(readFileSync(config.restored_artifacts+'/runtime/native-lock.json')))assert.equal(sha(readFileSync(config.restored_artifacts+'/runtime/bin/'+native.component)),native.restorableSha256)
  checkpoint('remote-only-restore-with-separate-key',{localBundleAbsent:!existsSync(config.local_bundle),credentialReset:restored.value.credentials_invalidated,recoveredImages,productionPitr:'NOT_ACCEPTED'})
  const readyStart=performance.now();restoredDb=await services.recoveryControl.restart(plan,config.restored_artifacts);segments.readinessSeconds=(performance.now()-readyStart)/1000
  const businessStart=performance.now()
  // Same JWT signing key deliberately retained here to prove database revocation.
  assert.equal((await services.request(services.recoveryControl.platformOrigin,'/api/auth/profile',{token:actor.session})).status,401)
  assert.equal((await oauth({grant_type:'refresh_token',refresh_token:actor.sessionResponse.refreshToken})).status,400)
  assert([401,403].includes((await services.businessCall(oldStores,'/stores-api/stores')).status))
  assert([401,403].includes((await services.businessCall(oldWork,'/work-orders-api/tickets')).status))
  assert.equal((await oauth({...auth,token:issued.data.access_token},'/oauth/introspect')).data.active,false)
  const tokenRows=(await restoredDb.platform.query('SELECT revoked,expires_at>now() AS unexpired FROM oauth_tokens WHERE token_hash=ANY($1)',[[oldStores.token,oldWork.token,issued.data.access_token,issued.data.refresh_token].map(sha)])).rows;assert.equal(tokenRows.length,4);assert(tokenRows.every(r=>r.revoked))
  for(const fields of [{grant_type:'refresh_token',refresh_token:issued.data.refresh_token},{grant_type:'authorization_code',code:pendingCode,redirect_uri:'https://localhost/callback'},{grant_type:'urn:ietf:params:oauth:grant-type:device_code',device_code:device}])assert.equal((await oauth({...auth,...fields})).status,400)
  const fresh=await services.authorize(actor,'A','stores'),work=await services.authorize(actor,'A','work-orders')
  assert.equal((await services.request(services.recoveryControl.platformOrigin,'/api/oauth/consent/'+requestId,{token:fresh.session})).status,404)
  const state=(await restoredDb.platform.query("SELECT (SELECT count(*) FROM platform_sessions WHERE revoked_at IS NULL AND issued_at < (SELECT max(created_at) FROM audit_logs WHERE action='RECOVERY_RESET')) AS old_sessions,(SELECT count(*) FROM oauth_authorization_codes WHERE consumed=false) AS codes,(SELECT count(*) FROM oauth_authorization_requests WHERE consumed=false) AS requests,(SELECT count(*) FROM oauth_device_codes WHERE status<>'EXPIRED') AS devices,(SELECT count(*) FROM audit_logs WHERE action='RECOVERY_RESET') AS audits")).rows[0]
  assert.equal(Number(state.old_sessions),0);assert.equal(Number(state.codes),0);assert.equal(Number(state.requests),0);assert.equal(Number(state.devices),0);assert.equal(Number(state.audits),1)
  checkpoint('all-old-credential-families-refused',{sameJwtKey:true,platformSession:401,platformRefresh:400,applicationTokens:'DENIED',oauthAccess:'inactive',oldPersistentRowsRevoked:tokenRows.length,unexpiredRevokedRows:tokenRows.filter(r=>r.unexpired).length,oauthRefreshCodeDevice:400,pendingConsent:404,recoveryAudits:Number(state.audits)})
  const readStore=await call(fresh,'/stores-api/stores/'+store.id);assert.equal(readStore.name,'恢复前门店')
  const updatedStore=await call(fresh,'/stores-api/stores/'+store.id,{method:'PATCH',body:{version:readStore.version,name:'恢复后门店'}});assert.equal(updatedStore.name,'恢复后门店')
  const creatorB=await services.authorize(actor,'B','stores')
  const newStore=await call(creatorB,'/stores-api/stores',{method:'POST',headers:{'idempotency-key':randomUUID()},body:{name:'恢复后新建门店',businessStatus:'OPEN',businessContact:'new'}},201);assert(newStore.id)
  const readTicket=await call(work,'/work-orders-api/tickets/'+ticket.id);assert.equal(readTicket.title,'恢复前工单')
  const updatedTicket=await call(work,'/work-orders-api/tickets/'+ticket.id,{method:'PATCH',body:{version:readTicket.version,title:'恢复后工单'}});assert.equal(updatedTicket.title,'恢复后工单')
  const newTicket=await call(work,'/work-orders-api/tickets',{method:'POST',headers:{'idempotency-key':randomUUID()},body:{title:'恢复后新建工单',description:'fresh login',priority:'NORMAL',assigneeUserId:actor.userId}},201)
  await call(work,'/work-orders-api/tickets/'+newTicket.id+'/remind',{method:'POST',body:{idempotencyKey:'phase33-after-restore'}})
  const inbox=await services.platformApi('/api/notifications',{token:work.session});assert(inbox.records.length>=inboxBefore.records.length+1)
  await services.platformApi('/api/notifications/'+inbox.records[0].id+'/read',{token:work.session,method:'PUT'})
  await call(work,'/work-orders-api/tickets/'+newTicket.id,{method:'DELETE',body:{version:newTicket.version}},200)
  const foreign=await services.authorize(actor,'B','stores');await call(foreign,'/stores-api/stores/'+store.id,{},404)
  checkpoint('restored-login-authorization-crud-notifications',{existingStoreRestored:true,existingTicketRestored:true,newCreates:2,updates:2,softDelete:1,inboxBefore:inboxBefore.records.length,inboxAfter:inbox.records.length,markRead:true,crossOrganization:404,restoredNginx:restoredDb.routeRestoredHashes})
  browser=await launchBrowser();const pages=[]
  for(const app of ['stores','work-orders'])for(const version of ['v1','v2']){
    if(version==='v2')await services.adminApi('/api/applications/'+services.apps[app].application.id+'/releases/'+versionReleases[app].id+'/activate','POST')
    const a=await services.authorize(actor,'A',app),context=await browser.newContext({viewport:{width:1440,height:1000}});contexts.push(context)
    await context.addInitScript(({session,refreshToken,applicationId})=>{if(window!==window.top||sessionStorage.getItem('p33'))return;localStorage.setItem('platform-session-v1',JSON.stringify({schema:1,version:1,sessionId:crypto.randomUUID(),status:'active',token:session,refreshToken}));localStorage.setItem('platform-current-app',applicationId);sessionStorage.setItem('p33','1')},{session:a.session,refreshToken:a.sessionResponse.refreshToken,applicationId:services.apps[app].application.id})
    const page=await context.newPage();page.setDefaultTimeout(30000);const errors=[];page.on('pageerror',e=>errors.push(e.message))
    await page.goto(services.origin+'/micro/'+app,{waitUntil:'domcontentloaded'})
    await page.getByTestId('micro-status-bar').getByText('运行中',{exact:true}).waitFor()
    const frame=page.frameLocator('.micro-mount-zone iframe');await frame.getByTestId(app==='stores'?'stores-app':'work-orders-app').waitFor()
    await frame.getByText(app==='stores'?'恢复后门店':'恢复后工单',{exact:false}).first().waitFor()
    const src=await page.locator('.micro-mount-zone iframe').getAttribute('src');assert(src.includes('/versions/'+(version==='v1'?'phase27':'v2')+'/'));assert.equal(errors.length,0)
    pages.push({app,version,entry:new URL(src,services.origin).pathname,restoredDataVisible:true,errors:errors.length})
    await page.screenshot({path:process.env.STORES_BROWSER_REPORT_DIR+'/recovery-'+app+'-'+version+'.png'})
  }
  checkpoint('four-restored-frontend-versions',{browser:browser.version(),pages})
  segments.businessAcceptanceSeconds=(performance.now()-businessStart)/1000;segments.rtoEndMonotonicMs=performance.now();segments.rtoSeconds=(segments.rtoEndMonotonicMs-rtoStart)/1000;segments.totalQuiescenceSeconds=(performance.now()-stopStart)/1000
  assert(segments.rpoSeconds<=86400&&segments.rtoSeconds<=4*3600)
  checkpoint('measured-pilot-rpo-rto',{profile:'pilot',rpoLimitSeconds:86400,rtoLimitSeconds:14400,segments,production15min1h:'NOT_ACCEPTED',physicalRemoteHost:'NOT_ACCEPTED'})
  // Explicit source databases are still present, original business rows remain.
  assert.equal((await services.businessDb.query('SELECT name FROM stores WHERE id=$1',[store.id])).rows[0].name,'恢复前门店')
  assert.equal((await services.workOrdersDb.query('SELECT title FROM work_orders WHERE id=$1',[ticket.id])).rows[0].title,'恢复前工单')
  checkpoint('sources-retained-and-no-overwrite',{sourceDatabases:3,sourceRowsUnchanged:true,failedTargetsNeverAutoDeleted:true})
}catch(error){failure=error;console.error('PHASE33_FAILURE '+(services?services.redact(error.stack):error.message))}
finally{
  for(const context of contexts)await context.close().catch(()=>{});await browser?.close().catch(()=>{})
  if(monitor){monitor.kill('SIGTERM');await Promise.race([new Promise(r=>monitor.once('close',r)),delay(5000)]);if(monitor.exitCode===null)monitor.kill('SIGKILL')}
  if(services)try{cleanup=await services.close({browserClosed:true})}catch(e){failure||=e}
}
const report={status:failure?'FAIL':'PASS',checks,segments,commands,cleanup,producerSha256:sha(readFileSync(fileURLToPath(import.meta.url))),source:services?.source,error:failure?services?.redact(failure.message):undefined}
console.log('PHASE33_RESULT '+JSON.stringify(report));if(failure)process.exitCode=1
