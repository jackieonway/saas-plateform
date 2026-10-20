#!/usr/bin/env node
// Native release builder. Node is a build tool; the only packaged API is Rust.
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { chmodSync, copyFileSync, cpSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, renameSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const publicEnvTemplates = ['deploy/compose.env.example', 'deploy/compose.legacy.env.example']
const deploymentFiles = ['deploy/README.md', 'deploy/compose.zip.yml', 'deploy/compose.legacy.yml', ...publicEnvTemplates, 'compose.legacy.yml']
export const sourceEntries = [
  '.nvmrc', 'rust-toolchain.toml', '.dockerignore', 'docker-compose.yml',
  ...deploymentFiles,
  'backend/Cargo.toml', 'backend/Cargo.lock', 'backend/Dockerfile', 'backend/src', 'backend/assets', 'backend/migrations',
  'crates/operational-telemetry/Cargo.toml', 'crates/operational-telemetry/Cargo.lock', 'crates/operational-telemetry/src',
  'crates/deployment-contract/Cargo.toml', 'crates/deployment-contract/Cargo.lock', 'crates/deployment-contract/src',
  'mini-services/oauth-playground/backend/Cargo.toml', 'mini-services/oauth-playground/backend/Cargo.lock', 'mini-services/oauth-playground/backend/src',
  'frontend/package.json', 'frontend/package-lock.json', 'frontend/.npmrc', 'frontend/tsconfig.json', 'frontend/vite.config.ts',
  'frontend/index.html', 'frontend/Dockerfile', 'frontend/src', 'frontend/public',
  'examples/micro-app-sdk', 'scripts/sync-micro-sdk.mjs',
  'business-apps/work-orders/frontend/vendor/iframe-bridge.js', 'business-apps/work-orders/frontend/vendor/micro-sdk.mjs',
  'business-apps/stores/frontend/vendor/micro-sdk',
  ...['micro-vue', 'micro-jquery'].flatMap(name => [
    `mini-services/${name}/package.json`, `mini-services/${name}/package-lock.json`,
    `mini-services/${name}/vite.config.mjs`, `mini-services/${name}/index.html`, `mini-services/${name}/public`,
  ]),
  ...['package.json', 'package-lock.json', 'tsconfig.json', 'vite.config.ts', 'index.html', 'frontend.tsx', 'src'].map(name => `examples/websocket/${name}`),
  'deploy/gateway', 'deploy/native',
  ...['finalize-build.mjs', 'prepare-history.mjs', 'nginx.conf', 'mime.types', 'README.md', 'history'].map(name => `deploy/static/${name}`),
]
const packages = ['mini-services/micro-vue', 'mini-services/micro-jquery', 'examples/websocket', 'frontend']
const requiredFiles = ['.nvmrc', 'rust-toolchain.toml', 'backend/Cargo.toml', 'backend/Cargo.lock', 'backend/src/main.rs', 'frontend/index.html', 'frontend/.npmrc',
  'crates/operational-telemetry/Cargo.toml', 'crates/operational-telemetry/Cargo.lock', 'crates/operational-telemetry/src/lib.rs',
  'crates/deployment-contract/Cargo.toml', 'crates/deployment-contract/Cargo.lock', 'crates/deployment-contract/src/lib.rs',
  'mini-services/oauth-playground/backend/Cargo.toml', 'mini-services/oauth-playground/backend/Cargo.lock',
  'mini-services/oauth-playground/backend/src/lib.rs', 'mini-services/oauth-playground/backend/src/main.rs',
  'examples/micro-app-sdk/release.json', 'examples/micro-app-sdk/generated/protocol.mjs', 'examples/micro-app-sdk/generated/types.d.ts',
  'scripts/sync-micro-sdk.mjs', 'business-apps/work-orders/frontend/vendor/iframe-bridge.js', 'business-apps/work-orders/frontend/vendor/micro-sdk.mjs',
  'deploy/static/finalize-build.mjs', 'deploy/gateway/policy.mjs', 'deploy/native/render.mjs',
  'deploy/README.md', 'deploy/compose.zip.yml', ...publicEnvTemplates,
  ...packages.flatMap(name => [`${name}/package.json`, `${name}/package-lock.json`])]
const slash = value => value.split(path.sep).join('/')
const sha256 = value => createHash('sha256').update(value).digest('hex')
const inside = (root, value) => value === root || value.startsWith(root + path.sep)
const json = (file, value) => writeFileSync(file, JSON.stringify(value, null, 2) + '\n', { mode: 0o600 })

// Check names before opening files. In particular, never read a private .env to
// decide whether it is safe. Generated compatibility bundles are rebuilt below.
export function excludedSource(relative) {
  const parts = relative.replaceAll('\\', '/').split('/')
  const normalized = relative.replaceAll('\\', '/')
  if (/\.env(?:\..*)?$/i.test(parts.at(-1)) && !publicEnvTemplates.includes(normalized)) return true
  return parts.some(part => /^(?:\.env(?:\..*)?|node_modules|target|dist|\.git|\.next|coverage|secrets?|private|logs?)$/i.test(part)
    || (part === '.npmrc' && normalized !== 'frontend/.npmrc')
    || /\.(?:pem|key|p12|pfx|crt|log|pid|tsbuildinfo)$/i.test(part)
    || /^(?:deployment|runtime|credentials|secrets?)\.(?:env|json|ya?ml|toml|ini|conf|config|txt)$/i.test(part))
    || relative.replaceAll('\\', '/').startsWith('frontend/public/sub-apps/')
    || relative.replaceAll('\\', '/') === 'frontend/public/sub-apps'
}

export function parseArgs(args) {
  const options = {}
  const keys = new Map([['--out-dir', 'outDir'], ['--source-root', 'sourceRoot'], ['--cargo', 'cargo'], ['--npm-cli', 'npmCli']])
  for (let index = 0; index < args.length; index++) {
    if (args[index] === '--help') return { help: true }
    const key = keys.get(args[index])
    if (!key || options[key] || !args[index + 1] || args[index + 1].startsWith('--')) throw new Error('Unknown, duplicate or missing build argument')
    options[key] = args[++index]
  }
  if (!options.outDir || !path.isAbsolute(options.outDir)) throw new Error('--out-dir must be a new absolute directory')
  if (options.sourceRoot && !path.isAbsolute(options.sourceRoot)) throw new Error('--source-root must be absolute')
  if (options.npmCli && !path.isAbsolute(options.npmCli)) throw new Error('--npm-cli must be absolute')
  return options
}

export function buildEnvironment(output, source = process.env) {
  const allowed = ['PATH', 'Path', 'SystemRoot', 'SYSTEMROOT', 'WINDIR', 'COMSPEC', 'ComSpec', 'PATHEXT', 'TEMP', 'TMP',
    'HOME', 'USERPROFILE', 'APPDATA', 'LOCALAPPDATA', 'ProgramFiles', 'ProgramFiles(x86)', 'ProgramW6432', 'CARGO_HOME', 'RUSTUP_HOME', 'LANG', 'LC_ALL']
  const env = Object.fromEntries(allowed.filter(key => source[key]).map(key => [key, source[key]]))
  const pathKey = process.platform === 'win32' && env.Path ? 'Path' : 'PATH'
  env[pathKey] = path.dirname(process.execPath) + path.delimiter + (env[pathKey] || '')
  // npm must not consult the operator's private npmrc or project .env files.
  env.NPM_CONFIG_USERCONFIG = path.join(output, 'work', 'npmrc')
  env.NPM_CONFIG_GLOBALCONFIG = path.join(output, 'work', 'npmrc-global')
  env.NPM_CONFIG_CACHE = path.join(output, 'work', 'npm-cache')
  env.NPM_CONFIG_UPDATE_NOTIFIER = 'false'
  env.CARGO_TARGET_DIR = path.join(output, 'work', 'cargo-target')
  env.CI = 'true'
  env.TZ = 'UTC'
  return env
}

function filesUnder(root, relative, callback) {
  if (excludedSource(relative)) return
  const absolute = path.join(root, relative)
  if (!existsSync(absolute)) return
  const stat = lstatSync(absolute)
  if (stat.isSymbolicLink()) throw new Error(`Source symlink is not allowed: ${slash(relative)}`)
  if (stat.isDirectory()) {
    for (const name of readdirSync(absolute).sort()) filesUnder(root, path.join(relative, name), callback)
  } else if (stat.isFile()) callback(absolute, slash(relative), stat)
  else throw new Error(`Unsupported source file: ${slash(relative)}`)
}

export function createSourceSnapshot(sourceRoot, destination) {
  const files = []
  const seen = new Set()
  for (const entry of sourceEntries) filesUnder(sourceRoot, entry, (absolute, relative) => {
    if (seen.has(relative)) return
    seen.add(relative)
    const content = readFileSync(absolute)
    if (packages.some(directory => relative === `${directory}/package-lock.json`)) validateLockContent(relative, content)
    if (relative === 'frontend/.npmrc') {
      const directives = content.toString('utf8').split(/\r?\n/).map(line => line.trim()).filter(Boolean)
      if (directives.length !== 1 || directives[0] !== 'engine-strict=true') throw new Error('Only public engine-strict=true is allowed in frontend/.npmrc')
    }
    const target = path.join(destination, relative)
    mkdirSync(path.dirname(target), { recursive: true })
    writeFileSync(target, content)
    files.push({ path: relative, bytes: content.length, sha256: sha256(content) })
  })
  for (const file of requiredFiles) if (!seen.has(file)) throw new Error(`Required build input is missing: ${file}`)
  files.sort((a, b) => a.path.localeCompare(b.path, 'en'))
  return { files, inputsSha256: sha256(JSON.stringify(files)) }
}

function sourceRevision(root) {
  const result = spawnSync('git', ['-c', `safe.directory=${slash(root)}`, '-C', root, 'rev-parse', 'HEAD'], { encoding: 'utf8', windowsHide: true, timeout: 10000 })
  if (result.status !== 0 || !/^[0-9a-f]{40,64}$/.test(result.stdout.trim())) throw new Error('A Git source checkout with a valid HEAD is required')
  return result.stdout.trim()
}

function npmEntry(explicit) {
  const candidates = [explicit, path.join(path.dirname(process.execPath), 'node_modules/npm/bin/npm-cli.js'),
    path.resolve(path.dirname(process.execPath), '../lib/node_modules/npm/bin/npm-cli.js')].filter(Boolean)
  const entry = candidates.find(candidate => existsSync(candidate) && lstatSync(candidate).isFile())
  if (!entry) throw new Error('npm CLI not found beside Node; pass --npm-cli with its absolute npm-cli.js path')
  return entry
}

function validateLockContent(relative, content) {
  let lock
  try { lock = JSON.parse(content) } catch { throw new Error(`Invalid npm lock JSON: ${relative}`) }
  if (![2, 3].includes(lock.lockfileVersion)) throw new Error(`Unsupported npm lock version: ${relative}`)
  for (const entry of Object.values(lock.packages || {})) {
    if (!entry.resolved) continue
    let url
    try { url = new URL(entry.resolved) } catch { throw new Error(`Non-registry npm dependency rejected: ${relative}`) }
    if (url.protocol !== 'https:' || url.username || url.password || url.search) throw new Error(`Credentialled or non-HTTPS npm dependency rejected: ${relative}`)
  }
}

function artifactFiles(root) {
  const output = []
  function visit(directory) {
    for (const name of readdirSync(directory).sort()) {
      const file = path.join(directory, name), stat = lstatSync(file)
      if (stat.isSymbolicLink()) throw new Error('Build artifacts cannot contain symlinks')
      if (stat.isDirectory()) visit(file)
      else if (stat.isFile()) output.push({ path: slash(path.relative(root, file)), bytes: stat.size, sha256: sha256(readFileSync(file)) })
    }
  }
  visit(root)
  return output
}

export function buildRelease(options, { runCommand = spawnSync } = {}) {
  const sourceRoot = path.resolve(options.sourceRoot || fileURLToPath(new URL('../..', import.meta.url)))
  if (!options.outDir || !path.isAbsolute(options.outDir)) throw new Error('--out-dir must be a new absolute directory')
  const outDir = path.resolve(options.outDir)
  if (existsSync(outDir)) throw new Error('Output directory already exists; choose a new release path')
  if (inside(outDir, sourceRoot) || sourceEntries.some(entry => inside(path.resolve(sourceRoot, entry), outDir))) throw new Error('Output directory overlaps source inputs')
  const sourceSha = sourceRevision(sourceRoot)
  const npmCli = npmEntry(options.npmCli)
  mkdirSync(path.dirname(outDir), { recursive: true })
  mkdirSync(outDir, { mode: 0o700 })
  mkdirSync(path.join(outDir, 'work'), { mode: 0o700 })
  mkdirSync(path.join(outDir, 'logs'), { mode: 0o700 })
  writeFileSync(path.join(outDir, 'work', 'npmrc'), 'engine-strict=true\n', { mode: 0o600 })
  writeFileSync(path.join(outDir, 'work', 'npmrc-global'), '', { mode: 0o600 })
  const snapshot = path.join(outDir, 'source'), staging = path.join(outDir, 'work', 'artifacts')
  const env = buildEnvironment(outDir)
  const commands = []
  let result = { schemaVersion: '1.0', status: 'BUILDING', sourceSha, startedAt: new Date().toISOString(),
    productionRuntime: 'Rust API and Nginx static frontend', nodeRuntimePackaged: false,
    excludedLegacyServices: ['business-apps/work-orders/backend (Phase 17)'],
    separatelyBuiltRustServices: ['mini-services/notification-ws (standalone Rust crate and scripts/ci/notification-image.mjs image gate)'] }
  const step = (name, command, args, cwd = snapshot) => {
    const execution = runCommand(command, args, { cwd, env, encoding: 'utf8', windowsHide: true, timeout: 30 * 60 * 1000, maxBuffer: 64 * 1024 * 1024 })
    const logfile = `logs/${String(commands.length + 1).padStart(2, '0')}-${name}.log`
    writeFileSync(path.join(outDir, logfile), String(execution.stdout || '') + String(execution.stderr || ''), { mode: 0o600 })
    commands.push({ name, executable: path.basename(command), args, cwd: slash(path.relative(outDir, cwd)), exitCode: execution.status, log: logfile })
    if (execution.error || execution.status !== 0) throw new Error(`Build step ${name} failed; inspect ${logfile}`)
    return String(execution.stdout || '').trim()
  }
  try {
    const input = createSourceSnapshot(sourceRoot, snapshot)
    result = { ...result, buildId: `${sourceSha.slice(0, 12)}-${input.inputsSha256.slice(0, 16)}`, inputsSha256: input.inputsSha256 }
    json(path.join(outDir, 'source-manifest.json'), { schemaVersion: '1.0', sourceSha, buildId: result.buildId,
      revisionMeaning: 'sourceSha identifies HEAD; files and inputsSha256 identify the current working-tree snapshot, including uncommitted changes', ...input })
    const node = step('node-version', process.execPath, ['--version'])
    const npm = step('npm-version', process.execPath, [npmCli, '--version'])
    const cargo = step('cargo-version', options.cargo || 'cargo', ['--version'])
    if (!/^v24\.(?:1[89]|[2-9]\d)\./.test(node) || !/^11\.(?:1[6-9]|[2-9]\d)\./.test(npm) || !/^cargo 1\.97\.0\b/.test(cargo)) throw new Error('Required tools: Node 24.18+ within 24.x, npm 11.16+ within 11.x, Cargo 1.97.0')
    result.tools = { node, npm, cargo, platform: process.platform, architecture: process.arch }
    for (const directory of packages) step(`install-${path.basename(directory)}`, process.execPath, [npmCli, 'ci', '--ignore-scripts', '--no-audit', '--no-fund'], path.join(snapshot, directory))
    step('sdk-drift', process.execPath, [path.join(snapshot, 'scripts/sync-micro-sdk.mjs'), '--check'])
    for (const directory of packages.slice(0, -1)) step(`build-${path.basename(directory)}`, process.execPath, [npmCli, 'run', 'build'], path.join(snapshot, directory))
    for (const [name, directory] of [['vue', packages[0]], ['jquery', packages[1]]]) {
      const dist = path.join(snapshot, directory, 'dist')
      if (!existsSync(path.join(dist, 'index.html'))) throw new Error(`Static build is missing index.html: ${directory}`)
      cpSync(dist, path.join(snapshot, 'frontend/public/sub-apps', name), { recursive: true, errorOnExist: true, force: false })
    }
    step('build-frontend', process.execPath, [npmCli, 'run', 'build'], path.join(snapshot, 'frontend'))
    step('finalize-frontend', process.execPath, [path.join(snapshot, 'deploy/static/finalize-build.mjs'),
      '--dist', path.join(snapshot, 'frontend/dist'), '--source', path.join(snapshot, 'frontend'),
      '--history', path.join(snapshot, 'deploy/static/history'), '--source-revision', sourceSha, '--build-id', result.buildId])
    step('build-platform', options.cargo || 'cargo', ['build', '--release', '--locked', '--bin', 'saas-platform-backend'], path.join(snapshot, 'backend'))
    const binaryName = `saas-platform-backend${process.platform === 'win32' ? '.exe' : ''}`
    const binary = path.join(env.CARGO_TARGET_DIR, 'release', binaryName)
    if (!existsSync(binary) || !lstatSync(binary).isFile() || lstatSync(binary).size === 0) throw new Error('Rust release binary was not produced')
    mkdirSync(path.join(staging, 'platform'), { recursive: true })
    copyFileSync(binary, path.join(staging, 'platform', binaryName))
    chmodSync(path.join(staging, 'platform', binaryName), 0o755)
    for (const [directory, destination] of [[packages[0], 'examples/vue'], [packages[1], 'examples/jquery'], [packages[2], 'examples/websocket'], ['frontend', 'frontend']]) {
      const dist = path.join(snapshot, directory, 'dist')
      if (!existsSync(path.join(dist, 'index.html'))) throw new Error(`Static build is missing index.html: ${directory}`)
      cpSync(dist, path.join(staging, destination), { recursive: true, errorOnExist: true, force: false })
    }
    // Deployment templates only. No runtime environment, service credentials,
    // source tree, build cache, Node dependency or old Node API enters artifacts.
    for (const directory of ['deploy/gateway', 'deploy/native', 'deploy/static']) {
      if (existsSync(path.join(snapshot, directory))) cpSync(path.join(snapshot, directory), path.join(staging, directory), { recursive: true })
    }
    if (existsSync(path.join(snapshot, 'docker-compose.yml'))) copyFileSync(path.join(snapshot, 'docker-compose.yml'), path.join(staging, 'docker-compose.yml'))
    for (const file of deploymentFiles) {
      if (!existsSync(path.join(snapshot, file))) continue
      mkdirSync(path.dirname(path.join(staging, file)), { recursive: true })
      copyFileSync(path.join(snapshot, file), path.join(staging, file))
    }
    const artifacts = artifactFiles(staging)
    renameSync(staging, path.join(outDir, 'artifacts'))
    result = { ...result, status: 'SUCCEEDED', completedAt: new Date().toISOString(), commands, artifacts }
    json(path.join(outDir, 'build-result.json'), result)
    return result
  } catch (error) {
    json(path.join(outDir, 'build-result.json'), { ...result, status: 'FAILED', completedAt: new Date().toISOString(), commands, error: error.message })
    throw error
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const options = parseArgs(process.argv.slice(2))
    if (options.help) console.log('Usage: node deploy/native/build.mjs --out-dir <new-absolute-directory> [--cargo <path>] [--npm-cli <absolute-npm-cli.js>] [--source-root <absolute-checkout>]')
    else { const result = buildRelease(options); console.log(JSON.stringify({ status: result.status, buildId: result.buildId, sourceSha: result.sourceSha, artifactFiles: result.artifacts.length })) }
  } catch (error) { console.error(`[native-build] ${error.message}`); process.exitCode = 1 }
}
