# Dante Cloud

基于 Rust、PostgreSQL、React 的多租户、多组织权限与微应用平台。

系统架构、数据库、身份权限、前后端接入、发布、通知、部署及恢复统一维护在 [系统架构与应用接入](docs/architecture.md)。本文保留仓库入口和快速启动说明，组件目录 README 提供各自的命令与参数。

## 系统组成

| 单元 | 技术与职责 |
| --- | --- |
| 平台 API | Rust / Axum / SQLx；身份、租户组织、授权、OAuth、通知持久化与应用发布 |
| 宿主前端 | React 19 / TypeScript / Vite；后台管理与微应用编排 |
| 通知服务 | Node 24 / Socket.IO；持久消息的实时提示与会话复验 |
| 独立业务应用 | work-orders：Vue 3 前端、Node 24 API、独立 PostgreSQL |
| 网关 | Caddy / Nginx；HTTPS、静态版本、API 与 WebSocket |

默认 HTML 微应用通过 opaque iframe 和 SDK 接入，静态版本独立发布；第一方 ESM 模块包含在宿主构建中。业务服务使用应用 token，通过平台内省与实时授权上下文验证身份及权限。

## 文档导航

- [系统总览与模块边界](docs/architecture.md#overview)
- [数据模型与迁移](docs/architecture.md#data)
- [租户、组织与数据授权](docs/architecture.md#authorization)
- [登录、会话与应用 token](docs/architecture.md#identity)
- [后台验证 token 与获取用户信息](docs/architecture.md#backend-integration)
- [微应用前端接入](docs/architecture.md#microfrontend)及[各框架开发示例](docs/architecture.md#frontend-examples)
- [构建、上传、激活与回滚](docs/architecture.md#releases)
- [通知协议](docs/architecture.md#notifications)
- [生产配置与部署](docs/architecture.md#deployment)
- [监控](docs/architecture.md#observability)、[备份恢复](docs/architecture.md#recovery)和[验收边界](docs/architecture.md#validation)

## 仓库结构

```text
backend/                    Rust 平台 API 与增量迁移
frontend/                   React 宿主与内置 ESM 模块
business-apps/work-orders/  独立业务前端、后端和部署工具
mini-services/             通知服务、OAuth 和微应用演示
examples/                  前端 SDK 与后端授权参考
scripts/                   版本打包与开发辅助工具
deploy/                    网关、原生部署、监控和恢复工具
docs/architecture.md        统一架构与应用接入文档
docs/sql/                  可执行的只读关系审计 SQL
tests/                     契约、数据库、浏览器和网关测试
```

## 本地开发

准备 PostgreSQL 17、仓库要求的 Rust 工具链、Node 24 和 npm。配置开发数据库与独立随机密钥，再分别启动后端和前端。

在 backend 目录设置环境或本地 .env，示例中的密码和密钥须自行替换：

```dotenv
DATABASE_URL=postgresql://dante:YOUR_PASSWORD@127.0.0.1:5432/dante_cloud
JWT_SECRET=YOUR_RANDOM_SECRET
BIND_ADDR=0.0.0.0:3001
AUTO_CREATE_DATABASE=false
```

```powershell
# 在 backend 目录。
cargo run --locked
```

```powershell
# 在 frontend 目录。
npm ci
npm run dev
```

宿主默认 http://localhost:3000，平台就绪 http://localhost:3001/api/health。宿主代理可通过 BACKEND_PROXY_TARGET 指定。数据库已存在时平台执行增量迁移；需要受控首次建库时显式启用 AUTO_CREATE_DATABASE，账号必须具有相应权限。

配置优先级为系统/服务环境优先；生产不自动加载 .env，使用绝对 DANTE_ENV_FILE 或服务管理器明确配置。开发空库可能初始化演示数据，生产使用独立初始化参数和安全门禁，详见 [运行部署](docs/architecture.md#deployment)。

实时服务和独立业务服务另行启动：

- [通知服务启动与测试](mini-services/notification-ws/README.md)
- [工单前端构建](business-apps/work-orders/frontend/README.md)
- [工单后端启动与数据库](business-apps/work-orders/backend/README.md)
- [工单应用登记与授权](business-apps/work-orders/deploy/README.md)

## 发布与运维

当前 Compose 使用 production 配置及显式 secret，生产前端镜像采用 Caddyfile.production。平台、通知和网关默认编排，工单 API/本地业务库按 profile 选择。使用 [环境模板](deploy/compose.env.example)准备私有配置，并在目标环境验证实际镜像、容器与 HTTPS。

```sh
docker compose --env-file /secure/dante.env config --quiet
docker compose --env-file /secure/dante.env --profile work-orders up -d --build
```

原生部署、监控和恢复工具分别见 [deploy/native](deploy/native/README.md)、[deploy/monitoring](deploy/monitoring/README.md)和 [架构文档的恢复章节](docs/architecture.md#recovery)。历史本机原生验收不等于现网或 Docker 已部署。

## 维护与验证

```powershell
# 在 backend 目录。
cargo check --locked
cargo test --locked
```

```powershell
# 在 frontend 目录。
npm run build
npm run test:micro
```

实际数据库和浏览器检查有独立环境要求，入口与证据边界见 [验证章节](docs/architecture.md#validation)。跳过不等于通过，不在生产数据库运行隔离测试。

新增持久化能力必须新增迁移，不修改已执行 SQL 校验和。架构变更同步更新统一文档，组件 README 保留各自执行说明，避免重新建立重复设计文档。
