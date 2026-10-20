# Phase Execution Result — Phase 14

## Executed Phase

Phase 14：提供受控数据 grant 管理 API。**COMPLETED**，已同步台账。依赖Phase6/13均已完成；仅执行该阶段。

## Execution Summary

增加正式 GET /api/data-grants、POST /api/data-grants/preview 与 PATCH /api/data-grants。独立读/写权限、默认空拒绝的服务端策略、实际平台根写边界、精确direct membership、已登记应用资源动作、差量版本冲突、UUID保留、严格事务审计和下一请求实时生效形成闭环。

源码HEAD：`32956c4532bbd6eff53584fd0f5e9bb6e188437c`；冻结输入511项，SHA-256：`b48d12194b9c888844ed493fd8fbedd5967ac4f2c5e7c6260092b540922ec1b7`。实际工作树含此前未提交阶段变更，HEAD不等于源码输入；所有最终门禁使用同一份冻结字节。范围核对21项允许变更、245项保留输入、零删除。未改任何既有迁移或依赖、SDK、前端页面、业务服务或部署目标。

## Deliverables

- [管理契约与操作指南](../data-grant-management.md)：策略、权限、目标发现、请求/响应、冲突、审计和采用边界。
- [用例](../../backend/src/data_grants.rs)、[严格策略](../../backend/src/data_grants/policy.rs)、[存储](../../backend/src/data_grants/store.rs)、[薄HTTP路由](../../backend/src/routes/data_grants.rs)与[真实HTTP/PG回归](../../backend/src/data_grant_tests.rs)。
- [成员发现](../../backend/src/routes/users.rs)增量返回membershipId；[独立权限种子](../../backend/src/seed.rs)只为fresh bootstrap显式三层授予，既有安装无自动backfill。
- [工单离线审核模板](../../business-apps/work-orders/deploy/grants.mjs)改为正式API/实时版本步骤，保留历史SQL应急兼容；未添加执行CLI或UI。
- [输入快照](phase-14-inputs.json)、[范围](phase-14-boundaries.json)、[运行与artifact哈希索引](phase-14-runtime.json)、[已采纳台账更新](phase-14-ledger-update.md)。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 缺独立权限/非实际平台根/失效成员/不匹配身份/跨app或管理范围拒绝；合法根可管理明确允许业务租户 | PASS | 真实HTTP正负矩阵及完整业务/审计快照；tenant/org/祖先到期与停用、未知动作和scope、精确子树均验证 |
| 合法受控写入；普通业务成员与操作无需grant管理权 | PASS | writer独立鉴权；真实客户端签发应用token，普通业务permissions仅ticket:create/update，实际成员列表与单成员查询成功 |
| UUID保持、增删影响下一受保护新请求、读范围严格限制 | PASS | preview不写、增改删/no-op/旧版本409、同版本并发单胜、三种pg_blocking_pids锁等待后复核；复用同一真实token验证contextVersion及成员访问变化 |
| 审计失败回滚，使用正式管理API | PASS | trigger RAISE与RETURN NULL分别注入完整delta，业务/审计全量状态不变；重试恰一条requestId/actor/完整target审计，正式API完成所有受验变更 |

## Validation

