// Phase12 reuses the Phase11 actual business/auth/browser matrix, changing only
// explicit production startup, immutable clean build artifacts and real TLS.
// Further gates exercise monitoring, faults and encrypted restore on this same
// actual two-database deployment.
import assert from 'node:assert/strict'
import test from 'node:test'
import { createServer } from 'node:net'
import { createHash, randomBytes, randomUUID } from 'node:crypto'
import { mkdirSync, readFileSync, readdirSync, renameSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { runtimeEnvironment } from './production-native-support.mjs'

const workspace=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..')
process.env.SAAS_PRODUCTION_ACCEPTANCE='1'
process.env.WORK_ORDERS_E2E_SCENARIO='full'
if(!process.env.WORK_ORDERS_E2E_PLATFORM_DATABASE_URL && process.env.TEST_DATABASE_URL)process.env.WORK_ORDERS_E2E_PLATFORM_DATABASE_URL=process.env.TEST_DATABASE_URL
const { fixture, runFullAcceptance }=await import('../business-apps/work-orders/tests/platform-e2e.mjs')
const required=['WORK_ORDERS_E2E_BACKEND_BINARY','WORK_ORDERS_E2E_PLATFORM_DATABASE_URL','NGINX_BIN','NGINX_MIME_TYPES','WORK_ORDERS_E2E_HOST_DIST','PLAYWRIGHT_MODULE','BROWSER_EXECUTABLE',
  'PRODUCTION_ARTIFACT_ROOT','PRODUCTION_SOURCE_ROOT','PRODUCTION_CA_FILE','PRODUCTION_GATEWAY_CERTIFICATE','PRODUCTION_GATEWAY_PRIVATE_KEY','PRODUCTION_CERTUTIL_BIN','PG_DUMP_BIN','PG_RESTORE_BIN','PSQL_BIN','PG_CLIENT_LIBRARY_PATH']
const missing=required.filter((key)=>!process.env[key])
const sleep=(ms)=>new Promise((resolve)=>setTimeout(resolve,ms))
async function childDeadline(item,milliseconds) {
  let timer
  try { await Promise.race([item.exited,new Promise((resolve)=>{timer=setTimeout(resolve,milliseconds)})]) }
  finally { clearTimeout(timer) }
}
async function until(fn,message,timeout=15000) { const deadline=Date.now()+timeout; while(!await fn()){assert.ok(Date.now()<deadline,message); await sleep(100)} }
function checkedContext(context) {
  return new Proxy(context,{get(target,key){
    if(key==='test')return async(...args)=>{
      const index=args.length-1,callback=args[index];assert.equal(typeof callback,'function')
      let completed=false
      args[index]=async(child)=>{await callback(checkedContext(child));completed=true}
      await target.test(...args)
      assert.ok(completed,`Acceptance stops after failed subtest: ${args[0]}`)
    }
    const value=Reflect.get(target,key);return typeof value==='function'?value.bind(target):value
  }})
}
async function headersGate(h,t) {
  await t.test('production HTTPS, security headers, strict routing and private metrics',async()=>{
    const home=await h.http(h.gatewayBase,'GET','/')
    for(const name of Object.keys(h.hostHashes)) {
      const content=readFileSync(path.join(h.hostRoot,name),'utf8')
      for(const secret of h.secrets)assert.ok(!content.includes(secret),'clean built host resources must not contain runtime secrets')
    }
    assert.equal(home.status,200); assert.equal(home.headers.get('x-content-type-options'),'nosniff')
    const weakWrite=await h.http(h.gatewayBase,'POST','/api/users',h.administrator.session,{username:'acceptance-weak-password-blocked',password:'admin123'})
    assert.equal(weakWrite.status,400,'the actual release binary must enforce production password writes')
    assert.equal((await h.platformDb.writer.query("SELECT count(*)::int AS n FROM users WHERE username='acceptance-weak-password-blocked'")).rows[0].n,0)
    assert.equal(home.headers.get('strict-transport-security'),'max-age=31536000')
    const csp=home.headers.get('content-security-policy'); assert.ok(csp.includes("script-src 'self'"));assert.ok(!csp.includes('unsafe-eval') && !/script-src[^;]*unsafe-inline/.test(csp))
    for(const route of ['/api/internal/metrics','/metrics','/work-orders-api/metrics','/assets/not-a-real-production-chunk.js','/sub-apps/work-orders/versions/1.0.0/missing.js','/api/missing','/oauth/missing']) assert.equal((await h.http(h.gatewayBase,'GET',route)).status,404,`strict private/static/API route ${route}`)
    for(const [base,endpoint,token] of [[h.platformInternalBase,'/api/internal/metrics',h.metrics.platform],[h.businessBase,'/work-orders-api/metrics',h.metrics.business],[`http://[::1]:${h.notificationPort}`,'/metrics',h.metrics.notification]]) {
      assert.equal((await h.http(base,'GET',endpoint)).status,401)
      assert.equal((await h.http(base,'GET',endpoint,'wrong-metrics-token')).status,401)
      const measured=await h.http(base,'GET',endpoint,token);assert.equal(measured.status,200);assert.match(measured.text,/^[a-zA-Z_:][a-zA-Z0-9_:]*(?:\{[^\r\n]*\})?\s+[0-9]/m)
      for(const secret of h.secrets) assert.ok(!measured.text.includes(secret),'public metric labels cannot contain secrets')
    }
    const requestId=randomUUID()
    const traced=await h.http(h.gatewayBase,'GET','/api/health',null,undefined,{headers:{'X-Request-Id':requestId,'X-Forwarded-For':'198.51.100.99','X-Real-IP':'198.51.100.99'}})
    assert.equal(traced.headers.get('x-request-id'),requestId)
    const origin=await h.http(h.gatewayBase,'OPTIONS','/api/auth/login',null,undefined,{headers:{Origin:'https://unapproved.example.test','Access-Control-Request-Method':'POST'}})
    assert.notEqual(origin.headers.get('access-control-allow-origin'),'https://unapproved.example.test')
    const business=await h.http(h.gatewayBase,'OPTIONS','/work-orders-api/tickets',null,undefined,{headers:{Origin:'https://unapproved.example.test','Access-Control-Request-Method':'POST','Access-Control-Request-Headers':'authorization'}})
    assert.ok(business.status>=400);assert.equal(business.headers.get('access-control-allow-origin'),null)
    const migrations=await h.platformDb.writer.query('SELECT version,success,encode(checksum,\'hex\') AS checksum FROM _sqlx_migrations ORDER BY version')
    const names=readdirSync(path.join(process.env.PRODUCTION_SOURCE_ROOT,'backend/migrations')).filter((name)=>/^\d+.*\.sql$/.test(name))
    assert.equal(migrations.rows.length,names.length);assert.ok(migrations.rows.every((row)=>row.success))
    for(const row of migrations.rows){const name=names.find((file)=>Number(file.split('_')[0])===Number(row.version));assert.equal(row.checksum,createHash('sha384').update(readFileSync(path.join(process.env.PRODUCTION_SOURCE_ROOT,'backend/migrations',name))).digest('hex'))}
    // The prelaunch runtime proof explicitly rejects the untrusted CA; the
    // entire scenario here loads the private CA through NODE_EXTRA_CA_CERTS.
    assert.equal(process.env.NODE_TLS_REJECT_UNAUTHORIZED,undefined)
    assert.equal(process.env.NODE_EXTRA_CA_CERTS,process.env.PRODUCTION_CA_FILE)
  })
}
async function faults(h,t) {
  await t.test('actual native service faults produce local DOWN and RECOVERED alerts',async()=>{
    const eventLog=path.join(h.root,'local-alerts.jsonl'),configFile=path.join(h.root,'monitor.json')
    const config={environment:'production',intervalMs:1000,timeoutMs:2000,downThreshold:2,recoveryThreshold:2,eventLog,targets:[
      {name:'platform-health',url:`${h.gatewayBase}/api/health`,kind:'health'},
      {name:'business-health',url:`${h.gatewayBase}/work-orders-api/health`,kind:'health'},
      {name:'notification-health',url:`${h.gatewayBase}/notification-health`,kind:'health'},
      {name:'active-work-orders',url:`${h.gatewayBase}/sub-apps/work-orders/versions/1.0.0/assets/config.js`,kind:'asset',contains:'"version":"1.0.0"'},
      {name:'platform-metrics',url:`${h.platformInternalBase}/api/internal/metrics`,kind:'prometheus',private:true,tokenEnv:'PLATFORM_METRICS_TOKEN'},
    ]}
    writeFileSync(configFile,JSON.stringify(config),{mode:0o600})
    const monitor=h.launch('monitor',process.execPath,[path.join(workspace,'deploy/monitoring/monitor.mjs')],{env:{...runtimeEnvironment(process.env),MONITOR_CONFIG:configFile,NODE_EXTRA_CA_CERTS:process.env.PRODUCTION_CA_FILE,PLATFORM_METRICS_TOKEN:h.metrics.platform}})
    const events=()=>{try{return readFileSync(eventLog,'utf8').trim().split('\n').filter(Boolean).map((line)=>JSON.parse(line))}catch{return []}}
    try {
    await sleep(3500)
    assert.ok(!events().some((event)=>event.event==='DOWN'),'healthy actual production deployment must not alarm')
    for(const [name,item,entry,environment] of [
      ['business-health',h.business,path.join(process.env.PRODUCTION_ARTIFACT_ROOT,'work-orders-api/index.mjs'),h.businessEnvironment],
      ['notification-health',h.notificationProcess,path.join(process.env.PRODUCTION_ARTIFACT_ROOT,'notification-ws/index.ts'),h.notificationEnvironment],
    ]) {
      await h.stop(item)
      await until(()=>events().some((event)=>event.target===name && event.event==='DOWN'),'actual service failure must alert')
      const replacement=h.launch(name==='business-health'?'business':'notification',process.execPath,[entry],{env:environment})
      if(name==='business-health')h.business=replacement;else h.notificationProcess=replacement
      await until(()=>events().some((event)=>event.target===name && event.event==='RECOVERED'),'actual service recovery must alert')
    }
    // Inject failure only into this fixture's randomly created platform DB.
    // The supplied PostgreSQL instance and all other databases remain online.
    const name=h.platformDb.name;assert.match(name,/^work_orders_platform_e2e_p_[0-9a-f]{32}$/)
    const writerPid=(await h.platformDb.writer.query('SELECT pg_backend_pid() AS pid')).rows[0].pid
    const beforeDatabaseFault=events().length
    await h.platformDb.admin.query(`ALTER DATABASE "${name}" ALLOW_CONNECTIONS false`)
    try {
      await h.platformDb.admin.query('SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=$1 AND pid<>$2 AND pid<>pg_backend_pid()',[name,writerPid])
      await until(()=>events().slice(beforeDatabaseFault).some((event)=>event.target==='platform-health'&&event.event==='DOWN'),'actual owned database failure must alert',25000)
      const dbMetric=await h.http(h.platformInternalBase,'GET','/api/internal/metrics',h.metrics.platform,undefined,{timeout:4000})
      assert.match(dbMetric.text,/platform_database_up 0/)
      await until(async()=>{const value=await h.http(`http://[::1]:${h.notificationPort}`,'GET','/metrics',h.metrics.notification);return /^saas_notification_listen_up\{service="notification-ws"\} 0$/m.test(value.text)},'notification LISTEN loss must be observable',15000)
    } finally { await h.platformDb.admin.query(`ALTER DATABASE "${name}" ALLOW_CONNECTIONS true`) }
    await until(()=>events().slice(beforeDatabaseFault).some((event)=>event.target==='platform-health'&&event.event==='RECOVERED'),'owned database recovery must alert',30000)
    await until(async()=>{try{return(await h.http(h.gatewayBase,'GET','/notification-health')).data?.status==='UP'}catch{return false}},'notification DB and LISTEN must recover',30000)
    await until(()=>events().slice(beforeDatabaseFault).some((event)=>event.target==='notification-health'&&event.event==='RECOVERED'),'restored notification database and LISTEN must close their monitor alert',15000)
    h.acceptanceAlerts=events(); assert.ok(h.acceptanceAlerts.length>=4)
    for(const event of h.acceptanceAlerts)for(const secret of h.secrets)assert.ok(!JSON.stringify(event).includes(secret))
    } finally { await h.stop(monitor) }
  })
}

test('Phase12 clean production native HTTPS + isolated services + encrypted recovery acceptance',{
  skip:missing.length?`Opt in with ${missing.join(', ')}`:false,timeout:1800000,
},async(rawT)=>{
  const t=checkedContext(rawT)
  const started=Date.now()
  assert.equal(process.platform,'linux');assert.equal(process.env.NODE_TLS_REJECT_UNAUTHORIZED,undefined)
  const h=await fixture(t)
  await headersGate(h,t)
  await migrationFailure(h,t)
  await measuredCapacity(h,t)
  await runFullAcceptance(t,h)
  await correlationAndRuntime(h,t)
  await faults(h,t)
  // Recovery uses the same real data produced above. Added below after the
  // backup CLI has passed its isolated database gates.
  await recovery(h,t)
  assert.ok(h.acceptanceAlerts?.length && h.acceptanceRecovery,'fault and recovery gates must both pass before recording accepted evidence')
  Object.assign(h.productionEvidence,{passed:true,completedAt:new Date().toISOString(),durationMs:Date.now()-started,nodeVersion:process.version,
    capacity:h.acceptanceCapacity,alerts:h.acceptanceAlerts,recovery:h.acceptanceRecovery,hostFiles:Object.keys(h.hostHashes).length,actualProductionHttps:true,
    cleanBuildManifestSha256:createHash('sha256').update(readFileSync(path.resolve(process.env.PRODUCTION_ARTIFACT_ROOT,'../source-manifest.json'))).digest('hex')})
})

async function measuredCapacity(h,t) {
  await t.test('measured authorized capacity and rejected client authentication attempts retain valid-client service',async()=>{
    const actor=h.actors.root,token=await h.appToken(actor),started=performance.now(),latencies=[]
    for(let batch=0;batch<10;batch++)await Promise.all(Array.from({length:4},async()=>{
      const start=performance.now(),response=await h.http(h.gatewayBase,'GET','/work-orders-api/tickets',token)
      assert.equal(response.status,200);latencies.push(performance.now()-start)
    }))
    const durationMs=performance.now()-started
    const failures=[]
    for(let i=0;i<35;i++) {
      const response=await fetch(`${h.gatewayBase}/oauth/introspect`,{method:'POST',headers:{Authorization:'Basic '+Buffer.from('work-orders-client:deliberately-invalid-acceptance-secret').toString('base64'),'Content-Type':'application/x-www-form-urlencoded'},body:'token=invalid-application-token',redirect:'error',signal:AbortSignal.timeout(5000)})
      failures.push(response.status);await response.body?.cancel()
    }
    assert.ok(failures.includes(401)&&failures.includes(429),'configured failed-authentication budget must reject a burst')
    assert.equal((await h.http(h.gatewayBase,'GET','/work-orders-api/tickets',token)).status,200,'rejected bad client credentials must not consume the valid confidential client quota')
    latencies.sort((a,b)=>a-b)
    h.acceptanceCapacity={requests:40,concurrency:4,durationMs,requestsPerSecond:40000/durationMs,p50Ms:latencies[19],p95Ms:latencies[37],authenticationFailures:failures.filter((status)=>status===401).length,rateLimited:failures.filter((status)=>status===429).length,configuredClientPerMinute:600}
    console.log('[Phase12 measured native capacity] '+JSON.stringify(h.acceptanceCapacity))
  })
}

async function migrationFailure(h,t) {
  await t.test('an actual incompatible fresh database stops migration before any healthy platform listener',async()=>{
    const owned=await h.createDatabase(h.platformAdminUrl,'p')
    await owned.writer.query('CREATE TABLE tenants (acceptance_marker text NOT NULL)')
    await owned.writer.query("INSERT INTO tenants(acceptance_marker) VALUES('preserve-incompatible-database')")
    const socket=createServer();await new Promise((resolve,reject)=>{socket.once('error',reject);socket.listen(0,'::1',resolve)})
    const port=socket.address().port;await new Promise((resolve)=>socket.close(resolve))
    const failed=h.launch('platform-migration-failure',process.env.WORK_ORDERS_E2E_BACKEND_BINARY,[],{env:{...h.platformEnvironment,DATABASE_URL:owned.connectionString,BIND_ADDR:`[::1]:${port}`}})
    await Promise.race([failed.exited,sleep(15000)])
    assert.ok(failed.closed);assert.notEqual(failed.child.exitCode,0,'actual duplicate table migration must halt startup')
    await assert.rejects(h.http(`http://[::1]:${port}`,'GET','/api/health',null,undefined,{timeout:1000}))
    assert.equal((await owned.writer.query('SELECT acceptance_marker FROM tenants')).rows[0].acceptance_marker,'preserve-incompatible-database')
    assert.equal((await owned.writer.query('SELECT count(*)::int AS n FROM _sqlx_migrations WHERE success=true')).rows[0].n,0)
    assert.equal((await h.http(h.gatewayBase,'GET','/api/health')).data.status,'UP','existing healthy deployment remains available')
  })
}

async function correlationAndRuntime(h,t) {
  await t.test('request correlation, tenant-scoped audit and authenticated runtime telemetry',async()=>{
    h.actors.root.token=null
    const app=await h.appToken(h.actors.root),requestId=randomUUID()
    const published=await h.http(h.gatewayBase,'POST','/api/apps/work-orders/notifications',app,{idempotencyKey:`trace-${randomUUID()}`,
      target:{kind:'USER',userId:h.actors.root.userId,organizationId:h.actors.root.organizationId},type:'INFO',title:'Production trace',content:'Actual correlated publication'},
      {headers:{'X-Request-Id':requestId,'X-Real-IP':'198.51.100.99','X-Forwarded-For':'198.51.100.99'}})
    assert.equal(published.status,200);assert.equal(published.headers.get('x-request-id'),requestId)
    const row=(await h.platformDb.writer.query('SELECT request_id,application_id,tenant_id,ip FROM audit_logs WHERE request_id=$1',[requestId])).rows
    assert.equal(row.length,1);assert.equal(row[0].application_id,h.ids.application);assert.equal(row[0].tenant_id,h.actors.root.tenantId)
    assert.ok(['::1','127.0.0.1'].includes(row[0].ip),'trusted gateway must overwrite client-provided forwarding IP')
    const menuId=randomUUID()
    await h.platformDb.writer.query("INSERT INTO menus(id,application_id,type,title,permission,visible) VALUES($1,$2,'BUTTON','Acceptance audit','system:audit',false)",[menuId,h.ids.application])
    await h.platformDb.writer.query("INSERT INTO role_menus(role_id,menu_id) SELECT id,$1 FROM roles WHERE code='p11-operations'",[menuId])
    await h.platformDb.writer.query("INSERT INTO tenant_menus(tenant_id,menu_id) SELECT id,$1 FROM tenants WHERE code IN ('p11-a','p11-b')",[menuId])
    await h.platformDb.writer.query("INSERT INTO organization_menus(organization_id,menu_id) SELECT id,$1 FROM organizations WHERE code LIKE 'p11-%'",[menuId])
    const own=await h.platformOk('GET',`/api/audit-logs?request_id=${requestId}&application_id=${h.ids.application}`,h.actors.root.session)
    assert.equal(own.total,1);assert.equal(own.records[0].requestId,requestId)
    const foreign=await h.platformOk('GET',`/api/audit-logs?request_id=${requestId}&tenant_id=${h.actors.root.tenantId}`,h.actors.remote.session)
    assert.equal(foreign.total,0,'an ordinary tenant auditor cannot expand its context with the tenant filter')
    assert.equal((await h.http(h.gatewayBase,'POST','/api/apps/work-orders/runtime-events',app,{kind:'LOAD_FAILED',stage:'mount'})).status,204)
    assert.equal((await h.http(h.gatewayBase,'POST','/api/apps/hermes/runtime-events',app,{kind:'LOAD_FAILED',stage:'mount'})).status,403)
    assert.equal((await h.http(h.gatewayBase,'POST','/api/apps/work-orders/runtime-events','invalid-token',{kind:'LOAD_FAILED',stage:'mount'})).status,401)
    assert.equal((await h.http(h.gatewayBase,'POST','/api/apps/work-orders/runtime-events',app,{kind:'LOAD_FAILED',stage:'mount',rawError:'forbidden'})).status,400)
    const measured=await h.http(h.platformInternalBase,'GET','/api/internal/metrics',h.metrics.platform)
    assert.match(measured.text,/platform_micro_runtime_events_total\{application="work-orders",kind="LOAD_FAILED",stage="mount"\} 1/)
    assert.match(measured.text,/platform_http_requests_total\{[^\n]*status="400"[^\n]*\} [1-9]/)
    assert.match(measured.text,/platform_http_authentication_failures_total\{[^\n]*\} [1-9]/)
    assert.match(measured.text,/platform_http_request_duration_seconds_count\{[^\n]*\} [1-9]/)
  })
  await t.test('actual missing micro application entry reports a runtime failure and recovers without host rebuild',async()=>{
    const actor=await h.login('p11-owner',h.actors.root.organizationId),browser=await h.launchBrowser()
    const context=await browser.newContext({viewport:{width:1440,height:1000}})
    await context.addInitScript(({token,app})=>{if(window.top===window){localStorage.setItem('platform-token',token);localStorage.setItem('platform-current-app',app)}},{token:actor.session,app:h.ids.application})
    const entry=path.join(h.root,'micro-apps/work-orders/versions/1.0.0/index.html'),held=path.join(h.root,'held-current-entry.html')
    const original=readFileSync(entry),originalHash=createHash('sha256').update(original).digest('hex')
    const before=(await h.http(h.platformInternalBase,'GET','/api/internal/metrics',h.metrics.platform)).text
    const count=(text)=>Number(text.match(/platform_micro_runtime_events_total\{application="work-orders",kind="LOAD_FAILED",stage="mount"\} (\d+)/)?.[1]??0)
    const page=await context.newPage(),reports=[]
    page.on('response',(response)=>{if(response.url().endsWith('/runtime-events'))reports.push(response.status())})
    renameSync(entry,held)
    try {
      await page.goto(`${h.gatewayBase}/micro/work-orders`)
      await page.getByText('子应用挂载失败',{exact:true}).waitFor({timeout:40000})
      await until(()=>reports.includes(204),'real host error must send accepted runtime telemetry',10000)
      const after=(await h.http(h.platformInternalBase,'GET','/api/internal/metrics',h.metrics.platform)).text
      assert.ok(count(after)>count(before))
      renameSync(held,entry)
      assert.equal(createHash('sha256').update(readFileSync(entry)).digest('hex'),originalHash)
      await page.getByRole('button',{name:/重试|重新加载/}).first().click()
      const frame=page.frameLocator('iframe[title="work-orders application"]')
      await frame.getByTestId('ticket-list').waitFor({timeout:20000})
      assert.equal((await h.http(h.gatewayBase,'GET','/api/health')).data.status,'UP')
      assert.equal((await h.http(h.gatewayBase,'GET','/work-orders-api/health')).data.status,'UP')
    } finally {
      try{renameSync(held,entry)}catch{}
      await context.close()
    }
  })
}

async function recovery(h,t) {
  await t.test('encrypted consistent two-database backup and new-database recovery with forced reauthentication',async()=>{
    const original={platform:await snapshot(h.platformDb.writer,'platform'),business:await snapshot(h.businessDb.writer,'business')}
    const oldSession=h.actors.root.session,oldApp=await h.appToken(h.actors.root)
    const liveSocket=await h.connectSocket(h.actors.root)
    const keyFile=path.join(h.root,'backup-key.bin'),backupRoot=path.join(h.root,'encrypted-backups'),envFile=path.join(h.root,'recovery.env')
    mkdirSync(backupRoot,{mode:0o700});writeFileSync(keyFile,randomBytes(32),{mode:0o600})
    await h.stop(h.business);await h.stop(h.notificationProcess);await h.stop(h.platform)
    const shared={BACKUP_PLATFORM_DATABASE_URL:h.platformDb.connectionString,BACKUP_WORK_ORDERS_DATABASE_URL:h.businessDb.connectionString,
      BACKUP_ROOT:backupRoot,BACKUP_KEY_FILE:keyFile,PG_DUMP_BIN:process.env.PG_DUMP_BIN,PG_RESTORE_BIN:process.env.PG_RESTORE_BIN,PSQL_BIN:process.env.PSQL_BIN,
      PG_CLIENT_LIBRARY_PATH:process.env.PG_CLIENT_LIBRARY_PATH,BACKUP_WRITES_QUIESCED:'true'}
    const restoredNames={platform:`work_orders_platform_e2e_p_${randomUUID().replaceAll('-','')}`,business:`work_orders_platform_e2e_b_${randomUUID().replaceAll('-','')}`}
    const registered=[]
    for(const [role,name]of Object.entries(restoredNames)){
      const connectionString=new URL(role==='platform'?h.platformAdminUrl:h.businessAdminUrl);connectionString.pathname=`/${name}`
      for(const key of ['database','dbname','application_name'])connectionString.searchParams.delete(key)
      const admin=new h.pg.Client({connectionString:role==='platform'?h.platformAdminUrl:h.businessAdminUrl});await admin.connect()
      assert.equal((await admin.query('SELECT count(*)::int AS n FROM pg_database WHERE datname=$1',[name])).rows[0].n,0)
      const item={name,expectedName:name,created:false,connected:true,admin,connectionString:connectionString.toString()};h.databases.push(item);registered.push(item);h.saveOwnership();h.secrets.add(item.connectionString)
    }
    const config={...shared,RESTORE_PLATFORM_MAINTENANCE_URL:h.platformAdminUrl,RESTORE_WORK_ORDERS_MAINTENANCE_URL:h.businessAdminUrl,
      RESTORE_PLATFORM_DATABASE:restoredNames.platform,RESTORE_WORK_ORDERS_DATABASE:restoredNames.business}
    writeFileSync(envFile,Object.entries(config).map(([key,value])=>`${key}=${value}`).join('\n')+'\n',{mode:0o600})
    const invoke=async(command,backupSet)=>{
      const item=h.launch(`recovery-${command}`,process.execPath,[path.join(workspace,'deploy/recovery/pg-backup.mjs'),command,'--env-file',envFile,...backupSet?['--backup-set',backupSet]:[]],{env:runtimeEnvironment(process.env)})
      await childDeadline(item,120000)
      if(!item.closed)await h.stop(item)
      // Both target names were proven absent before invoking this owned CLI.
      // Discover a partially created target even after timeout or malformed
      // CLI output; registered admin connections own their exact cleanup.
      if(command==='restore')for(const owned of registered) {
        const exists=(await owned.admin.query('SELECT count(*)::int AS n FROM pg_database WHERE datname=$1',[owned.expectedName])).rows[0].n===1
        if(exists)owned.created=true
      }
      h.saveOwnership();assert.ok(item.closed,'backup/restore child must finish within the drill target')
      const output=item.stdout.join('').trim().split('\n').filter(Boolean).map((line)=>JSON.parse(line))
      const failure=item.stderr.join('').trim();let failureCode='UNKNOWN',safeFailure
      if(failure){try{const parsed=JSON.parse(failure);failureCode=/^[A-Z0-9_]{2,64}$/.test(parsed.code)?parsed.code:'UNKNOWN';
        safeFailure={code:failureCode,.../^[a-zA-Z0-9_.:-]{1,64}$/.test(parsed.operationStage??'')?{operationStage:parsed.operationStage}:{},
          .../^[A-Z0-9_]{1,64}$/.test(parsed.pgFailureClass??'')?{pgFailureClass:parsed.pgFailureClass}:{},
          .../^[A-Z0-9]{5}$/.test(parsed.sqlState??'')?{sqlState:parsed.sqlState}:{},
          ...Number.isInteger(parsed.processExitCode)?{processExitCode:parsed.processExitCode}:{}}
        for(const target of parsed.createdTargets??[]){const owned=registered.find((row)=>row.name===target.database);if(owned)owned.created=true}h.saveOwnership()
        h.productionEvidence.recoveryFailure=safeFailure
      }catch{}}
      assert.equal(item.child.exitCode,0,`encrypted backup/restore must complete through the real PostgreSQL client (${JSON.stringify(safeFailure??{code:failureCode})})`);return output.at(-1)
    }
    const backup=await invoke('backup');assert.equal(backup.event,'backup-complete')
    const inspected=await invoke('inspect',backup.backupSet);assert.equal(inspected.event,'backup-verified')
    const started=Date.now(),restored=await invoke('restore',backup.backupSet)
    assert.equal(restored.event,'restore-complete');assert.equal(restored.jwtKeyRotationRequired,true);assert.equal(restored.reloginRequired,true)
    assert.ok(Date.now()-started<3600000,'RTO1h is a measured drill target')
    for(const itemof of registered){itemof.created=true;itemof.writer=new h.pg.Client({connectionString:itemof.connectionString});await itemof.writer.connect()};h.saveOwnership()
    const [platform,business]=registered
    assert.deepEqual(await snapshot(business.writer,'business'),original.business,'work-order and soft-deleted rows must restore exactly')
    const platformAfter=await snapshot(platform.writer,'platform')
    assert.deepEqual(platformAfter.grants,original.platform.grants);assert.deepEqual(platformAfter.notifications,original.platform.notifications);assert.deepEqual(platformAfter.publications,original.platform.publications);assert.deepEqual(platformAfter.migrations,original.platform.migrations)
    assert.ok(platformAfter.auditCount>=original.platform.auditCount+1,'restored auth reset creates an auditable record')
    const freshJwt=randomBytes(48).toString('hex');h.secrets.add(freshJwt)
    const restoredOldKeyEnvironment={...h.platformEnvironment,DATABASE_URL:platform.connectionString}
    h.platformEnvironment={...restoredOldKeyEnvironment,JWT_SECRET:freshJwt}
    h.businessEnvironment={...h.businessEnvironment,WORK_ORDERS_DATABASE_URL:business.connectionString}
    h.notificationEnvironment={...h.notificationEnvironment,DATABASE_URL:platform.connectionString}
    h.platform=h.launch('platform-restore-auth-check',process.env.WORK_ORDERS_E2E_BACKEND_BINARY,[],{env:restoredOldKeyEnvironment})
    await until(async()=>{try{return (await h.http(h.gatewayBase,'GET','/api/health')).status===200}catch{return false}},'restored platform must become ready',60000)
    h.business=h.launch('business',process.execPath,[path.join(process.env.PRODUCTION_ARTIFACT_ROOT,'work-orders-api/index.mjs')],{env:h.businessEnvironment})
    h.notificationProcess=h.launch('notification',process.execPath,[path.join(process.env.PRODUCTION_ARTIFACT_ROOT,'notification-ws/index.ts')],{env:h.notificationEnvironment})
    await until(async()=>{try{return (await h.http(h.gatewayBase,'GET','/work-orders-api/health')).data?.status==='UP' && (await h.http(h.gatewayBase,'GET','/notification-health')).data?.status==='UP'}catch{return false}},'restored independent services must become ready',30000)
    assert.equal((await h.http(h.gatewayBase,'GET','/api/auth/profile',oldSession)).status,401)
    assert.equal((await h.http(h.gatewayBase,'GET','/work-orders-api/tickets',oldApp)).status,401)
    await h.stop(h.platform)
    h.platform=h.launch('platform',process.env.WORK_ORDERS_E2E_BACKEND_BINARY,[],{env:h.platformEnvironment})
    await until(async()=>{try{return (await h.http(h.gatewayBase,'GET','/api/health')).status===200}catch{return false}},'restored signing key rotation must become ready',60000)
    assert.equal((await h.http(h.gatewayBase,'GET','/api/auth/profile',oldSession)).status,401)
    assert.ok(!liveSocket.socket.connected,'pre-restore socket must be disconnected')
    // Query/auth helpers continue against the restored databases. Keeping the
    // original cleanup owners separate preserves all four target identities.
    h.platformDb.writer=platform.writer;h.businessDb.writer=business.writer
    h.actors.root=await h.login('p11-owner',h.actors.root.organizationId)
    h.actors.remote=await h.login('p11-remote',h.actors.remote.organizationId)
    for(const [actor,scope]of [[h.actors.root,'ORG'],[h.actors.remote,'TENANT']]){await h.setGrants(actor,scope);actor.token=null}
    const rows=await h.records(h.actors.root);assert.ok(rows.length);assert.ok(rows.every((row)=>row.tenantId===h.actors.root.tenantId))
    await h.denied(h.actors.remote,'GET',`/work-orders-api/tickets/${rows[0].id}`)
    const created=await h.create(h.actors.root,'Recovered real create')
    const updated=await h.accepted(h.actors.root,'PATCH',`/work-orders-api/tickets/${created.id}`,{version:created.version,title:'Recovered real edit'},200)
    assert.equal(updated.title,'Recovered real edit')
    const exportResult=await h.businessCall(h.actors.root,'GET','/work-orders-api/tickets/export.csv');assert.equal(exportResult.status,200);assert.ok(exportResult.text.includes(updated.number))
    const socket=await h.connectSocket(h.actors.root)
    const reminder=await h.accepted(h.actors.root,'POST',`/work-orders-api/tickets/${updated.id}/remind`,{idempotencyKey:`recovered-${randomUUID()}`},200)
    assert.ok(reminder);await until(()=>socket.events.length>0,'restored notification must reach the actual Socket.IO connection')
    const freshApp=await h.appToken(h.actors.root)
    await platform.writer.query('DELETE FROM membership_data_grants g USING memberships m WHERE g.membership_id=m.id AND m.user_id=$1 AND m.organization_id=$2 AND g.application_id=$3 AND g.resource_code=$4 AND g.action_code=$5',[h.actors.root.userId,h.actors.root.organizationId,h.ids.application,'ticket','read'])
    assert.equal((await h.http(h.gatewayBase,'GET','/work-orders-api/tickets',freshApp)).status,403,'restored same app token obeys immediate data revocation')
    const fullRecoveryDurationMs=Date.now()-started
    assert.ok(fullRecoveryDurationMs<3600000)
    assert.ok(Date.now()-Date.parse(backup.completedAt)<24*3600000)
    h.acceptanceRecovery={backupDurationMs:backup.durationMs,cliRestoreDurationMs:restored.durationMs,fullRecoveryDurationMs,rpoTargetHours:24,rtoTargetHours:1,authenticationReset:restored.authenticationReset,oldTokensRejectedBeforeKeyRotation:true,oldTokensRejected:true,newLogin:true}
  })
}
async function snapshot(client,role) {
  if(role==='business')return(await client.query('SELECT row_to_json(w) AS row FROM work_orders w ORDER BY id')).rows
  return {grants:(await client.query('SELECT row_to_json(g) AS row FROM membership_data_grants g ORDER BY id')).rows,
    notifications:(await client.query('SELECT row_to_json(n) AS row FROM notifications n ORDER BY id')).rows,
    publications:(await client.query('SELECT row_to_json(p) AS row FROM notification_publications p ORDER BY id')).rows,
    migrations:(await client.query('SELECT version,success,encode(checksum,\'hex\') AS checksum FROM _sqlx_migrations ORDER BY version')).rows,
    auditCount:(await client.query('SELECT count(*)::int AS n FROM audit_logs')).rows[0].n}
}
