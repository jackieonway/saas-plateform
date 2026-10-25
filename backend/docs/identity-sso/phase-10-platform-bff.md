# Phase 10 平台 BFF、实际消费者与协议兼容边界

状态：**COMPLETED**。依赖 [Phase 9](validation/phase-09.md) 已完成。Rust修复串链完整PASS：静态六项／195普通、定点14／14、BFF新14／14、既有212／212及rollout3／3；最终再扩既有policy测试，另一双类型client不得扩大本RP准入的实际PG负例1／1、7.63秒通过，不增加为第15组。runtime修复后前端round7为505／505，lint0错误／46既有warning、typecheck／build PASS，受影响qiankun4组真实Edge通过。最终a62e…普通binary配置66／66实际PASS，真实driver10／browser9均PASS：浏览器98条检查记录含辅助记录，业务计数7／12／13，真实续期、跨tab切组织撤旧Bearer／unmount、outage503与恢复、PLATFORM／GLOBAL注销、memberless、不同账号隔离、无pageerror和Storage均通过，9张最终PNG逐张核验PASS。498文件最终snapshot和不变量审计已刷新，五项Self-Check全部PASS。TASK_OWNED验收不代表生产部署或默认开关开放。历史失败、修正与范围见 [Phase 10 验收](validation/phase-10.md)。

## 当前范围与认证 owner

[台账 Phase 10](task-ledger.md)要求平台以自己的 confidential OIDC client 完成 code＋S256 登录，由服务端保存 OIDC token 和平台 refresh，再按明确的有效员工 `membership_id` 建立平台 context。浏览器只获得防护 Cookie 与最长 300 秒、仅放内存的平台 context Bearer，用于现有 HTTP、qiankun 与 socket 合同。无成员账号可以完成账号认证，不自动创建成员、组织或管理员权限。

本阶段不创建客户门户或独立身份服务，不导入历史 refresh。认证账号与密码验证继续使用 [Phase 7 账号 port](phase-07-authentication-boundary.md)及 [Phase 8 全局 SSO session](phase-08-global-session.md)的同一权威 owner。平台 context、成员／应用策略和 legacy 应用凭据仍归平台 owner；新 OIDC grant、code 和 refresh 归认证 provider，不能因同进程或同数据库就共用凭据命名空间。

## 实际消费者清单

以下保留 Phase 10 开始、BFF 替换宿主前的消费者盘点，作为兼容决策依据；新宿主按服务端模式分别进入 BFF 或显式 legacy 分支。表中原始调用点不代表本机或生产环境已部署相应服务。未读取私密环境、凭据、keyset 或 token fixture。端点存在、测试支持、配置登记和实际运行时调用分别判断。

