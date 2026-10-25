# 三库协调备份与隔离恢复

UUID-U05 的离线配置入口为 `uuid-config-rekey`，只支持既有封闭 policy/grants owner，并将旧 preview state 处理为仅计数的待重建产物；完整输入、私有输出与凭据丢弃合同见 [类型化配置改号合同](uuid-config-rekey-contract.md)。该入口不连接数据库、不重新应用授权、不启动服务。

UUID-U04 的独立目标导入使用 `uuid-import`，配置、完整验证、重复拒绝与失败恢复见 [UUID 导入合同](uuid-import-contract.md) 和 [uuid-import.example.json](uuid-import.example.json)。该入口只写明确登记且已完成真实迁移的三个全新 U04 预演库。

UUID v7 前置清点增加独立只读入口 `uuid-inventory`；它不执行备份恢复协议、不修改数据库，也不建立目标库。按 [uuid-inventory.example.json](uuid-inventory.example.json) 在源码外创建 mode 0600 的私有配置，使用本地 socket 或 loopback 和三个明确命名的 preview 源库。仅支持 `uuid-local-preview`，远程和生产不在授权范围。

```sh
saas-recovery uuid-inventory /absolute/private/uuid-inventory.json
```

该命令清除隐藏 libpq 环境连接参数，强制只读、超时和新报告路径，校验实际库名与连接地址，并要求实际 UUID 列与 [类型化引用注册表](uuid-references.json) 完全一致。完整 catalog／函数定义和聚合统计只保存到 tmpfs 上的新 mode 0600 私有报告，stdout 仅输出摘要；不输出 UUID 原值、密码或 token。报告父目录须预先建立，离开 tmpfs 的留存副本须由受控备份流程认证加密。每库独立 repeatable-read 不是跨库协调快照。需要 PostgreSQL 17+ 的 `psql`、服务端 XML 支持和 Linux tmpfs；不要求升级 PostgreSQL 18。实际导出与存量重编号使用 [保数据合同](../../docs/identity-sso/uuid-export-contract.md)，不能将清点报告当作备份包。

## UUID 离线导出、转换和证据封存

`uuid-export` 将共同停写后的本地预览三库导出为认证加密的原始证据；`uuid-transform` 仅从这些文件分配类型化 UUIDv7 映射并生成转换包；`uuid-verify-package` 完整认证回读并比较同映射重跑结果；`uuid-seal-evidence` 封存操作者收集的运行材料。四个入口均不创建目标数据库、不导入数据、不控制服务生命周期，也不执行切流。实际环境的执行结果与阶段门禁记录在 [UUID 台账](../../docs/identity-sso/uuid-v7-task-ledger.md) 及其 validation 文件，以下是操作合同和配置说明。

### 运行环境与私有材料

实际文件操作要求 Linux、`findmnt` 和私有 tmpfs；导出另外要求 PostgreSQL 17+ 的 `psql`。源码可位于 Windows 工作区，实际入口须构建为 Linux 目标并在 WSL／Linux 中运行。四个命令只接受 `profile=uuid-local-preview`。当前转换器精确限定以下三个源库名；远程开发库、生产库及其他环境不属于本流程。

| kind | 本地预览源库 | U01 已审查的 `schema_material` SHA-256 |
| --- | --- | --- |
| `platform` | `qiankun_platform_preview_20261024` | `2a16ccb1b285bb8613a56b33a84df02032ad8a190c2e5b20697ab35fd4b084c2` |
| `stores` | `qiankun_stores_preview_20261024` | `cdb1c3a42077033d8692afe8b09b8dc192c41a0491d327cc4fe686624843ba08` |
| `work_orders` | `qiankun_orders_preview_20261024` | `ce5768293dd291e16320641a31f8bef97e127acf83b21416fd5718d5a11be96c` |

上述指纹来自 [U01 脱敏 catalog 摘要](../../docs/identity-sso/validation/uuid-u01-catalog-summary.json)，用于核对尚未追加 UUID 迁移的原始源 schema。不能用目标新 schema 的指纹替换它们，也不能在导出失败后直接更新摘要绕过变化审查。导出与转换使用字节精确匹配编译版本的 [v2 引用注册表](uuid-references-v2.json)；原始 [U01 注册表](uuid-references.json) 仍用于冻结审查事实。

