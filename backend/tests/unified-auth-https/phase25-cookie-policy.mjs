/** Chromium's real BLOCK content setting; never intercept requests/cookies. */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const PREFIX = 'rust-saas-phase25-cookie-policy-';
const OWNER = 'unified-auth-phase25-cookie-policy/1\n';

export async function withCookiesBlocked(chromium, options, action, afterCookiesAllowed) {
  const temporary = fs.realpathSync(os.tmpdir());
  const profile = fs.mkdtempSync(path.join(temporary, PREFIX));
  const marker = path.join(profile, 'OWNER');
  let context;
  try {
    fs.writeFileSync(marker, OWNER, { flag: 'wx', mode: 0o600 });
    fs.mkdirSync(path.join(profile, 'Default'));
    fs.writeFileSync(path.join(profile, 'Default', 'Preferences'), JSON.stringify({
      profile: { default_content_setting_values: { cookies: 2 } },
    }), { flag: 'wx', mode: 0o600 });
    context = await chromium.launchPersistentContext(profile, {
      headless: true, channel: 'chromium', serviceWorkers: 'block', viewport: { width: 1100, height: 800 }, ...options,
    });
    const blocked = await action(context);
    if (afterCookiesAllowed === undefined) return blocked;
    assert.equal(typeof afterCookiesAllowed, 'function', 'PHASE25_COOKIE_RECOVERY_CALLBACK_REQUIRED');
    // Chromium reads the content setting at profile startup. Reopen the same
    // private profile only after its blocked context has actually closed.
    await context.close(); context = undefined;
    const preferences = path.join(profile, 'Default', 'Preferences');
    const info = fs.lstatSync(preferences);
    assert(info.isFile() && !info.isSymbolicLink() && info.size < 1024 * 1024
      && fs.readFileSync(marker, 'utf8') === OWNER, 'PHASE25_COOKIE_PREFERENCES_OWNERSHIP_FAILED');
    const values = JSON.parse(fs.readFileSync(preferences, 'utf8'));
    assert.equal(values.profile?.default_content_setting_values?.cookies, 2, 'PHASE25_COOKIE_BLOCK_SETTING_CHANGED');
    values.profile.default_content_setting_values.cookies = 1;
    fs.writeFileSync(preferences, JSON.stringify(values), { mode: 0o600 });
    context = await chromium.launchPersistentContext(profile, {
      headless: true, channel: 'chromium', serviceWorkers: 'block', viewport: { width: 1100, height: 800 }, ...options,
    });
    return { ...blocked, recoveryAfterPolicyChange: await afterCookiesAllowed(context), samePrivateProfileRestored: true };
  } finally {
    // close errors are fatal. Do not claim cleanup or delete a still-open profile.
    if (context) await context.close();
    assert(path.dirname(profile) === temporary && path.basename(profile).startsWith(PREFIX)
      && fs.realpathSync(profile) === profile && !fs.lstatSync(profile).isSymbolicLink()
      && fs.readFileSync(marker, 'utf8') === OWNER, 'PHASE25_COOKIE_PROFILE_OWNERSHIP_FAILED');
    fs.rmSync(profile, { recursive: true, maxRetries: 8, retryDelay: 100 });
    assert(!fs.existsSync(profile), 'PHASE25_COOKIE_PROFILE_CLEANUP_FAILED');
  }
}

// This function is deliberately closure-free so Playwright can execute it in
// the actual denied page. It returns counts only; storage and URL values stay
// in that browser's memory and are never included in an assertion message.
export function credentialFallbackFacts({ sensitiveValues = [] } = {}) {
  const privateValues = sensitiveValues.filter(value => typeof value === 'string' && value.length >= 8);
  const credentialName = /(?:^|[_.:-])(?:access.?token|refresh.?token|id.?token|logout.?token|token|password|client.?secret|code.?verifier|verifier|csrf(?:.?token)?|authorization|cookie)(?:$|[_.:-])/i;
  const credentialValue = value => privateValues.some(secret => value.includes(secret))
    || /\bBearer\s+\S+/i.test(value) || /\beyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]{6,}\b/.test(value);
  const nestedCredential = (value, depth = 0) => {
    if (depth > 16) return true;
    if (typeof value === 'string') return credentialValue(value);
    if (Array.isArray(value)) return value.some(item => nestedCredential(item, depth + 1));
    if (value && typeof value === 'object') return Object.entries(value).some(([key, item]) =>
      credentialName.test(key) || nestedCredential(item, depth + 1));
    return false;
  };
  const url = new URL(location.href);
  const parameters = [...url.searchParams, ...new URLSearchParams(url.hash.slice(1))];
  let fragment = url.hash.slice(1); try { fragment = decodeURIComponent(fragment); } catch {}
  let fragmentObject = null; try { fragmentObject = JSON.parse(fragment); } catch {}
  const urlCredentialFields = Number(Boolean(url.username || url.password)) + parameters.filter(([key, value]) =>
    credentialName.test(key) || /^(?:code|secret)$/i.test(key) || credentialValue(value)).length
    + Number(credentialValue(fragment) || nestedCredential(fragmentObject));
  let localStorageKeys = 0, sessionStorageKeys = 0, storageCredentialEntries = 0, storageReadFailures = 0, storageBlockedByBrowser = 0;
  for (const kind of ['localStorage', 'sessionStorage']) {
    try {
      const storage = window[kind];
      for (let index = 0; index < storage.length; index++) {
        const key = storage.key(index), value = storage.getItem(key) ?? '';
        if (kind === 'localStorage') localStorageKeys++; else sessionStorageKeys++;
        let parsed = null; try { parsed = JSON.parse(value); } catch {}
        if (credentialName.test(key) || credentialValue(value) || nestedCredential(parsed)) storageCredentialEntries++;
      }
    } catch (error) {
      if (error?.name === 'SecurityError') storageBlockedByBrowser++;
      else storageReadFailures++;
    }
  }
  return { urlCredentialFields, localStorageKeys, sessionStorageKeys, storageCredentialEntries, storageReadFailures, storageBlockedByBrowser };
}

