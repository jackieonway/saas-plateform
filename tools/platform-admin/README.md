# Rust 平台管理 CLI

`saas-platform-admin` 是独立 Rust crate，只通过现有平台管理 API 登记工单和门店应用、执行明确授权及管理 URL 发布版本。它不依赖平台实现 crate，使用自己的 Cargo.lock，不要求仓库根 Cargo workspace。旧 `business-apps/work-orders/deploy/platform-admin.mjs`、`platform-admin-core.mjs`、`grants.mjs` 与旧 Node 管理测试已退役；前端构建与打包的 Node 工具继续保留。

依赖仅用于必要边界：`reqwest` 关闭默认 feature，使用 blocking/json/rustls 做受控 HTTPS；`serde`/`serde_json` 校验闭合清单和 API JSON；`chrono` 校验成员与组织的实际有效期。独立锁文件冻结传递依赖，不引入通用 CLI 框架或数据库驱动。

## 构建与命令

在仓库根目录构建和验证：

```text
cargo build --manifest-path tools/platform-admin/Cargo.toml --locked
cargo test --manifest-path tools/platform-admin/Cargo.toml --locked
cargo fmt --manifest-path tools/platform-admin/Cargo.toml -- --check
cargo clippy --manifest-path tools/platform-admin/Cargo.toml --locked --all-targets -- -D warnings
cargo doc --manifest-path tools/platform-admin/Cargo.toml --locked --no-deps
```

产物为 `tools/platform-admin/target/debug/saas-platform-admin`，Windows 加 `.exe`。下文假设已将该受控产物加入 PATH；也可直接使用其绝对路径。安装时只需分发当前平台的二进制，管理命令无需 Node 运行时。

| 命令 | 行为 |
| --- | --- |
| 无参数或 `plan` | 显示内嵌 bootstrap 本地计划，不联网、不读取网络配置 |
| `inspect` | 读取并核对选定应用、指定 OAuth 客户端及精确菜单 |
| `bootstrap [--apply]` | 默认离线；显式 apply 才创建缺失应用/菜单、关联现有未绑定客户端 |
| `grants-plan --manifest FILE` | 严格校验清单，输出离线授权计划 |
| `grants-apply --manifest FILE [--apply]` | 默认离线；显式 apply 才通过正式 API 执行已批准授权 |
| `register --release-file FILE [--apply]` | 默认校验本地 URL 发布清单；显式 apply 登记 DRAFT |
| `releases` / `deployments` | 读取本应用目标环境的安全发布摘要或部署指针 |
| `retention-preview [--minimum-age-days 30] [--keep-latest 5]` | GET 读取真实引用；候选保护期只能加严，拒绝 apply；无删除能力，见 `docs/upgrade-and-retention.md` |
| `preflight --release-id UUID [--apply]` | 显式 apply 才提交平台预检；不会自动启用 |
| `activate --release-id UUID [--apply]` | 显式 apply 才启用已 PASSED 版本；平台仍重新预检 |
| `rollback [--apply]` | 显式 apply 才请求平台重新预检并切换到上一版本 |

共同参数为 `--app-code work-orders|stores`、`--environment NAME`（默认 `production`）、可选 `--app-id UUID` 和 `--timeout-ms 10..35000`（默认 `35000`）。内置应用集合只有这两个值，未知应用拒绝；bootstrap、inspect 和发布命令省略 app-code 时保持工单兼容行为。授权命令以清单 `applicationCode` 为来源，显式 app-code 必须与之相同，显式 app-id 也须匹配清单 applicationId，冲突在读取凭据或联网前拒绝。整条远程命令的所有 HTTP 请求共享该总预算，不因后续请求重新计时。未知/重复参数、无效组合、秘密参数及旧 `--sql-out` 均拒绝。只读命令和 `grants-plan` 不接受 `--apply`。

## 环境与秘密

CLI 不登录、不加载 `.env`，也不枚举整个进程环境。凭据仅由受控进程环境或 secret store 注入，不能放入 argv、清单、前端或版本目录。