从下方示例复制配置到源码之外，并把所有占位路径改为实际 Linux 绝对路径。配置、密钥和停写证据须属于执行用户，mode 0600；私有目录使用 mode 0700。密钥是独立保管的原始 32 字节安全随机值，不是 32 字符口令或编码后的字符串。明文材料只能在 tmpfs 内，密文目录可位于普通磁盘；密钥应与密文分开保管。拒绝符号链接和路径中的 `.`／`..`。输出父目录须预先建立，每次输出目录须是新路径。

### `uuid-export`：共同停写后的原始证据

先冻结旧版本可执行文件、实际配置、路由和需要恢复的库外材料，再停止全部写入者、排空事务，把两个已登记旧服务角色设为 `NOLOGIN`，并把三个源数据库的 `default_transaction_read_only` 设为 `on`。这是操作者执行的维护步骤；导出器只检查，不会修改源数据库或角色设置。不要在未保存旧可执行文件时假设磁盘上的同名文件仍是当前运行版本。

`quiescence_file` 是私有 JSON，字段如下；不得填入假摘要或仅凭容器名宣称已经停写。

| 字段 | 合同 |
| --- | --- |
| `format` | 固定 `saas-uuid-quiescence/1` |
| `profile` | 固定 `uuid-local-preview` |
| `writes_quiesced` | 已核实全部写入者停止后填 `true` |
| `databases` | 三个对象，每个仅含与导出配置精确一致的 `kind`、`name` |
| `stopped_writers` | 下列六个容器名各一次 |
| `captured_ms` | 实际共同停写点的 Unix 毫秒时间；执行导出时不得晚于当前时间或早于一小时 |
| `writer_evidence_sha256` | 原始停写／运行材料证据的 SHA-256，64 位小写十六进制 |

当前已登记的 `stopped_writers` 为：

```json
[
  "qiankun-platform-preview-20261024",
  "qiankun-stores-preview-20261024",
  "codex-qiankun-orders-preview-api-20261024",
  "qiankun-preview-notification-20261024",
  "qiankun-preview-examples-oauth-20261024",
  "qiankun-preview-examples-chat-20261024"
]
```

还须核对相关 worker、scheduler、agent、CLI 和库外操作。固定列表是本地预览部署的证据合同，不能自动发现所有外部写入者。导出器在每个源只读 repeatable-read 会话内先核验 catalog，再按实际主键顺序流出各表，最后严格复核 snapshot、schema、表行数、序列集合和 `last_value`／`is_called` 状态。源会话内检查默认只读、无其他源连接及无 prepared transaction；集群 WAL 的初末值分别保存在 `lsn`／`final_lsn`，作为格式合法且不倒退的观察记录，不要求相等，也不把 LSN 当作共同停写证明。

当前准入检查见 [export/admission.rs](src/uuid_rekey/export/admission.rs)。每个源 RR 建立前及该会话关闭后，均从独立维护库 `postgres` 的 `READ COMMITTED READ ONLY` 观察事务读取当前准入状态，避免用源 RR 的旧权限快照证明角色仍受控。三库共六份 `format=saas-uuid-source-admission/1` 的 `Proof` 保存于 source `Snapshot.admission_before`／`admission_after`；各数组恰有三项，位置与 `databases` 对齐，所有六份身份与准入元数据必须精确相同。

准入证明要求旧角色 `qiankun_preview_20261024` 和 `qiankun_notification_preview_20261024` 均存在、非 superuser 且 `NOLOGIN`；除受控维护身份 `postgres` 外，不得有其他可 LOGIN 且可 CONNECT 任一源库的角色。检查包含其他 superuser、通过 `PUBLIC` 或角色继承取得的 CONNECT，以及 `privileged_set`、`can_admin_source_roles` 和 LOGIN CREATEROLE 等提权旁路；同时要求源库默认只读、源 backend 为零、prepared transaction 为零。维护期间的 `postgres` 是可信受控身份，操作者不得另开源写会话；这些检查不承诺抵抗恶意超级用户主动改角色、改设置或写数据。三库的一致业务状态依赖这组准入围栏、实际共同停写和每库严格快照检查，顺序快照仍不是 PostgreSQL 跨库原子快照。

