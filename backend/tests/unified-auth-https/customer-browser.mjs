/** Real three-service TLS verification. Browser credentials stay in memory;
 * SQL changes only guarded fixture boundaries, never fabricates a grant.
 * The private negative probe decrypts only task-owned server access/pending
 * envelopes inside the namespace and emits an HTTP status, never credentials.
 */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { randomBytes } from 'node:crypto';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repository = path.resolve(here, '../../..');
const options = {};
const args = process.argv.slice(2);
for (let i = 0; i < args.length; i += 2) {
  assert(['--environment', '--output', '--proxy', '--diagnostic'].includes(args[i]) && args[i + 1], 'INVALID_ARGUMENTS');
  assert(!Object.hasOwn(options, args[i]), 'DUPLICATE_ARGUMENT');
  options[args[i]] = args[i + 1];
}
assert(options['--environment'] && options['--output'], 'ENVIRONMENT_AND_OUTPUT_REQUIRED');
const environmentFile = options['--environment'];
assert(/^\/tmp\/rust-saas-unified-auth-20261028\/(?:[A-Za-z0-9_.-]+\/)*https-environment\.json$/.test(environmentFile)
  && !environmentFile.split('/').includes('..'), 'OWNED_ENVIRONMENT_REQUIRED');
const outputFile = path.resolve(options['--output']);
assert(outputFile.endsWith('.json'), 'JSON_OUTPUT_REQUIRED');
assert(!['https-environment.json', 'https-database.json', 'https-portal-keys.json', 'https-bff-keys.json',
  'https-context-keys.json', 'https-signing.json'].includes(path.basename(outputFile)), 'REPORT_CANNOT_REPLACE_PRIVATE_CONFIGURATION');
const taskDirectory = environmentFile.replace(/\/https-environment\.json$/, '');
const proxy = options['--proxy'] || 'http://127.0.0.1:24445';
assert(proxy === 'http://127.0.0.1:24445', 'OWNED_PROXY_REQUIRED');
assert(!options['--diagnostic'] || options['--diagnostic'] === 'last-pending', 'FIXED_DIAGNOSTIC_REQUIRED');
const uuid7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const AUTH = 'https://auth.saas.test';
const CONSOLE = 'https://console.saas.test';
const CUSTOMER = 'https://customer.saas.test';
const FAULT = 'phase14_https_pending_save_fault';
const FAULT_MARKER = 'unified-auth-https/phase14/pending-save-fault';

function linuxFile(filename) {
  try {
    if (process.platform === 'linux') return fs.readFileSync(filename, 'utf8');
    return execFileSync('wsl.exe', ['-u', 'root', '--', 'cat', filename],
      { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true, timeout: 10_000 });
  } catch { throw new Error('OWNED_PRIVATE_FILE_UNAVAILABLE'); }
}
function privateJson(filename) {
  try { return JSON.parse(linuxFile(filename)); }
  catch { throw new Error('OWNED_PRIVATE_JSON_UNAVAILABLE'); }
}
const environment = privateJson(environmentFile);
const manifest = privateJson(`${taskDirectory}/https-database.json`);
const ids = environment.ids;
assert(environment.portal && manifest.portal, 'PORTAL_FIXTURE_REQUIRED');
for (const value of Object.values(ids)) assert(uuid7.test(value), 'FIXTURE_ID_REQUIRED');
assert(/^unified_auth_https_[0-9a-f]{32}$/.test(manifest.database)
  && manifest.portal.database === manifest.database.replace('unified_auth_https_', 'unified_auth_portal_')
  && /^[0-9]{10,24}$/.test(manifest.clusterId), 'OWNED_DATABASE_MANIFEST_REQUIRED');
assert(manifest.portal.marker === `unified-auth-https-portal/1/${ids.deployment}`, 'PORTAL_MARKER_REQUIRED');
assert(new URL(environment.portal.DATABASE_URL).pathname === `/${manifest.portal.database}`, 'PORTAL_DATABASE_BINDING_REQUIRED');
const databases = {
  shared: { name: manifest.database, marker: `identity-deployment/1/${ids.deployment}` },
  portal: { name: manifest.portal.database, marker: manifest.portal.marker },
};

