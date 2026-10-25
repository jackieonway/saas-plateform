/** Real U05 UI/API/static-byte acceptance. --check is entirely offline. */
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import vm from 'node:vm'

const ORIGIN = 'http://127.0.0.1:13000'
const MATERIALS = '/var/lib/saas-uuid-u05-materials-20261024'
const DEFAULT_HANDOFF = `${MATERIALS}/credential-handoff.private.json`
const DEFAULT_MANIFEST = fileURLToPath(new URL('../../../docs/identity-sso/validation/uuid-u05-frontend-builds.json', import.meta.url))
const APPS = ['vue-workshop', 'legacy-jquery', 'report-center', 'websocket-chat', 'oauth-playground', 'work-orders', 'stores']
const VERSIONS = Object.fromEntries(APPS.map(code => [code, code === 'work-orders' ? '2.0.1-uuid-u05' : '1.0.1-uuid-u05']))
const UUID7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/
const digest = bytes => createHash('sha256').update(bytes).digest('hex')
const requireBrowser = createRequire(new URL('../../../../frontend/tests/browser/package.json', import.meta.url))
const pause = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds))

class CheckFailure extends Error {
  constructor(code) { super(code); this.code = code }
}
function requireCheck(value, code) { if (!value) throw new CheckFailure(code) }

function optionsOf(argv) {
  if (argv.length === 1 && argv[0] === '--check') return { check: true }
  requireCheck(argv[0] === '--run', 'EXPLICIT_RUN_REQUIRED')
  const options = { handoff: DEFAULT_HANDOFF, output: `${MATERIALS}/browser-validation-01`, distro: 'Ubuntu', manifest: DEFAULT_MANIFEST }
  for (let index = 1; index < argv.length; index += 2) {
    const key = argv[index]?.slice(2), value = argv[index + 1]
    requireCheck(['handoff', 'output', 'distro', 'manifest'].includes(key) && value, 'ARGUMENTS_INVALID')
    options[key] = value
  }
  requireCheck(options.distro === 'Ubuntu', 'FIXED_WSL_DISTRO_REQUIRED')
  requireCheck(options.handoff.startsWith(MATERIALS + '/') && !options.handoff.split('/').some(part => part === '.' || part === '..'), 'PRIVATE_HANDOFF_PATH_INVALID')
  requireCheck(new RegExp(`^${MATERIALS}/browser-validation-[A-Za-z0-9-]+$`).test(options.output), 'NEW_PRIVATE_OUTPUT_PATH_REQUIRED')
  return options
}

const PRIVATE_READ = String.raw`
import json,os,pathlib,stat,sys
p=pathlib.Path(sys.argv[1])
for a in (p,*p.parents):
 if a.is_symlink(): raise ValueError('private input link')
s=p.stat()
if not stat.S_ISREG(s.st_mode) or s.st_uid!=0 or stat.S_IMODE(s.st_mode)!=0o600 or s.st_nlink!=1 or s.st_size>4*1024*1024: raise ValueError('private input mode')
def pairs(items):
 d={}
 for k,v in items:
  if k in d: raise ValueError('duplicate key')
  d[k]=v
 return d
d=json.loads(p.read_text(encoding='utf-8-sig'),object_pairs_hook=pairs)
sys.stdout.write(json.dumps(d))
`
const PRIVATE_CREATE = String.raw`
import os,pathlib,stat,sys
p=pathlib.Path(sys.argv[1]);parent=p.parent
for a in (parent,*parent.parents):
 if a.is_symlink(): raise ValueError('private output link')
s=parent.stat()
if s.st_uid!=0 or stat.S_IMODE(s.st_mode)!=0o700: raise ValueError('private output parent')
os.mkdir(p,0o700)
`
const PRIVATE_WRITE = String.raw`
import os,pathlib,stat,sys
p=pathlib.Path(sys.argv[1]);name=sys.argv[2]
if name not in ('transcript.private.json','summary.json') and not (name.startswith('app-') and name.endswith('.png') and all(c.isalnum() or c in '-.' for c in name)): raise ValueError('private filename')
for a in (p,*p.parents):
 if a.is_symlink(): raise ValueError('private output link')
s=p.stat()
if s.st_uid!=0 or stat.S_IMODE(s.st_mode)!=0o700: raise ValueError('private output mode')
fd=os.open(p/name,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
with os.fdopen(fd,'wb') as f: f.write(sys.stdin.buffer.read())
`
function wsl(options, program, args = [], input) {
  try {
    return execFileSync('wsl.exe', ['-d', options.distro, '-u', 'root', '--', 'python3', '-c', program, ...args], {
      input, encoding: input instanceof Buffer ? undefined : 'utf8', windowsHide: true,
      stdio: ['pipe', 'pipe', 'pipe'], maxBuffer: 8 * 1024 * 1024, timeout: 30_000,
    })
  } catch { throw new CheckFailure('PRIVATE_WSL_IO_FAILED') }
}
function writePrivate(options, name, bytes) { wsl(options, PRIVATE_WRITE, [options.output, name], bytes) }

