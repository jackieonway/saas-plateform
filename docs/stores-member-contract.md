# 工单与门店受控成员契约差异

日期：2026-10-18（Asia/Shanghai）。本文记录 Phase27 的源码契约与正式r2双消费者浏览器实证，供 Phase30 收敛公共能力使用。三个实际浏览器产品分别报告，当前源码规则与本次运行覆盖范围分开；不继承 Phase19／21／26 的历史数量。完整门禁、原始失败及资源清理见[Phase27验证记录](validation/phase-27.md)。

## 平台协议与业务投影

Phase 30 将两服务的授权输入收敛到机密客户端 Basic 合并验证（可显式配置 legacy 内省＋context），保留普通用户应用 Bearer 成员协议。两个独立 Rust 服务以逐请求授权 和平台 `/api/apps/{code}/members`、`/members/{id}` 受控 HTTP 获取有效直接成员。服务不读平台身份表，不要求通用用户目录管理权；平台、工单、门店各有独立数据库账号。目录不产生 capability、permission 或 grant，必须先登记精确 resource/action 并具备当前有效授权。

| 维度 | 工单 `work-orders` | 门店 `stores` |
| --- | --- | --- |
| 业务目录 | `/work-orders-api/assignees?action=create` 或 `update` | `/stores-api/members`，显式 resource／action |
| 平台 memberLookups | `ticket/create`、`ticket/update`，两种 | `store/create`、`store/update`、`store-staff/assign`，三种 |
| 平台成员 DTO | `{id, displayName, organizationId}` | 同一 DTO |
| 业务目录 DTO | 后端将平台 `id` 投影为 `userId` | 保留 `{id, displayName, organizationId}` |
| 前端提交映射 | 选中 `userId` 写入 `assigneeUserId` | 选中 `id` 写入 `managerUserId`；员工关联写入 `userId` |
| 成员用途 | 创建／更新工单处理人 | 可变负责人和独立员工关联；日报没有成员用途 |
| SELF owner | 工单创建人，处理人不扩大 SELF | 门店创建人／被关联员工／日报录入人；manager 与 `createdByUserId` 不改变 owner |
| 创建恢复 | 必需 Idempotency-Key，以原键／正文及 receipt 对账 | 无 receipt／Idempotency-Key，显式读回原业务域，不自动重发 |

源码入口：[工单目录](../business-apps/work-orders/backend/src/directory.rs)、[工单 query](../business-apps/work-orders/backend/src/directory/query.rs)、[门店目录](../business-apps/stores/backend/src/directory.rs)、[门店 API](../business-apps/stores/backend/API.md)、[门店 domain](../business-apps/stores/frontend/src/domain.ts)、[门店 controller](../business-apps/stores/frontend/src/controller.ts)。当前只记录真实消费者需要的差异，不因代码相似提前发布公共框架。

## 数据范围与成员见证

两方均使用现有 SELF、ORG、ORG_SUBTREE、TENANT，scope 不是可按序号比较的等级。每次操作须有精确 permission、唯一且身份绑定的对应 grant，再交集 tenant 与 context 组织硬上限。目录 SELF／ORG 目标只限当前组织；ORG_SUBTREE／TENANT 仍不能越过组织硬上限或借 query 扩权。

门店 store／staff／report 分别使用各自 resource/action；staff／report 校验目标店 tenant／organization，不暗加 `store/read`。staff SELF 为被关联员工，创建关联的人另记 `createdByUserId`；store SELF 为创建人，report SELF 为录入人。日报列表和 SQL 汇总共用 report/read，没有独立 summary capability，负责人或员工关系不能替代其他资源授权。

目录只提供短期候选。新分配／负责人变化／员工关联都由后端再次实际验证。需要旧业务状态的更新先非锁预读、释放业务连接，再发平台 HTTP，最后持锁复核组织、version 和旧成员快照。省略或相同 UUID 保留已有名称快照；真正变更／清除仍受组织 scope 约束，清除不要求失效历史成员重新有效。

