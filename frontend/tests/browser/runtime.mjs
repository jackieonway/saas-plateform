import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const browserPackage = path.dirname(fileURLToPath(import.meta.url));
const projectCache = path.join(browserPackage, '.browsers');
const requestedCache = process.env.PLAYWRIGHT_BROWSERS_PATH;
assert(!requestedCache || path.resolve(requestedCache) === projectCache ||
  (process.platform === 'linux' && requestedCache === '/opt/ci/browser/.browsers'),
  'PLAYWRIGHT_BROWSERS_PATH must name the project cache or the fixed CI image browser layer');
export const browserCache = requestedCache || projectCache;
// Always resolve both the dependency and downloaded browser from this repository.
process.env.PLAYWRIGHT_BROWSERS_PATH = browserCache;
const require = createRequire(import.meta.url);
export const playwright = require('playwright');
export const playwrightVersion = require('playwright/package.json').version;
export const socketServer = require('socket.io').Server;
export const browserName = process.env.SAAS_TEST_BROWSER || 'chromium';
export const supportedBrowsers = ['chromium', 'chrome', 'msedge', 'firefox', 'webkit'];
assert(supportedBrowsers.includes(browserName), `Unknown SAAS_TEST_BROWSER: ${browserName}`);
const engine = browserName === 'firefox' ? playwright.firefox : browserName === 'webkit' ? playwright.webkit : playwright.chromium;
export async function launchBrowser() {
  const browser = await engine.launch({
    headless: true,
    ...(browserName === 'chrome' || browserName === 'msedge' ? { channel: browserName } : {}),
    ...(engine === playwright.chromium ? { args: ['--no-proxy-server'] } : {}),
  });
  console.log(`BROWSER ${JSON.stringify({ product: browserName, version: await browser.version(), playwright: playwrightVersion })}`);
  return browser;
}
