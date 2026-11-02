/** Owned, real HTTPS key lifecycle and canonical isolated recovery rehearsal.
 * Every raw credential remains in memory or an owner-only task file. The
 * environment/HBA owners retain cleanup authority even when this runner fails.
 */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { execFileSync, spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { createHash, createHmac, randomBytes, sign } from 'node:crypto';
import { runKeyCases } from './phase23-key-cases.mjs';
import { checkDualIdentityIssuance } from './phase23-dual-identity-cases.mjs';
import { selectionSnapshot, selectionFacts, selectionAction, verifyCurrentTotpSource, safeFailureCode, recordIdentityInteraction } from './phase23-identity-driver.mjs';
import { protocolFailureFact, safeProtocolCause } from './phase23-protocol-diagnostics.mjs';
import { checkServiceGetControls } from './phase23-service-get-cases.mjs';
import { checkServiceAccessControls, openSessionAccess, openSessionEnvelope } from './phase23-service-access-cases.mjs';
import { checkContextCase, sourceProof } from './phase23-service-context-cases.mjs';
import { whileUnexpired } from './phase23-carrier-window.mjs';
import { validateFileNegativeEvidence } from './phase23-file-negative-evidence.mjs';
import { prepareRecoveryKeyCarriers } from './phase23-recovery-key-cases.mjs';

const TASK = '/tmp/rust-saas-unified-auth-20261028';
const AUTH = 'https://auth.saas.test', CONSOLE = 'https://console.saas.test';
const CUSTOMER = 'https://customer.saas.test', RP = 'https://customer.saas.test:24446';
const args = {};
for (let i = 2; i < process.argv.length; i += 2) {
  assert(['--snapshot', '--output', '--file-negative-evidence', '--validation-profile', '--phase'].includes(process.argv[i]) && process.argv[i + 1], 'INVALID_ARGUMENTS');
  assert(!Object.hasOwn(args, process.argv[i]), 'DUPLICATE_ARGUMENT'); args[process.argv[i]] = process.argv[i + 1];
}
const executionPhase = args['--phase'] ?? '23';
assert(['23','25'].includes(executionPhase), 'EXECUTION_PHASE_INVALID');
const label = args['--snapshot']; assert(new RegExp(`^phase${executionPhase}-tools-r[1-9][0-9]*$`).test(label ?? ''), 'FROZEN_TOOLS_REQUIRED');
const validationProfile=args['--validation-profile']??'full';assert(['full','recovery'].includes(validationProfile),'PHASE23_VALIDATION_PROFILE_INVALID');
const output = path.resolve(args['--output'] ?? ''); assert(new RegExp(`^phase-${executionPhase}-[a-z0-9-]+\\.json$`).test(path.basename(output)), 'SAFE_OUTPUT_REQUIRED');
const fileNegativeEvidencePath=args['--file-negative-evidence']?path.resolve(args['--file-negative-evidence']):null;
assert(!fileNegativeEvidencePath||(path.dirname(fileNegativeEvidencePath)===path.dirname(output)&&/^phase-23-[a-z0-9-]+\.json$/.test(path.basename(fileNegativeEvidencePath))),'PHASE23_FILE_NEGATIVE_EVIDENCE_PATH_INVALID');
let fileNegativeEvidence;
const frozen = `${TASK}/source-snapshot-${label}`, tools = `${frozen}/saas-plateform/backend/tests/unified-auth-https`;
const recovery = `${frozen}/saas-plateform/backend/ops/identity-recovery/recovery.py`;
const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const uuid = /^[a-f0-9]{8}-[a-f0-9]{4}-7[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/;
const lit = value => `'${String(value).replaceAll("'", "''")}'`;
const hash = value => createHash('sha256').update(value).digest('hex');
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
const newId = () => { const b = randomBytes(16); b.writeUIntBE(Date.now(), 0, 6); b[6] = (b[6] & 15) | 112; b[8] = (b[8] & 63) | 128; const h = b.toString('hex'); return `${h.slice(0,8)}-${h.slice(8,12)}-${h.slice(12,16)}-${h.slice(16,20)}-${h.slice(20)}`; };
function wsl(command, { user = 'jackie', input, timeout = 45000 } = {}) {
  try { return execFileSync('wsl.exe', ['-d', 'Ubuntu', '-u', user, '--exec', ...command], { input, encoding: 'utf8', windowsHide: true, timeout, maxBuffer: 4 * 1024 * 1024, stdio: ['pipe', 'pipe', 'pipe'] }); }
  catch (failure) { const error = new Error('OWNED_SUBPROCESS_FAILED'); const raw = String(failure.stderr ?? ''); error.code = /(?:RECOVERY|PHASE23)_[A-Z_0-9]+/.exec(raw)?.[0]; throw error; }
}
function privateText(filename) {
  assert(filename.startsWith(TASK+'/')&&!filename.includes('..'),'PRIVATE_INPUT_PATH_REJECTED');
  if(filename===`${TASK}/namespace.pid`)return wsl(['python3','-c','import json,os,pathlib,stat,sys; root=pathlib.Path(sys.argv[1]); p=root/"namespace.pid"; s=p.lstat(); assert stat.S_ISREG(s.st_mode) and s.st_uid==0 and stat.S_IMODE(s.st_mode) in (0o600,0o644) and s.st_nlink==1; v=json.loads(p.read_text()); pid=v["pid"]; proc=pathlib.Path("/proc")/str(pid); assert pid>1 and proc.stat().st_uid==0 and proc.joinpath("stat").read_text().rsplit(")",1)[1].split()[19]==v["startTime"] and os.readlink(proc/"ns/mnt")==v["namespace"]==root.joinpath("namespace.identity").read_text().strip(); print(json.dumps(v))',TASK],{user:'root'});
  return wsl(['python3','-B','-c','import pathlib,sys; sys.path.insert(0,sys.argv[1]); import core; root=core.safe_path(sys.argv[2]); core.private_directory(root); p=core.safe_path(sys.argv[3],root); print(core.private_read(p).decode(),end="")',`${frozen}/saas-plateform/backend/ops/identity-recovery`,TASK,filename]);
}
const privateJson = filename => JSON.parse(privateText(filename));
function privateWrite(filename, value) {
  assert(filename.startsWith(`${TASK}/phase23-`) && !filename.includes('..'), 'PRIVATE_OUTPUT_PATH_REJECTED');
  wsl(['python3','-B','-c','import sys; sys.path.insert(0,sys.argv[1]); import core; root=core.safe_path(sys.argv[2]); core.private_directory(root); p=core.safe_path(sys.argv[3],root); core.private_write(p,sys.stdin.buffer.read())',`${frozen}/saas-plateform/backend/ops/identity-recovery`,TASK,filename], { input: JSON.stringify(value) });
}
let environment = privateJson(`${TASK}/https-environment.json`);
const sourceEnvironment=structuredClone(environment);
const manifest = privateJson(`${TASK}/https-database.json`), ids = environment.ids, phase = environment.phase21;
const clusterBinding = JSON.parse(wsl(['python3','-B',`${tools}/phase23_cluster_binding.py`,'show','--task-dir',TASK]));
assert(manifest.clusterId === clusterBinding.systemIdentifier && /^unified_auth_https_[a-f0-9]{32}$/.test(manifest.database)
  && environment.phase23.contractVersion === 'unified-auth-phase23/1' && phase.rp.third.RP_ORIGIN === RP, 'OWNED_PHASE23_REQUIRED');
let databaseMode = 'SOURCE';
function sql(kind, statement) {
  const sourceName = kind === 'shared' ? manifest.database : manifest.portal.database;
  const name = databaseMode === 'SOURCE' ? sourceName : sourceName.replace(kind === 'shared' ? 'unified_auth_https_' : 'unified_auth_portal_', kind === 'shared' ? 'unified_auth_restore_' : 'unified_auth_restore_portal_');
  const marker = databaseMode === 'SOURCE' ? kind === 'shared' ? `identity-deployment/1/${ids.deployment}` : manifest.portal.marker : `phase23-recovery/2/${ids.deployment}`;
  const input = `BEGIN; SET LOCAL log_statement=none; SET LOCAL log_min_error_statement=panic; DO $$ BEGIN IF current_database()<>${lit(name)} OR (SELECT system_identifier::text FROM pg_control_system())<>${lit(manifest.clusterId)} OR current_setting('data_directory')<>${lit(clusterBinding.dataDirectory)} OR (SELECT shobj_description(oid,'pg_database') FROM pg_database WHERE datname=current_database()) IS DISTINCT FROM ${lit(marker)} THEN RAISE EXCEPTION 'owned phase23 guard failed'; END IF; END $$; ${statement}; COMMIT;`;
  return wsl(['/usr/lib/postgresql/18/bin/psql', '-X', '-q', '-v', 'ON_ERROR_STOP=1', '-v', 'VERBOSITY=sqlstate', '-At', `postgres://postgres@127.0.0.1:55494/${name}`], { input }).trim();
}
const row = (kind, statement) => JSON.parse(sql(kind, `SELECT row_to_json(facts) FROM (${statement}) facts`));
function envAction(action, options = []) { return JSON.parse(wsl(['python3', '-B', `${tools}/phase23-environment.py`, action, '--task-dir', TASK, ...options], { user: 'root', timeout: 180000 })); }
function coreAction(action, minimum, options = []) { return JSON.parse(wsl(['python3', '-B', recovery, action, '--config', `${TASK}/phase23-recovery-config.json`, ...(minimum ? ['--minimum-sequence', String(minimum)] : []), ...options], { timeout: 180000 })); }
async function until(predicate, code, timeout = 20000) { const end = Date.now() + timeout; for (;;) { if (await predicate()) return; assert(Date.now() < end, code); await delay(150); } }

// Raw request/response data stays in stdin/stdout pipes and is never logged.
const wireCode = String.raw`
import base64,json,os,pathlib,re,subprocess,sys,urllib.parse
try:
 r=json.load(sys.stdin);root=pathlib.Path('/tmp/rust-saas-unified-auth-20261028')
 if os.readlink('/proc/self/ns/mnt')!=(root/'namespace.identity').read_text().strip():raise ValueError()
 ops=r['recoveryTools']
 if not re.fullmatch(str(root)+r'/source-snapshot-phase(?:23|25)-tools-r[1-9][0-9]*/saas-plateform/backend/ops/identity-recovery',ops):raise ValueError()
 sys.path.insert(0,ops);import core
 core.private_directory(root);e=json.loads(core.private_read(root/'https-environment.json'));c=e['phase20']['collector'];headers={'Accept':'application/json'}
 if r['mode']=='control':
  if r['path'] not in ['/control','/control/stats','/control/peek','/control/logout-evidence']:raise ValueError()
  url=c['controlUrl']+r['path'];headers['X-Task-Control-Key']=r.get('controlSecret',c['controlSecret']);data=None if r.get('body') is None else json.dumps(r['body']).encode();headers['Content-Type']='application/json'
 elif r['mode']=='service-get':
  path=r['path']
  if not (re.fullmatch(r'/internal/identity/accounts/[0-9a-f-]{36}',path) or re.fullmatch(r'/internal/identity/auth/review/applications/[0-9a-f-]{36}/test',path) or path in ['/internal/identity/auth/invitations','/internal/identity/auth/clients']):raise ValueError()
  url='https://auth.saas.test:24444'+path;data=None;headers['Authorization']='Basic '+base64.b64encode((r['client']+':'+r['credential']).encode()).decode()
 elif r['mode']=='service-post':
  path=r['path']; identity=['/internal/identity/auth/account-proof','/internal/identity/auth/customer-account-proof','/internal/identity/customer-access-proof']
  if path not in identity+['/internal/customer-portal/memberships']:raise ValueError()
  url=('https://auth.saas.test:24444' if path in identity else 'https://console.saas.test:24444')+path
  data=json.dumps(r['body']).encode();headers['Content-Type']='application/json';headers['Authorization']='Basic '+base64.b64encode((r['client']+':'+r['credential']).encode()).decode()
 elif r['mode']=='context-post':
  path=r['path']
  if path not in ['/internal/platform/context-selection/choices','/internal/platform/context-selection/result']:raise ValueError()
  url='https://console.saas.test:24444'+path;data=json.dumps(r['body']).encode();headers['Content-Type']='application/json';headers['Authorization']='Basic '+base64.b64encode((r['client']+':'+r['credential']).encode()).decode()
 else:
  if r['path'] not in ['/oauth/token','/oauth/introspect','/oauth/logout'] and not r['path'].startswith('/oauth/logout/operations/'):raise ValueError()
  url='https://auth.saas.test'+r['path'];data=urllib.parse.urlencode(r['form']).encode();headers['Content-Type']='application/x-www-form-urlencoded';headers['Authorization']='Basic '+base64.b64encode((r['client']+':'+r['credential']).encode()).decode()
 config=[('url',url)]+[('header',key+': '+value) for key,value in headers.items()]
 if data is not None:config.append(('data',data.decode()))
 command=['curl','--disable','--silent','--max-time','10','--noproxy','*','--cacert',e['identity']['AUTH_DELIVERY_CA_FILE'],'--proto','=http,https','--max-redirs','0','--request',r.get('method','POST'),'--write-out','\n%{http_code}','--config','-']
 result=subprocess.run(command,input='\n'.join(k+' = '+json.dumps(v) for k,v in config)+'\n',text=True,capture_output=True,timeout=12)
 if result.returncode:raise ValueError()
 raw,status=result.stdout.rsplit('\n',1)
 if len(raw)>131072:raise ValueError()
 if r['mode']=='context-post':
  value=json.loads(raw) if raw else {}; error=value.get('error') if isinstance(value,dict) else None
  print(json.dumps({'status':int(status),'error':error if error in ['selection_not_allowed'] else None}))
 else:print(json.dumps({'status':int(status),'body':None if r['mode'] in ['service-get','service-post'] else json.loads(raw) if raw else None}))
except Exception:print('OWNED_PRIVATE_HTTP_FAILED',file=sys.stderr);sys.exit(1)
`;
function wire(request) { const ns = privateJson(`${TASK}/namespace.pid`); return JSON.parse(wsl(['nsenter', '--mount', '--target', String(ns.pid), 'setpriv', '--reuid=1000', '--regid=1000', '--clear-groups', 'python3','-B', '-c', wireCode], { user: 'root', input: JSON.stringify({...request,recoveryTools:`${frozen}/saas-plateform/backend/ops/identity-recovery`}) })); }
const control = (endpoint, body) => { const result = wire({ mode: 'control', path: endpoint, method: body === undefined ? 'GET' : 'POST', body }); assert.equal(result.status, 200, 'COLLECTOR_CONTROL_FAILED'); return result.body; };
const serviceGet = request => wire({mode:'service-get', method:'GET', path:request.path, client:request.client, credential:request.secret}).status;
const servicePost = request => wire({mode:'service-post', method:'POST', path:request.path, client:request.client, credential:request.secret, body:request.body}).status;
const contextPost = request => wire({mode:'context-post', method:'POST', path:request.path, client:request.client, credential:request.secret, body:request.body});
function sessionAccess(kind) {
  const employee=kind==='employee', table=employee?'bff_sessions':'portal_sessions';
  const session=row(employee?'shared':'portal',`SELECT id,issuer,oidc_client_id AS "clientId",access_ciphertext AS "accessCiphertext" FROM ${table} WHERE user_id=${lit(ids[kind])}::uuid AND revoked_at IS NULL AND expires_at>clock_timestamp() AND access_expires_at>clock_timestamp() ORDER BY created_at DESC,id DESC LIMIT 1`);
  assert(session && session.clientId===(employee?ids.client:ids.portalClient) && session.issuer===AUTH,'FRESH_SESSION_REQUIRED');
  const ring=privateJson(employee?environment.platform.BFF_TOKEN_KEY_FILE:environment.portal.PORTAL_TOKEN_KEY_FILE);
  const accessToken=openSessionAccess(kind,session,ring);canaries(accessToken,'token');
  return {clientId:session.clientId,accessToken};
}

process.env.PLAYWRIGHT_BROWSERS_PATH = path.join(repository, 'frontend/tests/browser/.browsers');
const require = createRequire(path.join(repository, 'frontend/tests/browser/package.json'));
const { chromium } = require('playwright');
const report = { contractVersion: `unified-auth-phase${executionPhase}-real-https/1`, executionPhase: Number(executionPhase), startedAt: new Date().toISOString(), result: 'FAIL', tlsErrorsIgnored: false, positiveMfaSeededBySql: false, httpIssuerFixtures: false, fixtureSignedReceiverNegatives: true, trustedDeliveryOwnerProofs: 'owned SQL fixture bindings plus actual worker delivery', checks: [], endpoints: {}, timing: {} };
report.validationProfile=validationProfile;report.keyLifecycleExecuted=validationProfile==='full';
let browser, protocol, protocolReady = false, protocolFailure = false, sequence = 0, activePage, step = 'PREFLIGHT', interrupted = false;
const pending = new Map(), contexts = [], observations = new Set(), tls = new Set(), factors = new Map();
const privateCanaries=new Set();
const selectionObservations=new WeakMap();
// Exact flow matching stays in private memory. Only fixed labels enter reports.
const identityDecisionObservations=new WeakMap();
function canaries(value,field='') { if(Array.isArray(value)){for(const item of value)canaries(item,field);}else if(value&&typeof value==='object'){for(const [name,item]of Object.entries(value))canaries(item,name);}else if(typeof value==='string'&&value.length>=8&&!value.startsWith(TASK+'/')&&/password|secret|token|nonce|state|verifier|cookie|value|privatekeypem|^key$|username|recoverycodes|rawseed/i.test(field)){privateCanaries.add(value);if(value.includes('\n'))for(const line of value.split(/\r?\n/))if(line.length>=20&&!line.startsWith('-----'))privateCanaries.add(line);} }
canaries(sourceEnvironment);
function scanOwnedLogs() {
  const code=String.raw`import json,os,pathlib,re,stat,sys
sys.path.insert(0,sys.argv[1]);import core
root=core.safe_path(sys.argv[2]);core.private_directory(root);values=json.load(sys.stdin)
assert isinstance(values,list) and len(values)<5000 and all(isinstance(v,str) and 8<=len(v)<=65536 for v in values)
logs=[]
for path in root.iterdir():
 if path.name not in ('gateway.log','postgres.log') and not re.fullmatch(r'(service-|phase23-)[A-Za-z0-9_.-]*\.log',path.name):continue
 with core.parent_descriptor(path) as (parent,name):
  fd=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=parent)
  try:
   s=os.fstat(fd);assert stat.S_ISREG(s.st_mode) and s.st_nlink==1 and s.st_uid==os.getuid() and stat.S_IMODE(s.st_mode)==0o600 and 0<=s.st_size<=32*1024*1024
   with os.fdopen(fd,'rb',closefd=False) as stream:data=stream.read(s.st_size)
   assert len(data)==s.st_size;logs.append(data)
  finally:os.close(fd)
assert len(logs)<=32 and sum(map(len,logs))<=64*1024*1024
print(json.dumps({'logFiles':len(logs),'candidatesCompared':len(values),'candidateFindings':sum(any(v.encode() in data for data in logs) for v in values)}))`;
  return JSON.parse(wsl(['python3','-B','-c',code,`${frozen}/saas-plateform/backend/ops/identity-recovery`,TASK],{input:JSON.stringify([...privateCanaries])}));
}
let lastStandardPage, adminPage, invitationPages = [], checkpointTaken = false, actualFaultStart, actualFaultStartMonotonic, lastAdminMfaVerifiedAtMs = 0;
function pass(facts = {}) { report.checks.push({ caseId: step, status: 'PASS', ...facts }); console.log(`PHASE23_CHECK ${step} PASS`); persist(); }
function persist() { fs.mkdirSync(path.dirname(output), { recursive: true }); fs.writeFileSync(output, JSON.stringify(report, null, 2) + '\n'); }
async function recoveryStep(name, execute) {
  const started=performance.now();report.recoverySubstage=name;persist();
  try{return await execute();}
  finally{report.recoveryStepMilliseconds??={};report.recoveryStepMilliseconds[name]=Math.round(performance.now()-started);persist();}
}
function freshBusinessStage(name) { report.freshBusinessStage=name;persist(); }
function observe(page) {
  activePage=page;page.setDefaultTimeout(25000);page.setDefaultNavigationTimeout(40000);
  page.on('response',response=>{const operation=(async()=>{
    const url=new URL(response.url());if(![AUTH,CONSOLE,CUSTOMER,RP].includes(url.origin))return;
    const method=response.request().method(),key=`${method} ${url.origin}${url.pathname.replace(/\/[a-f0-9-]{36}(?=\/|$)/g,'/:id')} ${response.status()}`;
    report.endpoints[key]=(report.endpoints[key]??0)+1;
    const decision=identityDecisionObservations.get(page);
    if(decision?.started&&url.origin===AUTH&&url.pathname===`/oauth/interaction/${decision.flow}`){
      const phase=method==='POST'?'DECISION_POST':method==='GET'&&decision.postObserved?'FOLLOWUP_GET':null;
      if(phase){
        if(method==='POST')decision.postObserved=true;
        let action=null;
        if(method==='POST'){
          try{action=JSON.parse(response.request().postData()??'{}').action;}catch{}
        }
        // Reserve the event in response order before any asynchronous body
        // read. Fast product follow-up GETs cannot overtake their actual POST.
        report.identityInteractionEvents??=[];
        const input={actor:decision.actor,mode:decision.mode,phase,status:response.status(),screen:decision.screen,action};
        const index=report.identityInteractionEvents.length;
        if(recordIdentityInteraction(report.identityInteractionEvents,input)){
          const event=report.identityInteractionEvents[index];persist();
          if(method==='GET'&&response.status()===200){
            // Retain just the allowlisted status enum; never persist a response.
            const sanitized=[];
            recordIdentityInteraction(sanitized,{...input,screen:(await response.json().catch(()=>null))?.status});
            event.screen=sanitized[0].screen;persist();
          }
        }
      }
    }
    if(url.origin===AUTH&&method==='GET'&&response.status()===200&&/^\/oauth\/interaction\/[a-f0-9-]{36}$/.test(url.pathname)){
      // Retain only opaque fixture bindings in memory. Names, CSRF and other
      // interaction fields never enter diagnostics or the persistent report.
      const snapshot=selectionSnapshot(await response.json().catch(()=>null));
      if(snapshot)selectionObservations.set(page,{flow:url.pathname.split('/').at(-1),snapshot});
    }
    const security=await response.securityDetails().catch(()=>null);if(security?.protocol)tls.add(security.protocol);
  })();observations.add(operation);operation.finally(()=>observations.delete(operation));});return page;
}
async function contextPage() { const context = await browser.newContext({ viewport: { width: 1440, height: 960 }, serviceWorkers: 'block' }); contexts.push(context); return { context, page: observe(await context.newPage()) }; }
async function json(page, endpoint, method = 'GET', body, headers = {}, omit = false) { return page.evaluate(async args => { const response = await fetch(args.endpoint, { method: args.method, cache: 'no-store', credentials: args.omit ? 'omit' : 'same-origin', redirect: 'error', headers: { Accept: 'application/json', ...(args.body === undefined ? {} : { 'Content-Type': 'application/json' }), ...args.headers }, ...(args.body === undefined ? {} : { body: JSON.stringify(args.body) }) }); let value = null; try { value = await response.json(); } catch {} return { status: response.status, body: value, noStore: response.headers.get('cache-control')?.includes('no-store') }; }, { endpoint, method, body, headers, omit }); }
function account(name) { return name === 'standard' ? environment.phase21.accounts.user : environment.accounts[name]; }
async function nextCode(name) { const factor = factors.get(name); assert(factor, 'REAL_TOTP_ENROLLMENT_REQUIRED'); for (;;) { const seconds = Number(sql('shared', 'SELECT extract(epoch FROM clock_timestamp())')), counter = Math.floor(seconds / 30); if (counter > factor.last && seconds % 30 < 27) { factor.last = counter; const bytes = Buffer.alloc(8); bytes.writeBigUInt64BE(BigInt(counter)); const digest = createHmac('sha1', factor.key).update(bytes).digest(), offset = digest[19] & 15; return ((digest.readUInt32BE(offset) & 0x7fffffff) % 1000000).toString().padStart(6,'0'); } await delay(Math.min(10000, Math.max(250, (30 - seconds % 30) * 1000 + 150))); } }
function base32(raw) { let n = 0, bits = 0; const bytes = []; assert(/^[A-Z2-7]{16,128}$/.test(raw), 'ENROLLMENT_SEED_INVALID'); for (const c of raw) { n = (n << 5) | 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'.indexOf(c); bits += 5; if (bits >= 8) { bits -= 8; bytes.push((n >>> bits) & 255); } } return Buffer.from(bytes); }
async function identity(page, name, destination, stepUp = false) {
  activePage = page;
  const initialFlow=new URL(page.url()).searchParams.get('flow');
  if(uuid.test(initialFlow??''))identityDecisionObservations.set(page,{actor:name,mode:databaseMode,flow:initialFlow,started:true,postObserved:false});
  await page.waitForFunction(dest => location.origin === dest || !!document.querySelector('#login-form:not([hidden])') || document.querySelector('#auth-content')?.dataset.screen==='CONTEXT_SELECTION_REQUIRED' || (document.querySelector('#auth-content')?.dataset.screen === 'CONSENT_REQUIRED' && !document.querySelector('#consent-panel')?.hidden && !document.querySelector('#consent-allow')?.disabled) || !!document.querySelector('#error-message:not([hidden])'), destination);
  if (new URL(page.url()).origin === AUTH && await page.locator('#login-form:not([hidden])').isVisible()) {
    const user = account(name); await page.locator('#username').fill(user.username); await page.locator('#password').fill(user.password);
    const first = page.waitForResponse(r => r.url() === `${AUTH}/api/sso/login` && r.request().method() === 'POST'); await page.locator('#login-submit').click(); const status = (await first).status();
    assert([200,428].includes(status), 'REAL_PRIMARY_LOGIN_FAILED');
    if (status === 428) { assert(factors.has(name), 'TOTP_FACTOR_MISSING'); await page.locator('#mfa-field:not([hidden])').waitFor(); await page.locator('#password').fill(user.password); await page.locator('#mfa-code').fill(await nextCode(name)); const accepted = page.waitForResponse(r => r.url() === `${AUTH}/api/sso/login` && r.request().method() === 'POST'); await page.locator('#login-submit').click(); assert.equal((await accepted).status(),200,'REAL_PASSWORD_TOTP_FAILED'); }
    else assert(!stepUp, 'STEPUP_TOTP_NOT_REQUIRED');
  }
  for (;;) {
    const transition = await page.waitForFunction(dest => { if (location.origin === dest && (dest.endsWith(':24446') || !location.pathname.startsWith('/auth/'))) return 'destination'; if (document.querySelector('#error-message:not([hidden])')) return 'error'; if (document.querySelector('#auth-content')?.dataset.screen === 'CONTEXT_SELECTION_REQUIRED' && document.querySelector('#context-selection')?.hidden===false) return 'selection'; if (document.querySelector('#auth-content')?.dataset.screen === 'CONSENT_REQUIRED' && !document.querySelector('#consent-panel')?.hidden && !document.querySelector('#consent-allow')?.disabled) return 'consent'; return false; }, destination);
    const outcome = await transition.jsonValue(); await transition.dispose(); assert.notEqual(outcome,'error','REAL_IDENTITY_FLOW_REJECTED');
    if (outcome === 'destination') break;
    if(outcome==='selection'){
      if(new URL(page.url()).origin!==AUTH)continue;
      const flow=new URL(page.url()).searchParams.get('flow');assert(uuid.test(flow??''),'REAL_CONTEXT_FLOW_REQUIRED');
      await until(()=>new URL(page.url()).origin!==AUTH||selectionObservations.get(page)?.flow===flow,'REAL_SELECTION_RESPONSE_NOT_OBSERVED');
      if(new URL(page.url()).origin!==AUTH)continue;
      assert(destination!==RP,'ACCOUNT_ONLY_RECEIVED_CONTEXT_CHOICES');
      const expected={accountId:ids[name],membershipId:ids[`${name}Member`],memberType:destination===CUSTOMER?'CUSTOMER':'EMPLOYEE'};
      const facts=selectionFacts(selectionObservations.get(page).snapshot,expected);
      const diagnostic={actor:name,mode:databaseMode,...facts};
      (report.identitySelections??=[]).push(diagnostic);persist();
      const action=selectionAction(facts,name==='customer'&&destination===CUSTOMER);diagnostic.driverAction=action;
      if(action==='AWAIT_AUTOMATIC_SELECTION'){
        diagnostic.manualMembershipSelections=0;
        // Rendering one radio is an intermediate product state. Only the real
        // page may submit it; a stalled automatic transition remains a failure.
        await page.waitForFunction(()=>location.origin!=='https://auth.saas.test'
          ||document.querySelector('#auth-content')?.dataset.screen!=='CONTEXT_SELECTION_REQUIRED'
          ||!!document.querySelector('#error-message:not([hidden])'));
        continue;
      }
      await page.locator(`input[name="context-membership"][value="${expected.membershipId}"]:not(:disabled)`).check();
      diagnostic.manualMembershipSelections=1;
    }
    const decisionFlow=new URL(page.url()).searchParams.get('flow');assert(uuid.test(decisionFlow??''),'REAL_CONTEXT_FLOW_REQUIRED');
    identityDecisionObservations.set(page,{actor:name,mode:databaseMode,flow:decisionFlow,started:true,postObserved:false,
      screen:outcome==='selection'?'CONTEXT_SELECTION_REQUIRED':'CONSENT_REQUIRED'});
    await page.locator('#consent-allow').click();
    await page.waitForFunction(previous=>location.origin!== 'https://auth.saas.test'
      ||document.querySelector('#auth-content')?.dataset.screen!==previous
      ||!!document.querySelector('#error-message:not([hidden])'), outcome==='selection'?'CONTEXT_SELECTION_REQUIRED':'CONSENT_REQUIRED');
    if(new URL(page.url()).origin===AUTH)assert(!(await page.locator('#error-message:not([hidden])').isVisible()),'REAL_IDENTITY_FLOW_REJECTED');
  }
  await page.waitForURL(url => url.origin === destination && (destination === RP || !url.pathname.startsWith('/auth/')), { waitUntil:'domcontentloaded' });
}
async function view(page, name, portal = false) { const result = await json(page,'/auth/session'); assert(result.status === 200 && result.noStore && result.body.account?.id === ids[name], 'REAL_BFF_SESSION_FAILED'); assert.equal(result.body.context?.memberType, portal ? 'CUSTOMER' : 'EMPLOYEE'); return result.body; }
async function currentAdminTotpSource(page) {
  const current=await view(page,'admin');assert(uuid.test(current.sessionId??''),'REAL_CURRENT_ADMIN_BFF_REQUIRED');
  const cookies=(await page.context().cookies(CONSOLE)).filter(cookie=>cookie.name==='__Host-saas_console');
  assert.equal(cookies.length,1,'REAL_CURRENT_ADMIN_COOKIE_REQUIRED');canaries(cookies[0].value,'cookie');
  const statement=`SELECT b.id AS "bffSessionId",b.user_id AS "bffUserId",b.token_hash AS "bffTokenDigest",browser.current_session_id AS "browserCurrentSessionId",b.global_session_id AS "globalSessionId",source.id AS "sourceSessionId",source.user_id AS "sourceUserId",grant_row.source_session_id AS "grantSourceSessionId",source.mfa_method AS "mfaMethod",extract(epoch FROM source.mfa_verified_at)*1000 AS "mfaVerifiedAtMs",(b.revoked_at IS NULL AND b.expires_at>clock_timestamp() AND browser.expires_at>clock_timestamp()) AS "bffLive",(source.revoked_at IS NULL AND source.expires_at>clock_timestamp()) AS "sourceLive",public.bff_auth_proof_active(b.user_id,b.global_session_id,b.oidc_grant_id,b.account_version,b.oidc_client_id,b.oidc_client_policy_version,b.issuer) AS "canonicalProofActive",EXISTS(SELECT 1 FROM auth_totp_factors factor WHERE factor.user_id=b.user_id) AS "totpEnabled",COALESCE(source.mfa_verified_at>=clock_timestamp()-interval '5 minutes' AND source.mfa_verified_at<=clock_timestamp(),false) AS "recentTotp" FROM bff_sessions b JOIN console_browsers browser ON browser.id=b.browser_id JOIN sso_sessions source ON source.id=b.global_session_id JOIN auth_grants grant_row ON grant_row.id=b.oidc_grant_id WHERE b.id=${lit(current.sessionId)}::uuid AND b.user_id=${lit(ids.admin)}::uuid`;
  const proof=JSON.parse(sql('shared',`SELECT COALESCE((SELECT row_to_json(facts) FROM (${statement}) facts),'null'::json)`));
  const evidence=verifyCurrentTotpSource(current,proof,{userId:ids.admin,cookieDigest:hash(cookies[0].value)});
  lastAdminMfaVerifiedAtMs=Number(proof.mfaVerifiedAtMs);assert(Number.isFinite(lastAdminMfaVerifiedAtMs)&&lastAdminMfaVerifiedAtMs>0,'REAL_ADMIN_MFA_TIMESTAMP_REQUIRED');
  return evidence;
}
async function consoleLogin(page, name, stepUp = false) { await page.goto(`${CONSOLE}${stepUp ? '/auth/step-up?return=/' : '/auth/login?return=/'}`, {waitUntil:'domcontentloaded'}); await identity(page,name,CONSOLE,stepUp); const result=await view(page,name);assert.equal(result.context?.membershipId,ids[`${name}Member`],'REAL_CONSOLE_MEMBERSHIP_MISMATCH'); await page.goto(`${CONSOLE}/auth/config`,{waitUntil:'domcontentloaded'}); return result; }
async function portalLogin(page,name) { await page.goto(CUSTOMER,{waitUntil:'domcontentloaded'}); await Promise.all([page.waitForURL(url=>url.origin===AUTH,{waitUntil:'domcontentloaded'}),page.getByTestId('portal-login').click()]); await identity(page,name,CUSTOMER); const result=await view(page,name,true);assert.equal(result.context?.membershipId,ids[`${name}Member`],'REAL_PORTAL_MEMBERSHIP_MISMATCH'); await page.goto(`${CUSTOMER}/auth/config`,{waitUntil:'domcontentloaded'}); return result; }
async function contextServiceControls(page, secondMember, target) {
  const fault='phase23_context_pending_fault', marker=`phase23-context-fault/1/${ids.deployment}`;
  const drop=()=>sql('portal',`DO $$ BEGIN IF EXISTS(SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public' AND p.proname=${lit(fault)} AND obj_description(p.oid,'pg_proc') IS DISTINCT FROM ${lit(marker)}) OR EXISTS(SELECT 1 FROM pg_trigger t WHERE t.tgrelid='public.portal_sessions'::regclass AND t.tgname=${lit(fault)} AND obj_description(t.oid,'pg_trigger') IS DISTINCT FROM ${lit(marker)}) THEN RAISE EXCEPTION 'context fault ownership mismatch'; END IF; END $$; DROP TRIGGER IF EXISTS ${fault} ON public.portal_sessions; DROP FUNCTION IF EXISTS public.${fault}()`);
  drop();
  try {
    sql('portal',`CREATE FUNCTION public.${fault}() RETURNS trigger LANGUAGE plpgsql SET search_path='pg_catalog','public' AS $fault$ BEGIN IF OLD.pending_selection_ciphertext IS NOT NULL AND NEW.pending_selection_ciphertext IS NULL AND OLD.user_id=${lit(ids.customer)}::uuid THEN RAISE EXCEPTION 'PHASE23_EXPECTED_PENDING_SAVE_FAILURE'; END IF; RETURN NEW; END $fault$; COMMENT ON FUNCTION public.${fault}() IS ${lit(marker)}; CREATE TRIGGER ${fault} BEFORE UPDATE ON public.portal_sessions FOR EACH ROW EXECUTE FUNCTION public.${fault}(); COMMENT ON TRIGGER ${fault} ON public.portal_sessions IS ${lit(marker)}`);
    await page.goto(CUSTOMER,{waitUntil:'domcontentloaded'});await page.getByTestId('portal-central-switch').waitFor();
    await Promise.all([page.waitForURL(url=>url.origin===AUTH,{waitUntil:'domcontentloaded'}),page.getByTestId('portal-central-switch').click()]);
    const radio=page.locator(`input[name="context-membership"][value="${secondMember}"]`);await radio.waitFor();
    const flow=new URL(page.url()).searchParams.get('flow');assert(uuid.test(flow??''),'FRESH_CONTEXT_FLOW_REQUIRED');
    const cookies=(await page.context().cookies(AUTH)).filter(cookie=>cookie.name==='__Host-saas_oauth_browser');assert.equal(cookies.length,1,'CONTEXT_BROWSER_COOKIE_REQUIRED');
    const proof=sourceProof(cookies[0].value,flow);canaries(cookies,'cookie');canaries(proof,'token');
    assert.equal(sql('shared',`SELECT count(*) FROM auth_context_interactions c JOIN auth_authorization_requests a ON a.id=c.authorization_request_id WHERE a.id=${lit(flow)}::uuid AND a.browser_digest=${lit(hash(cookies[0].value))} AND c.source_digest=${lit(hash(proof))} AND c.client_id=${lit(ids.portalClient)}::uuid AND c.selected_membership_id IS NULL AND c.expires_at>clock_timestamp() AND a.status='PENDING'`),'1','FRESH_CONTEXT_BINDING_REQUIRED');
    const choice={contractVersion:'context-selection/1',sourceProof:proof,expectedClientId:ids.portalClient};
    const selection=await checkContextCase({source:sourceEnvironment,target,body:choice,name:'selection',request:contextPost});
    await radio.check();await page.locator('#consent-allow').click();await page.waitForURL(url=>url.origin===CUSTOMER&&!url.pathname.startsWith('/auth/'),{waitUntil:'domcontentloaded'});
    await until(()=>sql('portal',`SELECT count(*) FROM portal_sessions WHERE user_id=${lit(ids.customer)}::uuid AND revoked_at IS NULL AND pending_selection_ciphertext IS NOT NULL`) === '1','PENDING_CONTEXT_REQUIRED');
    const session=row('portal',`SELECT id,version::text AS version,issuer,oidc_client_id AS "clientId",access_ciphertext AS "accessCiphertext",pending_selection_ciphertext AS "pendingCiphertext" FROM portal_sessions WHERE user_id=${lit(ids.customer)}::uuid AND revoked_at IS NULL AND pending_selection_ciphertext IS NOT NULL`);
    assert.equal(session.clientId,ids.portalClient);const ring=privateJson(environment.portal.PORTAL_TOKEN_KEY_FILE);
    const pending=JSON.parse(openSessionEnvelope('customer',session,ring,'pending-selection',session.pendingCiphertext));
    const access=openSessionAccess('customer',session,ring);canaries(access,'token');canaries(pending.extension.credential,'token');canaries(pending.oldAccess,'token');
    const body={...pending.extension,expectedClientId:session.clientId,oidcAccessToken:access,bffSessionId:session.id,version:session.version,oldOidcAccessToken:pending.oldAccess};
    const consumer=await checkContextCase({source:sourceEnvironment,target,body,name:'portalConsumer',request:contextPost});
    return [selection,consumer];
  } finally { drop(); }
}
async function enroll(page,name) { await page.goto(`${AUTH}/sso/security`,{waitUntil:'domcontentloaded'}); await page.locator('#security-panel:not([hidden])').waitFor(); await page.locator('#start-enrollment').click(); await page.locator('#enrollment-panel:not([hidden])').waitFor(); const rawSeed=(await page.locator('#secret').textContent()).trim();canaries(rawSeed,'rawSeed');factors.set(name,{key:base32(rawSeed),last:-1}); await page.locator('#confirm-code').fill(await nextCode(name)); await page.locator('#confirm-form button[type=submit]').click(); await page.locator('#recovery-panel:not([hidden])').waitFor();canaries((await page.locator('#recovery-codes').textContent()).trim().split(/\s+/),'recoveryCodes'); await page.locator('#recovery-done').click(); await page.evaluate(()=>{ for(const id of ['secret','otpauth-uri','recovery-codes'])document.querySelector('#'+id).textContent=''; }); await consoleLogin(page,name,true); }
async function refreshAdminMfa(reason) {
  await consoleLogin(adminPage,'admin',true);const evidence=await currentAdminTotpSource(adminPage);
  (report.realAdminStepUps??=[]).push({reason,...evidence});persist();
  return {realPasswordTotp:true,currentBrowserSourceVerified:true};
}
async function management(request) {
  if((request.method??'POST')!=='GET'){
    assert(lastAdminMfaVerifiedAtMs>0,'REAL_ADMIN_MFA_TIMESTAMP_REQUIRED');
    const now=Number(sql('shared','SELECT extract(epoch FROM clock_timestamp())*1000'));
    if(now-lastAdminMfaVerifiedAtMs>=240000)await refreshAdminMfa('BEFORE_WRITE_FOUR_MINUTE_REFRESH');
  }
  const current=await view(adminPage,'admin');const cap=await json(adminPage,'/auth/context-token','POST',{expectedVersion:current.version},{'X-CSRF-Token':current.csrfToken});
  assert.equal(cap.status,200,'REAL_ADMIN_CAPABILITY_FAILED');const operation=request.operation??newId();
  const response=await json(adminPage,request.path,request.method??'POST',request.body,{Authorization:`Bearer ${cap.body.accessToken}`,'X-Identity-Operation':operation});return {...response,operation};
}
async function operationLookup(operation, kind='oidc') { return management({method:'GET',path:kind==='oidc'?`/api/identity/key-operations/${operation}`:`/api/identity/${kind}-operations/${operation}`}); }
async function command(request, kind='oidc') { const result=await management(request); if(result.status!==503)return result; for(let i=0;i<10;i++){const lookup=await operationLookup(result.operation,kind);if(lookup.status===200&&lookup.body?.state==='COMPLETED')return {status:200,body:lookup.body.result,operation:result.operation};assert.equal(lookup.status,503,'REMOTE_OPERATION_LOOKUP_FAILED');await delay(500);}assert.fail('REMOTE_OPERATION_NOT_RECOVERED'); }
function startProtocol() { const ns=privateJson(`${TASK}/namespace.pid`); protocol=spawn('wsl.exe',['-d','Ubuntu','-u','root','--exec','nsenter','--mount','--target',String(ns.pid),'setpriv','--reuid=1000','--regid=1000','--clear-groups',`${TASK}/phase21-node-runtime/bin/node`,`${tools}/phase23-protocol.mjs`,'--control',`${TASK}/phase23-protocol-control.json`],{windowsHide:true,stdio:['pipe','pipe','pipe']}); createInterface({input:protocol.stdout}).on('line',line=>{try{const value=JSON.parse(line);if(value.ready)protocolReady=true;else{const operation=pending.get(value.id);if(operation){pending.delete(value.id);operation.resolve(value);}}}catch{protocolFailure=true;}});protocol.stderr.on('data',()=>{protocolFailure=true;});protocol.on('error',()=>{protocolFailure=true;for(const operation of pending.values())operation.reject(new Error('PROTOCOL_DRIVER_START_FAILED'));pending.clear();});protocol.on('exit',()=>{protocolFailure=true;for(const operation of pending.values())operation.reject(new Error('PROTOCOL_DRIVER_EXITED'));pending.clear();}); }
function rpcRaw(command, values={}) { return new Promise((resolve,reject)=>{const id=++sequence;const timer=setTimeout(()=>{pending.delete(id);reject(new Error('PROTOCOL_DRIVER_TIMEOUT'));},25000);pending.set(id,{resolve:value=>{clearTimeout(timer);const fact=protocolFailureFact(command,value);if(fact){report.protocolFailures??=[];if(report.protocolFailures.length<64)report.protocolFailures.push(fact);persist();}resolve(value);},reject:error=>{clearTimeout(timer);reject(error);}});protocol.stdin.write(JSON.stringify({id,command,...values})+'\n');}); }
async function rpc(command,values={}) { const result=await rpcRaw(command,values);if(!result.ok)throw Object.assign(new Error('STANDARD_PROTOCOL_REJECTED'),{status:result.status,code:result.code,protocolCause:protocolFailureFact(command,result)});return result.result; }
async function protocolLogin(profile='third') { const {page}=await contextPage();lastStandardPage=page;const begin=await rpc('begin',{profile});let callback;await page.route(`${RP}/auth/callback?*`,async route=>{callback=route.request().url();await route.fulfill({status:200,contentType:'text/plain',body:'Private standard callback captured.'});});const response=await page.goto(begin.authorizationUrl,{waitUntil:'domcontentloaded'});if(response.status()>=400)return {status:response.status()};await identity(page,'standard',RP);await page.unroute(`${RP}/auth/callback?*`);assert(callback,'STANDARD_CALLBACK_REQUIRED');try{return await rpc('finish',{key:begin.key,callback});}catch(error){if(error.status)return {status:error.status};throw error;} }
function interrupt(){interrupted=true;void browser?.close().catch(()=>{});protocol?.stdin.end();}
process.on('SIGINT',interrupt);process.on('SIGTERM',interrupt);

// Scenario body is below the shared, bounded transport/UI helpers.

function ownedCollectorRp() {
  const rp={id:newId(),application:newId(),group:newId(),target:newId(),secret:randomBytes(32).toString('hex'),redirect:`${AUTH}/sso/applications`};
  const endpoint='https://auth.saas.test:24444/internal/task-delivery/backchannel';
  sql('shared',`INSERT INTO auth_applications(application_id,ownership,status,display_name,purpose,approved_scopes) VALUES(${lit(rp.application)}::uuid,'FIRST_PARTY','APPROVED','Owned Phase23 delivery RP','ACCOUNT_LOGIN',ARRAY['openid','profile']);
    INSERT INTO auth_clients(id,client_id,display_name,application_id,environment,client_type,runtime_kind,subject_mode,context_mode,grant_types,token_endpoint_auth_method,redirect_uris,status) VALUES(${lit(rp.id)}::uuid,${lit(rp.id)},'Owned Phase23 delivery RP',${lit(rp.application)}::uuid,'test','CONFIDENTIAL','WEB_BFF','USER','ACCOUNT_ONLY',ARRAY['authorization_code','refresh_token'],'client_secret_basic',ARRAY[${lit(rp.redirect)}],'ENABLED');
    INSERT INTO auth_client_credentials(id,client_id,credential_version,secret_digest) VALUES(${lit(newId())}::uuid,${lit(rp.id)}::uuid,1,${lit(hash(rp.secret))});
    INSERT INTO auth_client_permissions(client_id,resource_id,scope_id) SELECT ${lit(rp.id)}::uuid,scope.resource_id,scope.id FROM auth_scopes scope JOIN auth_resources resource ON resource.id=scope.resource_id WHERE resource.resource_key='userinfo' AND scope.scope_name=ANY(ARRAY['openid','profile']);
    INSERT INTO auth_consent_groups(id,application_id,environment,display_name,purpose,context_mode,status) VALUES(${lit(rp.group)}::uuid,${lit(rp.application)}::uuid,'test','Owned Phase23 delivery consent','ACCOUNT_LOGIN','ACCOUNT_ONLY','ACTIVE');
    INSERT INTO auth_consent_group_clients(group_id,client_id,application_id,environment,subject_mode,context_mode) VALUES(${lit(rp.group)}::uuid,${lit(rp.id)}::uuid,${lit(rp.application)}::uuid,'test','USER','ACCOUNT_ONLY');
    INSERT INTO auth_delivery_targets(id,purpose,status,client_id,url,allowed_ips,verified_reference,verified_at,expires_at) VALUES(${lit(rp.target)}::uuid,'BACK_CHANNEL','ACTIVE',${lit(rp.id)}::uuid,${lit(endpoint)},ARRAY['127.0.0.2'],${lit(newId())}::uuid,clock_timestamp(),clock_timestamp()+interval '1 hour');
    UPDATE auth_clients SET logout_mode='BACK_CHANNEL',backchannel_target_id=${lit(rp.target)}::uuid,backchannel_logout_uri=${lit(endpoint)},policy_version=policy_version+1,version=version+1 WHERE id=${lit(rp.id)}::uuid`);
  return rp;
}
async function signLogoutWithCollector() {
  const rp=ownedCollectorRp(),page=lastStandardPage;
  const verifier=randomBytes(32).toString('hex'),nonce=randomBytes(32).toString('hex'),state=randomBytes(32).toString('hex');
  const url=new URL(`${AUTH}/oauth/authorize`);url.search=new URLSearchParams({response_type:'code',client_id:rp.id,redirect_uri:rp.redirect,scope:'openid profile',state,nonce,code_challenge_method:'S256',code_challenge:createHash('sha256').update(verifier).digest('base64url')});
  await page.goto(url.href,{waitUntil:'domcontentloaded'});
  await page.waitForFunction(()=>new URL(location.href).searchParams.has('code')||document.querySelector('#auth-content')?.dataset.screen==='CONSENT_REQUIRED');
  if(!new URL(page.url()).searchParams.has('code')){await page.locator('#consent-allow:enabled').click();}
  await page.waitForURL(value=>value.origin===AUTH&&value.pathname==='/sso/applications'&&value.searchParams.has('code'),{waitUntil:'domcontentloaded'});
  const callback=new URL(page.url());assert.equal(callback.searchParams.get('state'),state,'COLLECTOR_STATE_MISMATCH');
  const exchanged=wire({mode:'oauth',path:'/oauth/token',client:rp.id,credential:rp.secret,form:{grant_type:'authorization_code',code:callback.searchParams.get('code'),redirect_uri:rp.redirect,code_verifier:verifier}});
  assert(exchanged.status===200&&exchanged.body?.refresh_token&&exchanged.body?.id_token,'COLLECTOR_REAL_EXCHANGE_FAILED');
  const jwtHeader=JSON.parse(Buffer.from(exchanged.body.id_token.split('.')[0],'base64url'));
  const source=row('shared',`SELECT grant_id,public_sid,public_sub FROM auth_rp_sessions WHERE client_id=${lit(rp.id)}::uuid ORDER BY created_at DESC,grant_id DESC LIMIT 1`);
  const operation=newId();const logged=wire({mode:'oauth',path:'/oauth/logout',client:rp.id,credential:rp.secret,form:{operation_id:operation,scope:'CURRENT_APP',token:exchanged.body.refresh_token,sid:source.public_sid,sub:source.public_sub}});
  assert.equal(logged.status,200,'COLLECTOR_REAL_LOGOUT_FAILED');
  await until(()=>sql('shared',`SELECT state FROM auth_delivery_jobs WHERE grant_id=${lit(source.grant_id)}::uuid AND kind='LOGOUT'`)==='DELIVERED','COLLECTOR_REAL_DELIVERY_MISSING');
  const observed=control('/control/logout-evidence',{expectedKid:'phase23-b',publicSidSha256:hash(source.public_sid)});
  assert(observed.matched&&observed.kid==='phase23-b'&&observed.count===1&&jwtHeader.kid==='phase23-b','COLLECTOR_REAL_SIGNING_KID_MISMATCH');
  return {kid:observed.kid,delivered:true};
}
async function verifyInvitation(invitationId) {
  const verification=newId(),target=environment.phase20.targets.invitationChannel;
  sql('shared',`INSERT INTO auth_invitation_verifications(id,invitation_id,recipient_ref,purpose,delivery_target_id,delivery_target_version,verified_reference,verified_at,expires_at,status) SELECT ${lit(verification)}::uuid,invitation.id,invitation.recipient_ref,invitation.purpose,target.id,target.version,${lit(newId())}::uuid,clock_timestamp(),LEAST(invitation.expires_at,target.expires_at),'ACTIVE' FROM auth_invitations invitation JOIN auth_delivery_targets target ON target.id=${lit(target)}::uuid AND target.recipient_ref=invitation.recipient_ref WHERE invitation.id=${lit(invitationId)}::uuid AND invitation.recipient_ref=${lit(environment.phase20.recipientRef)} AND invitation.status='ACTIVE'`);
  await until(()=>sql('shared',`SELECT state<>'LEASED' FROM auth_delivery_jobs WHERE invitation_id=${lit(invitationId)}::uuid`)==='t','INVITATION_LEASE_NOT_SETTLED');
  sql('shared',`UPDATE auth_delivery_jobs SET available_at=clock_timestamp() WHERE invitation_id=${lit(invitationId)}::uuid AND state='PENDING' AND (attempt_count=0 OR last_error_code='INVITATION_VERIFICATION_PENDING')`);
  return verification;
}
async function peekInvitation(invitationId) { let material;await until(()=>{const response=wire({mode:'control',path:'/control/peek',method:'POST',body:{kind:'INVITATION',invitationId}});assert([200,404].includes(response.status),'INVITATION_PEEK_FAILED');material=response.body?.message;return material?.invitationId===invitationId;},'INVITATION_MATERIAL_NOT_DELIVERED');await until(()=>sql('shared',`SELECT state FROM auth_delivery_jobs WHERE invitation_id=${lit(invitationId)}::uuid`)==='DELIVERED','INVITATION_JOB_NOT_DELIVERED');return {...material,deliveryState:'DELIVERED'}; }
async function acceptInvitation(input) {
  const page=observe(await (await browser.newContext()).newPage());invitationPages.push(page);contexts.push(page.context());
  if(input.operationId){await page.goto(`${AUTH}/sso/invitations/${input.invitationId}`,{waitUntil:'domcontentloaded'});const request={...input};delete request.invitationId;return {...await json(page,`/api/sso/invitations/${input.invitationId}/accept`,'POST',request,{},true),request:input};}
  await page.goto(`${AUTH}/sso/invitations/${input.invitationId}`,{waitUntil:'domcontentloaded'});await page.locator('#invitation-submit:enabled').waitFor();
  const request={...input,operationId:await page.locator('#operation-id').inputValue(),username:`phase23_created_${newId().replaceAll('-','')}`,password:`Phase23-${randomBytes(20).toString('hex')}!`};
  await page.locator('#invitation-token').fill(request.token);await page.locator('#verification-ref').fill(request.verificationRef);await page.locator('#new-username').fill(request.username);await page.locator('#new-password').fill(request.password);
  const response=page.waitForResponse(value=>value.url()===`${AUTH}/api/sso/invitations/${input.invitationId}/accept`&&value.request().method()==='POST');await page.locator('#invitation-submit').click();const raw=await response;const body=await raw.json();await page.locator('#invitation-complete:not([hidden])').waitFor();return {status:raw.status(),body,request};
}
async function denyInvitation(input) { const {page}=await contextPage();await page.goto(`${AUTH}/sso/invitations/${input.invitationId}`,{waitUntil:'domcontentloaded'});const operationId=newId(),username=`phase23_denied_${newId().replaceAll('-','')}`;assert.notEqual(operationId,input.previousOperationId);const result=await json(page,`/api/sso/invitations/${input.invitationId}/accept`,'POST',{operationId,token:input.token,verificationRef:input.verificationRef,username,password:`Phase23-${randomBytes(20).toString('hex')}!`},{},true);return {status:result.status,operationId,createdAccounts:Number(sql('shared',`SELECT count(*) FROM users WHERE username=${lit(username)}`))}; }
function fixtureLogout(kid,client,changes={}) { const keyset=privateJson(environment.identity.OIDC_KEYSET_PATH),key=keyset.keys.find(value=>value.kid===kid);assert(key,'FIXTURE_SIGNING_MATERIAL_REQUIRED');const now=Math.floor(Number(sql('shared','SELECT extract(epoch FROM clock_timestamp())'))),encode=value=>Buffer.from(JSON.stringify(value)).toString('base64url');const input=encode({alg:'RS256',typ:'logout+jwt',kid})+'.'+encode({iss:AUTH,aud:client,sid:hash(newId()),sub:hash(newId()),iat:now,exp:now+120,jti:newId(),events:{'http://schemas.openid.net/event/backchannel-logout':{}},...changes});const token=input+'.'+sign('RSA-SHA256',Buffer.from(input),key.privateKeyPem).toString('base64url');key.privateKeyPem='';return token; }
async function receiver(page,token) { return page.evaluate(async value=>{const response=await fetch('/auth/backchannel-logout',{method:'POST',credentials:'omit',redirect:'error',cache:'no-store',headers:{'Content-Type':'application/x-www-form-urlencoded'},body:new URLSearchParams({logout_token:value})});let body=null;try{body=await response.json();}catch{}return {status:response.status,body};},token); }
function readInvitationReceipt({invitationId,operationId}) {
  return row('shared',`SELECT receipt.operation_id AS "operationId",receipt.invitation_id AS "invitationId",receipt.verification_id AS "verificationId",receipt.user_id AS "userId",receipt.delivery_target_id AS "deliveryTargetId",receipt.delivery_target_version AS "deliveryTargetVersion",invitation.status AS "invitationStatus",invitation.activation_operation_id AS "activationOperationId",invitation.activated_account_id AS "activatedAccountId",invitation.verification_ref AS "verificationRef",(SELECT count(*)::int FROM auth_invitation_activation_receipts WHERE invitation_id=receipt.invitation_id) AS "receiptCount",(SELECT count(*)::int FROM users WHERE id=receipt.user_id) AS "accountCount",subject.status AS "accountStatus",subject.is_super_admin AS "isSuperAdmin",(SELECT count(*)::int FROM memberships WHERE user_id=receipt.user_id) AS memberships,(SELECT count(*)::int FROM user_roles WHERE user_id=receipt.user_id) AS roles,(SELECT count(*)::int FROM sso_sessions WHERE user_id=receipt.user_id) AS sessions,(SELECT count(*)::int FROM auth_grants WHERE user_id=receipt.user_id) AS grants FROM auth_invitation_activation_receipts receipt JOIN auth_invitations invitation ON invitation.id=receipt.invitation_id JOIN users subject ON subject.id=receipt.user_id WHERE receipt.operation_id=${lit(operationId)}::uuid AND receipt.invitation_id=${lit(invitationId)}::uuid`);
}
const invitationPorts={issue:async()=>{const result=await command({path:'/api/identity/invitations',body:{recipientRef:environment.phase20.recipientRef,purpose:'ACCOUNT_ONBOARDING'}},'invitation');assert.equal(result.status,200);return result.body;},verify:verifyInvitation,peek:peekInvitation,revoke:async input=>{const result=await command({path:`/api/identity/invitations/${input.invitationId}/revoke`,body:{expectedVersion:input.expectedVersion}},'invitation');assert.equal(result.status,200);return result.body;},accept:acceptInvitation,readReceipt:readInvitationReceipt,deny:denyInvitation};
function policyReadFacts({clientId,memberId,userId}) {
  for(const id of [clientId,memberId,userId])assert(uuid.test(id),'POLICY_FACT_ID_REQUIRED');
  const client=row('shared',`SELECT id,client_id AS "clientId",application_id AS "applicationId",status,version,policy_version AS "policyVersion",(SELECT COALESCE(jsonb_agg(jsonb_build_object('resourceKey',permission.resource_key,'scopes',permission.scopes) ORDER BY permission.resource_key),'[]'::jsonb) FROM (SELECT resource.resource_key,array_agg(scope.scope_name ORDER BY scope.scope_name) AS scopes FROM auth_client_permissions binding JOIN auth_resources resource ON resource.id=binding.resource_id JOIN auth_scopes scope ON scope.id=binding.scope_id WHERE binding.client_id=client.id GROUP BY resource.resource_key) permission) AS permissions,(SELECT COALESCE(jsonb_agg(jsonb_build_object('id',credential.id,'credentialVersion',credential.credential_version,'revoked',credential.revoked_at IS NOT NULL,'validNow',credential.revoked_at IS NULL AND (credential.expires_at IS NULL OR credential.expires_at>clock_timestamp())) ORDER BY credential.credential_version),'[]'::jsonb) FROM auth_client_credentials credential WHERE credential.client_id=client.id) AS credentials,(SELECT count(*)::int FROM auth_client_credentials credential WHERE credential.client_id=client.id AND credential.revoked_at IS NULL AND (credential.expires_at IS NULL OR credential.expires_at>clock_timestamp())) AS "liveCredentialCount" FROM auth_clients client WHERE id=${lit(clientId)}::uuid`);
  const exists=sql('shared',`SELECT EXISTS(SELECT 1 FROM memberships WHERE id=${lit(memberId)}::uuid)`)==='t';
  const member=exists?row('shared',`SELECT id,user_id AS "userId",true AS exists,status,identity_version AS "identityVersion" FROM memberships WHERE id=${lit(memberId)}::uuid`):{id:memberId,userId,exists:false};
  const user=row('shared',`SELECT id,status,session_version AS "sessionVersion" FROM users WHERE id=${lit(userId)}::uuid`);
  return {client,member,user};
}
function policyPorts(applicationId) { return {
  bindings:{applicationId,retireMemberId:environment.phase23.ids.retireMember,retireUserId:phase.ids.otherUser,protectedClientIds:[ids.client,ids.portalClient,phase.ids.thirdClient,phase.ids.offlineClient,phase.ids.machineClient,phase.ids.resourceClient],protectedMembershipIds:[ids.adminMember,ids.employeeMember,ids.customerMember],protectedUserIds:[ids.admin,ids.employee,ids.customer,phase.ids.user]},
  command:request=>command(request,'client'),
  machine:({clientId,clientSecret,scope})=>wire({mode:'oauth',path:'/oauth/token',client:clientId,credential:clientSecret,form:{grant_type:'client_credentials',scope}}),
  resource:async({accessToken})=>{const page=lastStandardPage;await page.goto(`${RP}/api/session`,{waitUntil:'domcontentloaded'});return page.evaluate(async value=>{const response=await fetch('/demo-resource',{headers:{Authorization:`Bearer ${value}`},cache:'no-store',credentials:'omit',redirect:'error'});return {status:response.status};},accessToken);},
  introspect:({accessToken})=>{const config=environment.phase21.resource;const result=wire({mode:'oauth',path:'/oauth/introspect',client:config.clientId,credential:privateText(config.secretFile).trim(),form:{token:accessToken,resource:`${RP}/demo-resource`}});return {status:result.status,active:result.body?.active};},
  readFacts:policyReadFacts,
  recoveryGate:query=>{const program='import importlib.util,json,pathlib,sys; path=pathlib.Path(sys.argv[1]); spec=importlib.util.spec_from_file_location("phase23_invariant",path); module=importlib.util.module_from_spec(spec); spec.loader.exec_module(module); data=module.load_recovery_gate(sys.argv[2],int(sys.argv[3])); q=json.load(sys.stdin); r=data["rearmed"]; print(json.dumps({"credentialFloor":data["credentialFloors"][q["clientId"]],"memberIdentityFloor":data["membershipFloors"][q["memberId"]],"userSessionFloor":data["userFloors"][q["userId"]],"rearmedClient":q["clientId"] in r["clients"],"rearmedUser":q["userId"] in r["users"],"rearmedMember":q["memberId"] in r["memberships"]}))';return JSON.parse(wsl(['python3','-B','-c',program,`${tools}/phase23-invariant-cases.py`,`${TASK}/phase23-recovery-config.json`,String(report.latestDenySequence)],{input:JSON.stringify(query)}));}
}; }
function targetAclDriftProbe(sequence) {
  const name=manifest.database.replace('unified_auth_https_','unified_auth_restore_'),owner=manifest.rolePrefix+'_migration_owner',runtime=manifest.rolePrefix+'_platform_runtime';
  assert(/^unified_auth_restore_[0-9a-f]{32}$/.test(name)&&/^uah_[0-9a-f]{12}_migration_owner$/.test(owner),'ACL_TARGET_BINDING_REQUIRED');
  const guard=`DO $$ BEGIN IF (SELECT system_identifier::text FROM pg_control_system())<>${lit(manifest.clusterId)} OR current_setting('data_directory')<>${lit(clusterBinding.dataDirectory)} OR NOT EXISTS(SELECT 1 FROM pg_database d JOIN pg_roles r ON r.oid=d.datdba WHERE d.datname=${lit(name)} AND r.rolname=${lit(owner)} AND d.datconnlimit=0 AND shobj_description(d.oid,'pg_database')=${lit('phase23-recovery/2/'+ids.deployment)}) THEN RAISE EXCEPTION 'owned target ACL guard failed'; END IF; END $$;`;
  const call=statement=>wsl(['/usr/lib/postgresql/18/bin/psql','-X','-qAt','-v','ON_ERROR_STOP=1','-v','VERBOSITY=sqlstate','postgres://postgres@127.0.0.1:55494/postgres'],{input:`BEGIN; ${guard} ${statement}; COMMIT;`}).trim();
  const catalog=`SELECT COALESCE(jsonb_agg(jsonb_build_array(acl.grantee,acl.grantor,acl.privilege_type,acl.is_grantable) ORDER BY acl.grantee,acl.grantor,acl.privilege_type,acl.is_grantable),'[]'::jsonb) FROM pg_database d CROSS JOIN LATERAL aclexplode(COALESCE(d.datacl,acldefault('d',d.datdba))) acl WHERE d.datname=${lit(name)}`;
  const before=call(catalog);assert.equal(call(`SELECT has_database_privilege(${lit(runtime)},${lit(name)},'CONNECT')`),'t','ACL_POSITIVE_BASELINE_REQUIRED');
  let rejected=false;
  try{call(`SET LOCAL ROLE "${owner}"; REVOKE CONNECT ON DATABASE "${name}" FROM "${runtime}"`);assert.equal(call(`SELECT has_database_privilege(${lit(runtime)},${lit(name)},'CONNECT')`),'f','ACL_DRIFT_NOT_APPLIED');try{coreAction('gate',sequence);}catch(error){assert.equal(error.code,'RECOVERY_DATABASE_ACL_CHANGED','ACL_GATE_WRONG_FAILURE');rejected=true;}assert(rejected,'ACL_DRIFT_GATE_OPENED');}
  finally{call(`SET LOCAL ROLE "${owner}"; GRANT CONNECT ON DATABASE "${name}" TO "${runtime}"`);assert.equal(call(catalog),before,'ACL_DRIFT_RESTORATION_FAILED');}
  coreAction('gate',sequence);return {caseId:'DR_ACTUAL_TARGET_DATABASE_ACL_DRIFT',status:'PASS',revokedConnectObserved:true,gateCode:'RECOVERY_DATABASE_ACL_CHANGED',originalEffectiveAclRestored:true,targetStayedQuarantined:true};
}

try {
  step='PREFLIGHT';
  const frozenInputs=privateJson(`${TASK}/snapshot-${label}.json`).inputs;
  for(const name of ['phase23-browser.mjs','phase23-key-cases.mjs','phase23-dual-identity-cases.mjs','phase23-identity-driver.mjs','phase23-protocol-diagnostics.mjs','phase23-invitation-cases.mjs','phase23-policy-cases.mjs','phase23-service-get-cases.mjs','phase23-service-access-cases.mjs','phase23-service-context-cases.mjs','phase23-carrier-window.mjs','phase23-file-negative-evidence.mjs','phase23-recovery-key-cases.mjs'])assert.equal(hash(fs.readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)),name))),frozenInputs[`saas-plateform/backend/tests/unified-auth-https/${name}`],'WINDOWS_RUNNER_NOT_FROZEN_INPUT');
  if(fileNegativeEvidencePath){
    const evidence=JSON.parse(fs.readFileSync(fileNegativeEvidencePath,'utf8').replace(/^\uFEFF/,''));
    fileNegativeEvidence=validateFileNegativeEvidence(evidence,frozenInputs);
    const sourceArtifact=path.join(path.dirname(output),fileNegativeEvidence.sourceArtifactName);
    assert.notEqual(sourceArtifact,output,'PHASE23_FILE_NEGATIVE_EVIDENCE_PATH_INVALID');
    const sourceBytes=fs.readFileSync(sourceArtifact);
    assert.equal(hash(sourceBytes),fileNegativeEvidence.sourceArtifactSha256,'PHASE23_FILE_NEGATIVE_SOURCE_ARTIFACT_CHANGED');
    const sourceReport=JSON.parse(sourceBytes.toString('utf8').replace(/^\uFEFF/,''));
    assert.deepEqual(fileNegativeEvidence.result,sourceReport.recoveryNegatives,'PHASE23_FILE_NEGATIVE_SOURCE_RESULT_CHANGED');
  }
  assert.equal(sql('shared',`SELECT count(*) FROM auth_totp_factors WHERE user_id=${lit(ids.admin)}::uuid`),'0','FRESH_ADMIN_MFA_REQUIRED');
  assert.equal(envAction('status').mode,'SOURCE','SOURCE_ENVIRONMENT_REQUIRED');
  privateWrite(`${TASK}/phase23-protocol-control.json`,{contractVersion:'identity-phase23-protocol-control/1',sourcePath:`${frozen}/saas-examples/oidc-rp`,caFile:`${TASK}/phase23-ca.pem`});
  startProtocol();await until(()=>protocolReady||protocolFailure,'STANDARD_PROTOCOL_NOT_READY');assert(!protocolFailure,'STANDARD_PROTOCOL_FAILED');
  report.scramBefore=JSON.parse(wsl(['python3','-B',`${tools}/phase23-scram-cases.py`,'--mode','SOURCE','--task-dir',TASK,'--environment',`${TASK}/phase23-source-environment.json`,'--output',`${TASK}/phase23-scram-source.json`],{timeout:120000}));
  browser=await chromium.launch({headless:true,proxy:{server:'http://127.0.0.1:24445'}});report.browserVersion=browser.version();
  adminPage=(await contextPage()).page;await consoleLogin(adminPage,'admin');await enroll(adminPage,'admin');
  report.adminTotpSource=await currentAdminTotpSource(adminPage);pass({realEnrollment:true,currentBrowserTotpSourceVerified:true});

  step=validationProfile==='full'?'KEY_LIFECYCLE':'RECOVERY_KEY_PREPARATION';
  const keyPorts={keyCommand:request=>command(request),lookupOperation:operation=>operationLookup(operation),authorize:protocolLogin,rpc,
    jwksKids:async()=>{const {page}=await contextPage();await page.goto(`${AUTH}/oauth/jwks`,{waitUntil:'domcontentloaded'});return page.evaluate(()=>JSON.parse(document.body.textContent).keys.map(key=>key.kid));},
    keyWindow:kid=>row('shared',`SELECT active,retired_at IS NOT NULL AS retired,revoked_at IS NOT NULL AS revoked,extract(epoch FROM last_signed_exp)::double precision AS "lastSignedExp",extract(epoch FROM clock_timestamp())::double precision AS "dbNowEpoch" FROM auth_signing_keys WHERE kid=${lit(kid)}`),
    profile:async ttl=>{const result=envAction('profile',['--ttl-seconds',String(ttl)]);assert(result.loadedAfterRestart);environment=privateJson(`${TASK}/https-environment.json`);},
    signLogoutWithCollector,refreshAdminMfa:()=>refreshAdminMfa('BEFORE_NO_ACTIVE_SIGNER_WINDOW'),wait:async ms=>{await delay(ms);assert(!interrupted,'INTERRUPTED');},
    checkpointProgress:async value=>{report.keyProgress=value;persist();console.log(`PHASE23_KEY ${value.caseId} ${value.status}`);
      if(value.caseId==='K03_ACTIVE_ORIGINAL_RESULT'&&value.status==='PASS'){
        const processes=envAction('status').identityInstances;
        assert(processes?.independentProcesses===2&&processes.distinctPids===true,'TWO_SUPERVISED_IDENTITY_PROCESSES_REQUIRED');
        report.dualIdentityIssuance=await checkDualIdentityIssuance({authorize:protocolLogin,
          verify:key=>rpc('verify-old-id-session',{key}),sameSubject:(leftKey,rightKey)=>rpc('same-subject',{leftKey,rightKey})},'phase23-b');
        report.dualIdentityIssuance.processOwnership=processes;persist();
      }}};
  const keys=validationProfile==='full'?await runKeyCases(keyPorts):await prepareRecoveryKeyCarriers(keyPorts);
  report.keyCases=keys.cases;delete report.keyProgress;pass({caseCount:keys.cases.length,checkpointCReady:true,keyLifecycleExecuted:validationProfile==='full'});

  step='CHECKPOINT_PREPARATION';
  await consoleLogin(adminPage,'admin',true);
  const invitationModule=await import('./phase23-invitation-cases.mjs');
  let invitations=await invitationModule.runBeforeCheckpoint(invitationPorts);report.invitationBefore=invitations.facts;
  canaries(invitations.privateCarriers);
  const policyModule=await import('./phase23-policy-cases.mjs'),extraApplication=newId();
  sql('shared',`INSERT INTO auth_applications(application_id,ownership,status,display_name,purpose,approved_scopes) VALUES(${lit(extraApplication)}::uuid,'FIRST_PARTY','APPROVED','Owned Phase23 recovery policy extra','EXPLICIT_OWNED_RECOVERY_FIXTURE',ARRAY['demo.read','demo.admin'])`);
  // The standard context is created before the extra's public resource call.
  await protocolLogin();const policy=policyPorts(extraApplication);
  let policyState=await policyModule.runBeforeCheckpoint(policy);report.policyBefore=policyState.facts;
  report.serviceGetSource=await checkServiceGetControls({source:sourceEnvironment,ids,applicationId:extraApplication,request:serviceGet});
  canaries(policyState.privateCarriers);
  canaries(JSON.parse(sql('shared','SELECT COALESCE(jsonb_agg(password_hash),\'[]\'::jsonb) FROM users')),'secret');
  const employee=(await contextPage()).page,customer=(await contextPage()).page;
  const employeeBefore=await consoleLogin(employee,'employee');await portalLogin(customer,'customer');
  const proposedSecondMember=newId();
  sql('shared',`INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES(${lit(proposedSecondMember)}::uuid,${lit(ids.customer)}::uuid,${lit(ids.secondOrganization)}::uuid,'CUSTOMER') ON CONFLICT(user_id,organization_id,member_type) DO UPDATE SET status='ENABLED',identity_version=memberships.identity_version+1 WHERE memberships.status<>'ENABLED'`);
  const secondMember=sql('shared',`SELECT id FROM memberships WHERE user_id=${lit(ids.customer)}::uuid AND organization_id=${lit(ids.secondOrganization)}::uuid AND member_type='CUSTOMER' AND status='ENABLED'`);assert(uuid.test(secondMember),'SECOND_CUSTOMER_MEMBER_REQUIRED');
  report.serviceContextSource=await contextServiceControls(customer,secondMember);
  const customerBefore=await view(customer,'customer',true);
  report.serviceAccessSource=await checkServiceAccessControls({source:sourceEnvironment,employee:sessionAccess('employee'),customer:sessionAccess('customer'),request:servicePost});
  const bffBefore={employee:{page:employee,context:employee.context(),session:employeeBefore},customer:{page:customer,context:customer.context(),session:customerBefore}};
  for(const [name,record] of Object.entries(bffBefore)){const origin=name==='employee'?CONSOLE:CUSTOMER;record.cookies=(await record.context.cookies(origin)).filter(cookie=>name==='employee'?cookie.name==='__Host-saas_console':['__Host-saas_customer','__Host-saas_customer_browser'].includes(cookie.name));assert.equal(record.cookies.length,name==='employee'?1:2,'ORIGINAL_BFF_COOKIE_MATERIAL_REQUIRED');const facts=row(name==='employee'?'shared':'portal',`SELECT oidc_grant_id AS "grantId",extract(epoch FROM expires_at)*1000 AS "expiresAtMs",extract(epoch FROM access_expires_at)*1000 AS "accessExpiresAtMs" FROM ${name==='employee'?'bff_sessions':'portal_sessions'} WHERE user_id=${lit(ids[name])}::uuid AND revoked_at IS NULL ORDER BY created_at DESC,id DESC LIMIT 1`);record.deadlines=facts;const client=name==='employee'?ids.client:ids.portalClient;assert.equal((await receiver(record.page,fixtureLogout('phase23-c',client))).status,200,'PUBLISHED_C_RECEIVER_CONTROL_FAILED');const liveD=fixtureLogout('phase23-d',client);assert.equal((await receiver(record.page,liveD)).status,200,'VALID_D_RECEIVER_CONTROL_FAILED');assert.equal((await receiver(record.page,liveD)).status,409,'PRECHECKPOINT_JTI_REPLAY_NOT_REJECTED');}
  const capBefore=await json(employee,'/auth/context-token','POST',{expectedVersion:employeeBefore.version},{'X-CSRF-Token':employeeBefore.csrfToken});assert.equal(capBefore.status,200);const oldBusinessBearer=capBefore.body.accessToken;const oldBusinessExpiry=JSON.parse(Buffer.from(oldBusinessBearer.split('.')[1],'base64url')).exp*1000;assert.equal((await json(employee,'/api/auth/profile','GET',undefined,{Authorization:`Bearer ${oldBusinessBearer}`})).status,200,'SOURCE_EMPLOYEE_BUSINESS_CONTROL_FAILED');assert.equal((await json(customer,'/api/profile')).status,200,'SOURCE_CUSTOMER_BUSINESS_CONTROL_FAILED');
  canaries(capBefore.body);for(const record of Object.values(bffBefore))canaries(record.cookies);
  const oldUser=await protocolLogin(),oldOffline=await protocolLogin('offline'),reuse=await protocolLogin(),logoutCarrier=await protocolLogin();
  assert.equal((await rpc('userinfo',{key:oldUser.key})).status,200);assert((await rpc('refresh',{key:oldUser.key})).rotated);assert((await rpc('refresh',{key:oldOffline.key})).rotated);assert((await rpc('refresh',{key:reuse.key})).rotated);
  assert.equal((await rpc('userinfo',{key:oldUser.key})).status,200,'SOURCE_SUCCESSOR_ACCESS_CONTROL_FAILED');assert.equal((await rpc('userinfo',{key:oldOffline.key})).status,200,'SOURCE_OFFLINE_ACCESS_CONTROL_FAILED');
  const machine=await rpc('machine');assert.equal((await rpc('resource',{key:machine.key})).status,200);assert((await rpc('introspect',{key:machine.key})).active);
  // Bind the exact source token row while it still exists. SDK expires_in is
  // relative to its response and can overestimate the actual database cutoff.
  const oldAccessCarriers=[];
  for(const [key,version] of [[oldUser.key,'current'],[oldOffline.key,'current'],[keys.privateCarriers.c,'original'],[keys.privateCarriers.d,'original'],[machine.key,'original']]){
    const window=await rpc('carrier-window',{key});
    const sha256=version==='current'?window.currentAccessSha256:window.originalAccessSha256;
    assert(/^[0-9a-f]{64}$/.test(sha256??''),'ORIGINAL_ACCESS_DIGEST_REQUIRED');
    const deadline=row('shared',`SELECT extract(epoch FROM expires_at)*1000 AS "expiresAtMs" FROM auth_access_tokens WHERE token_digest=${lit(sha256)}`);
    assert(deadline&&Number.isFinite(Number(deadline.expiresAtMs)),'ORIGINAL_ACCESS_DEADLINE_REQUIRED');
    oldAccessCarriers.push({key,version,sha256,expiresAtMs:Number(deadline.expiresAtMs)});
  }
  const markerOrganization=environment.phase23.ids.markerOrganization;
  assert(uuid.test(markerOrganization??'')&&!([ids.organization,ids.secondOrganization].includes(markerOrganization)),'PHASE23_MARKER_ORGANIZATION_NOT_ISOLATED');
  const markerA=`phase23-checkpoint-${newId()}`,markerB=`phase23-post-checkpoint-${newId()}`;
  assert.equal((await management({method:'PUT',path:`/api/organizations/${markerOrganization}`,body:{description:markerA}})).status,200,'BUSINESS_MARKER_A_WRITE_FAILED');
  // Canonical codes retain their real 60-second policy. Issue the original
  // unconsumed code only after all preparatory management and token lookups.
  const lostCode=await rpc('begin',{profile:'third'});const codePage=(await contextPage()).page;let codeCallback;
  await codePage.route(`${RP}/auth/callback?*`,async route=>{codeCallback=route.request().url();await route.fulfill({status:200,contentType:'text/plain',body:'Private unused code held.'});});await codePage.goto(lostCode.authorizationUrl,{waitUntil:'domcontentloaded'});await identity(codePage,'standard',RP);assert(codeCallback,'UNCONSUMED_CODE_REQUIRED');
  const codeDeadline=row('shared',`SELECT extract(epoch FROM expires_at)*1000 AS "expiresAtMs",consumed_at IS NULL AS unconsumed FROM auth_authorization_codes WHERE code_digest=${lit(hash(new URL(codeCallback).searchParams.get('code')))}`);assert(codeDeadline.unconsumed,'SOURCE_CODE_ALREADY_CONSUMED');
  const sourceEnvironmentHash=hash(privateText(`${TASK}/phase23-source-environment.json`));
  // Park every browser page so no background BFF refresh races the offline cut.
  for(const context of contexts)for(const page of context.pages())await page.goto('about:blank');
  report.timing.checkpointPauseStartedAt=new Date().toISOString();envAction('stop-writers');
  report.recoveryCheckpointSteps={adapterPrepare:false,coreInit:false,coreCapture:false,negativeBaselineSaved:false};
  report.recoverySubstage='CHECKPOINT_ADAPTER_PREPARE';persist();
  const filesReady=JSON.parse(wsl(['python3','-B',`${tools}/phase23-recovery-files.py`,'prepare','--task-dir',TASK,'--environment',`${TASK}/phase23-source-environment.json`,'--manifest',`${TASK}/https-database.json`,'--output-config',`${TASK}/phase23-recovery-config.json`,'--tls-materials',`${TASK}/phase23-tls-materials.json`]));
  assert.equal(filesReady.status,'PHASE23_RECOVERY_FILES_PREPARED');report.recoveryCheckpointSteps.adapterPrepare=true;
  report.recoverySubstage='CHECKPOINT_CORE_INIT';persist();coreAction('init');report.recoveryCheckpointSteps.coreInit=true;
  report.recoverySubstage='CHECKPOINT_CORE_CAPTURE';persist();
  const checkpoint=coreAction('capture');assert(checkpoint.sequence>=1,'CHECKPOINT_SEQUENCE_REQUIRED');checkpointTaken=true;report.recoveryCheckpointSteps.coreCapture=true;
  report.checkpoint={sequence:checkpoint.sequence,sourceEnvironmentSha256:sourceEnvironmentHash,stoppedWriterDatabases:2,method:'sequential pg_dump while all three writers stopped'};
  report.recoverySubstage='CHECKPOINT_NEGATIVE_SAVE_BASELINE';persist();
  JSON.parse(wsl(['python3','-B',`${frozen}/saas-plateform/backend/ops/identity-recovery/negative_probe.py`,'--stage','save-baseline','--config',`${TASK}/phase23-recovery-config.json`,'--minimum-sequence',String(checkpoint.sequence)]));
  report.recoveryCheckpointSteps.negativeBaselineSaved=true;report.recoverySubstage='CHECKPOINT_BASELINE_SAVED';persist();
  envAction('start-writers');await employee.goto(`${CONSOLE}/auth/config`);await view(employee,'employee');await customer.goto(`${CUSTOMER}/auth/config`);await view(customer,'customer',true);await adminPage.goto(`${CONSOLE}/auth/config`);await view(adminPage,'admin');
  assert.equal((await rpc('userinfo',{key:oldUser.key})).status,200);report.timing.checkpointPauseEndedAt=new Date().toISOString();report.sourceResumedBusinessAfterCheckpoint=true;pass({freshLiveUserAndMachineCarriers:true,sourceReallyResumed:true});

  step='POST_CHECKPOINT_DENY';
  assert.equal((await management({method:'PUT',path:`/api/organizations/${markerOrganization}`,body:{description:markerB}})).status,200,'BUSINESS_MARKER_B_WRITE_FAILED');
  const revokedC=await command({path:'/api/identity/keys/phase23-c/revoke'});assert.equal(revokedC.status,200);
  for(const [name,record] of Object.entries(bffBefore)){await record.page.goto(`${name==='employee'?CONSOLE:CUSTOMER}/auth/config`);const client=name==='employee'?ids.client:ids.portalClient;assert.equal((await receiver(record.page,fixtureLogout('phase23-c',client))).status,400,'REVOKED_C_SYSTEM_RP_RECEIVER_ACCEPTED');assert.equal((await receiver(record.page,fixtureLogout('phase23-d',client))).status,200,'VALID_D_SYSTEM_RP_RECEIVER_REJECTED');}
  const cFacts=await rpc('verify-old-id-session',{key:keys.privateCarriers.c});assert(cFacts.cachedKeySignatureValid&&!cFacts.currentAuthorityJwtValid&&cFacts.currentVerificationCode==='KEY_NOT_CURRENT'&&cFacts.onlineStatus===200,'EMERGENCY_C_THREE_LAYER_SEMANTICS_FAILED');
  const dFacts=await rpc('verify-old-id-session',{key:keys.privateCarriers.d});assert(dFacts.currentAuthorityJwtValid&&dFacts.onlineStatus===200,'VALID_D_STOPPED_WORKING');report.keyEmergency={cachedRsaMath:true,freshAuthorityRejectsC:true,opaqueGrantStillLiveBeforeRecovery:true,validD:true};
  invitations=await invitationModule.runAfterCheckpoint(invitationPorts,invitations.privateCarriers);report.invitationAfterCheckpoint=invitations.facts;
  policyState=await policyModule.runAfterCheckpoint(policy,policyState.privateCarriers);report.policyAfterCheckpoint=policyState.facts;
  canaries(policyState.privateCarriers);
  await rpc('logout',{key:logoutCarrier.key,scope:'CURRENT_APP'});
  const consentPage=lastStandardPage;await consentPage.goto(`${AUTH}/sso/applications`,{waitUntil:'domcontentloaded'});
  const ssoBefore=await json(consentPage,'/api/sso/session');assert.equal(ssoBefore.status,200,'SOURCE_SSO_CONTROL_FAILED');
  const oldSsoCookies=await consentPage.context().cookies(AUTH);assert(oldSsoCookies.length,'ORIGINAL_SSO_COOKIE_REQUIRED');
  const offlineConsent=row('shared',`SELECT id,version FROM auth_consents WHERE user_id=${lit(phase.ids.user)}::uuid AND group_id=${lit(phase.ids.offlineGroup)}::uuid AND status='ACTIVE'`);
  const consentRevoke=await json(consentPage,`/api/sso/consents/${offlineConsent.id}/revoke`,'POST',{operationId:newId(),expectedVersion:offlineConsent.version},{'X-CSRF-Token':ssoBefore.body.csrfToken});assert.equal(consentRevoke.status,200,'REAL_CONSENT_REVOKE_FAILED');
  const consentHide=await json(consentPage,`/api/sso/consents/${offlineConsent.id}`,'DELETE',{operationId:newId(),expectedVersion:consentRevoke.body.version},{'X-CSRF-Token':ssoBefore.body.csrfToken});assert.equal(consentHide.status,200,'REAL_CONSENT_DISPLAY_DELETE_FAILED');
  const replay=await rpcRaw('replay',{key:reuse.key});assert(!replay.ok&&replay.status>=400,'REAL_REFRESH_REPLAY_ACCEPTED');
  const staleRefresh=await rpcRaw('refresh',{key:reuse.key});assert(!staleRefresh.ok&&staleRefresh.status>=400,'REUSED_FAMILY_STILL_ACTIVE');
  // Canonical current-app commands generate immutable receipts and real worker
  // tombstones in the platform and portal, while the original cookies are kept.
  for(const [name,record] of Object.entries(bffBefore)){const page=record.page;await page.goto(`${name==='employee'?CONSOLE:CUSTOMER}/auth/config`);const result=await json(page,'/auth/logout','POST',{expectedVersion:record.session.version,scope:name==='employee'?'PLATFORM':'PORTAL'},{'X-CSRF-Token':record.session.csrfToken});assert.equal(result.status,200,'BEFORE_RECOVERY_BFF_LOGOUT_FAILED');}
  for(const [name,record] of Object.entries(bffBefore)){await until(()=>sql('shared',`SELECT state FROM auth_delivery_jobs WHERE grant_id=${lit(record.deadlines.grantId)}::uuid AND kind='LOGOUT'`)==='DELIVERED','POST_CHECKPOINT_SYSTEM_RP_DELIVERY_MISSING');assert(Number(sql(name==='employee'?'shared':'portal',`SELECT count(*) FROM ${name==='employee'?'platform':'portal'}_backchannel_logout_receipts`))>=1,'SYSTEM_RP_RECEIPT_MISSING');}
  pass({emergencyCRevoked:true,refreshReuseDenied:true,bffLogoutCommands:2,standardLogoutReceipt:true,offlineConsentRevokedAndDisplayHidden:true});

  step='DISASTER_CUT';
  for(const context of contexts)for(const page of context.pages())await page.goto('about:blank');
  actualFaultStart=Date.now();actualFaultStartMonotonic=performance.now();report.timing.actualFaultStartedAt=new Date(actualFaultStart).toISOString();await recoveryStep('STOP_SOURCE_WRITERS',()=>envAction('stop-writers'));
  const deny=await recoveryStep('CAPTURE_LATEST_DENY',()=>coreAction('capture-deny',checkpoint.sequence));assert(deny.sequence>checkpoint.sequence,'LATEST_DENY_SEQUENCE_REQUIRED');report.latestDenySequence=deny.sequence;
  await recoveryStep('QUARANTINE_SOURCE',()=>envAction('quarantine-source'));
  // File mutation probes are a separate component suite. An explicit receipt
  // may reuse this turn's actual suite only when every loader byte still
  // matches. All real restore/authentication/catalog gates below still run.
  if(fileNegativeEvidence){report.recoveryNegatives={...fileNegativeEvidence.result,sourceSnapshot:fileNegativeEvidence.sourceSnapshot,sourceArtifactName:fileNegativeEvidence.sourceArtifactName,sourceArtifactSha256:fileNegativeEvidence.sourceArtifactSha256,separatelyExecuted:true,inputDigestsVerified:true};}
  else{report.recoveryNegatives=await recoveryStep('ACTUAL_FILE_NEGATIVES',()=>JSON.parse(wsl(['python3','-B',`${frozen}/saas-plateform/backend/ops/identity-recovery/negative_probe.py`,'--stage','probe','--config',`${TASK}/phase23-recovery-config.json`,'--minimum-sequence',String(deny.sequence)],{timeout:180000})));}
  const selection={users:[ids.admin,ids.employee,ids.customer,phase.ids.user],clients:[ids.client,ids.portalClient,phase.ids.thirdClient,phase.ids.offlineClient,phase.ids.machineClient,phase.ids.resourceClient],memberships:[ids.adminMember,ids.employeeMember,ids.customerMember,secondMember],services:{identity_service_clients:[ids.accountService,ids.sourceService,ids.reviewService,ids.invitationService,ids.authAdminService,environment.phase23.ids.keyAdminService,ids.customerAuthorityService,ids.portalSourceService],context_selection_services:[ids.selectionService,ids.portalConsumerService],platform_resource_services:[ids.profileClient]}};
  privateWrite(`${TASK}/phase23-rearm-selection.json`,selection);
  const cycle=await recoveryStep('RESTORE_RECONCILE_REARM_GATE',()=>JSON.parse(wsl(['python3','-B',`${tools}/phase23-recovery-cycle.py`,'--config',`${TASK}/phase23-recovery-config.json`,'--environment',`${TASK}/phase23-source-environment.json`,'--selection',`${TASK}/phase23-rearm-selection.json`,'--output-env',`${TASK}/phase23-target-environment.json`,'--output-rearm',`${TASK}/phase23-target-rearm.json`,'--minimum-sequence',String(deny.sequence),'--output',`${TASK}/phase23-recovery-cycle-receipt.json`],{timeout:300000})));
  report.recoveryCycle=cycle;
  assert.equal(cycle.status,'GATE_PASSED_QUARANTINED','RESTORE_CYCLE_NOT_QUARANTINED_GATE');
  report.actualAclDrift=await recoveryStep('ACTUAL_TARGET_ACL_DRIFT',()=>targetAclDriftProbe(deny.sequence));
  const target=privateJson(`${TASK}/phase23-target-environment.json`);
  canaries(target);
  await recoveryStep('ROTATE_DATABASE_PASSWORDS',()=>envAction('rotate-db-passwords',['--passwords',target.phase23.runtimePasswordsFile,'--environment',`${TASK}/phase23-target-environment.json`]));
  const gate=coreAction('gate',deny.sequence);privateWrite(`${TASK}/phase23-final-gate.json`,gate);
  envAction('use-target',['--environment',`${TASK}/phase23-target-environment.json`,'--gate-receipt',`${TASK}/phase23-final-gate.json`,'--recovery-config',`${TASK}/phase23-recovery-config.json`]);envAction('open-target');
  // A real fresh-password SCRAM probe runs while the three target pools are 0.
  const scram=await recoveryStep('FRESH_CONNECTION_SCRAM',()=>JSON.parse(wsl(['python3','-B',`${tools}/phase23-scram-cases.py`,'--mode','RESTORE','--task-dir',TASK,'--environment',`${TASK}/phase23-target-environment.json`,'--output',`${TASK}/phase23-scram-restore.json`],{timeout:120000})));report.scramAfter=scram;
  coreAction('gate',deny.sequence);
  // Bring up the already gated identity pair first, and test the exact old
  // code before spending its short window starting both business pools.
  await recoveryStep('START_TARGET_IDENTITY_PAIR',()=>envAction('profile',['--ttl-seconds','300']));environment=target;databaseMode='RESTORE';
  await rpc('reload');
  const authorityNow=()=>Number(sql('shared','SELECT extract(epoch FROM clock_timestamp())*1000'));
  report.oldCarrierCheck='AUTHORIZATION_CODE';
  const codeResult=await whileUnexpired({expiresAtMs:Number(codeDeadline.expiresAtMs),now:()=>{const now=authorityNow();report.oldCodeRemainingObservationMs=Math.floor(Number(codeDeadline.expiresAtMs)-now);return now;},run:()=>rpcRaw('finish',{key:lostCode.key,callback:codeCallback})});
  assert(!codeResult.result.ok&&[400,401].includes(codeResult.result.status),'OLD_CODE_REVIVED');
  report.oldAuthorizationCode={status:'PASS',originalUnconsumedCode:true,canonicalPolicySeconds:60,remainingAfterDenialMs:Math.floor(codeResult.remainingMs),checkedBeforeBusinessPoolsStarted:true};
  await recoveryStep('START_TARGET_BUSINESS_POOLS',()=>envAction('start-writers'));pass({targetFreshStarted:true,sourceRemainsQuarantined:true,originalCodeRejectedWhileUnexpired:true});

  step='OLD_RAW_CARRIERS_REJECTED';
  const denialDeadlines=[];
  for(const carrier of oldAccessCarriers){
    const window=await rpc('carrier-window',{key:carrier.key});
    assert.equal(carrier.version==='current'?window.currentAccessSha256:window.originalAccessSha256,carrier.sha256,'ORIGINAL_ACCESS_CARRIER_CHANGED');
  }
  for(const carrier of oldAccessCarriers.slice(0,4)){
    report.oldCarrierCheck=['USER_ONLINE','USER_OFFLINE','USER_C','USER_D'][oldAccessCarriers.indexOf(carrier)];
    const observed=await whileUnexpired({expiresAtMs:carrier.expiresAtMs,now:authorityNow,run:()=>rpcRaw('userinfo',{key:carrier.key,version:carrier.version})});
    assert(!observed.result.ok&&observed.result.status===401,'OLD_USER_ACCESS_REVIVED');denialDeadlines.push(observed.remainingMs);
  }
  for(const key of [oldUser.key,oldOffline.key]){const result=await rpcRaw('refresh',{key});assert(!result.ok&&[400,401].includes(result.status),'OLD_REFRESH_REVIVED');}
  const machineDeadline=oldAccessCarriers.at(-1).expiresAtMs;
  report.oldCarrierCheck='MACHINE_ACCESS';
  const oldMachine=await whileUnexpired({expiresAtMs:machineDeadline,now:authorityNow,run:()=>rpc('introspect',{key:machine.key,version:'original'})});
  assert(!oldMachine.result.active,'OLD_MACHINE_TOKEN_REVIVED');denialDeadlines.push(oldMachine.remainingMs);
  const oldMachineResource=await whileUnexpired({expiresAtMs:machineDeadline,now:authorityNow,run:()=>rpc('resource',{key:machine.key,version:'original'})});
  assert.equal(oldMachineResource.result.status,401);denialDeadlines.push(oldMachineResource.remainingMs);
  for(const [profile,operation,key] of [['third','refresh',oldUser.key],['offline','refresh',oldOffline.key],['machine','machine',undefined],['resource','introspect',machine.key]]){const result=await rpc('old-credential',{profile,operation,...(key?{key}:{})});assert([401,403].includes(result.status),'OLD_CLIENT_SECRET_NOT_REJECTED');}
  assert.equal((await rpc('logout',{key:logoutCarrier.key,scope:'CURRENT_APP',recover:true})).status,200,'ORIGINAL_LOGOUT_RECEIPT_LOST');
  report.oldCarrierCheck='SSO_COOKIE';
  const oldSso=await whileUnexpired({expiresAtMs:Date.parse(ssoBefore.body.expiresAt),now:authorityNow,run:async()=>{
    await consentPage.context().addCookies(oldSsoCookies);await consentPage.goto(`${AUTH}/sso/applications`,{waitUntil:'domcontentloaded'});return json(consentPage,'/api/sso/session');
  }});assert.equal(oldSso.result.status,401,'OLD_SSO_COOKIE_REVIVED');
  const receipt=await invitationModule.runAfterRecovery(invitationPorts,invitations.privateCarriers);report.invitationAfterRecovery=receipt.facts;
  const policyRestored=await policyModule.runAfterRecovery(policy,policyState.privateCarriers);report.policyAfterRecovery=policyRestored.facts;
  for(const [name,record]of Object.entries(bffBefore)){
    const page=record.page;
    report.oldCarrierCheck=name==='employee'?'EMPLOYEE_BFF_COOKIE':'CUSTOMER_BFF_COOKIE';
    const oldBff=await whileUnexpired({expiresAtMs:Math.min(Number(record.deadlines.expiresAtMs),Number(record.deadlines.accessExpiresAtMs)),now:authorityNow,run:async()=>{
      await record.context.addCookies(record.cookies);await page.goto(`${name==='employee'?CONSOLE:CUSTOMER}/auth/config`);return json(page,'/auth/session');
    }});assert.equal(oldBff.result.status,401,'OLD_BFF_COOKIE_REVIVED');
    if(name==='employee'){
      report.oldCarrierCheck='BUSINESS_BEARER';
      const oldBusiness=await whileUnexpired({expiresAtMs:oldBusinessExpiry,now:authorityNow,run:()=>json(page,'/api/auth/profile','GET',undefined,{Authorization:`Bearer ${oldBusinessBearer}`})});
      assert.equal(oldBusiness.result.status,401,'OLD_BUSINESS_BEARER_REVIVED');
    }
  }
  for(const name of ['admin','employee','customer','standard']){const before=name==='standard'?sourceEnvironment.phase21.accounts.user:sourceEnvironment.accounts[name];const after=account(name);assert.notEqual(before.password,after.password,'PRIMARY_PASSWORD_NOT_ROTATED');const {page}=await contextPage();const begin=await rpc('begin',{profile:'third',prompt:'login'});await page.goto(begin.authorizationUrl,{waitUntil:'domcontentloaded'});await page.locator('#login-form:not([hidden])').waitFor();await page.locator('#username').fill(before.username);await page.locator('#password').fill(before.password);const denied=page.waitForResponse(response=>response.url()===`${AUTH}/api/sso/login`&&response.request().method()==='POST');await page.locator('#login-submit').click();assert.equal((await denied).status(),401,'OLD_PRIMARY_PASSWORD_REVIVED');await page.goto('about:blank');}
  const restoredMarker=row('shared',`SELECT description,status FROM organizations WHERE id=${lit(markerOrganization)}::uuid`);assert.equal(restoredMarker.description,markerA,'CHECKPOINT_BUSINESS_MARKER_NOT_RESTORED');assert.notEqual(restoredMarker.description,markerB,'POST_CHECKPOINT_DATA_WRONGFULLY_PRESENT');
  report.businessRpo={sourceMutationVia:'actual authorized platform PUT',restoredObservationVia:'guarded owner SQL',checkpointValueRestored:true,postCheckpointValueLost:true,restoredOrganizationStatus:restoredMarker.status,securityIsolationSeparate:true};pass({originalProtocolProcessRetained:true,exactOriginalCookieBytesResubmitted:true,oldUserAccess:4,oldRefresh:2,oldMachine:1,oldClientSecrets:4,oldPrimaryPasswords:4,oldCode:1,oldBffCookies:2,originalDatabaseAccessDeadlines:true,deadlinesCheckedAfterDenial:true,minimumOldAccessRemainingMs:Math.floor(Math.min(...denialDeadlines)),oldCodeRemainingMs:Math.floor(codeResult.remainingMs)});
  step='RECOVERED_OLD_INVARIANTS';
  report.invariants=JSON.parse(wsl(['python3','-B',`${tools}/phase23-invariant-cases.py`,'--config',`${TASK}/phase23-recovery-config.json`,'--source-env',`${TASK}/phase23-source-environment.json`,'--target-env',`${TASK}/phase23-target-environment.json`,'--minimum-sequence',String(report.latestDenySequence),'--validation-profile',validationProfile,'--output',`${TASK}/phase23-invariants.json`],{timeout:120000}));pass({beforeAnyFreshPositiveLogin:true});

  step='FRESH_RECOVERED_BUSINESS';
  freshBusinessStage('AUXILIARY_RESTART');
  const oldControl=privateJson(`${TASK}/phase23-source-environment.json`).phase20.collector.controlSecret;
  envAction('restart-rp');envAction('restart-collector');assert.equal(wire({mode:'control',path:'/control/stats',method:'GET',controlSecret:oldControl}).status,404,'OLD_COLLECTOR_KEY_ACCEPTED');assert.equal(wire({mode:'control',path:'/control/stats',method:'GET'}).status,200);
  const freshEmployee=(await contextPage()).page,freshCustomer=(await contextPage()).page;
  freshBusinessStage('EMPLOYEE_LOGIN');await consoleLogin(freshEmployee,'employee');
  freshBusinessStage('CUSTOMER_LOGIN');await portalLogin(freshCustomer,'customer');
  freshBusinessStage('EMPLOYEE_BUSINESS');
  const employeeSession=await view(freshEmployee,'employee'),employeeCap=await json(freshEmployee,'/auth/context-token','POST',{expectedVersion:employeeSession.version},{'X-CSRF-Token':employeeSession.csrfToken});assert.equal(employeeCap.status,200);assert.equal((await json(freshEmployee,'/api/auth/profile','GET',undefined,{Authorization:`Bearer ${employeeCap.body.accessToken}`})).status,200,'NEW_EMPLOYEE_BUSINESS_FAILED');
  freshBusinessStage('CUSTOMER_BUSINESS');const profile=await json(freshCustomer,'/api/profile');assert.equal(profile.status,200,'NEW_CUSTOMER_BUSINESS_FAILED');
  freshBusinessStage('STANDARD_LOGIN');
  const freshStandard=await protocolLogin();assert.equal(freshStandard.kid,'phase23-d');assert.equal((await rpc('userinfo',{key:freshStandard.key})).status,200);assert((await rpc('same-subject',{leftKey:keys.privateCarriers.d,rightKey:freshStandard.key})).matches,'OPAQUE_SUBJECT_CHANGED_AFTER_RECOVERY');
  freshBusinessStage('MACHINE_RESOURCE');const newMachine=await rpc('machine');assert.equal((await rpc('resource',{key:newMachine.key})).status,200);
  freshBusinessStage('SERVICE_GET_HTTP');
  report.serviceGetTarget=await checkServiceGetControls({source:sourceEnvironment,target:environment,ids,applicationId:extraApplication,request:serviceGet});
  freshBusinessStage('SERVICE_ACCESS_HTTP');report.serviceAccessTarget=await checkServiceAccessControls({source:sourceEnvironment,target:environment,employee:sessionAccess('employee'),customer:sessionAccess('customer'),request:servicePost});
  freshBusinessStage('SERVICE_CONTEXT_HTTP');report.serviceContextTarget=await contextServiceControls(freshCustomer,secondMember,environment);
  await view(freshCustomer,'customer',true);
  assert.equal(report.serviceGetSource.length+report.serviceAccessSource.length+report.serviceContextSource.length,11,'PHASE23_SOURCE_SERVICE_HTTP_COVERAGE_INCOMPLETE');
  assert.equal(report.serviceGetTarget.length+report.serviceAccessTarget.length+report.serviceContextTarget.length,11,'PHASE23_SERVICE_HTTP_COVERAGE_INCOMPLETE');
  assert(!('OIDC_TOKEN_TTL_SECONDS' in environment.identity),'RETIRED_TTL_CONFIGURATION_PRESENT');
  freshBusinessStage('COMPLETE');
  report.timing.recoveredBusinessCompletedAt=new Date().toISOString();report.timing.rtoMilliseconds=Math.round(performance.now()-actualFaultStartMonotonic);report.timing.rtoWallClockMilliseconds=Date.now()-actualFaultStart;report.timing.rpoMilliseconds=new Date(report.timing.actualFaultStartedAt)-new Date(report.timing.checkpointPauseStartedAt);report.timing.target={rpoMinutes:15,rtoHours:4,status:'INTERNAL_CANDIDATE'};
  envAction('stop-writers');
  canaries(employeeCap.body);
  for(const record of [sourceEnvironment,environment])for(const kind of ['identity','platform','portal'])canaries(decodeURIComponent(new URL(record[kind].DATABASE_URL).password),'password');
  for(const record of [sourceEnvironment,environment])for(const kind of ['identity','platform','portal'])for(const [name,value]of Object.entries(record[kind]))if(name.endsWith('_SECRET')&&typeof value==='string'&&typeof record[kind][name.slice(0,-7)+'_ID']==='string')canaries(Buffer.from(record[kind][name.slice(0,-7)+'_ID']+':'+value).toString('base64'),'secret');
  for(const record of [sourceEnvironment,environment]){canaries(privateJson(record.identity.OIDC_KEYSET_PATH));for(const field of ['AUTH_INVITATION_KEYSET_PATH','CONTEXT_SELECTION_KEY_FILE'])canaries(privateJson(record.identity[field]));canaries(privateJson(record.platform.BFF_TOKEN_KEY_FILE));canaries(privateJson(record.portal.PORTAL_TOKEN_KEY_FILE));}
  canaries(privateText(`${TASK}/phase23-server.key`),'privateKeyPem');
  canaries(Object.values(privateJson(`${TASK}/phase23-recovery-keys.json`)),'secret');
  canaries(JSON.parse(sql('shared','SELECT COALESCE(jsonb_agg(password_hash),\'[]\'::jsonb) FROM users')),'secret');
  report.protocolLogRedaction=await rpc('scan-task-logs');report.ownerLogRedaction=scanOwnedLogs();assert.equal(report.protocolLogRedaction.candidateFindings,0,'PROTOCOL_CARRIER_LOG_LEAK');assert.equal(report.ownerLogRedaction.candidateFindings,0,'PRIVATE_MATERIAL_LOG_LEAK');
  pass({newEmployee:true,newCustomer:true,newStandardRp:true,newMachine:true,
    requestedLegacyOidcTokenTtlSeconds:300,canonicalDeadlinesCheckedFromActualCarriers:true,subjectStable:true});
  await Promise.allSettled([...observations]);assert(tls.size&&[...tls].every(value=>/^TLS [12]\./.test(value)),'VERIFIED_TLS_REQUIRED');assert(!interrupted,'INTERRUPTED');report.result='PASS';
} catch(error) {
  report.failedStep=step;report.failureCode=safeFailureCode(error,interrupted);process.exitCode=1;
  const protocolCause=safeProtocolCause(error?.protocolCause);if(protocolCause)report.protocolFailureCause=protocolCause;
  if(activePage&&!activePage.isClosed())report.page=await activePage.evaluate(()=>({origin:location.origin,pathname:location.pathname.replace(/\/[a-f0-9-]{36}(?=\/|$)/g,'/:id'),screen:document.querySelector('#auth-content')?.dataset.screen??null,
    candidateCount:document.querySelectorAll('input[name="context-membership"]').length,
    candidateMemberTypes:[...new Set([...document.querySelectorAll('.member-choice-type')].map(node=>node.textContent==='员工身份'?'EMPLOYEE':node.textContent==='客户身份'?'CUSTOMER':'UNKNOWN'))].sort(),
    identityBusy:document.querySelector('#auth-content')?.getAttribute('aria-busy')==='true',
    organizationStepVisible:document.querySelector('#organization-step')?.hidden===false,
    selectionSearchActive:Boolean(document.querySelector('#selection-search')?.value),
    consentEnabled:document.querySelector('#consent-allow')?.disabled===false})).catch(()=>null);
} finally {
  if(protocolReady&&!protocolFailure&&protocol?.exitCode===null){try{report.protocolLogRedaction=await rpc('scan-task-logs');if(report.protocolLogRedaction.candidateFindings){report.result='FAIL';process.exitCode=1;}}catch{report.protocolLogRedaction={result:'SCAN_FAILED'};report.result='FAIL';process.exitCode=1;}}
  try{report.ownerLogRedaction=scanOwnedLogs();if(report.ownerLogRedaction.candidateFindings){report.result='FAIL';process.exitCode=1;}}catch{report.ownerLogRedaction={result:'SCAN_FAILED'};report.result='FAIL';process.exitCode=1;}
  protocol?.stdin.end();await Promise.allSettled([...observations]);for(const context of contexts)await context.close().catch(()=>{});await browser?.close().catch(()=>{});
  if(protocol){await Promise.race([new Promise(resolve=>protocol.once('exit',resolve)),delay(3000)]);if(protocol.exitCode===null)protocol.kill();}
  try { report.environmentCleanupFirstPass=envAction('cleanup'); } catch { report.environmentCleanupFirstPass={result:'FAIL',requiresOwnerFollowup:true};report.result='FAIL';process.exitCode=1; }
  for(const factor of factors.values())factor.key.fill(0);report.tlsProtocols=[...tls];report.endedAt=new Date().toISOString();report.cleanupRequiresEnvironmentAndHbaOwners=true;report.checkpointTaken=checkpointTaken;
  assert(!JSON.stringify(report).match(/"(?:access_token|refresh_token|id_token|logout_token|password|clientSecret|cookie|verifier)"\s*:/i),'REPORT_SECRET_GUARD_FAILED');persist();console.log(`PHASE23_HTTPS_RESULT ${JSON.stringify({result:report.result,checks:report.checks.length,failedStep:report.failedStep,failureCode:report.failureCode})}`);process.removeListener('SIGINT',interrupt);process.removeListener('SIGTERM',interrupt);
}

