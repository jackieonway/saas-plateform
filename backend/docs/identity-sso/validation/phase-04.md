# Phase 4：通知受众、历史隔离与实时撤权验收

日期：2026-10-24。状态：**COMPLETED**。最终平台格式与 all-targets Clippy PASS；修复 UUID 目录门禁后，九项通知专项及 UUID 用例 **10／10 PASS，13.84 秒**；普通测试 **185 PASS**，文档检查 PASS。通知服务独立门禁为普通 **36 PASS**、隔离集成 **3 PASS**，格式、检查、Clippy、二进制构建和文档检查均 PASS。65 项相关数据库用例在分组重跑后逐项有通过证据；保留首轮 64 PASS／1 FAIL 的真实记录，不宣称单次 65 项全绿。

阶段范围与四项验收标准见[主台账 Phase 4](../task-ledger.md)；设计与行为说明见[通知的成员绑定与实时权威证明](../phase-04-notification-binding.md)。本文只记录本阶段通知改动及实际证据，不接管独立 UUID 专项，也不把工作区先前的部署、SDK 或其他改动全部归于本阶段。真实环境客户／第二身份创建仍关闭，未发布 Phase 6 收缩迁移。

## 实际交付

### 持久收件证明与发布受众

[0015 增量迁移](../../../migrations/0015_notification_member_binding.sql)为 `notifications` 增加 `recipient_membership_id`、`recipient_member_type`、`recipient_identity_version`，为 `notification_publications` 增加发布成员的对应三列。三列必须全部为空或完整；完整 proof 的成员 ID 必须为 UUID v7，类型只能是 `EMPLOYEE`／`CUSTOMER`，身份版本至少为 1。两个软引用 UUID 列各自具有独立 `is_uuid_v7` CHECK，供现有逐列目录门禁精确识别。通知归属、recipient proof 及 publication proof 创建后不可改绑。

成员证明是保留原值的软引用，没有指向 `memberships` 的删除级联外键。成员离职、删除、重新创建或代际变化不会把旧消息自动挂给新成员。既有账号、租户、组织与应用整体删除的级联语义继续按原合同执行。

APP 投递唯一键为 `(publication_id, recipient_membership_id, recipient_identity_version)`。发布幂等唯一键同时加入发布成员 ID 与身份版本；事务 advisory namespace 精确使用 `[tenant, organization, application, user, publisherMembership, publisherIdentityVersion, idempotencyKey]`。相同成员代际重试保持原结果，不重建已删除的 delivery；新身份不能借旧发布者的幂等记录。

[发布核心](../../../src/notifications.rs)的直接目标和组织／租户广播只展开当前有效员工；同账号同组织客户行不提供员工资格，也不增加投递数。`recipientCount` 继续按不同账号计数，`deliveryCount` 按具体成员计数，同账号在不同组织的有效员工关系可分别接收。旧 SYSTEM 三元上下文 helper 只解析有效员工，先去重再写入完整 proof；客户专属用途尚未登记，公开发送入口不能发送客户广播或借员工账号权限发送。

控制台发布与 app token 发布均要求应用启用并明确允许 `EMPLOYEE`，同时保留当前应用菜单／权限及独立数据范围验证。补充的控制台回归先保留有效管理权限与数据授权，再将目标应用改为仅允许 `CUSTOMER`；`POST /api/notifications` 必须返回 403，publication、delivery 和成功审计快照不变。该用例用于直接验证不经过 app token 的控制台入口。

### HTTP 收件箱及锁等待后的实时复验

[通知专用 context](../../../src/notification_context.rs)只从服务器 session 记录取得具体账号、租户、组织、成员、类型、身份版本与账号版本。HTTP 请求不能提交另一个成员 ID 或版本来覆盖该绑定。通用员工管理 `CurrentUser` 继续执行员工门禁。