配置见 [uuid-export.example.json](uuid-export.example.json)：

| 字段 | 用途 |
| --- | --- |
| `host`、`port`、`user` | 明确的本地 Unix socket／loopback、端口和受控维护用户 `postgres`；示例使用 `/run/postgresql` 的 socket peer 认证。Ubuntu 的 `/var/run` 是符号链接，应使用解析后的实际目录，不放宽路径检查 |
| `databases` | 固定三库的 `kind`、`name` 和已审查 `expected_schema_sha256` |
| `scratch` | 已存在的 tmpfs 根目录，工具在其中新建私有临时子目录 |
| `registry` | 与工具内置字节一致的 `uuid-references-v2.json` |
| `key_file` | 私有原始 32 字节加密密钥 |
| `output_dir` | 尚不存在的独立密文导出目录 |
| `quiescence_file` | 上述私有停写证据 JSON |
| `timeout_seconds` | 每库导出会话的整体期限，10～1800 秒 |

```sh
UUID_WRITES_QUIESCED=true saas-recovery uuid-export /srv/saas/private/uuid-export.json
```

导出器清除隐藏 `PG*` 连接参数、禁用密码提示和默认 passfile，以配置中的连接目标为准。每张表保存为 `kind.schema.table.ndjson.enc`，认证 manifest 使用 `saas-uuid-source-snapshot/1`，记录源 catalog、snapshot、初末 LSN、序列状态、六份准入证明、注册表摘要、行数、每文件明文摘要／字节数／帧数及加密停写证据。原始凭据行也留在加密源证据中，供审查与撤销证明使用；它们不会因此成为目标的活动凭据。

### `uuid-transform`：类型化映射与逐行对账

配置见 [uuid-transform.example.json](uuid-transform.example.json)。`source_dir` 指向完整且未改动的源导出目录，`output_dir` 是与源分离的新目录，`scratch`、`registry`、`key_file` 与导出阶段的合同相同。首次执行 `resume_mapping` 为 `null`。转换器不连接 PostgreSQL。

```sh
saas-recovery uuid-transform /srv/saas/private/uuid-transform.json
```

转换器先认证源 manifest、停写证据和每表数据，完整分配权威主键及有证据的历史墓碑，再按登记的 UUID 列、结构化 JSON／TEXT 和路由 owner 转换引用。普通引用不能自行创建源实体，未知历史生产者、未登记路径、歧义的业务标识或无法闭合的引用均使转换失败。自由文本不做全局 UUID 正则替换。

映射以 `(authority, old_uuid)` 为唯一键，跨库同一权威实体复用目标。可信创建时间用于 UUIDv7 时间部分，缺失、负值或晚于冻结点时使用固定停写时间；原业务时间保持。已有合法且不晚于冻结点的 v7 保持原值。`CREDENTIAL_EXCLUDED` 和 `REBUILD_METADATA` 行在目标包中排除，源证据与逐行排除记录保留。任务效果、旧签名 receipt／validation 需要 owner 对账；当前入口遇到尚无可验证对账合同的非空任务效果时中止，不自动改为 READY 或改写签名证据。

转换输出包含加密表数据、每表 `proof.enc`、`typed-map.enc`，以及最后写入的 `saas-uuid-transformed-snapshot/1` 认证 manifest。逐行证据绑定源行摘要、目标行摘要、保留／排除策略和业务字段投影；验证完整目标须等于登记 owner 对原始行的确定转换，不能仅凭 UUID 格式正确、行数一致或重新计算一个 hash 通过。JSON 大整数与精确小数保留数值表示，不经 JavaScript 浮点数转换。

