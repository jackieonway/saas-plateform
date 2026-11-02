/** Standard SDK operations for the current Phase25 issuer. Raw carriers stay in memory. */
import { randomBytes } from 'node:crypto';
import { safeSdkCode } from './phase23-protocol-diagnostics.mjs';

const need = (condition, code) => { if (!condition) throw new Error(`PHASE25_PROTOCOL_${code}`); };
const label = () => randomBytes(24).toString('base64url');
export const authorizeMutations = Object.freeze({
  plain: ['code_challenge_method', 'plain'],
  missing: ['code_challenge', null],
  short: ['code_challenge', 'abc'],
  long: ['code_challenge', 'a'.repeat(129)],
  syntax: ['code_challenge', 'a'.repeat(42) + '!'],
  redirect: ['redirect_uri', 'https://phase25-unregistered.invalid/callback'],
  scope: ['scope', 'openid profile unapproved.phase25.scope'],
});

export function mutateAuthorization(value, mutation) {
  const url = new URL(value);
  if (mutation !== undefined) {
    need(Object.hasOwn(authorizeMutations, mutation), 'MUTATION_REJECTED');
    const [name, next] = authorizeMutations[mutation];
    if (next === null) url.searchParams.delete(name); else url.searchParams.set(name, next);
  }
  return url.href;
}

