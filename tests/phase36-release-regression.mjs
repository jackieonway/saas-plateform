// Existing release/ZIP/upgrade/retention contracts against the rehearsal's disposable PG.
import {spawn} from 'node:child_process'
import {readFileSync} from 'node:fs'
import {runPostgres} from '../scripts/ci/postgres.mjs'
const runtime=JSON.parse(readFileSync('/runtime/regression.json'))
const env={...process.env,TEST_DATABASE_URL:runtime.databaseUrl,CI_TEST_POSTGRES_OWNER:runtime.owner}
const result=await runPostgres({outDir:'/workspace/target/phase36-release-regression',env,suites:[{name:'release_tests::',expected:17}],
 runCargo:(args,root,childEnv)=>new Promise((resolve,reject)=>{
  const child=spawn('cargo',args,{cwd:root,env:{...childEnv,CARGO_TARGET_DIR:'/cache/target'},stdio:['ignore','pipe','pipe']})
  let output='',timedOut=false;const timer=setTimeout(()=>{timedOut=true;child.kill('SIGTERM')},900000)
  for(const stream of [child.stdout,child.stderr])stream.on('data',b=>output+=b)
  child.on('error',e=>{clearTimeout(timer);reject(e)});child.on('close',code=>{clearTimeout(timer);resolve({code,output,timedOut})})
 })})
console.log('PHASE36_RELEASE_REGRESSION '+JSON.stringify({status:result.status,passed:result.passed,failed:result.failed,skipped:result.skipped}))
