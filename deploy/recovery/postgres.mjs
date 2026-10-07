import { spawn } from 'node:child_process'
import { fail, RecoveryError } from './envelope.mjs'

const SQLSTATE_CLASSES=new Set(['00','01','02','03','08','09','0A','0B','0F','0L','0P','0Z','20','21','22','23','24','25','26','27','28','2B','2D','2F','2H','30','34','38','39','3B','3D','3F','40','42','44','53','54','55','57','58','72','F0','HV','P0','XX'])
const TOOL_NAMES={PG_DUMP_BIN:'pg-dump',PG_RESTORE_BIN:'pg-restore',PSQL_BIN:'psql'}
const SIGNALS=new Set(['SIGTERM','SIGKILL','SIGINT','SIGABRT','SIGSEGV','SIGPIPE'])
export function classifyPostgresFailure(stderr='',{exitCode,signal,spawnCode}={}) {
  // Never return any original text. Only fixed classes and validated protocol codes.
  const text=String(stderr).slice(-32768)
  const match=text.match(/(?:\bSQLSTATE[ :=]+|\b(?:ERROR|FATAL|PANIC):\s+)([0-9A-Z]{5})(?=\s|:|$)/)
  const sqlState=match && SQLSTATE_CLASSES.has(match[1].slice(0,2)) ? match[1] : undefined
  let pgFailureClass='UNKNOWN'
  if(spawnCode)pgFailureClass='TOOL_EXECUTION'
  else if(sqlState==='57014' || /timeout expired|timed out|statement timeout|canceling statement|cancelled|canceled/i.test(text))pgFailureClass='TIMEOUT_OR_CANCELLED'
  else if(sqlState?.startsWith('28') || /password authentication failed|no password supplied|authentication failed|no pg_hba.conf entry/i.test(text))pgFailureClass='AUTHENTICATION'
  else if(sqlState==='42501' || /permission denied|must be owner|must be superuser/i.test(text))pgFailureClass='PERMISSION'
  else if(sqlState?.startsWith('23') || /violates .*constraint|duplicate key|foreign key constraint/i.test(text))pgFailureClass='CONSTRAINT'
  else if(sqlState?.startsWith('08') || ['57P01','57P02','57P03'].includes(sqlState) || /could not connect|connection to server|connection refused|server closed the connection|server closed connection|could not receive data|could not send data|ssl connection has been closed|ssl error|eof detected|connection reset|network is unreachable/i.test(text))pgFailureClass='CONNECTION'
  else if(sqlState?.startsWith('53') || /no space left|out of memory|too many connections|remaining connection slots/i.test(text))pgFailureClass='RESOURCE'
  else if(sqlState?.startsWith('3D') || /database .* does not exist|database .* already exists/i.test(text))pgFailureClass='DATABASE'
  else if(/unsupported version .*file header|input file does not appear|could not read from input file|could not open input file|unexpected end of file|invalid archive/i.test(text))pgFailureClass='ARCHIVE'
  else if(sqlState || /could not execute query|error:|fatal:|panic:/i.test(text))pgFailureClass='SQL_OR_SERVER'
  else if(SIGNALS.has(signal))pgFailureClass='PROCESS_SIGNAL'
  const result={pgFailureClass}
  if(sqlState)result.sqlState=sqlState
  if(Number.isInteger(exitCode) && exitCode>=0 && exitCode<=255)result.processExitCode=exitCode
  if(SIGNALS.has(signal))result.processSignal=signal
  return result
}

export function postgresEnv(config, connection, database = connection.database) {
  const env = { PATH: '/usr/bin:/bin', LANG: 'C', PGHOST: connection.host, PGPORT: connection.port, PGUSER: connection.user, PGPASSWORD: connection.password, PGDATABASE: database, PGSSLMODE: connection.sslmode, PGCONNECT_TIMEOUT: connection.connectTimeout, PGAPPNAME: 'saas-recovery' }
  if (connection.sslrootcert) env.PGSSLROOTCERT = connection.sslrootcert
  if (connection.sslcert) env.PGSSLCERT = connection.sslcert
  if (connection.sslkey) env.PGSSLKEY = connection.sslkey
  if (config.values.PG_CLIENT_LIBRARY_PATH) env.LD_LIBRARY_PATH = config.values.PG_CLIENT_LIBRARY_PATH
  // The acceptance host requires its process-local TCP helper; never inherit PG* overrides.
  if (process.env.LD_PRELOAD) env.LD_PRELOAD = process.env.LD_PRELOAD
  return env
}

export function pgProcess(config, tool, args, connection, database) {
  const child = spawn(config.values[tool], args, { shell: false, env: postgresEnv(config, connection, database), stdio: ['pipe','pipe','pipe'], timeout: 600000 })
  // Client stderr can include connection details or SQL values. Retain a bounded
  // private tail to classify failures, then expose only enums/protocol codes.
  let stderr=''
  child.stderr.on('data',chunk=>{stderr=(stderr+chunk.toString('utf8')).slice(-32768)})
  child.stdin.on('error',()=>{})
  const completion = new Promise((resolve,reject) => {
    const failed=(exitCode,signal,spawnCode)=>{const error=new RecoveryError('POSTGRES_TOOL_FAILED');Object.assign(error,classifyPostgresFailure(stderr,{exitCode,signal,spawnCode}),{pgTool:TOOL_NAMES[tool]});stderr='';reject(error)}
    child.once('error', error => failed(undefined,undefined,error.code || 'UNKNOWN'))
    child.once('close', (code,signal) => {if(code===0){stderr='';resolve()}else failed(code,signal)})
  })
  completion.catch(() => {})
  return { child, completion }
}

