# UUID-U04：独立目标三库初始化、导入与恢复

阶段：`COMPLETED`。只覆盖本地 WSL Ubuntu 的 U04 隔离预演目标。已完成真实迁移、失败目标重建恢复、原认证包重新导入、逐行对账和独立只读复核；目标继续隔离、默认只读，尚未启动服务或切换流量。下一阶段为 UUID-U05。

## 已真实执行

2026-10-24 已通过 `scripts/prepare-uuid-preview-targets.py` 的默认只读预检，核对三源只读、两旧角色 NOLOGIN、源 backend/prepared 为零，且固定目标库名及新 owner 均不存在。随后 `--execute` 创建以下独立目标；原库未清空或删除。

| 服务 | 新目标库 | 新 NOLOGIN owner |
|---|---|---|
| 平台 | `qiankun_platform_uuid7_u04_20261024` | `qiankun_platform_uuid7_u04_owner_20261024` |
| 门店 | `qiankun_stores_uuid7_u04_20261024` | `qiankun_stores_uuid7_u04_owner_20261024` |
| 工单 | `qiankun_orders_uuid7_u04_20261024` | `qiankun_orders_uuid7_u04_owner_20261024` |

通过本机 `/run/postgresql:5432` 的 postgres peer 连接，以 SQLx 原生 startup `options[role]` 选择各目标 owner。三个服务真实 migrate-only binary 的 `--import` 均 exit 0，分别执行平台 12、门店 3、工单 4 个正式迁移；没有伪写迁移历史。执行 binary 的准确 SHA 和耗时记录于[脱敏机器证据](uuid-u04-summary.json)。平台入口后续补充的控制字符拒绝补丁不改变已执行正常 URL 的迁移结果，后续重建使用最终修复版。

初始 catalog 已私有保存，并计算 `schema_material` 的 SHA-256。目标共 147 个 UUID 列，业务表无 seed；平台只保留正式 migration 创建的 `platform_settings` 单例。随后 `--seal` 设三目标持久默认只读，owner 保持 NOLOGIN，等待受控导入。

| 服务 | 初始 schema SHA-256 |
|---|---|
| 平台 | `8940c083085342cf75dd54f2611fec1499ceaded6f524ece82fd3dfb45d2ba48` |
| 门店 | `82d96cd29c38ee9c7fc30345f1a980d640539dd8a9f4724b2f45b2fa058b13c5` |
| 工单 | `9870eb28b8e12c10ba967fc02a3738812db631c250b57ccd74b1bda922bfbf08` |

## 初始化入口验证

三个服务新增薄 migrate-only 入口，调用各自正式库迁移函数，不自动建库、不加载 dotenv、不启动服务或 worker。`--seed`、混选和非法参数在连库前拒绝；平台正常启动的 import 模式会阻止 seed 和 `region_seed` 运行。

Windows 与 Linux 的专用参数、URL、隐式 PG 环境与 pgpass、控制字符边界测试已通过。独立 PG17 fixture 实跑三条正式迁移链，重跑后完整迁移记录不变，检查的业务表为空，公共表/索引/序列归 NOLOGIN owner，退出后连接清零；临时库及角色已清理。具体最终门禁随本阶段交付统一记录。

## 阶段验收

| 验收项 | 当前结果 |
|---|---|
| 固定独立目标、真实迁移和 seed/import 互斥 | PASS；真实 12／3／4 项迁移及既有参数／进程负例 |
| 加密包导入、逐行/业务/引用/UUID 对账 | PASS；666,610 条保留行、逐行 source／target／owner proof 和实际行回读 |
| 导入后触发器、权限、schema 和序列有效 | PASS；147 个 UUID 列、67 个 FK、完整原始结构摘要、角色围栏和序列独立核对 |
| 同组重建与保数据恢复演练 | PASS；失败且未开放的固定三目标重建、真实迁移、重新封存、复用原认证包成功导入 |

三库没有跨库原子 COMMIT。后库失败时，已提交的前库继续隔离；`setval` 不随事务回滚，失败目标须按固定目标重建合同处理，不能只用 ROLLBACK 声称完全恢复。旧源和 U03 加密映射、源包、转换包继续保留。

## 导入试跑记录

`import-01` 使用二进制 `e3dabc102d34a06991f3f29511051ab4fbe653eeb2640f2fdd02968a0c06c557`，在完整包与行证明检查后的目标预检阶段失败，用时 91.220 秒、进程树峰值 RSS 27,344 KiB。错误为 native evidence 提前结束，没有创建导入输出目录或任何提交证据。

