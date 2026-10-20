# Suggested Task Ledger Update — Phase 15

阶段终态建议依据已核实的原始r2证据和三产品各11项设置补验；最终538项摘要、537项未变、driver归档及清理已通过[独立核验](../../target/phase-15/theme-supplement-verification.json)。原r2 Chromium失败及既有静态债务保留各自真实状态。

## Phase Status Update

Phase 15：`COMPLETED`。

Phase 16及后续仍为`NOT_STARTED`，本次未执行。

## Output Summary

- 独立`/system/data-grants`菜单/路由，独立read/write与实际平台根/可写目标校验；功能菜单和直接成员数据范围分开表达。
- 同Phase14读取边界的窄应用/有效直接成员发现，不授予通用用户或应用目录权限；严格五UUID、能力/scope闭集、预览/差量/版本、冲突/失权/unknown恢复和审计requestId关联。
- 普通用户功能授权和数据范围操作指南；已有安装显式采用新MENU，不自动扩权；历史不可编辑授权处理限制明确记录。
- r2：完整PG12组74项、fast28步568项PASS零必需skip；五真实浏览器版本live80项/5 harnessPASS，Node原有工单服务+独立业务PG同普通用户token实证SELF1/ORG2/撤销403及4次审计关联。
- r2原fixture：Chrome200通过、Edge200通过、Chromium196通过/1失败/3未运行，原总体FAIL保留。唯一后续源修复为既有平台设置测试保存等待竞态；Chromium/Chrome/Edge各11项完整设置套件通过、失败/跳过均0，与r2分列，不写600项一次全PASS。
- Windows96通过/92ignored；check/doc/选定七模块fmt/diff空白PASS；全仓strict fmt/Clippy既有FAIL保留，Clippy bin15/test-bin38与Phase14文件/消息多重集一致，无新增诊断。

## Completed Items

- 三项Phase15业务验收均PASS；完整设置补验、最终source-equivalence、driver/freeze归档和清理均已完成，详情/汇总可采纳阶段终态。
- 输入/范围、原始runner和driver/freeze归档已保留，r2七组host/container同538项`9fc2f8d68d46093746b874a40a7d2fc84fec5fec91ee5a52a6d275546e5287c7`；owned容器/网络/卷空、平台和业务owned库删除、pool关闭、runner及audit WSL lease exit0。
- 新MENU fresh seed三层显式平台管理员授权，已有安装不回填，原隐藏BUTTON保持；原Snapshot/Preview/PATCH契约与Phase14写范围保持。
- 原有工单backend加入验证输入前捕获摘要`277ffcab8794f4aebe3853b5022dcfe0c7cda49fa45a34fdc58ae659c5f48718`；19业务文件原样保留，13项新纳入受管输入，无Phase16实现。
- 早轮组织夹具失败、scope/VM前验失败、r1 UUID深链缺陷、r2主题竞态及各轮不同快照结果分别保留，未改写历史FAIL。

## Remaining Items

Phase15阶段剩余：**无**。全仓strict fmt/Clippy已披露的历史债务和原台账后续环境问题保留，不扩展本阶段实施范围。

Phase16及后续未执行，下一阶段需要新的明确指令。

## Self-Check for Ledger Adoption

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 平台根有独立权限可配置/撤销指定有效成员SELF/ORG等允许范围，不改平台DB。 | PASS | 正式页面/API完成SELF→ORG→撤销，预览不写、UUID保留，无日常SQL管理。 |
| 组织管理员无写入口，手改请求仍403；越界目标、失效成员、未登记动作拒绝。 | PASS | 只读根/组织管理员无写控件、真实伪造403；后端发现/读写及UI负向、精确路由和迟到响应fence通过。 |
| 保存后普通用户业务请求按新范围生效，审计可关联请求，无完整审批引擎依赖。 | PASS | 原有Node工单服务+独立PG、同一真实普通用户应用Bearer：SELF1/ORG2/撤销403，范围外不返回；4个requestId关联4行事务审计，无审批引擎。 |

## New Decisions