[通知路由](../../../src/routes/notifications.rs)为列表、详情、未读数、已读、单条／批量删除以及 socket delivery 使用相同可见性条件：具体 session 必须有效，recipient proof 必须精确匹配当前成员与身份版本。APP 消息额外要求应用启用、租户及组织授权仍有效，并允许当前成员类型。跨身份详情／删除返回 404；跨身份标记已读不修改记录；批量读写只影响当前身份可见行。

读取接口返回前重新验证当前 session。修改接口在事务内执行，通知行或偏好行的真实锁等待结束后，用独立 READ COMMITTED 语句重新验证 session、成员与账号版本；APP 修改还重新验证受影响应用的当前准入。任一条件已撤销则事务整体回滚，不保留已读、删除、偏好或成功审计的部分变更。数据库权威状态不可确认时返回 503。

新专项为单条已读、批量已读、全部已读、单条删除、批量删除和偏好更新设置真实行锁屏障，通过 `pg_stat_activity` 与 `pg_blocking_pids` 确认 HTTP 操作已等待，再提交成员或账号撤销。另两组在 APP 通知行等待期间撤组织应用授权或收紧允许成员类型，要求 403、完整快照不变，并确认一般成员 session 仍有效，避免把应用撤权误当成账号撤销。

### 旧结构归档与成员代际偏好

0015 不对已有通知或 publication 补填 proof。旧 LEGACY 行、带账号／组织的旧 SYSTEM 行以及旧 APP delivery 都保持原始字段和空 proof；当前成员关系不能证明它们原来的身份。普通收件箱、详情、已读、删除及 socket replay 均不展示或修改这些行。

`GET /api/notifications/legacy` 保留平台员工及 `system:audit` 检查，只提供 `source_kind='LEGACY'` 或 recipient proof 为空的归档记录。升级专项先应用到 0014，写入旧结构通知／publication，再执行真实 0015，逐项比较原值并验证双方身份均不能取得旧行的 delivery proof。本身份功能使用新库，没有真实历史消息分类、搬迁或回填任务；这项行为由隔离 fixture 验证。

`notification_member_prefs` 以 `(membership_id, identity_version)` 为主键，身份键不可更改。默认免打扰关闭，时间为 `22:00–07:00`，静音类型为空。旧 `notification_prefs` 原样保留，不复制到任何新成员。不同组织、同组织员工／客户、成员重新启用后的新身份版本相互独立；相同成员代际的新 session 仍读取原偏好。偏好公开 JSON 继续只有 `dndEnabled`、`dndStart`、`dndEnd`、`mutedTypes` 四字段。

账号全局版本用于使该账号所有旧凭据／连接失效，不写入持久收件箱的分代键。账号版本提升后，新签发且成员代际未变的有效 context 仍能读取该成员原有消息；成员自身换代后则不能读旧代消息或继承旧代偏好。

### 固定公共协议与私有实时适配

Notification 1、Context 1、MicroProps 2 及其已发布 SDK 生成副本保持原合同。Socket.IO 认证输入仍为 `{token}`；`authenticated` 精确五字段；`notification` 精确十二字段：`contractVersion`、`id`、`userId`、`tenantId`、`organizationId`、`applicationId`、`publicationId`、`type`、`title`、`content`、`link`、`createdAt`。持久层的 `sourceKind` 和内部 proof 不进入该公开 payload。

原 `GET /api/notifications/socket-context` 仍精确返回六字段：`contractVersion`、`userId`、`tenantId`、`organizationId`、`sessionId`、`expiresAt`。新私有 `GET /api/internal/notifications/context-proof` 携带相同 session bearer，可带 `notificationId`；返回精确十一字段：

```text
proofVersion="1", contractVersion="1.0",
userId, tenantId, organizationId, sessionId, expiresAt,
membershipId, memberType, identityVersion, accountVersion
```

UUID 必须是规范 UUID v7；`identityVersion` 为至少 1 的 JSON i64 整数，`accountVersion` 为至少 0 的 JSON i64 整数；有效期限取 JWT 与数据库 session 的较早值。指定消息时再次执行当前成员可见性验证，不可见返回 404，无效／撤销 session 返回 401，权威不可用返回 503。