随后将实际 fence SQL 以 `BEGIN READ ONLY` 在平台新目标复现，定位为 PostgreSQL 对 AND 条件求值重排：即使 WHERE 含 `relkind='S'`，仍可能对 toast 等非序列关系调用 `has_sequence_privilege`，产生“不是序列”的错误。只读 CASE 原型通过且其余 owner、ACL、角色准入和只读门禁均满足。正式实现正在补 CASE 类型门控与真实 PG17 回归；这次失败不计为导入通过，也不绕过权限校验。

CASE 修正版 `884a142ffdd06390b0a5528d11c0dad8715f74acae392fc1e3be8eae2898ea64` 已通过[真实 PG17 fence 回归与门禁](../../../ops/recovery/uuid-import-tests.md)。`import-02` 随后通过 fence，但元数据 SQL 没有物理行尾换行，psql 等待后续输入；目标会话停在只读 catalog 检查，未创建输出目录、未提交。经核对精确 executable、父子 PID 和本次配置后，仅结束该 psql 子进程，使连接关闭并回滚事务；总用时 423.183 秒、峰值 RSS 27,448 KiB。正在中央 SQL 发送接口补行尾帧边界。

本机 PG18 的目标 catalog 同时确认 `NOT NULL` 约束登记为 `contype=n`：平台 298、门店 55、工单 43 项。兼容修正必须将这些约束保留在真实源/目标对账中，未知约束类型继续拒绝，不能为继续导入而忽略它们。

`import-03` 使用中央补 LF 与 PG18 `n` 兼容版 `a320172b384fd33b77566e36168035c76c2f5f6d57f40a1775b892205ff86153`，目标预检在 92.992 秒、RSS 33,380 KiB 时拒绝门店 schema contract，无输出目录或提交。逐库只读复查确认平台 12 项、门店 3 项、工单 4 项 SQLx 历史及工单 legacy 历史均完整；门店指纹函数依赖原 `search_path=public,pg_catalog`，在统一 `pg_catalog` 上下文中返回不同 deparse 指纹。原 owner 上下文实测为 true。实现正在仅为该校验恢复原上下文并随后恢复 `pg_catalog`，不改已存指纹、不放宽历史核验。

截至上述预检时，受控重建工具已完成 14 项独立 Linux fixture，真实默认只读 record 也已通过；当时尚未执行 DROP 或重建演练。该代目标 OID、owner、集群身份、源围栏、角色准入和六旧 writer generation 已绑定到持久私有 receipt。脚本 SHA 为 `670e3bf22cbd62ff6dffa58968022da36acd1260a3ae0ca1f30bff4e13e82e55`，当时 record 摘要为 `664b53edb2af0e1c1930c44ee495e6b11b7dd408fb187035c61a8e4d73402f38`；随后实际重建见下文。

## 接续审计、真实重建与本代初始化

上述内容保留各次执行当时的状态；最新接续事实如下，初始摘要与初始 OID 不复用于重建后的目标代。

[接续审计](resume-audit-20261024.md)确认 `import-05` 已在 369.185 秒后失败退出，没有完成文件、逐库提交证据或数据库 comment，目标业务表为空。私有 PostgreSQL 日志把错误定位到正式组织层级触发器：导入器的 `pg_catalog` 搜索路径使其内部未限定的 `organizations` 无法解析。

修复仅在真实 INSERT 的触发器执行窗口使用 `pg_catalog,public`，随后恢复 `pg_catalog`。历史函数、迁移、正常触发器及结构／权限检查保持原有合同。Windows 153 项、Linux 161 项测试通过；两平台 Clippy、格式及 Linux release 通过。独立 PostgreSQL 17 回归先复现原故障，再验证组织／部门父子数据的真实 CSV→COPY→INSERT 与逐行回读，原触发器仍拒绝循环更新。执行版 SHA 为 `71802231f06f75c64b77bac62b284a3cf7a02b7882820266bf134334933883d4`。

主线程依据固定目标清单实际完成失败恢复演练：先重新观察并登记未开放的三目标，再只重建这三库、保留全部源与角色，以冻结的真实迁移二进制执行 12／3／4 项迁移，最后重新设置默认只读。重建前 receipt SHA 为 `50275437c13076f74559c710cb7366e2200672473c839ef9c9eedd889146e9e5`，重建完成 receipt SHA 为 `0988c84818bda203704ba54fca85ee82a041407096f8f00f4eee3f097e987647`。本代数据库 OID 为平台 24339、门店 24340、工单 24341；原 owner OID 22102／22104／22106 保留，均为 NOLOGIN。

