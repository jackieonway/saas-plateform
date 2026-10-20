import assert from 'node:assert/strict'
import {readFileSync,writeFileSync,mkdirSync} from 'node:fs'
import {launchBrowser,browserName} from './browser/runtime.mjs'
const runtime=JSON.parse(readFileSync('/runtime/browser.json')),mode=process.argv[2]
assert(['--submit','--verify'].includes(mode))
mkdirSync('/workspace/target',{recursive:true})
const result={status:'FAIL',product:browserName,ignoreTlsErrors:false,requests:[]}
let browser,context
try{
 browser=await launchBrowser();result.version=browser.version();context=await browser.newContext({viewport:{width:1440,height:1100}})
 await context.addInitScript(({session,refreshToken,systemId})=>{
  if(window!==window.top||sessionStorage.getItem('phase36-seeded'))return
  localStorage.setItem('platform-session-v1',JSON.stringify({schema:1,version:1,sessionId:crypto.randomUUID(),status:'active',token:session,refreshToken}))
  localStorage.setItem('platform-current-app',systemId);localStorage.setItem('platform-theme','light');sessionStorage.setItem('phase36-seeded','true')
 },runtime)
 const page=await context.newPage();page.setDefaultTimeout(30000);const errors=[];page.on('pageerror',e=>errors.push(e.message))
 await page.goto(runtime.origin+'/system/applications');const card=page.locator('div.group.relative').filter({has:page.getByText(runtime.applicationName,{exact:true})})
 await card.getByRole('button',{name:'发布版本',exact:true}).click()
 const dialog=page.getByRole('dialog',{name:runtime.applicationName+' · 发布版本',exact:true});await dialog.waitFor()
 const panel=dialog.getByRole('region',{name:'受管镜像部署',exact:true});await panel.waitFor()
 if(mode==='--submit'){
  let lost=false,claimed=false,handlerError
  await page.route('**/api/applications/'+runtime.applicationId+'/managed-deployments',async route=>{
   if(route.request().method()!=='POST')return route.continue()
   const payload=route.request().postDataJSON();result.requests.push(payload)
   if(claimed)return route.continue();claimed=true
   try{const response=await route.fetch();assert.equal(response.status(),200);await response.dispose();await route.abort('failed');lost=true}catch(e){handlerError=e;try{await route.abort('failed')}catch{}}
  })
  await panel.getByLabel('镜像发布版本',{exact:true}).selectOption(runtime.releaseId)
  await panel.getByLabel('部署目标',{exact:true}).selectOption(runtime.targetId)
  await panel.getByLabel('服务模板',{exact:true}).selectOption(runtime.templateId)
  await panel.getByLabel('固定镜像地址',{exact:true}).fill(runtime.imageRef)
  await panel.getByRole('button',{name:'提交镜像部署',exact:true}).click()
  await panel.getByRole('alert').waitFor()
  const deadline=Date.now()+30000;while(!lost&&!handlerError&&Date.now()<deadline)await new Promise(r=>setTimeout(r,20));if(handlerError)throw handlerError;assert(lost)
  const response=page.waitForResponse(r=>r.request().method()==='POST'&&r.url().endsWith('/managed-deployments'))
  await panel.getByRole('button',{name:'提交镜像部署',exact:true}).click();assert.equal((await response).status(),200)
  assert.equal(result.requests.length,2);assert.equal(result.requests[0].requestId,result.requests[1].requestId)
  await panel.getByText('等待执行',{exact:true}).first().waitFor()
  await page.screenshot({path:'/workspace/target/phase36-managed-ui.png',fullPage:true})
 }else{
  const article=panel.locator('article').filter({has:page.getByText('3.0.0 · '+runtime.targetId,{exact:true})})
  await article.getByText('部署就绪',{exact:true}).waitFor()
  await page.screenshot({path:'/workspace/target/phase36-ready.png',fullPage:true})
 }
 assert.deepEqual(errors,[])
 result.status='PASS';console.log('PHASE36_BROWSER '+JSON.stringify({status:result.status,product:result.product,version:result.version,mode}))
}catch(error){
 if(context?.pages()[0])await context.pages()[0].screenshot({path:'/workspace/target/phase36-failure.png',fullPage:true}).catch(()=>{})
 throw error
}finally{
 if(context)await context.close();if(browser)await browser.close();result.browserClosed=true
 writeFileSync('/workspace/target/phase36-browser-result.json',JSON.stringify(result,null,2))
}
