import test from 'node:test'
import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { randomBytes, randomUUID, createHash } from 'node:crypto'
import { spawn } from 'node:child_process'
import { readFile, writeFile, mkdtemp, chmod, mkdir, readdir, rm, cp } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'
import {psql}from'../postgres.mjs'
import {parseDatabaseUrl}from'../config.mjs'

const root=fileURLToPath(new URL('../../../',import.meta.url))
const require=createRequire(new URL('../../../business-apps/work-orders/backend/package.json',import.meta.url))
const pg=require('pg')
const cli=path.join(root,'deploy/recovery/pg-backup.mjs')
const base=process.env.TEST_DATABASE_URL
if (!base) throw new Error('TEST_DATABASE_URL_REQUIRED')
const server=new URL(base);server.pathname='/postgres'
const clientDirectory=process.env.RECOVERY_PG_BIN || path.join(root,'target/phase12-runtime/postgres-client/root/usr/lib/postgresql/17/bin')
const libraryDirectory=process.env.RECOVERY_PG_LIB || path.join(root,'target/phase12-runtime/postgres-client/root/usr/lib/x86_64-linux-gnu')
const makeConnection=name=>{const url=new URL(base);url.pathname=`/${name}`;return url.toString()}
async function connected(url) {
  const client=new pg.Client({connectionString:url,connectionTimeoutMillis:10000})
  try { await client.connect();return client } catch { await client.end().catch(()=>{});throw new Error('ISOLATED_POSTGRES_CONNECTION_FAILED') }
}
async function invoke(args) {
  return new Promise((resolve,reject)=>{
    const child=spawn(process.execPath,[cli,...args],{shell:false,env:process.env,stdio:['ignore','pipe','pipe']})
    let stdout='',stderr='';child.stdout.on('data',c=>stdout+=c);child.stderr.on('data',c=>stderr+=c)
    child.once('error',()=>reject(new Error('CLI_START_FAILED')))
    child.once('close',code=>{const text=code === 0 ? stdout : stderr;let json;try{json=JSON.parse(text.trim())}catch{reject(new Error('CLI_JSON_REQUIRED'));return}resolve({code,stdout,stderr,json})})
  })
}
async function snapshot(client,tables) {
  const result={}
  for (const table of tables) result[table]=(await client.query(`SELECT COALESCE(json_agg(x ORDER BY x::text),'[]'::json)::text AS rows FROM (SELECT to_jsonb(t) AS x FROM ${table} t${table==='audit_logs' ? " WHERE action<>'RECOVERY_AUTH_RESET'" : ''}) s`)).rows[0].rows
  return result
}

