// Owned rehearsal process launcher; never used as a production backend.
import {spawn} from 'node:child_process'
import {readFileSync,openSync,writeFileSync} from 'node:fs'
const env=JSON.parse(readFileSync('/runtime/platform.env.json'))
const log=openSync('/runtime/platform.log','a',0o600)
const child=spawn('/cache/target/debug/saas-platform-backend',[],{env:{PATH:process.env.PATH,LANG:'C.UTF-8',...env},cwd:'/runtime',stdio:['ignore',log,log]})
writeFileSync('/runtime/platform.pid',String(child.pid))
child.on('error',()=>process.exit(1));child.on('close',code=>process.exit(code??1))