function newId() {
  const bytes = randomBytes(16);
  bytes.writeUIntBE(Date.now(), 0, 6);
  bytes[6] = (bytes[6] & 15) | 112;
  bytes[8] = (bytes[8] & 63) | 128;
  const hex = bytes.toString('hex');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
function sql(database, statement) {
  const { name, marker } = databases[database] || {};
  assert(name && marker, 'FIXED_DATABASE_REQUIRED');
  const input = `BEGIN; DO $$ BEGIN IF current_database()<>'${name}'
    OR (SELECT system_identifier::text FROM pg_control_system())<>'${manifest.clusterId}'
    OR (SELECT shobj_description(oid,'pg_database') FROM pg_database WHERE datname=current_database()) IS DISTINCT FROM '${marker}'
    THEN RAISE EXCEPTION 'owned phase14 fixture guard failed'; END IF; END $$;
    ${statement}; COMMIT;`;
  try {
    const command = process.platform === 'linux' ? 'psql' : 'wsl.exe';
    const prefix = process.platform === 'linux' ? [] : ['-u', 'root', '--', 'psql'];
    return execFileSync(command, [...prefix, '-X', '-q', '-v', 'ON_ERROR_STOP=1', '-v', 'VERBOSITY=sqlstate',
      '-At', `postgres://postgres@127.0.0.1:55494/${name}`],
    { input, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true, timeout: 15_000 }).trim();
  } catch (failure) {
    const error = new Error('OWNED_SQL_FAILED');
    const code = /ERROR:\s+([0-9A-Z]{5})\b/.exec(String(failure.stderr || ''))?.[1];
    error.code = code ? `SQLSTATE_${code}` : 'OWNED_SQL_TRANSPORT_FAILED';
    throw error;
  }
}
function row(database, statement) {
  const value = sql(database, `SELECT row_to_json(v)::text FROM (${statement}) v`);
  assert(value && !value.includes('\n'), 'EXACT_FIXTURE_ROW_REQUIRED');
  return JSON.parse(value);
}
function latestPortal(account = 'customer', pending = false) {
  return row('portal', `SELECT id::text,version::text,user_id::text,oidc_grant_id::text,global_session_id::text,
    expires_at::text,access_expires_at::text,pending_selection_ciphertext IS NOT NULL AS pending
    FROM portal_sessions WHERE user_id='${ids[account]}' AND revoked_at IS NULL
      ${pending ? 'AND pending_selection_ciphertext IS NOT NULL' : ''} ORDER BY created_at DESC,id DESC LIMIT 1`);
}
function committedSelection(sessionId) {
  assert(uuid7.test(sessionId), 'BFF_SESSION_ID_REQUIRED');
  return row('shared', `SELECT s.transaction_id::text,s.consumed_at IS NOT NULL AS consumed,
    c.id::text AS context_id,c.token_hash AS capability_digest,c.expires_at::text AS capability_expiry,
    c.source_expires_at::text AS source_expiry,s.expires_at::text AS recovery_expiry,
    (SELECT count(*) FROM audit_logs a WHERE a.action='CONTEXT_SELECTION_CONSUME'
      AND a.detail::jsonb->>'transactionId'=s.transaction_id::text) AS consumption_count
    FROM context_selections s JOIN customer_portal_contexts c ON c.id::text=s.result->>'contextId'
    WHERE c.bff_session_id='${sessionId}' AND c.user_id='${ids.customer}' AND s.client_id='${ids.portalClient}'`);
}
function resetExtraMemberships() {
  sql('shared', `UPDATE memberships SET status='DISABLED',identity_version=identity_version+1
    WHERE member_type='CUSTOMER' AND status='ENABLED'
      AND user_id='${ids.customer}' AND organization_id='${ids.secondOrganization}'`);
}
function dropFault() {
  sql('portal', `DO $$ BEGIN
    IF EXISTS(SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
      WHERE n.nspname='public' AND p.proname='${FAULT}' AND obj_description(p.oid,'pg_proc') IS DISTINCT FROM '${FAULT_MARKER}')
      OR EXISTS(SELECT 1 FROM pg_trigger t WHERE t.tgrelid='public.portal_sessions'::regclass
      AND t.tgname='${FAULT}' AND obj_description(t.oid,'pg_trigger') IS DISTINCT FROM '${FAULT_MARKER}')
    THEN RAISE EXCEPTION 'phase14 fault ownership marker mismatch'; END IF; END $$;
    DROP TRIGGER IF EXISTS ${FAULT} ON public.portal_sessions;
    DROP FUNCTION IF EXISTS public.${FAULT}()`);
}
function installFault() {
  dropFault();
  sql('portal', `CREATE FUNCTION public.${FAULT}() RETURNS trigger LANGUAGE plpgsql
    SET search_path='pg_catalog','public' AS $fault$
    BEGIN IF OLD.pending_selection_ciphertext IS NOT NULL AND NEW.pending_selection_ciphertext IS NULL
      AND OLD.user_id='${ids.customer}'::uuid THEN
      RAISE EXCEPTION 'PHASE14_EXPECTED_PENDING_SAVE_FAILURE'; END IF; RETURN NEW; END $fault$;
    COMMENT ON FUNCTION public.${FAULT}() IS '${FAULT_MARKER}';
    CREATE TRIGGER ${FAULT} BEFORE UPDATE ON public.portal_sessions FOR EACH ROW EXECUTE FUNCTION public.${FAULT}();
    COMMENT ON TRIGGER ${FAULT} ON public.portal_sessions IS '${FAULT_MARKER}'`);
}

// This helper has no secret-bearing stdout. It proves actual role isolation or
// sends deliberately invalid variants of one original pending private request.
const privateProbe = String.raw`
import base64,json,os,pathlib,re,subprocess,sys,urllib.parse,uuid
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
stage='input'
try:
 request=json.load(sys.stdin);root=pathlib.Path(request['environment']).resolve().parent
 if root.parent!=pathlib.Path('/tmp') or not root.name.startswith('rust-saas-unified-auth-') or not (root/'OWNER').is_file(): raise ValueError()
 stage='namespace'
 if os.readlink('/proc/self/ns/mnt')!=(root/'namespace.identity').read_text().strip(): raise ValueError()
 stage='configuration'
 config=json.loads((root/'https-environment.json').read_text());manifest=json.loads((root/'https-database.json').read_text())
 portal=config['portal'];ids=config['ids'];session=request.get('sessionId','')
 def sql(kind,statement):
  name=manifest['database'] if kind=='shared' else manifest['portal']['database']
  marker='identity-deployment/1/'+ids['deployment'] if kind=='shared' else manifest['portal']['marker']
  guard="BEGIN; DO $$ BEGIN IF current_database()<>'"+name+"' OR (SELECT system_identifier::text FROM pg_control_system())<>'"+manifest['clusterId']+"' OR (SELECT shobj_description(oid,'pg_database') FROM pg_database WHERE datname=current_database()) IS DISTINCT FROM '"+marker+"' THEN RAISE EXCEPTION 'owned probe guard failed'; END IF; END $$;"
  result=subprocess.run(['psql','-X','-q','-v','ON_ERROR_STOP=1','-v','VERBOSITY=sqlstate','-At','postgres://postgres@127.0.0.1:55494/'+name],input=guard+statement+';COMMIT;',text=True,capture_output=True,timeout=10)
  if result.returncode:
   failure=ValueError();match=re.search(r'ERROR:\s+([0-9A-Z]{5})\b',result.stderr);failure.sqlstate=match.group(1) if match else None;raise failure
  return result.stdout.strip()
 if request['mode']=='runtime-isolation':
  stage='runtime_binding'
  parsed=urllib.parse.urlparse(portal['DATABASE_URL']);runtime=parsed.username
  if parsed.hostname!='127.0.0.1' or parsed.port!=55494 or parsed.path!='/'+manifest['portal']['database'] or runtime!=manifest['portal']['runtime']: raise ValueError()
  env={'PATH':'/usr/lib/postgresql/18/bin:/usr/bin:/bin','PGHOST':'127.0.0.1','PGPORT':'55494','PGUSER':runtime,'PGPASSWORD':urllib.parse.unquote(parsed.password or ''),'PGDATABASE':manifest['portal']['database']}
  stage='runtime_own_login'
  own=subprocess.run(['psql','-X','-q','-At','-c','SELECT current_user=session_user'],env=env,text=True,capture_output=True,timeout=10)
  env['PGDATABASE']=manifest['database']
  stage='runtime_shared_login'
  denied=subprocess.run(['psql','-X','-q','-v','VERBOSITY=sqlstate','-At','-c','SELECT current_user'],env=env,text=True,capture_output=True,timeout=10)
  stage='runtime_acl_query'
  permissions=sql('portal',"SELECT NOT has_database_privilege('"+runtime+"','"+manifest['database']+"','CONNECT') AND NOT has_schema_privilege('"+runtime+"','public','CREATE') AND NOT has_table_privilege('"+runtime+"','public._sqlx_migrations','SELECT,INSERT,UPDATE,DELETE')")
  print(json.dumps({'ownLogin':own.returncode==0 and own.stdout.strip()=='t','sharedConnectionDenied':denied.returncode!=0 and ('42501' in denied.stderr or 'permission denied for database' in denied.stderr.lower()),'restricted':permissions=='t'}));sys.exit(0)
 stage='pending_binding'
 if str(uuid.UUID(session))!=session or uuid.UUID(session).version!=7: raise ValueError()
 data=json.loads(sql('portal',"SELECT row_to_json(v)::text FROM (SELECT id,version,user_id,issuer,oidc_client_id,access_ciphertext,pending_selection_ciphertext FROM portal_sessions WHERE id='"+session+"' AND user_id='"+ids['customer']+"' AND pending_selection_ciphertext IS NOT NULL) v"))
 keys=json.loads(pathlib.Path(portal['PORTAL_TOKEN_KEY_FILE']).read_text())['keys']
 def open_envelope(kind,value):
  version,kid,encoded=value.split('.')
  if version!='v1': raise ValueError()
  material=next(k for k in keys if k['kid']==kid)
  decode=lambda s:base64.urlsafe_b64decode(s+'='*((-len(s))%4))
  payload=decode(encoded);key=decode(material['key'])
  aad=('customer-portal-bff-aead/1\0'+data['id']+'\0'+data['issuer']+'\0'+data['oidc_client_id']+'\0'+kind).encode()
  return AESGCM(key).decrypt(payload[:12],payload[12:],aad).decode()
 stage='pending_envelope'
 pending=json.loads(open_envelope('pending-selection',data['pending_selection_ciphertext']))
 body={**pending['extension'],'expectedClientId':data['oidc_client_id'],'oidcAccessToken':open_envelope('oidc-access',data['access_ciphertext']),
  'bffSessionId':data['id'],'version':str(data['version']),'oldOidcAccessToken':pending['oldAccess']}
 mode=request['mode']
 if mode=='wrong-bff': body['bffSessionId']=ids['customer']
 elif mode=='wrong-version': body['version']=str(data['version']+1)
 elif mode=='wrong-rp': body['expectedClientId']=ids['client']
 elif mode=='wrong-source': body['oidcAccessToken']='0'*64
 elif mode in ['original-result','reverse-proof','selection-source']: pass
 else: raise ValueError()
 origin=portal['CUSTOMER_RESOURCE_URL']
 if origin!='https://console.saas.test:24444': raise ValueError()
 service_id=portal['CONTEXT_SELECTION_CONSUMER_ID'];service_secret=portal['CONTEXT_SELECTION_CONSUMER_SECRET']
 endpoint='/internal/platform/context-selection/result';ca_file=portal['CUSTOMER_RESOURCE_CA_FILE']
 if mode in ['reverse-proof','selection-source']:
  platform=config['platform'];origin=platform['IDENTITY_SERVICE_URL'];ca_file=platform['IDENTITY_SERVICE_CA_FILE']
  if origin!='https://auth.saas.test:24444': raise ValueError()
  if mode=='reverse-proof':
   endpoint='/internal/identity/customer-access-proof';service_id=platform['CUSTOMER_AUTHORITY_SERVICE_CLIENT_ID'];service_secret=platform['CUSTOMER_AUTHORITY_SERVICE_CLIENT_SECRET']
   body={'contractVersion':'customer-access-proof-request/1','expectedClientId':data['oidc_client_id'],'oidcAccessToken':body['oidcAccessToken']}
  else:
   endpoint='/internal/identity/auth/context-selection/source';service_id=platform['CONTEXT_SOURCE_SERVICE_CLIENT_ID'];service_secret=platform['CONTEXT_SOURCE_SERVICE_CLIENT_SECRET']
   body={'contractVersion':'context-selection/1','transactionId':pending['extension']['transactionId'],'expectedClientId':data['oidc_client_id'],'oidcAccessToken':body['oidcAccessToken']}
 authorization=base64.b64encode((service_id+':'+service_secret).encode()).decode()
 stage='private_https'
 # Keep credentials out of argv, files and diagnostics. Curl validates the
 # configured CA and DNS name; no insecure option or redirect is allowed.
 # Python 3.14's additional X509_STRICT rejects this local CA's absent AKI,
 # although the normal CA/hostname validation used by rustls/curl succeeds.
 curl_config='\n'.join(key+' = '+json.dumps(value) for key,value in [
  ('url',origin+endpoint),('header','Content-Type: application/json'),
  ('header','Authorization: Basic '+authorization),('data',json.dumps(body))])+'\n'
 response=subprocess.run(['curl','--disable','--silent','--max-time','8','--noproxy','*','--cacert',ca_file,
  '--proto','=https','--max-redirs','0','--request','POST','--write-out','\n%{http_code}','--config','-'],
  input=curl_config,text=True,capture_output=True,timeout=10)
 if response.returncode:
  failure=ValueError();failure.curlcode=response.returncode;raise failure
 raw,http_status=response.stdout.rsplit('\n',1);status=int(http_status)
 if len(raw)>65536: raise ValueError()
 safe={'status':status}
 if mode=='original-result' and status==200:
  result=json.loads(raw);safe['operationStatus']=result.get('status') if result.get('status') in ['PENDING','COMMITTED'] else 'OTHER'
 print(json.dumps(safe))
except BaseException as failure:
 if isinstance(failure,SystemExit): raise
 reason=getattr(failure,'reason',None)
 print(json.dumps({'error':'OWNED_PRIVATE_PROBE_FAILED','stage':stage,'type':type(failure).__name__,'sqlstate':getattr(failure,'sqlstate',None),
  'cause':type(reason).__name__ if reason else None,'errno':getattr(reason,'errno',None),'curlcode':getattr(failure,'curlcode',None)}));sys.exit(1)
`;
function probe(mode, sessionId) {
  let record;
  try { record = privateJson(`${taskDirectory}/namespace.pid`); }
  catch {
    const error = new Error('OWNED_NAMESPACE_METADATA_UNAVAILABLE');
    error.code = 'OWNED_PROBE_NAMESPACE_METADATA_UNAVAILABLE';
    throw error;
  }
  assert(Number.isSafeInteger(record.pid) && record.pid > 0, 'OWNED_NAMESPACE_REQUIRED');
  const payload = JSON.stringify({ environment: environmentFile, mode, ...(sessionId ? { sessionId } : {}) });
  try {
    const command = process.platform === 'linux' ? 'nsenter' : 'wsl.exe';
    const prefix = process.platform === 'linux' ? [] : ['-u', 'root', '--exec', 'nsenter'];
    return JSON.parse(execFileSync(command, [...prefix, '-t', String(record.pid), '-m', '--', 'python3', '-c', privateProbe],
      { input: payload, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true, timeout: 25_000 }));
  } catch (failure) {
    const error = new Error('OWNED_PRIVATE_PROBE_FAILED');
    if (Number.isSafeInteger(failure.status)) error.code = `OWNED_PROBE_PROCESS_EXIT_${failure.status}`;
    const startupType = /\b(SyntaxError|IndentationError|ModuleNotFoundError|FileNotFoundError)\b/.exec(String(failure.stderr || ''))?.[1];
    if (startupType) error.code = `OWNED_PROBE_STARTUP_${startupType.toUpperCase()}`;
    try {
      const detail = JSON.parse(String(failure.stdout || ''));
      if (/^[a-z_]+$/.test(detail.stage) && /^[A-Za-z]+$/.test(detail.type)) {
        error.code = `OWNED_PROBE_${detail.stage}_${detail.type}`.toUpperCase();
        if (/^[0-9A-Z]{5}$/.test(detail.sqlstate || '')) error.code += `_SQLSTATE_${detail.sqlstate}`;
        if (/^[A-Za-z]+$/.test(detail.cause || '')) error.code += `_${detail.cause.toUpperCase()}`;
        if (Number.isSafeInteger(detail.errno)) error.code += `_ERRNO_${detail.errno}`;
        if (Number.isSafeInteger(detail.curlcode)) error.code += `_CURL_${detail.curlcode}`;
      }
    } catch { /* Never include raw subprocess diagnostics or private requests. */ }
    throw error;
  }
}

function diagnoseOriginal(sessionId) {
  const diagnostic = {};
  for (const mode of ['original-result', 'reverse-proof', 'selection-source']) {
    try { diagnostic[mode] = probe(mode, sessionId); }
    catch (failure) { diagnostic[mode] = { error: failure.code || 'OWNED_PRIVATE_PROBE_FAILED' }; }
  }
  diagnostic.rows = row('shared', `SELECT
    (SELECT count(*) FROM customer_portal_contexts WHERE bff_session_id='${sessionId}') AS contexts,
    (SELECT count(*) FROM context_selections WHERE client_id='${ids.portalClient}' AND result IS NOT NULL) AS committed`);
  return diagnostic;
}

// Read-only diagnosis of the original encrypted operation; no consume, token
// refresh, artificial context, account generation change or browser is involved.
if (options['--diagnostic']) {
  const diagnostic = diagnoseOriginal(latestPortal('customer', true).id);
  console.log(`CUSTOMER_HTTPS_READONLY_DIAGNOSTIC ${JSON.stringify(diagnostic)}`);
  process.exit(0);
}

process.env.PLAYWRIGHT_BROWSERS_PATH = path.join(repository, 'frontend/tests/browser/.browsers');
const require = createRequire(path.join(repository, 'frontend/tests/browser/package.json'));
const { chromium } = require('playwright');
const report = { contractVersion: 'unified-auth-customer-real-https-browser/1', date: new Date().toISOString(),
  tlsErrorsIgnored: false, httpFixtures: false, sqlBoundarySetup: true, privateNegativeProbes: true,
  checks: [], endpoints: {}, result: 'FAILED' };
let browser;
let step = 'owned-database-preflight';
let originalPendingId;
let interrupted = false;
const contexts = [];
const observations = new Set();
const tlsProtocols = new Set();
function pass(name, details = {}) { report.checks.push({ name, result: 'PASS', ...details }); }
function observe(page) {
  page.on('response', response => {
    const url = new URL(response.url());
    if (![AUTH, CONSOLE, CUSTOMER].includes(url.origin)) return;
    const endpoint = url.pathname.replace(/\/[0-9a-f-]{36}(?=\/|$)/g, '/:id');
    if (/^\/(auth|oauth|api)\//.test(endpoint)) {
      const key = `${url.host}${endpoint} ${response.status()}`;
      report.endpoints[key] = (report.endpoints[key] || 0) + 1;
    }
    if (response.request().isNavigationRequest()) {
      const pending = response.securityDetails().then(value => { if (value?.protocol) tlsProtocols.add(value.protocol); })
        .catch(() => {}).finally(() => observations.delete(pending));
      observations.add(pending);
    }
  });
}
async function newContext() {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, serviceWorkers: 'block' });
  contexts.push(context);
  return context;
}
async function pageIn(context) {
  const page = await context.newPage();
  page.setDefaultTimeout(20_000);
  page.setDefaultNavigationTimeout(35_000);
  observe(page);
  return page;
}
function secretFree(value) {
  if (!value || typeof value !== 'object') return true;
  return Object.entries(value).every(([key, child]) => !/^(?:access_?token|refresh_?token|id_?token|customerContext)$/i.test(key) && secretFree(child));
}
async function http(page, endpoint) {
  assert(['/auth/session', '/api/profile'].includes(endpoint), 'FIXED_BROWSER_ENDPOINT_REQUIRED');
  return page.evaluate(async endpoint => {
    const response = await fetch(endpoint, { credentials: 'same-origin', cache: 'no-store', redirect: 'error', headers: { Accept: 'application/json' } });
    return { status: response.status, noStore: response.headers.get('cache-control')?.includes('no-store') === true,
      body: await response.json() };
  }, endpoint);
}
async function portalView(page, account = 'customer') {
  const response = await http(page, '/auth/session');
  assert(response.status === 200 && response.noStore && secretFree(response.body), 'CUSTOMER_SESSION_FAILED');
  const value = response.body;
  assert(value.contractVersion === 'portal-session/1' && value.account?.id === ids[account]
    && value.centralizedContextSelection === true && value.context?.memberType === 'CUSTOMER'
    && value.memberships.every(member => member.memberType === 'CUSTOMER' && member.userId === ids[account]), 'CUSTOMER_ISOLATION_FAILED');
  return value;
}
async function identityProgress(page, account, expectSso = false) {
  if (new URL(page.url()).origin !== AUTH) return;
  await page.waitForFunction(() => document.querySelector('#auth-content')?.dataset.screen
    && document.querySelector('#auth-content').dataset.screen !== 'LOADING');
  const login = page.locator('#login-form:not([hidden])');
  if (await login.isVisible()) {
    assert(!expectSso, 'EXISTING_SSO_WAS_NOT_REUSED');
    await page.locator('#username').fill(environment.accounts[account].username);
    await page.locator('#password').fill(environment.accounts[account].password);
    await page.locator('#login-submit').click();
    await page.waitForFunction(() => location.origin !== 'https://auth.saas.test'
      || document.querySelector('#auth-content')?.dataset.screen !== 'LOGIN_REQUIRED');
  }
  if (new URL(page.url()).origin === AUTH
      && await page.locator('#auth-content').getAttribute('data-screen') === 'CONSENT_REQUIRED') {
    await page.locator('#consent-allow').click();
  }
}
async function startPortal(page, account, expectSso = false) {
  await page.goto(CUSTOMER, { waitUntil: 'domcontentloaded' });
  await Promise.all([
    page.waitForURL(url => url.origin === AUTH, { waitUntil: 'domcontentloaded' }),
    page.getByTestId('portal-login').click(),
  ]);
  await identityProgress(page, account, expectSso);
}
async function portalArrived(page) {
  await page.waitForURL(url => url.origin === CUSTOMER && !url.pathname.startsWith('/auth/'), { waitUntil: 'domcontentloaded' });
}
async function consoleLogin(page) {
  await page.goto(`${CONSOLE}/auth/login?return=%2F`, { waitUntil: 'domcontentloaded' });
  await identityProgress(page, 'admin');
  await page.waitForURL(url => url.origin === CONSOLE && !url.pathname.startsWith('/auth/'), { waitUntil: 'domcontentloaded' });
  const response = await http(page, '/auth/session');
  assert(response.status === 200 && response.body.account?.id === ids.admin && response.body.context?.memberType === 'EMPLOYEE', 'CONSOLE_SESSION_FAILED');
  return response.body;
}
function interrupt() {
  interrupted = true;
  process.exitCode = 1;
  report.result = 'FAILED';
  report.failedStep = step;
  report.failureCode = 'BROWSER_INTERRUPTED';
  if (browser) void browser.close().catch(() => {});
}
process.on('SIGINT', interrupt);
process.on('SIGTERM', interrupt);

