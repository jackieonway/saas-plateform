import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';

export function sourceProof(browser, flowId) {
  assert(typeof browser === 'string' && /^[0-9a-f]{64}$/.test(browser), 'CONTEXT_BROWSER_INVALID');
  assert(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(flowId), 'CONTEXT_FLOW_INVALID');
  return createHash('sha256').update(`auth-context-selection-source-v1:${browser}:${flowId}`).digest('hex');
}

export function contextCase(environment, name, body) {
  let value;
  if (name === 'selection') {
    assert(body?.contractVersion === 'context-selection/1' && /^[0-9a-f]{64}$/.test(body.sourceProof) && body.expectedClientId, 'FRESH_CHOICE_REQUIRED');
    value={name, client:environment.identity.CONTEXT_PLATFORM_SERVICE_ID, secret:environment.identity.CONTEXT_PLATFORM_SERVICE_SECRET, path:'/internal/platform/context-selection/choices', body};
  } else {
    assert.equal(name,'portalConsumer','CONTEXT_CASE_INVALID');
    assert(body?.contractVersion === 'context-selection/1' && body.transactionId && body.credential && body.expectedClientId && body.oidcAccessToken && body.bffSessionId && /^[1-9][0-9]*$/.test(body.version), 'FRESH_RESULT_REQUIRED');
    value={name, client:environment.portal.CONTEXT_SELECTION_CONSUMER_ID, secret:environment.portal.CONTEXT_SELECTION_CONSUMER_SECRET, path:'/internal/platform/context-selection/result', body};
  }
  assert(typeof value.client === 'string' && value.client && typeof value.secret === 'string' && value.secret, 'SERVICE_BINDING_MISSING');
  return value;
}

// Call each case while its real source is still valid. No request material is returned.
export async function checkContextCase({ source, target, body, name, request }) {
  const previous=contextCase(source,name,body);
  if (!target) assert.equal((await request(previous)).status,200,'SOURCE_CONTEXT_CONTROL_FAILED');
  else {
    const fresh=contextCase(target,name,body);
    assert.equal(previous.client,fresh.client,'SERVICE_CLIENT_CHANGED');
    assert.notEqual(previous.secret,fresh.secret,'SERVICE_SECRET_NOT_ROTATED');
    assert.equal((await request(fresh)).status,200,'NEW_CONTEXT_CONTROL_FAILED');
    const denied=await request({...fresh,secret:previous.secret});
    assert.equal(denied.status,403,'OLD_CONTEXT_SECRET_NOT_REJECTED');
    assert.equal(denied.error,'selection_not_allowed','OLD_CONTEXT_REJECTION_REASON_INVALID');
  }
  return {name,source:!target,target:Boolean(target)};
}