`import-06` 在 92.919 秒、RSS 30,732 KiB 时被结构摘要预检拒绝，没有目标写入。两路独立只读审查确认：1541／228／165 项结构材料中，仅 PostgreSQL 重建生成的 256／8／4 个内部外键触发器自身名称随系统 OID 改变；非内部结构项完整相等。仅在离线分析副本中规范化这些名称及 definition 中的唯一精确同名 token 后，完整多重集相等；正式迁移 checksum、工单 legacy 历史和门店 v1／v2／v3 fingerprint 均通过。生产完整原始摘要算法没有改动，旧配置及失败证据保留。

| 服务 | 本代经审查的完整原始 schema SHA-256 |
|---|---|
| 平台 | `a11665c8c4a4282bf406a54cb61b82105fc9ea2c077b1ff717c9947eed47e3af` |
| 门店 | `2d989b86b44b7f597aef31bfadf5b3eae3b697364e2fd5030cb32fc676cda550` |
| 工单 | `b8a502cb0e3bcfdd7ed4c00fdfbbf21af1f527feddb5b61bebc4f13859732a3e` |

初始化审查记录 SHA 为 `725a184b70612d1e6ca91d034401bdd3cd37b42a7881c0fafaddad9132ad15d1`，独立复核记录 SHA 为 `bedb056ca14a99422b5867049d31bf17d57e0308ed272f28b06adb5015e0106b`。新建的 `import-07` 配置只绑定这代经审查的原始摘要和新输出目录，不修改旧配置、映射或数据包；该轮随后真实导入和独立复核均已通过。

接续期间另清理了旧外部中断留下的三份派生明文，共 974,478,788 字节；先核对固定私有路径、owner、权限、精确文件集合／大小和无真实导入进程，再逐文件删除。原始加密源包、转换包、映射与冻结归档保持受控留存。

## 最终导入与独立复核结果

`import-07` exit 0，用时 357.234 秒，进程树峰值 RSS 38,180 KiB，stderr 为空。三库均完成 COMMIT、逐库密文证据写入与认证回读，最后发布并完整认证 `complete.enc`；状态为 `QUARANTINED_THREE_TARGET_IMPORT_VERIFIED`，没有启动任何服务。完成报告明文 SHA-256 为 `f49f6a37a7da14d34648ada3f9ff972abb5a3324f9e9a15d91f3e1262a3a7562`，密文文件 SHA-256 为 `1c237202564b9c316e6e2dffe28e7e27762432f361bfe99ffd92ef4e7e3f921b`。

| 服务 | 保留行 | 实际重建元数据行 | 目标总行 | UUID 列／非空单元格 | FK |
|---|---:|---:|---:|---:|---:|
| 平台 | 666,605 | 12 | 666,617 | 107／1,929 | 64 |
| 门店 | 5 | 6 | 11 | 23／28 | 2 |
| 工单 | 0 | 8 | 8 | 17／0 | 1 |
| 合计 | 666,610 | 26 | 666,636 | 147／1,957 | 67 |

源共 666,750 行；按既定合同排除 119 条旧凭据和 21 条源迁移元数据，新目标的 26 条元数据全部由正式迁移真实生成。665,923 条地区数据保持源业务内容；没有重新运行 seed。导入器复算所有保留行的 source／target／owner projection proof，并按源 ordinal 回读真实目标行，比对所有列、金额、时间、JSON、计数器及类型化引用；提交前业务图、约束、结构、角色及序列检查全部通过。

随后独立只读查询再次确认：147 个 UUID 列均无 v4 默认，1,957 个非空 UUID 单元格版本／variant 全部合法；67 个 FK 全部 validated，disabled trigger 为零，六类 credential 表均为零行。三库完整原始结构摘要与本代初始化交接相同，三个 comment 绑定同一导入批次；目标默认只读、owner 继续 NOLOGIN。平台 audit 序列的 `last_value`、`is_called`、increment 和已保留最大 ID 与源完全相同，下一候选值高于已保留记录。

最后用新的输出目录执行真实重复导入负例 `import-08-duplicate`，在 91.594 秒后按 target admission／clean epoch 围栏拒绝，exit 1；没有创建输出目录或覆盖已完成证据。全部脱敏计数、逐库结果、receipt、执行版本和密文摘要保存于 [uuid-u04-summary.json](uuid-u04-summary.json)。

本阶段的恢复结论仅覆盖失败且从未开放业务写入的 U04 目标组。U05 的实际配置、凭据轮换和服务运行验收，以及 U06 的最终新鲜源证明、正式新目标、切流和新写后的恢复仍须分别执行；本记录不冒领这些阶段完成。
