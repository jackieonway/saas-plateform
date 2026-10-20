# 工程、运行入口与协议基线

最近更新：2026-10-19（Asia/Shanghai），同步Phase28固定业务镜像、外部URL管理、持久发行清单与分层交付入口，保留Phase26／27双消费者及此前阶段事实。本文记录当前源码可核对的事实及后续迁移归属；阶段状态以[架构优化台账](architecture-optimization-ledger.md)为准，实际执行与验收结论由各阶段证据单独记录。工具基线见[Phase1](validation/phase-01.md)，Vite／开发入口见[Phase2](validation/phase-02.md)，Nginx／镜像见[Phase3](validation/phase-03.md)，分层CI与干净Linux／PG／浏览器／交付见[Phase4](validation/phase-04.md)。Phase5的实际状态见[验证记录](validation/phase-05.md)，接口和运维流程见[ZIP对账与暂存恢复](../deploy/zip-recovery.md)。命令清单与实际证据分别维护，不以新增声明替换历史失败或冒记PASS。

2026-10-16 阶段编号同步：主线按执行顺序统一为 Phase 1–34，条件扩展为 Phase 35–37。本文后续阶段引用采用当前编号；历史验证记录和证据文件保留原编号，对照见[阶段编号映射](phase-numbering-map.md)。

目标是全部前端及示例前端使用Vite、后台使用Rust、生产静态及入口使用Nginx。Phase2清除活动Bun入口，Phase3替换Caddy；Phase21工单已使用独立Vue模块／Vite工程，保留的Node业务后台仍属兼容路线，具体阶段如下。源码实现不等于完成实际验收，也不授权操作生产。

Phase 7 同步：宿主可信load/mount/update/unmount各有10秒截止和页面级故障锁存；隔离iframe不再等待可信卸载barrier。`tests/micro-runtime-faults.mjs`进入本地与正式CI必跑契约，覆盖永久pending、取消、迟到结果和回收失败；真实产品浏览器额外验证故障UI、运行域分离与整页恢复。截止不终止可信代码，实际验收以 [Phase 7](validation/phase-07.md) 为准，后续阶段不继承该阶段通过结果。

Phase 8 同步：相同epoch、凭据、两种期限、完整授权/部署投影及signal的纯展示更新复用授权；真实替换取消旧请求。宿主、bridge、SDK client和工单采用同源分类，签名只存内存、不进日志；工单主题不重取列表，路由读取独立取消，失权立即清除服务端快照。可执行入口取自单一实时context，profile完整tuple变化只触发复核；网络/503取消旧授权后最多3次退避恢复，单尝试5秒、单恢复过程20秒，不重试真实失权或契约/模式错误，也不解除Phase7故障锁。当前SDK1.0.0候选由统一生成入口更新，wire、legacy、后端及锁文件不变；旧静态版本不被覆盖，实际验收见 [Phase 8](validation/phase-08.md)。

Phase 9 同步：宿主菜单/权限/profile查询使用统一options工厂，菜单仍按应用与会话epoch隔离，所有调用传递取消signal并复核返回时的会话；同epoch的正常token轮换不分裂缓存，全局cancel/clear保持。展示入口与直达守卫共用显式策略：个人工作台/个人中心/本人通知允许登录用户访问，宿主页要求对应权限和精确MENU，平台根页面仍校验平台上下文，微应用使用本应用明确模式，外链要求精确URL菜单，未知拒绝。选应用按共享代次防迟到导航，工单创建固定为`/micro/work-orders/ticket-create`并同步菜单/工具校验；实际验收与旧菜单采用边界见[Phase 9](validation/phase-09.md)。

Phase 10 同步：宿主通过Web Locks串行会话操作，access/refresh只存一份带版本的原子记录，跨页共享登录及当前组织；广播仅通知版本，旧键只受锁导入一次。一次性请求发送后无法确认结果时隔离旧服务器视图，不重耗旧refresh；登录替换、退出和切组织按捕获会话条件提交，原请求最多重试一次。缺少Web Locks或持久存储明确拒绝，BroadcastChannel缺失使用存储事件与恢复重读。旧宿主bundle须关闭/重载，回滚旧宿主需重新登录；实际多页/版本矩阵及限制见[Phase 10](validation/phase-10.md)。

Phase 11 同步：普通 OAuth 由 `oauth_issuance` 维护统一实时身份规则，code/device/refresh 消费、完整 token 对与签发审计同事务；同秒新 token 用随机 jti 独立撤销。0003 用 storage_version 区分旧行与新签发，新行不写 JWT 原文，旧无 jti/原文行仍可读取及撤销，不自动清理。userinfo/introspect/refresh 拒绝停用、到期、丢失直接成员或不完整上下文；机器 token 按客户端独立判断。恢复有效状态可使未撤销/未消费/未到期旧凭据恢复；显式撤销和消费保持终态。管理员 kick 的平台会话/OAuth 撤销同事务属于本期闭合，全面管理审计仍归 Phase 13。采用/回退须保留 0003 且排空旧 issuer，未知提交需重新授权，详见[普通 OAuth 契约](oauth-issuance.md)及[Phase 11 实际证据](validation/phase-11.md)。

Phase 12 同步：进程内 `LoginLimits` 统一来源/输入/账号的 4096 条共享容量、600 秒单调固定窗口，来源默认 60 次准入、账号默认 5 次已记录失败；键长度与每次过期回收有界，饱和拒新不驱逐活跃预算。全 HTTP 登录/OAuth password/创建/改密/重置/联系人初始化共享 `PasswordService`，默认执行 2、等待 8、队列 250ms、结果 10000ms；超时或取消的任务仍持许可到真实结束。账号标识默认最多 320 字节，共享别名全部候选验证共用 10000ms，不返回部分 ticket；成功匹配不惩罚同别名其它账号。来源从真实 peer 与精确信任代理链右侧解析并规范化，不使用任意最左 XFF。Argon2 默认 m19456/t2/p1 保持，非默认 PHC 成本不由本次默认配置测量覆盖。预算仅进程内、N 副本有 N 份且重启清空，未增加 Redis/共享全局模式或生产 Nginx 限流；启动 seed 独立。配置与边界见[登录保护](login-protection.md)，实际通过/失败与压力结果见[Phase 12](validation/phase-12.md)。

Phase 13 同步：关键用户/成员/角色、租户/组织、客户端/撤销、会话/密码、应用/发布及平台设置在业务事务内调用统一审计 helper；失败则回滚，缓存失效在提交后。删除先保留身份快照，普通配置与规范集合的无实质变化重试不重复写变更事件，requestId 仅关联。预检独立提交观察结果，不与后续部署事务混为一体；统一 helper 写入失败有日志和进程内饱和指标，不能外推为全部历史原生 SQL 签发审计的计数。敏感正文/凭据不进详情，通知既有持久发布事务与诊断 best-effort 分类保持。完整清单与限制见[关键管理审计](management-audit.md)，实际结果见[Phase 13](validation/phase-13.md)。

工单Phase16建立独立Rust crate与严格业务库接管，见 [Phase16](validation/phase-16.md)；Phase17增加有界平台授权、同范围读取／CSV和CORS／requestId，见 [Phase17](validation/phase-17.md)；Phase18增加创建幂等、版本写入及0002，见 [Phase18](validation/phase-18.md)。Phase19成员分配按非锁预读→释放连接→平台HTTP→锁内组织／version／oldAssignee复核，单调窗口最多3秒且不续期，见 [Phase19](validation/phase-19.md)。Phase20当前read／publish双授权后保存0003不可变意图，提交并释放连接才POST；同键换工单／目标409，准备与发布未知分别恢复，不改version，不因实时未就绪改判持久失败。跨库不原子，0003不能直接回退原Phase19，旧Node不提供新意图语义，见 [Phase20](validation/phase-20.md)。完整契约／回退见 [Rust说明](work-orders-rust-baseline.md)，生产未切流。