export async function checkBlockedCookieLogin(context, { authOrigin, consoleOrigin, account,
  observePage = page => page, progress = () => {} }) {
  const page = observePage(await context.newPage());
  page.setDefaultTimeout(15000); page.setDefaultNavigationTimeout(25000);
  const cdp = await context.newCDPSession(page);
  const blockedReasons = new Set(), loginRequestIds = new Set(), requestCookieHeaders = new Map();
  let navigations = 0, interactionStatus = null;
  const checkpoint = (stage, facts = {}) => progress(stage, facts);
  async function diagnostic() {
    const state = await page.evaluate(({ authOrigin, consoleOrigin }) => {
      const visible = id => Boolean(document.querySelector(id)?.getClientRects().length);
      const pathname = ['/oauth/login', '/auth/login', '/auth/config'].includes(location.pathname)
        ? location.pathname : /^\/oauth\/interaction\/[0-9a-f-]{36}$/.test(location.pathname) ? '/oauth/interaction/:id' : 'OTHER';
      const screen = document.querySelector('#auth-content')?.dataset.screen;
      return { origin: location.origin === authOrigin ? 'AUTH' : location.origin === consoleOrigin ? 'CONSOLE' : 'OTHER',
        pathname, screen: ['LOGIN_REQUIRED', 'CONSENT_REQUIRED', 'CONTEXT_SELECTION_REQUIRED', 'RESTART_REQUIRED'].includes(screen) ? screen : 'OTHER',
        loginVisible: visible('#login-form'), errorVisible: visible('#error-message'),
        documentCookiePresent: document.cookie.length > 0 };
    }, { authOrigin, consoleOrigin }).catch(() => ({ pageUnavailable: true }));
    return { ...state, navigationCount: navigations, interactionStatus,
      userPreferenceCookieRejectionObserved: blockedReasons.has('UserPreferences'),
      loginRequestCount: loginRequestIds.size };
  }
  cdp.on('Network.responseReceivedExtraInfo', event => {
    for (const blocked of event.blockedCookies ?? []) for (const reason of blocked.blockedReasons) blockedReasons.add(reason);
  });
  cdp.on('Network.requestWillBeSent', event => {
    if (event.request.url === `${authOrigin}/api/sso/login` && event.request.method === 'POST') loginRequestIds.add(event.requestId);
  });
  cdp.on('Network.requestWillBeSentExtraInfo', event => {
    requestCookieHeaders.set(event.requestId, Object.keys(event.headers).some(name => name.toLowerCase() === 'cookie'));
  });
  page.on('response', response => {
    const url = new URL(response.url());
    if (url.origin === authOrigin && /^\/oauth\/interaction\/[0-9a-f-]{36}$/.test(url.pathname)
      && response.request().method() === 'GET') interactionStatus = response.status();
  });
  page.on('request', request => {
    if (request.isNavigationRequest() && request.frame() === page.mainFrame()) navigations++;
  });
  try {
    checkpoint('ENABLE_BROWSER_NETWORK_OBSERVATION'); await cdp.send('Network.enable');
    checkpoint('NAVIGATE_WITH_BLOCKED_FLOW_COOKIE');
    await page.goto(`${consoleOrigin}/auth/login?return=%2Fsystem%2Foauth%2Fclients`, { waitUntil: 'domcontentloaded' });
    checkpoint('AWAIT_VISIBLE_ENTRY_OUTCOME');
    const handle = await page.waitForFunction(() => {
      if (document.querySelector('#error-message')?.getClientRects().length) return 'error';
      if (document.querySelector('#login-form')?.getClientRects().length) return 'login';
      return false;
    });
    let entryOutcome;
    try { entryOutcome = await handle.jsonValue(); } finally { await handle.dispose(); }
    checkpoint('CHECK_FLOW_COOKIE_GATE', await diagnostic());
    // The current canonical interaction is bound to its browser cookie. Cookie
    // BLOCK therefore rejects that real GET before a login form can be shown.
    assert(entryOutcome === 'error' && interactionStatus === 403
      && !await page.locator('#login-form').isVisible(), 'PHASE25_UI_BLOCKED_COOKIE_FLOW_GATE_NOT_REJECTED');
    checkpoint('PREAUTH_AND_LOGIN_WITH_BLOCKED_COOKIES');
    const authentication = await page.evaluate(async ({ authOrigin, account }) => {
      if (location.origin !== authOrigin) throw new Error('PHASE25_UI_COOKIE_AUTH_ORIGIN_CHANGED');
      const preauth = await fetch('/api/sso/preauth', { credentials: 'same-origin', cache: 'no-store',
        redirect: 'error', headers: { Accept: 'application/json' }, signal: AbortSignal.timeout(10000) });
      const challenge = await preauth.json();
      if (preauth.status !== 200 || typeof challenge.csrfToken !== 'string') return { preauthStatus: preauth.status, loginStatus: null };
      const login = await fetch('/api/sso/login', { method: 'POST', credentials: 'same-origin', cache: 'no-store', redirect: 'error',
        headers: { Accept: 'application/json', 'Content-Type': 'application/json', 'X-CSRF-Token': challenge.csrfToken },
        body: JSON.stringify({ username: account.username, password: account.password }), signal: AbortSignal.timeout(10000) });
      return { preauthStatus: preauth.status, loginStatus: login.status, privateChallenge: challenge.csrfToken };
    }, { authOrigin, account });
    const sensitiveValues = [account.password, authentication.privateChallenge];
    delete authentication.privateChallenge;
    checkpoint('CHECK_BLOCKED_PREAUTH_LOGIN', authentication);
    assert(authentication.preauthStatus === 200 && authentication.loginStatus === 403, 'PHASE25_UI_BLOCKED_COOKIE_LOGIN_ACCEPTED');
    checkpoint('CHECK_ANONYMOUS_SSO_AND_EMPTY_COOKIE_STORE');
    const ssoStatus = await page.evaluate(() => fetch('/api/sso/session', { credentials: 'same-origin', cache: 'no-store',
      redirect: 'error', signal: AbortSignal.timeout(10000) }).then(response => response.status));
    assert.equal(ssoStatus, 401, 'PHASE25_UI_BLOCKED_COOKIE_SSO_AUTHENTICATED');
    assert.equal(await page.evaluate(() => document.cookie), '', 'PHASE25_UI_BLOCKED_DOCUMENT_COOKIE_PRESENT');
    assert.equal((await context.cookies()).length, 0, 'PHASE25_UI_BLOCKED_COOKIE_STORED');
    assert(blockedReasons.has('UserPreferences') && loginRequestIds.size === 1
      && [...loginRequestIds].every(id => requestCookieHeaders.get(id) === false), 'PHASE25_UI_BROWSER_COOKIE_POLICY_NOT_OBSERVED');
    checkpoint('CHECK_BOUNDED_STABLE_ERROR');
    const stableNavigations = navigations;
    await new Promise(resolve => setTimeout(resolve, 1000));
    assert(navigations === stableNavigations && navigations > 0 && navigations <= 12
      && new URL(page.url()).origin === authOrigin && await page.locator('#error-message').isVisible(),
    'PHASE25_UI_COOKIE_RESTRICTION_REDIRECT_LOOP');
    checkpoint('CHECK_NO_URL_OR_STORAGE_CREDENTIAL_FALLBACK');
    const fallback = await page.evaluate(credentialFallbackFacts, { sensitiveValues });
    checkpoint('CHECK_DENIED_PAGE_STORAGE_RESULT', fallback);
    assert.equal(fallback.urlCredentialFields, 0, 'PHASE25_UI_COOKIE_URL_CREDENTIAL_FALLBACK');
    assert(fallback.storageCredentialEntries === 0 && fallback.storageReadFailures === 0,
      'PHASE25_UI_COOKIE_STORAGE_CREDENTIAL_FALLBACK');
    checkpoint('CHECK_ANONYMOUS_CONSOLE_SESSION');
    const consoleProbe = observePage(await context.newPage());
    await consoleProbe.goto(`${consoleOrigin}/auth/config`, { waitUntil: 'domcontentloaded' });
    const consoleStatus = await consoleProbe.evaluate(() => fetch('/auth/session', { credentials: 'same-origin', cache: 'no-store',
      redirect: 'error', signal: AbortSignal.timeout(10000) }).then(response => response.status));
    assert.equal(consoleStatus, 401, 'PHASE25_UI_BLOCKED_COOKIE_BFF_AUTHENTICATED');
    checkpoint('COMPLETE', await diagnostic());
    return { browserCookiePolicy: 'BLOCK_ALL', actualBrowserRejection: 'UserPreferences', interactionDeniedStatus: 403,
      entryLoginFormRendered: false, preauthStatus: 200, loginDeniedStatus: 403, ssoSessionStatus: 401,
      consoleSessionStatus: 401, cookiesStored: 0, loginCookieSent: false,
      navigationCount: navigations, stableErrorObservedMilliseconds: 1000, responseInterception: false,
      deniedPageCredentialFallback: fallback };
  } catch (error) {
    checkpoint('FAILED', await diagnostic());
    throw error;
  } finally {
    await cdp.detach().catch(() => {});
  }
}
