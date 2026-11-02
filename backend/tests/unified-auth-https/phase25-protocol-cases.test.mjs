import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createProtocolCases, mutateAuthorization } from './phase25-protocol-cases.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const source = path.resolve(here, '../../../..', 'saas-examples/oidc-rp');
const require = createRequire(path.join(source, 'package.json'));
const client = await import(pathToFileURL(require.resolve('openid-client')));
const jose = await import(pathToFileURL(require.resolve('jose')));
const runtime = await import(pathToFileURL(path.join(source, 'src/oidc.mjs')));
const { secureFetch } = await import(pathToFileURL(path.join(source, 'src/transport.mjs')));
const { fixture, tls } = await import(pathToFileURL(path.join(source, 'tests/support.mjs')));

async function withDriver(action, { fetchPort = outbound => outbound } = {}) {
  const issuer = await fixture();
  const outbound = secureFetch({ origin: issuer.config.issuer, ca: tls.cert, timeoutSeconds: 3 });
  const configurations = { third: issuer.config, offline: { ...issuer.config, clientId: 'different-client' },
    machine: { ...issuer.config, scope: 'demo.read' }, resource: { ...issuer.config, scope: 'demo.read' } };
  const oidc = Object.fromEntries(Object.entries(configurations).map(([name, config]) => [name, {
    ...issuer.oidc,
    configuration: new client.Configuration(issuer.oidc.metadata, config.clientId,
      { client_secret: config.clientSecret, id_token_signed_response_alg: 'RS256' }, client.ClientSecretBasic(config.clientSecret)),
  }]));
  const driver = createProtocolCases({ runtime, client, jose, configurations, oidc, fetch: fetchPort(outbound),
    concurrent: async () => ({ processesDistinct: true, statuses: [200, 400], errors: [null, 'invalid_grant'] }) });
  async function begin() {
    const windowStart = Date.now(), started = await driver.execute({ command: 'begin', profile: 'third' });
    const response = await issuer.request(started.authorizationUrl);
    assert.equal(response.status, 302);
    return { key: started.key, callback: response.headers.location, authWindow: [windowStart, Date.now()] };
  }
  try { await action({ driver, issuer, begin }); }
  finally { driver.clear(); assert.equal(driver.size, 0); await issuer.close(); }
}

test('real trusted HTTPS SDK exchange checks claims, primary auth_time window, signature and wrong issuer/audience; code replay is denied', async () => {
  await withDriver(async ({ driver, begin }) => {
    const result = await driver.execute({ command: 'finish', ...await begin() });
    for (const field of ['issuerExact', 'audienceExact', 'opaqueSubject', 'nonceExact', 'authTimeWithinActualPrimaryWindow',
      'idTokenUserInfoSubjectExact', 'signedTokenWrongExpectedIssuerRejected', 'signedTokenWrongExpectedAudienceRejected']) assert.equal(result[field], true);
    assert(!JSON.stringify(result).includes('fixture-only-secret'));
    assert(!JSON.stringify(result).includes('opaque-access-'));
    assert((await driver.execute({ command: 'code-replay', key: result.key })).rejected);
  });
});

test('actual SDK rejects wrong state and issuer before token exchange, then wrong nonce, verifier, client and callback', async () => {
  await withDriver(async ({ driver, issuer, begin }) => {
    for (const mode of ['state', 'issuer', 'nonce', 'verifier', 'client', 'redirect']) {
      const before = issuer.controls.tokenCount;
      const rejected = await driver.execute({ command: 'finish', mode, ...await begin() });
      assert.equal(rejected.rejected, true);
      if (['state', 'issuer'].includes(mode)) assert.equal(issuer.controls.tokenCount, before);
      if (['verifier', 'redirect'].includes(mode)) assert.equal(rejected.upstreamStatus, 400);
      if (mode === 'client') assert.equal(rejected.upstreamStatus, 401);
    }
  });
});

test('actual SDK rejects stale auth_time and driver rejects an incorrect fresh-primary observation window', async () => {
  await withDriver(async ({ driver, issuer, begin }) => {
    issuer.controls.claims.auth_time = Math.floor(Date.now() / 1000) - 120;
    const rejected = await driver.execute({ command: 'finish', mode: 'stale-auth-time', ...await begin() });
    assert.equal(rejected.sdkCode, 'OAUTH_JWT_TIMESTAMP_CHECK_FAILED');
    await assert.rejects(driver.execute({ command: 'finish', ...await begin() }), /PHASE25_PROTOCOL_AUTH_TIME_WINDOW_MISMATCH/);
  });
});

test('actual refresh rotates without subject or scope growth, and reuse is recognized as a server denial', async () => {
  await withDriver(async ({ driver, begin }) => {
    const session = await driver.execute({ command: 'finish', ...await begin() });
    assert.deepEqual(await driver.execute({ command: 'refresh', key: session.key }), {
      rotated: true, subjectAndSidPreserved: true, scopeNotExpanded: true,
    });
    assert.equal((await driver.execute({ command: 'refresh-replay', key: session.key })).rejected, true);
    // This fixture deliberately does not revoke a whole family on reuse. The
    // harness must fail the post-reuse family check instead of inventing PASS.
    await assert.rejects(driver.execute({ command: 'refresh-denied', key: session.key }), /PHASE25_PROTOCOL_NEGATIVE_ACCEPTED/);
  });
});