Phase21工单前端独立锁定Vue3.5.43／Vite8.3.4，标准模块入口和相对动态chunk，生产由静态Nginx提供MIME／匿名CORS／真实404。创建首次固定header键与完整DTO，提醒固定独立事件；仅验证合法成功DTO后结清，网络／协议／5xx与畸形2xx保留原事件并显式恢复。版本409保留输入，版本写入未知先读回；纯主题不多取列表，失权立即隐藏服务器内容。草稿／intent仅同一存活iframe内按app／tenant／org／user分区，跨route／续期／切组织返回可恢复，刷新或销毁不保证保留；无opaque localStorage／新SDK能力，区别于后端0003持久意图。创建／详情精确MENU、Origin:null／省略Cookie及独立依赖保持；实现与实际验收分开，结果由 [Phase21](validation/phase-21.md)记录，不预记PASS或生产部署。

Phase26新增 [stores-api独立crate](../business-apps/stores/backend/README.md)及自己的Cargo.lock，配置只读取STORES_DATABASE_URL专属库；平台／工单／门店三库三账号独立。store、store-staff、store-daily-report各有自身permission／grant／SQL scope，员工动作是assign／unassign，汇总使用report/read；三类owner分别为门店创建人／被关联员工／日报录入人，manager与关联身份不扩SELF。平台逐请求Basic内省＋应用Bearer context，成员窄接口预查绝对窗口最多3秒后再短事务复核，不承诺跨库原子；日报用NUMERIC(18,2)及十进制字符串、Asia/Shanghai日期与同店同日唯一409，未知写先读回。Phase26只交付后端；[API／配置说明](../business-apps/stores/backend/API.md)与源码可核对，实际门禁不能据此预记PASS。

Phase27新增[门店独立前端](../business-apps/stores/frontend/README.md)，精确锁定React19.3.0／TypeScript5.9.3／Vite8.3.4，固定SDK1.0.0完整ESM与类型，Node24.18.0／npm11.16.0独立npm ci／dev／build／preview。薄入口同步连接bridge再加载React，relative base、opaque Origin:null／omit Cookie与公开config／asset-manifest／build-info保持边界；VITE环境不自动注入。11项菜单、九capability和三memberLookups由公开manifest／Rust CLI对齐，不串用工单allowlist。门店id DTO和员工userId映射、三个owner／scope差异及真实两消费者成员HTTP见[成员契约](stores-member-contract.md)。上海日期显式发送、十进制金额／SQL汇总、409重读／明确采纳version、未知写显式读回；只写无读不附加隐式getStore。formId／identity／route／epoch隔离旧处理器，同身份续期保留合法输入但重读版本，纯主题保留请求，内存草稿无刷新持久保证。正式r2的Chromium156.0.8078.4、Chrome155.0.8059.84、Edge155.0.4283.62各21个必需浏览器组通过、零失败／跳过，包含独立空缓存构建、真实平台／两业务Rust／三库及共享会话／组织隔离，实际门禁分别见[Phase27验证记录](validation/phase-27.md)。Phase28镜像／公开v1v2路由与公共框架不在本阶段。

门店client默认每次HTTP有15秒绝对总期限，fetch、成功和错误body共用预算；read超时为固定安全408／REQUEST_TIMEOUT，已发送write超时为unknown、保留原输入后显式读回。超时不invalidate当前context或其他pair，不自动retry mutation。

门店403在响应头到达时锁定精确resource/action并取消同用途并行请求，不等待错误正文；相关旧视图与目录隐藏，同用途迟到200不能恢复，主题不解锁，其他用途不因此撤权。401完整invalidate当前context。原写入的inFlight由跨route／同身份续期恢复的form共享，原promise结束前禁止读回、确认或重提；未知员工unassign从原发送事件恢复核对入口，即使列表已无原行也不丢目标。空列表仅是当前授权下的读回，不充当历史删除回执。

两业务共享平台受控成员DTO `{id, displayName, organizationId}`，工单目录投影userId再提交assigneeUserId，门店保留id并分别提交managerUserId／userId；memberLookups分别仅ticket/create、ticket/update与store/create、store/update、store-staff/assign。成员见证从HTTP发起前计算最多3秒绝对预算，包含连接／锁等待及提交前复核，不因前端15秒期限延长。平台库与业务库没有共同事务，最后成员／grant检查至COMMIT仍有撤权或失效竞态，不承诺跨库原子或自动撤销历史关联。

## 1. 工程与锁文件

