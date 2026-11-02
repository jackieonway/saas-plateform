/** Actual Phase20 TLS acceptance against one newly prepared, owned fixture.
 * Credentials, invitation material and signed tokens stay in process memory.
 * This file never creates an environment, bypasses TLS or exports browser state.
 */
import assert from 'node:assert/strict';
import path from 'node:path';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { createHash, createHmac, randomBytes, sign } from 'node:crypto';
import { loadOwnedBrowserInputs, readOwnedBrowserJson, validateBrowserReport, saveBrowserReport } from './phase25-browser-binding.mjs';

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const options = {};
for (let i = 2; i < process.argv.length; i += 2) {
  assert(['--environment', '--output', '--proxy', '--browser', '--phase', '--snapshot'].includes(process.argv[i]) && process.argv[i + 1], 'INVALID_ARGUMENTS');
  assert(!Object.hasOwn(options, process.argv[i]), 'DUPLICATE_ARGUMENT'); options[process.argv[i]] = process.argv[i + 1];
}
const environmentFile = options['--environment'];
const executionPhase = Number(options['--phase'] ?? 20); assert([20, 25].includes(executionPhase), 'EXECUTION_PHASE_INVALID');
assert(/^\/tmp\/rust-saas-unified-auth-20261028\/(?:[A-Za-z0-9_.-]+\/)*https-environment\.json$/.test(environmentFile || '') && !environmentFile.split('/').includes('..'), 'OWNED_ENVIRONMENT_REQUIRED');
const outputFile = path.resolve(options['--output'] || '');
assert(outputFile.endsWith('.json') && !path.basename(outputFile).startsWith('https-'), 'SAFE_REPORT_REQUIRED');
validateBrowserReport({ outputFile, executionPhase, legacyPhase: 20 });
const proxy = options['--proxy'] || 'http://127.0.0.1:24445'; assert(proxy === 'http://127.0.0.1:24445', 'OWNED_PROXY_REQUIRED');
const product = options['--browser'] || 'chromium'; assert(['chromium', 'chrome', 'msedge'].includes(product), 'BROWSER_REQUIRED');
const AUTH = 'https://auth.saas.test', CONSOLE = 'https://console.saas.test', CUSTOMER = 'https://customer.saas.test';
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const hash = value => createHash('sha256').update(value).digest('hex');
const lit = value => `'${String(value).replaceAll("'", "''")}'`;
function newId() {
  const b = randomBytes(16); b.writeUIntBE(Date.now(), 0, 6); b[6] = (b[6] & 15) | 112; b[8] = (b[8] & 63) | 128;
  const h = b.toString('hex'); return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}
const ownedInputs = loadOwnedBrowserInputs({ environmentFile, executionPhase, legacyPhase: 20,
  snapshot: options['--snapshot'], entrypoint: fileURLToPath(import.meta.url) });
const privateJson = readOwnedBrowserJson;
const { environment, manifest } = ownedInputs;
const taskDirectory = path.posix.dirname(environmentFile), ids = environment.ids;
assert(environment.phase20?.enabled && environment.phase20.contractVersion === 'unified-auth-phase20/1' && environment.portal && manifest.portal, 'FRESH_PHASE20_REQUIRED');
assert(/^unified_auth_https_[0-9a-f]{32}$/.test(manifest.database) && manifest.portal.database === manifest.database.replace('unified_auth_https_', 'unified_auth_portal_') && manifest.clusterId === ownedInputs.clusterId, 'OWNED_DATABASE_REQUIRED');
for (const key of ['deployment', 'admin', 'employee', 'customer','reviewer','reviewerMember','organization','client', 'portalClient']) assert(uuid.test(ids[key]), 'FIXTURE_ID_REQUIRED');
assert(![ids.admin,ids.employee,ids.customer].includes(ids.reviewer),'OWNED_REVIEWER_MUST_BE_DISTINCT');
const db = { shared: [manifest.database, `identity-deployment/1/${ids.deployment}`], portal: [manifest.portal.database, manifest.portal.marker] };
function sql(kind, statement) {
  const [name, marker] = db[kind] || []; assert(name && marker, 'FIXED_DATABASE_REQUIRED');
  const actorGuard=kind==='shared'?` OR (SELECT count(*) FROM users subject JOIN memberships member ON member.user_id=subject.id WHERE subject.id=${lit(ids.reviewer)}::uuid AND NOT subject.is_super_admin AND subject.username=${lit(environment.accounts.reviewer.username)} AND member.id=${lit(ids.reviewerMember)}::uuid AND member.organization_id=${lit(ids.organization)}::uuid AND member.member_type='EMPLOYEE')<>1`:'';
  const input = `BEGIN; DO $$ BEGIN IF current_database()<>${lit(name)} OR (SELECT system_identifier::text FROM pg_control_system())<>${lit(manifest.clusterId)} OR current_setting('data_directory')<>${lit(ownedInputs.dataDirectory)} OR (SELECT shobj_description(oid,'pg_database') FROM pg_database WHERE datname=current_database()) IS DISTINCT FROM ${lit(marker)}${actorGuard} THEN RAISE EXCEPTION 'owned phase20 guard failed'; END IF; END $$; ${statement}; COMMIT;`;
  try { return execFileSync(process.platform === 'linux' ? 'psql' : 'wsl.exe', [...(process.platform === 'linux' ? [] : ['-u', 'root', '--', 'psql']), '-X', '-q', '-v', 'ON_ERROR_STOP=1', '-v', 'VERBOSITY=sqlstate', '-At', `postgres://postgres@127.0.0.1:55494/${name}`], { input, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true, timeout: 15_000 }).trim(); }
  catch (failure) { const error = new Error('OWNED_SQL_FAILED'); error.code = /ERROR:\s+([A-Z0-9]{5})\b/.exec(String(failure.stderr || ''))?.[1]; throw error; }
}
const row = (kind, statement) => JSON.parse(sql(kind, `SELECT row_to_json(facts) FROM (${statement}) facts`));
async function until(predicate, code, timeout = 15_000) { const deadline = Date.now() + timeout; for (;;) { if (await predicate()) return; assert(Date.now() < deadline, code); await delay(150); } }
function safe(value) { return !value || typeof value !== 'object' || Object.entries(value).every(([k, v]) => !/^(?:token|password|secret|seed|recoveryCodes|clientSecret|access_?token|refresh_?token|id_?token|logout_token)$/i.test(k) && safe(v)); }

