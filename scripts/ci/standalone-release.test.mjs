import {test} from 'node:test'
import assert from 'node:assert/strict'
import {mkdtempSync,readFileSync,writeFileSync,rmSync,existsSync} from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import {exportStandaloneRelease,verifyStandaloneRelease} from './standalone-release.mjs'

for(const application of ['work-orders','stores'])test(`${application}: closed independent distribution and tamper refusal`,()=>{
  const temporary=mkdtempSync(path.join(os.tmpdir(),'standalone-release-'))
  try{
    const outDir=path.join(temporary,application),sourceRoot=process.cwd()
    const manifest=exportStandaloneRelease({application,outDir,sourceRoot})
    assert(manifest.files.some(f=>f.path==='crates/operational-telemetry/src/lib.rs'))
    assert(!existsSync(path.join(outDir,'backend')))
    assert(!manifest.files.some(f=>/(node_modules|\/target\/|\/dist\/|server\.mjs|\.pem$|\.key$)/.test(f.path)))
    assert.equal(readFileSync(path.join(outDir,`business-apps/${application}/backend/Cargo.lock`),'utf8'),readFileSync(`business-apps/${application}/backend/Cargo.lock`,'utf8'))
    assert.throws(()=>exportStandaloneRelease({application,outDir,sourceRoot}),/overwrite/)
    writeFileSync(path.join(outDir,'README.md'),'changed')
    assert.throws(()=>verifyStandaloneRelease(outDir),/differs/)
  }finally{rmSync(temporary,{recursive:true,force:true})}
})
