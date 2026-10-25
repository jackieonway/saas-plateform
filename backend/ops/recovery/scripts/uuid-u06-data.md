# U06 本地 v7 数据快照与差量协议

`uuid-u06-data.py` 用于已授权本地 U06 窗口。它不创建数据库、不执行迁移、不变更角色或服务、不发布通知，也不建立重新编号映射。当前 U05/U04 的三个 v7 库是新鲜业务源；原 v4 库继续只读留存。生产 profile 只接受脚本常量中的 source/final/recovery 九个精确库名及对应 owner。

工具通过本机 PostgreSQL socket，以 `runuser postgres` 启动固定 `psql`，无新增 Python 依赖。需 Linux root、PostgreSQL 17+、`findmnt`、私有 tmpfs 和受保护的 U06 recovery codec。实际本地演练为 PostgreSQL 18.6；不能把该结果记作独立 PG17 验证。

## 操作边界

主控负责登记实际 cluster/system identifier、数据库 OID、owner OID、本代原始 schema 指纹与严格语义等价指纹；停止 API、CLI、worker、agent、通知和其它写入方；排空连接与 prepared transaction；设置默认只读、runtime/notification/owner NOLOGIN，移除其它 LOGIN 的 CONNECT 或提权入口。工具只验证这些围栏，不替主控执行它们。

每个快照库使用独立 `REPEATABLE READ READ ONLY`，并在该会话建立前与关闭后，从维护库的新 `READ COMMITTED READ ONLY` 会话检查准入。六份证明必须与共同初末证明完全一致。快照中保留维护身份、role/connect 目录和顺序快照边界。顺序三库事务依赖共同停写；它不是 PostgreSQL 的跨库原子快照，也不抵抗受信任超级用户故意写入。

主控在收到全部库成功报告、完整复核及新 auth/task epoch 验证前不得开放任一目标。三库分别提交；失败时不自动换回旧库。`setval` 不随事务回滚，失败副本也必须继续隔离。

## 固定数据合同

生产模式严格覆盖平台 40、门店 8、工单 6，共 54 张表。新增、遗漏、重复表或 schema 变化均拒绝。原始 schema 使用三服务已有 `schema_material.sql` 合同，其 UTF-8 源码摘要固定为 `204b5d744b099f2eda02fa363f5c54d3379da75753ae4673474017e6fb51ade2`。

每代原始 catalog 单独 pin。完整语义 catalog 仅对 `internal=true`、名称匹配 `RI_ConstraintTrigger_[ac]_<数字>` 的内部外键触发器，将 `name` 和 `definition` 内唯一同名替换为固定 OID 标记，然后将全部 catalog item 按完整 canonical JSON 排序。其它列、类型、约束、索引、默认值、触发器、函数正文、策略和序列定义保持。未知内部触发器拒绝。相同 kind 的三代语义指纹必须相同。

迁移历史由真实迁移创建。工具验证完整连续版本、描述、SHA-384、success、installed_on 与 execution_time 合法性，固定链为 12/3/4；工单旧迁移表还验证四个精确文件名、SHA-256 和 applied_at。门店三个 schema contract 校验实际当前 owner 函数与历史指纹链。工具不写入、补齐或复制迁移元数据。

每行的外层 envelope 是 PostgreSQL 生成的 JSON：

```json
{"key":["主键的原生text"],"row_text":"未经Python解析的row_to_json(t)::text","json_texts":{"JSON列名":"该列的原生text或SQL NULL"}}
```

业务 `row_text` 仅作为字符串保存、比较和传回 PostgreSQL，不经过 Python 数字解析或 JSON 对象归一化。因此精确 numeric、大整数、重复 JSON key、JSON 原文、bytea、日期和时间由数据库自身类型处理。`json_texts` 保留每个 JSON/JSONB 列的原生 text（SQL NULL 对应 null），因此 SQL NULL、JSON null、JSON 根值前后空白和重复键都能精确重建。会话固定 UTC、ISO YMD、postgres interval、hex bytea 和 extra_float_digits=3。行按原生主键顺序输出；每表记录行数、字节数和 SHA-256。

以下六表随源证据认证封存，但不恢复、不参与差量重放，目标必须为空：`oauth_authorization_codes`、`oauth_authorization_requests`、`oauth_device_codes`、`oauth_tokens`、`platform_session_refresh_tokens`、`platform_sessions`。迁移 metadata 同样封存但由目标真实迁移重建。其它所有表保持原 v7 ID 和完整原始业务行，包括发布状态、回执、创建幂等记录、通知、提醒 intent、审计和计数器。

