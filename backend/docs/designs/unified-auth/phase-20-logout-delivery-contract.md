# Phase 20 登出、可靠通知与受控邀请合同

版本：1，2026-10-28。对应[任务台账](../unified-oauth-oidc-sso-task-ledger.md) Phase 20，依赖7、9、10、13、14、19均已完成。本合同固定各owner共同实现的接口；实际生产责任、收件渠道和网络目的仍须可信owner核实，缺失时失败关闭。本阶段测试只使用任务collector和明确owned recipient，不向真实外部收件人发送消息。

## 范围与来源

| 操作 | 同一事务内的失效范围 | 同意记录 |
| --- | --- | --- |
| CURRENT_APP | 发起RP原token所绑定的一个原USER grant及其access/refresh family；原RP本地会话和derived会话 | 保留 |
| GLOBAL | 原token所属账号的全部SSO、USER grants及在线/离线families；不影响其他账号或MACHINE | 保留 |
| 撤销同意 | 该用户、同意组和原consent generation的grant/code/token来源 | 保留最小撤销事实；按已有规则隐藏展示 |
| 停用账号、改变账号安全代际、恢复 | 同一账号来源按当前安全合同失效，统一产生原RP来源的通知事实 | 不扩大或恢复同意 |

CURRENT_APP不等于退出同一公司所有设备或另一个grant。GLOBAL不等于删除同意。资源令牌的本地失效和通知是两个结果：撤销事务只写持久事实，不等待DNS、TLS或接收方；对方不可用不能延迟或撤销已经提交的本地失效。

`auth_rp_sessions`为每个原USER grant保存64位小写hex的随机opaque `public_sid`及原pairwise `public_sub`，精确绑定client/user/source SSO/grant且不可改。sid不是global_session UUID，也不是可用于认证的bearer credential。授权码兑换的ID Token包含sid，刷新保留原映射；两个BFF持久化已验证的`oidc_public_sid`和`oidc_public_sub`。刷新若返回ID Token，必须匹配原sid/sub；不返回则保留原值，不采用新来源。

## Canonical RP登出命令

`POST /oauth/logout`使用机密USER RP的Basic认证和form body：

```text
token=<原access或refresh opaque token>
scope=CURRENT_APP|GLOBAL
operation_id=<规范UUIDv7>
sid=<原ID Token opaque sid>
sub=<原已验证pairwise subject>
```

可选`client_id`只用于标准Basic一致性检查，不能覆盖认证RP。禁止query、Cookie、未知/重复字段和MACHINE/PUBLIC来源。第一次执行必须验证当前准确token及原client/grant/source、sid/sub，锁定账号和原grant后重新检查；不得从旧token换取账号最新来源。

200结果固定为：

```json
{"operationId":"<UUIDv7>","scope":"CURRENT_APP","state":"COMPLETED","revokedGrantCount":1,"revokedSsoCount":0,"notificationCount":1}
```

`auth_logout_command_receipts`与撤销同事务插入，绑定client、原grant/user、原token摘要、scope、sid/sub及请求摘要；只返回无秘密的原结果，不返回内部账号或global session ID。runtime仅SELECT/INSERT。重复相同原operation不重复推进账号代际；改变内容或来源冲突。

`POST /oauth/logout/operations/{operationId}`使用相同Basic和原token/sid/sub/scope查询；可选body operation_id必须与path相同。恢复已提交结果不要求原token仍有效，因为该操作本身可能已经撤销它。只有明确404、JSON code `NOT_FOUND`才允许同原operation再次POST命令，不能更换operation重放未知请求。

这是一项准确标记的认证RP命令扩展，不冒称Basic命令等同浏览器RP-Initiated Logout的`end_session_endpoint`。当前SSO自身的退出也调用同一canonical失效主体和事务通知事实。公开发现文档只发布实际已实现的能力。

## Back-channel接收与重放

两个BFF的固定接收端为`POST /auth/backchannel-logout`，body为form字段`logout_token`。只能使用当前固定issuer的JWKS与RS256；不按token提供的jku/x5u抓取密钥，不允许算法降级或ID Token当logout token。

header为`typ=logout+jwt`和已登记kid。claims严格为固定iss、单字符串aud等于该RP client ID、sid、sub、UUIDv7 jti、iat、exp及events；不含nonce。events准确为：

```json
{"http://schemas.openid.net/event/backchannel-logout":{}}
```

sid/sub都必须符合合同并匹配同一个本地来源；iat最多超前5秒、年龄不超过120秒，exp必须严格等于iat+120秒且尚未过期。错误签名、issuer/audience、现有sid与sub不匹配、事件、期限或未知claims返回400，不产生撤销。有效签名且未知sid可200零作用，以支持尚未完成回调或已经清理的原RP来源；不能因此查询或影响其他subject。

接收方在一事务中登记issuer/client/jti摘要与短期回放事实，锁定同sid/sub的来源，关闭其BFF/browser/derived会话。原sid已关闭时，新jti仍可200零作用；任何同jti重放一律409、code `BACKCHANNEL_REPLAY`，不得重复撤销或重新建立会话。通知可以先于发起登出的RPC回包到达；调用方应把同一原来源已关闭视为正常完成，不替换成新会话。

worker每次attempt生成新的jti/iat/exp，但outbox job及原sid/sub不变。丢响应后重试的新JWT只会再次指向已经关闭的旧来源；不把任意409当成功，也不会通知后来新建的会话。

## 可信目的、网络与可靠投递

