import assert from 'node:assert/strict'

export const workOrdersReminderLiveChecks = Object.freeze([
  'real-login-app-token-official-grants',
  'persisted-assignee-reminder-no-ticket-write',
  'same-event-idempotent-receipt',
  'owner-fallback-and-self-scope',
  'missing-ticket-read-permission-denied',
  'missing-ticket-read-grant-denied',
  'missing-notification-publish-permission-denied',
  'missing-notification-publish-grant-denied',
  'recipient-enabled-direct-membership-and-scope',
  'precommit-unknown-intent-recipient-conflict',
  'committed-response-loss-same-key-recovery',
  'realtime-absent-persistence-still-success',
])

export const workOrdersReminderFixtureProofs = Object.freeze({
  unknownResult: 'WORK_ORDERS_REMINDER_UNKNOWN_RESULT_PROOF persisted=true same_key=true unchanged_payload=true publication_count=1 no_automatic_retry=true',
  databaseRelease: 'WORK_ORDERS_REMINDER_DB_RELEASE_PROOF pool_available=true row_unlocked=true publication_pending=true',
  recipientChange: 'WORK_ORDERS_REMINDER_RECIPIENT_CHANGE_PROOF same_key_conflict=true explicit_new_event=true uncommitted_intent_preserved=true application_restarted=true',
  preparation: 'WORK_ORDERS_REMINDER_PREPARATION_PROOF commit_rejected=true publication_posts=0 ticket_unchanged=true',
})
const normalizedLines = output => output.split(/\r?\n/).map(line => line.replace(/^test \S+ \.\.\. (?=(?:WORK_ORDERS_REMINDER_|REMINDER_LIVE_))/, ''))
const ownedCleanup = Object.freeze({ assertionsOk: true, databaseRemoved: true, poolClosed: true })

export function validateWorkOrdersReminderFixtureResult(document, expected) {
  assert.equal(document?.cleanup?.length, expected)
  for (const cleanup of document.cleanup) assert.deepEqual(cleanup, ownedCleanup)
  assert.deepEqual(document.proofs, Object.fromEntries(Object.keys(workOrdersReminderFixtureProofs).map(key => [key, true])))
  return document
}

export function parseWorkOrdersReminderFixtureOutput(output, expected) {
  const lines = normalizedLines(output), normalized = lines.join('\n')
  assert.equal(lines.filter(line => line.startsWith('WORK_ORDERS_REMINDER_FIXTURE_CLEANUP')).length, expected)
  const cleanup = [...normalized.matchAll(/^WORK_ORDERS_REMINDER_FIXTURE_CLEANUP assertions_ok=(true|false) database_removed=(true|false) pool_closed=(true|false)\s*$/gm)]
    .map(entry => ({ assertionsOk: entry[1] === 'true', databaseRemoved: entry[2] === 'true', poolClosed: entry[3] === 'true' }))
  const proofs = Object.fromEntries(Object.entries(workOrdersReminderFixtureProofs).map(([key, marker]) => {
    const entries = lines.filter(line => line.startsWith(marker.split(' ')[0]))
    return [key, entries.length === 1 && entries[0].trimEnd() === marker]
  }))
  return validateWorkOrdersReminderFixtureResult({ cleanup, proofs }, expected)
}

export function validateWorkOrdersReminderLiveResult(document) {
  assert.equal(document?.result?.schemaVersion, 1)
  assert.equal(document.result.transport, 'explicit-development-loopback-http')
  assert.equal(document.result.passed, workOrdersReminderLiveChecks.length)
  assert.equal(document.result.failed, 0)
  assert.equal(document.result.skipped, 0)
  assert.equal(document.result.checks?.length, workOrdersReminderLiveChecks.length)
  for (const name of workOrdersReminderLiveChecks) assert.equal(document.result.checks.filter(check => check.name === name && check.status === 'PASS').length, 1)
  assert.deepEqual(document.platformCleanup, ownedCleanup)
  assert.deepEqual(document.businessCleanup, ownedCleanup)
  assert.deepEqual(document.processCleanup, { stopped: true })
  assert.deepEqual(document.processRestart, { stopped: true })
  return document
}

export function parseWorkOrdersReminderLiveOutput(output) {
  const lines = normalizedLines(output), normalized = lines.join('\n')
  assert.equal(lines.filter(line => line.startsWith('WORK_ORDERS_REMINDER_LIVE_RESULT')).length, 1)
  const result = [...normalized.matchAll(/^WORK_ORDERS_REMINDER_LIVE_RESULT (\{[^\r\n]*\})\s*$/gm)]
  assert.equal(result.length, 1, 'Exactly one actual platform/work-orders reminder result is required')
  const cleanup = name => {
    assert.equal(lines.filter(line => line.startsWith(`REMINDER_LIVE_${name}_CLEANUP`)).length, 1)
    const entries = [...normalized.matchAll(new RegExp(`^REMINDER_LIVE_${name}_CLEANUP assertions_ok=(true|false) database_removed=(true|false) pool_closed=(true|false)\\s*$`, 'gm'))]
    assert.equal(entries.length, 1, `Exactly one owned ${name} cleanup is required`)
    return { assertionsOk: entries[0][1] === 'true', databaseRemoved: entries[0][2] === 'true', poolClosed: entries[0][3] === 'true' }
  }
  const process = marker => {
    assert.equal(lines.filter(line => line.startsWith(`REMINDER_LIVE_PROCESS_${marker}`)).length, 1)
    const entries = [...normalized.matchAll(new RegExp(`^REMINDER_LIVE_PROCESS_${marker} stopped=(true|false)\\s*$`, 'gm'))]
    assert.equal(entries.length, 1, `Exactly one independent Rust process ${marker} is required`)
    return { stopped: entries[0][1] === 'true' }
  }
  return validateWorkOrdersReminderLiveResult({ result: JSON.parse(result[0][1]), platformCleanup: cleanup('PLATFORM'), businessCleanup: cleanup('BUSINESS'),
    processCleanup: process('CLEANUP'), processRestart: process('RESTART') })
}