| 环境变量 | 用途 |
| --- | --- |
| `PLATFORM_API_ORIGIN` | 精确平台 origin，例如 `https://saas.example.com`；不允许路径、query、fragment 或用户名/密码 |
| `PLATFORM_ADMIN_BEARER` | 已登录管理员的当前平台 session Bearer；不接受应用 token 或客户端 secret 代替 |
| `PLATFORM_CLIENT_ID` | inspect/bootstrap 必需，必须是平台生成的实际 clientId |
| `PLATFORM_CA_FILE` | 可选公开 PEM CA 链文件，最大 64 KiB，仅接受 CERTIFICATE 块；保持证书及主机名验证 |

离线计划不读取这些配置。远程管理请求固定在同一 origin，使用 reqwest/rustls，拒绝环境代理、redirect 和自动重试，JSON 响应最多 1 MiB。正常 TLS 校验始终开启；CLI 不隐式读取 Node 的 `NODE_EXTRA_CA_CERTS`，不接受私钥或关闭验证的参数。

HTTP 仅限明确的开发回环：每条命令同时指定 `--environment development --allow-loopback-http`，host 必须是字面 `localhost`、`127.0.0.1` 或 `[::1]`。改环境名或解析到私网/回环的其它域名均不能放宽规则。

成功和失败摘要只投影允许字段；远程总出口还递归拒绝当前 Bearer 出现在合法字符串字段中的回显，返回固定 `SENSITIVE_OUTPUT_REJECTED`。写命令出现此情况仍保留 `outcomeUnknown` 和强制对账提示，不能把输出被拒绝解释为平台没有写入。

管理员先在平台 UI 创建名称 `work-orders-client` 或 `stores-client` 的对应 CONFIDENTIAL 客户端，grantTypes 仅 `client_credentials`、scopes 仅 `openid`、redirectUris 为空、accessTokenValidity/refreshTokenValidity 均为 300 秒。保存平台一次性返回的 secret 到业务服务 secret store，并记录平台生成的实际 ID；名称不能代替 clientId。CLI 不读取 `PLATFORM_CLIENT_SECRET`，不创建、轮换或自动重置客户端，不回显 secret。

## Bootstrap 与七个菜单

[bootstrap.manifest.json](manifests/bootstrap.manifest.json) 在编译期内嵌，CLI 不接受外部 bootstrap 路径。它与[公开兼容 source](../../business-apps/work-orders/deploy/bootstrap.manifest.json)同步；[bootstrap.schema.json](schemas/bootstrap.schema.json) 定义完整字段和固定菜单顺序。

| key | 类型与路径 | 可见性与权限 |
| --- | --- | --- |
| `root` | MENU `/micro/work-orders` | 可见，空功能权限 |
| `ticket:read` | MENU `/micro/work-orders/tickets/:id` | 隐藏，`ticket:read` |
| `ticket:create` | MENU `/micro/work-orders/ticket-create` | 可见，`ticket:create` |
| `ticket:update`、`ticket:delete`、`ticket:export`、`notification:publish` | BUTTON，空路径 | 隐藏，各自同名权限 |

应用固定为 `work-orders`、IFRAME + HTML，`autoCreateMenu:false`，默认 grants 为空。详情与创建 MENU 分别授权；根菜单、读权限或 data grant 都不会隐含创建权限。旧 `/tickets/new` 属于详情标识，不是创建别名。

```text
saas-platform-admin
saas-platform-admin bootstrap --environment production
saas-platform-admin inspect --environment production
saas-platform-admin bootstrap --environment production --apply
saas-platform-admin inspect --environment production
```

Bootstrap 只复用精确匹配的已有应用/客户端/菜单，创建缺项；重复执行不增加默认授权，不抢占其它应用的客户端或菜单。客户端已绑定其它应用、同名重复、菜单类型/路径/父节点/权限不符或目标应用冲突时拒绝，不通过删除/重写已有对象修复。默认占位入口 `/sub-apps/work-orders/unreleased/index.html` 应保持 404，实际内容使用不可变发布版本。