成员见证从 HTTP 发起前以单调时钟计算 `min(配置超时, 3 秒)` 绝对截止；响应、连接／锁等待和提交前复核消耗同一窗口，不在锁后续期。HTTP 不可用和提交前见证过期分别拒绝；提交后期限／COMMIT 无法确认则保留未知语义。平台库与业务库没有共同事务，最后 member／grant 检查至 COMMIT 仍有撤权或失效竞态，不承诺跨库强一致或自动撤销历史关联。

## 双页与前端边界

工单、门店消费同一宿主会话，两页共享登录及当前租户／组织，没有门店登录或每页独立组织。SDK context 只用于请求和展示，后端仍逐请求复验。真实凭据／期限／身份／permission／grant／MENU／signal 替换隔离旧工作；纯主题不重置合法输入或解除 403。

门店 client 在收到 403 响应头时即锁定精确 resource/action 并取消同用途并行请求，不等待错误正文；controller 隐藏相关资源的服务器视图、编辑快照和目录。同用途迟到 200 不能回填旧视图，主题更新不解锁，其他用途不因此失权。401 使当前 context 完整失效并清除旧服务器视图。业务接口跨应用 Bearer 拒绝与平台成员接口跨应用拒绝是不同层级；未登记成员用途也须按真实接口状态单独记录，不能混写为同一种 403。

门店草稿只在存活 iframe 内存按 app／tenant／organization／user 分区；事件传 formId，提交重新核验 identity／route／context epoch 并取得新请求快照。切组织后旧处理器不能携旧草稿使用新凭据；同身份续期可保留合法输入，编辑版本需重读。刷新／关闭／销毁可能丢草稿，内存草稿不是工单创建 receipt 或提醒持久意图。

未知写保留原组织／门店／员工／营业日／version，再显式读回；发送时冻结用户确认的上海营业日，金额保持十进制字符串。只写无读仍可写；未知后无对应 read 时提示无法核对，不自动重发。原发送事件的 inFlight 状态由路由返回及同身份续期后的表单共同持有，只有原请求结束才能释放；等待期间禁止提前读回、确认核对或再次提交。

未知员工解除关联从原发送事件恢复原目标与核对入口，不依赖当前列表仍有该行。因此，即使解除已在服务端生效、下一次 GET 返回空列表，原事件也能继续核对。空列表仅是当前授权下的读回结果，不能当作证明历史删除成功的 receipt；用户明确确认后也不会自动发送新写入。

门店client每次HTTP默认15秒绝对总期限，fetch与成功／错误正文共用预算；read超时固定安全408／REQUEST_TIMEOUT，已发送write超时仍为unknown并要求读回，不自动重发，不撤销当前context或其他用途。成员见证的最多3秒提交窗口与此前端请求期限是不同边界，较长浏览器等待不能延长后端见证有效期。

## Phase27 正式r2运行证据

真实驱动为 `tests/phase27-stores-browser.mjs` 与 `tests/helpers/phase27-stores-services.mjs`：当前生产平台、工单／门店 Rust、三个自有隔离 PostgreSQL 库、生产宿主、静态 ESM、正常验证 HTTPS；应用／菜单／grant经正式API与Rust CLI建立。三产品正式外层run均为PASS／exit0，使用同一809项冻结输入：revision `32956c4532bbd6eff53584fd0f5e9bb6e188437c`，inputs SHA256 `02304b07f8c9b25a34b124898c7ce06ef283f092eb95910dde73fa828b559b9d`，见[候选输入清单](../target/phase-27/inputs-candidate-r2.json)。独立npm ci／build从自有空缓存与无node_modules目录开始，验证dist无秘密；真实业务响应由Rust服务及其业务库产生。结果丢失／迟到场景只丢弃或延迟真实响应，不伪造业务成功。

