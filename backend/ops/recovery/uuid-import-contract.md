# UUID-U04 独立三库导入合同

`uuid-import` 将已经认证的源包、转换包、类型化映射和逐行证明导入明确登记的 U04 预演目标。创建数据库、真实迁移、服务停写和以后切流由操作者执行。入口不创建数据库、不执行 seed、不修改旧源库、不重写迁移历史，也不自动清理失败目标。实际阶段验收以 [UUID 台账](../../docs/identity-sso/uuid-v7-task-ledger.md) 与 validation 为准。

## 固定目标与初始化

仅接受本机 `/run/postgresql:5432`、`postgres` peer 身份以及下列三组目标。不会接受旧源名、其他本地库、远程连接、`/var/run` 别名或自选超级用户。

| kind | 全新 U04 目标 | 登记 owner |
| --- | --- | --- |
| platform | `qiankun_platform_uuid7_u04_20261024` | `qiankun_platform_uuid7_u04_owner_20261024` |
| stores | `qiankun_stores_uuid7_u04_20261024` | `qiankun_stores_uuid7_u04_owner_20261024` |
| work_orders | `qiankun_orders_uuid7_u04_20261024` | `qiankun_orders_uuid7_u04_owner_20261024` |

操作者先核实这些名称与 owner 均未使用，以 `template0` 建立独立目标，禁止 PUBLIC CONNECT；新 owner 保持 `NOLOGIN`、非超级用户。通过三个服务的真实 migration-only 入口，以登记 owner 建立表、函数、索引和序列。不要启动普通后端 bootstrap：平台普通启动会执行演示数据、665,923 行地区 seed、菜单授权和后台任务。

导入前将目标库的持久 `default_transaction_read_only` 设为 `on`，排空其他连接与 prepared transaction，保持全部 API、worker、replay、outbox 和 agent 停止。导入器使用唯一受控 `postgres` 通道，显式建立 `READ COMMITTED READ WRITE` 事务。除该身份外的 LOGIN 角色不得有效 CONNECT、写目标数据、CREATE schema 对象，或通过 SET ROLE／ADMIN OPTION 获得登记 owner 权限。新 owner 在导入后仍保持 NOLOGIN，目标默认只读保持开启。

每个目标的 `expected_schema_sha256` 必须由真实迁移后的完整 `schema_material` 规范 JSON 计算并审查，再写入源码外私有配置。其计算与源清点一致：严格解析 JSON，按 serde_json 当前规范序列化 `schema_material` 后 SHA-256。不要直接把 U01 源 hash 当作目标 hash，U02 已添加 v7 CHECK、删除默认值，门店还新增 v3 metadata。

该完整摘要绑定具体的数据库初始化代，其中包含 PostgreSQL 为内部外键触发器生成的 `RI_ConstraintTrigger_[ac]_<OID>` 名称。同一迁移链重建数据库后，系统 OID 及相应原始摘要会改变。恢复演练后的新一代目标须重新保存 catalog、核验真实迁移历史和逻辑结构，再通过新配置登记经审查的完整原始摘要；旧配置和失败证据保留。不能将前一代的摘要直接作为新一代预期，也不能未经审查就用现库摘要替换预期。

审查时可仅在离线副本中规范化上述内部触发器自身名称，以及 definition 中唯一精确引用该名称的 token，并比较完整多重集；其余结构项必须逐项完全相等，真实迁移 checksum、外键、触发器状态和角色围栏也必须通过。生产导入和后续 `imported` 启动仍严格核对本代未经规范化的完整原始摘要，不改变 hash 算法或已记录的 schema fingerprint。

## 配置与运行

复制 [uuid-import.example.json](uuid-import.example.json) 到源码外，把目录和三个指纹占位值替换为实际值。配置／key 必须为执行用户拥有的 mode 0600 常规文件，目录为私有 mode 0700。key 为原始 32 字节随机值，沿用该源与转换包的加密 key。`scratch` 必须是 tmpfs；拒绝符号链接、相对路径和 `.`／`..`。`output_dir` 必须是新目录，不能与源包、转换包或 key 重叠。

`mode` 只能为 `import`，`seed` 必须明确为 `false`。任何同时选择 seed 的配置在连接数据库前拒绝。

```sh
UUID_TARGETS_QUIESCED=true saas-recovery uuid-import /absolute/private/uuid-import.json
```

环境变量是操作者断言，不能替代实际目标身份、权限、连接与 prepared 检查。stderr、行数据、映射和明文报告仅在私有 tmpfs 中；stdout 只输出阶段状态、计数、batch 与摘要，不输出行 UUID、密码、token 或连接 URL。

## 验证与导入

首次目标写入前，复用 `uuid-verify-package` 的完整认证回读：检查 source／Converted manifest、实际源 manifest hash、registry 原始字节、六份 admission proof、源 schema、序列、完整表集合、派生文件名与 AEAD purpose、typed map 独立来源覆盖，以及每个文件的完整 authenticated footer。然后逐行复算 source hash、target hash 与 owner projection proof，拒绝缺失、额外、重复字段、截断、超限行或不连续 proof index。

三个目标先全部完成无写 preflight，再逐库导入。目标必须只有真实迁移 metadata；业务表为空，平台只额外允许初始迁移建立的默认 `platform_settings` 单例。迁移 metadata 不取自源表：平台完整验证 12 项、门店 3 项、工单 4 项 SQLx SHA384 历史；工单同时验证四项 legacy SHA256 历史，门店验证真实 v1/v2/v3 schema fingerprint 链。任何未知版本、失败记录、错误摘要或伪合同均拒绝。

