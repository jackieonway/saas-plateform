# Phase 31 执行与验收

状态：**COMPLETED**。执行范围仅为“交付受支持升级与 dry-run 保留策略”；Phase32 未实施。生产 apply、外部发行和生产部署均未启用或执行。

## 输入与产出

- Git HEAD：`32956c4532bbd6eff53584fd0f5e9bb6e188437c`。原工作树未提交内容全部保留；没有 reset/restore/commit。
- 基线：862 项，`178ffd6ed48417df5769fc9126c5afad4013e9d2aa2f5f631d20a3c1b7c98b43`；最终：885 项，`621ea69d5326f9b043cde3a01bf6ccbd2dd454179abb6b71efc78a76847f339e`，36 个变更路径。Git HEAD 不能替代工作树输入摘要。
- [输入与差量](phase-31-inputs.json)、[逐命令/日志 SHA/原始 ELF/清理证据](phase-31-runtime.json)、[支持矩阵、扩展窗口和分类政策](../upgrade-and-retention.md)、[建议台账更新](phase-31-ledger-update.md)。正式执行源和宿主源码前后摘要一致。
- 平台仅新增 0004；所有旧平台/工单迁移、Cargo.toml/Cargo.lock 字节不变。135 份原有正式证据受保护；其他36个阶段详情保持原样。
- 新增纯策略库、平台根限定 GET 预览、Rust CLI retention-preview、持久在途保护、真实升级/预览/旧适配器测试及 CI 入口。没有新增第三方依赖、共享业务数据库模型或根 workspace。

## 正式实测结果

最终受管验收为 `target/phase-31/formal-r3/result.json`，32 步全部 PASS；以下均为本次最终同一输入实际执行，没有继承前期通过数。总计 108 次 Rust 测试执行（含专项、发布回归和 CI 入口对相同测试的重跑，不称 108 个独立用例）及17项 Node政策测试；必需选择零 FAIL、零 ignored/skip。

| 验证组 | 实际结果 | 原始记录 |
| --- | --- | --- |
| 公共保留策略 | 2 PASS，年龄/最近版本并集、30天边界、弱政策拒绝、active/previous/共享/跨环境/在途保护与指针交换 | target/phase-31/formal-r3/public/reports/retention-public-tests.log |
| Rust 管理 CLI | 46 PASS，含新命令拒绝 apply/弱参数、两次实际 GET 和安全字段投影 | target/phase-31/formal-r3/public/reports/admin-tests.log |
| 冻结旧业务客户端 combined / legacy | 各 1 / 1 父测试 PASS；每次真实平台、门店和工单三库/独立角色及业务 ELF；创建/更新/列表、工单同键重放保持 UUID 和编号 | target/phase-31/formal-r3/public/reports/old-business-clients.log；target/phase-31/formal-r3/public/reports/old-business-clients-legacy.log |
| 平台升级/只读预览/冻结旧宿主 | 2 PASS | target/phase-31/formal-r3/public/reports/phase31-live.log |
| 全部选定发布回归 | 17 PASS，包含6项 ZIP 登记/丢失响应/冲突/摘要保护，以及激活、重新预检、回滚及权限保持；包含上述2项的重跑 | target/phase-31/formal-r3/public/reports/release-regressions.log |
| 工单迁移矩阵 | 27 PASS；Node SHA256、SQLx v1/v2、混合历史到v3及重启，UUID/编号/receipt/intent 不丢，漂移/并发/失败回滚 | target/phase-31/formal-r3/public/reports/work-orders-upgrades.log |
| 门店初始化 | 1 父测试 / 8具名 SQL 检查 PASS，含空库、再启动、迁移和 schema 漂移拒绝 | target/phase-31/formal-r3/public/reports/stores-initialization.log |
| 管理审计回归 | 8 PASS，含发布审计 SQL抛错/零行抑制、事务回滚、重试及一次审计 | target/phase-31/formal-r3/public/reports/management-audit.log |
| 正式 CI 执行入口 | 3/3精确测试 PASS；真实结果标志及三库角色/数据库/连接池清理完整 | target/phase-31/formal-r3/public/reports/phase31-ci-protocol/postgres-result.json |
| 政策检查 | 17 PASS，保留现有平台12模块/76项声明、隔离数据库/流程/发行依赖边界 | target/phase-31/formal-r3/public/reports/ci-policy-tests.log |
| 构建/格式/静态/文档 | 平台 all-target check、平台/两业务 ELF；新策略/路由/操作/测试源定向fmt；平台窄库严格Clippy；管理CLI与两业务crate fmt/严格Clippy/doc；平台doc全部PASS | runtime逐命令表 |

平台旧库测试从确切 0001–0003 开始，先放入非空身份和数据授权，再逐表比较所有原 public 表及完整 SQLx 旧记录，升级0004后再启动一次；独立破坏旧 checksum 时迁移器拒绝且不修复任何行。工单/门店使用各自原迁移入口，未把平台实现 crate 导入业务服务。

## 只读和资源保护证据

预览用 READ ONLY REPEATABLE READ 读取全局引用及数据库时钟。测试在调用前后比较全部 public 表（含审计、token、迁移和操作保护表），证明无写入。未知 query/apply参数400、POST405、非平台根403；CLI只发送两次GET，拒绝任意apply和更弱政策，去除服务器附带字段与凭据。

