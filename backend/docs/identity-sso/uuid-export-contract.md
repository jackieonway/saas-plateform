# UUID 保数据导出与交接合同

本合同用于 UUID-U01～U06 的本地预览实施。用户已选择仅处理本地预览三库；远程开发库和生产库未审计，也不连接或初始化。工具入口为 [recovery](../../ops/recovery/README.md)。[uuid-references.json](../../ops/recovery/uuid-references.json) 冻结 U01 审查事实；U03 导出和转换使用与编译版本逐字节一致的 [uuid-references-v2.json](../../ops/recovery/uuid-references-v2.json)，其中登记当前列、结构化路径和每表保留策略。本文分别说明 U03 离线文件合同和 U04～U06 必须落实的目标导入／切流门禁，不以离线包生成代替后续验收。

## 范围和保留

源为 WSL Ubuntu 本机 PostgreSQL 的 `qiankun_platform_preview_20261024`、`qiankun_stores_preview_20261024`、`qiankun_orders_preview_20261024`。源库、已有独立备份和历史发行材料均保留。目标只允许尚不存在、明确登记的独立预览库，初始化和导入不覆盖源库。旧库留存到用户另行决定删除，不由迁移工具自动清理。

默认保留账号与密码哈希、租户组织、成员角色授权、应用配置、门店与工单业务记录、审计、通知内容及状态、业务编号、金额、时间、序列和业务计数器。每表确切策略由注册表声明。会话、刷新、授权码、device、token 等凭据不恢复为可用状态；根据表合同丢弃临时行或保留已撤销元数据，签名字符串不能通过改 UUID 延长使用。密码哈希保持原字节，新登录重新认证并签发凭据。

历史签名、不可变审计证据、SDK 固定发行和归档保持原字节，并附映射索引。归档中的旧 UUID 不作为活业务引用。已签 manifest 或协议若需要作为活动配置使用，必须由 owner 重新生成并签名，不能只替换字段后保留旧摘要。

## 共同停写与源 schema

只读清点使用每库独立的 `REPEATABLE READ READ ONLY` 事务，只能证明各库自身的一致视图，不能当作跨库协调快照。实际导出须停止本地平台、门店、工单、通知、示例 RP 及相关 worker／scheduler／agent／CLI 写入，排空事务，记录所有写入者和共同停写边界，再顺序取得三库快照。库外安装／通知效果单独冻结和对账。

U03 `uuid-export` 每库在同一个只读 repeatable-read 事务中取得 catalog 和表数据，核对三库无其他源连接／prepared transaction，要求数据库级与当前会话的默认只读设置为 `on`，并在导出前后严格复核 snapshot、schema、表行数、序列集合与状态。集群 WAL 初末观察值分别保存为 `DatabaseSnapshot.lsn`／`final_lsn`，须格式合法且不倒退，不要求相等；它们包含同集群其他数据库及 checkpoint 等活动，不能用作源库停写或跨库一致性的独立证明。

角色准入由 [export/admission.rs 的 `Proof` 与校验器](../../ops/recovery/src/uuid_rekey/export/admission.rs) 提供，证明的 `format=saas-uuid-source-admission/1`。每个源 RR 建立前及其会话关闭后，分别从独立维护库 `postgres` 的 `READ COMMITTED READ ONLY` 观察事务读取当前角色与数据库准入状态，不使用源 RR 中的旧权限视图充当事后证明。三库共六份证明写入 `Snapshot.admission_before`／`admission_after`，两个数组各三项，位置对应 `databases`；每份均须独立通过校验，且六份身份与准入元数据完整相等。

准入证明必须确认固定旧角色 `qiankun_preview_20261024`、`qiankun_notification_preview_20261024` 都存在、非 superuser 且 `NOLOGIN`；除受控维护身份 `postgres` 外，不存在可 LOGIN 且可 CONNECT 任一源库的其他角色，包括其他 superuser，以及经 `PUBLIC` 或角色继承获得 CONNECT 的角色。证明还校验 `privileged_set`、`can_admin_source_roles` 并拒绝 LOGIN CREATEROLE 等可改变准入的提权旁路。观察时三源库默认只读、source backend 为零、prepared transaction 为零。操作者须持续冻结库外写入者，并确保维护期间可信受控的 `postgres` 不另开源写会话；此合同不宣称可抵抗恶意超级用户主动绕过维护围栏。认证保存的停写证据、六次新鲜准入观察与每源严格快照检查共同支撑业务一致性，不能只用 LSN 或一个布尔断言替代。

