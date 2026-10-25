# 平台 API

`saas-platform-backend` 是独立 Rust 包，使用 Axum 0.8、Tokio、SQLx 和 PostgreSQL。默认监听 `0.0.0.0:3001`；`main.rs` 进入 `bootstrap.rs`，先校验配置和连接数据库。默认本地初始化分支执行内嵌迁移、平台及区域初始化；`IDENTITY_REMOTE_ENABLED=true` 的独立认证部署分支只验证已交接的部署与成员 readiness，跳过迁移、seed 和区域初始化，再装配 HTTP。`imported` 分支的独立校验规则见下文。

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

进程环境优先；`APP_ENV_FILE` 可指定绝对路径环境文件。开发模式可读取本目录 `.env`，生产不自动查找环境文件。`DATABASE_URL` 指向显式的平台业务库，独立认证过渡部署可与 identity 共用物理数据库，各运行角色分别交接最小权限；`AUTO_CREATE_DATABASE` 默认关闭。默认初始化分支的空生产库管理员通过成对的 `BOOTSTRAP_ADMIN_USERNAME`、`BOOTSTRAP_ADMIN_PASSWORD` 初始化，开发库使用独立的演示 seed；独立认证运行态不承担这些初始化，见开发指南与部署合同。

离线新目标库使用专用命令 `cargo run --locked --bin platform-migrate -- --import`，也可省略 `--import`。两种调用都只执行 `database::run_migrations` 的真实内嵌迁移后关闭连接退出，不执行数据导入、平台 seed、地区目录或地区菜单／授权初始化，不启动 HTTP、分发或维护 worker。迁移命令只读取进程 `DATABASE_URL`，不读取 `.env` 或 `APP_ENV_FILE`，不自动建库，也不要求平台 JWT／应用／CORS 配置；目标库必须预先创建，URL 必须明确包含 PostgreSQL 主机、账号与业务库名。拒绝 `PG*` 环境变量以避免隐式 libpq 连接配置；端口省略时固定为 5432。连接获取预算 10 秒，迁移总预算与 statement timeout 为 300 秒，lock timeout 为 30 秒。

迁移 URL 还支持显式 Unix socket 与受控角色切换，例如 `postgresql://postgres@localhost:5432/<target>?host=/run/postgresql&options[role]=<owner>`：以 `postgres` 本机认证，迁移对象归 `NOLOGIN` owner 所有。`host` query 只接受绝对 Unix socket 目录，`options[role]` 只接受 1–63 位 ASCII 标识符（首位字母或下划线，其余可含数字）；其他角色／数据库覆盖参数、原始 `options`、重复与未知 query 都会拒绝。解码后含 NUL 或控制字符的账号、密码、数据库名和 query 值也会拒绝，防止改变 PostgreSQL 启动协议字段。TLS 参数沿用 SQLx 的显式 URL 配置；命令不读取默认 `.pgpass`。

`--seed` 在迁移入口被拒绝，`--seed --import`（任意顺序）会在读取 URL 和连接前报 `SEED_IMPORT_MUTUALLY_EXCLUSIVE`，重复／未知参数也会失败。正常服务的 `PLATFORM_DATABASE_INIT_MODE` 默认是 `seed`，适用于原有新库初始化；显式 `import` 会在数据库连接前拒绝正常启动并提示使用迁移命令。导入准备期间应保持正常服务和所有 worker 停止。

已完成认证导入、对账及凭据轮换的独立目标库使用 `PLATFORM_DATABASE_INIT_MODE=imported`，并同时提供以下三个交接值：

| 字段 | 约束 |
| --- | --- |
| `PLATFORM_EXPECTED_DATABASE` | 明确目标业务库名，必须与 `DATABASE_URL` 解码后的库名及实际 `current_database()` 完局一致 |
| `PLATFORM_IMPORT_MARKER` | 数据库 comment 中的精确完成标记：`saas-uuid-import/1/<canonical UUIDv7 batch>/<64 位小写 SHA256>` |
| `PLATFORM_EXPECTED_SCHEMA_SHA256` | 认证导入交接证明中的完整有序 `schema_material` SHA256，64 位小写十六进制 |

三个交接字段缺失、格式错误，或出现在非 `imported` 模式都会拒绝。`imported` 还要求显式 `DATABASE_URL` 与独立强 `JWT_SECRET`，禁止 `AUTO_CREATE_DATABASE=true` 和 `BOOTSTRAP_ADMIN_*`。URL 不接受数据库／账号覆盖或任意启动 options。启动先用只读 repeatable-read 事务核验实际库名、comment、独立 LOGIN 角色、owner 的 NOLOGIN 状态，以及自身或可切换／继承角色均无 owner／高权／数据库或业务 schema CREATE 权限，再验证真实迁移版本／checksum 与完整 catalog 结构摘要。摘要仅从服务自有 `src/database/schema_material.sql` 读取 catalog，不扫描业务行、不创建 reference schema；必须消费正式初始化及导入证明的可信摘要，不能读取任意现库后自签为基线。

