import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const folder = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const args = process.argv.slice(2);
assert(args.some((value) => ['chromium', 'chrome', 'msedge', 'firefox', 'webkit'].includes(value)) &&
  args.every((value) => ['--with-deps', 'chromium', 'chrome', 'msedge', 'firefox', 'webkit'].includes(value)),
  'Usage: node tests/browser/install.mjs [--with-deps] chromium [chrome msedge firefox webkit]');
const child = spawn(process.execPath, [path.join(path.dirname(require.resolve('playwright/package.json')), 'cli.js'), 'install', ...args], {
  stdio: 'inherit', windowsHide: true,
  env: { ...process.env, PLAYWRIGHT_BROWSERS_PATH: path.join(folder, '.browsers') },
});
child.on('error', (error) => { console.error(error.message); process.exitCode = 1; });
child.on('exit', (code) => { process.exitCode = code ?? 1; });
