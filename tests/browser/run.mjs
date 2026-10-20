import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { mandatorySuites } from './manifest.mjs';
import { classifyBrowserResult } from './results.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const args = process.argv.slice(2);
let product = 'chromium', reportPath;
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--browser') product = args[++i];
  else if (args[i] === '--report') reportPath = path.resolve(args[++i]);
  else throw new Error(`Unknown browser runner argument: ${args[i]}`);
}
assert(['chromium', 'chrome', 'msedge', 'firefox', 'webkit'].includes(product), 'Unsupported browser product');
if (reportPath) assert(!existsSync(reportPath), 'Browser report already exists; choose a fresh output path');
process.env.SAAS_TEST_BROWSER = product;
const report = {
  schemaVersion: 1, product, startedAt: new Date().toISOString(),
  node: process.versions.node, platform: process.platform, architecture: process.arch,
  lockSha256: createHash('sha256').update(readFileSync(path.join(root, 'tests/browser/package-lock.json'))).digest('hex'),
  scopes: ['real host/example frontends', 'isolated HTTP/Socket.IO fixtures'],
  suites: [], totals: { pass: 0, fail: 0, skip: 0 },
};
const redact = (text) => text.replace(/Bearer\s+[^\s"']+/g, 'Bearer [redacted]')
  .replace(/(?:postgres(?:ql)?:\/\/)[^\s"']+/g, '[database-url-redacted]');
try {
  const { launchBrowser, browserCache, playwrightVersion } = await import('./runtime.mjs');
  report.playwrightVersion = playwrightVersion;
  const probe = await launchBrowser();
  report.browserVersion = await probe.version();
  await probe.close();
  // Do not let local selectors or private service settings shrink a mandatory gate.
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) =>
    /^(PATH|PATHEXT|SYSTEMROOT|WINDIR|COMSPEC|TEMP|TMP|TMPDIR|HOME|USERPROFILE|LOCALAPPDATA|PROGRAMFILES|PROGRAMFILES\(X86\)|SYSTEMDRIVE|DISPLAY|XDG_RUNTIME_DIR|CI)$/i.test(key)));
  Object.assign(env, { SAAS_TEST_BROWSER: product, PLAYWRIGHT_BROWSERS_PATH: browserCache });
  for (const suite of mandatorySuites) {
    const started = Date.now();
    let output = '', spawnError;
    const child = spawn(process.execPath, [path.join(root, suite.script)], {
      cwd: path.join(root, suite.cwd || ''), env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
    });
    child.stdout.on('data', (chunk) => { output += chunk; process.stdout.write(redact(String(chunk))); });
    child.stderr.on('data', (chunk) => { output += chunk; process.stderr.write(redact(String(chunk))); });
    child.on('error', (error) => { spawnError = error.message; });
    const timer = setTimeout(() => child.kill('SIGKILL'), 10 * 60_000);
    const code = await new Promise((resolve) => child.once('close', resolve));
    clearTimeout(timer);
    const { pass, skip, fail, reportedFailures } = classifyBrowserResult({ output, exitCode: code,
      expectedCheckGroups: suite.expectedCheckGroups, error: spawnError });
    report.suites.push({ ...suite, elapsedMs: Date.now() - started, exitCode: code,
      pass, fail, skip, reportedFailures, ...(spawnError ? { error: spawnError } : {}) });
    report.totals.pass += pass; report.totals.fail += fail; report.totals.skip += skip;
  }
} catch (error) {
  report.environmentError = redact(error.message);
  report.totals.fail++;
  console.error(report.environmentError);
} finally {
  report.finishedAt = new Date().toISOString();
  if (reportPath) { mkdirSync(path.dirname(reportPath), { recursive: true }); writeFileSync(reportPath, JSON.stringify(report, null, 2) + '\n', { flag: 'wx' }); }
  console.log(`BROWSER_GATE ${JSON.stringify(report)}`);
  if (report.totals.fail || report.totals.skip || report.suites.length !== mandatorySuites.length) process.exitCode = 1;
}
