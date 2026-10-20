# Phase Execution Result — Phase 15

状态：**COMPLETED**。验收采用已归档 r2 完整运行和唯一测试文件修复后的三产品完整受影响套件补验；原始 r2 Chromium FAIL、不同输入摘要及既有静态债务分别保留。来源与清理总证明见[补验与输入等价性核验](../../target/phase-15/theme-supplement-verification.json)。

## Executed Phase

Phase 15：实现数据 grant 管理界面与授权闭环。`COMPLETED`。

依赖 Phase 9、14 已完成。本次只执行 Phase 15，保留此前未提交内容，不执行 Phase 16、业务 Rust 迁移、外部发行或生产操作。台账仍为阶段范围和验收依据；本稿对原始运行的失败、跳过和不同输入快照分别记录。

## Execution Summary

新增独立 `/system/data-grants` 页面、同 Phase 14 读取边界的窄应用/直接成员发现、严格 DTO/API 封装和共享会话 transport 的 PATCH 与成功 requestId 元数据。页面区分功能权限、数据范围与 grant 管理权限，显示 stored/effective scope 和组织硬边界，只允许明确登记的能力与范围。可写目标先预览精确差量，再确认保存；冲突、失权、未知保存结果、目标/会话变化均有明确恢复流程和旧响应隔离。审计查询继续独立要求 `system:audit`。

r2 的真实平台/隔离 PostgreSQL/原有工单服务闭环在五个真实浏览器产品版本中共 **80 项检查、5 项 Rust harness** 全部通过；完整 PostgreSQL **12 组 74 项**、fast **28 步 568 项**均通过，必需检查零跳过。原三产品 fixture 矩阵实际为 Chrome 200/0、Edge 200/0、Chromium 196/1，失败是既有平台设置主题测试未等待保存完成的竞态。r2 原始总体状态因此继续保留 **FAIL**。

随后仅对 `tests/platform-settings-browser.mjs` 增加 `saveThemeColor` 等待保存完成的辅助流程，三产品完整平台设置套件各 **11 项通过、0 失败、0 跳过**。最终 538 项输入摘要为 `7a7df9f10b03a7dda2dd0caec646f5ada16eef57245c7193ab0db8fd3ffe37b4`，其中其余 537 项逐字节不变，相关 driver、结果、清理及 WSL lease 已核对。r2 与补验分别列示，不称为“同次三产品 600 项全部通过”，也不将补验所用 managed fast 生命周期称为完整 fast 重跑。

## Deliverables

- 独立[管理页面](../../frontend/src/pages/system/data-grants/DataGrantsPage.tsx)与[编辑状态机](../../frontend/src/pages/system/data-grants/useGrantEditor.ts)；独立路由及原 Authorizations/Users 页的有权入口。
- [API封装](../../frontend/src/api/data-grants.ts)、[DTO/差量校验](../../frontend/src/lib/data-grants.ts)；[共享transport](../../frontend/src/lib/api.ts)增加 PATCH 和经过 session fence 的安全成功 requestId。
- 窄发现[用例](../../backend/src/data_grants/discovery.rs)、[存储](../../backend/src/data_grants/store.rs)与[HTTP边界](../../backend/src/routes/data_grants.rs)。原 Snapshot、Preview、PATCH 契约及 Phase 14 写入边界保持。
- 独立可见 MENU `/system/data-grants` 和 `system:data-grant:read` 入口；保留原隐藏 read/write BUTTON。fresh seed 仅向初始化平台管理员角色、平台租户和平台根组织显式授权，已有安装不自动回填扩权。
- 回归覆盖：[后端发现](../../backend/src/data_grant_discovery_tests.rs)、[真实UI harness](../../backend/src/data_grant_ui_tests.rs)、[浏览器闭环](../../tests/phase15-data-grants-browser.mjs)、[竞态](../../tests/phase15-data-grants-races.mjs)、[差量校验](../../tests/data-grants.mjs)、[transport](../../tests/data-grants-transport.mjs)。
- CI managed browser 增加真实平台/owned PostgreSQL 管理闭环；原 fixture 矩阵继续保留自己的 UI-only 语义。主题测试的等待补丁只修改测试同步，不修改产品行为或断言目标。
- [管理契约](../data-grant-management.md)、[普通工单用户授权指南](../data-grant-user-guide.md)、[README](../../README.md)、[架构](../architecture.md)与[CI说明](../../deploy/ci/README.md)。
- 证据入口：[最终输入快照](phase-15-inputs.json)、[阶段范围核对](phase-15-boundaries.json)、[运行及产物索引](phase-15-runtime.json)、[台账更新建议](phase-15-ledger-update.md)；完整 r2 和补验原始报告保持各自摘要与状态。

