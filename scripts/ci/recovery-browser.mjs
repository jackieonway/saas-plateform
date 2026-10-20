import {parseStoresBrowserArgs,runStoresBrowser} from './stores-browser.mjs'
import {runCommand} from './report.mjs'
import {spawn} from 'node:child_process'
// Only finite ownership handshakes are streamed; credentials, arbitrary service
// output and full command logs stay in the existing redacted evidence pipeline.
function execute(program,args,options){
  if(args[0]!=='tests/phase33-recovery-browser.mjs')return runCommand(program,args,options)
  return new Promise(resolve=>{
    const child=spawn(program,args,{cwd:options.cwd,env:{...options.env,LD_LIBRARY_PATH:'/opt/17/lib'},stdio:['ignore','pipe','pipe']});let output='',pending='',timedOut=false
    child.stdout.on('data',b=>{output+=b;pending+=b;while(pending.includes('\n')){const i=pending.indexOf('\n'),line=pending.slice(0,i).trim();pending=pending.slice(i+1);if(/^PHASE33_(?:BUILD_IMAGES_READY|RESTORED_IMAGES_READY|CHECK [a-z0-9-]+ PASS)$/.test(line))console.log(line)}})
    child.stderr.on('data',b=>output+=b)
    const timer=setTimeout(()=>{timedOut=true;child.kill('SIGTERM')},options.timeout)
    child.once('error',()=>{clearTimeout(timer);resolve({exitCode:-1,timedOut,output})})
    child.once('close',exitCode=>{clearTimeout(timer);resolve({exitCode,timedOut,output})})
  })
}
const result=await runStoresBrowser({...parseStoresBrowserArgs(process.argv.slice(2)),recovery:true,execute})
console.log('PHASE33_BROWSER '+result.status)
if(result.status!=='PASS')process.exitCode=1
