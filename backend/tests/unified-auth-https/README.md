# 真实 HTTPS 浏览器验证

`prepare.py` 为当前 canonical `0001` 建立任务专属空库和 owner/runtime 角色；由环境执行者启动当前身份服务、平台、严格 TLS 网关及仅限回环的 CONNECT 代理。`browser.mjs` 使用仓库锁定的 Playwright 和 Chromium，直接访问这些实际服务，不启动 Docker、服务或替代 HTTP fixture。

```powershell
node backend/tests/unified-auth-https/browser.mjs `
  --environment /tmp/rust-saas-unified-auth-20261028/https-environment.json `
  --proxy http://127.0.0.1:24445 `
  --output backend/docs/designs/unified-auth/validation/phase-13-https-browser.json
```

执行前必须由环境执行者确认服务及受信任 CA 已就绪。脚本不关闭证书验证；默认 Chromium 使用 `frontend/tests/browser/.browsers`，可显式选择 `--browser chrome` 或 `--browser msedge`。Windows 上通过 `wsl.exe` 在内存中读取仅供任务使用的私有配置，密码、Cookie、CSRF 和令牌不写入结果。脚本只输出固定场景、端点状态计数及脱敏结果，不导出浏览器 storage state、trace 或登录表单截图。

覆盖管理员及普通员工的真实 `/auth/login` → canonical OAuth → SSO → 同意 → 单候选自动选择 → 回调、Cookie 与刷新恢复、实际 refresh family 轮换、真实组织切换、CUSTOMER 账号不能建立员工会话、原 SSO 来源撤销后的拒绝及另一账号隔离。管理员还通过实际 `/api/users` → 私有账号命令创建新账号，再经员工成员管理 API 建立身份，以独立浏览器验证新账号的 canonical 登录；脚本不通过 SQL 设置新账号版本。报告只保留账号命令前后版本数字。

期限及第二身份边界通过任务 PostgreSQL 设置：每次 SQL 均在事务内核对固定回环端口、数据库名、集群标识和部署注释，且仅修改配置中明确绑定的 fixture 账号及会话。

fixture 为管理员准备实际 `system:user` 菜单及当前应用、租户、组织的授权；`is_super_admin` 本身不绕过这些上限。脚本按相同定义幂等核验并补齐最小菜单 grant。第二员工身份只有在状态实际改变时才推进 `identity_version`，保留 canonical 成员变更约束与撤销触发器。

真实宿主登录和刷新断言完成后，脚本将受控 API／期限边界操作放在同源实际 `/auth/config` JSON 页面执行，保持一个续期驱动者，避免测试自身与 React 宿主续期管理器同时竞争单次轮换租约。并发失败恢复由相应并发回归单独验收，本文脚本不把人工第二消费者造成的 503 作为可以忽略的成功结果。

同一任务环境可以重试。第二员工身份在场景开始和 `finally` 中停用，在切换场景临时启用；本次数据库内记录由 `prepare.py --cleanup` 随整个专属数据库清除。浏览器 context 与进程在成功或失败时都会关闭。身份服务、平台、网关、代理、数据库、角色、私有文件及环境日志仍由环境执行者在完整验证结束后统一定向清理。本脚本的 PASS 不代表正式发布、外部第三方互操作或后续阶段已完成。

## 并行开发时固定输入

`snapshot-build.py` 只复制白名单源码、静态文件、当前迁移、角色 SQL、
Cargo 配置、本地 crate、本目录验证工具和参与前端的 `dist`，不复制
`.git`、`target`、`.env` 或运行秘密。
复制完成后其他阶段可继续修改工作区；编译使用任务私有快照、同一任务构建
缓存，并记录相对文件 SHA、快照 SHA、工具链与二进制 SHA。

运行 `prepare.py` 时用 `--backend-source "$TASK_DIR/source-snapshot-NAME/saas-plateform/backend"`
选择与 `platform-migrate` 同一快照的角色/provision/verify SQL。该参数只允许
当前 backend 或本次任务目录内的快照；TLS CA 仍从实际工作区的受保护本机环境读取。
这样并行阶段新增的 live schema/ACL 不会混入另一阶段已冻结的验收输入。
不得通过更新 `_sqlx_migrations` 的 checksum 处理版本变化。

后续阶段可以继续构建宿主前端。并行验收时，使用
`environment.sh start --workspace "$WORKSPACE" --static-workspace "$TASK_DIR/source-snapshot-NAME" ...`
固定网关实际服务的前端产物；其文件 SHA 与后端输入一起保存在快照清单。
`--static-workspace` 只接受实际工作区或本次任务目录内的快照，TLS 证书仍从
明确的实际工作区读取。省略此参数保留直接读取工作区产物的现有行为。

仅修复一个组件时，`snapshot-build.py build --service identity|platform|portal ...`
只构建并替换该组件的任务二进制。该次 attestation 的 `builtServices` 与
`binaries` 只证明这些产物；报告必须另外引用继续运行组件的原始 attestation，
不能把一次局部构建写成三服务均已重建。只有确认 schema 与未重启组件输入
没有变化后才可复用原库和其他进程。

Phase 25 完整候选使用 `copy --include-all-projects --require-frontend-outputs`，覆盖八个独立仓库的
当前源输入、门店／工单的 vendor 与迁移、后端顶层集成测试与协议 fixture、
CLI 的 manifests／schemas／tests、SDK 当前交付及不可变归档和正式示例。
对应 `build` 使用原锁文件离线构建身份、平台、门户、门店、工单、通知、
四个 migrator、管理 CLI 和数据库认证 probe；`builtServices` 与逐二进制 SHA
共同限定该次实际构建范围，不代表运行或跨服务测试已经通过。

前端可以构建到本次私有临时目录，再通过重复的
`--frontend-output PROJECT=ABSOLUTE_DIRECTORY` 在复制时写入快照的
`PROJECT/dist`。支持平台、门户、门店、工单，以及 websocket、micro-vue、
micro-jquery、report-center 四个示例。该参数要求完成的真实目录及 index.html，
拒绝重复项目、符号链接和私钥／环境秘密；不改动工作区已有 dist。
`frontendOutputs` 保存每个新产物目录的文件摘要及整体摘要，必须再关联
前端构建者的源码输入与检查报告。全部八个显式新产物齐备时，manifest 与
attestation 的 `completeFrontendOutputs` 才为 true。最终验收必须使用与这些新产物同一份
manifest 的完整候选 attestation；较早只用于预热编译缓存的快照不构成完整候选。

门店与工单的服务端统一使用 `PLATFORM_API_ORIGIN=https://console.saas.test:24444`
及相同受信任 CA。私有网关只为这两个已参与验证的 app code 放行无 query 的
精确 `POST /api/apps/{code}/authorize`，使同一 origin 同时支持授权与
`/internal/platform/apps/...` 目录。公开网关拒绝内部目录和该服务授权路径；
服务仍需各自的 Basic 凭据，原用户 token 只放在 POST 表单。

