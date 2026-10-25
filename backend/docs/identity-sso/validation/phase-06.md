# Phase 6：客户最小 API、独立收缩与环境就绪验收

日期：2026-10-25。状态：**COMPLETED，六项验收 PASS**。客户最小资源、精确用途登记、独立收缩／readiness 与写入器审查已完成；修复后的格式／check／Clippy、193 项普通测试、三项 rollout 专项、最终 bins 构建及文档均通过。后端 main 的 178 项 ignored 按首轮 176 PASS 与修复后定向 2 PASS 取得逐项证据，原始 176 PASS／2 FAIL 保留。真实 TASK_OWNED 新库完成 manifest、收缩和 receipt；三个启动负例通过，正确 receipt 在首次 seed timeout 后经同目标恢复启动，health 与 30 项 HTTP 验收全部 PASS，进程干净退出。本次完成范围不包含生产创建开关开放。

范围依据[主台账 Phase 6](../task-ledger.md)，合同分别见[客户本人资料](../phase-06-customer-profile.md)、[独立收缩与启用](../phase-06-rollout.md)、[写入器与实例审查](../phase-06-writer-readiness.md)。本文记录本任务身份新库及自有测试资产，不接管预览 UUID 专项，不宣称生产部署、生产资产清单获批或真实生产创建已启用。

## 本阶段交付与界限

### 客户最小资源

[customer_identity](../../../src/customer_identity/mod.rs)独立提供三个 GET：`/api/customer/context`、`/api/customer/profile`、`/api/customer/applications`。固定用途为 `CUSTOMER_PROFILE`，入口 `/api/customer/profile`；服务器成对配置 `CUSTOMER_PROFILE_CLIENT_REF`（`oauth_clients.id`）和 `CUSTOMER_PROFILE_APPLICATION_ID`。默认均不配置，缺登记拒绝，不选择首 client 或默认 app。

成功凭据路径复用实际 `POST /oauth/token` 的显式 `membership_id` 用户签发与刷新。新资源只接受持久 `token_use='oauth'`、`storage_version=2` 的 access，核验 token 摘要、JWT／jti／expiry 绑定、`profile` scope、实际 CUSTOMER 成员、账号／成员／客户端／应用版本、精确 USER 来源、应用关联、租户及组织完整授权。读取资料后再次校验原证明；权威不可用拒绝，不修补缺失证明。

独立 `customer-profile/1` DTO 仅返回当前客户 context 和本人 `id`、`username`、`realName`、`avatar`，无员工授权或超级管理员信息；应用列表只含登记的最小应用。非空 query 不能传入另一主体代读。没有新增登录 API、客户 platform session、客户通知发送、订单／工单业务或客户门户。

精确登记配置已按独立审查修正为同时校验 UUID v7 的 RFC variant 与 canonical 小写连字符形式，复用 `util::uuid7::is_uuid_v7`。`config::tests::customer_profile_registration_requires_a_canonical_rfc_v7_pair` 已在本轮普通测试实际 PASS，覆盖坏 variant、v4、大小写、空值与缺另一字段等负例，错误信息也不回显无效值。

旧 `CurrentUser` 与控制台仍只接受员工。既有 OAuth userinfo／introspection 保留基础协议身份合同，客户角色为空；Phase 4 已验收的类型中立通知传输基座保留，本客户 access 不因此变成通知用 platform session。固定 SDK、Context 1、MicroProps 2、Notification 1 不因本资源扩展字段。

### 独立收缩与 readiness

[platform-membership-rollout](../../../src/database/identity_readiness.rs)及[操作合同](../phase-06-rollout.md)把收缩放在正常 SQLx 迁移链外。当前普通链截至 0015，未经外置收缩的目标仍保留旧二元唯一键；服务自动迁移不会删除它，也不会自动制造 ready receipt。本次实际 development 目标另经显式 apply 完成收缩。

显式工具使用专属 `MEMBERSHIP_ROLLOUT_DATABASE_URL` 和明确目标。`inspect` 核对真实数据库／集群身份、迁移 checksum、成员 CHECK／三元唯一／不可变保护和二元键依赖。`apply` 要求真实 manifest、文件摘要及资产／gate 完整性，在事务锁及 `RESTRICT` 下执行[独立收缩 SQL](../../../ops/identity/phase-06-contract-memberships.sql)，随后创建[readiness receipt](../../../ops/identity/phase-06-readiness-receipts.sql)，不改写 SQLx 历史。