| 单元 | 实际工程入口与依赖边界 | 当前运行／构建方式 | 锁文件 |
| --- | --- | --- | --- |
| 仓库根 | [package.json](../package.json)，命令聚合器；没有根 `Cargo.toml`，也没有 npm workspace | Phase 1 增加检查，Phase 2 改为 npm/Vite 构建与 Node 内置 API 前台调度；默认仅静态前端 | 新增根 `package-lock.json`，删除 `bun.lock`；各前端仍必须在本工程独立 npm ci |
| 平台 API | [backend/Cargo.toml](../backend/Cargo.toml)，单 crate `saas-platform-backend`，含 binary 与 Phase30 窄策略 [lib.rs](../backend/src/lib.rs)；Axum/Tokio/SQLx/reqwest-rustls | `cargo run --manifest-path backend/Cargo.toml --locked`；入口 [main.rs](../backend/src/main.rs) 装配路由、配置与数据库，授权投影 SQL 与用例留在私有模块；公开策略集成测试进入 fast | `backend/Cargo.lock` |
| 宿主前端 | [frontend/package.json](../frontend/package.json)、[vite.config.ts](../frontend/vite.config.ts)；React 19/TypeScript/Vite | `npm --prefix frontend run dev`，3000；`build` 为 `tsc -b && vite build`；Phase 1 补齐 lint/typecheck/选定契约入口 | `frontend/package-lock.json` |
| 工单前端 | [package.json](../business-apps/work-orders/frontend/package.json)、[vite.config.mjs](../business-apps/work-orders/frontend/vite.config.mjs)、标准index／main及兼容参数build wrapper | Vue3.5.43／Vite8.3.4独立模块工程，`npm ci`及 `npm run build -- --version <新版本> --allowed-host-origin <精确HTTPS origin>`；相对ESM／动态chunk／CSS、登记classic bridge，公开config／asset-manifest／build-info，无secret或root node_modules依赖 | `business-apps/work-orders/frontend/package-lock.json` |
| 工单 Rust API | [business-apps/work-orders/backend/Cargo.toml](../business-apps/work-orders/backend/Cargo.toml)、薄 [main.rs](../business-apps/work-orders/backend/src/main.rs)、窄 [lib.rs](../business-apps/work-orders/backend/src/lib.rs)；独立 Axum/Tokio/SQLx crate | `cargo run --manifest-path business-apps/work-orders/backend/Cargo.toml --locked`，需业务库和完整私有平台配置；默认3004；双历史接管、live/ready、逐请求授权、读写／CSV、创建幂等、成员分配及0003提醒意图／官方通知发布；未切流，无Node运行依赖 | `business-apps/work-orders/backend/Cargo.lock` |
| 工单兼容业务 API | 保留的 [package.json](../business-apps/work-orders/backend/package.json)、[index.mjs](../business-apps/work-orders/backend/index.mjs)、[server.mjs](../business-apps/work-orders/backend/server.mjs) | Node 24/`pg`，`npm start`；当前完整 `/work-orders-api` 业务实现；使用自己的 PostgreSQL 库和账号，尚未切换至 Rust | `business-apps/work-orders/backend/package-lock.json` |
| 门店前端 | [package.json](../business-apps/stores/frontend/package.json)、[vite.config.mjs](../business-apps/stores/frontend/vite.config.mjs)、标准index／main／lazy lifecycle；React19.3.0／TS5.9.3／Vite8.3.4 | 独立`npm ci`／dev／build／preview／test；dev与preview默认127.0.0.1:3014，根调度3009；build要求精确allowedHostOrigins，可传version／api-base／受限out-dir；根`build:stores:frontend`转发，`build:stores`仍是Rust | `business-apps/stores/frontend/package-lock.json` |
| 门店 Rust API | [business-apps/stores/backend/Cargo.toml](../business-apps/stores/backend/Cargo.toml)、薄main／窄lib；独立Axum/Tokio/SQLx，业务模块私有 | `cargo run --locked --manifest-path business-apps/stores/backend/Cargo.toml`；显式stores专属DSN及完整私有平台配置，不自动读.env，默认3031；门店／员工关联／日报／SQL汇总及prefixed live／ready | `business-apps/stores/backend/Cargo.lock` |
| 通知实时服务 | [Cargo.toml](../mini-services/notification-ws/Cargo.toml)、薄 [main.rs](../mini-services/notification-ws/src/main.rs)、窄 [lib.rs](../mini-services/notification-ws/src/lib.rs) | 独立 Rust/Axum/Socket.IO，SQLx 只读/LISTEN；`cargo run --locked --manifest-path mini-services/notification-ws/Cargo.toml`，3003、`/socket.io/`；npm仅测试driver | `mini-services/notification-ws/Cargo.lock`；测试工具独立 npm lock |
| OAuth 演练 | [组件说明](../mini-services/oauth-playground/README.md)、[frontend/package.json](../mini-services/oauth-playground/frontend/package.json)、[backend/Cargo.toml](../mini-services/oauth-playground/backend/Cargo.toml) | 独立 Vite 页面 3010、Rust/Axum 流程 API 3011；原 Node index.ts/pages.ts 已退役，root package 仅转发构建/开发命令。`--with-oauth` 启动独立 Cargo + Vite，HTTP health readiness；服务端私有 OAUTH_* 配置，历史 password 默认关闭 | `frontend/package-lock.json`、`backend/Cargo.lock`；组件根 lock 仅转发壳，无运行依赖 |
| Vue 静态微应用 | [mini-services/micro-vue/package.json](../mini-services/micro-vue/package.json)、根 `index.html`、`vite.config.mjs`、`public/` | Vite 8.3.4 dev/build/preview，3005；相对资源、classic生命周期；Bun index.ts已删除，vendor/app.js字节保持 | `mini-services/micro-vue/package-lock.json` |
| jQuery 静态微应用 | [mini-services/micro-jquery/package.json](../mini-services/micro-jquery/package.json)、根 `index.html`、`vite.config.mjs`、`public/` | Vite 8.3.4 dev/build/preview，3006；相对资源、classic生命周期；Bun index.ts已删除 | `mini-services/micro-jquery/package-lock.json` |
| Socket.IO 聊天示例 | [examples/websocket/README.md](../examples/websocket/README.md)、[Rust 参考后台](../examples/websocket/backend/) | 独立 React/TypeScript/Vite8.3.4 前端3007、默认离线、显式endpoint；独立Rust后台默认回环3008/path `/`，保持join/message事件，不接入平台通知身份 | 前端 npm lock 保持；后台独立 Cargo.lock 固定 socketioxide0.18.7/engineioxide0.17.7；旧Node server.ts已删除 |
| 历史后台授权兼容测试 | [退役说明](../examples/app-backend/README.md)、[authorize.mjs](../examples/app-backend/authorize.mjs)、[authorize.test.mjs](../examples/app-backend/authorize.test.mjs) | Phase17已退役的JS helper，仅保留既有fixture行为；新后台接入参考 [Rust 工单授权](../business-apps/work-orders/backend/src/authorization.rs)，不提供公共后台SDK、监听器或部署单元 | 无独立锁文件；Node内建测试仅用于历史兼容 |

工具版本从根 `.nvmrc`、`.node-version`、`rust-toolchain.toml` 与 `frontend/package.json`、`.npmrc` 读取；前端依赖使用 `npm ci` 和已提交锁文件。各 Cargo 工程均为独立 crate，仓库没有根 Cargo workspace；应分别使用对应 manifest 和锁文件，不以平台测试代替工单等组件验收。

`frontend/public/sub-apps/{vue,jquery}` 是构建后的兼容静态副本；源码为 `mini-services/micro-*` 的根index与public。`build:all`先构建示例，再调用[sync-microapp.mjs](../scripts/sync-microapp.mjs)同步各自dist，最后构建宿主，防止宿主带旧副本。开发调度器不复制源码；版本发布用独立dist和已有打包器。工单保持自身工程，不属于这两个内置副本。

## 2. 已完成迁移、遗留入口与退出阶段

