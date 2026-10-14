# Dante Cloud — API 契约 (v1)

> 前后端共同遵循的接口规范。后端 Rust (Axum) 监听 **3001**,前端 Vite 代理 `/api` 与 `/oauth` 到 3001。
> 所有响应 JSON。错误统一格式:`{"code":"UNAUTHORIZED|FORBIDDEN|NOT_FOUND|BAD_REQUEST|CONFLICT|SERVER_ERROR","message":"中文错误消息"}`
> 分页请求参数:`?page=1&size=10&keyword=xxx`,响应:`{"records":[...],"total":123,"page":1,"size":10}`
> 时间一律 ISO8601 字符串,ID 一律 UUID 字符串。
> 需登录的接口带 `Authorization: Bearer <sessionJWT>`。

## 1. 认证 (登录需选择租户下的组织)

### POST /api/auth/login
Body: `{"username":"用户名、电话或 Email","password":"登录密码"}`。电话按数字和国际区号匹配，Email 忽略大小写。先校验密码，再选择租户与组织，不要求登录前填写租户编码。
200 → `{"ticket":"...","accountCount":1,"memberships":[{"userId":"...","username":"...","realName":"...","organizationId":"...","organizationName":"总公司","organizationPath":"总公司","tenantId":"...","tenantName":"Dante 云平台","departmentId":"...|null","departmentName":"...|null"}]}`
电话或邮箱可由多个账号共用：只返回密码验证通过且有可用成员身份的账号及组织；同一登录信息验证通过多个账号时，票据类型为 `credential_multi`，每个选择项携带自己的 `userId`。返回票据尚不能访问业务接口。
(organizationPath 为层级路径如 "总公司 / 华东区分公司 / 上海办事处")
401 → 错误凭证。

### POST /api/auth/context
Body: `{"organizationId":"<uuid>","userId":"<登录选择项的 uuid，可选>"}`。多账号票据按所选 `userId` 建立会话；若同一组织有多个已验证账号，必须提交 `userId`。服务端同时核对签名、账号版本、状态及该账号在所选组织的成员身份。普通票据及会话不允许切换到其他用户 ID。
认证方式(二选一):
1. 登录后立即选择: login 响应中的 `ticket`(credentialTicket JWT, typ=credential 或 credential_multi, 10分钟有效) 放 Header `Authorization: Bearer <ticket>`;
2. 已登录会话切换组织: 携带 session JWT,校验用户在目标组织有 membership(用于顶栏无缝切换组织)。
返回:
`{"accessToken":"<sessionJWT>","expiresIn":43200,"refreshToken":"psr_<随机令牌>","refreshExpiresIn":604800,"user":{...},"tenant":{...},"organization":{...}}`

### POST /oauth/token（平台会话刷新）

表单 `grant_type=refresh_token&refresh_token=<context 返回的 psr_ 令牌>`，无需客户端 ID 或密钥。返回 `access_token`、`refresh_token`、`expires_in`、`refresh_expires_in`、`token_type: "Bearer"`，刷新令牌一次性轮换，绝对有效期不会续长。旧令牌重复使用、会话撤销、密码版本变化、成员移除、租户或组织失效时拒绝刷新。平台刷新令牌与 OAuth 应用刷新令牌独立；该请求不能附带 OAuth 客户端凭据或 scope。实际期限受 `SESSION_TTL_HOURS`、`REFRESH_TOKEN_TTL_SECONDS` 与租户/组织有效期约束。

前端并发过期请求共用一次刷新，并仅重试原请求一次；刷新时保留现有组织上下文与页面数据。网络或服务临时错误保留会话以便重试，刷新凭证确已失效才返回登录。切换上下文时清除旧查询，旧响应不得写入新上下文。

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

权限码(挂在菜单 permission 上，后端按模块校验；超管绕过只适用于实际平台租户上下文):
`system:tenant` `system:organization` `system:department` `system:user` `system:role` `system:menu` `system:application` `system:oauth` `system:monitor`