这些 API 没有跨应用/OAuth/菜单事务或菜单唯一键，客户端二次读取也不构成原子比较更新。必须在单实例、暂停并发管理的窗口执行。失败后可能留下已创建项；先 inspect 和核对平台状态，再审阅下一次命令。

## 显式授权与正式 grant API

复制 [grants.manifest.example.json](manifests/grants.manifest.example.json)，替换全部占位 UUID 并按批准目标填写 `targets`。占位符刻意无效，`targets` 省略或空数组表示没有授权意图。[grants.schema.json](schemas/grants.schema.json) 拒绝未知字段，并固定七个 menuIds key。Rust 额外检查不同菜单 UUID、直接 membership/用户组织重复、角色归属一致性以及 resource/action 重复；JSON 重复属性也拒绝。

每个 target 明确包含 `tenantId`、`organizationId`、`roleId`、`userId`、`membershipId`、`permissions`、`dataGrants`。功能权限允许六个同名工单/提醒动作；数据项明确为 `{resource, action, scope}`，scope 为 `SELF | ORG | ORG_SUBTREE | TENANT`。两者独立，不补齐任何动作或 scope；最终还须符合平台登记能力、目标组织 ceiling 与 `DATA_GRANT_POLICY`。

```text
saas-platform-admin grants-plan --manifest path/to/approved-grants.json
saas-platform-admin grants-apply --manifest path/to/approved-grants.json
saas-platform-admin grants-apply --manifest path/to/approved-grants.json --apply
```

前两条完全离线，输出明确租户/组织请求、角色当前值并集要求，以及数据 grant 的实时读取、预览、保存顺序。最后一条先核对真实应用/菜单及租户、组织、角色、用户、已有直接 membership，再调用正式 API。CLI 不创建/删除/替换 membership。菜单 API 未提供 status 字段；CLI 核对精确归属、类型、路径、权限、父节点和打开方式，数据 grant 与受保护业务请求继续由平台校验有效菜单。

租户/组织授权仅提交明确项目，但平台范围一致性清理可能移除已有不一致项。角色菜单/用户 PUT 是完整集合替换，CLI 读取当前集合后取并集并核对保存结果；没有 CAS，所以必须暂停并发管理，并审阅共享角色所有现有成员受新增权限的影响。优先使用专用角色。

数据授权始终经 `GET /api/data-grants` → `POST /api/data-grants/preview` → `PATCH /api/data-grants`，预览和 PATCH 使用同一次读取的 opaque version。此 CLI 为增量授权：相同 resource/action 的同 scope 保留，缺项新增，已有不同 scope 返回 `GRANT_SCOPE_CONFLICT`；scope 更新和撤销另由平台 UI/API 审阅。未列出的 action 保留，remove 为空，不假定初始版本、不默认授予 `TENANT`。

## 门店应用与十一菜单

门店使用独立内嵌 [stores-bootstrap.manifest.json](manifests/stores-bootstrap.manifest.json) 及 [stores-bootstrap.schema.json](schemas/stores-bootstrap.schema.json)，与 [stores 公开接入目录](../../business-apps/stores/deploy/README.md) 中对应文件逐字节一致。原工单七菜单及两份原 schema 保持兼容。门店固定为 `stores`、`/micro/stores`、HTML/IFRAME，初始入口 `/sub-apps/stores/unreleased/index.html`。

门店十一项 key、顺序、路径、权限及可见性由 schema 固定。其中 `store-daily-report:read` 与 `store-daily-report:detail` 是两条独立 MENU，共享 `store-daily-report:read` permission；授权该 permission 时会选入两条菜单，detail 不是新的数据动作。bootstrap 用非空 MENU 路径识别各菜单，因此重复执行不会把共享 permission 误判为重复对象。三项 BUTTON 为空路径、BLANK、隐藏；其余 MENU 为 MICRO，所有子项 parentKey=root。