数据库私有 `new_notification` 提示改为精确八字段：`hintVersion="1"`、`id`、`userId`、`tenantId`、`organizationId`、`membershipId`、`memberType`、`identityVersion`。旧四字段提示、未知类型、非整数／非法版本及缺失／多余字段均拒绝，不按现有账号关系补 proof。

独立通知服务的[协议](../../../../../saas-notification/src/protocol.rs)、[权威 HTTP 适配](../../../../../saas-notification/src/platform.rs)、[存储读取](../../../../../saas-notification/src/store.rs)与[连接投递](../../../../../saas-notification/src/server.rs)使用完整成员证明。队列按账号、租户、组织、成员、类型及身份版本分区；每条提示先从持久行读取同一 recipient proof，再调用平台验证指定消息的可见性。提示不作为授权依据，也不提供持久重放；断线恢复从 HTTP 补拉。

首次认证和周期复验失败终止连接，逐条投递权威失败拒绝该条消息；不缓存正向授权来跨过故障。连接固定绑定成员 proof、session 与账号版本，不允许改绑，复验不延长首次有效期。局部成员撤销不影响同账号仍有效的另一身份；账号版本提升撤销全部身份连接。通知服务不持有平台签名密钥、不写收件箱、不运行平台迁移、不查询账号或成员表；专用只读角色增加的只是三个通知 proof 列的 SELECT 权限。

## 证据边界与隔离 fixture

三层测试分别证明不同边界，不能互相替代：

| 层次 | 实际组成 | 可证明范围 |
| --- | --- | --- |
| 平台前八项专项 | 当前真实迁移、独占 PostgreSQL、自有数据库、真实 Router HTTP 调用 | 平台 SQL 的成员隔离、迁移归档、发布展开、生命周期、偏好及事务末端实时复验 |
| 通知服务三项集成测试 | 真实通知服务二进制、隔离 PostgreSQL 的最小通知表、Engine.IO／Socket.IO polling、受控 HTTP authority | 消费者对 proof／hint、连接绑定、周期复验及错误响应的行为；fixture bearer 与人工会话映射不证明平台自己的 JWT、SQL、迁移或真实生命周期实现 |
| 平台第九项真实双服务联调 | 真实平台 Router 监听 loopback HTTP、当前独立通知服务二进制、真实数据库 LISTEN／NOTIFY、Engine.IO／Socket.IO polling | 实际平台发布和成员状态 API、内部权威 HTTP、在线投递、局部撤销、重连与旧代 replay 的服务间链路 |

[平台专项](../../../src/notification_identity_tests.rs)只读取显式 `IDENTITY_PHASE04_TEST_DATABASE_URL`，要求管理库名为 `postgres`，每项创建并清理 `identity_phase04_<v7>` 自有库；不读取业务 `.env` 或生产连接。仅在双身份测试的目标 schema 准备中去掉旧二元唯一约束，已发布迁移及真实环境继续保留扩展期约束。客户 session／app token 以完整成员与来源证明构造，拒绝用例先验证相应 fixture 凭据有效。

第九项还要求显式绝对路径 `IDENTITY_PHASE04_NOTIFICATION_BINARY`，无旧二进制 fallback。通知进程清空继承环境，只接收测试所需配置；启动失败路径和退出路径回收平台监听任务、子进程与输出线程。客户通知由隔离 fixture 写入明确 CUSTOMER proof 并发送私有提示，用于证明客户收件与在线撤销边界；不据此声明客户公开发送用途已开放。这项实际 HTTP／Socket.IO polling 联调也不代表生产部署、浏览器 WebSocket 升级、容量或多实例验收。

[通知服务集成测试](../../../../../saas-notification/tests/identity_isolation.rs)额外要求 `CI_TEST_POSTGRES_OWNER` 为合法 `ci-` 标识，数据库 URL 只接受 loopback PostgreSQL、显式 `/postgres`、无 query／fragment。它们自行建库／删库，不接受普通 `TEST_DATABASE_URL` 作为隐式替代。独立通知仓最终三个 fixture 均已清理。

