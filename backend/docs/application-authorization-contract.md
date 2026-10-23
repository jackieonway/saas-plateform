# 应用授权契约

本文描述平台向 qiankun 微应用及其业务后端提供的现行授权接口。以 `src/app_authorization.rs`、`src/app_token.rs`、`src/routes/app_authorize.rs`、`src/routes/app_members.rs` 和 SDK 2.0 公共协议为准。

## 身份与调用方

平台会话、普通 OAuth 令牌、应用专用令牌具有不同用途。壳通过 `POST /api/oauth/session-token`，使用当前平台会话和 `{ "appCode": "work-orders" }` 获取短期应用令牌。该令牌绑定用户、租户、所选组织、应用、机密客户端、父平台会话和唯一 `jti`，最长 300 秒且不晚于父会话到期。qiankun 的挂载属性由 SDK 2.0 承载；令牌不应出现在入口 URL、路由查询串或审计明文中。

应用令牌每次使用都要验证数据库状态：账户和客户端启用、应用启用、租户与完整组织链有效、直接成员关系存在、租户及组织均授权该应用、父会话未撤销且未到期，以及令牌记录未撤销。仅验 JWT 签名不能证明当前授权仍有效。

## 获取当前应用上下文

```http
GET /api/apps/work-orders/context
Authorization: Bearer <application-access-token>
```

成功响应是上下文本身，字段如下。

| 字段 | 含义 |
| --- | --- |
| `contractVersion` | 当前为 `1.0`，是授权上下文契约版本，与 SDK 包版本不同 |
| `contextVersion` | 当前权限投影的 SHA-256 版本，用于检测变化，不是凭据 |
| `expiresAt` | 不晚于令牌到期与响应生成后 30 秒的较早时间 |
| `appCode`、`application` | 应用代码、ID、名称及当前环境的入口元数据 |
| `user`、`tenant`、`organization` | 当前应用身份，不从客户端提交的 ID 推导 |
| `roles` | 当前应用有效角色的最小信息 |
| `permissions` | 有效菜单和按钮导出的操作权限字符串 |
| `dataGrants` | 当前直接成员关系上独立配置的资源、动作、数据范围 |
| `organizationScopeIds` | 组织硬边界；平台租户根组织返回 `null`，其他上下文返回当前有效组织子树 |
| `menus` | 当前应用的有效菜单树 |

`application` 包含 `runtimeMode`、`entryType`、`entryUrl`、`deploymentVersion` 和 `entryVersionId`。现行独立微应用使用 `TRUSTED + HTML`。环境 URL 入口的 `deploymentVersion` 为 `null`，`entryVersionId` 是 URL revision；版本发布使用 release 的版本和 ID。入口选择规则见[环境 URL 入口](environment-url-entries.md)。

上下文在一次数据库语句快照中组装，再独立读取最新投影和令牌状态；不一致时有限重试，无法证明一致则返回 503。`contextVersion` 相同不能省略后续请求的撤销检查。客户端不能将 30 秒过期时间解释为后端 30 秒授权缓存许可。

## 业务后端授权入口

```http
POST /api/apps/work-orders/authorize
Authorization: Basic <base64(client_id:client_secret)>
Content-Type: application/x-www-form-urlencoded

token=<application-access-token>&resource=ticket&action=read
```

请求体只接受 `token`、`resource`、`action`，上限 32 KiB。调用者必须是已启用且绑定该应用的机密客户端。服务端同时检查：

1. 调用客户端的应用 ID、令牌应用 ID、路由 `appCode` 和令牌 audience 相互匹配。
2. 资源动作已在服务端 `DATA_GRANT_POLICY.applications[].capabilities` 中登记。
3. 上下文包含 `${resource}:${action}` 操作权限。
4. 上下文恰有一项匹配资源、动作、当前用户和租户的数据授权。
5. 响应前重新验证客户端密钥、启用状态及应用绑定。

成功返回 `{ "contractVersion": "1.0", "introspection": { ... }, "context": { ... } }`。`introspection` 含 `active`、`scope`、`client_id`、`token_type`、`sub`、`tenant_id`、`organization_id`、`aud`、`jti`、`exp`。业务后端仍需将返回的数据范围落实为 SQL 过滤，并验证具体记录的租户、组织和所有者。该接口不替业务后端判断某条业务记录是否属于 `SELF`。

## 数据范围

操作权限决定能否执行动作，独立数据授权决定能影响哪些记录，二者同时满足才允许访问。角色继承不会自动产生独立数据授权。

| 范围 | 业务约束 |
| --- | --- |
| `SELF` | 当前用户拥有的记录，仍与租户及组织硬边界相交 |
| `ORG` | `organizationIds` 中的当前组织 |
| `ORG_SUBTREE` | `organizationIds` 中的当前有效组织子树 |
| `TENANT` | 当前租户范围，但不能扩大所选组织的硬边界 |

除平台租户根组织外，即使数据库中配置 `TENANT`，投影也收窄为 `ORG_SUBTREE`。非平台根的 `SELF` 同样携带组织子树作为硬上限。范围名称不是可按大小排序的权限等级，不能根据某范围“更大”推导另一范围已获准。

## 受控成员查询

`GET /api/apps/{appCode}/members` 和 `GET /api/apps/{appCode}/members/{userId}` 使用应用 Bearer 令牌。必须提供 `resource`、`action`、`organizationId`，且该资源动作还要登记在同一应用的 `memberLookups` 中。普通 capability 不自动开放成员目录。

列表另接受 `page`（1–1000000，默认 1）、`size`（1–50，默认 20）和 `q`（最多 100 字）；单成员校验不接受分页或搜索参数。服务端将数据授权与组织硬边界相交，并检查目标组织、成员和两级应用授权。`SELF` 在成员选择场景仅约束可选组织，不会把被选成员变成业务记录所有者。返回前再次校验授权变化，调用方只能使用接口提供的最小成员信息。

成员目录可用于经登记的联系人选择或只读成员查看。门店使用 `store/create`、`store/update` 校验营业联系人，使用 `store-staff/read` 读取平台门店成员；这些查询不会建立或修改组织成员关系。

## 受控组织查询

平台组织管理维护 STORE 门店身份；应用通过独立用途控制的 `/api/apps/{appCode}/organizations` 与单项端点读取新鲜组织元数据，不复用平台管理树，也不扩充 MicroProps。目录证明不代替业务 SELF owner 校验。完整 DTO、3 秒绝对窗口和总部范围规则见[应用受控组织目录](application-organization-directory.md)。

## 失败处理与验证

业务 API 使用 `{ "code": "...", "message": "..." }`：401 为令牌或客户端无效，403 为操作或数据授权缺失，404 为目标不可见，400 为未登记用途或参数错误，429 附带 `Retry-After`，503 表示未能及时证明新鲜授权。业务后端在这些失败情况下应终止当前操作。

授权任务受 `APP_AUTHORIZATION_MAX_CONCURRENT`（默认 32）和 `APP_AUTHORIZATION_TIMEOUT_MS`（默认 5000）限制。令牌内省另有客户端与来源预算，见 [OAuth 签发](oauth-issuance.md)。

回归依据包括 `src/authorization_isolation_tests.rs`、`src/data_grant_tests.rs`、`src/notification_tests.rs`、`src/public_contract_tests.rs` 及 `tests/authorization_policy.rs`。数据库测试会创建和删除隔离数据库，按[开发文档](development.md)配置测试数据库后显式运行 ignored 测试。