// The namespace and fixture checks precede every private HTTP/control request.
// Subprocess stdout is captured in Node memory; raw stderr is never reported.
const privateWire = String.raw`
import base64,json,os,pathlib,re,subprocess,sys,urllib.parse
try:
 r=json.load(sys.stdin);root=pathlib.Path(r['environment']).resolve().parent
 if not str(root).startswith('/tmp/rust-saas-unified-auth-20261028') or not (root/'OWNER').exists() and not (root.parent/'OWNER').exists():raise ValueError()
 if os.readlink('/proc/self/ns/mnt')!=(root/'namespace.identity').read_text().strip():raise ValueError()
 e=json.loads((root/'https-environment.json').read_text());c=e['phase20']['collector'];mode=r['mode'];headers={'Accept':'application/json'}
 if mode=='control':
  base=c['controlUrl'];u=urllib.parse.urlparse(base)
  if u.scheme!='http' or u.hostname!='127.0.0.1' or u.port!=23443:raise ValueError()
  if r['path'] not in ['/control','/control/stats','/control/peek']:raise ValueError()
  url=base+r['path'];headers['X-Task-Control-Key']=c['controlSecret'];data=None if r.get('body') is None else json.dumps(r['body']).encode();headers['Content-Type']='application/json'
 else:
  if r['path'] not in ['/oauth/token','/oauth/logout'] and not re.fullmatch('/oauth/logout/operations/[0-9a-f-]{36}',r['path']):raise ValueError()
  url='https://auth.saas.test'+r['path'];data=urllib.parse.urlencode(r['form']).encode();headers['Content-Type']='application/x-www-form-urlencoded';headers['Authorization']='Basic '+base64.b64encode((r['client']+':'+r['credential']).encode()).decode()
 config=[('url',url)]+[('header',key+': '+value) for key,value in headers.items()]
 if data is not None:config.append(('data',data.decode()))
 curl_config='\n'.join(key+' = '+json.dumps(value) for key,value in config)+'\n'
 command=['curl','--disable','--silent','--max-time','10','--noproxy','*','--cacert',e['identity']['AUTH_DELIVERY_CA_FILE'],'--proto','=http,https','--max-redirs','0','--request',r.get('method','POST'),'--write-out','\n%{http_code}','--config','-']
 response=subprocess.run(command,input=curl_config,text=True,capture_output=True,timeout=12)
 if response.returncode:raise ValueError()
 raw,status=response.stdout.rsplit('\n',1)
 if len(raw)>65536:raise ValueError()
 try:body=json.loads(raw) if raw else None
 except ValueError:body=None
 print(json.dumps({'status':int(status),'body':body}))
except Exception:print(json.dumps({'error':'OWNED_WIRE_FAILED'}));sys.exit(1)
`;
function wire(request) {
  const record = privateJson(`${taskDirectory}/namespace.pid`); assert(Number.isSafeInteger(record.pid) && record.pid > 0, 'OWNED_NAMESPACE_REQUIRED');
  try { return JSON.parse(execFileSync(process.platform === 'linux' ? 'nsenter' : 'wsl.exe', [...(process.platform === 'linux' ? [] : ['-u', 'root', '--exec', 'nsenter']), '-t', String(record.pid), '-m', '--', 'python3', '-c', privateWire], { input: JSON.stringify({ environment: environmentFile, ...request }), encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true, timeout: 20_000 })); }
  catch { throw new Error('OWNED_PRIVATE_HTTP_FAILED'); }
}
const control = (operation, body) => { const result = wire({ mode: 'control', path: operation, method: body === undefined ? 'GET' : 'POST', body }); assert(result.status === 200, 'COLLECTOR_CONTROL_FAILED'); return result.body; };
const mode = (kind, status, delayMs = 0) => control('/control', { operation: 'setMode', kind, status, delayMs });

