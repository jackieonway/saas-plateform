import assert from 'node:assert/strict'
import test from 'node:test'
import { spawnSync } from 'node:child_process'
import { lstatSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { render } from '../deploy/native/render.mjs'

const workspace=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..')
const input={publicOrigin:'https://localhost:8443',listener:'127.0.0.1:8443',instanceRoot:'/srv/platform',hostRoot:'/srv/platform/current/frontend',microRoot:'/srv/platform/micro-apps',
  certificate:'/srv/platform/secrets/cert.pem',certificateKey:'/srv/platform/secrets/key.pem',mimeTypes:'/etc/nginx/mime.types',backend:'127.0.0.1:3001',notification:'127.0.0.1:3003',workOrders:'127.0.0.1:3004'}
test('native production policy rejects insecure origins, injected paths and non-private upstreams',()=>{
  for(const replacement of [{publicOrigin:'http://localhost:8443'},{publicOrigin:'https://localhost:8443/path'},{applicationOrigins:['https://evil.test/']},
    {instanceRoot:'/srv/platform\nhttp { }'},{hostRoot:'/srv/../etc'},{certificateKey:'/srv/platform/$PRIVATE_KEY'},{backend:'10.0.0.10:3001'},{listener:'127.0.0.1:70000'}]) assert.throws(()=>render({...input,...replacement}))
})
test('native default CSP is iframe policy; trusted eval requires explicit deployment selection',()=>{
  const nginx=render(input)['nginx.conf']
  assert.match(nginx,/ssl_protocols TLSv1\.2 TLSv1\.3/);assert.ok(!nginx.includes('unsafe-eval'))
  assert.ok(!nginx.includes('$proxy_add_x_forwarded_for'))
  assert.match(nginx,/location \^~ \/api\/internal\/ \{ return 404; \}/)
  assert.match(nginx,/location \/assets\/ \{\s+try_files \$uri =404;/)
  const metadataGuard='location ~* (?:^|/)release\\.json(?:/|$) { return 404; }'
  assert.ok(nginx.includes(metadataGuard),'native ZIP storage also keeps registration metadata private')
  assert.ok(nginx.indexOf(metadataGuard)<nginx.indexOf('location ~ ^/sub-apps/'))
  assert.ok(render({...input,trustedHtmlEval:true})['nginx.conf'].includes("script-src 'self' 'unsafe-eval'"))
  const trusted=render({...input,trustedHtmlEval:true})['nginx.conf']
  const versionLocation=trusted.slice(trusted.indexOf('location ~ ^/sub-apps/'),trusted.indexOf('location /sub-apps/'))
  assert.ok(!versionLocation.includes('unsafe-eval'),'the trusted host loader setting must not weaken opaque version HTML')
})
test('native renderer exports only Nginx and explicitly allowlists private upstreams',()=>{
  assert.deepEqual(Object.keys(render(input)),['nginx.conf'])
  assert.doesNotThrow(()=>render({...input,backend:'10.20.0.9:3001',allowedUpstreamIps:['10.20.0.9']}))
  for(const replacement of [{backend:'https://127.0.0.1:3001'},{backend:'8.8.8.8:443',allowedUpstreamIps:['8.8.8.8']},{listener:'0.0.0.0:8443'},{trustedHtmlEval:'true'}]) assert.throws(()=>render({...input,...replacement}))
  assert.match(render({...input,imageOrigins:['https://images.example.test']})['nginx.conf'],/img-src 'self' data: blob: https:\/\/images\.example\.test;/)
  assert.throws(()=>render({...input,imageOrigins:['http://images.example.test']}))
})
function composeRuntime() {
  if (process.env.COMPOSE_BIN) {
    const binary=process.env.COMPOSE_BIN
    return {binary,prefix:/^docker(?:\.exe)?$/i.test(path.basename(binary))?['compose']:[],translate:(value)=>value}
  }
  if (process.platform==='win32') {
    return {binary:'wsl.exe',prefix:['--distribution',process.env.COMPOSE_WSL_DISTRO??'Ubuntu','--','docker','compose'],translate:(value)=>{
      const resolved=path.resolve(value).replaceAll('\\','/'),match=/^([A-Za-z]):\/(.*)$/.exec(resolved)
      assert.ok(match,'Windows Compose fixture must be on a drive mapped into WSL')
      return `/mnt/${match[1].toLowerCase()}/${match[2]}`
    }}
  }
  return {binary:'docker',prefix:['compose'],translate:(value)=>value}
}

test('actual Compose parser enforces the image-only edge and explicit persistent ZIP boundary',(t)=>{
  const root=mkdtempSync(path.join(os.tmpdir(),'platform-production-compose-config-')),file=path.join(root,'runtime.env')
  t.after(()=>{assert.equal(path.dirname(root),os.tmpdir());assert.ok(path.basename(root).startsWith('platform-production-compose-config-'));assert.ok(!lstatSync(root).isSymbolicLink());rmSync(root,{recursive:true,force:true})})
  const runtime=composeRuntime()
  const cleanEnv=Object.fromEntries(['PATH','SystemRoot','TEMP','TMP','COMSPEC','WINDIR','LOCALAPPDATA','APPDATA','LANG','LC_ALL'].filter((key)=>process.env[key]).map((key)=>[key,process.env[key]]))
  const run=(args)=>spawnSync(runtime.binary,[...runtime.prefix,...args],{env:cleanEnv,encoding:'utf8',timeout:30000,windowsHide:true})
  const version=run(['version','--short'])
  assert.equal(version.status,0,`The actual Compose parser is required (install Docker Compose or provide COMPOSE_BIN): ${version.error?.message??version.stderr}`)
  assert.match(version.stdout,/\d+\.\d+/)
  writeFileSync(file,'PUBLIC_APP_URL=https://saas.example.test\n')
  const args=['--project-directory',runtime.translate(workspace),'--file',runtime.translate(path.join(workspace,'docker-compose.yml')),'--env-file',runtime.translate(file)]
  const rejected=run([...args,'config','--quiet'])
  assert.notEqual(rejected.status,0);assert.match(rejected.stderr,/POSTGRES_IMAGE|POSTGRES_PASSWORD|PLATFORM_DATABASE_URL|required/i)
  const values={POSTGRES_PASSWORD:'configurationFixtureStrongDatabasePassword',PLATFORM_DATABASE_URL:'postgres://platform:configurationFixture@postgres:5432/saas_platform',
    JWT_SECRET:'ConfigurationFixtureRandomSigningKey123456789',PLATFORM_METRICS_TOKEN:'ConfigurationFixtureMetricsToken1234567890',
    BOOTSTRAP_ADMIN_USERNAME:'configuration-admin',BOOTSTRAP_ADMIN_PASSWORD:'ConfigurationFixtureAdministratorPassword123',PUBLIC_APP_URL:'https://saas.example.test',
    GATEWAY_PRIVATE_IP:'172.30.0.10',EDGE_NETWORK_NAME:'configuration-fixture-edge',
    GATEWAY_CONFIG_FILE:runtime.translate(path.join(root,'nginx.conf')),TLS_CERTIFICATE_FILE:runtime.translate(path.join(root,'certificate.pem')),TLS_PRIVATE_KEY_FILE:runtime.translate(path.join(root,'private-key.pem')),
    POSTGRES_IMAGE:`postgres@sha256:${'a'.repeat(64)}`,PLATFORM_IMAGE:`platform@sha256:${'b'.repeat(64)}`,FRONTEND_IMAGE:`frontend@sha256:${'c'.repeat(64)}`,GATEWAY_IMAGE:`gateway@sha256:${'d'.repeat(64)}`}
  for(const name of ['nginx.conf','certificate.pem','private-key.pem','micro-static.conf'])writeFileSync(path.join(root,name),'configuration fixture only\n')
  mkdirSync(path.join(root,'micro-apps'))
  writeFileSync(file,Object.entries(values).map(([key,value])=>`${key}=${value}`).join('\n')+'\n')
  function parse(extra=[]) {
    const result=run([...args,...extra,'config','--format','json'])
    assert.equal(result.status,0,result.stderr)
    return JSON.parse(result.stdout)
  }
  const base=parse(),baseServices=base.services
  assert.deepEqual(Object.keys(baseServices).sort(),['backend','frontend-web','gateway','postgres'])
  assert.equal(baseServices.backend.environment.MICRO_APPS_DIR,'')
  assert.ok(!baseServices.backend.volumes?.length&&!baseServices['frontend-web'].volumes?.length,'recommended image services do not share release files')
  assert.equal(baseServices.backend.environment.TRUSTED_PROXY_IPS,baseServices.gateway.networks.edge.ipv4_address)
  assert.equal(base.networks.edge.external,true)
  assert.equal(base.networks.edge.name,values.EDGE_NETWORK_NAME)
  assert.equal(base.networks.database.internal,true)
  for(const [name,service] of Object.entries(baseServices)) {
    assert.match(service.image,/@sha256:[a-f0-9]{64}$/)
    assert.equal(service.build,undefined)
    if(name!=='gateway')assert.ok(!service.ports?.length,`${name} must remain private`)
  }
  assert.deepEqual(baseServices.gateway.ports.map((entry)=>Number(entry.published)).sort((a,b)=>a-b),[80,443])
  for(const mount of baseServices.gateway.volumes) {
    assert.equal(mount.read_only,true)
    // Compose omits the false zero value from its normalized JSON output.
    assert.equal(mount.bind.create_host_path??false,false)
    assert.ok(mount.target.startsWith('/etc/nginx/'))
  }
  assert.equal(baseServices.gateway.user,'101:101')
  assert.equal(baseServices.gateway.read_only,true)
  writeFileSync(path.join(root,'monitor.json'),'{}\n')
  writeFileSync(file,Object.entries({...values,MONITOR_IMAGE:`monitor@sha256:${'f'.repeat(64)}`,MONITOR_CONFIG_FILE:runtime.translate(path.join(root,'monitor.json'))}).map(([key,value])=>`${key}=${value}`).join('\n')+'\n')
  const monitored=parse(['--file',runtime.translate(path.join(workspace,'deploy/compose.monitoring.yml')),'--profile','monitoring'])
  const monitor=monitored.services.monitor
  assert.equal(monitor.user,'10004:10004');assert.equal(monitor.read_only,true)
  assert.ok(!monitor.ports?.length);assert.deepEqual(monitor.cap_drop,['ALL'])
  assert.equal(monitor.volumes.length,1);assert.equal(monitor.volumes[0].read_only,true)
  assert.equal(monitor.volumes[0].target,'/run/secrets/monitor-config.json')
  assert(!JSON.stringify(monitor).includes('docker.sock'))
  const zipValues={...values,MICRO_STATIC_IMAGE:`micro-static@sha256:${'e'.repeat(64)}`,MICRO_STATIC_CONFIG_FILE:runtime.translate(path.join(root,'micro-static.conf')),MICRO_APPS_DIR:runtime.translate(path.join(root,'micro-apps'))}
  writeFileSync(file,Object.entries(zipValues).map(([key,value])=>`${key}=${value}`).join('\n')+'\n')
  const zip=parse(['--file',runtime.translate(path.join(workspace,'deploy/compose.zip.yml')),'--profile','zip'])
  assert.equal(zip.services.backend.environment.MICRO_APPS_DIR,'/micro-apps')
  const writable=zip.services.backend.volumes.find((mount)=>mount.target==='/micro-apps')
  const readable=zip.services['micro-static'].volumes.find((mount)=>mount.target==='/micro-apps')
  assert.equal(writable.source,readable.source)
  assert.notEqual(writable.read_only,true)
  assert.equal(readable.read_only,true)
  assert.equal(writable.bind.create_host_path??false,false)
  assert.equal(readable.bind.create_host_path??false,false)
  assert.ok(!zip.services['micro-static'].ports?.length)
  assert.equal(zip.services['micro-static'].read_only,true)
  assert.equal(zip.services['micro-static'].user,'101:101')
  // The historical override is retained as an empty, retired filename.
  writeFileSync(file,Object.entries(values).map(([key,value])=>`${key}=${value}`).join('\n')+'\n')
  const legacyArgs=['--file',runtime.translate(path.join(workspace,'deploy/compose.legacy.yml')),'--profile','legacy']
  const legacy=parse(legacyArgs)
  assert.equal(legacy.services['work-orders-api'],undefined,'retired Node profile must not declare a service')
  assert.equal(legacy.services['notification-ws'],undefined)
  const notificationArgs=['--file',runtime.translate(path.join(workspace,'deploy/compose.notification.yml')),'--profile','notification']
  const rejectedNotification=run([...args,...notificationArgs,'config','--quiet'])
  assert.notEqual(rejectedNotification.status,0)
  assert.match(rejectedNotification.stderr,/NOTIFICATION_IMAGE|NOTIFICATION_DATABASE_URL|NOTIFICATION_METRICS_TOKEN|required/i)
  const notificationValues={...values,NOTIFICATION_IMAGE:`rust-notification@sha256:${'f'.repeat(64)}`,
    NOTIFICATION_DATABASE_URL:'postgres://notification_ws:configurationFixture@postgres:5432/saas_platform',NOTIFICATION_METRICS_TOKEN:'ConfigurationFixtureNotificationMetrics1234567890'}
  writeFileSync(file,Object.entries(notificationValues).map(([key,value])=>`${key}=${value}`).join('\n')+'\n')
  const notification=parse(notificationArgs),service=notification.services['notification-ws']
  assert.ok(service,'the independently enabled notification profile must declare the Rust service')
  assert.deepEqual(service.profiles,['notification'])
  assert.match(service.image,/@sha256:[a-f0-9]{64}$/)
  assert.equal(service.build,undefined)
  assert.ok(!service.ports?.length&&!service.volumes?.length,'the service has no public port, source bind mount or socket mount')
  assert.equal(service.user,'10003:10003')
  assert.equal(service.read_only,true)
  assert.deepEqual(service.cap_drop,['ALL'])
  assert.deepEqual(service.security_opt,['no-new-privileges:true'])
  assert.equal(service.environment.NOTIFICATION_ENV,'production')
  assert.equal(service.environment.JWT_SECRET,undefined)
  assert.equal(service.environment.DATABASE_URL,notificationValues.NOTIFICATION_DATABASE_URL)
  assert.equal(service.environment.PLATFORM_API_BASE_URL,values.PUBLIC_APP_URL)
  assert.equal(service.environment.METRICS_TOKEN,notificationValues.NOTIFICATION_METRICS_TOKEN)
  assert.deepEqual(Object.keys(service.networks).sort(),['database','edge'])
  assert.ok(service.healthcheck.test.includes('http://127.0.0.1:3003/health/ready'))
  assert.ok(!service.healthcheck.test.includes('node'))
  assert.equal(notification.services.gateway.depends_on['notification-ws'],undefined,'public TLS must start before notification readiness')
  const caFile=runtime.translate(path.join(root,'certificate.pem'))
  writeFileSync(file,Object.entries({...notificationValues,NOTIFICATION_PLATFORM_CA_FILE:caFile}).map(([key,value])=>`${key}=${value}`).join('\n')+'\n')
  const notificationCa=parse([...notificationArgs,'--file',runtime.translate(path.join(workspace,'deploy/compose.notification-ca.yml'))])
  const caService=notificationCa.services['notification-ws']
  assert.equal(caService.environment.NOTIFICATION_PLATFORM_CA_FILE,'/etc/rust-saas/platform-ca.pem')
  assert.equal(caService.volumes.length,1)
  assert.equal(caService.volumes[0].source,caFile)
  assert.equal(caService.volumes[0].target,'/etc/rust-saas/platform-ca.pem')
  assert.equal(caService.volumes[0].read_only,true)
  assert.equal(caService.volumes[0].bind.create_host_path??false,false)
  const source=readFileSync(path.join(workspace,'docker-compose.yml'),'utf8')
  assert.ok(!source.includes('admin123')&&!source.includes('change-me-in-production')&&!source.includes('saas_platform_2024'))
})