function handoffOf(value) {
  requireCheck(value?.format === 'saas-uuid-u05-runtime/1' && value.platformOrigin === ORIGIN, 'HANDOFF_FORMAT_OR_ORIGIN_INVALID')
  requireCheck(typeof value.login?.username === 'string' && value.login.username && typeof value.login?.password === 'string' && value.login.password, 'PRIVATE_LOGIN_REQUIRED')
  const applications = value.applications || value.apps
  requireCheck(Array.isArray(applications) && APPS.every(code => applications.some(app => app.code === code && UUID7.test(app.applicationId || app.id))), 'SEVEN_APPLICATION_BINDINGS_REQUIRED')
  // The runtime handoff's top-level context is the administrative context.
  // Each application carries its own authoritative business identity binding.
  const bindings = APPS.map(code => applications.find(app => app.code === code).context)
  const context = value.businessContext || bindings[0]
  requireCheck(context && ['userId', 'tenantId', 'organizationId'].every(key => UUID7.test(context[key])), 'BUSINESS_CONTEXT_V7_REQUIRED')
  requireCheck(bindings.every(binding => binding && ['userId', 'tenantId', 'organizationId'].every(key => binding[key] === context[key])), 'SEVEN_APPLICATIONS_MUST_SHARE_BUSINESS_CONTEXT')
  return { login: value.login, context, applications }
}

function expectedBytes(manifest) {
  requireCheck(manifest.format === 'saas-uuid-u05-frontends/1' && manifest.origin === ORIGIN && manifest.sdk_version === '3.0.0', 'BUILD_MANIFEST_INVALID')
  const map = new Map()
  for (const artifact of manifest.artifacts) {
    const code = artifact.app === 'work_orders' ? 'work-orders' : artifact.app
    requireCheck(code === 'platform' || APPS.includes(code), 'BUILD_APPLICATION_INVALID')
    if (code !== 'platform') requireCheck(artifact.version === VERSIONS[code], 'BUILD_VERSION_INVALID')
    for (const file of artifact.files) {
      requireCheck(typeof file.path === 'string' && !file.path.startsWith('/') && !file.path.split('/').some(part => !part || part === '.' || part === '..'), 'BUILD_FILE_PATH_INVALID')
      requireCheck(Number.isSafeInteger(file.bytes) && file.bytes >= 0 && /^[a-f0-9]{64}$/.test(file.sha256), 'BUILD_FILE_RECORD_INVALID')
      const pathname = code === 'platform' ? '/' + file.path : `/sub-apps/${code}/versions/${artifact.version}/${file.path}`
      requireCheck(!map.has(pathname), 'BUILD_FILE_DUPLICATE')
      map.set(pathname, { ...file, code })
    }
  }
  requireCheck(APPS.every(code => [...map.values()].some(record => record.code === code)), 'BUILD_SEVEN_APPLICATIONS_REQUIRED')
  return map
}

