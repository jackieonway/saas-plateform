/** Independent RP callback audience boundary, using a controlled signed SDK transport port. */
import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const workspace = path.resolve(here, '../../../..');
const source = path.join(workspace, 'saas-examples/oidc-rp');
const validation = path.resolve(here, '../../docs/designs/unified-auth/validation');
const output = path.join(validation, process.env.PHASE25_AUDIENCE_REPORT ?? 'phase-25-rp-audience-components.json');
assert(path.dirname(output) === validation && /^phase-25-rp-audience-components(?:-r[1-9][0-9]*)?\.json$/.test(path.basename(output)), 'PHASE25_AUDIENCE_SAFE_OUTPUT');
const descriptor = fs.openSync(output, 'wx', 0o600);
const require = createRequire(path.join(source, 'package.json'));
const client = await import(pathToFileURL(require.resolve('openid-client')));
const jose = await import(pathToFileURL(require.resolve('jose')));
const { fixture } = await import(pathToFileURL(path.join(source, 'tests/support.mjs')));
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const ordered = value => Array.isArray(value) ? value.map(ordered)
  : value && typeof value === 'object' ? Object.fromEntries(Object.keys(value).sort().map(key => [key, ordered(value[key])])) : value;
const relativeTest = path.relative(workspace, fileURLToPath(import.meta.url)).replaceAll('\\', '/');
const inputs = [relativeTest, 'saas-examples/oidc-rp/package.json', 'saas-examples/oidc-rp/package-lock.json',
  'saas-examples/oidc-rp/src/app.mjs', 'saas-examples/oidc-rp/src/oidc.mjs', 'saas-examples/oidc-rp/src/transport.mjs',
  'saas-examples/oidc-rp/src/config.mjs', 'saas-examples/oidc-rp/src/store.mjs', 'saas-examples/oidc-rp/tests/support.mjs'];
const sourceSha256 = Object.fromEntries(inputs.map(relative => [relative, sha(fs.readFileSync(path.join(workspace, relative)))]));
const report = { contractVersion: 'unified-auth-phase25-rp-audience-components/1', phase: 25,
  startedAtUtc: new Date().toISOString(), result: 'FAIL',
  evidenceLayer: 'STANDARD_SDK_ACTUAL_TLS_FIXTURE_CONTROLLED_SIGNED_TOKEN_AT_TRANSPORT_PORT',
  scope: 'Actual independent RP application callback and session store. The issuer token response is deliberately replaced at the standard SDK customFetch port with a same-key signed ID Token, changing only aud or only iss. This is not real identity interoperability.',
  sourceSha256, versions: { node: process.version, standardClient: 'openid-client@6.8.8', jwtLibrary: 'jose@6.2.13' },
  command: 'node --test saas-plateform/backend/tests/unified-auth-https/phase25-rp-audience.test.mjs',
  checks: [], rawCarriersPersisted: false, tlsErrorsIgnored: false,
  cleanup: { ownedFixtureGroupsOpened: 0, ownedFixtureGroupsClosed: 0, databasesCreated: 0,
    dockerCreated: 0, dockerReused: 0, actualSaasProcessesStarted: 0,
    temporaryCertificatesRemovedByFixtureTrap: true },
  limitations: [
    'The signed response substitution is explicit at the SDK transport port; it must not be reported as a real identity server returning an incorrect audience.',
    'The same fixture signs each negative and its positive control with identical claims except the single tested aud or iss field; the RP callback and session code is the delivered current source.',
    'No application process, database, issuer configuration or pre-existing service is changed.',
  ],
};

