# Phase 2 应用与客户端准入策略接口

应用管理沿用 `system:application` 权限和平台租户门禁，OAuth 客户端沿用 `system:oauth` 与客户端菜单权限。策略接口使用已验证的平台会话；写入在锁定操作者和策略 owner 后重新核验当前会话与权限。

| 对象 | 读取 | 保存 |
| --- | --- | --- |
| 应用 | `GET /api/applications/{id}/access-policy` | `PUT /api/applications/{id}/access-policy` |
| OAuth 客户端 | `GET /api/oauth/clients/{id}/access-policy` | `PUT /api/oauth/clients/{id}/access-policy` |

读取返回 `id`、`status`、`allowedMemberTypes`、`accessPolicyVersion`；客户端还返回 `subjectMode`。应用与客户端普通列表、单项读取、创建和更新响应同样包含这些字段。`GET /api/oauth/clients/{id}` 提供单项客户端读取，响应不含密钥或密钥哈希。

保存应用策略的请求示例：

```json
{
  "allowedMemberTypes": ["EMPLOYEE"],
  "accessPolicyVersion": 1,
  "reason": "关闭已停止运营的客户入口"
}
```

客户端保存请求可另传 `subjectMode`，缺省时保留已登记的用途。策略写入必须携带读取时的 `accessPolicyVersion`，旧版本返回 `409 CONFLICT`；版本缺失、非正整数及不合法的集合返回 `400 BAD_REQUEST`。`reason` trim 后要求 1～500 个字符，不接受控制字符。独立策略接口拒绝未知键，应用策略不接受 `subjectMode`。调用方收到冲突后应重新读取、展示当前值并再次确认修改意图，不能自动把旧清单改成最新版本后重试。

`allowedMemberTypes` 只接受 `EMPLOYEE`、`CUSTOMER`，保存时去重并按此顺序规范化。应用和 `USER` 客户端的集合必须非空。`MACHINE` 必须为 `CONFIDENTIAL`、仅允许 `client_credentials`，集合必须为空；机器凭据不提供人员身份准入。禁止新建 `LEGACY_MIXED` 或将其他用途改为此模式，只有数据库中已登记的混合客户端可以在原兼容用途内维护。内置 `platform-console-legacy` 来源客户端固定为员工专用的 `PUBLIC`／`USER`、授权码加 PKCE、无回调与无关联应用，不能删除或改成客户／机器入口。

旧创建请求省略准入字段时，应用与人员客户端继续默认为仅员工，单一机器 grant 推导为 `MACHINE`／空集合。显式 `null` 不作为省略。普通应用更新可以携带 `allowedMemberTypes`，普通客户端更新可携带 `allowedMemberTypes`／`subjectMode`；这些显式策略编辑同样必须提供旧版本与真实原因。客户端用途转换需要在同一普通更新请求内提供兼容的 grant、clientType、成员集合和其他完整配置。

策略 owner 在一个数据库事务中完成行锁、旧版本检查、规范配置保存、凭据失效及审计。既有数据库触发器在影响准入的配置变化时提升版本；无配置变化时保持版本、不新增变更审计。任何安全策略版本变化都会使绑定旧版本的凭据失效，物理撤销和逐请求权威版本校验共同生效。应用只影响绑定该应用的凭据；客户端只影响将它作为来源、用户 OAuth 客户端或应用服务凭据的相关链路。收紧后该客户端内仍符合集合的账号可重新授权；其他未变更应用／客户端的凭据保持独立。

来源登录客户端与应用后端服务客户端是两个独立引用。应用服务客户端的 `clientId` 不得覆盖来源登录客户端；两者若都有用户准入限制，则分别收窄，机器凭据不能扩大来源允许的身份。

CLI 命令、严格清单和离线预览说明见 [platform-admin README](../../tools/platform-admin/README.md)。隔离 API 契约位于 `src/routes/admission_policy_tests.rs`，需要显式 `IDENTITY_PHASE02_TEST_DATABASE_URL`，只在新建自有测试数据库中执行全迁移与审计／并发故障验证。测试通过记录以阶段验收文件为准。