| 消费者 | 当前真实调用与源码证据 | Phase 10 处理边界 |
| --- | --- | --- |
| 平台活动 frontend | [`api/auth.ts`](../../../frontend/src/api/auth.ts)的登录与成员选择调用 `/api/auth/login`、`/api/auth/context`；[`lib/api.ts`](../../../frontend/src/lib/api.ts)以 `grant_type=refresh_token`、`psr_` 平台 refresh 调用 `/oauth/token`，401、切组织与 logout 复用刷新协调。当前不是 `/api/auth/refresh`。 | 迁入 BFF Cookie、CSRF 与内存短 context Bearer。替换旧宿主实现不能自动成为保留浏览器 refresh 或制造 refresh→SSO 兑换桥的理由；必要兼容路径以最终调用点与默认开关单列。 |
| 固定 SDK 3.0.0 | 宿主及 release 的 package 均为 3.0.0；[`context-client.mjs`](../../../../saas-micro-app-sdk/src/context-client.mjs)第 8～44 行消费宿主 token／更新；[`http.mjs`](../../../../saas-micro-app-sdk/src/http.mjs)第 107／112 行强制 Bearer 与 `credentials:'omit'`。相同实现见 [`releases/3.0.0/sdk.js`](../../../../saas-micro-app-sdk/releases/3.0.0/sdk.js)第 297 行起，无用户登录或 refresh 请求。 | 保持 SDK、MicroProps、Context 与 Notification wire contract；宿主负责取得和续期短 token，不能把 OIDC token 或 HttpOnly Cookie 放进 SDK `token`。 |
| 工单正式 Rust API | [`authorization/client.rs`](../../../../saas-work-orders/backend/src/authorization/client.rs)第 82～139 行：legacy 模式逐次 `/oauth/introspect`＋`/api/apps/work-orders/context`，combined 模式 `/api/apps/work-orders/authorize`。平台 client ID／secret 用于当前请求认证；正式 API 无 `/oauth/token` 调用。Node 文件仅为已退役运行时的兼容材料，见其 [README](../../../../saas-work-orders/backend/README.md)。 | 保留 app token、introspection／authorize 和现有服务端授权 1.0 合同；不为工单伪造人类 password/code/device/refresh 需求。服务客户端的 `client_credentials` 登记不等于业务 API 已兑换机器 token。 |
| 门店正式 Rust API | [`authorization/client.rs`](../../../../saas-stores/backend/src/authorization/client.rs)第 82～139 行同样使用 `/oauth/introspect`＋`/api/apps/stores/context` 或 `/api/apps/stores/authorize`；额外读取受控组织与成员目录，无登录或 token 兑换调用。 | 保留人员 app token、精确应用／组织／成员证明和原授权 1.0 合同；不把机器登记扩大成人员准入。 |
| 工单／门店 qiankun frontend | 两仓 [`工单 vendor package`](../../../../saas-work-orders/frontend/vendor/micro-sdk/package.json)与 [`门店 vendor package`](../../../../saas-stores/frontend/vendor/micro-sdk/package.json)均为 3.0.0，lifecycle 从 `props.platform` 接收宿主上下文；例如 [`门店 domain.ts`](../../../../saas-stores/frontend/src/domain.ts)第 64 行校验MicroProps 2.0、可选sdkVersion 3.0.0与token／期限。 | 继续消费宿主派生的app token，不自行登录或refresh；SDK包3.0.0、MicroProps 2.0、业务Context 1.0分别保持，不能混淆包版本与wire合同版本。 |
| 独立 notification 服务 | [`platform.rs`](../../../../saas-notification/src/platform.rs)第 66 行构造 `/api/internal/notifications/context-proof`；[`server.rs`](../../../../saas-notification/src/server.rs)第 393 行接收严格 `authenticate({token})`，服务不登录、不签发或刷新 token。 | 短 context Bearer 继续驱动 HTTP notification context 与 socket 认证、到期重连和实时撤权；公开五字段 authenticated／十二字段 notification 载荷不改。 |
| 平台 `platform-admin` CLI | [`http.rs`](../../tools/platform-admin/src/http.rs)第 56～71 行只输入 `PLATFORM_ADMIN_BEARER`，第 141 行限制请求为 `/api/*`；CLI 不登录、不读取 password、不刷新或兑换 OAuth。 | 保持当前管理 Bearer 与权限检查；可以消费有足够剩余期限的短 context Bearer。CLI 自身不是旧 refresh grant 的保留依据，其现有整条命令预算仍须遵守 token 到期。 |
| 工单／门店机器客户端登记 | 两仓 [工单 manifest](../../../../saas-work-orders/backend/manifests/bootstrap.manifest.json)、[门店 manifest](../../../../saas-stores/backend/manifests/bootstrap.manifest.json)及 [CLI bootstrap](../../tools/platform-admin/src/bootstrap.rs)只允许机器 `client_credentials`，成员允许集合为空表示机器不适用人员身份。 | 保留机器协议与 client secret 认证边界，不产生员工／客户 context。清单不是实际业务请求量、生产登记或旧人类凭据的证明。 |
| OAuth playground 演练 BFF | [`http.rs`](../../../../saas-examples/oauth-playground/backend/src/http.rs)实现旧 code、device、client_credentials、password 与 refresh；[`config.rs`](../../../../saas-examples/oauth-playground/backend/src/config.rs)按可选专用 client 广告能力，password 另有默认 false 的 `OAUTH_PASSWORD_ENABLED`。 | 这是明确的样例消费者，按环境配置和登记 grant 显式开启，不强制为平台业务常开全部旧 grant。后续新 provider/RP 路径与该 legacy 演练分开；不得把样例实现当作生产迁移窗口。 |

普通业务 API、SDK 与通知服务没有 `/api/auth/refresh` 调用；当前平台实际 refresh endpoint 是同路径 `/oauth/token` 中按命名空间区分的两个 owner：`psr_` 平台 session refresh 与经过 client 认证的 legacy OAuth refresh。OIDC refresh 为独立 `/oidc/token`，三者不能互换。

宿主改造必须同时覆盖登录、切组织、普通请求 401 和 logout。盘点时 [`session-coordinator.ts`](../../../frontend/src/lib/session-coordinator.ts)第 23／115／269 行的 record 包含平台 token pair 并写 `localStorage`；这部分凭据持久化不能在 BFF 模式保留。跨页版本／取消协调可复用，但短 context Bearer 只能留内存。实际应用派生调用见 [`MicroAppContainer.tsx`](../../../frontend/src/components/micro/MicroAppContainer.tsx)第 334／338 行：先请求 `/api/oauth/session-token`，再用返回的 app token 读取 app context，MicroProps 使用后者；通知 socket 则消费短平台 context Bearer，不能将两种 token 混写。

## 逐 grant 兼容决策登记

[`config.rs`](../../src/config.rs)、[`routes/auth.rs`](../../src/routes/auth.rs)与 [`routes/oauth2.rs`](../../src/routes/oauth2.rs)已加入受控兼容门禁：**BFF_ENABLED=true 时**，旧 login／context 和 `psr_` refresh 需要额外 `BFF_LEGACY_LOGIN_ENABLED=true`，旧人类 OAuth client 需要准确出现在 `BFF_LEGACY_HUMAN_CLIENTS`，并继续通过自身登记的 grant、账号／成员／app 策略。两项兼容配置分别默认 false／空集合。BFF_ENABLED=false 保留此前 legacy 行为，这是明确部署模式，不是上游故障时自动回退；不能将这些条件门禁写成所有部署中的旧 grant 均已无条件停用。机器 client_credentials 与平台人员 app token 继续按各自用途校验。