源 schema 指纹是导出 catalog 的 `schema_material` 经工具 JSON 序列化后取得的 SHA-256，保存在 `catalog.schema_fingerprint_sha256`，须与导出配置的 `databases[].expected_schema_sha256` 一致。配置使用 [U01 已审查摘要](validation/uuid-u01-catalog-summary.json) 中的原始源 schema 指纹；目标追加 UUID 迁移后的 schema 另行核验，不能覆盖源基线。完整源 catalog 保留原始 migration 版本／checksum、表列类型、约束、索引、触发器、函数和序列定义等证据；注册表校验精确覆盖表、UUID 列和有序 FK，未登记变化须先审查。

## U03 分离式离线包格式

源证据目录、转换目录及冻结运行材料共同组成交接集合，使用相互绑定的多个认证文件。以下是当前入口的实际 wire 合同，字段拼写以对应源码结构体为准。

| 项 | 必填内容 |
|---|---|
| 源 manifest | `format=saas-uuid-source-snapshot/1`；[export.rs 的 `Snapshot`／`DatabaseSnapshot`／`TableSnapshot`](../../ops/recovery/src/uuid_rekey/export.rs) 声明 `batch_id`、`frozen_ms`、`registry_sha256`、三库 `kind`／`name`／`catalog`／`snapshot`／`lsn`／`final_lsn`／`sequences`／`tables`，与三库位置对齐的 `admission_before`／`admission_after` 各三份证明，以及停写证据的文件名、purpose 和摘要 |
| 转换 manifest | `format=saas-uuid-transformed-snapshot/1`；[transform.rs 的 `Converted`／`ConvertedDatabase`／`ConvertedTable`](../../ops/recovery/src/uuid_rekey/transform.rs) 声明相同批次与冻结点、`source_manifest_sha256`、内嵌 `source` 元数据、映射文件身份与摘要、各表 `policy`／`source_rows`／`rows` 以及转换数据和 proof 的文件身份与摘要 |
| 类型化映射 | [mapping.rs 的 `WireMap`／`WireEntry`](../../ops/recovery/src/uuid_rekey/mapping.rs) 是单个 JSON 对象，顶层 `schemaVersion=1`、`frozenMs`、`entries`；不是 NDJSON，也不使用 snapshot 的 `format` 字段 |
| 认证文件摘要 | [codec.rs 的 `FileDigest`](../../ops/recovery/src/uuid_rekey/codec.rs) 为 `{sha256, plaintext_bytes, frames}`，绑定认证后的完整明文；manifest 中每个逻辑文件另有相对 `file` 和 `purpose`，不是用密文 SHA-256 替代明文对账 |
| 停写与运行证据 | source 的 `quiescence.enc`、其精确摘要和 `saas-uuid-quiescence/1` 内容，以及单独经 `uuid-seal-evidence` 封存的旧二进制／配置／库外状态材料；source manifest 不自动证明库外归档内容完整 |

源目录最终写入 `manifest.enc`；每表文件名为 `kind.schema.table.ndjson.enc`，解密后每行是 [export.rs 的 `RowEnvelope`](../../ops/recovery/src/uuid_rekey/export.rs)：`{"row":{列名:值,...},"createdAtMs":整数或null}`。`row` 是完整原始行，`createdAtMs` 是供发号使用的创建时间证据，不额外写入业务表。源目录包含被排除凭据与 metadata 的原始行，作为私有加密证据保存。

转换目录也以最后写入的 `manifest.enc` 为提交标记，另含 `typed-map.enc`、每表 `kind.schema.table.ndjson.enc` 和 `kind.schema.table.proof.enc`。转换表解密后每行直接是 `{列名:值,...}`，没有 `RowEnvelope` 外壳。`CREDENTIAL_EXCLUDED` 和 `REBUILD_METADATA` 策略的目标数据为零行，但 proof 仍逐原始行记录排除策略；其他表必须符合登记的保留／转换 owner 合同。

