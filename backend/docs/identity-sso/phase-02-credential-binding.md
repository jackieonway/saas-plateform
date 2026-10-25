# Phase 2 凭据与具体成员绑定

本阶段的数据库增量为 `0014_credential_identity_binding.sql`。它保持扩展期的二元／三元唯一约束，不开放真实客户或第二身份创建。双身份正向验证仅在专用测试库显式移除二元约束。

## 签发与权威校验

`platform_sessions` 保存账号凭证版本 `account_version`、具体 `membership_id`、签发时 `member_type`／`identity_version`，以及来源 `origin_issuer`、`origin_client_ref` 和来源策略版本。账号版本的权威来源仍是 `users.session_version`。JWT 的账号／组织／会话与服务器记录相匹配后，还必须检查该记录的全部版本；不能由当前 user＋org 查询补出旧凭据缺失的证明。

旧控制台的来源由服务器固定选择已登记的 `platform-console-legacy`，issuer 为 `urn:saas:legacy-platform`。该注册是员工专用 PUBLIC USER 客户端，不关联应用，没有可用 redirect URI；其 S256 设置不把旧控制台密码登录变成 OIDC。管理员不能把它改为客户或机器用途、绑定应用、增加回调或删除。正式 OIDC issuer 和 BFF 客户端仍由后续阶段交付。

密码验证后的选择票据保存每个已验证账号的版本、所提供的具体成员 ID／版本及来源客户端策略版本。`POST /api/auth/context` 接受 `membershipId`；兼容只传 `organizationId` 的旧员工请求时，必须唯一定位该账号在该组织的有效 EMPLOYEE。旧无快照票据、伪造成员、组织不匹配和过期版本均拒绝。普通切换选择其他已有有效关系，不改写成员类型或状态。

平台 refresh 的 `session_version` 与父 session 的签发账号版本相等；刷新沿用父成员和来源，不重新选择成员，也不将旧证明升级为当前版本。员工 `CurrentUser` 和员工 API 的事务守卫在完整权威检查后仍要求 EMPLOYEE。已绑定的客户 fixture 可以验证独立会话与刷新，不构成客户生产入口开放。

普通 OAuth access／refresh／code／approved device 保存相同的账号和成员证明，以及目标客户端、关联应用的策略版本。pending consent／device 在产生时就保存客户端和应用策略版本，批准时再次验证，不能把策略变化前的请求按最新策略激活。password grant 仅在本次密码验证和明确成员选择后捕获新证明；刷新、code 和 device 兑换只沿用已保存证明。

由平台 session 批准的 code／device 及其派生 OAuth token 另存 `platform_session_id`，逐次检查原父会话、成员、账号和来源策略。因此父会话注销、来源策略变化或绝对到期会使派生凭据失效。直接密码 grant 没有虚构父会话，其目标客户端策略和本次密码证明构成直接认证来源。

应用 token 继承父 context 的成员、账号及 `origin_*`，单独保存应用策略、服务客户端策略和 `application_service_client_ref`。已有 `client_id/cid` 继续证明应用服务绑定，不覆盖登录来源。机器客户端只证明服务资格；它的空成员集合不代表允许任意人员身份。应用 token 使用时同时验证父 session、来源客户端、服务客户端、应用和租户／组织两级应用授权。

## 撤销与事务

成员状态和版本变化、删除以及身份替换按不可变的 `membership_id` 撤销 session、平台 refresh、应用／OAuth token、code 和 approved device；同账号同组织的另一成员行及其他组织不受影响。凭据保存成员引用作为撤销证据，不因成员删除而丢失。

账号版本增加使所有用户凭据立即失效；同库过渡阶段的触发器还在同事务物理撤销。应用策略变化只撤对应应用凭据；客户端策略变化只撤该客户端及其来源派生凭据。使用方每次仍比较权威版本，不依赖清理是否已经完成。策略保存、版本变化、物理撤销和原因审计共用事务，审计失败回滚。

签发／兑换锁序为账号 → 应用 → 按稳定顺序排列的来源／目标客户端 → 一次性凭据。等待之后重新检查已证明的版本、当前有效性和到期时间。生命周期服务对操作者和目标账号采用稳定排序，并在等待后及提交前再次核验具体会话与权限。

权威数据库连接关闭、网络故障、连接预算或锁／查询超时返回 `503`，受保护业务不得继续。凭据已撤销或证明不匹配与依赖故障分别表达，不能降级为仅验 JWT 签名。

## 旧凭据处理与协议边界

追加列允许 NULL 是为了保留旧行作审计，所有接受入口都拒绝缺失证明。迁移一次性撤销原有 session／refresh／token／code／device／pending consent，不读取当前成员来回填，不迁入历史凭据。历史 schema 和旧格式负例仅以隔离 fixture 验证。

本阶段不修改固定 SDK、MicroProps 2、Context 1 或 Notification 1 的发布字节和含义。管理 context/profile 可以返回独立的成员信息；完整客户 DTO、员工授权隔离、通知隔离及客户端接入仍由各自后续阶段验收。策略 API 与 CLI 见 [Phase 2 准入管理](phase-02-policy-api.md)。实际测试结果和限制在 `validation/phase-02.md` 记录，不能以本文设计说明代替通过证据。
