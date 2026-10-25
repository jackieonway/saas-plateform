# Phase 13 活动消费者、兼容入口与双身份歧义审查

审查日期：2026-10-25。范围是本工作区八个仓库的当前源码与新身份库合同。此次审查没有发现需要新增产品代码修复的双身份安全阻断；实际运行的活动消费者合同共 **233 项 Node 测试、184 项 Rust 普通测试全部通过**，另通过 SDK 20 个生成发行物和平台固定 SDK 字节校验。此次没有访问数据库、启动业务服务或重建 Linux 正常运行二进制。真实灰度、双 RP、通知撤权、35 分钟负载和双库恢复以 [Phase 13 验收记录](validation/phase-13.md) 的独立 receipt 为准，不能由本文的源代码或普通测试结果代替。

## 证据与复跑入口

本轮命令、逐项状态和日志摘要见 [consumer-node-contracts.json](../../../local-identity-sso-phase13-qa/consumer-node-contracts.json)；Rust 结果、13 份日志的凭据模式扫描、成员逐命中清单和 **806 个当前源码／合同／固定发行物文件**的 SHA256 见 [consumer-source-sql-audit.json](../../../local-identity-sso-phase13-qa/consumer-source-sql-audit.json)。该清单记录精确选择范围及文件字节，不把未选择文件说成全仓已封存。当前 content-set SHA256 为 `fcaffcb65143f90b700b852c0e32ee9b1ca5e5f35679ca5ab18a93f2c9c9b933`；全阶段最终源码封存由主执行者另外核对。

可复跑 [run-consumer-contracts.ps1](../../../local-identity-sso-phase13-qa/run-consumer-contracts.ps1) 和 [capture-consumer-audit.ps1](../../../local-identity-sso-phase13-qa/capture-consumer-audit.ps1)。后者读取实际成功的 Rust 日志并重新计算源码／输出摘要，不执行数据库或 Cargo。Rust 在各自 Windows 仓库目录运行 `cargo test --locked --lib`，显式移除数据库 URL 环境，门店和工单分别通过 `STORES_TEST_NODE`／`WORK_ORDERS_TEST_NODE` 使用 Node 24.22.1 的自建 HTTP fixture。未运行的 ignored PostgreSQL 合同不计入本轮结果。

| 本轮合同 | 实际结果 | 涵盖的边界 |
| --- | --- | --- |
| SDK 生成器 `tools/build.mjs --check` | PASS，20 artifacts | 当前 generated／release 来源一致；没有重新生成或修改 vendor。 |
| SDK `tests/*.test.mjs` | 30 passed | 授权 HTTP、context epoch／expiry／scope、通知发布、v7、不可变归档及生成契约。 |
| 平台 `scripts/check-sdk.mjs`；`.github/tests/sdk-consumer-contract.mjs` | PASS；2 passed | 固定 SDK3 与 Rust 公开 DTO、逐文件摘要、修改发行字节必须拒绝。 |
| 门店 frontend `tests/client.test.mjs` | 74 passed | 当前 MicroProps／context、挂载与新上下文、过期／中止和 HTTP 客户端合同。 |
| 工单 frontend `tests/*.mjs` | 90 passed | SDK3／qiankun 与当前应用行为；不把业务 owner user ID 当作认证成员 ID。 |
| examples delivery／build-provenance | 4 passed | SDK3 classic 生命周期、来源／字节证明与不合法交付拒绝。 |
| chat lifecycle／HTTP；report-center；OAuth playground frontend | 7／8／18 passed | 挂载上下文、请求失败／中止、代码／设备流程与前端代理合同。 |
| 门店 Rust lib | 77 passed；0 failed／ignored；11.39s | legacy／combined 真实本地 HTTP transport、无成功缓存、无 fallback、有界响应／预算、成员／组织 witness。 |
| 工单 Rust lib | 71 passed；0 failed／ignored；3.34s | 独立操作授权、逐请求重验、无 fallback、context／scope 与 witness 到期。 |
| notification Rust lib | 36 passed；0 failed／ignored；0.01s | 私有成员 proof、公开 DTO、hint 绑定、调度有界及 transport 撤权。 |

以上各套测试独立，不是 417 个端到端场景。业务 PostgreSQL 测试中的 HTTP authority fixture 也不等于实际平台成员数据库；本轮没有冒用这些测试宣称新部署已切流。

