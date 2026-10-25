# Phase 7：同进程认证边界与账号管理 port 验收

后续状态：本文为 Phase 7 的历史验收记录。此处保留的个人改密在途请求限制已由 [Phase 8](phase-08.md) 的锁后原会话／成员再验与六项真实等待回归闭环；全局 SSO 会话也由该后续阶段建立。原失败、修复与 Phase 7 当时的实现边界不改写。

日期：2026-10-25。状态：**COMPLETED，三项验收 PASS**。依赖 [Phase 6](phase-06.md) 已完成。全目标静态检查、194 项普通测试、178 项既有数据库回归、新增四项 PostgreSQL／HTTP 合同及当前后端二进制的一项receipt启动回归均已通过；首轮静态及fixture失败、修正和定点重跑记录完整保留。

范围依据[实施台账](../task-ledger.md)的 Phase 7。当前保持平台包与进程形态，整理账号认证、密码／全局账号管理、平台组织授权及混合 legacy OAuth 的职责。SSO 全局 session、Cookie、OIDC、BFF、客户门户和独立认证部署仍属于后续阶段。

## 当前执行边界

- 认证与账号管理由消费者定义窄 port，生产适配保持现有外部入口、审计原子性、既有锁等待后的权威重验及账号／成员撤销语义。个人改密的旧在途请求例外及 Phase 8 必须增加的提交前认证上下文再验，见[边界说明](../phase-07-authentication-boundary.md#个人改密的既有在途请求限制与-phase-8-必闭环项)。
- 用户基础身份和凭证归账号职责；成员、租户、组织、角色、菜单和数据授权归平台。`users.is_super_admin` 不能成为通用认证声明。
- 存储与迁移 owner 按真实数据用途逐项记录，不按 `/oauth/*` 前缀整体迁移。
- 新回归只使用显式 `IDENTITY_PHASE07_TEST_DATABASE_URL` 及任务自有随机数据库；Phase 6 的真实开发库与 receipt 保留为已验收快照。新二进制不能使用旧文件摘要的 receipt 冒充就绪。
- 既有固定 SDK、Context 1、MicroProps 2、Notification 1 与历史迁移字节保持对应合同。
- 本阶段未改前端业务代码或 SDK 发行物，沿用 [Phase 6](phase-06.md) 的前端／浏览器和 SDK 消费证据；没有把未重跑的前端或 SDK 门禁计作本阶段新执行的 PASS。

## 验收检查

[账号认证边界与存储 owner](../phase-07-authentication-boundary.md)已逐字段／表／用途记录当前实现与未来部署限制；端口源码为 [platform_accounts.rs](../../../src/platform_accounts.rs)，适配与用例位于 [authentication](../../../src/authentication/mod.rs)。HTTP 路由继续拥有原授权、锁和提交边界；同库过渡复用应用错误，尚不属于独立的无框架认证服务。

`AccountMutationTx` 是显式的同库事务能力，PostgreSQL适配借用原连接、不自行commit；没有使用 `Any`／downcast隐藏数据库依赖。`CredentialHash`、`AccountCredential` 和 `PreparedPassword` 无Debug／Serialize，但编码getter为 `pub(crate)`，属于provider／存储适配可访问的内部句柄，不能称为整个crate都无法读取hash。`AccountFuture`仍复用实现Axum `IntoResponse` 的既有 `AppError`，没有把同进程适配层夸称为完全无框架domain core。

账号创建、密码验证／重置／个人改密、全局启停和kick已接入账号port；基础资料UPDATE、账号DELETE与seed仍按边界文档登记为受控过渡writer。成员、组织、角色、菜单、数据授权、平台context和legacy OAuth用途继续归平台；`users.is_super_admin` 不进入认证证明。当前没有新增独立服务、全局SSO session或OIDC存储。

## 实际门禁结果

| 门禁 | 实际状态 |
| --- | --- |
| Linux production binary `cargo check --locked --offline --bin saas-platform-backend` | **PASS，23.70 秒**，见 [production-check.log](../../../../local-identity-sso-phase07-qa/production-check.log)。只覆盖生产目标，不等同于全部测试编译通过。 |
| 首轮全目标 fmt／check／Clippy | fmt 与全目标 check 通过（61 秒），check 有两项 unused Result 警告；Clippy `-D warnings` 因同两处遗漏失败，尚未执行普通／数据库测试。 |
| 修正后全目标 fmt／check／Clippy、普通测试 | **PASS**；check 18.58 秒、Clippy 72 秒、普通 **194／194**（14 library＋166 main＋14 外部测试；main 11.83 秒），见 [static-and-new-tests.log](../../../../local-identity-sso-phase07-qa/static-and-new-tests.log)。 |
| 实际账号用例＋内存事务 capability 普通合同 | **PASS**，已包含在 194 项中；`phase07_real_account_use_case_accepts_a_non_postgres_transaction_capability` 使用内存事务能力，未连接 PostgreSQL。 |
| fixture 修正后的 fmt／Clippy、新 4 项 ignored 数据库／HTTP 测试 | **PASS**；fmt无差异、Clippy `-D warnings` 33.44 秒、新合同 **4／4，30.32 秒**，见 [new-tests-final.log](../../../../local-identity-sso-phase07-qa/new-tests-final.log)。四个自有库均记录 `assertions_ok=true`、`database_removed=true`、`pool_closed=true`。首轮0／4失败单独保留，不以最终PASS覆盖。 |
| 文档与既有员工、客户、legacy OAuth／刷新、账号保护及撤销基线 | **PASS**，文档 26.66 秒、既有 main ignored **178／178，659.31 秒**，见 [backend-existing-baseline.log](../../../../local-identity-sso-phase07-qa/backend-existing-baseline.log)；命令明确排除新 `authentication_boundary_tests`，与4项新增合同分组计数。 |
| 当前后端二进制的非test receipt启动回归 | **PASS，1／1，53.53秒，退出0**；已有 `membership_rollout_phase06_development_startup_requires_live_receipt` 单独执行并记录owned database cleanup PASS，见 [startup-final.log](../../../../local-identity-sso-phase07-qa/startup-final.log)。在自有新库以当前文件hash验证缺失／错误receipt拒绝、有效receipt启动和结构漂移拒绝，没有改用Phase6已保留开发库的旧receipt。 |

普通194项与两组数据库合同分别为 **194／194、178／178、4／4**，没有把被普通测试忽略的数据库测试混计为普通PASS。新增合同的真实HTTP为Axum Router请求，数据库为任务自有PostgreSQL；附加启动回归另行验证实际平台可执行文件，不能用Router测试替代进程启动证明。

## 新增合同实际覆盖

测试源码见 [authentication_boundary_tests.rs](../../../src/authentication_boundary_tests.rs)。四项新增数据库合同均已通过：

| 合同 | 已验证行为 |
| --- | --- |
| 替换 `AuthenticationPort` | 同时替换控制台与ordinary OAuth密码消费者；以已确认不匹配本地hash的测试输入证明没有偷偷走本地密码旁路。fake认证成功仍不能制造EMPLOYEE成员；客户专属和无成员账号继续被旧控制台拒绝。拒绝／权威不可用／容量受限分别保留既有错误语义、503／429及Retry-After，不签部分凭据或成功登录审计。 |
| 替换 `AccountAdministrationPort` | 创建、管理员reset、个人改密、单项／批量状态及全局kick六类真实管理请求遇到注入不可用均返回503；账号、凭据、通知和审计快照无部分提交，原员工session仍有效。 |
| 真实账号reset | 新hash替换旧hash、账号代际只提升一次，成员ID／类型／状态不变；员工／客户／跨组织及legacy人员凭据失效，不波及其他账号和机器凭据；新密码可建立新的员工context，审计不含密码或PHC。 |
| 真实全局状态 | ENABLED→DISABLED、重复DISABLED、再ENABLED不更改密码和成员状态；代际只在首次禁用提升，重新启用不能恢复原凭据，必须建立新context；无关账号与机器凭据保留。 |

## 台账标准

| 台账标准 | 结果 | 所需证据 |
| --- | --- | --- |
| 平台组织授权不直接操作密码验证实现；重置密码、全局封禁从账号管理 port 发起 | **PASS** | 实际provider／route调用链、账号管理port的六类失败原子性、真实reset及全局启停合同通过；平台保留管理scope、原事务审计和通知。 |
| 每类账号／会话／OAuth 数据有明确 owner，不按前缀迁走全部 OAuth | **PASS** | [边界清单](../phase-07-authentication-boundary.md)逐项覆盖14个users字段、平台session／refresh／app token、未来全局session及五类混合legacy OAuth存储；声明当前单一迁移owner与Phase12投影／权限交接，保留同库过渡限制。 |
| fake／契约验证通过，原员工、客户和 legacy OAuth 行为回归通过 | **PASS** | 内存事务capability普通合同、新四项真实PG／Router合同及178项既有数据库基线分别通过，相关静态与194项普通测试通过；附加receipt启动回归单独记录。 |

以上三项及附加启动回归均依据已经取得的源码审查和实际运行结果标记PASS，Phase7完成。台账和导航状态由根代理统一推进，后续Phase8继续处理下述已明确的认证会话及提交前再验要求。

## 首轮失败与修正

[initial-gates.log](../../../../local-identity-sso-phase07-qa/initial-gates.log)保留首次全目标 check 的两个警告及 Clippy 失败：新测试在客户不能借 fake 账号认证取得员工身份的两处 `expect(...)` 调用中没有传播返回的 `Result`。已仅补 `?`，保留原 Forbidden 断言并确保失败进入自有数据库清理；没有忽略返回值、放宽 lint 或修改生产实现。修复后的静态与普通门禁已通过，实际结果见上表及 [static-and-new-tests.log](../../../../local-identity-sso-phase07-qa/static-and-new-tests.log)。

修复后静态与普通门禁通过，但新四项数据库测试首轮在共同fixture初始化时报 `new legacy mixed clients are prohibited`，结果为 **0／4，4.86秒**，尚未进入行为断言。员工ordinary OAuth fixture随后显式登记为 `USER`／`EMPLOYEE`，客户为 `USER`／`CUSTOMER`，机器使用独立 `MACHINE`／空成员集合；legacy表示现有协议入口，不表示可以创建已禁止的新混合模式。没有禁trigger、绕过schema或放宽业务断言。原始四项失败及 `assertions_ok=false`／清库成功保留于 [static-and-new-tests.log](../../../../local-identity-sso-phase07-qa/static-and-new-tests.log)；修正后 **4／4 PASS** 和再次fmt／Clippy通过保留于 [new-tests-final.log](../../../../local-identity-sso-phase07-qa/new-tests-final.log)。

## 已确认限制与下一阶段交接

个人改密保留原有在途请求语义：CurrentUser及oldPassword先验证，而账号锁后只比较原ID／hash，ban、kick或成员撤销若先提交，已开始的命令仍可能完成。它不会恢复session、成员或禁用状态，管理员reset已改hash则409；ban情形不能视为重新建立了有效账号认证。此结论来自只读交错分析，不冒领尚未执行的并发测试结果。

Phase8须按[台账](../task-ledger.md)和[精确边界](../phase-07-authentication-boundary.md#个人改密的既有在途请求限制与-phase-8-必闭环项)闭环：legacy个人改密在账号锁后重验原session／具体成员，SSO自助命令重验global sid、账号状态与版本；旧context失效须拒绝pending命令，不能补读当前版本复活旧证明。以受控真实锁等待覆盖ban／kick／成员撤销／password reset，通用管理员reset仍支持受授权地重置DISABLED账号。本阶段不为纯抽离引入临时版本冲突或SSO能力。

SSO Cookie、全局session、OIDC、BFF和独立部署均未由本阶段交付；生产客户／双身份开关仍默认关闭。Phase6任务开发库的原二进制、manifest与receipt保持其原验收范围，本阶段新文件hash不能复用那份receipt宣称新的环境就绪。