/** Observe real SDK input enumeration; do not replace responses, scripts or props. */
function installPropsObserver(appCodes) {
  const original = Object.keys, seen = new WeakMap(), observations = [], signals = new Map()
  const allowed = new Set(appCodes)
  const validId = value => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value)
  Object.keys = function keys(value) {
    const result = original(value)
    try {
      if (value && typeof value === 'object' && allowed.has(value.appCode) && typeof value.sdkVersion === 'string' && value.signal instanceof AbortSignal) {
        // import-html-entry supplies the actual response URL as sourceURL. The
        // same object is enumerated by the host before its child SDK receives it.
        const sdkConsumer = String(new Error().stack || '').includes(`/sub-apps/${value.appCode}/versions/`)
        const kind = sdkConsumer ? 'child' : 'host', kinds = seen.get(value) || new Set()
        if (kinds.has(kind)) return result
        kinds.add(kind); seen.set(value, kinds)
        signals.set(value.appCode, value.signal)
        observations.push({ app: value.appCode, sdk: value.sdkVersion, contract: value.contractVersion,
          sdkConsumer,
          v7: validId(value.user?.id) && validId(value.tenant?.id) && validId(value.organization?.id),
          hasToken: typeof value.token === 'string' && value.token.length > 0,
          cleanDto: !Object.hasOwn(value, 'container') && !Object.hasOwn(value, 'services'),
          version: value.deploymentVersion, route: value.routePath })
      }
    } catch { /* Observation cannot change the application's Object.keys result. */ }
    return result
  }
  Object.defineProperty(window, '__uuidU05Observation', { configurable: false, value: Object.freeze({
    snapshot: code => observations.filter(item => item.app === code).map(item => ({ ...item })),
    aborted: code => signals.has(code) && signals.get(code).aborted,
    restore: () => { Object.keys = original },
  }) })
}

async function waitFor(predicate, code, milliseconds = 30_000) {
  const deadline = Date.now() + milliseconds
  while (Date.now() < deadline) {
    if (await predicate()) return
    await pause(100)
  }
  throw new CheckFailure(code)
}

function flattenMenus(nodes) {
  const records = Array.isArray(nodes) ? nodes : nodes?.menus || nodes?.records || []
  return records.flatMap(item => [item, ...flattenMenus(item.children || [])])
}

function publicSummary(checks, counts, failure, evidenceSha256) {
  return { format: 'saas-uuid-u05-browser-result/1', status: failure ? 'FAIL' : 'PASS',
    scope: 'QUARANTINED_PREVIEW_REAL_UI_API_AND_FROZEN_STATIC_BYTES', checks, counts,
    ...(failure ? { failedCheck: failure } : {}), ...(evidenceSha256 ? { evidenceSha256 } : {}),
    limitations: ['No production traffic or final cutover', 'No fixture routes, response replacement or fabricated platform authorization used'] }
}