同包重跑须将 `resume_mapping` 指向前次的加密 `typed-map.enc`，并选择新的 `output_dir`。工具从原始源证据独立重建完整来源集合，仅在 authority／旧 ID、创建时间、时间理由、冻结策略及完整墓碑证据全部精确相同时复用旧映射；额外条目和同数量换 key 均拒绝。应比较映射及转换数据的明文 SHA-256；由于每次加密使用新的随机材料，密文字节不同是正常现象。源数据变化须重新导出和审查，不通过复用映射掩盖。

### `uuid-verify-package`：完整回读与同映射重跑比对

此入口不连接数据库。它重新认证完整 source 和 converted 目录，核对每个文件的 purpose 与 `FileDigest`，从 source 权威主键和闭合历史证据独立重建来源集合，再校验 map 的完整来源覆盖。目录不能含未登记文件、子目录或符号链接。用于解析 manifest、停写证据、映射和源行的明文仅在私有 tmpfs；其余 table／proof 认证回读直接写入 sink。

私有配置字段如下；`converted_dirs` 接受 1～8 个互不重复的完整转换目录。提供多个目录时，工具要求全部 converted manifest、map、每表 target／proof 的明文摘要、字节数、帧数及 authority counts 精确一致，不比较随机密文。

```json
{
  "profile": "uuid-local-preview",
  "source_dir": "/var/lib/saas-uuid-preview/source-export-01",
  "converted_dirs": [
    "/var/lib/saas-uuid-preview/converted-01",
    "/var/lib/saas-uuid-preview/converted-02"
  ],
  "scratch": "/dev/shm/saas-uuid-preview",
  "registry": "/srv/saas/source/backend/ops/recovery/uuid-references-v2.json",
  "key_file": "/srv/saas/private/uuid-preview.key"
}
```

```sh
saas-recovery uuid-verify-package /srv/saas/private/uuid-verify-package.json
```

报告仅含脱敏计数、表名、authority counts、明文摘要／字节数／帧数和比较结果，足以记录大地区表与两次转换的完整性。认证回读及 map 来源覆盖不替代导入时对目标行／proof 的业务投影复核，也不证明数据库迁移或切流完成。

### `uuid-seal-evidence`：冻结旧运行材料

此入口只封存已由操作者收集的一个私有 tmpfs 常规文件，可用于包含旧二进制、配置和停写检查的归档。命令不自行收集文件、停止服务或核实归档内容完整性。以下是独立私有配置的字段示例，`REPLACE_WITH_UNIQUE_BATCH_ID` 必须换成本次唯一批次标识；同一批次的不同材料使用不同逻辑文件名。

```json
{
  "profile": "uuid-local-preview",
  "input": "/dev/shm/saas-uuid-preview/runtime-freeze.tar",
  "output": "/var/lib/saas-uuid-preview/runtime-freeze.tar.enc",
  "key_file": "/srv/saas/private/uuid-preview.key",
  "purpose": "saas-uuid-preview-freeze/1/REPLACE_WITH_UNIQUE_BATCH_ID/runtime-freeze"
}
```

```sh
saas-recovery uuid-seal-evidence /srv/saas/private/uuid-seal-evidence.json
```

`purpose` 必须以 `saas-uuid-preview-freeze/1/` 开头，含批次和逻辑文件身份，长度最多 256 字节，不含控制字符；密钥不得与输入或输出为同一文件。成功报告只包含状态、明文 SHA-256、字节数和帧数。命令保留输入；操作者完成密文与证据交接后，按受控留存流程处理 tmpfs 原件。

### 流式加密、容量和失败处理

UUID 文件协议 `SAASRKF1` 以最多 1 MiB 的帧读写，每帧使用 HKDF-SHA256 派生密钥及 AES-256-GCM 认证，绑定 purpose、顺序、长度和文件终结证据。截断、重排、拼接、尾随数据或错误 purpose 均被拒绝。调用者只有在整文件认证成功后才能消费解密结果。输出以 create-new／mode 0600 创建，不覆盖已有文件；解密明文须在当前执行用户的私有 tmpfs 中。

