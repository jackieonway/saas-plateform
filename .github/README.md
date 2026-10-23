# 平台持续集成

`workflows/ci.yml` 在 push、pull request 和手动触发时运行。工作流仅检出 `saas-plateform`，权限为 `contents: read`，checkout 不保留凭据。它检查单仓库可构建性，不拉取同工作区业务仓库，也不发布镜像或执行部署。

## 任务与门禁

| Job | 实际执行内容 |
| --- | --- |
| `contracts` | 仓库交付结构、SDK 2 固定发行物/跨端 Rust DTO、后端 Nginx 渲染策略 |
| `frontend` | Node 24.22.1、npm 11.16.0、锁定安装、SDK/qiankun 校验、ESLint、TypeScript、Node 测试、Vite 构建及产物门禁 |
| `backend` | Rust 1.97.0、格式检查、check、严格 Clippy、普通测试、独立 deployment-contract 测试 |
| `backend-integration` | PostgreSQL 17 服务下明确列出的 ignored 数据库回归 |
| `operations` | monitoring、recovery 两个独立 Rust 工具的锁定测试 |
| `images` | 分别以 frontend、backend、backend/ops/monitoring 为上下文构建三份镜像 |

`images` 传入 `SOURCE_REVISION=github.sha` 和本次 run/attempt 构建标识，但不会 push registry。任务没有定义 `needs` 依赖，会由 GitHub 并行调度；应以整个工作流全部必要任务的状态判断通过，不能只看镜像构建完成。

### 本地重放：前端与契约

在仓库根目录运行：

```powershell
node --test .github/tests/repository-structure.mjs .github/tests/sdk-consumer-contract.mjs backend/tests/nginx-render-policy.mjs
```

然后进入 `frontend`：

```powershell
npm ci --ignore-scripts --no-audit --no-fund
npm run sdk:check
npm run qiankun:check
npm run lint
npm run typecheck
npm test
npm run build
```

仓库结构测试检查 Git 跟踪文件和未被忽略的新增源码，要求实际交付的非隐藏顶层目录只有 `frontend` 和 `backend`。被忽略的本地 `local-*` 工具不参与交付检查；显式跟踪或新增非忽略的意外目录仍会失败。测试用临时 Git 仓库覆盖这些边界，不删除本机辅助目录。

SDK 消费者测试校验固定 SDK 2.0.0 发行物，并逐字比较 `backend/src/public_contract.rs` 与前端 SDK `protocol.rs`。这与 SDK 仓库的生成测试分工不同，不能通过修改哈希或现场重新生成 vendor 来掩盖发行物漂移。

qiankun 门禁在 `frontend` job 安装锁定依赖后运行，扫描活动前端源码、public、当前 SDK 和构建产物，拒绝旧桥接代码、SDK 1、宿主内置微应用副本及子应用 HTML 的 module 脚本。HTML 使用开发依赖 parse5 按标准解析真实 script 元素与属性实体，注释或脚本字符串不作为标签。宿主 index.html 自身仍是合法 Vite ESM 入口，历史 SDK releases 不在活动扫描范围。构建入口自动执行产物检查，因此 Docker 构建也经过同一门禁。

### 本地重放：Rust

在 `backend` 运行：

```powershell
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo test --locked --manifest-path crates/deployment-contract/Cargo.toml
```

monitoring、recovery 分别进入 `backend/ops/monitoring`、`backend/ops/recovery` 执行 `cargo test --locked`。CI 安装 Rust 1.97.0；本地应按各 crate 的 toolchain 配置使用相同工具链。

## PostgreSQL 回归

`backend-integration` 使用独立 runner、30 分钟任务期限和 `postgres:17` 服务。连接串由任务内 `TEST_DATABASE_URL` 明确提供，`pg_isready` 检查健康后执行测试；账号密码是该临时实例的测试值，不读取生产配置或仓库 secrets。

| 测试筛选 | 线程数 | 覆盖内容 |
| --- | --- | --- |
| `oauth_issuance_tests::` | 2 | 签发、刷新、撤销与并发消费 |
| `management_audit_tests::oauth_` | 1 | OAuth 管理、PKCE 迁移与审计回滚 |
| `entity_creation_tests::` | 2 | 实体生命周期与菜单关系 |
| `notification_tests::` | 2 | 通知发布、幂等、可见性 |
| `organization_subtree_reporting_tests::` | 2 | 组织子树统计、搜索与审计 |
| `organization_subtree_user_role_tests::` | 2 | 角色分配资格与并发成员变更 |
| `region_catalog_mutations_rollback_when_audit_is_rejected` | 1 | 地区/版本变更的审计原子性 |
| `release_tests::managed_gate::` | 1 | 历史版本就绪、重试和清理围栏 |
| `release_tests::database_upgrade::` | 1 | 标准初始化与历史基线升级 |
| `release_tests::qiankun_only::` | 1 | qiankun 准入、旧入口处理与历史发布归档 |
| `release_tests::url_entries::` | 1 | 环境 URL 入口与代理配置 |
| `data_grant_tests::` | 2 | 数据授权写入与隔离 |
| `data_grant_discovery_tests::` | 2 | 授权发现与成员范围 |
| `platform_session_refresh_tests::` | 2 | 会话刷新和成员变更 |
| `routes::application_setup::tests::` | 2 | 微应用就绪、授权、审计与锁顺序 |

在专用可丢弃的 PostgreSQL 实例上设置 `TEST_DATABASE_URL`，于 `backend` 执行对应命令。例如：

```powershell
cargo test --locked --bin saas-platform-backend release_tests::qiankun_only:: -- --ignored --test-threads=1
cargo test --locked --bin saas-platform-backend release_tests::url_entries:: -- --ignored --test-threads=1
```

测试 fixture 创建唯一名称数据库、执行真实迁移/SQL/HTTP 检查，再移除自建数据库，连接账号需要创建和删除数据库的权限。普通 `backend` job 不会自动执行其余 ignored 用例；新增测试也只有显式加入筛选或普通测试入口后才会被相应任务覆盖。

## 覆盖限制与维护

当前 CI 不安装真实浏览器，不执行 Playwright 完整入口或跨仓库七应用产物验收；执行方式见[浏览器检查](../frontend/tests/browser/README.md)。Nginx 测试中有依赖本机运行条件的真实进程专项，出现 SKIP 时应保留该状态。Docker 构建不代表镜像运行、TLS、网关路由、外部上游、数据库迁移或正式发布已验收。

`retired-paths.json` 和 `scripts/cleanup-retired.ps1` 是限定退役路径的历史维护清单，不参与构建。脚本默认预览，`-Apply` 才删除；清单含历史项目名和跨仓库布局，维护时必须先核对解析后的绝对根目录与当前 checkout，不能把旧清单直接当作本次文档清理指令。

以最新工作流文件和对应 GitHub Actions 运行记录为门禁事实；本地一次通过不能替代远端结果。开发、部署和日常验收入口见[项目文档](../backend/docs/README.md)。