## Self-Check

以下三项验收均已满足。真实业务证据、唯一测试同步修复的完整受影响套件补验、输入等价性和资源清理分别核对；这里的 PASS 不改变 r2 原始 fixture 失败状态。

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 平台根有独立权限可配置/撤销指定有效成员SELF/ORG等允许范围，不改平台DB。 | PASS | 五个真实浏览器版本均经 Vite→Axum→隔离 PG 选择目标、核对空 grant 默认拒绝、只读预览、保存 SELF、修改 ORG、同值无变更及撤销；保存均走正式 API。修改 scope 保留 grant UUID，membership UUID 保留。“不改平台DB”指无需管理者直接 SQL 操作；正常 API 事务仍持久化授权。 |
| 组织管理员无写入口，手改请求仍403；越界目标、失效成员、未登记动作拒绝。 | PASS | 只读根和组织管理员无写控件；真实伪造请求403。PG discovery 3项与保留 grant 7项验证独立权限、有效直接成员、租户/组织/祖先状态、精确目标及未知能力/范围。真实 UI 覆盖负向矩阵、精确 MENU/read 路由、迟到目标、编辑后的预览、替换会话和未知提交后先对账。 |
| 保存后普通用户业务请求按新范围生效，审计可关联请求，无完整审批引擎依赖。 | PASS | 每个 live harness 连接原有 Node 工单服务与独立业务 PG，通过真实平台内省/context，使用同一普通用户应用 Bearer：SELF 1条、ORG 2条、撤销后403，范围外记录始终不返回；4次真实差量的 requestId 顺序与 audit_logs 一致，审计查询独立授权。未引入审批引擎。 |

## Source Provenance and Change Boundary

源码 HEAD：`32956c4532bbd6eff53584fd0f5e9bb6e188437c`。工作树包含前阶段未提交内容，HEAD 不能代替真实输入摘要。

| Snapshot / verification | Actual evidence |
| --- | --- |
| Phase 14 基线 | 511项，`b48d12194b9c888844ed493fd8fbedd5967ac4f2c5e7c6260092b540922ec1b7` |
| r1 候选 | 538项，`e4c9956f7e5054ad5fce3d10f21f96fe8e2fbe973f45de7d058368ce8eb01da6`；历史结果单独保留 |
| 已完成 r2 候选 | 538项，`9fc2f8d68d46093746b874a40a7d2fc84fec5fec91ee5a52a6d275546e5287c7` |
| r2 边界核对 | [r2边界记录](../../target/phase-15/boundaries-r2.json) 为 PASS；42个新增/变化路径，497个保留输入逐字节不变，0删除；另外登记13项原有工单 backend 输入，不能将这些计数直接相加当作总文件数 |
| 原有业务源码加入输入范围前摘要 | [原始业务文件清单](../../target/phase-15/business-backend-before-inclusion/manifest.json)；业务文件摘要 `277ffcab8794f4aebe3853b5022dcfe0c7cda49fa45a34fdc58ae659c5f48718`，19个公开业务文件保持，13项首次纳入受管输入；这不是新业务实现，也不追溯宣称 Phase14 验收过遗漏文件 |
| r2 运行归档 | 七组 Linux 结果的 `candidate-evidence-verification.json` 均核对 host/container 前后同538项摘要、driver/freeze 归档、owned资源清理和 WSL lease；Chromium gate 的原始状态仍为 FAIL，归档核对通过不把它改成 PASS |
| 补丁后输入 | 538项，`7a7df9f10b03a7dda2dd0caec646f5ada16eef57245c7193ab0db8fd3ffe37b4`；[逐文件等价性证明](../../target/phase-15/theme-source-equivalence-r3.json) PASS：537项未变，0新增/删除，唯一变化文件为`tests/platform-settings-browser.mjs`；其sha256由`05da2d145f509b9584b98062ab79351b0750ebec65ebbb6d6f26876b91efb04e`变为`fe88e9f90737837b32504632f595c915f58d2e139ae6cb326733d3e7ea84cb5a`。 |