此流式文件协议没有旧 `pilot` 认证集合的 512 MiB 总包限制，但不代表任意规模导出都能在当前机器运行。表明文仍逐表暂存；转换时需容纳正在读取的源表、目标表和行证明，tmpfs 空间须覆盖并发峰值。行上限 16 MiB，注册表上限 4 MiB；转换读取源 manifest 和恢复映射各限 64 MiB，映射与来源证据还会占用内存。配置和私有停写 JSON 使用 1 MiB 读取上限。超限、超时、空间不足或认证失败均中止，不能退回普通磁盘保存明文。

manifest 作为最后的提交标记；失败目录可能含已认证的部分密文或映射，须继续隔离，不当作可导入的完整包。修复原因后选择新输出目录，保留原始源证据。工具的普通退出会清理自身临时子目录；异常终止后的残留 tmpfs 必须由操作者按批次核查。stdout 仅用于脱敏计数和摘要，原始主体 ID、映射、token、密码哈希与完整配置不得写入仓库或公共日志。离线转换成功也不代表目标已迁移、业务已验收或可以切流。

## pilot 协调备份与恢复

`saas-recovery` 的 `pilot` 协议将共同停写后的平台、工单、门店数据库与冻结版本材料组成认证加密集合，上传远端后重新取回校验；恢复只面向新的隔离目标。`production-pitr` 未实现。该协议仍有固定试点材料约束，尚未成为任意 qiankun 应用的通用恢复系统。

## 1. 工具和私有配置

在本目录执行：

```sh
cargo test --locked
cargo build --release --locked
```

实际执行环境需要 Linux、PostgreSQL 17 的 `pg_dump`、`pg_restore`、`psql` 和 `findmnt`，数据库与 HTTPS 对象存储可达。按 [config.example.json](config.example.json) 在源码外创建私有 JSON，使用绝对路径交给 CLI。

| 字段 | 内容 |
| --- | --- |
| `profile` | 当前仅 `pilot` |
| `service_file`、`admin_service` | PostgreSQL service 连接文件及恢复管理员 service |
| `databases` | 固定 platform/work_orders/stores 三项源 service、目标库和 owner |
| `key_file` | 独立托管的原始 32 字节密钥 |
| `scratch` | 可验证的私有 tmpfs，明文 dump 不落普通磁盘 |
| `artifact_root` | 本次冻结的完整版本材料目录 |
| `local_bundle`、`remote_url` | 本地密文和远端 HTTPS 对象位置 |
| `remote_token_file`、`ca_file` | 对象存储 Bearer 和 CA |
| `monitor_url`、`monitor_token_file` | 私有监控事件入口及独立凭据 |
| `sequence_file` | 持久递增事件序号，与本次索引和锁同属受控目录 |
| `restored_artifacts` | 新恢复材料路径，不覆盖已有目录 |
| `timeout_seconds` | 当前配置的整体工作期限 |

凭据权限、所有者、符号链接、tmpfs 和锁受检查。`pilot` 认证集合上限 512 MiB、文件最多 20000，不能据此承诺大型生产库恢复目标；该总包限制不适用于上方独立的 UUID 流式文件协议。密钥和备份密文分开托管，运行密钥和 TLS 私钥不放版本材料。

## 2. 冻结真实发布材料

当前校验器强制以下布局，四个业务前端目录至少含 `index.html`：

```text
host/                          宿主完整产物与保留资源
stores/v1/                     门店试点版本一
stores/v2/                     门店试点版本二
work-orders/v1/                工单试点版本一
work-orders/v2/                工单试点版本二
images/lock.json               四份镜像的身份与归档摘要
images/stores-v1.tar
images/stores-v2.tar
images/work-orders-v1.tar
images/work-orders-v2.tar
routes/nginx.conf              实际入口配置
```

`images/lock.json` 是四项数组，每项有 `app`、`version`、`imageId`、`archiveSha256`、`archiveBytes`；app 固定工单或门店，version 固定 `v1`/`v2`。`imageId` 是 Docker 本地镜像身份，与 registry manifest digest 不同，不能互换。

这一布局是当前 pilot 校验协议，不能因前端已改 qiankun 而省略。它也不自动覆盖当前数据库实际引用的所有版本。还应把实际环境 URL 对应的完整静态材料、不可变发布目录、ZIP 内容、所需镜像、SDK、网关路由和可重建配置收集到 `artifact_root`，一并冻结。