| 旧入口／grant | 实际消费者及用途 | 拟定处置与验收要求 |
| --- | --- | --- |
| `/api/auth/login`＋`/api/auth/context` | BFF 改造前的平台 frontend；回归 fixture 也使用。 | BFF 模式由 `BFF_LEGACY_LOGIN_ENABLED` 默认拒绝；显式兼容仍同一认证 owner、实时账号与精确员工成员门禁。SSO 不可用不改变模式，不导入历史凭据；配置／后端回归及真实outage验证已通过。 |
| `refresh_token`＋`psr_` | 改造前宿主真实依赖；SDK、两业务 API、notification、CLI 均不直接消费。 | BFF 在服务端 AEAD 保存并按原 session／绝对期限轮换。旧公开 wire endpoint 在 BFF 模式同受默认关闭的 legacy login 开关控制；不转成 SSO refresh，仍拒 OAuth client credentials／scope。 |
| legacy OAuth `refresh_token` | playground 对自己获得的 legacy token 进行演练刷新。 | BFF 模式仅准确人类 client allowlist 及原登记 grant／账号／成员／来源策略允许；不把它当 BFF 的 OIDC refresh。机器来源仍按机器用途独立处理。 |
| `authorization_code` | playground 显式配置 confidential code client 后执行旧 `/oauth/authorize` 与 `/oauth/token`。 | BFF 模式只对准确 allowlist client 开放，保留原 PKCE、callback、成员与 client／app 策略约束；不能误称新标准 OIDC code。 |
| `urn:ietf:params:oauth:grant-type:device_code` | playground 显式配置 device client；宿主保留 device approval UI。 | BFF 模式只对准确 allowlist client 开放，创建／审批／兑换仍绑定当前精确员工 context 与 client；不借另一个 RP 准入。 |
| `password` | playground 的专用可选演练 client；其 `OAUTH_PASSWORD_ENABLED` 默认 false。 | 无平台业务必需消费者。BFF 模式下平台准确 allowlist 默认为空，样例还须显式开 password 开关并登记该 grant；账号验证同 owner，不作故障回退。平台新门禁是人类 client allowlist，不能写成新增独立 PASSWORD_ENABLED 配置。 |
| `client_credentials` | playground 有运行时演练；两业务仓与 CLI 有机器客户端登记合同。 | 保留真正机器用途，confidential 认证＋MACHINE 主体；不得产生人员身份、context 或允许集合解释为空即任意成员。 |
| `/api/oauth/session-token` | 宿主为 qiankun 应用申请精确 app token；属于平台人员 app 凭据派生，**不是 `/oauth/token` grant_type**。 | 必须保留业务合同，短 context Bearer 经实时 BFF／原 sid／当前 RP／成员／目标应用交集检查后才可派生。机器服务 client 可以是目标，不得冒充来源人类 client。 |
| 历史凭据导入／旧用户转换 | 本身份新库无历史凭据、旧账号或外部旧 subject 输入。 | **不适用**。不制造生产迁移窗口、导入任务或虚构已退役流量；预览 UUID 专项仍按独立台账管理。 |

playground 五类调用位于 `http.rs` 第 255（client_credentials）、279（password）、349（authorization_code）、501（device_code）、556（refresh_token）行，统一由 [`upstream.rs`](../../../../saas-examples/oauth-playground/backend/src/upstream.rs)第 116～127 行发送旧 `/oauth/token`。其 `config.rs` 第 57／63／69 行为可选 code／device／machine client，第 198～217 行在没有 ID／secret 时返回 `None`；只有配置成功才由 `http.rs` 第 118 行广告可用，并于第 219 行执行前再次检查。password 额外需要第 51 行默认 false 的演练开关及第 75～83 行专用 client。平台另核 client 认证、登记 grant 及 BFF 模式下新加的准确人类 client allowlist。管理页面的 grant 选项与 monitor 展示不构成自身兑换 token 的消费者证据。

## 当前实现与配置

[`bff`](../../src/bff/mod.rs)保持同 crate 内的窄模块：`routes` 负责 Cookie／CSRF 和协议入口，`protocol` 验证 RP 响应，`transport` 定义 consumer-owned `OidcTransport` port，`store` 管理平台 BFF 持久状态，`context` 选择／续期具体员工身份，`crypto` 封装服务端凭据。测试adapter通过同一port调真实provider Router，14组PG／HTTP专项已实际完整执行；最终真实TLS BFF、notification与Edge浏览器另有独立PASS证据。

| 配置 | 默认与当前约束 |
| --- | --- |
| `BFF_ENABLED` | false。`GET /auth/config`只由此配置返回 BFF／LEGACY，不查询上游；上游失败不能把 BFF 改为本地密码模式。 |
| `PUBLIC_APP_URL` | BFF 开启时必须是单一、准确 HTTPS origin，与 issuer 使用不同 hostname。 |
| `BFF_OIDC_ISSUER` | BFF 开启时必填准确 HTTPS origin；如果本进程同时配置 `OIDC_ISSUER`，两者必须精确相同。 |
| `BFF_OIDC_CLIENT_ID`／`BFF_OIDC_CLIENT_SECRET` | 实际签发的 canonical UUIDv7 client ID／64 位小写十六进制 opaque secret，不由请求选择 client。 |
| `BFF_OIDC_REDIRECT_URI` | 必须精确等于 `PUBLIC_APP_URL + /auth/callback`。 |
| `BFF_TOKEN_KEY_FILE` | BFF 开启时必填外部绝对路径，启动读取 AES-256-GCM keyring。 |
| `BFF_CA_CERT_FILE` | 可选外部绝对 CA bundle 路径，最多 1 MiB且至少一个有效证书，追加至严格 HTTPS client 的正常信任；不关闭 TLS 验证。 |
| `BFF_AUTHORITY_TIMEOUT_MS` | 3000，范围 100～10000；同一 Chain 内全部 Discovery／JWKS／token／introspection／userinfo 共用绝对 deadline。连接上限 1 秒。 |
| `BFF_MAX_CONCURRENT` | 16，范围 1～64；Chain 使用无等待 permit，耗尽 fail closed。 |
| `BFF_CONTEXT_TTL_SECONDS` | 300，范围 1～300；还取原认证、live access、平台 context、组织与租户的最早期限。 |
| `BFF_LOGIN_TTL_SECONDS` | 300，范围 30～600；state／nonce／PKCE 绑定的登录 flow 绝对期限。 |
| `BFF_LEGACY_LOGIN_ENABLED` | false；在 BFF 模式内额外控制旧 login／context 与公开 `psr_` refresh，不能由故障触发。 |
| `BFF_LEGACY_HUMAN_CLIENTS` | 空集合；BFF 模式下准确 ID allowlist，最多 64 项，保留的 client 仍需自身 grant／subject／成员和策略验证。 |

