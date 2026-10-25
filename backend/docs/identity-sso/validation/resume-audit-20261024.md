# 2026-10-24 接续审计：UUID 预演运行状态

本记录是接续任务中的只读事实核查，不把代码或 fixture 验证视为真实导入、部署或切流验收。审计使用 WSL Ubuntu 的进程清单、Docker 结构化检查、私有文件元数据与 PostgreSQL `BEGIN READ ONLY` 聚合查询；未打印连接 URL、凭据、映射、业务行或原始主体 ID。审计者未执行写库、导入、服务启停或数据库重建。

本记录初始数据库观察早于主线程随后执行的 U04 固定目标重建演练。下列旧目标 OID、空表计数及无 comment 结论只绑定初始观察时点，不能当作新 OID 的最终验收。审计结束时间为北京时间 2026-10-24 16:01；各观察并非跨库原子快照。

## 1. 接续时导入已经失败退出

最后登记的执行入口为 `/usr/local/libexec/saas-recovery-u04 uuid-import /dev/shm/saas-uuid-u03-data-20261024/import-05.json`。执行二进制 SHA-256 为 `f679596024021544d21290088d6ab5a46769aa38790065203300131c828ca830`，与 runtime volume 中 `uuid-recovery-target/release/saas-recovery` 一致。

私有 `/dev/shm/saas-uuid-u03-data-20261024/import-05-result.json` 记录：

| 项目 | 结果 |
|---|---|
| exit code | 1 |
| 执行时长 | 369.185 秒 |
| 进程树峰值 RSS | 35,424 KiB |
| stdout | 空 |
| 静态错误 | `recovery failed: import native evidence ended early` |
| 结果文件修改时间 | 北京时间 2026-10-24 15:52:06.850524 |

接续进程清单中没有运行中的 `saas-recovery`、`psql`、`cargo` 或 `rustc`。`/var/tmp/saas-uuid-u03-data-20261024/import-05` 存在且为空。此输出根目录下未发现 `complete.enc` 或任何 `committed-*.enc`。因此不能把旧 chat 的“开始导入”视为成功，不能将 UUID-U04 标记为完成。

配置仅核对了允许披露的路径及模式：source 为 `/var/tmp/saas-uuid-u03-data-20261024/source-04`，converted 为同根的 `converted-05`，scratch 为 `/dev/shm/saas-uuid-u03-data-20261024`，mode 为 `import`、seed 为 `false`、timeout 为 1800 秒。本次已产生结果文件，不是仍等待输出的活跃进程。

私有 tmpfs 中还留有 `recovery-2632da358584f63f0e72c230`，其修改时间为北京时间 15:43:41，只有 `import-source.ndjson`、`import-target.ndjson` 和 `import-proof.ndjson`，分别为 326,807,506、302,834,278 和 344,837,004 字节。未读取这些业务明文。该残留须继续按私有敏感材料处理，不能复制到仓库或用作完成标记。

## 2. 初始数据库观察

初始目标均没有数据库 comment，因而也没有 `saas-uuid-import/1/` 提交标记。

| 服务 | 目标名 | 初始 OID | public 表数 | 全表总行数 | 非迁移元数据行数 |
|---|---|---:|---:|---:|---:|
| 平台 | `qiankun_platform_uuid7_u04_20261024` | 22103 | 40 | 13 | 1 |
| 门店 | `qiankun_stores_uuid7_u04_20261024` | 22105 | 8 | 6 | 0 |
| 工单 | `qiankun_orders_uuid7_u04_20261024` | 22107 | 6 | 8 | 0 |

平台的 13 行为 12 项 `_sqlx_migrations` 和 1 行初始 `platform_settings`。其余 38 张业务表均为零行，包括 users、regions、audit_logs、OAuth token、session 和 refresh 表。门店为 3 项 SQLx 历史和 v1/v2/v3 三个 schema contract 单例，四张业务表均为空。工单为 4 项 SQLx 历史和 4 项 legacy 历史，四张业务表均为空。

初始三目标 disabled trigger 计数均为 0。平台有 2 项历史 `NOT VALID` 约束，门店与工单为 0；私有初始 catalog 已包含平台的 `application_releases_entry_type_check` 与 `ck_oauth_tokens_oauth_no_plaintext`。不能把这两个历史值归因于本轮导入失败，也不能由本记录宣布约束最终验收完成。

