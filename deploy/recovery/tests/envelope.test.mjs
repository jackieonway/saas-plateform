import test from 'node:test'
import assert from 'node:assert/strict'
import { randomBytes } from 'node:crypto'
import { mkdtemp, chmod, readFile, writeFile, rm } from 'node:fs/promises'
import { Readable, Writable } from 'node:stream'
import path from 'node:path'
import os from 'node:os'
import { encryptStream, decryptStream } from '../envelope.mjs'
import { parseDatabaseUrl, targetName, sameDatabase } from '../config.mjs'

const sink = () => { const chunks=[];return { chunks,stream:new Writable({ write(chunk,_encoding,cb) { chunks.push(Buffer.from(chunk));cb() } }) } }
test('authenticated envelope detects ciphertext, metadata, truncation and wrong-key damage', async t => {
  const directory=await mkdtemp(path.join(os.tmpdir(),'saas-recovery-unit-'));await chmod(directory,0o700)
  t.after(()=>rm(directory,{ recursive:true,force:true }))
  const key=randomBytes(32), file=path.join(directory,'data.enc'), plaintext=randomBytes(65536)
  const digest=await encryptStream(Readable.from([plaintext]),file,key,{ kind:'database',role:'platform',setId:'a' })
  const output=sink(), restored=await decryptStream(file,output.stream,key,{kind:'database',role:'platform',setId:'a'})
  assert.deepEqual(Buffer.concat(output.chunks),plaintext);assert.equal(restored.sha256,digest.sha256);assert.equal(restored.bytes,plaintext.length)
  await assert.rejects(decryptStream(file,sink().stream,randomBytes(32)),{code:'ARTIFACT_AUTHENTICATION_FAILED'})
  await assert.rejects(decryptStream(file,sink().stream,key,{role:'work-orders'}),{code:'ARTIFACT_METADATA_MISMATCH'})
  const encrypted=await readFile(file)
  for (const [name,mutate] of [['cipher',b=>{b[b.length-30]^=1;return b}],['tag',b=>{b[b.length-1]^=1;return b}],['truncated',b=>b.subarray(0,14)]]) {
    const tampered=path.join(directory,`${name}.enc`);await writeFile(tampered,mutate(Buffer.from(encrypted)),{mode:0o600})
    await assert.rejects(decryptStream(tampered,sink().stream,key))
  }
  assert.equal(encrypted.includes(plaintext.subarray(0,32)),false)
  await assert.rejects(encryptStream(Readable.from([plaintext]),file,key,{}),{code:'EEXIST'})
})
test('configuration rejects target injection, maintenance databases and unsupported URL options',()=>{
  const connection=parseDatabaseUrl('postgresql://operator:p%40ss@127.0.0.1:5432/source?sslmode=verify-full')
  assert.equal(connection.password,'p@ss');assert.equal(connection.database,'source')
  assert.equal(sameDatabase(connection,{...connection,user:'other'}),true)
  assert.equal(targetName('saas_restore_test','postgres'),'saas_restore_test')
  for (const name of ['postgres','template0','template1','source','bad-name','x;DROP DATABASE y','x'.repeat(64)]) assert.throws(()=>targetName(name,'source'))
  assert.throws(()=>parseDatabaseUrl('postgresql://user:secret@localhost/db?options=-cfoo'))
  assert.throws(()=>parseDatabaseUrl('https://user:secret@localhost/db'))
  assert.throws(()=>parseDatabaseUrl('postgresql://user:secret@localhost/db%2Finvalid'))
})
