# 项目受管的检查入口

本目录提供台账 Phase 4 建立的分层检查，Phase 5 接入 ZIP 原子发布回归，Phase 6 接入版本化 SDK 的生成物漂移与公共协议检查，Phase 7 接入可信生命周期故障与运行域隔离回归，Phase 8 接入展示更新、授权替换和入口快照的一致性回归，Phase 9 接入共享授权查询、路由判定、导航竞态与创建路径隔离回归，Phase 10 接入多标签页的原子会话、轮换与共享组织回归，Phase 11 接入普通 OAuth 原子签发、实时失权和平台刷新回归，Phase 12 接入有界登录、密码计算与可信来源回归，Phase 13 接入管理变更与审计同事务的故障回归，Phase 14 接入受控数据 grant 管理回归，Phase 15 接入 grant 发现与真实浏览器保存/撤销闭环，Phase 16 接入独立 Rust 工单 crate 的严格检查和业务迁移接管回归，Phase 17 接入工单只读授权/SQL 和真实平台联合回归。命令以当前实际 manifest 为准：平台是 `backend/Cargo.toml` 下的单 crate，Phase 30 增加窄 library target 与公共策略集成测试，工单 Rust 服务持独立 Cargo.toml/Cargo.lock，Vue、jQuery、WebSocket 示例分别有自己的 npm 工程。没有根 Cargo workspace，也不把尚未迁移的 Node 业务服务计作 Rust 验收通过。

工具版本固定为 Node **24.18.0**、npm **11.16.0**、Rust **1.97.0**、Playwright **1.64.1**。PostgreSQL 测试使用 **17.6**；OCI 交付检查要求 Docker Engine **29 或以上**，GitHub delivery job 声明 **29.1.3**。当前实际镜像验收对象为 `linux/amd64`。Chrome、Edge 安装使用真实产品 channel，每次报告实际版本；这不等同于锁定它们的发布版本或宣称所有稳定版本受支持。

## 五层 gate

Phase 22 增加独立 Rust Socket.IO fixture 与聊天参考的单元测试，以及 `rust-socketio-compatibility` 真实客户端矩阵。单独入口为根 `npm run test:realtime`；它只启动 Rust peer，以宿主安装的 Socket.IO 客户端和真实聊天 factory 验证传输、事件、拒绝、到期、重连与来源。库版本、合成认证边界和 Phase 23 必需联合验收见 [协议选型](../../docs/realtime-protocol.md)。必需检查适用零失败、零跳过规则；主机阻止 Rust 回环入站时应在受控 Linux 环境运行，不能将环境错误作为协议 fallback 依据。

Phase 23 将通知服务接入三个层次的必需检查。`fast` 对独立 `mini-services/notification-ws/Cargo.toml` 执行 fmt、全目标/全 feature check、严格 Clippy、完整单元测试和 doc，并执行 `notification-ci-policy.mjs` 的失败反例。`postgres` 的 `notification-live` 使用 [notification.mjs](notification.mjs) 构建真实 Rust 平台、通知服务和受控延迟代理，在随机独占 PostgreSQL 中串行执行平台授权、分区调度和传输边界套件；`browser` 再用同一个入口的 `--browser` 执行真实双页组织切换与 HTTP 收件箱恢复。数据库环境仅由 Linux launcher 注入，临时 PG 使用 2 GiB tmpfs 满足真实地区 seed 的存储需求，保持 40 个连接的上限；缺少自有数据库、二进制、必需检查或发生 skip 时失败。Node 只提供测试驱动和 Vite。

`delivery` 的 `notification-image` 使用 [notification-image.mjs](notification-image.mjs) 单独构建 `linux/amd64` Rust 通知镜像。九项必需验证覆盖锁定源码构建、无 Node 与非 root 运行、默认关闭 stdin 仍常驻、依赖不可用时 live 200/ready 503、独立 metrics Bearer、只读私网容器、SIGTERM 零退出、凭据脱敏，以及当前完整输入 hash 与候选一致。该镜像检查不代替真实平台/PG 联合验收。原生平台发行包也不捆绑通知二进制；通知通过自己的 Cargo 清单、Dockerfile 和正式 `notification` Compose profile 交付。

```sh
node scripts/ci/run.mjs --gate fast --out-dir /absolute/new-output/fast
node scripts/ci/linux-runner.mjs --gate browser --image saas-ci:phase04 --browser chromium --out-dir /absolute/new-output/browser
node scripts/ci/run.mjs --gate delivery --platform linux/amd64 --out-dir /absolute/new-output/delivery
node scripts/ci/run.mjs --gate business-delivery --platform linux/amd64 --out-dir /absolute/new-output/business-delivery
```

输出路径必须是尚不存在的绝对目录，不能覆盖已有证据或放入源码输入目录。`target/` 下的新子目录也可用。每次运行使用新的目录。