工具校验已收集文件的路径、摘要和认证包，不逐项核对数据库发布引用、外部 URL 内容或 HTML 资源依赖。`runtime/releases`、`runtime/bin`、`runtime/native-lock.json` 不是当前必填布局。不要用补占位 HTML 或只满足固定目录结构代替完整材料验收。

相对文件名按实际路径组件保存，拒绝非 UTF-8 名称、字面反斜线、NUL、重复路径及父文件/子路径冲突。确定无效的包在创建恢复数据库或写出材料前被拒绝。

## 3. 协调备份

1. 进入维护状态，停止平台、业务、通知、发布任务及全部外部写入者，排空事务并冻结版本材料。
2. 记录共同停写点。`BACKUP_WRITES_QUIESCED=true` 是操作者的明确断言，工具不能发现所有写入者。
3. 运行新集合备份：

```sh
BACKUP_WRITES_QUIESCED=true saas-recovery backup-new /srv/saas/private/recovery.json
```

工具执行三库 custom dump，记录迁移与材料指纹，用 AES-256-GCM 认证加密。`backup-new` 为本地路径及远端对象增加唯一后缀，成功后写出 `sequence_file` 同目录的 latest 配置索引；`backup` 使用原配置位置且不覆盖已有集合。

远端存储需支持 Bearer、`PUT` 携带 `If-None-Match: *` 返回 201，以及 `GET` 返回 200。上传后重新下载、检查摘要和认证，通过后才更新最新索引。失败对象不自动删除。

4. 无论成功或失败，核对维护前写入者和活动服务，按明确流程恢复运行，记录停写时间。命令失败先查原结果，不能立即覆盖已有远端对象。

## 4. 远端校验与恢复点年龄

`verify`、`age` 都从远端取回并认证。使用原配置或实际生成的最新索引，例如当 sequence 文件为 `events.sequence` 时：

```sh
saas-recovery verify /var/lib/saas-recovery/events.latest.json
saas-recovery age /var/lib/saas-recovery/events.latest.json
```

`age` 超过 86400 秒失败，恢复点以已远端取回认证的集合为依据，不能用本地 dump 时间替代。备份周期、重试和告警阈值需给验证留下余量。

备份操作成功后，监控事件投递失败也会使命令失败。核对集合、索引、远端对象和事件状态，区分数据操作成功与告警链路失败。不要重置 sequence 绕过重放保护。

## 5. 恢复到新隔离目标

创建新的私有恢复配置：三目标名称各不相同、以 `recovery_` 开头、均不存在且不等于源库；`restored_artifacts` 也必须是新路径。恢复管理员具备受控建库和恢复权限，目标不接业务流量或旧定时任务。

```sh
saas-recovery restore /srv/saas/private/restore.json
```

工具先认证全集合并检查来源和目标，再建库、逐库恢复，比较迁移与业务 catalog。只对固定已知约束作受限兼容处理，未知表达式失败。跨库失败不保证全局回滚，失败目标继续隔离，工具不自动 DROP。

平台恢复会使旧 session、refresh、应用/普通 OAuth 凭据、授权码和设备流程失效并记录审计。成功状态仅为 `QUARANTINED`，尚未完成切流。

## 6. 隔离验收和切流

加载并核对镜像归档、网关配置和静态版本，在隔离 HTTPS 下重新登录，验证：

- 旧凭据拒绝、新登录可用，三库业务记录和迁移一致。
- 当前直接成员、组织切换、功能权限、数据范围和业务写入符合预期。
- 通知、工单提醒以及外部调用使用隔离环境配置。
- 宿主 CSP 满足 qiankun，应用加载、卸载、重进和真实 JS/CSS 正常。
- 固定两版本及数据库实际引用的其他版本、旧资源均可访问，缺资源真实 404。

全部验收后再明确切流，记录从恢复请求到业务可用的完整 RTO。尚未建立持续 WAL、基础备份、timeline 和共同恢复目标，因此不能将当前 dump 流程作为 PITR 证明。调度器和外部告警由运维单独建立，见[监控指南](../monitoring/README.md)。
