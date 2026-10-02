# Dante Cloud — API 契约 (v1)

> 前后端共同遵循的接口规范。后端 Rust (Axum) 监听 **3001**,前端 Vite 代理 `/api` 与 `/oauth` 到 3001。
> 所有响应 JSON。错误统一格式:`{"code":"UNAUTHORIZED|FORBIDDEN|NOT_FOUND|BAD_REQUEST|CONFLICT|SERVER_ERROR","message":"中文错误消息"}`
> 分页请求参数:`?page=1&size=10&keyword=xxx`,响应:`{"records":[...],"total":123,"page":1,"size":10}`
> 时间一律 ISO8601 字符串,ID 一律 UUID 字符串。
> 需登录的接口带 `Authorization: Bearer <sessionJWT>`。

## 1. 认证 (登录需选择租户下的组织)

### POST /api/auth/login
Body: `{"username":"admin","password":"admin123"}`
200 → `{"memberships":[{"organizationId":"...","organizationName":"总公司","organizationPath":"总公司","tenantId":"...","tenantName":"Dante 云平台","departmentId":"...|null","departmentName":"...|null"}]}`
(organizationPath 为层级路径如 "总公司 / 华东区分公司 / 上海办事处")
401 → 错误凭证。

### POST /api/auth/context
Body: `{"organizationId":"<uuid>"}`
认证方式(二选一):
1. 登录后立即选择: login 响应中的 `ticket`(credentialTicket JWT, typ=credential, 10分钟有效) 放 Header `Authorization: Bearer <ticket>`;
2. 已登录会话切换组织: 携带 session JWT,校验用户在目标组织有 membership(用于顶栏无缝切换组织)。
返回:
`{"accessToken":"<sessionJWT>","expiresIn":43200,"user":{...},"tenant":{...},"organization":{...}}`

### GET /api/auth/profile (Bearer session)
→ `{"user":{"id","username","realName","email","phone","avatar","gender","isSuperAdmin"},"tenant":{"id","name","code"},"organization":{"id","name","path"},"applications":[{"id","code","name","icon","basePath"}],"memberships":[{"organizationId","organizationName","organizationPath","tenantId","tenantName"}]}`
applications = 租户已授权 ∩ 组织已授权 的启用应用,按 sortOrder 排序。memberships 用于顶栏切换组织。

### GET /api/auth/menus?applicationId= (Bearer session)
→ `{"menus":[TreeNode]}` 当前用户在当前组织上下文中、该应用下可见的菜单树(角色∩组织∩租户授权)。
TreeNode: `{"id","parentId","type":"DIR|MENU|BUTTON","title","path","icon","permission","sortOrder","visible","children":[TreeNode]}`

### GET /api/auth/permissions (Bearer session)
→ `{"permissions":["system:tenant",...], "roles":[{"id","name","code"}]}`

### POST /api/auth/logout → `{"ok":true}` (前端清除 token;后端记录审计)

### PUT /api/auth/password Body `{"oldPassword","newPassword"}` → `{"ok":true}`

## 2. 平台管理模块 (需相应权限码)

权限码(挂在菜单 permission 上,后端按模块校验,超管 bypass):
`system:tenant` `system:organization` `system:department` `system:user` `system:role` `system:menu` `system:application` `system:oauth` `system:monitor`

### 2.1 租户 /tenants
- GET `/api/tenants?page&size&keyword` → 分页 `{id,name,code,description,status,createdAt,organizationCount,userCount,authorizedAppCount}`
- POST `/api/tenants` `{name,code,description}` ; PUT `/api/tenants/:id` ; DELETE `/api/tenants/:id`(级联删组织/授权/角色)
- PUT `/api/tenants/:id/status` `{"status":"ENABLED|DISABLED"}`

### 2.2 租户授权 /tenants/:id/authorizations
- GET `/api/tenants/:id/authorizations` → `{"applications":[{"id","code","name","icon","authorized":bool,"menuCount":n,"authorizedMenuCount":n}],"menus":{"<applicationId>":[<该应用全部菜单平铺,含 authorized:bool>]}}`
- PUT `/api/tenants/:id/authorizations` Body `{"applications":[{"applicationId","authorized":true|false}],"menus":[{"menuId","authorized":true|false}]}`
  事务内:取消应用授权时同步清理该应用下菜单授权;取消菜单授权时级联清理下属组织的 organization_menus 与该租户角色的 role_menus。