### 2.1 租户 /tenants
- GET `/api/tenants/options` → 最小下拉选项 `[{id,name,code,isPlatform}]`，需拥有组织、部门、用户、角色、授权、审计或租户管理中的任一相关权限；普通租户上下文仅返回当前租户。
- GET `/api/tenants?page&size&keyword`（所有上下文均需 `system:tenant`）→ 分页 `{id,name,code,tenantType,contactName,contactPhone,contactEmail,description,province,city,district,township,village,address,longitude,latitude,status,createdAt,organizationCount,userCount,authorizedAppCount}`；经纬度未标注时均为 `null`。
- POST `/api/tenants` `{name,code,tenantType?,contactName?,contactPhone?,contactEmail?,description,province?,city?,district?,township?,village?,address?,longitude?,latitude?}`；`tenantType` 支持 `PERSONAL`（个人）、`COMPANY`（公司）、`PUBLIC_INSTITUTION`（事业单位）、`GOVERNMENT`（政府）、`OTHER`（其他），缺省为 `COMPANY`。事务内自动以租户名称建立根组织，初始化租户范围的内置超级管理员角色及基础管理授权；根组织继承租户地点和对应类型（个人映射为其他）。联系人字段为 `contactName`、`contactPhone`、`contactEmail`。新建表单要求姓名及至少一个登录标识；联系人完整时，事务内自动创建或匹配用户、根组织成员关系及「超级管理员」授权。电话优先用作新账号用户名，没有电话时使用 Email；用户名被占用时分配独立用户名。创建响应附加 `administrator:{userId,username,roleName,created,initialPassword?}`，新账号随机初始密码只返回这一次。旧客户端完全省略联系人仍可创建未分配用户的管理员角色。后续账号资料和权限由用户、角色管理维护。PUT `/api/tenants/:id` 可更新类型、联系人及地点；修改联系人不自动改动已有账号或授权，联系人字段省略保留、空字符串清除。省略类型或乡村字段时保留原值，乡村传空字符串清除；DELETE `/api/tenants/:id`(级联删组织/授权/角色)。根组织名称后续可在组织管理中独立维护。
- PUT `/api/tenants/:id/status` `{"status":"ENABLED|DISABLED"}`

### 2.2 租户授权 /tenants/:id/authorizations
- GET `/api/tenants/:id/authorizations` → `{"applications":[{"id","code","name","icon","authorized":bool,"menuCount":n,"authorizedMenuCount":n}],"menus":{"<applicationId>":[<该应用全部菜单平铺,含 authorized:bool>]}}`
- PUT `/api/tenants/:id/authorizations` Body `{"applications":[{"applicationId","authorized":true|false}],"menus":[{"menuId","authorized":true|false}]}`
  事务内:取消应用授权时同步清理该应用下菜单授权;取消菜单授权时级联清理下属组织的 organization_menus 与该租户角色的 role_menus。新增授权自动同步到根组织及其租户范围的内置超级管理员角色。

### 2.3 应用 /applications
- GET `/api/applications` → 全量列表(含 menuCount)
- POST `/api/applications` `{code,name,icon,basePath,description,sortOrder,status}`
- PUT `/api/applications/:id` ; DELETE(级联删菜单与各层级授权)

### 2.4 菜单 /menus
- GET `/api/menus/tree?applicationId=` → 全量菜单树(管理端,不受授权过滤)
- POST `/api/menus` `{applicationId,parentId,type,title,path,icon,component,permission,sortOrder,visible}`
- PUT `/api/menus/:id` ; DELETE `/api/menus/:id`(级联子菜单)