本轮沿用独占 PostgreSQL 容器 `identity-sso-01a16e22-pg` 与 Linux 隔离编译环境；连接、密码、原始 token、secret 和 fixture 密钥不写入本文。执行结果保存在本会话工具输出，未另存日志文件，不编造文件路径。65 项相关回归的唯一失败已经修复并重跑通过；最终目录审计仅有系统 postgres 库，通知临时角色数量为零。测试基础设施保留供后续阶段使用。

## 专项覆盖与执行状态

平台专项统一前缀为 `notification_identity_phase04_`。首轮九项共 11.03 秒，前八项通过，第九项在公开 payload 精确字段检查处失败。修正后九项随 65 项相关回归全部 PASS，包括新增控制台应用准入回归；其后仅 UUID 目录用例失败。独立 UUID CHECK 修正后，九项及 UUID 门禁最终 10／10 PASS，13.84 秒，下表保留已发生的实际结果。

| 专项后缀 | 关键断言 | 实际执行进展 |
| --- | --- | --- |
| `employee_audiences_deduplicate_and_emit_bound_hints` | 同账号同组织员工／客户及多组织受众去重；有效客户凭据不能借员工资格发布；SYSTEM helper 只解析有效员工；publication／hint 绑定具体成员；控制台 CUSTOMER-only 应用拒绝 | 65 项回归中 PASS（含新增控制台回归）；最终重跑 PASS |
| `inbox_mutations_and_legacy_archive_never_cross_identity` | 员工／客户／另一组织各自收件箱；跨身份详情、已读、批量操作及删除不越界；旧 NULL 行仅供平台审计 | 65 项回归中 PASS；最终重跑 PASS |
| `public_payload_and_internal_context_proofs_are_separate` | 公开六字段与私有十一字段严格分离，整数版本与 DB proof 一致；跨身份 delivery proof 拒绝 | 65 项回归中 PASS；最终重跑 PASS |
| `upgrade_preserves_unbound_rows_without_guessing_recipients` | 真实 0014→0015 升级保留旧行；不猜测收件成员，不允许当前员工或客户重放旧无 proof 消息 | 65 项回归中 PASS；最终重跑 PASS |
| `local_departure_and_replacement_preserve_other_receipts` | 员工／客户双向局部停用及替换；其他身份与组织保留；旧凭据失效，新代不能重放旧代消息 | 65 项回归中 PASS；最终重跑 PASS |
| `preferences_are_exact_member_generation_and_keep_wire_shape` | 旧账号偏好不复制；同组织双身份、多组织及新代偏好独立；保留四字段 JSON | 65 项回归中 PASS；最终重跑 PASS |
| `account_revocation_and_authority_failure_fail_closed` | 账号版本提升撤销全部旧身份凭据；新凭据不丢同成员代际收件箱；数据库不可用时读写／proof 全部 503 且快照不变 | 65 项回归中 PASS；最终重跑 PASS |
| `mutation_waits_recheck_member_and_account_before_commit` | 六种真实锁等待后的成员／账号复验，另测 APP 撤授权与类型收紧；失败后通知、偏好、publication 与审计无部分提交 | 65 项回归中 PASS；最终重跑 PASS |
| `actual_platform_and_external_socketio_keep_receipt_identity` | 两个真实服务：APP 发布→DB hint→Socket.IO；同账号双身份收件分离；员工局部撤销、客户存活、新代恢复、反向撤客户及旧代 replay 拒绝 | 首轮 FAIL：测试 allowlist 误含 `sourceKind`；修正测试后重跑 PASS，实际十二字段 payload 未改 |

通知服务三个默认 ignored 集成测试已实际串行运行，**3 PASS，8.65 秒**：

