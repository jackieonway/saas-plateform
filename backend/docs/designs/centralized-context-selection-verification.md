# 统一身份选择：实施验证记录

日期：2026-10-27。状态：部分实现验证，完整功能尚未完成。

本记录对应本次[总体方案](centralized-context-selection.md)的持续实施，不替代总体方案验收矩阵。最新代码的证据以末节“分步页面与两个 BFF 接入”为准；早先段落只保留实施过程和当时的验证边界。

## 本次代码范围

平台后端新增 `context_selection`、`context_selection_store` 和 `context_selection_http`，在当前 `0001_init.sql` 增加平台所有的选择表、RP/服务登记及身份域的交互表。独立身份服务新增 `oidc::context`，接入交互创建、授权绑定和来源核验；权限交接及 readiness 核验同步扩展。未执行已有库升级、清库或成员数据搬迁。客户门户和前端尚未接入。

## 已执行检查

| 命令 | 结果与证明范围 |
| --- | --- |
| `rustfmt --edition 2021 --check src/context_selection.rs src/context_selection_store.rs tests/context_selection.rs tests/context_selection_database.rs` | 通过；仅检查本次新增 Rust 文件 |
| `cargo test --locked --lib --test context_selection --test context_selection_database` | 库测试 8 项、协议核心 9 项通过；需要显式测试库配置的数据库测试在此命令中忽略，随后单独执行 |
| `cargo test --locked --test context_selection_database -- --ignored --nocapture` | 已显式设置任务自有 `CONTEXT_SELECTION_TEST_ADMIN_URL`，真实 PostgreSQL 18 测试 1 项通过 |
| `cargo clippy --locked --lib --test context_selection --test context_selection_database -- -D warnings` | 通过；范围为库与上述测试目标 |

数据库测试从空库执行当前完整初始化基线，覆盖：仅保存凭据摘要，同一交互不能重复签发、消费服务隔离、员工/客户结果类型隔离、消费后事务回滚、两个并发消费只允许一个提交且失败方写入回滚、原结果恢复、原会话绑定校验、保留恢复窗口占用和容量限制。数据库触发器验证包括拒绝延长原截止时间与拒绝覆盖已提交结果。

并发测试中的业务上下文以事务内专属标记表模拟，因此证明的是选择存储的事务行为，不证明真实员工会话、客户资源证明或派生 token 撤销链路已接通。

## 环境与清理

首次尝试在 WSL Ubuntu、Docker context `default` 拉取 PostgreSQL 17 Alpine 镜像，因 Docker Hub 网络超时失败。未创建容器，也未形成镜像、卷、网络或构建缓存。失败后的 `docker system df` 仍为：镜像 0B、容器 0B、卷 0B、构建缓存 0B，与尝试前一致。

随后使用 WSL 已安装的 PostgreSQL 18 工具，作为当前普通用户建立任务专属临时集群，端口 55437，独立于既有 5432 主集群。Rust 测试创建自己的随机命名空库并在成功、失败或测试任务 panic 后删除该精确数据库；没有修改既有主集群或已有业务库。

外层 PowerShell `finally` 已停止本次临时 PostgreSQL 进程，并在检查绝对路径、目录名和实际解析路径后删除专属 `/tmp` 数据与日志目录。未导出临时数据库日志，未创建应用镜像或应用容器。实际创建/复用容器数量均为 0。

本次没有执行 WSL 虚拟磁盘压缩，不声明 Windows 虚拟磁盘文件已缩小。Rust 本地构建输出保留供后续实现复用。

## 尚未证明的要求

OIDC code 扩展及 refresh 隔离、认证中心选择页、员工与客户 BFF 接入、真实业务会话与派生凭据撤销、跨库恢复、跨标签页及宿主微应用状态同步、完整浏览器和独立部署验收均待继续实现与验证。查询/确认测试的来源端口使用受控假实现；真实身份来源处理器另有数据库测试，尚未证明两服务通过真实 HTTPS 串联的完整链路。

当前记录不能作为统一身份选择功能已可部署或总体目标已完成的依据。

## 本轮新增验证

全部数据库验证复用一个新的任务专属 PostgreSQL 18 临时集群，仍使用 55437 端口；各测试自行创建并删除不同的随机空库。没有创建容器。环境通过 [原生测试集群脚本](../../scripts/context-selection-postgres.sh)运行，脚本有退出清理、信号处理和最长 30 分钟租约，可在修复重试时复用同一集群。

