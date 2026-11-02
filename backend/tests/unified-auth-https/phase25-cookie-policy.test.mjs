import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { withCookiesBlocked, checkBlockedCookieLogin, credentialFallbackFacts } from './phase25-cookie-policy.mjs';

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
process.env.PLAYWRIGHT_BROWSERS_PATH = path.join(repository, 'frontend/tests/browser/.browsers');
const require = createRequire(path.join(repository, 'frontend/tests/browser/package.json'));
const { chromium } = require('playwright');
const prefix = 'rust-saas-phase25-cookie-policy-';
const profiles = () => fs.readdirSync(os.tmpdir()).filter(value => value.startsWith(prefix)).sort();

test('Chromium BLOCK setting rejects HTTP Set-Cookie and document cookies without interception; profile closes on failure', async () => {
  const before = profiles();
  const requests = [];
  const server = http.createServer((request, response) => {
    requests.push({ path: request.url, cookie: request.headers.cookie });
    response.writeHead(200, { 'Content-Type': 'text/html', 'Set-Cookie': 'phase25_policy_probe=public-test-value; Path=/; SameSite=Lax' });
    response.end('<!doctype html><title>Public cookie policy test</title>');
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    const origin = `http://127.0.0.1:${server.address().port}`;
    await assert.rejects(withCookiesBlocked(chromium, {}, async context => {
      const page = await context.newPage();
      const cdp = await context.newCDPSession(page), blocked = [];
      cdp.on('Network.responseReceivedExtraInfo', event => {
        for (const value of event.blockedCookies ?? []) blocked.push(...value.blockedReasons);
      });
      await cdp.send('Network.enable');
      await page.goto(origin);
      assert.equal(await page.evaluate(() => { document.cookie = 'phase25_script_probe=public-script-value'; return document.cookie; }), '');
      await page.evaluate(() => fetch('/second', { credentials: 'include' }).then(response => response.text()));
      assert.equal((await context.cookies()).length, 0);
      assert(requests.length >= 2 && requests.every(value => !value.cookie));
      assert(blocked.includes('UserPreferences'), 'actual Chromium cookie rejection was not observed');
      throw new Error('PUBLIC_ACTION_FAILURE');
    }), { message: 'PUBLIC_ACTION_FAILURE' });
    assert.deepEqual(profiles(), before);
  } finally {
    await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  }
});

test('blocked interaction cookie is an explicit early denial, while direct browser preauth/login and sessions still fail closed', async () => {
  const before = profiles();
  const interaction = '/oauth/interaction/019b0000-0000-7000-8000-000000000001';
  let interactionStatus = 403, loginCalls = 0;
  const requests = [], stages = [];
  const server = http.createServer((request, response) => {
    const pathname = new URL(request.url, 'http://fixture.invalid').pathname;
    requests.push({ pathname, cookiePresent: Boolean(request.headers.cookie) });
    if (pathname === '/auth/login') {
      response.writeHead(302, { Location: '/oauth/login?flow=public-fixture-flow', 'Set-Cookie': 'flow_cookie=public-flow-binding; Path=/; SameSite=Lax' });
      response.end(); return;
    }
    if (pathname === '/oauth/login') {
      response.writeHead(200, { 'Content-Type': 'text/html' });
      response.end(`<!doctype html><main id="auth-content" data-screen="RESTART_REQUIRED"><div id="error-message" hidden>Cookie binding rejected</div><form id="login-form" hidden></form></main><script>
        fetch('${interaction}', {credentials:'same-origin'}).then(response => {
          document.querySelector(response.status === 403 ? '#error-message' : '#login-form').hidden = false;
        });</script>`); return;
    }
    if (pathname === interaction) {
      response.writeHead(interactionStatus, { 'Content-Type': 'application/json' }); response.end('{}'); return;
    }
    if (pathname === '/api/sso/preauth') {
      response.writeHead(200, { 'Content-Type': 'application/json', 'Set-Cookie': 'preauth_cookie=public-preauth-binding; Path=/; SameSite=Lax' });
      response.end(JSON.stringify({ csrfToken: 'public-component-csrf-value' })); return;
    }
    if (pathname === '/api/sso/login') {
      loginCalls++;
      request.resume();
      response.writeHead(request.headers.cookie ? 200 : 403, { 'Content-Type': 'application/json' }); response.end('{}'); return;
    }
    const status = ['/api/sso/session', '/auth/session'].includes(pathname) ? 401 : pathname === '/auth/config' ? 200 : 404;
    response.writeHead(status, { 'Content-Type': 'application/json' }); response.end('{}');
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    const origin = `http://127.0.0.1:${server.address().port}`;
    const options = { authOrigin: origin, consoleOrigin: origin, account: { username: 'public-user', password: 'public-test-password' },
      progress: (stage, facts) => stages.push({ stage, ...facts }) };
    const result = await withCookiesBlocked(chromium, {}, context => checkBlockedCookieLogin(context, options));
    assert.equal(result.interactionDeniedStatus, 403);
    assert.equal(result.entryLoginFormRendered, false);
    assert.equal(result.loginDeniedStatus, 403);
    assert.equal(result.ssoSessionStatus, 401);
    assert.equal(result.consoleSessionStatus, 401);
    assert(result.navigationCount > 0 && result.navigationCount <= 12);
    assert.equal(loginCalls, 1);
    assert(requests.every(request => !request.cookiePresent));
    assert.equal(stages.at(-1).stage, 'COMPLETE');
    assert(!JSON.stringify(stages).includes('public-user') && !JSON.stringify(stages).includes('flow='));
    interactionStatus = 200;
    await assert.rejects(withCookiesBlocked(chromium, {}, context => checkBlockedCookieLogin(context, options)),
      { message: 'PHASE25_UI_BLOCKED_COOKIE_FLOW_GATE_NOT_REJECTED' });
    assert.equal(stages.at(-1).stage, 'FAILED');
    assert.equal(stages.at(-1).loginVisible, true);
    assert.equal(stages.at(-1).interactionStatus, 200);
    assert.equal(loginCalls, 1);
    assert.deepEqual(profiles(), before);
  } finally {
    await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  }
});

test('same private Chromium profile permits cookies only after explicit policy restoration and detects URL and storage credential fallbacks without exporting values', async () => {
  const before = profiles();
  const server = http.createServer((_request, response) => {
    response.writeHead(200, { 'Content-Type': 'text/html',
      'Set-Cookie': 'phase25_recovery_probe=public-fixture-cookie; Path=/; SameSite=Lax' });
    response.end('<!doctype html><title>Public policy restoration probe</title>');
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    const origin = `http://127.0.0.1:${server.address().port}`;
    let blockedContext;
    const result = await withCookiesBlocked(chromium, {}, async context => {
      blockedContext = context;
      const page = await context.newPage(); await page.goto(origin);
      assert.equal((await context.cookies()).length, 0);
      const facts = await page.evaluate(credentialFallbackFacts);
      assert.equal(facts.urlCredentialFields, 0);
      assert.equal(facts.storageCredentialEntries, 0);
      assert.equal(facts.storageReadFailures, 0);
      return { blockedCookies: 0, blockedCredentialFallbackEntries: 0 };
    }, async context => {
      assert.notEqual(context, blockedContext);
      assert.equal(blockedContext.pages().length, 0, 'blocked context was not closed before policy changed');
      const page = await context.newPage(); await page.goto(origin);
      assert((await context.cookies()).some(cookie => cookie.name === 'phase25_recovery_probe'));
      const facts = [];
      facts.push(await page.evaluate(credentialFallbackFacts));
      await page.evaluate(() => {
        localStorage.setItem('fallback', JSON.stringify({ accessToken: 'public-component-opaque-value' }));
        sessionStorage.setItem('theme', 'public-component-private-canary');
      });
      facts.push(await page.evaluate(credentialFallbackFacts, { sensitiveValues: ['public-component-private-canary'] }));
      assert.equal(facts.at(-1).storageCredentialEntries, 2);
      assert.equal(facts.at(-1).storageReadFailures, 0);
      await page.evaluate(() => {
        localStorage.clear(); sessionStorage.clear();
        history.replaceState(null, '', '?code=public-component-code#access_token=public-component-fragment');
      });
      facts.push(await page.evaluate(credentialFallbackFacts));
      assert.equal(facts.at(-1).urlCredentialFields, 2);
      await page.evaluate(() => { history.replaceState(null, '', location.pathname); });
      const restored = await page.evaluate(credentialFallbackFacts);
      assert.equal(restored.urlCredentialFields, 0);
      assert.equal(restored.storageCredentialEntries, 0);
      assert.equal(restored.storageReadFailures, 0);
      assert.equal(restored.storageBlockedByBrowser, 0);
      assert(!JSON.stringify(facts).includes('public-component-'), 'fallback facts exposed a captured value');
      return { cookiesAllowed: true, storageAllowed: true, negativeFallbacksDetected: 4 };
    });
    assert.equal(result.blockedCookies, 0);
    assert.equal(result.samePrivateProfileRestored, true);
    assert.equal(result.recoveryAfterPolicyChange.cookiesAllowed, true);
    assert.equal(result.recoveryAfterPolicyChange.negativeFallbacksDetected, 4);
    assert.deepEqual(profiles(), before);
  } finally {
    await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  }
});