| 测试 | 关键断言 | 结果 |
| --- | --- | --- |
| `dual_identity_delivery_local_revocation_replacement_and_reconnect` | 同账号双身份精确投递、单侧撤销、身份替换、另一身份存活、重新认证／重连拒绝 | PASS |
| `legacy_hints_null_recipient_proofs_and_authority_faults_fail_closed` | 旧／畸形 hint、空持久 proof、权威 proof 错配、503 及旧公开 context 不能授权 | PASS |
| `periodic_member_and_account_proof_changes_close_without_a_hint` | 无提示情况下，周期检查发现成员／类型／身份版本／账号版本变化后关闭连接，失效凭据无法重连 | PASS |

### 首轮失败与修复

通知服务最初 `cargo check` 失败于测试导入 `axum::extract::Query`，该仓当前 axum 未启用 `query` feature。修复仅把测试 authority 的 query 读取改用已有 `Uri`，没有新增 feature 或修改生产依赖。之后格式、检查、Clippy、普通及三项数据库集成测试通过。

平台首轮真实双服务用例期望 Notification 1 含 `sourceKind`，与已发布十二字段合同不一致。实际独立通知服务发出的十二字段正确；修复删除测试 allowlist 的错误字段，未改变生产 payload 或固定 SDK。修复后九项在相关综合回归中均已实际 PASS。

独立审查还发现控制台发布的应用类型准入原只在 app token 分支检查。发布核心将 `EMPLOYEE` 准入加入两种发布入口共用的应用查询及 fanout 查询，并增加上述无 app token 控制台 403 回归。双服务 fixture 的进程启动失败清理也已补齐。两项变更均已纳入随后通过的九项专项。

65 项相关数据库回归实际为 **64 PASS／1 FAIL，172.30 秒**；旧通知、会话、管理审计、历史升级及新九项均通过。唯一失败是 `release_tests::database_upgrade::uuid_v7_target_enforces_versions_variants_and_application_generated_ids` 的列数断言仍固定为 116，当前真实目录已经有 119 个 UUID 列。0015 新增 recipient、publisher 软引用及成员偏好的三个 UUID 列；其中两个 soft reference 原本在三元 proof CHECK 中已有 UUID v7 约束，但现有目录门禁要求每列可单独识别的约束。

修复为两个 soft reference 增加各自独立的 `is_uuid_v7` CHECK，并将测试列数精确更新为 119；不改成范围判断，不移除精确列集合或非法版本／variant 负例。修复后的 UUID 用例及九项通知专项最终 **10／10 PASS，13.84 秒**。失败的相关回归命令当时未执行后续普通测试与文档检查；最终独立命令已实际执行：普通测试 **185 PASS**（library 9、binary 163、external 13），文档检查 **PASS，18.67 秒**。

### 专用数据库角色的实际权限验收

另创建自有空库 `identity_phase04_role_audit`，使用 `psql` 按真实 SQL 文件顺序执行 0001～0015，再执行原 `notification-role.sql`。实际 `SET ROLE` 后三个新增 recipient proof 列均可读取；`users`、`memberships`、通知偏好表不可读取，通知表没有写权限，schema 没有 CREATE 权限，角色 `default_transaction_read_only=true`。用例结束后自有数据库与本轮新建角色均已删除。

该验证是**真实 SQL 链及角色权限验收**，不宣称写入 SQLx migration history；独立的本轮 SQLx 空库基线另已 PASS，检查到 **119 个 UUID 列、64 个外键、零 UUID 默认生成表达式**。首轮角色验收与此基线发生于新增独立 UUID CHECK 之前；修正后的真实 0001～0015 SQL 链和角色权限已再次运行并全部 PASS，自有数据库与本轮新建角色均已清理。角色验收辅助脚本仅保存在私有 TEMP，名为 `identity-sso-01a16e22-notification-role-audit.sh`，不包含凭据，不冒充仓库提交文件。

## 已执行门禁

