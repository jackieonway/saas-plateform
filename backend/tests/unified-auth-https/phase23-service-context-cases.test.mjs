import test from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {sourceProof, checkContextCase} from './phase23-service-context-cases.mjs';

const browser='a'.repeat(64),flow='01234567-89ab-7cde-8fab-0123456789ab';
const choice={contractVersion:'context-selection/1',sourceProof:sourceProof(browser,flow),expectedClientId:flow};
const result={contractVersion:'context-selection/1',transactionId:flow,credential:'b'.repeat(64),expectedClientId:flow,oidcAccessToken:'c'.repeat(64),bffSessionId:flow,version:'1',oldOidcAccessToken:null};
function environment(prefix) {return {identity:{CONTEXT_PLATFORM_SERVICE_ID:'selection',CONTEXT_PLATFORM_SERVICE_SECRET:prefix+'-selection'},portal:{CONTEXT_SELECTION_CONSUMER_ID:'consumer',CONTEXT_SELECTION_CONSUMER_SECRET:prefix+'-consumer'}};}
test('source handle follows the current browser and flow contract',()=>{
  assert.equal(choice.sourceProof,createHash('sha256').update(`auth-context-selection-source-v1:${browser}:${flow}`).digest('hex'));
  assert.throws(()=>sourceProof('bad',flow),/CONTEXT_BROWSER_INVALID/);
  assert.throws(()=>sourceProof(browser,'bad'),/CONTEXT_FLOW_INVALID/);
});
test('each live context control changes only secret and checks denial reason',async()=>{
  const source=environment('old'),target=environment('new');
  for(const [name,body] of [['selection',choice],['portalConsumer',result]]) {
    const calls=[];
    await checkContextCase({source,name,body,request:async input=>{calls.push(input);return {status:200};}});
    const evidence=await checkContextCase({source,target,name,body,request:async input=>{calls.push(input);return input.secret.startsWith('new')?{status:200}:{status:403,error:'selection_not_allowed'};}});
    assert.deepEqual(calls[1].body,calls[2].body);assert.equal(calls[1].client,calls[2].client);
    assert(!JSON.stringify(evidence).includes('credential'));
    await assert.rejects(checkContextCase({source,target,name,body,request:async input=>input.secret.startsWith('new')?{status:200}:{status:403,error:'other'}}),/OLD_CONTEXT_REJECTION_REASON_INVALID/);
  }
});