## Phase 20 登出与受控投递

当前基线可显式启用 `prepare.py --include-phase20 --include-phase19 --include-portal ...`。
它要求同一固定源码的当前 migrator/role SQL，并从空库准备两个 RP 的
owner 核验 back-channel 目的及一个定向邀请渠道，不设置正向 MFA 事实。
配置回写仅保存于任务私有 `https-environment.json`；生产责任保持关闭。
门户运行角色对两个 logout 事实表只有 SELECT/INSERT，独立库共有五张业务表。

`collector.py --task-dir "$TASK_DIR"` 是本次任务的内存接收方，绑定 WSL
127.0.0.1:23443。任务私有 TLS 网关仅把准确的
`https://auth.saas.test:24444/internal/task-delivery/{invitations,backchannel}`
POST 转到它。控制面不经过网关，使用独立 `X-Task-Control-Key`，不会记录
消息、token、密码或请求头；namespace 退出时 collector 自动关闭并清空内存。
不得把这个 collector、`.test` 回环例外或 owner SQL fixture 当成真实收件渠道。

`phase20-browser.mjs` 使用同样的 `--environment`、`--proxy` 和 `--output`
参数，正式输出为 `validation/phase-20-https-browser.json`。它验证真实两个 RP
的准确退出范围、worker 签名投递和独立 ACCOUNT_ONLY RP 的真实授权码流程；
仅在观察到原投递失败或核验等待后，按归属调整测试 job 的 `available_at`，
不能跳过原 lease、换 event/source 或把未知结果当成功。邀请材料只在 collector
和脚本内存中传递，领取创建新纯账号，不向真实外部收件人发送消息。

写完 prepare、通过 Python/Node 语法检查或启动 collector 均不是实际验收通过。
应用、worker、严格 CA、固定前端、RP 和控制器就绪后才运行浏览器；正式报告
须说明 runtime 快照、实际失败/重试、负向 fixture 边界及最终清理。

## 可选客户门户环境

