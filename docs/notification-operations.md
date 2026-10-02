# 持久通知的实时服务部署与排查

持久发布由平台完成事务、归属和幂等校验。Socket.IO 只传递已落库通知的新消息提示；未读、列表和已读状态始终取平台 API。`onNotify` 保持纯 Toast，不进入通知表。内部与公开 DTO、授权矩阵见[通知接入设计](./notification-integration-plan.md)，服务配置见[notification-ws](../mini-services/notification-ws/README.md)。

## Compose 配置

仓库 Compose 增加 `notification-ws`，使用本服务 Dockerfile 和标准 Node 24。依赖 PostgreSQL 健康与 backend 启动，仅暴露容器内 3003。web 网关按 `/socket.io/` 代理 polling 和 WebSocket，不再依赖 `XTransformPort`。

在根目录 Git 忽略的 `.env.docker` 配置公开站点 origin：

```dotenv
PUBLIC_APP_URL=https://saas.example.com
NOTIFICATION_ALLOWED_ORIGINS=https://saas.example.com
# 可选：使用有 CONNECT 和 SELECT notifications 权限的独立读取账号。
NOTIFICATION_DATABASE_URL=postgresql://notification_ws:REPLACE_PASSWORD@postgres:5432/dante_cloud
```

使用 `docker compose --env-file .env.docker up -d --build`。通知容器的 `PLATFORM_API_BASE_URL` 固定使用内部 `http://backend:3001`；不传 JWT 签名密钥，不读取 backend `.env`。允许来源为精确 HTTP(S) origin 列表，包括必要端口，不接受 `*`、`null`、路径或用户名密码。默认仅适配 localhost 80；开发 Vite 的 `http://localhost:3000` 和自定义 8080 端口都需明确加入。

生产需实际配置 HTTPS/TLS 入口。Caddy 默认 HTTP 配置不因 `PUBLIC_APP_URL` 自动改变监听：原生 Caddy TLS 需域名、移除 `auto_https off`、映射 443 并持久化 `/data` 和 `/config`；前置 TLS 入口也须支持 polling POST 和 WebSocket Upgrade。Nginx 示例需要相应证书与上游配置。

## 路径与认证

服务、宿主默认路径为 `/socket.io/`。Caddy 保留路径转发到 `notification-ws:3003`；Nginx 保留路径转发到本机 3003，显式使用 HTTP/1.1 和 Upgrade/Connection。Vite `/socket.io` 代理默认到 3003，可用 `NOTIFICATION_DEV_PROXY_TARGET` 覆盖。不要把此路由回退为宿主 HTML。

公开 `authenticated` 事件含 `contractVersion/userId/tenantId/organizationId/expiresAt`，不含 token 或 sessionId。内部 `GET /api/notifications/socket-context` 额外含 sessionId，由 WS 严格绑定并在初次、每次投递和周期再验时核对；客户端不能通过再次 authenticate 更换身份。

投递前内部请求带 `notificationId`，校验通知归属和 APP 的当前租户/组织应用授权及启用状态。通知特定 403/404 跳过该条，继续允许其它合法应用消息；401、到期或身份变化断开连接。APP 历史收件操作同样受当前应用访问权限制，SYSTEM 按当前 session 归属。成员退出、组织或租户停用、密码变化和 session 撤销由平台验证生效。

## 健康与恢复

| 检查 | 接口或信号 | 解释 |
| --- | --- | --- |
| 进程存活 | `/health/live` 200 | HTTP 服务还在运行 |
| 服务就绪 | `/health` 200/503 | DB schema、LISTEN、平台路由全部 UP 才就绪 |
| DB | `checks.database` | 新通知字段可查询；旧 schema 不会误报 UP |
| LISTEN | `checks.listen` | 独立连接已监听，并由自身 SELECT 1 探针确认 |
| 平台 | `checks.platform` | 无 Bearer 探针获得预期 401，确认保护路由可达 |

