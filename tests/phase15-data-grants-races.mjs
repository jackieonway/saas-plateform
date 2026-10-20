/** UI race checks only: hold genuine server responses and ignore selected aborts.
 * Positive grants, server 403s and PostgreSQL assertions never use these hooks.
 */
import assert from 'node:assert/strict'

export const raceCheckIds = ['late-snapshot-target-switch', 'late-preview-edit-fence',
  'late-response-session-fence', 'unknown-save-reconciliation']

export function installRaceTransport() {
  const nativeFetch = window.fetch.bind(window)
  const state = window.__phase15Race = { armed: null, events: [] }
  window.fetch = function (input, options = {}) {
    const url = new URL(typeof input === 'string' ? input : input.url, location.href)
    const method = options.method || 'GET'
    const armed = state.armed
    let member = url.searchParams.get('membershipId')
    if (method === 'POST' && typeof options.body === 'string') {
      try { member = JSON.parse(options.body).target?.membershipId } catch { /* irrelevant request */ }
    }
    if (armed && url.pathname === armed.path && method === armed.method && member === armed.member) {
      state.armed = null
      const signal = options.signal
      const event = { signalPresent: Boolean(signal), aborted: Boolean(signal?.aborted), completed: false }
      state.events.push(event)
      signal?.addEventListener('abort', () => { event.aborted = true }, { once: true })
      return nativeFetch(input, { ...options, signal: undefined }).then(response => {
        event.completed = true
        event.status = response.status
        return response
      })
    }
    return nativeFetch(input, options)
  }
}

const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done }); return { promise, resolve } }

export async function holdRealResponse(page, { method, pathname, member }) {
  const ready = deferred(), release = deferred(), settled = deferred()
  let held = false, failure
  const matches = url => url.pathname === pathname
  const handler = async route => {
    const request = route.request(), url = new URL(request.url())
    const requestedMember = method === 'GET' ? url.searchParams.get('membershipId') : request.postDataJSON()?.target?.membershipId
    if (held || request.method() !== method || requestedMember !== member) return route.continue()
    held = true
    try {
      const response = await route.fetch()
      ready.resolve(response.status())
      await release.promise
      await route.fulfill({ response })
    } catch (error) {
      failure = error
      ready.resolve(-1)
    } finally { settled.resolve() }
  }
  await page.route(matches, handler)
  let timer
  const readyWithinDeadline = Promise.race([ready.promise, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error('Expected real ' + method + ' response did not reach the UI race barrier')), 20_000)
  })]).finally(() => clearTimeout(timer))
  return {
    ready: readyWithinDeadline,
    async release() {
      release.resolve()
      await settled.promise
      await page.unroute(matches, handler)
      if (failure) throw new Error('Held real HTTP response could not complete')
    },
  }
}

async function arm(page, method, path, member) {
  await page.evaluate(armed => { window.__phase15Race.armed = armed }, { method, path, member })
}

async function settledTransport(page, wait) {
  await wait(() => page.evaluate(() => window.__phase15Race.events.some(event => event.completed && event.aborted && event.signalPresent)),
    'The selected real response completes after its production signal was cancelled')
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))
}

