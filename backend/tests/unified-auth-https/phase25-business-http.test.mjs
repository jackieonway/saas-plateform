import test, { before, after } from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { fetchBusinessJson, navigateBusinessReadiness, closeBusinessBrowser } from './phase25-business-http.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const repository = path.resolve(here, '../../..');
process.env.PLAYWRIGHT_BROWSERS_PATH = path.join(repository, 'frontend/tests/browser/.browsers');
const { chromium } = createRequire(path.join(repository, 'frontend/tests/browser/package.json'))('playwright');
let browser, app, health, appUrl, healthUrl, context, page, writes = 0;
const receipts = new Map(), observed = [];
function listen(server) {
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => resolve(`http://127.0.0.1:${server.address().port}`));
  });
}
function closeServer(server) { return new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve())); }
before(async () => {
  app = http.createServer(async (request, response) => {
    if (request.url !== '/tickets') { response.writeHead(200, { 'Content-Type': 'text/html' }).end('<title>Owned transport fixture</title>'); return; }
    let bytes = ''; for await (const chunk of request) bytes += chunk;
    const key = request.headers['idempotency-key']; observed.push({ headers: request.headers, bytes });
    response.setHeader('Content-Type', 'application/json');
    if (!key) { response.writeHead(400).end(JSON.stringify({ code: 'MISSING_KEY' })); return; }
    const old = receipts.get(key);
    if (old && old.bytes !== bytes) { response.writeHead(409).end(JSON.stringify({ code: 'IDEMPOTENCY_CONFLICT' })); return; }
    const value = old ?? { bytes, ticket: { id: `owned-fixture-ticket-${++writes}`, ...JSON.parse(bytes) } };
    receipts.set(key, value); response.writeHead(201).end(JSON.stringify(value.ticket));
  });
  health = http.createServer((request, response) => {
    response.writeHead(request.url === '/health/ready' ? 200 : 503, { 'Content-Type': 'application/json' }).end('{}');
  });
  appUrl = await listen(app); healthUrl = await listen(health);
  browser = await chromium.launch({ headless: true });
  context = await browser.newContext({ serviceWorkers: 'block' }); page = await context.newPage();
  await page.goto(appUrl);
});
after(async () => {
  await browser?.close();
  await Promise.all([app, health].filter(Boolean).map(closeServer));
});

test('actual Chromium request carries one stable idempotency key; replay preserves one write and changed input conflicts', async () => {
  const input = { endpoint: '/tickets', method: 'POST', body: { title: 'Owned fixture ticket' },
    bearer: 'public-fixture-bearer', csrf: 'public-fixture-csrf', idempotencyKey: 'owned-fixture-key' };
  const created = await page.evaluate(fetchBusinessJson, input);
  const replayed = await page.evaluate(fetchBusinessJson, input);
  assert.equal(created.status, 201); assert.deepEqual(replayed, created); assert.equal(writes, 1);
  const conflict = await page.evaluate(fetchBusinessJson, { ...input, body: { title: 'Changed body' } });
  assert.equal(conflict.status, 409); assert.equal(writes, 1);
  const absent = await page.evaluate(fetchBusinessJson, { ...input, idempotencyKey: undefined });
  assert.equal(absent.status, 400); assert.equal(writes, 1);
  for (const request of observed.slice(0, 3)) {
    assert.equal(request.headers['idempotency-key'], 'owned-fixture-key');
    assert.equal(request.headers.authorization, 'Bearer public-fixture-bearer');
    assert.equal(request.headers['x-csrf-token'], 'public-fixture-csrf');
  }
  assert.equal(observed[0].bytes, observed[1].bytes);
});

test('actual Chromium cannot fetch health without CORS; navigation checks health then restores the application origin even on failure', async () => {
  await assert.rejects(page.evaluate(url => fetch(url), `${healthUrl}/health/ready`));
  await navigateBusinessReadiness(page, `${healthUrl}/health/ready`, `${appUrl}/`);
  assert.equal(page.url(), `${appUrl}/`);
  await assert.rejects(navigateBusinessReadiness(page, `${healthUrl}/unhealthy`, `${appUrl}/`), /PHASE25_BUSINESS_NOTIFICATION_READY_FAILED/);
  assert.equal(page.url(), `${appUrl}/`);
});

test('cleanup retains failed closure and live-connection facts instead of claiming success', async () => {
  const stillOpen = { close: async () => { throw new Error('fixture-close-failed'); }, pages: () => [{}] };
  const stillConnected = { close: async () => { throw new Error('fixture-browser-failed'); }, isConnected: () => true };
  assert.deepEqual(await closeBusinessBrowser(stillConnected, [stillOpen]), {
    contextsClosed: false, browserClosed: false, failures: 2,
  });
  const closedDespiteError = { close: async () => { throw new Error('fixture-ack-failed'); }, isConnected: () => false };
  assert.deepEqual(await closeBusinessBrowser(closedDespiteError, []), {
    contextsClosed: true, browserClosed: true, failures: 1,
  });
});

test('owned actual Chromium context and browser are verified closed', async () => {
  assert.deepEqual(await closeBusinessBrowser(browser, [context]), {
    contextsClosed: true, browserClosed: true, failures: 0,
  });
});
