import test from 'node:test';
import assert from 'node:assert/strict';
import {createCipheriv, randomBytes} from 'node:crypto';
import {openSessionAccess, openSessionEnvelope, checkServiceAccessControls} from './phase23-service-access-cases.mjs';

test('session AEAD binds session, client, issuer and service domain', () => {
  const key = randomBytes(32), nonce = randomBytes(12);
  const session = {id:'session', issuer:'https://issuer', clientId:'client'};
  const cipher = createCipheriv('aes-256-gcm', key, nonce);
  cipher.setAAD(Buffer.from(`platform-bff-aead/1\0${session.id}\0${session.issuer}\0${session.clientId}\0oidc-access`));
  const encrypted = Buffer.concat([cipher.update('sample-access'),cipher.final(),cipher.getAuthTag()]);
  session.accessCiphertext = `v1.test.${Buffer.concat([nonce,encrypted]).toString('base64url')}`;
  const ring = {keys:[{kid:'test',key:key.toString('base64url')}]};
  assert.equal(openSessionAccess('employee',session,ring),'sample-access');
  assert.throws(() => openSessionAccess('customer',session,ring));
  assert.throws(() => openSessionAccess('employee',{...session,clientId:'other'},ring));
  assert.throws(() => openSessionAccess('employee',session,{keys:[]}));
  const pendingCipher=createCipheriv('aes-256-gcm',key,nonce);
  pendingCipher.setAAD(Buffer.from(`customer-portal-bff-aead/1\0${session.id}\0${session.issuer}\0${session.clientId}\0pending-selection`));
  const pending=Buffer.concat([nonce,pendingCipher.update('{"extension":{}}'),pendingCipher.final(),pendingCipher.getAuthTag()]);
  assert.equal(openSessionEnvelope('customer',session,ring,'pending-selection',`v1.test.${pending.toString('base64url')}`),'{"extension":{}}');
  assert.throws(()=>openSessionEnvelope('customer',session,ring,'oidc-access',`v1.test.${pending.toString('base64url')}`));
});

function environment(prefix) {
  const platform = {}, portal = {};
  for (const [group, name] of [[platform,'CONTEXT_SOURCE_SERVICE_CLIENT'],[portal,'CUSTOMER_SOURCE_SERVICE_CLIENT'],[platform,'CUSTOMER_AUTHORITY_SERVICE_CLIENT'],[portal,'CUSTOMER_RESOURCE_CLIENT']]) {
    group[`${name}_ID`] = name;
    group[`${name}_SECRET`] = `${prefix}-${name}`;
  }
  return {platform,portal};
}
const employee={clientId:'employee-client',accessToken:'fresh-employee'}, customer={clientId:'customer-client',accessToken:'fresh-customer'};
test('access proof controls differ only by secret and reject false positives', async () => {
  const source=environment('old'),target=environment('new'),calls=[];
  const sourceResult=await checkServiceAccessControls({source,employee,customer,request:async request=>{calls.push(request);return 200;}});
  const targetResult=await checkServiceAccessControls({source,target,employee,customer,request:async request=>{calls.push(request);return request.secret.startsWith('old-')?request.denied:200;}});
  assert.equal(sourceResult.length,4);assert.equal(targetResult.length,4);assert.equal(calls.length,12);
  for(let i=0;i<4;i++) { const a=calls[4+i*2],b=calls[5+i*2];assert.equal(a.path,b.path);assert.equal(a.client,b.client);assert.deepEqual(a.body,b.body); }
  assert(!JSON.stringify(targetResult).includes('fresh-'));
  await assert.rejects(checkServiceAccessControls({source,target,employee,customer,request:async req=>req.secret.startsWith('new-')?200:503}),/OLD_SERVICE_ACCESS_NOT_REJECTED/);
});
