/** Current frontend candidate builds and component tests, without live dist writes.
 * Dependencies are the caller's already installed, user-owned locked packages.
 * Output is a new task-owned temporary workspace; only this workspace is written.
 */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';

const script = fileURLToPath(import.meta.url);
const workspace = path.resolve(path.dirname(script), '../../../..');
const args = Object.fromEntries(process.argv.slice(2).map((v, i, all) => v.startsWith('--') ? [v, all[i + 1]] : []).filter(Boolean));
const output = path.resolve(args['--output'] ?? '');
assert(/^phase-25-frontend-components(?:-[a-z0-9-]+)?\.json$/.test(path.basename(output)), 'SAFE_REPORT_REQUIRED');
assert.equal(process.versions.node, '24.22.1', 'PINNED_NODE_REQUIRED');
const digest = value => createHash('sha256').update(value).digest('hex');
const projectNames = [
  'saas-plateform/frontend', 'saas-customer-portal/frontend',
  'saas-stores/frontend', 'saas-work-orders/frontend', 'saas-micro-app-sdk',
  'saas-examples/websocket', 'saas-examples/micro-vue',
  'saas-examples/micro-jquery', 'saas-examples/report-center', 'saas-examples/oidc-rp',
];
const selectedProjects = args['--projects'] ? args['--projects'].split(',') : projectNames;
assert(selectedProjects.length && selectedProjects.every(name => projectNames.includes(name)), 'UNKNOWN_PROJECT');
const npmCli = args['--npm-cli'] ? path.resolve(args['--npm-cli']) : null;
if (npmCli) assert.equal(JSON.parse(fs.readFileSync(path.resolve(path.dirname(npmCli), '../package.json'))).version, '11.16.0', 'PINNED_NPM_REQUIRED');
const skippedDirectories = new Set(['.git', 'node_modules', 'dist', '.browsers', '.cache', 'coverage', 'artifacts']);
const skippedFile = name => /^\.env(?:\.|$)/.test(name) || /\.tsbuildinfo$/.test(name);
const inputs = {};
const temporaryParent = fs.realpathSync(os.tmpdir());
const temporary = fs.mkdtempSync(path.join(temporaryParent, 'rust-saas-phase25-frontends-'));
fs.writeFileSync(path.join(temporary, 'OWNER'), 'rust-saas-unified-auth-phase25-frontends/1\n', { mode: 0o600 });
function sourceFiles(relative, explicit = false) {
  const root = path.join(workspace, relative), found = [];
  function walk(current, rel) {
    const info = fs.lstatSync(current);
    assert(!info.isSymbolicLink(), 'SOURCE_SYMLINK_REJECTED');
    if (info.isDirectory()) {
      for (const entry of fs.readdirSync(current).sort()) {
        if ((!explicit && skippedDirectories.has(entry)) || skippedFile(entry)) continue;
        walk(path.join(current, entry), `${rel}/${entry}`);
      }
    } else if (info.isFile()) found.push(rel);
  }
  walk(root, relative);
  return found;
}
function copyFiles(files) {
  for (const relative of files) {
    const bytes = fs.readFileSync(path.join(workspace, relative));
    inputs[relative] = digest(bytes);
    const target = path.join(temporary, relative);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, bytes, { mode: 0o600 });
  }
}
for (const project of projectNames) copyFiles(sourceFiles(project));
copyFiles(sourceFiles('saas-stores/artifacts', true));
copyFiles(sourceFiles('saas-examples/scripts'));
for (const file of ['.node-version', '.nvmrc', '.npmrc', 'dependencies.lock.json']) {
  copyFiles([`saas-examples/${file}`]);
}
const dependencies = [];
for (const project of projectNames) {
  const modules = path.join(workspace, project, 'node_modules');
  assert(fs.lstatSync(modules).isDirectory() && !fs.lstatSync(modules).isSymbolicLink(), 'NORMAL_DEPENDENCY_DIRECTORY_REQUIRED');
  const installedLock = path.join(modules, '.package-lock.json');
  dependencies.push({ project, installation: modules, symbolicLink: false,
    rootLockSha256: digest(fs.readFileSync(path.join(workspace, project, 'package-lock.json'))),
    installedLockSha256: fs.existsSync(installedLock) ? digest(fs.readFileSync(installedLock)) : null,
    trust: 'Existing normal Windows directory owned by the invoking user, verified before this runner; no npm install or lifecycle scripts.' });
  fs.symlinkSync(modules, path.join(temporary, project, 'node_modules'), process.platform === 'win32' ? 'junction' : 'dir');
}
const sourceManifest = Object.fromEntries(Object.entries(inputs).sort(([a], [b]) => a.localeCompare(b, 'en')));
const report = { contractVersion: 'unified-auth-phase25-frontend-components/1',
  startedAtUtc: new Date().toISOString(), status: 'RUNNING', nodeVersion: process.version,
  runnerSha256: digest(fs.readFileSync(script)), temporaryWorkspace: temporary,
  selectedProjects, npmCli, npmVersion: npmCli ? '11.16.0' : '11.19.1',
  liveDistChanged: false, dockerCreated: 0, dockerReused: 0, databaseUsed: false,
  evidenceLayer: 'Current source component tests, fixture HTTPS peers and production bundles; no real issuer/browser or deployment claim.',
  sourceInputs: sourceManifest, sourceInputSetSha256: digest(JSON.stringify(sourceManifest)),
  sourceInputSetHashEncoding: 'SHA256 of UTF-8 compact JSON with keys sorted using en localeCompare',
  dependencies, checks: [], builds: [], cleanup: { status: 'RETAINED_FOR_CANDIDATE_FREEZE',
    owner: 'rust-saas-unified-auth-phase25-frontends/1',
    scope: 'New temporary workspace and logs only. Existing node_modules, dist and source are outside cleanup scope.' } };