| Gate | 当前必须执行的范围 | 前提与结果边界 |
| --- | --- | --- |
| `fast` | 执行当前 `run.mjs` 声明的全部必需步骤；独立工单、门店与管理 CLI crate 各自验证，门店另构建独立二进制，管理 CLI 另构建真实二进制并执行 HTTPS/TLS driver，合计步数以本次 summary 为准 | 需要固定 Node/npm/Rust 与 openssl；不启动平台或业务数据库。TLS driver 只启动自有临时 HTTPS fixture；步骤清单见 `run.mjs`，测试文件清单见 `manifest.mjs` |
| `postgres` | 平台十二个选定模块当前 76 项（原 74 项保留，成员声明策略新增 2 项）；工单迁移升级 27 项、只读契约 13 项、写入契约 14 项、成员契约 12 项、提醒契约 12 项、Phase17/18/19/20 真实平台联合各 1 项分别登记；门店 SQL/联合各 1 项分别包含 8/16 个命名检查；通知联合门禁保持独立 | 由 `linux-runner.mjs` 创建受控环境，再在容器内调用 `run.mjs --gate postgres`；不得传生产连接串或借用 `.env.test` |
| `browser` | 十套前端 fixture 套件每产品 200 组，随后独立执行真实 Axum/PG 的 grant UI 闭环：精确 1 项 Rust 测试、16 个命名浏览器检查与 4 条关联审计 | 由 Linux launcher 创建临时 PG；只有 live harness 收到受控数据库环境。Chromium、Chrome、Edge 分别执行，缺产品、报告、检查或清理直接失败 |
| `delivery` | Compose 策略、干净源码镜像构建、完整 OCI 字节校验、Rust Linux 二进制探测、真实 Docker/Nginx/HTTPS 七组 smoke、独立 Rust 通知镜像九项检查，以及 Rust API/PostgreSQL/Nginx 的 ZIP 发布回归 | 需要本机 Linux Docker daemon、Compose、openssl 和正常 CA 校验；不会推送 registry 或部署目标环境 |
| `business-delivery` | 工单/门店独立静态版本与 Rust API 镜像候选、隔离 registry 推送/拉取、固定 digest Compose、v1/v2/回退和三个产品的真实版本入口 | 由宿主 Node 调用专属 driver，自己持有隔离 registry、业务数据库和 HTTPS 资源；必需 typed 报告、当前源码摘要及清理证据全部通过后才保留候选 archive |

直接运行 `fast`、`browser` 会按锁重新安装依赖，在构建前执行 `node scripts/sync-micro-sdk.mjs --check`，随后重建前端产物。漂移检查校验已登记的 classic/ESM、类型/schema、Rust DTO 和精确列出的 vendor 副本；CI 不先执行同步命令，漂移失败阻止后续构建。需要有意更新 SDK 时，开发者先运行统一生成入口并审查源码和产物的变更，再提交给 CI。需要独立工作目录的开发机应采用下述 Linux 容器流程。GitHub job 的调用关系、产物上传及分支保护建议见 [GitHub CI 说明](../../.github/README.md)。

`fast` 的无 DB 契约包含 `tests/micro-runtime-faults.mjs`，本地 `npm --prefix frontend run test:contracts` 也执行该文件。回归使用可控时钟验证可信生命周期永久 pending 后的稳定故障、凭据取消、迟到结果及运行域隔离；它不声称截止时间能终止原异步操作或主线程同步循环。故障测试和其他必需契约采用相同的零失败、零 skip/todo/cancelled 规则，不能因等待超时而跳过测试。

展示/授权更新的专项覆盖宿主 `tests/micro-context-updates.mjs`、iframe 协议、登记 SDK 与真实工单 client/app。修改既有测试文件的新增用例同样属于必需契约；本地 `test:contracts`、`test:micro` 和正式 fast 都包含这些更新契约。验收区分纯主题更新保留业务提交、路由相关读取、授权替换取消旧请求、入口与 props 版本一致，以及临时 503 的有界 fail-closed 恢复；不能由旧阶段的通过记录代替本次输入上的实际执行，也不能把 401/403/404 或协议错误改成宽泛重试来取得通过。

授权与导航专项为 `tests/auth-queries.mjs`、`tests/route-policy.mjs`、`tests/route-policy-guard.mjs` 和 `tests/application-navigation.mjs`。正式 fast、本地 `test:contracts` 与 `npm --prefix frontend run test:auth` 均执行这些文件和工单 client/app、管理员 manifest 校验与离线 grant 生成回归。它们验证同会话查询共享、取消与旧 epoch 拒绝、明确页面策略、A→B 快选后迟到结果失效，以及创建 MENU 与详情读授权分离；未知授权继续拒绝。回归检查既有缓存及选择竞态，不把缓存 key 的差异表述为已证实的跨租户漏洞。

Phase 9 首次纳入的两个 Node 管理工具套件在 Phase 25 迁至独立 `tools/platform-admin` Rust crate。fast 执行该 crate 的 fmt、全目标 check、严格 Clippy、完整测试和 doc；`tests/platform-admin-contract.mjs` 保留七个菜单 key、创建/详情路径及 schema/manifest 的共同契约。公开源码快照包含 Rust 实现、锁、schema 和 manifest；原 deploy 目录只保留明确的 `bootstrap.manifest.json` 与 `grants.manifest.example.json`，旧 Node 管理实现及测试退役。快照还明确包含 `business-apps/work-orders/tests/platform-e2e.mjs`，用于核对其显式 Rust 二进制入口。Phase 15 纳入的既有业务后台兼容源码继续保留；`.env`、私有文件、node_modules 与生成物排除规则保持生效。真实 CLI→平台/隔离 PG 的受管验证另由 `scripts/ci/platform-admin.mjs` 执行，不把离线计划或 HTTP fixture 当真实授权通过。