| 仓库／门禁 | 实际结果 |
| --- | --- |
| 通知服务格式检查 | PASS |
| 通知服务 `cargo check`（上述测试导入修复后） | PASS，1.36 秒 |
| 通知服务 Clippy | PASS，2.83 秒 |
| 通知服务普通测试 | 36 PASS |
| 通知服务 `identity_isolation` 三项 ignored 集成 | 3 PASS，8.65 秒，fixture 全部清理 |
| 通知服务二进制构建 | PASS，产物供平台第九项显式选择 |
| 通知服务文档检查 | PASS，2.28 秒 |
| 平台生产检查 | PASS，25.02 秒 |
| 平台 all-targets 检查 | PASS，45.38 秒 |
| 平台 all-targets Clippy | PASS，58.98 秒 |
| 平台九项 Phase 4 专项首轮 | 8 PASS／1 FAIL，11.03 秒；失败及修复如上 |
| 平台 65 项相关数据库回归 | **64 PASS／1 FAIL，172.30 秒**；新九项全部 PASS，唯一失败为 UUID 目录精确列数期望过时 |
| 真实平台 HTTP＋独立通知服务 Socket.IO 第九项重跑 | PASS；实际十二字段公开 payload 保持不变 |
| 专用通知数据库角色真实 SQL 链／权限 | 独立 CHECK 修正前及修正后均 PASS；三个 proof 列可读，账号／成员／偏好不可读，无通知写及 schema CREATE，默认只读；自有库／新角色已清理 |
| 本轮 SQLx 空库 UUID 基线 | 独立 CHECK 修正前 PASS，119 UUID 列、64 外键、零 UUID 默认生成表达式 |
| 最终 UUID 用例＋九项通知专项重跑 | **10／10 PASS，13.84 秒**；最终专用 PG 仅有 postgres，临时通知角色为零 |
| 平台普通测试与文档检查 | **185 PASS**（9 library＋163 binary＋13 external）；binary 13.07 秒；文档检查 PASS，18.67 秒 |
| 最终平台格式／all-targets Clippy | PASS；Clippy 32.65 秒 |
| Node `notification-context` 合同 | 9／9 PASS |
| Pinned SDK 检查 | PASS，固定发布副本未修改 |

以下是可复现的专项入口，需先按各自 fixture 合同显式注入独占测试配置；普通 `cargo test` 不执行这些 ignored 数据库用例：

```sh
# saas-plateform/backend；第九项另需显式通知服务二进制绝对路径
cargo test --locked --offline --bin saas-platform-backend notification_identity_phase04 -- --ignored --test-threads=1 --nocapture

# saas-notification；需合法 CI_TEST_POSTGRES_OWNER 与 loopback 管理连接
cargo test --locked --test identity_isolation -- --ignored --nocapture --test-threads=1
```

## 四项验收对照

| 台账验收标准 | 当前证据 | 当前结论 |
| --- | --- | --- |
| 客户不能经列表、详情、已读、补拉或已有连接取得员工广播／定向消息；旧结构以 fixture 验证 | 平台受众／收件箱／升级专项和真实双服务已 PASS；通知服务双身份集成通过 | PASS：最终 UUID 与九项专项重跑通过 |
| 替换／停用后旧连接、重连与重放不能恢复资格，另一有效身份可继续收件，反向同理 | 平台双向生命周期专项及真实双服务已 PASS；通知服务局部撤销／重连与周期复验通过 | PASS：最终 UUID 与九项专项重跑通过 |
| 同组织双身份切换不混用消息、已读、广播及路由，不重复投递 | 受众计数、精确收件箱修改、成员代际偏好及 link／payload 均 PASS，新增控制台准入回归已通过 | PASS：最终 UUID 与九项专项重跑通过 |
| 权威故障拒绝继续授权，Notification 1 消费者合同不变 | 平台全路由 503／无变更专项已 PASS；通知服务故障拒绝及严格协议集成通过；Node 9／9 与 Pinned SDK 检查通过 | PASS：最终 UUID 与九项专项重跑通过 |

根代理已逐项核对四项验收；Phase 4 完成，主台账及导航已推进 Phase 5。这里的真实服务联调、容量／部署边界与失败重跑记录均保留；未把本阶段验证扩张成后续 OIDC、客户门户或生产部署验收。