| 现存入口／依赖 | 当前事实与限制 | 台账退出阶段（待实施） |
| --- | --- | --- |
| 根 `dev`、`build` 与锁文件 | 已迁移：`dev.mjs`调度本工程Vite，build代理npm；根npm锁替代Bun锁，旧Node后台依赖未伪称已Rust化 | Phase 2完成；Phase 34再次扫描 |
| [scripts/dev.mjs](../scripts/dev.mjs)、[dev-child.mjs](../scripts/dev-child.mjs)与两个shell wrapper | 已迁移：跨平台前台supervisor、固定端口、总readiness预算、父进程退出/失败回收自己拥有的树；占用者保留。默认host/vue/jquery，websocket可选；不自动安装、启动PG或后台 | Phase 2完成；Linux路径尚未真实执行。显式--with-*会访问真实服务，Rust服务替换仍跟随后续阶段 |
| 原 `micro-vue/index.ts`、`micro-jquery/index.ts` | 两个Bun静态服务已删除；独立Vite工程保留真实classic生命周期。另删除无调用方的 `tool-results/inspect18.ts` 旧Bun.SQL本机探针，未运行其数据库查询 | Phase 2完成；源码可从Git历史恢复 |
| 原根 Caddyfile | Phase 3 已删除，旧动态任意端口开发网关退出；Nginx 只代理明确白名单上游 | 不提供 Caddy 兼容选项 |
| [frontend/Dockerfile](../frontend/Dockerfile)、[docker-compose.yml](../docker-compose.yml) | Node24 仅锁定构建；非 root/只读 Nginx 宿主与独立 gateway；默认镜像无共享产物卷；ZIP 单列 override | Phase 3 实测分类见证据；Phase 28 联合验收业务版本路由 |
| 工单 Rust API镜像、[backend/Dockerfile](../business-apps/work-orders/backend/Dockerfile)与保留Node兼容入口 | Phase28当前Dockerfile编译独立Rust crate；[business override](../deploy/compose.business.yml)显式使用Rust镜像。旧Node源码保留兼容，legacy override接收部署方已提供的独立旧镜像，不使用当前Rust Dockerfile重建Node | Phase16–21历史证据保持；Phase28当前源码提供独立镜像路线，生产切流不由模板或隔离运行推断 |
| 工单／门店静态版本与门店Rust镜像 | [deploy/microapps](../deploy/microapps/README.md)、两业务各自API Dockerfile及business Compose；四静态镜像使用各自前端锁文件源码构建，静态根目录自带dist，无平台产物卷 | 受管business-delivery实际模式为冻结Linux源码按Cargo.lock编译两API，校验ELF／SHA／架构／来源后runtime装箱，报告为verified-prebuilt-ELF；默认API Dockerfile builder另有实现，不据此宣称已运行 |
| 原工单esbuild IIFE构建路线 | Phase21由标准Vue模块／Vite入口与独立lock替换；保留build wrapper原公开CLI参数，移除全局Vue依赖，classic SDK bridge仍按登记字节输出 | Phase21当前实现和实际验收分别记录，前端版本必须新建，不能覆盖旧chunk或据实现宣称切流 |
| 通知 Rust 后台、[Dockerfile](../mini-services/notification-ws/Dockerfile)、[独立 Compose](../deploy/compose.notification.yml) | 已移除 Node index/config/service/store，非root只读 Rust 镜像；保持 Socket.IO 与 notification1.0；真实平台复验、有界调度/连接及只读12列 | Phase 23 独立证据见[验证记录](validation/phase-23.md)；未操作外部registry或生产部署，Phase34复核 |
| `examples/websocket/backend` | Rust 聊天参考已替换旧Node server.ts，单独Cargo.lock、薄main/窄lib、来源/JSON对象/输入限额和连接身份校验；无平台/数据库依赖 | Phase 22当前产品客户端实证见[协议选型](realtime-protocol.md)及[证据](validation/phase-22.md)；Phase 34核查整体运行栈 |
| 原 OAuth Node HTTP 与内嵌页面 | 已删除；独立 Vite/Rust 工程和各自锁文件。秘密/PKCE verifier/token 留 Rust 内存，流程会话绑定且有界 TTL/一次性消费；非 root 静态和 API 镜像、同站 HTTPS 入口、私网 Compose 模板 | Phase 24 实施；真实平台流程、安全拒绝、独立构建及 Linux 镜像结果见[验证记录](validation/phase-24.md)，Phase 34 再扫描有效运行入口 |
| `examples/app-backend/authorize.mjs` | Phase17明确退役，原函数仅作兼容测试，不再推荐新后台接入使用；规范参考为实际Rust工单授权 | Phase17替换活动推荐；Phase34复核活动示例与推荐，防止旧参考复活 |
| [tools/platform-admin](../tools/platform-admin/README.md) | Phase 25 独立 Rust 管理 CLI，`saas-platform-admin` 使用正式管理 API；旧 Node 管理入口与 SQL 草案输出已退役 | 默认离线 plan；写操作显式 `--apply`，保留幂等、冲突拒绝与 secret 保护，不执行 SQL 或控制 Docker |
| CI 中 `scripts/ci/run.mjs` | fast／postgres／browser／delivery分层保留，Phase28新增business-delivery及独立protected publication工作流；固定安装、必测manifest、脱敏日志与实际工作树摘要，原生构建保留独立入口 | 候选、真实registry推拉、目标health与版本路由、平台UI／授权分别报告；远端Actions与生产采用不由本地源码或运行推断。监控／灾难恢复仍由Phase32／33完成 |
| 同机 ZIP 产物与暂存维护 | Phase 5 实现 `release_archives` 的规范清单、同文件系统无覆盖安装，0002 追加跨环境产物绑定；`--micro-staging` 在配置/DB/HTTP 前分派、默认 SCAN、持维护锁且只清理归属可验证的暂存 | 实际验收见 Phase 5 记录；不提供正式目录回收、跨机分发或灾难恢复。升级须停止并排空旧 writer，禁止与新版共写同卷 |

允许保留的 Node 用途是前端 Vite/打包/lint、SDK JavaScript、前端与浏览器测试驱动。`scripts/package-microapp.mjs` 和 `sync-microapp.mjs` 属于前端产物辅助工具；这不豁免 Node 业务后台或 Node 管理 CLI 的台账迁移项。

## 3. 部署源码与尚存入口风险

Phase 1 发现根 deploy 缺失。Phase 3 已重建 `deploy/gateway`、`deploy/static`、`deploy/native` 与 Compose 环境/ZIP/legacy 模板；宿主 Dockerfile 和网关测试不再引用 Caddy。配置渲染只证明输入/策略，实际镜像/HTTPS结果单独记录。完整原生生产/恢复 E2E 仍涉及未来监控、恢复工具与数据库环境，不能凭 render 导入修复宣称通过。

Phase 2已修复 `frontend/vite.config.ts` 的默认版本目录问题：仅收到版本请求时才realpath，目录缺失不影响宿主启动；缺版本/资源返回404。`/sub-apps/<code>/...`兼容静态路径也不再被宿主SPA HTML吞404。设置 `MICRO_APPS_DEV_DIR` 仍须为明确绝对目录，真实版本读取保留realpath限界和GET/HEAD约束，没有伪造根deploy源码。

现有 `.github/workflows/production-build.yml` 使用 Ubuntu 24.04 和固定 Node 24.18.0/npm 11.16.0/Rust 1.97.0。PR 必需 fast → postgres 与三浏览器矩阵；main/手动候选追加delivery及business-delivery，聚合任务拒绝失败、取消或跳过的必需层。fork PR 不读取生产秘密，checkout不保留凭据；候选不登录外部registry或部署实际目标，业务门禁可在自有隔离registry验证真实推拉。可复现安装与实际manifest见[CI使用说明](../scripts/ci/README.md)，本机干净容器实测不代表GitHub托管runner已执行。

Phase28的[业务镜像驱动](../scripts/ci/business-images.mjs)与[交付驱动](../scripts/ci/business-delivery.mjs)把四个版本静态镜像、两独立Rust API及实际输入／archive／镜像身份分开记录。当前受管路径在冻结Linux源码中分别执行locked Cargo，验证预构建ELF后装箱，报告apiBuildMode=verified-prebuilt-ELF；四静态镜像执行源码Docker构建。此路径不证明默认API Dockerfile的locked-Docker-source-build模式已执行。registry distribution manifest digest、Docker config／image ID和archive SHA256是不同身份，须分别核对。隔离演练允许自有loopback registry；同daemon先删除本次候选引用再固定digest拉取仍可能复用基础层，不能写成第二台机器或全空daemon。

[business-publish.yml](../.github/workflows/business-publish.yml)是独立受保护的手工推送路径，核对同仓库main准确run／attempt、成功候选、来源和archive后推送同一产物，不部署Linux目标。部署方提供真实registry、保护环境和凭据；文件存在不表示远端已触发。Linux目标用Shell／Compose拉固定digest并检查health；所有环境共享权威持久inventory，先Rust CLI绑定／核验再推进版本路由和平台登记。inventory不是临时runner目录、Actions保留artifact或cache，单机文件锁不提供多宿主私有副本的分布式一致性。步骤和变量见[业务交付说明](../deploy/microapps/README.md)。

`scripts/ci/linux-runner.mjs` 将白名单源码复制到全新 Linux 容器，不挂宿主源码、个人 home、依赖缓存或 Docker socket。PG 使用独占网络、随机临时凭据和 tmpfs，无宿主端口；Phase 4 实际执行四个明确套件 21 项，Phase 5 当时增加 ZIP 登记 6 项，共五套件 27 项。Phase 11 加入普通 OAuth 10 项与平台 `psr_` 刷新 4 项，形成七套件 41 项；Phase 12 再加入登录保护后为八套件 46 项。Phase 13 声明新增管理审计八项及既有通知/成员十项，规划十套件 64 项；精确选择以 `scripts/ci/manifest.mjs` 为准，实际数量、失败及通过结果见各阶段 runner 报告。浏览器驱动由 `tests/browser/package-lock.json` 固定为 Playwright 1.64.1，下载目录仅项目 `.browsers` 或受控工具镜像层；真实 Chrome/Edge channel 不回退为 Chromium。缺环境、零测试、必需 skip、缺报告都阻止门禁。交付记录 Linux 架构、HEAD、buildId、实际源码摘要、OCI manifest 和归档摘要，实际探测平台 ELF/UID/动态库并运行真实 HTTPS smoke；Phase 5 另声明真实 ZIP 联合门禁，本地 OCI digest 不能证明远端已发布或可拉取。