Phase 25 的 `platform-admin-build` 先用独立 manifest、`--locked` 与明确的 `--target-dir target/phase25-platform-admin` 构建 `saas-platform-admin`，失败会阻止后续依赖步骤。`platform-admin-tls` 用 step 的独立环境将该产物的绝对路径传为 `PHASE25_CLI_BIN`，将本次全新输出目录下的 `platform-admin-tls` 传为 `PHASE25_TLS_OUT`，不继承管理员 Bearer、生产 CA 或数据库配置。driver 自行生成临时 CA/证书与合成 Bearer，执行 9 个真实 TLS/秘密输出检查；在 Node TAP 中计为 1 个必需文件测试，命名检查与二进制摘要另见 `platform-admin-tls/result.json`。独立 `platform-admin-tls` 结果格式同时要求 TAP 恰有 1 个通过文件、JSON schemaVersion/status/计数正确、9 个精确检查 ID 各通过一次、临时 TLS 清理成功及有效二进制 SHA-256；缺报告、重复/缺项、FAIL JSON 或只有 TAP 成功都不能通过。缺少 openssl、二进制或 fresh 输出、TLS 断言失败、超时、skip/todo/cancelled 同样失败。临时 TLS 私钥在 driver 结束时清理，不使用关闭证书/主机名验证的选项。此 fixture 不代替真实平台/PG 联合验收。

Phase 10 的 `tests/session-coordinator.mjs`、`tests/session-integration.mjs` 与既有 `tests/session-refresh.mjs` 一起进入正式 contracts。本地 `npm --prefix frontend run test:auth`、`test:auth:refresh` 和 `test:contracts` 采用相同的三个会话回归文件；路径相对于 npm 实际使用的 `frontend` 工作目录。浏览器专项入口为 `npm --prefix frontend run test:auth:session:browser`。这些测试覆盖跨页竞争及单页刷新约束，不能用只检查存储结构的单元结果代替真实双页执行。

共享会话依赖同源 Web Locks 和可读写的原子存储；广播只提示版本变化，接收页重读共享记录。Phase 10 要求实际核验能力缺失时的拒绝与提示，以及 Chrome/Edge 各最近两个稳定主版本。受管三产品 gate 继续运行全部套件；相邻稳定版本另在固定官方产品镜像中执行同一会话专项。具体产品版本、镜像与能力结果必须以该阶段实际报告绑定，不能由安装成功或 manifest 声明推导为已支持。

Phase 12 的 fast 单元声明为 limiter 8、密码服务 8、client IP 6、config 4 项；PG `login_protection_tests` 声明 5 项，覆盖随机账号与伪造来源、共享别名无连带失败、聚合验证截止不签部分 ticket、密码 HTTP 生命周期，以及混合 Argon2 压力下普通请求响应。许可在真实后台任务退出前不释放，测试取消/超时不等于 CPU 被终止；过程内预算与可信代理边界见 [登录保护](../../docs/login-protection.md)。这些数量是必测声明，实际通过/失败与测量见 [Phase 12](../../docs/validation/phase-12.md)，不扩张为多副本全局配额或任意 PHC 成本容量结论。

Phase 13 的 `management_audit_tests` 声明 8 项，要求在临时测试数据库内用审计触发器分别抛出 SQL 异常和 `RETURN NULL`（零写入）注入故障，核对主写、集合替换、级联副作用及审计一起回滚；移除故障后重试一次，应得到一次成功变更和一条对应审计。选定矩阵涵盖角色/授权、用户/密码/平台设置、成员/用户角色、租户/组织、OAuth 管理、应用/菜单、发布及通知。发布与通知回归继续核验各自既有事务和幂等边界；既有 `notification_tests` 也纳入必需门禁，精确执行 8 项通知及 2 项应用成员回归。[Phase 13](../../docs/validation/phase-13.md) 保留十组 64 项的历史执行结果、快照摘要与故障明细；[Phase 12](../../docs/validation/phase-12.md) 保留原八组 46 项的历史结果。

Phase 14 的 `data_grant_tests` 声明 7 项真实 PG 回归，要求验证独立读/写权限与平台根上下文、明确业务租户管理范围、有效 direct membership 和目标 tenant/org 一致性、应用资源动作与 scope 上限、UUID 保留的差量写入及冲突、下一受保护请求的即时变化，以及同事务审计失败回滚。fast 同时选择完整 `data_grants::` 单元，共 7 项（policy 5、usecase 2），config 当前声明 6 项。以上是当前必测范围，实际测试数量、通过/失败与冻结输入见 [Phase 14](../../docs/validation/phase-14.md)，不得由 Phase 13 的历史通过推导本阶段结果。

## 干净 Linux 容器流程

使用本机 Linux Docker daemon 和固定 Node 执行 launcher。测试工具镜像只用于构建与检查，不是应用运行镜像。先构建不含浏览器的 `toolchain` stage，以及包含项目浏览器层的完整镜像：

```sh
docker build --target toolchain --build-arg TARGETARCH=amd64 -f deploy/ci/Dockerfile -t saas-ci:phase04-tools .
docker build --build-arg TARGETARCH=amd64 -f deploy/ci/Dockerfile -t saas-ci:phase04 .
docker pull postgres@sha256:45cd22f8d32e189d245403954882f88e7a8714301fda80dab6da90f1265b25a3
docker tag postgres@sha256:45cd22f8d32e189d245403954882f88e7a8714301fda80dab6da90f1265b25a3 postgres:17.6-bookworm

node scripts/ci/linux-runner.mjs --gate fast --image saas-ci:phase04-tools --out-dir /absolute/new-output/linux-fast
node scripts/ci/linux-runner.mjs --gate postgres --image saas-ci:phase04-tools --out-dir /absolute/new-output/linux-postgres
node scripts/ci/linux-runner.mjs --gate browser --image saas-ci:phase04 --browser chromium --out-dir /absolute/new-output/linux-chromium
node scripts/ci/linux-runner.mjs --gate browser --image saas-ci:phase04 --browser chrome --out-dir /absolute/new-output/linux-chrome
node scripts/ci/linux-runner.mjs --gate browser --image saas-ci:phase04 --browser msedge --out-dir /absolute/new-output/linux-msedge
```