| 产品／实际版本 | 必需浏览器组 | 实际成员HTTP次数（门店／工单） | 原始typed报告 |
| --- | --- | --- | --- |
| Chromium 156.0.8078.4 | 21 PASS／0 FAIL／0 SKIP | 6／4 | [chromium r2](../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/result.json) |
| Chrome 155.0.8059.84 | 21 PASS／0 FAIL／0 SKIP | 6／4 | [chrome r2](../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/result.json) |
| Edge 155.0.4283.62 | 21 PASS／0 FAIL／0 SKIP | 6／3 | [msedge r2](../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/result.json) |

每产品另直接执行Rust管理CLI的9项具名TLS检查且全部通过，分别见[Chromium](../target/phase-27/final-browser-chromium-r2/public/reports/cli-tls/result.json)、[Chrome](../target/phase-27/final-browser-chrome-r2/public/reports/cli-tls/result.json)、[Edge](../target/phase-27/final-browser-msedge-r2/public/reports/cli-tls/result.json)。这些直接执行没有TAP父测试，9项检查与21个浏览器组分别计数；同一组在不同产品的执行也不合并成更多独立行为。

| 必需观察 | 三产品实际结果 | typed证据字段 |
| --- | --- | --- |
| 普通用户在两业务查询成员并分配／关联 | PASS；普通用户完成门店负责人、员工关联／解除及工单处理人分配，实际平台成员HTTP两消费者均有请求与记录 | `store-create-manager-directory`、`staff-association-unassignment`、`dual-actual-controlled-member-http`；`members.stores`／`members["work-orders"]` |
| 工单userId与门店id的提交映射 | PASS；同一peer在工单目录为userId、门店为id，分别实际写入assigneeUserId与managerUserId／userId | `dual-actual-controlled-member-http`及创建／关联记录 |
| 未登记成员用途拒绝 | PASS；门店业务成员入口与平台成员入口的store/read均400。unassign／日报无成员用途是源码声明边界，本轮未另立其运行组 | `cross-app-bearer-permission-grant-denials`中的`undeclaredMemberPurpose`／`actualPlatformMemberPurpose` |
| 跨应用Bearer、功能权限与当前grant隔离 | PASS；两业务跨应用token401，平台跨应用成员请求403，无create功能403，撤销store/read grant后真实Rust403并清旧行，主题不恢复 | `cross-app-bearer-permission-grant-denials`、`revocation403-freezes-clears` |
| 共享会话与组织隔离 | PASS；四个真实宿主页、两应用切A→B，旧门店／工单／汇总／目录隐藏；旧草稿处理器不产生跨组织写；两页刷新、四页退出 | `dual-host-shared-session-org-switch`、`stale-form-handler-cannot-cross-org`、`dual-host-refresh-logout-fences` |
| 授权替换、未知结果及资源独立 | PASS；实际应用token续期保留11.22输入，真实提交后响应丢失按原日期读回且零自动重试；仅staff／report读取者无store/read仍能完成员工读取与日报汇总 | `normal-renewal-keeps-legal-input`、`real-write-response-loss-readback`、`write-only-unknown-no-retry`、`staff-report-without-store-read` |

Phase30 再依据真实结果决定是否收敛 wire 校验、用途声明或消费者映射；不能通过扩大白名单、增加 STORE scope 或放松 owner 检查抹去差异。

本阶段浏览器结果只归纳实际执行的普通用户授权、成员查询／分配、拒绝、续期和跨页隔离观察；不能据此声称重新覆盖全部 SELF／ORG／ORG_SUBTREE／TENANT 组合、所有成员失效时点或最后检查至 COMMIT 的跨库竞态。上述数据范围和见证规则由当前源码说明，历史阶段的专项数量不并入 Phase27。首次正式r1浏览器虽原始21组通过，typed报告因workOrders／work-orders字段校验失配而外层FAIL，不能改写为本表的r2通过；完整原失败保留在阶段证据。