async function runAudienceCase(name, wrongAudience, wrongIssuer = false) {
  let current, outcome;
  try {
    current = await fixture(); report.cleanup.ownedFixtureGroupsOpened++;
    let substitutions = 0, cryptographicSignatureVerified = false, unchangedOtherClaims = false;
    const originalFetch = current.oidc.fetch;
    current.oidc.configuration[client.customFetch] = async (input, init) => {
      const response = await originalFetch(input, init);
      const url = new URL(input instanceof Request ? input.url : input);
      if (url.pathname !== '/token' || response.status !== 200) return response;
      const tokens = await response.json();
      if (typeof tokens.id_token !== 'string') return new Response(JSON.stringify(tokens), { status: response.status, headers: response.headers });
      const original = jose.decodeJwt(tokens.id_token), audience = wrongAudience ? 'phase25-intentionally-different-rp' : current.config.clientId;
      const issuer = wrongIssuer ? 'https://phase25-intentionally-different-issuer.invalid' : current.config.issuer;
      const expected = { ...original, aud: audience, iss: issuer };
      const signed = await current.logoutToken({ ...expected, jti: undefined, events: undefined }, { typ: 'JWT' });
      unchangedOtherClaims = JSON.stringify(ordered(jose.decodeJwt(signed))) === JSON.stringify(ordered(expected));
      assert(unchangedOtherClaims, 'PHASE25_AUDIENCE_UNRELATED_CLAIM_CHANGED');
      const jwks = await current.request(current.oidc.metadata.jwks_uri);
      assert.equal(jwks.status, 200, 'PHASE25_AUDIENCE_FIXTURE_JWKS_UNAVAILABLE');
      await jose.jwtVerify(signed, jose.createLocalJWKSet(jwks.json()), {
        issuer, audience, algorithms: ['RS256'], clockTolerance: 0,
      });
      cryptographicSignatureVerified = true; substitutions++;
      return new Response(JSON.stringify({ ...tokens, id_token: signed }), { status: response.status, headers: response.headers });
    };
    const browser = current.browser();
    await browser.call('/api/session');
    const { response } = await browser.callback();
    const session = (await browser.call('/api/session')).json();
    assert.equal(substitutions, 1, 'PHASE25_AUDIENCE_SUBSTITUTION_NOT_OBSERVED');
    assert(cryptographicSignatureVerified && unchangedOtherClaims, 'PHASE25_AUDIENCE_CONTROL_NOT_VALIDATED');
    assert.equal(current.controls.tokenCount, 1, 'PHASE25_AUDIENCE_EXPECTED_ONE_CODE_EXCHANGE');
    if (wrongAudience || wrongIssuer) {
      assert.equal(response.status, 400, 'PHASE25_AUDIENCE_WRONG_CLIENT_ACCEPTED');
      assert.equal(session.authenticated, false, 'PHASE25_AUDIENCE_WRONG_CLIENT_CREATED_SESSION');
      assert.equal(current.controls.userInfoCount, 0, 'PHASE25_AUDIENCE_REJECTION_NOT_AT_ID_TOKEN_CHECK');
    } else {
      assert.equal(response.status, 303, 'PHASE25_AUDIENCE_POSITIVE_CALLBACK_FAILED');
      assert.equal(session.authenticated, true, 'PHASE25_AUDIENCE_POSITIVE_SESSION_MISSING');
      assert.equal(current.controls.userInfoCount, 1, 'PHASE25_AUDIENCE_POSITIVE_USERINFO_MISSING');
    }
    outcome = { name, status: 'PASS', wrongAudience, wrongIssuer, cryptographicSignatureVerified, otherClaimsPreserved: true,
      changedClaim: wrongAudience ? 'aud' : wrongIssuer ? 'iss' : null,
      callbackStatus: response.status, sessionEstablished: session.authenticated,
      actualCodeExchanges: current.controls.tokenCount, userInfoRequests: current.controls.userInfoCount,
      signedResponseSubstitutions: substitutions };
    report.checks.push(outcome);
  } catch (error) {
    report.checks.push({ name, status: 'FAIL', wrongAudience, wrongIssuer,
      failureCode: /^PHASE25_AUDIENCE_[A-Z_]+$/.test(error?.message ?? '') ? error.message : 'PHASE25_AUDIENCE_COMPONENT_FAILED' });
    throw new Error('PHASE25_AUDIENCE_COMPONENT_FAILED');
  } finally {
    if (current) { await current.close(); report.cleanup.ownedFixtureGroupsClosed++; }
  }
}

test('validly signed ID Token with incorrect audience is rejected by the actual RP callback before UserInfo and leaves no session', async () => {
  await runAudienceCase('incorrect-audience-valid-signature-no-session', true);
});
test('same signer and all other claims with the expected audience establish the actual RP session', async () => {
  await runAudienceCase('expected-audience-same-other-claims-session-established', false);
});
test('validly signed ID Token with incorrect issuer is rejected by the actual RP callback before UserInfo and leaves no session', async () => {
  await runAudienceCase('incorrect-issuer-valid-signature-no-session', false, true);
});
test('same signer and all other claims with the expected issuer establish the actual RP session', async () => {
  await runAudienceCase('expected-issuer-same-other-claims-session-established', false, false);
});
after(() => {
  report.finishedAtUtc = new Date().toISOString();
  report.sourceInputsUnchangedAfter = Object.entries(sourceSha256).every(([relative, expected]) => sha(fs.readFileSync(path.join(workspace, relative))) === expected);
  report.testsRun = report.checks.length; report.passed = report.checks.filter(value => value.status === 'PASS').length;
  report.failed = report.checks.filter(value => value.status === 'FAIL').length; report.skipped = 0;
  report.cleanup.status = report.cleanup.ownedFixtureGroupsOpened === 4 && report.cleanup.ownedFixtureGroupsClosed === 4 ? 'PASS' : 'FAIL';
  report.result = report.testsRun === 4 && report.passed === 4 && report.sourceInputsUnchangedAfter && report.cleanup.status === 'PASS' ? 'PASS' : 'FAIL';
  const bytes = JSON.stringify(report, null, 2) + '\n';
  assert(!/"(?:access_token|refresh_token|id_token|nonce|password|clientSecret|cookie|verifier)"\s*:/.test(bytes), 'PHASE25_AUDIENCE_REPORT_SECRET_FIELDS');
  fs.ftruncateSync(descriptor, 0); fs.writeSync(descriptor, bytes); fs.fsyncSync(descriptor); fs.closeSync(descriptor);
  assert.equal(report.result, 'PASS', 'PHASE25_AUDIENCE_REPORT_NOT_PASS');
});
