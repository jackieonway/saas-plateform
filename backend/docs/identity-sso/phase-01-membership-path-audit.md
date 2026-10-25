# Phase 1 成员写入、引用与双身份歧义审查

审查日期：2026-10-24。范围为平台当前源码、追加迁移、管理工具和关联消费者；本身份功能使用全新数据库，不读取或搬迁预览库数据。本文是路径清单与阶段边界记录，执行结果以 [Phase 1 验收](validation/phase-01.md) 和 [空库 v7 基线](validation/fresh-v7-baseline.md) 为准，不能把此清单本身当作双身份安全验收。

## 审查方法与结论

从生产 `memberships` 的 INSERT／UPDATE／DELETE 出发，追踪路由、seed、联系人管理员、角色双向维护、CLI、离线恢复以及账号／组织级联删除；同时搜索二元 `ON CONFLICT`、成员 JOIN、触发器和通知受众。测试 fixture 与历史 migration 单独识别，历史 migration 不改写。

执行前共有四组成员写入来源：普通创建用户、批量维护用户组织成员、租户／组织联系人管理员和 seed。角色“确保成员”函数只验证既有关系，没有隐式创建成员。当前管理 CLI 没有用户、成员或任意 SQL 写入能力；业务路由未发现成员 CSV／批量导入入口。专用 UUID 恢复 importer 属于另外的固定预览三库流程，不能用于本身份新库。

现有外键没有引用 `memberships(user_id, organization_id)` 的二元唯一键。员工授权、会话、OAuth 和通知仍有按账号／组织消费成员的旧路径；它们分属 Phase 2～4，Phase 1 不宣称已经全部消除。扩展期必须保留二元唯一约束，并关闭客户、第二身份和新增生命周期入口的正式启用。

Phase 1 另设一项必要的先行安全门禁：所有旧人类凭据签发、刷新及成员资格验证，临时只接受 **ENABLED EMPLOYEE**。已审查的源码落点包括 `routes/auth.rs`、`session.rs`、`app_token.rs`、`oauth_issuance.rs` 和 `routes/oauth2.rs`。它保证隔离测试创建的 CUSTOMER 不能从旧登录／context／password OAuth 流程得到员工型凭据，也防止停用员工立即通过旧入口重领凭据。该门禁支撑本阶段“客户尚无凭据”的兼容撤销分支；它没有实现 account／membership／policy 版本绑定，也没有允许客户登录，完整绑定及同组织双身份精确凭据撤销仍由 Phase 2 验收。实际负例测试结果见阶段验收，不能仅以源码过滤判定通过。

可重跑的生产路径搜索（从 `backend/` 执行）：

```powershell
rg -n 'INSERT INTO memberships|UPDATE memberships|DELETE FROM memberships|ON CONFLICT\(user_id,organization_id' src -g '*.rs' -g '!**/*tests*' -g '!**/release_tests/**'
rg -n 'REFERENCES memberships|memberships_user_id_organization_id_key|revoke_sessions_on_membership_delete' migrations
rg -l 'memberships' src -g '*.rs' -g '!**/*tests*' -g '!**/release_tests/**'
rg -n 'membership|SQL execution|client creation' tools/platform-admin/src
```

## 成员产生与变更路径

下表中的“处理合同”是当前阶段需要守住的行为，实际断言和运行结果登记在阶段验收；不是仅凭搜索即可判为通过。