### 2.3 应用 /applications
- GET `/api/applications` → 全量列表(含 menuCount)
- POST `/api/applications` `{code,name,icon,basePath,description,sortOrder,status}`
- PUT `/api/applications/:id` ; DELETE(级联删菜单与各层级授权)

### 2.4 菜单 /menus
- GET `/api/menus/tree?applicationId=` → 全量菜单树(管理端,不受授权过滤)
- POST `/api/menus` `{applicationId,parentId,type,title,path,icon,component,permission,sortOrder,visible}`
- PUT `/api/menus/:id` ; DELETE `/api/menus/:id`(级联子菜单)

### 2.5 组织 /organizations (租户下多层级树)
- GET `/api/organizations/tree?tenantId=` → `[{id,tenantId,parentId,name,code,sortOrder,description,status,children:[...]}]`
- POST `/api/organizations` `{tenantId,parentId,name,code,sortOrder,description}`
- PUT `/api/organizations/:id` ; DELETE(级联子组织+部门+成员关系+org授权+角色)
- PUT `/api/organizations/:id/status`

### 2.6 组织授权 /organizations/:id/authorizations
- GET → 同租户授权结构,但应用/菜单仅展示 **该组织所属租户已授权** 的范围(超出的不出现),authorized 字段表示组织是否授权。
- PUT Body 同租户授权。写入校验:菜单必须属于租户已授权应用+已授权菜单。取消授权时级联清理该组织下角色的 role_menus。

### 2.7 部门 /departments (组织下树)
- GET `/api/departments/tree?organizationId=` → 树
- POST `{organizationId,parentId,name,code,sortOrder,description}` ; PUT `/api/departments/:id` ; DELETE

### 2.8 用户 /users
- GET `/api/users?page&size&keyword&organizationId&tenantId` → `{id,username,realName,email,phone,gender,status,lastLoginAt,createdAt,memberships:[{organizationId,organizationName,departmentId,departmentName}],roleCount}`
- POST `/api/users` `{username,password,realName,email,phone,gender}` (普通租户管理员创建后直接加入当前会话组织；平台管理员创建时默认无 membership)
- PUT `/api/users/:id` `{realName,email,phone,gender}` ; DELETE
- PUT `/api/users/:id/status` ; POST `/api/users/:id/reset-password` `{"newPassword"}`
- GET `/api/users/:id/memberships` → `[{organizationId,tenantId,departmentId,position}]`
- PUT `/api/users/:id/memberships` `{"items":[{"organizationId","departmentId","position"}]}`
- GET `/api/users/:id/roles` → `[{roleId,organizationId}]`
- PUT `/api/users/:id/roles` `{"items":[{"roleId","organizationId"}]}` (角色所属组织需在用户 membership 组织或其祖先内)

**管理 API 作用域（Phase 3）**：普通租户的用户列表、成员详情、角色详情和角色数量只包含当前租户；修改成员或角色的全量替换请求也只替换当前租户记录。普通租户管理员不能修改跨租户共享账号的全局资料、状态、密码或删除该账号；平台管理员可以执行跨租户操作。另一个租户的组织、部门、角色或用户 ID 与不存在的 ID 均返回 `404 NOT_FOUND`，以免暴露资源归属。平台跨租户修改会按目标租户写入审计记录。

`/api/applications` 和 `/api/menus` 是全局目录管理接口，要求平台上下文及相应权限码。`/api/search` 返回的用户组织名按当前租户过滤，跨租户身份不会带出另一租户的组织名。

### 2.9 角色 /roles (基于授权菜单建立)
- GET `/api/roles?page&size&keyword&tenantId&organizationId` → `{id,name,code,description,organizationId,organizationName,tenantId,status,builtIn,menuCount,userCount,createdAt}`
- POST `/api/roles` `{name,code,description,organizationId}` (角色 scope=某组织;organizationId 必填)
- PUT `/api/roles/:id` ; DELETE (built_in 角色禁删)
- GET `/api/roles/:id/menus` → `{"menuIds":[...]}`
- PUT `/api/roles/:id/menus` `{"menuIds":[...]}` — 校验:只能分配 **角色所属组织已授权** 的菜单(且在租户授权内)。
- GET `/api/roles/:id/users` → `[{userId,username,realName}]`
- PUT `/api/roles/:id/users` `{"userIds":[...]}`