## 活动消费者清单

这里的“活动”指当前代码和固定发行物支持的合同。登记项、任务流量和生产流量是不同事实；仓库私有 SDK 的 `REPOSITORY_CANDIDATE` 不代表已在外部包仓发布。

| 消费者 | 当前载体与边界 | 当前维护归属／退役处理 |
| --- | --- | --- |
| 平台员工 BFF | [bff/protocol.rs](../../src/bff/protocol.rs)、[context.rs](../../src/bff/context.rs)、[mod.rs](../../src/bff/mod.rs)：固定 `PLATFORM_CONSOLE`／`EMPLOYEE`，精确成员选择，原 sid／账号及成员代际／来源 client／policy 在线复验。OIDC refresh 留服务端。 | 平台仓；当前目标入口，保留。身份依赖失败不切本地密码模式。 |
| 独立客户门户 BFF | [protocol.rs](../../../../saas-customer-portal/backend/src/protocol.rs)、[resource.rs](../../../../saas-customer-portal/backend/src/resource.rs) 与平台 [customer_portal/store.rs](../../src/customer_portal/store.rs)：固定 `CUSTOMER_PORTAL`／`CUSTOMER`，与 resource client／app／tenant／org 交集；opaque context 留服务端。 | 门户仓＋平台客户资源 port；当前本人资料入口，保留。门户 DTO 不属于 SDK Context1，客户首版无通知、员工目录及 RBAC。 |
| 独立 identity | [lib.rs](../../../../saas-identity/src/lib.rs)、[internal.rs](../../../../saas-identity/src/internal.rs)：账号／SSO／标准 OIDC；稳定 issuer；私有 service Basic 与人类 proof 分开；不依赖平台或门户业务 crate。 | identity 仓；当前账号认证权威，保留。 |
| SDK3 宿主及微应用 | [release.json](../../../../saas-micro-app-sdk/release.json) 固定 SDK `3.0.0`／MicroProps `2.0`／Context `1.0`／Notification `1.0`；[http.mjs](../../../../saas-micro-app-sdk/src/http.mjs) 捕获本 context Bearer、`credentials: omit`；通知发布走 HTTP。 | SDK 仓及各 vendor 消费者；当前固定发行物保留。SDK 不登录、不 refresh、不创建通知 Socket.IO 连接。SDK1／2 仅不可变归档，不算活动兼容 runtime。 |
| stores API | [config.rs](../../../../saas-stores/backend/src/config.rs):217 默认 `legacy`；[authorization/client.rs](../../../../saas-stores/backend/src/authorization/client.rs) 的 legacy introspect＋context 与显式 combined `/authorize` 两种模式。 | stores 仓；两种当前 HTTP 合同保留。逐请求有界验证，错误直接传播，无自动 fallback；生产客户端不调用 `/oauth/token`。 |
| work-orders API | [config.rs](../../../../saas-work-orders/backend/src/config.rs):218 默认 `combined`；[authorization/client.rs](../../../../saas-work-orders/backend/src/authorization/client.rs) 仍允许显式 legacy。后端授权 HTTP `contractVersion: 1.0` 与前端 MicroProps2／SDK3 分别版本化。 | work-orders 仓；两种模式保留。创建／处理人／原处理人重验、actor 与目录 witness 的时效不放宽。 |
| notification 传输 | [protocol.rs](../../../../saas-notification/src/protocol.rs)、[platform.rs](../../../../saas-notification/src/platform.rs)、[server.rs](../../../../saas-notification/src/server.rs)、[store.rs](../../../../saas-notification/src/store.rs)：严格 `{token}`、私有在线 proof、持久原收件身份、每条投递及 emit 前后复验。 | notification 仓；保留。不登录、不签发／refresh；周期 proof 不换绑或续原期限。公开 ACK 5 字段和 notification 12 字段没有成员私有元数据。 |
| MACHINE／管理 CLI | [oauth2.rs](../../src/routes/oauth2.rs) 现有 confidential `MACHINE`／`LEGACY_MIXED` client credentials；[platform-admin/bootstrap.rs](../../tools/platform-admin/src/bootstrap.rs) 新登记严格 MACHINE＋空人类类型＋client_credentials；CLI 只接受输入 Bearer。 | 平台 OAuth 与 platform-admin 工具；当前机器／app 服务合同保留。CLI 没有登录、密码兑换或 refresh 功能；新 bootstrap 不等于已退役全部 LEGACY_MIXED。 |
| Vue／jQuery／报表 | [Vue](../../../../saas-examples/micro-vue/public/app.js)、[jQuery](../../../../saas-examples/micro-jquery/public/app.js)、[报表](../../../../saas-examples/report-center/src/app.tsx)：props.platform／app Bearer／omit／signal。 | examples 仓；当前接入样例保留，源码存在不证明生产调用量。 |
| 聊天样例 | [qiankun-lifecycle.ts](../../../../saas-examples/websocket/src/qiankun-lifecycle.ts)、[server.rs](../../../../saas-examples/websocket/backend/src/server.rs)：挂载期 SDK 授权；后端聊天室按匿名 socket.id 标识连接。 | examples 仓；保留。它不是 membership writer，也不是平台持久通知消费者，不把聊天室连接标识改成账号成员凭据。 |
| OAuth playground | [http.rs](../../../../saas-examples/oauth-playground/backend/src/http.rs)、[upstream.rs](../../../../saas-examples/oauth-playground/backend/src/upstream.rs)、[config.rs](../../../../saas-examples/oauth-playground/backend/src/config.rs)：legacy machine／code＋PKCE／device／refresh；password 默认关闭，各流程独立 client。 | examples 仓；演示合同保留。缺配置不广告能力，不把该样例当成标准 OIDC RP 或生产历史流量。 |