### 2.5 组织 /organizations (租户下多层级树)
- GET `/api/organizations/tree?tenantId=` → `[{id,tenantId,parentId,name,code,organizationType,contactName,contactPhone,contactEmail,sortOrder,description,province,city,district,township,village,address,longitude,latitude,status,children:[...]}]`
- POST `/api/organizations` `{tenantId,parentId,name,code,organizationType?,contactName?,contactPhone?,contactEmail?,sortOrder,description,province?,city?,district?,township?,village?,address?,longitude?,latitude?}`；`organizationType` 支持 `COMPANY`（公司）、`SUBSIDIARY`（子公司）、`BRANCH`（分公司）、`PUBLIC_INSTITUTION`（事业单位）、`GOVERNMENT`（政府）、`OTHER`（其他），缺省为 `COMPANY`。根组织初始化租户范围的内置超级管理员角色，子组织初始化内置组织管理员角色；子组织从父组织继承租户授权范围内的基础管理权限，联系人字段及创建响应与租户一致，自动建立首位管理员账号或匹配同一联系人的可用账号，并分配当前组织的成员关系和内置管理员角色。仅共用电话或邮箱的不同联系人创建独立用户，不自动合并授权。普通租户创建组织时只能复用本租户成员账号，平台上下文可复用全局同一联系人账号。更新时省略类型保留原值；乡村字段缺省为空，更新时省略保留、空字符串清除。
- PUT `/api/organizations/:id` 可维护三个联系人字段，省略保留、空字符串清除；修改联系人不自动改动已有用户、密码或管理员授权。DELETE(级联子组织+部门+成员关系+org授权+角色)
- PUT `/api/organizations/:id/status`
- 组织新增应用及菜单授权时，自动同步到该组织的内置管理员角色；撤销授权时同步清理。默认管理员角色的菜单由授权维护，不接受手工编辑。

### 2.5.1 地图资源 /maps
- GET `/api/maps/tiles/:z/:x/:y.png` → PNG 瓦片；服务端仅代理有效瓦片坐标，浏览器可缓存 24 小时。
- GET `/api/maps/search?q=` → OpenStreetMap 地点搜索结果，最多 5 项，关键词上限 120 字符。
- GET `/api/maps/reverse?lat=&lon=` → 当前坐标的 OpenStreetMap 反向地址解析结果；租户/组织表单确认位置时通过地区目录解析接口回填省市县乡村及详细地址。IP 估算位置只回填行政区划，避免把城市中心误当精确门牌。
- GET `/api/maps/ip-location` → `{success,latitude,longitude}`；只根据可信网关规范化后的访问者公网 IP 估算，不接受客户端指定的目标 IP。回环或内网 IP 返回错误。
- 前端只调用同源接口；开发环境由 Vite 代理固定外部上游，部署环境由 Rust 后端代理。瓦片每 IP 每分钟最多 300 次，搜索/反向解析/IP 定位合计最多 20 次。

### 2.6 组织授权 /organizations/:id/authorizations
- GET → 同租户授权结构,但应用/菜单仅展示 **该组织所属租户已授权** 的范围(超出的不出现),authorized 字段表示组织是否授权。
- PUT Body 同租户授权。写入校验:菜单必须属于租户已授权应用+已授权菜单。取消授权时级联清理该组织下角色的 role_menus。

### 2.7 部门 /departments (组织下树)
- GET `/api/departments/organizations` → `[{id,tenantId,parentId,name,code,status,children:[...]}]`；要求 `system:department` 权限，返回当前登录组织及其全部下级组织，以当前组织为根（`parentId:null`）。平台用户和超级管理员也按当前会话组织限定选择范围；客户端无需传入租户或组织参数。该接口用于部门页面选择管理组织，不切换登录会话，也不要求 `system:organization` 权限。
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
- GET `/api/platform/branding` → `{platformName,platformDescription,logoUrl,loginTitle,loginDescription,footerText}`（无需认证，只公开展示配置）。
- GET / PUT `/api/platform/settings`：要求平台管理上下文和 `system:settings` 权限；读取额外返回 `platformTenantName,platformTenantCode`，完整保存七个可写字段，编码固定为 `admin-tenant`，不接受保存编码。详见[平台设置](./platform-settings.md)。
- 401 → 错误码 UNAUTHORIZED;403 → FORBIDDEN;统一 JSON。

