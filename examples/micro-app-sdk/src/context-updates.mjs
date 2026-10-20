const authorizationFields = Object.freeze(['sdkVersion', 'contractVersion', 'appCode', 'basePath',
  'contextEpoch', 'contextVersion', 'token', 'expiresAt', 'tokenExpiresAt', 'deploymentVersion', 'entryVersionId',
  'user', 'tenant', 'organization', 'organizationScopeIds', 'roles', 'permissions', 'dataGrants', 'menus'])
const knownFields = new Set([...authorizationFields, 'signal', 'theme', 'routePath', 'onNavigate', 'onNotify'])

function canonical(value) {
  if (value === undefined) return ['undefined']
  if (value === null) return ['null']
  if (typeof value === 'string' || typeof value === 'boolean') return [typeof value, value]
  if (typeof value === 'number' && Number.isFinite(value)) return ['number', value]
  if (Array.isArray(value)) return ['array', value.map(canonical)]
  if (Object.prototype.toString.call(value) === '[object Object]') {
    return ['object', Object.keys(value).sort().map(key => [key, canonical(value[key])])]
  }
  throw new TypeError('Unsupported context value')
}

/** Memory-only authorization identity: contains the token and must never be logged or sent as telemetry. */
export function authorizationContextSignature(props) {
  if (!props || typeof props !== 'object' || Object.keys(props).some(key => !knownFields.has(key))) return null
  try {
    return JSON.stringify(authorizationFields.map(key => [key,
      Object.hasOwn(props, key) ? canonical(props[key]) : ['missing']]))
  } catch { return null }
}

/** Signal identity, cancellation and expiry are checked separately by each live context owner. */
export function sameAuthorizationContext(previous, next) {
  const before = authorizationContextSignature(previous), after = authorizationContextSignature(next)
  return before !== null && after !== null && before === after
}