试点只对静态发布提供影响预览，30天与最近5个不同版本取保护并集，参数只能加严。active、previous、同版本跨环境、共享URL/digest、在途、未来时间、外部所有权均保护；无理由者也仅 MANUAL_REVIEW，始终 applyAvailable=false、deletionPermitted=false。全局超过5000条/输出超过1MiB拒绝，计算移至阻塞线程池；不截断后给出遗漏引用的结果。

发布操作表只存未完成保护。可确定的结束只释放本次UUID；明确审计拒绝时原表最终数据保持不变。取消、进程退出、依赖/数据库传输失败或结果未知保持保护，无TTL。实测先取消一个受控在途操作，再单独保留结果未知操作；移除前者后仍全部保护，只有在临时库按确切UUID核对并移除两者后才重新出现人工审阅项。未知发布结果增加内部类型标签，现有409/CONFLICT响应契约不变。

旧静态镜像实际身份为 `sha256:1deec48663947c704f785d6b625e1d938182bf47ded4933dda39e626c4b23a41`，镜像版本1.0.0、来源输入 `f77df330f956da2d9bcb5ec96a5ae6fe617dc752f15267be565c3033d3be05c5`。本次隔离容器非root/read-only/cap-drop ALL/无宿主端口；仅在/tmp生成明确记录的版本前缀适配配置，不修改镜像文件。实际GET入口及HTML引用的脚本/样式共5个资源，验收前后均200且SHA256逐项一致；预览测试另登记并读取固定版本目录URL，字节保持。它证明本次隔离环境的在线URL，未声称历史生产路由或完整浏览器验收。

ZIP另行验证：9个真实上传版本的独立目录中app.js均保持原字节；ZIP登记/摘要/重试/跨环境与回滚回归独立执行。镜像仍存在不能替代URL在线，镜像URL成功也不替代ZIP磁盘证据。

8份旧消费者源码（宿主1份、门店/工单client/domain/SDK共7份）逐字节来自Phase30冻结源；provenance侧文件及旧源清单SHA均核对。宿主仅替换传输层，实际执行旧登记、发布列表及部署列表；业务客户端实际调用现有服务。结果覆盖API适配器层，不扩大为浏览器UI、多标签会话或Chrome/Edge完整版本矩阵通过。两模式实测不改变工单combined、门店legacy的生产默认。

## 失败与修复记录

- development-r1：年龄/最近版本测试预期错误，修正允许旧且非最近版本进入人工审阅。
- development-r2：迁移测试错误使用Migrator的Default；改为其显式DEFAULT。
- development-r3：测试造旧日期被不可变发布触发器拒绝；仅在唯一临时库事务中设定日期并恢复触发器。
- development-r4：旧镜像登记URL缺版本目录；隔离Nginx增加已记录的前缀适配，旧镜像字节不变。
- development-r5：23项工单迁移通过、4项Node互操作失败；补齐锁定pg测试依赖后最终27项重新执行全部通过。
- formal-r1：审计回归7通过/1失败，发现完成保护行会破坏原有最终状态回滚；改为仅持有未完成保护、确切完成释放，未知结果仍保留。
- formal-r2：legacy旧消费者失败；隔离fixture未挂OAuth内省路由，补成正式路由组合后两种模式重验。

上述FAIL保留原始输入、日志和空资源审计；最终PASS使用新的冻结输入，不把原轮剩余未执行门禁算作通过。完整索引见runtime.failedRunsPreserved。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 受支持旧库升级不改旧checksum，不丢UUID/编号/授权数据；旧前端兼容结果可追溯。 | PASS | 平台3→4全旧表/历史、工单27、门店初始化；8份冻结消费者及真实HTTP，兼容窗口见协议。 |
| dry-run只输出目标/数量/引用原因，不修改生产数据；未确定政策的表不自动apply。 | PASS | 全表只读对比、CLI GET-only与安全投影、HTTP/CLI拒绝apply；没有连接生产库。 |
| 版本清理拒绝active/previous/跨环境引用/在途任务/保留期资源；保留与回退测试覆盖试点。 | PASS | 纯策略+真实引用/取消/未知结果保护、旧镜像实际URL、9个ZIP目录及17项发布/回滚回归；没有删除执行器。 |

## 资源、限制与回退

最终受管容器、网络均按owner核验删除，数据库/角色/连接池与独立进程清理通过；本阶段专有缓存卷已核对标签后删除，空查见target/phase-31/cache-cleanup.json。以前阶段的镜像和证据保留。

生产期限、过期凭据缓冲、法律保留及旧页面最长生命周期仍属Q08；仅隔离候选完成，不启用生产apply。audit180/普通通知90/业务软删30天只列候选，未执行表purge。备份、恢复及法律保留与在线清理分开。完整浏览器/外部registry/生产部署、本阶段外的容量和加密灾备均未冒称执行。

全平台严格fmt/Clippy原有债务未宣布清洁，未顺手格式化旧模块；本次范围内实际通过项见上表及完整argv。旧ELF的SQLx迁移器可能拒绝新迁移，不能未经验证直接重启旧ELF当数据库回退；保持扩展schema，服务回退需另验迁移策略或恢复到经验证副本。静态回退沿用重新预检与active/previous交换。Phase32保持NOT_STARTED，须新请求。
