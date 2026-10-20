# SaaS Platform

两套业务工程的独立发行、构建、部署与兼容矩阵见[独立业务交付](docs/independent-business-delivery.md)。

基于 Rust、PostgreSQL、React 的多租户、多组织权限与微应用平台。

系统架构、数据库、身份权限、前后端接入、发布、通知、部署及恢复统一维护在 [系统架构与应用接入](docs/architecture.md)。本文保留仓库入口和快速启动说明，组件目录 README 提供各自的命令与参数。

目标技术栈已确定为：前端及示例前端统一 Vite，后端统一 Rust，宿主前端独立部署到 Nginx；禁止 Bun、Caddy、Node 后端和 Next.js。下文组件表描述各组件的当前实现与迁移边界，具体迁移、微应用发布和完整分离示例要求见 [优化方案](docs/architecture-optimization-plan.md#nginx-rust-vite)。

目标交付推荐 GitHub Actions 构建镜像、Linux Docker/Compose 运行，由独立入口 Nginx 容器代理宿主与微应用静态镜像；保留 ZIP 持久卷兼容及外部 URL 登记。Phase 3 已实现 Nginx/镜像部署基础，Phase 4 已建立项目受管的分层 CI 与隔离验收入口。Phase 5 的同机 ZIP 实现增加不可变产物摘要、跨环境复用、上传重试对账和离线暂存维护，实际验收状态以阶段证据为准；业务 Rust 迁移继续按台账推进。首期平台不控制 Docker，工单和简单门店的目标是分别交付 Vite 前端、Rust API 与独立业务库，详见 [部署与运行决策](docs/architecture-optimization-plan.md#deployment-runtime-decisions)及[前端镜像部署方案](docs/architecture-optimization-plan.md#docker-frontend-nginx)。

## 系统组成

| 单元 | 技术与职责 |
| --- | --- |
| 平台 API | Rust / Axum / SQLx；身份、租户组织、授权、OAuth、通知持久化与应用发布 |
| 宿主前端 | React 19 / TypeScript / Vite；后台管理与微应用编排 |
| 兼容微应用 | Vue/jQuery classic 生命周期，独立 Vite dev/build/preview |
| 聊天示例前端 | 独立 React/TypeScript/Vite，默认离线、显式连接；旧聊天后台仍待迁移 |
| OAuth 演练 | 独立 Vite 页面 3010 与 Rust 流程 API 3011；秘密、PKCE 与 token 只在服务端 |
| 平台管理 CLI | 独立 Rust `saas-platform-admin`；默认离线计划，显式应用登记、授权与发布操作 |
| 通知服务 | 独立 Rust / Socket.IO；只读 PostgreSQL LISTEN、有界收件人调度与实时会话复验 |
| 独立业务应用 | work-orders：独立Vue 3／Vite前端、独立PostgreSQL及Rust API；包含读写／CSV、创建幂等、成员分配、持久提醒意图／正式通知发布，以及按当前业务身份隔离的前端内存草稿／未知结果恢复；Node参考模块仅用于测试，服务入口统一Rust |
| 门店业务应用 | stores：独立React／TypeScript／Vite前端、固定SDK、独立Rust crate及专属PostgreSQL库／账号；门店资料、受控负责人／员工关联、每日十进制营业记录与授权SQL汇总；公开登记manifest和内存草稿／显式未知结果读回，Phase28提供独立镜像交付入口，生产采用另行配置 |
| 网关 | 独立入口 Nginx；宿主静态 Nginx 镜像，HTTPS、版本路由、API 与 WebSocket |

默认 HTML 微应用通过 opaque iframe 和 SDK 接入，静态版本独立发布；第一方 ESM 模块包含在宿主构建中。业务服务使用应用 token，通过平台内省与实时授权上下文验证身份及权限。

Phase 6 建立了 [SDK 单一版本源](examples/micro-app-sdk/README.md)：首个登记版本 `1.0.0` 提供 classic/ESM、TypeScript、JSON Schema 和 Rust wire DTO，保持 context 1.0 与 iframe/MicroProps 2.0。宿主、Vue/jQuery 及工单消费者接入同版产物；`npm run sdk:sync` 生成，`npm run sdk:check` 在构建前拒绝 vendor/类型/DTO 漂移。SDK 强制省略 cookie、限制响应容量、保留安全 requestId 并区分 HTTP/网络/协议错误；现有静态 bridge 作为真实字节兼容基线保留，实际验收见 [Phase 6](docs/validation/phase-06.md)。

Phase 7 为可信应用的 load/mount/update/unmount 分别设置 10 秒截止。无法证明执行结束时，宿主取消当前授权工作、显示稳定故障并锁定本页可信运行域，需明确重新加载宿主恢复；后续隔离 iframe 使用独立销毁边界。截止不会终止原异步任务或同步循环，实际回归与限制见 [Phase 7](docs/validation/phase-07.md)。

Phase 8 将纯主题/路由更新与真实授权替换分开：相同授权的展示更新保留业务请求，凭据、期限、身份或授权变化仍取消旧工作。微应用资源与 props 使用同一份实时 context 入口快照；网络或 503 故障先取消旧授权、移除旧内容，再有界申请新 token/context。SDK 1.0.0 仍是未外部发行的仓库候选，wire 版本不变，旧静态文件需通过新前端版本采用这些行为；实际验收与兼容限制见 [Phase 8](docs/validation/phase-08.md)。

Phase 9 统一菜单查询的会话 epoch、取消和刷新规则，让侧栏、命令面板、工作台、最近访问和直达守卫共用路由策略。应用快选按选择代次拒绝迟到跳转；命令面板打开后才加载其全应用菜单。个人页面、平台根限定、权限与明确 MENU 各自有显式规则，未知授权拒绝；工单创建路径使用 `/micro/work-orders/ticket-create`，避免与详情读路由重叠。实施状态与实际验证见 [Phase 9](docs/validation/phase-09.md)。

Phase 10 的宿主会话采用单一原子记录与 Web Locks，所有标签页共享登录和当前租户/组织。刷新、登录替换、退出和切组织使用捕获会话条件提交，版本通知不携带凭据。一次性操作发送后响应丢失或页面关闭时，宿主停止旧请求和服务器视图，提示重新登录；不会重复使用结果未知的 refresh。Web Locks 或持久存储不可用时明确提示；BroadcastChannel 缺失使用存储事件与恢复重读路径。采用新宿主时需关闭或重载仍运行旧宿主 bundle 的标签页，回滚旧宿主需要重新登录。实际版本、验收状态与兼容边界见 [Phase 10](docs/validation/phase-10.md)。

Phase 11 的普通 OAuth 将授权码、设备码和 refresh 消费、完整 token 对及签发审计放入同一事务，userinfo、introspect 和 refresh 共用当前用户/直接成员/组织祖先/租户/客户端有效性规则。新 access 带随机 jti，数据库保存摘要与元数据；0003 保留旧无 jti/原文行的读取及撤销兼容，不自动清理历史凭据。机器 token 按客户端规则独立处理，平台 `psr_` 和应用 token 协议保持。动态恢复、未知提交及禁止混跑旧 issuer 的边界见 [普通 OAuth 契约](docs/oauth-issuance.md)，实际结果见 [Phase 11](docs/validation/phase-11.md)。

Phase 12 为平台登录与 OAuth password 增加有容量/TTL 的来源和账号预算，所有 HTTP 密码计算共用有执行与等待上限的 `PasswordService`。默认同时执行 2、排队 8，等待执行 250ms、等待结果 10 秒；超时或取消不提前释放仍在计算的许可。密码算法成本保持，来源只按真实 peer 与精确信任代理链解析；预算按进程独立，不能把多副本说成共享全局配额。配置、别名登录、代理头和采用边界见 [登录与密码保护](docs/login-protection.md)，实际状态与运行结果见 [Phase 12](docs/validation/phase-12.md)。

Phase 13 将关键用户、成员、角色、租户/组织、客户端、会话、应用及发布管理变更接入主事务审计；审计写入失败使对应变更回滚。删除前保存对象身份快照，相同配置/集合的重复 PUT 不制造变更事件；requestId 用于关联，不能充当幂等键。预检与部署切换各有独立事务，失败计数经受保护指标导出。动作清单、秘密边界及结果未知处理见 [关键管理审计](docs/management-audit.md)，实际验收见 [Phase 13](docs/validation/phase-13.md)。

Phase 14 的数据 grant API 提供目标快照、变更预览与带版本的差量保存，独立使用 `system:data-grant:read/write`。写入要求实际平台根和服务端 `DATA_GRANT_POLICY` 明确允许的业务租户/组织及应用能力，默认空策略拒绝；操作者自己的业务 grant 不决定转授上限。scope 更新保留 grant UUID、未改成员保持，真实变更与审计同事务，下一受保护新请求读取最新授权。Phase 25 已退役旧工单 Node 管理入口和 SQL 草案输出，日常维护使用正式 API；配置、权限采用和示例见 [受控数据 grant 管理](docs/data-grant-management.md)，实际结果见 [Phase 14](docs/validation/phase-14.md)。

Phase 15 增加独立 `/system/data-grants` 页面，以受限应用/直接成员选项读取当前范围，先预览精确差量再确认保存；编辑要求实际平台根、独立 write 权限和目标可写策略，具备独立 read 且目标可读的组织管理员仅能查看。409 和保存结果未知时必须重新读取并审阅；403 隐藏旧快照并退出写态，手动重读获准后只读展示。审计查询另需 `system:audit` 并关联本次 requestId。已有安装需显式注册并授权新 MENU，保留原读写按钮；普通工单用户的功能菜单、角色、直接成员数据范围、验证和撤销步骤见[操作指南](docs/data-grant-user-guide.md)，实际结果见 [Phase 15](docs/validation/phase-15.md)。

Phase 16 建立独立 `work-orders-api` Rust crate、自己的 Cargo 锁文件与业务 PostgreSQL，在同一旧迁移锁事务中安全初始化或登记 SQLx 接管，保留存量业务记录与身份。Phase 17 增加逐请求机密客户端内省与应用 Bearer context 校验，列表、count、详情、筛选、分页和 CSV 共用租户／组织硬上限／data grant 的参数化 SQL 范围；保留 `Origin:null` CORS、上下游 requestId 和明确错误。

Phase18增加Rust创建、版本更新、状态转换与软删除。创建必需 `Idempotency-Key`，在tenant／org／user／create／key域内将规范正文摘要、工单和原201同事务保存；未知结果同键／正文恢复，PATCH／DELETE先读回版本、不自动重试。新增0002兼容旧Node及SQLx双历史，0001原字节保持；历史结果见 [Phase18](docs/validation/phase-18.md)。

Phase19接入普通应用Bearer的成员目录和真实分配预校验，平台策略须显式配置 [memberLookups](docs/data-grant-management.md#member-lookups) 的ticket/create、ticket/update，默认空且不改变capabilities公开三字段DTO。需要新分配时，范围内预读释放业务连接后才调用平台成员HTTP，再持行锁复核组织／version／旧处理人；变动409，校验从请求开始单调计时、最多3秒，不在锁后续期。成员HTTP超时503 `PLATFORM_UNAVAILABLE`，提交前成员结果过期503 `MEMBER_VALIDATION_EXPIRED`并回滚；提交后context／成员到期仍用 `CREATE_RESULT_UNKNOWN`／`WRITE_RESULT_UNKNOWN`恢复。省略／同UUID更新保留原快照，真正change／clear受组织scope限制；创建历史重放不重查旧member，但仍须当前create授权。跨平台／业务库仍有验证后到提交间的竞态，不承诺跨库原子性。Phase19本身未新增schema或依赖，历史结果见 [Phase19](docs/validation/phase-19.md)。

Phase20增加独立 `POST /work-orders-api/tickets/{id}/remind`，正文仅 `{idempotencyKey}`，区别于创建header。当前读工单及notification发布双重权限／grant和组织上限校验后，在短业务事务中保存不可变提醒意图；提交并释放连接后，才以当前应用Bearer向平台官方通知API发送一次。0003意图保存可信应用／发布者域、工单／收件目标和完整正文，不保存token、不缓存本地成功；同键换工单或当前目标变化409，即使原平台POST未提交。准备结果未知与发布结果未知分别恢复，不自动换键或重发；平台200后不重新读取已变化工单再改判失败。收件箱持久结果与实时Toast分开，未启动实时服务也可核对发布；不改工单version，不直接写平台通知表。0003库不能简单退回原Phase19，旧Node提醒也不提供新意图保障。详细采用／回退见 [Rust工单说明](docs/work-orders-rust-baseline.md)，实际证据由 [Phase20](docs/validation/phase-20.md)记录；Phase20本身未实施前端恢复、实时改造或生产切流。

Phase21将工单前端改为独立锁依赖的Vue模块／Vite标准入口，使用相对ESM、动态chunk和登记SDK1.0.0；生产仅交付静态文件，由Nginx提供正确MIME和匿名CORS，opaque iframe仍以Origin:null、应用Bearer和省略Cookie的请求消费Rust API。创建保存一次逻辑事件的header键／完整正文，提醒保留独立事件键；网络、协议或5xx结果未知时由用户显式恢复原事件，不自动重发。409编辑保留输入，版本写入未知先读回当前状态。草稿／intent仅在同一存活iframe内按app／tenant／organization／user分区，跨路由、续期和切组织返回可恢复相应状态；刷新或销毁iframe不保证保留，请保持页面直至未知结果核对完成。它不使用opaque localStorage或新增SDK能力，与后端0003持久意图不同。创建与详情仍要求各自精确MENU；实际构建／浏览器／平台联合结果见 [Phase21](docs/validation/phase-21.md)，不据实现说明预记验收通过，也不执行生产部署或切流。

Phase26新增 [stores独立Rust后端](business-apps/stores/README.md)，逐请求以机密客户端Basic内省及stores应用Bearer context验证身份，门店／员工关联／日报各按自身permission、grant与组织硬上限执行SQL。store owner固定创建人，staff owner为经平台验证的关联员工，report owner为录入人；负责人或员工关系不扩大其他资源SELF。新关联先做最多3秒的成员HTTP预校验，再在短事务复核，跨平台库／业务库仍有最后检查至COMMIT竞态。日报金额为NUMERIC(18,2)及API十进制字符串，营业日默认Asia/Shanghai当天、允许有效历史／未来日期，同店同日唯一冲突409，未知写先读回不自动重发。平台／工单／门店三库三账号独立；[配置及API](business-apps/stores/backend/README.md)只描述当前实现，真实构建／三库联合与并发验收以本阶段实际证据和台账为准，不包含Phase27前端、应用菜单manifest或Docker部署。

Phase 25 的 [Rust 平台管理工具](tools/platform-admin/README.md) 提供 inspect、bootstrap、grants-plan/grants-apply 与分步发布命令。独立 crate 使用自己的锁文件；默认计划不联网，写操作显式加 `--apply`，Bearer 仅从环境读取，OAuth 客户端使用平台生成的实际 ID。管理请求限定精确 HTTPS origin、拒绝 redirect、限制超时与响应容量；结果未知时先读取状态。授权经正式 API，仍受实际平台根、独立 grant 读写权限和服务端策略约束，不隐含 scope。工具不执行 SQL、不控制 Docker；前端 Node 构建与尚未完成的业务迁移保持各自范围。

Phase27新增门店独立React19.3.0／TypeScript5.9.3／Vite8.3.4工程和登记SDK1.0.0，`npm ci`与dev／build／preview均在自身目录执行。相对ESM和动态chunk、公开配置及asset-manifest／build-info为后续镜像化输入；根`build:stores:frontend`构建前端，`build:stores`仍为Rust。11项菜单、九个capability和store/create、store/update、store-staff/assign三种成员用途独立登记；员工／日报页直接接受门店ID，不隐取store/read。表单显式发送上海营业日，保留十进制字符串、409原输入及未知写显式读回；内存草稿按app／tenant／organization／user隔离，刷新或销毁不保证保留。双页共享宿主登录和当前组织，不新增门店会话。实现细节见[前端README](business-apps/stores/frontend/README.md)，双消费者差异与真实成员HTTP见[成员契约](docs/stores-member-contract.md)。正式r2的Chromium／Chrome／Edge各21个必需浏览器组通过、零失败／跳过，实际执行独立空缓存安装构建、真实平台／两业务Rust／三库、普通用户操作及跨页组织隔离；完整证据与其他门禁分别见[Phase27验证记录](docs/validation/phase-27.md)。Phase28镜像发行／公开v1v2路由、生产部署与公共框架不在本阶段。

门店客户端每次HTTP默认15秒绝对总期限，包含fetch与成功／错误正文读取；读取超时给安全提示，已发送写入超时保留未知结果并要求读回核对，不自动重发，也不撤销当前context或其他动作授权。

门店收到403响应头即停用对应resource/action并取消同用途并行请求，不等待错误正文；同用途迟到200不能回填旧行或解除拒绝，纯主题更新也不解锁。401使当前context失效并隐藏全部旧服务器视图。已发送写入在路由切换／同身份续期后共享原请求的inFlight状态，原请求结束前不能读回、确认或再次提交；未知员工解除关联即使查询已无原行，仍按原事件保留核对入口。空列表只表示当前授权下的读回结果，不是历史删除成功回执；双消费者的字段映射、成员用途与跨库竞态限制见[成员契约](docs/stores-member-contract.md)。

## 文档导航

- [系统总览与模块边界](docs/architecture.md#overview)
- [数据模型与迁移](docs/architecture.md#data)
- [租户、组织与数据授权](docs/architecture.md#authorization)
- [受控数据 grant 查询、预览与差量维护](docs/data-grant-management.md)
- [普通工单用户功能授权、数据范围与撤销](docs/data-grant-user-guide.md)
- [登录、会话与应用 token](docs/architecture.md#identity)
- [普通 OAuth 原子签发、实时失权与升级兼容](docs/oauth-issuance.md)
- [OAuth 演练：Vite/Rust、流程矩阵与独立 HTTPS 镜像](mini-services/oauth-playground/README.md)
- [登录预算、异步密码计算与可信来源](docs/login-protection.md)
- [关键管理变更、事务审计与重试](docs/management-audit.md)
- [后台验证 token 与获取用户信息](docs/architecture.md#backend-integration)
- [Rust 工单授权、读写／CSV、成员、创建与提醒恢复](docs/work-orders-rust-baseline.md)、[独立Vue／Vite前端与恢复生命周期](business-apps/work-orders/frontend/README.md)及[旧 Node 授权参考退役说明](examples/app-backend/README.md)
- [简单门店业务](business-apps/stores/README.md)、[独立React／TypeScript／Vite前端](business-apps/stores/frontend/README.md)、[配置与HTTP契约](business-apps/stores/backend/README.md)及[双消费者成员契约与实证](docs/stores-member-contract.md)
- [微应用前端接入](docs/architecture.md#microfrontend)及[各框架开发示例](docs/architecture.md#frontend-examples)
- [构建、上传、激活与回滚](docs/architecture.md#releases)
- [工单／门店固定镜像、持久发行清单与外部 URL 接入](deploy/microapps/README.md)
- [通知协议](docs/architecture.md#notifications)
- [Rust 实时协议选型与客户端支持矩阵](docs/realtime-protocol.md)
- [生产配置与部署](docs/architecture.md#deployment)
- [监控](docs/architecture.md#observability)、[备份恢复](docs/architecture.md#recovery)和[验收边界](docs/architecture.md#validation)
- [前端微应用与整体架构优化方案](docs/architecture-optimization-plan.md)（Nginx/Vite/Rust 部署、分离示例与分阶段验收）
- [架构优化任务台账](docs/architecture-optimization-ledger.md)（实施状态、阶段依赖、验收与当前执行指针）
- [工程与验证基线](docs/engineering-baseline.md)（工具版本、组件/遗留运行入口、协议与分层检查清单）及 [Phase 1 实测证据](docs/validation/phase-01.md)
- [Phase 2 实测证据](docs/validation/phase-02.md)（独立 Vite 示例、版本资源、Bun 清除和受控开发入口）
- [项目受管 CI](scripts/ci/README.md)及 [Phase 4 实测证据](docs/validation/phase-04.md)（干净 Linux、隔离 PostgreSQL、受管浏览器与 OCI/HTTPS 交付门禁）
- [ZIP 产物对账与暂存恢复](deploy/zip-recovery.md)及 [Phase 5 验证记录](docs/validation/phase-05.md)（实现说明与实际验收结果分别记录）
- [版本化 SDK 与框架接入](examples/micro-app-sdk/README.md)、[公开 wire/schema](examples/micro-app-sdk/contracts/README.md)及 [Phase 6 验证记录](docs/validation/phase-06.md)

## 仓库结构

```text
backend/                    Rust 平台 API、初始化基线与增量迁移
frontend/                   React 宿主与内置 ESM 模块
business-apps/work-orders/  独立业务前端、后端和部署工具
mini-services/             通知服务、OAuth 和微应用演示
examples/                  前端 SDK、Rust 聊天与历史授权兼容测试
scripts/                   版本打包与开发辅助工具
deploy/                    Nginx入口/宿主镜像、Compose模板、原生构建、Rust私有监控；恢复待Phase33
docs/architecture.md        统一架构与应用接入文档
docs/sql/                  可执行的只读关系审计 SQL
tests/                     契约、数据库、浏览器和网关测试
```

## 本地开发

准备 PostgreSQL 17、Rust 1.97.0（由根 rust-toolchain.toml 固定）、Node 24.18.0（.nvmrc / .node-version）和 npm 11.16.0。先运行 node --version、npm --version 核对实际 PATH，再配置开发数据库与独立随机密钥，分别启动后端和前端。

在 backend 目录设置环境或本地 .env，示例中的密码和密钥须自行替换：

```dotenv
DATABASE_URL=postgresql://platform:YOUR_PASSWORD@127.0.0.1:5432/saas_platform
JWT_SECRET=YOUR_RANDOM_SECRET
BIND_ADDR=0.0.0.0:3001
AUTO_CREATE_DATABASE=false
```

```powershell
# 在 backend 目录。
cargo run --locked
```

```powershell
# 仓库根目录；每个前端使用自己的锁文件安装。
npm ci --ignore-scripts
npm --prefix frontend ci
npm --prefix mini-services/micro-vue ci
npm --prefix mini-services/micro-jquery ci
npm --prefix examples/websocket ci
npm run dev
```

`npm run dev` 在前台启动宿主 3000、Vue 3005 和 jQuery 3006，Ctrl+C 清理本次拥有的进程树；它不会启动数据库或后台。`npm run dev:all` 另加独立 WebSocket 聊天前端 3007，该页面默认离线。只启动宿主可用 `npm run dev:frontend`。端口占用会失败并保留原监听者，不附着已有进程或自动换端口。

端口和监听地址可通过 `npm run dev -- --host ::1 --host-port 3100 --vue-port 3105 --jquery-port 3106` 配置，宿主代理随之更新；完整选项见 `node scripts/dev.mjs --help`。旧 shell 脚本也转交同一调度器。后台仍需独立、明确配置并启动；`--with-backend` 等可选参数会访问真实服务，平台启动可能执行 SQLx 迁移。

Vue/jQuery 现在各有独立 Vite `dev/build/preview`，保留 classic `__MICRO_APP__` 兼容生命周期；WebSocket 示例也有独立 React/Vite 工程。入口分别见 [Vue](mini-services/micro-vue/README.md)、[jQuery](mini-services/micro-jquery/README.md)、[聊天前端](examples/websocket/README.md)。版本目录缺失时宿主可正常启动，对缺失版本和静态资源返回 404。

OAuth 演练已拆为独立 Vite 页面与 Rust API，原 Node HTTP 入口及内嵌页面已移除。先在 `mini-services/oauth-playground/frontend` 独立 `npm ci`，配置 Rust 私有 `OAUTH_*` 环境及平台注册 callback 后，使用 `npm run dev:oauth` 启动默认页面 3010/API 3011；可用 `--oauth-port`、`--oauth-api-port` 和 `--host` 调整。平台不随该命令启动，历史 password 兼容默认关闭；四种流程、秘密边界及非 root HTTPS 独立部署见 [OAuth 演练说明](mini-services/oauth-playground/README.md)，实际验收见 [Phase 24](docs/validation/phase-24.md)。

宿主默认 http://localhost:3000，平台就绪 http://localhost:3001/api/health。宿主代理可通过 BACKEND_PROXY_TARGET 指定。空库启动时平台通过 SQLx 顺序执行 [初始化基线 0001](backend/migrations/0001_init.sql)、[产物绑定迁移 0002](backend/migrations/0002_release_artifacts.sql) 和 [OAuth 存储迁移 0003](backend/migrations/0003_oauth_issuance.sql)；已采用兼容基线的库只追加未执行迁移，不重建数据库。需要受控首次建库时显式启用 AUTO_CREATE_DATABASE，账号必须具有相应权限。

配置优先级为系统/服务环境优先；生产不自动加载 .env，使用绝对 APP_ENV_FILE 或服务管理器明确配置。开发空库可能初始化演示数据，生产使用独立初始化参数和安全门禁，详见 [运行部署](docs/architecture.md#deployment)。

实时服务和独立业务服务另行启动：

- [Rust 通知服务启动、容量与切流](mini-services/notification-ws/README.md)，[Phase 23 验证记录](docs/validation/phase-23.md)
- [工单前端构建](business-apps/work-orders/frontend/README.md)
- [工单后端启动与数据库](business-apps/work-orders/backend/README.md)
- [Rust 工单：独立构建、平台授权与只读／CSV](docs/work-orders-rust-baseline.md)
- [工单应用登记与授权](business-apps/work-orders/deploy/README.md)

早期整合时的单一初始化基线替换是历史迁移边界，不能作为当前删除数据库或重置 SQLx 记录的操作指令。当前升级保留已执行的迁移及校验和，追加 0002/0003 等未执行项；更早且不兼容的库须先在隔离副本评估迁移。0003 采用前须停用并排空旧 OAuth issuer，回退使用保留新存储/状态契约的兼容构建，不删除迁移记录或降 schema。默认数据库为 `saas_platform`，环境文件参数为 `APP_ENV_FILE`，后端程序为 `saas-platform-backend`。已有私有配置须同步更新；浏览器会话和偏好采用 `platform-*` 存储键，更新后需要重新登录。微应用宿主与 SDK 须一同更新，使用 `MicroIframe`、`MICRO_*` 消息和 `__MICRO_APP__` 生命周期导出。

## 发布与运维

Phase 3 已实现独立入口 Nginx、非 root 宿主静态镜像、私网 Compose 与原生干净构建。默认镜像部署没有微应用共享卷；ZIP 使用独立兼容 override。配置、证书、网络和旧 hash 资源保留步骤见 [部署说明](deploy/README.md)，本次实际运行的结果以 [Phase 3 证据](docs/validation/phase-03.md)为准。

Phase28新增[工单／门店镜像交付](deploy/microapps/README.md)和应用管理中的“登记已部署 URL”。外部先部署并保留固定版本路由，再登记公开 HTML 入口；登记得到 DRAFT，单独预检的正文必须为 PASSED，HTTP200加FAILED仍拒绝激活。平台只管理入口与发布指针，不拉取镜像、启停容器、创建Nginx路由或自动授予用户权限。UI与原RegisterBody只提交environment／version／entryType／entryUrl／healthUrl／resources，镜像来源和digest由Rust CLI的独立持久inventory绑定，不存入该平台接口。

GitHub候选构建、受保护的手工镜像推送与Linux目标部署分别执行。Linux部署方明确提供registry、架构、真实HTTPS来源、证书、三座数据库的独立账号及各业务实际机密客户端；所有环境共用同一权威持久inventory。推进顺序为固定digest拉取和容器health、CLI绑定／核验、受控版本路由、登记／预检／激活，再独立核对普通用户授权。v1相对API路径用于同域入口，外部静态origin的v2明确使用平台绝对HTTPS API地址。当前受管交付驱动以冻结源码在Linux按Cargo.lock编译并核验两份API ELF后装箱，四个静态镜像由各自前端源码锁构建；该模式不代表执行默认API Dockerfile builder。工作流文件、源码或本地演练均不表示已执行远端GitHub推送或生产部署；Phase28状态与实际验收另见[台账](docs/architecture-optimization-ledger.md#phase-28)。

同机 ZIP 通过私有随机暂存目录、规范清单及同文件系统无覆盖原子安装发布。一个应用版本跨环境共享同一产物摘要，同环境同内容重试保留 release ID；提交或响应结果未知时保留正式文件，通过 storage 的四种状态核对后重试。离线 `--micro-staging` 默认为 SCAN，只可显式清理归属可验证的暂存项，不删除正式版本。升级前须停止全部旧上传/发布写入并等待在途请求结束；旧 writer 不遵守新锁，不能与新版混用同一卷。完整操作及 0002 回退边界见 [ZIP 恢复说明](deploy/zip-recovery.md)。

```sh
docker compose --env-file /secure/platform.env config --quiet
docker compose --env-file /secure/platform.env up -d
```

这些命令面向已审查的实际部署目标；配置检查不启动数据库，up会执行正常服务启动行为，不代表现网已部署。通知已由Phase23独立Rust服务接管，使用专用notification profile。工单Rust包含授权、读写／CSV、成员预校验及持久提醒意图／正式通知发布，Phase21前端已接入创建键与显式未知结果恢复；旧Node工单入口与legacy部署已退役；业务部署使用Rust business profile。原生构建见 [deploy/native](deploy/native/README.md)；Phase32新增[私有Rust监控与候选基线](deploy/monitoring/README.md)，实际验收及性能限制见[Phase32证据](docs/validation/phase-32.md)；恢复仍属于Phase33，历史恢复工具链接不能当当前交付。

## 维护与验证

项目受管入口为 `node scripts/ci/run.mjs --gate fast|postgres|browser|delivery|business-delivery --out-dir <全新绝对目录>`。GitHub PR 运行 fast、隔离 PostgreSQL 和 Chromium/Chrome/Edge；main 或手动候选另有宿主交付与工单／门店business-delivery门禁，具体依赖以workflow为准。业务门禁声明隔离registry真实push／固定digest pull、独立Compose、URL管理UI和SDK／Rust授权验收，实际结果须按本阶段报告核对。必需测试失败、跳过、取消、零测试或缺报告都会失败；配置和本地实测不表示 GitHub 托管 runner 已运行。安装、各层前置条件及源码/版本报告见 [CI 使用说明](scripts/ci/README.md)。

Phase 5 当时的清单为 fast 的 99 项无 DB 契约及归档安装/暂存维护 Rust 单元组、postgres 的 5 套件 27 项，以及 delivery 的 9 项 ZIP 联合检查，历史结果见 [Phase 5 验证记录](docs/validation/phase-05.md)。Phase 11 为七组 41 项，Phase 12 加入登录保护后为八组 46 项；Phase 13 声明新增管理审计八项，并纳入既有通知/成员十项，规划十组 64 项，精确清单见 [CI manifest](scripts/ci/manifest.mjs)，实际数量及结果由 [Phase 13](docs/validation/phase-13.md) runner 证据确认。fast 声明 27 步，包含 limiter、密码服务和 client IP 回归；实际执行、失败及重跑见各阶段报告，不继承历史 PASS。

```powershell
# 仓库根目录；Windows PowerShell 与 Linux shell 使用相同检查命令。
npm --prefix frontend ci
npm run lint
npm run typecheck
npm run test:contracts
cargo check --locked --manifest-path backend/Cargo.toml --all-targets
cargo test --locked --manifest-path backend/Cargo.toml --bin saas-platform-backend
```

前端 lint 使用 ESLint，typecheck 单独运行 TypeScript，无数据库契约组使用 Node 测试驱动。单 crate 的 fmt、Clippy 和文档检查也有根 npm 脚本，完整命令、既有告警及环境分类见 [工程基线](docs/engineering-baseline.md)与阶段证据。根目录现有 npm 锁文件，Bun 命令/API及 bun.lock 已在 Phase 2 清除；根依赖中保留的旧后台示例依赖仍待相应 Rust 阶段迁移。

```powershell
# 独立静态构建；build:all 先构建示例，再同步兼容副本，最后构建宿主。
npm run build:examples
npm run micro:sync:examples
npm run build
# 等价的统一构建入口。
npm run build:all

# 受控开发进程与 Vite/版本资源/产物配置隔离回归。
npm run test:dev
npm run test:vite
```

各工程产物为 `dist/index.html` 与版本内相对资源。兼容副本位于 `frontend/public/sub-apps/{vue,jquery}`，需要独立版本时用现有 `scripts/package-microapp.mjs --input <工程>/dist --output-root <明确目录>` 打包；不要以 `public/` 代替 Vite 构建目录。Phase 3 验收通用 Nginx/容器传输与静态交付，业务镜像联合交付仍归 Phase 28。

实际数据库和浏览器检查有独立环境要求，入口与证据边界见 [验证章节](docs/architecture.md#validation)。跳过不等于通过，不在生产数据库运行隔离测试。

新增持久化能力必须新增迁移，不修改已执行 SQL 校验和。架构变更同步更新统一文档，组件 README 保留各自执行说明，避免重新建立重复设计文档。


## Phase 28 正式验收（2026-10-19）

工单／门店镜像与统一外部 URL 接入已完成本阶段隔离验收：同一 839 项源码输入下，完整 fast 63 步、1083 项通过，0 失败／跳过；真实六镜像 registry push／候选清除／固定 digest pull 和 Compose 联合检查 14/14 通过，Chromium／Chrome／Edge 各 9/9 通过。来源、实际命令、旧失败及五项原验收映射见[Phase 28 验证报告](docs/validation/phase-28.md)。

Phase 28 的外部 URL 流程只登记、预检与激活公开 URL；digest 不变性由 Rust CLI／CI 权威持久 inventory 负责。两 API 按 Cargo.lock 在隔离 Linux 编译、核验 ELF 后 runtime 装箱；默认 API Dockerfile 的源码 builder 未执行。同一 daemon 的 pull 前确认本轮候选镜像身份已清除，基础层缓存可能仍在；本结果不声称跨机器空缓存。远端 Actions、外部 registry 发行与生产部署未执行。前端回退须保留旧版本路由与资源，API／数据库独立升级。

Phase33新增独立Rust加密三库备份和隔离新库恢复：[恢复runbook](deploy/recovery/README.md)。试点候选24h/4h；完整远端取回、凭据失效与业务验收结果见阶段证据。生产PITR、真实异机/密钥托管、外部告警和定时安装另行验收。

Phase36 提供可选的受管静态镜像部署，配置、独立权限、Rust agent、READY 回执与激活/清理边界见 [受管部署说明](deploy/managed/README.md)。未配置目标时继续使用现有外部 URL 流程。验收范围仅本机隔离 Docker 环境，不代表真实受管目标已接入。

Phase36 本机隔离受管部署验收已完成，正式源码、81步fast/18部署/17发布回归与清理证据见 [Phase36实施报告](docs/validation/phase-36-implementation.md)。