默认 `prepare.py` 仍只准备平台与身份服务。当前可选门户模式适用于包含 Phase 15 `platform_resource_services` 的新 `0001` 基线；缺少该表的旧冻结源码会明确拒绝并清理新建 fixture，不自动回退旧登记。在已核对的同一任务 PostgreSQL、namespace 和网关中使用：

```sh
python3 saas-plateform/backend/tests/unified-auth-https/prepare.py \
  --task-dir "$TASK_DIR" --admin-url "$TASK_ADMIN_URL" --cluster-id "$TASK_CLUSTER_ID" \
  --migration-bin "$TASK_DIR/bin/platform-migrate" \
  --include-portal --portal-migration-bin "$TASK_DIR/bin/portal-migrate"
```

该命令只接受尚未存在的任务 fixture，不能对运行中的旧环境追加表或改写 migration checksum。重新准备前，先按本次 PID 归属停止相关服务，再用原 manifest 执行 `--cleanup`；保留同一个任务 PostgreSQL、网关与构建缓存继续验证。不要将此命令用于既有业务库。

门户模式在共享 fixture 中登记独立 CUSTOMER application、canonical RP、同意组、sector 和 `client_registry=AUTH` 的选择消费登记；保留员工 RP 和原账号。门户自己的 `CONTEXT_SELECTION_SOURCE` 服务仅允许客户 RP，平台反向的 `CUSTOMER_ACCESS_PROOF` 使用另一组凭据。RP、门户 source、反向 proof、profile 内部服务、选择 consumer 和门户数据库各自生成独立秘密。现有身份到平台的选择 authority、平台到身份的上下文 source 仅在同方向扩充两个明确 RP 的 allowlist，门户不会借用它们的凭据。

`unified_auth_portal_<UUIDv7 hex>` 是独立空数据库，由当前 `portal-migrate` 初始化。门户 owner 不可登录，runtime 使用真正的独立 LOGIN，只获得三张门户表的 DML 和 UUID 约束函数执行权；没有 DDL、owner membership、迁移历史访问权或共享平台／身份库的 CONNECT 权限。两个数据库之间没有表、视图或 FDW 连接。私有 `https-environment.json` 增加 `portal` 配置，监听任务端口 23101；`https-portal-keys.json` 与平台 keyring 独立。

本人资料端口使用独立的 `platform_resource_services` 登记：用途为 `CUSTOMER_PROFILE`，绑定明确 portal application、其应用 code 受众、`CUSTOMER` 成员类型和 `customer:profile` 操作。fixture 不再把该服务写入 `oauth_clients`；原 `profileClient` UUID 和 Basic service ID 仅作为私有资源服务标识，不能代替 canonical 客户 RP 签发 OAuth。

已有 Phase 14 验收报告仅证明其记录的冻结源码与当时环境，过渡登记属于历史说明；这些报告不能作为当前 Phase 15+ 源码通过的证据。当前准备脚本不支持旧结构兼容模式，也不修改历史 checksum。

manifest 同时记录门户数据库、两个角色和部署 marker。成功清理或准备失败后的清理均先核对集群、数据库名、角色前缀、数据库 owner、数据库／角色 comment，再删除本次门户库及角色。运行秘密仅留在权限 0600 的任务文件中，最终由环境执行者定向清理；脚本不会创建 Docker 资源。此准备选项本身未运行浏览器场景，Phase 14 必须另外记录三服务真实 HTTPS、独立 Cookie、跨库原结果恢复和客户隔离的验收结果。

## 可选审核与邀请环境

新建 Phase 19 fixture 时增加 `--include-phase19`，并明确传入与迁移二进制相同的 `--backend-source "$TASK_DIR/source-snapshot-NAME/saas-plateform/backend"`。该模式要求本次任务目录内的冻结源码，不能使用运行中的旧 fixture，也不能通过改写 migration checksum 接管旧库。可以与 `--include-portal` 同时使用；服务、namespace、数据库和角色仍由同一环境执行者统一管理。

私有配置增加 `phase19: {contractVersion: "unified-auth-phase19/1", enabled: true}`，以及独立的 `accounts.applicant`、`accounts.reviewer`、`accounts.publisher`。三者均为本次根组织的非超级管理员员工，拥有不同 UUID、密码、成员和角色。申请方拥有 VIEW、REGISTER、ASSIGN；审核方拥有 VIEW、REVIEW、INVITE；发布方拥有 VIEW、PUBLISH、DISABLE，并需通过实际应用委派 API 获得应用范围。fixture 为 OAuth 根菜单和十个细权限建立实际 application、tenant、organization grant；原 root admin 也受这些上限约束。