三源和初始三目标持久数据库设置均为 `default_transaction_read_only=on`；两旧服务角色 `qiankun_preview_20261024`、`qiankun_notification_preview_20261024` 以及三目标 owner 均 `NOLOGIN` 且非 superuser。独立维护库观察未发现六库活动连接，集群 prepared transaction 为 0。此结果只证明该次观察，不替代导入器的完整角色提权与 ACL 准入验证。

源库保留聚合：

| 源库 | public 表数 | 总行数 | 非迁移元数据行数 |
|---|---:|---:|---:|
| `qiankun_platform_preview_20261024` | 40 | 666,735 | 666,724 |
| `qiankun_stores_preview_20261024` | 7 | 9 | 5 |
| `qiankun_orders_preview_20261024` | 6 | 6 | 0 |
| 合计 | 53 | 666,750 | 666,729 |

平台源 regions 为 665,923 行、audit_logs 为 188 行、users 为 7 行；旧 OAuth token 为 105 行，session 与 refresh 各 7 行。这些是只读源证据，不能继续成为新 epoch 的活动凭据。门店业务行是 stores 1、daily_reports 1、audit_events 3、staff 0。工单业务表均为空。以上仅输出计数，没有查询业务行内容。

源任务及库外效果登记表的只读计数如下：

| 源服务 / 表 | 总行数 | active | terminal | 口径 |
|---|---:|---:|---:|---|
| 平台 `application_release_operations` | 0 | 0 | 0 | 所有保留记录都是 pending operation；本次无行 |
| 平台 `managed_deployment_jobs` | 0 | 0 | 0 | QUEUED/RUNNING 与其余登记 state 均无行 |
| 平台 `managed_image_artifacts` | 0 | 0 | 0 | 所有登记 state 均无行 |
| 平台 `zip_distribution_jobs` | 0 | 0 | 0 | QUEUED/RUNNING 与 READY/FAILED 均无行 |
| 平台 `zip_distribution_nodes` | 0 | 0 | 0 | PENDING 与 READY/FAILED 均无行 |
| 平台 `notification_publications` | 0 | 不适用 | 不适用 | append-only publication 证据，无 active/terminal 字段 |
| 平台 `notifications` | 0 | 不适用 | 不适用 | 投递记录，本次无行 |
| 工单 `work_order_reminder_intents` | 0 | 不适用 | 不适用 | immutable intent，不是重试队列，无 active/terminal 字段 |
| 门店 | — | — | — | 本次登记表集中没有任务队列表 |

任务表为空不能代替认证失效边界、agent 栅栏或库外效果核对。

## 3. 旧 writer 与私有材料定位

Docker 观察中平台、门店、工单、OAuth 示例和 chat 示例五个旧 writer 均为 exited；notification 固定旧容器名已经不存在，与既有 auto-remove 冻结合同相符。该事实本身不重新证明历史停写过程，原始冻结证据继续保留。

运行 volume 宿主路径为 `/var/lib/docker/volumes/qiankun-preview-runtime-20261024/_data`。旧冻结材料位于 `/dev/shm/uuid-preview-freeze-20261024T053211Z-9ffb368fa6a7871f`，其中有 `freeze-status.private.json`、`quiescence.json`、`failure.private.json`、`evidence/` 和 `resume-20261024T054336Z-5687d03eeb1682e9/`。密钥位于独立的同批次 `-key/` 目录；未读取或输出密钥字节。

U05 后续输入定位仅列文件名、路径与字段名，不保存值：

