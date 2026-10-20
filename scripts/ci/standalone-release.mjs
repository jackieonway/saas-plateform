/** Build-tool only: export one self-contained business source distribution. */
import assert from 'node:assert/strict'
import {createHash} from 'node:crypto'
import {existsSync,lstatSync,mkdirSync,readFileSync,readdirSync,realpathSync,writeFileSync} from 'node:fs'
import path from 'node:path'
import {pathToFileURL} from 'node:url'

const applications = ['work-orders','stores']
const sha = bytes => createHash('sha256').update(bytes).digest('hex')
const excluded = name => ['node_modules','target','dist','.git'].includes(name) || name.startsWith('.env') || /\.(?:pem|key|log|tsbuildinfo)$/.test(name)

function tree(root, relative = '') {
  return readdirSync(path.join(root,relative)).sort().flatMap(name => {
    const next = path.posix.join(relative,name), file=path.join(root,next), stat=lstatSync(file)
    assert(!stat.isSymbolicLink(),'Distribution must not contain symbolic links')
    if (stat.isDirectory()) return tree(root,next)
    assert(stat.isFile(),'Distribution accepts regular files only')
    const bytes=readFileSync(file)
    return [{path:next,bytes:bytes.length,sha256:sha(bytes)}]
  })
}

export function verifyStandaloneRelease(directory) {
  const manifest=JSON.parse(readFileSync(path.join(directory,'release-manifest.json')))
  assert.equal(manifest.schemaVersion,1);assert(applications.includes(manifest.application))
  const actual=tree(directory).filter(f=>f.path!=='release-manifest.json')
  assert.deepEqual(actual,manifest.files,'Distribution content differs from its manifest')
  return manifest
}

export function exportStandaloneRelease({sourceRoot,application,outDir}) {
  assert(applications.includes(application),'Select work-orders or stores')
  assert(path.isAbsolute(sourceRoot)&&path.isAbsolute(outDir),'Absolute paths required')
  const root=realpathSync(sourceRoot)
  assert(!existsSync(outDir),'Refusing to overwrite a distribution')
  // Prevent an output beneath an included tree, including junction ancestors.
  let ancestor=path.resolve(outDir),tail=[]
  while(!existsSync(ancestor)){tail.unshift(path.basename(ancestor));ancestor=path.dirname(ancestor)}
  const physical=path.join(realpathSync(ancestor),...tail)
  const inputs=[`business-apps/${application}/frontend`,`business-apps/${application}/backend/src`,`business-apps/${application}/backend/migrations`,
    `business-apps/${application}/deploy`,'crates/operational-telemetry']
  for(const input of inputs){const relative=path.relative(path.join(root,input),physical);assert(relative!==''&&(relative==='..'||relative.startsWith('..'+path.sep)||path.isAbsolute(relative)),'Output must be outside included source trees')}
  const files=new Map()
  function collect(relative){
    if(excluded(path.basename(relative)))return
    const source=path.join(root,relative),stat=lstatSync(source)
    assert(!stat.isSymbolicLink(),'Source links are refused')
    if(stat.isDirectory()){for(const name of readdirSync(source).sort())collect(path.posix.join(relative,name));return}
    assert(stat.isFile());files.set(relative,readFileSync(source))
  }
  for(const input of inputs)collect(input)
  for(const name of ['Cargo.toml','Cargo.lock','README.md','API.md','config.rust.env.example','config.env.example','policy.example.json']){
    const relative=`business-apps/${application}/backend/${name}`;if(existsSync(path.join(root,relative)))collect(relative)
  }
  for(const name of ['.npmrc','.node-version','.nvmrc','rust-toolchain.toml'])if(existsSync(path.join(root,name)))collect(name)
  assert(files.has(`business-apps/${application}/backend/Cargo.lock`));assert(files.has(`business-apps/${application}/frontend/package-lock.json`))
  const cargo=files.get(`business-apps/${application}/backend/Cargo.toml`).toString()
  assert(!/path\s*=\s*"[^"]*(?:backend\/src|platform)/.test(cargo),'Platform implementation dependencies are forbidden')
  files.set('README.md',Buffer.from(`# ${application} standalone source distribution\n\nRun from this directory using Node 24.18.0/npm 11.16.0 and Rust 1.97.0.\n\n\
\`\`\`sh\nnpm --prefix business-apps/${application}/frontend ci --ignore-scripts --no-audit --no-fund\nnpm --prefix business-apps/${application}/frontend run build\ncargo build --locked --manifest-path business-apps/${application}/backend/Cargo.toml\n\`\`\`\n\nThe dependency-free operational-telemetry crate is included with its original relative path. No platform implementation, root npm installation, cache or credentials are included. The frontend carries its pinned vendored SDK.\n\nRead backend/README.md and deploy manifests beneath business-apps/${application}. Provision a dedicated PostgreSQL database/owner and configure the external HTTPS platform API, application client and allowed origins. Register menus and ordinary-user permissions/data grants using the platform administrator API/UI or separately delivered Rust CLI. Publishing does not grant access. Replace manifest identity placeholders with actual IDs; never ship secrets in frontend builds.\n\nBackend, database and frontend versions have separate upgrade/rollback lifecycles. The full repository deployment and compatibility runbooks are separate operator deliverables. Source archive hashes identify these bytes; they do not claim an image was built or externally published.\n`))
  mkdirSync(outDir,{recursive:true})
  for(const [relative,bytes] of files){const target=path.join(outDir,relative);mkdirSync(path.dirname(target),{recursive:true});writeFileSync(target,bytes)}
  const manifest={schemaVersion:1,application,format:'standalone-business-source',platformImplementationIncluded:false,sharedCrates:['operational-telemetry'],files:tree(outDir)}
  writeFileSync(path.join(outDir,'release-manifest.json'),JSON.stringify(manifest,null,2)+'\n')
  return verifyStandaloneRelease(outDir)
}

if(process.argv[1]&&import.meta.url===pathToFileURL(path.resolve(process.argv[1])).href){
  const [application,outDir,...extra]=process.argv.slice(2)
  assert.equal(extra.length,0,'Usage: node scripts/ci/standalone-release.mjs work-orders|stores <fresh absolute output>')
  console.log(JSON.stringify(exportStandaloneRelease({sourceRoot:process.cwd(),application,outDir})))
}
