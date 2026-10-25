# Phase 3：员工授权、目录与双向授权入口隔离验收

日期：2026-10-24。状态：**COMPLETED／PASS**。七项验收标准全部通过。平台九项员工隔离专项最终 **9／9 PASS**，13.51 秒，逐项 fixture 清理均成功；经全量失败修复与分组重跑，本阶段 **160 个 ignored 数据库用例每项均已有实际 PASS**。最终普通测试 **185 PASS**，格式／检查／Clippy／文档及固定 SDK／仓库边界合同通过，独立 PostgreSQL 集群只剩 `postgres` 维护库。本文保留失败与重跑经过，不将分组覆盖写成单次 160 项全绿。

阶段范围与七项验收标准见[主台账 Phase 3](../task-ledger.md)。本证据只记录员工授权、目录、角色及数据授权隔离，不接管独立 UUID 专项，不将工作区既有 SDK、部署、恢复等修改全部归于本阶段。没有发布 Phase 6 收缩迁移，也没有开放真实环境的客户／第二身份创建。

## 实际交付

### 平台资格与授权投影

- [员工授权模块](../../../src/authorization.rs)增加池内与事务内的实时 `FreshEmployee` 证明，按当前 `session_id` 核验具体成员、身份版本、账号版本和来源策略。员工检查先于角色继承与超级管理员分支；超级管理员状态从权威账号读取，不信任先前提取的 `CurrentUser.is_super_admin`。
- [平台状态门面](../../../src/state.rs)、[认证路由](../../../src/routes/auth.rs)及适用 OAuth 投影使用当前员工 context 计算权限、菜单、角色和平台管理范围。同账号同组织存在客户行不会否决合法员工，客户 context 也不能借员工行证明资格。
- [应用令牌](../../../src/app_token.rs)在内部返回具体成员及版本；[应用授权用例](../../../src/app_authorization.rs)与[存储适配](../../../src/app_authorization/store.rs)在读取员工角色／授权前拒绝客户，并按令牌已绑定的 `membership_id` 读取数据授权。身份绑定进入内部快照与 context 修订计算，保持既有公开 Context／MicroProps 字段合同。
- [数据授权](../../../src/data_grants.rs)、[存储](../../../src/data_grants/store.rs)和[选项查询](../../../src/data_grants/discovery.rs)将可选 target 限定为真实、有效的员工成员；客户成员 ID、停用员工、错误账号／组织／租户组合均不能获得员工数据授权。事务内重新核验 actor、target 及其版本。

### 员工维护与目录

- [用户管理](../../../src/routes/users.rs)和[角色管理](../../../src/routes/roles.rs)保留现有 user ID／组织 DTO，在内部以有效员工关系证明目标资格；成员→角色和角色→成员两个方向使用相同规则。角色列表、用户数及角色投影不把客户或停用员工配置作为有效资格。
- 临时停用可以保留 `user_roles` 配置，但配置不再授权，也不出现在有效员工角色投影中。永久删除、显式身份替换及 bulk 移除通过[生命周期服务](../../../src/membership_lifecycle/service.rs)计算操作完成后的有效员工支持集合，只清理无支持的相关组织／祖先角色；不按账号清空其他组织授权或会话。
- 旧无类型 bulk 只维护 `EMPLOYEE`，以三元键处理冲突并保留客户。保留的员工 ID、状态及身份版本不因旧请求自动启用；仅有客户时，只有隔离目标 schema 与双身份开关允许才可新增独立员工行。重复组织输入是 `409`，相同最终集合的并发保存仍保持每类至多一行。
- 旧 bulk 保留合法的员工自行退出合同。当前 actor 的员工成员删除与最终角色清理延后至其他 DML／审计和全部已存在的凭据／配置子行等待之后；仍有源成员时最后重验实时范围及 `system:user`，随后才精确删除并清理。客户成员及其会话／刷新凭据不进入该删除范围，也不放宽生命周期显式自行停用保护。
- [应用成员目录](../../../src/routes/app_members.rs)的 count、分页列表和单人直接 ID 校验均明确限定 `EMPLOYEE + ENABLED`。客户成员 ID 不能冒充旧 DTO 的 user ID；只有客户关系或员工已停用的账号不能成为员工候选，双身份账号的有效员工只出现一次。actor 仍需通过当前应用的操作权限及独立数据范围验证。

### 门店与工单消费