非隔离 test 环境开启任一成员写开关时，配置与启动必须校验指定 receipt、manifest 摘要、环境、数据库／集群、当前后端文件、迁移和完整 catalog。检查发生在 seed、worker 和 HTTP 开放之前。客户或第二身份仍要求生命周期总开关；无凭据、错误目标或结构漂移应阻止启动。代码存在及合成输入单元测试不是实际环境批准。

### 写入器与人数语义

[写入器审查](../phase-06-writer-readiness.md)覆盖平台、CLI、seed、联系人初始化、批任务、离线程序与活动消费者，区分显式员工三元写入、具体成员写入、账号全局操作和纯集合／展示查询，没有把所有 user＋org 匹配误判为首行身份选择。

本轮发现并修复组织树 `userCount` 直接按成员行计数的遗漏：同账号员工＋客户只计一个关联账号；审计 `memberCount` 仍计两条关系。两项组织范围 PostgreSQL fixture 已增加双行回归，并在本轮全量 ignored 基线实际 PASS；这不改变同轮另两项准入策略失败的事实。

本任务没有身份新库生产部署清单或外部进程 owner。旧 binary 清退只在**本任务新库范围**登记为不适用；可见的其他 UUID／预览容器不是本任务资产，不借此宣称未知环境没有旧版本，也不操作它们证明本阶段完成。

## 已实际取得的合同与继承证据

本轮原始日志保存在平台仓库自有 [local-identity-sso-phase06-qa](../../../../local-identity-sso-phase06-qa) 中，不记录连接 URL、密码或原始 token。