export function createProtocolCases({ runtime, client, jose, configurations, oidc, fetch, concurrent }) {
  const flows = new Map(), sessions = new Map();
  let lastStatus = 0;
  const trackedFetch = async (...args) => { const response = await fetch(...args); lastStatus = response.status; return response; };
  const wire = configuration => {
    configuration[client.customFetch] = trackedFetch;
    client.enableNonRepudiationChecks(configuration);
    return configuration;
  };
  for (const value of Object.values(oidc)) wire(value.configuration);
  function alternative(profile, { clientId, secret, clockTolerance } = {}) {
    const config = configurations[profile];
    return wire(new client.Configuration(oidc[profile].metadata, clientId ?? config.clientId,
      { client_secret: secret ?? config.clientSecret, id_token_signed_response_alg: 'RS256',
        [client.clockTolerance]: clockTolerance ?? 30 }, client.ClientSecretBasic(secret ?? config.clientSecret)));
  }
  const checks = flow => ({ pkceCodeVerifier: flow.verifier, expectedState: flow.state,
    expectedNonce: flow.nonce, idTokenExpected: true });
  const flowFor = key => { const value = flows.get(key); need(value, 'FLOW_UNKNOWN'); return value; };
  const sessionFor = key => { const value = sessions.get(key); need(value, 'SESSION_UNKNOWN'); return value; };
  async function denied(action, expected, status) {
    lastStatus = 0;
    try { await action(); } catch (error) {
      const sdkCode = safeSdkCode(error);
      need(expected.includes(sdkCode) && (!status || status.includes(lastStatus)), 'EXPECTED_DENIAL_NOT_OBSERVED');
      return { rejected: true, sdkCode, upstreamStatus: lastStatus };
    }
    throw new Error('PHASE25_PROTOCOL_NEGATIVE_ACCEPTED');
  }
  async function rawToken(profile, parameters, wrongSecret = false) {
    const config = configurations[profile];
    const encode = value => new URLSearchParams({ v: value }).toString().slice(2);
    const auth = Buffer.from(`${encode(config.clientId)}:${encode(wrongSecret ? 'phase25-deliberately-wrong-secret' : config.clientSecret)}`).toString('base64');
    const response = await trackedFetch(oidc[profile].metadata.token_endpoint, { method: 'POST',
      headers: { Authorization: `Basic ${auth}`, 'Content-Type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams(parameters) });
    let value; try { value = await response.json(); } catch { value = {}; }
    return { response, value };
  }
  async function execute(message) {
    need(message && typeof message === 'object' && !Array.isArray(message), 'INPUT_REJECTED');
    switch (message.command) {
      case 'begin': {
        need(['third', 'offline'].includes(message.profile) && flows.size < 64, 'PROFILE_OR_CAPACITY_REJECTED');
        const config = configurations[message.profile];
        const flow = await runtime.beginLogin(oidc[message.profile], config, message.prompt ?? 'consent');
        const key = label(); flows.set(key, { ...flow, profile: message.profile, attempts: 0 });
        return { key, authorizationUrl: mutateAuthorization(flow.authorizationUrl, message.mutation) };
      }
      case 'finish': {
        const flow = flowFor(message.key), callback = new URL(message.callback), config = configurations[flow.profile];
        need(callback.origin === config.origin && callback.pathname === '/auth/callback' && !callback.hash
          && !callback.username && !callback.password && callback.searchParams.has('code') && ++flow.attempts <= 4, 'CALLBACK_REJECTED');
        const mode = message.mode ?? 'valid';
        need(['valid', 'state', 'issuer', 'nonce', 'verifier', 'client', 'redirect', 'stale-auth-time', 'concurrent'].includes(mode), 'MODE_REJECTED');
        if (mode === 'concurrent') {
          need(typeof concurrent === 'function', 'CONCURRENCY_PORT_REQUIRED');
          const result = await concurrent({ profile: flow.profile, code: callback.searchParams.get('code'),
            verifier: flow.verifier, redirectUri: config.redirectUri });
          need(result.processesDistinct === true && result.statuses.length === 2
            && [...result.statuses].sort().join(',') === '200,400'
            && result.errors.filter(value => value === 'invalid_grant').length === 1, 'DUAL_PROCESS_CODE_RESULT_INVALID');
          return { independentProcesses: true, successfulExchanges: 1, rejectedExchanges: 1, invalidGrant: true };
        }
        const expected = checks(flow);
        let configuration = oidc[flow.profile].configuration;
        if (mode === 'state') callback.searchParams.set('state', label());
        if (mode === 'issuer') callback.searchParams.set('iss', 'https://phase25-wrong-issuer.invalid');
        if (mode === 'nonce') expected.expectedNonce = label();
        if (mode === 'verifier') expected.pkceCodeVerifier = client.randomPKCECodeVerifier();
        if (mode === 'client') configuration = alternative(flow.profile, { clientId: configurations[flow.profile === 'third' ? 'offline' : 'third'].clientId,
          secret: configurations[flow.profile === 'third' ? 'offline' : 'third'].clientSecret });
        if (mode === 'redirect') callback.pathname += '/unregistered';
        if (mode === 'stale-auth-time') { configuration = alternative(flow.profile, { clockTolerance: 0 }); expected.maxAge = 0; }
        if (mode !== 'valid') {
          const sdkCodes = mode === 'stale-auth-time' ? ['OAUTH_JWT_TIMESTAMP_CHECK_FAILED']
            : ['OAUTH_INVALID_RESPONSE', 'OAUTH_JWT_CLAIM_COMPARISON_FAILED', 'OAUTH_RESPONSE_BODY_ERROR', 'OAUTH_WWW_AUTHENTICATE_CHALLENGE'];
          const status = ['client', 'redirect', 'verifier'].includes(mode) ? [400, 401] : mode === 'nonce' || mode === 'stale-auth-time' ? [200] : undefined;
          return denied(() => client.authorizationCodeGrant(configuration, callback, expected), sdkCodes, status);
        }
        const raw = await client.authorizationCodeGrant(configuration, callback, expected);
        const claims = raw.claims(), token = runtime.tokenData(raw, undefined, config.scope);
        const info = await runtime.userInfo(oidc[flow.profile], token);
        need(claims?.iss === config.issuer && (Array.isArray(claims.aud) ? claims.aud.length === 1 && claims.aud[0] === config.clientId : claims.aud === config.clientId)
          && claims.nonce === flow.nonce && info.sub === claims.sub && token.subject === claims.sub
          && typeof claims.sub === 'string' && !/^[0-9a-f]{8}-[0-9a-f-]{27}$/i.test(claims.sub), 'VALIDATED_CLAIMS_MISMATCH');
        need(Array.isArray(message.authWindow) && message.authWindow.length === 2
          && message.authWindow.every(Number.isFinite) && Number.isInteger(claims.auth_time)
          && claims.auth_time >= Math.floor(message.authWindow[0] / 1000) - 1
          && claims.auth_time <= Math.ceil(message.authWindow[1] / 1000) + 1, 'AUTH_TIME_WINDOW_MISMATCH');
        const cache = client.getJwksCache(configuration); need(cache?.jwks?.keys?.length, 'VALIDATED_JWKS_REQUIRED');
        const keySet = jose.createLocalJWKSet(cache.jwks);
        await jose.jwtVerify(raw.id_token, keySet, { issuer: config.issuer, audience: config.clientId, algorithms: ['RS256'] });
        for (const [field, bad] of [['issuer', 'https://phase25-wrong-issuer.invalid'], ['audience', 'phase25-wrong-client']]) {
          const negative = await denied(() => jose.jwtVerify(raw.id_token, keySet, {
            issuer: config.issuer, audience: config.clientId, algorithms: ['RS256'], [field]: bad }), ['ERR_JWT_CLAIM_VALIDATION_FAILED']);
          need(negative.rejected, 'JWT_EXPECTATION_ACCEPTED');
        }
        const key = label(); sessions.set(key, { profile: flow.profile, token, raw, originalRefresh: token.refreshToken, flow, callback: callback.href });
        return { key, issuerExact: true, audienceExact: true, opaqueSubject: true, nonceExact: true,
          authTimeWithinActualPrimaryWindow: true, idTokenUserInfoSubjectExact: true,
          signedTokenWrongExpectedIssuerRejected: true, signedTokenWrongExpectedAudienceRejected: true,
          hasRefresh: typeof token.refreshToken === 'string' };
      }
      case 'code-replay': {
        const session = sessionFor(message.key);
        return denied(() => client.authorizationCodeGrant(oidc[session.profile].configuration,
          new URL(session.callback), checks(session.flow)), ['OAUTH_RESPONSE_BODY_ERROR'], [400]);
      }
      case 'subjects': {
        const left = sessionFor(message.left), right = sessionFor(message.right);
        return { sameSubject: left.token.subject === right.token.subject, sameApplicationProfile: left.profile === right.profile };
      }
      case 'refresh': {
        const session = sessionFor(message.key), previous = session.token;
        const raw = await client.refreshTokenGrant(oidc[session.profile].configuration, previous.refreshToken);
        session.token = runtime.tokenData(raw, previous); session.raw = raw;
        return { rotated: session.token.refreshToken !== previous.refreshToken,
          subjectAndSidPreserved: session.token.subject === previous.subject && session.token.sid === previous.sid,
          scopeNotExpanded: session.token.scope.split(' ').every(value => previous.scope.split(' ').includes(value)) };
      }
      case 'refresh-replay': {
        const session = sessionFor(message.key);
        return denied(() => client.refreshTokenGrant(oidc[session.profile].configuration, session.originalRefresh), ['OAUTH_RESPONSE_BODY_ERROR'], [400]);
      }
      case 'refresh-denied': {
        const session = sessionFor(message.key);
        return denied(() => client.refreshTokenGrant(oidc[session.profile].configuration, session.token.refreshToken), ['OAUTH_RESPONSE_BODY_ERROR'], [400]);
      }
      case 'refresh-scope': {
        const session = sessionFor(message.key), previous = session.token;
        const originalScopes = previous.scope.split(/\s+/).filter(Boolean);
        need(originalScopes.includes('openid') && originalScopes.includes('profile')
          && !originalScopes.includes('offline_access'), 'REFRESH_SCOPE_TEST_BASELINE_INVALID');
        const shrunk = await rawToken(session.profile, { grant_type: 'refresh_token', refresh_token: previous.refreshToken, scope: 'openid' });
        need(shrunk.response.status === 200 && shrunk.value.scope === 'openid'
          && typeof shrunk.value.access_token === 'string' && shrunk.value.access_token.length > 0
          && typeof shrunk.value.refresh_token === 'string' && shrunk.value.refresh_token.length > 0
          && shrunk.value.refresh_token !== previous.refreshToken, 'REFRESH_SCOPE_SHRINK_FAILED');
        session.token = { ...previous, refreshToken: shrunk.value.refresh_token, accessToken: shrunk.value.access_token, scope: 'openid' };
        // A narrower access response does not remove the original grant's
        // authorization. It is legal to select its original scopes again.
        const restored = await rawToken(session.profile, { grant_type: 'refresh_token', refresh_token: session.token.refreshToken, scope: previous.scope });
        const restoredScopes = typeof restored.value.scope === 'string' ? restored.value.scope.split(/\s+/).filter(Boolean) : [];
        need(restored.response.status === 200 && restoredScopes.length === originalScopes.length
          && new Set(restoredScopes).size === restoredScopes.length
          && restoredScopes.every(value => originalScopes.includes(value))
          && typeof restored.value.access_token === 'string' && restored.value.access_token.length > 0
          && typeof restored.value.refresh_token === 'string' && restored.value.refresh_token.length > 0
          && restored.value.refresh_token !== session.token.refreshToken,
        'REFRESH_ORIGINAL_GRANT_SCOPE_RESELECTION_FAILED');
        session.token = { ...previous, refreshToken: restored.value.refresh_token, accessToken: restored.value.access_token, scope: restored.value.scope };
        const escalation = await rawToken(session.profile, { grant_type: 'refresh_token', refresh_token: session.token.refreshToken, scope: 'openid offline_access' });
        need(escalation.response.status === 400 && escalation.value.error === 'invalid_grant', 'REFRESH_SCOPE_EXPANSION_ACCEPTED');
        return { shrunkToOpenid: true, originalGrantScopeReselectionAllowed: true,
          scopeOutsideOriginalGrantDenied: true, unauthorizedScope: 'offline_access', rejection: 'invalid_grant', rotated: true };
      }
      case 'wrong-secret': {
        const response = await rawToken('machine', { grant_type: 'client_credentials', scope: configurations.machine.scope }, true);
        need(response.response.status === 401 && response.value.error === 'invalid_client', 'WRONG_SECRET_NOT_REJECTED');
        return { httpStatus: 401, invalidClient: true };
      }
      case 'client-separation': {
        const tokens = [];
        for (const profile of ['machine', 'resource']) {
          const issued = await rawToken(profile, { grant_type: 'client_credentials', scope: configurations[profile].scope });
          need(issued.response.status === 200 && typeof issued.value.access_token === 'string'
            && !issued.value.id_token && !issued.value.refresh_token, 'INDEPENDENT_MACHINE_CLIENT_FAILED');
          tokens.push(issued.value.access_token);
        }
        need(tokens[0] !== tokens[1], 'INDEPENDENT_CLIENT_CARRIERS_EQUAL');
        return { independentClientsIssued: 2, opaqueCarriersDistinct: true, noPersonOrRefreshTokens: true };
      }
      default: throw new Error('PHASE25_PROTOCOL_COMMAND_REJECTED');
    }
  }
  return { execute, clear() { flows.clear(); sessions.clear(); }, get size() { return flows.size + sessions.size; } };
}