`OidcTransport`只接受 `Endpoint` 枚举，固定在可信 issuer 下的 discovery、JWKS、token、userinfo、introspect、revoke 与 account/logout，不接受调用者 URL或token中的 jku/jwk。HTTP仅HTTPS、无代理与重定向，显式 `retry::never()`禁自动重试，单响应最多65536字节；callback／refresh每次受预算获取一份新Discovery和JWKS，未建公钥缓存，未知kid不额外循环重取。预算与故障断言已由PG／HTTP专项验证，最终真实上游outage503及恢复另有浏览器证据。

[`crypto.rs`](../../src/bff/crypto.rs)使用已锁定 ring 的 AES-256-GCM。外部 JSON 合同为 `activeKid` 和 1～8 个 `{kid,key}`；kid 为 1～80 个字母数字／下划线／连字符且唯一，key 为 canonical 无 padding base64url 的 32 字节材料。文件最多 32768 字节，activeKid 必须存在，不派生自 JWT signing secret 或 OIDC RSA key。每次加密使用独立 CSPRNG 96 位 nonce，envelope 为 `v1.kid.<base64url(nonce+ciphertext+tag)>`；AAD 绑定版本、record ID、issuer、client 和 token 类型。PKCE verifier、OIDC access／refresh 和平台 refresh 均在服务端以不同 kind 封装，ID Token仅在服务端验证，不下发给浏览器。keyring 在配置加载时读取；轮换须保留仍有密文依赖的旧 kid，并通过受控重启／配置重新加载生效，当前没有运行时热重载实现。

## 0018、来源权威与具体成员

[`0018_platform_bff.sql`](../../migrations/0018_platform_bff.sql)新增平台owner的 `console_browsers`、`console_login_flows`、`bff_sessions`及 `platform_sessions`的BFF绑定列；认证owner另持有 `oidc_account_logout_receipts`。独立源码计数新增**18个UUID列／18个独立v7 CHECK**。[`uuid_v7_target_enforces_versions_variants_and_application_generated_ids`](../../src/release_tests/database_upgrade.rs)已实际通过：catalog精确为**155列**、每列匹配validated单列 `is_uuid_v7` CHECK的完整集合、数据库UUID默认值为0，并拒绝v4／nil／错误variant与省略应用ID。首轮即有该单项PASS，后续修复后的212／212也完整通过；首轮198／14失败历史不删除。

同轮 [`phase31_platform_upgrade_preserves_every_old_row_and_checksum`](../../src/release_tests/phase31.rs)已实际通过3→18升级、重复迁移、历史记录／字段保留和腐坏checksum拒绝；日志明确 `old=3 current=18 ... corruption_rejected`，对应任务库清理成功。源码断言保留原迁移记录，旧无绑定session按既定迁移显式撤销，不伪造新成员来源。该运行证据与下面只读history不变量审计分别记录，未改写旧migration checksum。

只读不变量审计核对了完整0001～0017：0001～0015按现有schema history的SHA384 checksum→有序JSON→SHA256算法重算，聚合值 `03a709d300ce35c85c110b17b7dc3b8394e0f7543d43e38a626da983332e74f1`匹配Phase6已保留receipt；0016／0017分别匹配Phase8／9独立源码snapshot。此处不是声称17份migration逐文件SHA都存在独立seal。当前0018的SHA256为 `210bc3f0aee9c2d6115d68c56d37ef8865f2c9b0f54e1f1df36ba623a9038552`；SDK1／2／3 archives及平台／工单／门店SOURCE文件全字节无漂移。该审计是源码／包不变量证据，与155列catalog、212全量及真实浏览器分别记证。

同库过渡提供三个明确读投影：`bff_oidc_proof_active`检查原user／global sid／grant／account_version／实际RP client／policy／固定issuer；`bff_session_active`再核browser当前代际、绝对期限、注销／uncertain状态；`platform_source_active`分别处理原legacy来源与真实OIDC来源。更新后的 `platform_session_active`将BFF selected sid／version与具体成员权威相交，OIDC分支强制 `purpose=PLATFORM_CONSOLE`、`allowed_member_types=['EMPLOYEE']`，不借 `platform-console-legacy`固定登记或另一个允许两种身份的ACCOUNT client。

