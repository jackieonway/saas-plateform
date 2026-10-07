// Build from a fresh allowlisted source tree. Dependency caches are allowed;
// existing source node_modules, build output, .env and Git data are not copied.
import { createHash } from 'node:crypto'
import { spawn } from 'node:child_process'
import { cpSync, existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, realpathSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const args = process.argv.slice(2)
const options = {}
for (let i=0; i<args.length; i+=2) { if (!['--out-dir','--cargo','--npm-cli'].includes(args[i]) || !args[i+1]) throw new Error('Usage: node build.mjs --out-dir /new/owned/path [--cargo /path/cargo] [--npm-cli /path/npm-cli.js]'); options[args[i].slice(2)] = args[i+1] }
if (process.platform !== 'linux' || !path.isAbsolute(options['out-dir'] ?? '') || existsSync(options['out-dir'])) throw new Error('Build requires Linux and a new absolute output directory')
const root = options['out-dir'], sourceRoot = path.join(root, 'source'), artifactRoot = path.join(root, 'artifacts')
mkdirSync(sourceRoot, { recursive:true, mode:0o700 }); mkdirSync(artifactRoot, { mode:0o700 })
const allowlist = ['backend/Cargo.toml','backend/Cargo.lock','backend/src','backend/migrations',
  'frontend/package.json','frontend/package-lock.json','frontend/tsconfig.json','frontend/vite.config.ts','frontend/index.html','frontend/src','frontend/public',
  'mini-services/notification-ws/package.json','mini-services/notification-ws/package-lock.json','mini-services/notification-ws/index.ts',
  'mini-services/notification-ws/config.mjs','mini-services/notification-ws/service.mjs','mini-services/notification-ws/platform-session.mjs','mini-services/notification-ws/postgres-store.mjs','mini-services/notification-ws/observability.mjs',
  'business-apps/work-orders/backend/package.json','business-apps/work-orders/backend/package-lock.json','business-apps/work-orders/backend/migrations',
  'business-apps/work-orders/frontend/package.json','business-apps/work-orders/frontend/package-lock.json','business-apps/work-orders/frontend/index.html',
  'business-apps/work-orders/frontend/src','business-apps/work-orders/frontend/vendor','business-apps/work-orders/frontend/scripts', 'scripts/package-microapp.mjs']
for (const name of readdirSync(path.join(workspace,'business-apps/work-orders/backend'))) if (name.endsWith('.mjs')) allowlist.push(`business-apps/work-orders/backend/${name}`)
const sourceHashes = {}
function copy(relative) {
  const source=path.join(workspace,relative), destination=path.join(sourceRoot,relative)
  if (!existsSync(source)) { if (relative==='frontend/public') return; throw new Error(`Missing build source ${relative}`) }
  const stat=lstatSync(source)
  if (stat.isSymbolicLink()) throw new Error(`Source links are not permitted: ${relative}`)
  if (stat.isDirectory()) { mkdirSync(destination,{recursive:true}); for (const name of readdirSync(source).sort()) copy(`${relative}/${name}`); return }
  if (!stat.isFile() || /(?:^|\/)(?:node_modules|\.env[^/]*|target|dist|\.git)(?:\/|$)/.test(relative)) throw new Error('Forbidden build source')
  mkdirSync(path.dirname(destination),{recursive:true}); const bytes=readFileSync(source); writeFileSync(destination,bytes)
  sourceHashes[relative]=createHash('sha256').update(bytes).digest('hex')
}
for (const relative of allowlist) copy(relative)
writeFileSync(path.join(root,'source-manifest.json'),JSON.stringify({schemaVersion:'1.0',files:sourceHashes},null,2)+'\n',{mode:0o600})
const permitted = ['PATH','LANG','LC_ALL','LD_LIBRARY_PATH','LD_PRELOAD','CARGO_HOME','RUSTC','RUSTFLAGS','RUSTUP_HOME','CC','CXX','AR','CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER','CARGO_BUILD_JOBS','ESBUILD_BINARY_PATH']
const env=Object.fromEntries(permitted.filter((key)=>process.env[key]).map((key)=>[key,process.env[key]]))
const npmCache=process.env.DANTE_NPM_CACHE ?? path.join(root,'npm-cache')
if(!path.isAbsolute(npmCache))throw new Error('DANTE_NPM_CACHE must be an absolute path')
const cargoTarget=process.env.DANTE_CARGO_TARGET_DIR ?? path.join(root,'cargo-target')
if(!path.isAbsolute(cargoTarget))throw new Error('DANTE_CARGO_TARGET_DIR must be an absolute path')
Object.assign(env,{npm_config_cache:npmCache,CARGO_TARGET_DIR:cargoTarget,NODE_ENV:'development'})
if(process.env.DANTE_NPM_OFFLINE==='1')env.npm_config_offline='true'
const npm=options['npm-cli'] ?? path.resolve(path.dirname(process.execPath),'../lib/node_modules/npm/bin/npm-cli.js')
async function command(label, executable, commandArgs, cwd, customEnv=env) {
  console.log(JSON.stringify({event:'build-start',step:label}))
  const log=path.join(root,`${label}.log`), chunks=[]
  const child=spawn(executable,commandArgs,{cwd,env:customEnv,stdio:['ignore','pipe','pipe'],windowsHide:true})
  child.stdout.on('data',(bytes)=>chunks.push(bytes)); child.stderr.on('data',(bytes)=>chunks.push(bytes))
  const code=await new Promise((resolve,reject)=>{child.once('error',()=>reject(new Error(`Build tool failed: ${label}`))); child.once('close',resolve)})
  writeFileSync(log,Buffer.concat(chunks),{mode:0o600})
  if(code!==0) throw new Error(`Build stopped at ${label}; inspect the private build log`)
  console.log(JSON.stringify({event:'build-complete',step:label}))
}
for (const [name, directory, omit] of [['frontend','frontend',false],['notification','mini-services/notification-ws',true],['work-orders','business-apps/work-orders/backend',true],['work-orders-frontend','business-apps/work-orders/frontend',false]]) {
  const cwd=path.join(sourceRoot,directory)
  await command(`${name}-npm-ci`,process.execPath,[npm,'ci','--ignore-scripts','--no-audit','--no-fund',...(omit?['--omit=dev']:[])],cwd)
}
const frontend=path.join(sourceRoot,'frontend')
await command('frontend-tsc',process.execPath,[path.join(frontend,'node_modules/typescript/bin/tsc'),'-b'],frontend)
await command('frontend-vite',process.execPath,[path.join(frontend,'node_modules/vite/bin/vite.js'),'build'],frontend)
const cargo=options.cargo ?? 'cargo'
await command('backend-release',cargo,['build','--release','--locked'],path.join(sourceRoot,'backend'))
mkdirSync(path.join(artifactRoot,'backend'))
cpSync(path.join(cargoTarget,'release/dante-cloud-backend'),path.join(artifactRoot,'backend/dante-cloud-backend'))
for(const [source, destination] of [[path.join(frontend,'dist'),'frontend'],[path.join(sourceRoot,'mini-services/notification-ws'),'notification-ws'],[path.join(sourceRoot,'business-apps/work-orders/backend'),'work-orders-api']]) cpSync(source,path.join(artifactRoot,destination),{recursive:true})
const result={schemaVersion:'1.0',builtAt:new Date().toISOString(),nodeVersion:process.version,sourceFiles:Object.keys(sourceHashes).length,sourceManifestSha256:createHash('sha256').update(readFileSync(path.join(root,'source-manifest.json'))).digest('hex'),artifactRoot:realpathSync(artifactRoot)}
writeFileSync(path.join(root,'build-result.json'),JSON.stringify(result,null,2)+'\n',{mode:0o600})
console.log(JSON.stringify({event:'build-ready',...result}))
