import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { render } from '../deploy/native/render.mjs'
import { fileURLToPath } from 'node:url'
import { homedir } from 'node:os'

// Browser and service processes receive only runtime settings. No ambient
// TEST_DATABASE_URL, password, JWT, client secret, proxy credential or HOME
// override is inherited. Service secrets are supplied individually by fixture.
export function runtimeEnvironment(source) {
  return Object.fromEntries(['PATH','LANG','LC_ALL','LD_LIBRARY_PATH','LD_PRELOAD'].filter((key)=>source[key]).map((key)=>[key,source[key]]))
}
export async function startGateway({root,hostRoot,releaseRoot,platformPort,businessPort,gatewayPort,notificationPort,nginx,launch}) {
  mkdirSync(releaseRoot,{recursive:true,mode:0o700})
  const tls=path.join(root,'tls'); mkdirSync(tls,{mode:0o700})
  const certificate=path.join(tls,'certificate.pem'),certificateKey=path.join(tls,'private-key.pem')
  writeFileSync(certificate,readFileSync(process.env.PRODUCTION_GATEWAY_CERTIFICATE),{mode:0o600})
  writeFileSync(certificateKey,readFileSync(process.env.PRODUCTION_GATEWAY_PRIVATE_KEY),{mode:0o600})
  for(const kind of ['client_body','proxy','fastcgi','uwsgi','scgi']) mkdirSync(path.join(root,'nginx-temp',kind),{recursive:true,mode:0o700})
  const configuration=render({publicOrigin:`https://[::1]:${gatewayPort}`,listener:`[::1]:${gatewayPort}`,instanceRoot:root,hostRoot,microRoot:releaseRoot,
    certificate,certificateKey,mimeTypes:process.env.NGINX_MIME_TYPES,backend:`[::1]:${platformPort}`,notification:`[::1]:${notificationPort}`,workOrders:`[::1]:${businessPort}`})
  const configPath=path.join(root,'nginx.conf')
  writeFileSync(configPath,configuration['nginx.conf'],{mode:0o600})
  const valid=spawnSync(nginx,['-t','-p',`${root}/`,'-c',configPath],{cwd:root,encoding:'utf8',timeout:10000,env:runtimeEnvironment(process.env)})
  assert.equal(valid.status,0,'TLS native Nginx configuration must validate before opening a listener')
  return launch('nginx',nginx,['-p',`${root}/`,'-c',configPath],{env:runtimeEnvironment(process.env)})
}
export async function prepareBrowser({root,environment}) {
  assert.ok(!existsSync(path.join(homedir(),'.pki/nssdb')),'Chromium must not select a legacy global NSS trust database; do not alter it')
  const dataRoot=path.join(root,'xdg-data'),nss=path.join(dataRoot,'pki/nssdb')
  mkdirSync(nss,{recursive:true,mode:0o700}); environment.XDG_DATA_HOME=dataRoot
  const certutil=process.env.PRODUCTION_CERTUTIL_BIN
  assert.ok(certutil && existsSync(certutil),'The owned NSS certutil runtime is required')
  for(const args of [['-N','--empty-password','-d',`sql:${nss}`],['-A','-d',`sql:${nss}`,'-n','Dante owned acceptance CA','-t','C,,','-i',process.env.PRODUCTION_CA_FILE]]) {
    const result=spawnSync(certutil,args,{encoding:'utf8',timeout:10000,env:runtimeEnvironment(process.env)})
    assert.equal(result.status,0,'Import only the dedicated acceptance CA into the owned NSS database')
  }
}
export function writeEvidence({root,result,cleanup,qaRoot}) {
  const directory=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../target/phase12-runtime')
  mkdirSync(directory,{recursive:true})
  const output=path.join(directory,`native-evidence-${path.basename(root)}.json`)
  writeFileSync(output,JSON.stringify({schemaVersion:'1.0',...result,cleanup,visualQaDirectory:qaRoot??null},null,2)+'\n',{mode:0o600})
  console.log('[Phase12 native evidence] '+output)
}
export function persistOwnership(manifest) {
  const directory=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../target/phase12-runtime')
  mkdirSync(directory,{recursive:true})
  assert.match(path.basename(manifest.root),/^dante-work-orders-platform-e2e-[A-Za-z0-9]+$/)
  writeFileSync(path.join(directory,`native-ownership-${path.basename(manifest.root)}.json`),JSON.stringify(manifest,null,2)+'\n',{mode:0o600})
}