Launcher 为每次运行复制干净源码快照，不包含 `.git`、私有环境文件、node_modules、dist、target 或个人浏览器缓存。容器没有宿主工作区、个人目录或 Docker socket 挂载；工具镜像和 PostgreSQL 镜像在运行前解析为实际 image ID 并记录。

PostgreSQL 和 browser gate 均使用随机临时密码、独立私网和 tmpfs 存储，不发布宿主端口。普通 PG 的十二个模块为 `production_security_tests`（8）、`platform_settings_tests`（4）、`authorization_isolation_tests`（3）、`entity_creation_tests`（6）、`release_tests::zip_registration`（6）、`oauth_issuance_tests`（10）、`platform_session_refresh_tests`（4）、`login_protection_tests`（5）、`management_audit_tests`（8）、`notification_tests`（12）、`data_grant_tests`（7）和 `data_grant_discovery_tests`（3），当前合计 76 项。Phase19 在原 74 项上增加两个成员声明策略真实 PG 用例；前五组的 27 项、Phase 11 七组 41 项、[Phase 12 八组 46 项](../../docs/validation/phase-12.md) 和 [Phase 13 十组 64 项](../../docs/validation/phase-13.md) 保留各自历史证据。当前精确预期数量统一定义于 `manifest.mjs`，每个模块必须恰好执行声明数量，零测试或 ignored 都失败。普通 OAuth 组覆盖至少 20 并发单胜、消费/token 对/审计写入失败回滚、实时身份状态、旧存储兼容与独立撤销；psr_/app 的原有边界继续单独回归。协议见 [普通 OAuth 契约](../../docs/oauth-issuance.md) 和 [登录保护](../../docs/login-protection.md)，原阶段实际结果见 [Phase 14](../../docs/validation/phase-14.md)。日志文件名将 Rust 模块分隔符 `::` 转为 `-`，兼容 Windows。容器/网络清理先核对 owner label，清理失败也使 launcher 失败。

Linux 使用显式本机 Unix socket；Windows launcher 调用 Ubuntu WSL，可用 `CI_WSL_DISTRO` 指定发行版。交付 driver 使用 `PHASE03_WSL_DISTRO`，两者在 Windows 下都应指向本次隔离 Linux daemon。它们不会自动使用未知远程 Docker context。详细工具镜像及中断恢复说明见 [Linux 环境说明](../../deploy/ci/README.md)。

## 浏览器检查

完整 browser gate 必须通过上面的 Linux launcher 创建临时 PG。只运行十套隔离 HTTP fixture 时，可在已有固定 Node/npm 的环境中安装产品并使用独立入口；这不执行 live grant 闭环：

```sh
npm ci --prefix tests/browser
node tests/browser/install.mjs --with-deps chromium chrome msedge
node tests/browser/run.mjs --browser chromium --report /absolute/new-output/chromium.json
node tests/browser/run.mjs --browser chrome --report /absolute/new-output/chrome.json
node tests/browser/run.mjs --browser msedge --report /absolute/new-output/msedge.json
```

| 套件 | 每个产品的预期检查组 |
| --- | ---: |
| phase29-host（每次重建production产物） | 9 |
| micro-host | 47 |
| notifications | 10 |
| platform-settings | 11 |
| regions | 12 |
| entities | 13 |
| route-permissions | 64 |
| phase9-navigation | 14 |
| phase10-session | 17 |
| departments | 11 |
| chat | 1 |
| 合计 | **209** |

完整清单来自 [browser manifest](../../tests/browser/manifest.mjs)。项目只接受 `tests/browser/.browsers` 或固定 Linux 镜像层 `/opt/ci/browser/.browsers`，不借用 Codex/个人缓存或任意 executable override。Chrome、Edge 使用真实产品，不回退为 Chromium；Windows 使用已安装的品牌浏览器，安装器不会替换用户产品。

Firefox、WebKit 仅作显式扩展，可通过安装器安装，再使用 `tests/browser/run.mjs --browser firefox` 或 `webkit` 执行十套 fixture 套件。GitHub 手工输入可开启这些扩展；它们不代替三个必需浏览器，也不进入必需候选依赖图。WebKit 结果不表示 Safari 产品验收。更多说明见 [浏览器包说明](../../tests/browser/README.md)。

Phase 15 的 `data-grant-ui-live` 步骤在十套 fixture 后独立运行 `data_grant_ui_tests::browser_management_closes_grant_loop`，完整 gate 共 14 步。它通过实际浏览器页面向真实 Axum API 保存 SELF/ORG、处理冲突并撤销；同一普通 app token 的下一 context/members HTTP 请求重新取授权，再由既有工单后台的真实内省/context 与独立业务 PG 查询验证 SELF 仅返回本人 1 条、ORG 返回本组织 2 条、外组织记录始终不可见、撤销后下一请求 403。PG 只读核对四条 `DATA_GRANT_CHANGE` 的请求 ID 和 membership UUID。测试准备身份、权限与代表业务记录可用受控 fixture SQL，grant 变更只走页面或真实冲突请求；不以 SQL 代替页面保存。报告必须同时有精确 Rust 数量、16 个闭集检查、实际产品/版本、独立业务查询结果、审计匹配及两套数据库删除/连接池关闭证据，单独 exit 0 不能通过。脱敏预览 PNG 仅写到受控报告路径并校验 SHA-256，不保存 trace、headers 或 token。普通 PG 74 项不包含这个需要产品浏览器的 ignored 测试；既有 Node 工单后台的测试不表示未来 Rust 业务迁移已完成。实际执行范围和结果见 [Phase 15](../../docs/validation/phase-15.md)。