| 路径 | 调用与输入 | 执行前风险 | Phase 1 处理合同 |
|---|---|---|---|
| 普通账号创建 | [`routes/users.rs`](../../src/routes/users.rs) 的 `POST /api/users`；租户管理员创建账号后加入当前组织，平台级创建可以先无成员 | INSERT 依赖员工默认值，后续扩展字段可能被误解为客户创建 | 旧 API 明确创建 EMPLOYEE；不接受任意类型输入；ID 仍由应用生成 v7；不复用手机号／邮箱合并账号 |
| 用户批量成员维护 | 同文件 `PUT /api/users/{id}/memberships`，旧 DTO 只有 organizationId／departmentId／position | 按组织覆盖所有成员、二元 UPSERT 和按组织去重会合并或删除客户；停用员工可能被旧请求恢复 | 所有读取、差异比较、DELETE 和 UPSERT 明确只操作 EMPLOYEE；三元冲突键；保留已有停用状态；客户行不参与员工请求的保留或删除集合 |
| 联系人管理员配置 | [`routes/onboarding.rs`](../../src/routes/onboarding.rs) 的 `provision_contact_administrator`，由租户／组织创建及联系人初始化调用 | 无冲突目标的 `ON CONFLICT DO NOTHING` 会把仅有客户误认为已有员工，再授予员工角色 | 明确 EMPLOYEE；按三元键处理；已停用员工不得隐式启用；仅有客户时扩展期必须拒绝第二行；创建角色关系前证明有效员工资格 |
| 开发演示 seed | [`seed.rs`](../../src/seed.rs) 的 `run` → `insert_membership` | 直接 SQL 可能绕过生命周期或依赖默认成员类别 | seed 只写显式 EMPLOYEE、ENABLED、v7；不得生成 CUSTOMER 或同组织第二身份；真实空库执行并验证唯一键／外键 |
| 空生产库引导 | `seed::initialize` → 同一个 `insert_membership`，账号为空时建立平台管理员 | 与开发 seed 共用底层写入，遗漏该路径会留下生产旁路 | 同一员工约束；生产密码继续使用现有密码策略与 Argon2；不对已有账号或成员做猜测性更新 |
| 新显式生命周期入口 | Phase 1 新增成员创建／绑定、启停、第二身份、替换、移除用例 | 绕过开关；无并发版本；按账号／组织误撤另一身份；原地改类型 | 默认关闭；成员 ID 精确定位、版本并发检查、类型不可变；同事务审计与撤销；目标账号／组织／租户／管理范围显式验证；绑定既有账号记录授权依据 |
| 新增第二身份 | 显式操作，目标类型与源 membershipId | 双约束扩展期提前创建第二行 | 独立开关加旧二元唯一约束双重阻止；仅专用目标 schema fixture 可以测试最终两行语义 |
| 替换身份 | 源 membershipId 与目标类型／已存在目标行 | 把源行类型原地改写；复用旧 ID／旧会话；自动恢复已停用目标 | 独立目标行；显式启用已有目标需权限与版本；停用源行；旧授权／会话不复活；扩展 schema 下若产生第二行则明确拒绝 |

### 角色“确保成员”与双向授权

[`routes/users.rs`](../../src/routes/users.rs) 的 `ensure_role_membership_in_tx` 由用户角色维护和 [`routes/roles.rs`](../../src/routes/roles.rs) 的角色用户维护共同调用。它在账号行锁之后，以成员组织及其祖先链证明角色组织资格，**不创建成员**。资格必须只由 ENABLED 的 EMPLOYEE 行支持，不能因为账号在同组织有 CUSTOMER 行而获得角色，也不能因该账号另有 CUSTOMER 而拒绝有效员工关系。

批量成员永久删除／替换后的角色清理，需要按剩余有效员工组织及祖先集合判断，不得按用户全删，也不得用客户行支持员工角色。临时停用可以保留配置，但读取资格须在后续身份授权阶段即时失效。角色列表、候选、计数与其他读取全部隔离属于 Phase 3 的独立验收。

### CLI、导入与离线工具

