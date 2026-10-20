# Phase 27 台账终态更新建议

## Suggested Task Ledger Update

### Phase Status Update

Phase 27: **COMPLETED**。阶段名称：交付门店 Vite 微应用与双消费者契约实证。当前指针推进 Phase28 NOT_STARTED；last_completed_phase 为 27，next_action_requires_new_execution_request 为 true，不自动执行 Phase28。

本建议采用 source revision `32956c4532bbd6eff53584fd0f5e9bb6e188437c`，输入 SHA256 `02304b07f8c9b25a34b124898c7ce06ef283f092eb95910dde73fa828b559b9d`，809 文件；[r2 输入](../../target/phase-27/inputs-candidate-r2.json)及[r2 边界](../../target/phase-27/boundaries-final-r2.json)与严格审计 [target/phase-27/managed-audit-r2.json](../../target/phase-27/managed-audit-r2.json)一致。严格审计状态 `PHASE27_GATES_PASS`，材料 625 份；开发和被拒绝的 r1 记录单独列入 [target/phase-27/development-index-r2.json](../../target/phase-27/development-index-r2.json)，不进入完成统计。

### Output Summary

- [独立 React／TypeScript／Vite 门店前端](../../business-apps/stores/frontend/README.md)：固定 SDK 1.0.0、相对 opaque ESM／动态 chunk、资料／负责人／员工／营业记录／服务端汇总，401／403／409／未知写、主题／路由／续期与身份隔离。
- [stores 精确登记与管理 CLI](../../business-apps/stores/deploy/README.md)：独立 appCode，11 菜单、9 capabilities、3 成员用途；工单7菜单及用途独立，secret 留在服务端。
- [双消费者真实成员契约和差异](../../docs/stores-member-contract.md)：同一平台受控 HTTP，stores id 与工单 userId 映射、用途／owner／恢复差异；业务跨应用401、平台跨应用403、未登记purpose400均有真实输出。
- [阶段报告](../../docs/validation/phase-27.md)、[输入清单](../../docs/validation/phase-27-inputs.json)、[边界](../../docs/validation/phase-27-boundaries.json)、[运行材料](../../docs/validation/phase-27-runtime.json)及本建议，由 write-once publisher 统一发布；不回写旧正式证据。

### Completed Items

| 门禁 | 状态 | 命令步数 | counts |
|---|---|---|---|
| fast | PASS | 63 | 1033/1033; fail 0; skip 0; cancelled 0; 未实现测试 0; filtered 1931 |
| browser-chromium | PASS | 14 | 21/21; fail 0; skip 0; cancelled 0; 未实现测试 0; filtered 0 |
| browser-chrome | PASS | 14 | 21/21; fail 0; skip 0; cancelled 0; 未实现测试 0; filtered 0 |
| browser-msedge | PASS | 14 | 21/21; fail 0; skip 0; cancelled 0; 未实现测试 0; filtered 0 |

完整 fast 63 命令全部通过；实际 TAP／Cargo 1033/1033，filtered 1931 单列，failure／skip／cancelled／未实现测试均0。每浏览器 21 typed 组独立计数。fast CLI TLS 实际 TAP父测试 1/1，内部9具名检查；每浏览器直接TLS脚本各9具名检查且无TAP父测试，不能合计成单一测试数。

三个产品均完成仓库外、初始空自有 npm cache／无 node_modules 的独立 npm ci 与 build，根 node_modules 和个人 cache 依赖均 false，dist 无 secret sentinel；门店及工单 dist 各自完整字节清单、四原始 ELF、实际 ESM／chunk 和截图全部绑定。真实平台／两业务 Rust／三独立库、正常 HTTPS、普通账号及官方 API／CLI 初始化均成立。

普通用户实际资料／员工／日报与汇总、显式版本冲突采纳、金额十进制字符串、实际提交丢响应后读回且零自动重试、只写无读行为均通过。实际300秒应用token自然续期保留11.22输入；主题不新增业务请求，路由取消迟到读，401清除与403冻结均实证。四实际页面共享组织A→B，旧行／汇总／目录清除，旧提交零跨组织写，两页刷新及四页退出通过。

Node v24.18.0／npm 11.16.0／rustc 1.97.0 (2d8144b78 2026-07-07)，实际浏览器版本由阶段报告和原始运行记录列出。四执行均保持宿主／冻结／运行副本输入字节、前台stdin lease至完成，owner容器／网络结束查询为空；内层服务／三库／账号／临时目录／浏览器清理全true。最终自有cache `ci-phase27-cache-1792286916077-ebc2d9dd-cargo` 实际移除，owner校验true、无附加容器、剩余owner volume为空；保留原始四条清理命令。

114份历史正式证据、其他36阶段与全局缓存保持；原r1四outer FAIL及三typed FAIL、开发失败和Windows POSIX-only skip都保留。原始失败、读审查诊断、补充构建与正式结果分列，未把原始raw21或审查正向案例改标为gate PASS。

## Self-Check

