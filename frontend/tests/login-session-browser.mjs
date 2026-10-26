/** Real platform UI and Cookie session consumer; isolated SSO HTTP authority. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { launchBrowser } from './browser/runtime.mjs'
import { fixtureUuid, installSsoAuthMode, profileWithIdentity, identityForProfile } from './browser/identity-fixture.mjs'
const workspace=path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const host=process.platform==='win32'?'::1':'127.0.0.1'
const server=net.createServer(); await new Promise(resolve=>server.listen(0,host,resolve));const port=server.address().port;await new Promise(resolve=>server.close(resolve))
const origin=`http://${host.includes(':')?'['+host+']':host}:${port}`
const provider='https://auth.sso-fixture.test'
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms))
const vite=spawn(process.execPath,['node_modules/vite/bin/vite.js','--host',host,'--port',String(port),'--strictPort'],{cwd:workspace,stdio:'ignore',windowsHide:true})
let browser
try {
  for(let i=0;i<100;i++){try{if((await fetch(origin)).ok)break}catch{}await pause(100)}
  browser=await launchBrowser()
  for(const returnTo of ['/profile?login-check=1','/dashboard']) {
    const context=await browser.newContext(), auth=await installSsoAuthMode(context), page=await context.newPage()
    const uid=fixtureUuid('sso-browser-user'),tid=fixtureUuid('sso-browser-tenant'),oid=fixtureUuid('sso-browser-organization')
    const member={userId:uid,tenantId:tid,organizationId:oid,membershipId:fixtureUuid('sso-browser-membership'),memberType:'EMPLOYEE',identityVersion:'1',status:'ENABLED',consoleSelectable:true,selectionDisabledReason:null,tenantName:'统一登录租户',organizationName:'统一登录组织',organizationPath:'/统一登录组织'}
    const profile=profileWithIdentity({membershipId:member.membershipId,memberType:"EMPLOYEE",identityVersion:"1",user:{id:uid,username:'sso-user',realName:'SSO 验证用户',email:'',phone:'',avatar:'',gender:'',isSuperAdmin:false},tenant:{id:tid,name:member.tenantName,code:'sso-fixture',isPlatform:false},organization:{id:oid,name:member.organizationName,path:member.organizationPath},applications:[{id:fixtureUuid('sso-system'),code:'system',name:'系统管理',basePath:'/system',icon:'LayoutGrid',entryType:'HTML',runtimeMode:'TRUSTED'}],memberships:[member],platformContext:false})
    const errors=[];page.on('requestfailed',r=>console.log('SSO_REQUEST_FAILED',new URL(r.url()).pathname,r.failure()?.errorText));page.on('pageerror',e=>{errors.push(e.message);console.log('SSO_PAGE_ERROR',e.message)});page.on('framenavigated',f=>{if(f===page.mainFrame())console.log('SSO_BROWSER_URL',new URL(f.url()).origin+new URL(f.url()).pathname)})
    let providerVisits=0, callbacks=0
    await context.route(url=>url.origin===origin && url.pathname==='/auth/login',route=>{console.log('SSO_LOGIN_REDIRECT');return route.fulfill({contentType:'text/html; charset=utf-8',body:'<script>location.replace('+JSON.stringify(provider+'/sso/login')+')</script>'})})
    await context.route(provider+'/**',async route=>{
      if(route.request().method()==='POST') {
        auth.proof=identityForProfile(profile);auth.profile=profile;auth.token='sso-only-context-bearer';auth.selected=false
        return route.fulfill({contentType:'text/html; charset=utf-8',body:'<script>location.replace('+JSON.stringify(origin+'/auth/callback?code=owned-fixture&state=owned-fixture')+')</script>'})
      }
      providerVisits++;console.log('SSO_PROVIDER_REQUEST',route.request().method())
      return route.fulfill({contentType:'text/html; charset=utf-8',body:'<!doctype html><html lang="zh"><title>统一身份认证</title><h1>统一身份认证</h1><form method="post" action="/sso/login"><label>账号<input name="username" autocomplete="username"></label><label>密码<input name="password" type="password" autocomplete="current-password"></label><button>登录</button></form></html>'})
    })
    await context.route(url=>url.origin===origin && url.pathname==='/auth/callback',route=>{callbacks++;return route.fulfill({status:303,headers:{location:origin+'/login?return='+encodeURIComponent(returnTo)}})})
    await context.route(url=>url.origin===origin && url.pathname.startsWith('/api/'),route=>{
      const p=new URL(route.request().url()).pathname
      const reply=body=>route.fulfill({contentType:'application/json',body:JSON.stringify(body)})
      if(p==='/api/platform/branding')return reply({platformName:'SSO 验收平台',platformDescription:'隔离 HTTP 认证夹具',footerText:''})
      if(p==='/api/auth/profile')return reply(profile)
      if(p==='/api/auth/permissions')return reply({permissions:[],roles:[]})
      if(p==='/api/auth/menus')return reply([])
      if(p.startsWith('/api/monitor/'))return route.fulfill({status:403,contentType:'application/json',body:'{"code":"FORBIDDEN"}'})
      if(p==='/api/notifications/prefs')return reply({enableSound:false,enablePopup:false,dndEnabled:false,mutedApplicationIds:[]})
      if(p==='/api/notifications/unread-count')return reply({count:0})
      if(p==='/api/auth/my-sessions')return reply({clients:[],platformSessions:[]})
      return reply({records:[],total:0,page:1,size:20})
    })
    await page.goto(origin+'/login?return='+encodeURIComponent(returnTo))
    await page.getByRole('button',{name:'继续统一登录',exact:true}).waitFor()
    assert.equal(await page.locator('input[type=password]').count(),0)
    await page.getByRole('button',{name:'继续统一登录',exact:true}).click()
    await page.waitForURL(url=>url.origin===provider)
    await page.getByLabel('账号',{exact:true}).fill('sso-user');await page.getByLabel('密码',{exact:true}).fill('owned-fixture-password')
    await page.getByRole('button',{name:'登录',exact:true}).click()
    await page.getByRole('heading',{name:'选择本次员工身份',exact:true}).waitFor()
    assert.equal(await page.locator('input[type=password]').count(),0)
    await page.getByRole('button').filter({hasText:member.organizationName}).click()
    await page.waitForURL(url=>url.pathname+url.search===returnTo)
    await page.getByRole('complementary',{name:'应用导航',exact:true}).waitFor({timeout:15000}).catch(async e=>{console.log('SSO_VISIBLE_STATE',await page.locator('body').innerText());throw e})
    await page.reload();await page.getByRole('complementary',{name:'应用导航',exact:true}).waitFor()
    assert.equal(providerVisits,1);assert.equal(callbacks,1)
    for(const key of ['platform-session-v2','platform-refresh-token','platform-token'])assert.equal(await page.evaluate(k=>localStorage.getItem(k),key),null)
    await page.getByRole('button',{name:'用户菜单',exact:true}).click()
    await page.getByRole('menuitem',{name:'退出本平台',exact:true}).click()
    await page.getByRole('button',{name:'继续统一登录',exact:true}).waitFor()
    assert.equal(providerVisits,1,'Logout must not silently begin another authentication')
    assert.deepEqual(errors,[])
    await context.close()
    console.log('PASS SSO redirect, provider form, callback, explicit employee selection, return path, Cookie reload and logout '+returnTo)
  }
  console.log('SSO login session browser checks: 2 passed (isolated SSO HTTP authority, real platform UI)')
} finally {await browser?.close();vite.kill();await pause(100)}
