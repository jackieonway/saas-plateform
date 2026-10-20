/** Isolated ZIP rehearsal. Only this test controller operates Docker; agents receive HTTPS data. */
import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { randomBytes, createHash } from 'node:crypto'
import { createWriteStream, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { pipeline } from 'node:stream/promises'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import https from 'node:https'
import { createSnapshot, validateOutputLocation } from './linux-runner.mjs'
import { createSourceManifest } from './provenance.mjs'
import { zipDistributionChecks, validateZipDistributionReport } from './zip-distribution-results.mjs'
import { resolveRehearsalImages } from './rehearsal-images.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const sha = b => createHash('sha256').update(b).digest('hex')
const json = (p,v) => writeFileSync(p,JSON.stringify(v,null,2)+'\n')
const slash = p => p.replaceAll('\\','/')
const linuxPath = p => process.platform === 'win32' ? slash(p).replace(/^([A-Za-z]):/, (_,d) => '/mnt/'+d.toLowerCase()) : p

export async function runZipDistribution(options) {
  const sourceRoot = path.resolve(options.sourceRoot || root), hostRoot=path.resolve(options.hostSourceRoot || sourceRoot), out=path.resolve(options.outDir)
  validateOutputLocation(out,sourceRoot); assert(!existsSync(out),'Fresh output required')
  for (const directory of ['', 'logs', 'original-binaries', 'screenshots', 'images']) mkdirSync(path.join(out,directory),{recursive:true})
  const owner='ci-phase35-'+Date.now()+'-'+randomBytes(4).toString('hex'), network=owner+'-network', tools=owner+'-tools', pg=owner+'-postgres', gateway=owner+'-gateway', registry=owner+'-registry'
  const privateRoot='/var/tmp/'+owner, secretValues=new Set(), containers=new Set(), images=new Set()
  const report={schemaVersion:1,kind:'zip-distribution',status:'FAIL',scope:'isolated-local',owner,adoptable:!options.development,productionOperations:false,startedAt:new Date().toISOString(),steps:[],checks:zipDistributionChecks.map(id=>({id,status:'NOT_RUN'})),commands:[],originalBinaries:[],cleanup:{errors:[]},registry:{scope:'owned-loopback-tls',normalTls:false},browser:{status:'NOT_RUN'},resources:{network,tools,pg,gateway,registry}}
  const save=()=>json(path.join(out,'result.json'),report)
  const clean=s=>{let v=String(s);for(const secret of secretValues)if(secret)v=v.replaceAll(secret,'[redacted]');return v.replace(/postgresql:\/\/[^\s"']+/g,'[database-url]').replace(/Bearer\s+[^\s"']+/gi,'Bearer [redacted]')}
  async function command(program,args,{input,timeout=120000,allowFailure=false,record=true,env}={}){
    const c=spawn(program,args,{windowsHide:true,stdio:['pipe','pipe','pipe'],env:env?{...process.env,...env}:process.env});let output='',timedOut=false
    for(const stream of [c.stdout,c.stderr])stream.on('data',b=>{output+=b;if(output.length>16*1024*1024)c.kill('SIGTERM')})
    c.stdin.on('error',()=>{});c.stdin.end(input)
    const timer=setTimeout(()=>{timedOut=true;c.kill('SIGTERM')},timeout)
    let exitCode
    try{exitCode=await new Promise((resolve,reject)=>{c.on('error',reject);c.on('close',resolve)})}finally{clearTimeout(timer)}
    if(record){const log='logs/'+String(report.commands.length+1).padStart(4,'0')+'.log';writeFileSync(path.join(out,log),clean(output));report.commands.push({program,args:args.map(clean),exitCode,timedOut,log});save()}
    if(!allowFailure)assert(exitCode===0&&!timedOut,program+' failed: '+clean(output.slice(-2200)))
    return {exitCode,output,timedOut}
  }
  const linux=async(args,settings={})=>{
    let result
    for(let attempt=0;attempt<4;attempt++){
      result=await command(process.platform==='win32'?'wsl.exe':args[0],process.platform==='win32'?['-d',process.env.CI_WSL_DISTRO||'Ubuntu','--exec',...args]:args.slice(1),{...settings,allowFailure:true})
      if(process.platform!=='win32'||result.exitCode!==4294967295||!result.output.replaceAll('\0','').includes('Wsl/Service/0x8007274c'))break
      report.preLaunchRetries??=[];report.preLaunchRetries.push({operation:args[0],attempt:attempt+1,exitCode:result.exitCode,commandNotStarted:true});save();await new Promise(r=>setTimeout(r,1000))
    }
    if(!settings.allowFailure)assert(result.exitCode===0&&!result.timedOut,args[0]+' failed: '+clean(result.output.slice(-2200)))
    return result
  }
  const docker=async(args,settings={})=>(await linux(['docker','--host','unix:///var/run/docker.sock',...args],settings)).output.trim()
  const inspect=async(kind,name)=>JSON.parse(await docker([kind,'inspect',name]))[0]
  const exec=async(args,settings={})=>docker(['exec',tools,...args],{timeout:60*60*1000,...settings})
  async function privateFile(relative,contents,mode='600'){
    assert(/^[a-z0-9/._-]+$/i.test(relative)&&!relative.includes('..'))
    return linux(['bash','-c','set -eu; umask 077; mkdir -p "$(dirname "$1")"; cat > "$1"; chmod "$2" "$1"','phase35-file',privateRoot+'/'+relative,mode],{input:contents})
  }
  async function copyToTools(relative,destination){await docker(['cp',privateRoot+'/'+relative,tools+':'+destination]);await exec(['chown','0:0',destination])}
  async function streamToFile(args,destination){
    const program=process.platform==='win32'?'wsl.exe':'bash',argv=process.platform==='win32'?['-d',process.env.CI_WSL_DISTRO||'Ubuntu','--exec',...args]:['-c','exec "$@"','phase35-stream',...args]
    const c=spawn(program,argv,{windowsHide:true,stdio:['ignore','pipe','pipe']});let error='';c.stderr.on('data',b=>error+=b)
    await Promise.all([pipeline(c.stdout,createWriteStream(destination,{flags:'wx'})),new Promise((resolve,reject)=>{c.on('error',reject);c.on('close',code=>code===0?resolve():reject(Error(clean(error))))})])
  }
  async function step(id,args){console.log('PHASE35_STEP '+id+' START');const start=Date.now();await exec(args);report.steps.push({id,status:'PASS',exitCode:0,seconds:(Date.now()-start)/1000});save();console.log('PHASE35_STEP '+id+' PASS')}
  const pass=(id,proof)=>{const c=report.checks.find(c=>c.id===id);assert(c&&c.status==='NOT_RUN',id);c.status='PASS';c.proof=proof;save();console.log('PHASE35_CHECK '+id+' PASS')}
  let networkCreated=false,privateCreated=false,trustDirectory,attachment,attachOutput='',cacheCreated=false
  const cacheName=options.cacheName||owner+'-cargo'
  report.cache={name:cacheName,retainedForThisPhase:!!options.retainCache,borrowed:!!options.cacheName}
  let serverOrigin,ca,request,policy
  const snapshot=path.join(out,'source')
  try{
    const input=createSourceManifest(sourceRoot,{sourceRevision:options.sourceRevision});report.sourceRevision=input.sourceRevision;report.inputsSha256=input.inputsSha256
    report.hostBefore=createSourceManifest(hostRoot,{sourceRevision:report.sourceRevision}).inputsSha256
    createSnapshot(snapshot,sourceRoot);json(path.join(out,'source-manifest.json'),input)
    report.driverSha256=sha(readFileSync(fileURLToPath(import.meta.url)))
    const { toolsImage, postgresImage, evidence } = await resolveRehearsalImages(docker, options)
    report.inputImages = evidence
    await linux(['bash','-c','set -eu; test ! -e "$1"; mkdir -m 700 "$1"; printf "%s" "$2" > "$1/owner"','phase35-root',privateRoot,owner]);privateCreated=true
    await docker(['network','create','--label','saas.deployment.owner='+owner,'--label','saas.ci.owner='+owner,network]);networkCreated=true
    if(options.cacheName){const v=await inspect('volume',cacheName);assert.equal(v.Labels?.['saas.phase35.cache'],'true')}else{await docker(['volume','create','--label','saas.ci.owner='+owner,'--label','saas.phase35.cache=true',cacheName]);cacheCreated=true}
    containers.add(tools)
    await docker(['create','--init','--name',tools,'--label','saas.ci.owner='+owner,'--network',network,'--cap-drop','ALL','--cap-add','CHOWN','--cap-add','SETUID','--cap-add','SETGID','--security-opt','no-new-privileges','--mount','type=volume,src='+cacheName+',dst=/cache','--env','CARGO_TARGET_DIR=/cache/target','--env','PLAYWRIGHT_BROWSERS_PATH=/opt/ci/browser/.browsers','--workdir','/workspace','--interactive',toolsImage,'node','-e','process.stdin.resume();process.stdin.on("end",()=>process.exit(0))'])
    const attachArgs=['docker','--host','unix:///var/run/docker.sock','start','--attach','--interactive',tools]
    attachment=spawn(process.platform==='win32'?'wsl.exe':attachArgs[0],process.platform==='win32'?['-d',process.env.CI_WSL_DISTRO||'Ubuntu','--exec',...attachArgs]:attachArgs.slice(1),{windowsHide:true,stdio:['pipe','pipe','pipe']})
    for(const stream of [attachment.stdout,attachment.stderr])stream.on('data',b=>attachOutput+=b)
    attachment.stdin.on('error',()=>{})
    await new Promise(r=>setTimeout(r,400));assert.equal((await inspect('container',tools)).State.Running,true)
    await exec(['mkdir','-p','/workspace','/runtime','/runtime/screenshots'])
    await linux(['bash','-c','set -euo pipefail; tar --numeric-owner --owner=0 --group=0 -C "$1" -cf - . | docker --host unix:///var/run/docker.sock cp - "$2:/workspace"','phase35-source',linuxPath(snapshot),tools],{timeout:120000})
    await exec(['sh','-c','mkdir -p /cache/workspace-target; ln -s /cache/workspace-target /workspace/target'])
    await exec(['sh','-c','set -eu; for p in backend/target crates/operational-telemetry/target crates/deployment-contract/target tools/artifact-agent/target deploy/monitoring/target deploy/recovery/target business-apps/work-orders/backend/target tools/platform-admin/target mini-services/oauth-playground/backend/target target/ci-rust target/phase22-socketio target/phase22-chat target/phase23-notification target/phase25-platform-admin tests/fixtures/gateway-rust/target; do if test ! -e "/workspace/$p"; then ln -s /cache/target "/workspace/$p"; fi; done'])
    for(const [name,manifest]of [['contract','crates/deployment-contract/Cargo.toml'],['agent','tools/artifact-agent/Cargo.toml']]){
      await step(name+'-fmt',['cargo','fmt','--manifest-path',manifest,'--','--check'])
      await step(name+'-doc',['cargo','doc','--locked','--manifest-path',manifest,'--no-deps'])
      await step(name+'-tests',['cargo','test','--locked','--manifest-path',manifest,'-j','2'])
      await step(name+'-clippy',['cargo','clippy','--locked','--manifest-path',manifest,'--all-targets','-j','2','--','-D','warnings'])
    }
    await step('platform-build',['cargo','build','--locked','--manifest-path','backend/Cargo.toml','-j','2'])
    await step('agent-build',['cargo','build','--locked','--manifest-path','tools/artifact-agent/Cargo.toml','-j','2'])
    await step('frontend-install',['npm','--prefix','frontend','ci','--ignore-scripts','--no-audit','--no-fund'])
    await step('frontend-build',['npm','--prefix','frontend','run','build'])
    await step('browser-install',['npm','--prefix','tests/browser','ci','--ignore-scripts','--no-audit','--no-fund'])
    if(!options.development){
      let fastPassed=false
      try{await step('current-source-fast',['node','scripts/ci/run.mjs','--gate','fast','--out-dir','/runtime/fast','--source-revision',report.sourceRevision]);fastPassed=true}
      finally{
        try{
          await exec(['sh','-c','set -eu; mkdir -p /runtime/fast-public; for f in summary.json source-manifest.json; do if test -f /runtime/fast/"$f"; then cp /runtime/fast/"$f" /runtime/fast-public/; fi; done; if test -d /runtime/fast/logs; then cp -R /runtime/fast/logs /runtime/fast-public/; fi'])
          await streamToFile(['docker','--host','unix:///var/run/docker.sock','cp',tools+':/runtime/fast-public','-'],path.join(out,'fast.tar'))
        }catch(error){if(fastPassed)throw error}
      }
    }
    for(const [name,binary]of [['platform','saas-platform-backend'],['agent','saas-artifact-agent']]){
      const tar=path.join(out,'original-binaries',name+'.tar');await streamToFile(['docker','--host','unix:///var/run/docker.sock','cp',tools+':/cache/target/debug/'+binary,'-'],tar)
      const extraction=spawnSync('tar',['-xf',tar,'-C',path.join(out,'original-binaries')],{encoding:'utf8',windowsHide:true});assert.equal(extraction.status,0,extraction.stderr)
      const file=path.join(out,'original-binaries',binary),bytes=readFileSync(file);assert(bytes.subarray(0,4).equals(Buffer.from([0x7f,69,76,70])));assert.equal(bytes.readUInt16LE(18),62)
      report.originalBinaries.push({name,path:'original-binaries/'+binary,bytes:bytes.length,sha256:sha(bytes),elfMachine:62})
    }
    await linux(['bash','-c','set -eu; mkdir -p "$1/bin" "$1/routes" "$1/state" "$1/tls" "$1/site"; chmod 755 "$1/routes" "$1/tls" "$1/site"; docker --host unix:///var/run/docker.sock cp "$2:/cache/target/debug/saas-artifact-agent" "$1/bin/agent"; chmod 700 "$1/bin/agent"; docker --host unix:///var/run/docker.sock cp "$2:/workspace/frontend/dist/." "$1/site/"','phase35-materials',privateRoot,tools])
    if (!options.buildOnly) {
    const setup=await import(pathToFileURL(path.join(snapshot,'tests/phase35-zip-distribution.mjs')).href)
    const context={owner,out,privateRoot,network,tools,pg,gateway,registry,report,secretValues,containers,images,docker,inspect,exec,linux,privateFile,copyToTools,step,pass,streamToFile,cacheName,toolsImage,postgresImage,save,clean}
    const result=await setup.rehearse(context)
    ;({trustDirectory,serverOrigin,ca,request,policy}=result)
    report.registry=result.registry;report.browser=result.browser
    } else { assert(options.development); report.compileOnly=true }
    report.sourceAfter=createSourceManifest(snapshot,{sourceRevision:report.sourceRevision}).inputsSha256;report.sourceUnchanged=report.sourceAfter===report.inputsSha256
    report.hostAfter=createSourceManifest(hostRoot,{sourceRevision:report.sourceRevision}).inputsSha256;report.hostSourceUnchanged=report.hostAfter===report.hostBefore
    assert(report.sourceUnchanged);if(!options.development)assert(report.hostSourceUnchanged)
    if(!options.buildOnly)assert(report.checks.every(c=>c.status==='PASS'));report.status='PASS'
  }catch(error){report.error=clean(error.stack||error);console.error(report.error)}finally{
    if(containers.has(tools))try{const log=await docker(['exec','--user','10001:10001',tools,'cat','/runtime/platform.log'],{allowFailure:true});writeFileSync(path.join(out,'logs/platform.log'),clean(log))}catch{}
    // A failed fixture may have created trust before returning; its owner file is the authority.
    if(privateCreated){try{const info=await linux(['bash','-c','test ! -f "$1/trust-path" || cat "$1/trust-path"','phase35-trust-query',privateRoot]);trustDirectory||=info.output.trim()}catch(error){report.cleanup.errors.push(clean(error.message))}}
    for(const name of containers){try{const state=await inspect('container',name);const label=state.Config.Labels?.['saas.ci.owner']||state.Config.Labels?.['saas.deployment.owner'];assert.equal(label,owner);await docker(['rm','--force',name])}catch(error){report.cleanup.errors.push(clean(error.message))}}
    // Agent-created version containers were not known in advance; filter only this exact owner.
    try{for(const name of (await docker(['ps','--all','--filter','label=saas.deployment.owner='+owner,'--format','{{.Names}}'])).split(/\r?\n/).filter(Boolean)){assert.equal((await inspect('container',name)).Config.Labels['saas.deployment.owner'],owner);await docker(['rm','--force',name])}report.cleanup.containersRemoved=(await docker(['ps','--all','--filter','label=saas.deployment.owner='+owner,'--format','{{.Names}}']))===''&&(await docker(['ps','--all','--filter','label=saas.ci.owner='+owner,'--format','{{.Names}}']))===''}catch(error){report.cleanup.errors.push(clean(error.message))}
    if(attachment){attachment.stdin.end();await new Promise(resolve=>{if(attachment.exitCode!==null)resolve();else{attachment.once('close',resolve);setTimeout(()=>{attachment.kill('SIGTERM');resolve()},5000)}});writeFileSync(path.join(out,'logs/tool-lease.log'),clean(attachOutput))}
    if(networkCreated){try{await docker(['network','rm',network]);report.cleanup.networksRemoved=(await docker(['network','ls','--filter','label=saas.deployment.owner='+owner,'--format','{{.Name}}']))===''}catch(error){report.cleanup.errors.push(clean(error.message))}}
    for(const image of images){try{const s=await inspect('image',image);assert.equal(s.Config.Labels['saas.ci.owner'],owner);assert.equal(await docker(['ps','--all','--filter','ancestor='+image,'--format','{{.Names}}']),'');await docker(['image','rm','--force',image])}catch(error){report.cleanup.errors.push(clean(error.message))}}
    report.cleanup.candidateImagesRemoved=(await docker(['image','ls','--filter','label=saas.ci.owner='+owner,'--format','{{.ID}}']))===''
    if((cacheCreated&&!options.retainCache)||(!options.development&&options.cacheName)){
      try{const cache=await inspect('volume',cacheName);assert.equal(cache.Labels?.['saas.phase35.cache'],'true');await docker(['volume','rm',cacheName]);report.cleanup.cacheRemoved=true}catch(error){report.cleanup.errors.push(clean(error.message))}
    }
    if(trustDirectory){try{assert(/^\/etc\/docker\/certs\.d\/127\.0\.0\.1:\d+$/.test(trustDirectory));const script='set -eu; test ! -L "$1"; test "$(realpath "$1")" = "$1"; test "$(cat "$1/phase35-owner")" = "$2"; rm -rf -- "$1"';if(process.platform==='win32')await command('wsl.exe',['-d',process.env.CI_WSL_DISTRO||'Ubuntu','--user','root','--exec','bash','-c',script,'phase35-trust-cleanup',trustDirectory,owner]);else await linux(['bash','-c',script,'phase35-trust-cleanup',trustDirectory,owner]);report.cleanup.registryTrustRemoved=true}catch(error){report.cleanup.errors.push(clean(error.message))}}else report.cleanup.registryTrustRemoved=true
    if(privateCreated){try{await linux(['bash','-c','set -eu; case "$1" in /var/tmp/ci-phase35-*) ;; *) exit 90;; esac; test ! -L "$1"; test "$(realpath "$1")" = "$1"; test "$(cat "$1/owner")" = "$2"; test "$(stat -c %u "$1")" = "$(id -u)"; rm -rf -- "$1"','phase35-owned-cleanup',privateRoot,owner]);report.cleanup.privateDirectoryRemoved=true}catch(error){report.cleanup.errors.push(clean(error.message))}}
    if(report.cleanup.errors.length)report.status='FAIL';report.finishedAt=new Date().toISOString();save()
  }
  if(report.status==='PASS'&&!options.development)validateZipDistributionReport(report)
  console.log('PHASE35_ZIP '+report.status+' '+path.join(out,'result.json'));return report
}

if(process.argv[1]&&import.meta.url===pathToFileURL(path.resolve(process.argv[1])).href){
  const args=process.argv.slice(2);assert(args.length===2&&args[0]==='--out-dir','Usage: node scripts/ci/zip-distribution.mjs --out-dir /fresh/absolute/path')
  const r=await runZipDistribution({outDir:args[1]});process.exitCode=r.status==='PASS'?0:1
}