| 门禁 | 实际结果 | 范围与记录 |
| --- | --- | --- |
| 后端集中格式／check／Clippy | **PASS** | 主执行统一运行；check 与 Clippy 分别完成 dev profile，耗时 1 分 02 秒、1 分 14 秒；原始输出见 [customer-and-static-gates.log](../../../../local-identity-sso-phase06-qa/customer-and-static-gates.log)。 |
| rollout 修复后集中格式／check／Clippy | **PASS** | [rollout-final-gates.log](../../../../local-identity-sso-phase06-qa/rollout-final-gates.log)，check 18.98 秒、Clippy 34.46 秒。 |
| 首轮后端普通测试 | **192／192 PASS** | [backend-full-baseline.log](../../../../local-identity-sso-phase06-qa/backend-full-baseline.log)：lib 13、main 165、集成 4＋1＋7＋2；main 普通组 11.93 秒，178 项 ignored 在另一运行中验证。两项薄 CLI 的普通组各为 0 项。 |
| rollout 修复后普通测试 | **193／193 PASS** | [rollout-final-gates.log](../../../../local-identity-sso-phase06-qa/rollout-final-gates.log)：lib 14、main 165、外部集成 14；main 普通组 7.58 秒。新增 PostgreSQL 17 CHECK deparse 回归实际 PASS。 |
| 首轮后端文档 | **PASS，27.51 秒** | [backend-full-baseline.log](../../../../local-identity-sso-phase06-qa/backend-full-baseline.log)记录 `cargo doc` 完成并生成 backend 与另两个 target 的文档。 |
| 最终 bins 构建与文档 | **PASS；build 59.96 秒，doc 26.81 秒** | [rollout-final-gates.log](../../../../local-identity-sso-phase06-qa/rollout-final-gates.log)记录最终实际后端、migrate 与 rollout 文件构建及三个 target 文档。 |
| rollout 修复后三项 PostgreSQL 专项 | **3／3 PASS，234.77 秒** | [rollout-final-gates.log](../../../../local-identity-sso-phase06-qa/rollout-final-gates.log)，输入拒绝原子性、真实 inspect／apply／身份约束及 fixture receipt 的 development 启动三项全部 PASS，三库清理 PASS；这不是实际任务 manifest／receipt。 |
| 真实 TASK_OWNED development 收缩与恢复启动 | **PASS** | [development-resume-start.log](../../../../local-identity-sso-phase06-qa/development-resume-start.log)及[实际启动结果](../../../../local-identity-sso-phase06-qa/development-final-evidence/attempts/resume-1416df0c213f79be/results/development-start.json)：同 receipt／manifest 恢复、不重 apply；health 200／UP、readiness 事件与干净退出 0。首次 health 失败仍保留。 |
| 真实收缩后的 HTTP 生命周期验收 | **30／30 PASS** | [http-smoke.json](../../../../local-identity-sso-phase06-qa/development-final-evidence/attempts/resume-1416df0c213f79be/results/http-smoke.json)逐项记录并发同类型 200／409、客户绑定确认、同组织双行、三元 UPSERT、员工 ID 保留、双向局部停用、旧 bearer 拒绝及 bulk 保留客户。 |
| 后端 main 完整 ignored 基线 | **176 PASS／2 FAIL，648.87 秒** | [backend-full-baseline.log](../../../../local-identity-sso-phase06-qa/backend-full-baseline.log)完整运行 main 的 178 项；集成 target 的三项 rollout 另列。两项准入策略 HTTP 400 的失败及后续复验要求见下文。自有数据库移除与连接池关闭成功，不能因此把失败断言记为通过。 |
| 准入策略失败项定向复验 | **2／2 PASS，6.55 秒** | [policy-final-gates.log](../../../../local-identity-sso-phase06-qa/policy-final-gates.log)，两项均 `assertions_ok=true database_removed=true pool_closed=true`；同轮格式与 Clippy PASS，Clippy 33.75 秒。main 的 178 项最终按 176＋2 分组取得 PASS，并非一次完整全绿重跑。 |
| 组织人数双行回归 | **2／2 PASS** | 全量基线中 `organization_subtree_reporting_in_ordinary_tenant` 与 `organization_subtree_reporting_in_platform_tenant` 均 PASS，验证双身份按账号去重。 |
| 平台通知完整隔离基线 | **9／9 PASS** | 全量基线中 `notification_identity_tests` 九项均 PASS，包括实际平台 HTTP＋当前外置 Socket.IO 服务、精确收据、双向局部撤销、等待后复核与权威失败；九库均断言及清理成功。 |
| 客户最小资源 PostgreSQL／HTTP | **6／6 PASS，33.82 秒** | [customer-and-static-gates.log](../../../../local-identity-sso-phase06-qa/customer-and-static-gates.log)逐项记录真实 OAuth 交换、资源读取与负例；六个自有数据库均记录 `assertions_ok=true database_removed=true pool_closed=true`。详见下表。 |
| 通知服务格式／check／Clippy、普通测试、构建与文档 | **PASS；普通测试 36／36，0.01 秒** | 本阶段实际重跑 [notification-gates.log](../../../../local-identity-sso-phase06-qa/notification-gates.log)，构建当前 `saas-notification-ws`，生成其文档。 |
| 通知服务真实 PostgreSQL／隔离专项 | **3／3 PASS，8.63 秒** | 同一日志包含双身份投递／局部撤销／替换／重连、旧 hint／空证明／权威故障及无 hint 的成员／账号 proof 周期变化；三个自有数据库均 `database_removed=true pools_closed=true`。 |
| 活动 SDK 生成产物检查 | **20 个产物核验 PASS** | [sdk-check.log](../../../../local-identity-sso-phase06-qa/sdk-check.log)；SDK 3.0.0，检查模式，没有重写固定发行物。 |
| SDK Node 测试 | **30／30 PASS，5.23 秒** | [sdk-tests.log](../../../../local-identity-sso-phase06-qa/sdk-tests.log)。 |
| repository contracts | **10／10 PASS，2.07 秒** | [repository-contracts.log](../../../../local-identity-sso-phase06-qa/repository-contracts.log)。 |
| 平台前端完整测试 | **继承 Phase 5：456／456 PASS，9.70 秒** | 本阶段前端源码未变；真实既有日志已复制为 [frontend-tests.log](../../../../local-identity-sso-phase06-qa/frontend-tests.log)，不是新执行一次的声明。 |
| 平台前端构建 | **继承 Phase 5：PASS，Vite 18.44 秒** | [frontend-build.log](../../../../local-identity-sso-phase06-qa/frontend-build.log)，含 TypeScript 与 401 files／SDK 3.0.0 dist 检查。 |
| 平台前端 lint | **继承 Phase 5：0 errors／46 warnings** | [frontend-lint.log](../../../../local-identity-sso-phase06-qa/frontend-lint.log)。 |

上述证据只支持各自记录的范围，客户与通知专项成功不单独证明收缩或 readiness 启动已通过本阶段门禁。没有因接口说明和 SDK 通过而补造其他仓库 Rust／数据库运行结果。