门店联系人与工单处理人继续以平台受控成员接口为权威，不增加跨仓业务 crate 依赖，也不根据浏览器提交的类型推断员工资格。客户端／服务端合同均区分 actor 与 target：客户 actor 在员工授权阶段被拒绝，合法员工 actor 仍须对最终非空目标 ID 做单人校验，历史候选列表不能替代写入时复核。

- [门店成员合同](../../../../../saas-stores/backend/docs/member-contract.md)明确三个目录用途、同组织双身份、仅客户／停用员工／客户成员 ID 拒绝规则，以及联系人保留、显式清空与写入失败无业务变更的行为。
- [工单 API 合同](../../../../../saas-work-orders/backend/API.md)明确 `/assignees` 与最终 `assigneeUserId` 的员工要求；显式相同非空 ID 仍重新校验，省略与显式 null 保持原有线协议含义。平台校验和业务提交之间不宣称跨库原子事务。

本阶段工单实际修改为 `backend/src/writes.rs`、`backend/tests/member_contracts.rs`、`backend/tests/write_contracts.rs` 与 `backend/API.md`；成员失效、同值非空 ID、到期、SELF 边界及目录故障时的完整回滚进入合同。门店本阶段仅修改 `backend/tests/postgres_contracts.rs` 和 `backend/docs/member-contract.md`，没有修改门店生产实现；既有联系人写入链由增加的 typed fixture 与实际 HTTP／数据库测试重新验证。工作区其余先前变更不据此归入本阶段。

两仓真实 PostgreSQL 测试使用受控平台 HTTP fixture 验证消费行为。该 fixture 不能证明平台自身 SQL 的成员类型隔离；平台的真实双身份及超管／祖先角色隔离仍由下列九项独立专项验收。

## 隔离 fixture 与安全边界

[employee_isolation_tests.rs](../../../src/employee_isolation_tests.rs)只读取显式的 `IDENTITY_PHASE03_TEST_DATABASE_URL`，要求管理连接的数据库名为 `postgres`；不读取业务 `.env`、预览配置或生产连接。每项自行创建 `identity_phase03_<v7>` 数据库，从当前真实迁移链运行；只在该自有测试库的目标 schema 准备中移除旧二元唯一约束，以构造同账号同组织两行。默认扩展 schema 和已发布迁移字节不因这些 fixture 被修改。

fixture 显式创建账号、组织、成员、客户端、应用与来源绑定，不为旧凭据补填当前成员或版本。客户 session 与应用 token 使用独立、完整的账号／成员／来源／策略证明；拒绝用例先确认相应客户凭据确实有效，防止以无效 token 假代替客户隔离。fixture 密钥、密码、原始 token、数据库连接与客户端 secret 不写入本文或失败输出。

清理仅接受已校验的 `identity_phase03_` 自有库名，关闭连接池并删除该库；最终九项输出中的 `assertions_ok`、`database_removed`、`pool_closed` 全部为 true。沿用本任务独立 PostgreSQL 容器 `identity-sso-01a16e22-pg`；连接只通过私有进程环境注入，不记录环境内容。结束时另以该容器中的 `psql -X -U postgres -d postgres -Atc` 只读查询数据库目录，仅有 `postgres` 维护库，所有自有 fixture 已清理。

Linux runner 使用隔离编译环境执行命令，辅助脚本保存在私有 TEMP。结果保存在本会话工具执行记录（包括 `phase03-regression`、`phase03-final-gates`、`phase03-final-specials`；最终执行会话 `43065`），没有另存日志文件，本文不编造日志路径。

## 专项覆盖与执行状态

以下九项已在最终源码上串行执行通过，共 13.51 秒；所有自有 fixture 均清理成功。

