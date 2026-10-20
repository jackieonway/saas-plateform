import assert from 'node:assert/strict'
import { createHash, X509Certificate } from 'node:crypto'
import { existsSync, lstatSync, mkdirSync, readFileSync, realpathSync, rmSync, writeFileSync, chmodSync } from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { createSourceManifest } from './provenance.mjs'
import { commandEnvironment, redact, runCommand } from './report.mjs'
import { validateOwnedPostgres } from './postgres.mjs'
import { validateOutputLocation } from './linux-runner.mjs'
import { parseStoresBrowserOutput, validateStoresBrowserReport, storesBrowserProducts } from './stores-browser-results.mjs'
import { validateMonitoringBrowser } from './monitoring-results.mjs'
import { validateRecovery } from './recovery-results.mjs'

const sha = value => createHash('sha256').update(value).digest('hex')
export function parseStoresBrowserArgs(args) {
  const names = { '--browser': 'product', '--out-dir': 'outDir', '--source-revision': 'sourceRevision' }, options = {}, seen = new Set()
  for (let index = 0; index < args.length; index += 2) {
    const key = names[args[index]], value = args[index + 1]
    if (!key || seen.has(key) || !value || value.startsWith('--')) throw new Error('Unknown, duplicate or missing work-order browser argument')
    seen.add(key); options[key] = value
  }
  if (seen.size !== 3 || !storesBrowserProducts.includes(options.product) || !path.isAbsolute(options.outDir || '') || !/^[a-f0-9]{40}$/.test(options.sourceRevision || '')) {
    throw new Error('Usage: node scripts/ci/stores-browser.mjs --browser chromium|chrome|msedge --out-dir <fresh absolute directory> --source-revision <full SHA>')
  }
  return options
}

export function storesBrowserEnvironment(source, { product, root, workDir, screenshotsDir }) {
  validateOwnedPostgres(source)
  if (new URL(source.TEST_DATABASE_URL).hash) throw new Error('Owned browser administration URL cannot contain a fragment')
  if (!storesBrowserProducts.includes(product)) throw new Error('Select a declared real browser product')
  const binary = process.platform === 'win32' ? '.exe' : ''
  return { ...commandEnvironment(source), TEST_DATABASE_URL: source.TEST_DATABASE_URL,
    CI_TEST_POSTGRES_OWNER: source.CI_TEST_POSTGRES_OWNER, SAAS_TEST_BROWSER: product,
    STORES_BROWSER_PLATFORM_BINARY: path.join(root, 'target/ci-rust/debug/saas-platform-backend' + binary),
    STORES_BROWSER_BUSINESS_BINARY: path.join(root, 'target/ci-rust/debug/stores-api' + binary),
    STORES_BROWSER_WORK_ORDERS_BINARY: path.join(root, 'target/ci-rust/debug/work-orders-api' + binary),
    STORES_BROWSER_ADMIN_BINARY: path.join(root, 'target/phase25-platform-admin/debug/saas-platform-admin' + binary),
    STORES_BROWSER_HOST_DIST: path.join(root, 'frontend/dist'),
    STORES_BROWSER_TLS_CERT: path.join(workDir, 'tls/server.pem'),
    STORES_BROWSER_TLS_KEY: path.join(workDir, 'tls/server.key'),
    STORES_BROWSER_TLS_CA: path.join(workDir, 'tls/ca.pem'),
    STORES_BROWSER_REPORT_DIR: screenshotsDir,
    STORES_BROWSER_ARTIFACT_DIR: path.join(workDir, '..', 'artifacts'),
    STORES_BROWSER_PROGRESS_FILE: path.join(workDir, '..', 'progress.json'),
    NGINX_BIN: '/usr/sbin/nginx', NGINX_MIME_TYPES: '/etc/nginx/mime.types',
    // Child-only HOME supports legacy Chromium NSS; XDG_DATA_HOME is the current layout.
    HOME: path.join(workDir, 'browser-home'), XDG_DATA_HOME: path.join(workDir, 'browser-data'),
    XDG_CONFIG_HOME: path.join(workDir, 'browser-config'), XDG_CACHE_HOME: path.join(workDir, 'browser-cache'),
    NODE_EXTRA_CA_CERTS: path.join(workDir, 'tls/ca.pem'), SSL_CERT_FILE: path.join(workDir, 'tls/ca.pem') }
}

