#!/usr/bin/env node
import { randomUUID } from 'node:crypto'
import { mkdir, mkdtemp, chmod, rm, open, rename, realpath, statfs } from 'node:fs/promises'
import { createWriteStream } from 'node:fs'
import { Readable, Writable } from 'node:stream'
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'
import { encryptStream, decryptStream, containedFile, fail, RecoveryError } from './envelope.mjs'
import { loadConfig, safeDirectory } from './config.mjs'
import { pgProcess, schemaMetadata, toolVersions, assertNewTargets, psql, resetRestoredAuthentication } from './postgres.mjs'

const ROLES = ['platform','work-orders']
const noop = () => new Writable({ write(_chunk,_encoding,callback) { callback() } })
const now = () => new Date().toISOString()
const OPERATION_STAGES=new Set(['backup-client-check','platform-backup-migrations','work-orders-backup-migrations','platform-dump','work-orders-dump','restore-manifest','restore-volatile-space','restore-artifact-verification','restore-client-check','restore-target-check','platform-create','work-orders-create','platform-dump-restore','work-orders-dump-restore','platform-auth-reset','platform-migration-verify','work-orders-migration-verify','restore-temporary-cleanup'])
export async function withOperationStage(stage,operation) {
  if(!OPERATION_STAGES.has(stage))fail('INVALID_OPERATION_STAGE')
  try{return await operation()}catch(error){error.operationStage=stage;throw error}
}

async function privateTemporaryDirectory() {
  // A fully authenticated dump must exist before database mutation. Spool only to
  // volatile tmpfs, never to a durable plaintext backup or disk temporary file.
  const candidates=[os.tmpdir(),'/dev/shm']
  let base
  for(const candidate of candidates) if((await statfs(candidate).catch(()=>null))?.type===0x01021994){base=candidate;break}
  if(!base)fail('VOLATILE_RESTORE_SPACE_REQUIRED')
  const directory = await mkdtemp(path.join(base,'saas-recovery-'))
  await chmod(directory,0o700)
  return directory
}
async function removeOwnTemporary(directory) {
  const resolved = path.resolve(directory)
  if (![path.resolve(os.tmpdir()),'/dev/shm'].includes(path.dirname(resolved)) || !path.basename(resolved).startsWith('saas-recovery-') || (await statfs(resolved)).type!==0x01021994) fail('UNSAFE_TEMPORARY_CLEANUP')
  await safeDirectory(resolved)
  await rm(resolved,{ recursive:true,force:true })
}
function validateManifest(manifest) {
  if (manifest?.format !== 1 || !/^[0-9a-f-]{36}$/.test(manifest.setId || '') || !manifest.startedAt || !manifest.completedAt || manifest.writesQuiesced !== true || !manifest.databases || Object.keys(manifest.databases).sort().join(',') !== [...ROLES].sort().join(',')) fail('INVALID_BACKUP_MANIFEST')
  for (const role of ROLES) {
    const db = manifest.databases[role]
    if (db.filename !== `${role}.enc` || !/^[0-9a-f]{64}$/.test(db.sha256 || '') || !Number.isSafeInteger(db.bytes) || db.bytes <= 0 || !Array.isArray(db.metadata?.migrations) || !db.metadata.migrations.length) fail('INVALID_BACKUP_MANIFEST')
  }
  return manifest
}
async function readManifest(config) {
  const source = await containedFile(config.backupSet,'manifest.enc')
  const chunks = []
  let total = 0
  const output = new Writable({ write(chunk,_encoding,callback) {
    total += chunk.length
    if (total > 1024*1024) callback(new RecoveryError('INVALID_BACKUP_MANIFEST'))
    else { chunks.push(Buffer.from(chunk)); callback() }
  } })
  const { header } = await decryptStream(source,output,config.key,{ kind:'manifest' })
  let manifest
  try { manifest = JSON.parse(Buffer.concat(chunks).toString('utf8')) } catch { fail('INVALID_BACKUP_MANIFEST') }
  validateManifest(manifest)
  if (header.setId !== manifest.setId) fail('ARTIFACT_METADATA_MISMATCH')
  return manifest
}
async function verifyArtifacts(config, manifest, spoolDirectory) {
  const paths = {}
  for (const role of ROLES) {
    const db = manifest.databases[role]
    const destination = spoolDirectory ? path.join(spoolDirectory,`${role}.dump`) : undefined
    const output = destination ? createWriteStream(destination,{ flags:'wx',mode:0o600 }) : noop()
    const result = await decryptStream(await containedFile(config.backupSet,db.filename),output,config.key,{ kind:'database',role,setId:manifest.setId })
    if (result.sha256 !== db.sha256 || result.bytes !== db.bytes) fail('ARTIFACT_DIGEST_MISMATCH')
    if (destination) paths[role] = destination
  }
  return paths
}

