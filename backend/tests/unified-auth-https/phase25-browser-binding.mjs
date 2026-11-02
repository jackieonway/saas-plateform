/** Descriptor-checked private inputs and frozen entrypoint proof for old suites. */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const TASK = '/tmp/rust-saas-unified-auth-20261028';
const here = path.dirname(fileURLToPath(import.meta.url));
const unix = filename => process.platform === 'linux' ? filename
  : '/mnt/' + filename[0].toLowerCase() + filename.slice(2).replaceAll('\\', '/');
let tools = unix(here);
const hash = value => createHash('sha256').update(value).digest('hex');

function python(program, argumentsList) {
  try {
    return JSON.parse(execFileSync(process.platform === 'linux' ? 'python3' : 'wsl.exe',
      [...(process.platform === 'linux' ? [] : ['-d', 'Ubuntu', '-u', 'root', '--exec', 'python3']),
        '-B', '-c', program, TASK, tools, ...argumentsList],
      { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true, timeout: 15000, maxBuffer: 8 * 1024 * 1024 }));
  } catch { throw new Error('OWNED_BROWSER_INPUT_BINDING_FAILED'); }
}
const header = String.raw`
import hashlib,json,os,pathlib,re,stat,sys
root=pathlib.Path(sys.argv[1]);tools=pathlib.Path(sys.argv[2])
assert root.resolve(strict=True)==root and tools.resolve(strict=True)==tools
sys.path.insert(0,str(tools));import phase23_cluster_binding as binding
bound=binding.load_binding(root);uid=bound['taskUid']
def private(p):
 assert p.is_relative_to(root) and p.resolve(strict=True)==p
 return binding.protected_read(p,uid=uid,mode=0o600,maximum=8*1024*1024)[0]
`;

export function readOwnedBrowserJson(filename) {
  assert(typeof filename === 'string' && filename.startsWith(TASK + '/') && !filename.split('/').includes('..'),
    'OWNED_BROWSER_FILE_REQUIRED');
  return python(header + String.raw`
p=pathlib.Path(sys.argv[3])
if p==root/'namespace.pid':
 info=p.lstat();assert stat.S_ISREG(info.st_mode) and info.st_uid==0 and info.st_nlink==1 and stat.S_IMODE(info.st_mode) in (0o600,0o644)
 value=json.loads(p.read_bytes());proc=pathlib.Path('/proc')/str(value['pid'])
 assert value['pid']>1 and proc.stat().st_uid==0 and proc.joinpath('stat').read_text().rsplit(')',1)[1].split()[19]==value['startTime']
 assert os.readlink(proc/'ns/mnt')==value['namespace']==root.joinpath('namespace.identity').read_text().strip()
 print(json.dumps(value))
else:print(private(p).decode())
`, [filename]);
}

export function loadOwnedBrowserInputs({ environmentFile, executionPhase, legacyPhase, snapshot, entrypoint }) {
  assert([legacyPhase, 25].includes(executionPhase) && ['phase19-browser.mjs', 'phase20-browser.mjs'].includes(path.basename(entrypoint)),
    'OWNED_BROWSER_PHASE_REQUIRED');
  if (executionPhase === 25) {
    assert(/^phase25-tools-r[1-9][0-9]*$/.test(snapshot ?? '') && environmentFile === TASK + '/https-environment.json',
      'PHASE25_BROWSER_FROZEN_SOURCE_REQUIRED');
    tools = TASK + '/source-snapshot-' + snapshot + '/saas-plateform/backend/tests/unified-auth-https';
  } else assert(snapshot === undefined, 'LEGACY_BROWSER_SNAPSHOT_ARGUMENT_REJECTED');
  const value = python(header + String.raw`
filename=pathlib.Path(sys.argv[3]);label=sys.argv[4];entry=sys.argv[5]
assert filename.name=='https-environment.json'
environment=json.loads(private(filename));manifest=json.loads(private(filename.with_name('https-database.json')))
assert manifest['clusterId']==bound['systemIdentifier'] and re.fullmatch(r'unified_auth_https_[0-9a-f]{32}',manifest['database'])
out={'environment':environment,'manifest':manifest,'clusterId':bound['systemIdentifier'],'dataDirectory':bound['dataDirectory'],'frozenInputs':{}}
if label:
 snapshot=root/('source-snapshot-'+label);source=json.loads(private(root/('snapshot-'+label+'.json')))
 assert source['snapshotSha256']==hashlib.sha256(json.dumps(source['inputs'],sort_keys=True).encode()).hexdigest()
 for name in [entry,'phase25-browser-binding.mjs','phase23_cluster_binding.py']:
  relative='saas-plateform/backend/tests/unified-auth-https/'+name
  assert hashlib.sha256(private(snapshot/relative)).hexdigest()==source['inputs'][relative]
  if name.endswith('.mjs'):out['frozenInputs'][name]=source['inputs'][relative]
 out['snapshotSha256']=source['snapshotSha256']
print(json.dumps(out))
`, [environmentFile, snapshot ?? '', path.basename(entrypoint)]);
  for (const [name, expected] of Object.entries(value.frozenInputs)) {
    assert.equal(hash(fs.readFileSync(path.join(here, name))), expected, 'PHASE25_BROWSER_LIVE_ENTRYPOINT_CHANGED');
  }
  return value;
}

export function validateBrowserReport({ outputFile, executionPhase, legacyPhase }) {
  if (executionPhase !== 25) return;
  const validation = path.resolve(here, '../../docs/designs/unified-auth/validation');
  const kind = legacyPhase === 19 ? 'management' : 'logout';
  assert(path.dirname(outputFile) === validation && new RegExp(`^phase-25-${kind}-[a-z0-9-]+\\.json$`).test(path.basename(outputFile))
    && !fs.lstatSync(validation).isSymbolicLink() && !fs.existsSync(outputFile), 'PHASE25_BROWSER_NEW_SAFE_OUTPUT_REQUIRED');
}

export function saveBrowserReport(outputFile, report, executionPhase) {
  fs.mkdirSync(path.dirname(outputFile), { recursive: true });
  fs.writeFileSync(outputFile, JSON.stringify(report, null, 2) + '\n', { flag: executionPhase === 25 ? 'wx' : 'w', mode: 0o600 });
}
