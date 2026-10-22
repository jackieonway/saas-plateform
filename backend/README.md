# 平台后端

Rust 1.97.0 / Axum / Tokio / SQLx 服务，默认监听 3001，使用平台专属 PostgreSQL。前端位于同仓库 frontend，业务应用在独立仓库。

在本 backend 目录执行：

```sh
cargo check --locked --all-targets
cargo test --locked --all-targets
cargo build --release --locked
cargo run --locked
```

启动前准备数据库、身份与运行配置。开发可读取当前目录 .env；生产从进程环境或绝对路径 APP_ENV_FILE 读取，不自动搜索配置。完整步骤见 [开发指南](docs/development.md)。

本目录是 API 镜像构建上下文，compose.yml 是平台运行入口：

```sh
docker build -t saas-platform-api:local .
```

真实部署需要镜像摘要、私有环境文件、网关、证书和数据库配置，见 [部署指南](docs/deployment.md)。

已有宿主机 Nginx 时，使用独立 compose.host-nginx.yml 和 [接入指南](docs/existing-nginx.md)，不与默认 compose.yml 合并。

| 目录 | 内容 |
| --- | --- |
| src、migrations | 平台实现和增量迁移；main.rs 只调用 bootstrap.rs 启动流程 |
| crates | 内部指标、部署契约 |
| tools | 独立管理与应用交付工具 |
| nginx | HTTPS 网关配置和镜像 |
| ops | 独立监控与恢复工程 |
| tests | Rust、固定协议材料与网关校验；前端和浏览器测试位于 frontend/tests |
| docs | 当前架构、开发、部署与接入文档 |

全部文档从 [文档导航](docs/project-README.md) 进入。根目录没有 npm 调度工程，各仓库自行构建。