test('wrong confidential secret is actually denied and missing scope-shrink support cannot be counted as success', async () => {
  await withDriver(async ({ driver, begin }) => {
    assert.deepEqual(await driver.execute({ command: 'wrong-secret' }), { httpStatus: 401, invalidClient: true });
    const session = await driver.execute({ command: 'finish', ...await begin() });
    await assert.rejects(driver.execute({ command: 'refresh-scope', key: session.key }), /PHASE25_PROTOCOL_REFRESH_SCOPE_SHRINK_FAILED/);
  });
});

function scopeFixturePort(requested, { acceptUnauthorized = false, mutateReply = value => value } = {}) {
  return outbound => async (url, options) => {
    const body = options?.body instanceof URLSearchParams ? options.body : undefined;
    if (body?.get('grant_type') !== 'refresh_token') return outbound(url, options);
    const scope = body.get('scope');
    requested.push(scope);
    if (scope === 'openid offline_access' && !acceptUnauthorized) {
      return new Response(JSON.stringify({ error: 'invalid_grant' }), { status: 400 });
    }
    const response = await outbound(url, options);
    assert.equal(response.status, 200);
    // Explicit test port: the shared TLS issuer deliberately has no requested
    // scope implementation. Only this controlled component test supplies it.
    const value = await response.json();
    return new Response(JSON.stringify(mutateReply({ ...value, scope }, scope)), { status: 200 });
  };
}

test('controlled refresh scope port allows original grant scopes after a narrower access response and denies an unauthorized scope', async () => {
  const requested = [];
  await withDriver(async ({ driver, begin }) => {
    const session = await driver.execute({ command: 'finish', ...await begin() });
    assert.deepEqual(await driver.execute({ command: 'refresh-scope', key: session.key }), {
      shrunkToOpenid: true, originalGrantScopeReselectionAllowed: true, scopeOutsideOriginalGrantDenied: true,
      unauthorizedScope: 'offline_access', rejection: 'invalid_grant', rotated: true,
    });
    assert.deepEqual(requested, ['openid', 'openid profile', 'openid offline_access']);
  }, { fetchPort: scopeFixturePort(requested) });
});

test('controlled refresh scope port accepting a scope outside the original grant fails the driver', async () => {
  const requested = [];
  await withDriver(async ({ driver, begin }) => {
    const session = await driver.execute({ command: 'finish', ...await begin() });
    await assert.rejects(driver.execute({ command: 'refresh-scope', key: session.key }), /PHASE25_PROTOCOL_REFRESH_SCOPE_EXPANSION_ACCEPTED/);
    assert.deepEqual(requested, ['openid', 'openid profile', 'openid offline_access']);
  }, { fetchPort: scopeFixturePort(requested, { acceptUnauthorized: true }) });
});

test('controlled refresh scope port cannot fake a restored set with duplicate names or empty token carriers', async () => {
  for (const [scope, overrides, expected] of [
    ['openid profile', { scope: 'openid openid' }, /PHASE25_PROTOCOL_REFRESH_ORIGINAL_GRANT_SCOPE_RESELECTION_FAILED/],
    ['openid profile', { access_token: '' }, /PHASE25_PROTOCOL_REFRESH_ORIGINAL_GRANT_SCOPE_RESELECTION_FAILED/],
    ['openid', { refresh_token: '' }, /PHASE25_PROTOCOL_REFRESH_SCOPE_SHRINK_FAILED/],
  ]) {
    await withDriver(async ({ driver, begin }) => {
      const session = await driver.execute({ command: 'finish', ...await begin() });
      await assert.rejects(driver.execute({ command: 'refresh-scope', key: session.key }), expected);
    }, { fetchPort: scopeFixturePort([], { mutateReply: (value, selected) => selected === scope ? { ...value, ...overrides } : value }) });
  }
});

test('authorize mutations preserve unrelated state and nonce and produce explicit protocol negatives', () => {
  const url = 'https://auth.saas.test/oauth/authorize?state=public-fixture-state&nonce=public-fixture-nonce&code_challenge=' + 'a'.repeat(43) + '&code_challenge_method=S256';
  for (const mode of ['plain', 'missing', 'short', 'long', 'syntax', 'redirect', 'scope']) {
    const mutated = new URL(mutateAuthorization(url, mode));
    assert.equal(mutated.searchParams.get('state'), 'public-fixture-state');
    assert.equal(mutated.searchParams.get('nonce'), 'public-fixture-nonce');
  }
  assert.equal(new URL(mutateAuthorization(url, 'plain')).searchParams.get('code_challenge_method'), 'plain');
  assert.equal(new URL(mutateAuthorization(url, 'missing')).searchParams.has('code_challenge'), false);
  assert.throws(() => mutateAuthorization(url, 'arbitrary'), /PHASE25_PROTOCOL_MUTATION_REJECTED/);
});