“归属”是仓库／组件边界，不代填用户未提供的生产个人 owner、外部客户端活动流量或停用日期。主阶段真实任务流量应以当次两 RP／机器 client／通知登记及 receipt 关联到本清单。

## 仍支持的 legacy 路径

| 路径 | 当前支持条件及撤销合同 |
| --- | --- |
| `POST /api/auth/login` → `POST /api/auth/context` | 旧员工登录／选择入口；BFF 已开启且不允许 legacy login 时拒绝。只签发唯一的有效 EMPLOYEE context；传 `membershipId` 精确选择或候选恰好一条后才取值，多行必须重新选择。不是客户门户旁路。 |
| `POST /oauth/token` 的 `psr_` 平台 refresh | [session.rs](../../src/session.rs) 与 [frontend/api.ts](../../../frontend/src/lib/api.ts)：真实路径仍是 `/oauth/token`，没有 `/api/auth/refresh`。原 platform session／源 client／成员及账号代际必须有效；BFF 禁止 legacy login 时此分支拒绝。不能把它与 OAuth refresh 或 `/oidc/token` 混成一种载体。 |
| legacy OAuth authorize／consent／authorization_code／device／password／refresh | `/oauth/*` 和 `/api/oauth/consent/*`；BFF 模式下人类入口须准确 client ID 在兼容 allowlist 内，grant／PKCE／client 资格另验。code／device／refresh 使用原存储 proof；缺失任一成员／类型／代际字段不补绑当前成员，重放及失效 grant 拒绝。password 新认证仍须唯一准确成员，password 不是 OIDC 推荐入口。 |
| app session token 与业务授权 | `POST /api/oauth/session-token` 从有效原 platform session 派生；app／来源登录 client 准入交集。`application_service_client_ref` 单独存储，机器 client 不扩大来源准入。stores／work-orders 的 introspect＋context 或 combined authorize 都必须命中原派生记录。 |
| legacy MACHINE client_credentials | 非人员 grant 仅在完整无人员 proof、client subject mode 和应用／client policy 匹配时有效；不借成员缺失降格成机器 token。 |
| legacy notification 行查询 | [routes/notifications.rs](../../src/routes/notifications.rs) 的 legacy 路由只承担显式归档查询合同；原无成员绑定行不自动绑定当前员工或客户，不进入普通 inbox／实时可见性。 |

当前兼容入口仍支持完整的新身份撤销边界；恢复或灰度回退不能恢复旧的 user＋org 唯一假设、缺失成员证明或停用前凭据。这里没有执行任何 legacy 退役。

## 双身份逐命中审查闭环

