/** 业务后端专用；clientSecret 只能来自服务端配置。 */
export class AuthorizationError extends Error {
  constructor(status, code) {
    super(code);
    this.status = status;
    this.code = code;
  }
}

function reject(status, code) {
  throw new AuthorizationError(status, code);
}

async function jsonResponse(response) {
  if (response.status === 401 || response.status === 403) reject(401, 'INVALID_APP_TOKEN');
  if (!response.ok) reject(503, 'PLATFORM_UNAVAILABLE');
  return response.json();
}

/**
 * 每个受保护请求都重新内省并获取上下文，不缓存正向结果。
 * loadRecord 必须在 SQL 中同时限制 id 和 tenant_id；返回 null 表示不可见。
 */
export async function authorizeRecord({
  token, platformUrl, clientId, clientSecret, appCode,
  permission, resource, action, recordId, loadRecord, fetchImpl = fetch,
}) {
  if (!token || !clientId || !clientSecret || !appCode) reject(401, 'MISSING_CREDENTIAL');
  const base = new URL(platformUrl);
  if (base.protocol !== 'https:' && !['127.0.0.1', 'localhost'].includes(base.hostname)) {
    reject(500, 'PLATFORM_HTTPS_REQUIRED');
  }
  const credentials = Buffer.from(`${clientId}:${clientSecret}`).toString('base64');
  const intro = await jsonResponse(await fetchImpl(new URL('/oauth/introspect', base), {
    method: 'POST',
    headers: { authorization: `Basic ${credentials}`, 'content-type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({ token }),
  }));
  if (intro.active !== true || intro.aud !== appCode || !intro.sub || !intro.tenant_id || !intro.organization_id) {
    reject(401, 'INVALID_APP_TOKEN');
  }
  const ctx = await jsonResponse(await fetchImpl(
    new URL(`/api/apps/${encodeURIComponent(appCode)}/context`, base),
    { headers: { authorization: `Bearer ${token}` } },
  ));
  if (ctx.contractVersion !== '1.0' || ctx.appCode !== appCode ||
      ctx.user?.id !== intro.sub || ctx.tenant?.id !== intro.tenant_id ||
      ctx.organization?.id !== intro.organization_id ||
      !ctx.contextVersion || Date.parse(ctx.expiresAt) <= Date.now()) {
    reject(401, 'INVALID_CONTEXT');
  }
  if (!Array.isArray(ctx.permissions) || !ctx.permissions.includes(permission)) {
    reject(403, 'OPERATION_DENIED');
  }
  const grant = Array.isArray(ctx.dataGrants)
    ? ctx.dataGrants.find((item) => item.resource === resource && item.action === action)
    : undefined;
  if (!grant || grant.tenantId !== intro.tenant_id || grant.subjectUserId !== intro.sub) {
    reject(403, 'DATA_DENIED');
  }
  const record = await loadRecord({ id: recordId, tenantId: intro.tenant_id });
  if (!record || record.tenant_id !== intro.tenant_id) reject(404, 'RESOURCE_NOT_FOUND');
  const allowed = grant.scope === 'SELF' ? record.owner_user_id === intro.sub
    : grant.scope === 'ORG' ? record.organization_id === intro.organization_id
    : grant.scope === 'ORG_SUBTREE' ? grant.organizationIds?.includes(record.organization_id) === true
    : grant.scope === 'TENANT';
  if (!allowed) reject(404, 'RESOURCE_NOT_FOUND');
  return { record, userId: intro.sub, tenantId: intro.tenant_id, organizationId: intro.organization_id };
}

/** 示例查询。业务表必须有可信的租户及资源归属字段。 */
export async function loadOrder(db, { id, tenantId }) {
  const result = await db.query(
    'SELECT id, tenant_id, organization_id, owner_user_id FROM orders WHERE id = $1 AND tenant_id = $2',
    [id, tenantId],
  );
  return result.rows[0] ?? null;
}
