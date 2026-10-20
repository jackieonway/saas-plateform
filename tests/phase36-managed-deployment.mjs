/** Actual platform/agent/engine acceptance; all SQL faults target the owned disposable database. */
import assert from 'node:assert/strict'
import {randomBytes,randomUUID,createHash} from 'node:crypto'
import {readFileSync,writeFileSync,existsSync} from 'node:fs'
import path from 'node:path'
import https from 'node:https'
const sha=b=>createHash('sha256').update(b).digest('hex')
const delay=ms=>new Promise(r=>setTimeout(r,ms))
async function wait(fn,message,timeout=60000){const end=Date.now()+timeout;let error;do{try{const value=await fn();if(value)return value}catch(e){error=e}await delay(200)}while(Date.now()<end);throw Error(message+(error?': '+error.message:''))}

export async function rehearse(c){
  const {owner,out,privateRoot,network,tools,pg,gateway,registry,report,secretValues,containers,images,docker,inspect,exec,linux,privateFile,copyToTools,pass,streamToFile,postgresImage,save}=c
  const origin='https://host.phase36.test:8443',agentToken=randomBytes(40).toString('hex'),otherToken=randomBytes(40).toString('hex'),password=randomBytes(32).toString('hex'),jwt=randomBytes(48).toString('hex')
  for(const value of [agentToken,otherToken,password,jwt,'admin123','Zs@12345'])secretValues.add(value)
  await linux(['bash','-c','set -eu; mkdir -p "$1/ca-private" "$1/gateway"; chmod 700 "$1/ca-private"; chmod 755 "$1/gateway"; openssl req -x509 -newkey rsa:2048 -nodes -keyout "$1/ca-private/key.pem" -out "$1/tls/ca.pem" -days 2 -subj /CN=phase36-owned-root -addext basicConstraints=critical,CA:TRUE; openssl req -new -newkey rsa:2048 -nodes -keyout "$1/tls/key.pem" -out "$1/ca-private/server.csr" -subj /CN=host.phase36.test; printf "%s\n" "basicConstraints=critical,CA:FALSE" "keyUsage=critical,digitalSignature,keyEncipherment" "extendedKeyUsage=serverAuth" "subjectAltName=DNS:host.phase36.test,DNS:localhost,IP:127.0.0.1" > "$1/ca-private/extensions"; openssl x509 -req -in "$1/ca-private/server.csr" -CA "$1/tls/ca.pem" -CAkey "$1/ca-private/key.pem" -CAcreateserial -out "$1/tls/cert.pem" -days 2 -extfile "$1/ca-private/extensions"; chmod 644 "$1/tls/ca.pem" "$1/tls/cert.pem" "$1/tls/key.pem"','phase36-ca',privateRoot])
  const caPath=path.join(out,'ca.pem');await streamToFile(['cat',privateRoot+'/tls/ca.pem'],caPath);const ca=readFileSync(caPath)
  const nginx=(await inspect('image','nginxinc/nginx-unprivileged:1.28.0-alpine')).Id
  const registryImage=(await inspect('image','registry:3.0.0')).Id
  report.baseImages={nginx,registry:registryImage,postgres:postgresImage}
  containers.add(registry);await docker(['create','--name',registry,'--label','saas.ci.owner='+owner,'--network',network,'--user','1000:1000','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--tmpfs','/var/lib/registry:rw,nosuid,size=512m,mode=1777','--tmpfs','/tmp:rw,nosuid,size=16m','--publish','127.0.0.1::5000','--mount','type=bind,src='+privateRoot+'/tls,dst=/tls,readonly','--env','REGISTRY_HTTP_TLS_CERTIFICATE=/tls/cert.pem','--env','REGISTRY_HTTP_TLS_KEY=/tls/key.pem',registryImage]);await docker(['start',registry])
  const registryPort=(await inspect('container',registry)).NetworkSettings.Ports['5000/tcp'][0].HostPort
  assert(/^\d+$/.test(registryPort));const registryAuthority='127.0.0.1:'+registryPort,repository=registryAuthority+'/'+owner+'/static'
  const trustDirectory='/etc/docker/certs.d/'+registryAuthority
  await privateFile('trust-path',trustDirectory)
  const trustScript='set -eu; case "$1" in /etc/docker/certs.d/127.0.0.1:*) ;; *) exit 90;; esac; test ! -e "$1"; mkdir -p "$1"; test "$(realpath "$1")" = "$1"; printf "%s" "$2" > "$1/phase36-owner"; cp "$3" "$1/ca.crt"; chmod 644 "$1/ca.crt"'
  if(process.platform==='win32')await c.linux(['true'])
  if(process.platform==='win32'){
    const {spawn}=await import('node:child_process');const args=['-d',process.env.CI_WSL_DISTRO||'Ubuntu','--user','root','--exec','bash','-c',trustScript,'phase36-trust',trustDirectory,owner,privateRoot+'/tls/ca.pem']
    const child=spawn('wsl.exe',args,{windowsHide:true,stdio:['ignore','pipe','pipe']});let output='';child.stdout.on('data',b=>output+=b);child.stderr.on('data',b=>output+=b);assert.equal(await new Promise((resolve,reject)=>{child.on('error',reject);child.on('close',resolve)}),0,output)
  }else await linux(['bash','-c',trustScript,'phase36-trust',trustDirectory,owner,privateRoot+'/tls/ca.pem'])
  report.registry={scope:'owned-loopback-tls',normalTls:true,authority:registryAuthority,caSha256:sha(ca),productionPublication:false};save()
  function httpsRequest(base,endpoint,{method='GET',body,token,trust=ca}={}){
    return new Promise((resolve,reject)=>{const data=body===undefined?undefined:JSON.stringify(body);const req=https.request(new URL(endpoint,base),{method,ca:trust,headers:{...(data?{'content-type':'application/json','content-length':Buffer.byteLength(data)}:{}),...(token?{authorization:'Bearer '+token}:{})}},res=>{const chunks=[];let length=0;res.on('data',b=>{length+=b.length;if(length>4*1024*1024){req.destroy();return}chunks.push(b)});res.on('end',()=>{const text=Buffer.concat(chunks).toString();let value;try{value=JSON.parse(text)}catch{}resolve({status:res.statusCode,text,value})})});req.setTimeout(30000,()=>req.destroy(Error('owned HTTPS deadline')));req.on('error',()=>reject(Error('owned HTTPS certificate/connection failed')));req.end(data)})
  }
  await wait(async()=>{const r=await httpsRequest('https://'+registryAuthority,'/v2/');return r.status===200},'Owned TLS registry readiness')
  const policy={id:'local-static',name:'本机隔离静态目标',publicOrigin:origin,repositories:[repository],templates:[{id:'static-nginx',applicationCode:'stores',healthPath:'/healthz'}]}
  const otherPolicy={...policy,id:'local-other',name:'回执隔离校验目标'}
  await privateFile('server.json',JSON.stringify({targets:[{policy,agentTokenRef:'/runtime/agent.token'},{policy:otherPolicy,agentTokenRef:'/runtime/other.token'}]}))
  await privateFile('agent.token',agentToken);await privateFile('other.token',otherToken)
  await privateFile('agent.json',JSON.stringify({policy,platformOrigin:'',caFile:privateRoot+'/tls/ca.pem',tokenFile:privateRoot+'/agent.token',network,gatewayContainer:gateway,routeDirectory:privateRoot+'/routes',stateDirectory:privateRoot+'/state',owner}))
  await privateFile('pg.env','POSTGRES_USER=ci\nPOSTGRES_PASSWORD='+password+'\nPOSTGRES_DB=platform\n')
  containers.add(pg);await docker(['create','--name',pg,'--label','saas.ci.owner='+owner,'--network',network,'--tmpfs','/var/lib/postgresql/data:rw,nosuid,size=1g','--shm-size','128m','--env-file',privateRoot+'/pg.env',postgresImage,'-c','max_connections=60']);await docker(['start',pg])
  await wait(async()=>{const r=await linux(['docker','--host','unix:///var/run/docker.sock','exec',pg,'pg_isready','-U','ci','-d','platform'],{allowFailure:true});return r.exitCode===0},'PostgreSQL readiness')
  const sql=async query=>{const output=await docker(['exec',pg,'psql','-U','ci','-d','platform','-v','ON_ERROR_STOP=1','-A','-t','-c',query]);return output.trim()}
  await privateFile('platform.env.json',JSON.stringify({DATABASE_URL:`postgresql://ci:${password}@${pg}:5432/platform`,JWT_SECRET:jwt,METRICS_TOKEN:randomBytes(40).toString('hex'),DB_STATEMENT_TIMEOUT_MS:'60000',BIND_ADDR:'0.0.0.0:3001',PUBLIC_APP_URL:origin,MICRO_GATEWAY_ORIGIN:origin,MICRO_ALLOWED_ORIGINS:origin,CORS_ALLOWED_ORIGINS:origin,APPLICATION_ENVIRONMENT:'development',APP_ENV:'test',AUTO_CREATE_DATABASE:'false',STATIC_DIR:'',RUST_LOG:'warn',MICRO_CA_CERT_FILE:'/runtime/ca.pem',DEPLOYMENT_CONFIG_FILE:'/runtime/server.json'}))
  for(const [src,dst]of [['platform.env.json','/runtime/platform.env.json'],['server.json','/runtime/server.json'],['agent.token','/runtime/agent.token'],['other.token','/runtime/other.token'],['tls/ca.pem','/runtime/ca.pem']])await copyToTools(src,dst)
  await exec(['chown','10001:10001','/runtime','/runtime/platform.env.json','/runtime/agent.token','/runtime/other.token']);await exec(['chmod','644','/runtime/server.json','/runtime/ca.pem'])
  await docker(['exec','--detach','--user','10001:10001',tools,'node','/workspace/tests/phase36-platform.mjs'])
  const nginxConfig=`pid /tmp/nginx.pid;\nerror_log /dev/stderr warn;\nevents { worker_connections 256; }\nhttp {\n include /etc/nginx/mime.types;\n access_log off;\n client_body_temp_path /tmp/body; proxy_temp_path /tmp/proxy; fastcgi_temp_path /tmp/fastcgi; uwsgi_temp_path /tmp/uwsgi; scgi_temp_path /tmp/scgi;\n server { listen 8443 ssl; server_name host.phase36.test 127.0.0.1; ssl_certificate /tls/cert.pem; ssl_certificate_key /tls/key.pem; root /site; client_max_body_size 20m;\n location /api/ { proxy_pass http://${tools}:3001; proxy_set_header Host $host; proxy_set_header X-Real-IP $remote_addr; proxy_read_timeout 120s; }\n include /routes/*.conf;\n location ^~ /sub-apps/ { return 404; }\n location / { try_files $uri $uri/ /index.html; }\n }\n}\n`
  await privateFile('gateway/nginx.conf',nginxConfig,'644')
  containers.add(gateway);await docker(['create','--name',gateway,'--label','saas.ci.owner='+owner,'--label','saas.deployment.owner='+owner,'--network',network,'--network-alias','host.phase36.test','--user','101:101','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--tmpfs','/tmp:rw,nosuid,size=32m','--publish','127.0.0.1::8443','--mount','type=bind,src='+privateRoot+'/gateway/nginx.conf,dst=/etc/nginx/nginx.conf,readonly','--mount','type=bind,src='+privateRoot+'/tls,dst=/tls,readonly','--mount','type=bind,src='+privateRoot+'/site,dst=/site,readonly','--mount','type=bind,src='+privateRoot+'/routes,dst=/routes,readonly','--entrypoint','nginx',nginx,'-g','daemon off;']);await docker(['start',gateway])
  const port=(await inspect('container',gateway)).NetworkSettings.Ports['8443/tcp'][0].HostPort,serverOrigin='https://127.0.0.1:'+port
  const request=(endpoint,settings={})=>httpsRequest(serverOrigin,endpoint,settings)
  await wait(async()=>{const r=await request('/api/health');return r.status===200},'Actual platform readiness',120000)
  const agentConfig={policy,platformOrigin:serverOrigin,caFile:privateRoot+'/tls/ca.pem',tokenFile:privateRoot+'/agent.token',network,gatewayContainer:gateway,routeDirectory:privateRoot+'/routes',stateDirectory:privateRoot+'/state',owner}
  await privateFile('agent.json',JSON.stringify(agentConfig))
  async function api(endpoint,body,token,expected=200){const r=await request(endpoint,{method:body===undefined?'GET':'POST',body,token});assert.equal(r.status,expected,`${endpoint}: ${r.value?.code||r.status} ${r.value?.message||''}`);return r.value}
  async function login(username,pwd,organizationId){const ticket=await api('/api/auth/login',{username,password:pwd});secretValues.add(ticket.ticket);const selected=organizationId||ticket.memberships?.[0]?.organizationId;assert(selected,'Actual login organization required');const result=await api('/api/auth/context',{organizationId:selected},ticket.ticket);for(const v of [result.accessToken,result.refreshToken])if(v)secretValues.add(v);return result}
  const rootOrg=await sql("SELECT o.id FROM organizations o JOIN tenants t ON t.id=o.tenant_id WHERE t.is_platform AND o.parent_id IS NULL LIMIT 1")
  const admin=await login('admin','admin123',rootOrg),adminToken=admin.accessToken
  const application=await api('/api/applications',{code:'stores',name:'隔离镜像部署示例',entryType:'HTML',entryUrl:origin+'/sub-apps/stores/versions/1.0.0/',basePath:'/micro/stores',runtimeMode:'IFRAME',status:'ENABLED',autoCreateMenu:false},adminToken)
  const app=application.id;assert(app)
  const jobsPath='/api/applications/'+app+'/managed-deployments',releasesPath='/api/applications/'+app+'/releases'
  const newRelease=async(version,environment='development')=>api(releasesPath,{environment,version,entryType:'HTML',entryUrl:origin+'/sub-apps/stores/versions/'+version+'/',healthUrl:origin+'/sub-apps/stores/versions/'+version+'/healthz',resources:[{kind:'SCRIPT',url:origin+'/sub-apps/stores/versions/'+version+'/app.js'}]},adminToken)
  const intent=(release,imageRef)=>({releaseId:release.id,requestId:randomUUID(),targetId:policy.id,templateId:'static-nginx',imageRef})
  const job=async id=>(await api(jobsPath,undefined,adminToken)).find(j=>j.id===id)
  const current=async()=>api('/api/applications/'+app+'/deployments',undefined,adminToken)
  const agent=async()=>{const r=await linux([privateRoot+'/bin/agent',privateRoot+'/agent.json','--once'],{timeout:300000});assert.equal(r.exitCode,0);return JSON.parse(r.output.trim())}
  const context={...c,origin,policy,otherPolicy,repository,registryAuthority,serverOrigin,ca,agentConfig,agentToken,otherToken,rootOrg,admin,adminToken,app,application,jobsPath,releasesPath,sql,request,api,login,newRelease,intent,job,current,agent,httpsRequest}
  const suite=await import('./phase36-managed-deployment-cases.mjs')
  await suite.acceptance(context)
  await privateFile('regression.json',JSON.stringify({owner,databaseUrl:`postgresql://ci:${password}@${pg}:5432/postgres`}))
  await copyToTools('regression.json','/runtime/regression.json')
  try{await c.step('existing-release-regression',['node','tests/phase36-release-regression.mjs'])}
  finally{await streamToFile(['docker','--host','unix:///var/run/docker.sock','cp',tools+':/workspace/target/phase36-release-regression','-'],path.join(out,'release-regression.tar')).catch(()=>{})}
  // Public and driver artifacts are copied before all private runtime directories are deleted.
  const browser=JSON.parse(await exec(['cat','/workspace/target/phase36-browser-result.json']))
  for(const name of ['phase36-managed-ui.png','phase36-ready.png']){
    const dest=path.join(out,'screenshots',name)
    await streamToFile(['docker','--host','unix:///var/run/docker.sock','cp',tools+':/workspace/target/'+name,'-'],dest+'.tar')
    const {spawnSync}=await import('node:child_process');const r=spawnSync('tar',['-xf',dest+'.tar','-C',path.join(out,'screenshots')],{windowsHide:true,encoding:'utf8'});assert.equal(r.status,0,r.stderr)
  }
  return {trustDirectory,serverOrigin,ca,request,policy,registry:report.registry,browser}
}