test('actual PG17 dual encrypted backup, failure guards, full restore and mandatory auth reset', {timeout:180000},async t=>{
  const directory=await mkdtemp(path.join(os.tmpdir(),'saas-recovery-pg-'));await chmod(directory,0o700)
  const suffix=randomBytes(7).toString('hex'), ownedNames=[], clients=[]
  const sourcePlatform=`saas_bkp_p_${suffix}`,sourceBusiness=`saas_bkp_b_${suffix}`
  const restoredPlatform=`saas_rstr_p_${suffix}`,restoredBusiness=`saas_rstr_b_${suffix}`
  const admin=await connected(server.toString())
  const existingNames=[sourcePlatform,sourceBusiness,restoredPlatform,restoredBusiness]
  for(const name of existingNames) assert.equal((await admin.query('SELECT count(*)::int n FROM pg_database WHERE datname=$1',[name])).rows[0].n,0)
  t.after(async()=>{
    await Promise.allSettled(clients.map(c=>c.end()))
    for(const name of ownedNames) {
      assert.match(name,new RegExp(`^saas_(?:bkp|rstr)_[pb]_${suffix}$`))
      await admin.query('SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=$1 AND pid<>pg_backend_pid()',[name])
      await admin.query(`DROP DATABASE "${name}"`)
    }
    const remaining=(await admin.query('SELECT count(*)::int n FROM pg_database WHERE datname=ANY($1)',[existingNames])).rows[0].n
    await admin.end();await rm(directory,{recursive:true,force:true})
    assert.equal(remaining,0)
    t.diagnostic('resource audit: own source/restore databases=0, own PG clients=0, own temporary backup/key/config directory removed')
  })
  for(const name of [sourcePlatform,sourceBusiness]) {await admin.query(`CREATE DATABASE "${name}" TEMPLATE template0`);ownedNames.push(name)}
  const platform=await connected(makeConnection(sourcePlatform)),business=await connected(makeConnection(sourceBusiness));clients.push(platform,business)
  await platform.query('CREATE TABLE _sqlx_migrations(version BIGINT PRIMARY KEY,description TEXT NOT NULL,installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),success BOOLEAN NOT NULL,checksum BYTEA NOT NULL,execution_time BIGINT NOT NULL)')
  const migrations=(await readdir(path.join(root,'backend/migrations'))).filter(f=>/^\d{4}_.+\.sql$/.test(f)).sort()
  for(const name of migrations){const sql=await readFile(path.join(root,'backend/migrations',name));await platform.query(sql.toString());await platform.query('INSERT INTO _sqlx_migrations(version,description,success,checksum,execution_time) VALUES($1,$2,true,$3,0)',[Number(name.slice(0,4)),name,createHash('sha384').update(sql).digest()])}
  const ids=Object.fromEntries(['tenant','org','user','app','membership','session','publication'].map(k=>[k,randomUUID()]))
  await platform.query('INSERT INTO tenants(id,name,code) VALUES($1,$2,$3)',[ids.tenant,'Recovery fixture','recovery-'+suffix])
  await platform.query('INSERT INTO organizations(id,tenant_id,name) VALUES($1,$2,$3)',[ids.org,ids.tenant,'Recovery org'])
  await platform.query('INSERT INTO users(id,username,password_hash,session_version) VALUES($1,$2,$3,7)',[ids.user,'recovery-'+suffix,'fixture_hash_not_a_login'])
  await platform.query('INSERT INTO applications(id,code,name) VALUES($1,$2,$3)',[ids.app,'work-orders','工单'])
  await platform.query('INSERT INTO memberships(id,user_id,organization_id) VALUES($1,$2,$3)',[ids.membership,ids.user,ids.org])
  await platform.query("INSERT INTO membership_data_grants(membership_id,application_id,resource_code,action_code,scope_type) VALUES($1,$2,'ticket','read','SELF')",[ids.membership,ids.app])
  await platform.query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at) VALUES($1,$2,$3,$4,now()+interval '1 day')",[ids.session,ids.user,ids.tenant,ids.org])
  await platform.query("INSERT INTO oauth_tokens(token_hash,token_type,client_id,user_id,tenant_id,organization_id,expires_at) VALUES('fixture_access','access','fixture',$1,$2,$3,now()+interval '1 day'),('fixture_refresh','refresh','fixture',$1,$2,$3,now()+interval '1 day')",[ids.user,ids.tenant,ids.org])
  await platform.query("INSERT INTO oauth_authorization_requests(client_id,redirect_uri,expires_at) VALUES('fixture','https://fixture.invalid',now()+interval '1 day')")
  await platform.query("INSERT INTO oauth_authorization_codes(code_hash,client_id,user_id,redirect_uri,expires_at) VALUES('fixture_code','fixture',$1,'https://fixture.invalid',now()+interval '1 day')",[ids.user])
  await platform.query("INSERT INTO oauth_device_codes(device_code_hash,user_code,client_id,status,expires_at) VALUES('fixture_device','FIXTURE','fixture','APPROVED',now()+interval '1 day')")
  await platform.query("INSERT INTO audit_logs(action,detail,tenant_id,user_id) VALUES('FIXTURE_RETAINED','original audit',$1,$2)",[ids.tenant,ids.user])
  await platform.query("INSERT INTO notification_publications(id,tenant_id,publisher_organization_id,application_id,publisher_user_id,publisher_username,application_name,idempotency_key,request_hash,request_body,recipient_count,delivery_count) VALUES($1,$2,$3,$4,$5,'fixture','工单','fixture',$6,'{}',1,1)",[ids.publication,ids.tenant,ids.org,ids.app,ids.user,'a'.repeat(64)])
  await platform.query("INSERT INTO notifications(user_id,title,content,source_kind,tenant_id,organization_id,application_id,publication_id) VALUES($1,'提醒','保留通知','APP',$2,$3,$4,$5)",[ids.user,ids.tenant,ids.org,ids.app,ids.publication])
  const businessSql=await readFile(path.join(root,'business-apps/work-orders/backend/migrations/0001_work_orders.sql'))
  await business.query('CREATE TABLE work_order_migrations(version TEXT PRIMARY KEY,checksum CHAR(64) NOT NULL,applied_at TIMESTAMPTZ NOT NULL DEFAULT now())')
  await business.query(businessSql.toString());await business.query('INSERT INTO work_order_migrations(version,checksum) VALUES($1,$2)',['0001_work_orders.sql',createHash('sha256').update(businessSql).digest('hex')])
  await business.query('INSERT INTO work_order_counters(tenant_id,next_number) VALUES($1,3)',[ids.tenant])
  await business.query("INSERT INTO work_orders(id,number,tenant_id,organization_id,organization_name,owner_user_id,owner_display_name,title,deleted_at) VALUES($1,'WO-1',$2,$3,'Recovery org',$4,'Recovery owner','保留工单',NULL),($5,'WO-2',$2,$3,'Recovery org',$4,'Recovery owner','已软删除',now())",[randomUUID(),ids.tenant,ids.org,ids.user,randomUUID()])
  const preserved=['tenants','organizations','memberships','membership_data_grants','applications','audit_logs','notifications','notification_publications','_sqlx_migrations']
  const beforePlatform=await snapshot(platform,preserved),beforeBusiness=await snapshot(business,['work_orders','work_order_counters','work_order_migrations'])
  const keyFile=path.join(directory,'backup.key'),wrongKey=path.join(directory,'wrong.key'),envFile=path.join(directory,'backup.env'),restoreEnv=path.join(directory,'restore.env')
  await writeFile(keyFile,randomBytes(32),{mode:0o600});await writeFile(wrongKey,randomBytes(32),{mode:0o600})
  const toolConfig={BACKUP_KEY_FILE:keyFile,PG_DUMP_BIN:path.join(clientDirectory,'pg_dump'),PG_RESTORE_BIN:path.join(clientDirectory,'pg_restore'),PSQL_BIN:path.join(clientDirectory,'psql'),PG_CLIENT_LIBRARY_PATH:libraryDirectory}
  const config={...toolConfig,BACKUP_ROOT:path.join(directory,'backups'),BACKUP_WRITES_QUIESCED:'true',BACKUP_PLATFORM_DATABASE_URL:makeConnection(sourcePlatform),BACKUP_WORK_ORDERS_DATABASE_URL:makeConnection(sourceBusiness)}
  await t.test('actual psql SQLSTATE diagnostics redact SQL exception values',async()=>{
    const secret='private_exception_fixture_value'
    await assert.rejects(psql({values:toolConfig},parseDatabaseUrl(makeConnection(sourcePlatform)),`DO $$ BEGIN RAISE EXCEPTION '${secret}' USING ERRCODE='23514'; END $$;`),error=>{
      assert.equal(error.code,'POSTGRES_TOOL_FAILED');assert.equal(error.pgFailureClass,'CONSTRAINT');assert.equal(error.sqlState,'23514');assert.equal(error.pgTool,'psql');assert.equal(error.processExitCode,3);assert.equal(JSON.stringify(error).includes(secret),false);return true
    })
  })
  const writeEnv=async(file,values)=>writeFile(file,Object.entries(values).map(([key,value])=>`${key}=${JSON.stringify(value)}`).join('\n')+'\n',{mode:0o600})
  await writeEnv(envFile,config)
  let set
  await t.test('actual immutable encrypted PG custom dumps and authenticated inspect',async()=>{
    const result=await invoke(['backup','--env-file',envFile]);assert.equal(result.code,0,result.json.code);set=result.json.backupSet
    assert.equal(result.json.clients.PG_DUMP_BIN,'17.11')
    const verified=await invoke(['inspect','--env-file',envFile,'--backup-set',set]);assert.equal(verified.code,0,verified.json.code)
    assert.equal(verified.json.databases.platform.migrations.length,migrations.length)
    for(const artifact of ['platform.enc','work-orders.enc','manifest.enc']) {const contents=await readFile(path.join(set,artifact));assert.equal(contents.includes(Buffer.from('fixture_hash_not_a_login')),false);assert.equal(contents.includes(Buffer.from('保留工单')),false)}
    assert.equal((await readdir(set)).length,3)
  })
  const restoreConfig={...toolConfig,RESTORE_PLATFORM_MAINTENANCE_URL:server.toString(),RESTORE_WORK_ORDERS_MAINTENANCE_URL:server.toString(),RESTORE_PLATFORM_DATABASE:restoredPlatform,RESTORE_WORK_ORDERS_DATABASE:restoredBusiness}
  const targetsAbsent=async()=>assert.equal((await admin.query('SELECT count(*)::int n FROM pg_database WHERE datname=ANY($1)',[[restoredPlatform,restoredBusiness]])).rows[0].n,0)
  await t.test('wrong key and damaged second dump reject before either CREATE DATABASE',async()=>{
    await writeEnv(restoreEnv,{...restoreConfig,BACKUP_KEY_FILE:wrongKey})
    const wrong=await invoke(['restore','--env-file',restoreEnv,'--backup-set',set]);assert.equal(wrong.code,1);assert.equal(wrong.json.code,'ARTIFACT_AUTHENTICATION_FAILED');await targetsAbsent()
    const damaged=path.join(directory,'tampered');await cp(set,damaged,{recursive:true});await chmod(damaged,0o700)
    const file=path.join(damaged,'work-orders.enc'),contents=await readFile(file);contents[contents.length-20]^=1;await writeFile(file,contents,{mode:0o600})
    await writeEnv(restoreEnv,restoreConfig)
    const tampered=await invoke(['restore','--env-file',restoreEnv,'--backup-set',damaged]);assert.equal(tampered.code,1);assert.equal(tampered.json.code,'ARTIFACT_AUTHENTICATION_FAILED');await targetsAbsent()
  })
  await t.test('existing second target and maintenance-name guard do not create first target',async()=>{
    await writeEnv(restoreEnv,{...restoreConfig,RESTORE_WORK_ORDERS_DATABASE:sourceBusiness})
    const existing=await invoke(['restore','--env-file',restoreEnv,'--backup-set',set]);assert.equal(existing.code,1);assert.equal(existing.json.code,'RESTORE_TARGET_EXISTS');await targetsAbsent()
    await writeEnv(restoreEnv,{...restoreConfig,RESTORE_PLATFORM_DATABASE:'postgres'})
    const maintenance=await invoke(['restore','--env-file',restoreEnv,'--backup-set',set]);assert.equal(maintenance.code,1);assert.equal(maintenance.json.code,'INVALID_RESTORE_TARGET');await targetsAbsent()
  })
  await t.test('restore preserves original data/checksums and irrevocably resets all snapshot authentication',async()=>{
    await writeEnv(restoreEnv,restoreConfig)
    const result=await invoke(['restore','--env-file',restoreEnv,'--backup-set',set]);
    if (result.json.createdTargets) ownedNames.push(...result.json.createdTargets.map(x=>x.database))
    assert.equal(result.code,0,result.json.code);assert.equal(result.json.jwtKeyRotationRequired,true);assert.equal(result.json.reloginRequired,true)
    assert.deepEqual(result.json.authenticationReset,{users:1,sessions:1,tokens:2,requests:1,codes:1,devices:1})
    const restoredP=await connected(makeConnection(restoredPlatform)),restoredB=await connected(makeConnection(restoredBusiness));clients.push(restoredP,restoredB)
    assert.deepEqual(await snapshot(restoredP,preserved),beforePlatform);assert.deepEqual(await snapshot(restoredB,['work_orders','work_order_counters','work_order_migrations']),beforeBusiness)
    assert.equal((await restoredP.query('SELECT session_version FROM users')).rows[0].session_version,'8')
    assert.equal((await restoredP.query('SELECT count(*)::int n FROM platform_sessions WHERE revoked_at IS NULL')).rows[0].n,0)
    assert.equal((await restoredP.query('SELECT count(*)::int n FROM oauth_tokens WHERE revoked=false')).rows[0].n,0)
    assert.equal((await restoredP.query('SELECT count(*)::int n FROM oauth_authorization_codes WHERE consumed=false OR expires_at>now()')).rows[0].n,0)
    assert.equal((await restoredP.query('SELECT count(*)::int n FROM oauth_authorization_requests WHERE consumed=false OR expires_at>now()')).rows[0].n,0)
    assert.equal((await restoredP.query("SELECT count(*)::int n FROM oauth_device_codes WHERE status IN ('PENDING','APPROVED') OR expires_at>now()")).rows[0].n,0)
    const audit=(await restoredP.query("SELECT detail FROM audit_logs WHERE action='RECOVERY_AUTH_RESET'")).rows
    assert.equal(audit.length,1);assert.equal(JSON.parse(audit[0].detail).setId,result.json.setId);assert.equal(audit[0].detail.includes(ids.user),false)
    const repeated=await invoke(['restore','--env-file',restoreEnv,'--backup-set',set]);assert.equal(repeated.json.code,'RESTORE_TARGET_EXISTS')
    t.diagnostic(`restore drill ${result.json.durationMs} ms; target RTO 3600000 ms`);assert.ok(result.json.durationMs<3600000)
  })
  await t.test('connection-failure diagnostics and success output redact real credentials',async()=>{
    const unreachable=new URL(config.BACKUP_PLATFORM_DATABASE_URL);unreachable.hostname='127.0.0.1';unreachable.port='1'
    await writeEnv(envFile,{...config,BACKUP_PLATFORM_DATABASE_URL:unreachable.toString()})
    const error=await invoke(['backup','--env-file',envFile]);assert.equal(error.code,1);assert.match(error.json.code,/POSTGRES_TOOL_FAILED/)
    assert.equal(error.json.operationStage,'platform-backup-migrations');assert.equal(error.json.pgFailureClass,'CONNECTION');assert.equal(error.json.pgTool,'psql')
    const actualPassword=decodeURIComponent(new URL(base).password)
    for(const value of [base,unreachable.toString(),actualPassword].filter(v=>v.length>3)){assert.equal(error.stdout.includes(value),false);assert.equal(error.stderr.includes(value),false)}
    assert.equal(error.stderr.includes(new URL(base).hostname),false)
  })
})