## 私有配置

所有配置、registration、密钥和报告为执行 root 所有、mode 0600、单硬链接普通文件；目录 mode 0700，禁止符号链接。明文只在现存私有 tmpfs 中创建，退出时删除；密文与原始 32-byte 密钥分开保管。stdout 只含操作、计数、耗时和通过状态；原生数据库错误不输出业务行。

每次操作一个新配置文件：

```json
{
  "format":"saas-uuid-u06-data/1",
  "profile":"uuid-local-u06",
  "registration":"/absolute/private/registration.json",
  "scratch":"/dev/shm/saas-uuid-u06-20261024",
  "codec":{
    "binary":"/usr/local/libexec/saas-recovery-u06-codec",
    "sha256":"主控核验的受保护可执行文件SHA256",
    "key_file":"/var/lib/saas-uuid-u06-keys-20261024/snapshot.key"
  },
  "group":"source",
  "output":{
    "path":"/var/lib/saas-uuid-u06-20261024/批次/snapshot.enc",
    "purpose":"saas-uuid-preview-freeze/1/批次/u06-snapshot"
  },
  "report":"/absolute/private/new-snapshot-report.json"
}
```

`registration` 必须包含固定 `format=saas-uuid-u06-registration/1`、实际 `system_identifier`、`groups` 和 `fenced_roles`。三个 group 名为 source/final/recovery，每组按 platform/stores/work_orders 顺序列出 `{kind,name,oid,owner,owner_oid,expected_schema_sha256,semantic_schema_sha256}`。`fenced_roles[group]` 列出主控登记的 runtime/notification 角色；所有 owner 也必须 NOLOGIN。所有库 OID 必须互不相同。registration 的原始文件摘要绑定每个认证包，后续配置使用同一冻结文件。

生产模式每组的四个 runtime 名称也由代码固定：source 为既有 U05 四角色，final/recovery 为对应 `qiankun_{platform,stores,orders,notification}_uuid7_u06_{final,recovery}_runtime_20261024`。source 角色和全部 owner 必须已存在；final/recovery 的四个固定未来角色可在 epoch helper 自身 CREATE 前暂缺，名字始终参与 SET/admin 旁路谓词，存在时即必须 NOLOGIN/non-superuser。不得用空名单代替未来围栏。实际 runtime OID 和创建、轮换事实另由主控 epoch journal 钉住。

恢复、重放、验证的 `input`，以及差量的 `before` 和 `after`，都是三字段包引用：`{path,purpose,sha256}`。`sha256` 是认证打开后 tar 明文摘要，不是随机密文摘要；从前一步私有报告的 `artifact` 中取这三项。snapshot/delta 的 output 是 `{path,purpose}`。所有操作必须使用新的 report 路径。

codec 的 argv 固定为 `binary uuid-seal-evidence private.json` 或 `binary uuid-open-evidence private.json`，配置不接受任意命令。binary 必须 root 所有、不可被组或其它用户写入且 SHA-256 精确匹配。适配器只在 scratch/批次 下暂存单个 tar；purpose、密文位置和 batch 由现有 codec 再次约束。打开时必须提供明文 expected_sha256，并验证完整认证结果。

## 五个入口

```sh
python3 uuid-u06-data.py --snapshot /private/snapshot.json
python3 uuid-u06-data.py --restore /private/restore.json
python3 uuid-u06-data.py --delta /private/delta.json
python3 uuid-u06-data.py --replay /private/replay.json
python3 uuid-u06-data.py --verify /private/verify.json
```