### 平台会话（Phase 4）

`POST /api/auth/login` 返回的 ticket 与 `POST /api/auth/context` 返回的 session 令牌分别绑定用户会话版本；session 还绑定持久会话 ID。旧版无 `sid/ver` 的 session 需重新登录。`POST /api/auth/context` 切换组织时撤销旧 session，且只能选直接加入、状态有效的组织。`POST /api/auth/logout` 撤销当前 session；`PUT /api/auth/password` 和管理员重置密码撤销目标用户全部平台 session。管理员 `/api/oauth/tokens/kick` 还会撤销全部平台 session，响应增加 `platformSessionsRevoked`。

`GET /api/auth/my-sessions` 在原有 OAuth `sessions`、`revokedTotal` 外增加 `platformSessions:[{id,tenantId,tenantName,organizationId,organizationName,issuedAt,expiresAt,revokedAt,ip,userAgent,active,isCurrent}]`。`DELETE /api/auth/sessions/:id` 只能撤销本人指定的设备会话。

### 应用专用令牌（Phase 5）

`POST /api/oauth/session-token`：以平台 session Bearer 认证，请求 `{"appCode":"your-app"}`。租户和组织均获应用授权、应用及关联机密客户端启用时返回 `201 {"accessToken":"...","tokenType":"Bearer","expiresIn":<=300,"audience":"your-app"}`；否则返回 404。JWT 含 `sub/tenant/org/cid/aud/jti/iat/exp`，同秒重签也有不同 `jti`。

业务后端用关联应用的机密客户端凭据调用 `POST /oauth/introspect`，只接受 `active:true` 且 `aud` 等于本应用 code 的 access token。应用令牌的活跃性还依赖当前平台 session、直接成员关系、租户/组织状态与两级应用授权。其他应用客户端内省返回 `active:false`；过期、撤销和 refresh token 也返回 `active:false`。详情见[应用令牌接入契约](./application-token-contract.md)。

### 应用授权上下文（Phase 6）

`GET /api/apps/{appCode}/context` 仅接受受众等于路径 `appCode` 的有效应用专用令牌。响应提供 `contractVersion`、稳定的 `contextVersion`、`expiresAt`、当前用户/租户/组织/应用、有效角色、菜单、操作权限码和独立的 `dataGrants`。角色不决定数据范围；数据授权绑定当前组织的直接成员关系，按应用、资源和操作显式配置，未授权默认拒绝。业务 API 需同时验证操作权限和数据授权。字段和范围语义见[应用授权上下文契约](./app-authorization-context.md)。

### 对外用户视图（Phase 7）

`GET /api/external/menus` 和 `GET /api/external/roles` 保留旧路径，但仅接受应用专用令牌，分别返回当前用户菜单和当前组织角色。`client_credentials` 令牌访问返回 403；原全量配置视图不再由这两个接口提供。业务后端参考校验组件、旧客户端迁移顺序见[对外 API 迁移说明](./external-api-migration.md)。
# 地区目录

