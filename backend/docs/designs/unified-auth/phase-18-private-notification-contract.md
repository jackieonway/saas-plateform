# Phase 18 私有通知发布与原操作恢复

本合同供持有业务资源权限的服务端使用。公开 `/api/apps/{appCode}/notifications` 继续只接受当前内部 `app_session`；它不增加外部 OAuth Token fallback。浏览器 SDK 保持 3.0.0，MicroProps 2.0、授权上下文 1.0、通知 1.0 的公开载荷不变。

## 私有请求

```text
POST /internal/platform/apps/{appCode}/notifications/publish
POST /internal/platform/apps/{appCode}/notifications/result
Authorization: Basic <independent APP_RESOURCE service credentials>
Content-Type: application/x-www-form-urlencoded

token=<original credential>
resource=notification
action=publish
request=<original PublishBody JSON>
```

两条路径均禁止 URL 查询参数、Cookie、重复／不明确的服务认证、重复或未知表单字段。表单最大 64 KiB，原 token 有独立长度和空白约束。`request` 使用现有严格通知 DTO：

```json
{
  "idempotencyKey": "ticket:019d0000-0000-7000-8000-000000000001:reminder:original-operation",
  "target": {
    "kind": "USER",
    "userId": "019d0000-0000-7000-8000-000000000002",
    "organizationId": "019d0000-0000-7000-8000-000000000003"
  },
  "type": "INFO",
  "title": "工单提醒",
  "content": "请处理当前组织的工单",
  "link": "/micro/work-orders/tickets"
}
```

组织和租户广播仍使用原 `ORGANIZATION`／`TENANT` target，并受当前直接成员的数据授权和组织硬边界约束。`SELF` 不允许组织广播。跨组织、失效或无法接收的 USER target 不会产生成功结果；链接须位于当前应用 `/micro/{appCode}` 内。

## 服务登记与业务授权

服务必须在独立的 `platform_resource_services` 登记为 `APP_RESOURCE`，绑定当前目标应用、准确 audience、`EMPLOYEE` 和 `notification:publish` operation。服务密钥不能复用 OAuth、身份服务或上下文服务凭据。

该应用还须显式登记 `platform_resource_bindings` 的 `notification/publish`、canonical resource、audience 和 OAuth scope。工单建议固定为 canonical resource／audience `work-orders`，scope `work-orders:notification:publish`；这是当前应用经审核的映射，不自动向其他客户端或应用开放。`data_grant_policy` 同时登记 notification/publish 及允许的数据范围。

外部 USER 使用原 opaque Access Token，在线消费身份服务 `resource-access-proof/1`，再读取原 COMMITTED selection 的不可变账号／成员绑定。OAuth 调用者应用与通知目标业务应用保持独立。公开 `sub` 不解析为用户 ID。MACHINE、CUSTOMER、ACCOUNT_ONLY、ID Token、refresh token、错误 scope／audience 或来源失效均拒绝；内部能力按摘要注册簿优先分类，已登记但失效的内部能力不能落入外部验证。

发布权限是当前应用 `notification:publish`，数据权限是当前直接成员的独立 `notification/publish` grant；拥有 ticket/read、资源 scope 或管理员身份均不能单独代替该数据授权。员工收件人必须有当前有效的成员、用户、租户、组织链及应用授权，投递行保留收件成员和身份代际。首版客户通知保持关闭。

私有 publication 还在不可变 `request_body.authority.recipientProofs` 中保存由服务端生成的完整原收件成员事实（用户、组织、成员、员工类型及身份代际），与 publication 的租户、应用和投递数量绑定。原请求摘要不包含这些动态服务端事实；调用方不能注入或替换它们。收件人删除收件箱投递后，结果恢复仍按这份独立事实复验全部原收件人。事实缺失、畸形、重复或数量不符返回 503；不能返回可触发补发的 `404 NOT_FOUND`。原收件资格已失效则拒绝恢复，仍有效时恢复原 publication，不重建已删除的收件箱行。

业务服务须先验证原业务操作、发布 scope、发布 permission 和独立数据授权，再创建提醒 intent。工单 intent 固定原 token 摘要、来源绑定摘要、幂等键和请求；更换 token、来源、账号或组织不能接管原 intent。不可持业务行锁或数据库连接等待平台／身份 HTTP；恢复也不使用新组织的凭据补偿旧组织结果。

## 成功与失败

两条路径成功均返回：

```json
{
  "contractVersion": "resource-notification/1",
  "expiresAt": "2030-01-01T00:00:03.000Z",
  "publication": {
    "publicationId": "019d0000-0000-7000-8000-000000000004",
    "recipientCount": 1,
    "deliveryCount": 1,
    "duplicate": false,
    "createdAt": "2030-01-01T00:00:00Z"
  }
}
```

wrapper 最长三秒，并截短到原来源、成员、业务范围和应用能力的期限；它不是离线写权限。数量表示持久投递，不表示实时收到或已读。所有私有响应和错误带 no-store、no-cache、no-referrer、nosniff。

平台使用原 token 摘要和精确绑定的摘要形成私有来源域，再以原 idempotencyKey 形成操作域。摘要域与规范化原请求写入现有 publication；不持久化原 token。相同原操作重复 publish 返回原 publication，`duplicate=true`；同键改变内容返回 409。更换来源不能找到另一来源的 publication。

`result` 只查原结果，不创建、不补发。仅明确的 `404` 和 `code=NOT_FOUND` 表示在该仍有效原来源和原请求下没有已提交结果；其他错误或无法确认的结果不能被当成未发布。调用方可以在该明确 404 后使用同原 token、原请求和原幂等键 publish。401／403 表示原身份、来源或当前授权无法使用；503 表示权威、期限或基础设施无法证明结果。过期的原凭据不能换成新凭据恢复。即便 publication 已存在，恢复仍复验当前发布授权、原来源及原收件人，不能把旧成功回执当作持续权限。

## 权威检查与事务边界

平台在打开发布事务前完成两次独立的原来源在线证明、服务凭据、资源映射和完整上下文检查。事务使用原来源／原操作域的 advisory barrier，在等待后复验发布范围；插入投递与审计后，以新的本地快照再次检查 COMMITTED selection、成员代际、服务版本、应用策略、permission、数据 grant 和原收件人，再提交。任何失败回滚 publication、投递及事务通知提示。结果查询走同一授权链，且不执行写入分支。

这些检查不宣称跨身份服务、平台数据库和网络响应的分布式原子性。在线权威检查属于有界检查点，私有 wrapper、上下文版本和目录结果不能直接授权后续写入；业务服务仍须在原操作提交边界复验原来源与业务条件。最终全服务 HTTPS、并发撤销和部署矩阵由 Phase 25 在固定版本环境重跑。

## 当前验证入口

- 平台：`cargo test --locked --bin saas-platform-backend phase18_notification_http -- --ignored --nocapture --test-threads=1`，显式设置隔离 PostgreSQL 管理连接；测试创建并删除自己的数据库和 LOGIN 角色。
- 宿主：`npm run test:auth:context:browser`，真实双标签页、宿主、qiankun、SDK3 和 Socket.IO，身份／BFF HTTP 为明确夹具。
- 工单：使用本阶段工单 adapter/reminder 的独立普通测试与隔离数据库门禁；参见 [Phase 18 报告](validation/phase-18.md)。
