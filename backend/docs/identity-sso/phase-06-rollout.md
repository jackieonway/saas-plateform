# Phase 6 独立收缩与环境启用

三个成员写入开关 `MEMBERSHIP_LIFECYCLE_ENABLED`、`CUSTOMER_MEMBERSHIPS_ENABLED`、`DUAL_MEMBERSHIPS_ENABLED` 默认均为 `false`。正常迁移链保留 `memberships_user_id_organization_id_key`；启动迁移不会自动删除该二元唯一键。客户及同账号同组织第二身份需要单独完成本页的收缩操作与环境就绪验证。

`APP_ENV=test` 且 `APPLICATION_ENVIRONMENT` 非 production 保留 Phase 1～5 隔离 fixture 的显式写入能力。它是测试设施兼容分支，不证明开发、预发布或生产实例已就绪。环境实际就绪必须通过非 test 服务启动、真实数据库与当前后端可执行文件验证。

## 操作边界

`platform-membership-rollout` 是与后端一起构建的薄入口，其流程在 `database::identity_readiness`。它只读取进程中的 **`MEMBERSHIP_ROLLOUT_DATABASE_URL`**；不会读取 `DATABASE_URL`、`TEST_DATABASE_URL`、`.env`、`APP_ENV_FILE`，不会猜测目标，也不会创建数据库、执行普通迁移、seed、地区初始化、worker 或 HTTP。URL 必须明确指定 PostgreSQL 主机、账号与业务库，拒绝维护库、全部 `PG*` 环境变量、libpq 重定向与未认可 query，使用已有迁移工具的显式 URL 解码和无 pgpass 构造。

连接预算 10 秒、操作总预算及 statement timeout 300 秒、lock timeout 30 秒。失败仅输出固定错误类别；成功输出单行 JSON，不包含 URL、密码、证据路径或证据内容。数据库名、数据库 OID、集群 system identifier 与摘要属于操作交接材料，不应公开服务连接凭据。

数据库必须先通过 `platform-migrate` 独立准备至当前完整版本。操作账号必须具有读取集群 system identifier、锁表、修改成员约束及创建就绪凭据表的权限；收缩工具不会自动赋予权限。

## 先检查当前目标

构建与测试完成后，对明确的新目标执行：

```powershell
# 由操作环境安全注入 MEMBERSHIP_ROLLOUT_DATABASE_URL；不在命令行传凭据。
platform-membership-rollout inspect --platform-binary <backend-executable> --environment development
```

`inspect` 使用只读 repeatable-read 事务。它验证真实 SQLx 迁移版本及 checksum（保留已登记的精确历史 v1 兼容）、成员列与四项 CHECK、有效三元唯一键、类型／归属不可变及版本单调 trigger 的实际函数体，并检查旧二元键的依赖外键与替代唯一索引。缺少或变造这些结构会拒绝；不会通过猜测或 `CASCADE` 修复。

报告字段如下；`receipt_id` 与 `manifest_sha256` 只在成功 `apply` 出现。

| 字段 | 用途 |
| --- | --- |
| `schema_version`、`operation`、`environment` | 报告版本、操作和目标环境 |
| `database`、`database_oid`、`cluster_system_identifier` | 从目标连接实际读取的数据库与集群身份 |
| `platform_binary_sha256` | `--platform-binary` 文件的 SHA256 |
| `binary_key_present` | 旧二元唯一键是否仍在 |
| `migration_sha256` | 全部真实 SQLx 版本、成功状态与 checksum 的有序摘要 |
| `schema_sha256` | 使用已有 `schema_material.sql` catalog 合同生成的完整结构摘要 |
| `receipt_id`、`manifest_sha256` | 收缩／更新凭据的 UUIDv7 和原始 manifest 字节摘要 |

## 汇总真实资产与验证结果

manifest 是部署负责人对资产清单完整性及证据来源的明确声明。工具不会自动发现未登记的服务器、离线导入程序或定时任务，文件摘要也不能替代人工核实实际部署状态。未知资产保持未就绪，不得用文档中的承诺替代完成证据。

每个证据文件必须真实存在、非空、为普通文件且不超过 16 MiB，SHA256 与 manifest 相同。路径只能使用相对于 manifest 所在目录的正向路径分量，例如 `results/backend.json`；拒绝绝对路径、反斜杠、`..` 与解析后逃逸到目录外的符号链接。manifest 最大 1 MiB，最多 128 项资产，拒绝未知字段、重复资产 ID、重复／缺失 gate 与未通过状态。`generated_at` 必须在当前时间过去 24 小时内，未来偏差不超过 5 分钟。

manifest 顶层字段：

| 字段 | 约束 |
| --- | --- |
| `schema_version` | 整数 `1` |
| `environment`、`scope` | `development` + `TASK_OWNED`，或 `production` + `PRODUCTION`，必须成对 |
| `target_database` | 与 inspect 的 `database` 完全一致 |
| `target_database_oid` | 与 inspect 的 `database_oid` 完全一致，正整数 |
| `target_cluster_system_identifier` | 与 inspect 的 `cluster_system_identifier` 完全一致，十进制字符串 |
| `platform_binary_sha256` | 当前待运行后端的 64 位小写 SHA256 |
| `generated_at` | RFC3339 时间 |
| `inventory` | 资产数组，结构见下 |
| `gates` | 验证数组，结构见下 |

每项 inventory 含 `id`、`kind`、`status`、可选 `binary_sha256` 和 `evidence: {path, sha256}`。必须覆盖以下全部 kind：`PLATFORM_INSTANCE`、`CLI`、`IMPORTER`、`BATCH`、`OFFLINE_JOB`。同类可以有多个资产，所有实例及离线写入器都必须列入。状态含义：

