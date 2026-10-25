# UUID-U03：离线保数据导出与类型化转换

阶段：`COMPLETED`。执行日期：2026-10-24。范围为用户指定的本地预览三库；远程开发库和生产库未连接。已完成真实冻结、源导出、两次同映射离线转换与完整认证回读。脱敏机器证据见 [uuid-u03-summary.json](uuid-u03-summary.json)。本阶段没有创建目标库、导入或切流；下一阶段为 UUID-U04。

## 实现与合同

- `backend/ops/recovery` 增加 `uuid-export`、`uuid-transform`、`uuid-verify-package`、`uuid-seal-evidence`。入口保持轻薄，文件加密、来源映射、行改写、结构化 owner、对账和认证回读分别为私有模块。
- `uuid-references.json` 的 U01 原始字节保持不变；新 `uuid-references-v2.json` 固定为 `da5eeacbf56af6a577d9b7cb4d7e2d2f10482bee79a3efb5254a10a88dd5c313`，扩展 49 个结构化处理条目。导出与转换要求编译时已审查的精确 v2 字节。
- 源库仍使用原 schema。三库名称、U01 完整 `schema_material` 指纹、53 表／147 UUID 列／67 有序 FK 都参与门禁，未知表、列、domain、数组或引用失败。
- 导出每库保持同一个只读 repeatable-read 事务，用有界 stdin/stdout 协议和有界分页接收稳定主键顺序的行。前后严格复核 catalog、行数、snapshot、序列集合及真实 `last_value`／`is_called`，不从 `MAX(id)` 猜测高水位。`lsn`／`final_lsn` 保留为合法且不倒退的集群 WAL 观察值，不要求相等。
- 每个源 RR 建立前及完全关闭后，从维护库 `postgres` 的独立 `READ COMMITTED READ ONLY` 观察事务读取准入；三库六份 `saas-uuid-source-admission/1` 证明均校验通过并完整相等。检查固定两旧角色非 superuser／NOLOGIN、受控 postgres 身份与源 owner、其他 LOGIN 的 CONNECT（含 PUBLIC／继承）、superuser、CREATEROLE、`privileged_set`／`can_admin_source_roles` 旁路，及源默认只读、backend／prepared 为零。此合同依赖可信维护 postgres 期间不另开源写会话，不承诺抵抗恶意超级用户。
- `SAASRKF1` 为流式逐帧认证文件协议：随机 salt、逐帧 HKDF-SHA256 派生 AES-256-GCM 密钥、随机 nonce、绑定 purpose／序号／长度／末帧，footer 认证总摘要、字节和帧数。支持超过旧 pilot 512 MiB 总包限制的文件。明文仅私有 tmpfs，输出新建且精确 0600；拒绝 symlink、错误 owner、截断、重排、拼接、尾随数据。证据封存后立即读取密文到 sink 再认证，不增加明文副本。
- 全部普通权威主键分配完成后，才从闭合 owner 审计收集历史删除／凭据证明。引用不会创建实体。人工地区来源要求 CREATE 审计与真实源地区行共同证明；无活源时必须具备明确删除证明。
- 重跑先从源快照独立构建来源集合，再与恢复映射精确比对 key、provenance、证据和时间策略，随后复用原目标 UUID。多余实体、伪墓碑、同数换 key 和伪造时间均拒绝。
- 每行先按列合同改写，再由结构化 owner 处理登记的 JSON／文本／路由与派生 hash，最后从原行重放 owner 求出唯一合法目标并严格比较。金额、bigint、日期、密码 hash、业务编号、OAuth 协议 clientId、历史摘要和自由文本保留各自语义。JSON 启用任意精度且逐层拒绝重复 key。
- 凭据及真实迁移元数据完整保存在受限源证据，目标对应表输出零行；迁移元数据由 U04 的真实目标迁移重建。非空任务、旧验证结果或回执缺乏外部效果对账时失败关闭，不改为 READY、不改签名字符串或假造 nonce。
- 源包、转换包、加密 typed map 和逐行 proof 需要一起保管。转换包的完成 manifest 最后生成；失败输出保持隔离，不覆盖既有目录。
- `uuid-verify-package` 不连接数据库，按 manifest 逐文件认证回读并比较完整 `FileDigest`，拒绝额外文件、错误 purpose／policy／覆盖范围和源摘要绑定；从原 source 重新收集主键及历史证据，校验 map 来源闭包。对多个 converted 比较 manifest、map、53 张目标表与 53 份 proof 的全部明文摘要／字节／帧数和 authority counts。

