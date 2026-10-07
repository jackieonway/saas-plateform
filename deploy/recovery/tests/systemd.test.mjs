import test from 'node:test'
import assert from 'node:assert/strict'
import {mkdtemp,chmod,readFile,writeFile,rm}from'node:fs/promises'
import {spawnSync}from'node:child_process'
import path from'node:path'
import os from'node:os'

test('systemd validates service/timer templates with deployment Node path resolved',async t=>{
  assert.equal(process.platform,'linux','native deployment template gate requires Linux')
  const directory=await mkdtemp(path.join(os.tmpdir(),'saas-backup-units-'));await chmod(directory,0o700)
  t.after(()=>rm(directory,{recursive:true,force:true}))
  for(const name of ['saas-backup.service','saas-backup.timer']){
    const source=await readFile(new URL(`../${name}`,import.meta.url),'utf8')
    await writeFile(path.join(directory,name),source.replace('/opt/saas/runtime/node/bin/node',process.execPath),{mode:0o644})
  }
  const result=spawnSync('/usr/bin/systemd-analyze',['verify',path.join(directory,'saas-backup.service'),path.join(directory,'saas-backup.timer')],{encoding:'utf8'})
  assert.equal(result.status,0,result.stderr)
  t.diagnostic('validated private template copies; no system units installed or timer enabled')
})