Phase 16 的 `work-orders-postgres` 使用 `business-apps/work-orders/backend/Cargo.toml --test migrations --locked`，显式执行 15 项 ignored integration tests 并要求零失败、零跳过。每项只使用本次 owner 的 PG、独立随机业务库；空库/旧 SHA-256 库、重复启动、Node/Rust 并发接管、结构与迁移记录漂移、历史数据及计数器保持分别验证。PG gate 先按锁为旧 Node initializer 安装 `pg`，仅用于迁移兼容测试驱动；Rust 服务启动和运行不依赖 Node。`workOrdersPostgresSuites`、`work-orders-isolated-postgres` 报告及 `work-orders-postgres` summary 步骤独立于原平台十二组 74 项，不能相互替代。实际范围与无 Node 容器证明见 [Phase 16](../../docs/validation/phase-16.md)。

Phase 18 保留原迁移 15 项，并将 `migrations` 当时必需数量扩为 21，覆盖不可变 `0002_create_idempotency.sql` 的 SQLx v1 升级、Node v2/混合接管、增量历史与 receipt catalog 漂移、失败回滚和并发升级。独立 `work-orders-write-postgres` 执行 `write_contracts` 的 14 项真实业务 PG/HTTP 测试；`work-orders-write-live-postgres` 另执行平台 bin 的 `phase18_work_orders_tests::real_platform_writes_and_idempotency`，要求 1 项 Rust 结果与完整 12 个命名检查及双库/进程清理证明。两者分别使用 `work-orders-write-isolated-postgres` 和 `work-orders-write-real-platform-postgres` 报告，不能替代 Phase17 原只读/联合报告或原平台 74 项。工作流只上传各目录的 `postgres-result.json` 与脱敏日志；创建幂等和未知结果恢复、授权变化、版本竞争、软删除、短事务预算及 Phase19 前拒绝非空处理人的实际结论以本阶段验证证据为准。

Phase 17 的 `work-orders-read-postgres` 以独立 `workOrdersReadPostgresSuites` 执行同一业务 crate 的 `read_contracts` integration target，报告类型为 `work-orders-read-isolated-postgres`。它通过真实业务 PostgreSQL 与显式平台 HTTP 协议夹具验证授权、统一 SQL 范围、CSV、CORS、参数边界及代表数据的实际查询计划；测试原始输出与计划保存在专属日志中。它不替代使用实际平台路由、平台库及独立 Rust 工单进程的联合 suite，也不改变原有平台 74 项或迁移 15 项的统计。所有必需 suite 要求预登记数量逐项匹配、零失败和零跳过。

独立 `work-orders-live-postgres` 先构建真实工单 Rust 二进制，再精确选择平台 bin 的 `phase17_work_orders_tests::real_platform_authorization_and_business_scope`。该 1 项 ignored 测试从真实平台路由签发应用 token，工单进程使用 Basic 内省与应用 Bearer context访问平台，并在单独业务数据库执行列表/count/detail/CSV；13 个命名检查证明四种有效范围、平台对普通租户的组织降级、错误受众、同 token 撤 grant、退出、移除 membership及平台失联。联合报告类型为 `work-orders-real-platform-postgres`；除精确 Rust 数量外，还要求完整命名结果、平台和业务库各自清理成功、独立业务进程退出。测试仅使用显式受管 loopback 开发 HTTP，不作为生产 HTTPS 验收；Node只承担受管测试编排。

前端 fixture 套件使用真实 Vite/React 页面、微应用字节以及隔离 API/Socket.IO fixture。它们证明对应 UI 行为；平台授权、数据库和传输协议各有独立检查，不能由浏览器结果推导为已通过业务权限或未来 Rust Socket.IO 迁移。

## 结果与失败判定

`run.mjs` 输出 `summary.json`、`source-manifest.json` 和 `logs/`。Summary 包含源码 revision、实际输入 SHA-256、工具版本、每步状态和计数，以及 `mandatoryFailureCount`、`mandatorySkipCount`。测试步骤分列 pass/fail/skip/cancelled/todo/filtered；安装、构建、静态检查是命令步骤，不伪造为测试数量。依赖失败后未执行的步骤为 `NOT_RUN`。

TAP 必须包含完整且非零的测试结果；选定 Cargo 测试必须真实执行，ignored 不能通过，未选择的 filtered 单列记录。浏览器每套必须达到自己的预期检查组数，PG 每组必须达到当前 manifest 的预期通过数量。无 DB 契约清单在 Phase 5 的 99 项之外，纳入 `micro-iframe-protocol.mjs`、`micro-sdk-client.mjs`、`micro-sdk-generation.mjs`、工单前端 client/app 单元和参考后台 CORS 的真实无 DB HTTP 回归，其中纯 Node 协议项不包含可选浏览器 skip；实际计数以当前报告为准。工单前端源码目录沿用私有配置和生成物排除规则进入 CI 快照，参考后台仅包含 CORS 的精确依赖清单，不引入未选择的旧业务数据库运行入口。Windows 的一次打包器选择中可能出现符号链接权限导致的 skip，这不能计作 Linux 门禁通过。命令超时、非零退出、报告缺失、必跑 skip/todo/cancelled、清理失败或结果不完整均会使对应 gate 非零退出。

Linux launcher 的外层报告是 `linux-runner-result.json`、`snapshot-manifest.json`，子 gate 位于 `reports/<gate>/`，PG 详细结果另在 `reports/postgres/postgres/`，独立工单迁移、只读契约及真实平台联合结果分别在 `reports/postgres/work-orders-postgres/`、`reports/postgres/work-orders-read-postgres/`、`reports/postgres/work-orders-live-postgres/`。报告记录 actual source bytes；`sourceRevision` 只表示 Git HEAD，未提交修改由 `inputsSha256` 和逐文件摘要绑定。导出的无 Git 快照需传入核实后的完整 `--source-revision`。