门店历史 fingerprint 对 PostgreSQL deparse 的 `search_path` 敏感，因此 metadata 与提交前合同验证仅在固定查询范围使用真实 migrator 的 `public,pg_catalog`，随后恢复导入器的 `pg_catalog`。不修改 fingerprint／迁移 metadata 或放宽目标 schema hash 与 owner／ACL。PG18 的 NOT NULL 约束 `contype='n'` 同样参与精确源目标对账，缺失、变化、未验证或未知约束类型均拒绝。

每表按 FK 图顺序处理；复合 FK 保持整组，同表 self-FK 使用整表单条 INSERT。按认证源流顺序生成 `(ordinal, JSON)` 标准 CSV，COPY 至 `pg_temp` staging。不能将 NDJSON 原样交给 COPY text：后者解释反斜线，会损坏业务 JSON 或文本。SQL 校验完整列集合、连续 ordinal、PK 与 JSON→真实列类型的完整转换，再显式列出全部列执行 `INSERT SELECT`。任意精度 JSON 数字不经过 f64，真实 PostgreSQL 类型负责金额、日期、时间、数组等转换；转换发生舍入、截断或值变化则拒绝。

插入后按真实完整 PK JOIN 回目标，并依照 staging ordinal 流式输出实际行。导入器再次绑定源行、转换行与逐行 projection proof。新 UUID 可能改变主键排序，不能改用目标 `ORDER BY 新主键` 后按位置核对。地区数据完整保留源版本、导入标记、手工来源、墓碑与时间，不重新生成地区 seed。

`platform_settings` 只在验证目标仍是初始迁移的完整默认单例后替换该单例。`store_staff` 只在本库恢复事务内具名暂停 `store_staff_legacy_read_only`，插入历史行后立即恢复；其他不可变／append-only 保护保持启用。不使用 `DISABLE TRIGGER ALL`、`session_replication_role=replica` 或通用 `ON CONFLICT DO NOTHING`。

历史组织／部门层级触发器内部使用未限定 schema 的表名，须按正常业务语义解析。每表显式 INSERT 仅在该语句期间设置 `search_path=pg_catalog,public`，完成后立即恢复 `pg_catalog`，再回读对账；catalog 始终优先。`public` 的对象集合、owner 与 CREATE 权限已受完整 schema 指纹和准入围栏约束，不改函数定义或禁用层级触发器。

所有 jsonb 列使用源 `body->column` 构造真实复合行，保留 NOT NULL jsonb 的顶层 JSON null。原协议不能区分 nullable jsonb 的 SQL NULL 与顶层 JSON null，因此保留行中遇到该歧义即拒绝，不能猜测。本次已登记的 nullable 列为平台 `managed_deployment_jobs.receipt` 与 `zip_distribution_nodes.receipt`；真实源聚合证据另行记录，不因它们的当前零行而移除未来门禁。

提交前恢复所有正常触发器，验证 FK／v7 CHECK、索引、组织／部门／菜单整图无环、完整业务计数、迁移 metadata、schema hash 与角色／对象 owner／ACL。metadata 行数不与旧源相等，因为它们由真实新迁移生成。credentials 对应目标表必须保持零行。

审计 bigint 不变，序列精确恢复源 `last_value` 和 `is_called`；先验证下一候选值高于保留 audit ID，若源本身会碰撞或耗尽则拒绝。不会用 MAX(id) 覆盖更高的源序列位置。工单 `next_number`、金额、日期和业务 version 都是原样保留事实。

## 完成、重复拒绝与失败恢复

每个目标只有全部校验通过后，才在 COMMIT 前写数据库 comment `saas-uuid-import/1/<batch>/<converted-manifest-sha256>`。进入时要求数据库没有既有 comment；已标记的目标拒绝重复导入，即使某库业务表本来零行也不能复用。此标记不修改 `_sqlx_migrations`、schema fingerprint 或新增 public 表。

每库 COMMIT 后写受限 `committed-<kind>.enc`，并完整认证回读。三个库全部提交和验证完成后，最后才发布并回读 `complete.enc`。该完成文件使用 `saas-uuid-import-completion/1/<batch>` purpose，内部绑定源／转换 manifest SHA 与三目标报告。成功状态仍为 `QUARANTINED_THREE_TARGET_IMPORT_VERIFIED`，不会启动服务或执行切流。

三库没有原子 COMMIT，`setval` 也不随普通事务回滚。中途失败可留下已提交目标、comment 或已改变的 sequence；没有完成包时不能视为整批成功。保留部分密文目录隔离，依据本批明确目标清单与恢复手册，由操作者核对后清理并重新创建独立目标、重新运行真实迁移，再使用新的输出目录重试。导入器不会静默清空、修复或复用脏目标。

旧源、原始加密包、typed map 和运行证据继续留存；旧源保持只读，旧服务角色保持 NOLOGIN。U04 仅为预演，U06 最终停写／新目标／切流使用另外的明确合同和名称。

实现门禁、显式只读 SQL 诊断与独立 PostgreSQL 17 fence 回归的实际记录见 [uuid-import-tests.md](uuid-import-tests.md)。它们与登记目标的真实导入验收分别记录，不以 fixture 代替真实三库结果。
