# Phase 6：已登记客户本人资料入口

本接口是平台进程内独立的客户本人资料资源。阶段范围见[主台账 Phase 6](task-ledger.md)，具体实现位于 [customer_identity](../../src/customer_identity/mod.rs)。本文定义调用合同，不代替实际测试或部署验收。

首版只允许读取当前账号和所选有效客户成员的最少资料，不增加订单、工单、支付、员工目录或客户角色能力。没有创建客户门户或 SSO 会话；员工控制台的 `CurrentUser`、旧 `/api/auth/*`、授权同意和 qiankun 应用 token 入口没有为客户放宽。

## 服务器登记

客户最小入口使用两个必须成对配置的 UUID v7：

| 配置 | 含义 |
| --- | --- |
| `CUSTOMER_PROFILE_CLIENT_REF` | 精确的 `oauth_clients.id`，即客户端数据库主键；不是公开 `client_id`，也不是 client secret。 |
| `CUSTOMER_PROFILE_APPLICATION_ID` | 精确的 `applications.id`。 |

两者默认均不配置。配置解析要求规范小写、带连字符的 RFC variant UUID v7；缺其中一个、空值或非法值拒绝启动。两个 ID 配置完成也不创建应用、客户端、成员、授权或组织。

服务器代码把这一对注册为固定用途 `CUSTOMER_PROFILE`，资源入口为 `/api/customer/profile`，只提供下述三个 GET。每次访问还要求：

- 客户端真实存在、启用且 `subject_mode='USER'`，精确关联登记的应用。`PUBLIC`／`CONFIDENTIAL` 保留原协议含义；`MACHINE`、`LEGACY_MIXED` 不能作为这个资源的来源用途。
- 当前客户类型同时被来源客户端和应用的 `allowed_member_types` 允许；两者的实际策略版本与令牌保存版本一致。
- 应用启用，且当前租户与当前组织都授权了该应用。平台菜单或同账号员工角色不能代替这些条件。
- 令牌拥有 `profile` scope，当前客户端注册的 scope 仍包含 `profile`。

没有登记时，即使携带有效普通 OAuth access token，新客户资源也返回 `403`。不同客户端、不同应用或只把 `CUSTOMER` 加入泛用允许集合，都不会自动登记此入口。系统不选择最早创建的客户端、不按名称猜应用，也不把机器 service client 当成人员来源。

这两个字段登记资源用途；成员创建仍受生命周期、客户、第二身份开关及整体验收条件约束。正式创建默认关闭，不能把资源注册当作收缩迁移、全实例 ready 或生产启用证明。运行此入口的实例应使用一致且获批准的注册配置。

## 真实凭据来源

本阶段复用已有 `POST /oauth/token` 普通用户签发链，没有新增密码登录 API 或 context 交换接口。最小闭环使用明确登记了 `password` grant 的 USER 客户端，并在表单中显式选择已有客户 `membership_id`：

```http
POST /oauth/token
Content-Type: application/x-www-form-urlencoded

grant_type=password&client_id=<public-client-id>&client_secret=<confidential-client-secret>&username=<existing-account>&password=<account-password>&membership_id=<customer-membership-v7>&scope=profile
```

这里的公开 `client_id` 用于原 OAuth 客户端认证，与服务器登记的数据库 `CLIENT_REF` 分开。机密客户端必须提供有效密钥；完整请求仍执行原密码验证、限流、用户锁定、来源客户端与应用准入和事务审计。新 USER 客户端默认 grant 继续为授权码加 S256，`password` 只在明确兼容用途下显式登记；本阶段不把密码 grant 改成默认，也不宣称它是 OIDC。

密码证明和具体 `membership_id` 经真实签发流程验证后，`oauth_issuance::capture_binding` 保存账号、具体成员、类型、身份版本、客户端版本和应用版本。令牌以 `token_use='oauth'`、`storage_version=2` 保存摘要及随机 jti，响应使用原 OAuth `access_token` 等字段。只有客户端明确登记 refresh grant 时才有刷新令牌，刷新继续由原接口轮换并校验同一成员证明。

同账号同组织的客户与员工是两条不同关系。双类型客户端／应用可以覆盖两种类型，但一次签发只选择一个已经存在且有效的 `membership_id`；不会新增关系、改变成员类型或合并两行权限。员工令牌即使来自双类型集合，也不能访问本客户资源。

本资源只接收普通 OAuth **access** token，不接收平台 session、credential ticket、qiankun `token_use='app'` access 或机器凭据。客户不能借此获得平台 session，也不能向旧员工应用换取 app token。普通 `/oauth/userinfo`、内省及 Phase 4 的通知内部证明仍保留各自已验收的基础协议含义；本资源登记不重定义或开放这些历史协议。

## 每次请求的证明

请求携带：

```http
Authorization: Bearer <ordinary-oauth-access-token>
```

服务端首先验证签名和精确有效期，再根据完整 token 的摘要读取唯一持久记录，核验用途、存储版本、撤销、jti 和过期状态。JWT 的主体、租户、组织、client ID、scope、jti 和整秒 expiry 必须与该持久记录一致；仅凭签名或 JWT 中的账号／组织不能补出一个成员。