async function run(options) {
  const checks = [], transcript = { events: [], screenshots: [], pages: [], failure: null }
  const counts = { applications: 0, actualScripts: 0, sdkProps: 0, unmounted: 0, pageErrors: 0, networkFailures: 0, consoleErrors: 0, httpErrors: 0 }
  let browser, context, page, failure = null, outputCreated = false, currentCheck = 'PRIVATE_INPUTS'
  const check = (name, condition) => { currentCheck = name; requireCheck(condition, name); checks.push({ name, passed: true }) }
  try {
    // Playwright/Node debug output can contain fill values or child-process stdin.
    requireCheck(!process.env.DEBUG && !process.env.PWDEBUG && !process.env.NODE_DEBUG, 'DEBUG_LOGGING_MUST_BE_DISABLED')
    const handoff = handoffOf(JSON.parse(wsl(options, PRIVATE_READ, [options.handoff])))
    const bytes = expectedBytes(JSON.parse(readFileSync(options.manifest, 'utf8').replace(/^\uFEFF/, '')))
    wsl(options, PRIVATE_CREATE, [options.output]); outputCreated = true
    check('PRIVATE_INPUTS_AND_BUILD_MANIFEST', true)
    const playwright = requireBrowser('playwright')
    browser = await playwright.chromium.launch({ headless: true, channel: 'msedge', args: ['--no-proxy-server'] })
    transcript.browser = { product: 'msedge', version: browser.version(), playwright: requireBrowser('playwright/package.json').version }
    context = await browser.newContext({ viewport: { width: 1440, height: 1100 }, serviceWorkers: 'block' })
    await context.addInitScript(installPropsObserver, APPS)
    page = await context.newPage(); page.setDefaultTimeout(30_000)
    const pending = new Set(), contexts = new Map(), issues = new Map(), scripts = new Map(), entries = new Set(), menus = new Map()
    const pageErrors = [], networkFailures = [], consoleErrors = []
    let loginResponse = null, selectedResponse = null, profile = null, observerFailure = null, lastIssueAt = 0
    const track = promise => { pending.add(promise); void promise.catch(error => {
      observerFailure = error instanceof CheckFailure ? error.code : 'NETWORK_OBSERVATION_FAILED'
      transcript.events.push({ kind: 'observationFailure', type: error.name, message: error.message, stack: error.stack })
    }).finally(() => pending.delete(promise)) }
    page.on('pageerror', error => {
      const item = { name: error.name, message: error.message, stack: error.stack }
      pageErrors.push(item); transcript.events.push({ kind: 'pageerror', ...item }); counts.pageErrors++
    })
    page.on('console', message => { if (message.type() === 'error') {
      consoleErrors.push(message.text()); counts.consoleErrors++
      transcript.events.push({ kind: 'consoleerror', message: message.text(), location: message.location() })
    } })
    page.on('requestfailed', request => {
      const detail = request.failure()?.errorText || ''
      transcript.events.push({ kind: 'requestfailed', url: request.url(), error: detail })
      if (!/ERR_ABORTED|NS_BINDING_ABORTED|AbortError/.test(detail)) { networkFailures.push(detail); counts.networkFailures++ }
    })
    page.on('response', response => track((async () => {
      const request = response.request(), url = new URL(response.url()), pathname = url.pathname
      if (response.status() >= 400) {
        counts.httpErrors++
        transcript.events.push({ kind: 'http-error', url: response.url(), method: request.method(), resourceType: request.resourceType(), status: response.status() })
      }
      if (url.origin !== ORIGIN) return
      if (['/api/auth/login', '/api/auth/context', '/api/auth/profile'].includes(pathname)) transcript.events.push({ kind: 'auth-response', path: pathname, method: request.method(), status: response.status() })
      if (pathname === '/api/auth/login' && request.method() === 'POST') {
        loginResponse = { status: response.status(), body: await response.json() }
      } else if (pathname === '/api/auth/context' && request.method() === 'POST') {
        const value = await response.json()
        selectedResponse = { status: response.status(), user: value.user, tenant: value.tenant, organization: value.organization }
      } else if (pathname === '/api/auth/profile' && response.ok()) { profile = await response.json(); transcript.profile = profile }
      else if (pathname === '/api/auth/menus' && response.ok()) menus.set(url.searchParams.get('applicationId'), await response.json())
      else if (pathname === '/api/oauth/session-token' && request.method() === 'POST') {
        const code = request.postDataJSON()?.appCode
        if (APPS.includes(code)) { lastIssueAt = Date.now(); issues.set(code, response.status()); transcript.events.push({ kind: 'session-token', app: code, status: response.status() }) }
      } else {
        const match = /^\/api\/apps\/([^/]+)\/context$/.exec(pathname)
        if (match && APPS.includes(match[1])) {
          const body = await response.json()
          contexts.set(match[1], { status: response.status(), body })
          transcript.events.push({ kind: 'context', app: match[1], status: response.status(), body })
        }
      }
      const expected = bytes.get(pathname)
      if (expected) {
        const actual = await response.body()
        requireCheck(response.status() === 200 && actual.length === expected.bytes && digest(actual) === expected.sha256, 'ACTUAL_STATIC_BYTES_MISMATCH')
        const headers = await request.allHeaders()
        if (expected.code !== 'platform') requireCheck(!headers.authorization && !headers.cookie, 'STATIC_CREDENTIALS_NOT_OMITTED')
        if (pathname.endsWith('/index.html')) entries.add(expected.code)
        if (/\.(?:js|mjs)$/.test(pathname)) {
          requireCheck(/(?:javascript|ecmascript)/i.test(response.headers()['content-type'] || ''), 'ACTUAL_SCRIPT_MIME_INVALID')
          if (!scripts.has(expected.code)) scripts.set(expected.code, [])
          scripts.get(expected.code).push({ path: pathname, sha256: digest(actual), bytes: actual.length })
          counts.actualScripts++
        }
        transcript.events.push({ kind: 'static-byte-match', app: expected.code, path: pathname, sha256: digest(actual), bytes: actual.length })
      } else if (/\.(?:js|mjs)$/.test(pathname) && (pathname.startsWith('/assets/') || pathname.startsWith('/sub-apps/'))) {
        throw new CheckFailure('UNMANIFESTED_ACTUAL_SCRIPT')
      }
    })()))

    currentCheck = 'REAL_UI_PASSWORD_LOGIN'
    const hostEntry = await page.goto(`${ORIGIN}/login`, { waitUntil: 'domcontentloaded' })
    const hostHtml = await hostEntry.body(), expectedHost = bytes.get('/index.html')
    check('ACTUAL_FROZEN_HOST_HTML', hostEntry.status() === 200 && /text\/html/i.test(hostEntry.headers()['content-type'] || '') && hostHtml.length === expectedHost?.bytes && digest(hostHtml) === expectedHost.sha256)
    currentCheck = 'REAL_UI_PASSWORD_LOGIN'
    await page.locator('#username').fill(handoff.login.username)
    await page.locator('#password').fill(handoff.login.password)
    await page.getByRole('button', { name: '登 录', exact: true }).click()
    await page.getByRole('heading', { name: '选择登录租户', exact: true }).waitFor()
    await waitFor(() => loginResponse, 'LOGIN_RESPONSE_NOT_OBSERVED')
    check('REAL_UI_PASSWORD_LOGIN', loginResponse.status === 200)
    const candidates = loginResponse.body?.memberships?.filter(item => item.organizationId === handoff.context.organizationId && item.tenantId === handoff.context.tenantId && (!item.userId || item.userId === handoff.context.userId)) || []
    check('REAL_MEMBERSHIP_MATCHES_BUSINESS_CONTEXT', candidates.length === 1)
    const membership = candidates[0]
    transcript.membership = membership
    // The selected names come only from the real credential response and matching v7 IDs.
    await page.getByRole('textbox', { name: '搜索租户', exact: true }).fill(membership.tenantName)
    await page.getByRole('button').filter({ has: page.getByText(membership.tenantName, { exact: true }) }).click()
    await page.getByRole('heading', { name: '选择登录组织', exact: true }).waitFor()
    await page.getByRole('textbox', { name: '搜索组织或登录身份', exact: true }).fill(membership.organizationName)
    let choice = page.getByRole('button').filter({ has: page.getByText(membership.organizationName, { exact: true }) })
    if (await choice.count() > 1 && membership.username) choice = choice.filter({ hasText: membership.username })
    check('BUSINESS_CONTEXT_CHOICE_UNAMBIGUOUS', await choice.count() === 1)
    await choice.click()
    await waitFor(() => selectedResponse && profile, 'REAL_CONTEXT_OR_PROFILE_NOT_OBSERVED')
    check('REAL_UI_BUSINESS_CONTEXT', selectedResponse.status === 200 && selectedResponse.user?.id === handoff.context.userId && selectedResponse.tenant?.id === handoff.context.tenantId && selectedResponse.organization?.id === handoff.context.organizationId)
    check('SEVEN_AUTHORIZED_PROFILE_APPLICATIONS', APPS.every(code => profile.applications?.some(app => {
      const binding = handoff.applications.find(item => item.code === code)
      return app.code === code && app.id === (binding.applicationId || binding.id)
    })))
    transcript.profile = profile
    loginResponse = null // The credential ticket is not part of the private transcript.

    let previousRoot = null, previousCode = null
    for (const code of APPS) {
      currentCheck = 'REAL_APPLICATION_NAVIGATION'
      // The live token endpoint shares a real 30/minute limit with other U05 checks.
      if (lastIssueAt) await pause(Math.max(0, 3000 - (Date.now() - lastIssueAt)))
      const app = profile.applications.find(item => item.code === code)
      const rail = page.getByRole('complementary', { name: '应用导航', exact: true })
      await rail.getByRole('button', { name: `切换到应用 ${app.name}`, exact: true }).click()
      await page.waitForURL(url => url.pathname === `/micro/${code}` || url.pathname.startsWith(`/micro/${code}/`))
      await waitFor(() => issues.has(code) && contexts.has(code), 'REAL_APP_TOKEN_OR_CONTEXT_NOT_OBSERVED')
      check('SESSION_TOKEN_201', issues.get(code) === 201)
      const current = contexts.get(code)
      check('APPLICATION_CONTEXT_200_AND_NEW_VERSION', current.status === 200 && current.body.application?.deploymentVersion === VERSIONS[code] && current.body.application?.entryUrl === `/sub-apps/${code}/versions/${VERSIONS[code]}/index.html` && UUID7.test(current.body.application?.entryVersionId))
      check('APPLICATION_CONTEXT_IDENTITY_V7', ['user', 'tenant', 'organization'].every(key => UUID7.test(current.body[key]?.id) && current.body[key].id === profile[key].id))
      currentCheck = 'APPLICATION_CONTENT_NOT_RENDERED'
      const root = page.locator('.micro-mount-zone[aria-busy="false"]')
      await root.waitFor({ state: 'visible' })
      await waitFor(async () => (await root.innerText()).trim().length > 5, 'APPLICATION_CONTENT_NOT_RENDERED')
      check('NO_MOUNT_OR_TRUSTED_RUNTIME_ERROR', await page.getByText('子应用挂载失败', { exact: true }).count() === 0 && await page.locator('[data-testid="trusted-runtime-fault"]').count() === 0)
      await waitFor(async () => (await page.evaluate(code => window.__uuidU05Observation.snapshot(code), code)).length > 0, 'ACTUAL_SDK_PROPS_NOT_OBSERVED')
      const props = await page.evaluate(code => window.__uuidU05Observation.snapshot(code), code)
      check('ACTUAL_SDK3_PROPS_AND_V7', props.some(value => value.sdkConsumer) && props.every(value => value.sdk === '3.0.0' && value.contract === '2.0' && value.v7 && value.cleanDto && value.hasToken && value.version === VERSIONS[code]))
      counts.sdkProps += props.length
      const route = new URL(page.url()).pathname
      const actualMenus = menus.get(app.id)
      check('REAL_AUTHORIZED_MENU_NAVIGATION', flattenMenus(actualMenus).some(item => item.type === 'MENU' && item.visible !== false && item.path && (route === item.path || route.startsWith(item.path.replace(/\/$/, '') + '/'))))
      if (previousRoot) {
        await waitFor(() => previousRoot.evaluate(element => !element.isConnected || element.textContent.trim() === ''), 'PREVIOUS_APPLICATION_DOM_NOT_CLEANED')
        check('PREVIOUS_APPLICATION_SCOPE_ABORTED', await page.evaluate(code => window.__uuidU05Observation.aborted(code), previousCode))
        counts.unmounted++
        await previousRoot.dispose()
      }
      await Promise.all([...pending])
      check('ACTUAL_FROZEN_APPLICATION_ENTRY_AND_SCRIPTS', entries.has(code) && (scripts.get(code)?.length || 0) > 0 && !observerFailure)
      const image = await page.screenshot({ fullPage: true })
      writePrivate(options, `app-${code}.png`, image)
      transcript.screenshots.push({ app: code, file: `app-${code}.png`, bytes: image.length, sha256: digest(image) })
      transcript.pages.push({ app: code, route, version: VERSIONS[code], props, scripts: scripts.get(code) })
      previousRoot = await root.elementHandle(); previousCode = code
      counts.applications++
      // Observe a settled real view before another application issues its token.
      await pause(3000)
    }
    currentCheck = 'FINAL_APPLICATION_UNMOUNT'
    await page.getByRole('button', { name: '用户菜单', exact: true }).click()
    await page.getByRole('menuitem', { name: '个人中心', exact: true }).click()
    await page.waitForURL(url => url.pathname === '/profile')
    await waitFor(() => previousRoot.evaluate(element => !element.isConnected || element.textContent.trim() === ''), 'FINAL_APPLICATION_DOM_NOT_CLEANED')
    check('FINAL_APPLICATION_SCOPE_ABORTED', await page.evaluate(code => window.__uuidU05Observation.aborted(code), previousCode))
    check('FINAL_UNMOUNT_QUEUE_COMPLETED_AND_DOMAIN_HEALTHY', await page.evaluate(async () => {
      const domain = window[Symbol.for('saas.micro.trusted-runtime-domain.v1')]
      if (!domain || !domain.chain || typeof domain.chain.then !== 'function') return false
      let timer
      try {
        const settled = await Promise.race([domain.chain.then(() => true), new Promise(resolve => { timer = setTimeout(() => resolve(false), 20_000) })])
        return settled && domain.fault === null && !domain.signal.aborted
      } finally { clearTimeout(timer) }
    }))
    counts.unmounted++; await previousRoot.dispose()
    await Promise.all([...pending])
    check('NO_PAGE_OR_TRANSPORT_ERRORS', pageErrors.length === 0 && networkFailures.length === 0 && !observerFailure)
    transcript.pageErrors = pageErrors; transcript.networkFailures = networkFailures; transcript.consoleErrors = consoleErrors
    check('SEVEN_REAL_MOUNTS_AND_UNMOUNTS', counts.applications === 7 && counts.unmounted === 7)
  } catch (error) {
    failure = error instanceof CheckFailure ? error.code : currentCheck
    transcript.failure = { check: currentCheck, type: error.name, message: error.message, stack: error.stack }
    if (page && outputCreated) {
      try { transcript.failurePage = { url: page.url(), text: await page.locator('body').innerText(), session: await page.evaluate(() => {
        try { const record = JSON.parse(localStorage.getItem('platform-session-v1') || 'null'); return { status: record?.status, hasToken: !!record?.token, version: record?.version } } catch { return { unreadable: true } }
      }) } } catch { /* Diagnostics remain optional and private. */ }
      try { const image = await page.screenshot({ fullPage: true }); writePrivate(options, 'app-failure.png', image) } catch { /* Never disclose screenshot or exception bytes. */ }
    }
  } finally {
    try { await context?.close() } catch { failure ||= 'BROWSER_CONTEXT_CLOSE_FAILED' }
    try { await browser?.close() } catch { failure ||= 'BROWSER_CLOSE_FAILED' }
  }
  const evidence = Buffer.from(JSON.stringify(transcript, null, 2) + '\n')
  const summary = publicSummary(checks, counts, failure, digest(evidence))
  try {
    requireCheck(outputCreated, 'PRIVATE_OUTPUT_NOT_CREATED')
    writePrivate(options, 'transcript.private.json', evidence)
    writePrivate(options, 'summary.json', Buffer.from(JSON.stringify(summary, null, 2) + '\n'))
  } catch { summary.status = 'FAIL'; summary.failedCheck = 'PRIVATE_EVIDENCE_WRITE_FAILED' }
  process.stdout.write(JSON.stringify(summary) + '\n')
  return summary.status === 'PASS' ? 0 : 1
}

