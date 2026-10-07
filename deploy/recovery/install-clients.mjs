#!/usr/bin/env node
import { readFile, mkdir, writeFile, rename, lstat } from 'node:fs/promises'
import { createHash, randomUUID } from 'node:crypto'
import { spawn } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

// Reproducible local extraction; no apt install, system package changes, or sudo.
const destination = path.resolve(process.argv[2] || 'target/phase12-runtime/postgres-client')
try {
  if (process.platform !== 'linux' || Number(process.versions.node.split('.')[0]) !== 24) throw new Error('LINUX_NODE24_REQUIRED')
  const pin = JSON.parse(await readFile(new URL('./postgres-client-pin.json',import.meta.url),'utf8'))
  await mkdir(destination,{ recursive:true,mode:0o700 })
  if (!(await lstat(destination)).isDirectory() || (await lstat(destination)).isSymbolicLink()) throw new Error('UNSAFE_DESTINATION')
  for (const pkg of pin.packages) {
    const archive = path.join(destination,`${pkg.name}.deb`)
    let contents = await readFile(archive).catch(()=>null)
    if (!contents) {
      const response = await fetch(new URL(pkg.filename,pin.source),{ signal:AbortSignal.timeout(120000) })
      if (!response.ok) throw new Error('CLIENT_DOWNLOAD_FAILED')
      contents = Buffer.from(await response.arrayBuffer())
      if (createHash('sha256').update(contents).digest('hex') !== pkg.sha256) throw new Error('CLIENT_CHECKSUM_MISMATCH')
      const temporary = `${archive}.${randomUUID()}.download`
      await writeFile(temporary,contents,{ flag:'wx',mode:0o600 }); await rename(temporary,archive)
    }
    if (createHash('sha256').update(contents).digest('hex') !== pkg.sha256) throw new Error('CLIENT_CHECKSUM_MISMATCH')
    await new Promise((resolve,reject) => {
      const child = spawn('/usr/bin/dpkg-deb',['--extract',archive,path.join(destination,'root')],{ shell:false,stdio:'ignore' })
      child.once('error',()=>reject(new Error('CLIENT_EXTRACTION_FAILED')))
      child.once('close',code=>code === 0 ? resolve() : reject(new Error('CLIENT_EXTRACTION_FAILED')))
    })
  }
  process.stdout.write(`${JSON.stringify({ event:'postgres-clients-ready',binDirectory:path.join(destination,'root/usr/lib/postgresql/17/bin'),libraryDirectory:path.join(destination,'root/usr/lib/x86_64-linux-gnu'),packages:pin.packages.map(({name,version,sha256})=>({name,version,sha256})) })}\n`)
} catch (error) {
  process.stderr.write(`${JSON.stringify({ event:'client-install-error',code:/^[A-Z_]+$/.test(error.message) ? error.message : 'CLIENT_INSTALL_FAILED' })}\n`)
  process.exitCode=1
}
