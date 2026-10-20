import {test} from 'node:test'
import assert from 'node:assert/strict'
import {readFileSync} from 'node:fs'
import {spawnSync} from 'node:child_process'
import {createRequire} from 'node:module'
import path from 'node:path'
const root=process.cwd(),yaml=createRequire(path.join(root,'scripts/ci/package.json'))('yaml')
test('work-orders npm start dispatches its locked Rust service',()=>{
 const p=JSON.parse(readFileSync(path.join(root,'business-apps/work-orders/backend/package.json')))
 assert.match(p.scripts.start,/^cargo run --locked --manifest-path Cargo\.toml$/)
})
test('direct retired Node entry refuses before configuration or network access',()=>{
 const r=spawnSync(process.execPath,[path.join(root,'business-apps/work-orders/backend/index.mjs')],{encoding:'utf8',timeout:3000,windowsHide:true,env:{WORK_ORDERS_DATABASE_URL:'invalid-no-network',PATH:path.dirname(process.execPath)}})
 assert.equal(r.status,1);assert.match(r.stderr,/NODE_RUNTIME_RETIRED.*Rust/);assert(!r.stdout.includes('ready'))
})
test('legacy Compose filename and env template expose no retired service',()=>{
 const p=yaml.parse(readFileSync(path.join(root,'deploy/compose.legacy.yml'),'utf8'));assert.deepEqual(p.services,{})
 assert(!/^LEGACY_WORK_ORDERS_IMAGE\s*=/m.test(readFileSync(path.join(root,'deploy/compose.legacy.env.example'),'utf8')))
})