`AUTH_REVIEW`、`AUTH_INVITATION`、`AUTH_ADMIN` 使用三组新生成的独立服务凭据。邀请派生 keyset 保存到权限 0600 的 `https-invitation-keys.json`，与 SSO TOTP、BFF、context 和 RP 凭据独立。fixture 不预种 TOTP 因子、近期 MFA 证明、审批、partner/application 申请或 production release binding；浏览器必须通过实际 SSO 绑定验证器，再经 API 完成申请、独立审核、委派、同步和邀请操作。仅准备文件或通过语法检查不代表真实 HTTPS 验收通过。

任务 TLS 私有监听为身份服务使用精确 method/path allowlist，只开放当前源码已实现的私有端口；未知路径和未实现的 HTTP assurance 端口不转发。平台管理 assurance 来自受限数据库函数，浏览器安全状态使用公共 `/api/identity/review/security`。公共监听继续拒绝 `/internal`，不得为浏览器测试开放私有管理入口。

已由环境执行者启动这份新 fixture 及相同快照的身份/平台进程后，运行：

```powershell
node backend/tests/unified-auth-https/phase19-browser.mjs `
  --environment /tmp/rust-saas-unified-auth-20261028/https-environment.json `
  --proxy http://127.0.0.1:24445 `
  --output backend/docs/designs/unified-auth/validation/phase-19-https-browser.json
```

脚本先核对独立 Phase19 标记、三名非超级管理员员工与数据库/cluster/deployment comment。它通过实际安全页绑定 TOTP，按数据库真实时间等待新的 30 秒时间步，再通过固定 `/auth/step-up` 完成密码+TOTP，核对新的 BFF、SSO与选定平台会话及旧代际失效。管理操作每次重新取得实际 session/context-token，避免与宿主 React 刷新管理器同时争用单次 refresh。缺少因子、受控陈旧时间边界和实际恢复码登录均须由服务端拒绝敏感写入；正向 MFA 不由 SQL 设置。

申请/独立测试批准、身份准入同步和邀请元数据签发/撤销均走真实 HTTPS。生产先验证默认关闭，再只对当前隔离 fixture 配置明确的审核/发布责任，验证真实审批、发布和暂停；该测试绑定不证明真实 Q3 或域名责任已确认。`finally` 恢复原 production binding 和陈旧时间边界，撤销仅三名本次新建 fixture 员工的来源，并清除其 TOTP 因子、待绑定记录和恢复码，保留审计，允许同一环境重试。原 admin/employee/customer 账号被明确排除。整个环境的库、角色、进程、私有文件和日志仍由环境执行者最终清理。

不导出 seed、恢复码、密码、Cookie、token、完整回调 query、trace、storage state 或安全页截图。报告仅包含固定场景、脱敏状态、TLS、审计增量及清理计数。通过语法/静态审查不能标作实际 HTTPS PASS；以本次运行写出的正式报告为准。

## 客户三服务验收脚本

三服务当前快照已启动、门户静态产物及受信 CA 已就绪后，才运行独立的客户验收：

```powershell
node backend/tests/unified-auth-https/customer-browser.mjs `
  --environment /tmp/rust-saas-unified-auth-20261028/https-environment.json `
  --proxy http://127.0.0.1:24445 `
  --output backend/docs/designs/unified-auth/validation/phase-14-https-browser.json