```text
saas-platform-admin plan --app-code stores
saas-platform-admin bootstrap --app-code stores --apply
saas-platform-admin inspect --app-code stores
saas-platform-admin grants-plan --app-code stores --manifest path/to/approved-stores-grants.json
saas-platform-admin grants-apply --app-code stores --manifest path/to/approved-stores-grants.json --apply
```

复制 [stores-grants.manifest.example.json](manifests/stores-grants.manifest.example.json)，填入实际应用、十一菜单及目标 UUID；占位符故意不能执行。门店只允许九个 resource/action：store 的 read/create/update、store-staff 的 read/assign/unassign、store-daily-report 的 read/create/update。每个应用单独校验自己的权限和 grant 白名单，不能把另一应用的动作写入新意图。读取现有 grant 时仍保留未列出的历史 resource/action，包括已停用能力；这些行不视为污染，不会被清洗或删除。

资源能力经菜单 permission 与平台既有 `DATA_GRANT_POLICY` 登记。将后端 [policy.example.json](../../business-apps/stores/backend/policy.example.json) 的九个 capabilities 和三个 memberLookups 精确 pair 合并进管理员实际配置，保留其它应用与已批准 managementScopes，替换实际应用 UUID；不引入资源登记 API、STORE scope 或新的 summary 动作。成员用途仅 store/create、store/update、store-staff/assign。

门店 manifest 的 `status:ENABLED` 声明预期可用性，`linkUrl` 固定为空。现有 MenuBody/menu_json 没有 status 字段，CLI 不向菜单 API 提交 status，也不伪造读取结果；它核对归属、父节点、类型、路径、permission、可见性、打开方式、linkUrl 与 sortOrder。实际授权和业务请求还须通过正式 grant API 及平台有效菜单链验证。

GET 需要独立 `system:data-grant:read`；preview 需要 read + write，PATCH 需要独立 write，两个变更入口同时要求实际平台根上下文。宽泛管理员 session、用户/角色/菜单管理权或操作者自己的业务 grant 均不能代替服务端策略。`DATA_GRANT_POLICY` 默认空拒绝，必须显式登记应用能力与允许管理的租户/组织范围。

整次授权不构成跨 API/跨目标事务。功能授权可能成功而数据 grant 随后失败；错误的 `progress.completed` / `attempted` 仅作对账提示。409、超时、连接/响应丢失或其它不确定结果都先读取实际授权、角色集合、目标 grant 及审计，再重新审阅，不能盲目重试或自行删除回滚。独立数据范围最终用指定组织内的真实非 super 用户和下一次受保护业务请求验证。详见[正式管理契约](../../docs/data-grant-management.md)。

## 独立 URL 发布

前端继续使用既有 Vite/Node 构建与 `scripts/package-microapp.mjs` 生成不可变 `release.json`。CLI 只接收 [release.schema.json](schemas/release.schema.json) 的 `environment/version/entryType/entryUrl/healthUrl/resources` 字段，`healthUrl` 可省略或为 null，resources 仅 `url/kind` 且最多 48 项；禁止未知字段、凭据 URL 和跨版本资源。

```text
saas-platform-admin register --environment production --release-file path/to/release.json
saas-platform-admin register --environment production --release-file path/to/release.json --apply
saas-platform-admin releases --environment production
saas-platform-admin preflight --environment production --release-id <release UUID> --apply
saas-platform-admin activate --environment production --release-id <release UUID> --apply
saas-platform-admin deployments --environment production
saas-platform-admin rollback --environment production --apply
```

同环境版本已存在返回 `VERSION_EXISTS`，不覆盖不可变版本；跨应用 release、环境或部署指针不符也拒绝。预检 HTTP 200 但平台结果 FAILED 返回 `PREFLIGHT_FAILED`。登记不自动预检/激活；切换不修改应用身份、客户端、功能权限或数据 grant。工具不安装 ZIP、不复制版本字节、不操作镜像卷、不执行 SQL、不控制 Docker，URL 登记无需 ZIP 安装前提。完整工单步骤见[部署指南](../../business-apps/work-orders/deploy/README.md)。