| 专项名称（统一前缀 `employee_isolation_phase03_`） | 关键断言 | 当前实测状态 |
| --- | --- | --- |
| `customer_actor_and_directory_targets` | 同账号同组织超管＋祖先角色，员工 context 正常；合法客户 session 不能访问员工管理 API，客户 app token 在员工授权投影前拒绝；目录 count／list／one／直接 ID 一致，双身份员工不重复，客户及停用员工不入选 | PASS |
| `bidirectional_roles_and_data_grants` | 两个方向均不能给客户／停用员工授权，双身份员工成功；旧结构 stale 角色负例不进入客户查询与有效用户数；数据授权 read／preview／PATCH 与 options 拒绝精确客户／停用成员，合法员工成功 | PASS |
| `bulk_preserves_other_identities_and_role_support` | 保留客户与停用员工，不隐式启用；仅客户账号可在目标 fixture 新增独立员工；并发保存不复制身份；bulk 删除仅撤相关成员，祖先角色按最终有效员工集合保留或清理 | PASS |
| `permanent_delete_and_replace_keep_supported_roles` | 删除与替换分别验证：清理无支持的直接角色，保留其他有效员工支持的祖先角色／会话，客户会话独立；最后一个员工永久退出后清理剩余无支持角色 | PASS |
| `fresh_identity_precedes_cached_administrator_flag` | `CurrentUser` 旧 true／DB false 不授权，旧 false／DB true 使用当前授权；客户 sid 即使账号当前为超管也拒绝；DB 降权后旧提取标志不能继续放行 | PASS |
| `scope_rechecked_after_real_account_lock_wait` | bulk、用户→角色、角色→用户三个真实锁屏障；等待期间把目标组织移出 actor 子树，释放后拒绝；成员、客户、角色、会话与审计完整快照无业务变更 | PASS |
| `publisher_rechecked_after_late_foreign_key_wait` | 真实应用通知 HTTP 发布在 notification 的 recipient 用户 FK 处等待；持锁事务停用发布者员工成员或提升发布者账号版本后提交，发布必须拒绝并回滚 publication、delivery 与成功审计；局部／全局撤销范围分别核验 | PASS |
| `role_parent_locked_before_replacing_children` | 用户角色从 `{R,S}` 替换为 `{R}` 时，先在明确的父 role `KEY SHARE` 锁等待；另一事务删除 R 并级联后安全拒绝，S 保留且无成功审计，避免先删 child 再等待 parent 的反向锁死 | PASS |
| `bulk_self_departure_preserves_customer_and_rechecks_after_credential_wait` | 普通平台根员工合法自行 bulk 退出成功，旧员工 session／实际 platform refresh 失效、同组织客户 session／refresh 保留；真实刷新凭据子行锁等待期间撤销 `system:user` 或根上下文平台管理范围，释放后拒绝并完整回滚成员、角色、会话和成功审计 | PASS |

最终专项验收入口如下；普通 `cargo test` 不执行这些 `ignored` 数据库用例：

```sh
cargo test --locked --offline --bin saas-platform-backend employee_isolation_phase03 -- --ignored --test-threads=1 --nocapture
```

### 独立审查发现的竞态与修复

审查发现，事务前缓存的 `scoped_org_ids` 可以在账号锁等待期间过时：目标原为操作者子树内的后代，等待期间被重挂至兄弟分支，原来的 ID 数组仍包含它。单独重验操作者 session／权限不足以证明目标仍在范围内。

用户和角色写入现增加事务内的 scope 重算及前后比较，在锁后与提交前验证；角色目标资格也在最终写入前复核。专项先持有真实目标用户锁，使用 `pg_stat_activity` 和 `pg_blocking_pids` 证明 HTTP 请求已经在 `users ... FOR ... UPDATE` 等待，再由独立连接提交组织层级变更，最后释放锁。它不通过猜测性 sleep 声称发生竞争，并比较完整成员／角色／会话及审计快照。该专项已在最终九项执行中 PASS。

另发现两处末端锁等待风险，作为本阶段员工资格和角色维护边界修复：

- 应用通知发布的早期权限 SELECT 不会等待收件人用户行锁，后续 `notifications.user_id` 的 recipient FK 才会等待。若发布者在此期间被停用、会话撤销或账号版本提升，仅在早期校验会提交撤权后的 publication。[通知发布](../../../src/notifications.rs)现于完整写入／审计之后、提交之前重新校验发布者与目标范围，幂等重复返回前也复核。新增专项持有独立收件人的用户锁，以精确的 `INSERT INTO notifications(` 阻塞查询证明竞争，再在持锁事务中撤销发布者身份或账号版本，核验无 publication／notification／成功审计残留。真实 schema 的 `notification_publications.publisher_user_id` 没有用户 FK，本文不以该列声称锁屏障。此修复只关闭员工发布 actor 的授权窗口，不代表 Phase 4 的收件身份、历史隔离和实时撤权已完成。
- 用户→角色替换若先删除 `user_roles` child，再重新插入保留 role，会与另一操作者的父 role 删除／FK 级联形成反序等待。现先按稳定顺序锁定请求保留的 role 父记录并验证，再变更 child。专项在真实 `roles ... FOR KEY SHARE` 屏障处执行另一事务的父删除，要求请求安全失败、无未删除角色的绑定损失与成功审计。