客户六项专项均由 [customer_identity_tests.rs](../../../src/customer_identity_tests.rs)通过真实 HTTP 路由与 PostgreSQL 权威执行，实际结果如下：

| 实际测试名 | 结果与所验证行为 |
| --- | --- |
| `phase06_real_customer_oauth_reads_only_its_registered_minimal_profile` | **PASS**；真实 password／refresh 交换、持久证明、独立 context／profile／applications 精确字段、仅登记应用及非空 query 代读拒绝。 |
| `phase06_dual_identity_requires_exact_selection_and_keeps_credential_kinds_separate` | **PASS**；同账号同组织的员工与客户显式选择，不合并权限；真实员工 session／app 与机器凭据不能冒用客户资源。 |
| `phase06_customer_entry_requires_exact_server_registration_scope_and_type_intersection` | **PASS**；缺登记 403、精确来源 client／app、token 与当前 client 的 `profile` scope，以及客户 client＋员工 app 和反向类型交集负例。 |
| `phase06_customer_reads_revalidate_every_live_authority_and_policy_generation` | **PASS**；18 类变化逐次撤权：成员停用／删除／版本、账号停用／版本、client 停用／版本、app 停用／版本、租户／组织停用、租户／组织授权移除、client／app 类型收紧、存储 token 过期／撤销及当前 client scope 移除。 |
| `phase06_local_revocation_keeps_other_identity_and_other_organization_independent` | **PASS**；员工与客户双向局部撤销均保留另一身份和另一组织的独立有效性。 |
| `phase06_customer_authority_unavailable_returns_503_without_cached_profile_access` | **PASS**；关闭资源权威连接池后，三个 GET 均为 503，管理池继续完成自有数据库清理。 |

## 已发生的失败与基础设施恢复