try {
  assert(sql('portal', `SELECT count(*) FROM information_schema.tables WHERE table_schema='public'
    AND table_name IN ('users','memberships','auth_clients','auth_grants')`) === '0', 'AUTHORITY_TABLE_LEAK');
  resetExtraMemberships();
  dropFault();
  const roles = probe('runtime-isolation');
  assert(roles.ownLogin && roles.sharedConnectionDenied && roles.restricted, 'PORTAL_DATABASE_ROLE_ISOLATION_FAILED');
  pass(step, { realRuntimeLogin: true, sharedDatabaseConnectionDenied: true });
  browser = await chromium.launch({ headless: true, proxy: { server: proxy } });
  report.browserVersion = browser.version();

  step = 'committed-platform-selection-survives-local-save-failure';
  installFault();
  const customerPage = await pageIn(await newContext());
  await startPortal(customerPage, 'customer');
  await portalArrived(customerPage);
  await customerPage.getByTestId('portal-unavailable').waitFor();
  assert((await http(customerPage, '/auth/session')).status === 503
    && (await http(customerPage, '/api/profile')).status === 503, 'PENDING_OPERATION_SERVED_PROFILE');
  step = 'load-original-customer-pending-session';
  const pendingSession = latestPortal('customer', true);
  originalPendingId = pendingSession.id;
  step = 'verify-real-platform-consumption-commit';
  const beforeRecovery = committedSelection(pendingSession.id);
  assert(beforeRecovery.consumed && Number(beforeRecovery.consumption_count) === 1, 'PLATFORM_DID_NOT_COMMIT_EXACTLY_ONCE');

  step = 'wrong-bff-version-rp-and-source-are-rejected';
  const denials = {};
  const expectedDenials = { 'wrong-bff': 403, 'wrong-version': 409, 'wrong-rp': 403, 'wrong-source': 401 };
  for (const mode of Object.keys(expectedDenials)) {
    const result = probe(mode, pendingSession.id);
    assert(result.status === expectedDenials[mode], 'PRIVATE_BOUNDARY_WRONG_BINDING_STATUS_CHANGED');
    denials[mode] = result.status;
  }
  const unchanged = committedSelection(pendingSession.id);
  assert(unchanged.context_id === beforeRecovery.context_id && unchanged.capability_digest === beforeRecovery.capability_digest
    && unchanged.capability_expiry === beforeRecovery.capability_expiry && Number(unchanged.consumption_count) === 1, 'DENIAL_CHANGED_OR_REISSUED_ORIGINAL_RESULT');
  pass(step, { statuses: denials });

  step = 'same-operation-recovers-same-capability-without-expiry-extension';
  dropFault();
  await customerPage.getByTestId('portal-retry').click();
  await customerPage.getByTestId('portal-profile').waitFor();
  let customer = await portalView(customerPage);
  const recovered = latestPortal();
  const afterRecovery = committedSelection(pendingSession.id);
  assert(recovered.id === pendingSession.id && recovered.version === pendingSession.version && !recovered.pending
    && customer.context.membershipId === ids.customerMember && customer.memberships.length === 1,
  'RECOVERY_CHANGED_SESSION_OR_SINGLE_CUSTOMER_SELECTION');
  assert(afterRecovery.context_id === beforeRecovery.context_id && afterRecovery.capability_digest === beforeRecovery.capability_digest
    && afterRecovery.capability_expiry === beforeRecovery.capability_expiry && afterRecovery.source_expiry === beforeRecovery.source_expiry
    && Number(afterRecovery.consumption_count) === 1, 'RECOVERY_REISSUED_OR_EXTENDED_CAPABILITY');
  const profile = await http(customerPage, '/api/profile');
  assert(profile.status === 200 && profile.noStore && secretFree(profile.body)
    && profile.body.account.id === ids.customer && profile.body.context.membershipId === ids.customerMember, 'SELF_PROFILE_FAILED');
  pass('committed-platform-selection-survives-local-save-failure', {
    platformCommittedBeforePortalSave: true, portalSessionStatus: 503, portalProfileStatus: 503, encryptedPending: true,
  });
  pass(step, { consumptionCount: 1, sameCapability: true, expiryExtended: false });

  step = 'customer-online-refresh-keeps-original-member';
  const beforeGeneration = Number(sql('shared', `SELECT max(t.generation) FROM auth_refresh_tokens t
    JOIN auth_refresh_families f ON f.id=t.family_id WHERE f.grant_id='${recovered.oidc_grant_id}'`));
  sql('portal', `UPDATE portal_sessions SET access_expires_at=LEAST(access_expires_at,clock_timestamp()+interval '20 seconds')
    WHERE id='${recovered.id}' AND user_id='${ids.customer}'`);
  customer = await portalView(customerPage);
  const renewed = latestPortal();
  const afterGeneration = Number(sql('shared', `SELECT max(t.generation) FROM auth_refresh_tokens t
    JOIN auth_refresh_families f ON f.id=t.family_id WHERE f.grant_id='${recovered.oidc_grant_id}'`));
  assert(afterGeneration > beforeGeneration && renewed.id === recovered.id && renewed.oidc_grant_id === recovered.oidc_grant_id
    && Date.parse(renewed.expires_at) <= Date.parse(recovered.expires_at) && customer.context.membershipId === ids.customerMember,
  'CUSTOMER_REFRESH_EXPANDED_OR_CHANGED_SOURCE');
  pass(step, { refreshFamilyRotated: true, sameOriginalMember: true });

  step = 'customer-real-central-context-switch';
  const secondMember = newId();
  sql('shared', `INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES('${secondMember}','${ids.customer}','${ids.secondOrganization}','CUSTOMER')
    ON CONFLICT(user_id,organization_id,member_type) DO UPDATE SET status='ENABLED',identity_version=memberships.identity_version+1
    WHERE memberships.status<>'ENABLED'`);
  const actualSecond = sql('shared', `SELECT id FROM memberships WHERE user_id='${ids.customer}'
    AND organization_id='${ids.secondOrganization}' AND member_type='CUSTOMER' AND status='ENABLED'`);
  assert(uuid7.test(actualSecond), 'SECOND_CUSTOMER_MEMBER_REQUIRED');
  await customerPage.getByTestId('portal-central-switch').click();
  await customerPage.waitForURL(url => url.origin === AUTH, { waitUntil: 'domcontentloaded' });
  await customerPage.locator(`input[name="context-membership"][value="${actualSecond}"]`).check();
  await customerPage.locator('#consent-allow').click();
  await portalArrived(customerPage);
  await customerPage.getByTestId('portal-profile').waitFor();
  customer = await portalView(customerPage);
  assert(customer.context.membershipId === actualSecond && customer.context.organization.id === ids.secondOrganization, 'CUSTOMER_SWITCH_FAILED');
  pass(step);

  step = 'employee-cannot-establish-customer-session';
  const employeePage = await pageIn(await newContext());
  await startPortal(employeePage, 'employee');
  await employeePage.locator('#selection-empty:not([hidden]), #error-message:not([hidden])').first().waitFor();
  assert(await employeePage.locator('input[name="context-membership"]').count() === 0, 'EMPLOYEE_RECEIVED_CUSTOMER_CHOICES');
  assert(!(await employeePage.context().cookies(CUSTOMER)).some(cookie => cookie.name === '__Host-saas_customer'), 'EMPLOYEE_RECEIVED_CUSTOMER_COOKIE');
  assert(sql('portal', `SELECT count(*) FROM portal_sessions WHERE user_id='${ids.employee}'`) === '0', 'EMPLOYEE_CUSTOMER_SESSION_CREATED');
  pass(step);

  step = 'same-browser-sso-with-independent-console-and-customer-cookies';
  assert(uuid7.test(ids.adminCustomerMember) && sql('shared', `SELECT count(*) FROM memberships
    WHERE id='${ids.adminCustomerMember}' AND user_id='${ids.admin}' AND organization_id='${ids.organization}'
      AND member_type='CUSTOMER' AND status='ENABLED'`) === '1', 'DUAL_ADMIN_FIXTURE_REQUIRED');
  const sharedContext = await newContext();
  const consolePage = await pageIn(sharedContext);
  const consoleBefore = await consoleLogin(consolePage);
  const ssoBefore = (await sharedContext.cookies(AUTH)).find(cookie => cookie.name === '__Host-saas_sso');
  assert(ssoBefore?.secure && ssoBefore.httpOnly, 'SSO_COOKIE_REQUIRED');
  const dualPortal = await pageIn(sharedContext);
  await startPortal(dualPortal, 'admin', true);
  await portalArrived(dualPortal);
  await dualPortal.getByTestId('portal-profile').waitFor();
  const dualView = await portalView(dualPortal, 'admin');
  assert(dualView.context.membershipId === ids.adminCustomerMember, 'ADMIN_CUSTOMER_IDENTITY_MISMATCH');
  const consoleAfter = await http(consolePage, '/auth/session');
  const dualSession = latestPortal('admin');
  const consoleSource = sql('shared', `SELECT global_session_id FROM bff_sessions WHERE id='${consoleBefore.sessionId}' AND user_id='${ids.admin}'`);
  const cookies = await sharedContext.cookies([AUTH, CONSOLE, CUSTOMER]);
  for (const [name, domain] of [['__Host-saas_console', 'console.saas.test'], ['__Host-saas_customer', 'customer.saas.test']]) {
    const cookie = cookies.find(cookie => cookie.name === name);
    assert(cookie?.domain === domain && cookie.secure && cookie.httpOnly && cookie.path === '/', 'INDEPENDENT_HOST_COOKIE_FAILED');
  }
  assert(consoleAfter.status === 200 && consoleAfter.body.sessionId === consoleBefore.sessionId
    && consoleAfter.body.version === consoleBefore.version && consoleAfter.body.context.memberType === 'EMPLOYEE'
    && dualSession.global_session_id === consoleSource
    && cookies.find(cookie => cookie.name === '__Host-saas_sso')?.value === ssoBefore.value, 'SHARED_SSO_OVERWROTE_INDEPENDENT_SESSION');
  pass(step, { sharedSso: true, independentHostCookies: true, employeeAndCustomerContextsSeparate: true });

  step = 'original-customer-source-revocation-fails-closed';
  const active = latestPortal();
  sql('shared', `UPDATE sso_sessions SET revoked_at=COALESCE(revoked_at,clock_timestamp())
    WHERE id='${active.global_session_id}' AND user_id='${ids.customer}'`);
  assert((await http(customerPage, '/auth/session')).status === 401 && (await http(customerPage, '/api/profile')).status === 401,
    'REVOKED_CUSTOMER_SOURCE_ACCEPTED');
  await customerPage.reload({ waitUntil: 'domcontentloaded' });
  await customerPage.getByTestId('portal-signed-out').waitFor();
  assert(await customerPage.getByTestId('portal-profile').count() === 0, 'REVOKED_SOURCE_LEFT_PROFILE_VISIBLE');
  assert((await http(consolePage, '/auth/session')).status === 200 && (await http(dualPortal, '/api/profile')).status === 200,
    'OTHER_ACCOUNT_SESSION_WAS_REVOKED');
  pass(step, { otherAccountUnaffected: true });

  step = 'browser-keeps-no-upstream-credentials';
  const safe = await dualPortal.evaluate(() => [localStorage, sessionStorage].every(storage =>
    !Object.entries(storage).some(([key, value]) => /token|password|secret|verifier/i.test(key)
      || /(?:access_token|refresh_token|id_token)/.test(value))));
  assert(safe, 'UPSTREAM_CREDENTIAL_PERSISTENCE_FOUND');
  await Promise.allSettled([...observations]);
  assert(tlsProtocols.size > 0 && [...tlsProtocols].every(protocol => /^TLS [12]\./.test(protocol)), 'TLS_EVIDENCE_REQUIRED');
  report.tlsProtocols = [...tlsProtocols];
  pass(step);
  report.result = 'PASS';
} catch (error) {
  report.failedStep = step;
  report.failureCode = interrupted ? 'BROWSER_INTERRUPTED'
    : error?.code === 'ERR_ASSERTION' && /^[A-Z0-9_]+$/.test(error.message) ? error.message
      : /^(?:SQLSTATE_[0-9A-Z]{5}|OWNED_PROBE_[A-Z_0-9]+)$/.test(error?.code || '') ? error.code
        : error?.name === 'TimeoutError' ? 'BROWSER_STEP_TIMEOUT' : 'BROWSER_STEP_FAILED';
  if (step === 'verify-real-platform-consumption-commit' && originalPendingId) {
    try { report.originalOperationDiagnostic = diagnoseOriginal(originalPendingId); }
    catch { report.originalOperationDiagnostic = { error: 'OWNED_READONLY_DIAGNOSTIC_FAILED' }; }
  }
  process.exitCode = 1;
} finally {
  if (interrupted) report.result = 'FAILED';
  for (const context of contexts) await context.close().catch(() => {});
  if (browser) await browser.close().catch(() => {});
  try {
    dropFault();
    resetExtraMemberships();
    report.fixtureBoundaryReset = true;
  } catch {
    report.fixtureBoundaryReset = false;
    report.result = 'FAILED';
    process.exitCode = 1;
  }
  fs.mkdirSync(path.dirname(outputFile), { recursive: true });
  fs.writeFileSync(outputFile, `${JSON.stringify(report, null, 2)}\n`);
  console.log(`CUSTOMER_HTTPS_BROWSER_RESULT ${JSON.stringify({ result: report.result, checks: report.checks.length,
    ...(report.failedStep ? { failedStep: report.failedStep, failureCode: report.failureCode } : {}) })}`);
}