r2 的 PG、fast、data-grant live、静态 Rust 检查及不属于平台设置套件的 fixture，其产品源码和相关测试输入未因该补丁改变。逐文件 source-equivalence 核对已通过，保留这些原始证据，同时将补丁覆盖的三个完整平台设置套件补验单列。它们不构成最终全部门禁在同一个新摘要下重新运行的声明。

## Actual Validation Results

Linux正式运行使用 Node `24.18.0`、npm `11.16.0`、Rust `1.97.0`；实际 argv 与时间以各 summary/result 为准。PG使用合成凭据的 owned 隔离库；没有访问生产连接或读取私有 `.env`。

| Gate / actual scope | Result | Evidence |
| --- | --- | --- |
| r2 完整 PostgreSQL | PASS；12组74项，0失败、0跳过 | [PG结果](../../target/phase-15/linux-postgres-r2/reports/postgres/postgres/postgres-result.json)；runner 4步，独立库删除与连接池关闭已核对 |
| r2 完整 fast | PASS；28步568项，必需失败/跳过均0 | [fast结果](../../target/phase-15/linux-fast-r2/reports/fast/summary.json)；契约385、部署36、CI55、dev/Vite19，Rust选定单元合计73；build/lint/typecheck/check通过 |
| r2 Chrome完整fixture + live | PASS；fixture200/0/0；live16项 + Rust1项 | [fixture汇总](../../target/phase-15/linux-browser-chrome-155-r2/reports/browser/summary.json)、[live结果](../../target/phase-15/linux-browser-chrome-155-r2/reports/browser/data-grant-ui/result.json)；Chrome `155.0.8059.84` |
| r2 Edge完整fixture + live | PASS；fixture200/0/0；live16项 + Rust1项 | [fixture汇总](../../target/phase-15/linux-browser-msedge-155-r2/reports/browser/summary.json)、[live结果](../../target/phase-15/linux-browser-msedge-155-r2/reports/browser/data-grant-ui/result.json)；Edge `155.0.4283.60` |
| r2 Chromium完整fixture + live | **FAIL**；fixture196通过/1失败/0跳过/3未运行；未完成200项，不能补记为200；live16项 + Rust1项均PASS | [fixture汇总](../../target/phase-15/linux-browser-chromium-155-r2/reports/browser/summary.json)、[live结果](../../target/phase-15/linux-browser-chromium-155-r2/reports/browser/data-grant-ui/result.json)；实际Chromium `156.0.8078.4`，目录中的155是选定runner标签，不当作实际产品版本 |
| r2 Chrome上一稳定版live | PASS；16项 + Rust1项，0失败/跳过 | [Chrome154结果](../../target/phase-15/linux-live-chrome-154-r2/reports/browser/data-grant-ui/result.json)；Chrome `154.0.8037.92`；不是完整fixture重跑 |
| r2 Edge上一稳定版live | PASS；16项 + Rust1项，0失败/跳过 | [Edge154结果](../../target/phase-15/linux-live-msedge-154-r2/reports/browser/data-grant-ui/result.json)；Edge `154.0.4258.62`；不是完整fixture重跑 |
| r2 fixture/live汇总 | fixture596通过/1失败/0跳过/3未运行；live80项/5 harness全PASS | [r2汇总](../../target/phase-15/candidate-r2-summary.json)保持总体FAIL及原失败详情；不声称600项一次通过 |
| 补丁后Chromium完整平台设置套件 | PASS；11项通过/0失败/0跳过；Chromium `156.0.8078.4` | [r3 Chromium设置补验](../../target/phase-15/linux-theme-chromium-r3/reports/fast/summary.json)；scope为`phase15-platform-settings-supplement-only`，实跑`tests/platform-settings-browser.mjs all`，覆盖两个自定义颜色及其余九场景；不称完整fast |
| 补丁后Chrome完整平台设置套件 | PASS；11项通过/0失败/0跳过；Chrome `155.0.8059.84` | [r3 Chrome设置补验](../../target/phase-15/linux-theme-chrome-r3/reports/fast/summary.json)；独立同scope运行，与r2 Chrome200项分别陈述 |
| 补丁后Edge完整平台设置套件 | PASS；11项通过/0失败/0跳过；Edge `155.0.4283.60` | [r3 Edge设置补验](../../target/phase-15/linux-theme-msedge-r3/reports/fast/summary.json)；独立同scope运行，与r2 Edge200项分别陈述 |
| Windows Rust check/doc | PASS | [静态结果](../../target/phase-15/static-final-r2/result.json)；check使用all-targets/all-features；doc使用no-deps |
| Windows默认Rust测试 | 命令PASS；96通过/0失败/**92 ignored** | [默认测试日志](../../target/phase-15/static-final-r2/windows-default-tests.log)；默认ignored不是PG通过证据，正式必需PG和live另列 |
| 选定Rust模块fmt、阶段diff空白 | PASS | [模块格式](../../target/phase-15/static-final-r2/new-module-format.log)、[diff空白](../../target/phase-15/static-final-r2/phase-diff-whitespace.log)；7个新增/变化模块，main/seed既有整文件fmt债务单列 |
| 全仓strict fmt | **FAIL**（既有格式债务，完整原始日志保留） | [strict fmt日志](../../target/phase-15/static-final-r2/strict-fmt.log)；不把局部格式通过写为全仓通过 |
| 全仓strict Clippy | **FAIL**；bin15、test-bin38；无新增文件/消息类别 | [Clippy日志](../../target/phase-15/static-final-r2/strict-clippy.log)、[诊断多重集比较](../../target/phase-15/static-final-r2/legacy-diagnostics.json)；与Phase14按文件/消息多重集比较，39渲染块前后相同、added/removed均空，分类PASS不等于Clippy命令PASS |
| 源/资源/lease核对 | r2与三组补验均PASS，最终538项摘要和537项未变证明PASS | [总核验](../../target/phase-15/theme-supplement-verification.json)：七组r2及三组r3 driver/freeze归档核对；owned容器/网络/卷空、平台/业务临时库删除及pool关闭；补验无host mounts/公开端口，专属资源空；runner与独立audit WSL lease均stopped且exit0。 |

PostgreSQL十二组的实际通过项数：production_security 8、platform_settings 4、authorization_isolation 3、entity_creation 6、zip_registration 6、oauth_issuance 10、platform_session_refresh 4、login_protection 5、management_audit 8、notification 10、data_grant 7、data_grant_discovery 3，共74。真实UI单独执行，不计入这74项。

## Real Browser and Business Proof

五个真实产品版本各运行同一16项闭环：根上下文空grant默认拒绝；预览不写；SELF保存与真实业务；ORG更新保留UUID；同值无请求；过期版本冲突；审计requestId关联；撤销后下一业务请求拒绝；只读根；组织管理员与伪造403；越界/无效成员/未知能力矩阵；独立read路由；旧目标迟到；编辑后旧预览迟到；会话替换后旧响应；未知保存结果先读取对账。

每个harness启动已有Node工单服务并连接独立owned业务PG，平台PG和业务PG分开。普通用户使用同一真实应用Bearer，通过本地隔离平台HTTP进行内省/context，再请求工单列表；SELF=1、ORG=2、撤销=403、组织外记录始终不返回。每轮4个不同mutation requestId与数据库4行`DATA_GRANT_CHANGE`按顺序对应，membership UUID保持、grant最终删除，平台和业务库/连接均清理。此证据是现有业务服务的记录级隔离实证，未提前建立Rust工单服务或迁移业务库，也不声明本轮验收了TLS或真实域名部署。

live harness仅隔离与本阶段无关的通知WebSocket；grant发现/读取/预览/PATCH/审计、平台内省/context和业务HTTP/SQL均真实。业务列表由驱动使用普通用户的真实Bearer发起；不声称完整工单iframe用户旅程或真实通知服务通过。[Chrome155正式r2管理预览截图](../../target/phase-15/linux-browser-chrome-155-r2/reports/browser/data-grant-ui/management-preview.png)已由主执行者检查：布局清晰，说明/控件延续原有风格，无凭据展示。前验[视觉核对记录](../../target/phase-15/visual-review-wip-r2.json)亦保留；未作移动端UX验收。

## Failure History and Theme Test Repair

- 早期524项输入`cb652f1a2f1d91455c7af91dff11e84f2130569765c2bea415f87a2e3523e674`：discovery3项通过；Chrome live夹具因`uq_org_root_per_tenant`拒绝同租户第二个根而失败，未启动页面。修正夹具为唯一业务根及两个子组织，未改变生产约束。早轮日志及`assertions_ok=false`如实保留。
- 首次业务下一请求证据仅为平台context/成员辅助接口，不能证明记录级SELF/ORG。后续加入原有Node工单服务及独立业务PG的真实列表，并先捕获原业务文件摘要；没有为验收提前实现Phase16。
- 新Node合同前验曾因scope排序与跨VM对象原型比较而失败，修正夹具/复用plain转换；两组21项通过后才进入完整fast，不放宽生产校验。
- r1（538项`e4c9956f…`）PG74/fast568/Windows96+92ignored、Chrome fixture200通过；live在带userId深链的第二页面失败。定位为前端UUID正则误写为8-4-4-12却要求36字符，修正为8-4-4-4-12，并保留原深链用例及fence。
- 修复后WIP r2与正式r2同538项`9fc2f8d6…`。WIP独立归档不替代正式门禁；正式r2七组运行的实际结果如上。
- 正式r2 Chromium fixture在`theme_custom_contrast_light`失败：断言读到上一笔`#000000`而期望`#FFFF00`（旧脚本第325行）。旧等待条件只看“输入已大写/提交已禁用”，按钮在保存进行中也会禁用，导致后续保存或断言赶在本次完成之前。其他产品通过不能抹去该FAIL。
- 唯一后续源补丁是`saveThemeColor`：保存前注册匹配本次大写颜色的PUT响应等待，等待`response.finished()`，再等待字段值、提交按钮文本/禁用态及fieldset恢复，最后检查只产生一次保存与精确颜色。两个自定义颜色场景复用它；非法颜色、对比度和其余场景断言保持。补丁只读审查无新增正确性问题；Chromium/Chrome/Edge分别完整运行11项全部通过，原r2 FAIL及其196通过/1失败/3未运行保持。

## Risks / Notes

- grant仍绑定当前有效直接成员，不继承祖先角色；空grant默认拒绝。SELF仍需要业务所有者校验，TENANT仍可能被目标组织硬边界收窄。菜单/按钮/业务grant三条权限不互相替代，工单和门店资源不混用。
- `DATA_GRANT_POLICY`仍是服务端启动配置，配置改变需要所有副本同步采用并重启；没有新增审批、委派、配置写API、热更新或默认TENANT。
- 新MENU已有安装必须经既有管理API显式采用，隐藏读写BUTTON不搬移，不自动给所有现有用户回填权限。
- 未登记pair或stored scope退出当前允许集合的行在UI只读，不进入差量；本阶段不增加revoke-only例外。策略收紧不会追溯删除业务grant。通过页面清理前，有权运维需受控恢复必要pair/scope登记，再按正式预览/PATCH撤销；后端对已登记pair的合法remove仍保持Phase14契约。
- 预览不预留锁，expectedVersion不是凭据，requestId不是幂等键。409/403/unknown不应禁用fence、扩大权限或自动重放。未知结果先GET，读取现状不代表上次保存成功。
- 默认Rust92 ignored、strict fmt/Clippy既有失败明确披露。隔离测试不是生产容量、真实外部部署、后续Rust工单迁移、门店、完整通知或完整业务iframe验收。
- 回退管理UI不应删除已提交grant/审计。恢复业务范围从当前快照生成明确反向差量并重新预览；不改旧迁移checksum、不删库、不重放未知提交。旧宿主bundle会话采用边界仍按Phase10。

## Suggested Task Ledger Update

见[结构化台账更新建议](phase-15-ledger-update.md)。Phase15三项验收全部PASS，阶段剩余无；下一建议是Phase16“建立 Rust 工单服务并接管业务库基线”，`NOT_STARTED`，需新的明确执行请求。