- `GET /api/regions/versions`：列出地区数据版本及记录数。
- `POST /api/regions/versions`：创建版本，参数 `version`、`label`、可选 `sourceVersion`；有来源版本时复制其地区数据。
- `GET /api/regions?version=2023&parentCode=-1&page=1&size=50`：默认加载国家级；选择国家后以 `parentCode=0` 加载省级，之后按父级编码分页加载。可传 `search` 搜索编码、名称或拼音。
- `GET /api/regions/options?version=&parentCode=`：租户和组织表单的五级地区选项，返回 `{version,records}`，不分页。未传 `version` 时使用尚未删除且有数据的最新版本；未传 `parentCode` 时直接返回省级（第 2 级），之后按父级编码返回下一层级的全部直属记录，最高第 6 级。东莞、中山等没有县级记录的地级市直接返回其第 5 级乡镇街道，选项按记录层级放入对应字段。指定版本不存在或无数据时返回 404。
- `GET /api/regions/resolve?version=&province=&city=&district=&township=&village=`：将地图或已有地址中的五级地区名称映射为地区目录的标准路径，返回 `{version,province,city,district,township,village}`，各地区为完整记录或 `null`。使用相同的版本选择规则，优先精确名称，再匹配明确的行政后缀及村委会、社区简称；只在已确定的父级内匹配，重名或未识别的下级返回 `null`。县级市名称可恢复其真实地级市父级；直辖市和省直辖县级行政区划保留目录中的第 3 级分组。无县级记录的地级市保留 `district:null`，直接解析下属乡镇街道；第 5、6 级仅查询已确定父级下的记录。
- `DELETE /api/regions/versions/{version}`：删除版本及其全部地区记录，保留内部删除标记，避免内置 2023 版在重启后重新导入。
- `POST /api/regions`、`PUT /api/regions/{id}`、`DELETE /api/regions/{id}?version=2023`：维护单条地区记录。写操作要求平台上下文和 `system:region` 权限；读取要求已登录。
- 地区记录的 `id` 是源文件主键或系统生成的 32 位标识，不按通用 UUID 格式解析；`code` 在同一版本内唯一。


### 既有租户和组织：初始化首位管理员

完善并保存联系人后，可在租户列表或组织详情点击“初始化首位管理员”。修改联系人本身仍不会创建人员。

- `POST /api/tenants/{id}/initialize-administrator`：平台上下文，需 `system:tenant`、`system:user`、`system:role`，使用租户联系人和唯一根组织。
- `POST /api/organizations/{id}/initialize-administrator`：需 `system:organization`、`system:user`、`system:role`，沿用组织租户隔离，使用组织联系人；根组织和子组织均授予“超级管理员”角色，权限分别限制在对应租户、组织授权范围内。
- 响应包含 `administrator`。新账号返回一次 `initialPassword`；复用账号保留原密码。已有已初始化管理员时返回 `alreadyInitialized: true`，不重复创建人员或修改授权。
- 已有单独维护的管理员、不可用管理员角色、联系人缺失或租户没有唯一根组织时，返回可操作的错误提示。旧实体缺少内置角色时补建角色，仅同步现有组织授权范围，不扩大租户或组织授权。
- 初始化在事务内锁定组织，租户入口、根组织入口和并发重复请求共用相同幂等检查。更换联系人后再执行也不会替换已初始化管理员。


成员关系维护保留仍存在的 membership ID 与数据授权；修改部门、职位、增加其他组织不撤销有效会话。移除组织成员时只撤销该组织上下文的会话，不能通过保留页面绕过撤销。


设备与会话管理的 `GET /api/oauth/tokens/sessions` 在原 `sessions` OAuth 聚合数组之外返回 `platformSessions` 登录设备数组，含用户、租户、组织、IP、User-Agent、时间、有效状态和当前设备标识。全局列表仍需平台上下文及 `system:oauth`；本人设备通过 `/api/auth/my-sessions` 按用户过滤。页面默认会话视图同时展示后台登录设备与第三方会话，设备码页仍用于 RFC 8628 授权。


平台品牌与设置接口新增 `primaryColor`（六位十六进制 `#RRGGBB`）。`GET /api/platform/branding` 对公开页面返回生效颜色，`GET/PUT /api/platform/settings` 仍需平台上下文及 `system:settings`。PUT 省略该字段保留原值，非法颜色返回400；支持海青、远山蓝、鸢尾紫、琥珀、玫瑰、松绿六组快捷预设与自定义颜色，保存后更新主要按钮、菜单选中态、图标和侧栏主题，保持用户选择的明暗模式。
