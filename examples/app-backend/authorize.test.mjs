import assert from 'node:assert/strict';
import test from 'node:test';
import { authorizeRecord, AuthorizationError, loadOrder } from './authorize.mjs';

const context = () => ({
  contractVersion: '1.0', contextVersion: 'v1',
  expiresAt: new Date(Date.now() + 30_000).toISOString(), appCode: 'orders',
  user: { id: 'user-a' }, tenant: { id: 'tenant-a' }, organization: { id: 'org-a' },
  permissions: ['order:read'],
  dataGrants: [{ resource: 'order', action: 'read', scope: 'ORG',
    tenantId: 'tenant-a', subjectUserId: 'user-a', organizationIds: ['org-a'] }],
});
const introspection = () => ({
  active: true, aud: 'orders', sub: 'user-a',
  tenant_id: 'tenant-a', organization_id: 'org-a',
});
const record = () => ({ id: 'order-1', tenant_id: 'tenant-a', organization_id: 'org-a', owner_user_id: 'user-a' });
const response = (value) => ({ ok: true, json: async () => value });

function options({ intro = introspection(), ctx = context(), row = record() } = {}) {
  const requests = [];
  return {
    token: 'token', platformUrl: 'https://platform.example', clientId: 'orders-server',
    clientSecret: 'server-only-secret', appCode: 'orders', permission: 'order:read',
    resource: 'order', action: 'read', recordId: 'order-1',
    fetchImpl: async (url, init) => {
      requests.push({ url: url.pathname, authorization: init.headers.authorization });
      return response(url.pathname === '/oauth/introspect' ? intro : ctx);
    },
    loadRecord: async ({ id, tenantId }) => {
      assert.equal(id, 'order-1');
      assert.equal(tenantId, 'tenant-a');
      return row;
    },
    requests,
  };
}

async function denied(input, code) {
  await assert.rejects(authorizeRecord(input), (error) =>
    error instanceof AuthorizationError && error.code === code);
}

test('valid same-tenant and in-scope request succeeds with server-side introspection', async () => {
  const input = options();
  assert.equal((await authorizeRecord(input)).record.id, 'order-1');
  assert.deepEqual(input.requests.map((r) => r.url),
    ['/oauth/introspect', '/api/apps/orders/context']);
  assert.ok(input.requests[0].authorization.startsWith('Basic '));
  assert.ok(input.requests[1].authorization.startsWith('Bearer '));
});

test('revoked, expired, or wrong-audience tokens fail closed', async () => {
  await denied(options({ intro: { ...introspection(), active: false } }), 'INVALID_APP_TOKEN');
  await denied(options({ intro: { ...introspection(), aud: 'billing' } }), 'INVALID_APP_TOKEN');
  await denied(options({ ctx: { ...context(), expiresAt: new Date(Date.now() - 1).toISOString() } }),
    'INVALID_CONTEXT');
  const rejectedContext = options();
  rejectedContext.fetchImpl = async (url) => url.pathname === '/oauth/introspect'
    ? response(introspection()) : { ok: false, status: 401 };
  await denied(rejectedContext, 'INVALID_APP_TOKEN');
});

test('operation permission and data grant are independent gates', async () => {
  await denied(options({ ctx: { ...context(), permissions: [] } }), 'OPERATION_DENIED');
  await denied(options({ ctx: { ...context(), dataGrants: [] } }), 'DATA_DENIED');
  await denied(options({ row: { ...record(), organization_id: 'org-b' } }), 'RESOURCE_NOT_FOUND');
});

test('resource ID tampering cannot cross tenant or SELF ownership', async () => {
  await denied(options({ row: { ...record(), tenant_id: 'tenant-b' } }), 'RESOURCE_NOT_FOUND');
  await denied(options({ row: null }), 'RESOURCE_NOT_FOUND');
  const own = context();
  own.dataGrants[0].scope = 'SELF';
  await denied(options({ ctx: own, row: { ...record(), owner_user_id: 'user-b' } }),
    'RESOURCE_NOT_FOUND');
});

test('reference SQL binds resource ID and authenticated tenant', async () => {
  const calls = [];
  const db = { query: async (sql, args) => {
    calls.push({ sql, args });
    return { rows: [record()] };
  } };
  assert.equal((await loadOrder(db, { id: 'order-1', tenantId: 'tenant-a' })).id, 'order-1');
  assert.match(calls[0].sql, /WHERE id = \$1 AND tenant_id = \$2/);
  assert.deepEqual(calls[0].args, ['order-1', 'tenant-a']);
});
