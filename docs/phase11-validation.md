# Phase 11 工单业务试点验收

## 验收状态

2026-10-07，工单试点的组件门禁和最终正常全量联合验收全部通过，台账 Phase 11 四项验收标准均为 PASS。最终联合门禁 **16/16 PASS、0 fail、0 cancel、0 skip，总时长 596.846 秒**；本记录作为阶段完成与提交的依据。

2026-10-02 首次联合运行通过 14 项实际 HTTP 场景，随后浏览器发现宿主循环分包与原生 `fetch` receiver 缺陷；两项以及认证失效、读写拒绝或详情不存在时的缓存／草稿处理均在暂停前修复。2026-10-07 恢复后继续实际浏览器验收，修正了 iframe 可用高度与窄屏编辑弹窗的滚动布局；前端 22 项回归再次通过。

恢复后的首轮正常全量运行报告 14/16 PASS、0 skip、约 489 秒，浏览器步骤因测试夹具在前置应用撤销后继续使用旧并发写入 token 而失败；旧 token 的 401 安全断言保留，并发写入改为经真实平台重新获取有效 token。提醒步骤等待真实发布提交及按钮恢复可用、主题切换查找实际 AppRail 控件等测试同步问题也已修正。这一轮没有标作阶段完成。

修正后的 browser-only 调试完整通过 2/2（含总测试）、0 skip、204.10 秒，覆盖实际 Vue 操作、冲突草稿复核、提醒未知结果重试、Socket.IO／收件箱、CSV、软删除、主题／窄屏、组织切换与回滚。随后正常全量运行独立通过全部 16 项，不以调试结果代替阶段门禁。最终验收使用冻结的生产宿主产物 `index-CdmaausN` 与 `MicroAppContainer-DsP4CNL3`；v1→v2→回滚过程中 100 个宿主文件的逐文件哈希保持一致。

业务范围来自用户于 2026-10-02 确认的[工单试点提案](./business-pilot-proposal.md)。系统控制层、应用上下文、独立数据授权、通知及发布能力继续使用 Phase 7～10 的契约。

## 交付内容

| 内容 | 实现与配置入口 |
| --- | --- |
| 独立业务应用 | [工单 README](../business-apps/work-orders/README.md)，应用编码 `work-orders`，默认 `IFRAME + HTML` |
| Vue 前端 | [frontend](../business-apps/work-orders/frontend/README.md)，Vue 3.5.43、classic IIFE、固定版本的 Vue 与 iframe SDK |
| Node 24 业务 API | [backend](../business-apps/work-orders/backend/README.md)，独立 PostgreSQL、逐请求应用令牌内省及当前应用上下文校验 |
| 业务迁移 | `business-apps/work-orders/backend/migrations/0001_work_orders.sql`；工单、租户编号计数器、范围索引及不可变身份 trigger |
| 最小成员辅助接口 | `backend/src/routes/app_members.rs`；按应用操作与独立数据 grant 返回有效直接成员的 ID、姓名、组织，不开放平台管理目录 |
| CSV 下载协议 | `frontend/src/components/micro/iframe.ts`、`examples/micro-app-sdk/iframe-bridge.js`；由宿主保存 CSV，iframe 保持 `allow-scripts allow-forms` |
| 管理员初始化和发布 | [deploy](../business-apps/work-orders/deploy/README.md)，bootstrap、显式授权草案、登记、预检、启用、回滚 |
| 服务与网关 | Compose 显式 `work-orders`／`work-orders-local-db` profile；Caddy、Nginx 保留 `/work-orders-api` 完整前缀 |
| 最终联合流程 | `business-apps/work-orders/tests/platform-e2e.mjs`；真实平台 main、两座隔离 PostgreSQL 数据库、业务服务、通知、生产宿主、Vue 与浏览器 |

平台与业务库分别保存控制数据和工单。业务服务不读取平台表、不共享平台 JWT 签名密钥；浏览器没有机密客户端 secret 或平台 session 存储。业务模型不以本地模拟数据代替真实 CRUD。

## 已完成的门禁