function binaryIdentity(file) {
  assert(!lstatSync(file).isSymbolicLink() && lstatSync(file).isFile(), 'Managed binary must be a regular file')
  const bytes = readFileSync(file)
  assert(bytes.length >= 20 && bytes.subarray(0, 4).toString('hex') === '7f454c46' && bytes[4] === 2 && bytes[5] === 1, 'Managed browser service must be a 64-bit little-endian ELF')
  const elfMachine = bytes.readUInt16LE(18)
  assert([62, 183].includes(elfMachine), 'Managed browser service ELF architecture must be declared')
  return { sha256: sha(bytes), bytes: bytes.length, elfMachine }
}

export async function runStoresBrowser({ root = process.cwd(), outDir, product, sourceRevision, env = process.env, execute = runCommand, monitoring = false, recovery = false } = {}) {
  root = path.resolve(root)
  if (!path.isAbsolute(outDir || '')) throw new Error('Real work-order browser evidence requires a fresh absolute directory')
  outDir = path.resolve(outDir)
  if (existsSync(outDir)) throw new Error('Real work-order browser evidence requires a fresh absolute directory')
  validateOutputLocation(outDir, root)
  const workDir = path.join(outDir, 'work'), screenshotsDir = path.join(outDir, 'screenshots')
  const childEnv = storesBrowserEnvironment(env, { product, root, workDir, screenshotsDir })
  const source = createSourceManifest(root, { sourceRevision }), ownership = validateOwnedPostgres(env)
  mkdirSync(path.join(outDir, 'logs'), { recursive: true })
  mkdirSync(workDir, { mode: 0o700 }); mkdirSync(path.join(workDir, 'tls'), { mode: 0o700 })
  const ownedReal = realpathSync(workDir), outputReal = realpathSync(outDir)
  const secrets = [env.TEST_DATABASE_URL, new URL(env.TEST_DATABASE_URL).password].filter(Boolean)
  const clean = value => redact(secrets.reduce((text, secret) => text.replaceAll(secret, '[REDACTED]'), String(value)))
  const report = { schemaVersion: 1, kind: 'stores-real-platform-browser', status: 'FAIL', product,
    sourceRevision: source.sourceRevision, inputsSha256: source.inputsSha256, ownership,
    startedAt: new Date().toISOString(), steps: [], cleanupErrors: [], sourceUnchanged: false, privateWorkRemoved: false }
  const save = () => writeFileSync(path.join(outDir, 'result.json'), JSON.stringify(report, null, 2) + '\n', { mode: 0o600 })
  async function command(id, executable, args) {
    const result = await execute(executable, args, { cwd: root, env: childEnv, timeout: id === 'real-browser' ? 15 * 60_000 : 60_000 })
    const output = clean(result.output || '')
    const log = `logs/${String(report.steps.length + 1).padStart(2, '0')}-${id}.log`
    writeFileSync(path.join(outDir, log), output, { mode: 0o600 })
    report.steps.push({ id, command: [path.basename(executable), ...args.map(value => value.startsWith(workDir) ? '<owned-work>/' + path.relative(workDir, value).replaceAll('\\', '/') : value)],
      exitCode: result.exitCode, timedOut: Boolean(result.timedOut), status: result.exitCode === 0 && !result.timedOut ? 'PASS' : 'FAIL', log })
    save()
    if (result.exitCode !== 0 || result.timedOut) throw new Error(`Managed ${id} command failed or timed out`)
    return output
  }
  try {
    report.binaries = { platform: binaryIdentity(childEnv.STORES_BROWSER_PLATFORM_BINARY), business: binaryIdentity(childEnv.STORES_BROWSER_BUSINESS_BINARY), workOrders: binaryIdentity(childEnv.STORES_BROWSER_WORK_ORDERS_BINARY), admin: binaryIdentity(childEnv.STORES_BROWSER_ADMIN_BINARY) }
    report.tools = { node: process.version, nginx: await command('nginx-version', childEnv.NGINX_BIN, ['-V']),
      nginxLibraries: await command('nginx-runtime-libraries', 'ldd', [childEnv.NGINX_BIN]), openssl: await command('openssl-version', 'openssl', ['version']),
      packages: await command('tool-package-versions', 'dpkg-query', ['--show', '--showformat=${Package}=${Version}\n', 'nginx', 'openssl', 'libnss3-tools']) }
    assert.match(report.tools.nginx, /nginx version: nginx\/1\.28\.0(?:\s|$)/, 'Use the declared production-compatible Nginx binary')
    assert.match(report.tools.nginxLibraries, /libc\.so/); assert.doesNotMatch(report.tools.nginxLibraries, /not found/i)
    const tls = path.join(workDir, 'tls'), ca = path.join(tls, 'ca.pem'), key = path.join(tls, 'ca.key')
    const serverKey = path.join(tls, 'server.key'), certificate = path.join(tls, 'server.pem'), request = path.join(tls, 'server.csr'), extensions = path.join(tls, 'server.ext')
    writeFileSync(extensions, 'basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=IP:127.0.0.1,DNS:localhost\n', { mode: 0o600 })
    await command('owned-ca', 'openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-sha256', '-days', '1', '-subj', '/CN=Phase27-Owned-Test-CA', '-keyout', key, '-out', ca,
      '-addext', 'basicConstraints=critical,CA:TRUE', '-addext', 'keyUsage=critical,keyCertSign,cRLSign'])
    await command('owned-server-request', 'openssl', ['req', '-new', '-newkey', 'rsa:2048', '-nodes', '-sha256', '-subj', '/CN=127.0.0.1', '-keyout', serverKey, '-out', request])
    await command('owned-server-certificate', 'openssl', ['x509', '-req', '-in', request, '-CA', ca, '-CAkey', key, '-CAcreateserial', '-out', certificate, '-days', '1', '-sha256', '-extfile', extensions])
    for (const file of [key, serverKey, ca, certificate]) chmodSync(file, 0o600)
    await command('owned-tls-verify', 'openssl', ['verify', '-CAfile', ca, '-verify_ip', '127.0.0.1', certificate])
    const caX509 = new X509Certificate(readFileSync(ca)), serverX509 = new X509Certificate(readFileSync(certificate))
    assert(caX509.ca && !serverX509.ca && serverX509.checkIP('127.0.0.1') && serverX509.verify(caX509.publicKey), 'Owned TLS chain and IP name must verify')
    report.tls = { caSha256: sha(caX509.raw), serverSha256: sha(serverX509.raw), ipVerified: true, ownedTrustOnly: true }
    for (const [name, database] of [['xdg', path.join(childEnv.XDG_DATA_HOME, 'pki/nssdb')], ['legacy', path.join(childEnv.HOME, '.pki/nssdb')]]) {
      mkdirSync(database, { recursive: true, mode: 0o700 })
      await command(`owned-nss-${name}-create`, 'certutil', ['-N', '--empty-password', '-d', 'sql:' + database])
      await command(`owned-nss-${name}-trust`, 'certutil', ['-A', '-d', 'sql:' + database, '-n', 'phase27-owned-ca', '-t', 'C,,', '-i', ca])
      await command(`owned-nss-${name}-verify`, 'certutil', ['-L', '-d', 'sql:' + database, '-n', 'phase27-owned-ca'])
    }
    const output = await command('real-browser', process.execPath, [recovery ? 'tests/phase33-recovery-browser.mjs' : monitoring ? 'tests/phase32-monitoring-browser.mjs' : 'tests/phase27-stores-browser.mjs'])
    if (recovery) {
      const lines = output.split(/\r?\n/).filter(line => line.startsWith('PHASE33_RESULT '))
      assert.equal(lines.length,1)
      const measured = JSON.parse(lines[0].slice('PHASE33_RESULT '.length))
      validateRecovery(measured)
      assert.equal(measured.status,'PASS');assert.equal(measured.checks.length,10)
      assert(measured.checks.every(c=>c.status==='PASS'))
      assert(Object.values(measured.cleanup).every(v=>v===true))
      assert.equal(measured.producerSha256,source.files.find(f=>f.path==='tests/phase33-recovery-browser.mjs')?.sha256)
      report.recovery=measured;report.status='PASS';return report
    }
    if (monitoring) {
      const lines = output.split(/\r?\n/).filter(line => line.startsWith('PHASE32_BROWSER_RESULT '))
      assert.equal(lines.length, 1, 'Exactly one monitoring result required')
      const measured = JSON.parse(lines[0].slice('PHASE32_BROWSER_RESULT '.length))
      validateMonitoringBrowser(measured, product)
      assert.equal(measured.status, 'PASS'); assert.equal(measured.product, product)
      assert(Object.values(measured.cleanup).every(value => value === true), 'All monitoring browser resources must be cleaned')
      for (const [name,key] of [['platform','platformBinary'],['business','businessBinary'],['workOrders','workOrdersBinary'],['admin','adminBinary']]) {
        assert.equal(measured.source[key].sha256, report.binaries[name].sha256, 'Monitoring must use the selected actual ELF')
      }
      assert.equal(measured.producerSha256, source.files.find(file => file.path === 'tests/phase32-monitoring-browser.mjs')?.sha256)
      for (const [name,file] of [['platform',childEnv.STORES_BROWSER_PLATFORM_BINARY],['business',childEnv.STORES_BROWSER_BUSINESS_BINARY],['workOrders',childEnv.STORES_BROWSER_WORK_ORDERS_BINARY],['admin',childEnv.STORES_BROWSER_ADMIN_BINARY]]) assert.deepEqual(binaryIdentity(file),report.binaries[name])
      report.monitoring = measured; report.status = 'PASS'
      return report
    }
    Object.assign(report, parseStoresBrowserOutput(output, { product }))
    const actual = report.result.source
    assert.equal(actual.platformBinary.sha256, report.binaries.platform.sha256, 'Browser must use the selected actual platform binary')
    assert.equal(actual.businessBinary.sha256, report.binaries.business.sha256, 'Browser must use the selected actual business binary')
    for (const [name, key] of [['workOrders', 'workOrdersBinary'], ['admin', 'adminBinary']]) assert.equal(actual[key].sha256, report.binaries[name].sha256, 'Selected actual ELF must match ' + name)
    assert.equal(report.result.sourceSha256, source.files.find(file => file.path === 'tests/phase27-stores-browser.mjs')?.sha256, 'Browser producer must match the frozen source manifest')
    for (const item of report.result.evidence.screenshots || []) {
      assert(path.isAbsolute(item.path) && path.dirname(item.path) === screenshotsDir && !lstatSync(item.path).isSymbolicLink(), 'Screenshots must remain in the owned screenshots directory')
      const bytes = readFileSync(item.path)
      assert.equal(bytes.subarray(0, 8).toString('hex'), '89504e470d0a1a0a', 'Screenshot must be PNG')
      assert.equal(sha(bytes), item.sha256, 'Screenshot bytes must match the browser evidence')
    }
    assert.deepEqual(binaryIdentity(childEnv.STORES_BROWSER_PLATFORM_BINARY), report.binaries.platform)
    assert.deepEqual(binaryIdentity(childEnv.STORES_BROWSER_BUSINESS_BINARY), report.binaries.business)
    assert.deepEqual(binaryIdentity(childEnv.STORES_BROWSER_WORK_ORDERS_BINARY), report.binaries.workOrders)
    assert.deepEqual(binaryIdentity(childEnv.STORES_BROWSER_ADMIN_BINARY), report.binaries.admin)
    report.status = 'PASS'
  } catch (error) { report.status = 'FAIL'; report.error = clean(error.message) }
  finally {
    try { report.sourceUnchanged = createSourceManifest(root, { sourceRevision: source.sourceRevision }).inputsSha256 === source.inputsSha256 }
    catch { report.sourceUnchanged = false }
    if (!report.sourceUnchanged) { report.status = 'FAIL'; report.error = 'Source inputs changed during real work-order browser execution' }
    try {
      assert(realpathSync(outDir) === outputReal && realpathSync(workDir) === ownedReal && path.dirname(ownedReal) === outputReal && !lstatSync(workDir).isSymbolicLink(), 'Only the owned private work directory may be removed')
      rmSync(workDir, { recursive: true, force: true }); report.privateWorkRemoved = !existsSync(workDir)
    } catch { report.cleanupErrors.push('owned TLS/NSS work directory removal failed'); report.status = 'FAIL' }
    report.finishedAt = new Date().toISOString()
    if (report.status === 'PASS' && !monitoring && !recovery) try { validateStoresBrowserReport(report, { product }) } catch (error) { report.status = 'FAIL'; report.error = clean(error.message) }
    save()
  }
  return report
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try { const result = await runStoresBrowser(parseStoresBrowserArgs(process.argv.slice(2))); console.log(`STORES_BROWSER ${result.product} ${result.status}`); if (result.status !== 'PASS') process.exitCode = 1 }
  catch (error) { console.error(redact(error.message)); process.exitCode = 1 }
}
