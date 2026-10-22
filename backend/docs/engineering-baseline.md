# 工程与验证

## 工具和依赖

使用 Rust 1.97.0、Node 24.18.0、npm 11.16.0。Rust 工程提交 Cargo.lock，实际 npm 工程提交 package-lock.json。npm ci 安装锁定依赖，npm run build 构建前端，cargo build --release --locked 编译后端。

前后端和运维工具分别编译，没有根统一构建命令。固定 SDK 校验会检查来源摘要、发布清单和文件内容，不检出其他仓库源码。

## 平台检查入口

在 frontend 执行：

```sh
npm ci
npm run sdk:check
npm run lint
npm run typecheck
npm test
npm run build
```

在 backend 执行：

```sh
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo test --locked --manifest-path crates/deployment-contract/Cargo.toml
```

网关配置测试在 backend 执行 node --test tests/nginx-render-policy.mjs。监控、恢复及 tools 中的独立工程进入各自目录执行 Cargo 命令。前端测试位于 frontend/tests，浏览器工具见 [浏览器验收](../../frontend/tests/browser/README.md)。

仓库根执行 node --test .github/tests/repository-structure.mjs .github/tests/sdk-consumer-contract.mjs，验证目录边界和跨端固定 SDK 协议。

## CI 实际行为

.github/workflows/ci.yml 仅检出本仓库。contracts 任务运行仓库结构、跨端 SDK 与后端网关检查；其他任务分别执行前端、后端、monitoring/recovery 测试，并以 frontend、backend、backend/ops/monitoring 为上下文构建镜像。不推送 registry、不部署、不准备业务源码。

backend-integration 任务使用专属 PostgreSQL 17 服务和显式 TEST_DATABASE_URL，分开执行 OAuth 发放与撤销、OAuth 管理与 PKCE 迁移审计、实体与菜单/地区、通知、组织子树报表、角色分配并发、地区事务审计、历史发布门禁、数据授权变更、数据授权发现、平台会话刷新、微应用接入的 ignored 用例。实体专项包含平台管理入口不可删除、停用或过期的保护，发布专项包含清理围栏下的失败部署重试。每个用例创建并删除自己的唯一数据库；普通 backend 任务仍保持默认非数据库测试行为。该配置只使用临时 CI 测试凭据，不读取生产 secrets。

工单、门店、通知、SDK 和示例各自拥有工作流。应用上线前仍需在目标环境确认网络、数据库、证书与授权配置。

## 结果口径

单元测试、静态协议、真实数据库、浏览器和镜像运行是不同证据。ignored 不算通过，默认 cargo test 不会自动运行需要数据库的忽略项。只在自有隔离数据库内按测试说明显式执行必要项，不能对生产库运行测试。

已有结果集中记录在 [验证状态](repository-split-validation.md)，不从源码存在或历史通过推导当前生产已验证。