| Decision | Reason | Impact |
| --- | --- | --- |
| 独立根级MENU与独立read共同守护数据授权页 | grant管理者不应被迫取得用户、应用或授权管理目录权 | 精确MENU证据、隐藏BUTTON保留、已有安装显式采用 |
| options与Phase14 read边界共用当前权限/有效直接成员/显式策略 | 发现下拉项不能扩大管理读取能力 | 严格五UUID、全局10000组织/1MiB预算、授权后SQL分页，canWrite仅提示 |
| 预览精确delta后确认；409/unknown先读取，403退出写态 | 避免版本冲突、未知提交或失权后重放旧意图 | 目标/会话/权限变化清理草稿、预览和迟到回执，读操作独立复验 |
| 历史未登记pair或退出allowed集合的stored scope在UI只读 | 沿用本阶段已冻结展示规则，不额外引入revoke-only行为 | 页面清理须受控恢复必要登记；后端Phase14合法remove契约不改变 |
| 成功回执与审计独立授权，按requestId/app/tenant/action关联 | grant管理权不应扩大审计目录权 | 安全requestId经过会话fence后显示，审计面板仅system:audit可用 |
| r2原失败保留；唯一测试等待补丁采用逐文件source-equivalence与相关三产品完整套件补验 | 既有主题fixture把pending禁用态误当完成，修复不改变产品或其它测试输入 | r2三产品fixture596/1与补验各11分列；不声称600项同次全PASS，不因局部测试同步修复重复无关PG/fast/live |

## New Open Questions

| Question | Impact | Suggested Owner |
| --- | --- | --- |
| 无新增开放问题；既有Q02–Q11继续按对应后续阶段处理 | 不将隔离验收泛化为生产部署、容量或Rust业务迁移完成 | 原台账Owner |

## Next Recommended Phase

Phase 16：建立 Rust 工单服务并接管业务库基线。`NOT_STARTED`，需新的明确请求。

## Proposed Final Ledger Text

### Header and Global Execution Scope

版本建议`v0.19`，更新日期2026-10-16（Asia/Shanghai）；Phase1–15 `COMPLETED`，Phase16及后续`NOT_STARTED`。本次Phase15实施、验收及终态同步已完成，未执行Phase16及后续、外部镜像发布或生产部署；下一阶段须新的明确请求。

### Phase 15 Evidence Note

完成独立数据grant页面、同Phase14范围的窄发现、共享PATCH/requestId、精确预览/冲突/失权/unknown对账及独立审计查看。r2同538项`9fc2f8d6…`的PG12组74、fast28步568、五产品版本live80项/5 harness均PASS；SELF1/ORG2/撤销403、范围外不返回与4次requestId审计关联通过。fixture Chrome200/Edge200通过、Chromium196通过/1个既有theme保存等待竞态FAIL及3未运行原样保留；仅修复该测试等待后，以最终538项`7a7df9f10b03a7dda2dd0caec646f5ada16eef57245c7193ab0db8fd3ffe37b4`、537项未变证据及Chromium/Chrome/Edge各11项通过/0失败/0跳过分列补验。Windows96/92ignored、strict fmt与Clippy bin15/test38既有FAIL按实保留；补验与所有owned资源/双库/pool/WSL lease核验见[Phase15验证报告](phase-15.md)。不声称600项同次全PASS，不提前实施Phase16。

### Current Execution Pointer

`current_phase: 16`，`current_status: NOT_STARTED`，目标“建立Rust工单服务并接管业务库基线”；本次授权Phase15已完成，下一阶段需新的明确指令。Phase16开始前继续以其输入/依赖/禁止事项核对范围。

### Change Log

2026-10-16：完成Phase15，范围、产物、真实五产品版本live、同快照PG/fast、r2原fixture失败与唯一测试等待补丁三产品补验分别记录；最终源等价性与资源清理核验后采纳终态，指针16 NOT_STARTED。旧迁移和业务源码保留，无后续阶段、外部发行或生产操作。

完整结果与产物索引见[Phase15验证报告](phase-15.md)和[运行索引](phase-15-runtime.json)。