| 位置（工作区相对路径） | 文件 | 已观察的配置项名 |
|---|---|---|
| `saas-plateform/backend/.integration/local-runtime/` | `database.env` | LOCAL_DATABASE_HOST、LOCAL_DATABASE_PORT、LOCAL_DATABASE_USER、PLATFORM_DATABASE_URL、STORES_DATABASE_URL、WORK_ORDERS_DATABASE_URL、NOTIFICATION_DATABASE_URL |
| 同上 | `platform.private.env` | APP_ENV、APPLICATION_ENVIRONMENT、AUTO_CREATE_DATABASE、BIND_ADDR、CORS_ALLOWED_ORIGINS、DATABASE_URL、DATA_GRANT_POLICY、DB_ACQUIRE_TIMEOUT_MS、DB_STATEMENT_TIMEOUT_MS、JWT_SECRET、MICRO_ALLOWED_ORIGINS、MICRO_GATEWAY_ORIGIN、PUBLIC_APP_URL、RUST_LOG |
| 同上 | `preview-data-grant-policy.json`、`preview-state.private.json` | 本轮仅观察文件元数据；字段合同见 U05 runtime field catalog 与 config-rekey contract |
| `saas-stores/.integration/local-runtime/` | `preview-api.env` | NODE_ENV、PLATFORM_API_ORIGIN、PLATFORM_CLIENT_ID、PLATFORM_CLIENT_SECRET、STORES_ALLOWED_ORIGINS、STORES_DATABASE_URL、STORES_HOST、STORES_PORT |
| `saas-work-orders/backend/.integration/local-runtime/` | `backend.private.env` | NODE_ENV、PLATFORM_API_ORIGIN、PLATFORM_CLIENT_ID、PLATFORM_CLIENT_SECRET、WORK_ORDERS_ALLOWED_ORIGINS、WORK_ORDERS_DATABASE_URL、WORK_ORDERS_HOST、WORK_ORDERS_PORT |
| `saas-notification/.integration/local-runtime/` | `notification-linux.env` | DATABASE_URL、NODE_ENV、NOTIFICATION_ALLOWED_ORIGINS、NOTIFICATION_ENV、NOTIFICATION_HOST、NOTIFICATION_PORT、NOTIFICATION_SOCKET_PATH、PLATFORM_API_BASE_URL |
| `saas-examples/.integration/local-runtime/` | `oauth-preview.env` | OAUTH_ALLOW_INSECURE_LOOPBACK、OAUTH_AUTH_CLIENT_ID、OAUTH_AUTH_CLIENT_SECRET、OAUTH_BIND_ADDR、OAUTH_CC_CLIENT_ID、OAUTH_CC_CLIENT_SECRET、OAUTH_DEVICE_CLIENT_ID、OAUTH_PASSWORD_ENABLED、OAUTH_PLATFORM_ORIGIN、OAUTH_PUBLIC_ORIGIN、OAUTH_SCOPE |
| 同上 | `oauth-disabled.env` | OAUTH_ALLOW_INSECURE_LOOPBACK、OAUTH_BIND_ADDR、OAUTH_PASSWORD_ENABLED、OAUTH_PLATFORM_ORIGIN、OAUTH_PUBLIC_ORIGIN、OAUTH_SCOPE |

chat 旧容器仅观察到 CHAT_ALLOWED_ORIGINS、CHAT_ALLOW_SANDBOX_ORIGIN、CHAT_BIND_ADDR 等环境项名；本记录没有找到并核验独立 chat 私有 env 文件。所列旧配置尚未经真实映射、轮换和新目标消费验证，不得直接沿用并称为 U05 完成。

## 4. 已编译 imported 正常服务产物

下面是读取文件 SHA-256、修改时间及 imported 环境项名称存在性所得；未执行 binary，也未以字符串存在性替代运行验收。路径以容器 `/runtime` 为前缀，对应上节 runtime volume。

| 服务 | 文件 | 字节数 | SHA-256 |
|---|---|---:|---|
| 平台 | `/runtime/target/debug/saas-platform-backend` | 179,982,256 | `45049fa89b9272920448e49a5ac3af9e78d59c741dc6b65d68cfe9d33f8e03f0` |
| 门店 | `/runtime/stores-u02-target/debug/stores-api` | 69,220,584 | `87aa8b97132a9a3d73afa7b7d2a1b1e5b341cbe0891d81a5dd9b2c89413ca119` |
| 工单 | `/runtime/orders-u02-target/debug/work-orders-api` | 68,796,656 | `998eedd23231103d572a8d88345c1992edea7fea77cc221b19a5742b825e01cc` |

以上三个文件的修改时间约为北京时间 15:35，分别包含 PLATFORM/STORES/WORK_ORDERS 的 DATABASE_INIT_MODE 和 IMPORT_MARKER 项名。共享 `/runtime/target/debug/stores-api` 与 `work-orders-api` 是较早产物，未包含相应 imported 项名，不能混用。

U05 配置工具 `/runtime/uuid-config-target/release/saas-recovery` 的 SHA-256 为 `ac778c31bc2ee00377bb5a59517e8c807a374b98a978385a2ba0c9c1058865a5`，与既有 U05 工具验证记录一致。

