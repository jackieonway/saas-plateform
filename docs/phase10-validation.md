# Phase 10 持久通知接入验收

## 验收结论

Phase 10 的实现及四项台账验收标准全部通过。持久通知发布、归属隔离、幂等、实时服务、宿主恢复和接入配置已完成；本记录作为阶段完成与提交的依据。

业务规则、API 和部署说明分别见[通知接入契约](./notification-integration.md)、[运维说明](./notification-operations.md)及[设计记录](./notification-integration-plan.md)。设计记录用于追踪改造方案，实际接口以接入契约为准。

## 已完成的门禁

| 门禁 | 结果 | 证据与范围 |
| --- | --- | --- |
| Rust 普通编译与默认单元测试 | PASS，4/4；0.24 秒 | Windows `cargo check`、WSL `cargo build`；默认测试不把 ignored 集成测试计为通过 |
| 真实 PostgreSQL 通知回归 | PASS，7/7；36.89 秒 | `backend/src/notification_tests.rs`；完整实际迁移、真实数据库与进程内 Axum API，覆盖发布授权、范围、幂等、事务、旧数据、收件箱、socket-context、SYSTEM 调用点及种子 |
| 既有发布回归 | PASS，8/8；24.64 秒 | `backend/src/release_tests.rs`；真实 PostgreSQL、HTTP 入口与资源；确认应用 token 新绑定字段未破坏独立发布、信任模式及授权保持 |
| 实际平台联合通知验收 | PASS，9/9；64.24 秒 | `mini-services/notification-ws/tests/platform-e2e.mjs`；8 个场景及总测试，真实 backend main、完整迁移/种子、PostgreSQL、平台登录/组织选择/应用 token API 与实际 Socket.IO；0 跳过 |
| 实时服务协议及配置 | PASS，71/71 | `mini-services/notification-ws/tests/config.mjs`、`tests/service.mjs`；实际 Socket.IO 客户端，平台 HTTP 与数据库接口使用 fixture |
| PostgreSQL LISTEN 与恢复 | PASS，5/5 | `mini-services/notification-ws/tests/postgres.mjs`；真实 PostgreSQL 最小 SQL fixture，验证事务、NOTIFY、归属及 LISTEN 断开/恢复；认证使用 HTTP fixture |
| 宿主通知上下文 | PASS，9/9 | `tests/notification-context.mjs`；捕获 bearer/epoch/signal、同步取消、协议和链接验证 |
| React / Socket.IO 浏览器流程 | PASS，10/10 | `tests/notification-frontend-browser.mjs`；真实 Vite、React、Chromium 与 Socket.IO；收件箱及认证响应使用隔离 fixture |
| Vite 开发代理 | PASS，9/9 | `mini-services/notification-ws/tests/vite-proxy.mjs`；真实 Vite 配置，Socket.IO 传输 fixture；含路径、Origin、polling、WebSocket、升级与 HMR |
| Caddy 网关 | PASS，23/23 | `tests/micro-release-gateways.mjs`；配置校验与实际 HTTP / Socket.IO 传输，微应用资源和通知代理边界 |
| Nginx 网关 | PASS，23/23 | 同一网关契约，实际配置与传输验证；WSL Nginx |
| Compose 配置 | PASS | 官方 Compose `config --quiet`，通知服务、环境配置、依赖及网关编排 |
| 前端完整 TypeScript 检查 | PASS | 完整 `tsc -b frontend/tsconfig.json` |
| 宿主生产构建 | PASS | 项目 frontend 的 Vite production build |

门禁的测试数采用各运行器报告，包含其总测试/子测试计数，不将它们合并为一个独立场景总数。

## 核心行为证据

- 应用发布要求有效应用 audience、当前 `notification:publish` 操作权限与直接成员关系上的 `notification/publish` 数据 grant；角色继承不扩大数据范围，超管也不自动取得数据 grant。
- SELF、ORG、ORG_SUBTREE、TENANT 的正负路径通过；另一租户、停用/到期成员上下文及无应用授权的目标被拒绝。组织广播只取直接成员，同一用户的多组织投递分别归属。
- 幂等按租户、发布组织、应用、发布用户及键隔离；同规范化内容返回首次 ID、时间和计数，不同内容返回 409。并发同键只写一次，删除收件行后重试不重建，撤销发布权限后的重试仍被拒绝。
- 通知或审计 INSERT 故障使发布整体回滚；SYSTEM 通知故障使账号创建、密码/version/session 变更、角色变更或设备批准随同业务事务回滚，移除故障后可重试。
- 0022 保留旧通知正文、时间及已读状态，将无归属记录隔离为 LEGACY；不根据用户当前的零个、一个或多个成员关系推断租户。新收件箱、未读数、读删操作及实时投递不读取 LEGACY。
- APP 历史通知按当前应用启用状态与租户/组织应用授权动态过滤；撤销访问权后隐藏历史消息，保留其数据库记录。SYSTEM 通知仍限定当前用户、租户与组织。
- 实时服务不持有 JWT 签名密钥，不读取开发机密钥文件。平台 socket-context 验证 session；每次实际投递前验证通知归属与当前 APP 权限，定时再验清理空闲失效连接。
- WS/LISTEN 中断期间平台仍持久保存通知；恢复认证后宿主补拉数据库列表及未读数，历史补拉不批量弹 Toast。数据记录、即时提示与已读状态分别处理。
- 实际平台联合验收通过自有测试数据库、子进程和监听连接清理；公开事件及内存日志的凭据扫描通过。