| 项目与命令 | 结果与范围 |
| --- | --- |
| 身份服务：`cargo check --locked --all-targets`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo fmt --all -- --check` | 通过 |
| 身份服务：`cargo test --locked --all-targets` | 常规 43 项通过，4 项要求显式环境的数据库测试默认忽略；新来源数据库测试另行显式执行 |
| 身份服务：显式配置 `CONTEXT_SELECTION_TEST_ADMIN_URL` 及绝对路径 `CONTEXT_SELECTION_BASELINE_FILE` 后，`cargo test --locked --lib oidc::context -- --include-ignored --nocapture` | 2 项通过，包含本次新建数据库的真实 HTTP 处理器测试 |
| 平台：`cargo check --locked --all-targets`、`cargo clippy --locked --all-targets -- -D warnings` | 通过 |
| 平台：`cargo fmt --all -- --check` | 未通过：既有未提交的 `identity_ui_tests.rs` 和 `seed.rs` 存在格式差异，本轮保留这些其他工作改动；本轮新增 Rust 文件另行执行定向格式检查 |
| 平台：`cargo test --locked --bin saas-platform-backend context_selection_http::tests -- --ignored --nocapture` | 真实数据库与平台 HTTP 处理器测试通过；账号来源端口采用假实现 |
| 平台：`cargo test --locked --bin saas-platform-backend context_selection_source_credentials_require_a_separate_remote_pair` | 凭据配置成对与独立身份要求测试通过 |
| 平台：显式配置 `IDENTITY_PHASE12_TEST_DATABASE_URL` 后，`cargo test --locked --bin saas-platform-backend phase12_real_roles_enforce_mixed_user_columns_and_no_ddl_owner_escape -- --ignored --nocapture` | 实际受限角色权限验证通过，包含新增身份/平台上下文表的互相拒绝读取；测试库及专属角色已删除 |
| 平台：`cargo test --locked --test context_selection_database -- --ignored --nocapture` | 当前完整基线下的存储事务与并发回归通过 |

身份来源测试验证：后端交互创建、nonce 替换拒绝、交互重复绑定拒绝、服务 RP 范围隔离、SSO 撤销失效、混合证明模式拒绝、事务标识不能替代原始 grant，以及平台持有账号写锁时来源核验仍能完成。

平台处理器测试验证：员工查询不返回客户身份、客户成员确认拒绝、组织应用准入撤销后的重新校验、确认凭据创建、同一交互重复签发拒绝、来源撤销失败关闭、服务用途与 RP 范围隔离。响应同时检查 `Cache-Control: no-store`。

初始化测试最初发现测试 fixture 使用了不存在的 `org_type` 列，已改为当前基线的 `organization_type` 并在同一临时集群重跑通过；没有修改正式数据库。身份来源数据库测试首次误带 `--include-ignored` 且未设置显式环境变量，按预期拒绝启动，随后在任务自有集群配置后通过。

本轮临时集群已停止并输出 `TASK_POSTGRES_CLEANED`，专属数据和日志目录已删除；既有主集群未停止。未创建/复用 Docker 容器、镜像或卷，未压缩 WSL 虚拟磁盘。

## 分步页面与两个 BFF 接入

日期：2026-10-27。已加入认证中心分步页面与一次性 code 扩展、员工 BFF 同库消费、客户门户独立消费和同结果恢复。当前基线仍只有 `0001`；本轮没有升级或删除已有业务库。

| 项目及检查 | 实际结果与边界 |
| --- | --- |
| 身份服务 `cargo check --locked --all-targets`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo fmt --all -- --check` | 通过 |
| 身份服务 `cargo test --locked --all-targets` | 常规 44 项通过；5 项要求显式环境的数据库检查默认忽略，当前上下文的 2 项数据库检查另行执行 |
| 身份服务 `tests/run-context-selection.ps1 -BaselineFile <当前平台0001绝对路径>` | 4 项上下文检查通过，含真实来源及真实 OIDC 页面/code/refresh 处理器；平台查询/确认端口使用明确的测试实现 |
| 身份页面 `node tests/context-selection-ui.mjs`，显式安装的 Chrome 路径 | 6 个真实浏览器场景通过；使用生产静态文件与 HTTP fixtures，见[页面报告](../../../../saas-identity/artifacts/context-selection/report.json) |
| 平台 `cargo check --locked --all-targets`、`cargo clippy --locked --all-targets -- -D warnings` | 通过；修改文件的定向格式检查通过，全仓格式门禁仍有之前记录的无关文件差异 |
| 平台 `cargo test --locked --bin saas-platform-backend centralized_ -- --ignored --nocapture` | 2 项测试通过，员工测试含正常提交及强制消费失败回滚两个新库场景；客户测试含初次消费、恢复、重放、错误消费方/新会话、续期、切换和撤销 |
| 平台 `context_selection_http::tests` 显式数据库测试 | 查询与确认处理器回归通过 |
| 平台协议与存储 `cargo test --locked --test context_selection --test context_selection_database -- --include-ignored` | 9 项协议测试和 1 项真实数据库并发/回滚检查通过 |
| 平台 `phase12_real_roles_enforce_mixed_user_columns_and_no_ddl_owner_escape` 显式数据库测试 | 当前完整基线的真实最小角色权限隔离通过；测试库和角色删除成功 |
| 员工前端 `node --test tests/bff-session.mjs tests/bff-integration.mjs tests/bff-login-recovery.mjs` 与 TypeScript 检查 | 35 项通过；新增认证中心切换的 CSRF/版本请求、导航校验、旧数据冻结，保留既有会话和延迟响应回归 |
| 门户后端 `cargo check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo fmt --all -- --check` | 通过 |
| 门户后端 `cargo test --locked --all-targets` | 常规 12 项通过，显式数据库测试另行运行 |
| 门户后端 `tests::pg`，显式任务集群 URL 和 `CONTEXT_SELECTION_TEST_CLUSTER_ID` | 6 项真实数据库测试通过，包括本地 UPDATE 被触发器抑制后保留原加密操作，再次仅查询原 COMMITTED 结果完成保存，没有再次签发/消费 |
| 门户前端 `npm test`、`npm run typecheck`、`npm run build` | 56 项通过，类型检查和生产构建通过 |
| 门户前端 `node tests/centralized-browser.mjs` | 生产 bundle 的桌面/390px 移动端共 2 场景通过：显示当前租户组织、隐藏本地成员按钮、以原版本发起认证中心切换；见[浏览器报告](../../../../saas-customer-portal/frontend/artifacts/centralized-selection/report.json) |

