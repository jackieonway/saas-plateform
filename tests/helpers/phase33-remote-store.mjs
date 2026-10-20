/** Test-only remote failure domain. No shared volume with the recovery client.
 * Production storage is an operator-owned immutable HTTPS object service. */
import https from 'node:https'
import { readFileSync,writeFileSync,existsSync } from 'node:fs'
const token=process.env.REMOTE_TOKEN
https.createServer({key:readFileSync('/data/server.key'),cert:readFileSync('/data/server.pem')},async(req,res)=>{
  if(req.headers.authorization!=='Bearer '+token){res.writeHead(401).end();return}
  const parts=[];let bytes=0
  for await(const b of req){bytes+=b.length;if(bytes>600*1024*1024){res.writeHead(413).end();return}parts.push(b)}
  const body=Buffer.concat(parts)
  if(req.url==='/internal/events'){
    const reply=await fetch(process.env.MONITOR_TARGET,{method:'POST',headers:{authorization:req.headers.authorization,'content-type':'application/json'},body})
    res.writeHead(reply.status).end(await reply.text());return
  }
  if(!/^\/objects\/[a-z0-9_-]+$/.test(req.url)){res.writeHead(404).end();return}
  const path='/data'+req.url
  if(req.method==='PUT'){
    if(req.headers['if-none-match']!=='*'){res.writeHead(428).end();return}
    try{writeFileSync(path,body,{flag:'wx',mode:0o600});res.writeHead(201).end()}catch{res.writeHead(412).end()}return
  }
  if(req.method==='GET'&&existsSync(path)){const data=readFileSync(path);res.writeHead(200,{'content-length':data.length}).end(data);return}
  res.writeHead(404).end()
}).listen(9443,'0.0.0.0')