## 工具验证与执行版本

- Windows 整包测试首轮 98/98 通过，包含精确 16 MiB 行边界、非法 envelope、缺字段、嵌套重复 key、任意精度数字、流式行数与回调停止、authority／retention、普通 UUID 与多维数组、结构化审计／任务／路由／hash、错误合法 v7 替换、历史证据保留和恢复映射来源闭包。
- Linux 新增文件权限回归通过，实际覆盖外 UID 65534、setuid 位、错误权限、inode 替换保护和失败清理；独立子进程 `umask 0777` 不影响并发测试，seal/open 输出仍精确 0600。
- Linux `verify_file` 回归通过：完整文件认证摘要与 seal 一致，错误 purpose 和坏 footer 拒绝。
- Linux exporter 包含 fake native 进程流式输出、非零退出、无输出超时杀进程、源 fence 和序列高水位／is_called 保留、状态漂移拒绝。这些 fixture 没有连接真实数据库。
- 新增完整包验证回归覆盖源／内嵌源摘要绑定、map 文件身份、表／proof purpose 与覆盖、排除策略、未登记文件／子目录、精确源行 framing／计数、任意精度值，以及 Linux 真文件 sink 回读的 digest 不符和损坏密文拒绝。
- 最终 U03 源码 Windows 整包 **111/111** 通过，`cargo fmt --check`、`cargo clippy --all-targets --locked --offline -- -D warnings` 通过。
- 最终 U03 源码 Linux `cargo test --locked --offline --all-targets` **117/117** 通过；`cargo clippy --locked --offline --all-targets -- -D warnings`、`cargo build --release --locked --offline` 均 exit 0。测试容器为 `saas-ci:phase04-tools`、`--network none`，不连接源数据库。早期 98／99／104 项迭代结果不作为最终门禁数量。

实际执行的三个 Linux 二进制分别冻结，不能将最终验证器版本冒认为早先导出或转换所用版本：

| 操作 | SHA-256 |
|---|---|
| 成功源导出 source-04 | `59af85ca10dce5c0e35777754c4a56c2f11f9d7f6a43d7ef613e0723a741ea55` |
| 成功转换 A／B | `eb48aadb8ce9436ca9ff2398ad4d72de74a70a5a4b69345d98ac34d3efa9f0e6` |
| 完整包认证回读 | `66319b10daa37016eec6a8fe95e1ab9ac3ffed539676395a2e201a6ab2bd6e35` |

## 真实环境执行记录

已只读核对六个实际预览容器及三源库。两组专用旧角色均为非超级用户；它们是全部具备源库 CONNECT 权限的非超级用户登录角色，prepared transaction 为零。默认预检只写私有 tmpfs 报告，没有修改服务或数据库。

停写脚本会冻结实际 `/proc/PID/exe` 的运行 inode、配置、挂载材料与镜像身份，按固定完整容器 ID 禁用 restart 并停止服务，再对两组旧角色执行 NOLOGIN、排空源会话并设置数据库默认只读。此围栏限制应用写入；不声称能够阻止拥有本机 postgres 超级用户权限的操作者主动改写。

本轮恢复范围为同一台本地 WSL 主机。已有不可变镜像按 Image ID 保留，冻结容器保持对镜像的引用；本流程不承诺本机镜像全部丢失后的跨主机冷恢复。工单 Node 进程是 Rust 启动 wrapper，其文件修改时间早于实际 Node 启动时间；真正 Rust 子进程也单独冻结运行 inode。

