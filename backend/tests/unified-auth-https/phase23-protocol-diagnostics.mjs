/** Fixed protocol labels only; never serialize SDK messages, causes or data. */
const COMMANDS = new Set(['begin','finish','reload','userinfo','refresh','replay','revoke','logout','machine',
  'resource','introspect','old-credential','verify-old-id-session','same-subject','carrier-window','scan-task-logs','jwks-cache']);
const STAGES = new Set(['finishLogin','tokenData','userInfo','freshKeys','math']);
const RPC_CODES = new Set(['CARRIER_DEADLINE_UNAVAILABLE','COMMAND_REJECTED','CONFIG_BINDING_CHANGED',
  'CONTROL_REJECTED','DEPENDENCIES_REJECTED','DISCOVERY_REJECTED','DRIVER_CLOSED','FLOW_CAPACITY',
  'FLOW_UNKNOWN_OR_RETIRED','FROZEN_SOURCE_REJECTED','HANDLE_UNKNOWN','ID_TOKEN_UNAVAILABLE',
  'INCOMPLETE_LINE','INPUT_REJECTED','JWKS_REJECTED','JWKS_UNAVAILABLE','JWT_REJECTED','LINE_LIMIT',
  'LOG_SCAN_BUDGET','LOG_SCAN_INPUT','MACHINE_SESSION_REQUIRED','OUTPUT_LIMIT','PRIVATE_INPUT_REJECTED',
  'PUBLIC_CONFIG_REJECTED','REFRESH_RETIRED','REFRESH_UNAVAILABLE','REQUEST_TIMEOUT','RUNTIME_REJECTED',
  'SESSION_CAPACITY','SOURCE_MISMATCH','TARGET_REJECTED','TASK_REJECTED','TOKEN_SHAPE_REJECTED',
  'TRANSPORT_CAPACITY','TRANSPORT_UNAVAILABLE','USER_SESSION_REQUIRED','SDK_REJECTED',
  'JWKS_CACHE_UNAVAILABLE','JWKS_CACHE_OBSERVATION_REJECTED']);
const SDK_CODES = new Set(['OAUTH_WWW_AUTHENTICATE_CHALLENGE','OAUTH_RESPONSE_BODY_ERROR','OAUTH_UNSUPPORTED_OPERATION',
  'OAUTH_AUTHORIZATION_RESPONSE_ERROR','OAUTH_JWT_USERINFO_EXPECTED','OAUTH_PARSE_ERROR','OAUTH_INVALID_RESPONSE',
  'OAUTH_INVALID_REQUEST','OAUTH_RESPONSE_IS_NOT_JSON','OAUTH_RESPONSE_IS_NOT_CONFORM','OAUTH_HTTP_REQUEST_FORBIDDEN',
  'OAUTH_REQUEST_PROTOCOL_FORBIDDEN','OAUTH_JWT_TIMESTAMP_CHECK_FAILED','OAUTH_JWT_CLAIM_COMPARISON_FAILED',
  'OAUTH_JSON_ATTRIBUTE_COMPARISON_FAILED','OAUTH_KEY_SELECTION_FAILED','OAUTH_MISSING_SERVER_METADATA',
  'OAUTH_INVALID_SERVER_METADATA','OAUTH_TIMEOUT','OAUTH_ABORT','ERR_INVALID_ARG_TYPE','ERR_INVALID_ARG_VALUE',
  'ERR_JWT_EXPIRED','ERR_JWT_CLAIM_VALIDATION_FAILED','ERR_JWS_SIGNATURE_VERIFICATION_FAILED',
  'ERR_JOSE_ALG_NOT_ALLOWED','ERR_JWKS_NO_MATCHING_KEY','ERR_JWK_INVALID','ERR_JWS_INVALID',
  'INVALID_TOKEN_RESPONSE','INVALID_PUBLIC_IDENTITY','INVALID_REFRESH_ROTATION','SOURCE_CHANGED','SCOPE_CHANGED']);

export function safeSdkCode(error) { return SDK_CODES.has(error?.code) ? error.code : undefined; }
export function protocolFailureFact(command, failure) {
  if (!COMMANDS.has(command) || failure?.ok !== false) return null;
  const value = { command, code: RPC_CODES.has(failure.code) ? failure.code : 'PROTOCOL_DIAGNOSTIC_REJECTED',
    status: Number.isInteger(failure.status) && failure.status >= 0 && failure.status <= 599 ? failure.status : 0 };
  if (command === 'finish') {
    if (STAGES.has(failure.stage)) value.stage = failure.stage;
    if (SDK_CODES.has(failure.sdkCode)) value.sdkCode = failure.sdkCode;
    for (const name of ['callbackCodePresent','callbackErrorPresent']) {
      if (typeof failure[name] === 'boolean') value[name] = failure[name];
    }
    if (Number.isInteger(failure.tokenResponseCount) && failure.tokenResponseCount >= 0 && failure.tokenResponseCount <= 16) {
      value.tokenResponseCount = failure.tokenResponseCount;
    }
    if (Number.isInteger(failure.lastTokenStatus) && failure.lastTokenStatus >= 0 && failure.lastTokenStatus <= 599) {
      value.lastTokenStatus = failure.lastTokenStatus;
    }
  }
  return value;
}
export function safeProtocolCause(value) {
  return value ? protocolFailureFact(value.command, { ...value, ok: false }) : null;
}