实际主体来自持久证明的 `user_id` 和 `membership_id`。共享 `oauth_issuance::binding_identity` 重新核验启用账号及账号版本、精确客户成员及身份版本、租户、完整组织链的状态／到期、客户端／应用策略版本和两级应用授权。如原令牌关联父平台 session，也要求父 session 对同一主体和成员仍有效。

本人资料读取后，使用新查询重新读取并核验原令牌及完整证明，防止读取期间发生的成员或准入失效继续返回旧上下文。权威数据库不可用返回 `503`；不会退化为只验 JWT、沿用缓存角色或按当前成员关系修补旧令牌。

## 三个只读接口

所有 GET 都没有主体、成员、租户、组织或应用选择参数。任何非空 query 返回 `400`，例如 `?userId=...`、`?membershipId=...`；路径也不提供任意 ID 代读。后端始终使用当前令牌已验证的关系，不能以同账号的员工成员替代客户成员。

### `GET /api/customer/context`

返回独立的十字段客户 DTO：

```json
{
  "contractVersion": "customer-profile/1",
  "purpose": "CUSTOMER_PROFILE",
  "userId": "<current-account-v7>",
  "membershipId": "<current-customer-membership-v7>",
  "memberType": "CUSTOMER",
  "identityVersion": "1",
  "accountVersion": "0",
  "tenant": { "id": "<current-tenant-v7>", "name": "<tenant-name>" },
  "organization": { "id": "<current-organization-v7>", "name": "<organization-name>" },
  "application": {
    "id": "<registered-application-v7>",
    "code": "<registered-application-code>",
    "name": "<registered-application-name>",
    "entryPath": "/api/customer/profile"
  }
}
```

`identityVersion` 为正整数规范十进制字符串，`accountVersion` 为非负整数规范十进制字符串，均不超过 `i64` 最大值。独立版本标识 `customer-profile/1` 不等于固定 SDK Context 1 合同，也不包含员工菜单、角色、权限、组织树或超级管理员标志。

### `GET /api/customer/profile`

精确返回 `context` 与 `account` 两个对象。`context` 是上一节完整客户 context，`account` 的固定结构为：

```json
{
  "id": "<same-current-account-v7>",
  "username": "<current-username>",
  "realName": "<current-real-name-or-empty>",
  "avatar": "<current-avatar-or-empty>"
}
```

账号 ID 必须等于 context 的 `userId`；最少本人资料仅含四个字段，不返回账号密码、联系方式、员工部门岗位或任何授权。没有另设重复 `/me` 路由或修改接口。

### `GET /api/customer/applications`

响应精确包含以下三个字段：

| 字段 | JSON 类型与内容 |
| --- | --- |
| `contractVersion` | 字符串 `customer-profile/1`。 |
| `context` | 上一节完整客户 context 对象。 |
| `applications` | 只含一个应用对象的数组，该对象与 `context.application` 相同。 |

列表只含已经通过本次全部校验的登记应用；不是员工应用目录，也不枚举租户或组织的所有应用。未登记、失效或未授权的注册目标直接拒绝，不以空目录掩盖无效 access token。

## 错误与隔离

| HTTP | 行为 |
| --- | --- |
| `400` | 有效身份提交任意非空 query；不接受代读选择。 |
| `401` | 缺少或非法 access、错误凭据用途、员工／机器主体、缺 scope、精确注册不匹配、过期或撤销，以及成员／账号／客户端／应用证明失效。统一错误不返回其他主体资料。 |
| `403` | 当前进程没有配置客户最小入口注册。 |
| `503` | 无法从权威数据库确认当前凭据或资料。 |

客户无权读取另一客户、同账号员工成员或跨组织资源。局部成员撤销只处理对应成员 ID，其他合法成员按自己的令牌独立校验；全局账号版本变化使该账号所有旧证明失效。策略收紧继续由原事务提升版本并撤销相关凭据，资源接口同时逐请求核对版本。

未来真实客户资源仍须单独冻结资源、动作和 owner 字段。不能把 `profile` scope、本资料接口通过或客户与员工共用 `users.id` 当作订单／工单等业务授权。

## 验证范围

Phase 6 客户专项使用显式 `IDENTITY_PHASE06_TEST_DATABASE_URL`，只操作自有新库。成功链应通过真实 `/oauth/token` 验证密码和具体成员后取得 access，再调用三个资源接口；不能仅手工签一个 JWT 或插入一条 token 行声称凭据签发闭环通过。

正向覆盖客户专属与双类型集合选择实际客户；负向覆盖员工、客户 client／员工 app 及反向组合、机器与 session／app token、缺注册／错误注册、scope、任意代读 query、账号／成员／策略／租户组织撤销和权威故障。收缩迁移、全实例 readiness、正式创建开关及已有阶段的员工／通知安全回归分别提供自己的证据，不能以本模块静态说明代替其门禁。