合法 bulk 自行退出还有两项同时必须满足的条件：删除后不能再用已不存在的 actor 成员证明提交权限；提前验证也不能跳过凭据子行、审计等末端等待中的撤权。实现将当前 actor 的精确成员删除和最终角色清理延后，并先按稳定次序锁定已有成员授权、session、刷新令牌、OAuth 凭据及角色子行，将实际等待置于最后实时证明之前。新增专项先证明自行退出成功和客户凭据独立，再用真实 `platform_session_refresh_tokens` 行锁与 `pg_blocking_pids` 屏障，在等待期间分别移除角色的 `system:user` 菜单、取消当前租户的 `is_platform` 属性，使管理范围从无限制 `None` 变为当前组织子树 `Some`；要求拒绝且所有本次业务／成功审计变更回滚，原员工及客户刷新仍可使用。该专项已在最终九项执行中 PASS。

相关模块重跑发现两项新 fixture 假设错误：自行退出 scope fixture 创建第二个根组织触发 `uq_org_root_per_tenant`，publisher fixture 等待不存在的 publication 用户 FK。已按实际 schema 修正为租户平台属性变化和独立收件人 FK 等待，最终九项完整重跑已通过。修正没有增加推测性 sleep，也没有放宽拒绝与回滚断言。

## 实际执行命令与结果

以下平台命令均在后端目录由 Linux runner 执行，数据库连接由私有环境注入。`phase03-regression` 依次执行普通测试、文档和全量 ignored；普通测试及文档通过，全量 ignored 返回失败：

```sh
cargo test --locked --offline --all-targets && cargo doc --locked --offline --no-deps && cargo test --locked --offline --bin saas-platform-backend -- --ignored --test-threads=1 --nocapture
```

修复后，`phase03-final-gates` 先通过 fmt／check／Clippy，再执行下列相关模块集合；这是 68 项的分组重跑，不是全量命令：

```sh
cargo test --locked --offline --bin saas-platform-backend -- --ignored --test-threads=1 --nocapture employee_isolation_phase03 data_grant_tests:: platform_admin_cli_tests:: organization_subtree_user_role_tests:: platform_session_refresh_tests:: membership_lifecycle_tests:: application_setup::tests:: management_audit_tests:: notification_tests:: departments::organization_tests:: scope_tests:: authorization_isolation_tests:: data_grant_discovery_tests::
```

修正两项新 fixture 后，`phase03-final-specials` 在最新源码执行以下完整命令，执行会话 `43065` 最终退出码为 0：

```sh
cargo fmt --all -- --check && cargo clippy --locked --offline --all-targets -- -D warnings && cargo test --locked --offline --bin saas-platform-backend employee_isolation_phase03 -- --ignored --test-threads=1 --nocapture && cargo test --locked --offline --all-targets && cargo doc --locked --offline --no-deps
```

| 检查／执行批次 | 实际结果 |
| --- | --- |
| Linux `cargo fmt --all -- --check` | PASS；最终专项批次再次通过 |
| Linux `cargo check --locked --offline --all-targets` | PASS；相关模块批次的前置门禁 |
| Linux `cargo clippy --locked --offline --all-targets -- -D warnings` | PASS；最终 fmt／Clippy 批次共 30.46 秒 |
| 首轮全量 ignored | **151 PASS／6 FAIL，共 157 项**，630.69 秒；保留失败及修复经过如下 |
| 修复后相关模块重跑 | **66 PASS／2 FAIL，共 68 项**，266.70 秒；六个旧失败全部通过，两个失败均是新增 fixture 假设错误 |
| 最终 Phase 3 九项隔离专项 | **9／9 PASS**，13.51 秒；逐项 assertions、database_removed、pool_closed 全 true |
| 本阶段 ignored 覆盖合计 | **160 项每项已有真实 PASS：151 个既有用例＋9 个本阶段专项**；通过全量／相关模块／最终专项分组覆盖，不声称单次 160 项全绿 |
| 最终普通后端测试 | **185 PASS**：library 9＋binary 163＋外部 4＋7＋2；binary 测试耗时 7.56 秒 |
| 最终 `cargo doc --locked --offline --no-deps` | PASS，19.22 秒 |
| Node 仓库结构及固定 SDK 合同 | **7／7 PASS** |
| 独立 PG 剩余数据库只读核验 | PASS；仅 `postgres`，自有 fixture 全部清理 |
| 工单 fmt／check／clippy／all-targets 普通测试／doc | PASS；普通测试计数未单独报告，本文不补造 |
| 工单 `member_contracts` PostgreSQL 合同 | **13／13 PASS** |
| 工单 `write_contracts` PostgreSQL 合同 | **14／14 PASS** |
| 门店 fmt／check／clippy／doc | PASS |
| 门店普通测试 | **88 PASS** |
| 门店 PostgreSQL 合同 | **6 PASS** |