命令环境过滤运行时秘密，npm 使用本次输出目录中的独立 user/global 配置与缓存。数据库 URL、Bearer 和常见秘密字段在日志中脱敏。GitHub 候选检查 workflow 不读取外部发行或生产秘密，权限为 `contents: read`，checkout 不保留 token，没有外部 registry 登录、发布或目标主机操作。仅上传列出的脱敏报告和固定镜像 archive；不要上传整个 `work/`、`release/source/` 或 `release/smoke/`，smoke 包含临时测试 TLS 私钥。ZIP 回归只上传 `zip-release/result.json` 和受控 `zip-release/logs/`，不上传其源码快照或任何私有运行目录。通知 PG/browser 上传其 `notification-result.json` 与显式列出的脱敏驱动日志；镜像步骤上传 `notification-image/result.json` 和 `notification-image/logs/`，不得扩大为整个通知源码或运行目录。

本仓库现有 Rust 严格 fmt/Clippy 债务、历史默认测试组的 ignored DB/network 项仍是已知限制；它们没有被此 manifest 宣称为通过。当前 fast 执行明确列出的 Rust config、全部 archive 嵌套单元、staging maintenance、公共协议、OAuth 签发、登录 limiter、密码服务、client IP、transport 选择及全目标编译检查。后续阶段应按实际迁移结果接入新测试，不得为达到绿色状态添加空断言或将必需项改为 skip。

## OCI 候选与外部发行边界

通知镜像的单独复核入口如下；基础 `rust:1.97.0-slim-bookworm` 与 `debian:bookworm-slim` 必须已在该 Linux daemon 可用，driver 记录实际 image ID/RepoDigest 后再构建，不修改 daemon 或降低 TLS 校验：

```sh
node scripts/ci/notification-image.mjs --out-dir /absolute/new-output/notification-image --source-revision <verified-full-source-SHA>
```

受限网络可显式增加 `--build-proxy http://127.0.0.1:PORT`。`delivery` 自动传入本次源码 revision 和显式代理设置，在 `notification-image/result.json` 保存九项检查、输入 hash、候选本地 image ID、运行限制、退出和清理结果。集中结果校验同时拒绝缺项、重复检查、旧源码、root/可写/公开端口声明、强制退出与清理失败；命令退出零仍不足以通过。通知的候选保留在本机，当前不输出到平台的三个 OCI archive 集合，也没有 registry 发布。上传仅限该目录的 `result.json` 和脱敏 `logs/`，不上传 `source/`。

`delivery` 调用 `release.mjs --smoke`，在 `release/` 输出 `release-result.json`、`source-manifest.json`、三个 `images/*.tar` 以及本地日志/检查空间。镜像分别是宿主静态、独立 Nginx 入口、Rust 平台；每个镜像记录目标平台、完整 commit、buildId、输入摘要、Docker image ID、archive SHA-256、OCI manifest/config digest 和验证过的层数。Rust 平台另实际检查 UID 10001、Linux ELF 机器类型、二进制摘要与共享库解析，探测不启动服务或连接数据库。

真实 HTTPS smoke 覆盖非 root/只读 Nginx、私网 API 透传、polling/WebSocket、静态与安全响应头、上游换 IP 后恢复、ZIP 静态容器重建持久性和旧宿主 hash 资源保留。七组都必须通过，所需环境缺失直接失败。Rust transport fixture 只证明传输，不能当成完整 Socket.IO 兼容性结论。

随后 `delivery` 调用以下独立回归，复用刚验收的三个候选 image ID，不重建宿主或平台镜像：

```sh
node tests/zip-release-e2e.mjs --release-report /absolute/new-output/delivery/release/release-result.json --out-dir /absolute/new-output/delivery/zip-release
```

Driver 核对候选的源码摘要与当前快照一致，再启动私网临时 PostgreSQL、真实 Rust 平台、只读 Nginx ZIP 静态容器与 HTTPS 入口。九组必需检查覆盖实际登录及部署边界、上传响应丢失后的 GET 对账和重试、v1 预检/激活、同版本并发幂等、跨环境不可变提升、v2 激活/回滚及旧资源保留、不同内容并发冲突、中断上传与 staging 隔离、存在的内部 metadata 及编码路径 GET/HEAD 仍为 404/no-store。真实套件的 HTTP 请求中断与 Rust 安装器故障回归分别记录，不能互相代替。

结果在 `zip-release/result.json`，明确分列 pass/fail/skip/notRun、固定检查 ID 和清理状态；缺项或清理失败阻止交付 gate。临时 PostgreSQL/JWT/bootstrap 凭据和 CA 私钥通过 stdin 写入 Linux 自有 0700 私有目录，位于证据树之外，退出时核对所有权后清理。报告与受控日志经过脱敏，实际运行结果仍以本次报告为准。

`ociManifestDigest` 标识已保存并校验的 OCI 内容，不证明 registry 已存在这个产物；本地 `imageId` 也不能直接作为跨机器拉取地址。清单明确记录 `registryPublication: NOT_REQUESTED` 和 `targetDeployment: NOT_RUN`。生产 registry、域名、CPU 架构、主机与凭据仍需真实配置；当前 `linux/amd64` 候选不推断这些值。

以后受控发行应使用同一已验收 archive，推送后记录实际 registry RepoDigest，Linux Compose 拉取 `repository@sha256:...`，目标部署单独报告。构建成功、隔离 smoke/ZIP 回归成功、registry 发布和目标部署是不同状态。ZIP 安装与登记对账由当前平台实现，delivery 负责真实部署回归；完整业务后台迁移、监控和灾备仍各属后续台账阶段。