所有验证通过后，`imported` 跳过迁移、平台 seed 与局部 `region_seed`（包括地区菜单／授权），然后正常装配 HTTP 和后台任务。此模式只验证目标身份与结构；旧凭据失效还必须有六项 credential 表排除／撤销证据、新 JWT 签名密钥、机密客户端等实际凭据轮换，以及与该导入标记绑定的新 runtime epoch 交接。它不负责生成或证明这些凭据已轮换。

| 入口 | 职责 |
| --- | --- |
| `/api/health` | 检查 HTTP 服务及数据库，数据库不可用时返回 503 |
| `/api/auth/*` | 兼容员工登录、会话和当前成员上下文 |
| `/auth/*` | 员工 OIDC BFF 登录、回调、精确成员选择、会话和注销 |
| `/api/applications/*` | 应用配置、接入检查、版本和环境入口 |
| `/api/apps/{appCode}/*` | 面向业务后端的应用授权、数据范围、成员与通知 |
| `/oauth/*` | OAuth 发放、内省、撤销及设备流程 |
| `/micro-proxy/*` | 已登记 URL 微应用的匿名静态资源代理 |

业务 API 不在此进程中；平台应用 token 必须经业务后端逐请求校验。代理也不转发业务数据或 OAuth 会话。

身份演进的 Phase 1～2 已交付[成员生命周期 API](docs/identity-sso/phase-01-api.md)、[凭据绑定](docs/identity-sso/phase-02-credential-binding.md)和[准入策略管理](docs/identity-sso/phase-02-policy-api.md)。session、refresh、应用及 legacy OAuth 凭据保存具体成员、账号和适用的来源／策略版本，旧缺绑定凭据要求重新认证；局部撤销按成员 ID，权威依赖不可用返回 503。实际门禁与分组回归见[Phase 2 验收](docs/identity-sso/validation/phase-02.md)，员工授权、目录、双向角色及 bulk／生命周期最终支持清理已通过[Phase 3 验收](docs/identity-sso/validation/phase-03.md)，通知成员／代际、HTTP／偏好与实时撤权已通过[Phase 4 验收](docs/identity-sso/validation/phase-04.md)，管理 UI、完整成员 namespace 和准入编辑已通过[Phase 5 验收](docs/identity-sso/validation/phase-05.md)，接口见[控制台成员证明](docs/identity-sso/phase-05-console-identity.md)。独立客户最小资源、真实新库收缩和 receipt 启动已通过[Phase 6 验收](docs/identity-sso/validation/phase-06.md)，接口与操作见[客户资料](docs/identity-sso/phase-06-customer-profile.md)和[收缩合同](docs/identity-sso/phase-06-rollout.md)。同进程认证／账号管理 port、原事务适配及存储 owner 已通过[Phase 7 验收](docs/identity-sso/validation/phase-07.md)，全局 SSO session、独立 Cookie／CSRF 与账号撤销已通过[Phase 8 验收](docs/identity-sso/validation/phase-08.md)，标准 OIDC 与独立密钥已通过[Phase 9 验收](docs/identity-sso/validation/phase-09.md)，平台 BFF 与兼容入口已通过[Phase 10 验收](docs/identity-sso/validation/phase-10.md)，独立客户门户已通过[Phase 11 验收](docs/identity-sso/validation/phase-11.md)，独立认证服务、网络 port 与同库最小权限已通过[Phase 12 验收](docs/identity-sso/validation/phase-12.md)；[Phase 13 验收](docs/identity-sso/validation/phase-13.md)已完成任务容量、指定客户端灰度、身份感知双库恢复和活动消费者最终验收，[台账](docs/identity-sso/task-ledger.md)的13个阶段均为 COMPLETED。容量和浏览器按用户要求复用成功项、仅补失败项，各轮与UI版本单独记证。`MEMBERSHIP_LIFECYCLE_ENABLED`、`CUSTOMER_MEMBERSHIPS_ENABLED`、`DUAL_MEMBERSHIPS_ENABLED` 默认均为 false。Phase 1～5 验收时仅允许隔离 `APP_ENV=test` 且应用环境非 production 开启；当前保留该 fixture 分支，其他环境须按[Phase 6 独立收缩与启用合同](docs/identity-sso/phase-06-rollout.md)提供环境绑定的 receipt 与 manifest 摘要并通过启动验证。Phase 6 仅在任务自有 development 新库完成受控启用，生产尚未启用客户或第二身份。平台 BFF 默认关闭，实际启用需完整 OIDC／Cookie／CSRF 配置；客户门户与独立认证服务均已独立交付；Phase 13 任务范围的分轮聚合已验收；生产尚未开放，活跃 legacy 继续支持，首个客户业务限本人资料。

## 文件边界

`src/routes/` 是 HTTP 适配，`src/public_contract.rs` 定义公共 DTO；数据库迁移在 `migrations/` 编译时嵌入。`tests/fixtures/micro-app-protocol/` 保存独立 SDK 协议校验材料，浏览器 SDK 副本在前端工程。发布、ZIP 分发、镜像部署等模块是平台控制面，不是跨仓库构建工具。

本目录是平台 API 镜像构建上下文：`docker build -t saas-platform-api:local .`。生产需按[部署指南](docs/deployment.md)准备镜像摘要、环境文件、网关和证书。