### 2.10 OAuth 客户端 /oauth/clients (管理端,权限 system:oauth)
- GET `/api/oauth/clients` → `[{id,clientId,clientName,clientType,redirectUris,grantTypes,scopes,accessTokenValidity,refreshTokenValidity,status,createdAt}]` (不返回 secret)
- POST `/api/oauth/clients` `{clientName,clientType:"CONFIDENTIAL|PUBLIC",redirectUris,grantTypes,scopes,accessTokenValidity,refreshTokenValidity,description}` → 响应含一次性明文 `clientSecret`
- PUT `/api/oauth/clients/:id` ; DELETE
- POST `/api/oauth/clients/:id/regenerate-secret` → `{clientSecret}` (一次性)
- GET `/api/oauth/tokens?resourceOwner=` → `[{id,tokenType,clientId,username,scope,createdAt,expiresAt,revoked}]`
- DELETE `/api/oauth/tokens/:id` → 吊销
- GET `/api/oauth/device-codes` → `[{id,userCode,clientId,clientName,scope,status,username,createdAt,expiresAt}]`
- PUT `/api/oauth/device-codes/:id/revoke`

### 2.11 审计 /audit-logs
- GET `/api/audit-logs?page&size&keyword&action=` → `[{id,username,tenantName,organizationName,action,detail,ip,createdAt}]`
  action 枚举:LOGIN / LOGIN_FAILED / CONTEXT_SWITCH / LOGOUT / CREATE_x / UPDATE_x / DELETE_x / TENANT_AUTH / ORG_AUTH / ROLE_MENUS / CONSENT_APPROVE / CONSENT_DENY / DEVICE_APPROVE / DEVICE_DENY / TOKEN_ISSUED / TOKEN_REVOKED

### 2.12 监控 /monitor
- GET `/api/monitor/overview` → `{"tenantCount","organizationCount","userCount","roleCount","applicationCount","menuCount","oauthClientCount","activeTokenCount","todayLoginCount","recentLogins":[{username,organizationName,ip,createdAt}],"loginTrend7d":[{"date":"09-28","count":n}],"auditTrend7d":[...]}`

`/api/monitor/overview` 和 `/api/monitor/system` 为平台级接口；普通租户可使用 `/api/monitor/tenant-insights` 及其 CSV 导出，仅返回当前租户的数据。审计列表和 CSV 导出同样由服务端限定当前租户。

## 3. 标准 OAuth2 对外服务 (前缀 /oauth,form-encoded,RFC 兼容)

发现文档: GET `/oauth/.well-known/openid-configuration` →
```json
{"issuer":"","authorization_endpoint":"/oauth/authorize","token_endpoint":"/oauth/token",
"device_authorization_endpoint":"/oauth/device/authorization","userinfo_endpoint":"/oauth/userinfo",
"introspection_endpoint":"/oauth/introspect","revocation_endpoint":"/oauth/revoke",
"response_types_supported":["code"],"grant_types_supported":["authorization_code","refresh_token","client_credentials","password","urn:ietf:params:oauth:grant-type:device_code"],
"subject_types_supported":["public"],"code_challenge_methods_supported":["S256","plain"]}
```

### 3.1 授权码 + PKCE
- GET `/oauth/authorize?client_id&redirect_uri&response_type=code&scope&state&code_challenge&code_challenge_method`
  校验 client/redirect_uri/response_type → 生成 `auth_request`(存表,10分钟) → **302 跳转到前端页 `/consent?request_id=<uuid>`** (PUBLIC_APP_URL 为空则相对路径)。
- GET `/api/oauth/consent/:requestId` (Bearer session) → `{client:{clientId,clientName,description},scope:"profile email",redirectUri,state,expiresAt}` — 已过期/已消费→404。
- POST `/api/oauth/consent/:requestId/decision` `{"approve":true}` → `{"redirectTo":"https://client/cb?code=xxx&state=yyy"}` ; deny → `{"redirectTo":"...?error=access_denied&state=yyy"}`
  code 存哈希,60s 有效,绑定 user/tenant/org/client/redirect/scope/PKCE。