门店发布命令显式加 `--app-code stores`；发行 JSON 保持相同字段，不添加 applicationCode、image、digest 或 target。CLI 将 entryUrl/resources 约束到 `/sub-apps/stores/versions/<version>/`；工单仍约束自己的同构前缀。发布来源由 CLI 选定应用决定，不能从任意 URL 猜测应用。版本路由仍由外部部署模板维护。

## 持久产物声明与受控登记

推荐镜像与外部 HTTPS 静态发行使用独立 [artifact.schema.json](schemas/artifact.schema.json)，参考 [artifact.manifest.example.json](manifests/artifact.manifest.example.json) 替换全部故意无效的占位值。清单记录选定 applicationCode、精确 publicOrigin、来源、SHA256 digest、完整 commit、buildId 和实际 inputsSha256。OCI_IMAGE 来源使用固定 `registry/repository@sha256:<64hex>` 与 `linux/amd64|linux/arm64`，digest 必须与引用相同；不能使用 Docker config/image ID 或归档文件 SHA 代替 distribution manifest digest。STATIC_ARTIFACT 来源使用无凭据、无 query/fragment 的固定 HTTPS 产物坐标，省略 targetPlatform，其 digest 由 CI 核验实际规范资源清单。CLI 离线保存声明，不向 registry 发请求或读取 Docker health。

```text
saas-platform-admin artifact-bind --app-code stores --environment production --release-file /absolute/release.json --artifact-file /absolute/artifact.json --inventory-dir /durable/artifact-inventory
saas-platform-admin artifact-bind --app-code stores --environment production --release-file /absolute/release.json --artifact-file /absolute/artifact.json --inventory-dir /durable/artifact-inventory --apply
saas-platform-admin artifact-verify --app-code stores --environment production --release-file /absolute/release.json --artifact-file /absolute/artifact.json --inventory-dir /durable/artifact-inventory
saas-platform-admin register --app-code stores --environment production --release-file /absolute/release.json --artifact-file /absolute/artifact.json --inventory-dir /durable/artifact-inventory --apply
saas-platform-admin release-reconcile --app-code stores --environment production --release-file /absolute/release.json --artifact-file /absolute/artifact.json --inventory-dir /durable/artifact-inventory
```

新产物路径必须是绝对路径，拒绝符号链接、Windows reparse point/网络路径、非普通输入文件和输入与 inventory 重叠；新 inventory 的父目录须已存在。bind 默认完全离线且不创建目录/文件，只有 `--apply` 写本地清单；verify 只读且不接受 apply，两个本地命令均不接受 app-id。每份 JSON 最大 1 MiB，未知及重复字段拒绝。所有旧发布命令与六字段 RegisterBody 保持兼容；旧 `register --release-file` 没有产物参数时仍可使用，但**不验证 digest**。生产 CI 应使用 artifact-file/inventory-dir 成对参数的受控 register，它在任何 HTTP 请求前验证已存在的精确绑定，提交的请求仍只有旧 URL 字段。输出明确 `artifactBindingVerified` 与 `remoteDigestStored:false`，不能理解为平台已经保存或验证镜像摘要。

持久目录中的 `.lock` 永久保留，OS 锁串行化遵守此协议的本机写者。锁内重放闭合的连续序号日志，再用本次独占的临时普通文件、sync_all 和同文件系统 hard_link 发布一条完整记录；最终序号文件永不覆盖。记录数上限 4096、最终文件总量上限 64 MiB。不同 environment 可有不同公开 origin，但全局 `(app, version)` 固定来源 kind/identity/架构、digest、commit、buildId 和 inputsSha256；同 `(app, 规范公开版本URL)` 固定完整产物与 URL 定义。相同绑定幂等，换源/换 digest 或同环境改入口/资源/health 返回 `ARTIFACT_CONFLICT`，不修改旧记录。