第二轮PG实测暴露已停用RP仍经provider `invalid_client`映射为503。当前 [`store::reject_inactive_source`](../../src/bff/store.rs)在上游前增加**只能拒绝的投影预检**：严格使用session记录的原user、global sid、grant、account_version、client、client policy version、issuer七个字段调用 `bff_oidc_proof_active`；false返回401，SQL失败仍503，true必须继续完整网络校验和提交前复验。这不是同库成功即放行的认证捷径，远端invalid_client也继续按503处理，不全局改成401。策略单组7.57秒、完整14／14的101.56秒，以及后续三类修正后再完整14／14的99.20秒分别保留；最新212／212与rollout3／3也已通过，不靠拼接多个局部结果证明全量。

`context`查询当前账号自己的有效 EMPLOYEE 成员、有效 tenant 与完整 organization chain，按明确 membership_id 选择；普通切换不写 member_type。选择会递增 BFF context version、撤原平台 context／refresh／app token／code／device，再写真实 OIDC 来源 tuple；无成员只返回已认证账号。续期保持原 selected sid 与成员证明，在同一事务消费并轮换服务端 `psr_`，不延长原绝对期限；响应有短 `accessToken` 及精确身份，没有 refresh 字段。

[`store::online`](../../src/bff/store.rs)在上游访问后仍比较最初 version／selected sid并核 live proof；`context`按 user→client→BFF session 的锁序重新比较原 account／sid／grant／issuer／client policy／browser／nonce／认证时间与选择版本。网络证明不允许把等待期间的新组织或新账号当成原请求已获准的身份。派生 app token、普通业务 context与notification认证通过原 platform sid先做在线精确证明，再由事务内读投影继续核权威状态；权威不可用不会只凭缓存签名放行。

OIDC access 距到期 30 秒内允许服务端刷新：数据库先 CAS 写 refresh lease 并提交，之后才发送 HTTP，网络期间不持数据库事务锁。并发请求不能重获同一 lease；lease 过期、发送结果未知或验证失败将保持 uncertain／fail closed，不重发旧 refresh。迟到响应提交前再次核原 version／lease／注销状态及 lease、ID Token、live proof和绝对期限，不能复活已变会话。ID Token、access 与 refresh introspection必须共同匹配原 sub／sid／grant／account／client policy／nonce／auth_time，userinfo sub也一致。

## 浏览器入口、注销与网关

| 入口 | 已验收合同与实现范围 |
| --- | --- |
| `GET /auth/config` | 只返回配置决定的 mode；前端先读此接口再考虑任何 legacy 持久凭据，不因 SSO 错误回退。 |
| `GET /auth/login` | 准确Host、受限return、来源限流，独立state／nonce／S256 flow；return同时拒/auth、/login、/sso/login及多重编码认证循环。`__Host-saas_console_login`是非账号browser binder，hash-only存储，86400秒。 |
| `GET /auth/callback` | 先拒重复／非法参数、错误 issuer／state／browser／flow代际，再一次消费 flow并兑换；提交前再次 CAS browser generation。一个标签成功后旧代际 callback返回冲突，不能覆盖当前账号。 |
| `GET /auth/session` | 精确来源Cookie读取安全账号、当前EMPLOYEE choices、context／version及CSRF；正常路径先在线核上游。存在此前POST持久的pending logout时，只按原命令继续确认；成功后401 anonymous，错误保留pending，不接受GET新scope／token，也不发送Set-Cookie。 |
| `POST /auth/context-token` | Cookie＋准确 Origin＋CSRF＋canonical expectedVersion，明确成员选择或原 context续期，返回不超过300秒的内存 Bearer。 |
| `POST /auth/logout` | Cookie＋Origin＋CSRF＋原version，显式PLATFORM／GLOBAL；完成服务端撤销后返回固定 `/login?signedOut=1`，前端清内存，不发送Set-Cookie，也不在signed-out页自动发起登录。 |

账号 Cookie 为独立 `__Host-saas_console`；两个 BFF Cookie均 Secure、HttpOnly、SameSite=Lax、Path=/、无 Domain。BFF响应 no-store／no-cache／no-referrer／nosniff／DENY，Vary Origin、Cookie。POST不接受 Bearer代替 Cookie，缺失／重复／错误来源、Cookie或CSRF均不建立权威；正常同源GET可用准确Host＋Sec-Fetch-Site补充无Origin读取。

PLATFORM注销调用provider `/oidc/revoke`只撤本RP grant，保留其他client和原全局SSO；GLOBAL调用新增Basic认证的私有命令 `/oidc/account/logout`，经同一账号owner提升该账号代际并撤全部全局SSO与派生凭据。它是项目backchannel扩展，不是标准OIDC end-session endpoint。全局命令以原client／token hash／grant／user／account version及request_id保存事务receipt；同一命令重试不会重复提升账号代际或领养新会话。BFF先持久原request_id／scope再发网络，不把DB锁带过HTTP；结果未知保留pending，POST使用原命令继续确认，不假装成功。**GET /auth/session已加入原pending logout恢复分支**：通过同源读取门禁后，仅复用此前CSRF POST持久授权的原scope／request_id／token，确认后返回401 anonymous，失败503保留pending；不能创建新的GET注销命令或采用新账号证明。解密／网络前检查准确原issuer和client；网络后再核browser.current_session_id仍为原BFF会话，其他标签已登录新会话则409。

