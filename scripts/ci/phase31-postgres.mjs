import assert from 'node:assert/strict'
import { readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { runPostgres } from './postgres.mjs'
import { storesPostgresEnvironment } from './stores-postgres.mjs'
const args=process.argv.slice(2)
assert(args.length===3 && args[0]==='--run-tests' && args[1]==='--out-dir' && path.isAbsolute(args[2]||''),'Expected --run-tests --out-dir <fresh absolute directory>')
const suites=[
  {name:'release_tests::phase31::phase31_platform_upgrade_preserves_every_old_row_and_checksum',expected:1},
  {name:'release_tests::phase31::phase31_preview_preserves_database_zip_and_online_urls',expected:1},
  {name:'phase26_stores_tests::phase31::phase31_old_business_clients',expected:1},
]
const report=await runPostgres({outDir:args[2],exact:true,kind:'phase31-upgrade-retention-postgres',suites,env:storesPostgresEnvironment(process.env,process.cwd(),{live:true})})
try {
  const logs=report.suites.map(suite=>readFileSync(path.join(args[2],suite.log),'utf8'))
  for(const [index,marker] of [[0,'PHASE31_PLATFORM_UPGRADE PASS'],[1,'PHASE31_RETENTION PASS'],[1,'PHASE31_OLD_HOST PASS'],[2,'PHASE31_OLD_BUSINESS PASS']]) assert(logs[index].includes(marker),'Missing actual protocol result '+marker)
  for(const kind of ['PLATFORM','STORES','WORK_ORDERS']) {
    const match=logs[2].match(new RegExp('STORES_LIVE_'+kind+'_CLEANUP (\\{[^\\n]+\\})'))
    assert(match,'Missing owned database cleanup');const cleanup=JSON.parse(match[1])
    for(const key of ['assertionsOk','databaseRemoved','roleRemoved','poolClosed'])assert.equal(cleanup[key],true)
  }
} catch(error){report.status='FAIL';report.error=error.message}
writeFileSync(path.join(args[2],'postgres-result.json'),JSON.stringify(report,null,2)+'\n')
assert.equal(report.status,'PASS',report.error)