1. 首次编译失败为 `OsString` 不能转换成 `anyhow::Error`，发生在独立 rollout 参数错误映射。原始 [initial-check.log](../../../../local-identity-sso-phase06-qa/initial-check.log) 保留 E0277；该错误映射已修复，修正后的集中 check、Clippy 与客户专项编译已通过。
2. 第二次运行因专用 PostgreSQL 已停止而以基础设施状态 125 中断，没有实际 Cargo 测试结果。检查显示 PostgreSQL 容器曾收到 fast shutdown 后干净退出，观察退出码 0、`OOMKilled=false`，发起来源未确定。不能把它描述为应用测试失败或猜测原因。
3. root 只重启本任务自有 PostgreSQL 容器，确认 **PostgreSQL 17.6 ready**，非模板数据库仅 `postgres`。事实记录见 [infrastructure-restore.json](../../../../local-identity-sso-phase06-qa/infrastructure-restore.json)。没有重启、迁移或读取独立 UUID／预览资产。
4. 恢复后集中静态检查和六项客户专项通过，但首轮三项 rollout 专项为 **0 PASS／3 FAIL，33.64 秒**。`membership_rollout_phase06_development_startup_requires_live_receipt`、`membership_rollout_phase06_inspect_apply_and_identity_contract`、`membership_rollout_phase06_refusals_are_atomic` 均在 rollout 报出 `MEMBERSHIP_ROLLOUT_SCHEMA_INVALID` 后失败；三项自有数据库清理均为 PASS。原始 [customer-and-static-gates.log](../../../../local-identity-sso-phase06-qa/customer-and-static-gates.log)保留失败。根因为 PostgreSQL 17.6 把客户 CHECK 的保留字列反编译为 `COALESCE("position", ...)`，原精确归一化与裸 `position` 比较而误拒绝；仅补充确切 `"position"` 标识符归一化，并回归验证弱化 `AND` 为 `OR`、改为不同标识符 `"Position"` 仍拒绝。实际 catalog 诊断见 [schema-diagnostic.log](../../../../local-identity-sso-phase06-qa/schema-diagnostic.log)，自有诊断库已清理。修复后同三项在 [rollout-final-gates.log](../../../../local-identity-sso-phase06-qa/rollout-final-gates.log)实际 **3／3 PASS，234.77 秒**，三项自有清理均 PASS；首轮不改写，fixture 通过也不替代实际任务环境批准。
5. main target 完整 ignored 运行覆盖 **178 项，176 PASS／2 FAIL，648.87 秒**，不包含另列的集成 target 三项 rollout。`routes::admission_policy_tests::admission_policy_api_contracts_and_versions` 与 `routes::admission_policy_tests::admission_policy_audit_atomicity_and_concurrent_conflicts` 均因 `admission API returned 400 Bad Request` 失败。两项明确为 `assertions_ok=false`，但 `database_removed=true pool_closed=true`；其他已通过项的自有清理也成功。原始 [backend-full-baseline.log](../../../../local-identity-sso-phase06-qa/backend-full-baseline.log)保留该轮非全绿结果。客户六项、平台通知九项及组织人数两项在本轮均 PASS。
6. 准入 fixture 仍向严格的控制台 context 请求提交不接受的 `memberType` 字段；只修正为实际 `userId`＋具体 `membershipId`／`organizationId` 选择，并加强响应中用户、成员、类型和组织四元证明断言，没有放宽生产请求 DTO。随后同两项在 [policy-final-gates.log](../../../../local-identity-sso-phase06-qa/policy-final-gates.log)实际 **2／2 PASS，6.55 秒**，断言、移库与关池均 true。最终 main 的 178 项是 **176＋2 分组通过**，首轮失败事实保留，没有宣称修复后重新执行了完整 178 项。
7. 真实任务新库收缩成功后，第一次 `apply-start` 仍以 **`DEVELOPMENT_HEALTH_FAILED`** 失败，原始 [development-apply-start.log](../../../../local-identity-sso-phase06-qa/development-apply-start.log)及[首次启动结果](../../../../local-identity-sso-phase06-qa/development-first-attempt/results/development-start.json)保留；实际为退出 1、health `NOT_REACHED`，输出已观察到 `MEMBERSHIP_READINESS_VERIFIED`。主执行实际核对 PostgreSQL 日志，失败为地区 seed 的 `COPY regions`（665,923 行输入）被默认 5 秒 statement timeout 取消；正确 receipt 在 seed 前已经验证通过，不能归类为 receipt 拒绝或把 health 失败写成成功。操作脚本只增加本次初始化预算，在同一目标使用已有 receipt 恢复启动，不重新 apply、不删除或伪造旧结果。
8. 实际 `resume-start` 使用 `DB_STATEMENT_TIMEOUT_MS=60000`、`DB_LOCK_TIMEOUT_MS=30000`，保持原 backend 文件、manifest 和 receipt。[resume-preflight.json](../../../../local-identity-sso-phase06-qa/development-final-evidence/attempts/resume-1416df0c213f79be/results/resume-preflight.json)重新核对真实目标、文件、schema 与实际 receipt，保留原 apply／失败启动／三个负例的路径和 SHA。随后实际 health 200／UP、HTTP 30／30、smoke 子进程退出 0，后端观察到 `MEMBERSHIP_READINESS_VERIFIED` 和 `SHUTDOWN_REQUESTED`，最终退出 0。恢复报告明确 `contraction_executed=false`，没有重建 backend、重新生成 manifest 或再次 apply；失败记录保持原字节。

## 真实任务目标与最终验收

[customer_identity_tests.rs](../../../src/customer_identity_tests.rs)只读显式 `IDENTITY_PHASE06_TEST_DATABASE_URL`，要求选中 `postgres` 管理库，只创建／迁移／清理 `identity_phase06_*` 自有随机数据库。上述六项已实际通过；成功用例通过真实 OAuth password／refresh 交换核对持久证明，不以手工签 JWT 或插 token 行代替成功链。

独立 rollout 的新增三项回归覆盖输入、收缩和 readiness 边界，其中合成 manifest 只证明测试输入合同。本次另用真实日志与文件制作任务 manifest，在独立新 development 目标执行显式工具和非 test 启动，实际凭据不复用测试 fixture。

本任务实际新 development 目标已完成 prepare，[development-prepare.log](../../../../local-identity-sso-phase06-qa/development-prepare.log)返回 `PREPARED`。目标为 `identity_phase06_development_01a16ef0f9cd7025aef9ccb4b6e8a2bd`，本次数据库 OID 为 `1158337`，实际 cluster system identifier 为 `7700230159816609830`，与显式选定集群一致。该目标独立于各测试 fixture 与预览资产；prepare 本身只证明准备完成，后续收缩、receipt、启动和 HTTP 分别记录。