| Acceptance Criterion | Result | Notes |
|---|---|---|
| 独立目录npm ci/build成功，无根node_modules/个人缓存依赖，dist无凭据。 | PASS | Chromium、Chrome、Edge 均由原始日志重解析；`production-react-vite-independent-build`；[chromium typed report](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/result.json)、[chrome typed report](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/result.json)、[msedge typed report](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/result.json) |
| 普通用户在真实平台/门店Rust/业务库完成资料、员工关联、每日记录及汇总，不用mock或直写平台DB授权。 | PASS | Chromium、Chrome、Edge 均由原始日志重解析；`ordinary-user-authorization-menu11`、`store-create-manager-directory`、`store-update-status`、`staff-association-unassignment`、`daily-create-update-duplicate-summary`、`staff-report-without-store-read`；[chromium typed report](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/result.json)、[chrome typed report](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/result.json)、[msedge typed report](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/result.json) |
| opaque iframe module/chunk、Origin:null/CORS、主题/路由/取消/续期/失权实际通过。 | PASS | Chromium、Chrome、Edge 均由原始日志重解析；`opaque-origin-null-rust-no-cookie`、`theme-keeps-draft-no-request`、`route-cancellation-and-not-found`、`normal-renewal-keeps-legal-input`、`revocation403-freezes-clears`、`unauthorized401-clears`；[chromium typed report](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/result.json)、[chrome typed report](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/result.json)、[msedge typed report](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/result.json) |
| 双页工单+门店刷新/切组织/退出无跨组织旧数据或草稿提交；两应用token/权限/数据grant不串用。 | PASS | Chromium、Chrome、Edge 均由原始日志重解析；`dual-host-shared-session-org-switch`、`dual-host-refresh-logout-fences`、`stale-form-handler-cannot-cross-org`、`cross-app-bearer-permission-grant-denials`；[chromium typed report](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/result.json)、[chrome typed report](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/result.json)、[msedge typed report](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/result.json) |
| 两业务同一成员协议实证和差异记录齐全，未登记动作/跨应用调用拒绝，Phase 30的输入无需虚构。 | PASS | Chromium、Chrome、Edge 均由原始日志重解析；`dual-actual-controlled-member-http`、`cross-app-bearer-permission-grant-denials`；[chromium typed report](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/result.json)、[chrome typed report](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/result.json)、[msedge typed report](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/result.json) |

## Suggested Task Ledger Update Continued

### Remaining Items

- Phase27实施与验收无余项。终态同步按root具体提案、独立审阅、绑定提案SHA后apply／verify执行。
- Phase28镜像、公开v1／v2路由、发行与生产部署均未执行，保持NOT_STARTED；Phase30公共能力收敛保留后续归属。
- 内存草稿无刷新持久保证，未知写无自动重试／补偿，成员见证最后校验至COMMIT的跨库竞态继续记录，不作为本阶段虚构保证。

### New Decisions

| Decision | Reason | Impact |
|---|---|---|
| 完整fast＋三产品定向真实双消费者浏览器作为Phase27最终矩阵。 | 覆盖本阶段独立构建、真实业务、opaque、会话／组织／撤权及成员协议；全部使用同一r2冻结输入。 | 不继承旧阶段结果，不将本矩阵描述为全部历史浏览器／PG重跑。 |
| stores／work-orders继续独立精确用途与DTO投影，暂不抽公共框架。 | 真实调用暴露id／userId、owner、用途、Idempotency-Key／显式读回差异。 | 供Phase30基于真实证据收敛，避免通过扩大白名单消除差异。 |
| 原始r1失败不采用；窄修复后新冻结并重跑四门禁。 | 原typed元数据键与旧policy失败不能由raw21观察或工具诊断替代。 | 保留旧FAIL与审批／边界原件，r2结果独立可复核。 |
| 最终材料只由严格收集器与write-once publisher发布。 | 原始命令／counts／source／ELF／dist／资源清理闭包均须核对。 | 终态提案独立审阅后应用，保护其他阶段和历史证据。 |

### New Open Questions

| Question | Impact | Suggested Owner |
|---|---|---|
| 无阻塞Phase27完成的新问题。 | 镜像发行／公开路由和跨库最后校验竞态按既定后续边界处理。 | Phase28／Phase30后续阶段负责人。 |

### Next Recommended Phase

**Phase 28：交付工单/门店镜像与统一外部 URL 接入**。状态保持NOT_STARTED，需用户新请求；输入为本次双frontend构建清单、原始四ELF、精确登记manifest、成员差异和四门禁证据。此建议本身不启动后续阶段。

### Synchronization Scope

建议将台账更新至v0.37，同步Phase27详情与五项原文验收、阶段摘要、当前执行指针、相关决策／变更记录和最新采纳说明；last_completed_phase=27、current_phase=28、phase28=NOT_STARTED、next_action_requires_new_execution_request=true。具体字节变更由root终态工具prepare产生，经独立审查具体proposal和SHA后apply，再verify确认全部绑定。不要把尚未执行的终态工具或下阶段工作冒记为已完成。