proof 是独立的按表 NDJSON，每行包含 `index`、`policy`、`source_sha256`、`target_sha256` 和 `business_proof`；排除行的目标摘要与业务投影为 `null`。[reconcile.rs 的 `ProjectionProof`](../../ops/recovery/src/uuid_rekey/reconcile.rs) 在验证完整目标等于登记 owner 对原始行的确定转换之后，记录源／目标和未变业务列的语义 JSON 摘要及保留／转换列数。这些行级摘要不冒充原始 JSON 文件字节摘要；完整原文件的明文摘要由各自 `FileDigest` 绑定。自由文本、金额、业务编号或内层已有 hash 不能仅因字段名包含 `id`／`hash` 而被省略。

`source_manifest_sha256` 是 source manifest 的认证明文 SHA-256。转换 manifest 内嵌源元数据，但不复制原始 source 表文件或停写文件；因此必须一起保留完整 source 目录、完整 converted 目录、映射、全部 proof 和冻结运行材料，并维护其批次关联。只有 converted 目录或一个 `typed-map.enc` 不构成完整恢复证据。密钥独立保管，不能把秘密配置或原始数据补写进公共摘要。

完整交接前使用 [verify.rs 的 `uuid-verify-package`](../../ops/recovery/src/uuid_rekey/verify.rs) 回读认证源与转换目录的全部登记文件，逐个比较 `FileDigest`，并核验 source manifest 摘要绑定、源身份元数据、表／策略／文件覆盖和 map 来源闭包。指定多个 converted 目录时，manifest、map、每表 target／proof 的明文摘要／字节／帧数及 authority counts 必须一致。manifest、停写证据、映射和来源行解析使用私有 tmpfs，其余 payload 回读到 sink；随机密文字节不同不影响比较。该操作不连接数据库，不代替后续导入的逐行 owner／proof 复核。

两类表文件均使用 UTF-8 NDJSON，保留 `null`、数组顺序、JSON 类型、业务时间和十进制／大整数的数据库表示，不经 JavaScript 不安全浮点数转换。源表按实际有序主键稳定排序，当前使用 `FROM ONLY` 对已登记关系导出；若出现新的分区、类型或关系布局，须先更新审查合同，不据此宣称可导出任意 schema。生成列由目标数据库计算；SQL migration 记录与存储 schema 合同作为校验材料，不作为业务行重写或伪造导入。

每库 `sequences` 显式保存 `{schema,name,last_value,is_called}`，其集合须与源 catalog 的序列定义一致，且导出前后状态必须相同。`last_value` 是有符号整数，`is_called` 原值必须保留；序列可能领先于表中最大值，不能用 `max(id)` 重建高水位，也不能因当前序列尚未调用就改成已调用。目标 owner、identity 导入方式及保留这些精确状态的 `setval`／等效恢复与核验属于 U04 导入门禁；离线导出保存状态不等于已在目标恢复。

映射条目为 `{authority,oldUuid,newUuid,sourceCreatedAtMs,timestampMs,timestampReason,provenance}`，唯一键是 `(authority,oldUuid)`。同一全局账号／租户／组织跨库复用同一映射，不同实体偶然相同的旧值不得合并。`provenance` 精确区分 `sourceEntity` 与 `historicalTombstone`；墓碑另外保留闭合的 reason 和 `evidenceSha256`，不能满足普通活业务引用。UUID 列、domain、数组、多态引用及结构化 JSON／TEXT／配置路径都须有已审查 owner；当前源未登记的类型或路径不能靠泛化替换通过。自由文本不做正则替换；未知疑似活动引用需在私有审查中解决，变换器停止而不是猜测。

## 时间与重跑规则

实体有可信 `created_at` 时使用其毫秒值构造标准 UUIDv7；时间在 Unix epoch 之前、晚于冻结时间或没有可靠创建时间时，使用 source／converted manifest 的固定 `frozen_ms`，映射 JSON 中同一字段拼为 `frozenMs`。`timestampReason` 明确记录 `trusted_created_at`、`missing_created_at`、`negative_created_at`、`created_at_after_freeze` 或 `preserved_v7`；已有合法且不晚于冻结点的 v7 保留原值。原业务时间列不修改。库的标准 v7 API 和 CSPRNG 负责随机位；映射一旦生成便成为不可变包内容，同包重跑使用 `resume_mapping` 指向原加密映射并写入新输出目录，不重新随机发号。恢复映射须与从原 source 重新收集的完整 authority／旧 ID／来源／时间证据／墓碑证据及策略精确相同，仅比较数量不能通过。重新加密后的随机密文可以不同，应比较映射和转换数据的明文摘要。不得用 UUID 代替严格事件顺序、计数器或版本号。