Phase 19 的 `work-orders-member-postgres` 独立执行 `member_contracts` 的 12 项真实业务 PostgreSQL/HTTP 回归；`work-orders-member-live-postgres` 选择 `phase19_work_orders_tests::real_platform_member_directory_and_assignment`，要求 1 项 Rust 结果、完整 10 个命名成员检查、双库删除/连接池关闭与独立进程停止。两者分别使用 `work-orders-member-isolated-postgres` / `work-orders-member-real-platform-postgres`，不能替代原迁移/读/写或 Phase17/18 联合回归。当前 fast 另显式执行 `routes::app_members::tests`，原 `data_grants::` 单元选择保留。成员目录只允许同应用显式登记的 `memberLookups` pair；旧 opt-in platform-e2e fixture 在真实 bootstrap 获得应用 UUID 后，以窄 ticket/create、ticket/update 声明重启平台，不放宽运行默认拒绝。各阶段实际运行数量与结果以其冻结候选和报告为准；本地选择这些业务及平台模块但排除独立 notification live 的执行必须注明 scoped PG，不能称为完整 postgres 门禁。

Phase 20 新增 `work-orders-reminder-postgres` 与 `work-orders-reminder-live-postgres`。前者精确执行 `reminder_contracts` 的 12 个 ignored 父测试，必须同时给出每测试独立业务库清理与四条确定性证明：未知发布保留同键/原正文且不自动重发、外部 POST 期间业务连接及行锁已释放、未确认 intent 跨进程重启后收件人变化同键冲突、准备事务提交被拒绝时零发布 POST。后者选择实际平台 bin 的 `phase20_work_orders_tests::real_platform_reminder_publication_and_recovery`，要求 1 个父测试和 12 个唯一命名检查，以及双库删除/池关闭、业务进程重启和最终停止证据；通过真实通知 HTTP 与持久收件箱核对结果，不启动实时服务。解析器不接受仅有 Cargo 通过数量的报告，也拒绝缺失、重复、false 或不完整标记。

提醒的持久事件身份新增不可变 `0003_reminder_intents.sql`，原 `0001`/`0002` 字节及 Cargo 依赖保持。当前迁移 target 的 27 项覆盖 fresh/v1/v2/v3 Node 与 SQLx 接管、既有 receipt/intent 保留、回滚和并发升级；历史 Phase16/18 的 15/21 项证据保留当时来源。本阶段结果以冻结候选的实际报告为准，不能将新注册数量或原生开发观察当成已执行的 Linux 门禁。提醒契约和原迁移/读取/写入/成员及其真实平台联合选择分别报告，独立 `notification-live` 仍由普通完整 postgres gate 执行。

Phase 26 的 `stores-postgres.mjs` 精确选择独立 `business-apps/stores/backend` 的 ignored `postgres_contracts` 父测试，`stores-live-postgres.mjs` 精确选择平台 bin 的 `phase26_stores_tests::real_platform_stores_and_cross_app_isolation`。两者都必须恰有 1 项 Rust 通过结果；8 个 SQL 检查与 16 个联合业务检查作为独立语义证据，不能冒充 Rust 测试数。SQL 报告须包含单库删除/池关闭；联合报告须包含门店、员工、日报、grant 与 grant 审计的实际行数，三个专属非超级用户账号互相连接被拒，三库/账号清理与门店、工单、平台进程停止。缺失、重复、skip、超时、错误计数或清理失败均拒绝。CI 构建 `target/ci-rust/debug/stores-api` 与 `work-orders-api`，只将计算所得产物路径和 owned PostgreSQL 配置传给联合测试，不继承产品凭据。workflow 仅上传两个专属目录的 `postgres-result.json` 与直属脱敏 `.log`。这是后端 HTTP/SQL 与账号隔离验收；Phase 27 浏览器和生产部署分别验证。

源码快照已纳入门店独立 Cargo 清单、锁、源码、迁移、测试、README 与公开 `config.example`/`policy.example.json`，同时绑定 `.gitignore` 的生成物排除设置。门店 `target/`、私有配置与秘密保持排除；不会把本地 ELF 或 SQL 数据当成源码输入。

Phase 27 将 `business-apps/stores/frontend` 纳入独立锁依赖安装、`build:all` 与必需 `stores-frontend-contracts`。`build:stores` 继续构建 Rust API，前端使用 `build:stores:frontend` / `test:stores:frontend`。构建必须显式提供公开 `STORES_ALLOWED_HOST_ORIGINS`；受管 CI 使用自己的测试宿主来源。完整 SDK 1.0.0 release 由 `sdk:sync` 生成到 `vendor/micro-sdk/`，运行时、引用声明与 manifest 一并受 `sdk:check` 校验；当前 all 选择共 44 项，历史阶段的数量保持原记录。

源码选择保留整个 stores 前端源码与 vendor，公开 `.npmrc` 只允许 `engine-strict=true`。门店 deploy 只显式选择 `bootstrap.manifest.json`、`bootstrap.schema.json`、`grants.manifest.example.json`、`grants.schema.json`，其余私有配置和构建目录不进入快照。

既有 `browser` gate 在独立 `stores-browser-live` 步骤执行 `scripts/ci/stores-browser.mjs --browser <product> --out-dir <fresh absolute directory> --source-revision <full SHA>`。它使用实际平台、工单和门店 Rust 二进制、Rust 管理 CLI、三个受管数据库与正常验证的 HTTPS/Nginx，结果类型为 `stores-real-platform-browser`；typed parser 要求全部具名检查及资源清理证据。普通 fixture runner 不接收 owned 数据库环境，也不承担这项验收。workflow 仅上传 `stores/result.json`、受控 `stores/logs/` 和 PNG 截图，不上传 TLS/NSS work 或数据库目录。注册入口与 parser 不等于已运行通过，实际结果以当次源码冻结和报告为准。

