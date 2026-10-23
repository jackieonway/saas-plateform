# 平台 API

`saas-platform-backend` 是独立 Rust 包，使用 Axum 0.8、Tokio、SQLx 和 PostgreSQL。默认监听 `0.0.0.0:3001`；`main.rs` 进入 `bootstrap.rs`，启动时依次校验配置、连接数据库、应用内嵌迁移、初始化平台和区域数据，再开放 HTTP。

完整操作从[文档导航](docs/README.md)进入；首次运行见[开发指南](docs/development.md)，微应用行为见[架构](docs/architecture.md)和[接入指南](docs/micro-application-onboarding.md)。

## 构建与检查

以下命令在本目录执行，工具链由 `rust-toolchain.toml` 固定：

```sh
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo doc --locked --no-deps
cargo run --locked
```

本包不是包含所有工具的 Cargo workspace。`crates/` 内的依赖库以及 `tools/`、`ops/` 的独立包需要按各自 manifest 测试。数据库集成测试默认 `#[ignore]`，普通 `cargo test` 不代表这些测试已运行，执行方式见[验证说明](docs/engineering-baseline.md)。

## 配置与接口

进程环境优先；`APP_ENV_FILE` 可指定绝对路径环境文件。开发模式可读取本目录 `.env`，生产不自动查找环境文件。`DATABASE_URL` 指向平台专属库；`AUTO_CREATE_DATABASE` 默认关闭。空生产库管理员通过成对的 `BOOTSTRAP_ADMIN_USERNAME`、`BOOTSTRAP_ADMIN_PASSWORD` 初始化，开发库使用独立的演示 seed，见开发指南。

| 入口 | 职责 |
| --- | --- |
| `/api/health` | 检查 HTTP 服务及数据库，数据库不可用时返回 503 |
| `/api/auth/*` | 登录、会话和当前用户上下文 |
| `/api/applications/*` | 应用配置、接入检查、版本和环境入口 |
| `/api/apps/{appCode}/*` | 面向业务后端的应用授权、数据范围、成员与通知 |
| `/oauth/*` | OAuth 发放、内省、撤销及设备流程 |
| `/micro-proxy/*` | 已登记 URL 微应用的匿名静态资源代理 |

业务 API 不在此进程中；平台应用 token 必须经业务后端逐请求校验。代理也不转发业务数据或 OAuth 会话。

## 文件边界

`src/routes/` 是 HTTP 适配，`src/public_contract.rs` 定义公共 DTO；数据库迁移在 `migrations/` 编译时嵌入。`tests/fixtures/micro-app-protocol/` 保存独立 SDK 协议校验材料，浏览器 SDK 副本在前端工程。发布、ZIP 分发、镜像部署等模块是平台控制面，不是跨仓库构建工具。

本目录是平台 API 镜像构建上下文：`docker build -t saas-platform-api:local .`。生产需按[部署指南](docs/deployment.md)准备镜像摘要、环境文件、网关和证书。
