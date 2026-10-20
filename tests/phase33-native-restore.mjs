/** Owned Linux/PostgreSQL native restore regression; not a production Node service. */
import assert from 'node:assert/strict'
import {createRequire} from 'node:module'
import {spawn} from 'node:child_process'
import{randomUUID,randomBytes}from'node:crypto'
import{mkdirSync,readFileSync,writeFileSync,mkdtempSync}from'node:fs'
import net from'node:net'
import{validateOwnedPostgres}from'../scripts/ci/postgres.mjs'
import{isAbsolute}from'node:path'
assert.equal(process.platform,'linux');validateOwnedPostgres(process.env)
for(const n of ['STORES_BINARY','WORK_ORDERS_BINARY'])assert(isAbsolute(process.env[n]||''),'Explicit native ELF required')
const pg=createRequire(new URL('../mini-services/notification-ws/package.json',import.meta.url))('pg'),adminURL=new URL(process.env.TEST_DATABASE_URL),admin=new pg.Client({connectionString:adminURL.href});await admin.connect()
const root=mkdtempSync('/scratch/p33-native-'),report={kind:'native-schema-diagnostic',components:[]};process.umask(0o077)
async function command(program,args,env=process.env){const p=spawn(program,args,{env,stdio:['ignore','pipe','pipe']});let log='';p.stdout.on('data',b=>log+=b);p.stderr.on('data',b=>log+=b);const exit=await new Promise(r=>p.on('close',r));assert.equal(exit,0,program+': '+log);return log}
async function port(){const s=net.createServer();await new Promise(r=>s.listen(0,'127.0.0.1',r));const p=s.address().port;await new Promise(r=>s.close(r));return p}
async function start(app,url){const n=await port(),prefix=app==='stores'?'STORES':'WORK_ORDERS',p=spawn(process.env[app==='stores'?'STORES_BINARY':'WORK_ORDERS_BINARY'],[],{env:{PATH:process.env.PATH,LD_LIBRARY_PATH:process.env.LD_LIBRARY_PATH,[prefix+'_DATABASE_URL']:url,[prefix+'_HOST']:'127.0.0.1',[prefix+'_PORT']:String(n),[prefix+'_ALLOWED_ORIGINS']:'null',PLATFORM_API_ORIGIN:'https://127.0.0.1:1',PLATFORM_CLIENT_ID:'phase33-schema-diagnostic',PLATFORM_CLIENT_SECRET:randomBytes(32).toString('hex'),NODE_ENV:'production'},stdio:['ignore','pipe','pipe']});let log='';p.stdout.on('data',b=>log+=b);p.stderr.on('data',b=>log+=b);let ready=false;const deadline=Date.now()+20000;while(Date.now()<deadline&&p.exitCode===null){try{if((await fetch('http://127.0.0.1:'+n+'/'+(app==='stores'?'stores':'work-orders')+'-api/health/ready',{signal:AbortSignal.timeout(500)})).status===200){ready=true;break}}catch{}await new Promise(r=>setTimeout(r,30))}return{p,ready,get log(){return log}}}
async function stop(s){if(s.p.exitCode===null){s.p.kill('SIGTERM');await new Promise(r=>s.p.once('close',r))}}
async function client(url){const c=new pg.Client({connectionString:url});await c.connect();return c}
for(const app of ['stores','work-orders']){
  const id=randomUUID().replaceAll('-',''),source='p33diag_'+id,target='recovery_diag_'+id,role='p33role_'+id,password=randomBytes(24).toString('hex'),u=new URL(adminURL);u.pathname='/'+source;u.username=role;u.password=password
  let a,b,s,t,sourceCreated=false,targetCreated=false,roleCreated=false;const item={app,source,target};report.components.push(item)
  try{
    await admin.query('CREATE ROLE "'+role+'" LOGIN PASSWORD '+"'"+password+"'"+' NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION');roleCreated=true;await admin.query('CREATE DATABASE "'+source+'" OWNER "'+role+'" TEMPLATE template0');sourceCreated=true
    s=await start(app,u.href);item.sourceReady=s.ready;item.sourceLog=s.log;assert(s.ready,'Source must start')
    a=await client(u.href);await a.query("SELECT set_config('search_path','pg_catalog,public,pg_temp',false)")
    const catalogSQL=readFileSync(new URL('../deploy/recovery/catalog.sql',import.meta.url),'utf8');const catalogResult=await a.query(catalogSQL);const sourceCatalog=(Array.isArray(catalogResult)?catalogResult.at(-1):catalogResult).rows
    const migrations=(await a.query("SELECT version::text,encode(checksum,'hex') AS checksum FROM _sqlx_migrations ORDER BY version")).rows;item.sourceMigrations=migrations
    let body,sourceItems
    if(app==='stores'){
      item.sourceFingerprint=(await a.query('SELECT fingerprint,stores_schema_fingerprint() AS actual FROM _stores_schema_contract')).rows
      body=(await a.query("SELECT prosrc FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public' AND p.proname='stores_schema_fingerprint'")).rows[0].prosrc
      const inner=body.match(/FROM \(([\s\S]*)\) contracts/)[1];sourceItems=(await a.query('SELECT item FROM ('+inner+') contracts ORDER BY item')).rows.map(r=>r.item);item.innerQuery=inner
    }
    await stop(s)
    const pgEnv={...process.env,PGHOST:adminURL.hostname,PGPORT:adminURL.port,PGDATABASE:source,PGUSER:adminURL.username,PGPASSWORD:adminURL.password}
    await command('pg_dump',['-Fc','--no-owner','--no-privileges','-f',root+'/'+app+'.dump'],pgEnv)
    await admin.query('CREATE DATABASE "'+target+'" OWNER "'+role+'" TEMPLATE template0');targetCreated=true
    await command('pg_restore',['--dbname',target,'--role',role,'--no-owner','--no-privileges','--single-transaction','--exit-on-error',root+'/'+app+'.dump'],pgEnv)
    u.pathname='/'+target;b=await client(u.href);await b.query("SELECT set_config('search_path','pg_catalog,public,pg_temp',false)")
    item.restoredMigrations=(await b.query("SELECT version::text,encode(checksum,'hex') AS checksum FROM _sqlx_migrations ORDER BY version")).rows
    if(app==='stores'){
      item.restoredFingerprint=(await b.query('SELECT fingerprint,stores_schema_fingerprint() AS actual FROM _stores_schema_contract')).rows
      const restoredItems=(await b.query('SELECT item FROM ('+item.innerQuery+') contracts ORDER BY item')).rows.map(r=>r.item)
      item.removed=sourceItems.filter(v=>!restoredItems.includes(v));item.added=restoredItems.filter(v=>!sourceItems.includes(v));delete item.innerQuery
    }
    const repair=readFileSync(new URL('../deploy/recovery/canonical-'+app+'.sql',import.meta.url),'utf8');
    const malicious=app==='stores'?"ALTER TABLE stores DROP CONSTRAINT stores_business_status_check; ALTER TABLE stores ADD CONSTRAINT stores_business_status_check CHECK (business_status IN ('OPEN','CLOSED','UNKNOWN'));":"ALTER TABLE work_orders DROP CONSTRAINT work_orders_priority_check; ALTER TABLE work_orders ADD CONSTRAINT work_orders_priority_check CHECK (priority IN ('LOW','NORMAL','HIGH','URGENT'));";
    let rejected=false;try{await b.query('BEGIN;'+malicious+repair)}catch{rejected=true}await b.query('ROLLBACK');assert(rejected,'Unexpected constraint must fail closed');item.unknownConstraintRefused=true;
    await b.query(repair);const actualCatalog=await b.query(catalogSQL);assert.deepEqual((Array.isArray(actualCatalog)?actualCatalog.at(-1):actualCatalog).rows,sourceCatalog,'Authenticated portable source catalog must be identical');item.portableCatalogUnchanged=true;
    if(app==='stores')item.canonicalFingerprint=(await b.query('SELECT fingerprint,stores_schema_fingerprint() AS actual FROM _stores_schema_contract')).rows;
    t=await start(app,u.href);item.restoredReady=t.ready;item.restoredLog=t.log;await stop(t);assert(t.ready,'Canonical restored schema must pass the unchanged business verifier')
  }catch(e){item.error=e.message;process.exitCode=1}finally{await a?.end();await b?.end();if(s)await stop(s);if(t)await stop(t);for(const[db,created]of [[target,targetCreated],[source,sourceCreated]])if(created)await admin.query('DROP DATABASE "'+db+'" WITH (FORCE)');if(roleCreated)await admin.query('DROP ROLE "'+role+'"')}
}
await admin.end();writeFileSync(process.env.PHASE33_NATIVE_REPORT||'/reports/native-restore.json',JSON.stringify(report,null,2));console.log(JSON.stringify(report))