可重跑检索为 `rg -l 'memberships' backend/src -g '*.rs' -g '!**/*tests*' -g '!**/release_tests/**'`，再逐文件去掉尾部 `#[cfg(test)]` 区域，追踪 mutation、JOIN／fetch／LIMIT／首条访问、ON CONFLICT、触发器和广播最终定义。当前 **29 个生产区域含 memberships 字面量的文件**全部在 evidence 中列明；这个计数包含配置和路由名，不把它们当作 SQL。早期准备记录的 28 不是当前该查询结果。

生产成员 mutation 为 **5 文件／11 个 SQL 位置**：lifecycle service 436／551／634，onboarding 210，users 561／1217／1241／1325，remote 206／244，seed 1336。`ON CONFLICT(user_id,organization_id,member_type)` 成员写入仅 onboarding 210 和 users 1242 两处，均为三元目标；未发现当前生产二元成员 UPSERT。

| 逐命中文件组 | 定位与结论 |
| --- | --- |
| [lifecycle/service.rs](../../src/membership_lifecycle/service.rs) | user＋org 读取采用 fetch_all 并锁全部行，随后按源 membership ID／目标 type 定位。同类型重复409，第二身份与替换为不同动作，写入不原地改变 type；启停按 ID bump identityVersion，删除精确 ID。 |
| [users.rs](../../src/routes/users.rs)、[remote.rs](../../src/routes/users/remote.rs) | 员工 bulk 查询／DELETE／三元 UPSERT 均限 EMPLOYEE，重复 org先拒；保留原 ID、状态和版本。准确员工自行离开延后至最终 proof。全账号 RETIRE 无 type 过滤符合全局语义，但只删除命令接收时捕获的 `(id,identityVersion)`，不领养后续创建／变更的另一行。 |
| [onboarding.rs](../../src/routes/onboarding.rs)、[seed.rs](../../src/seed.rs) | 显式 EMPLOYEE 写入、v7；onboarding 三元 DO NOTHING 后重新核有效员工，不把仅有客户当员工角色资格；seed 不生成客户或隐式第二身份。 |
| [auth.rs](../../src/routes/auth.rs)、[oauth2.rs](../../src/routes/oauth2.rs) | 展示可包含不可选成员；选择票据的 accounts.first只作签名锚点。真正 context／password grant 先按准确成员、类型、状态、client allowed集合过滤，候选数不是1即拒，之后才 `[0]`／remove(0)。不能拿首个关系猜身份。 |
| [oauth_issuance.rs](../../src/oauth_issuance.rs)、[app_token.rs](../../src/app_token.rs)、[app_authorization/store.rs](../../src/app_authorization/store.rs) | 人员凭据 JOIN 定位原 membership ID，核 user／tenant／org／type／身份及账号代际／app／来源 client／父 session。StoredBinding 任一必需字段为 NULL 即失败，不能由当前 user＋org 关系“修复”。机器路径必须全无人员元数据且匹配机器用途。 |
| [authorization.rs](../../src/authorization.rs)、[state.rs](../../src/state.rs)、[roles.rs](../../src/routes/roles.rs)、[application_setup.rs](../../src/routes/application_setup.rs) | 角色／超管读取在原完整 session proof 后限制 EMPLOYEE；state effective_roles 的 user＋org EXISTS 也明确 ENABLED EMPLOYEE。祖先角色支持集合与最终员工清理不让 CUSTOMER 支撑员工角色。 |
| [data_grants/store.rs](../../src/data_grants/store.rs)、[discovery.rs](../../src/data_grants/discovery.rs)、[app_members.rs](../../src/routes/app_members.rs) | 数据授权写具体 membership；核目标及actor完整身份。目录候选仅有效直接员工；DTO 的 userId 是业务主体，不能反向当成员证明。 |
| [bff/context.rs](../../src/bff/context.rs)、[customer_portal/store.rs](../../src/customer_portal/store.rs) | 员工／客户 RP分别固定类型，与 client／app／tenant／org 精确相交。资源变更前重验原 source／账号／成员代际，客户不能拿全局认证代替组织准入。 |
| [notifications.rs](../../src/notifications.rs) | 发布者精确 session／membership／version；APP受众过滤 ENABLED EMPLOYEE。SYSTEM按 user／tenant／org 去重后 INSERT SELECT解析唯一员工ID／version；三元唯一键使该明确类型至多一行。同账号客户不产生第二条员工收件。publication保存发布者成员及代际，收件原 proof 与 hint不可升级。 |
| [organizations.rs](../../src/routes/organizations.rs)、[tenants.rs](../../src/routes/tenants.rs)、[search.rs](../../src/routes/search.rs)、[monitor.rs](../../src/routes/monitor.rs) | 展示／统计或组织级删除依赖：人数用 DISTINCT user，关系计数可为双行。搜索 LIMIT1只作名称展示，范围和员工资格另验，不签凭据。实体级删除是该实体全部关系语义。 |
| [database/identity_readiness.rs](../../src/database/identity_readiness.rs)、[schema.rs](../../src/database/identity_readiness/schema.rs) | catalog验证三元唯一、immutable类型、版本单调及独立收缩约束；runtime不自动执行 migration。schema安全回退不能重建二元唯一。 |
| [config.rs](../../src/config.rs)、[lifecycle/routes.rs](../../src/membership_lifecycle/routes.rs)、[customer_portal/mod.rs](../../src/customer_portal/mod.rs) | 配置／HTTP route／DTO清单，不是额外SQL writer；正式客户／第二身份开关仍默认false。客户 port输入和 source proof交给上表实际 store校验。 |