[development-manifest.log](../../../../local-identity-sso-phase06-qa/development-manifest.log)随后实际返回 `MANIFEST_GENERATED`，包含 **7 项 inventory／10 项 gate**，manifest SHA256 为 `bb4bca570aa241fc463a1afd45d8e14646de9e69e15e2cefd60f01b3c02541b3`。真实 [apply-report.json](../../../../local-identity-sso-phase06-qa/development-first-attempt/results/apply-report.json)记录旧二元键已不存在，收缩和 receipt `01a16ef5-6465-7473-9446-34a64a61dfd6` 已提交；绑定实际 backend SHA256 `5d686e09a8de3e4b3391308e3d1f94a8a78332e0839d7138e3f64c918468e038`。本次三个启动负例均实际 PASS：

| 真实启动负例 | 实际结果 |
| --- | --- |
| [缺 receipt](../../../../local-identity-sso-phase06-qa/development-first-attempt/results/start-missing-receipt.json) | 退出 1，`MEMBERSHIP_READINESS_REQUIRED`。 |
| [错误 manifest SHA](../../../../local-identity-sso-phase06-qa/development-first-attempt/results/start-wrong-manifest.json) | 退出 1，`MEMBERSHIP_READINESS_VERIFICATION_FAILED`。 |
| [错误 canonical receipt](../../../../local-identity-sso-phase06-qa/development-first-attempt/results/start-wrong-receipt.json) | 退出 1，`MEMBERSHIP_READINESS_VERIFICATION_FAILED`。 |

正确 proof 的首次启动越过 readiness 校验后出现上文 seed timeout。恢复成功的证据位于 [development-final-evidence](../../../../local-identity-sso-phase06-qa/development-final-evidence)，同时保留首次失败；最终 [development-start.json](../../../../local-identity-sso-phase06-qa/development-final-evidence/attempts/resume-1416df0c213f79be/results/development-start.json)与 [http-smoke.json](../../../../local-identity-sso-phase06-qa/development-final-evidence/attempts/resume-1416df0c213f79be/results/http-smoke.json)确认正常非 test 启动和 30 项真实 HTTP 验收均通过。

主执行随后实际查询该任务库，确认地区记录 `665923`、导入完成标记 true、ready receipt 恰好 `1` 条、同账号同组织双身份组合 `1` 对、旧二元唯一键不存在／三元唯一键存在，且服务连接剩余 `0`。服务已干净停止，任务数据库保留供审查；没有保留开启三项写开关的后台服务。

| 验证项目 | 当前状态 |
| --- | --- |
| 集中格式／check／Clippy、客户六项 PostgreSQL／HTTP 专项 | **PASS；客户 6／6**，原始日志与自有资产清理见上文。 |
| rollout 三项 PostgreSQL 专项 | **修复后 3／3 PASS，234.77 秒**；首轮 0／3 与 `MEMBERSHIP_ROLLOUT_SCHEMA_INVALID` 保留，两个运行的清理均通过。 |
| 后端 main 的 178 项 ignored 基线 | **逐项分组 PASS：176＋2**；保留首轮 176 PASS／2 FAIL 和修复后两项 6.55 秒定向复验；三项 rollout 另列。 |
| 普通后端测试与文档检查 | **修复后普通 193／193 PASS**，含登记配置 RFC variant 与 PostgreSQL 17 CHECK 回归；最终 bins build 59.96 秒／doc 26.81 秒 PASS。 |
| 组织人数双行 PostgreSQL 回归 | **PASS；2／2**，普通租户与平台租户全量基线均通过。 |
| 通知服务及真实通知授权／局部撤销基线 | **PASS；服务 36 项普通＋3 项 PostgreSQL，平台 9 项 PostgreSQL**，本阶段实际执行并清理自有资产。 |
| 真实机器产物与本任务资产／gate manifest | [gate-results.json](../../../../local-identity-sso-phase06-qa/gate-results.json)十项均有真实日志摘要并说明失败重跑／继承，34 个 artifact 引用已逐个核对磁盘 SHA 一致；[inventory-review.json](../../../../local-identity-sso-phase06-qa/inventory-review.json)为 REVIEWED、完整且仅 TASK_OWNED 新库范围。真实 manifest 已生成，7 项 inventory／10 项 gate；未申请生产 inventory 批准。 |
| 任务自有目标的 inspect／apply 与非 test development 启动 | **PASS**；真实 prepare／manifest／apply、三项启动负例、同目标已有 receipt 的恢复启动及 health 200／UP 均有实际记录，首次 seed timeout 历史保留。 |
| 收缩后同类型重复／并发 409、员工 ID／授权／三元 UPSERT 与双向局部隔离 | **PASS**；真实收缩后三十项 HTTP 与既有角色／授权／凭据／通知基线联合验证，具体成员 ID、另一身份和旧 bearer 行为均有断言。 |