实际冻结执行中，六个 stop 命令均成功，但通知容器原配置为 `AutoRemove=true`，退出后被 Docker 删除，旧批量 inspect 在复核时失败。原运行二进制、进程和配置已在 stop 前归档；其余五个容器保持停止。显式恢复冻结流程随后核对原完整 ID、同名容器缺失和原进程消失，另记自动删除证明，并对其余五个原容器及数据库围栏重验。原失败结果保留，不冒认为首轮全成功。

随后已核实并设置两组旧角色 NOLOGIN、三源库默认只读；源 backend 为 0，prepared transaction 为 0。没有重启旧服务，没有改写源业务数据。32 字节密钥已复制到归档目录之外的两处持久私有托管位置，按执行用户分别 0700 目录／0600 文件，逐字节核对一致。

成功冻结归档密文为 **732,014,692 字节**，SHA-256 为 `5d5a966ae1837488431acecee8ca65a332849bbfe13c8b29cd574749aa62d602`，完整认证回读通过。该真实文件已超过旧 pilot 512 MiB 总包容量，使用独立流式协议封存。密钥与归档分离持久托管，原运行材料及映射继续受限保存。

失败轮次均保留为失败证据，后续修复使用新目录：

| 轮次 | 原因与处理 |
|---|---|
| 导出前预检 | `/var/run` 符号链接被路径门禁拒绝；改用 canonical `/run/postgresql`，未放宽 symlink 检查 |
| source-01 | 最终 fence 漂移，未生成完成 manifest |
| source-02 | 集群 WAL 变化触发旧相等门禁；后续保留初末 LSN，并加入独立新鲜准入证明，继续严格核验源 snapshot／schema／表计数／序列 |
| source-03 | PostgreSQL OID 的 JSON 字符串不符合数值 DTO；改为 SQL 显式 bigint 投影，未放宽 DTO 类型 |
| converted-04 | `menus.path` 的配置路由参数 `:id` 被当成具体实体 UUID；精确菜单配置 owner 修正并增加回归，具体业务链接仍严格校验 |

## 真实导出、两次转换与认证回读

成功 source-04 导出 **3 库／53 表／666,750 行**。A、B 都转换为 **666,610 行**，按合同排除 **140 行**；其中凭据 119 行、待真实迁移重建的 metadata 21 行。562 个映射覆盖 19 类 authority，包含业务实体、OAuth token／jti 与 request 等不同用途；映射条目数不等于账号数量。完整逐 authority 计数及全部 53 张表的源／目标／proof 摘要见机器证据。

| 操作 | 退出码 | 秒 | 最大 RSS（KiB） |
|---|---:|---:|---:|
| source-04 导出 | 0 | 52.353 | 21,752 |
| 转换 A | 0 | 128.215 | 17,936 |
| 使用 A 的 map 转换 B | 0 | 128.644 | 18,580 |
| source＋A＋B 完整认证回读 | 0 | 35.329 | 35,296 |

这些是本机预览观测值，不是生产 SLO。失败 source-02 的性能不并入成功 source-04 数据。

| 认证明文 | 字节 | 帧 | SHA-256 |
|---|---:|---:|---|
| source manifest | 1,256,274 | 2 | `1a45c72fce5a0bc2073e1e996692b12c2b9c17cd2e68756c34adbe32e501e4f2` |
| A／B converted manifest | 1,291,373 | 2 | `29aa57140250a9ab1af2caa2c0374637ed517437f108bec4fd30532d41c753e6` |
| A／B typed map | 152,307 | 1 | `5d4913b49e2ddf2ca1798011023d775b7953b613bc196b86bd1ac263842bfc22` |

A／B 原命令摘要逐项精确相同。完整回读再确认两份 manifest、map、全部目标表和 proof 的明文摘要／字节数／帧数及 authority counts 相同；随机加密字节不作相等要求。实际认证 **271 文件／1,628,018,517 明文字节／1,699 帧**，包括 source 55 文件和每份 converted 各 108 文件；各份 converted 为 649,778,540 明文字节、676 帧。回读后均与 manifest 的完整 `FileDigest` 相同，没有仅用 seal 时摘要替代读回。