| 来源 | 审查结果 | 本阶段边界 |
|---|---|---|
| [`tools/platform-admin`](../../tools/platform-admin/README.md) | 命令为应用 bootstrap、发布、授权与分发；帮助明确无 client creation／SQL execution；未发现用户／成员写命令 | 不新增客户／成员 CLI。以后新增时必须走同一生命周期与开关，不能直接 INSERT |
| 业务成员导入 | `routes/` 未发现成员 import／CSV 导入路由；审计 CSV 为导出，与成员创建无关 | 当前无活动成员导入写入器；以后新增需登记并复用生命周期 |
| [`ops/recovery` UUID importer](../../ops/recovery/src/uuid_rekey/import.rs) | 使用严格固定 U04 目标名、owner、catalog／schema 摘要和认证包；COPY 可能含 members 数据，但属于独立预览恢复流程 | 本身份新库历史导入不适用；禁止以该工具向身份目标库导入旧 schema／成员；不改其固定范围、不运行导入 |
| 历史 migration | 原始成员表及旧约束由 `0001_init.sql` 建立，UUID 改造为追加 `0012_uuid_v7.sql` | 保留历史字节；Phase 1 只追加扩展 migration；Phase 6 才追加删除二元唯一键的收缩 migration |
| SQL 测试 fixture | 大量测试显式 INSERT 员工或构造旧 schema／v4 负例 | 只作用自建隔离库；员工默认仅为兼容 fixture；负例不改成活动 v7 记录；最终两行 fixture 的去约束不是部署 migration |

## 删除、外键及触发器

| 路径／引用 | 执行前行为 | 审查要求 |
|---|---|---|
| `membership_data_grants.membership_id` | 外键仅引用 `memberships.id` 并级联删除 | 保留具体成员 ID 的授权边界；不改成 user＋org；客户不得被赋予员工数据授权 |
| 成员→用户／组织 | `memberships.user_id`、`organization_id` 分别为单列外键，ON DELETE CASCADE | 用户删除属于全账号删除；组织／租户删除属于组织范围删除，不伪装为只删某身份。全账号与组织撤销须覆盖适用凭据 |
| 成员→部门 | `department_id` 单列外键及 `(department_id,organization_id)` 同组织外键 | 后者引用 departments 的组合唯一键，不引用成员二元键。部门删除 SET NULL 仅影响资料字段，不改变成员类别／状态 |
| `memberships_user_id_organization_id_key` | 原二元唯一键 | 未发现外键依赖；扩展期继续保留；与三元唯一键并存，禁止提前删约束 |
| `revoke_sessions_on_membership_delete`／`trg_membership_session_revoke` | 旧函数按 `OLD.user_id + OLD.organization_id` 撤平台 session | 新迁移需替换为可证明的成员定位；兼容无绑定凭据仅在唯一员工语义中安全处理；双身份不可按二元范围误撤另一行 |
| 用户、组织、租户删除 | `routes/users.rs::delete`、`organizations.rs::delete`、`tenants.rs::delete` 经 FK 级联移除成员 | 全局／组织销毁语义保留；不得把它们开放成客户单身份删除方式。局部退出必须走具体 membershipId 生命周期 |
| 用户停用、组织／租户状态触发器 | 原触发器分别撤全用户或组织范围 session | 这是原有实体级撤销，独立于成员启停。成员启停不能靠全局实体触发器代替 |

## 二元读取与广播清单

以下是已识别的后续消费点。Phase 1 保留开关和二元约束，防止这些旧读取在客户／第二身份真实存在时运行；后续必须逐项迁移并通过双身份 fixture，不能据此文档宣称完成。