依赖故障用 DOWN/UNKNOWN 表达，不能只据 live 或数据库提交成功宣称客户端收到消息。服务重建 LISTEN 时使用有上限退避，不声称重放离线即时事件。宿主认证/重连后立即补拉当前上下文列表、未读数与偏好；离线历史不补弹 Toast，已读状态由原 API 改变。

数据库中的通知持久存在与在线提示成功分别衡量。通知行写入成功后不要因即时通道故障换幂等键重发。日志只包含安全状态和固定错误类别，排查时不复制 Bearer、数据库连接串或用户通知正文。

## 验证

服务协议与真实 Socket.IO 回归：

```powershell
cd mini-services/notification-ws
npm ci --ignore-scripts
node --test tests/config.mjs tests/service.mjs
node --env-file=../../.env.test --test tests/postgres.mjs
```

`tests/postgres.mjs` 只在随机自有测试库验证真实 PostgreSQL 和 LISTEN；平台认证仍是 HTTP fixture。网关测试位于 `tests/micro-release-gateways.mjs`，验证静态资源、polling、WebSocket 与原路径转发；其 Socket.IO 上游为明确的传输 fixture。Vite 代理测试使用实际前端配置，并同时检查宿主 HTML、HMR 和 Socket.IO 路径。

实际平台联合验收使用 `tests/platform-e2e.mjs`，需先编译包含迁移 0022 的后端 main，并显式指定其绝对路径：

```powershell
# 在 mini-services/notification-ws 目录执行；路径应对应本机编译产物。
$env:NOTIFICATION_E2E_BACKEND_BINARY = (Resolve-Path ../../backend/target/debug/dante-cloud-backend.exe).Path
node --env-file=../../.env.test --test tests/platform-e2e.mjs
```

Linux 使用该平台的 backend 二进制与 Node 24，设置相同环境变量后执行相同测试。`.env.test` 仅供隔离验收，包含 `NOTIFICATION_TEST_DATABASE_URL` 或 `TEST_DATABASE_URL`；测试账号需有创建自有数据库及终止自有 LISTEN 连接的权限。缺少数据库 URL 或 backend 路径时明确跳过，不把 fixture 或跳过计入实际平台通过。

联合测试启动真实 main，执行全部实际迁移和种子；两名非 super 用户通过真实 login/context/application-token API 取得凭据。覆盖发布与幂等、USER/ORGANIZATION/TENANT 目标、操作权限和数据授权分别缺失时零写入、四个 Socket.IO 上下文隔离、两层应用访问撤销及应用停用、WS/LISTEN 断线期间持久收件恢复、已读隔离、上下文切换、session 登出和成员退出。每次创建随机自有数据库，在自有临时目录放空 `.env` 防止 backend 覆盖隔离配置，完成后关闭自有进程和连接再删除自有数据库；日志和公共事件会检查凭据泄漏。

### 2026-10-02 验收记录

| 验证 | 结果 | 实际运行范围 |
| --- | --- | --- |
| `tests/config.mjs` + `tests/service.mjs` | 71 通过 | Node 24、真实 HTTP 与 Socket.IO，平台及 DB 接口 fixture |
| `tests/postgres.mjs` | 5 通过 | 隔离 PostgreSQL、真实事务与 LISTEN，平台 HTTP fixture |
| `tests/platform-e2e.mjs` | 9 通过，0 跳过，退出码 0 | 最新真实 backend main、完整迁移/种子、真实授权 API、PostgreSQL、Socket.IO；Linux Node 24.3.0，64.24 秒 |
| `tests/vite-proxy.mjs` | 9 通过 | 实际 Vite 配置、宿主 HTML/HMR 与真实 Socket.IO 传输 fixture |
| `tests/micro-release-gateways.mjs` | Caddy 23、Nginx 23 通过 | 实际 Caddy 与 Nginx，静态发布路径及真实 Socket.IO 传输 fixture |
| Compose 配置 | 通过 | Compose 2.39.4 `config --quiet` |

实际平台测试的 9 项包括 8 个场景和总测试；自有进程/数据库清理与日志/公开事件凭据扫描一并通过。本机没有 Docker daemon，镜像构建、Compose 容器启动和生产 TLS 配置需在具备条件的部署环境验证。