| 门禁 | 结果 | 实际范围与证据 |
| --- | --- | --- |
| 独立业务 API | PASS，48/48；30.90 秒；0 skip | `business-apps/work-orders/backend/tests/config.test.mjs`、`api.test.mjs`；真实 Node HTTP 与 PostgreSQL，平台内省、上下文、成员及发布响应使用受控 HTTP fixture |
| 平台成员辅助 API | PASS，2/2 组 | `backend/src/notification_tests.rs` 的 `app_members_scopes_minimal_dto_and_recipient_validity` 与 `app_members_authorization_and_next_request_revocation`；真实完整平台迁移、PostgreSQL 和进程内 Axum API |
| 工单实际应用 handler 单元 | PASS，8/8 | `business-apps/work-orders/frontend/tests/app.mjs`；加载实际业务 bundle 并调用实际渲染 handler，Vue、DOM 和 HTTP 使用单元 fixture，不计为真实浏览器验收 |
| 工单客户端与领域单元 | PASS，12/12 | `business-apps/work-orders/frontend/tests/client.mjs`；权限表达、四范围、状态、真实 fetch 调用方式、取消、迟到响应、401、409、提醒及 CSV |
| 独立构建配置 | PASS，2/2 | `business-apps/work-orders/frontend/tests/build.mjs`；拒绝远程 HTTP，允许 HTTPS 与字面回环开发来源；独立生产构建通过 |
| iframe 协议与 CSV broker | PASS，21/21；0 skip | `tests/micro-iframe-protocol.mjs`；真实隔离浏览器中的 MessagePort、nonce、epoch、生命周期、授权与下载边界；文件保存适配使用宿主测试注入 |
| 管理员 bootstrap／授权工具 | PASS，31/31 | `business-apps/work-orders/deploy/tests/platform-admin.mjs`、`grants.mjs`；CLI 子进程、本地 HTTP fixture 和离线 SQL 草案，不执行真实平台授权 SQL |
| Caddy 网关 | PASS，30/30 | `tests/micro-release-gateways.mjs`；实际 Caddy 及 HTTP 传输 fixture，验证静态版本、通知和独立业务 API 路由 |
| Nginx 网关 | PASS，30/30 | 同一网关契约；实际 Nginx，覆盖完整前缀、方法、Bearer、正文、查询及未知路径 404 |
| Compose profiles | PASS，配置检查 | 两个工单 profile 的 `config` 校验；未执行镜像构建或容器启动 |
| 平台 backend 检查与构建 | PASS | Rust 检查及含最新成员辅助 API 的 Linux main 构建 |
| 宿主 TypeScript 与生产构建 | PASS | 完整 TypeScript 检查，修复循环分包后的 Vite production build；最终浏览器 gate 使用实际生产产物 |
| 实际浏览器诊断流程 | PASS，2/2（含总测试）；204.10 秒；0 skip | 同一联合脚本的 browser-only 调试模式，实际平台／双库／生产宿主／Vue／通知；完整浏览器流程通过，但不计作正常全量 HTTP + 浏览器门禁完成 |
| 最终实际平台联合验收 | PASS，16/16；596.846 秒；0 fail／cancel／skip | `business-apps/work-orders/tests/platform-e2e.mjs` 正常全量运行；计数含总测试及四个嵌套 scope，主体 592.799 秒，其中实际浏览器流程 134.577 秒。真实平台 main、双库、业务 API、通知、Nginx、生产宿主与 Vue，未使用授权 HTTP fixture |

表中计数采用各测试运行器报告，可能包含总测试与子测试。它们不合并为“独立业务场景总数”。前端三组共 22 项是不同范围的单元／构建验证，不能代替平台和数据库授权验收。

## 核心行为与已验证边界

### 身份、租户与组织

- 每个业务请求以工单 audience 的短期应用 Bearer 进行机密客户端内省，再查询当前应用上下文。错误 audience、平台 session、无效 token、身份／租户／组织不匹配或授权依赖异常均拒绝；不缓存正向授权。
- SQL 范围始终包含当前租户和 `deleted_at IS NULL`。SELF 按 `owner_user_id`，ORG 按当前组织，ORG_SUBTREE 按平台返回的有效组织集合，TENANT 仍限定当前租户。SELF 可访问本租户本人先前组织的工单；处理人身份不扩大 SELF。
- 功能权限与独立数据授权分别检查。继承的祖先角色可以带来操作权限，但不能提供当前直接 membership 上缺失的 data grant。更新、删除、导出和通知各自需要对应 action。
- 租户、组织、编号、创建人、姓名和时间由服务端写入，严格 DTO 拒绝覆盖或搬移。业务数据库 trigger 也拒绝修改已有工单身份。
- 成员辅助 API 只返回允许组织的有效直接成员并检查该组织的应用资格。SELF 用户可保留本人旧组织工单的原处理人，改变旧组织处理人需获得目标组织范围或切换至该组织；目录支持搜索和分页，不限于首批 50 人。

### 写入与保留