`auth_delivery_targets`是identity owner控制的核验事实，identity runtime只有SELECT，平台runtime无权限。BACK_CHANNEL精确绑定client；VERIFIED_CHANNEL精确绑定recipient_ref及channel_purpose（ACCOUNT_ONBOARDING或SECURITY）。字段为id、purpose、status、client_id或recipient_ref/channel_purpose、url、allowed_ips、version、verified_reference、verified_at、expires_at和created_at。

目的必须是规范HTTPS，禁止用户信息、fragment、query、重定向和代理。每次投递核对原target/version、当前核验状态/期限与准确client登记；第三方地址还须匹配独立审核manifest/admission。DNS结果必须全部在原verified allowed_ips内，并通过公开可路由网络检查；连接固定到本次已检查IP，TLS仍验证原hostname，避免第二次DNS解析和重绑定。生产拒绝loopback、私网、link-local、multicast、metadata及其它保留网络。任务test模式可显式配置准确HTTPS测试origin，并且仍需owner fact及准确IP；禁止在production启用这个例外。

LOGOUT准备按application、client、admission、owner target的既有管理锁序保留当前登记，第三方admission必须是APPROVED、BACK_CHANNEL且URI精确匹配。签名等待后再次复核登记、owner事实和原租约期限。投递决定在该短事务提交时确定，网络随后发送；取消登记与尚未提交的投递决定串行化。

`auth_clients`默认`logout_mode=SHORT_SESSION_ONLINE_CHECK`且没有backchannel字段。启用BACK_CHANNEL必须具备准确`backchannel_logout_uri`及`backchannel_target_id`，匹配当前owner核验事实和第三方审核；不能由任意UUID自证。

`auth_delivery_jobs`统一承载LOGOUT、INVITATION、SECURITY。job按稳定event_key唯一，保存target/version和源grant/invitation/security event引用，不保存logout JWT、邀请token或密码。状态为PENDING、LEASED、DELIVERED、TERMINAL或DEGRADED；数据库claim使用FOR UPDATE SKIP LOCKED，持久lease_id/lease_until，网络前提交事务，完成时CAS原lease。

每次最多50个候选、有限并发，网络总期限3秒，lease30秒；每job最多8次、总窗口24小时，重试从60秒指数增长且最多1小时。408/429/5xx/网络故障受控重试；协议拒绝、失效目的/登记和永久4xx终止。过期lease可以被后续worker接管，旧worker不能覆盖新lease结果。终态写固定错误码和terminal_at，并产生结构化终态告警，不记录token、响应正文或不必要资料。

运行环境必须采集包含WARN的日志级别（例如`RUST_LOG=info`）及`identity_delivery_terminal`事件；ERROR-only过滤不能作为终态告警验收依据。告警可见性与日志脱敏须在实际运行配置下同时验证。

现有consent/security outbox保留源事实并记录expanded/terminal状态；网络只由中央jobs持lease。grant撤销触发器统一产生LOGOUT事实，账号/SSO撤销触发canonical USER grants失效，覆盖安全恢复和停用；缺少backchannel时写DEGRADED/短会话合同，不假称已通知。security目的必须是独立核验的SECURITY渠道；没有可信目的则降级告警，不从邮箱字符串推断可投递。

不支持通知的合作方须采用不超过5分钟本地会话，并在敏感资源每次请求或至多60秒间隔做当前在线检查；资源端仍立即消费canonical来源失效。不存在“所有第三方页面瞬间退出”的承诺，iframe不是唯一登出手段。

## Q5邀请投递与领取

邀请签发必须找到当前唯一、有效、精确recipient/purpose的VERIFIED_CHANNEL owner fact，并与邀请/INVITATION job同事务写入；缺少核验事实时403。管理API继续只返回元数据。取消旧的raw delivery-material和仅claim消费HTTP出口；worker只能按当前job/lease/target/version取得内存材料。

`auth_invitation_verifications`为owner-only可信核验事实，runtime仅SELECT，绑定invitation、recipient、ACCOUNT_ONBOARDING、delivery target/version、证据、核验/到期时间和状态。任意UUID不代表核验完成。

`POST /api/sso/invitations/{id}/accept` body为operationId、token、username、password、verificationRef。它在一事务内锁邀请、核对原token摘要及有效期/状态、可信verification和current target，使用现有安全密码hash创建全新账号、消费邀请、插入activation receipt和审计。username必须唯一；不启用既有停用账号、不按邮箱合并、不创建业务成员/权限、不自动建立登录会话。

activation receipt按operation唯一，绑定邀请/verification/新user及目的分离HMAC请求摘要；密码和token不进入receipt。相同原operation和内容可以恢复仅含operationId/invitationId/status=ACCOUNT_CREATED的原结果；不同operation重复领取冲突。邀请token只在worker内存构造发送体、以及受控领取请求中短暂存在，不放入日志、可复用投递记录或管理浏览器。

## Owner与验收

统一canonical0001、provision/verify/readiness、portal schema及角色测试由schema owner独占。identity登出/worker/整体合同由Phase20 owner实现；两个BFF owner负责持久化、RPC/receiver和各自测试；邀请owner负责Q5核验/领取/材料/测试。共享文件变更先协调，不混入Phase16/17运行时。

最小实际HTTPS门禁为：三服务真实登录产生opaque sid/sub；CURRENT_APP只失效原RP、另一RP同账号仍可用且consent保留；GLOBAL关闭两RP及原SSO并保留consent；真实签名通知接收、重放/错受众拒绝；任务collector先故障后受控重试、撤销响应不等待它，永久失败可观察；owner核验的owned recipient邀请投递和单次全新无成员账号领取，重复/过期/撤销拒绝且无秘密日志。DNS/私网/目的变更及lease并发采用独立PG/transport负向矩阵，不能用旧阶段证据冒充本阶段通过。