function offlineChecks() {
  const checks = []
  assert.throws(() => optionsOf([]), /EXPLICIT_RUN_REQUIRED/); checks.push('EXPLICIT_RUN_REQUIRED')
  assert.throws(() => optionsOf(['--run', '--output', MATERIALS + '/../public']), /NEW_PRIVATE_OUTPUT_PATH_REQUIRED/); checks.push('PRIVATE_OUTPUT_FENCE')
  const exampleId = '019a0b00-0000-7000-8000-000000000001'
  assert.throws(() => handoffOf({ format: 'saas-uuid-u05-runtime/1', platformOrigin: 'http://127.0.0.1:3000' }), /HANDOFF_FORMAT_OR_ORIGIN_INVALID/); checks.push('FIXED_REAL_ORIGIN')
  const probe = vm.createContext({ window: {}, AbortSignal, AbortController })
  vm.runInContext(`(${installPropsObserver.toString()})(${JSON.stringify(APPS)}); globalThis.scope = new AbortController(); globalThis.value = {appCode:'stores',sdkVersion:'3.0.0',contractVersion:'2.0',user:{id:'${exampleId}'},tenant:{id:'${exampleId}'},organization:{id:'${exampleId}'},signal:scope.signal,token:'must-remain-private',deploymentVersion:'1.0.1-uuid-u05'}; Object.keys(value);`, probe)
  const observed = vm.runInContext('window.__uuidU05Observation.snapshot("stores")', probe)
  assert.equal(observed.length, 1); assert.equal(observed[0].v7, true)
  assert.ok(!JSON.stringify(observed).includes('must-remain-private') && !JSON.stringify(observed).includes(exampleId)); checks.push('REAL_PROPS_OBSERVER_REDACTS_VALUES')
  vm.runInContext('Object.keys(value)', probe, { filename: 'http://127.0.0.1:13000/sub-apps/stores/versions/1.0.1-uuid-u05/assets/micro-app.umd.js' })
  const consumed = vm.runInContext('window.__uuidU05Observation.snapshot("stores")', probe)
  assert.equal(consumed.length, 2); assert.equal(consumed[0].sdkConsumer, false); assert.equal(consumed[1].sdkConsumer, true); checks.push('CHILD_SDK_CALLSITE_DISTINCT_FROM_HOST')
  vm.runInContext('scope.abort()', probe)
  assert.equal(vm.runInContext('window.__uuidU05Observation.aborted("stores")', probe), true); checks.push('UNMOUNT_SIGNAL_OBSERVATION')
  const summary = publicSummary([], { applications: 0 }, 'FIXED_CHECK_NAME', 'a'.repeat(64))
  assert.ok(!JSON.stringify(summary).includes('must-remain-private')); checks.push('PUBLIC_RESULT_REDACTION')
  assert.deepEqual(flattenMenus({ menus: [{ type: 'GROUP', children: [{ type: 'MENU', path: '/micro/stores' }] }] }).filter(item => item.type === 'MENU').map(item => item.path), ['/micro/stores']); checks.push('REAL_MENUS_RESPONSE_ENVELOPE')
  const business = { userId: exampleId, tenantId: exampleId, organizationId: exampleId }
  const binding = handoffOf({ format: 'saas-uuid-u05-runtime/1', platformOrigin: ORIGIN, login: { username: 'offline', password: 'private-offline' }, context: { ...business, organizationId: '019a0b00-0000-7000-8000-000000000002' }, applications: APPS.map(code => ({ code, applicationId: exampleId, context: business })) })
  assert.equal(binding.context.organizationId, exampleId); checks.push('APPLICATION_BUSINESS_CONTEXT_PRECEDES_ADMIN_CONTEXT')
  process.stdout.write(JSON.stringify({ status: 'PASS', mode: 'OFFLINE_ONLY', checks, count: checks.length }) + '\n')
}

try {
  const options = optionsOf(process.argv.slice(2))
  if (options.check) offlineChecks()
  else process.exitCode = await run(options)
} catch (error) {
  process.stdout.write(JSON.stringify({ format: 'saas-uuid-u05-browser-result/1', status: 'FAIL', failedCheck: error instanceof CheckFailure ? error.code : 'BROWSER_VALIDATOR_INITIALIZATION_FAILED' }) + '\n')
  process.exitCode = 1
}
