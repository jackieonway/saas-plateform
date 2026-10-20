/** The controller alone owns Docker. Platform and node agents have no engine socket. */
import assert from 'node:assert/strict'
import {randomBytes,randomUUID,createHash} from 'node:crypto'
import {readFileSync,writeFileSync} from 'node:fs'
import path from 'node:path'
import https from 'node:https'
import {crc32} from 'node:zlib'
const sha=b=>createHash('sha256').update(b).digest('hex')
async function wait(fn,label,timeout=120000){const end=Date.now()+timeout;let error;do{try{const v=await fn();if(v)return v}catch(e){error=e}await new Promise(r=>setTimeout(r,200))}while(Date.now()<end);throw Error(label+': '+(error?.message||'timeout'))}
function archive(version){
 const files={'index.html':`<!doctype html><html><head><meta charset="utf-8"><title>ZIP ${version}</title></head><body><h1>${version}</h1><script src="./app.js"></script></body></html>`,'app.js':`document.documentElement.dataset.version=${JSON.stringify(version)};`}
 const local=[],central=[];let offset=0
 for(const [name,value]of Object.entries(files)){const n=Buffer.from(name),b=Buffer.from(value),h=Buffer.alloc(30),c=Buffer.alloc(46);h.writeUInt32LE(0x04034b50);h.writeUInt16LE(20,4);h.writeUInt32LE(crc32(b),14);h.writeUInt32LE(b.length,18);h.writeUInt32LE(b.length,22);h.writeUInt16LE(n.length,26);local.push(h,n,b);c.writeUInt32LE(0x02014b50);c.writeUInt16LE(20,4);c.writeUInt16LE(20,6);c.writeUInt32LE(crc32(b),16);c.writeUInt32LE(b.length,20);c.writeUInt32LE(b.length,24);c.writeUInt16LE(n.length,28);c.writeUInt32LE(offset,42);central.push(c,n);offset+=h.length+n.length+b.length}
 const directory=Buffer.concat(central),end=Buffer.alloc(22);end.writeUInt32LE(0x06054b50);end.writeUInt16LE(2,8);end.writeUInt16LE(2,10);end.writeUInt32LE(directory.length,12);end.writeUInt32LE(offset,16);return Buffer.concat([...local,directory,end])
}
export async function rehearse(c){
 const {owner,out,privateRoot,network,tools,pg,gateway,report,secretValues,containers,docker,inspect,exec,linux,privateFile,copyToTools,pass,streamToFile,toolsImage,postgresImage}=c
 const origin='https://host.phase35.test:8443',password=randomBytes(32).toString('hex'),jwt=randomBytes(48).toString('hex'),tokens=[randomBytes(40).toString('hex'),randomBytes(40).toString('hex')]
 for(const v of [password,jwt,...tokens,'admin123','Zs@12345'])secretValues.add(v)
 await linux(['bash','-c','set -eu; mkdir -p "$1/ca-private" "$1/gateway" "$1/node1" "$1/node2"; chmod 755 "$1/gateway"; openssl req -x509 -newkey rsa:2048 -nodes -keyout "$1/ca-private/key.pem" -out "$1/tls/ca.pem" -days 2 -subj /CN=phase35-owned-root -addext basicConstraints=critical,CA:TRUE; openssl req -new -newkey rsa:2048 -nodes -keyout "$1/tls/key.pem" -out "$1/ca-private/server.csr" -subj /CN=host.phase35.test; printf "%s\n" "basicConstraints=critical,CA:FALSE" "keyUsage=critical,digitalSignature,keyEncipherment" "extendedKeyUsage=serverAuth" "subjectAltName=DNS:host.phase35.test,DNS:node1.phase35.test,DNS:node2.phase35.test,IP:127.0.0.1" > "$1/ca-private/extensions"; openssl x509 -req -in "$1/ca-private/server.csr" -CA "$1/tls/ca.pem" -CAkey "$1/ca-private/key.pem" -CAcreateserial -out "$1/tls/cert.pem" -days 2 -extfile "$1/ca-private/extensions"; chmod 644 "$1/tls/ca.pem" "$1/tls/cert.pem" "$1/tls/key.pem"','phase35-ca',privateRoot])
 await streamToFile(['cat',privateRoot+'/tls/ca.pem'],path.join(out,'ca.pem'));const ca=readFileSync(path.join(out,'ca.pem'))
 const nginx=(await inspect('image','nginxinc/nginx-unprivileged:1.28.0-alpine')).Id
 const policy={schemaVersion:1,id:'zip-local',name:'本机隔离双节点',publicOrigin:origin,applications:['stores'],nodes:[{id:'node1',origin:'https://node1.phase35.test:8443'},{id:'node2',origin:'https://node2.phase35.test:8443'}]}
 const config={groups:[{policy,nodes:tokens.map((_,i)=>({nodeId:'node'+(i+1),caRef:'/runtime/ca.pem',tokenRef:'/runtime/node'+(i+1)+'.token'}))}]}
 await privateFile('server.json',JSON.stringify(config));await copyToTools('server.json','/runtime/zip.json')
 const baseNginx=`pid /tmp/nginx.pid; error_log /dev/stderr warn; events {worker_connections 256;} http {include /etc/nginx/mime.types; access_log off; client_body_temp_path /tmp/body; proxy_temp_path /tmp/proxy; fastcgi_temp_path /tmp/fastcgi; uwsgi_temp_path /tmp/uwsgi; scgi_temp_path /tmp/scgi;`
 const nodeNames=[]
 for(let i=0;i<2;i++){
  const id='node'+(i+1),name=owner+'-'+id;nodeNames.push(name)
  await privateFile(id+'/token',tokens[i]);await privateFile(id+'/agent.json',JSON.stringify({policy,nodeId:id,bind:'127.0.0.1:3009',rootDirectory:'/data/sub-apps',tokenFile:'/runtime/token'}))
  await privateFile('node'+(i+1)+'.token',tokens[i]);await copyToTools('node'+(i+1)+'.token','/runtime/node'+(i+1)+'.token')
  // The node's own tmpfs is its local storage; no other container mounts it.
  await privateFile(id+'/nginx.conf',baseNginx+`server {listen 8443 ssl; ssl_certificate /tls/cert.pem; ssl_certificate_key /tls/key.pem; root /data; client_max_body_size 25m; location /agent/ {proxy_pass http://127.0.0.1:3009; proxy_read_timeout 60s;} location ~ /\\. {return 404;} location /sub-apps/ {try_files $uri =404;} location / {return 404;} } }`,'644')
  containers.add(name)
  await docker(['create','--init','--name',name,'--label','saas.ci.owner='+owner,'--network',network,'--network-alias',id+'.phase35.test','--user','1000:1000','--cap-drop','ALL','--security-opt','no-new-privileges','--tmpfs','/data:rw,nosuid,size=128m,mode=777','--tmpfs','/tmp:rw,nosuid,size=32m','--mount','type=bind,src='+privateRoot+'/tls,dst=/tls,readonly','--mount','type=bind,src='+privateRoot+'/'+id+',dst=/runtime,readonly','--mount','type=bind,src='+privateRoot+'/bin,dst=/bin-owned,readonly','--publish','127.0.0.1::8443','--entrypoint','sh',toolsImage,'-c','mkdir -p /data/sub-apps; /bin-owned/agent /runtime/agent.json & exec nginx -c /runtime/nginx.conf -g "daemon off;"'])
  await docker(['start',name])
 }
 await privateFile('pg.env','POSTGRES_USER=ci\nPOSTGRES_PASSWORD='+password+'\nPOSTGRES_DB=platform\n')
 containers.add(pg);await docker(['create','--name',pg,'--label','saas.ci.owner='+owner,'--network',network,'--tmpfs','/var/lib/postgresql/data:rw,nosuid,size=1g','--env-file',privateRoot+'/pg.env',postgresImage]);await docker(['start',pg])
 await wait(async()=>{const r=await linux(['docker','--host','unix:///var/run/docker.sock','exec',pg,'pg_isready','-U','ci','-d','platform'],{allowFailure:true});return r.exitCode===0},'postgres')
 const sql=async q=>(await docker(['exec',pg,'psql','-U','ci','-d','platform','-v','ON_ERROR_STOP=1','-A','-t','-c',q])).trim()
 await privateFile('platform.env.json',JSON.stringify({DATABASE_URL:`postgresql://ci:${password}@${pg}:5432/platform`,JWT_SECRET:jwt,METRICS_TOKEN:randomBytes(40).toString('hex'),DB_STATEMENT_TIMEOUT_MS:'60000',BIND_ADDR:'0.0.0.0:3001',PUBLIC_APP_URL:origin,MICRO_GATEWAY_ORIGIN:origin,MICRO_ALLOWED_ORIGINS:origin,CORS_ALLOWED_ORIGINS:origin,APPLICATION_ENVIRONMENT:'development',APP_ENV:'test',AUTO_CREATE_DATABASE:'false',STATIC_DIR:'',RUST_LOG:'warn',MICRO_APPS_DIR:'/runtime/platform-artifacts',MICRO_CA_CERT_FILE:'/runtime/ca.pem',ZIP_DISTRIBUTION_CONFIG_FILE:'/runtime/zip.json'}))
 for(const [src,dst]of [['platform.env.json','/runtime/platform.env.json'],['tls/ca.pem','/runtime/ca.pem']])await copyToTools(src,dst)
 await exec(['mkdir','-p','/runtime/platform-artifacts']);await exec(['chmod','644','/runtime/zip.json','/runtime/ca.pem']);await exec(['chown','-R','10001:10001','/runtime'])
 await docker(['exec','--detach','--user','10001:10001',tools,'node','/workspace/tests/phase36-platform.mjs'])
 const nginxConfig=baseNginx+`upstream nodes {server node1.phase35.test:8443; server node2.phase35.test:8443;} server {listen 8443 ssl; ssl_certificate /tls/cert.pem; ssl_certificate_key /tls/key.pem; root /site; client_max_body_size 20m; location /api/ {proxy_pass http://${tools}:3001; proxy_read_timeout 180s;} location = /_zip_ready {internal; proxy_pass http://${tools}:3001/api/zip-distribution/route-ready; proxy_pass_request_body off; proxy_set_header Content-Length ""; proxy_set_header X-Original-URI $request_uri;} location /sub-apps/ {auth_request /_zip_ready; proxy_pass https://nodes; proxy_ssl_verify on; proxy_ssl_trusted_certificate /tls/ca.pem; proxy_ssl_server_name on; proxy_ssl_name node1.phase35.test;} location / {try_files $uri $uri/ /index.html;} } }`
 await privateFile('gateway/nginx.conf',nginxConfig,'644');containers.add(gateway)
 await docker(['create','--name',gateway,'--label','saas.ci.owner='+owner,'--network',network,'--network-alias','host.phase35.test','--user','101:101','--read-only','--cap-drop','ALL','--tmpfs','/tmp:rw,nosuid,size=32m','--publish','127.0.0.1::8443','--mount','type=bind,src='+privateRoot+'/gateway/nginx.conf,dst=/etc/nginx/nginx.conf,readonly','--mount','type=bind,src='+privateRoot+'/tls,dst=/tls,readonly','--mount','type=bind,src='+privateRoot+'/site,dst=/site,readonly','--entrypoint','nginx',nginx,'-g','daemon off;']);await docker(['start',gateway])
 const port=(await inspect('container',gateway)).NetworkSettings.Ports['8443/tcp'][0].HostPort,serverOrigin='https://127.0.0.1:'+port
 function httpsRequest(base,endpoint,{method='GET',body,token,trust=ca,servername}={}){return new Promise((resolve,reject)=>{const data=body===undefined?undefined:Buffer.isBuffer(body)?body:Buffer.from(JSON.stringify(body));const req=https.request(new URL(endpoint,base),{method,ca:trust,servername,agent:false,headers:{...(data?{'content-type':Buffer.isBuffer(body)?'application/zip':'application/json','content-length':data.length}:{}),...(token?{authorization:'Bearer '+token}:{})}},res=>{const chunks=[];res.on('data',b=>chunks.push(b));res.on('end',()=>{const bytes=Buffer.concat(chunks),text=bytes.toString();let value;try{value=JSON.parse(text)}catch{}resolve({status:res.statusCode,bytes,text,value})})});req.on('error',()=>reject(Error('owned TLS/connection failed')));req.setTimeout(180000,()=>req.destroy());req.end(data)})}
 const request=(p,o)=>httpsRequest(serverOrigin,p,o)
 await wait(async()=>{const r=await request('/api/health');return r.status===200},'platform')
 async function api(p,body,token,expected=200){const r=await request(p,{method:body===undefined?'GET':'POST',body,token});assert.equal(r.status,expected,p+' '+JSON.stringify(r.value));return r.value}
 const rootOrg=await sql("SELECT o.id FROM organizations o JOIN tenants t ON t.id=o.tenant_id WHERE t.is_platform AND o.parent_id IS NULL LIMIT 1")
 async function login(username,password,org){const t=await api('/api/auth/login',{username,password});secretValues.add(t.ticket);const s=await api('/api/auth/context',{organizationId:org||t.memberships[0].organizationId},t.ticket);secretValues.add(s.accessToken);secretValues.add(s.refreshToken);return s}
 const admin=await login('admin','admin123',rootOrg),token=admin.accessToken
 const app=await api('/api/applications',{code:'stores',name:'隔离ZIP分发示例',entryType:'HTML',entryUrl:'/sub-apps/stores/versions/1.0.0/index.html',basePath:'/micro/stores',runtimeMode:'IFRAME',status:'ENABLED',autoCreateMenu:false},token)
 const releases='/api/applications/'+app.id+'/releases',jobs='/api/applications/'+app.id+'/zip-distributions'
 const upload=async v=>api(releases+'/upload?environment=development&version='+v+'&entryType=HTML&format=zip',archive(v),token)
 const intent=r=>({releaseId:r.id,groupId:policy.id,requestId:randomUUID()})
 const job=async id=>(await api(jobs,undefined,token)).jobs.find(j=>j.id===id)
 const terminal=async id=>wait(async()=>{const j=await job(id);return ['READY','FAILED'].includes(j.state)&&j},'job terminal')
 const current=()=>api('/api/applications/'+app.id+'/deployments',undefined,token)
 const nodeRequest=async(i,p,o)=>{const port=(await inspect('container',nodeNames[i])).NetworkSettings.Ports['8443/tcp'][0].HostPort;return httpsRequest('https://127.0.0.1:'+port,p,o)}
 const details=[]
 for(const n of [tools,...nodeNames]){const info=await inspect('container',n);assert(!info.Mounts.some(m=>String(m.Source).includes('docker.sock')));assert.equal(info.HostConfig.Privileged,false);details.push({name:n,mounts:info.Mounts.map(m=>({type:m.Type,destination:m.Destination,source:m.Type==='tmpfs'?'independent-tmpfs':m.Source}))})}
 assert.equal((await inspect('container',nodeNames[0])).HostConfig.Tmpfs['/data'].includes('128m'),true)
 report.storage={sharedReleaseMount:false,nodes:details};pass('independent-local-storage',report.storage)
 const r1=await upload('1.0.0');assert.equal(r1.validationStatus,'DRAFT');const one=intent(r1)
 for(const body of [{...one,groupId:'foreign'},{...one,url:'https://arbitrary.test'},{...one,command:['sh']},{...one,rootDirectory:'/tmp'}])await api(jobs,body,token,400)
 const publicTargets=JSON.stringify(await api('/api/distribution-targets',undefined,token));assert(!tokens.some(t=>publicTargets.includes(t)));assert(!publicTargets.includes('tokenRef'));pass('closed-intents',{negativeRequests:4,secretsExcluded:true})
 const other=await login('zhangsan','Zs@12345');await api('/api/distribution-targets',undefined,other.accessToken,403)
 for(const [permission,p,body]of [['system:distribution:read','/api/distribution-targets',undefined],['system:distribution:write',jobs,one]]){const menu=await sql(`SELECT id FROM menus WHERE permission='${permission}' LIMIT 1`);assert(menu);await sql(`DELETE FROM organization_menus WHERE organization_id='${rootOrg}' AND menu_id='${menu}'`);await api(p,body,token,403);await sql(`INSERT INTO organization_menus(organization_id,menu_id) VALUES('${rootOrg}','${menu}')`)}pass('root-permissions',{organizationAdmin:403,independentReadWrite:true})
 await assert.rejects(request('/api/health',{trust:Buffer.alloc(0)}));await assert.rejects(nodeRequest(0,'/agent/health',{servername:'wrong.phase35.test'}));assert.equal((await nodeRequest(0,'/agent/health',{token:tokens[1]})).status,401)
 pass('tls-authentication',{wrongCA:true,wrongHostname:true,wrongNodeToken:401,normalTLS:true})
 await docker(['stop',nodeNames[1]])
 const created=await api(jobs,one,token);const duplicate=await api(jobs,one,token);assert.equal(created.id,duplicate.id);await api(jobs,{...one,groupId:'foreign'},token,400)
 let j=await terminal(created.id);assert.equal(j.state,'FAILED',JSON.stringify(j));assert.equal(j.nodes.find(n=>n.id==='node1').state,'READY');assert.equal((await request('/sub-apps/stores/versions/1.0.0/index.html')).status,403)
 await api(releases+'/'+r1.id+'/preflight',{},token,409);await api(releases+'/'+r1.id+'/activate',{},token,409);assert.deepEqual(await current(),[])
 pass('all-nodes-before-public',{offlineNode:'node2',otherNodeInstalled:true,publicCandidate:403,preflight:409,activation:409,activeCount:0})
 const before=await docker(['exec',nodeNames[0],'stat','-c','%i','/data/sub-apps/stores/versions/1.0.0/index.html'])
 await docker(['start',nodeNames[1]]);await api(jobs+'/'+created.id+'/retry',{},token);j=await terminal(created.id);assert.equal(j.state,'READY',JSON.stringify(j));assert.equal(await docker(['exec',nodeNames[0],'stat','-c','%i','/data/sub-apps/stores/versions/1.0.0/index.html']),before)
 assert.equal(await sql(`SELECT count(*) FROM zip_distribution_jobs WHERE release_id='${r1.id}'`),'1');for(let i=0;i<2;i++)assert.equal((await nodeRequest(i,'/sub-apps/stores/versions/1.0.0/app.js')).status,200)
 pass('idempotent-retry',{sameJob:true,attempt:j.attempt,node1InodeUnchanged:true,allNodesActualBytes:true})
 const check=await api(releases+'/'+r1.id+'/preflight',{},token);assert.equal(check.validationStatus,'PASSED',JSON.stringify(check));await api(releases+'/'+r1.id+'/activate',{},token)
 const context={...c,origin,serverOrigin,ca,request,httpsRequest,nodeRequest,nodeNames,policy,tokens,config,sql,api,login,admin,token,app,releases,jobs,upload,intent,job,terminal,current,rootOrg,r1,created,archive}
 await (await import('./phase35-zip-cases.mjs')).acceptance(context)
 await privateFile('regression.json',JSON.stringify({owner,databaseUrl:`postgresql://ci:${password}@${pg}:5432/postgres`}));await copyToTools('regression.json','/runtime/regression.json');await c.step('existing-release-regression',['node','tests/phase36-release-regression.mjs']);await streamToFile(['docker','--host','unix:///var/run/docker.sock','cp',tools+':/workspace/target/phase36-release-regression','-'],path.join(out,'release-regression.tar'))
 return {serverOrigin,ca,request,policy,registry:{scope:'none',normalTls:true},browser:report.browser}
}