Phase 6 新增唯一SDK登记与生成入口：[release.json](../examples/micro-app-sdk/release.json)登记候选1.0.0，`npm run sdk:sync`统一生成classic/ESM/TS/schema/Rust与allowlist vendor，`npm run sdk:check`在fast/browser/native构建前检测漂移。平台context边界真实构造公开Rust DTO，宿主与Vue/jQuery/工单消费同版。旧versionless iframe2.0真实字节保留为兼容基线；请求强制omit、有限流读取、结构错误与安全requestId，缺API能力不降级隔离。参见[SDK说明](../examples/micro-app-sdk/README.md)及[Phase6实际验证](validation/phase-06.md)。

## 4. 当前公开协议与兼容边界

| 契约 | 当前来源与行为 | 后续必须保持／核验的边界 |
| --- | --- | --- |
| 门店前端／成员／恢复 | [前端README](../business-apps/stores/frontend/README.md)、[API](../business-apps/stores/backend/API.md)、[双消费者差异](stores-member-contract.md) | 八个精确页面MENU与三个BUTTON、九能力、三成员用途，根不授权数据；staff／report独立store/read。Member.id映射为员工userId；SELF按各自owner与hard ceiling。无receipt／自动mutation重试；原组织／门店／员工／日期／version显式读回，缺read则保留输入并提示。草稿仅存活iframe内存，双页共享宿主会话 |
| 宿主生命周期、MicroProps 2.0 | [frontend/src/components/micro/types.ts](../frontend/src/components/micro/types.ts)、[runtime.ts](../frontend/src/components/micro/runtime.ts)；`__MICRO_APP__` 暴露 mount/unmount/update；props 带 token、授权上下文、`contextVersion`、`contextEpoch`、到期时间及 AbortSignal | 身份/租户/组织切换、失权、过期和卸载取消旧任务；迟到响应不可写入新上下文。展示更新优化仍属后续阶段 |
| opaque iframe SDK | [examples/micro-app-sdk/iframe-bridge.js](../examples/micro-app-sdk/iframe-bridge.js)、[frontend/src/components/micro/iframe.ts](../frontend/src/components/micro/iframe.ts)；精确宿主 origin/MessageChannel，`MICRO_CONNECT/READY/MOUNT/UPDATE/ACK/ERROR/UNMOUNT` 以及导航、Toast、下载消息 | iframe 不读取宿主共享存储；短期应用 token 由宿主注入；下载桥只在功能权限和数据 grant 共同允许时工作。工单 `vendor/iframe-bridge.js` 是需同步核对的副本 |
| 上下文请求取消 | [examples/micro-app-sdk/context-client.mjs](../examples/micro-app-sdk/context-client.mjs)、工单 [src/client.mjs](../business-apps/work-orders/frontend/src/client.mjs) | 请求必须绑定当前 signal/epoch；响应后重新检查当前上下文，不依赖 fetch 实现一定及时取消 |
| 平台应用授权 HTTP 1.0 | [backend/src/routes/app_context.rs](../backend/src/routes/app_context.rs)、[oauth2.rs](../backend/src/routes/oauth2.rs)、[Rust 工单 authorization.rs](../business-apps/work-orders/backend/src/authorization.rs)；`POST /api/oauth/session-token`签发应用Bearer；业务后台以机密客户端Basic调内省，再以应用Bearer取context | 逐请求核对active/aud/用户/租户/组织与context1.0；permission及resource/action grant分别判断；组织ceiling与参数化SQL共同生效，不信浏览器身份、不用平台session代替应用token |
| 工单 Rust HTTP／数据库 | [src/http.rs](../business-apps/work-orders/backend/src/http.rs)、[authorization.rs](../business-apps/work-orders/backend/src/authorization.rs)、[reads.rs](../business-apps/work-orders/backend/src/reads.rs)、[migrations.rs](../business-apps/work-orders/backend/src/migrations.rs) | 授权两次HTTP共用默认3000ms总预算、每份JSON256 KiB、进程并发默认32且饱和503；读取／CSV共用scope及必要的一致快照。0002保存创建receipt并补齐Node／SQLx双历史，0001不改；live/ready含必需receipt对象，仍不代表每个调用方授权或完整业务可用 |
| 工单 Rust 写入与恢复 | [writes.rs](../business-apps/work-orders/backend/src/writes.rs)、[writes/dto.rs](../business-apps/work-orders/backend/src/writes/dto.rs)、[0002](../business-apps/work-orders/backend/migrations/0002_create_idempotency.sql) | POST必需1–128个ASCII `[A-Za-z0-9._:-]` Idempotency-Key；规范化固定DTO SHA-256（非空assignee UUID也规范化）与tenant/org/user/create/key receipt、工单、原201同事务。同键同正文重放原历史，异正文409；已commit重放复验当前create授权，不重查历史member。PATCH／DELETE范围内行锁和版本竞争。64KiB超限413 REQUEST_LIMIT，commit前context到期401／成员结果到期503 MEMBER_VALIDATION_EXPIRED并回滚；COMMIT错误／提交后context或成员到期／15秒写截止503 CREATE_RESULT_UNKNOWN或WRITE_RESULT_UNKNOWN，分别同键对账或读回版本，不自动retry |
| 成员目录与分配 | [directory.rs](../business-apps/work-orders/backend/src/directory.rs)、[directory/query.rs](../business-apps/work-orders/backend/src/directory/query.rs)、[平台成员入口](../backend/src/routes/app_members.rs)及 [memberLookups声明](data-grant-management.md#member-lookups) | GET /assignees保留必需action=create/update，organizationId默认当前组织，原始q先限100个Unicode字符再trim，page默认1／最大1000000、size默认20／最大50；返回records／total／page／size。同应用memberLookups默认空，最多128个唯一已登记pair，公开capabilities三字段不变；普通appBearer使用原permission／grant和组织硬上限，不新增成员grant。更新省略／同UUID保留快照，真正change／clear复核目录组织scope，非空change先成员HTTP；非锁预读释放连接后调用，行锁内核对组织／version／oldAssignee。成员期限从HTTP发起时按单调时钟计算min(配置超时,3秒)，不续期；HTTP超时503 PLATFORM_UNAVAILABLE，与提交前结果过期区分。SELF仍按owner；跨库撤权／失效竞态未消除 |
| 工单兼容业务 HTTP | 保留的 [server.mjs](../business-apps/work-orders/backend/server.mjs)、[auth.mjs](../business-apps/work-orders/backend/auth.mjs) | Node保留旧业务作兼容输入；Rust已提供授权、读取／CSV、写入、成员及持久提醒。Phase21前端消费Rust创建键、合法成功DTO与显式恢复；两种后台契约仍有差异，不直接切流。业务库独立，opaque Origin:null保持精确CORS。0003升级后原Phase19及更早严格基线不能直接回退，保留receipt／intent／publication并先对账 |
| OAuth | [backend/src/routes/oauth2.rs](../backend/src/routes/oauth2.rs)、[oauth_issuance.rs](../backend/src/oauth_issuance.rs)、`device.rs` 和[独立 Vite/Rust 演练](../mini-services/oauth-playground/README.md)；授权码/PKCE、device-code、refresh、client_credentials、显式开启的历史 password 演练；普通一次性消费、token 对及签发审计同事务，新行摘要存储与随机 jti | [普通 OAuth 契约](oauth-issuance.md)定义用户/直接成员/租户/组织祖先/客户端的实时有效性、机器例外、scope 收窄及旧 token 兼容；平台 `psr_` 与 app 协议独立。演练 Rust 私有配置与浏览器展示分离，固定 callback、会话绑定 state/verifier、明确错误；不支持能力拒绝，不做生产凭据来源 |
| 关键管理审计 | [audit.rs](../backend/src/util/audit.rs) 的事务接口及各管理路由，保留 action 与 requestId，显式最小前后摘要、删除前名称快照、规范集合摘要和写入失败观测 | 审计失败与主写一起回滚；同值/同集合重试不额外制造变更，不能把 requestId 当幂等键或把未知提交当回滚。用户跨租户作用按目标租户记录；预检、部署及诊断分类见[管理审计契约](management-audit.md) |
| 通知 Socket.IO 4／1.0 DTO | [notification-ws/README.md](../mini-services/notification-ws/README.md)、[frontend/src/lib/notification-protocol.ts](../frontend/src/lib/notification-protocol.ts)；`/socket.io/` 同时支持 polling/Upgrade；`authenticate {token}` → `authenticated`，投递 `notification`，失效 `unauthorized` | 平台 session 由 `GET /api/notifications/socket-context` 实时验证；房间绑定 tenant/org/user，逐通知复验当前权限；客户端不得指定身份；平台 DB inbox 为权威，重连补拉不冒充实时重放 |
| 通知持久化与 LISTEN | [backend/src/routes/notifications.rs](../backend/src/routes/notifications.rs)、[notification-ws/src/store.rs](../mini-services/notification-ws/src/store.rs)、[examples/micro-app-sdk/notification-publisher.mjs](../examples/micro-app-sdk/notification-publisher.mjs) | 持久发布/幂等与实时提示分离；`new_notification` 提示须核对通知 ID 与完整归属后查询行；APP 权限变化即时复验，LISTEN 断线需恢复，实时队列不能改变持久发布语义 |
| 独立静态版本发布 | [scripts/package-microapp.mjs](../scripts/package-microapp.mjs)、[backend/src/routes/application_releases.rs](../backend/src/routes/application_releases.rs)、[release_archives.rs](../backend/src/release_archives.rs)、[releases.rs](../backend/src/releases.rs) | ZIP 同文件系统无覆盖安装，规范清单/摘要与 0002 跨环境产物绑定；同环境重试稳定 ID，跨环境共享产物、各有 release ID；未知提交保留正式字节，以 storage 四态和所有环境引用对账。preflight/activate/rollback 验证绑定字节后检查 HTTP；镜像/外部 URL 的联合业务交付仍归 Phase 28 |
| 外部URL登记与镜像来源 | 发布管理UI、`applicationsApi.registerRelease`、Rust CLI的artifact-bind／verify／release-reconcile | RegisterBody仍仅六个旧字段，HTML固定；URL登记为DRAFT，独立preflight即使HTTP200也只认PASSED，activate再次预检。image／digest／source不存入平台登记；CLI持久inventory约束同app/version产物、同环境完整定义与同公开版本URL，跨环境允许不同origin但须保持同一产物。Docker health和普通用户授权分别核验 |

`examples/websocket` 的 `join/message/test` 聊天事件和根 path 不属于通知授权契约；Rust参考使用精确Origin列表。Phase 22分别用真实宿主和聊天客户端验证各自协议，冻结SOCKET_IO_COMPATIBLE；其loopback通知fixture只有合成token与内存房间；Phase23另用真实平台/PG、Rust通知与实际宿主完成独立联合验收，见[Phase23](validation/phase-23.md)。Windows Rust入站回环受阻记为环境未完成，真实矩阵改在隔离Linux运行；环境缺失不能作为触发37的证据。

## 5. 受管验证清单与证据分类

统一记录 `PASS`（实际执行成功）、`FAIL`（实际执行失败）、`SKIP`（测试自身跳过）、`NOT_RUN`（本次未执行）和 `UNAVAILABLE`（工具／前置资源不可用）。总退出码为零也必须单列 skip；历史通过记录不得填入本阶段结果。

| 层级 | 真实入口 | 实际证明范围与环境边界 |
| --- | --- | --- |
| 前端静态门禁 | 根 `npm run lint`、`npm run typecheck`；依赖由 `npm --prefix frontend ci` 安装 | lint 与 TypeScript 工程检查，不连接 DB，不证明生产构建/浏览器/部署。必须在干净依赖安装后执行 |
| Vite构建/交付与开发进程 | `npm run test:vite`、`npm run test:dev`，各示例 `npm test` | 真实独立Vite dev/build/preview、相对版本资源/404、哨兵配置不进入产物、readiness/占用者保护/父子进程清理。仅自有临时目录和fixture；不证明生产Nginx或后台迁移 |
| 选定无 DB 契约 | 根 `npm run test:contracts` 保留原八文件入口；受管 CI 的 `scripts/ci/manifest.mjs` 另明确包含 `tests/package-microapp.mjs`，Phase5九文件99项为历史；Phase6再明确加入纯Node协议、SDK生成/客户端与工单消费者/CORS，实际数量见Phase6报告 | Node 内存、VM、受控时钟及 fixture；部分需 frontend TypeScript。根旧入口与 CI 扩展清单分别记录，必测组须零失败/跳过，实际结果见阶段证据 |
| 旧微应用扩展契约 | `npm --prefix frontend run test:micro` 中另含 `tests/micro-iframe-protocol.mjs`、`tests/package-microapp.mjs`；后者现在也纳入受管 CI | Phase6将真实浏览器分支拆为零skip独立`tests/micro-iframe-browser.mjs`，原文件仅纯Node协议并纳入必需CI；历史 Windows symlink 条件跳过仍保留在旧证据。当前 CI 打包组的安全路径/产物检查须按其实际清单和结果单列，不能以旧扩展组替代必测证据 |
| 通知无 DB 单元与边界 | `cargo test --locked --manifest-path mini-services/notification-ws/Cargo.toml --all-targets --all-features` | 配置/协议、调度公平与容量、TCP撤销/所有权；不代替真实平台或PG。Rust fmt/check/Clippy/doc进入fast |
| 工单无 DB 契约 | `business-apps/work-orders/backend/tests/{config,observability}.test.mjs`；`frontend/tests/{app,client,build}.mjs` | 兼容后台配置／HTTP fixture；前端VM验证草稿／事件／取消与恢复，Vite独立构建、产物元数据、秘密哨兵和版本路径静态HTTP分别验证。VM／构建HTTP不是真实opaque浏览器或平台联合，旧Node管理工具已退役 |
| 工单 Vue／Vite真实浏览器 | `tests/phase21-work-orders-browser.mjs`、`tests/helpers/phase21-work-orders-services.mjs`；原Phase8轻量竞态保持独立 | 实际平台／独立Rust业务API、两座自有PG、生产宿主与Nginx／TLS，验证opaque module／动态chunk、Origin:null／无Cookie、普通用户MENU／CRUD／成员／CSV／持久提醒及未知结果恢复。资源归属、实际数量与PASS／FAIL／skip见 [Phase21](validation/phase-21.md)，不预记通过，不以旧Node或VM替代 |
| 门店独立构建／单元 | `npm --prefix business-apps/stores/frontend ci`、`run typecheck`、`test`及`run build`，具体公开参数见[README](../business-apps/stores/frontend/README.md) | 独立lock与依赖、typecheck、form／client／lazy／build边界；干净复制构建须隔离根node_modules和个人缓存，产物校验无凭据，不能替代真实opaque浏览器 |
| 门店与工单双消费者真实浏览器 | `tests/phase27-stores-browser.mjs`与`tests/helpers/phase27-stores-services.mjs`；受管CI独立required `stores-browser-live`，typed format `stores-browser` | 当前生产平台／两业务Rust／三库、生产宿主／Nginx／正常TLS；普通用户资料／员工／日报／汇总、opaque静态ESM和Origin:null、双页共享会话、同一受控成员协议／跨app拒绝。正式r2三产品各21组PASS、0失败／跳过；每产品另直接执行CLI TLS的9项具名检查，不与浏览器组相加。版本、原始失败、范围和清理见[Phase27证据](validation/phase-27.md)，不继承21或26的PASS |
| 工单／门店镜像与URL联合交付 | `scripts/ci/business-delivery.mjs`、`tests/phase28-delivery-browser.mjs`及专属helper；受管business-delivery入口 | 声明实际隔离registry push／消除候选引用后固定digest pull、六镜像Compose／health、持久inventory冲突拒绝、真实平台URL UI的DRAFT／200FAILED／激活重验、v1同域相对API／v2外部origin绝对API、两版本chunk／回滚、普通用户SDK／Rust授权及三库隔离。实现与typed检查清单不是PASS，当前状态和实际执行另见[Phase28台账](architecture-optimization-ledger.md#phase-28)，不继承Phase27结果 |
| Rust 平台管理 CLI | `tools/platform-admin/Cargo.toml` 的单元与集成测试、严格 schema/manifest；具体命令见 [工具说明](../tools/platform-admin/README.md) | 离线计划、受控 HTTP fixture、冲突拒绝、授权 API 与秘密脱敏分别验证；不把 fixture 当作真实平台或生产部署证据 |
| 工单 Rust 基线验证 | 独立 Cargo manifest 的 lib/doc/Clippy 等门禁；真实 PG 用例 `business-apps/work-orders/backend/tests/migrations.rs` | 单元与真实 PostgreSQL 接管、并发、拒绝及无 Node 二进制运行分别记录。PG 用例默认 ignored，须由证明资源归属的受管隔离驱动执行，默认跳过不计通过；实际命令、结果及限制见 [Phase 16](validation/phase-16.md) |
| 工单 Rust 授权／只读验证 | `src/authorization/tests.rs`、`reads/{query,csv}.rs`及HTTP单元；真实PG／HTTP用例 `tests/read_contracts.rs` | 无DB边界与真实业务SQL／受控平台HTTP fixture分别报告；PG用例默认ignored，要求自有隔离库、显式owner及清理。代表查询计划与实际CI结果见 [Phase17](validation/phase-17.md)最终报告，不沿用Phase16或Node联合测试通过结论 |
| 工单 Rust 写入验证 | `src/writes/dto.rs`及HTTP单元、`tests/write_contracts.rs`与迁移用例 | DTO／创建键、receipt并发／回滚／恢复、版本竞争、0002双历史与有界资源需按实际套件验证；PG用例默认ignored，必须显式执行并证明归属／清理。最终数量、真实通过／失败及限制见 [Phase18](validation/phase-18.md)，不借用Phase17结果，不把测试声明记为PASS |
| 工单 Rust 成员验证 | `src/directory{,/query}.rs`、`tests/member_contracts.rs`与平台 `backend/src/phase19_work_orders_tests.rs`；精确受管选择见 [manifest](../scripts/ci/manifest.mjs) | 目录契约、有效直接成员、scope／组织硬上限、分配预读与锁内复核、单调TTL和跨库竞态分别验证；真实业务PG／受控HTTP与真实平台联合证据分列。ignored用例须由自有隔离资源驱动显式执行并清理，本轮实际数量、通过／失败及限制见 [Phase19](validation/phase-19.md)，不预记PASS |
| 工单 Rust 提醒验证 | `src/reminders{,/dto,/store}.rs`、`tests/reminder_contracts.rs`与平台 `backend/src/phase20_work_orders_tests.rs`；精确受管选择见 [manifest](../scripts/ci/manifest.mjs) | 0003意图、同事件并发／冲突、read／publish独立授权、两种unknown、HTTP回执投影和连接释放分别验证；受控HTTP的发布记录不能代替真实平台publication／接收者session收件箱。实时服务未启动仍须区分持久成功，实际选择、数量、结果与限制见 [Phase20](validation/phase-20.md)，不预记PASS |
| Rust 默认测试 | 根 `npm run test:rust`，即当前 bin 的 `cargo test --locked` | 默认单元测试与不依赖 DB 的检查；`#[ignore]` 必须单列。多数 ignored 项需要隔离 PostgreSQL，`release_tests` 还有独立标记的本机 IPv6 HTTP fixture；不能统称所有 ignored 都是 DB，也不能为追求零 ignored 直接运行 `--include-ignored` |
| Rust 归档安装与暂存维护 | fast 显式选择 `release_archives::` 和 `release_archive_maintenance::tests`；清单见 `scripts/ci/run.mjs` | 无 DB 验证规范摘要、路径/容量限制、无覆盖安装、锁及暂存归属；实际平台和文件系统结果分别记录，不由 Windows 或旧源码结果代替最终 Linux 检查 |
| Rust 登录/密码/来源单元 | fast 显式选择 `login_limits::tests`、`util::password::tests`、`client_ip::tests` 与 `config::tests` | Phase 12 声明分别 8/8/6/4 项；验证容量/可控 TTL、许可持有/取消/超时、hash 兼容、输入上限、可信代理与默认配置。fast 当前共 27 步，实际结果见 Phase 12，不能替代真实 PG HTTP 压力 |
| Rust 隔离数据库 | `backend/src/*_tests.rs`；受管选择见 `scripts/ci/postgres.mjs` 和 `manifest.mjs` | Phase 4 实测四组 21 项；Phase 5 五组 27 项、Phase 11 七组 41 项分别保留历史证据，Phase 12 八组 46 项增加登录保护。Phase 13 声明 `management_audit_tests` 八项，并纳入既有通知/成员十项，规划十组 64 项，覆盖审计故障/回滚/重试及发布通知边界；精确选择见 manifest，实际数量与结果见 Phase 13 报告。显式 `--bin saas-platform-backend` 和 `--ignored --test-threads=1`，验证自有库移除和 pool 关闭；未列入的历史 ignored 不计 PASS |
| Node 真实 PostgreSQL | 通知 `tests/postgres.mjs`；工单后台 `tests/api.test.mjs` | 分别要求 `NOTIFICATION_TEST_DATABASE_URL`／`WORK_ORDERS_TEST_DATABASE_URL` 或 `TEST_DATABASE_URL`，账号可创建/删除自有库。仍以 HTTP 平台 fixture 验证服务；不是完整平台权限实现。工单 helper 还可能读根忽略的 `.env.test`，所以 Phase 1 不盲目执行所有组件 `npm test` |
| 通知真实平台联合 | `node scripts/ci/notification.mjs --run-tests --out-dir /absolute/new-report`；浏览器另加 `--browser chromium` | 必需owned PG，构建当前平台main/Rust通知/仅延迟真实请求的Rust proxy；运行platform-e2e/scheduler/boundaries/auth-races，缺配置/零测试/skip失败；双页host另验共享切组织/旧HTTP/重连补拉；镜像用notification-image.mjs独立验证 |
| 工单实际联合／浏览器 | `business-apps/work-orders/tests/platform-e2e.mjs` | 必需当前 backend binary、显式 `PLATFORM_ADMIN_BIN` Rust CLI、隔离 PG、NGINX_BIN、宿主构建、项目明确提供的 PLAYWRIGHT_MODULE/BROWSER_EXECUTABLE；Linux 同系统进程。真实平台+业务库+通知+Vue iframe+版本切换，创建并清理自有资源；与前端 VM 测试不同 |
| 宿主浏览器 fixture | 七组宿主 `tests/*-browser.mjs` 及独立聊天；精确清单 `tests/browser/manifest.mjs` | Phase 4 已修订陈旧管理/路由/branding/部门等待 fixture，并改用项目受管运行时。143 个必测检查组覆盖 8 套件；Linux 三产品完整运行各 143/143，最终部门等待修复另各补跑受影响 11 组及 helper 单元，两个源码快照和结果边界见 Phase 4 证据。实际 UI + 隔离 API/Socket.IO fixture，不代表真实平台业务授权或未来 Rust 通知协议已验收；不宣称最近两稳定版完整支持 |
| 网关传输 fixture | `tests/micro-release-gateways.mjs`，或 `npm --prefix frontend run test:micro:gateways` | Phase 3 改为真实 Linux Docker/Nginx + 独立 Rust transport fixture；正常验证 TLS，覆盖静态/代理/polling/Upgrade/DNS/持久化/旧资源；缺必需环境失败，不能静默跳过。不是完整 Socket.IO 或业务授权验收 |
| 原生配置/构建与旧完整生产恢复 | `tests/production-native-config.mjs`、`deploy/native/tests/build.test.mjs`、`production-native-e2e.mjs` | renderer 与干净构建已恢复；完整旧 E2E 仍需未来监控/恢复、生产产物、TLS/NSS/PG及真实联合环境。构建器受控编译测试与真实 release 编译分别记录 |
| Docker/Compose | 根 `docker-compose.yml`、ZIP/legacy overrides，平台/宿主/入口/ZIP/旧通知/旧工单 Dockerfile；另有独立 test-only Rust Dockerfile | Compose config 只证明解析；镜像 build、非 root/只读、HTTPS、DNS/历史资源/ZIP 持久化由 Phase 3 单列实测；不等于业务或完整灾难恢复通过。未读取私有部署配置 |
| ZIP 真实发布与恢复 | `tests/zip-release-e2e.mjs`；delivery 复用当前候选三镜像后执行，结果写入独立 `result.json` | Phase 5 声明 9 项必测：真实隔离 PG、同卷 RW/RO、受信任 HTTPS 下验证上传/未知响应对账/同内容重试/跨环境复用/冲突/激活回滚与旧资源保留，并核对宿主镜像和容器不变。实际通过数量、源码和清理以 Phase 5 最终报告为准 |
| SQL/已有环境探针 | `tests/tenant-{relationship-constraints,isolation-policy-fixture}.sql`、`tenant-isolation-api.mjs` | SQL 需要明确选择隔离 DB；API 探针需要显式的双租户 session 与 fixture ID，可能从 `.env.test` 读取配置。不是无 DB 默认检查，Phase 1 不执行 |
| 历史宿主打包测试 | `tests/{python-runtime-build,python-runtime-container,database-runtime-build}.sh` | 指向当前不存在的 `.zscripts`；出现旧 Python/Next 产物命名及 SQLite/Bun fixture，container 另需旧 runner 镜像。属于历史遗留，不能当作当前 Rust/PostgreSQL 构建验证；Phase 2/4 分类替换，Phase 34 清理活动入口 |

2026-10-14 的“前端 build 通过、六组 88 项/86 通过/2 跳过、lint 缺 ESLint、原生配置缺 render 失败”只保留为[台账背景](architecture-optimization-ledger.md)和[优化方案](architecture-optimization-plan.md)中的历史记录。各阶段实际结果使用各自的新报告；现网、灾难恢复及未列入 manifest 的测试不能借用历史数字宣称完成。既有平台严格 fmt/Clippy 债务仍见 Phase 1；历史 52 ignored 只是当时默认测试计数，Phase 4 显式选定 21 项、Phase 5 当时声明 27 项，当前数量以 manifest 为准，未选择项与未运行结果不计 PASS。

## 6. Windows／Linux 标准检查命令

在仓库根执行；以下无 DB 检查命令在 PowerShell 和 Bash 中一致。先启用仓库声明的 Node 24/npm 与 Rust 工具链，核对每条命令的退出码。PowerShell 若执行策略拦截 `npm.ps1`，使用同一安装中的 `npm.cmd`，不要切换到 PATH 中旧版 Node。

```text
node --version
npm --version
rustc --version
cargo --version
npm --prefix frontend ci
npm --prefix mini-services/micro-vue ci
npm --prefix mini-services/micro-jquery ci
npm --prefix examples/websocket ci
npm run lint
npm run typecheck
npm run test:contracts
npm run check:rust
npm run test:rust
npm run test:dev
npm run test:vite
```

`npm run check:rust`、`test:rust` 等始终指向 `backend/Cargo.toml`。需要直接核验时：

```text
cargo check --manifest-path backend/Cargo.toml --locked --all-targets
cargo test --manifest-path backend/Cargo.toml --locked --bin saas-platform-backend
```

按变更范围补充以下 Rust 门禁，并把既有格式化／warning 问题与本阶段引入问题分开记录，不为满足全量命令批量重排无关源码：

```text
npm run fmt:rust
npm run clippy:rust
npm run doc:rust
```

需要扩大无 DB 组件验证时，先安装各自锁定依赖，然后显式列文件，避免通配运行误触数据库测试。以下从根目录执行，均只启动自有 fixture 或临时构建：

```text
npm --prefix mini-services/notification-ws ci --ignore-scripts
cargo test --locked --manifest-path mini-services/notification-ws/Cargo.toml --all-targets --all-features
npm --prefix business-apps/work-orders/backend ci --ignore-scripts
node --test business-apps/work-orders/backend/tests/config.test.mjs business-apps/work-orders/backend/tests/observability.test.mjs
npm --prefix business-apps/work-orders/frontend ci
node --test business-apps/work-orders/frontend/tests/app.mjs business-apps/work-orders/frontend/tests/client.mjs business-apps/work-orders/frontend/tests/build.mjs
cargo test --manifest-path tools/platform-admin/Cargo.toml --locked
```

数据库、浏览器、网关和容器检查只按上一节的独立前置条件启用。Bash/Linux 下的环境赋值与 PowerShell 的 `$env:NAME` 语法分别使用，不把旧 `scripts/start-all.sh` 当作跨平台检查入口；不读取或打印 `.env`、连接串、token、客户端 secret 或生产参数来编造可运行示例。


## Phase 28 正式验收（2026-10-19）

工单／门店镜像与统一外部 URL 接入已完成本阶段隔离验收：同一 839 项源码输入下，完整 fast 63 步、1083 项通过，0 失败／跳过；真实六镜像 registry push／候选清除／固定 digest pull 和 Compose 联合检查 14/14 通过，Chromium／Chrome／Edge 各 9/9 通过。来源、实际命令、旧失败及五项原验收映射见[Phase 28 验证报告](validation/phase-28.md)。

平台仍只登记、预检与激活公开 URL；digest 不变性由 Rust CLI／CI 权威持久 inventory 负责。两 API 按 Cargo.lock 在隔离 Linux 编译、核验 ELF 后 runtime 装箱；默认 API Dockerfile 的源码 builder 未执行。同一 daemon 的 pull 前确认本轮候选镜像身份已清除，基础层缓存可能仍在；本结果不声称跨机器空缓存。远端 Actions、外部 registry 发行与生产部署未执行。前端回退须保留旧版本路由与资源，API／数据库独立升级。
