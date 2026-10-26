# 平台 API

Rust/Axum 平台 API。账号认证由独立身份服务提供，平台拥有租户、组织、成员、授权、应用、发布、审计与通知业务。

## 数据库与启动

当前唯一结构基线为 `migrations/0001_init.sql`，目标为 PostgreSQL 17+ 的空数据库。应用生成 UUIDv7；SQLx 历史和 checksum 必须匹配，不能接管旧库或重写历史。

在本目录设置明确的 `DATABASE_URL` 后执行：

```sh
cargo run --locked --bin platform-migrate
```

该命令只初始化结构，不启动服务、不导入行、不创建用户或演示业务数据。独立身份部署由管理作业完成初始化与 owner/ACL 交接；平台 runtime 设置 `IDENTITY_REMOTE_ENABLED=true`，只做 readiness，不代为迁移和 seed。

[全新安装](docs/fresh-installation.md)说明四个库的边界和初始化顺序。[开发指南](docs/development.md)说明 SSO、RP、TLS、配置和运行命令。受控初次 bootstrap 使用明确管理员配置；不得将默认演示账号或历史客户端用于正式部署。

## 开发与检查

```sh
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo run --locked
```

默认 API 监听 3001。SSO/BFF 配置缺失时启动失败，旧平台密码登录、ticket 和公开平台 refresh 不存在。数据库集成测试必须使用显式的任务自有 PostgreSQL；具体命令和环境变量见各测试模块。

## 工程导航

- [当前文档](docs/README.md)、[架构](docs/architecture.md)、[部署](docs/deployment.md)
- [SSO](docs/sso-only.md)、[身份角色交接](ops/identity-deployment/README.md)
- [应用授权](docs/application-authorization-contract.md)、[微应用接入](docs/micro-application-onboarding.md)
- [管理 CLI](tools/platform-admin/README.md)、[网关](nginx/README.md)、[运维](ops/README.md)

工单、门店、客户门户和通知服务独立部署，构建本项目不读取相邻仓库的业务实现。SDK 固定发布物按其自己的完整性合同校验。