Phase 28 新增独立 `business-delivery` gate，由 [business-delivery.mjs](business-delivery.mjs) 管理本次隔离 registry、Compose、PostgreSQL 和 HTTPS 资源。它通过 [business-images.mjs](business-images.mjs) 构建工单与门店各自 v1/v2 静态镜像和两个独立 Rust API 镜像，并从同一归档执行真实 registry 推送/拉取、digest 固定与版本 URL 联合验证。普通 browser fixture 不接收这些数据库或发行环境；现有 `delivery`、平台/业务 PG 选择和三个产品 browser gate 继续各自执行。

集中报告只接受 `business-delivery-results.mjs` 的完整闭集检查、三个实际产品的业务报告、当前 `sourceRevision`/`inputsSha256`、健康失败时零路由/登记/激活调用，以及全部自有容器、网络、卷和私有目录清理证据。命令退出零、候选构建通过或登记接口 HTTP 200 均不足以通过。GitHub 公开证据精确包含专属 summary/source manifest、`business-delivery/result.json`、直属 `logs/*.log`、`screenshots/*.png`、三个 `browser-{chromium,chrome,msedge}/fixture-result.json` 原始报告、四个具名原始 ELF、四个版本的静态产物，以及发行 inventory 中仅 20 位数字文件名的公开 JSON 记录；`.lock` 不上传。全部验收成功后，独立 artifact 只包含 `candidate/candidate-result.json`、`candidate/source-manifest.json` 与 `candidate/images/*.tar`。禁止上传 work、source、私钥、数据库、registry 存储或登录配置。

干净 GitHub runner 在执行前构建 `deploy/ci/Dockerfile` 的 `browsers` stage，显式准备 registry 3.0.0、Nginx 1.28.0、Node 24.18.0、Debian runtime 与批准 digest 的 PostgreSQL，并通过 `CI_TOOLCHAIN_IMAGE` 传给专属 driver。driver 记录实际工具镜像 ID、Node/npm/Rust 版本和各基础镜像身份。Rust API 可从冻结源码在此 Linux 工具镜像内按锁构建，再校验源码摘要、ELF 架构及二进制 SHA 后装箱；报告中的 `apiBuildMode` 与 `prebuilt` 证据区分该路径和完整 Dockerfile 源码构建，不能由 verified ELF 装箱推断 builder stage 已执行。四个静态版本仍从冻结源码分别进行 Docker 构建。

公开源码精确允许 `deploy/compose.business.env.example`，并覆盖 `deploy/microapps`、business Compose 和两个 Rust API Dockerfile；其他 `.env`、运行时凭据和生成物仍排除。外部发行通过独立受保护手工 [business-publish.yml](../../.github/workflows/business-publish.yml) 消费同一已验收候选：先验证同仓库 main 的成功候选 run、精确 attempt 和业务/最终 gate，再核对原始归档并推送到明确配置的 registry。实际外部发行、目标部署和平台 URL 登记/预检/激活分别报告，本次隔离演练不代表生产发布。

Phase 30 授权链、受控成员协议、分页上限、预算参数与显式 legacy 回滚见[应用授权协议](../../docs/application-authorization-contract.md)。

Phase 31 在 fast 增加公共 retention 策略测试，在两个业务 ELF 构建后增加独立 `phase31-upgrade-retention-postgres` 步骤，精确执行平台旧库升级、只读预览/ZIP 保持/冻结旧宿主适配器、冻结旧门店与工单客户端三项测试；既有十二模块/76 项选择保持。该步骤拒绝零测试、ignored 或缺失实际结果及数据库清理证据。旧镜像实际 URL 的 HTTP/字节检查另列于 Phase31 受管验证记录，不从 registry 存在推断通过；生产政策和 apply 均未启用。

Phase33在fast为独立deploy/recovery crate增加fmt、locked test和严格Clippy，并增加recovery-results闭集结果测试。完整恢复为单独的隔离演练，入口scripts/ci/recovery-browser.mjs复用stores的实际HTTPS/三库/普通用户与生产前端控制面，仅选择Phase33 producer；它不由普通fixture或无环境的fast替代。

受管Linux演练须提供已有stores-browserEnvironment的三个Rust ELF/管理CLI、生产宿主dist、PG owner及TLS/NSS环境；另准备saas-recovery与saas-monitor ELF、PostgreSQL17客户端、findmnt/strip/tar、1GiB tmpfs /scratch，以及独立无共享卷的HTTPS不可变对象存储。测试专用远端配置仅存在/remote-private.json和/remote-ca.pem，密钥不会复制进远端对象存储。外层owner监听有限PHASE33_BUILD_IMAGES_READY/PHASE33_RESTORED_IMAGES_READY握手，按导出的两应用v1/v2构建/保存四镜像，再从恢复出的归档加载、启动并写accepted结果；内层没有Docker socket、主机端口或后台Docker权限。具体本轮owner、driver源码与校验和、基线ELF来源/源码等价、镜像归档、容器/网络清理见Phase33运行索引。

结果必须包含十个唯一具名检查、六类负向、所有旧凭据拒绝和恢复审计、四实际镜像加载/启动、四个实际浏览器版本、每段RPO/RTO与清理。缺项/跳过/生产指标冒认均失败。只归档脱敏报告、日志、截图、公开静态/镜像和实际ELF；不归档PG数据、恢复密文、私有配置、TLS私钥或测试key escrow。物理异机、外部告警、生产timer和15min/1h PITR另行验收。
