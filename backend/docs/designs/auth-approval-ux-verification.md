# 认证中心持久授权与选择交互

日期：2026-10-27。本记录仅覆盖本次源码和本机验证，不代表统一身份选择总体方案的全部验收。

## 行为

认证中心将应用同意独立保存在账号与 RP 维度的 `oidc_consents`，与 SSO、grant、access/refresh token 的期限分离。后端覆盖检查当前请求 scope；未撤销且范围已覆盖时复用授权，新增 scope 重新确认。退出账号和组织切换不会撤销同意记录。

选择页只显示必要步骤：只有一个租户直接展示组织；选定租户下只有一个真实可用成员身份时，自动通过相同 CSRF 确认接口继续。搜索过滤出一条不会触发自动确认。多租户用卡片选择，多个组织继续显式选择。最终服务端仍核验来源、应用资格、成员状态和版本。

`/sso/applications` 提供独立账号登录及应用授权管理，分已授权和已撤销列表，展示 scope、时间、应用状态，并支持逐个撤销。撤销需当前 SSO、来源和 CSRF，并在账号锁内重新核验会话，同时撤销该 RP 的现有 grant 和未完成授权请求。其他 RP 及 SSO Cookie 不因这个操作直接撤销。已撤销记录可确认删除，后端通过同一账号锁与活动状态校验拒绝删除重新授权的记录，保留已失效 grant 和安全审计；删除后仍需重新授权。

## 验证

| 检查 | 结果 |
| --- | --- |
| 身份服务 `cargo test --locked --all-targets` | 常规 44 项通过，6 项要求显式外部环境的检查默认忽略；本次相关数据库检查另行执行 |
| 身份服务 `cargo fmt --all -- --check`、`cargo clippy --locked --all-targets -- -D warnings` | 通过 |
| 身份服务 `tests/run-context-selection.ps1 -BaselineFile <当前平台0001>` | 5 项通过，包含持久授权、scope 覆盖、账号隔离、CSRF/来源拒绝、显式撤销、失效旧 flow、重新授权，以及已有来源/code/refresh 回归 |
| `node tests/context-selection-ui.mjs`，显式 Chrome 路径 | 11 个真实浏览器场景通过，包含撤销记录删除的取消、确认、计数与空态；HTTP 数据使用显式 fixtures，不能单独证明真实后端链路 |
| 平台 `cargo test --locked --lib database::tests` | 2 项通过 |
| 平台 `phase12_real_roles_enforce_mixed_user_columns_and_no_ddl_owner_escape`，显式专属测试集群 | 通过；新增平台 runtime 不能读写持久授权表的断言；测试库、角色和连接完成清理 |
| 身份、平台当前源码 Linux `cargo build --offline --locked --bin ...` | 通过，新程序已替换本机运行文件 |
| 本机 Chrome 与真实 HTTPS、两个 BFF 和真实数据库 | 首次平台同意一次；新浏览器/新 SSO 会话登录不再确认；管理页撤销后原平台会话接口返回 401；再次访问重新确认；客户门户单租户/单身份授权后自动进入；管理页显示两个已授权应用；390px 页面无横向溢出 |
| 本机服务探针 | 17 项均返回 200 |
| 已撤销授权删除的真实 HTTPS 检查 | 确认后记录从列表消失；活动授权 DELETE 返回 409；删除后客户门户重新出现授权确认，并在单组织条件下自动进入；认证程序已单独更新，未重建数据库 |

UI fixtures 报告位于身份服务 `artifacts/context-selection/report.json`。真实环境验证、程序哈希、数据库基线和资源清理记录保存在本机私有 `.local-run/authux-*.json`；这些本机文件含运行环境引用，不应提交。

## 数据与资源边界

使用 WSL 原生 PostgreSQL 18 专属临时集群执行数据库测试，复用同一集群至本轮检查结束；脚本有退出处理和限时租约。测试随机库由测试包装器负责删除。没有使用 Testcontainers、Compose 或 Docker 容器。

运行验证新增独立本机平台库 `saas_platform_authux_dev`，使用当前 `0001` 全新初始化并执行角色交接，没有改写旧库 checksum。既有平台库及门店、工单、门户三个库保持保留。新库和应用服务继续运行供用户验证；专属测试集群、一次性初始化文件和无用构建缓存另行清理。

Docker context 为 `default`，创建和复用容器均为 0；Docker 镜像、容器、卷和构建缓存前后均为 0B。未执行 WSL 虚拟磁盘压缩。保留当前程序、必要运行配置/日志和正式验证报告，不将逻辑缓存删除量等同于 Windows 物理空间回收量。