| 状态 | 必需证据 |
| --- | --- |
| `READY` | 已升级为按具体成员定位、显式成员类型及三元唯一键安全的资产；必须记录其可执行文件 SHA256 |
| `RETIRED` | 实际停机／撤销执行权限／禁用调度的证据；如仍保留旧文件，可同时记录 hash |
| `NOT_APPLICABLE` | 经实际清单核实不存在该类资产的证据；不填写 binary hash |

至少有一项 `READY` 的 `PLATFORM_INSTANCE`，其 hash 必须等于待运行后端 hash；其他所有 READY 平台实例也必须相同。不能把从未存在的 importer 或任务写成“已退休”。

每项 gates 含 `id`、`status: "PASS"` 和 `evidence: {path, sha256}`。恰好包含 `backend`、`frontend`、`notification`、`contracts`、`phase_01`、`phase_02`、`phase_03`、`phase_04`、`phase_05`、`phase_06` 十项。可以引用同一份新执行的完整基线机器结果，只要结果真实覆盖相关阶段；不得补造过去未保存的日志或把未运行项标成 PASS。工具回归测试中的合成 manifest 仅用于正负输入合同，不构成这里的环境验收结果。

## 显式收缩并记录凭据

```powershell
platform-membership-rollout apply --manifest <reviewed-manifest.json> --platform-binary <backend-executable>
```

工具先验证文件及摘要，再开启事务、获取本操作专属 advisory lock 与 `memberships ACCESS EXCLUSIVE` 锁，重新检查实际数据库身份、迁移与成员结构。首次操作只接受保留已知旧键、且还没有就绪凭据表的 expansion 状态。存在指向旧二元键的外键、未登记的其他二元唯一索引或类型不可变保护异常时，整个操作拒绝。

实际 DDL 在 `ops/identity/phase-06-contract-memberships.sql`，只使用 `DROP CONSTRAINT ... RESTRICT`，保留三元唯一键、数据、成员类型和不可变 trigger。随后同一事务创建 `ops/identity/phase-06-readiness-receipts.sql` 声明的 `public.identity_membership_readiness_receipts`，记录环境、范围、真实数据库／集群身份、后端文件／manifest／迁移／完整结构摘要、资产清单和验证引用，最后提交。失败不会留下半完成的收缩或凭据；不伪造 SQLx 历史，不把 receipt 当作新迁移。

此表由 rollout 操作账号拥有，owner 必须与 `memberships` owner 一致，PUBLIC 没有写权限。分离低权 runtime 的部署需显式授予该表 SELECT，并保持 runtime 无 receipt 写入权。当前集群核验调用 `pg_catalog.pg_control_system()`，低权 runtime 还需操作员明确授予该 catalog 函数的读取执行权限；它不增加业务表权限。后续独立服务／部署角色收敛时应使用受控 owner、固定 `search_path=pg_catalog`、撤销 PUBLIC 的窄 helper，仅返回 system identifier。任务自有 owner 连接的演练不等同于生产低权角色已验收。

后端重建、迁移或受审查的结构变更之后必须重新执行完整门禁并生成新 manifest，然后显式 `apply` 更新凭据。已收缩目标必须存在同数据库／环境的历史 receipt；工具不会接受无来源的手工删键数据库。凭据追加保存，不会删除历史记录。

## 服务启动门禁与客户入口

非隔离 test 环境在开启任一成员写入开关时，必须成对设置：

```text
MEMBERSHIP_READINESS_RECEIPT=<apply 返回的 canonical UUIDv7>
MEMBERSHIP_READINESS_MANIFEST_SHA256=<apply 返回的 64 位小写 SHA256>
```

启用客户或双身份还要求 `MEMBERSHIP_LIFECYCLE_ENABLED=true`。服务在普通迁移或 imported 只读验证完成后、任何 seed／地区初始化／worker／listen 之前，验证 receipt 指定的实际环境、数据库名／OID／集群 system identifier、当前正在运行的可执行文件 hash、全部迁移及完整 catalog 结构。旧键仍在、缺失／错误 receipt、错误环境／目标／文件、迁移变化或结构漂移都会关闭连接并拒绝启动。启动不会读取 manifest 路径，也不会因 README 的声明而启用。

`PLATFORM_DATABASE_INIT_MODE=imported` 的 UUID 导入交接合同独立保留。收缩会改变结构摘要，必须同步完成该独立目标的可信导入交接更新，并分别满足 imported 验证及本 receipt 验证；不能用 Phase 6 receipt 绕过导入标记、低权身份或凭据轮换要求。

客户最小入口还须成对配置 `CUSTOMER_PROFILE_CLIENT_REF`（`oauth_clients.id`，不是公开 `client_id`）与 `CUSTOMER_PROFILE_APPLICATION_ID`，两者均为 canonical RFC UUIDv7。未配时客户用途关闭。该 pair 只登记固定 `CUSTOMER_PROFILE` 用途；实际成员、来源 USER client、该 client 的应用绑定与应用准入、版本及租户／组织关系仍需逐请求满足。它不会把旧员工专用应用、CurrentUser、SDK 或旧 Context 合同开放给 CUSTOMER。

本页给出可执行门禁与操作合同；只有相应环境的真实资产清单、完整门禁结果和 apply／启动验证记录才构成该范围的就绪证据。任务新库的 `TASK_OWNED` 验证不代表生产部署已启用。