进程中断只留下旧完整历史或完整追加记录；符合本协议名称的遗留 pending 普通文件不算提交，也不会被其它命令删除。未知名称、断号、坏最终记录、链接和矛盾历史都失败关闭。锁繁忙立即返回 `ARTIFACT_STORE_BUSY`，OS 会在进程退出后释放锁，不删除锁文件恢复。Unix 同步目录及新根的父目录；Windows 不承诺未经验证的断电持久性。清单根必须是受管可信本地持久存储，所有环境共享同一权威历史；不同宿主的私有副本、临时 runner、过期 Actions artifact/cache 和未验证的网络文件系统不能提供全局保证。

绑定只保留不可变发行意图。CI 在放行公开版本路由前绑定，独立核验 registry/字节和容器健康，完成入口路由及公开 HTTPS 检查后再登记、预检、激活。失败不释放版本给另一份产物；只在对账后恢复同一候选。CLI 不启动容器、修改路由、隐式重试或自动激活。

register/激活结果未知时保留既有对账标志。`release-reconcile` 是只读远程命令，不接受 apply；比较完整环境、版本、HTML 入口、规范 URL、resources 和 healthUrl，返回 `MATCHED` 或 `ABSENT`，不一致返回 `RELEASE_DEFINITION_CONFLICT` 与安全 `CONFLICT` 摘要。带成对产物参数时先验证本地绑定，并用声明的 publicOrigin 解析本地预期；省略产物参数时，本地 entryUrl、非空 healthUrl 都必须是绝对 URL。远端实际 entryUrl 和非空 healthUrl 在两种模式下一律必须绝对，相对 resources 可按已证明的绝对入口解析。管理 API origin、静态入口 origin 和后端 MICRO_GATEWAY_ORIGIN 可各不相同，本地 manifest 也不能证明服务器的 gateway 设置；歧义返回 `RECONCILIATION_ORIGIN_REQUIRED`。本地歧义在联网前拒绝，应提供经批准并绑定的产物参数；远端歧义须核对服务端网关配置及实际完整定义，不能据此宣称 MATCHED 或盲目重试。guarded register 提交绝对字段，但响应若歧义仍保留 outcomeUnknown。原始和规范发送的 URL 均不得超过后端的 2000 字符门禁，无效定义在绑定创建目录前拒绝。该对账仍不证明远端字节 digest；再用 deployments 核对 active/previous，CI 独立核对实际产物与部署证据。

## 错误与验证边界

成功输出为安全 JSON 摘要；失败只输出固定 `category`、可选 HTTP `status` 与必要的已核对 UUID/进度，不回显 URL、请求参数、Bearer、secret、原始正文、PEM 或异常堆栈。常见类别包括 `INVALID_ARGUMENT`、`INVALID_ORIGIN`、`HTTPS_REQUIRED`、`INVALID_CA_CONFIGURATION`、`HTTP_ERROR`、`REQUEST_TIMEOUT`、`APPLICATION_CONFLICT`、`CLIENT_OWNERSHIP_CONFLICT`、`MENU_CONFLICT`、`GRANT_SCOPE_CONFLICT`。401/403 应核对会话及独立权限；归属冲突不能通过覆盖修复。

可能已提交的写入会标记 `outcomeUnknown` / `reconciliationRequired` 和 `INSPECT_AND_RECONCILE_BEFORE_RETRY`。这些标志不表示回滚。bootstrap 后先 inspect，发布后先 releases/deployments，授权后先读取精确目标与审计；确认实际状态后才能决定新的显式命令。

Cargo 测试覆盖独立二进制、离线行为、受控 HTTP fixture、严格输入、冲突和秘密脱敏；`tests/platform-admin-contract.mjs` 仅校验公开 schema/manifest 与前端路由契约，不代替 Rust 运行时校验。真实平台/PG 与生产 HTTPS、业务联合结果分别以阶段验收证据为准。既有工单联合测试必须显式设置当前预构建二进制的绝对路径 `PLATFORM_ADMIN_BIN`（或 `PHASE25_CLI_BIN`），不自动构建也不回退旧管理实现。