| 消费点 | 当前成员定位／歧义 | 负责阶段 |
|---|---|---|
| [`routes/auth.rs`](../../src/routes/auth.rs) | 登录选择与成员列表仅返回组织、曾按第一条成员预选；context 只按 user／org 定位 | Phase 2：具体成员 ID／版本与显式选择 |
| [`session.rs`](../../src/session.rs) | session／refresh 核验以 user／org 成员 JOIN 或 EXISTS 证明资格 | Phase 2：权威 account 与 membership 版本、状态、逐载体拒绝 |
| [`app_token.rs`](../../src/app_token.rs)、[`oauth_issuance.rs`](../../src/oauth_issuance.rs)、[`routes/oauth2.rs`](../../src/routes/oauth2.rs) | app token、普通用户 OAuth、授权码与设备选择含二元成员假设，OAuth 有首个可用成员逻辑 | Phase 2：凭据绑定与来源 client；保留 MACHINE 用途合同 |
| [`app_authorization/store.rs`](../../src/app_authorization/store.rs) | grant 已按具体 membershipId 引用，但消费时仍按账号／组织筛成员 | Phase 2～3：context 精确成员与有效员工门禁 |
| [`data_grants/store.rs`](../../src/data_grants/store.rs)、[`data_grants/discovery.rs`](../../src/data_grants/discovery.rs) | 目标写入按成员 ID，但 actor runtime 为 user／org JOIN；发现／候选会列出所有成员 | Phase 2～3：actor 身份绑定、候选仅员工与按成员返回 |
| [`routes/users.rs`](../../src/routes/users.rs)、[`routes/roles.rs`](../../src/routes/roles.rs) | 用户范围、角色用户、成员摘要和计数仍有 user／org EXISTS | Phase 1 先限制员工写入及“确保”资格；Phase 3 完成员工授权读取与候选隔离 |
| [`routes/app_members.rs`](../../src/routes/app_members.rs)、[`routes/application_setup.rs`](../../src/routes/application_setup.rs) | 应用成员目录、处理人、初始化 actor／角色资格含二元关系 | Phase 3：有效员工、直接成员目录与受控动作 |
| [`routes/onboarding.rs`](../../src/routes/onboarding.rs) | 联系人复用范围及角色初始化资格为成员 EXISTS | Phase 1 确保员工写入安全；Phase 3 全部读取资格只认有效员工 |
| [`routes/search.rs`](../../src/routes/search.rs) | 用户组织展示有取首个组织，过滤通过成员 EXISTS | Phase 3：明确员工目录含义；展示首个组织不能用于鉴权 |
| [`routes/organizations.rs`](../../src/routes/organizations.rs)、[`routes/tenants.rs`](../../src/routes/tenants.rs)、[`routes/monitor.rs`](../../src/routes/monitor.rs) | 用户／成员统计可能将两行当两人，未区分成员类型 | Phase 3～5：明确人数／员工数／客户数语义，按业务口径去重 |
| [`notifications.rs`](../../src/notifications.rs) | actor 为二元 JOIN，收件选择 `m.user_id,o.id`，通知行与实时提示仍按 user／org，system_in_tx 以二元成员 EXISTS 验证 | Phase 4：recipient membershipId／类型／版本，缺省只员工，客户通知关闭；实时重验当前身份 |
| [`routes/notifications.rs`](../../src/routes/notifications.rs) | 收件列表、已读、未读数与实时通道需与通知绑定一起改 | Phase 4：具体成员隔离；不能先只修改 fanout 就宣称隔离完成 |

### 生命周期锁与等待后的权威检查

审查发现并修复了账号→父实体与联系人管理员父实体→账号之间的锁顺序倒置。当前 `membership_lifecycle/service.rs::lock_users` 将操作者和目标账号按 UUID 排序，以 `FOR NO KEY UPDATE` 统一锁定，之后锁具体成员；不再晚取组织／租户／祖先链的强锁，避免与 `onboarding::initialize_existing_administrator` 持父实体再等待账号形成锁环。这与已有 `data_grants/store.rs::lock_target` 的顺序约定一致。

当前源码已实现 `authority_in_tx` 与 `revalidate`：同事务读取 actor sessionId、账号状态／版本、当前有效 EMPLOYEE 的 ID／版本、实时权限及组织／租户边界指纹，并在账号／成员锁等待后及变更提交前重新核验。有效期额外使用 `clock_timestamp()` 和 `org_chain_expires_at`，不只依赖事务开始时冻结的 `now()`。角色清理由有效员工集合支持，替换／永久删除清理具体成员的数据授权；客户分支不撤员工凭据。为避免本事务自行撤销 actor 后与最终权威校验冲突，当前 API 明确拒绝停用、移除或替换当前登录员工身份，由其他管理员执行；这不是隐式自操作回滚。