另行追踪没有直接写 `memberships` 字面量的关键消费者：

| 范围 | 当前有效定义及结论 |
| --- | --- |
| [routes/notifications.rs](../../src/routes/notifications.rs) 的共用 VISIBLE／prefs；[notification_context.rs](../../src/notification_context.rs) | 列表、详情、改删、read、delivery proof使用原session与通知 recipient_membership_id／member_type／identity_version相等，prefs按成员＋代际。NULL／旧代及客户不能用 user＋org 取得员工行。 |
| [0013](../../migrations/0013_membership_identity_policy.sql)／[0014](../../migrations/0014_credential_identity_binding.sql)／[0015](../../migrations/0015_notification_member_binding.sql) | identity不可改写、版本单调；成员删除／变更trigger按 OLD.id 局部撤销派生；通知收件字段immutable，业务publication幂等不是成员二元键。 |
| [0018](../../migrations/0018_platform_bff.sql)／[0020](../../migrations/0020_identity_deployment.sql) | platform_session_active以0018替换后的定义为准，0020再替换来源验证与账号bridge。账号代际按user全局撤销符合全局语义；历史0001／0013函数不是当前最终生效定义，不改其历史字节。 |
| [ops/identity-recovery/recovery.py](../../ops/identity-recovery/recovery.py):291 | deny-only对账按原 membership id定位，核user／org／type不变，删除缺失旧id，状态交集及不下降generation。没有按user＋org从另一身份找替代，也不插入checkpoint后新增权限。实际恢复／fence由主阶段另验。 |
| CLI／八仓其他生产代码 | 业务仓无平台memberships写入权限或写入器；identity不读组织RBAC表，portal经私有resource port验证。UUID专项importer及测试COPY／fixture独立归类，不当本新库历史导入入口。 |

这个闭环说明检索命中为何安全或为何不承担授权，不凭一次 grep 零结果宣称授权成立。成员局部／全局撤销、实际旧载体拒绝和最终数据库 trigger catalog仍须与本阶段 live receipt关联；相关已有集成测试来源在 [phase-13.md](validation/phase-13.md) 的 R1～R10交叉表中可追溯，本轮没有把那些测试名计为已执行。

## 退役决议与后续客户范围

本 TASK_OWNED 身份库是新库，没有导入历史账号、凭据或外部 RP旧主体，也没有已登记并计划退役的历史客户端。因此本范围的历史客户端迁移窗口、停用日期、原账户分类和旧主体映射均为 **不适用**。依据是台账明确范围、以上当前消费者与兼容合同清单，以及本阶段独立任务client fixture；不是声称所有其他部署都没有历史调用。未提供的生产client流量、个人owner和日期不编造。

当前活动 SDK／app派生／业务授权／机器grant／CLI／playground和受控legacy入口继续支持；本次没有因“历史退役不适用”停用它们。全量生产开放、真实生产SLO／备份PITR及外部资产仍按主台账边界处理。首个客户业务仍限本人资料、已有客户成员选择和已登记应用目录；订单／工单客户owner／actor、客户通知事件和路由须在确定资源及动作后另行实施。