const writeReport = () => fs.writeFileSync(output, JSON.stringify(report, null, 2) + '\n');
writeReport();
async function run(project, name, argumentsList, { liveSource = false } = {}) {
  const number = report.checks.length + 1;
  const logPath = path.join(temporary, `${String(number).padStart(2, '0')}-${name}.log`);
  const log = fs.openSync(logPath, 'wx', 0o600);
  const startedAtUtc = new Date().toISOString();
  const cwd = path.join(liveSource ? workspace : temporary, project);
  const cleanEnvironment = { ...process.env, npm_config_user_agent: `npm/${report.npmVersion} node/v24.22.1 win32 x64`, ...(npmCli ? { npm_execpath: npmCli } : {}) };
  for (const key of Object.keys(cleanEnvironment)) {
    if (/^(?:VITE_|STORES_|WORK_ORDERS_|OIDC_|RP_|DATABASE_URL|BACKEND_PROXY_TARGET|CHAT_ENDPOINT)/.test(key)) delete cleanEnvironment[key];
  }
  const result = await new Promise(resolve => {
    const child = spawn(process.execPath, argumentsList, { cwd, env: cleanEnvironment,
      windowsHide: true, stdio: ['ignore', log, log] });
    const timeout = setTimeout(() => child.kill(), 240000);
    child.once('error', () => { clearTimeout(timeout); resolve({ exitCode: null, error: 'SPAWN_FAILED' }); });
    child.once('exit', (code, signal) => { clearTimeout(timeout); resolve({ exitCode: code, signal }); });
  });
  fs.closeSync(log);
  const bytes = fs.readFileSync(logPath), text = bytes.toString('utf8');
  const count = key => {
    const matches = [...text.matchAll(new RegExp(`^(?:#|ℹ) ${key} (\\d+)\\s*$`, 'gm'))];
    return matches.length ? Number(matches.at(-1)[1]) : null;
  };
  const check = { project, name, command: [process.execPath, ...argumentsList],
    cwd, liveSourceForNonTemporaryOutputGuard: liveSource, startedAtUtc, finishedAtUtc: new Date().toISOString(),
    ...result, status: result.exitCode === 0 ? 'PASS' : 'FAIL',
    logSha256: digest(bytes), logLogicalBytes: bytes.length,
    testCounts: count('tests') === null ? null : { tests: count('tests'), pass: count('pass'),
      fail: count('fail'), cancelled: count('cancelled'), skipped: count('skipped'), todo: count('todo') } };
  report.checks.push(check); writeReport();
  console.log(`PHASE25_FRONTEND ${project} ${name} ${check.status}`);
  return check.status === 'PASS';
}
const tsc = ['node_modules/typescript/bin/tsc', '--noEmit'];
const vite = ['node_modules/vite/bin/vite.js', 'build'];
for (const project of selectedProjects) {
  const pkg = JSON.parse(fs.readFileSync(path.join(temporary, project, 'package.json')));
  if (project === 'saas-plateform/frontend') {
    await run(project, 'tests', pkg.scripts.test.split(' ').slice(1));
    await run(project, 'typecheck', [...tsc, '-p', 'tsconfig.json']);
    await run(project, 'lint', ['node_modules/eslint/bin/eslint.js', 'src', 'vite.config.ts']);
    await run(project, 'sdk-check', ['scripts/check-sdk.mjs']);
    if (await run(project, 'build', vite)) await run(project, 'delivery-check', ['scripts/check-qiankun-delivery.mjs', '--dist', 'dist']);
  } else if (project === 'saas-customer-portal/frontend') {
    await run(project, 'tests', ['--test', 'tests/*.test.mjs']);
    await run(project, 'typecheck', tsc);
    await run(project, 'lint', ['node_modules/eslint/bin/eslint.js', 'src', 'tests', 'vite.config.ts']);
    await run(project, 'build', vite);
  } else if (project === 'saas-stores/frontend') {
    await run(project, 'tests', ['scripts/test.mjs'], { liveSource: true });
    await run(project, 'build', ['scripts/build.mjs', '--allowed-host-origin', 'https://console.saas.test']);
  } else if (project === 'saas-work-orders/frontend') {
    await run(project, 'tests', ['--test', 'tests/*.mjs'], { liveSource: true });
    await run(project, 'build', ['scripts/build.mjs', '--allowed-host-origin', 'https://console.saas.test']);
  } else if (project === 'saas-micro-app-sdk') {
    await run(project, 'tests', ['--test', 'tests/*.test.mjs']);
    await run(project, 'generated-byte-check', ['tools/build.mjs', '--check']);
  } else if (project === 'saas-examples/oidc-rp') {
    await run(project, 'check', ['scripts/check.mjs']);
    await run(project, 'tests', ['--test', '--test-concurrency=1', 'tests/*.test.mjs']);
  } else {
    if (pkg.scripts.typecheck || pkg.scripts.build.includes('tsc')) await run(project, 'typecheck', tsc);
    await run(project, 'build', vite);
    await run(project, 'tests', pkg.scripts.test.startsWith('npm run build')
      ? ['--test', 'tests/*.test.mjs'] : pkg.scripts.test.split(' ').slice(1));
  }
  const dist = path.join(temporary, project, 'dist');
  if (fs.existsSync(dist)) {
    const artifacts = {};
    const walk = directory => {
      for (const item of fs.readdirSync(directory, { withFileTypes: true })) {
        const file = path.join(directory, item.name);
        assert(!fs.lstatSync(file).isSymbolicLink(), 'BUILD_SYMLINK_REJECTED');
        if (item.isDirectory()) walk(file);
        else if (item.isFile()) artifacts[path.relative(dist, file).replaceAll('\\', '/')] = digest(fs.readFileSync(file));
      }
    };
    walk(dist);
    report.builds.push({ project, directory: dist, artifacts }); writeReport();
  }
}
report.sourceInputsUnchanged = Object.entries(sourceManifest).every(([file, hash]) =>
  digest(fs.readFileSync(path.join(workspace, file))) === hash);
report.sourceCopiesUnchanged = Object.entries(sourceManifest).every(([file, hash]) =>
  digest(fs.readFileSync(path.join(temporary, file))) === hash);
report.finishedAtUtc = new Date().toISOString();
report.status = report.checks.every(check => check.status === 'PASS') && report.sourceInputsUnchanged && report.sourceCopiesUnchanged ? 'PASS' : 'FAIL';
writeReport();
console.log(`PHASE25_FRONTEND_RESULT ${report.status} ${report.checks.length} checks ${report.builds.length} builds`);
process.exitCode = report.status === 'PASS' ? 0 : 1;