export async function backup(config) {
  const started = Date.now(), startedAt = now(), setId = randomUUID()
  await mkdir(config.values.BACKUP_ROOT,{ recursive:true,mode:0o700 })
  const root = await safeDirectory(config.values.BACKUP_ROOT)
  const keyRelative = path.relative(root,await realpath(config.values.BACKUP_KEY_FILE))
  if (!keyRelative || (!keyRelative.startsWith(`..${path.sep}`) && keyRelative !== '..' && !path.isAbsolute(keyRelative))) fail('KEY_MUST_BE_OUTSIDE_BACKUPS')
  const name = `${startedAt.replace(/[:.]/g,'-')}-${setId}`
  const temporary = path.join(root,`.incomplete-${setId}`), final = path.join(root,name)
  await mkdir(temporary,{ mode:0o700 })
  const databases = {}
  const versions = await withOperationStage('backup-client-check',()=>toolVersions(config,config.sources.platform))
  for (const role of ROLES) {
    const metadata = await withOperationStage(`${role}-backup-migrations`,()=>schemaMetadata(config,role,config.sources[role]))
    if (Math.floor(Number(metadata.serverVersionNumber)/10000) !== 17) fail('POSTGRES17_SOURCE_REQUIRED')
    const { child, completion } = pgProcess(config,'PG_DUMP_BIN',['--format=custom','--no-password','--no-owner','--no-privileges'],config.sources[role])
    child.stdin.end()
    let digest
    try { digest = await encryptStream(child.stdout,path.join(temporary,`${role}.enc`),config.key,{ kind:'database',role,setId,startedAt }); await completion }
    catch(error) { child.kill();let clientFailure;await completion.catch(cause=>{clientFailure=cause});const failure=clientFailure || error;failure.operationStage=`${role}-dump`;if(!(failure instanceof RecoveryError))failure.code='DATABASE_BACKUP_FAILED';throw failure }
    databases[role] = { filename:`${role}.enc`,...digest,metadata }
  }
  const completedAt = now()
  const manifest = { format:1,setId,startedAt,completedAt,writesQuiesced:true,clients:versions,databases }
  await encryptStream(Readable.from([Buffer.from(JSON.stringify(manifest))]),path.join(temporary,'manifest.enc'),config.key,{ kind:'manifest',setId })
  // Validate our durable encrypted artifacts before publishing the immutable set.
  const verification = { ...config,backupSet:temporary }
  await verifyArtifacts(verification,await readManifest(verification))
  for (const name of ['platform.enc','work-orders.enc','manifest.enc']) {
    const handle = await open(path.join(temporary,name),'r')
    await handle.sync(); await handle.close()
  }
  await rename(temporary,final)
  const directory = await open(root,'r'); await directory.sync(); await directory.close()
  return { event:'backup-complete',backupSet:final,setId,startedAt,completedAt,durationMs:Date.now()-started,clients:versions }
}

export async function inspect(config) {
  const started = Date.now(), manifest = await readManifest(config)
  await verifyArtifacts(config,manifest)
  return { event:'backup-verified',setId:manifest.setId,startedAt:manifest.startedAt,completedAt:manifest.completedAt,durationMs:Date.now()-started,clients:manifest.clients,databases:Object.fromEntries(ROLES.map(role => [role,{ bytes:manifest.databases[role].bytes,sha256:manifest.databases[role].sha256,...manifest.databases[role].metadata }])) }
}