- 创建为 OPEN；允许 OPEN→IN_PROGRESS→RESOLVED 和 RESOLVED→IN_PROGRESS 重开，非法跳转拒绝且不写入。
- 更新和删除使用行锁及版本号；过期版本 409。真实 PostgreSQL 并发门禁证明只有一份当前版本写入成功。
- 等待实际行锁及最终 UPDATE／DELETE SQL 期间上下文到期时，提交前再检查并回滚。取消或到期不留下部分字段、版本或删除标记。
- 删除仅设置软删除标记。列表、详情、修改、提醒和导出排除已删除记录，底层记录仍保留；没有自动清理业务、通知或审计数据的任务。

### CSV 与提醒

- CSV 使用独立 `ticket:export` 权限及 `ticket/export` grant。真实数据库测试验证组织／创建人过滤、Unicode、引号与换行，并中和电子表格公式前导字符；超过 1000 条或 4 MiB 明确拒绝，不返回截断导出。
- opaque iframe 通过 `downloadCsv` 把内容交宿主。宿主验证当前有效上下文、对应 export 权限和 grant、nonce／epoch、单调 request ID、安全 CSV 文件名、UTF-8 4 MiB 上限及下载频率。`allow-same-origin` 和 `allow-downloads` 没有加入沙箱。
- 提醒是独立操作，不再保存工单；先检查工单 read 权限与范围，再检查通知发布权限与 data grant，平台发布接口继续验证接收资格。
- 未分派工单提醒创建人，已分派工单提醒处理人。编号、内容和链接固定，同一未知结果按原事件键重试；处理人变化后同键不同目标 409，用户显式发起新的提醒。

### 前端上下文与独立发布

- 请求捕获当前应用 token、上下文和 generation；组织切换、更新、到期或卸载取消旧请求。忽略 AbortSignal 的迟到响应也不能进入新上下文。
- 401 清空服务端视图及删除确认；列表 403 清空缓存并暂停读取，详情 403／404 清理该记录。写入 403 保留用户输入并阻止重发，等待新 props；目录 403 清空候选且只暂停处理人修改。
- 409 保留草稿并加载当前服务器修订，要求用户明确选择后再保存；续期期间不会继续显示过期的冲突快照。普通网络失败、400 或 409 不当作认证失效。
- 管理工具只创建指定应用、客户端关联和缺失菜单；新应用保持 IFRAME、空授权。显式 grant 草案默认 ROLLBACK，不默认赋权，不重建 membership，不自动执行全量替换授权 API。
- 前端独立构建并打包为不可变版本，登记、预检、启用和回滚分步执行；后端版本和应用授权不随前端发布改变。最终联合门禁证明 v1→v2→回滚期间 100 个宿主文件哈希、document／session 标识与宿主 JavaScript 请求计数保持不变。

## 真实浏览器诊断与视觉复核

2026-10-07 的 browser-only 完整诊断使用真实平台 main、两座 PostgreSQL 数据库、业务 API、通知服务、Nginx、生产宿主和 Vue iframe，已通过以下流程：

- 创建工单；制造实际版本冲突后，原草稿保留、查看最新修订并显式重存，正常完成状态流转。
- 手动提醒在发布已实际提交但响应丢失时，保留事件键重试；数据库只有原 publication，Socket.IO 投递、宿主收件箱和已读更新正常。
- 通过宿主 Blob 下载真实 CSV，保留 UTF-8 BOM，导出只含当前授权与筛选结果；软删除后业务行保留但不再显示。
- 主题和窄屏页面正常显示；编辑弹窗头部、底部保持可用，正文可滚动。根代理与前端代理已复核实际截图。
- 组织切换阻止旧请求响应进入新视图，旧组织 token 直接请求返回 401。
- 实际 v1→v2→回滚，宿主 JavaScript 没有重新加载，document／session 页面标识保持，应用授权与业务数据继续有效。

该诊断的自有数据库、连接、服务及浏览器 PID、活跃临时目录均已清理至 0。最终正常运行再次采集实际浏览器截图，根代理及验收代理复核冲突编辑与深色窄屏布局，确认编辑操作、滚动区域和按钮可用。视觉 QA 审查产物另保留在精确的 Git 忽略目录：自动批准审查拒绝了删除这些 QA 目录，因而没有继续删除或把它们计为仍运行的服务资源；保留的是非秘密截图、元数据与归属记录，没有秘密配置或凭据。

## 完整联合门禁验收清单

下列项目由最终一次正常完整运行证明，未使用 `WORK_ORDERS_E2E_SCENARIO=browser` 等调试模式替代。实际运行使用 Linux Node 24.3.0、Nginx 1.28、Chrome 151.0.7922.34、Playwright 1.62.1，以及两座 PostgreSQL 17.6 随机隔离库。