大表 `regions` 保留 **665,923 → 665,923 行**。源表明文 326,807,506 字节／312 帧，目标 302,834,278 字节／289 帧，proof 344,837,004 字节／329 帧；源含 `RowEnvelope`、目标为裸行，字节数不同符合合同。目标 SHA 为 `8d1a0b78182869ab79664ae298ace0c8039daee78202a8ccb1d19074236d4c41`，proof SHA 为 `dcfff1d33ba0a5b333c31b9230451c225d27df9a6343a63c7f3e8a91340e2913`。单独地区表没有超过 512 MiB；真实超限冻结归档、converted 总量与 513 MiB codec fixture 分开记录。

| 凭据表 | 源行 | 目标包行 |
|---|---:|---:|
| oauth_authorization_codes | 0 | 0 |
| oauth_authorization_requests | 0 | 0 |
| oauth_device_codes | 0 | 0 |
| oauth_tokens | 105 | 0 |
| platform_session_refresh_tokens | 7 | 0 |
| platform_sessions | 7 | 0 |
| 合计 | 119 | 0 |

metadata 另排除平台 `_sqlx_migrations` 11 行、门店 `_sqlx_migrations` 2 行和两代 schema contract 各 1 行、工单两种 migration 表各 3 行，共 21 行；原行保留在 source 证据，不作为目标迁移记录导入。上述零行是转换包状态，不宣称已在目标验证旧凭据拒绝；实际凭据失效和新登录验收仍在 U04～U06。

平台源数据 666,735 行、门店 9 行、工单 6 行分别完整处理。门店的真实 store／daily report／audit 行复用平台 tenant／organization／user 映射并通过 owner 投影；工单当前只有 6 行 metadata，目标业务数据为零，不能称其非空业务引用已实测。UUID 数组／domain 与极值金额等边界由 fixture 覆盖，未冒认为当前源存在这些数据。非空任务效果缺少对账时的失败关闭由负例验证，当前任务效果表为空。

## 最终源留存检查

| 源 kind | UUID 列 | 非空 UUID 单元格 | v4 | v7 | 默认只读 | 其他 backend／prepared |
|---|---:|---:|---:|---:|---|---|
| platform | 107 | 2706 | 2706 | 0 | true | 0／0 |
| stores | 23 | 28 | 28 | 0 | true | 0／0 |
| work_orders | 17 | 0 | 0 | 0 | true | 0／0 |

最终只读复查确认 147 列的非空单元格与版本计数仍与 U01 一致，非空 UUID 均为 v4；源库未执行原地重编号或新迁移。六个写入者保持停止，旧角色 NOLOGIN；原库、冻结材料、source、A／B converted、map 和 proof 均留存。本阶段没有创建目标库、执行导入或切流。

## 当前验收

| 验收项 | 结果 | 证据 |
|---|---|---|
| 类型化生成、引用与结构化 owner 的失败边界 | PASS | 单元／协议／文件 fixture |
| 同一真实迁移包重跑映射与行摘要相同 | PASS | A／B manifest、562 映射、53 target／proof 和 authority counts 精确相同；271 文件认证回读 |
| 三库真实数据引用闭包、行数与业务不变量 | PASS | 666,750 → 666,610 行，排除 140 行有逐表策略；保留行的 owner 投影及 map 来源闭包通过 |
| 源凭据不作为目标活动登录状态恢复 | PASS | 六凭据表共 119 → 0 行，原证据留存；目标运行拒绝旧凭据留后续阶段 |
| 源库只读、可恢复的旧材料与长期映射 | PASS | 原源 v4 计数不变、三库只读／backend0／prepared0；冻结归档回读和独立持久密钥托管 |

UUID-U03 四项阶段验收均已满足，指针进入 UUID-U04；目标新库真实迁移、导入、schema／约束／触发器与恢复验收由下一阶段完成。
