/** Actual Rust binary over authenticated HTTPS; no production configuration. */
import assert from 'node:assert/strict'
import { spawn, execFileSync } from 'node:child_process'
import { randomBytes, createHash } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, writeFileSync, realpathSync, rmSync } from 'node:fs'
import https from 'node:https'
import path from 'node:path'

const binary = process.env.PHASE25_CLI_BIN
const out = process.env.PHASE25_TLS_OUT
assert(path.isAbsolute(binary || '') && existsSync(binary), 'Explicit prebuilt CLI binary required')
assert(path.isAbsolute(out || '') && !existsSync(out), 'Fresh absolute TLS evidence directory required')
mkdirSync(out, { recursive: true })
const privateDir = path.join(out, 'private')
mkdirSync(privateDir, { mode: 0o700 })
const file = name => path.join(privateDir, name)
const bearer = 'owned-phase25-' + randomBytes(24).toString('hex')
const privateBody = 'owned-body-' + randomBytes(24).toString('hex')
const checks = [], commands = []
let server, requests = 0, badAuthentication = false, mode = 'reject', failure
function openssl(args) {
  commands.push(['openssl', ...args])
  try { execFileSync('openssl', args, { stdio: 'ignore', timeout: 15_000, windowsHide: true }) }
  catch { throw new Error('Owned certificate generation failed') }
}
async function cli(origin, ca, timeout = 2000, command = ['releases']) {
  const env = Object.fromEntries(['PATH', 'Path', 'SystemRoot', 'SYSTEMROOT', 'WINDIR', 'TEMP', 'TMP'].filter(name => process.env[name]).map(name => [name, process.env[name]]))
  Object.assign(env, { PLATFORM_API_ORIGIN: origin, PLATFORM_ADMIN_BEARER: bearer })
  if (ca) env.PLATFORM_CA_FILE = ca
  const args = [...command, '--timeout-ms', String(timeout)]
  const result = await new Promise((resolve, reject) => {
    const child = spawn(binary, args, { env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = '', stderr = ''
    const timer = setTimeout(() => child.kill('SIGKILL'), 10_000)
    const collect = (name, bytes) => { if (stdout.length + stderr.length > 65536) child.kill('SIGKILL'); else if (name === 'stdout') stdout += bytes; else stderr += bytes }
    child.stdout.on('data', bytes => collect('stdout', bytes)); child.stderr.on('data', bytes => collect('stderr', bytes))
    child.once('error', () => { clearTimeout(timer); reject(new Error('Unable to execute owned CLI')) })
    child.once('close', code => { clearTimeout(timer); resolve({ code, stdout, stderr }) })
  })
  const output = result.stdout + result.stderr
  assert(![bearer, privateBody].some(secret => output.includes(secret)), 'CLI leaked a private fixture value')
  let error
  try { error = JSON.parse(result.stderr.trim() || result.stdout.trim()) } catch { throw new Error('CLI returned an invalid safe diagnostic') }
  assert(result.code !== 0 && error.ok === false && typeof error.category === 'string', 'Expected a fixed failure diagnostic')
  return error
}
function check(id) { checks.push({ id, status: 'PASS' }); console.log('PASS phase25 TLS ' + id) }
try {
  openssl(['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', file('ca.key'), '-out', file('ca.pem'), '-days', '1', '-subj', '/CN=Owned Phase25 Test CA'])
  openssl(['req', '-newkey', 'rsa:2048', '-nodes', '-keyout', file('server.key'), '-out', file('server.csr'), '-subj', '/CN=127.0.0.1'])
  writeFileSync(file('server.ext'), 'subjectAltName=IP:127.0.0.1\nextendedKeyUsage=serverAuth\nbasicConstraints=critical,CA:FALSE\n')
  openssl(['x509', '-req', '-in', file('server.csr'), '-CA', file('ca.pem'), '-CAkey', file('ca.key'), '-CAcreateserial', '-out', file('server.pem'), '-days', '1', '-extfile', file('server.ext')])
  server = https.createServer({ key: readFileSync(file('server.key')), cert: readFileSync(file('server.pem')) }, (request, response) => {
    requests++
    if (request.headers.authorization !== 'Bearer ' + bearer) badAuthentication = true
    response.setHeader('Content-Type', 'application/json')
    if (mode === 'echo') {
      const appId = '11111111-1111-4111-8111-111111111111'
      const release = { id: '22222222-2222-4222-8222-222222222222', applicationId: appId,
        environment: 'production', entryType: 'HTML', version: bearer,
        validationStatus: request.method === 'POST' ? 'PASSED' : 'DRAFT', active: false }
      const payload = request.url === '/api/applications'
        ? [{ id: appId, code: 'work-orders', runtimeMode: 'IFRAME', entryType: 'HTML', basePath: '/micro/work-orders', status: 'ENABLED' }]
        : request.method === 'POST' ? release : [release]
      response.writeHead(200); response.end(JSON.stringify(payload)); return
    }
    if (mode === 'redirect') { response.writeHead(302, { Location: 'https://unused.example.invalid/' + privateBody }); response.end(JSON.stringify({ client_secret: privateBody })); return }
    if (mode === 'large') { response.writeHead(200); response.end(' '.repeat(1024 * 1024 + 1)); return }
    if (mode === 'slow') {
      response.writeHead(200); response.write(' ')
      const drip = setInterval(() => response.write(' '), 20)
      const finish = setTimeout(() => { clearInterval(drip); response.end('{}') }, 2000)
      response.once('close', () => { clearInterval(drip); clearTimeout(finish) }); return
    }
    response.writeHead(403); response.end(JSON.stringify({ error: 'denied', client_secret: privateBody, token: bearer }))
  })
  server.on('tlsClientError', () => {})
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '::', resolve) })
  const port = server.address().port
  const origin = `https://127.0.0.1:${port}`
  const trusted = await cli(origin, file('ca.pem'))
  assert(trusted.category === 'HTTP_ERROR' && trusted.status === 403 && requests === 1 && !badAuthentication, 'Trusted TLS must reach the authenticated endpoint')
  check('trusted-CA-and-hostname-with-private-bearer')
  let count = requests
  const untrusted = await cli(origin)
  assert(untrusted.category === 'NETWORK_ERROR' && requests === count, 'Untrusted certificate must fail before HTTP')
  check('untrusted-certificate-rejected')
  const wrongName = await cli(`https://localhost:${port}`, file('ca.pem'))
  assert(wrongName.category === 'NETWORK_ERROR' && requests === count, 'Trusted CA cannot bypass hostname validation')
  check('wrong-certificate-hostname-rejected')
  const privateKey = await cli(origin, file('server.key'))
  assert(privateKey.category === 'INVALID_CA_CONFIGURATION' && requests === count, 'Private key must never become trusted CA configuration')
  check('private-key-as-CA-rejected-without-echo')
  mode = 'redirect'
  const redirect = await cli(origin, file('ca.pem'))
  assert(redirect.category === 'REDIRECT_REJECTED' && requests === count + 1, 'Redirect must be rejected without credential forwarding')
  check('HTTPS-redirect-rejected-and-redacted')
  mode = 'large'
  const large = await cli(origin, file('ca.pem'))
  assert(large.category === 'RESPONSE_TOO_LARGE', 'HTTPS response body must be bounded')
  check('HTTPS-response-size-limit')
  mode = 'echo'
  const echoedRead = await cli(origin, file('ca.pem'))
  assert(echoedRead.category === 'SENSITIVE_OUTPUT_REJECTED' && !echoedRead.outcomeUnknown,
    'Valid successful response fields must not echo the Bearer')
  check('successful-summary-bearer-echo-rejected')
  const echoedWrite = await cli(origin, file('ca.pem'), 2000,
    ['preflight', '--release-id', '22222222-2222-4222-8222-222222222222', '--apply'])
  assert(echoedWrite.category === 'SENSITIVE_OUTPUT_REJECTED' && echoedWrite.outcomeUnknown === true &&
    echoedWrite.reconciliationRequired === true && echoedWrite.nextAction === 'INSPECT_AND_RECONCILE_BEFORE_RETRY',
  'Sensitive write output must retain mandatory unknown-result reconciliation')
  check('write-summary-bearer-echo-preserves-reconciliation')
  mode = 'slow'
  const slow = await cli(origin, file('ca.pem'), 200)
  assert(slow.category === 'REQUEST_TIMEOUT', 'Continuously dripping HTTPS cannot extend the total deadline')
  check('HTTPS-total-deadline-and-redaction')
} catch (error) { failure = error.message; process.exitCode = 1 }
finally {
  if (server) { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)) }
  assert.equal(path.dirname(realpathSync(privateDir)), realpathSync(out), 'Certificate cleanup stays in the owned output')
  rmSync(privateDir, { recursive: true, force: true })
  const report = { schemaVersion: 1, status: failure ? 'FAIL' : 'PASS', checks, passed: checks.length, failed: failure ? 1 : 0, skipped: 0, commands, privateTlsRemoved: true,
    binarySha256: createHash('sha256').update(readFileSync(binary)).digest('hex'), ...(failure ? { error: failure } : {}) }
  writeFileSync(path.join(out, 'result.json'), JSON.stringify(report, null, 2) + '\n')
  console.log('PHASE25_TLS_RESULT ' + JSON.stringify(report))
}
