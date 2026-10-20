import assert from 'node:assert/strict'
import test from 'node:test'
import { createRequire } from 'node:module'
import { readFileSync } from 'node:fs'
import { validateCapacity } from './monitoring-results.mjs'
import { evaluate } from './report.mjs'
const yaml=createRequire(import.meta.url)('yaml')
test('monitoring templates parse with fixed private targets, file credentials and no external receiver',()=>{
  const collector=yaml.parse(readFileSync(new URL('../../deploy/monitoring/prometheus.example.yml',import.meta.url),'utf8'))
  assert.deepEqual(collector.scrape_configs.map(job=>job.job_name),['platform','work-orders','stores','notification','monitor'])
  for(const job of collector.scrape_configs){assert.equal(job.authorization.type,'Bearer');assert.match(job.authorization.credentials_file,/^\/run\/secrets\//);assert(!job.authorization.credentials)}
  const alerts=yaml.parse(readFileSync(new URL('../../deploy/monitoring/saas-alerts.yml',import.meta.url),'utf8'))
  assert(alerts.groups[0].rules.length>=6);assert(alerts.groups[0].rules.every(rule=>rule.for&&rule.expr))
})
function candidate() {
  return { kind:'phase32-capacity', records:{workOrders:50000,dailyReports:50000}, distinctPrincipals:1,
    rounds:[100,150,200].map(activeSessions => ({ activeSessions, onlineAppSessions:200, maxConcurrentRequests:activeSessions, elapsedSeconds:2, requestsPerSecond:250,
      resourcesBefore:[{},{},{}],resourcesAfter:[{},{},{}],cohorts:['stores','work-orders'].map(app => ({app,samples:activeSessions*5/2,statusCounts:{200:activeSessions*5/2},successfulSamples:activeSessions*5/2,p50Ms:100,p95Ms:600,p99Ms:900,unexpectedErrorRatio:0,candidatePass:false})) })),
    faults:{events:['DOWN','RECOVERED','DOWN','RECOVERED'].map(event=>({component:'stores',event})),externalDelivery:false,backupImplementation:false},revocationNextRequest:403,
    metrics:Array(3).fill('# TYPE saas_stage_duration_seconds histogram\nsaas_stage_inflight{service="stores",stage="request"} 0\n') }
}
test('measured candidate misses do not become fictional capacity/SLA or a failed observation gate',()=>{
  const capacity=candidate();validateCapacity(capacity)
  const document={schemaVersion:1,kind:'phase32-capacity-fault-postgres',status:'PASS',passed:1,failed:0,skipped:0,suites:[{name:'phase26_stores_tests::phase32::phase32_capacity_and_locks',status:'PASS',expected:1,passed:1,failed:0,skipped:0}],capacity}
  assert.equal(evaluate({format:'monitoring-postgres',exitCode:0,document}).status,'PASS')
  assert.equal(evaluate({format:'monitoring-postgres',exitCode:1,document}).status,'FAIL')
  for(const broken of [{...document,capacity:undefined},{...document,skipped:1},{...document,suites:[]},{...document,status:'FAIL'}])assert.equal(evaluate({format:'monitoring-postgres',exitCode:0,document:broken}).status,'FAIL')
})
test('capacity evidence rejects missing load/data/faults and secret-bearing labels',()=>{
  for(const mutate of [v=>{v.rounds.pop()},v=>{v.records.workOrders=49999},v=>{v.faults.events.pop()},v=>{v.revocationNextRequest=200},v=>{v.metrics[0]+='m{user="identity"} 1'},v=>{v.metrics[0]+='m{route="https://secret.example/x"} 1'},v=>{v.rounds[0].cohorts[0].candidatePass=true}]){
    const value=candidate();mutate(value);assert.throws(()=>validateCapacity(value))
  }
})