- snapshot：group 指定停写源；输出认证封存的 54 表、catalog、历史投影、序列、快照与全部 fresh 围栏证据。
- restore：input 指向 snapshot，group 仅可为 final/recovery，指向另一个已真实迁移、无业务数据的新组。平台迁移 0001 自带的 `platform_settings` singleton 是唯一受审例外：必须每个字段等于已知 DDL 默认值且 updated_at 等于该真实迁移的 installed_on，才可事务内定点删除并用源原行替换；其它非空业务数据均拒绝。按外键拓扑装载 staging，使用 `json_populate_record` 原生类型转换和显式 INSERT；排除 generated 列，identity 用 `OVERRIDING SYSTEM VALUE`。全部业务触发器和外键保持启用。每库提交前重新读取全部保留行与序列并精确匹配源文件摘要。
- delta：before/after 指向同一 final 三库的 T0/T1 快照，实际 OID/owner/cluster 身份必须完全相同，capture 时间递增、序列不倒退。所有保留表必须闭合；摘要相同的大表直接跳过。变化表按完整主键比较未经解码的原行和 JSON 原文投影，生成 before/after-image、操作和计数。变化表暂时载入内存；超大变化表应在执行前评估内存，本工具不声称支持无限数据量。
- replay：input 指向 delta，group 指向隔离恢复组。再次认证 delta 及其 T0/T1 边界包；先全量确认目标仍精确等于 T0，再对每个主键锁行核对 before-image，按反外键顺序删除、正外键顺序插入/更新。禁止业务 TRUNCATE、关闭 trigger、盲目 upsert 和修改 append-only 历史。identity 更新须前后相同并从 SET 排除。最终全部保留行和序列必须精确等于 T1 后才逐库提交。
- verify：input 指向预期 snapshot，group 指向隔离目标，以新只读会话重新全量读取比较。验证不开放目标，也不证明 JWT/task epoch 或库外效果已完成。

任何 changed-table 内跨表依赖变更若无法按固定顺序满足现有即时 FK/业务触发器，会原生失败并维持隔离；不通过关闭约束绕过。当前受控演练只使用不会触发该边界的正式 API 操作。

门店两张受审表 `stores`、`store_daily_reports` 的原生 UPDATE guard 要求 version 恰好加一。当前差量合同只接受版本差 1 或 2；差 2 时，仅在完整 schema semantic 指纹等于已冻结门店合同的前提下，事务内先执行一条仅 `version=version+1` 的中间 UPDATE，再应用完整 T1 after-image。全部触发器保持 origin，受审 guard 不生成审计或库外效果；中间步骤不单独提交，最终原始全行、审计表与 T1 必须精确相同。其它版本差拒绝，不通过自动改版本抹平未知历史。

若平台已完成 T1 提交、后续库失败，操作者可在新的 replay 私有配置中显式提供唯一允许的 `resume_after_kinds:["platform"]`。此分支先用独立 `READ ONLY` 事务验证平台全部 T1 保留行、schema、历史及序列，完全不创建 staging、不 DML、不 setval；成功后才打开后续库的写事务。未声明的库仍必须精确等于 T0。平台实际仍为 T0、非固定名单、源／final 目标以及普通重复 replay 都拒绝。该入口不自动判断或修正 partial 状态，也不重置恢复演练计时。

数据库失败的原生 stderr 留存在新 report 相邻的 root:0700 诊断目录内，文件 root:0600，失败 report 只登记静态错误码、私有诊断引用及摘要；stdout/stderr 对话输出仍仅含静态错误类别。操作者应将诊断随同本次私有恢复证据保管。

## 受控无损恢复演练

T0 停写快照先恢复至独立 recovery 三库，完成 warm baseline。final 开放后通过正式 API 产生新工单和带版本前置条件的描述/状态变化；再次停止全部 final 写入者，取得权威 T1，计算 T0→T1 差量。recovery 从 T0 应用精确差量后与 T1 全量核对，包含新 ID、时间、审计、回执、幂等键、计数器和所有用户写入，随后才由主控提升 auth/task epoch、轮换运行材料、拒绝旧凭据并恢复服务。

这里的 RPO 0 是保有完整已停写 T1 源的本地受控恢复。没有连续 WAL/PITR 或服务端事务日志时，T0 加客户端命令日志无法证明源永久丢失情况下的 RPO 0。RTO 计时由主控从实际失败/停写边界开始，包含差量、认证、核对、epoch、启动、新登录和读写探针；应记录 warm/cold 前提，不把预先准备计作故障后的耗时。

## 隔离真实 PostgreSQL fixture

`uuid-u06-data-fixture.py` 只建立 `uuid_u06_fixture_` 前缀的独立库和 NOLOGIN owner，独立 profile 不放宽生产清单。fixture 同样强制受保护的固定 codec，不接受配置 SQL 或任意可执行文件。它使用真实 PostgreSQL 原生类型、真实触发器和 FK，覆盖 numeric 精度、重复 JSON key 与根空白、SQL NULL/JSON null、bytea、微秒时间、generated/identity、序列、凭据排除、差量增改删、before-image 冲突、源库写入入口、逆向差量、序列回退与认证篡改拒绝。fixture 报告不能作为正式三库已执行的证据。