export async function runGrantRaces({ fixture, open, chooseApplication, chooseScope, preview,
  snapshot, wait, check }) {
  // Old target A's actual HTTP response is delivered after target B is visible.
  const switched = await open(fixture.sessions.admin, '', { raceTransport: true })
  await chooseApplication(switched.page)
  await arm(switched.page, 'GET', '/api/data-grants', fixture.target.membershipId)
  const oldSnapshot = await holdRealResponse(switched.page, { method: 'GET', pathname: '/api/data-grants', member: fixture.target.membershipId })
  await switched.page.getByTestId('grant-member-' + fixture.target.membershipId).click()
  assert.equal(await oldSnapshot.ready, 200)
  await switched.page.getByTestId('grant-member-' + fixture.alternateTarget.membershipId).click()
  const newer = switched.page.getByTestId('grant-current-snapshot')
  await wait(async () => await newer.getAttribute('data-membership-id') === fixture.alternateTarget.membershipId,
    'The explicitly selected newer direct member renders first')
  await oldSnapshot.release()
  await settledTransport(switched.page, wait)
  assert.equal(await newer.getAttribute('data-membership-id'), fixture.alternateTarget.membershipId,
    'Late old target cannot replace the newer target')
  assert.deepEqual(switched.errors, [])
  check('late-snapshot-target-switch')
  await switched.context.close()

  // A changed draft invalidates the earlier reviewed preview even when its HTTP
  // response ignores native cancellation and arrives successfully.
  const edited = await open(fixture.sessions.admin, '?userId=' + fixture.target.userId, { raceTransport: true })
  await chooseApplication(edited.page)
  await edited.page.getByTestId('grant-member-' + fixture.target.membershipId).click()
  await edited.page.getByTestId('grant-current-snapshot').waitFor()
  await chooseScope(edited.page, 'SELF')
  await arm(edited.page, 'POST', '/api/data-grants/preview', fixture.target.membershipId)
  const oldPreview = await holdRealResponse(edited.page, { method: 'POST', pathname: '/api/data-grants/preview', member: fixture.target.membershipId })
  await edited.page.getByRole('button', { name: '预览变更', exact: true }).click()
  assert.equal(await oldPreview.ready, 200)
  await chooseScope(edited.page, 'ORG')
  await oldPreview.release()
  await settledTransport(edited.page, wait)
  assert.equal(await edited.page.getByRole('button', { name: '确认保存', exact: true }).count(), 0,
    'Late previous-draft preview cannot enable commit')
  assert.match(await edited.page.getByRole('combobox', { name: 'ticket/read 数据范围', exact: true }).innerText(), /ORG/)
  const currentPreview = await preview(edited.page)
  assert.equal(currentPreview.after[0].scope, 'ORG', 'A new preview uses the current draft')
  assert.deepEqual(edited.errors, [])
  check('late-preview-edit-fence')
  await edited.context.close()

  // Replace the shared platform session with a real valid account lacking the
  // independent read permission while the old authorized request is in flight.
  const replaced = await open(fixture.sessions.admin, '', { raceTransport: true })
  await chooseApplication(replaced.page)
  await arm(replaced.page, 'GET', '/api/data-grants', fixture.target.membershipId)
  const oldSession = await holdRealResponse(replaced.page, { method: 'GET', pathname: '/api/data-grants', member: fixture.target.membershipId })
  await replaced.page.getByTestId('grant-member-' + fixture.target.membershipId).click()
  assert.equal(await oldSession.ready, 200)
  await replaced.page.evaluate(async token => {
    const { authStore } = await import('/src/stores/auth.ts')
    await authStore.getState().setSession(token)
  }, fixture.sessions.noRead)
  await replaced.page.getByText('无访问权限', { exact: true }).waitFor()
  await oldSession.release()
  await settledTransport(replaced.page, wait)
  assert.equal(await replaced.page.getByTestId('grant-current-snapshot').count(), 0)
  assert.equal(await replaced.page.getByRole('button', { name: '确认保存', exact: true }).count(), 0)
  assert.deepEqual(replaced.errors, [])
  check('late-response-session-fence')
  await replaced.context.close()

  // Abort before forwarding: this produces an unknown browser outcome while
  // keeping the owned database untouched, then proves explicit reconciliation.
  const unknown = await open(fixture.sessions.admin, '?userId=' + fixture.target.userId)
  await chooseApplication(unknown.page)
  await unknown.page.getByTestId('grant-member-' + fixture.target.membershipId).click()
  await unknown.page.getByTestId('grant-current-snapshot').waitFor()
  const before = await snapshot()
  await chooseScope(unknown.page, 'SELF')
  await preview(unknown.page)
  let attempts = 0
  const abortPatch = async route => {
    if (route.request().method() !== 'PATCH') return route.continue()
    attempts++
    await route.abort('failed')
  }
  await unknown.page.route(url => url.pathname === '/api/data-grants', abortPatch)
  await unknown.page.getByRole('button', { name: '确认保存', exact: true }).click()
  await unknown.page.getByRole('alert').filter({ hasText: '保存结果待确认，请重新读取授权' }).waitFor()
  assert.equal(attempts, 1, 'Unknown outcome never blindly retries PATCH')
  assert.equal(await unknown.page.getByRole('button', { name: '确认保存', exact: true }).count(), 0)
  assert.equal(await unknown.page.getByRole('button', { name: '预览变更', exact: true }).count(), 0)
  const reconciliation = await holdRealResponse(unknown.page, { method: 'GET', pathname: '/api/data-grants', member: fixture.target.membershipId })
  await unknown.page.getByRole('button', { name: '重新读取授权', exact: true }).click()
  assert.equal(await reconciliation.ready, 200)
  assert.equal(await unknown.page.getByRole('button', { name: '确认保存', exact: true }).count(), 0)
  assert.equal(await unknown.page.getByTestId('grant-current-snapshot').count(), 0)
  await reconciliation.release()
  await wait(async () => await unknown.page.getByTestId('grant-current-snapshot').getAttribute('data-version') === before.version,
    'Explicit GET reconciles the saved version after an unknown outcome')
  assert.deepEqual(await snapshot(), before, 'The aborted-before-forwarding race never writes a grant')
  assert.equal(attempts, 1)
  assert.equal(await unknown.page.getByRole('button', { name: '确认保存', exact: true }).count(), 0)
  assert.deepEqual(unknown.errors, [])
  check('unknown-save-reconciliation')
  await unknown.context.close()
}