主线程随后通知：三个服务 binary 已按上述摘要复制保护至 `/usr/local/libexec/saas-uuid-u05-platform`、`saas-uuid-u05-stores`、`saas-uuid-u05-orders`。该复制由主线程执行，本审计不把随后构建的身份功能 binary 视为上述 UUID 预演冻结产物。

## 5. 阶段缺口与接续边界

旧 chat 六项代理工作的接续结果由主线程读取其完成消息后汇总如下；此表是代理交接摘要，不把历史摘要替代当前工作树或运行验收：

| 旧代理任务 | 交接状态 | 当前限制 |
|---|---|---|
| `inspect_identity` | mapping 检查完成 | 其后文件摘要已变化，历史 hash 不能直接绑定当前实现 |
| `inspect_auth` | imported 启动代码与测试完成 | 真实新目标启动仍待验收 |
| `uuid_platform_tests` | import 前三项修复完成 | 最新 import-05 仍失败；主线程后续诊断为 trigger 中未限定 `organizations` 关系，修复另行执行 |
| `mapping_resume_review` | 配置工具代码与测试完成 | 真实配置改号与消费尚未执行 |
| `write_identity_ledger` | freeze / recreate 脚本测试完成 | 不等于导入、保数据恢复和切流完成 |
| `review_rollout` | 最终只读 rollout 审查完成 | U06 需要新的 final/recovery 固定目标和 fresh source 等价证明；既有 pilot 不能直接套用 |

主线程确认员工/客户身份 Phase 1 由用户另一个活动 chat 独占执行，本 chat 停止重复身份实现，本 chat 的 Phase 1 净文件变更为 0。此协调不取消本专项剩余工作。

| 阶段 | 已有事实 | 尚缺真实验收 |
|---|---|---|
| UUID-U04 | 固定目标真实迁移初始化；import-05 已失败退出，无完成包；初始业务目标仍为空 | 定位并修复 native EOF；新输出目录实际导入；逐行、业务、UUID/引用、触发器/权限/schema/序列核验；三库 committed 与 complete 认证回读；同组重建后的同映射保数据恢复 |
| UUID-U05 | SDK 3.0.0 repository candidate 与活跃消费者代码验证；配置工具实现门禁；imported 服务已编译 | 真实配置改号/重建与消费；JWT key 和 confidential client secret 轮换；旧凭据拒绝、新登录；六运行服务及平台/门店/工单/OAuth/通知/外部 RP 端到端验收 |
| UUID-U06 | 台账为 NOT_STARTED | 最终共同停写证据、最终包与独立最终目标、切流、旧源只读留存、恢复/重放与不会回退的认证及任务栅栏证据 |

实际后续应先保留失败目录和私有结果文件，依据诊断修复 native EOF；任何新尝试使用新的输出目录。失败后即使业务表为空，`setval` 的非事务性影响仍须处理，不能只凭 ROLLBACK 或空表复用旧目标。固定 U04 recreate 合同仅适用于从未开放流量的登记目标；不能扩展到旧源、生产库或切流后新写恢复。

主线程在本审计初始观察后已执行固定目标重建演练，回传 `sealed-before-rehearsal-01.json` SHA-256 `50275437c13076f74559c710cb7366e2200672473c839ef9c9eedd889146e9e5`，以及 `recreated-rehearsal-01.json` SHA-256 `0988c84818bda203704ba54fca85ee82a041407096f8f00f4eee3f097e987647`，后者状态为 `CREATED_UNMIGRATED`；两文件在 `/dev/shm/saas-uuid-resume-u04-20261024/`。主线程另回报之后三条真实迁移 12/3/4 均 exit 0 并再次 seal。该信息仅作接续交接，新的 OID、实际导入和完成证据应由后续执行记录另行绑定。

身份新库 Phase 1 与 UUID 保数据专项的预览目标必须分别管理。U04/U05 保持已冻结的 12/3/4 正式迁移链；不可在其上叠加员工/客户身份的 0013 迁移，亦不可因独立身份新库开发完成而修改 UUID-U04～U06 的未完成状态。本审计未修改任何台账状态。

## 6. 后续执行交接：新 generation 与 import-07

本节追加主线程在初始只读审计之后实际执行并回传的事实，不回写初始观察，也不把正在运行的导入标为验收完成。本节撰写时 `import-07` 正在运行，尚无最终退出结果、三库提交结论或完成认证回读结论。