最终注销合同是：**POSTlogout和GETpending恢复的所有响应都不发送任何Set-Cookie**。提交前current sid检查只能拒绝提交前的竞态；若提交后新登录先返回、旧注销响应后到，固定名Cookie expiry仍可能抹去新Cookie，因此不发送删除指令。服务端revoke保证旧HttpOnly值已无权使用，旧值只保留到原Max-Age自然到期或下次login覆盖；前端清内存并按固定地址跳转。unknown logout恢复、迟到旧注销及延迟成功响应均在最新14／14中通过；真实浏览器第9次又实际完成PLATFORM／GLOBAL注销、响应不擦除较晚登录Cookie、PLATFORM保留SSO、GLOBAL撤原Bearer和原账号SSO且不撤不同账号。

前端 [`authentication-mode.ts`](../../../frontend/src/lib/authentication-mode.ts)、[`bff-session.ts`](../../../frontend/src/lib/bff-session.ts)与登录页已加入独立BFF流程。token／CSRF只在内存，广播只含sessionId、revision及失效标记，不含凭据；旧localStorage token pair不进入BFF恢复。切换、续期、取消和较晚响应按generation／version fence退役；客户端恢复能力不依赖Web Locks。legacy分支独立使用 `safeLegacyReturn`保留经过验证的本地query／hash及默认 `/`，BFF的 `safePlatformReturn`继续严格限制认证return，不把兼容导航扩大到OIDC回调。最新round7的npm505项及lint0错误／46既有warning、typecheck／build通过；SDK／qiankun静态证据继承round2。真实Edge隔离fixture完整九套round3单轮160个场景＋round1三专项10／14／5，共189通过，先前产品return回归与旧菜单selector失败分别保留。该189证据继承；runtime本地到期恢复修复后另实际执行受影响qiankun4组Edge全通过，13项owned micro内存生命周期artifact单独记证，均不冒充真实TLS BFF完整流程。

[`session-status.tsx`](../../../frontend/src/components/session-status.tsx)仅为**已确认BFF模式**的 `/login`开放unknown或unavailable恢复页：保留账号状态但清空可用token，现有账号可显式“重新确认账号”，fresh-load GET503且session-null也显示“继续统一登录”。账号重新确认由用户点击发起并防快速重复，不自动重定向、不清除服务端权威、不切回密码；private route、LEGACY或未确认mode的unavailable仍受门禁。新增5个实际coordinator／gate／component回归全部通过；最终browser9也实际完成wrong state、unknown refresh、fresh-page503和显式重新确认，并继续走完完整BFF浏览器流程。早期失败尝试分别保留。

root新增 [`nginx/policy.mjs`](../../nginx/policy.mjs)的 `/auth`精确命名空间转发及 [host Nginx例子](../examples/host-nginx.conf.example)对应路由：准确平台Host、保留Cookie／Origin／Set-Cookie、no-store与no-referrer，避免被SPA兜底。生成网关access log只记 `$uri`，不记callback query；auth错误上下文禁写error log，手写例子进一步关闭auth access log。真实Nginx1.28.0已有15项通过、1项可选嵌套Chromium跳过，证明网关路由和502日志边界；这不等于真实 BFF/provider浏览器闭环。

## BFF 独立审查合同

以下要求已对照源码，Rust／PG／配置、真实浏览器和最后跨client策略专项分别有实际PASS，详细证据与历史失败独立登记；源码审查本身不替代运行断言。

1. 每次登录生成独立 state、nonce、S256 verifier，绑定同一发起浏览器、精确 callback 和有限期限；服务端一次消费。callback 拒绝重复参数、错误／缺失 state、错误 issuer、错误 nonce、code 重放与不匹配浏览器，多个标签页不能静默覆盖彼此事务或把账号 A 的 flow 重绑账号 B。
2. RP 固定可信 HTTPS issuer、自己的真实 client ID 与精确 callback；发现文档的 issuer／endpoint 可信范围、token Basic 认证、RS256／kid、JWKS RSA 参数、UUIDv7 sub、aud、exp、iat、nonce、auth_time 和 sid 均显式验证。不接受 token 自带任意 jku/jwk 或算法降级；不能用 provider 自己的签发函数替代 RP 验签。
3. HTTP、Discovery、JWKS、token、UserInfo／introspection 有明确连接与共享总预算、响应大小上限、无重定向和有界并发。预算覆盖整个callback／refresh链，不能每一步重新获得完整总预算。当前实现每次callback／refresh受控读取一次Discovery／JWKS，无缓存，也不因未知kid再发无界请求；后续若采用缓存，须另立缓存期限、未知kid受控刷新次数及并发singleflight的合同与验证。公钥验签成功不替代实时账号或grant证明。
4. OIDC refresh 与平台 refresh 在服务端；同一会话旋转须跨并发请求／实例 singleflight，不能由一个正常并发刷新触发 provider reuse 撤族。网络发送后结果未知不盲目重试旧 refresh；lease 过期或进程崩溃后也不能由其他实例拿原 token 重发。结果未知应 fail closed 并要求重新认证，CAS loser 不能复活旧 token；失败、取消和期限届满均不部分提交或续长原绝对期限。
5. context 持久绑定原全局 sid、账号版本、当前 RP client 和其 policy 版本、具体员工 membership／identity_version、tenant／org；派生 app token 另核目标 application／service client 与版本。每次 context、短 Bearer、refresh、app token 和通知消费均重新验证，不能从其他允许两种身份的 client 借准入。
6. 防护 Cookie、精确来源、CSRF、no-store 与浏览器内存凭据边界贯穿选择、切组织、刷新和注销；短 context Bearer 的 exp 不能晚于 300 秒及原 sid／grant／context 截止。直接 ID Token 访问业务 API 必须拒绝。
7. 账号封禁／重置／全局注销撤销所有 BFF 派生凭据；局部成员撤销只影响相关身份。同一账号双身份、客户专用或无成员不得获得员工角色；SSO／权威状态不可用返回 503，无本地 password 自动旁路、旧 issuer 或仅缓存放行。

