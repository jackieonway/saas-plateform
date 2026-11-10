import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { build } from 'esbuild'
import { renderToStaticMarkup } from 'react-dom/server'
const require=createRequire(import.meta.url)
async function compile(file) {
  const output=await build({entryPoints:[fileURLToPath(new URL(file,import.meta.url))],bundle:true,write:false,platform:'node',format:'cjs',packages:'external',tsconfig:fileURLToPath(new URL('../tsconfig.json',import.meta.url))})
  const compiled={exports:{}}
  new Function('require','module','exports',output.outputFiles[0].text)(require,compiled,compiled.exports)
  return compiled.exports
}
const {ClientAuthorizationSettings}=await compile('../src/pages/system/oauth/ClientAuthorizationSettings.tsx')
const {validTokenSeconds,tokenSeconds}=await compile('../src/pages/system/oauth/client-policy.ts')
const draft={autoAuthorize:false,refreshToken:true,accessTokenTtlSeconds:'',refreshTokenTtlSeconds:'',clientTokenTtlSeconds:''}
test('login settings distinguish consent and show inherited lifetimes with rotation',()=>{
  const html=renderToStaticMarkup(ClientAuthorizationSettings({machine:false,editing:false,firstParty:true,value:draft,onChange(){},defaults:{accessTokenTtlSeconds:300,refreshTokenTtlSeconds:1800,clientTokenTtlSeconds:300}}))
  for(const label of ['授权设置','授权码（PKCE）','刷新令牌','授权确认方式','用户确认授权（默认）','继承全局配置','访问令牌有效期','刷新令牌空闲有效期','刷新令牌策略：轮换']) assert.ok(html.includes(label),label)
  assert.ok(!html.includes('客户端令牌有效期'))
})
test('machine settings expose only the machine lifetime and no user consent controls',()=>{
  const html=renderToStaticMarkup(ClientAuthorizationSettings({machine:true,editing:false,firstParty:true,value:{...draft,refreshToken:false},onChange(){}}))
  assert.ok(html.includes('客户端令牌有效期'))
  for(const label of ['授权确认方式','访问令牌有效期','刷新令牌空闲有效期']) assert.ok(!html.includes(label),label)
})
test('TTL parsing preserves inheritance and rejects permanent, fractional or out-of-range values',()=>{
  assert.equal(tokenSeconds(''),null)
  assert.equal(tokenSeconds('60'),60)
  assert.equal(tokenSeconds('604800',true),604800)
  for(const value of ['-1','0','59','3601','1.5','1e3',' 300']) assert.equal(validTokenSeconds(value),false,value)
  assert.throws(()=>tokenSeconds('-1'))
  assert.equal(validTokenSeconds('299',true),false)
})