身份服务真实处理器验证：应用授权与身份选择分离；未授权提交成员返回冲突；确认授权不签发 code；刷新后仍保留本次授权确认；最终选择才签发 code。选择凭据使用 AEAD 绑定事务/flow/RP/issuer，code 兑换时只交付一次并清除数据库密文，refresh 不交付，重放拒绝，浏览器回调只含标准 code/state/iss。

员工测试执行真实 OIDC token/JWKS/introspection/userinfo 处理器、平台回调、续期和业务 API；新的身份选择扩展及来源交互通过明确测试端口提供。测试证明选择消费与真实 BFF/业务会话在同一事务提交；预先消费导致最终失败时，BFF、业务会话和浏览器指针均回滚，不留下半完成会话。该测试不证明认证中心新页面通过真实 HTTPS 到平台已完整串联。

客户测试执行真实客户资源证明和平台事务，身份交互来源仍为明确测试端口。恢复返回逐字段相同的原 DTO 和同一 cpc 能力，不新增上下文；通用结果 JSON 不含 token，能力只在独立用途的 AEAD 密文列中保存。客户证明绑定门户 BFF ID/版本；切换同时撤销该旧版本全部历次续期证明。成员停用并递增代际后，原结果不能恢复。客户结果不会成为员工业务会话。

本轮还将旧客户资源测试 fixture 对“第 19 次迁移及旧二元成员约束”的要求改为只读检查当前空库 `0001` 与三元成员唯一键；门户数据库测试改为要求显式提供任务集群 system identifier，避免依赖旧机器集群编号。没有在正式数据库执行兼容 DDL。

尚未验收：三服务通过真实 CA/HTTPS 的完整登录和切换，两个新流程的完整浏览器回调和并发竞争，选择期间所有标签页统一冻结、宿主微应用重挂载及通知重订阅，部署登记与私网网关配置，空闲时的定时到期清理。当前平台签发时已有有限批次过期选择清理；不能将其表述为独立定时清理任务。公开网关已固定拒绝 `/internal/*`，部署必须提供独立的受限 HTTPS 私有入口，不能把公网控制台 origin 直接当作私有服务地址。

本轮只使用 WSL 原生 PostgreSQL 18，实际创建/复用 Docker 容器均为 0。第一个专属临时集群按 30 分钟租约自动回收，后续验证使用新建的单个专属集群并复用至本轮结束；没有同时创建多套验证环境。两次专属数据和日志目录均已核查不存在；第二个集群主动结束并输出 `TASK_POSTGRES_CLEANED`。清理前仅剩默认 postgres 数据库，测试库、临时角色和签名 fixture 已分别清除；55437 无监听，既有 5432 主集群仍在线。

Docker context 为 `default`；前后 `docker system df` 的镜像、容器、卷及构建缓存均为 0B，复查没有任何容器或命名卷。未创建 Docker 临时网络或专用镜像，没有执行全局 prune 或 WSL 虚拟磁盘压缩。保留正式验证报告、截图和常规 Rust/前端构建产物，不声明 Windows 虚拟磁盘已缩小。