## 安全与并发复核修正

| 修正 | 验证 |
| --- | --- |
| TENANT 目标不能吞掉 `userId/organizationId/tenantId` 等混入字段 | 严格 DTO 负向测试返回 400，无发布记录 |
| namespace 锁等待后重新检查 token、session、权限、grant 与目标范围 | 控制实际 PostgreSQL advisory lock，确认请求进入等待后撤销 token，释放后返回 401 且不写库 |
| 到期检查使用当前时刻，不依赖事务开始时的 `now()` | 锁等待中应用数据库 token 与管理员 session JWT 自然到期均拒绝；socket-context 对刚过期 JWT 也严格拒绝 |
| 64 KiB 限制覆盖原始请求体 | 合法 JSON 加大空白 padding 被拒绝；验证没有只依靠 content 字段长度限制 |
| 链接规范化保留合法中文与 ASCII reserved escapes | 200 字中文标题、8192 字中文正文、100 字中文路径通过；外部/协议相对、反斜线、控制字符、应用前缀近似及多层编码越界链接拒绝 |
| UI 上下文切换同步取消旧请求与 socket，重试保留幂等键与正文 | 上下文与真实 React 浏览器门禁通过；迟到响应/事件不进入新上下文 |
| 修正通知界面的重复 React Portal key | 浏览器发送、重试及切换流程通过 |

## Self-Check

对应 `saas-microfrontend-task-ledger.md` 的 Phase 10 四项 Acceptance Criteria：

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| `onNotify` 只弹 Toast；持久发布接口写库、可标记已读、可审计。 | PASS | Toast 与发布分开；真实平台与 PostgreSQL 验证发布、审计、读取和已读；事务故障无部分成功 |
| 另一租户目标被拒绝；同一去重键的重试结果符合已批准策略。 | PASS | 四级范围与五维 namespace 矩阵；并发、异正文 409、删除后重试及撤授权重试验证 |
| 通知服务在生产配置下可健康启动，断线重连后未读状态与数据库一致。 | PASS | 显式部署配置下的真实 Node 服务、平台 main 和 PostgreSQL 验证启动、健康、依赖故障、WS/LISTEN 恢复与数据库补拉；容器执行边界见下节 |
| 客户端无法使用普通显示回调绕过服务端发布权限。 | PASS | `onNotify` 不调用持久接口；应用/用户发布路径都由服务端重新检查功能权限与独立 grant，缺失、错误受众、撤销和过期均拒绝 |

## 清理与验收边界

Rust 通知与发布矩阵共创建 **14 个随机 PostgreSQL 数据库**，逐库确认 `assertions_ok=true`、`database_removed=true`、`pool_closed=true`。实时服务的真实 PostgreSQL和实际平台 E2E 自有数据库、监听连接、临时目录与子进程另行完成清理，不把这些资源混入上述 14 个数据库计数。

71 项服务协议测试、5 项最小 PostgreSQL fixture、10 项 React 浏览器测试及传输代理测试分别证明各组件行为。只有实际平台联合门禁使用真实 backend main、完整迁移和真实登录/token API；其 9 项结果独立报告，fixture 不替代平台授权和持久数据验收。

通知服务已验证并支持标准 **Node 24**，实际平台联合验收使用 Linux Node 24.3.0。Bun 未通过本阶段验证，不承诺兼容性。

当前机器没有 Docker daemon。Compose 语法、服务进程、网关及生产构建均通过；没有执行 Docker 镜像构建、容器启动或生产集群部署。容器部署、正式 SLA、容量与生产网络演练继续属于交付阶段。

WSL/VPN 链路的大报文问题采用 `TCP_MAXSEG=1000` 的局部测试进程兼容设置。该设置仅作用于 Git 忽略的测试工具与对应进程，未改变系统网络、业务 SQL、API 内容或安全断言；不作为生产配置或运行时兼容承诺。

本阶段验收不读取生产凭据，不在报告中保存数据库连接串、session/app token 或客户端密钥。