process.env.PLAYWRIGHT_BROWSERS_PATH = path.join(repository, 'frontend/tests/browser/.browsers');
const require = createRequire(path.join(repository, 'frontend/tests/browser/package.json'));
const { chromium } = require('playwright');
const report = { contractVersion: 'unified-auth-phase20-real-https/1', date: new Date().toISOString(), tlsErrorsIgnored: false, httpFixtures: false, positiveMfaSeededBySql: false, fixtureSignedNegativeTokens: true, checks: [], endpoints: {}, result: 'FAILED' };
let browser, step = 'fixture-preflight', activePage, interrupted = false,mfaCleanupEligible=false;
const contexts = [], protocols = new Set(), observations = new Set(); let factor;
function pass(facts = {}) { report.checks.push({ name: step, result: 'PASS', ...facts }); }
function observe(page) { activePage = page; page.on('response', response => { const url = new URL(response.url()); if (![AUTH, CONSOLE, CUSTOMER].includes(url.origin)) return; const name = `${url.pathname.replace(/\/[0-9a-f-]{36}(?=\/|$)/g, '/:id')} ${response.status()}`; report.endpoints[name] = (report.endpoints[name] || 0) + 1; if (response.request().isNavigationRequest()) { const pending = response.securityDetails().then(value => { if (value?.protocol) protocols.add(value.protocol); }).catch(() => {}).finally(() => observations.delete(pending)); observations.add(pending); } }); }
async function pageIn(context) { const page = await context.newPage(); page.setDefaultTimeout(25_000); page.setDefaultNavigationTimeout(40_000); observe(page); return page; }
async function newContext() { const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, serviceWorkers: 'block' }); contexts.push(context); return context; }
async function json(page, endpoint, method = 'GET', body, extra = {}, omit = false) { return page.evaluate(async args => { const response = await fetch(args.endpoint, { method: args.method, credentials: args.omit ? 'omit' : 'same-origin', cache: 'no-store', redirect: 'error', headers: { Accept: 'application/json', ...(args.body === undefined ? {} : { 'Content-Type': 'application/json' }), ...args.extra }, ...(args.body === undefined ? {} : { body: JSON.stringify(args.body) }) }); let value = null; try { value = await response.json(); } catch {} return { status: response.status, noStore: response.headers.get('cache-control')?.includes('no-store') === true, body: value }; }, { endpoint, method, body, extra, omit }); }
async function view(page, account, origin) { const response = await json(page, '/auth/session'); assert(response.status === 200 && response.noStore && response.body.account?.id === ids[account] && safe(response.body), 'REAL_SESSION_FAILED'); if (origin === CONSOLE) assert(response.body.context?.memberType === 'EMPLOYEE', 'EMPLOYEE_CONTEXT_REQUIRED'); else assert(response.body.context?.memberType === 'CUSTOMER', 'CUSTOMER_CONTEXT_REQUIRED'); return response.body; }
async function identity(page, account, destination, primary = false) {
  activePage = page;
  if(primary)await page.locator('#login-form:not([hidden])').waitFor();
  else {const ready=await page.waitForFunction(dest=>location.origin===dest||!!document.querySelector('#login-form:not([hidden])')||(document.querySelector('#auth-content')?.dataset.screen==='CONSENT_REQUIRED'&&!document.querySelector('#consent-panel')?.hidden&&!document.querySelector('#consent-allow')?.disabled)||!!document.querySelector('#error-message:not([hidden])'),destination);await ready.dispose();}
  if (new URL(page.url()).origin === AUTH && await page.locator('#login-form:not([hidden])').isVisible()) {
    assert(!primary || account === 'reviewer', 'SSO_REUSE_EXPECTED');
    await page.locator('#username').fill(environment.accounts[account].username); await page.locator('#password').fill(environment.accounts[account].password);
    const first=page.waitForResponse(response=>response.url()===`${AUTH}/api/sso/login`&&response.request().method()==='POST');await page.locator('#login-submit').click();assert((await first).status()===(primary?428:200),'REAL_PRIMARY_AUTHENTICATION_STATUS_FAILED');
    if (primary) { await page.locator('#mfa-field:not([hidden])').waitFor(); await page.locator('#password').fill(environment.accounts[account].password); await page.locator('#mfa-code').fill(await nextCode());const accepted=page.waitForResponse(response=>response.url()===`${AUTH}/api/sso/login`&&response.request().method()==='POST'); await page.locator('#login-submit').click();assert((await accepted).status()===200,'REAL_PASSWORD_TOTP_AUTHENTICATION_FAILED'); }
  }
  const transition=await page.waitForFunction(dest=>{if(location.origin===dest&&!location.pathname.startsWith('/auth/'))return 'destination';if(document.querySelector('#error-message:not([hidden])'))return 'error';if(document.querySelector('#auth-content')?.dataset.screen==='CONSENT_REQUIRED'&&!document.querySelector('#consent-panel')?.hidden&&document.querySelector('#consent-allow')?.disabled===false)return 'consent';return false;},destination);const outcome=await transition.jsonValue();await transition.dispose();assert(outcome!=='error','REAL_IDENTITY_FLOW_REJECTED');if(outcome==='consent')await page.locator('#consent-allow').click();
  await page.waitForURL(url => url.origin === destination && !url.pathname.startsWith('/auth/'), { waitUntil: 'domcontentloaded' });
}
async function consoleLogin(page, account, stepUp = false) {if(stepUp&&new URL(page.url()).origin!==CONSOLE)await page.goto(`${CONSOLE}/auth/config`,{waitUntil:'domcontentloaded'});const before=stepUp?source('console',(await view(page,account,CONSOLE)).sessionId):null; await page.goto(`${CONSOLE}${stepUp ? '/auth/step-up?return=/system/oauth/clients' : '/auth/login?return=/'}`, { waitUntil: 'domcontentloaded' }); await identity(page, account, CONSOLE, stepUp); const result = await view(page, account, CONSOLE);if(before){const after=source('console',result.sessionId);assert(after.id!==before.id&&after.global_session_id!==before.global_session_id&&after.selected_session_id!==before.selected_session_id,'STEPUP_DID_NOT_REPLACE_ORIGINAL_SOURCE');assert(sql('shared',`SELECT mfa_method='totp' AND mfa_verified_at>clock_timestamp()-interval '5 minutes' FROM sso_sessions WHERE id=${lit(after.global_session_id)}::uuid`)==='t','CURRENT_ORIGINAL_SOURCE_HAS_NO_REAL_TOTP');} await page.goto(`${CONSOLE}/auth/config`, { waitUntil: 'domcontentloaded' }); return result; }
async function portalLogin(page, account) { await page.goto(CUSTOMER, { waitUntil: 'domcontentloaded' }); await Promise.all([page.waitForURL(url => url.origin === AUTH, { waitUntil: 'domcontentloaded' }), page.getByTestId('portal-login').click()]); await identity(page, account, CUSTOMER); const result = await view(page, account, CUSTOMER); await page.goto(`${CUSTOMER}/auth/config`, { waitUntil: 'domcontentloaded' });return {...result,fixtureSessionId:sql('portal',`SELECT id FROM portal_sessions WHERE user_id=${lit(ids[account])}::uuid AND revoked_at IS NULL ORDER BY created_at DESC,id DESC LIMIT 1`)}; }
async function logout(page, source, scope) { const response = await json(page, '/auth/logout', 'POST', { expectedVersion: source.version, scope }, { 'X-CSRF-Token': source.csrfToken }); assert(response.status === 200 && safe(response.body), 'REAL_LOGOUT_COMMAND_FAILED'); return response; }
function source(kind, id) { assert(uuid.test(id), 'ORIGINAL_BFF_ID_REQUIRED'); return row(kind === 'console' ? 'shared' : 'portal', `SELECT id,oidc_grant_id,oidc_public_sid,oidc_public_sub,global_session_id,${kind==='console'?'selected_session_id':'NULL::uuid AS selected_session_id'},revoked_at IS NOT NULL AS revoked FROM ${kind === 'console' ? 'bff_sessions' : 'portal_sessions'} WHERE id=${lit(id)}::uuid`); }
function signedLogout(client, sid, sub, changes = {}) { const now = Math.floor(Number(sql('shared', 'SELECT extract(epoch FROM clock_timestamp())'))); const keyset = privateJson(environment.identity.OIDC_KEYSET_PATH), key = keyset.keys.find(value => value.kid === 'owned-https'); assert(key, 'OWNED_SIGNING_KEY_REQUIRED'); const encoded = [Buffer.from(JSON.stringify({ alg: 'RS256', kid: key.kid, typ: 'logout+jwt' })).toString('base64url'), Buffer.from(JSON.stringify({ iss: AUTH, aud: client, sid, sub, iat: now, exp: now + 120, jti: newId(), events: { 'http://schemas.openid.net/event/backchannel-logout': {} }, ...changes })).toString('base64url')].join('.'); const signature = sign('RSA-SHA256', Buffer.from(encoded), key.privateKeyPem).toString('base64url'); key.privateKeyPem = ''; return `${encoded}.${signature}`; }
async function receiver(page, token) { return page.evaluate(async token => { const response = await fetch('/auth/backchannel-logout', { method: 'POST', credentials: 'omit', cache: 'no-store', redirect: 'error', headers: { 'Content-Type': 'application/x-www-form-urlencoded' }, body: new URLSearchParams({ logout_token: token }) }); let body = null; try { body = await response.json(); } catch {} return { status: response.status, body }; }, token); }
function base32(raw) { let n = 0, bits = 0; const bytes = []; assert(/^[A-Z2-7]{16,128}$/.test(raw), 'ENROLLMENT_SEED_INVALID'); for (const c of raw) { n = (n << 5) | 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'.indexOf(c); bits += 5; if (bits >= 8) { bits -= 8; bytes.push((n >>> bits) & 255); } } return Buffer.from(bytes); }
async function nextCode() { for (;;) { const seconds = Number(sql('shared', 'SELECT extract(epoch FROM clock_timestamp())')), counter = Math.floor(seconds / 30); if (counter > factor.last && seconds % 30 < 27) { factor.last = counter; const message = Buffer.alloc(8); message.writeBigUInt64BE(BigInt(counter)); const digest = createHmac('sha1', factor.key).update(message).digest(), offset = digest[19] & 15; const code = ((digest.readUInt32BE(offset) & 0x7fffffff) % 1_000_000).toString().padStart(6, '0'); digest.fill(0); message.fill(0); return code; } await delay(Math.min(30_000, Math.max(250, (30 - seconds % 30) * 1000 + 150))); } }
async function enrollReviewer(page) { await page.goto(`${AUTH}/sso/security`, { waitUntil: 'domcontentloaded' }); await page.locator('#security-panel:not([hidden])').waitFor(); await page.locator('#start-enrollment').click(); await page.locator('#enrollment-panel:not([hidden])').waitFor(); factor = { key: base32((await page.locator('#secret').textContent()).trim()), last: -1 }; await page.locator('#confirm-code').fill(await nextCode()); await page.locator('#confirm-form button[type=submit]').click(); await page.locator('#recovery-panel:not([hidden])').waitFor(); await page.locator('#recovery-done').click(); await page.evaluate(() => { document.querySelector('#secret').textContent = ''; document.querySelector('#otpauth-uri').textContent = ''; document.querySelector('#recovery-codes').textContent = ''; }); await consoleLogin(page, 'reviewer', true); }
async function invitationCommand(page, endpoint, body) { const session = await view(page, 'reviewer', CONSOLE); const capability = await json(page, '/auth/context-token', 'POST', { expectedVersion: session.version }, { 'X-CSRF-Token': session.csrfToken }); assert(capability.status === 200, 'REAL_ADMIN_CAPABILITY_FAILED'); const operation = newId(); const response = await json(page, endpoint, 'POST', body, { Authorization: `Bearer ${capability.body.accessToken}`, 'X-Identity-Operation': operation }); assert(safe(response.body), 'MANAGEMENT_SECRET_LEAK'); if (response.status === 200) return response.body; assert(response.status === 503 && response.body?.code === 'OIDC_REMOTE_RESULT_UNKNOWN', 'INVITATION_COMMAND_FAILED'); for (let i = 0; i < 12; i++) { const lookup = await json(page, `/api/identity/invitation-operations/${operation}`, 'GET', undefined, { Authorization: `Bearer ${capability.body.accessToken}` }); if (lookup.status === 200 && lookup.body?.state === 'COMPLETED') return lookup.body.result; assert(lookup.status === 503, 'INVITATION_LOOKUP_FAILED'); await delay(500); } assert.fail('INVITATION_OPERATION_PENDING'); }
async function verifyInvitation(id) { const verification = newId(), target = environment.phase20.targets.invitationChannel; assert(uuid.test(id) && uuid.test(target), 'OWNED_INVITATION_BINDING_REQUIRED'); sql('shared', `INSERT INTO auth_invitation_verifications(id,invitation_id,recipient_ref,purpose,delivery_target_id,delivery_target_version,verified_reference,verified_at,expires_at,status) SELECT ${lit(verification)}::uuid,invitation.id,invitation.recipient_ref,invitation.purpose,target.id,target.version,${lit(newId())}::uuid,clock_timestamp(),LEAST(invitation.expires_at,target.expires_at),'ACTIVE' FROM auth_invitations invitation JOIN auth_delivery_targets target ON target.id=${lit(target)}::uuid AND target.recipient_ref=invitation.recipient_ref WHERE invitation.id=${lit(id)}::uuid AND invitation.recipient_ref=${lit(environment.phase20.recipientRef)} AND invitation.status='ACTIVE'`);
  await until(()=>sql('shared',`SELECT state<>'LEASED' FROM auth_delivery_jobs WHERE invitation_id=${lit(id)}::uuid`)==='t','INVITATION_ORIGINAL_LEASE_DID_NOT_SETTLE');
  sql('shared',`UPDATE auth_delivery_jobs SET available_at=clock_timestamp() WHERE invitation_id=${lit(id)}::uuid AND state='PENDING' AND (attempt_count=0 OR last_error_code='INVITATION_VERIFICATION_PENDING')`);return verification; }
async function peekInvitation(id) { let result; await until(() => { const value=wire({mode:'control',path:'/control/peek',method:'POST',body:{kind:'INVITATION',invitationId:id}}); assert([200,404].includes(value.status),'COLLECTOR_PEEK_FAILED');result=value.body?.message; return result?.invitationId === id; }, 'REAL_INVITATION_NOT_RECEIVED'); return result; }
function ownedCollectorRp() {
  const rp={id:newId(),application:newId(),group:newId(),target:newId(),secret:randomBytes(32).toString('hex'),redirect:`${AUTH}/sso/applications`};
  const url='https://auth.saas.test:24444/internal/task-delivery/backchannel';
  sql('shared',`INSERT INTO auth_applications(application_id,ownership,status,display_name,purpose,approved_scopes) VALUES(${lit(rp.application)}::uuid,'FIRST_PARTY','APPROVED','Owned Phase20 delivery RP','ACCOUNT_LOGIN',ARRAY['openid','profile']);
    INSERT INTO auth_clients(id,client_id,display_name,application_id,environment,client_type,runtime_kind,subject_mode,context_mode,grant_types,token_endpoint_auth_method,redirect_uris,status) VALUES(${lit(rp.id)}::uuid,${lit(rp.id)},'Owned delivery RP',${lit(rp.application)}::uuid,'test','CONFIDENTIAL','WEB_BFF','USER','ACCOUNT_ONLY',ARRAY['authorization_code','refresh_token'],'client_secret_basic',ARRAY[${lit(rp.redirect)}],'ENABLED');
    INSERT INTO auth_client_credentials(id,client_id,credential_version,secret_digest) VALUES(${lit(newId())}::uuid,${lit(rp.id)}::uuid,1,${lit(hash(rp.secret))});
    INSERT INTO auth_client_permissions(client_id,resource_id,scope_id) SELECT ${lit(rp.id)}::uuid,scope.resource_id,scope.id FROM auth_scopes scope JOIN auth_resources resource ON resource.id=scope.resource_id WHERE resource.resource_key='userinfo' AND scope.scope_name=ANY(ARRAY['openid','profile']);
    INSERT INTO auth_consent_groups(id,application_id,environment,display_name,purpose,context_mode,status) VALUES(${lit(rp.group)}::uuid,${lit(rp.application)}::uuid,'test','Owned delivery consent','ACCOUNT_LOGIN','ACCOUNT_ONLY','ACTIVE');
    INSERT INTO auth_consent_group_clients(group_id,client_id,application_id,environment,subject_mode,context_mode) VALUES(${lit(rp.group)}::uuid,${lit(rp.id)}::uuid,${lit(rp.application)}::uuid,'test','USER','ACCOUNT_ONLY');
    INSERT INTO auth_delivery_targets(id,purpose,status,client_id,url,allowed_ips,verified_reference,verified_at,expires_at) VALUES(${lit(rp.target)}::uuid,'BACK_CHANNEL','ACTIVE',${lit(rp.id)}::uuid,${lit(url)},ARRAY['127.0.0.2'],${lit(newId())}::uuid,clock_timestamp(),clock_timestamp()+interval '1 hour');
    UPDATE auth_clients SET logout_mode='BACK_CHANNEL',backchannel_target_id=${lit(rp.target)}::uuid,backchannel_logout_uri=${lit(url)},policy_version=policy_version+1,version=version+1 WHERE id=${lit(rp.id)}::uuid`);
  return rp;
}
async function collectorGrant(page,rp){
  const verifier=randomBytes(32).toString('hex'),nonce=randomBytes(32).toString('hex'),state=randomBytes(32).toString('hex');
  const url=new URL(`${AUTH}/oauth/authorize`);url.search=new URLSearchParams({response_type:'code',client_id:rp.id,redirect_uri:rp.redirect,scope:'openid profile',state,nonce,code_challenge_method:'S256',code_challenge:createHash('sha256').update(verifier).digest('base64url')});
  await page.goto(url.toString(),{waitUntil:'domcontentloaded'});
  await page.waitForFunction(()=>new URL(location.href).searchParams.has('code')||document.querySelector('#auth-content')?.dataset.screen==='CONSENT_REQUIRED');
  if(!new URL(page.url()).searchParams.has('code')){await page.locator('#consent-allow:enabled').waitFor();await page.locator('#consent-allow').click();}
  await page.waitForURL(url=>url.origin===AUTH&&url.pathname==='/sso/applications'&&url.searchParams.has('code'),{waitUntil:'domcontentloaded'});
  const callback=new URL(page.url());assert(callback.searchParams.get('state')===state,'COLLECTOR_RP_STATE_MISMATCH');
  const response=wire({mode:'oauth',path:'/oauth/token',client:rp.id,credential:rp.secret,form:{grant_type:'authorization_code',code:callback.searchParams.get('code'),redirect_uri:rp.redirect,code_verifier:verifier}});
  assert(response.status===200&&response.body?.refresh_token&&response.body?.id_token,'REAL_COLLECTOR_CODE_EXCHANGE_FAILED');
  const claims=JSON.parse(Buffer.from(response.body.id_token.split('.')[1],'base64url').toString('utf8'));
  const source=row('shared',`SELECT grant_id,public_sid,public_sub FROM auth_rp_sessions WHERE client_id=${lit(rp.id)}::uuid ORDER BY created_at DESC,grant_id DESC LIMIT 1`);
  assert(claims.sid===source.public_sid&&claims.sub===source.public_sub&&claims.nonce===nonce&&claims.aud===rp.id,'COLLECTOR_ID_TOKEN_ORIGINAL_SOURCE_FAILED');
  await page.goto(`${AUTH}/sso/applications`,{waitUntil:'domcontentloaded'});
  return {source,refresh:response.body.refresh_token};
}
function collectorLogout(rp,grant){const operation=newId(),form={operation_id:operation,scope:'CURRENT_APP',token:grant.refresh,sid:grant.source.public_sid,sub:grant.source.public_sub};const started=performance.now();const response=wire({mode:'oauth',path:'/oauth/logout',client:rp.id,credential:rp.secret,form});const elapsed=performance.now()-started;assert(response.status===200&&response.body?.state==='COMPLETED','REAL_COLLECTOR_LOGOUT_FAILED');return {operation,form,elapsed};}
function interrupt() { interrupted = true; process.exitCode = 1; if (browser) void browser.close().catch(() => {}); }
process.on('SIGINT', interrupt); process.on('SIGTERM', interrupt);

try {
  assert(sql('shared',`SELECT count(*) FROM auth_totp_factors WHERE user_id=${lit(ids.reviewer)}::uuid`)==='0','FRESH_UNENROLLED_REVIEWER_REQUIRED');mfaCleanupEligible=true;
  assert(sql('shared', `SELECT count(*) FROM auth_delivery_targets WHERE id=ANY(ARRAY[${Object.values(environment.phase20.targets).map(id => `${lit(id)}::uuid`).join(',')}]) AND status='ACTIVE'`) === String(Object.keys(environment.phase20.targets).length), 'REGISTERED_OWNER_TARGETS_REQUIRED');
  browser = await chromium.launch({ headless: true, proxy: { server: proxy }, ...(product === 'chromium' ? {} : { channel: product }) }); report.browserVersion = browser.version();
  const shared = await newContext(), consolePage = await pageIn(shared), portalPage = await pageIn(shared);
  step = 'real-two-rp-login-produces-distinct-opaque-sid';
  let consoleSession = await consoleLogin(consolePage, 'admin'); const portalSession = await portalLogin(portalPage, 'admin');
  const a = source('console', consoleSession.sessionId), b = source('portal', portalSession.fixtureSessionId);
  assert(/^[0-9a-f]{64}$/.test(a.oidc_public_sid) && /^[0-9a-f]{64}$/.test(b.oidc_public_sid) && a.oidc_public_sid !== b.oidc_public_sid && a.global_session_id === b.global_session_id && a.oidc_public_sid !== a.global_session_id, 'OPAQUE_RP_SESSION_BINDING_FAILED'); pass();
  const unrelatedPage = await pageIn(await newContext()), unrelated = await consoleLogin(unrelatedPage, 'employee');
  const otherDevicePage=await pageIn(await newContext()),otherDevice=await consoleLogin(otherDevicePage,'admin'),device=source('console',otherDevice.sessionId);assert(device.oidc_grant_id!==a.oidc_grant_id&&device.oidc_public_sid!==a.oidc_public_sid&&device.global_session_id!==a.global_session_id,'INDEPENDENT_ORIGINAL_GRANT_REQUIRED');
  step = 'current-app-original-grant-only-with-consent-and-other-rp-preserved';
  await logout(consolePage, consoleSession, 'PLATFORM');
  await until(() => sql('shared', `SELECT state FROM auth_delivery_jobs WHERE grant_id=${lit(a.oidc_grant_id)}::uuid`) === 'DELIVERED', 'CURRENT_APP_NOTIFICATION_NOT_DELIVERED');
  assert(source('console', a.id).revoked && !source('portal', b.id).revoked && !source('console', unrelated.sessionId).revoked, 'CURRENT_APP_CROSSED_RP_OR_ACCOUNT');
  assert(sql('shared', `SELECT NOT EXISTS(SELECT 1 FROM auth_consents WHERE user_id=${lit(ids.admin)}::uuid AND status<>'ACTIVE') AND EXISTS(SELECT 1 FROM sso_sessions WHERE id=${lit(a.global_session_id)}::uuid AND revoked_at IS NULL)`) === 't', 'CURRENT_APP_CHANGED_CONSENT_OR_SSO'); await view(portalPage, 'admin', CUSTOMER); pass();
  step='same-rp-independent-original-grant-survives-current-app';assert(!source('console',device.id).revoked,'CURRENT_APP_REVOKED_ANOTHER_SAME_RP_GRANT');await view(otherDevicePage,'admin',CONSOLE);pass();
  const collectorPage=await pageIn(shared),rp=ownedCollectorRp();
  step='collector-outage-does-not-delay-canonical-revoke-and-recovers-same-operation';
  let collector=await collectorGrant(collectorPage,rp);mode('LOGOUT',503,3000);
  const original=collectorLogout(rp,collector);
  assert(original.elapsed<3000&&sql('shared',`SELECT revoked_at IS NOT NULL FROM auth_grants WHERE id=${lit(collector.source.grant_id)}::uuid`)==='t','COLLECTOR_DELAYED_LOCAL_REVOKE');
  await until(()=>sql('shared',`SELECT state='PENDING' AND attempt_count>=1 AND last_error_code IS NOT NULL FROM auth_delivery_jobs WHERE grant_id=${lit(collector.source.grant_id)}::uuid`)==='t','LOGOUT_OUTAGE_NOT_RETRYABLE');
  mode('LOGOUT',200);sql('shared',`UPDATE auth_delivery_jobs SET available_at=clock_timestamp() WHERE grant_id=${lit(collector.source.grant_id)}::uuid AND state='PENDING'`);
  await until(()=>sql('shared',`SELECT state FROM auth_delivery_jobs WHERE grant_id=${lit(collector.source.grant_id)}::uuid`)==='DELIVERED','LOGOUT_RETRY_DID_NOT_RECOVER');
  const lookup=wire({mode:'oauth',path:`/oauth/logout/operations/${original.operation}`,client:rp.id,credential:rp.secret,form:original.form});assert(lookup.status===200&&lookup.body.operationId===original.operation&&lookup.body.state==='COMPLETED','ORIGINAL_REVOKED_TOKEN_RECEIPT_RECOVERY_FAILED');pass({responseMilliseconds:Math.ceil(original.elapsed),collectorDelayMilliseconds:3000,retryClockAdvancedAfterObservedFailure:true});
  step='permanent-delivery-failure-is-terminal-and-never-restores-authority';
  collector=await collectorGrant(collectorPage,rp);mode('LOGOUT',400);collectorLogout(rp,collector);
  await until(()=>sql('shared',`SELECT state='TERMINAL' AND attempt_count=1 AND last_error_code='DELIVERY_HTTP_REJECTED' AND terminal_at IS NOT NULL FROM auth_delivery_jobs WHERE grant_id=${lit(collector.source.grant_id)}::uuid`)==='t','PERMANENT_DELIVERY_NOT_TERMINAL');
  assert(sql('shared',`SELECT revoked_at IS NOT NULL FROM auth_grants WHERE id=${lit(collector.source.grant_id)}::uuid`)==='t','TERMINAL_DELIVERY_RESTORED_AUTHORITY');mode('LOGOUT',200);pass();
  step='short-session-contract-is-explicitly-degraded-and-rejects-request-destinations';
  sql('shared',`UPDATE auth_clients SET logout_mode='SHORT_SESSION_ONLINE_CHECK',backchannel_target_id=NULL,backchannel_logout_uri=NULL,policy_version=policy_version+1,version=version+1 WHERE id=${lit(rp.id)}::uuid`);
  collector=await collectorGrant(collectorPage,rp);
  const denied=wire({mode:'oauth',path:'/oauth/logout',client:rp.id,credential:rp.secret,form:{token:collector.refresh,scope:'CURRENT_APP',operation_id:newId(),sid:collector.source.public_sid,sub:collector.source.public_sub,backchannel_uri:'https://169.254.169.254/latest/meta-data/'}});assert(denied.status===400&&sql('shared',`SELECT revoked_at IS NULL FROM auth_grants WHERE id=${lit(collector.source.grant_id)}::uuid`)==='t','REQUEST_CONTROLLED_DESTINATION_ADMITTED');collectorLogout(rp,collector);
  assert(sql('shared',`SELECT state='DEGRADED' AND last_error_code='SHORT_SESSION_CONTRACT' AND target_id IS NULL FROM auth_delivery_jobs WHERE grant_id=${lit(collector.source.grant_id)}::uuid`)==='t','SHORT_SESSION_DEGRADATION_NOT_EXPLICIT');pass();
  step = 'global-closes-both-rps-and-sso-with-consent-preserved';
  consoleSession = await consoleLogin(consolePage, 'admin'); const fresh = source('console', consoleSession.sessionId);
  await logout(portalPage, await view(portalPage, 'admin', CUSTOMER), 'GLOBAL');
  await until(() => source('console', fresh.id).revoked && source('portal', b.id).revoked&&source('console',device.id).revoked, 'GLOBAL_BACKCHANNEL_DID_NOT_CLOSE_BOTH_RPS');
  assert(sql('shared', `SELECT NOT EXISTS(SELECT 1 FROM sso_sessions WHERE user_id=${lit(ids.admin)}::uuid AND revoked_at IS NULL)`) === 't' && sql('shared', `SELECT NOT EXISTS(SELECT 1 FROM auth_consents WHERE user_id=${lit(ids.admin)}::uuid AND status<>'ACTIVE')`) === 't' && !source('console', unrelated.sessionId).revoked, 'GLOBAL_SCOPE_OR_CONSENT_FAILED');
  assert(Number(sql('shared', `SELECT count(*) FROM platform_backchannel_logout_receipts WHERE client_id=${lit(ids.client)}::uuid`)) >= 1 && Number(sql('portal', `SELECT count(*) FROM portal_backchannel_logout_receipts WHERE client_id=${lit(ids.portalClient)}::uuid`)) >= 1, 'SIGNED_WORKER_RECEIVER_RECEIPTS_MISSING'); pass({ notificationsBeforeSessionReads: true });
  step = 'signed-receiver-audience-signature-sub-and-replay-denials';
  for (const [page, client, original] of [[consolePage, ids.client, fresh], [portalPage, ids.portalClient, b]]) {
    const unknown = hash(newId()), subject = hash(newId()), token = signedLogout(client, unknown, subject);
    assert((await receiver(page, token)).status === 200, 'VALID_UNKNOWN_SID_FAILED'); const replay = await receiver(page, token); assert(replay.status === 409 && replay.body?.code === 'BACKCHANNEL_REPLAY', 'REPLAY_NOT_REJECTED');
    assert((await receiver(page, signedLogout(client, unknown, subject, { aud: newId() }))).status === 400, 'WRONG_AUDIENCE_ACCEPTED');
    const parts=token.split('.'),signature=Buffer.from(parts[2],'base64url');signature[0]^=1;const bad=`${parts[0]}.${parts[1]}.${signature.toString('base64url')}`;signature.fill(0);assert((await receiver(page, bad)).status === 400, 'WRONG_SIGNATURE_ACCEPTED');
    assert((await receiver(page, signedLogout(client, original.oidc_public_sid, hash(newId())))).status === 400, 'EXISTING_SID_WRONG_SUB_ACCEPTED');
  } pass();

  step = 'real-reviewer-password-totp-enrollment-and-invitation-issue';
  const reviewer = await pageIn(await newContext()); await consoleLogin(reviewer, 'reviewer'); await enrollReviewer(reviewer); report.realEnrollment = true;
  mode('INVITATION', 503);
  const invitation = await invitationCommand(reviewer, '/api/identity/invitations', { recipientRef: environment.phase20.recipientRef, purpose: 'ACCOUNT_ONBOARDING' });
  assert(uuid.test(invitation.id) && invitation.status === 'ACTIVE' && !Object.hasOwn(invitation, 'token'), 'REAL_INVITATION_ISSUE_FAILED'); const verification = await verifyInvitation(invitation.id); pass();
  step = 'verified-channel-failure-retries-without-durable-invitation-material';
  await until(() => sql('shared', `SELECT state='PENDING' AND attempt_count>=1 AND last_error_code='DELIVERY_HTTP_RETRYABLE' FROM auth_delivery_jobs WHERE invitation_id=${lit(invitation.id)}::uuid`) === 't', 'INVITATION_RETRY_NOT_DURABLE');
  const failed = row('shared', `SELECT attempt_count,extract(epoch FROM available_at-clock_timestamp())::integer AS delay FROM auth_delivery_jobs WHERE invitation_id=${lit(invitation.id)}::uuid`); const expectedBackoff=Math.min(3600,60*2**Math.min(failed.attempt_count-1,6));assert(failed.delay > 0 && failed.delay <= expectedBackoff, 'INVITATION_BACKOFF_NOT_BOUNDED');
  mode('INVITATION', 200); sql('shared', `UPDATE auth_delivery_jobs SET available_at=clock_timestamp() WHERE invitation_id=${lit(invitation.id)}::uuid AND state='PENDING'`);
  const material = await peekInvitation(invitation.id); await until(() => sql('shared', `SELECT state FROM auth_delivery_jobs WHERE invitation_id=${lit(invitation.id)}::uuid`) === 'DELIVERED', 'INVITATION_RETRY_DID_NOT_DELIVER');
  assert(material.verificationRef === verification && material.recipientRef === environment.phase20.recipientRef && material.acceptUrl === `${AUTH}/sso/invitations/${invitation.id}` && /^[A-Za-z0-9_-]{43}$/.test(material.token), 'INVITATION_DELIVERY_BINDING_FAILED');
  const durableJob=sql('shared',`SELECT row_to_json(auth_delivery_jobs)::text FROM auth_delivery_jobs WHERE invitation_id=${lit(invitation.id)}::uuid`);assert(!durableJob.includes(material.token),'DURABLE_INVITATION_TOKEN_LEAK');pass({ retryClockAdvancedOnlyAfterObservedFailure: true });
  step = 'real-public-invitation-form-creates-one-account-and-no-business-identity';
  const recipient = await pageIn(await newContext()); await recipient.goto(material.acceptUrl, { waitUntil: 'domcontentloaded' });
  await recipient.locator('#invitation-submit:enabled').waitFor(); const operation = await recipient.locator('#operation-id').inputValue(); assert(uuid.test(operation), 'PAGE_OPERATION_NOT_UUIDV7');
  const username = `phase20_owned_${newId().replaceAll('-', '')}`, password = `Owned-Invitation9!${randomBytes(12).toString('hex')}`;
  await recipient.locator('#invitation-token').fill(material.token); await recipient.locator('#verification-ref').fill(verification); await recipient.locator('#new-username').fill(username); await recipient.locator('#new-password').fill(password); await recipient.locator('#invitation-submit').click(); await recipient.locator('#invitation-complete:not([hidden])').waitFor();
  assert(await recipient.locator('#invitation-token').inputValue() === '' && await recipient.locator('#new-password').inputValue() === '', 'FORM_RETAINED_SECRET');
  const created = row('shared', `SELECT id,status,is_super_admin,(SELECT count(*) FROM memberships WHERE user_id=users.id) AS members,(SELECT count(*) FROM user_roles WHERE user_id=users.id) AS roles,(SELECT count(*) FROM sso_sessions WHERE user_id=users.id) AS sessions,(SELECT count(*) FROM auth_grants WHERE user_id=users.id) AS grants FROM users WHERE username=${lit(username)}`);
  assert(created.status === 'ENABLED' && !created.is_super_admin && created.members === 0 && created.roles === 0 && created.sessions === 0 && created.grants === 0, 'INVITATION_CREATED_BUSINESS_OR_SESSION_AUTHORITY');
  assert(sql('shared',`SELECT EXISTS(SELECT 1 FROM auth_invitation_activation_receipts receipt JOIN auth_invitations invitation ON invitation.id=receipt.invitation_id JOIN auth_delivery_targets target ON target.id=receipt.delivery_target_id WHERE receipt.operation_id=${lit(operation)}::uuid AND receipt.invitation_id=${lit(invitation.id)}::uuid AND receipt.verification_id=${lit(verification)}::uuid AND receipt.user_id=${lit(created.id)}::uuid AND receipt.delivery_target_id=${lit(environment.phase20.targets.invitationChannel)}::uuid AND receipt.delivery_target_version=target.version AND invitation.status='CONSUMED' AND invitation.activation_operation_id=receipt.operation_id AND invitation.activated_account_id=receipt.user_id AND invitation.verification_ref=receipt.verification_id)`)==='t','ACTIVATION_RECEIPT_SOURCE_TUPLE_MISMATCH');
  const repeated = await json(recipient, `/api/sso/invitations/${invitation.id}/accept`, 'POST', { operationId: newId(), token: material.token, verificationRef: verification, username, password }, {}, true); assert(repeated.status === 409, 'INVITATION_REUSED');
  const recovered = await json(recipient, `/api/sso/invitations/${invitation.id}/accept`, 'POST', { operationId: operation, token: material.token, verificationRef: verification, username, password }, {}, true); assert(recovered.status === 200 && recovered.body.status === 'ACCOUNT_CREATED' && recovered.body.operationId === operation && safe(recovered.body), 'ORIGINAL_ACCEPTANCE_NOT_RECOVERABLE'); material.token = ''; pass();
  step='delivered-invitation-revocation-and-expired-owner-proof-deny-new-account';
  for(const reason of ['revoked','expired-proof']){
    const pending=await invitationCommand(reviewer,'/api/identity/invitations',{recipientRef:environment.phase20.recipientRef,purpose:'ACCOUNT_ONBOARDING'}),proof=await verifyInvitation(pending.id),delivered=await peekInvitation(pending.id);
    await until(()=>sql('shared',`SELECT state FROM auth_delivery_jobs WHERE invitation_id=${lit(pending.id)}::uuid`)==='DELIVERED','NEGATIVE_INVITATION_NOT_DELIVERED');
    if(reason==='revoked'){const value=await invitationCommand(reviewer,`/api/identity/invitations/${pending.id}/revoke`,{expectedVersion:pending.version});assert(value.status==='REVOKED','REAL_INVITATION_REVOKE_FAILED');}
    else sql('shared',`UPDATE auth_invitation_verifications SET verified_at=clock_timestamp()-interval '2 hours',expires_at=clock_timestamp()-interval '1 hour' WHERE id=${lit(proof)}::uuid`);
    const targetUsername=`phase20_denied_${newId().replaceAll('-','')}`,denied=await json(recipient,`/api/sso/invitations/${pending.id}/accept`,'POST',{operationId:newId(),token:delivered.token,verificationRef:proof,username:targetUsername,password},{},true);
    assert(denied.status===(reason==='revoked'?409:403)&&sql('shared',`SELECT count(*) FROM users WHERE username=${lit(targetUsername)}`)==='0','REVOKED_OR_EXPIRED_INVITATION_CREATED_ACCOUNT');delivered.token='';
  }
  pass({expiredBoundary:'owner verification deadline; original invitation absolute deadline separately covered by canonical PG matrix'});
  step = 'durable-record-redaction-and-actual-tls';
  assert(sql('shared', "SELECT NOT EXISTS(SELECT 1 FROM auth_logout_command_receipts WHERE result_json ?| ARRAY['token','password','sid','sub','userId']) AND NOT EXISTS(SELECT 1 FROM auth_invitation_activation_receipts WHERE result_json ?| ARRAY['token','password','userId']) AND NOT EXISTS(SELECT 1 FROM identity_audit_logs WHERE detail::jsonb ?| ARRAY['token','password','secret','seed','recoveryCodes'])") === 't', 'DURABLE_SECRET_FIELDS_FOUND');
  await Promise.allSettled([...observations]); assert(protocols.size > 0 && [...protocols].every(value => /^TLS [12]\./.test(value)), 'ACTUAL_TLS_REQUIRED'); pass(); assert(!interrupted, 'BROWSER_INTERRUPTED'); report.result = 'PASS';
} catch (error) {
  report.failedStep = step; const network = /net::(ERR_[A-Z0-9_]+)/.exec(error?.message || '')?.[1]; report.failureCode = interrupted ? 'BROWSER_INTERRUPTED' : /^[A-Z0-9]{5}$/.test(error?.code || '') ? `SQLSTATE_${error.code}` : error?.code === 'ERR_ASSERTION' && /^[A-Z0-9_]+$/.test(error.message) ? error.message : network || (error?.name === 'TimeoutError' ? 'BROWSER_STEP_TIMEOUT' : 'BROWSER_STEP_FAILED');
  try { if (activePage && !activePage.isClosed()) { const url = new URL(activePage.url()); report.pageState = { origin: [AUTH, CONSOLE, CUSTOMER].includes(url.origin) ? url.origin : 'OTHER', pathname: url.pathname.replace(/\/[0-9a-f-]{36}(?=\/|$)/g, '/:id'), screen: await activePage.evaluate(()=>document.querySelector('#auth-content')?.dataset.screen||null).catch(() => null) }; } } catch {} process.exitCode = 1;
} finally {
  await Promise.allSettled([...observations]); for (const context of contexts) await context.close().catch(() => {}); if (browser) await browser.close().catch(() => {});
  report.browserCleanup={contextsClosed:contexts.every(context=>context.pages().length===0),browserClosed:!browser||!browser.isConnected()};
  if(!report.browserCleanup.contextsClosed||!report.browserCleanup.browserClosed){report.result='FAILED';report.failureCode='OWNED_BROWSER_CLEANUP_FAILED';process.exitCode=1;}
  if(mfaCleanupEligible){try{sql('shared',`UPDATE sso_sessions SET revoked_at=clock_timestamp() WHERE user_id=${lit(ids.reviewer)}::uuid AND revoked_at IS NULL;DELETE FROM auth_totp_recovery_codes WHERE user_id=${lit(ids.reviewer)}::uuid;DELETE FROM auth_totp_enrollments WHERE user_id=${lit(ids.reviewer)}::uuid;DELETE FROM auth_totp_factors WHERE user_id=${lit(ids.reviewer)}::uuid`);assert(sql('shared',`SELECT NOT EXISTS(SELECT 1 FROM auth_totp_factors WHERE user_id=${lit(ids.reviewer)}::uuid) AND NOT EXISTS(SELECT 1 FROM auth_totp_recovery_codes WHERE user_id=${lit(ids.reviewer)}::uuid) AND NOT EXISTS(SELECT 1 FROM auth_totp_enrollments WHERE user_id=${lit(ids.reviewer)}::uuid) AND NOT EXISTS(SELECT 1 FROM sso_sessions WHERE user_id=${lit(ids.reviewer)}::uuid AND revoked_at IS NULL)`)==='t','OWNED_MFA_CLEANUP_FAILED');report.ownedReviewerMfaReset=true;}catch{report.ownedReviewerMfaReset=false;report.result='FAILED';report.failureCode='OWNED_MFA_CLEANUP_FAILED';process.exitCode=1;}}
  if (factor) factor.key.fill(0); for (const account of Object.values(environment.accounts)) delete account.password; delete environment.phase20.collector.controlSecret;
  if(interrupted){report.result='FAILED';report.failureCode='BROWSER_INTERRUPTED';process.exitCode=1;}report.tlsProtocols = [...protocols]; report.executionPhase=executionPhase;report.snapshotSha256=ownedInputs.snapshotSha256;assert(safe(report), 'REPORT_SECRET_GUARD_FAILED'); saveBrowserReport(outputFile, report, executionPhase);
  console.log(`PHASE20_HTTPS_BROWSER_RESULT ${JSON.stringify({ result: report.result, checks: report.checks.length, ...(report.failedStep ? { failedStep: report.failedStep, failureCode: report.failureCode } : {}) })}`); process.removeListener('SIGINT', interrupt); process.removeListener('SIGTERM', interrupt);
}
