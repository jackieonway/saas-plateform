import assert from 'node:assert/strict'
import test from 'node:test'
import { createHash } from 'node:crypto'
import { cpSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { syncSdk, generateArtifacts, validateRelease } from '../scripts/sync-micro-sdk.mjs'
import { SDK_VERSION, isSupportedSdkVersion, assertSupportedSdkVersion } from '../examples/micro-app-sdk/generated/protocol.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sdk = 'examples/micro-app-sdk'
const sha = bytes => createHash('sha256').update(bytes).digest('hex')
const release = JSON.parse(readFileSync(path.join(root,sdk,'release.json')))
function isolated(t) {
  const directory=mkdtempSync(path.join(os.tmpdir(),'saas-sdk-'))
  t.after(()=>{
    const absolute=path.resolve(directory),parent=path.resolve(os.tmpdir())
    assert(absolute.startsWith(parent+path.sep)&&path.basename(absolute).startsWith('saas-sdk-'))
    rmSync(absolute,{recursive:true,force:true})
  })
  mkdirSync(path.join(directory,'examples'),{recursive:true})
  cpSync(path.join(root,sdk),path.join(directory,sdk),{recursive:true})
  return directory
}

test('registered SDK package, classic/ESM/types/schema/Rust and every vendor match the fixed generation entry', async () => {
  const first=await generateArtifacts(),second=await generateArtifacts()
  assert.deepEqual([...first.artifacts],[...second.artifacts])
  const result=await syncSdk({check:true})
  assert.equal(result.sdkVersion,SDK_VERSION)
  assert.equal(result.scope,'all')
  assert.equal(result.artifactCount,44)
  const manifest=JSON.parse(readFileSync(path.join(root,sdk,'releases',SDK_VERSION,'artifact-manifest.json')))
  for(const file of manifest.files){
    const bytes=readFileSync(path.join(root,sdk,'releases',SDK_VERSION,file.path))
    assert.equal(bytes.length,file.bytes,file.path)
    assert.equal(sha(bytes),file.sha256,file.path)
  }
  assert.equal(manifest.contracts.context,'1.0')
  assert.equal(manifest.contracts.microProps,'2.0')
})

test('drift in consumers and generated contracts fails check without silently rewriting any input', async t => {
  const directory=isolated(t)
  await syncSdk({root:directory})
  for(const file of [`${sdk}/generated/types.d.ts`,`${sdk}/generated/schema.json`,`${sdk}/generated/protocol.mjs`,
    `${sdk}/releases/${SDK_VERSION}/index.mjs`,'backend/src/public_contract.rs',
    'business-apps/work-orders/frontend/vendor/iframe-bridge.js','business-apps/work-orders/frontend/vendor/micro-sdk.mjs',
    'mini-services/micro-vue/public/vendor/micro-sdk.js','mini-services/micro-jquery/public/vendor/micro-sdk.js',
    ...[...((await generateArtifacts({root:directory})).artifacts.keys())].filter(file=>file.startsWith('business-apps/stores/frontend/vendor/micro-sdk/'))]){
    const target=path.join(directory,file),original=readFileSync(target)
    const altered=Buffer.concat([original,Buffer.from('\n/* deliberate drift probe */\n')])
    writeFileSync(target,altered)
    await assert.rejects(syncSdk({root:directory,check:true}),/SDK artifact drift/)
    assert.deepEqual(readFileSync(target),altered,'CHECK must not repair drift')
    writeFileSync(target,original)
  }
  assert.equal((await syncSdk({root:directory,check:true})).status,'PASS')
})

test('unregistered release files are rejected and never deleted by generation', async t => {
  const directory=isolated(t)
  await syncSdk({root:directory})
  const target=path.join(directory,sdk,'releases',SDK_VERSION,'unexpected.js')
  writeFileSync(target,'/* unregistered artifact */')
  await assert.rejects(syncSdk({root:directory,check:true}),/Unregistered files/)
  await assert.rejects(syncSdk({root:directory}),/Unregistered files/)
  assert.equal(readFileSync(target,'utf8'),'/* unregistered artifact */')
})

test('unregistered files in the complete stores SDK consumer are rejected without deletion', async t => {
  const directory=isolated(t)
  await syncSdk({root:directory})
  const target=path.join(directory,'business-apps/stores/frontend/vendor/micro-sdk/unregistered.d.mts')
  writeFileSync(target,'/* unregistered consumer declaration */')
  await assert.rejects(syncSdk({root:directory,check:true}),/Unregistered files in SDK consumer/)
  await assert.rejects(syncSdk({root:directory}),/Unregistered files in SDK consumer/)
  assert.equal(readFileSync(target,'utf8'),'/* unregistered consumer declaration */')
  assert.equal((await syncSdk({root:directory,check:true,scope:'sdk'})).status,'PASS')
})

test('only actual registered SDK versions are accepted and unsupported input is not echoed', () => {
  assert.equal(isSupportedSdkVersion(SDK_VERSION),true)
  for(const version of ['0.9.0','1.99.0','2.0.0','sentinel-private-token',null,{}]){
    assert.equal(isSupportedSdkVersion(version),false)
    assert.throws(()=>assertSupportedSdkVersion(version),error=>error.code==='UNSUPPORTED_SDK_VERSION'&&!String(error).includes('sentinel'))
  }
  assert.throws(()=>validateRelease({...release,sdkVersion:'../private'}))
  assert.throws(()=>validateRelease({...release,supportedSdkVersions:[SDK_VERSION,'2.0.0']}))
  assert.throws(()=>validateRelease({...release,compatibility:{...release.compatibility,retirementDate:'invented'}}))
})

test('the captured legacy bridge is the actual unchanged baseline and remains a separate compatibility input', () => {
  const legacy=readFileSync(path.join(root,sdk,release.legacy[0].path))
  assert.equal(sha(legacy),'fb041923bbcff457b4d8b559f42637c5ee74ebc9c34824a901ba87d60e74b42a')
  assert.equal(sha(legacy),release.legacy[0].sha256)
  assert(!legacy.includes(Buffer.from('1.0.0')))
  assert.equal(release.compatibility.breakingChangeCandidateDays,90)
  assert.equal(release.compatibility.retirementDate,null)
})

test('versioned package declarations resolve independently and reject unknown wire/SDK versions', t => {
  const directory=isolated(t)
  const require=createRequire(path.join(root,'frontend/package.json')),ts=require('typescript')
  const module=path.join(directory,sdk,'releases',SDK_VERSION,'index.mjs').replaceAll('\\','/')
  const file=path.join(directory,'consumer.mts')
  writeFileSync(file,`import {SDK_VERSION,createContextClient,MicroSdkError,installIframeBridge,authorizationContextSignature,sameAuthorizationContext} from ${JSON.stringify(module)}\n`
    + `import type {MicroProps,AppAuthorizationContext,NotificationPublishBody,MicroIframeApi} from ${JSON.stringify(module)}\n`
    + `const client=createContextClient({} as MicroProps);const version:typeof SDK_VERSION=client.sdkVersion\n`
    + `const error=new MicroSdkError('FORBIDDEN');const status:number=error.status\n`
    + `const contextVersion:AppAuthorizationContext['contractVersion']='1.0'\n`
    + `const target:NotificationPublishBody['target']={kind:'TENANT'}\n`
    + `const bridge:MicroIframeApi=window.MicroIframe;installIframeBridge(window)\n`
    + `const key:string|null=authorizationContextSignature({});const same:boolean=sameAuthorizationContext({},{})\n`
    + `// @ts-expect-error unknown wire version\nconst unknown:MicroProps['contractVersion']='3.0'\n`
    + `// @ts-expect-error unknown SDK version\nconst unknownSdk:MicroProps['sdkVersion']='99.0.0'\nvoid [version,status,contextVersion,target,bridge,unknown,unknownSdk]\n`)
  const program=ts.createProgram([file],{strict:true,noEmit:true,module:ts.ModuleKind.NodeNext,moduleResolution:ts.ModuleResolutionKind.NodeNext,
    target:ts.ScriptTarget.ES2022,lib:['lib.es2022.d.ts','lib.dom.d.ts','lib.dom.iterable.d.ts'],types:[],skipLibCheck:false})
  const diagnostics=ts.getPreEmitDiagnostics(program)
  assert.deepEqual(diagnostics.map(d=>ts.flattenDiagnosticMessageText(d.messageText,'\n')),[])
})

test('stores receives the complete registered release and its declarations compile without SDK source imports', async t => {
  const directory=isolated(t)
  await syncSdk({root:directory})
  const vendor='business-apps/stores/frontend/vendor/micro-sdk'
  const {artifacts}=await generateArtifacts({root:directory})
  const releasePrefix=`${sdk}/releases/${SDK_VERSION}/`
  const registered=[...artifacts.keys()].filter(file=>file.startsWith(releasePrefix))
  const consumer=[...artifacts.keys()].filter(file=>file.startsWith(`${vendor}/`))
  assert.equal(consumer.length,registered.length)
  for(const file of registered) assert.deepEqual(readFileSync(path.join(directory,vendor,file.slice(releasePrefix.length))),readFileSync(path.join(directory,file)))
  const require=createRequire(path.join(root,'frontend/package.json')),ts=require('typescript')
  const module=path.join(directory,vendor,'index.mjs').replaceAll('\\','/')
  const file=path.join(directory,'stores-consumer.mts')
  writeFileSync(file,`import {SDK_VERSION,requestWithContext,detectBrowserCapabilities,authorizationContextSignature,installIframeBridge} from ${JSON.stringify(module)}\n`
    + `import type {MicroProps,MicroLifecycle,MicroTheme,MicroDataScope,ContextRequestOptions,MicroSdkErrorKind} from ${JSON.stringify(module)}\n`
    + `declare const props:MicroProps;declare const lifecycle:MicroLifecycle;declare const theme:MicroTheme;declare const scope:MicroDataScope\n`
    + `const options:ContextRequestOptions={props,url:'https://platform.example.test/stores-api/stores'};const kind:MicroSdkErrorKind='FORBIDDEN'\n`
    + `requestWithContext({...options,responseType:'json'});detectBrowserCapabilities('iframe');authorizationContextSignature(props);installIframeBridge(window)\n`
    + `void [SDK_VERSION,lifecycle,theme,scope,kind]\n`)
  const program=ts.createProgram([file],{strict:true,noEmit:true,module:ts.ModuleKind.NodeNext,moduleResolution:ts.ModuleResolutionKind.NodeNext,
    target:ts.ScriptTarget.ES2022,lib:['lib.es2022.d.ts','lib.dom.d.ts','lib.dom.iterable.d.ts'],types:[],skipLibCheck:false})
  assert.deepEqual(ts.getPreEmitDiagnostics(program).map(d=>ts.flattenDiagnosticMessageText(d.messageText,'\n')),[])
})