本次启用清单如下，范围仅限这一任务新库及一次受控 development 服务：

| 启用条件或边界 | 本次实际状态 |
| --- | --- |
| 全阶段代码与活动消费者门禁 | 十项 gate PASS；main 178 为 176＋2 分组证据，固定 SDK／线协议保持原合同，前端继承来源已注明。 |
| 实例与写入器 | 7 项 REVIEWED inventory 仅覆盖 TASK_OWNED 新库；实际 backend／migrate／rollout 文件有真实 SHA，未部署的导入器、批任务等按本新库范围记不适用，没有冒称未知生产实例已清退。 |
| 独立收缩和 proof | 实际 apply 后二元键不存在、三元键存在，receipt 绑定同数据库名／OID／cluster、环境、backend、manifest、迁移与 schema；恢复时再次权威校验。 |
| 受控开关 | 仅本次 `APP_ENV=development`、`APPLICATION_ENVIRONMENT=development` 进程以三项成员开关 true 启动；HTTP capabilities 精确返回 lifecycle／customer／dual 均 true。 |
| 实际创建与局部变更 | 管理员创建自有 fixture 账号，客户缺绑定确认被 400 拒绝、未知组织 404；明确确认后同组织员工＋客户各有独立 ID；重复及并发、UPSERT、双向停用和 bulk 的 30 项 HTTP 全通过。 |
| 结束与保留 | 服务退出 0、剩余服务连接 0；新库和脱敏证据保留供审查，默认配置继续关闭，未修改生产注册或开关。 |

## 六项验收标准

六项标准均在本任务已声明的新库范围完成，依据为实际执行结果与源码审查的联合证据：

| 台账标准 | 当前结论 | 所需实际依据 |
| --- | --- | --- |
| 客户仅能读自己的有效 profile/context，不能读其他客户、跨租户或员工授权 | **PASS** | 客户六项真实 OAuth／HTTP：精确本人 DTO、任意 query 代读拒绝、实际成员与租户组织权威、员工权限隔离及 503；继承角色／目录回归在 main 基线通过。 |
| 旧应用不能给客户 app token；显式登记最小客户入口可用，未登记拒绝 | **PASS** | 客户六项的缺注册 403、错误来源／应用／scope、旧员工应用和真实 session／app／机器凭据负例，以及真实 password／refresh 本人资料正例；未放宽旧控制台入口。 |
| 成员类型∩来源 client∩app、组织及策略版本；双集合单成员；机器拒绝 | **PASS** | 客户 client＋员工 app 与反向组合、同组织双行显式选择、18 类实时状态／策略／版本撤权；准入 API 两项修正 fixture 后 2／2 通过。 |
| 收缩前全部实例／写入器 ready；收缩后双行、同类型重复／并发 409及员工保留 | **PASS，本任务新库范围** | 写入器审查及 7 项真实清单；旧 binary 仅在该新库范围不适用。真实 manifest／catalog／apply／receipt、三项 rollout 与收缩后 HTTP 确认独立双行、200／409、员工 ID 和三元 UPSERT 保留。 |
| 第二行开启后 bulk 与双向停用不误写／误撤另一身份，无首行歧义 | **PASS** | main 的员工／客户／平台通知基线、通知服务三项、组织人数 2／2；收缩后真实 HTTP 验证另一行原 ID／版本保留、员工 bulk 仅删除／新增员工、旧 bearer 不能复活。 |
| 阶段 1～6 全部门禁及独立收缩后才开启；校验真实目标与绑定依据 | **PASS，本任务新库范围** | 十项真实 gate、34 个 artifact 引用摘要、真实目标 name／OID／cluster 与文件绑定、独立 apply；三个失败启动及正确 receipt 恢复成功。真实 HTTP 绑定确认／未知组织负例与生命周期原因／管理范围既有基线共同通过。 |

正式创建默认配置继续关闭。本次只在明确登记的 `TASK_OWNED` development 新库完成受控启用与验收，服务已停止，不声称生产环境已允许客户或第二身份。未来真实生产目标必须另有获批准的资产清单、对应二进制与环境证据；本任务新库的完成不能代替它们。预览 UUID 专项及其清退／部署状态继续独立维护，Phase 7 及后续 SSO／OIDC／门户内容也不由本阶段证据冒领。
