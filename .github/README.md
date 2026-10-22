# 本仓库持续集成

workflows/ci.yml 只检出本仓库，执行：

1. frontend 的依赖安装、SDK、代码检查、测试与构建。
2. backend 的 Rust 格式、检查、严格 Clippy、测试及独立 deployment-contract 协议测试。
3. backend-integration 使用 PostgreSQL 17 运行指定的隔离数据库回归。
4. ops/monitoring、ops/recovery 的独立测试。
5. frontend、backend、ops/monitoring 的 Docker 构建。

contracts 任务单独运行 .github/tests 中的仓库结构、跨端 SDK 检查，以及 backend/tests/nginx-render-policy.mjs。前端测试与浏览器工具保存在 frontend/tests，后端不再承载 acceptance 目录。

工作流不发布镜像、执行部署或读取生产配置。真实浏览器和未纳入下列任务的数据库专项验收需准备专门环境，见 [工程与验证](../backend/docs/engineering-baseline.md)。

## PostgreSQL 回归任务

`backend-integration` 在独立 GitHub runner 中启动 `postgres:17` 服务，通过 `pg_isready` 健康检查后开始测试。连接信息仅来自任务内显式设置的 `TEST_DATABASE_URL`，账号密码为该临时服务专用测试值；不引用仓库 secrets 或生产连接串，也不需要 `.env.test`。

任务按以下顺序执行指定的 ignored 数据库回归，模块用例限制 `--test-threads=2`，审计与历史发布门禁专项使用单线程：

| 测试模块 | 覆盖范围 |
| --- | --- |
| `oauth_issuance_tests` | OAuth 签发、刷新、撤销与并发消费 |
| `management_audit_tests::oauth_` | OAuth 管理、PKCE 默认/兼容迁移、策略更新审计失败回滚 |
| `entity_creation_tests` | 实体创建、平台控制面生命周期保护、菜单父子关系与组织数据关联 |
| `notification_tests` | 通知发布、幂等和收件箱可见性 |
| `organization_subtree_reporting_tests` | 组织子树的统计、搜索与审计范围 |
| `organization_subtree_user_role_tests` | 两个角色分配入口的成员资格及锁等待期间成员变更 |
| `region_catalog_mutations_rollback_when_audit_is_rejected` | 地区与版本五类变更的审计失败回滚、复制及无变化更新 |
| `release_tests::managed_gate` | 版本入口别名与失败部署重试不能绕过历史就绪及清理围栏 |
| `data_grant_tests` | 数据授权修改、权限及隔离边界 |
| `data_grant_discovery_tests` | 数据授权发现与成员选项范围 |
| `platform_session_refresh_tests` | 平台会话刷新和成员变更后的有效性 |
| `routes::application_setup::tests` | 微应用就绪、组织选项、追加授权、审计回滚及并发锁顺序 |

模块命令如下；OAuth 管理审计、历史发布门禁模块及地区审计专项使用 `--test-threads=1`。地区审计使用完整测试名称，不附加 `::`：

```sh
cargo test --locked --bin saas-platform-backend <模块>:: -- --ignored --test-threads=2
```

测试通过独立数据库 fixture 创建唯一名称的数据库、执行迁移、验证实际 HTTP/SQL 行为，随后移除自建数据库。服务账号需要创建和删除数据库的权限，因此本地复验也必须使用专用可丢弃的 PostgreSQL 实例。任务结束时 GitHub runner 销毁临时数据库服务；失败不会触发部署。

原有 `backend` 任务继续执行普通 `cargo test --locked --all-targets`，不自动启用其余 ignored 用例。新增任务定义不等于已执行远端 CI；是否通过以对应 GitHub Actions 运行记录为准。

retired-paths.json 与 scripts/cleanup-retired.ps1 只用于限定退役路径的维护核对，不参与构建；脚本默认预览，清单内 105 个路径已核对不存在。