## U04～U06 目标交接与切流门禁

U03 的 `uuid-export`／`uuid-transform`／`uuid-verify-package`／`uuid-seal-evidence` 只生成和核验离线材料，不创建目标数据库、不执行导入、不轮换凭据、不推进认证 epoch／任务 fence，也不执行恢复或切流。后续交接 manifest／认证证据必须在上述 source 和 converted 合同之外补充以下信息；不能将这些要求当成当前 `Snapshot`／`Converted` 已有字段或已实现行为。

| 后续证据 | U04～U06 必须补充并核验的内容 |
|---|---|
| 实例和目标 owner | 私有配置／冻结材料绑定的源连接目标，明确的新目标 kind、地址、端口、库名、owner、migration writer 及导入责任；U03 source manifest 的 `kind`／`name` 不替代这些字段 |
| 目标 schema 与导入 | 原 source 指纹、完整历史和追加迁移版本／checksum、新目标 catalog 指纹、identity／生成列策略、精确序列恢复结果、全部目标约束和触发器状态 |
| 目标业务验收 | 与 source／converted／map／proof 批次绑定的逐表计数、业务字段／金额／编号／计数器／序列对账、跨库软引用及新业务写入结果 |
| 凭据失效与恢复栅栏 | 不随旧快照倒退的认证 epoch 和任务 fence 的权威位置／版本／生效证据，旧 access／refresh／session／code／device／nonce／lease／receipt 拒绝证明，客户端和 agent 凭据轮换清单 |
| 活动产物与库外效果 | 需重新生成／重签的 artifact、SDK 和配置 owner、外部文件／路由／运行版本索引、历史原字节归档关联、安装／通知／任务效果对账与冻结证明 |
| 切流和恢复 | 最终共同停写点、最终同步包身份、目标验收、切换记录及产生新写入后的受控恢复证据，不用仅有离线转换报告宣布切流成功 |

目标初始化分为互斥模式：`clean-seed` 用于空白环境验证，`import-data` 用于本次保数据重建。正式导入先运行全部真实历史迁移及新增迁移，再导入转换业务数据；不得运行 seed 后用冲突忽略掩盖实体碰撞，不修改 `_sqlx_migrations` checksum，不刷新存储 fingerprint 伪装 schema 相同。

## 校验与恢复门禁

导入前验证完整包、源 schema、注册表覆盖、全部映射的 v7／RFC variant 和类型化引用闭合；导入后按表对账行数、业务摘要、计数器／序列、约束与触发器、UUID 版本、跨库软引用、配置及活动产物引用。append-only 表的导入只能采用已审查的受控新目标流程，最终触发器和权威 schema 合同须恢复并匹配；未知约束失败保留隔离目标，不跳过。

所有旧 access／refresh／session／code／device／派生凭据必须验证拒绝；认证 epoch 与任务 fence 保存在不会被旧备份倒退的位置。恢复、切流前都再次推进这些边界，轮换并隔离旧 agent nonce／lease／receipt，避免快照复活已消费资格。旧库的临时只读不是凭据失效措施。

遇到非空任务效果、旧签名 receipt 或 validation，必须先由 owner 对账并建立可验证的交接合同。当前离线转换器对缺少该合同的非空效果失败关闭；不能静默清空历史证据、改写 nonce／签名或把状态改为 READY，再用目标行数或重算摘要宣称成功。

新目标产生写入后，恢复必须导出并验证增量／反向映射与库外效果，再恢复到受控副本；不能直接切回过期旧库。恢复演练必须包含新业务写入，并证明记录、序列和业务状态未丢失，同时旧 JWT 和任务资格仍失效。实际停写、切换及恢复耗时记录为本地观测值，不宣称生产 SLO。

导出、完整 catalog、映射和配置含敏感身份关联：明文仅放私有 tmpfs；离开暂存区须认证加密并校验，密钥独立保管。代码库只提交不含秘密和原始主体值的摘要、合同和工具，禁止将原始数据或映射写入验收日志。
