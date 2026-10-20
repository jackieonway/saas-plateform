/** Built Vite page -> actual Rust playground -> actual platform/owned PostgreSQL. */
import assert from 'node:assert/strict'
import { createHash, randomBytes } from 'node:crypto'
import { readFileSync, readdirSync, mkdirSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { launchBrowser, browserName, playwrightVersion } from './browser/runtime.mjs'
import { oauthPlaygroundChecks as CHECK_IDS } from '../scripts/ci/oauth-playground.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const chunks = []
for await (const chunk of process.stdin) chunks.push(chunk)
const fixture = JSON.parse(Buffer.concat(chunks).toString('utf8'))
assert.equal(fixture.schemaVersion, 1)
for (const origin of [fixture.origin, fixture.platformOrigin]) assert(/^http:\/\/127\.0\.0\.1:\d+$/.test(origin), 'Owned loopback origin required')
const checks = [], contexts = [], observed = [], browserErrors = []
const secrets = [fixture.session, fixture.password, ...fixture.secrets]
let browser, screenshot, stage = 'initialization'
const check = id => {
  assert(CHECK_IDS.includes(id) && !checks.some(value => value.id === id), 'Unique mandatory check ID required')
  checks.push({ id, status: 'PASS' })
  console.log('PASS phase24 live ' + id)
}
const safe = value => secrets.reduce((text, secret) => text.replaceAll(secret, '[REDACTED]'), String(value))
  .replace(/([?&](?:code|state|code_verifier|device_code)=)[^&\s"']*/gi, '$1[REDACTED]')
  .replace(/\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b/g, '[JWT REDACTED]')
function noPrivate(value, message) {
  const text = typeof value === 'string' ? value : JSON.stringify(value)
  assert(!secrets.some(secret => text.includes(secret)), message)
  assert(!/"(?:access_token|refresh_token|device_code|code_verifier|client_secret|verifier)"\s*:/i.test(text), message)
}
async function post(context, endpoint, body = {}, origin = fixture.origin) {
  stage = 'playground-api-' + endpoint
  const response = await context.request.post(fixture.origin + '/api/' + endpoint, {
    headers: { Origin: origin, 'Content-Type': 'application/json' }, data: body,
  })
  const value = await response.json()
  noPrivate(value, 'Playground response must contain only public metadata')
  return { status: response.status(), body: value }
}
async function platform(endpoint, body, token = fixture.session) {
  const response = await fetch(fixture.platformOrigin + endpoint, {
    method: body === undefined ? 'GET' : 'POST',
    headers: { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    signal: AbortSignal.timeout(15_000),
  })
  return { status: response.status, body: await response.json() }
}
async function ui(page, id, endpoint, expected = 200) {
  stage = 'browser-resource-' + endpoint
  const pending = page.waitForResponse(response => new URL(response.url()).pathname === '/api/' + endpoint && response.request().method() === 'POST')
  await page.getByTestId(id).click()
  const response = await pending
  assert(response.status() === expected, 'Actual resource operation returned its declared platform result: ' + endpoint)
  const value = await response.json()
  noPrivate(value, 'UI responses contain only public metadata')
  return value
}
async function open() {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1100 } })
  contexts.push(context)
  const page = await context.newPage()
  page.setDefaultTimeout(20_000)
  page.on('request', request => {
    if (!request.url().startsWith(fixture.origin + '/api/')) return
    const body = request.postData() || ''
    assert(!/"(?:access_token|refresh_token|code_verifier|client_secret|verifier)"\s*:/i.test(body), 'Browser does not submit retained tokens or PKCE verifier')
  })
  page.on('pageerror', error => browserErrors.push(safe(error.message)))
  page.on('response', async response => {
    if (!response.url().startsWith(fixture.origin + '/api/')) return
    const value = await response.text().catch(() => '')
    if (observed.length < 300) observed.push(value)
  })
  await page.goto(fixture.origin, { waitUntil: 'networkidle' })
  await page.getByTestId('flow-authorization_code').waitFor()
  return { context, page }
}
function scanDist(directory) {
  for (const name of readdirSync(directory, { withFileTypes: true })) {
    const target = path.join(directory, name.name)
    if (name.isDirectory()) scanDist(target)
    else {
      const bytes = readFileSync(target).toString('utf8')
      noPrivate(bytes, 'Built dist contains private fixture material')
      assert(!bytes.includes('OAUTH_AUTH_CLIENT_SECRET'), 'Vite output must not consume private server configuration')
    }
  }
}

try {
  stage = 'built-dist-scan'
  scanDist(path.join(root, 'mini-services/oauth-playground/frontend/dist'))
  check('built-dist-no-private-material')
  browser = await launchBrowser()
  stage = 'open-built-vite-page'
  const { context, page } = await open()
  stage = 'authorization-start-response'
  // The UI navigates immediately after reading auth-start. Observe its actual
  // fetch response inside that document before returning the same Response to
  // the application. This avoids CDP losing its body during navigation and
  // preserves every real HTTP request and response without routing or stubs.
  let authorizationStarted
  const startReply = new Promise(resolve => { authorizationStarted = resolve })
  await page.exposeFunction('__phase24AuthorizationStarted', value => authorizationStarted(value))
  await page.evaluate(() => {
    const originalFetch = window.fetch.bind(window)
    window.fetch = async (...args) => {
      const response = await originalFetch(...args)
      if (new URL(response.url).pathname === '/api/auth-start') {
        await window.__phase24AuthorizationStarted(await response.clone().json())
      }
      return response
    }
  })
  await page.getByTestId('start-auth').click({ noWaitAfter: true })
  const start = await startReply
  noPrivate(start, 'PKCE initiation must never return verifier')
  const authorize = new URL(start.authorizeUrl)
  assert(authorize.searchParams.get('code_challenge_method') === 'S256', 'Actual authorization uses PKCE S256')
  assert(authorize.searchParams.has('code_challenge') && !authorize.searchParams.has('code_verifier'), 'Authorization URL includes only the PKCE challenge')
  stage = 'platform-consent-navigation'
  await page.waitForURL(url => url.origin === fixture.platformOrigin && url.pathname === '/consent')
  const request = new URL(page.url()).searchParams.get('request_id')
  assert(request, 'Real platform created a consent request')
  stage = 'platform-consent-detail'
  const consent = await platform('/api/oauth/consent/' + request)
  assert(consent.status === 200, 'Actual platform consent is readable through its authenticated API')
  stage = 'platform-consent-approval'
  const decision = await platform('/api/oauth/consent/' + request + '/decision', { approve: true })
  assert(decision.status === 200, 'Actual platform consent approval commits')
  const callback = new URL(decision.body.redirectTo)
  const state = callback.searchParams.get('state'), code = callback.searchParams.get('code')
  assert(state && code, 'Platform returned an actual authorization code')
  const stranger = await browser.newContext()
  contexts.push(stranger)
  await stranger.request.get(fixture.origin + '/api/config')
  const wrongSession = await post(stranger, 'callback', { state, code })
  assert(wrongSession.status >= 400 && wrongSession.body.error === 'invalid_state', 'A different initiating session cannot consume PKCE state')
  const wrongState = await post(context, 'callback', { state: randomBytes(32).toString('base64url'), code })
  assert(wrongState.status >= 400 && wrongState.body.error === 'invalid_state', 'Wrong PKCE state is rejected')
  stage = 'browser-callback-exchange'
  const completed = page.waitForResponse(response => new URL(response.url()).pathname === '/api/callback')
  await page.goto(callback.href)
  const exchanged = await completed
  assert(exchanged.status() === 200 && (await exchanged.json()).granted === true, 'Callback UI exchanges code through the actual Rust service')
  await page.getByTestId('token-summary').waitFor()
  check('pkce-real-consent-callback')
  const replay = await post(context, 'callback', { state, code })
  assert(replay.status >= 400 && replay.body.error === 'invalid_state', 'Consumed callback cannot be replayed')
  check('state-wrong-session-replay')

  await page.getByTestId('resource-flow').selectOption('authorization_code')
  await ui(page, 'resource-userinfo', 'userinfo')
  const active = await ui(page, 'resource-introspect', 'introspect')
  assert(active.active === true, 'Issued authorization token is active on the real platform')
  const refreshed = await ui(page, 'resource-refresh-token', 'refresh-token')
  assert(refreshed.granted === true && refreshed.hasRefreshToken === true, 'Real refresh rotates the retained server-side pair')
  await ui(page, 'resource-external-menus', 'external-menus', 403)
  await ui(page, 'resource-external-roles', 'external-roles', 403)
  await ui(page, 'resource-revoke', 'revoke')
  const inactive = await ui(page, 'resource-introspect', 'introspect', 400)
  assert(inactive.error === 'no_token', 'Revocation removes retained local bearer material')
  await ui(page, 'resource-userinfo', 'userinfo', 400)
  check('authorization-userinfo-introspection-refresh-revoke')

  stage = 'browser-device-start'
  await page.getByTestId('flow-device').click()
  const deviceReply = page.waitForResponse(response => new URL(response.url()).pathname === '/api/device-start')
  await page.getByTestId('start-device').click()
  const device = await (await deviceReply).json()
  noPrivate(device, 'Device initiation never returns the retained device_code')
  const userCode = device.user_code
  assert(typeof userCode === 'string' && userCode.length > 0, 'Real device authorization returned a user code')
  stage = 'platform-device-detail'
  const deviceInfo = await platform('/api/device/' + encodeURIComponent(userCode))
  assert(deviceInfo.status === 200 && deviceInfo.body.status === 'PENDING', 'Actual platform owns a pending device approval')
  const polling = page.waitForResponse(async response => new URL(response.url()).pathname === '/api/device-token' && response.status() === 200)
  stage = 'platform-device-approval'
  const deviceApproved = await platform('/api/device/' + encodeURIComponent(userCode) + '/decision', { approve: true })
  assert(deviceApproved.status === 200, 'Actual authenticated platform device approval succeeds')
  stage = 'browser-device-poll-until-issued'
  const deviceSuccess = await polling
  assert((await deviceSuccess.json()).granted === true, 'Browser polling observes actual device token issuance')
  check('device-real-approval-and-poll')
  await page.getByTestId('resource-flow').selectOption('device')
  await ui(page, 'resource-userinfo', 'userinfo')
  const publicIntrospection = await post(context, 'introspect', { flow: 'device' })
  assert(publicIntrospection.status >= 400 && publicIntrospection.body.error === 'unsupported_flow', 'Public device client explicitly refuses confidential introspection')
  check('public-device-introspection-explicitly-unsupported')

  await page.getByTestId('flow-client_credentials').click()
  const service = await ui(page, 'issue-client_credentials', 'cc-token')
  assert(service.granted === true && service.hasRefreshToken === false, 'Controlled client credentials issue an actual service token')
  await page.getByTestId('resource-flow').selectOption('client_credentials')
  const serviceActive = await ui(page, 'resource-introspect', 'introspect')
  assert(serviceActive.active === true, 'Service token introspection authenticates its own configured client')
  await ui(page, 'resource-userinfo', 'userinfo', 401)
  const noRefresh = await post(context, 'refresh-token', { flow: 'client_credentials' })
  assert(noRefresh.status >= 400, 'Unconfigured service refresh is explicitly rejected')
  check('client-credentials-controlled-service')

  await page.getByTestId('flow-password').click()
  await page.locator('[name="username"]').fill(fixture.username)
  await page.locator('[name="password"]').fill(fixture.password)
  const legacy = await ui(page, 'issue-password', 'password-token')
  assert(legacy.granted === true && legacy.hasRefreshToken === true, 'Explicit legacy password configuration exercises actual platform compatibility')
  await page.getByTestId('resource-flow').selectOption('password')
  await ui(page, 'resource-userinfo', 'userinfo')
  const legacyActive = await ui(page, 'resource-introspect', 'introspect')
  assert(legacyActive.active === true, 'Legacy issued token introspection is real')
  await ui(page, 'resource-refresh-token', 'refresh-token')
  check('legacy-password-explicitly-enabled')
  for (const flow of ['device', 'client_credentials', 'password']) {
    await page.getByTestId('resource-flow').selectOption(flow)
    await ui(page, 'resource-external-menus', 'external-menus', 403)
    await ui(page, 'resource-external-roles', 'external-roles', 403)
  }
  check('ordinary-oauth-external-default-deny')

  stage = 'platform-app-token-issuance'
  const appIssued = await platform('/api/oauth/session-token', { appCode: fixture.application })
  assert(appIssued.status === 201 && typeof appIssued.body.accessToken === 'string', 'Actual app-bound issuer supplies the supported external-view token')
  secrets.push(appIssued.body.accessToken)
  stage = 'platform-app-bound-external-positive'
  const menus = await platform('/api/external/menus', undefined, appIssued.body.accessToken)
  const roles = await platform('/api/external/roles', undefined, appIssued.body.accessToken)
  assert(menus.status === 200 && menus.body.application.code === fixture.application && menus.body.menus.length === 1, 'App-bound external menus expose only current app permission')
  assert(roles.status === 200 && roles.body.organizations.length === 1 && roles.body.organizations[0].roles.length === 1, 'App-bound external roles expose only current user organization')
  stage = 'platform-app-bound-external-negative'
  const wrongAudience = await platform('/api/apps/other-app/context', undefined, appIssued.body.accessToken)
  const platformSessionDenied = await platform('/api/external/menus')
  assert(wrongAudience.status === 403 && platformSessionDenied.status === 403, 'Wrong audience and platform session cannot bypass application token boundary')
  check('app-bound-external-current-user-positive-negative')

  const forgedOrigin = await post(context, 'cc-token', {}, 'https://wrong-origin.example.test')
  assert(forgedOrigin.status === 403, 'Mutation requires exact configured origin')
  const verifier = await context.request.get(fixture.origin + '/api/verifier?state=' + encodeURIComponent(state))
  assert([404, 405].includes(verifier.status()), 'No public verifier enumeration endpoint exists')
  const missingAsset = await context.request.get(fixture.origin + '/assets/phase24-missing.js')
  assert(missingAsset.status() === 404, 'Missing static asset must not receive SPA HTML')
  check('origin-and-verifier-endpoint-rejected')
  await page.getByTestId('resource-flow').selectOption('password')
  await ui(page, 'resource-revoke', 'revoke')
  await page.locator('[name="password"]').fill('')
  stage = 'browser-private-material-scan'
  const storage = await page.evaluate(() => ({ local: { ...localStorage }, session: { ...sessionStorage }, url: location.href }))
  noPrivate(storage, 'Browser URL and storage do not retain private material')
  assert(!/[?&](?:code|state|code_verifier)=/.test(storage.url), 'Callback clears code/state from browser address')
  for (const value of observed) noPrivate(value, 'Browser-visible API response does not leak private material')
  assert(browserErrors.length === 0, 'Browser completed without script errors')
  check('browser-storage-url-response-no-private-material')
  if (process.env.PHASE24_SCREENSHOT_PATH) {
    stage = 'browser-screenshot'
    const output = path.resolve(process.env.PHASE24_SCREENSHOT_PATH)
    assert(path.isAbsolute(process.env.PHASE24_SCREENSHOT_PATH), 'Screenshot path must be absolute')
    assert(!output.startsWith(root + path.sep) || output.startsWith(path.join(root, 'target') + path.sep), 'Screenshot must not overwrite managed source')
    mkdirSync(path.dirname(output), { recursive: true })
    await page.screenshot({ path: output, fullPage: true })
    screenshot = { path: output, sha256: createHash('sha256').update(readFileSync(output)).digest('hex') }
  }
  assert(checks.length === CHECK_IDS.length, 'All mandatory actual-platform checks completed')
  console.log('PHASE24_LIVE_RESULT ' + JSON.stringify({ schemaVersion: 1, product: browserName,
    browserVersion: await browser.version(), playwrightVersion, passed: checks.length, failed: 0, skipped: 0,
    checks, approvalMode: 'authenticated-platform-consent-and-device-APIs', frontendMode: 'actual-built-vite-dist',
    externalMatrix: { ordinaryOAuth: 403, appBoundMenus: 200, appBoundRoles: 200, wrongAudience: 403 }, screenshot }))
} catch (error) {
  console.error('PHASE24_LIVE_FAILURE stage=' + stage + ' ' + safe(error.message))
  process.exitCode = 1
} finally {
  for (const context of contexts) await context.close().catch(() => {})
  if (browser) await browser.close()
}