台账“client 或 app 策略改变后旧上下文失效”按上下文的实际绑定范围验收：平台组织 context 不绑定单一应用；当前 RP client policy 收紧会使该 RP 组织 context／派生凭据失效，目标 app policy 收紧则使该 app 的旧 context／token 及后续派生实时被拒。其他应用或仍合法的组织 context 可继续存在，但不能因此扩大目标 app 准入。不能把组织会话仍有效误判成 app 策略绕过，也不能声称任意 app 变更会全局注销所有组织会话。

最终只读审查曾确认一项运行证据缺口：原“另一client同时允许两类型”BFF专项只assert两个client ID不同，不能据原新14声称实际执行该负例。live结束后已扩既有 [`policy测试`](../../src/bff_tests.rs)，以真实ENABLED ACCOUNT client同时允许CUSTOMER／EMPLOYEE，确认本RP仍只列EMPLOYEE、伪选CUSTOMER返回403；停用本RP后，另一client仍ENABLED／version1／双集合，原RP context POST仍401。最终fmt检查及定向1／1、7.63秒实际PASS，见 [result](../../../local-identity-sso-phase10-qa/cross-client-policy-result.json)和 [log](../../../local-identity-sso-phase10-qa/cross-client-policy.log)。这是扩既有14内的一组，不增加第15组；DB／pool／cipher／keyset均清理，normal binary仍a62e…不漂移，关闭第2条Self-Check。

## 源码审查修正与后续证据

开发中已按源码审查收窄以下行为，不将其记成已执行失败测试：P1网络返回后仍保留并重核最初version／selected sid，不能采用等待期间的新选择；P2 live introspection到期和ID Token exp分别传播到提交期限及短Bearer上限，不只保存refresh绝对期限；SQL来源检查使用 `expected_issuer`避免issuer列／参数影蔽。HTTP client已显式禁自动重试；GETsession恢复原pending logout、原来源预验、所有注销响应不发送Set-Cookie及return认证循环拒绝也已逐段核对源码，已完成轮次14／14包含相应负例。

既有212首轮完整结果为198／14失败，分三类处理且不放宽断言：7项旧fixture未装当前HTTP代码需要的完整迁移，已仅把 [`identity_binding_tests.rs`](../../src/identity_binding_tests.rs)和 [`identity_ui_tests.rs`](../../src/identity_ui_tests.rs)的当前HTTP fixture升级MAX，保留历史13→14迁移测试；1项consent在user锁后已安全拒绝，但共享 `invalid_grant`经session API变成400，现 [`oauth_error_into`](../../src/routes/oauth2.rs)仅恢复session API401，标准token400及其他参数400／权威503不变，并新增非DB回归。

其余6项为device已撤终态协议：撤权后原platform proof已不可用，先查该proof会挡住本应返回的 `expired_token`。当前只增加**拒绝路径**：以device code hash＋当前已认证client精确匹配单条autocommit UPDATE，仅当状态已EXPIRED或数据库已到期时更新last_polled_at／EXPIRED并返回400 expired_token；未命中继续原完整proof／client／成员／策略验证，绝不据此签发，也不把数据库锁带过HTTP。三类修正后exact14／14、75.30秒、静态六项／195普通、新PG14／14、99.20秒、完整212／212、1013.47秒及rollout3／3、250.63秒已按真实串链全通过，任务清理成功；首轮失败仍保留。

早期真实浏览器失败保留如下：driver1～3在LIVE_BFF_START失败，root固定stage追到harness不支持的HTTPConnection with用法并改 `contextlib.closing`；部分外层receipt为RemoteDisconnected，不把所有外层类别改成同一底层原因。旧CA:true证书同时用作server leaf的尝试出现/login503，未直接捕获具体rustls拒绝原因。后改独立task CA＋CA:false leaf，provision PKI v2和严格TLS discovery probe实际200通过，issuer仍为 `https://auth.saas.test`，RP仍为 `https://console.saas.test`；只使用任务容器临时hosts映射并恢复，不改用户主机DNS或系统信任。

后续harness还遇Playwright APIRequest不继承浏览器测试DNS而Node unhandled，以及早期launcher未捕获进程流造成缺少fresh receipt；已改严格TLS／Cookie Node GET helper、固定stage崩溃记录与stdout／stderr安全hash。`browser-round-04/05-result.json`均为**FAIL／EDGE_CONTEXT_SELECTION**，各记录31条成功检查（含fixture／control／清理记录），失败名 `real_context_is_memory_only_employee`，appIssuances／appContexts／socketUpgrades全0。第5次安全诊断确认当时仍为switching；随后已确认安装版本Playwright的waitForFunction把async回调Promise当作truthy，故改为有界await-evaluate轮询，保留原复合断言。此项是harness修正，无生产修改。