export async function restore(config) {
  const started = Date.now(), manifest = await withOperationStage('restore-manifest',()=>readManifest(config)), createdTargets = []
  let authenticationReset
  const temporary = await withOperationStage('restore-volatile-space',()=>privateTemporaryDirectory())
  try {
    // No database subprocess or CREATE is allowed before every GCM tag and digest passes.
    const dumps = await withOperationStage('restore-artifact-verification',()=>verifyArtifacts(config,manifest,temporary))
    await withOperationStage('restore-client-check',()=>toolVersions(config,config.targets.platform.maintenance))
    await withOperationStage('restore-target-check',()=>assertNewTargets(config))
    for (const role of ROLES) {
      const target = config.targets[role]
      await withOperationStage(`${role}-create`,()=>psql(config,target.maintenance,`CREATE DATABASE "${target.name}" TEMPLATE template0;`))
      createdTargets.push({ role,database:target.name })
      const { child, completion } = pgProcess(config,'PG_RESTORE_BIN',['--no-password','--dbname',target.name,'--single-transaction','--exit-on-error','--no-owner','--no-privileges',dumps[role]],target.maintenance,target.name)
      child.stdin.end(); child.stdout.resume(); await withOperationStage(`${role}-dump-restore`,()=>completion)
      // Reset immediately after restoring the platform, including if the later
      // business restore fails. New targets must remain isolated until success.
      if(role==='platform')authenticationReset=await withOperationStage('platform-auth-reset',()=>resetRestoredAuthentication(config,manifest.setId))
      await withOperationStage(`${role}-migration-verify`,async()=>{
        const restored=await schemaMetadata(config,role,target.maintenance,target.name)
        if(JSON.stringify(restored.migrations)!==JSON.stringify(manifest.databases[role].metadata.migrations))fail('RESTORED_MIGRATIONS_DIFFER')
      })
    }
    return { event:'restore-complete',setId:manifest.setId,createdTargets,authenticationReset,jwtKeyRotationRequired:true,reloginRequired:true,durationMs:Date.now()-started,migrations:Object.fromEntries(ROLES.map(role => [role,manifest.databases[role].metadata.migrations])) }
  } catch (error) { error.createdTargets = createdTargets; throw error }
  finally { await withOperationStage('restore-temporary-cleanup',()=>removeOwnTemporary(temporary)) }
}

export async function main(argv = process.argv.slice(2)) {
  let config
  try {
    const mode = argv.shift()
    if (!['backup','inspect','restore'].includes(mode) || argv.length % 2) fail('INVALID_ARGUMENTS')
    const options = {}
    while (argv.length) { const key = argv.shift(), value = argv.shift(); if (!['--env-file','--backup-set'].includes(key) || options[key] || !value) fail('INVALID_ARGUMENTS'); options[key] = value }
    if (!options['--env-file'] || (mode !== 'backup' && !options['--backup-set']) || (mode === 'backup' && options['--backup-set'])) fail('INVALID_ARGUMENTS')
    config = await loadConfig(options['--env-file'],mode,options['--backup-set'])
    const result = await ({ backup,inspect,restore })[mode](config)
    process.stdout.write(`${JSON.stringify(result)}\n`)
  } catch (error) {
    const result = { event:'recovery-error',code:error instanceof RecoveryError ? error.code : 'RECOVERY_FAILED' }
    if(OPERATION_STAGES.has(error.operationStage))result.operationStage=error.operationStage
    if(['UNKNOWN','TOOL_EXECUTION','TIMEOUT_OR_CANCELLED','AUTHENTICATION','PERMISSION','CONSTRAINT','CONNECTION','RESOURCE','DATABASE','ARCHIVE','SQL_OR_SERVER','PROCESS_SIGNAL'].includes(error.pgFailureClass))result.pgFailureClass=error.pgFailureClass
    if(/^[0-9A-Z]{5}$/.test(error.sqlState || ''))result.sqlState=error.sqlState
    if(['pg-dump','pg-restore','psql'].includes(error.pgTool))result.pgTool=error.pgTool
    if(Number.isInteger(error.processExitCode) && error.processExitCode>=0 && error.processExitCode<=255)result.processExitCode=error.processExitCode
    if(['SIGTERM','SIGKILL','SIGINT','SIGABRT','SIGSEGV','SIGPIPE'].includes(error.processSignal))result.processSignal=error.processSignal
    if (error.createdTargets?.length) result.createdTargets = error.createdTargets
    process.stderr.write(`${JSON.stringify(result)}\n`)
    process.exitCode = 1
  } finally { config?.key.fill(0) }
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main()
