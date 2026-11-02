# 应用授权契约

本文描述平台向 qiankun 微应用及其业务后端提供的现行授权接口。以 `src/app_capabilities.rs`、`src/app_authorization.rs`、`src/app_token.rs`、`src/routes/app_authorize.rs`、`src/routes/app_members.rs` 和 SDK 3.0.0 公共协议为准。

通知的外部 USER 服务端入口见[私有通知发布与原操作恢复合同](designs/unified-auth/phase-18-private-notification-contract.md)：独立 APP_RESOURCE Basic 服务认证，POST Form 携带原 token 和完整原通知请求；publish 与只读 result 均复验原来源、notification/publish scope、permission、独立数据授权和员工收件人。公开微应用通知入口仍只接受内部 app_session，MACHINE 与 CUSTOMER 不能发布员工提醒。

## 身份与调用方

平台会话、普通 OAuth 令牌、应用专用令牌具有不同用途。壳通过 `POST /api/apps/session-token`，使用当前平台会话和 `{ "appCode": "work-orders" }` 获取短期应用令牌。该内部 `app_session` 能力绑定用户、租户、所选组织、应用、父平台会话和唯一 `jti`，由独立摘要账本保存原始代际，最长 300 秒且不晚于父会话到期。qiankun 的挂载属性由 SDK 3.0.0 承载；令牌不应出现在入口 URL、路由查询串或审计明文中。

应用令牌每次使用都要验证数据库状态：账户和原身份来源启用、应用启用、租户与完整组织链有效、直接成员关系存在、租户及组织均授权该应用、父会话未撤销且未到期，以及令牌记录未撤销。仅验 JWT 签名不能证明当前授权仍有效。

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
Authorization: Basic <base64(resource_service_id:resource_service_secret)>
Content-Type: application/x-www-form-urlencoded

token=<application-access-token>&resource=ticket&action=read
```

该 POST 是服务间私有端口，必须从已配置且受网络访问控制的私有 HTTPS origin 调用；平台公开浏览器网关对此路径返回 404。门店／工单的 `PLATFORM_API_ORIGIN` 指向该私有入口，证书覆盖实际主机名，并由 `PLATFORM_CA_CERT_FILE` 验证信任链。私有入口只允许对应业务的准确方法和路径；浏览器的 `/api/apps/session-token`、`/context` 和内部能力目录不因此关闭。

请求体只接受 `token`、`resource`、`action`，上限 32 KiB。调用者必须来自 `platform_resource_services`，用途为 `APP_RESOURCE`，绑定目标业务应用、准确 audience 和资源动作 allowlist。其服务凭据不是 OAuth 客户端凭据。内部能力与外部 canonical opaque Access Token 由独立验证器处理，先按能力摘要账本分类；已登记但撤销的能力不会转入外部验证。服务端同时检查：

1. 服务的目标业务应用、路由 `appCode`、准确 audience 和 `platform_resource_bindings` 的 canonical resource/scope 相互匹配。OAuth 调用者应用 ID 与目标业务应用 ID 分开处理。
2. 资源动作已在服务端 `DATA_GRANT_POLICY.applications[].capabilities` 中登记。
3. 上下文包含 `${resource}:${action}` 操作权限。
4. 上下文恰有一项匹配资源、动作、当前用户和租户的数据授权。
5. 响应前重新验证服务密钥代际、启用状态、用途、资源映射及原始凭据期限。外部 token 还须两次在线验证身份来源，内部能力须新鲜验证原 BFF 与当前成员/应用。

成功返回 `contractVersion: "resource-authorization/1"`。共同字段为 `issuer`、`tokenUse`、`subjectMode`、`targetApplicationId`、`resource`、`action`、`audience` 和 `expiresAt`。

USER 返回 `principal`（内部 `userId`、`membershipId`、`memberType`、`identityVersion`、`accountVersion`、`tenantId`、`organizationId`）、`context`（仍为公开上下文 `1.0`）及最小 `introspection`。外部 token 另含 `clientId` 与 `sourceApplicationId`，其 `introspection.sub` 保持公开 opaque subject，不能解析为 UUID。内部 `app_session` 的 `introspection.client_id` 是 `app.code` 内部引用，不是 OAuth client；该字段不属于 SDK MicroProps。

MACHINE 返回独立 `delegation`，包含明确登记的源 canonical client／调用方应用、目标业务应用、租户、组织、资源动作、数据范围和代际。它没有 `principal`、`context`、`sub` 或任何虚构用户／成员。未登记唯一有效委托、错原凭据代际或范围越界均拒绝；人员写操作可由资源服务明确拒绝。业务后端仍须将 USER dataGrants 或 MACHINE delegation 落实为自己的 SQL 过滤，并验证具体记录的租户、组织和所有者。

完整私有 issuer、DTO、接口分类和失败关闭规则见 [Phase 15 资源能力合同](designs/unified-auth/phase-15-resource-capability-contract.md)。

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

独立资源后端携带 canonical opaque token 时，使用私有 POST `/internal/platform/apps/{appCode}/members` 或 `/members/{userId}`，以独立服务 Basic 认证，表单携带 `token`、`resource`、`action`、`organizationId` 和可选分页。响应为 `resource-directory/1` 包装及原目录 `data`；MACHINE 不能访问人员目录。公开 GET 的 Bearer 接口仍仅支持内部 `app_session`。私有组织目录对应 `/internal/platform/apps/{appCode}/organizations` 和 `/{organizationId}`。完整字段与检查点见[Phase 15 合同](designs/unified-auth/phase-15-resource-capability-contract.md)。目录响应只能用于受控引用，不能单独授权后续写入；业务服务必须在自己的提交前复验原 operation 和身份来源。

## 受控组织查询

平台组织管理维护 STORE 门店身份；应用通过独立用途控制的 `/api/apps/{appCode}/organizations` 与单项端点读取新鲜组织元数据，不复用平台管理树，也不扩充 MicroProps。目录证明不代替业务 SELF owner 校验。完整 DTO、3 秒绝对窗口和总部范围规则见[应用受控组织目录](application-organization-directory.md)。

## 失败处理与验证

业务 API 使用 `{ "code": "...", "message": "..." }`：401 为令牌或客户端无效，403 为操作或数据授权缺失，404 为目标不可见，400 为未登记用途或参数错误，429 附带 `Retry-After`，503 表示未能及时证明新鲜授权。业务后端在这些失败情况下应终止当前操作。

授权任务受 `APP_AUTHORIZATION_MAX_CONCURRENT`（默认 32）和 `APP_AUTHORIZATION_TIMEOUT_MS`（默认 5000）限制。令牌内省另有客户端与来源预算，见 [OAuth 签发](oauth-issuance.md)。

回归依据包括 `src/authorization_isolation_tests.rs`、`src/data_grant_tests.rs`、`src/notification_tests.rs`、`src/public_contract_tests.rs` 及 `tests/authorization_policy.rs`。数据库测试会创建和删除隔离数据库，按[开发文档](development.md)配置测试数据库后显式运行 ignored 测试。