export async function psql(config, connection, sql, database) {
  const { child, completion } = pgProcess(config, 'PSQL_BIN', ['-X','--no-password','--quiet','--tuples-only','--no-align','--set=ON_ERROR_STOP=1','--set=VERBOSITY=sqlstate'], connection, database)
  let output = '', tooLarge = false
  child.stdout.on('data', chunk => {
    if (output.length + chunk.length > 4 * 1024 * 1024) { tooLarge = true; child.kill() }
    else output += chunk.toString('utf8')
  })
  child.stdin.end(sql)
  await completion
  if (tooLarge) fail('POSTGRES_RESULT_TOO_LARGE')
  return output.trim()
}

export async function schemaMetadata(config, role, connection, database) {
  const migration = role === 'platform'
    ? "SELECT json_agg(json_build_object('version',version::text,'checksum',encode(checksum,'hex'),'success',success) ORDER BY version) FROM _sqlx_migrations"
    : "SELECT json_agg(json_build_object('version',version,'checksum',trim(checksum)) ORDER BY version) FROM work_order_migrations"
  const text = await psql(config, connection, `SELECT json_build_object('serverVersion',current_setting('server_version'),'serverVersionNumber',current_setting('server_version_num'),'migrations',COALESCE((${migration}),'[]'::json))::text;`, database)
  let metadata
  try { metadata = JSON.parse(text) } catch { fail('SCHEMA_METADATA_UNAVAILABLE') }
  if (!Array.isArray(metadata.migrations) || !metadata.migrations.length || metadata.migrations.some(m => typeof m.version !== 'string' || !/^[0-9a-f]+$/.test(m.checksum || '') || m.success === false)) fail('INVALID_MIGRATION_METADATA')
  return metadata
}

export async function toolVersions(config, connection) {
  const versions = {}
  for (const key of ['PG_DUMP_BIN','PG_RESTORE_BIN','PSQL_BIN']) {
    const { child, completion } = pgProcess(config,key,['--version'],connection)
    let output = ''
    child.stdout.on('data', chunk => { if (output.length < 1000) output += chunk.toString('utf8') })
    child.stdin.end()
    await completion
    const match = output.match(/\(PostgreSQL\)\s+(\d+)\.(\d+)/)
    if (!match || match[1] !== '17') fail('POSTGRES17_CLIENT_REQUIRED')
    versions[key] = `${match[1]}.${match[2]}`
  }
  return versions
}

export async function assertNewTargets(config) {
  // Check BOTH before CREATE. CREATE itself remains the atomic guard against races.
  for (const target of Object.values(config.targets)) {
    const result = await psql(config,target.maintenance,`SELECT json_build_object('existing',(SELECT count(*) FROM pg_database WHERE datname='${target.name}'),'serverVersionNumber',current_setting('server_version_num'))::text;`)
    let guard
    try{guard=JSON.parse(result)}catch{fail('RESTORE_TARGET_CHECK_FAILED')}
    if(Math.floor(Number(guard.serverVersionNumber)/10000)!==17)fail('POSTGRES17_TARGET_REQUIRED')
    if(guard.existing!==0)fail('RESTORE_TARGET_EXISTS')
  }
}

export async function resetRestoredAuthentication(config, setId) {
  if (!/^[0-9a-f-]{36}$/.test(setId)) fail('INVALID_BACKUP_MANIFEST')
  const target = config.targets.platform
  const sql = `BEGIN;
DO $$ BEGIN IF EXISTS (SELECT 1 FROM users WHERE session_version=9223372036854775807) THEN RAISE EXCEPTION 'recovery version overflow'; END IF; END $$;
CREATE TEMP TABLE recovery_auth_counts(kind text,n bigint) ON COMMIT DROP;
WITH changed AS (UPDATE users SET session_version=session_version+1 RETURNING 1) INSERT INTO recovery_auth_counts SELECT 'users',count(*) FROM changed;
WITH changed AS (UPDATE platform_sessions SET revoked_at=COALESCE(revoked_at,now()) WHERE revoked_at IS NULL RETURNING 1) INSERT INTO recovery_auth_counts SELECT 'sessions',count(*) FROM changed;
WITH changed AS (UPDATE oauth_tokens SET revoked=true WHERE revoked=false RETURNING 1) INSERT INTO recovery_auth_counts SELECT 'tokens',count(*) FROM changed;
WITH changed AS (UPDATE oauth_authorization_requests SET consumed=true,expires_at=LEAST(expires_at,now()) WHERE consumed=false RETURNING 1) INSERT INTO recovery_auth_counts SELECT 'requests',count(*) FROM changed;
WITH changed AS (UPDATE oauth_authorization_codes SET consumed=true,expires_at=LEAST(expires_at,now()) WHERE consumed=false RETURNING 1) INSERT INTO recovery_auth_counts SELECT 'codes',count(*) FROM changed;
WITH changed AS (UPDATE oauth_device_codes SET status=CASE WHEN status IN ('PENDING','APPROVED') THEN 'EXPIRED' ELSE status END,expires_at=LEAST(expires_at,now()) WHERE expires_at>now() OR status IN ('PENDING','APPROVED') RETURNING 1) INSERT INTO recovery_auth_counts SELECT 'devices',count(*) FROM changed;
INSERT INTO audit_logs(action,detail) SELECT 'RECOVERY_AUTH_RESET',json_build_object('setId','${setId}','authenticationReset',true,'counts',json_object_agg(kind,n),'restoredAt',now())::text FROM recovery_auth_counts;
SELECT json_object_agg(kind,n)::text FROM recovery_auth_counts;
COMMIT;`
  const result = await psql(config,target.maintenance,sql,target.name)
  try { return JSON.parse(result) } catch { fail('AUTHENTICATION_RESET_FAILED') }
}
