import assert from 'node:assert/strict'
export function validateCapacity(value) {
  assert.equal(value.kind, 'phase32-capacity')
  assert.equal(value.records.workOrders + value.records.dailyReports, 100000)
  assert.equal(value.distinctPrincipals, 1)
  assert.deepEqual(value.rounds.map(round => round.activeSessions), [100,150,200])
  for (const round of value.rounds) {
    assert.equal(round.onlineAppSessions, 200)
    assert(round.maxConcurrentRequests > 0 && round.maxConcurrentRequests <= round.activeSessions)
    assert(round.elapsedSeconds > 0 && round.requestsPerSecond > 0)
    assert.equal(round.resourcesBefore.length, 3); assert.equal(round.resourcesAfter.length, 3)
    assert.deepEqual(round.cohorts.map(item => item.app), ['stores','work-orders'])
    for (const cohort of round.cohorts) {
      assert.equal(cohort.samples, round.activeSessions * 5 / 2)
      assert.equal(Object.values(cohort.statusCounts).reduce((sum,n) => sum+n,0), cohort.samples)
      assert([cohort.p50Ms,cohort.p95Ms,cohort.p99Ms].every(Number.isFinite))
      assert(cohort.p50Ms > 0 && cohort.p95Ms >= cohort.p50Ms && cohort.p99Ms >= cohort.p95Ms)
      assert.equal(cohort.candidatePass, cohort.successfulSamples === cohort.samples && cohort.p95Ms <= 500 && cohort.p99Ms <= 1000 && cohort.unexpectedErrorRatio < 0.001)
    }
  }
  assert.equal(value.faults.events.filter(e => e.component === 'stores' && e.event === 'DOWN').length, 2)
  assert.equal(value.faults.events.filter(e => e.component === 'stores' && e.event === 'RECOVERED').length, 2)
  assert.equal(value.faults.externalDelivery, false); assert.equal(value.faults.backupImplementation, false)
  assert.equal(value.revocationNextRequest, 403)
  assert(value.metrics.length === 3 && value.metrics.every(text => text.includes('# TYPE saas_stage_duration_seconds histogram')))
  for (const text of value.metrics) {
    assert(!/https?:\/\/|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i.test(text), 'No URL or identity may enter metrics')
    for (const match of text.matchAll(/(?:\{|,)([a-z_]+)="/g)) assert(['service','stage','outcome','le','route','method','status','reason','application','kind'].includes(match[1]), 'Unexpected metric label')
  }
  return value
}
export function validateMonitoringBrowser(value, product) {
  assert.equal(value.status, 'PASS'); assert.equal(value.product, product)
  assert.equal(value.dataset.workOrders + value.dataset.dailyReports,100000)
  assert(Object.values(value.cleanup).length >= 8 && Object.values(value.cleanup).every(v => v === true))
  const navigation = value.measurements.filter(m => m.cache)
  assert.equal(navigation.length, 4)
  for (const app of ['stores','work-orders']) for (const cache of ['cold-context','warm-context']) {
    const items = navigation.filter(m => m.app === app && m.cache === cache); assert.equal(items.length, 1)
    const m = items[0]; assert(Number.isFinite(m.lcpMs) && m.lcpMs > 0)
    assert(Number.isFinite(m.observedInteractionUpperBoundMs) && m.observedInteractionUpperBoundMs >= 16)
    for (const stage of ['entry','ready','mount']) assert(m.lifecycle.some(e => e.stage === stage && Number.isFinite(e.durationMs)))
  }
  const revoke = value.measurements.find(m => m.kind === 'revocation')
  assert.equal(revoke.nextRequest, 403); assert.equal(revoke.candidatePass, revoke.awarenessMs !== null && revoke.awarenessMs <= 20000)
  assert.equal(value.privateMetrics.length, 3)
  for (const m of value.privateMetrics) { assert.equal(m.privateStatus, 200); assert.equal(m.publicStatus, 404) }
  assert.equal(value.publicMetricsDenials.length, 7)
  return value
}