| 场景 | 最终状态 | 验证方式 |
| --- | --- | --- |
| 实际平台迁移、登录、CLI bootstrap／重复执行和 inspect | PASS | 最新 backend main + 自有平台库 + 真实平台管理 API；平台迁移 1～22 全部成功且不存在 `work_orders` 表 |
| 两租户及同一全局用户的租户隔离、祖先角色与 direct grant 分离 | PASS | 实际 token、业务接口与持久行核对；业务库只执行自身 0001 迁移，checksum 与源码一致且不存在平台 `users` 表 |
| 四范围真实 CRUD、CSV、处理人、软删除及版本冲突 | PASS | 真实业务 API 与独立业务库，无授权 HTTP fixture |
| 同 token 功能／数据／应用／成员撤销与组织切换 | PASS | 实际平台变更后直接请求业务 API，旧 token 拒绝；并发写入角色另经真实平台获取有效 token |
| 持久提醒、Socket.IO、收件箱、去重和已读 | PASS | 平台 PostgreSQL、实际通知服务和宿主浏览器；已提交但响应丢失后同键重试不新增 publication |
| 生产宿主与 Vue iframe 的创建、编辑、状态、删除及 CSV 下载 | PASS | 实际 Chromium；CSV 保存事件来自宿主 Blob URL，UTF-8 BOM 与筛选范围正确 |
| v1→v2→回滚 v1，宿主文件与页面标识不变 | PASS | 真实登记／预检／启用／回滚 API，100 个固定宿主文件逐文件哈希、document／session 标识及宿主 JS 请求计数一致 |
| 自有资源清理与敏感内容检查 | PASS | 独立清理审计 exit 0：自有数据库、数据库连接、服务／浏览器 PID、活跃临时目录均为 0；秘密日志扫描通过，已审查 QA 产物保留边界见上节 |

## Self-Check

对应任务台账 Phase 11 四项 Acceptance Criteria：

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 同一试点由两租户使用，双方数据互不可见；组织数据范围与已确认规则一致。 | PASS | 真实平台及独立业务 PostgreSQL、四范围、同一全局用户及两租户负向矩阵均通过 |
| 隐藏按钮对应 API 直接调用、篡改资源 ID、旧组织 token 均被服务端拒绝。 | PASS | 实际 token 的功能／数据／应用／成员撤销、组织切换、直接 API 和越界资源 ID 均拒绝；迟到响应不进入新视图 |
| 应用独立发布／回滚不要求重建宿主；通知在授权范围内入库并展示。 | PASS | 实际 Vue v1→v2→回滚时 100 个宿主文件哈希及页面标识不变；提醒真实写库、Socket.IO 展示、同键重试与已读通过 |
| 业务方确认试点操作流程、字段和数据保留口径。 | PASS | 用户已确认工单提案、SELF 按创建人、状态重开、软删除保留和手动提醒 |

## 清理与验收边界

业务 API 的六座随机测试数据库均已删除，剩余业务连接为 0；平台成员辅助测试自有数据库已清理。最终联合运行另创建两座随机自有数据库，独立审计 exit 0，确认自有数据库 0、数据库连接 0、服务及浏览器 PID 0、活跃临时目录 0；各门禁的数据库计数不混用。Git 忽略的非秘密 QA 审查产物按已说明的审批拒删结果保留。

测试运行时、字体、浏览器缓存、截图和临时产物只放在 Git 忽略或本次自有临时目录，不加入业务构建。敏感环境文件、数据库连接串、管理员 session、应用 token 和客户端 secret 不写入验收报告或仓库。无关 `outputs/` 不纳入交付。

当前机器没有可用 Docker daemon。两网关真实进程与 Compose 配置通过，不代表 Docker 镜像、容器或现网部署已经执行。用户已确认 [Phase 12 本机隔离 Linux 原生准生产方案](./delivery-acceptance-proposal.md)：实际 HTTPS、故障告警、加密备份与新库恢复，RPO 24 小时／RTO 1 小时作为演练目标，不承诺商业 SLA，数据不自动清理。它属于后续阶段实际门禁。

WSL／VPN 较大 TCP 报文问题使用测试进程级 `TCP_MAXSEG=1000` 兼容 helper，未修改系统网络、业务 SQL、API 数据或安全断言；该设置不作为生产配置。完整 E2E 保留现有内省每分钟 30 次限流，按约 2.1 秒节奏请求并真实续签应用 token。容量和可配置内省配额在 Phase 12 验证。