- POST `/oauth/token` (client_secret_basic 或 client_secret_post):
  - grant_type=authorization_code + code + redirect_uri (+ code_verifier): 校验 PKCE,签发 access_token(JWT, 含 sub/tenant/org/scope/cid,存哈希) + refresh_token(opaque,存哈希)。
  - grant_type=refresh_token → 轮换签发新对。
  - grant_type=client_credentials → 仅 client 主体 token。
  - grant_type=password + username+password+scope → 资源密码模式(选默认第一个 membership 作为上下文)。
  - grant_type=urn:ietf:params:oauth:grant-type:device_code + device_code → PENDING→`{"error":"authorization_pending"}` / DENIED→`access_denied` / 过期→`expired_token` / interval 内重复轮询→`slow_down`。
- GET `/oauth/userinfo` (Bearer access) → `{"sub","preferred_username","real_name","email","phone","tenant":{"id","name"},"organization":{"id","name"},"roles":[],"scope":""}`
- POST `/oauth/introspect` (client 认证) → RFC7662 JSON
- POST `/oauth/revoke` → RFC7009

### 3.2 设备码 (RFC 8628, 支持输入设备码的设备)
- POST `/oauth/device/authorization` (client_id [+ secret for confidential], scope) →
  `{"device_code":"...","user_code":"HTBK-MNPQ","verification_uri":"/device","verification_uri_complete":"/device?user_code=HTBK-MNPQ","expires_in":600,"interval":5}`
  user_code 8 位形如 `XXXX-XXXX`(去除易混字符 0O1I)。
- 前端 `/device` 页(需登录): GET `/api/device/:userCode` → `{clientId,clientName,scope,status:"PENDING|APPROVED|DENIED|EXPIRED",expiresAt}` ; POST `/api/device/:userCode/decision` `{"approve":true}` → `{"ok":true}` 审计 DEVICE_APPROVE/DENY。批准后绑定 user/tenant/org,设备端轮询即得 token。

## 4. 公共
- GET `/api/health` → `{"status":"UP","version":"1.0.0","database":"UP"}` (无需认证)
- 401 → 错误码 UNAUTHORIZED;403 → FORBIDDEN;统一 JSON。

### 平台会话（Phase 4）

`POST /api/auth/login` 返回的 ticket 与 `POST /api/auth/context` 返回的 session 令牌分别绑定用户会话版本；session 还绑定持久会话 ID。旧版无 `sid/ver` 的 session 需重新登录。`POST /api/auth/context` 切换组织时撤销旧 session，且只能选直接加入、状态有效的组织。`POST /api/auth/logout` 撤销当前 session；`PUT /api/auth/password` 和管理员重置密码撤销目标用户全部平台 session。管理员 `/api/oauth/tokens/kick` 还会撤销全部平台 session，响应增加 `platformSessionsRevoked`。

`GET /api/auth/my-sessions` 在原有 OAuth `sessions`、`revokedTotal` 外增加 `platformSessions:[{id,tenantId,tenantName,organizationId,organizationName,issuedAt,expiresAt,revokedAt,ip,userAgent,active,isCurrent}]`。`DELETE /api/auth/sessions/:id` 只能撤销本人指定的设备会话。

### 应用专用令牌（Phase 5）

`POST /api/oauth/session-token`：以平台 session Bearer 认证，请求 `{"appCode":"your-app"}`。租户和组织均获应用授权、应用及关联机密客户端启用时返回 `201 {"accessToken":"...","tokenType":"Bearer","expiresIn":<=300,"audience":"your-app"}`；否则返回 404。JWT 含 `sub/tenant/org/cid/aud/jti/iat/exp`，同秒重签也有不同 `jti`。

业务后端用关联应用的机密客户端凭据调用 `POST /oauth/introspect`，只接受 `active:true` 且 `aud` 等于本应用 code 的 access token。应用令牌的活跃性还依赖当前平台 session、直接成员关系、租户/组织状态与两级应用授权。其他应用客户端内省返回 `active:false`；过期、撤销和 refresh token 也返回 `active:false`。详情见[应用令牌接入契约](./application-token-contract.md)。

### 应用授权上下文（Phase 6）

`GET /api/apps/{appCode}/context` 仅接受受众等于路径 `appCode` 的有效应用专用令牌。响应提供 `contractVersion`、稳定的 `contextVersion`、`expiresAt`、当前用户/租户/组织/应用、有效角色、菜单、操作权限码和独立的 `dataGrants`。角色不决定数据范围；数据授权绑定当前组织的直接成员关系，按应用、资源和操作显式配置，未授权默认拒绝。业务 API 需同时验证操作权限和数据授权。字段和范围语义见[应用授权上下文契约](./app-authorization-context.md)。
