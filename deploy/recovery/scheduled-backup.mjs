#!/usr/bin/env node
import { readFile } from 'node:fs/promises'
import { parseEnv } from 'node:util'
import { spawn } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { safeFile, fail, RecoveryError } from './envelope.mjs'

const DEFAULT_SYSTEMCTL='/usr/bin/systemctl'
export function parseSchedule(values) {
  for(const key of Object.keys(values)) if(!['BACKUP_ENV_FILE','BACKUP_WRITE_UNITS'].includes(key)) fail('UNKNOWN_SCHEDULE_KEY')
  if(!path.isAbsolute(values.BACKUP_ENV_FILE || '')) fail('ABSOLUTE_ENV_FILE_REQUIRED')
  const units=(values.BACKUP_WRITE_UNITS || '').split(',').map(v=>v.trim())
  if(!units.length || units.length>12 || new Set(units).size!==units.length || units.some(v=>!(/^[a-zA-Z0-9][a-zA-Z0-9@_.-]{0,100}\.service$/).test(v) || v.includes('backup'))) fail('INVALID_WRITE_UNITS')
  return { envFile:values.BACKUP_ENV_FILE,units }
}
function run(program,args,{env=process.env,signal}={}) {
  return new Promise((resolve,reject)=>{
    const child=spawn(program,args,{shell:false,env,signal,stdio:['ignore','pipe','pipe'],timeout:600000})
    let stdout='';child.stdout.on('data',chunk=>{if(stdout.length<1024*1024)stdout+=chunk.toString()});child.stderr.resume()
    child.once('error',()=>reject(new RecoveryError('SCHEDULE_PROCESS_FAILED')))
    child.once('close',code=>resolve({code,stdout:stdout.trim()}))
  })
}

// Adapter is injectable only through the trusted module API, never schedule env or CLI.
export async function runScheduled(schedule,adapter) {
  const active=[]
  let result, failure
  try {
    for(const unit of schedule.units) {
      const state=await adapter.state(unit)
      if(!['active','inactive','failed'].includes(state)) fail('WRITE_SERVICE_STATE_UNSAFE')
      if(state==='active') active.push(unit)
    }
    for(const unit of active) {
      await adapter.stop(unit)
      if(await adapter.state(unit)!=='inactive') fail('WRITE_SERVICE_NOT_QUIESCED')
    }
    result=await adapter.backup(schedule.envFile)
  } catch(error) {failure=error}
  finally {
    // Restart only units that were active, on both success and failure.
    for(const unit of active) {
      try {await adapter.start(unit);if(await adapter.state(unit,{recovering:true})!=='active')fail('WRITE_SERVICE_RESTART_FAILED')}
      catch {failure=new RecoveryError('WRITE_SERVICE_RESTART_FAILED')}
    }
  }
  if(failure) throw failure
  return {...result,event:'scheduled-backup-complete',restartedWriteServices:active.length}
}

export async function main(argv=process.argv.slice(2)) {
  const abort=new AbortController()
  const terminate=()=>abort.abort()
  process.on('SIGTERM',terminate);process.on('SIGINT',terminate)
  try {
    if(process.platform!=='linux' || Number(process.versions.node.split('.')[0])!==24)fail('LINUX_NODE24_REQUIRED')
    if(argv.length!==2 || argv[0]!=='--schedule-file' || !path.isAbsolute(argv[1]))fail('INVALID_ARGUMENTS')
    await safeFile(argv[1])
    let values
    try{values=parseEnv(await readFile(argv[1],'utf8'))}catch{fail('INVALID_SCHEDULE_FILE')}
    const schedule=parseSchedule(values);await safeFile(schedule.envFile)
    const adapter={
      async state(unit,{recovering=false}={}){const result=await run(DEFAULT_SYSTEMCTL,['is-active',unit],{signal:recovering?undefined:abort.signal});return result.stdout},
      async stop(unit){if((await run(DEFAULT_SYSTEMCTL,['stop',unit],{signal:abort.signal})).code!==0)fail('WRITE_SERVICE_STOP_FAILED')},
      async start(unit){if((await run(DEFAULT_SYSTEMCTL,['start',unit])).code!==0)fail('WRITE_SERVICE_RESTART_FAILED')},
      async backup(envFile){
        const result=await run(process.execPath,[fileURLToPath(new URL('./pg-backup.mjs',import.meta.url)),'backup','--env-file',envFile],{signal:abort.signal})
        if(result.code!==0)fail('SCHEDULE_BACKUP_FAILED')
        let json;try{json=JSON.parse(result.stdout)}catch{fail('SCHEDULE_BACKUP_FAILED')}
        if(json.event!=='backup-complete')fail('SCHEDULE_BACKUP_FAILED')
        return json
      },
    }
    process.stdout.write(`${JSON.stringify(await runScheduled(schedule,adapter))}\n`)
  }catch(error){process.stderr.write(`${JSON.stringify({event:'scheduled-backup-error',code:error instanceof RecoveryError?error.code:'SCHEDULE_BACKUP_FAILED'})}\n`);process.exitCode=1}
  finally{process.removeListener('SIGTERM',terminate);process.removeListener('SIGINT',terminate)}
}
if(process.argv[1] && path.resolve(process.argv[1])===fileURLToPath(import.meta.url))await main()
