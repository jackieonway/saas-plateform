import assert from 'node:assert/strict'
import test from 'node:test'
import { spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
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
  assert.match(nginx,/location \/assets\/ \{ try_files \$uri =404; \}/)
  assert.ok(render({...input,trustedHtmlEval:true})['nginx.conf'].includes("script-src 'self' 'unsafe-eval'"))
})
const compose=process.env.COMPOSE_BIN??(process.platform==='linux'?(existsSync('/usr/local/bin/docker-compose')?'/usr/local/bin/docker-compose':'/usr/bin/docker'):path.join(workspace,'target/phase9-gateway-tools/docker-compose.exe'))
const composePrefix=path.basename(compose)==='docker'?['compose']:[]
const caddy=process.env.CADDY_BIN??(process.platform==='linux'?'/usr/bin/caddy':path.join(workspace,'target/phase9-gateway-tools/caddy/caddy.exe'))
test('actual Caddy adapter parses both production certificate strategies',{skip:!existsSync(caddy)},(t)=>{
  const root=mkdtempSync(path.join(os.tmpdir(),'platform-production-caddy-config-')),file=path.join(root,'Caddyfile')
  t.after(()=>{assert.equal(path.dirname(root),os.tmpdir());assert.ok(path.basename(root).startsWith('platform-production-caddy-config-'));rmSync(root,{recursive:true,force:true})})
  writeFileSync(file,render(input).Caddyfile)
  for(const source of [file,path.join(workspace,'deploy/Caddyfile.production')]) {
    const result=spawnSync(caddy,['adapt','--config',source,'--adapter','caddyfile'],{encoding:'utf8',timeout:10000,windowsHide:true,
      env:{...process.env,PUBLIC_APP_URL:'https://saas.example.test',PUBLIC_SOCKET_ORIGIN:'wss://saas.example.test',MICRO_FRAME_ORIGINS:'https://saas.example.test'}})
    assert.equal(result.status,0,'generated and Compose production Caddyfiles must parse with actual Caddy')
  }
})
const composeAvailable=existsSync(compose)&&spawnSync(compose,[...composePrefix,'version'],{encoding:'utf8',timeout:10000}).status===0
test('actual Compose parser refuses missing secrets and accepts both declared production profiles',{skip:!composeAvailable},(t)=>{
  const root=mkdtempSync(path.join(os.tmpdir(),'platform-production-compose-config-')),file=path.join(root,'runtime.env')
  t.after(()=>{assert.equal(path.dirname(root),os.tmpdir());assert.ok(path.basename(root).startsWith('platform-production-compose-config-'));rmSync(root,{recursive:true,force:true})})
  writeFileSync(file,'PUBLIC_APP_URL=https://saas.example.test\n')
  const cleanEnv=Object.fromEntries(['PATH','SystemRoot','TEMP','TMP','COMSPEC','WINDIR','LOCALAPPDATA','APPDATA'].filter((key)=>process.env[key]).map((key)=>[key,process.env[key]]))
  const args=['--project-directory',workspace,'--file',path.join(workspace,'docker-compose.yml'),'--env-file',file]
  const rejected=spawnSync(compose,[...composePrefix,...args,'config','--quiet'],{env:cleanEnv,encoding:'utf8',timeout:15000,windowsHide:true})
  assert.notEqual(rejected.status,0);assert.match(rejected.stderr,/POSTGRES_PASSWORD|PLATFORM_DATABASE_URL|required/i)
  const values={POSTGRES_PASSWORD:'configurationFixtureStrongDatabasePassword',PLATFORM_DATABASE_URL:'postgres://platform:configurationFixture@postgres:5432/saas_platform',
    NOTIFICATION_DATABASE_URL:'postgres://notification:configurationFixture@postgres:5432/saas_platform',WORK_ORDERS_DATABASE_URL:'postgres://work_orders:configurationFixture@work-orders-postgres:5432/work_orders',
    WORK_ORDERS_POSTGRES_PASSWORD:'configurationFixtureStrongBusinessPassword',JWT_SECRET:'ConfigurationFixtureRandomSigningKey123456789',PLATFORM_METRICS_TOKEN:'ConfigurationFixtureMetricsToken1234567890',
    NOTIFICATION_METRICS_TOKEN:'ConfigurationFixtureSocketMetricsToken1234567890',WORK_ORDERS_METRICS_TOKEN:'ConfigurationFixtureBusinessMetricsToken1234567890',
    BOOTSTRAP_ADMIN_USERNAME:'configuration-admin',BOOTSTRAP_ADMIN_PASSWORD:'ConfigurationFixtureAdministratorPassword123',PUBLIC_APP_URL:'https://saas.example.test',PUBLIC_SOCKET_ORIGIN:'wss://saas.example.test',
    WORK_ORDERS_PLATFORM_CLIENT_ID:'configuration-client',WORK_ORDERS_PLATFORM_CLIENT_SECRET:'configurationFixtureClientSecret',MICRO_APPS_DIR:root}
  writeFileSync(file,Object.entries(values).map(([key,value])=>`${key}=${value}`).join('\n')+'\n')
  for(const profiles of [[],['--profile','work-orders','--profile','work-orders-local-db']]) {
    const result=spawnSync(compose,[...composePrefix,...args,...profiles,'config','--quiet'],{env:cleanEnv,encoding:'utf8',timeout:15000,windowsHide:true})
    assert.equal(result.status,0,'declared production Compose configurations must parse')
  }
  const source=readFileSync(path.join(workspace,'docker-compose.yml'),'utf8')
  assert.ok(!source.includes('admin123')&&!source.includes('change-me-in-production')&&!source.includes('saas_platform_2024'))
})