### 失败修复与分组覆盖

首轮全量的六个失败分别来自数据授权独立 read／write 合同、同一合同的真实 CLI 回归、两个组织子树角色并发 fixture、旧 bulk 自行退出以及部门组织 fixture。它们没有被忽略或删除：

- 数据授权继续区分独立 read 和 write；write 不隐含 read，只有 write 的合法 actor 可执行正式 PATCH。数据授权及真实 CLI 用例的该合同在修复后的相关模块批次通过。
- 两个组织子树角色 fixture 原锁屏障只识别 `FOR UPDATE`，员工维护改为 `FOR NO KEY UPDATE` 后未识别实际等待。屏障现识别这两类真实用户锁，仍通过 `pg_blocking_pids` 核验竞争，原范围、资格、拒绝和回滚断言保留；两个用例均通过。
- 旧 bulk 在最后重验 actor 前已删除其员工成员，导致合法自行退出因当前 session 失效而回滚。现按上述精确延后删除、末端等待后复核规则修复；旧保留成员／授权／会话回归及新增自行退出正反例均通过。
- 部门组织 fixture 手造空／不匹配的 session ID，实时员工证明正确拒绝。现从 fixture 已签名 token 提取其绑定 sid，为相应组织上下文建立完整的数据库会话；原部门权限、组织边界和只读行为断言保留，该用例通过。

68 项相关模块重跑已将六个旧失败全部验证为 PASS；当轮仅两个新增 fixture 失败，原因与修正见上文。最终九项专项完整通过，补齐这两个新用例及其余七个专项的最新结果。最终普通测试和文档也在该 fixture 修正后重新执行通过，不借用 Phase 2 的旧 binary 结果。

## 七项验收追踪

| 台账标准 | 实现／证据入口 | 本阶段结论 |
| --- | --- | --- |
| 1. 超管或祖先角色账号的客户 context 不能访问员工 API | FreshEmployee、当前 context 菜单／角色门面、客户 actor 专项与旧管理员标志专项 | PASS |
| 2. 同组织双身份分别有权，两个角色管理方向均不能授权客户 | 用户／角色双向管理及角色投影；双向角色与数据授权专项 | PASS |
| 3. 客户／停用目标不能列出或直接选择，双身份有效员工可选 | 平台成员 count／list／one；门店联系人／工单处理人消费合同 | PASS |
| 4. bulk 保留客户、不启停用员工；目标 schema 可新增独立员工，重复为 409 | 三元 UPSERT、重复组织冲突、bulk 与真实范围竞态专项 | PASS |
| 5. 停用员工不以客户证明角色资格，其他员工角色及另一身份会话保留 | 当前员工资格、具体成员撤销及最终员工支持集合 | PASS |
| 6. 临停保配置不授权，永久退出／删除／替换仅清无支持配置 | 生命周期删除／替换、bulk 最终集合清理、数据库配置与实时访问断言 | PASS |
| 7. 目录／角色／数据授权 JOIN 消除双行歧义 | 成员 ID 绑定、明确员工谓词、相关 EXISTS 与独立 target 校验；上述正反例 | PASS |

## 后续阶段边界

本文件完成 Phase 3 验收证据收口，主台账与导航由执行主代理同步更新，并按依赖推进 Phase 4。相同 user／org 两行 JOIN 安全已由真实 SQL／HTTP 正反例及并发专项验证，不仅依赖源码审查。

通知隔离、管理 UI／缓存、最小客户 API、Phase 6 收缩迁移和真实创建启用均仍按后续阶段执行。Phase 3 不新增客户 RBAC、不臆造客户业务、不执行生产切流，也不修改已发布 SDK／Notification／Context 线协议副本来绕过服务器端隔离。