### 重建、真实迁移与私有残留处理

主线程完成固定三目标重建、12/3/4 条真实正式迁移和再次 seal。重建前 receipt SHA-256 为 `50275437c13076f74559c710cb7366e2200672473c839ef9c9eedd889146e9e5`，重建 receipt SHA-256 为 `0988c84818bda203704ba54fca85ee82a041407096f8f00f4eee3f097e987647`；私有 receipt 路径见前节。新目标身份为：

| 服务 | 新 database OID | owner OID |
|---|---:|---:|
| 平台 | 24339 | 22102 |
| 门店 | 24340 | 22104 |
| 工单 | 24341 | 22106 |

主线程清理了唯一旧中断 scratch `/dev/shm/saas-uuid-u03-data-20261024/recovery-2632da358584f63f0e72c230` 下三个派生明文文件。清理前核实固定路径、owner、mode、大小及无真实 import 进程；清理字节总数为 974,478,788，即 326,807,506 + 302,834,278 + 344,837,004。原始加密源包、转换包和 typed map 保留。本操作由主线程执行，本审计者没有执行清理。

### trigger search_path 修复与 import-06

主线程完成 trigger search_path 修复，避免函数在导入器受限上下文中访问未限定的 `organizations` 关系。交接的验证结果为：Linux 161 项测试、Windows 153 项测试通过，两平台 clippy 与 fmt 通过；真实 PG17 合成层级的 COPY、INSERT 及循环拒绝验证通过。新执行 binary SHA-256 为 `71802231f06f75c64b77bac62b284a3cf7a02b7882820266bf134334933883d4`。

`import-06` 在目标预检阶段失败，用时 92.919 秒，峰值 RSS 为 30,732 KiB，错误为 `prepared target schema differs`。该次失败保留，不视为导入通过。

### 重建后 schema 原始摘要审查

主线程执行只读审查，并取得独立第二路审查。两路结论均为：重建仅改变内部 RI 触发器自身由 OID 派生的名称；平台、门店和工单 schema material item 数分别为 1541、228、165，非 internal 项完整相等，分析用 normalized multiset 全等。生产 raw hash 算法没有修改，也没有用 normalized 分析替代真实目标的 raw hash。

本次新 generation 已登记的新 raw schema SHA-256：

| 服务 | raw schema SHA-256 |
|---|---|
| 平台 | `a11665c8c4a4282bf406a54cb61b82105fc9ea2c077b1ff717c9947eed47e3af` |
| 门店 | `2d989b86b44b7f597aef31bfadf5b3eae3b697364e2fd5030cb32fc676cda550` |
| 工单 | `b8a502cb0e3bcfdd7ed4c00fdfbbf21af1f527feddb5b61bebc4f13859732a3e` |

12/3/4 项 SQLx metadata 已逐 SHA384 核对，工单 legacy 4 项及门店 fingerprint v1/v2/v3 均通过。主审查摘要为 `725a184b70612d1e6ca91d034401bdd3cd37b42a7881c0fafaddad9132ad15d1`，独立审查摘要为 `bedb056ca14a99422b5867049d31bf17d57e0308ed272f28b06adb5015e0106b`。

主线程为此新 generation 建立了新的 `import-07` 私有配置，保留旧 `import-06` 配置与失败证据后启动新尝试。`import-07` 的运行中状态不能替代每库 comment、`committed-*.enc`、`complete.enc` 的认证回读与业务验收。UUID-U04、U05、U06 的完成结论仍须等待后续真实证据；本次追加没有修改台账或宣告阶段完成。

## 主线程最终验收补记

本段补记晚于前述只读审计与运行中观察：`import-07` 已 exit 0，357.234 秒、峰值 RSS 38,180 KiB；三库逐库提交及最终完成文件均已认证回读。独立只读复核确认 666,610 条保留行、147 个 UUID 列、67 个有效外键、零非法 UUID 单元格、零旧凭据、原始结构摘要及序列一致。真实重复导入 `import-08-duplicate` 按已完成目标围栏拒绝，没有创建输出目录。UUID-U04 已完成，目标继续隔离且默认只读，未启动服务或切流；U05／U06 的真实运行与最终恢复验收尚未执行。最终逐库证据见 [uuid-u04.md](uuid-u04.md) 及 [uuid-u04-summary.json](uuid-u04-summary.json)。