主执行者已集中运行 Phase 1 四项 PostgreSQL 专项，结果 **4/4 通过，17.53 秒**，其中包含六种可观察用户锁屏障下的 session、permission、organization status／expiry、tenant status／expiry 变更，以及客户旧登录／context／OAuth 拒绝、v12 员工 fixture 升级与空库真实 seed。命令、独立库名、最终重跑和限制以 [Phase 1 验收](validation/phase-01.md) 为准；本审查者未自行执行或冒领该数据库运行。上述修复只完成 Phase 1 的管理事务及员工兼容门禁，完整成员凭据绑定仍留在 Phase 2。

## UUID 发号、入口与消费者复核

平台生产统一从 [`util/uuid7.rs`](../../src/util/uuid7.rs) 调用 `Uuid::now_v7()`；校验同时检查 version 7 和 RFC variant。Path／DTO／optional query 使用该入口，nil、v4、坏 variant 和错误过滤值必须失败。追加 `0012_uuid_v7.sql` 为所有 107 个现有 UUID 列移除默认并建立检查，Phase 1 新 UUID 绑定列也必须补齐同样约束。

seed、成员 INSERT、session、通知 fanout 和其他生产 INSERT 必须显式传 v7，不以数据库函数补发。新自然唯一键不会替代 v7 主键；同类型重复创建仍需确定的 409，并发创建不泄漏数据库错误。密码、客户端 secret、授权 code／refresh、PKCE 和随机退避不使用 v7 替代 CSPRNG。

关联消费者入口：

| 范围 | 入口与实际合同 |
|---|---|
| SDK 维护与发行 | `saas-micro-app-sdk` 的 `npm run check`、`npm test`；当前活动版本 **3.0.0**，不是 U02 时尚待发布的状态。`tests/uuid-v7.test.mjs` 验证并发／回拨、typed context、variant 与请求 ID；历史 1.0.0／2.0.0 字节继续只读 |
| 平台浏览器消费者 | `saas-plateform/frontend` 的 `npm run sdk:check`；`tests/uuid-v7.mjs` 与协议 tests；MicroProps 2、Context 1、Notification 1 仍为各自线协议版本 |
| 门店 Rust | `saas-stores/backend` 的普通 `cargo test --locked --all-targets`；`tests/postgres_contracts.rs` 为显式 ignored PG 合同，含 UUID 各列及 v3 schema 合同、授权／范围／事务 fixture |
| 工单 Rust | `saas-work-orders/backend` 的普通测试，以及 `tests/member_contracts.rs`、`migrations.rs`、read/write/reminder 合同；成员目录、实时权威校验与 UUID 约束不可只靠编译证明 |
| 工单 Node 兼容 | `saas-work-orders/backend` 的 `npm test`（`tests/api.test.mjs` 等），真实 PG 测试需要显式受控环境；member／context／scope fixture 不连接真实平台服务 |

2026-10-24 本次审查中实际执行：Windows 固定 Node **24.22.1**，SDK `npm run check` 验证 **20** 个 generated artifacts，SDK `npm test` **30/30**，平台前端 `npm run sdk:check` 通过。

另在分别阅读两仓 backend README 后，以 Windows **rustc 1.97.0 (2d8144b78 2026-07-07)** 并行运行各自 `cargo test --locked --lib`，使用各仓既有 target，不占平台 target 构建锁：

| 活动消费者 | 本次实际运行结果 | 覆盖与限制 |
|---|---|---|
| `stores-api 0.1.0` | **77 passed，0 failed，0 ignored**；测试耗时 **6.31 秒** | Context 1 授权、legacy／combined、实时失败封闭、成员与组织投影、v7 typed ID／请求 ID／并发发号；未运行 `tests/postgres_contracts.rs` |
| `work-orders-api 0.1.0` | **71 passed，0 failed，0 ignored**；测试耗时 **3.02 秒** | Context 1 身份／范围／到期、legacy／combined、member witness、目录／提醒 DTO、v7 生成解析；未运行 member／migration／read／write／reminder PostgreSQL 集成测试 |