```

脚本使用真实 Chromium、三个 HTTPS 服务和严格 CA／主机名校验。它验证客户单候选、本人资料、在线 refresh、组织切换、纯员工拒绝，以及同一浏览器中管理员员工／客户双身份共享原 SSO 而保持独立主机 Cookie 与本地会话。共享 SSO 场景使用准备脚本登记的 `adminCustomerMember`；纯客户和纯员工账号各自保持明确用途。

跨库故障场景仅在任务门户库建立带归属 comment 的窄临时触发器：该客户原 pending 正要清除时抛出错误。脚本先通过真实平台消费的审计及结果记录证明平台已提交，再确认门户暂不提供 session/profile。恢复后必须仍是同一个门户会话、同一平台能力引用及摘要、原期限，消费审计数为 1。错误 BFF ID、version、RP 和 access 来源通过真实私有 HTTPS `/result` 负向请求验证，不构造成功凭据；负向探针只在任务 namespace 内解开原服务器 AEAD 的 access/pending envelope，读取独立测试配置，标准输出仅返回 HTTP 状态，令牌不会进入浏览器或报告。

私有探针通过 `curl --cacert` 同时校验 CA 和主机名，限定 HTTPS、不跟随重定向，请求凭据和 body 仅经 stdin 传入，不进入命令参数或临时文件。现本机 CA 未声明 Authority Key Identifier，Python 3.14 额外的 `X509_STRICT` 策略会拒绝它；探针使用与当前 Rust／Chromium 正常信任链校验一致的 curl，不使用 `-k` 或关闭证书验证。诊断时可在上述命令末尾增加 `--diagnostic last-pending`，只读取原客户 pending 并查询相同 `/result`、反向 access proof 和原选择 source，输出状态及上下文计数；此模式不创建浏览器、不消费选择、不刷新 token、不生成新上下文，且不覆盖正式验收 JSON。

每条 SQL 都核对本次集群、库名和各自数据库 comment；门户 runtime 还实际尝试跨库连接并被拒绝。脚本只临时启用纯客户的第二 CUSTOMER 成员，状态变化同时递增 `identity_version`，在成功、失败或中断的 `finally` 中停用它并删除触发器／函数。临时对象的归属 comment 不匹配时停止清理并报告失败，禁止扩大删除范围。报告只保存固定场景、端点状态、TLS 和布尔结论，不含密码、token、Cookie、完整回调 query、storage state 或页面截图。脚本写完／语法通过不等于真实运行通过，实际结果以本次生成的报告为准。

## Phase 23 标准客户端缓存与密钥窗口

`phase23-key-cases.mjs` 保留同一个 `phase23-protocol.mjs` 进程及真实标准库缓存。B 发布后先按数据库实际时间等待至少 61 秒，并只读 `openid-client.getJwksCache()` 的实际公开年龄与 kid 集合；每次 `ports.wait` 最多 10 秒。最初的 A 用于暖缓存，等待结束后重新签发 A 并覆盖保留载体，再激活 B。当前 JWKS 对未过期 A 的验证与标准 SDK 对新 B 的验证分别保留证据。

C/D 在 B 仍活动时一同预发布，完成第二个至少 61 秒的实际发布窗口后，强制一次真实密码与 TOTP step-up，再进入短的 B 撤销 → 签发 503 → D 激活窗口。B 撤销后公钥集合为 C/D、活动集合为空；D 的正常 unknown-kid 缓存刷新会同时取得 C/D，因此随后 C/D 载体不需要清缓存或重启 SDK。四分钟后的管理写入会先真实刷新管理员认证，服务端五分钟验收保持。

当前 canonical 人员 Access Token 和 ID Token 的实际最大期限均为 300 秒，并受原 grant／来源上限约束；refresh Access Token 同样最多 300 秒。`profile(30/600/300)` 仅作为场景标签重启身份双实例，不写入 token 期限环境变量。结果分别记录 `requestedProfileSeconds` 和固定的 `canonicalTokenMaxTtlSeconds=300`；前者不代表生效期限。A 的正常退役等待实际 `last_signed_exp + 60`，所以不能按场景标签 30 推断只需等待约 90 秒。恢复前后每份旧载体必须通过实际 `expiresAt`／`carrier-window` 保证尚未自然到期，场景标签 600 不得替代该门禁。

源 fixture 元数据保留 `normalTokenTtlSeconds=300`；另外两个场景请求使用
`keyRetirementRequestedProfileSeconds` 和 `longCarrierRequestedProfileSeconds`。
恢复文件加载器严格检查这些字段，拒绝旧 token/code/refresh 期限环境变量，不能将旧配置拷入恢复进程。

每次发布窗口记录数据库起止时间、真实时长、SDK 缓存更新时间及年龄、缓存中的公开 kid、分段等待次数和总请求等待毫秒。时间倒退、缺少实际缓存观察、等待超出预算、管理员来源不匹配或旧载体自然过期均为失败，不以 mock 时钟或静态通过报告替代实际演练。

## Phase 23 原载体期限与分组验证

canonical 授权码固定为 60 秒；当前 fixture 不提供 code 期限覆盖配置。驱动在 checkpoint 前最后签发一枚真实未消费代码，并保存数据库准确截止。隔离恢复、密码轮换、零应用连接的真实 SCRAM 和最终 gate 通过后，先启动目标身份双实例，重新加载标准客户端的新凭据，在代码仍未自然到期时验证原代码被拒绝；随后启动平台／门户连接池。USER、MACHINE、SSO、BFF 与派生 bearer 同样在每次拒绝请求前后检查原截止和真实 PG 时钟。RTO 使用单调时钟，另记录 UTC 起止及各恢复步骤耗时。

文件篡改与回放矩阵可以作为独立组件组执行。默认驱动仍在当轮恢复前实际运行九项；为保留短代码的观测窗口，可以显式传 `-FileNegativeEvidence <本目录对应 validation 下的正式 JSON 证据包>`。证据格式见 [校验器](phase23-file-negative-evidence.mjs)：它必须绑定同阶段已实际完成的九项只读负测、原报告字节摘要以及六个恢复／认证源码的逐文件 SHA256。browser 还会核对原报告中的实际结果与证据包逐项相等，并把 `separatelyExecuted` 写入报告。输入漂移、缺项、目标库非零、原字节未还原或源报告变化都会拒绝复用；不能据此跳过真实 restore-new 的认证预检、隔离恢复、reconcile、rearm、ACL 漂移及 gate。整体报告必须分别引用组件组与本轮旧载体／新业务证据。

RPO 的业务 marker 使用任务专属、没有成员的子组织；客户双候选验证使用另一组织。恢复对变化 marker 的保守禁用不影响客户正向用例，也不需要放宽 deny-only 合并规则。

`-ValidationProfile recovery` 是显式的恢复套件：只做 A 标准客户端缓存预热、C/D 至少 61 秒真实预发布和两次标准 SDK 签发，然后执行完整备份／撤销合并／隔离恢复／旧载体与新业务验证。报告使用 `RECOVERY_KEY_PREPARATION`，并标记 `keyLifecycleExecuted=false`；只读不变量仍验证全部旧来源、墓碑、receipt、代际及 C 撤销／D 活动，A/B 必须保持认证见证中的原事实。此套件没有执行 A 退役、B 生命周期或无活动签名者场景，不能单独完成整个 Phase 23。阶段验收必须另行引用同应用 binary／签名源码的实际完整密钥生命周期证据。默认 `-ValidationProfile full` 保留完整密钥演练。

## Phase 25 独立容量与故障复验

主验收环境清理并释放 TLS lease 后，由同一环境 owner 在 WSL root 下执行独立的新空库容量组。
`phase22-run.py --phase 25` 保留原 Phase 22 的负载与故障协议，并通过当前候选校验器核对完整源码、
七个构建组件及十二个二进制；集群、目录、inode、owner 和 marker 使用当前受保护 binding，
不接受历史集群 ID。`--label` 指向本轮工具快照；若只有工具变化，`--build-label` 可指向同编译输入的完整候选。

```bash
python3 -B /tmp/rust-saas-unified-auth-20261028/source-snapshot-PHASE25_TOOLS/saas-plateform/backend/tests/unified-auth-https/phase22-run.py \
  --phase 25 --label PHASE25_TOOLS --build-label PHASE25_CANDIDATE \
  --tls-workspace /mnt/d/WorkSpace/rust/rust-saas