`live-round-04/05`证据中进程均exit0、读取完整且敏感模式扫描无命中，DB连接归零、DB／notification role／任务容器hosts均清理。driver总状态仍FAIL，因为没有OIDC_REFRESH实际审计；**round05 BFF process另有binary_unchanged=false／status FAIL**，原因确认是root当时并行的membership_rollout integration Cargo重建normal backend，reqwest/json feature统一使路径SHA从 `fe896827c3e590eb5d66476c05a027d55997966bfcba44bae4c5605d74407075`变为 `a62e8091da50466d7d9003fa219785dbf8229e1e0ebc437aabc629888e03b53c`。保留该有效失败，不能写所有process PASS。Rust全链现已结束，不再并行Cargo；最终a62e…普通非cfg(test) binary已重新实际完成配置66／66 PASS，使用新task CA并清理临时材料，不复用fe896…旧receipt。

`live-round-06`各进程PASS／binary不变、清理完成，但driver仍因无OIDC_REFRESH整体FAIL，对应浏览器第5次31条记录的轮询工具问题。修正后 `live-round-07` driver整体PASS且实际OIDC_REFRESH=1；浏览器第6次已真实qiankun挂载，42条记录、appIssuances=1／appContexts=2／socketUpgrades=2，仍FAIL于REAL_QIANKUN_AND_SOCKET。Node Socket.IO的engine.io option pick丢弃顶层lookup导致DNS失败，harness改用带固定任务DNS的https.Agent，继续使用真实域名及严格TLS，无生产修改。

`live-round-08` driver整体PASS、OIDC_REFRESH=3，各进程exit0／binary不变、连接归零、DB／role／hosts清理成功；浏览器第7次仍FAIL于REAL_SHORT_TOKEN_RENEWAL，47条成功记录、appIssuances=3／appContexts=5／socketUpgrades=7。真实notification authenticate、短token续期两次、socket重连及服务端OIDC refresh已有记录，随后qiankun续期DOM计数等待超时。该轮最初尚无法区分计数工具与实际生命周期问题，因此新增安全DOM计数／状态和失败PNG；后续第8次已确认实际产品bug，不能继续把当前原因写作计数或等待假设。

`live-round-09` driver整体PASS、OIDC_REFRESH=3，各进程PASS／exit0／binary不变、连接归零、DB／role／hosts清理成功；浏览器第8次仍FAIL／REAL_SHORT_TOKEN_RENEWAL／OWNED_PLAYWRIGHT_WAIT_TIMEOUT，47条记录、三项业务计数仍为3／5／7。安全生命周期诊断明确mounts=1／updates=1／unmounts=1／rootCount=0，[失败PNG](../../../local-identity-sso-phase10-qa/screenshots-round-08/failure-real-short-token-renewal.png)显示“子应用挂载失败／应用授权上下文已过期或与目标应用不一致”，确认host runtime永久卸载的产品bug。现 [`runtime.ts`](../../../frontend/src/components/micro/runtime.ts)已作最小修正：有效app／context lease在传输中本地到期时，先清旧快照，再按现有有界恢复重新申请；私有LocalExpiredLeaseError通过instanceof分类，协议错误及401／403等永久拒绝不重试，不延长原期限。4项fakeclock已纳最新505／505，受影响qiankun4组真实Edge也全部通过，补丁只读审查无阻断。第8次产品失败保留，最终第9次真实浏览器结果另列。

最终 [`browser-round-09-result.json`](../../../local-identity-sso-phase10-qa/browser-round-09-result.json)为PASS，98条检查记录含transport／control／cleanup辅助记录，不称98个独立场景；appIssuances=7／appContexts=12／socketUpgrades=13。实际短token及OIDC续期、qiankun fresh props／live context、socket重连、跨tab切组织使旧Bearer失效并卸载旧应用、上游outage503及恢复、PLATFORM／GLOBAL注销、memberless不建成员、不同账号隔离、无pageerror及各阶段Storage检查均通过。最终9张PNG由root逐张view_image核验，见 [visual review](../../../local-identity-sso-phase10-qa/browser-round-09-visual-review.json)。

对应 [`live-round-10-evidence.json`](../../../local-identity-sso-phase10-qa/live-round-10-evidence.json)driver整体PASS，实际OIDC_REFRESH=3、BFF_CONTEXT_RENEW=7／SELECT=3／LOGIN=4／LOGOUT=2；bootstrap／BFF／notification三个进程均PASS／exit0，读取完整、全流敏感模式扫描无命中、binary不变，任务DB连接0、DB／notification role／容器hosts清理均true。bridge、driver及browser已退出；这轮driver与browser各自有独立PASS证据。其后跨client策略专项1／1也通过，五条Self-Check全部PASS，Phase10完成。

最终 [source-snapshot.json](../../../local-identity-sso-phase10-qa/source-snapshot.json)已刷新498个选定source／QA helper文件，文件SHA256为 `a0476bf6eb916ac15e2d82f4d95e9a732c0cfd127f5cb2abe1094d2e59487fdf`；动态文档／证据／私密材料／媒体不在source selection中。先前830cc…摘要及495文件不变核对仅属修复前记录，不作为最终代码的seal。最终 [invariant-audit.json](../../../local-identity-sso-phase10-qa/invariant-audit.json)SHA256为 `f5f1e4c1f4f1814c9431bd8824faa365545cfe5d38adc592a82e144a9f8060d7`，重新核对0001～0017历史、固定SDK archives／消费者字节和最终TASK_OWNED证据，全部PASS。SDK3.0.0、MicroProps2.0、Context1.0与Notification1.0合同分别保持；源码hash不替代前述实际运行结果。此阶段完成不代表生产部署、客户门户或独立认证服务已完成，默认开关仍保持关闭边界。