这两次子进程显式移除 `TEST_DATABASE_URL`、`DATABASE_URL`、`STORES_DATABASE_URL`、`WORK_ORDERS_DATABASE_URL`，未传入任何 PG URL。按两仓 README 的 Windows 支持方式，将 `STORES_TEST_NODE`／`WORK_ORDERS_TEST_NODE` 指向固定 Node **24.22.1**，仅为单测自建 HTTP fixture 监听并转发到原 Axum handler；生产 reqwest 与原断言保持原样，未启真实业务服务或数据库。门店／工单 PostgreSQL 用例在本次审查中未运行，不能把 [UUID-U02 已有证据](validation/uuid-u02.md) 描述为本次新运行。SDK 3.0 的发行验证与 Context 1 后端授权消费验证是不同合同，不改写 MicroProps 2／Context 1／Notification 1 的含义。

## 隔离数据库与运行门禁

只读核验发现 Windows Rust 可用但 PATH 无 docker／psql；WSL Ubuntu 提供 Docker、psql，Rust 1.97 二进制在用户本地工具目录。现有独立测试容器为 `saas-uuid-u02-pg17-20261024`，镜像 `postgres:17.6-bookworm`，端口仅绑定 `127.0.0.1:55437`，Docker 数据卷同名。容器创建时间、镜像、端口及 U02 文档一致；检查时只有维护库 `postgres` 与测试管理库 `uuid_u02_admin`。本审查没有启动停止任何服务、创建数据库或读取 `.env`。

Phase 1 专用四项和空库基线只在该已核实测试集群中创建全新、明确命名的 `identity_phase01_*` 库，或另建同样明确的专用测试集群。更广回归沿用已审查的既有自建／自清理 helper 前缀，例如 `release_test_*`、`production_security_test_*`；它们使用同一显式隔离测试集群。执行者须记录目标集群、名称范围和清理结果。不得使用预览三库、U04 转换目标、默认本地 5432 或 `.env` 推断的连接。

最短基线方案为：自建 `identity_phase01_*` 空库 → 调用真实 `database::run_migrations` → 调用真实 `seed::run` → 检查每个 UUID 列、无 UUID 默认、FK 与自然唯一键 → 运行 UUID 输入／发号与活动 SDK 合同 → 从 v12 员工 fixture 验证追加扩展迁移。若用 `platform-migrate` 命令，可仅从进程显式传入目标 URL；该命令不读 `.env`，但它不执行 seed，不能仅以迁移退出码证明 seed 成功。

现有 `release_tests::TestDatabase` 固定创建 `release_test_*`，scope 测试复用它；生产安全 helper 使用 `production_security_test_*`。本次平台完整 ignored 回归实际沿用这些既有 helper，专用基线仍使用 `identity_phase01_*`，不能将完整回归写成只创建了后者。门店 PG helper 的 `phase26_stores_sql_*` 及工单专有前缀在本次未运行；消费者只运行无数据库单测。各 fixture 保留原测试语义，并核验自有数据库清理。Linux 与 Windows 编译缓存分开，不把 Windows `.exe` 当 Linux 已构建产物；复用缓存时也需重新嵌入当前 migration 并运行当前源码。

## 后续阶段明确保留事项

- Phase 2：所有 session／access／refresh／app／OAuth／code／device 绑定具体成员与版本，权威不可用返回 503；完成应用／客户端准入策略 API、CLI／seed 适配。
- Phase 3：员工 RBAC、目录、候选、祖先资格与计数的完整隔离，不以“角色写入已有员工过滤”代替全部读取验收。
- Phase 4：通知各通道绑定具体成员，客户通知继续关闭；没有历史通知回填工作。
- Phase 6：真实收缩 migration、实例／写入器安全清单及第二身份开放门禁。新库无旧 binary 清退属不适用；这不取消分阶段开关和约束验收。
