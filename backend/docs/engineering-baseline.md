# 工程基线与验证入口

本页列出当前仓库可重复执行的检查。命令存在、历史通过、单元测试通过与目标环境部署成功是不同证据，交付时记录实际执行命令、退出状态、跳过项和环境。

## 工具与依赖

平台使用 Rust `1.97.0`、Node `24.22.1`、npm `11.16.0`，分别由后端 `rust-toolchain.toml`、前端 `.nvmrc` 和 `package.json` 声明。Rust 提交 `Cargo.lock`，npm 使用 `package-lock.json` 和 `npm ci`。宿主依赖 qiankun `2.10.16`，固定 SDK 2.0.0 由本仓库 vendored 文件与来源摘要校验。

本次 Windows 环境实际执行 Node `24.22.1`、npm `11.19.1`；仓库仍声明 npm `11.16.0` 为包管理器基线。七份最终微应用候选的隔离浏览器补验使用已有 WSL Docker 镜像，其 Node 为 `24.18.0`、Chromium 为 `156.0.8078.4`；该 Node 仅启动临时静态测试服务，未安装依赖、编译平台或重建候选。此项只读产物验收不能作为当前平台 Node 构建基线的验证结果。

前端、后端、运维和管理工具分别构建，没有根统一构建命令。不要通过删除锁文件或降级校验绕过失败。

## 前端检查

在平台 `frontend` 执行：

```sh
npm ci --ignore-scripts --no-audit --no-fund
npm run sdk:check
npm run lint
npm run typecheck
npm test
npm run build
```

`npm test` 包含 qiankun 生命周期、宿主服务、上下文更新、应用配置、URL 入口、版本发布、会话、授权和静态资源契约测试。真实浏览器检查是独立入口，例如：

```sh
npm run test:micro:qiankun:browser
npm run test:applications:browser
```

浏览器测试依赖已准备的运行环境与测试身份，按[浏览器验收说明](../../frontend/tests/browser/README.md)配置。它们不由普通 `npm test` 自动执行。

## 后端检查

在平台 `backend` 执行：

```sh
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo test --locked --manifest-path crates/deployment-contract/Cargo.toml
node --test tests/nginx-render-policy.mjs
```

数据库集成用例被标记为 ignored，默认测试不会执行。CI 使用专属 PostgreSQL 17 和 `TEST_DATABASE_URL` 显式运行，包括历史迁移升级、qiankun 准入、环境 URL、发布清理围栏、OAuth、数据授权、组织与会话专项。只对自有隔离测试数据库执行这些命令。

例如明确验证迁移与 qiankun 准入时，在已配置专属 `TEST_DATABASE_URL` 的环境执行：

```sh
cargo test --locked --bin saas-platform-backend release_tests::database_upgrade:: -- --ignored --test-threads=1
cargo test --locked --bin saas-platform-backend release_tests::qiankun_only:: -- --ignored --test-threads=1
cargo test --locked --bin saas-platform-backend release_tests::url_entries:: -- --ignored --test-threads=1
```

## 独立工具与仓库边界

在平台仓库根执行静态结构与 SDK 契约检查：

```sh
node --test .github/tests/repository-structure.mjs .github/tests/sdk-consumer-contract.mjs
```

`backend/ops/monitoring`、`backend/ops/recovery`、`backend/tools/platform-admin`、`backend/tools/artifact-agent`、`backend/tools/deployment-agent` 都有各自的 `Cargo.toml` 和锁文件。分别在各目录执行：

```sh
cargo test --locked
cargo build --release --locked
```

当前 `.github/workflows/ci.yml` 的 operations 矩阵只含 monitoring/recovery，平台主 Cargo 命令不会自动覆盖 tools 三个独立工程。交付相应工具时要额外运行它自己的检查。

## CI 覆盖与上线验收

当前 CI 只检出平台仓库：contracts 执行结构、SDK 和网关策略；frontend 执行 SDK、静态、类型、单元与构建；backend 执行格式、编译、Clippy 与测试；backend-integration 执行显式数据库专项；operations 检查两个运维工程；images 构建宿主、平台 API、monitor 镜像。它不推送 registry、不部署业务、不执行线上切流。

上线必须另验真实镜像、网络、证书、数据库、最终 CSP、qiankun 加载与卸载、API 权限、直接成员和组织上下文、静态资源 404、旧版本保留及恢复材料。网关样例与宿主默认 CSP 已启用 qiankun 所需的可信求值，但需检查站点或其他代理是否叠加了更严格策略；默认 Compose 不会启用 ZIP 卷、节点分发或受管镜像路由。部署细节见[部署指南](deployment.md)。