| Gate / actual command | Result | Evidence |
| --- | --- | --- |
| 固定Node24 driver：full PG | PASS：11组 / 71通过 / 0失败 / 0跳过 | [PG报告](../../target/phase-14/linux-postgres-r2/reports/postgres/postgres/postgres-result.json)、[grant实际日志](../../target/phase-14/linux-postgres-r2/reports/postgres/postgres/data_grant_tests.log) |
| 同冻结快照：fast | PASS：28/28步骤，542项测试，必需零skip | [fast报告](../../target/phase-14/linux-fast-r2/reports/fast/summary.json) |
| cargo check --manifest-path backend/Cargo.toml --locked --all-targets --all-features；cargo doc --manifest-path backend/Cargo.toml --locked --no-deps | PASS | [静态报告](../../target/phase-14/static-final-r2/result.json) |
| Windows默认cargo test | 95通过 / 0失败 / 88 ignored，ignored不算PG通过 | [默认日志](../../target/phase-14/static-final-r2/windows-default-tests.log) |
| 新5模块rustfmt + diff whitespace | PASS | [新模块格式](../../target/phase-14/static-final-r2/new-module-format.log)、[差异空白](../../target/phase-14/static-final-r2/phase-diff-whitespace.log) |
| 全仓strict fmt / strict Clippy -D warnings | FAIL（已披露旧债务）；38个test-target错误，文件/消息多重集与Phase13一致，无新增诊断 | [债务比较](../../target/phase-14/static-final-r2/legacy-diagnostics.json)、[Clippy](../../target/phase-14/static-final-r2/strict-clippy.log)、[fmt](../../target/phase-14/static-final-r2/strict-fmt.log) |
| Docker Compose config及模板POSIX读取 | PASS：unset/完整嵌套JSON/显式空值与三模板 | [配置传值证据](../../target/phase-14/compose-policy-nested-result.json) |
| 源一致性/owned资源清理/WSL lease | PASS；所有最终源hash一致，专属容器/网络/卷与命名资源为空、lease退出0 | [runner终验](../../target/phase-14/runner-final-verification.json) |

fast新增策略5项与用例2项，config现6项；保留Phase13十组64项，不以新组替换旧回归。所有脚本/日志/输入及外部配置示例摘要在运行索引中可复核。全仓格式/Clippy失败未被禁用或标为通过。

前验保留：初稿中的201响应码/真实客户端fixture遗漏在运行PG前修正；一次新增Clippy needless borrow已修复，[前验日志](../../target/phase-14/clippy-preflight-r1.log)保留。r1的PG71与fast28/542实际通过，但最终静态比对发现审计测试匿名元组新增1条类型复杂度诊断（39个test-target错误），故未作为最终完成轮；命名结果结构修复后以新冻结字节重跑全部最终门禁。[r1诊断比较](../../target/phase-14/static-final-r1/legacy-diagnostics.json)和两候选原始输入/日志/清理证据完整保留，不将不同快照PASS拼接。

## Risks / Notes

- DATA_GRANT_POLICY是服务启动配置，所有副本采用同一版本并重启；策略不是热更新/委派API。positive-union管理目标范围与显式登记应用能力共同生效，默认空拒绝。新旧安装均需明确采用策略；既有安装通过现有菜单/授权API登记独立权限，不能依赖自动backfill。
- 配置收紧不会自动删除legacy grant。GET如实显示受限数量的stored grant；变更pair含remove仍需登记，清理前应保留该登记。stored TENANT在非平台根目标仍effective剪至ORG_SUBTREE，不按scope排序，不默认TENANT。
- 用户/成员核心锁串行正式grant写入；父租户/组织不加反序强锁，锁后与写后重新校验最新状态，授权线性化于该检查。并不声称所有平台授权操作全球串行；下一业务请求继续实时检查。
- 每member/app最多128条，差量1–128项；组织展开10000项，响应1MiB，超限提交前拒绝/回滚。target版本只做冲突检查，授权reader可交接给独立合法writer；不是凭据或全局幂等键。
- 下一请求验收使用真实平台context/成员辅助接口与真实签发app token；未把本轮扩大为外部工单CRUD、门店、生产部署、UI或性能容量验收。回退应用代码前先停用管理入口，数据库未新增迁移，已提交grant按审计核对并用正式差量API恢复。

## Suggested Task Ledger Update

[结构化更新块](phase-14-ledger-update.md)已按用户要求实际采纳：Phase14 COMPLETED；四项验收勾选、摘要/决策/Q06/Change Log同步；当前指针Phase15 NOT_STARTED。Phase15及后续尚未执行。
