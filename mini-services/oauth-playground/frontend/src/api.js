const privateFields = /^(access_?token|refresh_?token|id_?token|client_?secret|code_?verifier|device_?code|password|token)$/i

/** Keep protocol/resource inspection useful without rendering credential fields. */
export function publicData(value) {
  if (Array.isArray(value)) return value.map(publicData)
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, privateFields.test(key) ? '[已隐藏]' : publicData(item)]))
  }
  return value
}

// The flow named "password" is a public capability, not a credential field.
// Decode configuration through its own allowlist instead of weakening the
// credential filter used for protocol and resource responses.
function publicConfig(value) {
  const string = key => typeof value?.[key] === 'string' ? value[key] : ''
  const flows = Object.fromEntries(['authorization_code', 'device', 'client_credentials', 'password'].map(flow => {
    const capability = value?.flows?.[flow]
    const reason = capability?.reason
    return [flow, {
      enabled: capability?.enabled === true,
      ...(['not_configured', 'legacy_password_disabled'].includes(reason) ? { reason } : {}),
    }]
  }))
  return { publicOrigin: string('publicOrigin'), platformOrigin: string('platformOrigin'), scope: string('scope'), flows }
}

export async function apiRequest(path, body, signal) {
  const timeout = AbortSignal.timeout(20000)
  const response = await fetch(path, {
    method: body === undefined ? 'GET' : 'POST',
    credentials: 'same-origin',
    cache: 'no-store',
    redirect: 'error',
    signal: signal ? AbortSignal.any([signal, timeout]) : timeout,
    ...(body === undefined ? {} : { headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }),
  })
  const data = await response.json()
  const configResponse = response.ok && path === '/api/config' && body === undefined
  return { ok: response.ok, status: response.status, data: configResponse ? publicConfig(data) : publicData(data) }
}

const explanations = {
  authorization_pending: '等待用户在平台批准设备授权。',
  slow_down: '平台要求降低频率；后续轮询间隔增加 5 秒。',
  access_denied: '用户拒绝授权，或当前身份没有访问权限。',
  expired_token: '授权已过期，请重新发起流程。',
  invalid_grant: '授权凭据已失效、已使用或不匹配，请重新发起流程。',
  invalid_state: '回调状态校验失败，请从本页重新发起授权。',
  invalid_request: '请求未通过校验，请检查流程、会话和配置。',
  invalid_token: '当前令牌不可用，或当前流程没有用户身份。',
  invalid_client: '平台拒绝客户端认证，请检查服务端私有配置。',
  unauthorized_client: '该客户端未获准使用此授权方式。',
  unsupported_grant_type: '平台不支持此授权方式。',
  insufficient_scope: '当前授权范围不足，平台已拒绝本次访问。',
  forbidden: '平台拒绝访问；普通 OAuth 令牌不具备应用绑定的资源权限。',
  not_configured: '此流程尚未配置，请由维护者完成服务端私有配置。',
  legacy_password_disabled: '资源密码模式为历史兼容演练，服务端默认禁用。',
  request_timeout: '请求超时，请检查平台连接后重试。',
  network_error: '未能连接流程 API，请检查服务与同源代理。',
  invalid_device_response: '设备授权响应不完整，轮询已停止。',
  invalid_session: '发起会话不存在或已失效，请从本页重新发起授权。',
  unsupported_flow: '服务端未启用此流程，或平台明确不支持此授权方式。',
  flow_busy: '同一流程已有操作进行，请等待完成。若上次请求结果未知，应重新发起授权，不可重用旧回调或刷新凭据。',
  upstream_unavailable: '平台暂时不可达，当前操作未能完成。请先恢复连接并核对会话状态。',
  no_token: '此会话没有可用令牌，或令牌已被移除。请重新完成所选流程。',
}

export function explainError(error) {
  return explanations[error] || '请求未完成。请查看协议响应中的错误代码并检查服务配置。'
}