```

替换两个标签为本轮已冻结的 `phase25-*` 名称。默认 TLS workspace 也是当前工作区，仅读取其
`.local-run` 证书和 CA；亦可显式指定本任务内的受保护 TLS workspace。创建前核对当前 HBA 是否允许
新建的专属 runtime 角色连接；HBA 仍由环境 owner 管理。此 runner 不创建容器、不改变共享 HBA 或正式网关。
它使用任务的 `phase21-node-runtime/bin/node`，复制工具时包含 binding 模块，并只为复制的控制器选择私有 metrics aperture。

Phase 25 模式禁止 `--faults-only`，每次实际新跑 CODE、REFRESH、INTROSPECTION 及数据库、平台超时、
身份重启、密码队列和本机告警场景。报告写入 validation 的 `phase-25-capacity-runtime.json` 与
`phase-25-capacity-cleanup.json`，不覆盖 Phase 22 历史报告。容量输入仍属于 owner SQL fixture，
terminal 告警事实仍明确标记人工构造；它们不能替代浏览器签发或真实通知投递的独立验收。

成功或失败都会按数据库 manifest 和进程身份执行 finally 清理，并比较私密日志后定向删除本轮临时文件。
报告列出保留的公开 `phase22-tools` 及其摘要；环境 owner 在整个 Phase 25 结束时继续清理这些公开工具、
快照和共享任务资源。清理失败会使报告失败并保留归属信息，不扩大删除范围。未执行真实 runner 时，
纯 mock 测试、Node 参数测试和语法检查只构成组件证据。
