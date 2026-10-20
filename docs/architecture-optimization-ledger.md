# SaaS Platform 架构优化任务台账

版本：v0.58。创建/最近更新：2026-10-20（Asia/Shanghai）。源码基线：`32956c4`（各阶段工作树/工具摘要见验证证据）。主线Phase1–34及条件Phase35/36 COMPLETED；35/36仅本机隔离验收，37未触发。

本文件按照 `task-ledger-creator` 创建，是后续分阶段执行的状态、约束、决策与下一步的唯一台账。用户后续明确指令优先；改变范围/技术决策后先更新本文件。原[优化方案](architecture-optimization-plan.md)提供分析和设计依据，[系统架构](architecture.md)记录实际实现；不能把提案或历史测试当成已实现状态。

## 1. Project Overview（项目概况）

### Project Name

SaaS Platform：Vite/Rust/Nginx 架构统一、独立微应用交付与可靠性优化。

### Project Goal

保持平台身份/多租户授权与独立业务边界，将宿主及示例前端统一为 Vite，全部后台统一为 Rust；GitHub Actions 构建、Linux Docker/Compose 部署、独立入口 Nginx 容器代理。宿主与微应用推荐静态镜像交付，保留 ZIP 兼容及外部 URL 登记，统一预检/激活/回滚。交付工单和简单门店两个真实的 Vite 前端 + Rust API + 独立业务数据库项目，实证通用成员/授权契约。

### Background

- 当前已有 Rust/Axum/SQLx 平台、React/Vite 宿主、opaque iframe/SDK、实时权限、应用 token、静态版本和回滚。优先复用这些能力。
- 初建基线中工单/通知/OAuth仍为Node，两个静态微应用为Bun。Phase2已将静态示例及聊天前端迁为独立Vite、清除活动Bun入口；Node后台、工单/OAuth前端和旧网关仍待对应阶段迁移，根deploy源码仍缺失。
- 本台账创建前仅完成分析、优化方案与 README 更新，未实施运行时迁移、Nginx 配置、镜像部署、数据库变更或新发布 UI。创建时工作树已有这些文档改动，必须保留。
- 2026-10-14历史检查：前端build通过；六组契约88项/86通过/2跳过，配置缺render和lint缺ESLint失败，仅作起点。当前执行结果按phase-01/02证据独立记录，不继承历史通过；Phase2的真实浏览器fixture也不等于真实平台/PG/Docker/现网/恢复验收。
- v0.1–v0.19的决策、变更及验证记录保留其历史编号。v0.20按实际主线执行顺序重新编号为 **Phase 1–34主线 + Phase 35–37条件阶段**；阶段总数、职责、依赖和状态不变。源方案22个优化编号（含X02）仍完整映射；旧新编号与历史阅读规则见[阶段编号对照](phase-numbering-map.md)。
- 当前主线编号与默认执行顺序一致：Phase 1 → 2 → … → 34，最终验收为Phase 34。条件Phase 37仅在Phase 22实证必须原生WS时插入22与23之间；Phase 35/36须主线完成后按独立需求执行，不自动启动。

### Success Criteria

- 主线 Phase1–34全部有可复核终态及证据；若Phase37按条件触发，也必须完成；必需运行验收无跳过，已知外部限制明确记录。
- 活动前端入口全部 Vite；生产后台全部 Rust；Nginx 为唯一网关/静态服务。禁止 Bun、Caddy、Node 后台及 Next.js，历史审查文字不属于运行入口。
- 干净环境可构建并运行宿主静态镜像和独立 Rust 服务；主/微前端更新与业务库升级分别管理，旧资源与回退不丢失。
- 工单工程可独立安装/构建/部署，通过外部平台HTTPS API完成普通用户授权、CRUD、导出、处理人、提醒、失权和恢复。
- 门店工程独立交付门店资料、员工关联和每日营业记录；与工单真实复用受控成员协议，应用token/客户端/权限/数据grant互不串用，双页组织切换与实际SQL隔离通过。
- 每微应用前端镜像可由CI先部署固定版本URL，再由平台登记/预检/激活；平台直接管Docker和无共享存储跨机分发不作为默认前提。
- 多租户边界、撤销、并发消费、创建幂等、可信运行故障、跨页会话和关键审计有真实回归；性能结论有同口径测量。

## 2. Global Constraints（全局约束）

### Technical Stack

| 类别 | 固定约束 |
| --- | --- |
| 前端 | 宿主React/TypeScript/Vite；工单Vue/Vite；门店React/TypeScript/Vite；其他保留示例前端也使用Vite |
| 前端工具 | Node 24/npm仅用于Vite、前端打包、lint及前端/浏览器测试驱动；锁依赖；不是生产后台 |
| 后台 | Rust，优先复用现有Axum/Tokio/SQLx/reqwest-rustls/serde/tracing经验；业务/通知/OAuth为独立部署单元 |
| 数据库 | 平台和业务PostgreSQL分别持有数据；开发/验收使用隔离库 |
| 部署 | GitHub Actions构建/发行；Linux Docker/Compose运行；独立入口Nginx容器 + 宿主/微应用非root静态镜像；Rust API直接代理 |
| 实时 | Phase22实证Socket.IO兼容；若结果NATIVE_WS_REQUIRED则Phase37联合适配；Phase23按选定协议迁移通知服务 |
| 镜像 | 固定版本/digest与buildId/源码SHA，禁止以latest作为唯一回退依据 |

### Architecture Constraints

1. 平台负责身份、租户/组织、授权、应用发布与通知持久化；工单/门店服务各自持有业务库和账号，可同PG实例，不导入平台业务实现或直查身份表。
2. 默认HTML + IFRAME，opaque sandbox只保留所需能力；可信代码与iframe分故障域。ESM仍是随宿主构建的第一方路径。
3. 浏览器接入的是版本HTML URL，镜像是部署产物。公开URL、镜像digest、平台release与部署证据需要可追踪绑定。
4. 宿主镜像、微应用版本、Rust业务API及数据库分别升级/回退。active/previous、其他环境引用、在途任务与旧浏览器资源必须保留。
5. 镜像自带静态文件，不要求平台/前端共享产物卷；ZIP兼容使用Rust可写/静态Nginx只读持久卷、暂存不公开。只有仍需平台ZIP跨机分发且无共享存储才触发Phase35，Docker网络互通不等于文件共享。
6. 当前注册API拒绝未知字段；image/digest/target不能直接塞入旧RegisterBody。独立metadata或任务协议必须真实建模。
7. 保持现平台单crate并逐步建立窄lib/用例出口；新独立服务可用独立crate。根workspace不默认存在，若确有收益需记录决策并同步锁/门禁。
8. 入口Nginx独立容器；同机私有网络/service DNS为模板基线，只有入口公开80/443；服务重建DNS更新/受控reload须验证。平台登记公开HTTPS URL不自动生成Nginx规则，外部部署模板维护路由。
9. 首期共享登录、共享当前租户/组织；跨页刷新/退出/切组织统一协调。SDK仅消费宿主上下文，草稿按app/tenant/org隔离，不先引入每页独立会话。
10. 先工单窄成员策略，再门店真实接入实证，最后Phase30收敛公共能力；门店不能依赖尚未由双消费者证明的最终通用框架。

### Business Constraints

- 功能权限与数据grant独立，data grant绑定当前有效direct membership，不继承祖先角色数据范围；缺失默认拒绝。
- tenant/organization/owner由可信后台确认，列表/count/detail/write/export执行相同范围；处理人身份不扩大SELF。
- 工单保持版本冲突、状态流、不可变归属、软删除、真实CSV和独立提醒；创建需稳定幂等键及结果未确认恢复。
- 创建路径不得落入详情`:id`的读授权。菜单、manifest、校验器、前端路由与Rust CLI同步。
- 发布不自动授权用户；受控grant API/UI是完整示例主线交付要求，不能长期以DBA SQL作为常态接入。
- 镜像为首期推荐交付方式，ZIP是保留的兼容链，外托管是部署责任而非第三种互斥产物。每release绑定一个确定来源/不可变产物，双通道分别验收，不强制所有业务迁同一来源。
- 门店只做资料、员工关联与每日营业记录；示例一店映射一组织节点，不把该映射固化进平台，不扩展支付/POS/复杂库存，不新增STORE数据scope。
- grant首版写入要求实际平台根上下文及独立管理权限；组织管理员无隐式写权，业务操作无需grant管理权。显式委派cap/递归转授/通用审批不纳入首版。

### Security / Compliance Constraints

- 业务API持本应用机密clientId/secret与用户应用Bearer逐请求授权；Phase30已验combined/legacy等价，工单默认combined、门店因耗时收益不足保留legacy，失败不隐式回退；context/成员仍验证当前身份与范围。
- 校验active/aud/身份/有效期、当前permissions/dataGrants与组织硬上限；contextVersion是内容hash，不是可信认证凭据。
- JWT签名密钥只在平台；secret/DB URL/宿主session不得进入VITE配置、dist、ZIP、URL、错误或日志。CORS `null`不构成身份认证。
- 静态资源可匿名CORS；opaque iframe业务请求仍为Origin:null，即使同域。业务CORS由Rust生成、Nginx透传，不全局放开宿主管理API。
- 生产保持HTTPS正常验证，目标来自白名单；不关闭TLS、不接受任意URL/命令、不把平台API挂Docker socket。
- 关键变更与审计同事务；平台失联或无法证明授权时停止操作。备份/清理/恢复仅在明确受控目标执行。

### Code Style / Engineering Rules

- 遵循适用AGENTS：先边界后目录、单向依赖、薄入口/窄库、不提前拆大量crate；复用现有能力，新增依赖说明必要性。
- 先补能复现目标问题的测试，再改实现；测试验证行为/协议/隔离，避免镜像实现的无价值断言。
- 不修改已执行迁移校验和；工单旧SHA256迁移接管与空库初始化分别验收，不能直接让SQLx重跑0001。
- 异步后台避免阻塞计算/IO；总deadline、密码池、平台HTTP、DB acquire/statement/lock分别有预算与背压。
- 每Phase完成时更新台账摘要、详情状态、证据、决策/变更日志和下一指针；同步受影响的现状架构/组件README。
- 每Phase证据写入`docs/validation/phase-XX.md`，包含源码/工具版本、实际命令、结果分类、脱敏日志、兼容/回退与限制。phase-01/02已创建，其余随阶段实施；不能只有一句“测试通过”。
- 当前创建台账只做文档；执行时默认只做用户指定/指针阶段，不自动跨阶段、真实外部发布、推送镜像、发告警或生产数据操作。

### Forbidden Actions

- Phase34独立交付、隔离验收及终态同步已完成；其他阶段、外部告警发送、外部镜像发布及生产部署须新的明确请求。
- 不把分析完成、旧测试、模板存在或静态配置解析当成Rust迁移/容器/数据库/浏览器已验收。
- 不用删除旧库、改历史checksum、强制覆盖版本、删除旧chunk、SPA吞404或放宽iframe解决问题。
- 不凭空填真实域名、仓库、目标主机、凭据或客户端ID；保留/SLO/RPO/RTO候选与生产承诺明确区分，不自动启用生产清理。
- 不自动启用Phase35/36；Phase37仅按Phase22真实兼容失败结果触发，不以缺环境冒充失败。不因Rust化引入Kubernetes/Redis/MQ/物理按租户分库。

### Status and Evidence Rules

Phase只允许`NOT_STARTED`、`IN_PROGRESS`、`BLOCKED`、`COMPLETED`。当前状态以汇总表及各阶段详情为准；本台账的计划状态与业务API的DRAFT/PASSED/FAILED不是同一字段。

执行时只有在必需输入/前置完成后进入IN_PROGRESS；实际外部阻碍无法继续则记录具体BLOCKED及恢复条件。必需验收未运行/失败/跳过不能标COMPLETED；可选外部渠道不在本地验收范围时必须写明，不能宣称现网已验证。用户没有要求暂停时不自造暂停状态。

条件Phase初始未触发，均保持NOT_STARTED。用户明确指定执行某条件阶段但其触发输入缺失时，记录BLOCKED表示该执行请求受输入阻塞，不表示触发条件已满足；未被请求且未触发的条件阶段仍保持NOT_STARTED。Phase35/36是独立业务扩展；Phase37是已纳入方案的技术失败分支，有证据触发后即成为23/34必需门禁，无需重复选择同一fallback。主线共34个阶段（1–34），最终验收为Phase 34；未触发条件阶段不计已完成。

## 3. Phase Status Summary（阶段状态汇总）

当前编号自v0.20生效：主线Phase 1–34按编号执行，条件Phase 35–37单列在末尾；条件37触发时插入22→37→23。[旧新编号对照](phase-numbering-map.md)只供历史追溯，不作为第二套执行编号。

| 里程碑 | 主线阶段 | 可审阅交付 |
| --- | --- | --- |
| M1 交付与验证基础 | 1–5 | 工具、Vite静态示例、独立入口Nginx/宿主镜像、GitHub CI、ZIP原子发布兼容 |
| M2 契约与身份授权 | 6–15 | SDK/生命周期、导航会话、OAuth/成本保护、审计与数据授权API/UI |
| M3 Rust示例及镜像接入 | 16–28 | 工单/门店真实业务链、Rust实时与演练、CLI、Linux镜像/外部URL入口管理 |
| M4 性能与运营验收 | 29–34 | 宿主/授权优化、数据策略、指标告警、恢复、完整分离示例发行 |
| 条件阶段 | 35–37 | ZIP跨机分发、平台容器生命周期；或22失败时在23之前执行原生WS联合适配 |

| Phase | Name | Status | Depends On | Output Summary |
| --- | --- | --- | --- | --- |
| [1](#phase-1) | 冻结实施基线、工具版本与验证口径 | COMPLETED | - | 固定工具版本；干净npm ci后lint/typecheck及70项无DB契约通过；单crate Rust入口与组件/运行/协议清单落地。既有失败、warning/ignored及未运行项见docs/validation/phase-01.md。 |
| [2](#phase-2) | 将静态兼容示例和根前端入口迁为 Vite | COMPLETED | 1 | 两个classic微应用与聊天前端独立Vite；根npm/锁/受控调度、相对产物与404。13项开发/5项交付/70项契约通过；旧管理UI fixture失败独立记录，见phase-02证据。 |
| [3](#phase-3) | 恢复 Nginx 部署基线并交付宿主静态镜像 | COMPLETED | 1, 2 | 独立入口/非root只读宿主Nginx、默认无共享卷及ZIP/legacy模板、干净原生构建。38策略/70契约/8 Rust测试与实际Docker HTTPS七组通过，DNS换IP、ZIP重建及旧入口资源保留实证见phase-03。 |
| [4](#phase-4) | 建立项目受管的分层 CI 与隔离测试环境 | COMPLETED | 1, 2, 3 | fast/PG/浏览器/delivery受管门禁；干净Linux fast19步、PG21项、三浏览器各143组及最终受影响11组、三OCI镜像/HTTPS七组通过。最终工具41项零skip，源码差异与外部发行限制见phase-04。 |
| [5](#phase-5) | 完善 ZIP 兼容静态包的原子安装与恢复对账 | COMPLETED | 3, 4 | ZIP/tar私有暂存、Linux/Windows禁止覆盖原子安装、规范摘要/跨环境幂等、0002及未知提交四态对账、离线维护/metadata隔离。Windows17、Linux16/维护3/PG27/契约99/CI48及真实production HTTPS9通过；原始失败/补验/清理与403项输入见phase-05。 |
| [6](#phase-6) | 版本化 SDK 与公开授权契约 | COMPLETED | 1, 4 | SDK1.0.0候选/同源schema+TS+Rust/classic+ESM及30项漂移检查；宿主+三个前端消费、旧401修复、fast23/PG27/三浏览器各146及真实宿主镜像通过，472输入与清理证据归档。 |
| [7](#phase-7) | 隔离可信微应用永久挂起的故障 | COMPLETED | 4, 6 | 四阶段10秒截止、页面可信队列/故障锁存及整页恢复，iframe独立销毁；runtime20+14、Windows29、Linux fast23/契约189与三产品各151通过。473输入/11路径变化，后端/SDK/业务/锁不变，失败及清理例外见phase-07。 |
| [8](#phase-8) | 分离展示更新、授权续期和版本重挂 | COMPLETED | 6, 7 | 完整授权投影与展示复用、context入口权威、失权清理及有界恢复；宿主51/消费者97/SDK CHECK30、Windows47、Linux fast23/契约221与三产品各169通过。478输入/45路径，旧快照与失败及清理见phase-08。 |
| [9](#phase-9) | 统一菜单查询、路由策略与应用导航 | COMPLETED | 4, 6 | 查询/显式路由策略与共享导航代次、父层单一应用推断、ticket-create菜单/工具闭环；专项22/24/70/16、Windows183、Linux fast23/契约311及三产品各183通过。493输入/39路径含6既有工具范围扩展，失败/清理见phase-09。 |
| [10](#phase-10) | 协调多标签页的会话轮换与上下文 | COMPLETED | 4, 9 | 原子record/Web Locks/版本通知、持久intent/unknown、条件提交及旧请求/通知/草稿隔离；正常平台轮换保留iframe。专项39/21、Windows200、fast23/契约362及Linux完整600/兼容34通过，499项freeze1/36路径，见phase-10。 |
| [11](#phase-11) | 修复普通 OAuth 原子签发与实时失权 | COMPLETED | 4, 6 | user→client→credential锁序、消费/完整token对/签发审计同事务、共享实时身份、随机jti与0003兼容迁移；PG7组41（OAuth10/psr4）及fast24通过，502输入/12路径，见phase-11。 |
| [12](#phase-12) | 限制登录内存和密码计算成本 | COMPLETED | 4 | 硬容量/TTL/时钟登录预算、2+8默认密码池与取消/截止背压、真实可信代理来源、全HTTP接入/改密CAS；100k计数/混合24请求回归，PG8组46及fast27通过，505项9b016ace…/20路径，见phase-12。 |
| [13](#phase-13) | 使关键管理变更与审计同事务 | COMPLETED | 4, 11 | 严格事务审计/安全前后摘要、删除身份/no-op与有界失败计数，关键管理/授权全调用链接入；审计抛错/静默跳过、重试及并发回归，PG10组64与fast27同506项582d2759…通过，见phase-13。 |
| [14](#phase-14) | 提供受控数据 grant 管理 API | COMPLETED | 6, 13 | typed API/用例/存储、新增必要迁移、契约与正负向隔离测试。 |
| [15](#phase-15) | 实现数据 grant 管理界面与授权闭环 | COMPLETED | 9, 14 | 独立页面/窄发现/预览差量/恢复及审计；PG74、fast28/568、五浏览器80 live通过；r2主题测试FAIL保留，修复后当前三产品各11项补验通过，537项输入不变，见phase-15。 |
| [16](#phase-16) | 建立 Rust 工单服务并接管业务库基线 | COMPLETED | 1, 4, 6 | 独立Rust基线/安全接管、真实PG15项与无Node ELF11项、配置/运维和完整证据。 |
| [17](#phase-17) | 实现 Rust 工单授权及只读/导出契约 | COMPLETED | 4, 6, 11, 16 | 有界两段平台授权/统一scope只读与CSV；PG9步（74/15/13及live1+13）、lib34/doc1、无Node11项及本阶段门禁通过，8份计划/674项证据；完整fast48/50、外部两失败与当前补验、原包装器FAIL等分列，见phase-17。 |
| [18](#phase-18) | 实现 Rust 工单写入、幂等和短事务 | COMPLETED | 6, 13, 17 | Rust创建幂等/版本状态/软删除、0002双历史与短事务；四项验收PASS，691项9bea11c1855b…候选；platform-postgres 74PASS、work-orders-postgres 21PASS、work-orders-read-postgres 13PASS、work-orders-write-postgres 14PASS、work-orders-live-postgres 1PASS/named13、work-orders-write-live-postgres 1PASS/named12；Linux lib43/doc1、同源runtime12项；完整fast子级状态PASS，52步/52PASS/0FAIL，测试745PASS/0FAIL/0skip，实际证据与限制见phase-18。 |
| [19](#phase-19) | 迁移 Rust 工单成员目录与分配校验 | COMPLETED | 14, 17, 18 | Rust成员目录/有效直接成员分配、平台窄用途声明、锁前预校验与锁内复核/短有效期/SELF不扩张；三项验收PASS，700项52ebb7a5904b…候选；platform-postgres 76PASS、work-orders-postgres 21PASS、work-orders-read-postgres 13PASS、work-orders-write-postgres 14PASS、work-orders-member-postgres 12PASS、work-orders-live-postgres 1PASS/named13、work-orders-write-live-postgres 1PASS/named12、work-orders-member-live-postgres 1PASS/named10；Linux lib50/doc1、同源runtime12项；完整fast子级状态PASS，53步/53PASS/0FAIL，测试765PASS/0FAIL/0skip，实际证据与限制见phase-19。 |
| [20](#phase-20) | 迁移 Rust 工单提醒到平台通知API | COMPLETED | 17, 18, 19 | Rust提醒双授权/五元域不可变意图、0003安全接管、同键恢复与目标变更冲突、真实持久收件箱；三项验收PASS，711项4f9a2cb80ece…候选；platform-postgres 76PASS、work-orders-postgres 27PASS、work-orders-read-postgres 13PASS、work-orders-write-postgres 14PASS、work-orders-member-postgres 12PASS、work-orders-reminder-postgres 12PASS、work-orders-live-postgres 1PASS/named13、work-orders-write-live-postgres 1PASS/named12、work-orders-member-live-postgres 1PASS/named10、work-orders-reminder-live-postgres 1PASS/named12；Linux lib56/doc1、同源runtime12项；完整fast 53/53步PASS、778PASS/0FAIL/0skip，实际来源及限制见phase-20。 |
| [21](#phase-21) | 交付 Vue + Vite 工单前端与未知结果恢复 | COMPLETED | 6, 8, 9, 10, 15, 17, 18, 19, 20 | 独立Vue/Vite相对ESM/chunk、同步SDK握手、Rust真实业务/未知显式恢复与四元隔离；三项PASS，726项675b639c3dd5…；完整fast 55步/832PASS/0FAIL/0skip；chromium真实15项、Phase8 4组/Phase10 17组；chrome真实15项、Phase8 4组/Phase10 17组；msedge真实15项、Phase8 4组/Phase10 17组；独立干净构建3步/15源码输入PASS；正式来源/清理/限制见phase-21。 |
| [22](#phase-22) | 验证 Rust 实时协议兼容并冻结选型 | COMPLETED | 1, 4, 6 | SOCKET_IO_COMPATIBLE；socketioxide0.18.7/engineioxide0.17.7，协议44、unit7、前端5、契约48通过；Rust聊天参考及63项证据，37未触发；执行指针不改。 |
| [23](#phase-23) | 迁移 Rust 通知服务并实现有界分区调度 | COMPLETED | 4, 6, 10, 12, 22（WS分支另需37） | 独立Rust/有界FIFO/真实复验与全TCP退出；Linux34联合+双页6、Rust29、回归87与镜像9通过，682输入匹配并清理；旧Node退役，指针不改，见phase-23证据。 |
| [24](#phase-24) | 拆分 OAuth 演练为 Vite 页面和 Rust 流程API | COMPLETED | 2, 4, 6, 11 | 独立Vite/Rust、服务端有界状态与秘密、四类真实平台流程；Linux27 JS/17 Rust/12 live+1 fixture、HTTPS10、native13通过，失败/清理/输入见phase-24；执行指针不改。 |
| [25](#phase-25) | 以 Rust CLI 替换平台登记与授权工具 | COMPLETED | 4, 9, 11, 13, 14 | 独立Rust CLI/严格schema与manifest、旧Node管理入口退役；Linux17步/21Rust/428JS/9TLS/13live+1fixture/9无Node通过，正式grant API及清理证据见phase-25；执行指针不改。 |
| [26](#phase-26) | 建立简单门店 Rust API 与独立业务库 | COMPLETED | 4, 6, 14, 17, 19 | 独立stores Rust/业务库/专属账号、受控成员、SQL范围/版本和十进制一店一日报；五项PASS，757项2a00d4ecf055…；完整fast 61步/894PASS/0FAIL/0skip；stores SQL 1父测试/8具名检查、real live 1父测试/16具名检查PASS；来源/清理/边界见phase-26。 |
| [27](#phase-27) | 交付门店 Vite 微应用与双消费者契约实证 | COMPLETED | 6, 8, 9, 10, 15, 21, 25, 26 | 独立React/TypeScript/Vite门店、11菜单与9精确动作、真实三库CRUD/员工/日报和双消费者；同源fast63步/1033PASS，三浏览器各21组PASS，五项原验收及资源/缓存清理通过。 |
| [28](#phase-28) | 交付工单/门店镜像与统一外部 URL 接入 | COMPLETED | 3, 4, 6, 21, 25, 27 | 四静态与双Rust API镜像、固定digest/权威清单及URL登记联合验收完成；839项f77df330f956…，fast63步/1083PASS，14交付检查与三浏览器各9组PASS；119历史证据及其他阶段保持。 |
| [29](#phase-29) | 收敛宿主加载、失败恢复和高复杂页面边界 | COMPLETED | 3, 4, 7, 9 | 壳/面板按需、局部恢复与应用编辑用例拆分；同口径5次冷启动transfer下降19.93%；完整Chromium209、三产品专项各9及既有200、Linux391/策略55/Nginx7通过。原FAIL、原始测量/预算/回退见phase-29。 |
| [30](#phase-30) | 优化授权查询链并收敛 Rust 能力模块 | COMPLETED | 6, 11, 13, 14, 17, 18, 19, 20, 23, 27 | 受控合并/新语句撤销与SHA投影复核、窄lib/私有SQL、checked分页及CI；862项178ffd6ed484…/45路径，正式31步/153 Rust与补充205政策PASS。双模式真实三库及工单四类联合、旧/新ELF与预算恢复PASS；combined HTTP减50%/PG调用减21.05%，门店耗时收益不足保留legacy。五项PASS及原失败/耗时回归/清理见phase-30。 |
| [31](#phase-31) | 交付受支持升级与 dry-run 保留策略 | COMPLETED | 4, 5, 11, 14, 16, 18, 23, 26, 30 | 新增0004/持久在途与未知保护、根限定GET/Rust CLI只读预览、升级矩阵/分类政策；885项621ea69d5326…，正式32步、工单27/发布17/审计8及旧消费者双模式真实三库PASS；旧URL/ZIP与全部owner清理见phase-31，生产apply不可用。 |
| [32](#phase-32) | 建立真实指标、告警与容量基线 | COMPLETED | 3, 5, 17, 18, 23, 26, 27, 28, 29, 30 | 私有Rust监控/固定指标、两轮真实故障/补拉/200-404隔离、100/150/200会话与10万业务记录；906项583f7f47d727…，正式服务40步/浏览器20步及宿主26项PASS，候选未达标如实记录，见phase-32。 |
| [33](#phase-33) | 恢复加密备份与隔离新库恢复工具 | COMPLETED | 3, 16, 18, 23, 26, 31, 32 | 独立Rust/AES-GCM/HTTPS远端认证与三库新目标、全凭据失效/审计、真实CRUD/通知/路由/四镜像/四页面；932项ef0082e18930…，正式20步/十检查/Rust3/CI33 PASS；pilot RPO 44.808s/RTO 109.137s，物理异机/生产PITR另验，详见phase-33。 |
| [34](#phase-34) | 完成独立示例交付与全仓目标栈验收 | COMPLETED | 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33（WS分支另需37） | 935输入9a23e8808cad…；双独立源码包/六镜像、fast74/PG24/delivery7步及三浏览器组合全覆盖；最终Socket.IO44/镜像各9/恢复十检查PASS，pilot RPO 44.170s/RTO 114.557s；主线完成，无自动下一阶段。 |
| [35](#phase-35) | 条件扩展：Rust跨机产物分发 | COMPLETED | 5, 28, 32, 34 | 本机隔离双节点ZIP无共享存储、Rust agent/API/UI、逐节点核验和门禁；15项实证/三浏览器，fast 82步/1133，17项发布回归；历史证据/旧迁移保持，资源清理；见phase-35-implementation。 |
| [36](#phase-36) | 条件扩展：平台受控镜像部署与容器生命周期 | COMPLETED | 28, 32, 34 | 本机隔离agent/API/UI、可信回执/别名门禁、回滚/引用保护；81步fast1127、18部署与3浏览器、17发布/ZIP/升级全部通过，资源清理完成；见phase-36-implementation。 |
| [37](#phase-37) | 条件分支：原生 WS 前后端协议联合适配 | NOT_STARTED | 3, 4, 6, 10, 22 | 未触发：22为SOCKET_IO_COMPATIBLE、34真实协议44/44通过；本次执行请求核验为BLOCKED_NOT_APPLICABLE，保持NOT_STARTED，见phase-37。 |

Allowed status values: `NOT_STARTED` / `IN_PROGRESS` / `BLOCKED` / `COMPLETED`。

## 4. Phase Details（阶段详情）

<a id="phase-1"></a>
### Phase 1: 冻结实施基线、工具版本与验证口径

#### Status

COMPLETED

#### Goal

建立可重复执行的工程入口和真实基线，后续阶段不再依赖个人缓存或历史通过记录。

#### Scope

- 核对 Rust/Node/npm 版本、锁文件和实际 package/Cargo 入口，建立组件与遗留运行时清单。
- 修复前端 lint 的缺依赖/配置；声明前端工具版本，整理 Windows/Linux 的标准检查命令。

#### Out of Scope

- 不迁移业务服务、不构建生产发布物，不执行数据库变更或真实部署。

#### Inputs

- 方案编号：E01、T01；本台账全局约束及源方案对应章节。
- package.json、frontend/package.json、backend/Cargo.toml、各组件 README、.github/workflows/production-build.yml。
- 现有源码 SHA 32956c4；优化方案中的历史测试和缺失 deploy 证据。

#### Execution Requirements

- 分别记录已执行、失败、跳过、不可用的检查；不把缺环境标为通过。
- Node 只作为前端工具；迁移清单列出剩余 Bun/Caddy/Node 后台及其退出阶段，不增加新的遗留入口。

#### Expected Outputs

- 工具版本声明、真实可执行的 lint/类型检查入口、受管验证清单。
- 组件/运行入口/现有协议基线清单，记录部署目录缺失与已知失败。
- 实施证据：[docs/validation/phase-01.md](validation/phase-01.md)，2026-10-15已创建。

#### Acceptance Criteria

- [x] 干净安装可执行前端 lint、类型检查和选定无数据库契约测试；结果绑定源码 SHA 与工具版本。
- [x] Rust 检查入口与当前单 crate 形态一致，不引用不存在的 workspace。
- [x] 清单覆盖工单、通知、OAuth、websocket 和 app-backend 示例；没有遗漏生产 Node 入口。

#### Risks

- 默认PATH确认为Node v16.13.1，已用明确的Node24.18.0/npm11.16.0执行；使用前仍须核对PATH。lint为0 errors/64 warnings，现有债务保持可见。既有source-map-js高危传递依赖、Rust fmt/Clippy失败和52 ignored未伪标通过。

#### Dependencies

无实施阶段依赖，可作为当前首个执行阶段。

#### Notes

不因旧缺陷将整个阶段泛称阻塞；先完成本阶段工具与证据产物，未能运行的必需验证按全局规则记录。

2026-10-15完成。工具版本为Node24.18.0/npm11.16.0/Rust1.97.0，前端干净npm ci、lint/typecheck与选定70/70契约全部成功且零skip；Rust check通过，默认测试37通过/52 ignored，doc成功含2 warnings，fmt/严格Clippy既有失败已分类记录。原生配置测试重现缺deploy/native/render.mjs失败。

输出：[工程/运行/协议及受管检查基线](engineering-baseline.md)、[源码/版本/命令/结果/限制/回退证据](validation/phase-01.md)。已同步根README与实际架构文档；未修改业务源码、迁移或启动真实服务，未构建生产产物。根Bun入口/静态示例迁移仍归Phase2，Nginx及deploy恢复归Phase3，受管CI归Phase4；本阶段仅给出检查入口与真实基线，不宣称全仓门禁已清零。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-2"></a>
### Phase 2: 将静态兼容示例和根前端入口迁为 Vite

#### Status

COMPLETED

#### Goal

移除 Bun 静态服务依赖，使宿主及 Vue/jQuery 等静态示例具备标准 Vite 开发和构建入口。

#### Scope

- 根前端调度脚本、mini-services/micro-vue、micro-jquery、websocket 示例的静态前端。
- 删除 Bun 命令/API和 bun.lock；修复个人绝对路径、端口配置与受控启动/停止。

#### Out of Scope

- 工单前端单独由 Phase 21 迁移；OAuth 页面由 Phase 24 拆分；不伪称全仓库已完成 Rust 化。

#### Inputs

- 方案编号：T01、E01；本台账全局约束及源方案对应章节。
- scripts/dev.sh、scripts/start-all.sh、mini-services/micro-vue、micro-jquery、examples/websocket。

#### Execution Requirements

- 使用 Vite dev/build/preview、版本内相对资源和真实生命周期；保留微应用取消/卸载语义。
- 依赖只从本工程锁文件安装；不借宿主 node_modules 作为正式依赖。

#### Expected Outputs

- 示例 vite.config、npm scripts、构建产物规范、跨平台开发入口与 README。
- Bun 清除记录及更新后的示例回归。
- 实施证据：[docs/validation/phase-02.md](validation/phase-02.md)与[62项输入摘要](validation/phase-02-inputs.sha256)，2026-10-15已创建。

#### Acceptance Criteria

- [x] 两个静态微应用均能独立 npm ci/build；产物在版本前缀下加载且缺资源真实 404。
- [x] 宿主与示例的有效前端启动命令不调用 Bun；未破坏现有生命周期/迟到响应测试。
- [x] 构建资源中不含 secret、数据库地址或宿主 session。

#### Risks

- 旧根绝对资源及import.meta.dir已退出；保留classic/TRUSTED兼容示例，未增加iframe同源权限。默认opaque iframe仍须SDK握手。Linux/POSIX清理路径未真实运行，旧全量browser management fixture仍失败，不计为Phase2通过。

#### Dependencies

Phase 1必须COMPLETED。

#### Notes

前端 .mjs、SDK 和 JS 测试驱动允许保留；禁止的是 Bun 和 Node 后台服务。

2026-10-15完成。三个示例分别独立npm ci，Vite8.3.4精确锁；Vue/jQuery各7/7、聊天5/5，根70项契约、13项开发（含四真实Vite/自定义代理/清理）、5项交付/产物扫描全部通过且零skip。宿主仍Vite6.4.3，未无关升级；所有app.js/vendor字节保持。新根/示例audit为0，宿主既有64条lint warning及Phase1旧债务保留。

已删除Bun锁/两个Bun静态服务及旧Bun.SQL探针，修复个人路径/启动停止/绝对readiness预算。根build:all先示例构建→同步dist→宿主构建；宿主缺版本目录可启动、静态缺资源真实404，四dist未含哨兵secret/DB/session。

真实浏览器vendors组1场景覆盖两应用、releases组3场景（含真实Vue独立build/打包/双版本/回滚）及聊天隔离协议均通过。旧全量宿主套件最终18PASS/1FAIL/2NOT_RUN，management的basePath/登记表单fixture与现UI不符；后续Phase4门禁须修订，不能继承为全量通过。初次独立Vue fixture旧public输入已修为本工程Vite构建，并有实际通过证据。

同步根/组件README、实际架构与工程基线，源码/工具/命令/失败/限制/回退详见[phase-02](validation/phase-02.md)。没有运行真实后台/DB/迁移或生产部署，未进入Phase3。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-3"></a>
### Phase 3: 恢复 Nginx 部署基线并交付宿主静态镜像

#### Status

COMPLETED

#### Goal

从干净检出构建非root Nginx宿主镜像，建立独立入口Nginx容器与静态/Rust上游的实际HTTPS通路；ZIP卷只属于兼容profile。

#### Scope

- frontend/Dockerfile、入口/静态Nginx、Compose私有网络及环境模板，恢复必需原生构建入口。
- 替换旧网关运行阶段、专用卷和路径；推荐镜像不共享产物，ZIP兼容单列static服务/读写卷。

#### Out of Scope

- 不恢复 Caddy 兼容选项，不迁移 Node 业务逻辑；监控/备份工具分别由 Phase 32/33 完成。

#### Inputs

- 方案编号：D01、D03、T01；本台账全局约束及源方案对应章节。
- frontend/Dockerfile、docker-compose.yml、tests/micro-release-gateways.mjs、缺失 deploy 路径清单。
- 方案第 12.4、13 节的配置与响应头约束。

#### Execution Requirements

- 入口独立容器使用service DNS，只有入口发布80/443，静态服务不发布公网端口；外部网络先受控创建。
- Rust API直接代理；公开origin和真实trusted proxy分别配置；容器重建后的DNS解析/受控reload必须验证。
- 非root/只读配置实际可写PID/temp，禁止向只读配置envsubst；宿主旧hash资源有可执行保留策略。
- 已登记版本使用明确路由，未知/sub-apps版本返回404；ZIP profile使用持久卷且暂存不公开。
- 早期代理通过真实Rust传输fixture验证polling/Upgrade，不提供放行业务的生产stub；完整协议/授权由23/34验收。

#### Expected Outputs

- 真实的 Nginx 配置、宿主多阶段 Dockerfile、Compose/环境模板和部署说明。
- 可重复的镜像构建、Nginx 语法及网关契约测试；修正 CI 的缺失路径调用。
- 实施证据：[docs/validation/phase-03.md](validation/phase-03.md)，2026-10-15已创建；含49项输入摘要、实际Docker/原生/基础镜像结构化证据。

#### Acceptance Criteria

- [x] 干净源码构建并非root/只读运行Nginx镜像，运行阶段无Node/Bun/Caddy后台。
- [x] 独立入口容器通过私网访问静态/Rust上游；重建上游后仍正确解析/受控reload，只有入口暴露公网端口。
- [x] 实际HTTPS验证API前缀、polling/Upgrade传输、CORS/CSP、MIME/缓存、严格404，未知微版本不进入宿主fallback。
- [x] 推荐纯镜像服务无需MICRO_APPS_DIR挂载；ZIP profile重建不丢目录，旧宿主chunk真实可用。
- [x] 使用真实WSL Linux Docker/Nginx验收；配置检查与运行分别记录，最终必需组无skip，初始环境/设置失败及恢复保留。

#### Risks

- 真实生产域名/registry/digest/CPU/主机/网络/TLS及升级条件仍由部署方提供；本次隔离验证不代表现网已部署。
- Docker DNS换IP已实测；生产仍须核对固定入口IP与TRUSTED_PROXY_IPS。历史assets不自动删除，正式窗口/容量清理由Phase31收敛。

#### Dependencies

Phase 1、2必须COMPLETED。

#### Notes

入口独立Nginx容器与Linux同机私网是首期模板基线，实际目标参数仍待提供；工单/门店镜像版本路由在Phase28联合验收。

2026-10-15完成。已恢复deploy/gateway、static、native与环境/Compose模板，删除活动Caddy入口/卷/网关测试。原生最终构建为Windows x64，289项输入/141产物摘要一致；Linux宿主镜像与Rust传输fixture另在WSL Docker29.1.3实测，最终8/8（父测试+7验收组）零失败/跳过。上游IP从172.18.0.3换为172.18.0.6后自动解析到fixture-b；ZIP静态重建保留字节；新入口与旧入口不同，旧入口不在currentAssets、仍在合并assets且旧URL200/摘要一致，宿主无共享卷。

输出：[实施/失败恢复/限制与Self-Check](validation/phase-03.md)、[实际容器摘要](validation/phase-03-runtime.json)、[最终原生审计](validation/phase-03-native.json)、[部署说明](../deploy/README.md)。实际测试资源已按owner核对清理，镜像与日志本地保留。本次未运行真实业务数据库/生产服务、未推送镜像；原生Windows产物不当Linux验收，Rust fixture不当Socket.IO协议选型或业务授权通过。Phase4及后续未实施。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-4"></a>
### Phase 4: 建立项目受管的分层 CI 与隔离测试环境

#### Status

COMPLETED

#### Goal

建立GitHub Actions Linux构建/验证与预发布验收分层，普通开发机和runner不依赖个人缓存。

#### Scope

- 固定Vite/Node/npm与Rust工具链，临时PG、Playwright、Docker及HTTPS网关门禁。
- PR快检、发行候选完整验收、镜像构建/发行清单与脱敏报告；真实部署步骤按目标环境配置。

#### Out of Scope

- 不以空断言替代真实服务，不连接生产库，不提前把尚未迁移服务的目标测试标为通过。

#### Inputs

- 方案编号：E01；本台账全局约束及源方案对应章节。
- .github/workflows/production-build.yml、frontend/package.json、tests/ 和现有服务测试。

#### Execution Requirements

- 按实际manifest执行，迁移服务逐阶段接入；隔离库用临时名和受控生命周期。
- GitHub只构建/检查/推送固定产物，LinuxCompose拉固定digest；域名/registry/CPU架构使用明确占位直到真实值提供。
- Fork PR不取得生产秘密，发行凭据仅受控发布步骤可用；构建成功与目标部署成功分别记录。
- Chromium为必需浏览器门禁；首期Chrome/Edge真实版本验收，Firefox/WebKit作为扩展检查，不把WebKit等同Safari产品支持。
- 缺必需环境时发行门禁失败，真实Linux预发布验证容器/HTTPS/恢复，不以mock代替。

#### Expected Outputs

- 快速 PR gate、数据库/浏览器 gate、交付 smoke gate 和可复现安装入口。
- 测试分层及必需执行清单，脱敏日志/版本/源码 manifest。
- 实施证据：[phase-04.md](validation/phase-04.md)、CI/PG/浏览器/交付紧凑 JSON、395 项最终源码 manifest；安装与实际清单见 [CI 使用说明](../scripts/ci/README.md)。

#### Acceptance Criteria

- [x] 干净Linux runner可运行无DB契约、临时PG和项目管理的浏览器测试，无个人缓存路径。
- [x] 必跑skip/失败能阻止对应门禁，报告分列pass/fail/skip及源码/工具版本。
- [x] 镜像构建输出目标架构、commit、buildId和digest清单，PR不泄漏生产秘密。
- [x] 未创建workspace时不使用空根cargo test --workspace伪门禁；预发布与外部发行限制明确。

#### Risks

- 当前已在授权 WSL Ubuntu 本地 Docker 中实际验收，PG 四模块串行、Rust jobs=2，私网/tmpfs/owner 清理已核验。GitHub 托管实际执行、生产 registry/架构/域名/主机与 Chrome/Edge 最近两稳定版完整矩阵仍需真实环境；本次不冒称通过。

#### Dependencies

Phase 1、2、3必须COMPLETED。

#### Notes

后续每个阶段只运行与变更相称的检查，并把新契约挂入本层级。

2026-10-15 实施结果：GitHub Ubuntu24.04 的 fast → 隔离PG/Chromium/Chrome/Edge → 非PR delivery 及最终聚合门禁落地；Node24.18.0/npm11.16.0/Rust1.97.0、Playwright1.64.1 与独立锁文件固定，源码/版本/计数/脱敏日志分列，必需失败/skip/零测试/缺报告均拒绝。fork PR 无生产秘密，checkout 不留凭据，当前 workflow 不执行 registry 推送或目标部署。

实际干净Linux fast r2为19/19步骤PASS：构建/lint/typecheck、70契约、34部署单元、19 dev/Vite、Rust全目标check、config3及transport8；临时PG四模块21/21，21个测试库移除/pool关闭确认。三Linux浏览器各8套/143组零fail/skip，随后最终部门等待助手变更在三产品各补跑11/11及3项等待回归，明确重复子集、不算154唯一组；Windows三产品143组及受影响补跑作补充。最终工具源码Linux/Windows各41项PASS零skip，含共享清理、创建/attach/回收时取消与POSIX后代监听者排空回归。

真实delivery五步骤PASS：Compose4项、三份linux/amd64 OCI archive完整字节图校验、平台实际ELF/UID10001/ldd、HTTPS父测试+七组8/8；覆盖上游换IP、ZIP静态容器重建与旧hash资源恢复。buildId为`32956c4532bb-e77009d4a4e461da`，OCI digest/归档摘要见[交付JSON](validation/phase-04-delivery.json)。它不证明完整业务/Socket.IO迁移或数据库灾难恢复。

最终源码395项`inputsSha256=00b536afd8b8baaa2406d074af952a7d560758c2ba8bbeabf366a1bcaa6fca27`，各次实际快照与最终差异单独保存，原交付289项生产输入在最终树逐文件摘要一致。初次fast、浏览器fixture和导出/网络失败均保留；本阶段修正测试基础设施及开发进程清理，没有重写业务UI/平台Rust/数据库。现有fmt/Clippy、历史ignored及64 lint warnings不算清零；GitHub远端运行NOT_RUN、registry NOT_REQUESTED、目标部署NOT_RUN。自有测试容器/网络已清理，镜像/日志本地保留。

输出：[实施与Self-Check](validation/phase-04.md)、[最终输入](validation/phase-04-inputs.json)、[CI/差异](validation/phase-04-ci.json)、[PG](validation/phase-04-postgres.json)、[浏览器](validation/phase-04-browser.json)、[交付](validation/phase-04-delivery.json)。根README、架构与工程基线已同步；Phase5及未来阶段未执行。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-5"></a>
### Phase 5: 完善 ZIP 兼容静态包的原子安装与恢复对账

#### Status

COMPLETED

#### Goal

使现有 ZIP/tar.gz 发布在同机共享目录中不可见半成品，并能安全处理重试与未知提交结果。

#### Scope

- 安全解包后的暂存、清单/摘要、同文件系统原子安装、并发同版本发布和数据库登记对账。
- 上传/预检/激活/回滚与 Nginx 只读版本目录的联通。

#### Out of Scope

- 不实现跨机分发、不构建或启动微应用镜像；不取消容量/路径/来源限制。

#### Inputs

- 方案编号：D02；本台账全局约束及源方案对应章节。
- backend/src/release_archives.rs、routes/application_releases.rs、releases.rs、tests/package-microapp.mjs。

#### Execution Requirements

- 暂存不对 Nginx 公开；禁止覆盖相同版本的不同字节，不能把普通跨平台 rename 假设为无覆盖原子安装。
- 数据库结果未知时先对账，不误删可能已登记产物；环境提升复用同一不可变 artifact。

#### Expected Outputs

- 原子发布实现、摘要/重试语义、恢复清理策略、发布失败回归和操作说明。
- 实施证据：[phase-05](validation/phase-05.md)、[runtime](validation/phase-05-runtime.json)、[403项最终输入](validation/phase-05-inputs.json)及[升级恢复说明](../deploy/zip-recovery.md)。

#### Acceptance Criteria

- [x] 写入中断时公开版本目录不存在或仍是完整旧内容；并发发布无混合字节。Windows17/Linux16包含真实写进程中断，HTTPS竞争连续观测只出现404或完整胜者；原有资源字节不变。
- [x] 同版本同摘要幂等，异摘要冲突；DB 提交后响应丢失可查明真实状态。PG ZIP6覆盖跨格式/跨环境/并发、实际COMMIT后ACK/回读丢失及保守对账；HTTPS实际body丢失后GET/稳定重试，普通URL旁路409且无行。
- [x] 真实 Nginx 上传→预检→激活→回滚成功，宿主镜像无需重建，旧资源可达。production Rust/隔离PG/正常TLS/Nginx九组通过，宿主镜像与container ID不变，旧v1/v2资源均200且SHA一致。

#### Risks

- 文件系统安装语义与权限差异；旧资源不能在激活后立即删除。

#### Dependencies

Phase 3、4必须COMPLETED。

#### Notes

这是保留ZIP兼容功能的主线阶段，不是镜像运行/URL登记的前置。只有仍需ZIP跨机分发且无共享存储才触发Phase 35；不共享文件的独立镜像无需agent。

2026-10-15实施结果：保留解包限制，增加私有stage、规范manifest/digest、Linux RENAME_NOREPLACE/Windows无覆盖安装、旧产物完整字节核验、共享/独占维护锁及离线SCAN/APPLY。仅新增0002，0001 checksum保持不变；应用+版本绑定不可变产物，跨环境同包提升，未知DB结果保留正式文件，受保护storage提供四态及全部环境引用；ZIP预检/激活/回滚先验证实际字节。Nginx阻止私有metadata与release.json，CI接入明确计数和真实ZIP gate。

最终Windows17/维护3、Linux16/维护3/正式PG27/完整契约99/CI48、真实HTTPS9均PASS且零skip。Linux r5原包装器因先前未生成示例产物的97/2保持FAIL/exit1；相同源码补齐锁定ci/build/sync后完整99通过，正式PG27独立通过，未改写原失败。生产首启发现nullable菜单parent解码问题已最小修复，并由PG重启断言及完整665923行初始化/真实登录补证。两种owner标签容器/网络/卷均空，证书/env与lease已清理；403项最终摘要及294项原生镜像输入零漂移。既有fmt/Clippy/rustdoc与Windows补充symlink权限限制独立披露，未外部发行/生产部署或执行未来阶段。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-6"></a>
### Phase 6: 版本化 SDK 与公开授权契约

#### Status

COMPLETED

#### Goal

给宿主、Vite 微应用及 Rust 消费者提供单一版本来源和可验证的边界契约。

#### Scope

- iframe bridge、context client、通知 helper 的版本产物和类型；context/props/notification DTO 或 schema。
- 统一 credentials:omit、结构化错误、响应限额、requestId 和脱敏；复制产物持续校验。

#### Out of Scope

- 不更换微前端内核，不改变真实授权含义，不共享数据库模型或业务实现。

#### Inputs

- 方案编号：C01、B03；本台账全局约束及源方案对应章节。
- examples/micro-app-sdk、frontend/src/components/micro/types.ts、runtime.ts、工单 vendor/bridge 与 client。

#### Execution Requirements

- 保持context1.0、MicroProps/iframe2.0必需安全能力，未知大版本拒绝；兼容发布清单列出真实版本/能力，不虚构历史。
- SDK主版本内保持兼容；破坏性升级候选窗口90天，具体下线日冻结后记录，缺隔离能力的旧版不自动保留。
- SDK只消费宿主token/epoch/signal，不读取共享存储、不自行刷新；跨页会话由Phase10协调。
- 生成代码/类型有固定入口；当前两份bridge一致是输入事实，不虚构已有漂移。

#### Expected Outputs

- 版本化 classic/ESM/类型产物、schema/DTO、同步与兼容检查、最小框架适配说明。
- 实施证据：[phase-06.md](validation/phase-06.md)、[phase-06-inputs.json](validation/phase-06-inputs.json)、[phase-06-runtime.json](validation/phase-06-runtime.json)；包含实际命令/工具、逐文件摘要、原始失败/回退/兼容与资源核验。

#### Acceptance Criteria

- [x] 宿主和至少两个前端消费同一登记版本，CI检测vendor/生成物漂移。
- [x] 401/403/409/网络/协议错误可区分，无凭据泄漏；未知版本明确拒绝。
- [x] nonce/来源/epoch/取消/组织上限/下载broker回归通过；支持区间内旧静态版本可激活/回滚。
- [x] 浏览器能力检测和SDK支持清单明确，兼容窗口不改变既有安全边界。

#### Risks

- 独立静态旧版本仍依赖旧 SDK；删除兼容能力会破坏回滚。

#### Dependencies

Phase 1、4必须COMPLETED。

#### Notes

本阶段建立版本源；展示更新/授权替换语义由 Phase 8 在宿主、bridge、client 联合实现。

2026-10-15实施结果：首次登记SDK1.0.0仓库候选，保留context1.0、MicroProps/iframe2.0与notification1.0；schema/登记生成公开TS、纯serde Rust DTO、classic/ESM/逐文件清单，统一30目标SYNC/CHECK，CHECK不修复或删除差异。宿主+Vue/jQuery/工单实际消费同版；平台context输出边界消费生成DTO，组织上限缺失/null/list三态保持。安全HTTP强制omit与捕获Bearer、有界流、结构化安全错误/requestId与能力拒绝；原versionless2.0字节保留并真实激活/回滚。独立审查先复现再修复工单旧401跨epoch清除新上下文竞态，保持既有授权含义。

最终完整Linux fast23步（契约169/部署36/CI49/dev19/Rust3、16、3、9、8）、正式PG五套件27、三产品各146/零skip、Windows宿主56/真实iframe1/Rust公开wire9、真实无缓存宿主镜像97资源均PASS。当前472项输入9c967…；PG初轮bce504…仅四个前端/测试路径有差异，相关backend/SDK输入不变；343项native输入d928…零漂移。r1 fast与三browser、r2 optional native遗漏均保持FAIL原记录；同锁r3完整通过，未改依赖掩盖问题。两种owner标签的容器/网络/卷最终6次只读查询全空，lease退出0、镜像保留。既有fmt/Clippy与未测额外浏览器版本明确披露；无外部发行/生产部署或未来阶段实施。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-7"></a>
### Phase 7: 隔离可信微应用永久挂起的故障

#### Status

COMPLETED

#### Goal

一个 TRUSTED 生命周期不结束时，宿主能诊断恢复，隔离 iframe 不被全局 barrier 永久拖住。

#### Scope

- 可信 load/mount/update/unmount 截止时间、故障状态、恢复入口和运行域分离。

#### Out of Scope

- 不声称 Promise.race 能终止原异步或同步循环，不在未清理的可信域继续执行其他可信代码。

#### Inputs

- 方案编号：M01；本台账全局约束及源方案对应章节。
- frontend/src/components/micro/runtime.ts、MicroAppContainer.tsx、现有生命周期/iframe 测试。

#### Execution Requirements

- 保留正常串行生命周期；对不可证明终止的可信执行标记故障并要求受控重载。
- iframe 使用自身可销毁边界，不再等待不相关的可信永久 pending。

#### Expected Outputs

- 运行域/超时/诊断实现和可恢复 UI；永久挂起回归。
- 实施证据：[phase-07.md](validation/phase-07.md)、[473项最终输入](validation/phase-07-inputs.json)及[运行汇总与原始证据SHA](validation/phase-07-runtime.json)已归档，三项验收均PASS。

#### Acceptance Criteria

- [x] mount、update、unmount 各自永不完成时均进入稳定故障状态，token/signal 被取消。
- [x] 后续隔离 iframe 可挂载；另一个 TRUSTED 不会无声绕过安全 barrier。
- [x] 迟到 resolve/reject、快速切组织/应用和正常最终卸载回归通过。

#### Risks

- 可信代码与宿主同执行域，超时只能诊断/隔离复用，不能强制杀死代码。

#### Dependencies

Phase 4、6必须COMPLETED。

#### Notes

默认仍为 IFRAME；不通过把第三方改为 TRUSTED 降低接入成本。

2026-10-15完成：四阶段各10秒执行预算，排队不消耗下一阶段预算；页面共享可信domain保留活动队列和故障，跨组件/同realm模块重求值不解锁，完整document重载才恢复。loader真实执行Promise、取消微任务空隙与开发模块重实例化边界均先复现再修复；截止只作逻辑取消，不能终止原代码或撤回已交付token。首次升级仍运行旧module-only runtime的开发页面须整页刷新。

最终473项输入为4a2f9ce6…，相对起点仅11源码路径变化，73项backend、42项SDK及全部业务消费者/锁文件逐项相同。新增20项及原14项runtime、typecheck/相关eslint、Windows最终r5完整29组、Linux fast23步（契约189/部署36/CI49/dev19/Rust3、16、3、9、8）与Chromium/Chrome/Edge各151组全部通过，零必需失败/跳过。初轮d4验收因必须修复页面domain而受控停止，原FAIL/partial/exit143及先失败回归保留，不虚构完整套件失败数。

最终四轮owner容器/网络/卷为空，lease均退出0，根6项标签空查再次通过；r2自有work缓存已移除，中断r1四个本地npm work目录保留，fast递归删除被自动执行策略以blocked by policy拒绝，没有绕过。新增导航helper只校验测试就绪，原快速选择专项保留，未改Phase9生产导航。未运行本阶段PG/静态镜像、未来阶段或外部发行/生产部署；默认仍为IFRAME。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-8"></a>
### Phase 8: 分离展示更新、授权续期和版本重挂

#### Status

COMPLETED

#### Goal

减少无效取消/重取，同时保持真实身份/凭据替换和撤权的即时保护。

#### Scope

- 宿主、iframe bridge 与工单 client 的变化分类；完整 runtime/entry/version 元组对照。
- 临时网络失败时 fail-closed 的有界恢复。

#### Out of Scope

- 不延长授权 TTL 来掩盖请求成本，不用 contextVersion 代替部署元组或可信凭据。

#### Inputs

- 方案编号：M02；本台账全局约束及源方案对应章节。
- runtime.ts、MicroAppContainer.tsx、iframe-bridge.js、工单 frontend/src/client.mjs 与 app.mjs。

#### Execution Requirements

- 只有同 epoch、同凭据且期限不变的纯展示更新可复用授权 controller。
- 真实 token/身份/授权替换仍中止旧请求；401/403/404、契约或模式错误不被宽泛重试掩盖。

#### Expected Outputs

- 联动的更新协议与适配实现、入口快照处理、安全恢复和回归。
- 实施证据：[phase-08.md](validation/phase-08.md)、[最终输入](validation/phase-08-inputs.json)、[运行索引](validation/phase-08-runtime.json)。473项实际起点保存于`target/phase-08/baseline-inputs.json`，最终478项b6e5ed…/45路径，旧快照不作为最终通过证据。

#### Acceptance Criteria

- [x] 纯主题变化不取消业务提交、不重新请求列表；路由变化只影响相关读取。
- [x] 续期后旧响应不能回写，移除 grant/member 后旧内容不能继续操作。
- [x] profile 与 context 之间激活/回滚竞态时，资源入口和 props 的版本一致。
- [x] 临时 503 可有界恢复，恢复前旧授权不可用；真实失权保持拒绝。

#### Risks

- 仅浏览器取消不能追回已发token或后台已接受写入；后端仍须逐请求校验。无update应用纯主题延后应用主题、路由/授权沿串行重挂fallback；旧静态bridge须新前端版本接入。

#### Dependencies

Phase 6、7必须COMPLETED。

#### Notes

新工单 Vite UI在 Phase 21 继续消费本阶段契约。

2026-10-15完成：依赖6/7已COMPLETED，Git HEAD保持32956c4，保留此前未提交内容。宿主/bridge/client/真实现有工单联动采用带类型完整授权投影、稳定source signal与controller；纯主题保留POST/PATCH/列表与拒绝状态，相关路由读取独立取消，真实续期/撤权清理旧快照并拒绝迟到响应。context为完整runtime/entry/version执行权威，profile只触发一次复核；503/传输白名单以500/1000/2000ms最多3次退避、token+context单尝试5秒/过程20秒恢复，旧授权先失效且不解除Phase7故障锁。SDK1.0.0仍为未发行仓库候选，当前生成物同源同步，wire及实际legacy字节不变。

最终专项51/97与CHECK30、Windows r4完整47/47、Linux fast r2完整23步（契约221/部署36/CI49/dev19/Rust3、16、3、9、8）、三产品r2各169/169均PASS，零必需skip。最终478项b6e5ed…与起点473项4a2f9c…逐项比较45路径变化，78后端、锁、legacy、wire/登记、部署及工具链不变。完整保留旧fixture失败、freeze2完整PASS和browser r1 FAIL/143；owner/lease清理及六项资源空查通过。真实UI使用受控HTTP/Socket.IO fixture，不声称新增DB、OCI/生产HTTPS、外部发行或生产部署验收；后续Phase9仍未开始。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-9"></a>
### Phase 9: 统一菜单查询、路由策略与应用导航

#### Status

COMPLETED

#### Goal

让侧栏、命令面板、工作台、最近访问和路由守卫共享数据与判定，消除晚返回导航和动态路径授权重叠。

#### Scope

- auth query options/key/epoch/signal/失效统一；类型化路由策略、免授权页面和平台根限定。
- 当前应用按需菜单、命令面板打开后加载和应用选择代次。

#### Out of Scope

- 不把现有缓存 key 分裂写成已证实串租户漏洞，不把未知权限默认放行。

#### Inputs

- 方案编号：F01；本台账全局约束及源方案对应章节。
- use-auth-data、use-authorized-paths、RoutePermissionGuard、CommandPalette、DashboardPage、AppLayout、micro-route-permissions。

#### Execution Requirements

- 所有消费者复用同一会话范围；保留全局 cancel/clear 和旧 epoch 拒绝。
- 创建路径避免被 /tickets/:id 读授权匹配；菜单/manifest/校验器同步，不用父路径隐式授权。

#### Expected Outputs

- 查询/路由策略工厂、共享导航用例、非重叠创建路由规范和真实浏览器回归。
- 实施证据：[phase-09.md](validation/phase-09.md)、[最终输入](validation/phase-09-inputs.json)、[运行索引](validation/phase-09-runtime.json)。478项实际起点与只读源码快照保存于`target/phase-09/baseline-inputs.json`及`baseline-source`，不继承Phase8通过结论。

#### Acceptance Criteria

- [x] 同一 app/epoch 共享菜单查询；A→B 快选后 A 晚返回不能导航回 A。
- [x] 入口展示与直达守卫一致；无关应用菜单失败不阻断已明确授权页面，未知授权仍拒绝。
- [x] 只有详情读权限、无创建 MENU 的用户访问创建路径被拒绝；跨组织迟到结果不回显。

#### Risks

- 菜单授权与数据 grant 是两条独立链，统一显示策略不应扩大后台数据权限。

#### Dependencies

Phase 4、6必须COMPLETED。

#### Notes

工单创建路径已冻结为`/micro/work-orders/ticket-create`，详情仍为`/micro/work-orders/tickets/:id`；旧tickets/new只作详情标识，不作创建别名。bootstrap保留7稳定key/UUID，将read/create改为隐藏详情/可见创建MENU；旧BUTTON形状须与新前端版本一起人工审阅采用，工具遇冲突先停止，不自动修改授权。

2026-10-15完成：Phase4/6依赖已完成，HEAD保持32956c4，保留此前全部未提交内容。统一query options/epoch/signal/刷新与失效，保留cancel/clear和旧epoch拒绝；显示/直达策略共用已知权限+精确MENU、个人白名单和平台根限制，未知拒绝，成功授权不受无关菜单错误阻断。rail/工作台/直达共享选择代次；AppLayout统一epoch/path/owner推断，删除微应用外层重复同步并保留全部执行、授权和故障域。

专项22/24/70/16、最终Windows r4完整9套183、Linux fast r1完整23步（契约311/部署36/CI51/dev19/Rust3、16、3、9、8）、三产品r1各183全部PASS，零必需skip。最终493项ca3df485…、39路径含6份既有工具首次纳入；78后端、锁、SDK/wire/legacy、session API/store、根deploy/、Compose与工具链不变，MicroMountZone后续执行代码原字节保持。原始负例、Windows r1真实FAIL、r2输入拒绝、r3旧路径期望FAIL、prefreeze Windows符号链接skip及重建基线限制全部分列保留；owner/lease清理与六项运行资源空查通过。仅受控fixture浏览器及隔离工具验收，无生产DB操作、外部发行或部署，不执行Phase10。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-10"></a>
### Phase 10: 协调多标签页的会话轮换与上下文

#### Status

COMPLETED

#### Goal

两个页面并发刷新、退出或切组织不会误清凭据、覆盖新会话或展示旧组织数据。

#### Scope

- 共享登录、共享当前租户/组织的跨页刷新、登录替换、退出和切换；原子会话记录与版本广播。

#### Out of Scope

- 不顺带引入 BFF/HttpOnly 重构，不悄悄赋予不同标签页独立组织语义。

#### Inputs

- 方案编号：S03；本台账全局约束及源方案对应章节。
- frontend/src/stores/auth.ts、lib/api.ts、main.tsx、tests/session-refresh.mjs、真实多页浏览器。

#### Execution Requirements

- 首期固定共享当前组织语义，不能只换sessionStorage或引入第二个共享token键。
- 使用Web Locks等已验证协调，锁内重读版本/身份后原子保存access/refresh；广播只通知版本变化。
- 切组织各页先取消旧请求、断开旧通知、移除旧组织服务器视图，再重建上下文；草稿按app/tenant/org隔离。
- 首期Chrome/Edge最近两稳定版必测；所需能力缺失时有验证的兼容路径或明确提示，不用固定等待/localStorage猜锁。

#### Expected Outputs

- 跨页协调实现、兼容策略、退出/切换行为说明和多页回归。
- 实施证据：[phase-10.md](validation/phase-10.md)、[最终输入](validation/phase-10-inputs.json)、[运行索引](validation/phase-10-runtime.json)。实际493项起点和只读源码快照见`target/phase-10/baseline-inputs.json`及`baseline-source`，不继承Phase9通过结论。

#### Acceptance Criteria

- [x] 双页同时过期刷新只消费一次旧refresh，失败/迟到页面不覆盖新会话。
- [x] 登录替换、一页退出/切组织、刷新中关闭页面和结果未知均不误删新凭据。
- [x] 双页同步到同一组织，旧请求/通知停止，草稿不被提交或显示到新组织。
- [x] 保留单页刷新单飞、一次重试及旧响应丢弃；能力缺失路径有真实验证。

#### Risks

- 受支持浏览器清单需绑定实际版本；组织切换与刷新同时发生必须按统一会话状态机处理。

#### Dependencies

Phase 4、9必须COMPLETED。

#### Notes

共享登录/组织已成为首期决策。每页独立组织是未来独立会话设计，不属于本阶段默认范围。

2026-10-16完成：依赖4/9已完成，HEAD32956c4及此前未提交内容保留。起点493/ca3→最终499/d52dd7d9…、36变化路径含6新文件，roots/私有排除/依赖范围未扩展。共享登录/组织使用一份原子记录与Web Locks，锁内重读、版本通知、持久intent和unknown隔离；登录替换、退出、切组织和API首await前条件校验统一，旧请求/通知退役，正常token轮换保留真实工单iframe/草稿，切组织旧草稿不显示或提交到B。

专项核心39（新30+原9）、集成21、旧纯模块62、真实多页17及通知10通过；本地60/181/326、政策52；最终Windows build+10套200、Linux fast23（契约362/部署36/政策52/dev19/Rust3/16/3/9/8）、三产品各200共600与Chrome/Edge154各17共34全部PASS，零必需skip。Chrome155.0.8059.84/154.0.8037.92、Edge155.0.4283.60/154.0.4258.62与官方包/工具镜像绑定。原始负例、正常轮换清草稿和通知fallback失败、源漂移与Windows symlink skip均分别保留；六空查/owner/lease退出0，旧34份验证证据未改。未修改后端/SDKwire/锁/工单/根部署工具，不执行Phase11、生产采用或外部发行；旧宿主bundle须重载，回滚旧宿主需重新登录。

必要调用边界补充：同步读取共享记录后，旧闭包捕获的平台Bearer不能当作普通外部Bearer绕过新会话校验。平台API与通知请求显式传递捕获会话条件；MicroAppContainer将平台session-token签发绑定渲染时逻辑会话条件。真实工单回归进一步发现正常平台access轮换会因effect依赖token而重建iframe、丢失同组织草稿，故该effect改按逻辑会话/epoch绑定，issue只在仍同逻辑会话时取当前平台凭据，不随正常轮换重建。其余授权、执行、截止、故障域、生命周期与恢复逻辑保持；credential ticket及应用token不参与平台刷新，不改变SDK/wire或后端协议。容器最终窄差异及原始失败在验证报告逐项保存，不沿用Phase9内层原字节不变结论。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-11"></a>
### Phase 11: 修复普通 OAuth 原子签发与实时失权

#### Status

COMPLETED

#### Goal

普通 OAuth 的一次性凭据、刷新轮换、用户状态和 token 存储形成可验证的状态机。

#### Scope

- 授权码/设备码/refresh 的事务消费与完整 token 对写入；用户型 token 有效性统一。
- 随机 jti、停止新 JWT 原文持久化、兼容迁移和窄 issuance 用例。

#### Out of Scope

- 不重写已经安全的 psr_ 刷新或应用 token 协议，不用用户状态规则误伤 client_credentials。

#### Inputs

- 方案编号：S01、B03；本台账全局约束及源方案对应章节。
- backend/src/routes/oauth2.rs、session.rs、app_token.rs、migrations/0001_init.sql。

#### Execution Requirements

- 锁顺序与会话/撤销一致，条件消费检查受影响行数；新增迁移不改基线。
- userinfo/introspect/refresh 对用户、成员、组织/租户及客户端状态使用明确一致规则。

#### Expected Outputs

- 原子签发与状态校验实现、新增迁移、并发/故障注入/停用回归、协议说明。
- 实施证据：[phase-11.md](validation/phase-11.md)，2026-10-16已创建，本次已按明确指令采纳完成状态。

#### Acceptance Criteria

- [x] 同一普通 refresh 至少20并发兑换仅一成功；重放 invalid_grant。
- [x] 消费或新 token 写入失败时无烧毁凭据/半套 token；授权码设备码保持单次消费。
- [x] 停用/到期/删成员后相关用户 token不能续发或继续返回有效身份。
- [x] 新 token只存摘要/元数据，同秒不同 token可独立撤销；psr_/app token回归通过。

#### Risks

- 存量普通 OAuth 的撤销政策和历史原文清理需要兼容窗口，不能直接清库。

#### Dependencies

Phase 4、6必须COMPLETED。

#### Notes

先冻结身份契约，再进行 Phase 30 的授权路径优化。

2026-10-16完成并于本次明确指令采纳状态。普通OAuth原子签发、共享实时身份与0003存储兼容，consent/password等锁/踢出窗口闭合；Linux PG7组41及fast24全部PASS零skip，Windows默认64PASS/68ignored分列。最终502项162fb670…，12路径变化；旧迁移、psr_/app及Cargo/前端/SDK输入不变。旧WIP fixture FAIL及严格fmt/Clippy既有失败保留。见[Phase11证据](validation/phase-11.md)、[输入](validation/phase-11-inputs.json)、[runtime](validation/phase-11-runtime.json)和[升级协议](oauth-issuance.md)。此前证据中的“原台账未直接修改”描述是执行当时事实，本次用户明确授权后采纳，不改写旧证据摘要。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-12"></a>
### Phase 12: 限制登录内存和密码计算成本

#### Status

COMPLETED

#### Goal

随机账号输入和密码运算压力下内存/计算并发有界，普通 API 不被同步 Argon2 阻塞。

#### Scope

- 登录失败计数容量/TTL/可控时钟；输入长度、来源/全局计算预算；统一异步密码 helper。
- 全请求密码调用链盘点发现联系人管理员初始化也执行hash；将onboarding及tenants/organizations的窄参数传递纳入同一密码服务，保持既有业务与事务，不借此实施Phase13/30。

#### Out of Scope

- 不默认引入 Redis；不通过降低 Argon2/密码策略满足性能目标。

#### Inputs

- 方案编号：S02；本台账全局约束及源方案对应章节。
- routes/auth.rs、users.rs、oauth2.rs、util/password.rs、state.rs、normalize_client_ip。

#### Execution Requirements

- 所有请求路径 verify/hash 在有界 blocking 工作池执行；排队/超时及后台计算资源有明确行为。
- 只信任配置的真实代理，不能让任意 X-Forwarded-For 决定限流身份。

#### Expected Outputs

- 有界计数存储、密码服务/背压、来源策略和容量回归。
- 实施证据：[phase-12.md](validation/phase-12.md)、[输入](validation/phase-12-inputs.json)、[runtime](validation/phase-12-runtime.json)、[边界](validation/phase-12-boundaries.json)，2026-10-16已创建。

#### Acceptance Criteria

- [x] 大量随机用户名/IP 不超配置容器上限，TTL 可回收；时钟跳变可测。
- [x] 并行 Argon2 数和排队上限可断言；过载错误明确，健康/普通请求不被 Tokio 同步计算拖住。
- [x] 多实例限流预算及重启行为写清；需要共享限流时按部署模式实现且测试，不虚称进程内计数是全局配额。

#### Risks

- 电话/email 多账号匹配会放大一次登录计算；多实例部署情况未确定。

#### Dependencies

Phase 4必须COMPLETED。

#### Notes

来源限流、用户维度和全局成本保护互补，不能仅以账号存在与否限制。

2026-10-16完成。来源/input/user共享硬容量和单调TTL，100k随机键/回拨/前跳及副本独立预算/重启清空通过；统一密码服务运行/等待双许可，取消/超时保留在途预算，全HTTP密码路径含联系人初始化接入。真实可信代理右剥离XFF，改密锁内CAS。正式Linux PG8组46（新HTTP5、原41）及fast27全部PASS零skip，Windows86PASS/73ignored分列；严格fmt/Clippy既有38诊断保留，新增文件/config格式通过。最终505项9b016ace…/20路径；旧迁移/Cargo/前端/SDK/OAuth存储不变，psr仅独立包装器测试标记，不改协议。WIP4PASS/1越界测试预期FAIL及修正、所有owner/lease清理保留，见phase-12证据。无Phase13或生产操作。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-13"></a>
### Phase 13: 使关键管理变更与审计同事务

#### Status

COMPLETED

#### Goal

成功的关键身份/授权/发布管理变更有可靠审计，失败时不会出现业务成功但审计丢失。

#### Scope

- 事务审计接口、关键角色/成员/状态/密码/客户端操作的接入和最小前后摘要。
- 完整调用链包括 roles/users/tenants/organizations/auth 的既有安全变更，menus/departments/application 管理，OAuth consent/device/revoke 与 session-token 持久签发，以及平台设置。新增接入仅把现有变更和成功审计置于同一事务，不扩展授权、撤销或 token 协议。
- 通知 publish、release register/adopt/preflight/activate/rollback 复用严格事务审计，保持既有幂等、独立预检提交和未知 commit 对账边界。普通登录成功/失败、个人通知偏好/已读等诊断维持原分类。
- 删除回归发现既有组织树/角色复合外键与单列级联的检查顺序冲突；组织和租户删除仅在主事务内暂缓两个已有 DEFERRABLE 检查，并在提交前恢复 IMMEDIATE。保留迁移校验和和最终完整性约束，不通过提前清成员绕过级联。

#### Out of Scope

- 普通诊断访问日志仍可 best-effort；不引入外部消息总线或记录敏感正文。

#### Inputs

- 方案编号：B02；本台账全局约束及源方案对应章节。
- util/audit.rs、roles/users/tenants/organizations/oauth_admin 路由、已有通知/发布事务实践。

#### Execution Requirements

- 审计在主事务内写入；外发如有需求先事务 outbox，不能在 commit 后补写承诺原子。
- 保留requestId、必要身份快照及失败计数，删除后仍能追踪对象。

#### Expected Outputs

- 事务审计实现、关键操作清单、失败回滚/重试回归及文档。
- 实施证据：[phase-13.md](validation/phase-13.md)、[输入冻结](validation/phase-13-inputs.json)、[范围核验](validation/phase-13-boundaries.json)、[运行索引](validation/phase-13-runtime.json)、[已采纳台账更新](validation/phase-13-ledger-update.md)。

#### Acceptance Criteria

- [x] 故障注入审计插入失败时，关键管理变更回滚或存在设计中可靠事务事件。
- [x] 成功操作恰有可追踪审计，无 token/secret；通知/发布现有事务语义不退化。

#### Risks

- 不能把池写函数简单挪到事务后就视作原子；删除对象的名称需在变更前取快照。

#### Dependencies

Phase 4、11必须COMPLETED。

#### Notes

后续 grant API 和 Rust CLI 使用本阶段的审计规则。

2026-10-16：用户明确执行Phase13；已核对Phase4/11依赖完成，采用现有审计表和事务，不新增迁移/依赖。审计静默跳过也按失败回滚；requestId只用于追踪，不作为全局幂等键。无实质变更的重试不增加相同变更事件，已有幂等操作保持原约定。

首轮管理审计WIP为7 PASS/1 FAIL，组织删除400已由独立隔离SQL确认组合外键顺序并作事务内窄修；原结果保留见phase-13。正式门禁保留原46项、新管理审计8项及既有通知/成员10项，PG十组64与fast27已在同一506项冻结快照全部通过。

实际结果：506项最终输入 SHA-256 `582d2759e38c06f62ea609bb9b0debfe634e3415cde6617cb604f401bccec9ab`；PostgreSQL十组64项（新增管理审计8项，补入既有通知10项）与完整fast27步同快照PASS、必需零skip。审计SQL异常/静默RETURN NULL都回滚，成功requestId/删除名称/安全摘要/失败计数与原通知/发布边界均有实证。完整strict Clippy/fmt既有FAIL与Windows默认ignored分别记录，详见phase-13。无剩余必需验收项，未实施Phase14。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-14"></a>
### Phase 14: 提供受控数据 grant 管理 API

#### Status

COMPLETED

#### Goal

平台通过正式授权用例管理数据 grants，业务团队不必把直接写平台 SQL 作为日常接入方式。

#### Scope

- direct membership/app/resource/action/scope的受控查询与差量写入、独立管理权限、审计/失效。
- 首版平台根上下文且持独立写权限才能修改；只读管理范围明确校验。

#### Out of Scope

- 不给组织管理员隐式写权，不默认授TENANT，不接受任意资源/动作，不实现显式委派/递归转授/通用审批引擎。

#### Inputs

- 方案编号：O01、X01、B03；本台账全局约束及源方案对应章节。
- membership_data_grants、app_context.rs、authorization.rs、工单 grants 工具和既有管理权限。

#### Execution Requirements

- 在现有权限注册规范内冻结独立grant读/写编码；用户/角色/菜单管理权不自动等价。
- 写入重新检查平台根上下文+独立权限、目标有效direct membership、已登记应用资源/动作及scope/组织上限。
- 管理员自身业务grant不等于可授上限，scope不能按枚举排序；未来委派cap单独建模。
- 差量维护保留UUID，必要迁移只新增；变更预览、冲突检查、同事务审计与即时授权失效。

#### Expected Outputs

- typed API/用例/存储、新增必要迁移、契约与正负向隔离测试。
- 实施证据：[Phase 14验证记录](validation/phase-14.md)；[输入快照](validation/phase-14-inputs.json)、[范围核对](validation/phase-14-boundaries.json)、[运行索引](validation/phase-14-runtime.json)。

#### Acceptance Criteria

- [x] 平台根缺独立权限、组织管理员直接构造写请求、失效成员均拒绝；membership与目标tenant/org不一致、资源动作不属于目标app、目标越出显式管理范围均拒绝。合法平台根可管理被明确允许的业务租户，不能仅因操作者处于平台租户而误拒绝。
- [x] 合法平台根操作者可在受控范围修改；普通业务用户查成员/执行业务无需grant管理权。
- [x] 未修改成员UUID保持，增删grant影响下一受保护新请求；读API不越只读管理范围。
- [x] 审计失败使写入回滚，不能用直写业务库或SQL脚本代替正式管理API。

#### Risks

- 根管理上下文不能绕过目标业务边界；最终权限编码由本阶段契约冻结，不复用宽泛菜单权。

#### Dependencies

Phase 6、13必须COMPLETED。

#### Notes

本阶段和15是工单/门店完整授权闭环的必要前置。显式委派仅保留设计边界，不作为首版完成项。

已完成受控API与全部必需验收：隔离PostgreSQL 11组71项、fast 28步/542项测试通过，必需零skip；Windows默认95通过/88 ignored单独列明。511项冻结输入 b48d12194b9c888844ed493fd8fbedd5967ac4f2c5e7c6260092b540922ec1b7；旧迁移/依赖/SDK/业务授权路径保持。严格fmt/Clippy仍有既有债务（38个test-target错误），诊断文件/消息多重集与Phase13相同，无新增诊断。

首版契约冻结：`system:data-grant:read` / `system:data-grant:write`独立BUTTON权限；服务端严格校验的`DATA_GRANT_POLICY`按应用UUID登记resource/action/可选scope，并显式列出tenant/org根及是否含子树的read/write正向管理范围，默认空拒绝，重叠范围取并集。读取还受操作者实际组织子树限制；写入与预览必须实际平台根+独立写权，预览同时要求读权。策略由受控服务配置管理，首版不增加委派/审批或策略写API。GET查询目标5个UUID；POST预览/PATCH差量含expectedVersion与upsert/remove，锁后重新检查，未修改和仅scope更新的grant UUID保留。存储scope与有效组织硬边界分别返回；不改旧迁移、不新增通用业务scope。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-15"></a>
### Phase 15: 实现数据 grant 管理界面与授权闭环

#### Status

COMPLETED

#### Goal

管理员能通过受控界面完成普通工单用户的功能授权和独立数据授权。

#### Scope

- membership/app/resource/action/scope 选择、当前范围显示、差量提交、冲突/拒绝提示与审计查看。

#### Out of Scope

- 不扩大 Phase 14 API 的管理边界，不用直接 SQL 替代页面保存，不自动授权所有人。

#### Inputs

- 方案编号：O01、X01；本台账全局约束及源方案对应章节。
- Phase 14 DTO与接口、现有 Authorizations/Users 页面、工单 bootstrap/grants manifest。

#### Execution Requirements

- UI表达独立grant管理权限与实际平台根上下文；无写权只读，不展示可提交编辑，服务端仍强制拒绝。
- 下拉项由可管理应用/成员/能力生成，无权项不预选；变更预览、冲突、审计、失效结果明确。
- 菜单/按钮/数据范围分别表达，空grant默认拒绝；工单和门店资源命名不混用。

#### Expected Outputs

- 管理页面/API封装、普通用户授权操作说明、UI/浏览器负向回归。
- 实施证据：[Phase 15验证记录](validation/phase-15.md)、[输入快照](validation/phase-15-inputs.json)、[范围核对](validation/phase-15-boundaries.json)、[运行索引](validation/phase-15-runtime.json)。

#### Acceptance Criteria

- [x] 平台根有独立权限可配置/撤销指定有效成员SELF/ORG等允许范围，不改平台DB。
- [x] 组织管理员无写入口，手改请求仍403；越界目标、失效成员、未登记动作拒绝。
- [x] 保存后普通用户业务请求按新范围生效，审计可关联请求，无完整审批引擎依赖。

#### Risks

- 将范围描述成角色继承会误导；data grant绑定当前直接成员而非祖先角色。

#### Dependencies

Phase 9、14必须COMPLETED。

#### Notes

具体页面可复用现有界面，禁止无关的大范围视觉重构。

已完成独立数据授权页面、窄发现、预览/差量保存、冲突/拒绝/未知结果恢复和审计关联。r2完整PG12组74项、fast28步568项通过；Chrome/Edge155各200项fixture通过，五个产品/版本共80项真实授权检查和5次Rust harness通过，工单独立业务库同token实证SELF1/ORG2/撤销403及四次审计关联。r2 Chromium fixture为196通过/1失败/3未运行，原始FAIL保留；唯一既有主题测试同步竞态已修复，在当前三个产品各补验完整设置11项全部通过。补验前后538项中仅1个测试文件变化，其余537项逐字节不变，不能称为全矩阵重跑或一次600项通过。最终输入 7a7df9f10b03a7dda2dd0caec646f5ada16eef57245c7193ab0db8fd3ffe37b4；原r2输入 9fc2f8d68d46093746b874a40a7d2fc84fec5fec91ee5a52a6d275546e5287c7。Windows默认96通过/92 ignored单列，全仓strict fmt/Clippy既有失败保留，Clippy相对Phase14无新增诊断。全部owned资源清理及WSL lease退出通过。

实施契约：新增独立`/system/data-grants`页面及read MENU证据，复用Phase14独立读写权限，写界面还要求服务端明确实际平台根与目标canWrite。新增窄只读`GET /api/data-grants/options`以同一Phase14读权限/显式策略/有效direct membership/应用授权边界发现应用与分页成员，不授予全局用户/应用目录权、不扩大管理范围。既有查询/预览/PATCH形状不变；共用会话fence客户端补PATCH与成功requestId元数据，精确预览后提交、冲突重新读取、未知结果先对账、切目标/上下文/失权丢弃旧草稿。审计查看仍独立system:audit。Users/Authorizations仅加可访问链接，功能菜单/按钮与数据grant分别表达。无新迁移、委派/审批或Phase16实施。

验收采用真实Vite→平台HTTP→隔离PostgreSQL浏览器闭环；再由现有工单服务连接独立隔离业务库，通过真实客户端内省/应用context执行同一普通用户token的工单列表，核对SELF本人、ORG同组织及撤销后的拒绝，并关联requestId审计。不修改工单生产源码、不提前迁移Rust工单；现有浏览器fixture矩阵保持其UI-only语义。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-16"></a>
### Phase 16: 建立 Rust 工单服务并接管业务库基线

#### Status

COMPLETED

#### Goal

独立 Rust 工单进程能安全初始化空库或接管现有 SHA-256 迁移库，先不重写全部业务。

#### Scope

- 独立Cargo crate、main/lib/config/errors、SQLx pool/migrations、live/ready与最小HTTP骨架。
- 旧 work_order_migrations 校验和/结构验证及迁移记录接管。

#### Out of Scope

- 不把工单表并入平台库，不删除旧库/改0001，不在本阶段实现完整CRUD/UI。

#### Inputs

- 方案编号：T01、X01、O01；本台账全局约束及源方案对应章节。
- business-apps/work-orders/backend/db.mjs、config.mjs、migrations/0001_work_orders.sql、现有API/错误契约。

#### Execution Requirements

- 保持计数器、UUID、归属、version和不可变身份trigger；并发启动接管有锁。
- 历史迁移SHA256体系与SQLx记录不同，显式设计一次性接管，不直接重跑CREATE。

#### Expected Outputs

- Rust服务骨架、配置模板、基线接管实现/运维说明、隔离DB夹具。
- 实施证据：[Phase 16验证记录](validation/phase-16.md)、[验证输入快照](validation/phase-16-inputs.json)、[范围核对](validation/phase-16-boundaries.json)、[运行索引](validation/phase-16-runtime.json)、[已采纳台账更新](validation/phase-16-ledger-update.md)。

#### Acceptance Criteria

- [x] 空库首启、同库再启、已有迁移记录与工单数据的首启、两个实例并发启动分别通过。
- [x] 结构/索引/trigger/checksum不符时停止，迁移失败不开放ready；计数器和历史工单不变。
- [x] Rust二进制独立启动，运行环境无Node依赖。

#### Risks

- 现有旧数据及迁移记录不匹配时必须报告，不能自动伪造SQLx校验记录。

#### Dependencies

Phase 1、4、6必须COMPLETED。

#### Notes

Node旧API只是兼容输入，最终切流前保留可比对证据，不新增Node运行路线。

已按用户明确请求完成，仅执行Phase16。独立Cargo crate位于现有`business-apps/work-orders/backend`，与保留的Node兼容文件共存，不新增根workspace、不依赖平台实现crate。迁移采用与旧Node相同的事务advisory锁、不可变旧SQL/SHA256与SQLx SHA384记录；在同事务随机临时验证schema中构建原始DDL并比较真实catalog后，才允许接管。迁移账号需要独立业务库的建schema/表/函数权限；临时验证schema在提交前删除，失败全部回滚。空库、旧库、并发、篡改/失败、数据保持和无Node二进制分别真实验收；完整业务端点、平台授权及切流不在本阶段。

已完成全部三项必需验收。604项验证输入`ab0a4b25c8618a53f9ef46ec53a8d37fddbb154ff777abaf1171533519260ebd`；Linux fast 40步、668项测试通过，平台独立PG12组74项、工单独立PG15项均零失败/零跳过；直接Linux ELF在无Node的Debian环境以UID10001完成11项运行检查。Windows fmt/全目标strict Clippy、13项单元与1项doctest通过；Linux另实际执行1项doctest。空库/再启/旧库/双实例及Node-Rust同锁竞争通过；结构、索引、约束、函数、trigger与迁移历史漂移拒绝，启动失败不开放ready，旧工单/version/计数器和迁移时间保持。全部owned库/pool/容器/网络清理与WSL lease退出核验通过。

旧SQL SHA256及17项Node/迁移可执行输入逐字节保持。运行容器为用于传输原始ELF的writable rootfs，不宣称生产镜像或read-only rootfs验收；迁移角色需独立业务库CREATE权限。独立锁文件复用依赖家族，有20个精确包版本不同于平台冻结锁；221项license声明已归档，不声称完整安全/许可审计。早轮r1源漂移拒绝、r2的14通过/1失败与driver清理保护修正均保留；最终代码重新验收通过。其他聊天并发修改只记录边界、不计作本阶段产出；CI README的一处纯文档计数更正与外部漂移分别记录，Phase16可执行输入逐字节保持。完整业务API、平台授权及切流仍未由本阶段实施。

状态与证据已同步；本次执行在Phase16终止，下一Phase17须新的明确请求。

---

<a id="phase-17"></a>
### Phase 17: 实现 Rust 工单授权及只读/导出契约

#### Status

COMPLETED

#### Goal

Rust API能以当前平台契约授权并在真实业务库执行一致的数据范围读取。

#### Scope

- bounded reqwest platform_client：Basic内省+应用Bearer context；scope policy/SQL。
- 列表/count/detail/筛选/分页、CSV导出、requestId、Origin:null CORS与错误映射；Rust app-backend参考。
- 工单分页边界、参数化搜索与代表数据EXPLAIN；按证据选择必要索引，不默认全文检索或分区。

#### Out of Scope

- 不删除内省、不跨请求缓存正向授权，不实现写入或提醒。

#### Inputs

- 方案编号：T01、X01、B01；本台账全局约束及源方案对应章节。
- 工单 auth.mjs、server.mjs只读/CSV、examples/app-backend/authorize.mjs、平台introspect/context契约。

#### Execution Requirements

- 检查aud/active/用户租户组织/到期与上下文一致，保留组织硬上限和grant身份绑定。
- 列表、count、详情、导出同scope；CSV保留公式注入处理、UTF-8/BOM、1000行/4MiB等当前明确限制。
- 后台参考代码迁为Rust或明确退役，不能继续推荐Node授权后台。

#### Expected Outputs

- Rust授权客户端/DTO/policy/只读与CSV routes、参考接入和数据库/HTTP回归。
- 实施证据：[验证报告](validation/phase-17.md)、[输入清单](validation/phase-17-inputs.json)、[边界](validation/phase-17-boundaries.json)、[运行与归档索引](validation/phase-17-runtime.json)、[查询计划](validation/phase-17-query-plans.json)及[台账更新记录](validation/phase-17-ledger-update.md)。

#### Acceptance Criteria

- [x] SELF/ORG/ORG_SUBTREE/TENANT及组织上限、其他租户、上级平级、无grant、错误受众均真实验证。
- [x] 平台不可达503、失效401、缺授权403、不可见404；客户端secret不进前端/错误。
- [x] CSV范围与列表一致，注入/容量/编码回归通过；OPTIONS及错误响应CORS正确。
- [x] 极大page稳定拒绝/受限而非溢出；列表/count/导出同范围，计划及读取成本有可复核基线。

#### Risks

- 只有本地解JWT或仅看HTTP200会弱化当前授权；CORS null本身不等于可信身份。

#### Dependencies

Phase 4、6、11、16必须COMPLETED。

#### Notes

授权调用合并与快照优化留给 Phase 30，先证明语言迁移契约等价。

2026-10-17完成并采纳Phase17。独立Rust工单API逐请求Basic内省→Bearer context，active/aud/身份/有效期/contractVersion与grant绑定严格验证；授权默认总3秒、并发32即时拒绝、响应256KiB上限，无正向授权缓存。所有读取叠加tenant/软删除/组织硬上限与独立scope；SELF仅owner，列表/count及导出容量/rows各用同一只读REPEATABLE READ快照，查询/编码后复验到期。page≤1,000,000且checked offset，CSV1000行/精确4MiB、BOM/CRLF/公式中和，CORS与requestId覆盖错误及预检。写入/assignees/remind仍拒绝，不改旧0001、不切业务流量。Node后台授权参考明确退役，活动文档指向实际Rust模块。

受测候选为674项`59b0f8d12db41b566e42f59d0aeba38f2afe72c547a18ca3501dabb53a654028`，23项独占执行输入与当前宿主等价、17项旧Node/SQL保持；第三方依赖原221项身份全部保持，增加32项。正式PG9步PASS：平台12组74、迁移15、受控HTTP+业务PG13、真实平台双库1 harness/13 named；额外Linux lib34/doctest1/build通过。PG原外层因共享宿主CI变化FAIL原样保留，独立补验同一ELF的无Node11项通过。完整fast实际50步48PASS/2FAIL，报告计数700PASS/1FAIL/0skip；失败是并行Phase25未收录fixture和格式问题。本阶段工单5步、CI策略63及原日志相关契约40均PASS，范围内门禁按派生报告审计；两项外部失败又以不同当前宿主字节独立补验（contract5/5与fmt）通过，不改写原完整fastFAIL。资源空查/lease0及归档hash见报告和运行索引。代表24,008行8份EXPLAIN归档，无新索引，不作生产SLA。

Windows早轮网络与路由失败保留，路由已修且Linux全库补齐；工单crate严格Clippy通过，当前宿主平台全目标严格Clippy53诊断（本阶段harness0）单列，不能冒充674候选或全仓无诊断。共享CI及其他聊天源码变化单独记录，当前全树不继承候选通过。四项验收与本阶段必需门禁均PASS，完整fast及平台strict未被宣称全绿；下一Phase18 NOT_STARTED，需新的明确请求。

---

<a id="phase-18"></a>
### Phase 18: 实现 Rust 工单写入、幂等和短事务

#### Status

COMPLETED

#### Goal

Rust创建/编辑/状态流/软删除保持现有业务契约，未知结果重试不会重复创建。

#### Scope

- 写入DTO/字段白名单、版本冲突、状态转换、不可变归属、创建幂等和请求总deadline。

#### Out of Scope

- 不在行锁中请求平台目录，不把提醒幂等当创建幂等，不自动重试非幂等写入。

#### Inputs

- 方案编号：T01、X01、M03、B01；本台账全局约束及源方案对应章节。
- Node工单写路由、业务DDL、Phase 17授权、新增幂等键/正文摘要契约。

#### Execution Requirements

- 幂等唯一域tenant/org/user/action/key；工单与结果同事务，异正文409。
- 锁定前完成外部预校验，锁内核对记录版本/组织/范围/授权期限；处理中止不代表未提交。
- 成员fixture只限测试注入；Phase19前运行路径必须拒绝未经平台验证的非空assignee，不得以允许所有成员的stub启动或切流。

#### Expected Outputs

- Rust写入用例/新增幂等迁移、兼容DTO、冲突与未知结果恢复回归。
- 实施证据：[验证报告](validation/phase-18.md)、[输入](validation/phase-18-inputs.json)、[边界](validation/phase-18-boundaries.json)、[运行及归档索引](validation/phase-18-runtime.json)与[台账更新记录](validation/phase-18-ledger-update.md)。

#### Acceptance Criteria

- [x] 同键同正文并发只一条记录，commit后丢响应按原键恢复；异正文409。
- [x] 新组织/已失权重试不能泄漏旧结果；版本竞争仅一方成功，删除后列表详情导出不可见。
- [x] DB锁/连接不随外部HTTP长等；总deadline与语句/锁预算可验证。
- [x] 未接入Phase19真实成员校验时，非空assignee明确拒绝；无分配/错误成员不会借测试替身绕过授权。

#### Risks

- 结果保存和创建必须原子；新版本迁移不可改旧0001或破坏现有编号。

#### Dependencies

Phase 6、13、17必须COMPLETED。

#### Notes

真实成员分配依赖Phase19；fixture只在测试隔离层注入，任何运行/切流路径均不得放行未验证成员。

2026-10-17完成并采纳Phase18。Rust创建保持严格兼容DTO，必需Idempotency-Key请求头；验证后title trim/description默认空/priority默认NORMAL/assignee省略与null归一，以固定DTO字段序列计算SHA-256。tenant/org/user/create/key receipt、工单、编号计数器及原201结果同事务；重放重新取得当前create授权，同键同正文返回创建历史，异正文409。后续编辑/软删不改原receipt，重放不代表当前详情；新组织/失权不能读出旧域结果。

受测输入691项`9bea11c1855bdea5126fec3503e6b22d69e87a0d00a31f5f86b4187e8825ffec`。正式运行索引状态PHASE18_GATES_PASS；platform-postgres 74PASS、work-orders-postgres 21PASS、work-orders-read-postgres 13PASS、work-orders-write-postgres 14PASS、work-orders-live-postgres 1PASS/named13、work-orders-write-live-postgres 1PASS/named12；Linux lib43/doc1、同源runtime12项；完整fast子级状态PASS，52步/52PASS/0FAIL，测试745PASS/0FAIL/0skip。完整fast的PASS仅属于索引绑定的实际候选与本次执行，不外推到当前全部宿主字节。原外层状态FAIL及原因“wsl.exe command failed (exit=1, timeout=false): Error response from daemon: invalid filter 'saas.phase18.owner'”保留，不能把子套件通过改写为外层通过。来源、清理及具体命令均引用正式报告/边界/runtime索引，不借用历史测试或预期数量。

PATCH/DELETE保留版本竞争、状态流与软删除；平台授权HTTP先于业务pool/事务/行锁，锁内只做本地范围/版本/期限校验。15秒请求总截止及各DB预算独立；commit前expiry401回滚，COMMIT错误、提交后到期或写截止返回CREATE_RESULT_UNKNOWN/WRITE_RESULT_UNKNOWN503，分别同key/正文恢复或读回当前版本，不自动重试非幂等写入。所有显式非null assignee（含已有同值）在Phase19前400；update省略保留、null清除仍受组织范围约束，无运行成员stub。

新增0002严格支持合法空库/Node1/SQLx1/Node已2双历史过渡，新增Upgraded结果，旧0001字节不改；不新增crate或依赖、不接入成员目录/提醒、不改前端提交恢复、不切流或部署。四项验收及本阶段范围必需门禁已PASS；当前宿主共享CI、其他聊天工作及平台strict等限制按正式证据原样保留。下一Phase19 NOT_STARTED，需新的明确请求。

状态与正式证据已同步；本次在Phase18终止，Phase19须新的明确执行请求。

---

<a id="phase-19"></a>
### Phase 19: 迁移 Rust 工单成员目录与分配校验

#### Status

COMPLETED

#### Goal

Rust业务API通过受控平台成员接口查询/验证处理人，且不扩大工单数据范围。

#### Scope

- assignees搜索分页、单成员验证、创建/编辑分配流程；平台成员用途的窄能力声明。

#### Out of Scope

- 不直接查平台成员表，不因处理人相同扩大SELF，不构造无第二消费者的通用业务框架。

#### Inputs

- 方案编号：T01、X01、B03；本台账全局约束及源方案对应章节。
- 工单members/assignees契约、平台routes/app_members.rs、Phase 14能力与grant规则。

#### Execution Requirements

- 目标组织在授权硬上限内，有效直接成员；成员预校验在工单行锁之前。
- 平台去掉散布的ticket特判，以登记能力/策略约束用途，未声明resource/action拒绝。

#### Expected Outputs

- Rust目录客户端/route及分配校验、平台窄策略调整、同步DTO和负向回归。
- 实施证据：[验证报告](validation/phase-19.md)、[输入](validation/phase-19-inputs.json)、[边界](validation/phase-19-boundaries.json)、[运行及归档索引](validation/phase-19-runtime.json)与[台账更新记录](validation/phase-19-ledger-update.md)。

#### Acceptance Criteria

- [x] 搜索/分页保持既有契约；禁用/越界/无成员或缺分配授权不能作为处理人。
- [x] 成员接口超时不拉长同工单行锁；记录预读/锁定组织不匹配拒绝。
- [x] SELF仍按owner及组织上限执行，分配关系不带来额外数据访问。

#### Risks

- 平台与业务库没有跨库原子事务，必须用短有效期断言及版本检查明确竞态界限。

#### Dependencies

Phase 14、17、18必须COMPLETED。

#### Notes

本阶段只完成工单消费者和平台窄内部策略。Phase26增加门店真实策略/调用，Phase27实证共用成员HTTP契约，再由Phase 30最终收敛公共能力；不提前宣称通用跨应用模块完成。

2026-10-17完成并采纳Phase19。Rust通过当前普通应用Bearer调用真实平台成员HTTP，保持assignees搜索/分页和最小DTO；原始q先限100个Unicode字符再trim，page最多1000000、size最多50。平台DATA_GRANT_POLICY按同应用memberLookups显式登记成员用途，默认空，最多128个唯一且已登记的resource/action；去掉生产路径ticket特判，公开capabilities仍只有resource/action/scopes，不新增成员grant或assign动作，不要求业务用户具有grant管理权。有效直接成员、当前permission/data grant和组织硬上限同时生效，SELF始终按owner，不因处理人扩权。

受测输入700项`52ebb7a5904b82fa94f5f16b69e9cad924b8afcea4ae60df75fa1423a101741e`。正式运行索引状态PHASE19_GATES_PASS；platform-postgres 76PASS、work-orders-postgres 21PASS、work-orders-read-postgres 13PASS、work-orders-write-postgres 14PASS、work-orders-member-postgres 12PASS、work-orders-live-postgres 1PASS/named13、work-orders-write-live-postgres 1PASS/named12、work-orders-member-live-postgres 1PASS/named10；Linux lib50/doc1、同源runtime12项；完整fast子级状态PASS，53步/53PASS/0FAIL，测试765PASS/0FAIL/0skip。完整fast的PASS仅属于索引绑定的实际候选与本次执行，不外推到当前全部宿主字节。原外层状态FAIL及原因“The r1 immutable candidate was stopped at root request after a new platform Clippy lint was found; its original cancellation, host drift guard and cleanup results are retained independently.”保留，不能把子套件通过改写为外层通过。来源、清理及具体命令均引用正式报告/边界/runtime索引，不借用历史测试或预期数量。

显式分配更新按业务scope非锁预读组织/version/oldAssignee，释放连接后做必要成员HTTP，再在行锁内复核三项，错配VERSION_CONFLICT409；外部HTTP不持业务连接或行锁。省略/同UUID保留旧分配与名称快照，不重查旧成员；真正change/clear受目录组织scope限制，非空change验证新成员。成员校验从HTTP发起时按单调时钟计min(配置超时,3秒)，响应/DB等待消耗原窗口且不续期；HTTP超时PLATFORM_UNAVAILABLE503，提交前成员结果过期MEMBER_VALIDATION_EXPIRED503并回滚；提交后context/member到期仍返回CREATE_RESULT_UNKNOWN/WRITE_RESULT_UNKNOWN503，按原恢复规则对账。平台库与业务库之间仍有最后验证到COMMIT的撤权/成员失效竞态，不承诺跨库原子性。

创建非空UUID规范化进hash，已提交同key/hash的原201 receipt仍需当前create授权但不重查历史member；0001/0002、receipt、schema、crate与依赖不变。退回原Phase18会恢复目录503/显式非null分配400，带非空assignee的历史创建正文也不能重放；保留分配与receipt，先对账在途结果或采用兼容构建。平台降至旧strict parser还须处理memberLookups配置且旧边界不等价。提醒仍503、前端创建header/恢复未适配、旧Node完整实现保留、不接入门店/最终通用框架、不切流或部署。三项验收及本阶段范围必需门禁已PASS；其他聊天成果、共享CI/平台strict等限制按正式证据保持。下一Phase20 NOT_STARTED，需新的明确请求。

状态与正式证据已同步；本次在Phase19终止，Phase20须新的明确执行请求。

---

<a id="phase-20"></a>
### Phase 20: 迁移 Rust 工单提醒到平台通知API

#### Status

COMPLETED

#### Goal

Rust工单后台能以用户应用身份发布真实、幂等、范围受控的提醒。

#### Scope

- 独立POST提醒、收件人选择、ticket读取与notification/publish授权、平台返回校验。

#### Out of Scope

- 不重复保存工单、不直接写平台通知表、不自行广播未验证消息。

#### Inputs

- 方案编号：T01、X01；本台账全局约束及源方案对应章节。
- server.mjs提醒路由、平台notifications API、发布幂等与收件契约。

#### Execution Requirements

- 先验证读工单及通知操作/data grant，使用当前应用Bearer调用平台。
- 结果未知保留同事件键/正文；收件人变化同键冲突，由用户明确新事件。

#### Expected Outputs

- Rust提醒用例/route、幂等与失权/变更回归和接入文档。
- 实施证据：[验证报告](validation/phase-20.md)、[输入](validation/phase-20-inputs.json)、[边界](validation/phase-20-boundaries.json)、[运行及归档索引](validation/phase-20-runtime.json)与[台账更新记录](validation/phase-20-ledger-update.md)。

#### Acceptance Criteria

- [x] 同事件重试只一条持久通知；工单version不因提醒改变。
- [x] 无读权限/发布权限/数据grant、越界收件人均拒绝；未知结果不自动换键重发。
- [x] 平台收件箱可核对结果，实时服务未就绪不被误判为持久发布失败。

#### Risks

- 在线提示与DB已提交是不同结果；不能为了补Toast再次发业务通知。

#### Dependencies

Phase 17、18、19必须COMPLETED。

#### Notes

完整Socket.IO联调由 Phase 23/34 进行，本阶段验证平台HTTP持久发布契约。

2026-10-17完成并采纳Phase20。Rust手动提醒保持POST /tickets/{id}/remind与正文{idempotencyKey}，以同一当前应用身份的Basic内省及Bearer context分别验证ticket/read与notification/publish功能权限、直接grant和组织硬上限，SELF读取仍按owner；目标为处理人或owner、工单组织，固定INFO/编号文本与详情链接。仅通过官方平台HTTP持久发布，不写平台通知表、不改工单version、不做成员预查阻断历史回执恢复，也不依赖实时服务在线。

0001/0002原字节保持，新增0003不可变提醒意图。平台对应tenant/publisher组织/application UUID/user/key五元域固定ticket、目标、完整正文及摘要；短事务按scope对工单FOR SHARE后claim，COMMIT明确成功且释放连接/锁后才POST。当前目标、工单或正文与已存意图不同同键409，即便原上游未提交；已存意图不更新、不级联删、不自动清理，没有本地成功缓存或自动发送队列。每次恢复仍要求当前read/publish权限、grant及目标范围；平台持久回执保持权威。

本地准备COMMIT或发前总期限未确认返回REMINDER_PREPARATION_UNKNOWN503，本次未向平台发送；若下一次确无已提交意图，可按当前快照首次claim，未落库的信息不能被承诺跨故障永久保存。已有意图后，平台POST超时、5xx、畸形成功响应、断流或发送后context过期返回REMINDER_RESULT_UNKNOWN503，保留原key/完整正文，不自动换键、重定向正文或重发。明确400/401/403/404/409分别保留确定拒绝，其他非200保守未知；成功严格投影publicationId/两计数1/duplicate/RFC3339 createdAt。

受测输入711项`4f9a2cb80ece51c86e07a86bdafb98ca765fc2498a30ae5f64e7ce08cec3f449`，正式PHASE20_GATES_PASS；platform-postgres 76PASS、work-orders-postgres 27PASS、work-orders-read-postgres 13PASS、work-orders-write-postgres 14PASS、work-orders-member-postgres 12PASS、work-orders-reminder-postgres 12PASS、work-orders-live-postgres 1PASS/named13、work-orders-write-live-postgres 1PASS/named12、work-orders-member-live-postgres 1PASS/named10、work-orders-reminder-live-postgres 1PASS/named12；Linux lib56/doc1、同源runtime12项；完整fast 53/53步PASS、778PASS/0FAIL/0skip。最终只采用索引绑定的同一候选；开发观察、readiness及任何未采纳原始失败按实际来源独立保留。完整PG未选notification-live，本阶段证明持久发布与真实收件箱，不能代替Socket.IO/浏览器/生产验收；运行资源清理及期限/意图/恢复证明见正式报告与归档。

平台与业务库无原子事务，意图确认后工单仍可能改变或删除；平台200后不再读票把已提交误报未发布。实时离线或Toast缺失不能触发第二条通知，删除收件箱后同事件重试不复活。原Phase19严格基线不认识0003，旧Node handler不固定意图，不能简单回退二进制或切回旧语义；保留意图、创建receipt和平台publication，先对账或采用兼容构建，不删表、改checksum或清除历史。仅完成20，前端21/实时改造/门店/部署不自动实施；下一Phase21 NOT_STARTED。

状态与正式证据已同步；本次在Phase20终止，Phase21须新的明确执行请求。

---

<a id="phase-21"></a>
### Phase 21: 交付 Vue + Vite 工单前端与未知结果恢复

#### Status

COMPLETED

#### Goal

工单前端独立Vite构建并消费Rust API，完成真实分离示例的UI端。

#### Scope

- 从window.Vue/esbuild IIFE迁Vue模块/Vite，公开配置、SDK、相对chunk和独立依赖。
- CRUD/筛选/目录/CSV/提醒、冲突/未知提交恢复与非重叠创建路由。

#### Out of Scope

- 不把后台secret注入VITE_*，不增加iframe同源或下载权限，不做无关视觉重设计。

#### Inputs

- 方案编号：T01、X01、M02、M03；本台账全局约束及源方案对应章节。
- 现有工单frontend、Phase 6/8 SDK、Phase 9菜单规范、Rust API契约。

#### Execution Requirements

- 使用Vue/Vite标准入口与相对资源，SDK版本来自6；保留真实工单CRUD/CSV/分配/提醒和未知结果恢复。
- 消费Phase10共享会话语义；草稿按app/tenant/org隔离，切组织取消旧请求，授权续期不丢合法草稿。
- 生产通过Nginx运行，opaque iframe真实验证module/chunk/CORS；静态文件不包含秘密。

#### Expected Outputs

- vite.config/标准入口、业务UI与client、构建清单、独立README和真实浏览器回归。
- 实施证据：[验证报告](validation/phase-21.md)、[输入](validation/phase-21-inputs.json)、[边界](validation/phase-21-boundaries.json)、[运行及归档索引](validation/phase-21-runtime.json)与[台账更新记录](validation/phase-21-ledger-update.md)。

#### Acceptance Criteria

- [x] 真实opaque iframe可加载module/动态chunk，Origin:null请求Rust API且无Cookie凭据。
- [x] 主题无多余列表请求、失权隐藏服务器内容、409保留草稿；commit后丢响应不重复创建。
- [x] 创建与详情MENU互不扩大授权；普通用户可完成真CRUD/导出/提醒。

#### Risks

- Vite转换只改script名称不足；Vue导入、旧vendor依赖和E2E固定文件假设均需更新。

#### Dependencies

Phase 6、8、9、10、15、17、18、19、20必须COMPLETED。

#### Notes

生产只部署静态产物；dev/preview仅供本地工具使用。

2026-10-17完成并采纳Phase21。独立锁依赖Vue模块/Vite标准入口、公开配置与相对module/动态chunk交付，薄入口同步登记SDK监听后由lifecycle shell加载真实UI，修复顶层等待动态import导致一次port握手错过的竞态。登记SDK1.0.0及opaque iframe权限保持，生产由静态Nginx服务，不以dev/preview充当交付。

当前应用Bearer/requestId及credentials:omit消费Rust CRUD、筛选、有效成员目录/分配、宿主broker CSV与独立提醒。创建稳定header键及完整冻结DTO、提醒JSON键均显式恢复，未知后401/403不释放此前可能已提交的原事件；预期状态与合法完整DTO才结束事件，畸形2xx/网络/协议/5xx保守未知。提醒准备未知与发布未知分开，409先核对目标再明确新事件；不因实时服务未就绪或缺Toast二发。

draft/intent只在同一存活iframe内按app/tenant/org/user隔离，授权续期保留合法输入、切组织取消旧请求、返回相同身份恢复对应原事件；失权隐藏服务器内容，主题无多余列表请求。409保留草稿并显式采最新version；PATCH/状态/删除未知先读回，删除未知按工单隔离。刷新/关闭/销毁页面不保证保留本地恢复信息，提示保持当前页面至核对完成；后端receipt/0003仍持久，但不新增浏览器存储或跨库原子承诺。

726项候选`675b639c3dd5b9e547b21c3b684167b6524855f192f252043d34cca8e8f992b0`，完整fast 55步/832PASS/0FAIL/0skip；chromium真实15项、Phase8 4组/Phase10 17组；chrome真实15项、Phase8 4组/Phase10 17组；msedge真实15项、Phase8 4组/Phase10 17组；独立干净构建3步/15源码输入PASS，四次同源outer/source/lease与资源空查PASS；独立源码安装/构建及秘密哨兵、实际production host/Nginx/TLS/opaque ESM/Origin:null无Cookie、普通用户真CRUD/导出/提醒、已commit丢响应不重复与精确MENU互斥按正式报告核验。Phase8/10轻量组与新真实服务检查分列；开发观察、原握手失败和环境/工具失败保留，不继承为本轮通过。

仅完成Phase21，其他36阶段及全局缓存保持。Phase22–25既有COMPLETED不重置，下一未完成主线Phase26 NOT_STARTED；不实施门店、后台语义/SQL/SDK改造、实时协议迁移、外部发行、切流或生产部署。

状态与正式证据已同步；本次在Phase21终止，Phase26须新的明确执行请求。

---

<a id="phase-22"></a>
### Phase 22: 验证 Rust 实时协议兼容并冻结选型

#### Status

COMPLETED

#### Goal

实证Rust Socket.IO与当前客户端兼容程度，冻结可执行的协议分支及证据，不把未验证候选当生产能力。

#### Scope

- 候选Rust Socket.IO/Engine.IO最小服务、真实客户端协议矩阵；websocket后台参考迁Rust。

#### Out of Scope

- 不替换生产通知，不把Axum裸WS当Socket.IO，不默认已选定某候选库。

#### Inputs

- 方案编号：T01、N01；本台账全局约束及源方案对应章节。
- 现socket.io版本/事件、frontend通知hook、通知协议tests、examples/websocket/server.ts。

#### Execution Requirements

- 记录候选库/许可证/版本，实测polling、Upgrade、事件认证、到期、重连与错误语义。
- 输出SOCKET_IO_COMPATIBLE或有证据的NATIVE_WS_REQUIRED；测试未跑/环境不足不能归为兼容失败。
- 兼容不满足的实证结论可完成选型阶段，同时触发37成为23之前必需门禁；不在23偷偷换协议。
- Rust实时参考只验证已声明协议能力，前端JS可作测试驱动，后台fixture必须Rust。

#### Expected Outputs

- Rust兼容性程序/测试、支持矩阵、依赖选型决策、Rust实时参考。
- 实施证据：`docs/validation/phase-22.md`，实际输入/运行索引为`phase-22-inputs.json`与`phase-22-runtime.json`。

#### Acceptance Criteria

- [x] 当前真实客户端与候选Rust的传输/认证/重连矩阵确实执行，结果及失败原因可复核。
- [x] SOCKET_IO_COMPATIBLE分支所有必需项通过；NATIVE_WS_REQUIRED分支列出关键不满足证据与37验收输入。
- [x] 缺环境保持真实未完成/BLOCKED，不以未知失败选择fallback。
- [x] 选型记录与台账条件触发同步，尚未完成37时不宣称WS兼容或通知迁移完成。

#### Risks

- 本次固定socketioxide0.18.7/engineioxide0.17.7；结论限于实测版本与矩阵，升级必须复验。真实平台/PG与调度仍归Phase23。

#### Dependencies

Phase 1、4、6必须COMPLETED。

#### Notes

本阶段完成的是选型结论，真实兼容失败并不等于本阶段无法完成。Phase37初始未触发；触发后无需为同一技术回退重复确认。

2026-10-16按明确指令完成Phase22。结论**SOCKET_IO_COMPATIBLE**：MIT的socketioxide0.18.7/engineioxide0.17.7与当前4.8.4真实客户端兼容；Phase37未触发，保持NOT_STARTED。Linux最终16步全部退出0，协议44/44（通知35+聊天7+两个父测试）、Rust单元7、聊天前端5、相关契约48通过，必需零skip。实测63项跟踪清单摘要d1355666f4220cedc83277278ce505d1f3e417397cc7304cb4cf3538ed6782a7；排除并行OAuth未执行脚本后62项当前字节一致。Rust聊天参考替换旧Node server.ts；生产通知仍为Node。Windows网络环境限制及Linux原失败/最终PASS、清理均见[证据](validation/phase-22.md)，支持范围见[协议决策](realtime-protocol.md)。按用户要求只标记本任务完成，Current Execution Pointer整个区段不改；其中旧realtime结果缓存不覆盖本节实测结论。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-23"></a>
### Phase 23: 迁移 Rust 通知服务并实现有界分区调度

#### Status

COMPLETED

#### Goal

Rust通知替代Node服务，保留实时安全/恢复契约并限制队列与复验并发。

#### Scope

- PgListener/最小读取、绑定身份、逐投递/周期复验、到期断开、分区队列/抖动、健康/退出和切流。

#### Out of Scope

- 不改变平台收件箱事实来源，不缓存长期正向授权，不无条件全局广播。

#### Inputs

- 方案编号：T01、N01、O02；本台账全局约束及源方案对应章节。
- 已冻结实时协议、notification-ws实现与配置/测试、平台socket-context接口和通知DB读取契约。

#### Execution Requirements

- 采用22冻结的协议；NATIVE_WS_REQUIRED时先完成37再接通知持久化/调度，不能让socket.io-client直连原生WS。
- 按user/tenant/org分区保序、全局并发与复验预算有界，过载可丢提示但数据库收件箱完整。
- 消费Phase10共享组织切换信号，停止旧身份连接；通知使用宿主平台会话，不用微应用token代替。
- 无JWT签名密钥，DB最小权限；受控切流最终退出Node镜像/脚本/Compose，旧宿主升级窗口明确。

#### Expected Outputs

- 独立Rust服务/镜像/配置、完整协议/PG/平台联合测试、队列与时延指标。
- 实施证据：[Phase23验证记录](validation/phase-23.md)、运行/输入/镜像/依赖/台账边界归档及更新建议均已创建。

#### Acceptance Criteria

- [x] 删成员/踢出/过期后下一次投递复验拒绝，连接在已配置并实测的周期复验窗口内断开；无跨租户组织消息，不承诺零延迟物理断连。
- [x] 慢复验不阻塞其他收件人，并发/队列/内存有界；LISTEN中断恢复，HTTP补拉完整。
- [x] 按所选协议验收：Socket.IO测试polling/Upgrade；原生WS测试首帧认证、心跳、关闭/重连/背压；均验证到期、live/ready、优雅退出。
- [x] 双页切组织停止旧身份投递；最终生产运行无Node后台，不把HTTP补拉宣称为未实现的自动polling回退。

#### Risks

- Rust协议验证不等于全套安全契约；单个SocketIO happy path不能完成本阶段。

#### Dependencies

Phase 4、6、10、12、22必须COMPLETED。

条件依赖：22结果为NATIVE_WS_REQUIRED时，37也必须COMPLETED；SOCKET_IO_COMPATIBLE时37保持未触发。

#### Notes

入口先有HTTPS再收敛通知ready，禁止启动循环。

2026-10-17实施结果：独立Rust crate/lock、薄main/窄lib，沿用SOCKET_IO_COMPATIBLE（socketioxide0.18.7/engineioxide0.17.7），37未触发。专用12列只读/LISTEN与真实平台socket-context绑定、逐投递/周期/独立到期检查；首次认证真实HTTP pending后的重复/超时/非法首帧/应用Bearer拒绝均有回归。按user/tenant/org FIFO公平调度，queue/worker/HTTP/Engine/TCP分别有界；raw旧SID物理拒绝与真实容量回收、全TCP停机修复实证。旧Node后台、启动文件、legacy通知Compose及活跃E2E导入退役；正式Rust镜像、配置/CA/角色/切流和HTTP恢复说明同步。

最终Linux r7：8个必需步骤PASS，四file真实平台PG联合34/34、Chromium双页1/1及6命名检查，均零失败/跳过/取消；最终五Rust门禁/29unit、受影响87回归与最新CI子集29、镜像9项和SQL角色实测通过。682项完整来源及5组/9共享导入当前字节匹配，自有DB/角色/容器/网络归零、WSLlease退出0。保留r1–r5失败及各补验，不继承旧证据；无远端registry或生产部署。

按用户明确要求仅标记本阶段完成；整个Current Execution Pointer（含旧实时字段缓存、规则及已验证基线）逐字节保持本次更新前原样，实际协议结论在本阶段与Phase22详情记录。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-24"></a>
### Phase 24: 拆分 OAuth 演练为 Vite 页面和 Rust 流程API

#### Status

COMPLETED

#### Goal

保留OAuth演练用途，同时移除Node后台、硬编码秘密和不安全的流程状态暴露。

#### Scope

- Vite静态页面/callback UI、Rust设备码/授权码PKCE/token/introspect/revoke/资源代理与配置。

#### Out of Scope

- 不使用Next.js，不推荐password grant作为新业务登录，不把机密交换移到浏览器。

#### Inputs

- 方案编号：T01；本台账全局约束及源方案对应章节。
- mini-services/oauth-playground/index.ts、pages.ts、现有四类流程、平台OAuth契约。

#### Execution Requirements

- 服务端state/verifier有界TTL、绑定发起会话、一次性消费；不能公开按state枚举verifier或长期存浏览器。
- 公开origin/callback/端口配置化，避免3004冲突；secret来自Rust私有配置。

#### Expected Outputs

- 独立前后端工程/镜像/部署模板、流程回归及去Node运行入口记录。
- 实施证据：[docs/validation/phase-24.md](validation/phase-24.md)，2026-10-16 已创建。

#### Acceptance Criteria

- [x] PKCE/设备码/受控服务凭据与历史兼容演练按声明矩阵通过真实平台；不支持项明确拒绝而非假响应。
- [x] 错误state、重放、过期、错误发起会话均拒绝；dist/URL/日志无secret或verifier泄漏。
- [x] Vite独立构建，Rust独立运行，端口/HTTPS/callback与文档一致。

#### Risks

- 历史demo依赖固定客户端和旧宽松CORS；应改配置及安全流程，不能照搬。

#### Dependencies

Phase 2、4、6、11必须COMPLETED。

#### Notes

只是演练参考，不新增生产用户登录体系。

2026-10-16 按明确指令完成 Phase24。OAuth 演练已拆为独立 Vite MPA/Rust API，旧 Node index.ts/pages.ts 退役；私有 secret、state/verifier、device code 和 token 保持服务端，具备 TTL/容量、会话绑定、一次消费与并发/未知结果失效。开发端口 3010/3011；独立非 root 镜像与 HTTPS 私网模板可实际运行。

最终 Linux r6 的 9 步、27 项 JS/开发入口、17 项 Rust 与真实平台/隔离 PG 的 12 项浏览器检查 + 1 项 fixture 全部通过，必需零 skip；最终 delivery-r5 两镜像/10 项正常 CA HTTPS 检查通过。原生快照 13 项、最终 JS/CI 43 项通过；平台既有 strict Clippy、Windows loopback 限制、前轮 FAIL/中止和资源回收分别保留。修复公开配置 password 能力误脱敏及平台构建 path dev-dependency 解析，旧依赖版本/迁移不变。输入、镜像、数据库及清理见 [Phase24 证据](validation/phase-24.md)。

按照用户补充要求，仅标记本阶段完成；整个 Current Execution Pointer 区段保持写入前内容，不推进或自动执行其他阶段。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-25"></a>
### Phase 25: 以 Rust CLI 替换平台登记与授权工具

#### Status

COMPLETED

#### Goal

提供可审阅、默认无写、凭据安全的Rust接入/发布工具，移除Node后台式管理工具依赖。

#### Scope

- inspect/plan/bootstrap、grants-plan及受控API apply、register/preflight/activate/deployments/rollback命令。

#### Out of Scope

- 不直接应用平台SQL草案、不默认执行外部写、不打印/自动重置客户端secret，不控制Docker生命周期。

#### Inputs

- 方案编号：T01、X01；本台账全局约束及源方案对应章节。
- 现platform-admin/grants工具、manifest固定校验、平台管理API和Phase14 grant契约。

#### Execution Requirements

- 默认plan，明确写命令/参数；固定HTTPS origin、拒redirect、限时/限响应、未知结果先查询。
- 同步创建/详情MENU、manifest key与校验器；客户端用实际ID，跨应用/已有版本冲突拒绝。
- grant apply使用14的正式API与独立写权限；CLI持有管理员session也不绕过平台根上下文规则。
- 通用URL登记与授权命令不依赖ZIP文件安装；若新增ZIP上传命令，另在5完成后联调，不制造镜像共享卷前提。

#### Expected Outputs

- 独立Rust CLI、schema/manifest、脱敏回归、命令与秘密配置说明，旧入口退役。
- 实施证据：[Phase25验证记录](validation/phase-25.md)、[输入清单](validation/phase-25-inputs.json)、[运行证据](validation/phase-25-runtime.json)和[结构化更新](validation/phase-25-ledger-update.md)。

#### Acceptance Criteria

- [x] plan无外部写；重复bootstrap幂等且不抢占他应用客户端/菜单。
- [x] 错误origin/redirect/超时/冲突输出固定类别，无Bearer/secret/正文泄漏。
- [x] register仅提交现有允许字段；data grant经管理API，不偷偷直写平台表。

#### Risks

- 原工具已有安全语义必须保留；新增菜单会触发固定数量/key校验变化。

#### Dependencies

Phase 4、9、11、13、14必须COMPLETED。

#### Notes

前端打包工具可继续用Node，不应粗暴删除所有.mjs；管理后台目标为Rust。

2026-10-17 按明确指令完成 Phase25。独立 Rust CLI 替代旧 Node 管理入口，默认离线计划、写命令显式 --apply；固定七菜单/实际clientId、精确HTTPS origin/共享deadline/限响应、成功与失败总出口敏感值保护、未知结果先对账。URL登记只提交现有允许字段；授权通过正式功能/Phase14 grant API，保留角色集合、direct membership只校验、scope差异拒绝，不执行SQL或管理Docker。

最终 Linux 17 步全部PASS，21项Rust、428项JS契约/CI、9项真实HTTPS、13项真实CLI/平台/隔离PG检查+1个fixture、9项无Node运行验证通过，必需零skip。release摘要 13d63b45fef71e1a17577e0e1af818aa431a2649faac56f50fa145a02d52dc44；输入/数据库/失败轮次和清理见[证据](validation/phase-25.md)。平台Cargo/旧迁移不变，无外部发行或生产部署。现有API无跨请求CAS/事务，按单管理员维护窗口执行，部分/未知结果先查询；菜单status由平台策略作最终判定。

遵照用户要求，仅标记本阶段任务完成；整个 Current Execution Pointer 区段保持本次写入前字节，不推进或自动执行其他阶段。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-26"></a>
### Phase 26: 建立简单门店 Rust API 与独立业务库

#### Status

COMPLETED

#### Goal

交付stores第二真实业务消费者的后端，验证平台授权/成员窄接口可用于工单之外的业务。

#### Scope

- 独立Rust crate/配置/数据库迁移、门店资料、员工关联、每日营业记录录入/查询/汇总。
- 应用token内省+context、资源动作/grant/SQL范围、平台成员查询/关联校验、乐观版本与不可变归属。

#### Out of Scope

- 不实现支付/POS/复杂库存/排班/审批，不直查平台身份表，不依赖Phase 30最终通用框架。

#### Inputs

- 方案X02/15.9；Phase6/14契约，17Rust接入经验，19窄成员接口与逐应用策略。
- 未来输出business-apps/stores/backend及独立配置，不把不存在的文件当已有实现。

#### Execution Requirements

- 一店映射一个组织节点仅为示例假设，业务库保留资料/营业状态，不复制身份；平台/工单/门店DB和账号独立，可同PG实例。
- 登记store、store-staff、store-daily-report资源及真实动作，direct membership关联、金额精度和营业日期语义明确。
- 日报限定一店一营业日一份，tenant/store/营业日期唯一约束+version；重复提交返回既有或409，汇总不重复累计。金额用十进制定点值（如NUMERIC(18,2)，API十进制字符串），不用浮点；示例营业日统一Asia/Shanghai，后台验证，不扩成交易流水。
- 后台确认tenant/org/owner及不可变归属，列表/count/detail/write/汇总统一SQL范围；店员SELF、店长ORG、区域ORG_SUBTREE均受组织硬上限。
- 使用stores机密客户端，Basic内省+应用Bearer context逐请求验证，SDK前端不持secret；业务用户无需grant管理权限。
- 在19窄策略下增加真实门店调用/能力登记，不先抽公共crate；服务薄入口/窄lib、依赖单向。

#### Expected Outputs

- stores Rust API、SQLx迁移、OpenAPI/DTO或明确契约、配置模板与README。
- 临时PG真实CRUD/汇总、成员校验和授权负向测试，独立Cargo锁文件与构建入口。
- 实施证据：[验证报告](validation/phase-26.md)、[输入](validation/phase-26-inputs.json)、[边界](validation/phase-26-boundaries.json)、[运行及归档索引](validation/phase-26-runtime.json)与[台账更新记录](validation/phase-26-ledger-update.md)。

#### Acceptance Criteria

- [x] 独立cargo build/test，空隔离库初始化；门店、员工关联、每日记录真实API和汇总可用。
- [x] 自己/本店/下属门店及平级/跨租户/组织硬上限按SQL执行，功能权限或grant缺失均拒绝。
- [x] 工单token调用门店、stores token调用工单、客户端错绑、撤销成员/授权均拒绝。
- [x] 负责人/员工由平台受控成员协议校验，过期/停用/越界成员不能关联；没有平台身份表直连。
- [x] 归属不可跨租户组织编辑；同店同日并发/重复提交不重复记录或计入汇总，版本冲突、十进制精度与Asia/Shanghai日期边界有回归；日志/错误不泄漏秘密。

#### Risks

- 组织节点与门店业务实体不能在平台层固化；日汇总和成员查询也要受完整数据边界。

#### Dependencies

Phase 4、6、14、17、19必须COMPLETED。

#### Notes

这是第二业务消费者后端，不可由工单服务改appCode或假API替代。先完成此阶段，再由Phase 27实证，Phase 30后续收敛。

2026-10-18完成并采纳Phase26。新增独立stores-api Rust crate、配置、SQLx迁移和业务库/专属账号，交付门店资料、负责人/员工关联、一店一营业日十进制日报及汇总。逐请求stores Basic内省及应用Bearer context；功能权限与direct grant独立，所有资源SQL均受tenant/组织硬上限/数据范围/SELF owner约束，关联关系不扩大其他资源范围。

新负责人/员工通过平台现有受控成员HTTP验证；身份/资源/动作/目标绑定的短期witness从成员HTTP开始计时，在业务锁内核对且不续期，跨库最后验证至COMMIT仍有竞态。业务不可变归属、version冲突及安全审计与写入同事务；未知提交须当前授权下读回核对，不自动重试。没有平台身份表直连，不新增通用crate或改变平台成员协议。

日报tenant/store/营业日期唯一，重复/并发提交明确409且汇总不重计；金额用十进制定点字符串、SQL NUMERIC及SUM::text，无浮点累计，合法年月日及Asia/Shanghai默认营业日边界有回归。负责人变更与历史关联移除分别按当前资源授权和成员有效性处理。

757项同源候选`2a00d4ecf05558d1b6019f73707160c216e1e4f5fd18f24ea95ee8752dfee5eb`；完整fast 61步/894PASS/0FAIL/0skip；stores SQL 1父测试/8具名检查、real live 1父测试/16具名检查PASS。原始命令/日志、冻结parser的真实SQL/live结果、实际stores/work-orders ELF、三独立非超级账号/跨库CONNECT拒绝、数据库/角色/进程/pool与受管容器/网络/lease清理均由正式记录绑定；全局cargo cache另行清理，不把缓存保留伪称为每轮无资源。开发观察与原始失败独立保存，历史门禁不继承。

本次完成Phase26后端与范围内验证；109份历史正式证据、其他36阶段及全局缓存保持，终态检查点指针Phase27 NOT_STARTED。用户已授权Phase27，待本阶段终态核验后单独启动；公共能力收敛、发行、切流及生产部署仍属后续阶段。

Phase26状态与正式证据已同步；用户已授权Phase27，待本阶段终态核验后按顺序启动。

---

<a id="phase-27"></a>
### Phase 27: 交付门店 Vite 微应用与双消费者契约实证

#### Status

COMPLETED

#### Goal

交付简单门店真实前端并联调Rust API，与工单共同证明SDK/成员/授权契约可复用。

#### Scope

- React/TypeScript/Vite独立工程、门店资料/员工关联/每日营业记录页面，SDK生命周期和受控权限状态。
- stores应用/客户端/菜单与资源manifest，真实双应用/双页组织切换和成员协议实证。

#### Out of Scope

- 不提前抽通用UI框架或公共crate；镜像/公开版本路由在Phase 28联合交付，不在此控制Docker。

#### Inputs

- 方案X02/15.9，Phase 26真实API，6/8的SDK与更新语义、9路由、10会话、15grant UI、21工单前端、25Rust管理CLI。

#### Execution Requirements

- 工程有package-lock、vite.config及dev/build/preview，生产base相对且opaque iframe加载；使用固定SDK来源，不读宿主storage。
- stores为稳定独立应用，/micro/stores与创建/详情路由不重叠授权；client secret只在Rust配置。
- 页面完成真实CRUD/成员关联/营业录入与汇总，401/403/409/超时可理解，按取消信号丢旧响应。
- 草稿按app/tenant/org隔离，两页共享当前组织；主题/续期不重置合法表单，失权停止操作。
- 工单/门店真实调用同一受控成员HTTP协议，记录资源/动作/数据范围差异供Phase 30收敛，不仅做fixture。

#### Expected Outputs

- business-apps/stores/frontend、Vite锁定依赖/SDK接入、页面与client、应用manifest和独立README。
- 双消费者契约实证报告及普通用户浏览器回归，构建产物清单供Phase 28镜像化。
- 实施证据：[验证报告](validation/phase-27.md)、[输入](validation/phase-27-inputs.json)、[边界](validation/phase-27-boundaries.json)、[运行索引](validation/phase-27-runtime.json)与[台账更新记录](validation/phase-27-ledger-update.md)。

#### Acceptance Criteria

- [x] 独立目录npm ci/build成功，无根node_modules/个人缓存依赖，dist无凭据。
- [x] 普通用户在真实平台/门店Rust/业务库完成资料、员工关联、每日记录及汇总，不用mock或直写平台DB授权。
- [x] opaque iframe module/chunk、Origin:null/CORS、主题/路由/取消/续期/失权实际通过。
- [x] 双页工单+门店刷新/切组织/退出无跨组织旧数据或草稿提交；两应用token/权限/数据grant不串用。
- [x] 两业务同一成员协议实证和差异记录齐全，未登记动作/跨应用调用拒绝，Phase 30的输入无需虚构。

#### Risks

- 浏览器上下文与权限变更竞态会影响两个应用；必须复用共享会话语义而非另做门店登录。

#### Dependencies

Phase 6、8、9、10、15、21、25、26必须COMPLETED。

#### Notes

门店前端与真实业务闭环在本阶段完成；镜像发行、v1/v2公开路由在Phase 28，最终公共能力在Phase 30。

2026-10-18 Phase27完成并终态核验：交付独立React/TypeScript/Vite门店微应用、固定SDK1.0.0、11项菜单/九精确动作与Rust CLI接入；普通用户在真实平台、工单/门店Rust和三个独立业务账号库完成资料、成员关联、日报与精确汇总，opaque iframe/Origin:null、主题/路由/取消/正常续期/失权及双页组织切换/刷新/退出均实证。最终r2输入809项、SHA256 02304b07f8c9b25a34b124898c7ce06ef283f092eb95910dde73fa828b559b9d，sourceRevision 32956c4532bbd6eff53584fd0f5e9bb6e188437c。

正式完整fast63步/1033PASS/0FAIL/0SKIP，1931 filtered单列；Chromium156.0.8078.4、Chrome155.0.8059.84、Edge155.0.4283.62各21组PASS，三产品直接CLI TLS各9具名检查另列。独立空缓存安装/构建、三库非超级账号、实际两消费者成员HTTP和跨应用/未登记用途拒绝通过；五条原验收、同源/host不变、资源及专属cache清理和625项managed证据全部核验。上轮r1 fast旧policy断言及三browser菜单元数据校验FAIL完整保留，不继承为本轮PASS。

草稿只在存活iframe内存，unknown写显式当前授权读回且不自动重发；成员见证最多3秒，最后检查至COMMIT仍有跨库竞态。本阶段浏览器不冒称重跑全部SQL scope矩阵。114份历史正式证据、其他36阶段和全局缓存字段保持；Phase28镜像/公开v1v2路由及生产部署未执行，下一指针28 NOT_STARTED。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-28"></a>
### Phase 28: 交付工单/门店镜像与统一外部 URL 接入

#### Status

COMPLETED

#### Goal

完成推荐镜像交付主链：GitHub Actions构建、LinuxCompose拉固定digest、独立入口Nginx版本路由，平台统一登记/预检/激活；外部HTTPS静态托管同契约接入。

#### Scope

- 已部署URL登记UI/API封装、工单/门店独立镜像模板、发行清单和受控版本路由。
- 两应用各自演示v1/v2入口、真实SDK/Rust授权、前端回退与旧资源；ZIP另由5交付，混合链由Phase 34总验收。

#### Out of Scope

- 不让平台拉镜像/启容器，不把image/digest/target塞入deny_unknown_fields旧RegisterBody。

#### Inputs

- 方案编号：D02、D03、X01；本台账全局约束及源方案对应章节。
- applicationsApi、ApplicationReleasesPanel、register/preflight/activate接口、方案第14节。

#### Execution Requirements

- 外部先部署再公共预检/激活；登记不自动创建Nginx规则、不负责Docker启停。
- GitHub构建/推送与Linux部署分离，固定架构/digest和持久发行清单；真实推送/目标操作按明确配置，隔离registry可用于演练。
- 按版本固定路由；前缀剥离与镜像根目录一致，未知版本404，旧版本容器/等价文件及路由保留。
- RustCLI/CI持久清单原子绑定app+公开版本URL+来源+digest，拒绝同版本换源/换digest；跨环境同版本同artifact。
- RegisterBody仅现有允许字段；平台未建metadata前，digest冲突由CLI/CI真实拒绝，不能伪称register已存储校验。
- 外部静态域名直载时配置明确绝对业务HTTPS API base或自有受控代理，验证Origin:null/CORS/CSP，宿主origin白名单不改成容器名。

#### Expected Outputs

- 外部入口登记UI/契约回归、Rust CLI/CI清单、工单镜像与Nginx版本接入说明。
- 实施证据：[验证报告](validation/phase-28.md)、[输入](validation/phase-28-inputs.json)、[边界](validation/phase-28-boundaries.json)、[运行索引](validation/phase-28-runtime.json)与[台账更新记录](validation/phase-28-ledger-update.md)。

#### Acceptance Criteria

- [x] 两应用可独立构建镜像，Linux隔离环境拉取启动；纯镜像无需平台产物卷，入口Nginx按版本代理。
- [x] URL登记DRAFT→预检PASSED→激活，HTTP200+FAILED仍拒绝；界面明确外部部署与平台管理的边界。
- [x] v1/v2、动态chunk与回滚真实可用；持久清单拒绝同公开版本URL换来源/digest，记录实际拒绝层。
- [x] CI/Compose检测容器不健康时不推进路由/登记/激活；平台拒绝入口/资源或已配置healthUrl的预检失败，分别记录拒绝层，不虚构平台可读Docker health。同域/批准外部origin的API解析与SDK授权实际验证。
- [x] 镜像部署不自动授权，不虚构平台Docker控制；镜像、RustAPI、数据库的版本分别可追踪。

#### Risks

- 多微应用/多版本静态容器增加占用，保留镜像不等于旧URL在线；实际registry/CPU/域名仍待提供。

#### Dependencies

Phase 3、4、6、21、25、27必须COMPLETED。

#### Notes

镜像是推荐交付，ZIP是兼容；本阶段不依赖5的文件安装机制。CI构建成功、容器healthy、URL预检、平台激活和业务验收分别记录。

2026-10-19 Phase28验收及终态收尾完成。用户在本聊天明确要求“验收并完成”；采用839项冻结输入f77df330f956da2d9bcb5ec96a5ae6fe617dc752f15267be565c3033d3be05c5，sourceRevision 32956c4532bbd6eff53584fd0f5e9bb6e188437c。完整fast 63步/1083PASS/0FAIL/0SKIP，filtered 1931另列；六镜像实际构建、隔离registry push/候选清除/固定digest pull与Compose，14项交付检查和Chromium/Chrome/Edge各9项真实HTTPS/SDK/业务/三库检查全部通过。

五条原验收均PASS；登记仍为既有六字段，平台不控制Docker、不伪存储digest、不自动授权；CLI/CI权威持久inventory实际拒绝同版本换源/digest及跨环境差异。v1/v2、动态chunk/旧路由、回退、健康拒绝/激活重检及外部Origin:null/CORS/CSP通过。

本次API镜像采用冻结源码按Cargo.lock编译、验证ELF后runtime装箱；默认API Dockerfile builder未运行。同daemon清除本轮候选后真实拉取，基础层cache可能保留，不宣称第二机器冷启动。远端workflow、外部registry发行及生产部署未执行；早期开发失败及首轮正式启动未就绪拒绝完整保留。正式原始证据、来源、清理及专属cache删除已严格核验。119历史正式文件、其他36阶段及全局cache字段保持；下一指针29 NOT_STARTED，无后续阶段执行。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-29"></a>
### Phase 29: 收敛宿主加载、失败恢复和高复杂页面边界

#### Status

COMPLETED

#### Goal

公开入口少加载宿主壳，页面异常可局部恢复，并用真实构建/浏览器数据验证收益。

#### Scope

- 受保护AppLayout懒加载、命令面板按需、Outlet局部Suspense/ErrorBoundary和chunk失败恢复。
- 选取一个有真实耦合痛点的管理页面按用例拆hook/组件，保持API契约。

#### Out of Scope

- 不按包名盲拆React/vendor，不仅调高warningLimit，不批量改所有页面/视觉。

#### Inputs

- 方案编号：F02；本台账全局约束及源方案对应章节。
- App.tsx、AppLayout、CommandPalette、构建模块图与同口径性能基线。

#### Execution Requirements

- 重放render throw/lazy chunk404；自动reload如使用必须有次数上限。
- 先记录gzip/初始传输/LCP/INP；20%是候选目标，正式预算用实测决策冻结，不虚报收益。

#### Expected Outputs

- 加载/错误边界实现、局部页面职责拆分、前后模块/资源/体验对比。
- 实施证据：[Phase29](validation/phase-29.md)、[原始测量](validation/phase-29-performance.json)、[预算](validation/phase-29-budget.json)、[输入](validation/phase-29-inputs.json)、[运行索引](validation/phase-29-runtime.json)及[终态建议](validation/phase-29-ledger-update.md)。

#### Acceptance Criteria

- [x] 登录入口不加载完整命令面板/导航/通知连接链；初始传输较同口径基线下降。
- [x] 注入页面异常时壳导航/恢复入口仍可用，旧chunk404不变成HTML。
- [x] 主流程/授权/表单回归无退化，报告给出原始测量而非仅总包大小。

#### Risks

- 全局Suspense/重动画会影响壳稳定；旧vendor循环初始化事故不能复发。

#### Dependencies

Phase 3、4、7、9必须COMPLETED。

#### Notes

只做有证据的单页边界收敛，其余无收益搬迁不在范围。

本阶段已完成：受保护壳与命令面板按需加载，页面render throw/lazy404/pending保持壳导航和恢复入口，自动reload次数为0；微应用epoch/appCode生命周期粒度与授权/会话协议保持。ApplicationsPage仅按创建/编辑用例拆hook和表单组件，API请求体及发布引导不变。实际production前后各5次冷启动，transfer 404515→323890B（下降19.93%）；20%候选不虚报。预算为本地实验回归门限，非生产SLO/线上INP。完整Chromium11套件209PASS；三产品专项各9PASS，既有10套件各200组PASS，旧整轮FAIL与专项修正分列；隔离Linux391、政策55、TS/lint、真实Nginx7请求及清理PASS。851公开输入300df64b7a9cb1fcff4aa3371f8a657e0149cf0ad10cb8c4bbc9905124012456，124历史证据保持；来源/原失败/兼容回退与未运行范围见正式报告。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-30"></a>
### Phase 30: 优化授权查询链并收敛 Rust 能力模块

#### Status

COMPLETED

#### Goal

在迁移契约稳定后降低重复平台HTTP/SQL成本，同时保持完整实时授权与单向模块边界。

#### Scope

- 请求内复用、一致context快照及新撤销/授权版本复核、最小机密客户端验证接口、预算/连接池参数。
- 平台窄lib/use-case/typed policy出口，相关handler与SQL编排分离。
- 平台通用paging用checked arithmetic/上限处理极大page；仅提取本授权链涉及的lib/typed policy/use-case。

#### Out of Scope

- 不在语言迁移时顺带删校验，不做长期正向缓存，不为每表造trait或无边界common。
- 不搬迁整个发布、用户、租户管理模块，不将全平台重构并入授权协议优化。

#### Inputs

- 方案编号：B01、B03；本台账全局约束及源方案对应章节。
- Rust工单/通知客户端回归、app_context/app_members/app_token、Phase14授权失效机制和容量基线。

#### Execution Requirements

- 以工单19和门店26/27实际共用成员HTTP协议为输入，冻结受应用能力限制的通用接口，再判断是否值得抽公共crate。
- 受控合并验证同时认证机密客户端application_id、token受众/身份、权限/grant；工单与门店双路径等价比较后各自切换。
- 一致context快照外有最新撤销/授权版本复核，不能在同一个REPEATABLE READ旧快照内假称看到新撤销。
- 保持单crate窄lib/typed用例及单向边界，不共享数据库模型；paging使用checked arithmetic/明确上限。

#### Expected Outputs

- typed用例与请求内授权复用、受控合并API及客户端切换、HTTP/SQL/锁/池预算对比。
- 实施证据：[Phase30验证记录](validation/phase-30.md)，逐文件输入、原始性能及运行索引同时归档。

#### Acceptance Criteria

- [x] 工单/门店真实使用同一受控成员协议，未登记资源动作、应用token互换、客户端错绑、平级/跨租户均拒绝。
- [x] 双应用新旧授权路径在身份/菜单/grant/组织硬上限/失效/错误语义等价，不能只测同一应用换appCode。
- [x] 同负载授权HTTP/SQL/锁/池预算有实际前后对比，收益不足保留旧路径并解释，不牺牲撤权。
- [x] contextVersion不能作认证凭据，公开API收敛且依赖单向，公共测试验证行为。
- [x] paging极大page不溢出500，稳定边界契约及回归进入CI。

#### Risks

- 缓存、快照和撤销竞态会改变安全语义；优化无收益时保留原路径并说明原因，不伪报完成性能目标。

#### Dependencies

Phase 6、11、13、14、17、18、19、20、23、27必须COMPLETED。

#### Notes

26/27必须先完成，不允许让门店等待Phase 30最终通用化而形成环。公共字段变更遵循6；是否workspace另凭独立复用/编译证据决策。

本阶段已完成。862项178ffd6ed48417df5769fc9126c5afad4013e9d2aa2f5f631d20a3c1b7c98b43冻结候选，31个正式受管步骤、153个Rust测试及205项补充政策通过，必需无FAIL/skip。旧平台ELF与当前真实ELF、双应用combined/legacy及工单只读/写入/成员/提醒联合均通过；公共测试与真实极大分页进入CI。

采用单语句一致投影和独立新语句SHA-256 revision/token复核，最多一次重建；无跨请求正向缓存、全局版本锁点、新迁移或公共业务crate。工单默认combined，门店因本轮总耗时/尾延迟收益不足保留legacy，两者均支持显式切换。combined HTTP调用减少50%、PG含嵌套调用减少21.05%，成员减少20.59%；SQL累计执行时间增加及门店耗时回归如实列入证据，不宣称全面提速。

130份既有正式证据、原迁移/Cargo.lock/生成公共契约及其他36阶段详情保持；本阶段容器/数据库/网络与专属cache已核验清理。全平台既有fmt/严格Clippy债务及未执行的完整浏览器/发行矩阵单列，无生产部署。下一阶段31仅为NOT_STARTED指针，须新请求。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-31"></a>
### Phase 31: 交付受支持升级与 dry-run 保留策略

#### Status

COMPLETED

#### Goal

受支持版本可增量升级，历史数据与静态版本的受控清理有可审阅策略和默认dry-run工具。

#### Scope

- 平台/业务受支持上一版本→新版本升级矩阵、旧前端兼容与expand/contract窗口。
- 列出审计/通知/会话/token历史/软删/静态版本保留政策，选择有限试点实现配置化dry-run与防误删；不一次实现全部历史表purge。

#### Out of Scope

- 不做分页/搜索性能重构（已在Phase17/30），不默认分区/每租户分库。
- 不擅定生产保留天数或执行真实清理，不删除active/previous。

#### Inputs

- 方案编号：O01；本台账全局约束及源方案对应章节。
- 各迁移与索引、paging、工单搜索、版本引用、产品保留需求和Phase30负载。

#### Execution Requirements

- 只新增迁移，验证平台/工单受支持旧库及门店新库，明确旧前端API兼容与expand/contract窗口。
- 候选：静态至少30天且至少最近5版（并集），audit在线180天、普通通知90天、业务软删30天；过期凭据缓冲待明确。
- active/previous/跨环境引用/在途任务始终保护；镜像旧URL在线与ZIP目录分别验证，不能仅看仓库镜像仍存在。
- 候选只用于隔离dry-run/影响预览，未确认生产政策的apply保持不可用；保留与备份分别建模。

#### Expected Outputs

- 受支持升级兼容矩阵/回归、分类保留政策及待确认项。
- 有限试点dry-run/影响预览工具和版本引用保护测试，其他表的实际purge另列后续任务。
- 实施证据：[验收报告](validation/phase-31.md)、[输入](validation/phase-31-inputs.json)、[runtime](validation/phase-31-runtime.json)、[升级与保留协议](upgrade-and-retention.md)。

#### Acceptance Criteria

- [x] 受支持旧库升级不改旧checksum，不丢UUID/编号/授权数据；旧前端兼容结果可追溯。
- [x] dry-run只输出目标/数量/引用原因，不修改生产数据；未确定政策的表不自动apply。
- [x] 版本清理拒绝active/previous/跨环境引用/在途任务/保留期资源；保留与回退测试覆盖试点。

#### Risks

- 保留窗口、法律/产品要求及旧浏览器最长生命周期尚未确定。

#### Dependencies

Phase 4、5、11、14、16、18、23、26、30必须COMPLETED。

#### Notes

政策细值与旧页面支持窗口的剩余项见Q08。可完成隔离候选dry-run，不能把未授权生产清理当已执行。

2026-10-19完成：仅执行Phase31。平台0001–0003→0004逐表/旧迁移记录不变、坏checksum拒绝；工单27项与门店初始化/重启通过。8份Phase30冻结消费者逐字节可追溯，宿主真实发布API与工单/门店combined、legacy真实三库均通过。静态试点30天或最近5版并集保护，参数只加严；只读REPEATABLE READ全表不变、active/previous/跨环境/共享引用/取消及结果未知保护、旧镜像实际URL及9个ZIP目录脚本字节、17项发布/回滚和8项审计回归通过。操作表只存未完成保护，明确结束仅释放本次UUID，未知/传输失败保留，不改409/CONFLICT契约。

最终885输入 621ea69d5326f9b043cde3a01bf6ccbd2dd454179abb6b71efc78a76847f339e，36路径变化，正式32步全PASS，3项CI精确入口及17政策通过；原5轮开发FAIL、2轮正式FAIL及修复与限制完整归档。旧迁移/依赖、135历史证据和其他36阶段详情保持；自有容器/网络/三库角色/进程及缓存已清理。全平台fmt/Clippy既有债务不冒称清洁，本阶段窄库/CLI/业务crate与定向格式、构建/doc门禁通过。

Q08生产期限、凭据缓冲和旧页面窗口仍待确认，候选仅隔离预览，全部apply不可用；不是全浏览器矩阵、外部发行或生产部署验收。仅expand，不contract，静态回滚仍重新预检；旧ELF数据库回退须另验迁移器。三项验收PASS，当前指针32 NOT_STARTED，下一阶段须新请求。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-32"></a>
### Phase 32: 建立真实指标、告警与容量基线

#### Status

COMPLETED

#### Goal

以可比较指标观察身份、授权、微应用、队列和依赖故障，恢复缺失监控交付。

#### Scope

- Rust/前端关键耗时分解、低基数失败原因、队列/连接池/限流指标，私有采集与状态转移告警模板。

#### Out of Scope

- 不将发送尝试命名为用户已收到，不以指标带user/token/任意URL，不默认商业SLA。

#### Inputs

- 方案编号：O02、N01、D01；本台账全局约束及源方案对应章节。
- 现observability模块、运行事件、方案监控/容量目标、各阶段性能证据。

#### Execution Requirements

- 按15.7候选口径首轮测量：常规API p95≤500ms/p99≤1s、LCP≤2.5s/INP≤200ms、非预期5xx<0.1%；固定机器/网络/数据与含授权口径。
- 初始模型100并发活跃会话/10万代表性业务记录后逐档增压；在线/活跃/并发请求分别统计，记录CPU/内存与瓶颈。
- 撤权提交后的新请求用最新授权；前端候选20秒感知上限，不能为性能用长正向缓存。
- committed/delivered、entry/READY/mount、DB锁/平台HTTP/SQL/密码池分别观测；ready/live/真实收件分别验收。
- 告警稳定状态并只推转换，实际渠道另配置；备份过旧/失败/校验失败纳入可扩展事件。

#### Expected Outputs

- 真实deploy/monitoring文件、指标/告警模板、基准脚本/数据规模与故障报告。
- 实施证据：`docs/validation/phase-32.md`、inputs/runtime/performance.json及ledger-update.md。

#### Acceptance Criteria

- [x] 双应用context/DB锁/通知分区/entry→ready可观测，无token/user/完整URL等高基数秘密标签。
- [x] 真实故障产生可重复DOWN/RECOVERED，恢复补拉完整，公开Nginx不能访问私有metrics。
- [x] 同口径负载/数据/硬件/网络和p95/p99、错误、资源结果齐全，候选未达标如实记录，不宣称当前容量或生产SLA。

#### Risks

- 无实际接收渠道时只能证明本地事件，不宣称外部告警送达。

#### Dependencies

Phase 3、5、17、18、23、26、27、28、29、30必须COMPLETED。

#### Notes

候选阈值是测量起点，不是已证明容量；正式SLO/资源预算待Q09确认。实际外部渠道未知时只验收本地事件。

三项原验收PASS；906项输入583f7f47d727…，正式服务40步、浏览器20步及宿主26项通过。100/150/200会话/10万记录均保留真实候选结果，不证明生产容量；有效运维凭据私网200/公网404，通知34联合+双页6检查与两轮DOWN/RECOVERED通过。边界、旧失败、debug/单主体/冷初始化预算、资源及清理详见[实施证据](validation/phase-32.md)。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-33"></a>
### Phase 33: 恢复加密备份与隔离新库恢复工具

#### Status

COMPLETED

#### Goal

恢复平台/工单/门店数据库与对应静态/镜像/路由的完整工具，在隔离目标验证业务恢复并测量RPO/RTO。

#### Scope

- 多库协调恢复点/停写或对账、加密异机备份、版本产物/镜像digest/配置、密钥分离、定时模板与校验。
- 试点完整恢复必测；生产15分钟候选需要WAL归档/PITR设计与真实环境验收，未验收明确标注。

#### Out of Scope

- 不覆盖现有生产库、不自动删失败目标、不把前端指针回滚当数据库回滚；不安装真实系统定时任务。

#### Inputs

- 方案编号：D01、O01、O02；本台账全局约束及源方案对应章节。
- 架构文档恢复契约、缺失deploy/recovery清单、各Rust服务与版本存储、保留政策。

#### Execution Requirements

- 后台使用Rust及原生DB/系统工具，不引入Node服务；不覆盖现有目标，恢复到受控新库。
- 候选试点RPO24h/RTO4h，生产RPO15min/RTO1h；每天dump不等于15分钟RPO，按选择的验收profile记录证据。
- 多库一致恢复点与静态/镜像/路由manifest协调，异机副本可取得性和加密校验必测；密钥分开保管。
- RPO按最近异机可用恢复点，RTO含取密钥/恢复/就绪/重新登录/业务验证，停写时间单列。
- 开放流量前提高session_version、撤销平台会话及应用token、普通OAuth access/refresh与未完成授权/设备凭据，记录恢复审计；只换JWT密钥不够。

#### Expected Outputs

- 真实恢复/验证/定时模板和测试、运维runbook、隔离恢复证据。
- 实施证据：`docs/validation/phase-33.md`，已创建。

#### Acceptance Criteria

- [x] 空包/损坏/错密钥/缺产物/同目标/现有目标负向回归通过。
- [x] 平台、工单、门店隔离恢复后实际登录/授权/CRUD/通知/两套前端版本验证，不只看CLI退出码。
- [x] 异机可用恢复点、RPO/RTO、停写/恢复/业务验证分段实测；生产PITR未验收时不得宣称15min/1h达标。
- [x] 旧session/refresh/应用token/OAuth与未完成授权凭据恢复后不可重用，必须重新登录。
- [x] 备份失败/过旧/校验失败能产生告警事件，真实外部投递和生产定时安装边界明确。

#### Risks

- 跨库无天然同事务快照，恢复点/停写窗口/对账须明确；真实备份目标、密钥与告警渠道尚未提供。

#### Dependencies

Phase 3、16、18、23、26、31、32必须COMPLETED。

#### Notes

试点与生产profile分开列结果；完整试点恢复是本阶段必需，正式生产目标需真实环境证明。模板存在不等于定时任务已安装。

2026-10-19完成独立Rust工具及完整隔离试点：932项输入 ef0082e18930a22d226ad32fac7fe1b8bfcc027617bd21dcb0e578bd3ab6031a；正式Linux20步/十具名检查、Rust3/CI33及Windows同源门禁PASS。恢复前全凭据失效、三库/原生/路由/四镜像/四实际页面通过；RPO 44.808s，完整RTO 109.137s。详见[Phase33](validation/phase-33.md)、[输入](validation/phase-33-inputs.json)、[运行索引](validation/phase-33-runtime.json)、[恢复实测](validation/phase-33-recovery.json)、[更新建议](validation/phase-33-ledger-update.md)。物理异机、生产PITR、外部告警与定时安装未验收/未启用。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-34"></a>
### Phase 34: 完成独立示例交付与全仓目标栈验收

#### Status

COMPLETED

#### Goal

从干净独立工作区和实际Nginx/Docker环境证明主线目标，统一现状文档与最终发行证据。

#### Scope

- 工单与门店两套独立Vite/Rust/业务库、GitHub/Linux镜像主链、ZIP兼容链、普通用户授权与真实业务。
- 选定实时协议、双页组织切换、旧资源/回滚/恢复、禁止项扫描、统一现状文档和发行证据。

#### Out of Scope

- 不自动执行35/36，不合并skip到通过，不执行未授权现网/外部推送；37仅按22结果成为条件必需。

#### Inputs

- 方案编号：D01/D02/D03/T01/X01/X02/E01/C01/O02；方案15节及本台账全局基线。
- Phase1–33的真实产物/报告；22结果要求时还包括37证据；隔离Linux/Nginx/数据库/浏览器环境。

#### Execution Requirements

- 两套工程复制到独立目录各自npm ci/build、cargo build，不依赖平台实现crate/根node_modules/个人缓存。
- 平台根加独立grant权限通过API/UI配置普通用户；组织管理员伪造写入拒绝。
- 分别验收镜像固定URL/digest/路由与ZIP原子安装/持久卷；Docker启动、URL预检、平台激活及业务验收分开记录。
- 双应用双页面共享组织切换，应用token/client/授权不串用；按已选协议测试通知，37触发时必须已完成。

#### Expected Outputs

- 独立示例发行包/镜像清单、完整端到端与禁止项报告、更新后的现状架构文档。
- 明确当前/旧前端/SDK/后端/DB兼容矩阵与未触发扩展说明。
- 实施证据：[验证报告](validation/phase-34.md)、[输入](validation/phase-34-inputs.json)、[运行索引](validation/phase-34-runtime.json)、[发行清单](validation/phase-34-releases.json)、[已采纳台账更新](validation/phase-34-ledger-update.md)。

#### Acceptance Criteria

- [x] 所有活动入口只有Vite前端、Nginx静态/网关和Rust后台，无Bun/Caddy/Node后台/Next.js有效启动方式。
- [x] 工单完整功能与门店资料/员工关联/营业记录真实可用，独立DB账号、双应用SDK与权限/grant/组织上限负向测试通过。
- [x] 镜像主链与ZIP兼容链各自通过，旧URL/chunk在线；实际拒绝同版本异产物，记录digest校验位于CLI/CI还是新平台存储。
- [x] 双页刷新/组织切换/退出、失权、未知结果、Origin:null、v1→v2→v1、容器替换、实际恢复无必需skip。
- [x] Socket.IO分支polling/Upgrade或WS分支认证/心跳/重连按选型全部通过；触发37时其证据完整。
- [x] 其余33个主线阶段（1–33）均COMPLETED且证据齐全；本阶段必需验收全部通过后再同步标Phase 34为COMPLETED。未触发35/36/37保持NOT_STARTED，不能填成已完成。

#### Risks

- 运行环境未提供不能以静态审查替代发行验证；必须明确具体阻塞事实和已完成部分。

#### Dependencies

Phase 1、2、3、4、5、6、7、8、9、10、11、12、13、14、15、16、17、18、19、20、21、22、23、24、25、26、27、28、29、30、31、32、33必须COMPLETED。

条件依赖：22结果为NATIVE_WS_REQUIRED时，37也必须COMPLETED；SOCKET_IO_COMPATIBLE时37保持未触发。

#### Notes

2026-10-19全部六项验收PASS并采纳COMPLETED。最终935项输入9a23e8808cad…；独立双源码包干净构建，fast74步、PG24步、delivery7步；三产品各209 fixture/工单15/门店21及最终镜像各9组；六镜像隔离分发、Socket.IO44与恢复20步/十检查，pilot RPO 44.170s/RTO 114.557s。旧Node活动入口退役，149历史证据/21旧锁迁移与其他阶段详情保持。原始失败、范围补验与清理见报告。主线验收完成，无自动下一阶段；外部发行/生产和物理异机/PITR边界保持。

最终主线验收为Phase 34；门店Phase 26/27先于镜像交付Phase 28和公共能力收敛Phase 30。完成后无自动下一阶段；若37触发而未完成，不可宣称主线完成。

已收到Phase34明确执行请求：核对全部主线前置及Socket.IO选型；执行独立发行包、当前源码完整分层门禁、实际镜像/ZIP/浏览器/通知/恢复复验及终态同步。当前实施中，尚未完成验收。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

<a id="phase-35"></a>
### Phase 35: 条件扩展：Rust跨机产物分发

#### Status

COMPLETED

#### Goal

仅在仍需平台ZIP跨机文件分发且无共享存储时，实现受控Rust分发与逐节点可激活证据。

#### Scope

- 白名单target/artifact/job/receipt、受认证HTTPS分发、暂存原子安装、重试与逐节点确认。

#### Out of Scope

- 不自动启动Docker容器，不允许任意URL/主机/shell，不把一次公共URL请求当所有节点完成。

#### Inputs

- 方案编号：D02；本台账全局约束及源方案对应章节。
- 已完成本地原子发布、外部入口契约、目标拓扑/CA/凭据管理和部署观测。

#### Execution Requirements

- 必须明确ZIP跨机需求、目标白名单与无共享存储事实才触发；独立镜像无共享卷不触发本阶段。
- 比较受控对象存储/静态托管；采用agent时按artifact摘要认证传输、暂存原子安装、任务/节点回执与重试。
- 所有接流量必需节点安装后再公共预检，单次LB预检不代替逐节点回执；失败保持active。

#### Expected Outputs

- 当前实施验收：[报告](validation/phase-35-implementation.md)、[运行结果](validation/phase-35-implementation-runtime.json)、[输入](validation/phase-35-implementation-inputs.json)、[保留校验](validation/phase-35-implementation-preservation.json)、[台账建议](validation/phase-35-implementation-ledger-update.md)。
- 独立Rust分发组件/协议/任务UI、节点失败恢复测试与多节点演练。
- 实施证据：[输入阻塞报告](validation/phase-35.md)、[输入清单](validation/phase-35-input-requirements.json)、[前置核验](validation/phase-35-preflight.json)、[台账更新](validation/phase-35-ledger-update.md)。

#### Acceptance Criteria

- [x] 节点缺文件/错摘要/离线时不能激活；幂等重试不覆盖已安装异版本。
- [x] 认证/TLS/目标白名单有效，未完成节点不接新版本流量；旧版本可回退。

#### Risks

- 验收限定本机隔离多节点；使用独立目标卷与测试CA/凭据。真实跨机/生产拓扑和运维接入未验证，不能外推。

#### Dependencies

Phase 5、28、32、34必须COMPLETED。

#### Notes

条件扩展，初始未触发；它管理ZIP文件送达，不负责镜像拉取或容器启停。

2026-10-19收到明确Phase35执行请求。依赖5/28/32/34已完成；ZIP跨机/无共享存储事实、目标白名单与拓扑/CA/secretRef仍未提供，故输入核对BLOCKED，尚未满足条件触发。已请求补齐或明确隔离多节点演练范围；未实现分发组件/迁移/任务UI，未运行节点验收，不计为通过。主线完成结论及36/37未触发保持。

2026-10-20用户已明确授权“仅在本机隔离多节点环境实现并演练 ZIP 无共享存储分发”。此范围作为后续Phase35恢复依据；先完成当前Phase36，再单独恢复35，不并行标记第二个IN_PROGRESS。原输入阻塞报告保持历史证据，真实受管主机与生产分发不在授权范围内。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

2026-10-20恢复：用户已明确“仅在本机隔离多节点环境实现并演练 ZIP 无共享存储分发”；Phase36已独立验收完成，现单独触发35并进入IN_PROGRESS。采用两个独立节点/独立卷，平台安全解包后的规范产物经受认证HTTPS送达，逐节点摘要/原子安装/可信回执与实时核验后再公共预检及显式激活/回滚。无跨节点共享发布目录；独立Rust分发组件不操作Docker、不接受任意URL/主机/shell。仅演练控制器创建本地一次性容器和测试CA；真实远端与生产不在本次范围。旧阻塞报告与Phase36实施证据均保持历史不可变。

2026-10-20本机隔离验收完成：独立Rust artifact-agent/协议/0006/API/UI，两个独立节点卷；逐节点HTTPS、摘要、回执和实时核验后再公共预检及显式切换。正式15项多节点检查与三浏览器、fast 82步/1133通过、原发布/ZIP/升级17项全部通过，owner资源清理，174份历史证据及旧迁移保持。证据：[实施报告](validation/phase-35-implementation.md)、[运行结果](validation/phase-35-implementation-runtime.json)、[输入](validation/phase-35-implementation-inputs.json)、[保留校验](validation/phase-35-implementation-preservation.json)、[台账建议](validation/phase-35-implementation-ledger-update.md)。真实远程/生产不在本次验收范围。

<a id="phase-36"></a>
### Phase 36: 条件扩展：平台受控镜像部署与容器生命周期

#### Status

COMPLETED

#### Goal

用受控Rustagent和任务协议部署固定镜像版本，在健康与公开预检后再允许激活。

#### Scope

- 可信registry+digest、target、服务端模板/secretRef、部署任务/节点回执、Nginx路由与安全清理。

#### Out of Scope

- 平台API不挂Docker socket，不接受任意Docker参数、网络/卷/命令，不自动部署业务DB升级。

#### Inputs

- 方案编号：D03；本台账全局约束及源方案对应章节。
- 外部CI镜像接入已验收、Rust管理权限与审计、目标容器平台/凭据和Nginx策略。

#### Execution Requirements

- 新API/表保存image metadata，不向旧RegisterBody塞未知字段；失败不切active。
- 部署意图→版本容器/路由→健康/摘要回执→公共预检→允许激活；旧资源按引用保留。

#### Expected Outputs

- Rust镜像发布agent、受控任务API/UI、审计/恢复/版本清理及故障演练。
- 当前实施证据：[验收报告](validation/phase-36-implementation.md)、[输入](validation/phase-36-implementation-inputs.json)、[运行索引](validation/phase-36-implementation-runtime.json)、[保护核对](validation/phase-36-implementation-preservation.json)。
- 历史输入阻塞证据（不代表当前状态）：[输入阻塞报告](validation/phase-36.md)、[输入清单](validation/phase-36-input-requirements.json)、[前置核验](validation/phase-36-preflight.json)、[台账更新](validation/phase-36-ledger-update.md)。

#### Acceptance Criteria

- [x] 不可信仓库/可变无digest引用/越界目标/任意命令被拒绝。
- [x] 坏镜像/不健康/路由失败时旧版本继续可用；重试幂等、回执可信。
- [x] 回滚与保留策略不删除active/previous/其他环境引用或在途任务资源。

#### Risks

- 基础设施控制权限较高；已限定一次性本机隔离目标。真实受管主机/生产registry、权限和运维边界未接入，不能外推为生产验收。

#### Dependencies

Phase 28、32、34必须COMPLETED。

#### Notes

初始未触发，不是每微应用镜像接入的前提；可以长期用CI部署+平台注册。

触发条件：用户明确要求在平台中一键拉镜像、启停容器/配置路由，并提供受管目标和权限边界。

已收到明确执行请求，但受管目标和权限边界尚未提供，触发条件未完整满足；记录输入BLOCKED，不随主线自动实施。

2026-10-19前置28/32/34核验完成；Phase35不是本阶段依赖。已询问仅本机隔离演练或真实受管目标配置，当前缺target/registry/template/secretRef/Nginx权限输入。未实现agent/API/UI/迁移，三项运行验收均NOT_RUN；主线与其他阶段状态保持。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---


2026-10-20恢复：用户已明确选择“仅本机隔离实现与演练”。本地一次性Docker目标、私有测试registry/CA、固定非root静态模板与受控Nginx路由作为测试配置，Phase36触发并实施；无真实受管主机/外部发布/生产权限。原2026-10-19阻塞报告保留，后续实现报告使用phase-36-implementation.md，避免改写历史证据。

2026-10-20实现验收：81步fast/1127通过、18项实际部署/3浏览器、17项发布与ZIP/升级回归均PASS，失败与跳过均0。含URL别名门禁、CLEANING期间新引用拒绝、审计失败原子性与owner清理。正式输入964项2ea7c8ff…，源码与宿主不变；本阶段COMPLETED。[实施报告](validation/phase-36-implementation.md)、[输入](validation/phase-36-implementation-inputs.json)、[运行索引](validation/phase-36-implementation-runtime.json)、[保护核对](validation/phase-36-implementation-preservation.json)、[台账更新](validation/phase-36-implementation-ledger-update.md)。旧阻塞报告保持历史证据。

<a id="phase-37"></a>
### Phase 37: 条件分支：原生 WS 前后端协议联合适配

#### Status

NOT_STARTED

#### Goal

仅当22实证得到NATIVE_WS_REQUIRED时，为Rust通知迁移准备可验收的原生WS传输与宿主客户端。

#### Scope

- Rust Axum WS协议适配/传输fixture、宿主客户端和版本化路径、消息schema/认证/心跳/重连/背压。
- Nginx代理、旧宿主升级提示/切流窗口、HTTP收件箱补拉衔接与协议测试。

#### Out of Scope

- 不重做通知持久化/PgListener/有界调度（23完成），不声明socket.io-client直连兼容，不靠常驻Node维持协议。

#### Inputs

- 22真实不兼容证据与NATIVE_WS_REQUIRED记录，6消息/身份契约、10共享组织语义、3Nginx基础、当前通知/收件箱API。

#### Execution Requirements

- 仅22技术失败结论触发；环境缺失不能替代证据。触发记录写入指针/决策，成为23必需前置。
- 浏览器连接后短超时首帧认证，使用宿主平台会话，不把token放URL；认证前零业务消息，重连重新验证。
- 明确消息版本、heartbeat/close/error、退避抖动、帧限额、并发/背压、到期和上下文切换的断开。
- Nginx新路径与前端适配一致；重连HTTP补拉不丢持久消息，未实现时不能宣称自动polling回退。
- 不能双协议兼容时安排受控旧页面升级窗口和提示；本阶段不提前切换正式生产通知。

#### Expected Outputs

- Rust原生WS适配/fixture与宿主客户端、版本化消息/路径/代理配置及协议迁移说明。
- 认证/异常/重连/上下文/背压和补拉契约回归，旧页面兼容限制与切流计划。
- 实施证据：[条件核验报告](validation/phase-37.md)、[核验摘要](validation/phase-37-preflight.json)、[台账更新记录](validation/phase-37-ledger-update.md)。这些是未适用核验证据，不是原生WS实现验收证据。

#### Acceptance Criteria

- [ ] 真实浏览器与Rust WS在HTTPS/Nginx下互通；错误/超时/过期/跨组织凭据不能接收消息。
- [ ] 首帧认证、心跳、断开/重连、上下文切换、帧限制/背压行为可复现，无URL/token日志泄漏。
- [ ] 重连通过HTTP收件箱补齐，协议变更不会被当作Socket.IO无感兼容。
- [ ] 23/34按WS分支同步门禁，Node最终退出和旧宿主升级窗口明确，37未完成不能进入23。

#### Risks

- 切协议影响已打开宿主，必须区分传输适配完成与完整通知服务切流；未经测试不承诺旧客户端透明兼容。

#### Dependencies

Phase 3、4、6、10、22必须COMPLETED。

#### Notes

初始未触发且NOT_STARTED。SOCKET_IO_COMPATIBLE时本阶段保持未触发，不影响主线；触发后成为必需门禁，无需重复为同一fallback选择确认。

2026-10-19收到明确执行请求后核验：Phase22为SOCKET_IO_COMPATIBLE，Phase34真实Socket.IO矩阵44/44通过且与当前935项源码输入相同。原生WS分支触发条件不成立，执行请求BLOCKED_NOT_APPLICABLE；阶段保持NOT_STARTED、未触发，未实现/未运行四项WS分支验收。不得改写22兼容结论或将未触发标COMPLETED。35/36记录及当前执行指针保持。

状态/证据更新时同步摘要表，未通过全部必需验收不得标记COMPLETED。

---

## 5. Current Execution Pointer（当前执行指针）

```yaml
current_phase: 35
current_status: COMPLETED
current_objective: Phase35本机隔离多节点ZIP无共享存储分发已验收完成；无自动下一阶段。
last_completed_phase: 35
last_execution_authorized_phases: [35]
implementation_authorized_in_this_request: true
authorization_scope: 用户明确授权本机隔离多节点ZIP无共享存储分发；仅一次性目标/独立卷/测试CA与凭据，平台与分发组件不操作Docker，无任意URL或shell，无真实远端/生产/37。
next_action_requires_new_execution_request: false
default_scope: 仅当前或用户明确指定阶段，满足依赖后执行；不自动跨阶段。
phase_numbering_version: v0.20
phase_numbering_map: docs/phase-numbering-map.md
mainline_end_phase: 34
mainline_phases: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34]
mainline_phase_count: 34
default_execution_order: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34]
conditional_phases: [35, 36, 37]
conditional_phases_triggered: [35, 36]
conditional_phases_requested: [35, 36]
realtime_protocol_result: SOCKET_IO_COMPATIBLE
ws_gate: 若Phase22结果为NATIVE_WS_REQUIRED，在Phase23之前插入Phase37；Phase34同样要求其完成。
```

**Current Phase：Phase 35，COMPLETED。Current Objective：本机隔离多节点 ZIP 无共享存储分发验收完成。** 36保持COMPLETED，37未触发。

**Current Restrictions：** 35/36仅本机隔离实现与演练完成；真实目标、外部发布、生产操作和未触发37不在本次范围。

完成本阶段全部实现和隔离必需验收后同步终态；不自动执行其他阶段。

### 执行与交接规则

1. 开始前核对本台账、适用AGENTS、当前Git工作树、依赖状态与Phase输入，记录实际起点。不得把旧摘要直接当当前事实。
2. 每个实现阶段只修改该阶段所需能力；发现必需但未声明的协议/迁移依赖，先更新台账依赖/决策，再推进，不把独立功能偷偷塞入当前阶段。
3. 测试使用隔离目标与受控环境；不默认复用现有私有.env里的数据库作为测试库，不打印配置秘密。实际生产/外部registry/外部告警操作不由创建台账或普通本地执行指令隐含授权。
4. 终态报告说明变更、实际命令、pass/fail/skip、未验收项、回退和阶段输出。证据缺失时保持真实状态；BLOCKED必须写具体缺少的环境/信息、已完成部分和解除方法。
5. 完成全部必需验收后，详情和汇总同步COMPLETED，追加Change Log；按汇总表/声明的default_execution_order选择前置已完成的下个阶段，不按编号大小猜顺序。22触发37时先插入37门禁再23；35/36不自动进入主线。
6. 用户请求并行时只并行无冲突且依赖已完成阶段；数据库迁移编号、公共契约和台账写入统一协调。默认逐阶段执行。
7. 其余33个主线阶段（1–33）及已触发37完成，且Phase 34的必需验收全部通过后，同步将Phase 34标COMPLETED并记录“主线验收完成，无自动下一阶段”；未被明确请求且未触发的35/36/37保持NOT_STARTED，不为清空表格伪标COMPLETED。明确请求但缺触发输入时按Status and Evidence Rules记录BLOCKED。

### 历史证据与待验收边界

| 证据 | 日期/来源 | 本台账如何使用 |
| --- | --- | --- |
| 前端build通过，主入口865.74kB/gzip268.57kB | 2026-10-14审查 | 仅初始参考，Phase1/29重取同口径数据 |
| 88项契约测试中86通过、2跳过 | 2026-10-14审查 | 证明当时基础能力，不计入新Rust/Vite/容器验收 |
| Phase1干净npm ci、lint/typecheck、选定70/70契约零skip；Rust check/default test/doc与已知失败分项记录 | 2026-10-15，[phase-01](validation/phase-01.md) | 当前工程入口基线；与旧88项集合不同，不证明业务迁移/真实PG/浏览器/容器通过 |
| Phase2三个示例独立ci/build；13项开发/5项交付/70项契约，浏览器兼容/版本/聊天组通过；旧全量管理fixture失败 | 2026-10-15，[phase-02](validation/phase-02.md) | Vite/相对产物/404/生命周期及受控启动实证；旧完整browser仍18PASS/1FAIL/2NOT_RUN，非真实平台/PG/Nginx/容器验收 |
| 配置测试缺deploy/native/render.mjs；lint缺ESLint | 2026-10-14审查 | Phase1/3需处理的明确输入，不当成新实现失败记录 |
| TRUSTED永久pending清理不结束 | 2026-10-14最小实验 | Phase7正式回归的复现依据，不表示已修复 |
| Phase4干净Linux fast19步、PG21、三浏览器143及最终部门11、三OCI/HTTPS七组、最终工具41 | 2026-10-15，[phase-04](validation/phase-04.md) | 当前manifest实测通过；各快照及修正子集分列，不证明完整真实业务、远端workflow/registry/生产部署 |
| Phase5 Windows安装17、Linux安装16/维护3/正式PG27/完整契约99/工具48、真实production HTTPS9 | 2026-10-15，[phase-05](validation/phase-05.md) | 最终403项输入与294项原生构建零漂移、幂等/冲突/未知提交/回滚及清理实证；初次包装器FAIL与同快照99补验分列，未宣称远端workflow/registry/生产部署 |
| Phase7 runtime20+14、Windows29、完整Linux fast23/契约189、三产品各151 | 2026-10-15，[phase-07](validation/phase-07.md)及运行JSON | 同一473项4a2f9c…输入下四阶段截止、iframe边界/可信锁存/迟到结果/模块重求值/整页恢复通过；11路径变化，后端/SDK/业务/锁不变，r1受控中断与scratch策略拒绝另列，6项运行资源空查 |
| 未来Rust业务服务、跨机agent、完整业务PG/浏览器/灾难恢复 | 当前无完成证据 | 对应后续Phase仍NOT_STARTED；不得继承历史现网成功或本次fixture结果 |

## 6. Decision Log（决策记录）

编号说明：下表截至v0.19的历史决策行保留原文和旧编号，按[编号对照](phase-numbering-map.md)阅读；其后的已采纳基线、覆盖矩阵和开放问题使用v0.20新编号。

| Date | Decision | Reason | Impact |
| --- | --- | --- | --- |
| 2026-10-19 | 工单/门店镜像采用既有六字段URL登记，来源/digest不变性由Rust CLI/CI权威持久清单独立维护；镜像、健康、预检、激活与授权分别验收。 | 真实隔离registry/Compose及三个浏览器全部通过，平台不扩Docker控制或伪造metadata。 | Phase28 COMPLETED；默认API builder、远端发行/生产未执行，下一29 NOT_STARTED。 |
| 2026-10-18 | 以工单和门店两个真实Rust消费者的成员HTTP、授权和共享会话结果作为后续公共能力收敛输入。 | 两方复用同一SDK和平台协议，同时保留DTO、owner和未知写恢复差异。 | Phase27完成；不提前抽公共框架，Phase28待新的明确请求。 |
| 2026-10-17 | Phase23正式通知入口迁独立Rust，保持Socket.IO；最小DB读取+平台实时复验+有界分区/HTTP/Engine/TCP与全TCP退出 | Linux真实平台/PG/双页、原始peer及镜像验收全部通过；持久收件箱保持事实来源 | 旧Node后台/legacy通知/测试导入退役；HTTP补拉不宣称自动polling回退，37未触发；用户要求整个执行指针不改 |
| 2026-10-17 | Phase25采用独立Rust管理CLI，默认离线/显式apply；grant只经正式API做批准的正向增量，scope差异拒绝 | 保留旧工具幂等、归属与秘密边界，避免SQL绕过策略/版本/审计；实际API无CAS限制明确 | 旧5个Node管理文件退役，前端构建/测试继续Node；未知/部分成功先对账，用户要求整个执行指针不改 |
| 2026-10-16 | Phase22冻结SOCKET_IO_COMPATIBLE，socketioxide0.18.7/engineioxide0.17.7（MIT）；37未触发 | 当前宿主与聊天客户端在Rust peer上完成全部必需矩阵；缺环境未当不兼容 | 23保持Socket.IO及notification1.0；真实平台/PG/运行边界另验；用户要求本次不改执行指针 |
| 2026-10-14 | 前端及全部示例前端统一Vite，禁止Bun | 用户明确要求 | 迁移静态演示/工单/演练，不仅更改命令文本 |
| 2026-10-14 | 后台统一Rust，禁止Node后台/Next.js | 用户明确要求 | 工单、通知、OAuth及后台参考迁移；Node仅前端工具与测试驱动 |
| 2026-10-14 | Nginx为唯一网关/静态服务，禁止Caddy | 用户明确要求 | Docker/Compose/原生脚本/CI/活跃文档同步退出旧网关 |
| 2026-10-14 | 示例必须真实前后端分离 | 用户明确要求 | 独立Vite前端、Rust API和业务库；不是只有静态页/假API |
| 2026-10-14 | 镜像接入保持HTML/IFRAME/SDK及不可变版本URL | 代码分析与设计结论 | 不因Docker化改变身份/授权；每版本旧资源仍可达 |
| 2026-10-15 | 推荐宿主静态Nginx镜像+入口Nginx、共享持久版本目录 | 已讨论的简化部署方案，台账可逆默认 | 不等于用户强制所有微应用镜像化；不默认引入agent |
| 2026-10-15 | 首期每微镜像接入采用外部CI部署+平台入口登记 | 现有Rust register/preflight/activate可复用 | 管理端补来源表单；镜像操作不进入平台后台 |
| 2026-10-15 | 跨机分发与容器生命周期分开，列为条件Phase33/34 | 两者适用条件和控制边界不同 | 主线不自动执行；分别记录触发依据 |
| 2026-10-15 | 迁Rust先保持内省+context两次实时授权 | 先证明兼容，再独立优化 | Phase17不删校验，Phase28再做等价优化 |
| 2026-10-15 | 通知先验证Socket.IO兼容，候选库未冻结 | 现有客户端不是裸WS | Phase22是Phase23强依赖，失败不得默换协议 |
| 2026-10-15 | 采用32主线+2条件的具体执行阶段 | 多独立部署单元和安全/数据边界需分别验收 | 四个大里程碑只导航；不产生跨阶段自动实施授权 |
| 2026-10-15 | 初始实现状态全为NOT_STARTED | 用户本次仅要求创建台账 | 当前指针Phase1，历史测试不计完成 |
| 2026-10-15 | v0.2采用镜像推荐、ZIP兼容、外部URL统一登记 | 用户要求按Q01–Q13建议完善文档再优化台账 | 更新并取代上方“共享目录主线”的初始推荐；共享卷仅用于ZIP兼容，旧行保留作历史 |
| 2026-10-15 | GitHub Actions构建、LinuxDocker/Compose部署、入口独立Nginx容器 | 用户明确构建/运行环境与入口位置 | 同机私网为模板，真实registry/CPU/主机/DNS/TLS仍待提供；平台首期不控Docker |
| 2026-10-15 | 共享登录、共享当前租户/组织 | 按建议采用首期语义，避免提前引入独立页会话 | Phase10统一刷新/切换/退出；21/23/36消费，双页草稿隔离 |
| 2026-10-15 | 首版grant写入要求平台根上下文+独立管理权限 | 管理权与业务数据访问权独立 | Phase14/15/25一致，组织管理员无隐式写权；委派cap/通用审批以后明确扩展 |
| 2026-10-15 | Socket.IO实证失败可转原生WS联合适配 | 原生WS与Socket.IO协议不同 | 22输出有证据选型；NATIVE_WS_REQUIRED触发37为23/32必需门禁，无环境不算技术失败 |
| 2026-10-15 | 第二真实消费者为简单门店系统 | 用户指定门店方向并要求按建议完善 | 新增35/36主线，先工单窄策略→门店实证→28最终通用化；保持工单完整范围 |
| 2026-10-15 | 保留/性能/恢复数值采用候选、先测量 | 未提供真实生产规模、预算和正式政策 | 隔离dry-run/基线可推进，生产清理及SLA不自动生效 |
| 2026-10-15 | 保留阶段标识，主线34个、条件3个 | 原阶段结构保留并新增门店与WS分支 | 新主线35/36在26/28之前，37条件插在22/23之间；最终32；全部仍NOT_STARTED |
| 2026-10-15 | 按用户执行指令完成Phase1并同步台账，未自动跨阶段 | 本次用户明确要求执行Phase1；台账要求每Phase同步状态/证据/指针 | Phase1 COMPLETED，指针2 NOT_STARTED；创建时的未授权文字作为历史保留 |
| 2026-10-15 | 固定Node24.18.0/npm11.16.0/Rust1.97.0并对齐CI/平台与宿主构建声明 | 选择本次实际安装、干净安装及检查成功的工具组合 | 不改变单crate形态或生产运行阶段，不宣称Linux/Docker已验证；源码SHA和工作树摘要绑定证据 |
| 2026-10-15 | lint保留可见既有warning，必需无DB契约与有条件扩展检查分别记录 | 避免Phase1批量改业务/关闭全局规则或以skip充当通过 | 必需70项零skip；Rust fmt/Clippy、依赖审计、缺deploy失败和未运行环境明确保留为后续输入 |
| 2026-10-15 | Phase2保留classic/TRUSTED兼容生命周期，独立示例精确锁Vite8.3.4，不强升宿主Vite6 | 既有HTML loader/取消语义需要保留，示例构建与宿主依赖独立 | 去开发ESM注入、相对资源与真实404；默认opaque iframe仍须SDK，不增加权限 |
| 2026-10-15 | 根默认仅前端、前台受控启停，readiness为绝对总预算 | 移除个人路径/自动PG/后台逃逸并实证慢响应及父退出清理 | Node仅为前端调度工具；显式旧后台选项不表示Rust迁移完成，Linux清理尚未实测 |
| 2026-10-15 | 统一构建先示例→同步dist→宿主，保留根npm锁替代Bun锁 | 避免发布public源码或让宿主打入旧静态副本 | 独立dist/版本资源可复核，旧版本不删除；根旧后台参考依赖仍有明确退出阶段 |
| 2026-10-15 | 旧管理UI浏览器fixture失败独立记录，不扩大Phase2重写发布UI | 当前fixture路由/表单陈旧，相关业务UI非本阶段改动 | 阶段选定兼容/版本组真实通过；全量18PASS/1FAIL/2NOT_RUN留给Phase4/发布阶段修订验收 |
| 2026-10-15 | Phase3采用独立入口/宿主Nginx、默认无产物卷，ZIP及旧Node服务各显式override | 用户授权Phase3，实际Linux容器边界已验收；不假称业务Rust迁移完成 | 移除Caddy入口/卷/测试，版本路由由部署方明确维护；平台不持Docker socket |
| 2026-10-15 | 当前/历史assets分列，更新和回退从在线不可变镜像导入完整资产 | 必须证明旧真实入口已退出currentAssets但旧URL仍在线；只存旧镜像不够 | 实测新入口变化、旧字节200/摘要一致；无运行时共享卷，不自动清理历史 |
| 2026-10-15 | WSL隔离Docker实证与Windows原生构建分别报告 | 用户确认WSL可用；网络超时经官方镜像正常TLS导入/构建级代理恢复 | 289输入/141产物原生审计及真实Docker七组通过；无真实外部发布、无业务DB，传输不等于Socket.IO选型 |
| 2026-10-15 | Phase4按当前显式manifest分层，必需失败/skip/零测试/缺报告一律拒绝 | 平台bin-only、未来服务尚未迁移，不能用空workspace或默认ignored换绿色 | 选定PG21、浏览器143、fast与真实delivery分别计数；已有fmt/Clippy/ignored债务独立保留 |
| 2026-10-15 | Playwright归项目锁定依赖，真实Chrome/Edge分别报告，最终源码按受影响套件补证 | 移除个人缓存；不同运行快照必须可追踪，不把修正后子集冒充整轮重跑 | 三Linux完整143与最终部门11及等待回归分列；最近两稳定版及Firefox/Safari产品支持未宣称完成 |
| 2026-10-15 | 候选保存完整校验OCI archive，外部发布与目标部署另记状态 | 当前只授权Phase4隔离验收，生产环境/registry值仍未提供 | 三镜像架构/HEAD/buildId/内容摘要及平台ELF真实验证；无外部推送或生产部署，Phase4完成后指针5 NOT_STARTED |
| 2026-10-15 | ZIP同应用+版本绑定规范artifact digest，跨环境复用，普通URL不得旁路绑定 | 同版本字节与定义必须不可变，压缩格式不应改变身份 | Linux/Windows禁止覆盖安装；相同包幂等/提升，异产物409；旧未绑定URL保留既有路径 |
| 2026-10-15 | DB结果未知时保留正式文件并查询四态/全部环境；仅维护可证明ownership的私有stage | 错误响应不能证明COMMIT未生效或无人引用 | 永不失败补偿删除公开版本；共享/独占锁协调上传与离线维护，新旧writer先停写排空升级 |
| 2026-10-15 | 真实首启nullable菜单parent最小修复，同快照补齐契约构建前置后重验 | production初始化阻断HTTPS验收；干净镜像无预生成示例产物 | NULL父节点/重启PG回归与全量地区初始化通过；原包装器FAIL保留，正式PG27/完整99/HTTPS9各实证，Phase5完成后指针6 NOT_STARTED |
| 2026-10-15 | Phase6首次登记SDK1.0.0仓库候选，SDK版本与context1.0/iframe2.0/notification1.0独立 | 不虚构发行历史或改变授权语义 | 保留实际versionless2.0基线和旧静态回退；未知声明版本明确拒绝，90天仍为候选窗口且无下线日 |
| 2026-10-15 | schema+版本登记生成公开TS/Rust与classic/ESM，CI只CHECK不先同步 | 共享边界与vendor必须持续可核对，避免构建掩盖漂移 | 全量30目标、Docker纯Node scoped25；宿主+三个前端消费同版，平台仅context输出使用纯wire DTO，无根workspace或新依赖 |
| 2026-10-15 | Phase6以完整最终fast及三产品验收收口，不改写早轮FAIL或合并不同快照 | 旧401竞态、Vite/DOM fixture及optional包遗漏均须如实追踪 | 472最终输入9c967…；PG相关输入与343原生输入逐项不变，资源空查询与lease退出0；Phase6 COMPLETED，指针7 NOT_STARTED |
| 2026-10-15 | Phase7四阶段各10秒执行预算，无法证明结束即锁存页面可信域，完整重载恢复 | 超时/取消race不能证明脚本或thenable结束；同页新模块也不能丢失旧活动队列 | iframe独立销毁；另一个TRUSTED不执行，首次旧开发页升级先刷新，不能强杀同步循环或撤回已交付token |
| 2026-10-15 | 真实脚本Promise受域管理，实际调用前复核取消，跨模块故障使用稳定品牌 | loader race、微任务空隙及开发模块重实例化均由实际负例/独立探针复现 | 保留正常串行和最终卸载；SDK/后端/授权协议不改，Phase8更新语义与Phase9生产导航仍独立待执行 |
| 2026-10-15 | Phase7按最终473项r2完整验收收口，保留旧r1受控中断和本地scratch策略拒绝 | 不把部分报告改成通过、不绕过自动执行策略或合并不同快照 | runtime20+14、Windows29、fast23/契约189及三产品各151通过；Docker资源/lease清理与本地缓存分列，Phase7 COMPLETED，指针8 NOT_STARTED |

| 2026-10-15 | Phase8使用带类型完整授权投影并缓存签名/source signal，展示复用稳定controller | contextVersion和对象引用不足，原位修改及组合signal不稳定均可漏判/误判 | 宿主/bridge/client/现有工单统一，续期与撤权拒绝旧响应；无update纯主题延后应用、路由仍串行fallback，wire不改 |
| 2026-10-15 | context为完整入口元组权威，profile变化只触发一次复核；临时恢复每次重取token | 激活/回滚交错不得混合资源与props；平台失联时须先撤下旧授权 | 仅503/网络错误500/1000/2000ms最多3次，尝试5秒/过程20秒；真实失权与可信永久故障保持拒绝 |
| 2026-10-15 | Phase8以478项freeze3完整门禁收口，原始旧PASS/FAIL均保留且不混算 | 两处旧fixture需与更早入口校验一致，部分/不同快照不能视作最终PASS | 专项51/97、Windows47、fast23/契约221、三产品各169通过，资源/lease空查；Phase8 COMPLETED，指针9 NOT_STARTED，无外部发行或生产部署 |

| 2026-10-15 | Phase9同epoch授权查询工厂与统一显式路由策略，个人工作台/资料/本人通知允许登录用户 | 原key/signal/新鲜度及显示/直达存在多套口径，未知或旧error数据不能继续授权 | 成功明确授权不受无关应用菜单失败阻断，保持全局cancel/clear和数据grant独立；不声称旧key已证明串租户 |
| 2026-10-15 | 共享选择代次，AppLayout唯一负责epoch/path/匹配owner推断 | A→B、A→B→A、布局卸载及旧micro route effect会覆盖新意图，同epoch新增授权也需正确推断 | 删除子层重复写入、保留selected-ID门禁及全部执行/授权/故障域，旧结果不导航 |
| 2026-10-15 | 工单创建固定ticket-create，7key/UUID稳定且read/create改显式MENU | tickets/new会与详情read动态模式重叠，单加菜单不足以拒绝创建 | 前端/manifest/校验器/授权计划与SQL守卫同步；旧形状需人工采用，不自动修改数据库或授权 |
| 2026-10-15 | 六份既有部署工具精确加入源范围，最终493项freeze4完整验收 | 干净快照原不含这些真实工具依赖；部分/不同源/skip不能作为最终PASS | 39路径含6项范围扩展，专项22/24/70/16、Windows183、fast23/契约311、三产品各183与资源审计通过；指针10 NOT_STARTED |

| 2026-10-16 | Phase10单原子record/Web Locks统一共享登录与组织，版本通知不传凭据 | 两键和每页单飞会重复消费一次性refresh；锁内必须重读 | 旧键只单向迁移，缺锁/storage明确拒绝，BC缺失有真实兼容路径 |
| 2026-10-16 | 持久intent，发送后未知或owner关闭隔离并要求重新登录 | 无服务端未提交/幂等恢复契约，不能安全重耗旧值 | 20秒执行预算不证明事务回滚；条件提交不删新登录，后端协议不改 |
| 2026-10-16 | 调用期fence与render逻辑session/epoch绑定 | 等锁后重捕身份可把旧草稿发往新组织，正常access轮换会误卸工单 | 平台API/通知/消费者闭合，容器7处窄替换保留其余执行域，HTTP fallback与真实草稿回归通过 |
| 2026-10-16 | Chrome/Edge155与154官方版本实测，旧宿主bundle关闭/重载采用 | Chromium不等于第二稳定产品版，旧bundle不认识原子协议 | 完整600与兼容34分列；回滚旧宿主重新登录，499项freeze1/362契约，指针11 NOT_STARTED |

| 2026-10-16 | Phase11采纳user→client→credential原子消费/签发/审计、随机jti和storage_version兼容 | 消费失败/半套token、旧原文UPDATE约束及实时用户失权需要同一状态机 | PG41/fast24同502输入通过；旧存储不清理、机器身份独立、未知提交重新授权，指针推进12 |
| 2026-10-16 | Phase12采用进程内硬容量/TTL、双许可取消/截止背压和真实可信代理来源，原策略不降低 | 模板单API不等于多实例全局配额；全HTTP密码链需共享成本边界 | cap/TTL/clock与独立副本/重启语义、24混合HTTP健康、PG46/fast27同505输入通过；N副本按N倍评估，未来共享需求另验 |

| 2026-10-16 | Phase14冻结独立grant读/写权限与服务端显式策略闭集 | 根上下文不隐式赋写权，业务scope不是委派cap，默认空策略拒绝 | typed API/差量与预览/版本冲突/事务审计；无新增委派或UI |

| 2026-10-16 | Phase15独立数据授权页+同边界窄发现、预览提交与关联审计 | grant-only管理者不应被迫取得宽泛用户/应用管理权，UI不能推导或扩大服务端范围 | 独立read菜单与可写提示，fence/版本/请求关联；真实浏览器+隔离PG闭环验收 |

| 2026-10-16 | Phase15采用独立MENU/read与窄发现、精确预览及未知结果恢复 | UI沿用Phase14边界，业务功能与直接成员数据范围分开，审计独立授权 | 五浏览器80项真实闭环；SELF1/ORG2/撤销403与四次审计关联；无审批/委派扩展 |
| 2026-10-16 | 既有主题测试同步修复仅按受影响套件补证 | r2 Chromium失败来自提前读取保存中状态；537/538输入逐字节相同 | 保留r2整体FAIL与596pass/1fail/3未运行，三个当前产品各补验完整11项；不冒称全门禁或600项一次通过 |

| 2026-10-16 | v0.20按用户要求重新编号 | 主线编号应与默认执行顺序一致 | 主线1–34、条件35–37；映射依赖/锚点/预期证据路径/现状文档，不执行新阶段，不改历史验证记录 |

| 2026-10-16 | Phase16采用独立工单Rust crate与事务内旧迁移接管 | 旧SHA256迁移不能直接冒充SQLx checksum或重跑CREATE | 同旧锁、实际catalog/trigger/索引/约束核对后记录SQLx SHA384，失败不开放ready；保留Node输入、不提前实现CRUD |

| 2026-10-17 | Phase17对两段授权采用共享总预算与即刻并发背压 | 将平台调用和响应流纳入同一可验证deadline，限制授权在途数量 | 旧每次3秒收紧为内省+context合计默认3秒（100–10000ms），默认32并发、饱和503、不排队；HTTP总预算15秒，配置与验证明确该变化 |
| 2026-10-17 | Phase17列表/count及CSV容量/数据使用各自同一只读REPEATABLE READ快照 | 并发写不能在容量校验和读取间改变被验证的数据集合或字段大小 | 所有查询共用完整scope，读取后复核授权到期；不提前实现写入，也不改旧基线DDL |
| 2026-10-17 | Phase17选择明确退役Node后台授权参考 | 台账允许迁Rust或退役；真实工单实现已承担可运行Rust参考，未出现第二使用方需要通用crate | examples/app-backend仅保留历史兼容测试，活动文档指向Rust授权/读取实现，不新增Node运行路线 |

| 2026-10-19 | Phase29采用受保护壳lazy、面板按需及Outlet/面板独立恢复边界；仅拆应用创建/编辑用例 | 登录生产模块图实证包含壳/面板/Socket.IO；原应用页混合校验、创建/编辑、列表删除与发布引导 | 快捷键移入壳，微应用epoch/appCode粒度保持；render可局部重试，chunk失败只显式reload，自动次数为0。330000B初始传输/320000B压缩body、4184ms中位LCP/100ms受控交互为本地实验预算；生产SLO/线上INP仍未承诺 |

### 已采纳基线、候选目标与非默认选择

- 本地/隔离采用同一Linux主机的独立入口Nginx容器、私有网络/service DNS；真实主机/网络/TLS参数不虚构。镜像无共享文件卷，ZIP兼容API读写/静态Nginx只读；入口可以只代理。
- 产物格式与部署责任分离，镜像推荐、ZIP兼容、批准的外部HTTPS入口可直接接SDK；一个release一个来源/产物，固定公开版本URL。外部静态origin必须正确解析业务API base及CORS/CSP。
- 首期Chrome/Edge最近两稳定版为验收矩阵，SDK版本/能力清单明确，破坏性升级候选90天；Firefox/Safari需真实验收后扩展。共享当前组织是已采纳产品语义，不是开放选项。
- grant独立读/写权限在14契约内冻结；写必须实际平台根上下文，组织管理员只读范围仍校验；不以业务grant或scope排序推导委派能力。
- 不默认根Cargo workspace、消息中间件、灰度产品、平台一键Docker、物理租户分库、每页独立组织或生产自动清理。保留的初始历史建议不覆盖本节最新基线。

| 候选项 | 台账内可独立恢复的设计起点 | 验收/生效边界 |
| --- | --- | --- |
| 静态保留 | 至少30天且至少最近5版（并集） | active/previous/跨环境引用/在途任务始终保护；旧URL须在线，仓库有镜像不够 |
| 数据保留 | audit在线180天、普通通知90天、业务软删30天；过期凭据缓冲待定 | Phase 31先隔离dry-run，生产apply需正式政策；备份独立 |
| 业务API/页面 | 常规查询p95≤500ms/p99≤1s；LCP≤2.5s/INP≤200ms；非预期5xx<0.1% | 同硬件/网络/数据口径测量，非当前容量或生产SLA |
| 负载/撤权 | 初轮100并发活跃会话、10万代表性记录；撤权后新请求最新授权，前端候选20秒感知 | 区分在线/活跃/并发请求；不以长授权缓存换性能 |
| 试点恢复 | RPO24h/RTO4h | 多库/产物/密钥/路由完整隔离恢复，停写时间单列 |
| 生产恢复 | RPO15min/RTO1h | 必须WAL/PITR与异机恢复实测；每日dump不证明达标，未验收明确记录 |
| 环境 | GitHub Linux CI临时PG/浏览器/Docker；Linux预发布真实HTTPS/升级/恢复 | PR不取生产秘密；真实推送/部署/告警渠道另按已提供环境执行 |

### 源方案覆盖矩阵

| 编号 | 优化项 | 主线Phase | 条件Phase |
| --- | --- | --- | --- |
| D01 | 恢复部署文件与校准引用 | 3, 32, 33, 34 | - |
| D02 | Nginx独立发布/产物分发 | 5, 28, 34 | 35 |
| D03 | 前端静态镜像与入口Nginx | 3, 28, 34 | 36 |
| T01 | Vite/Rust与禁止项统一 | 1, 2, 3, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 34 | - |
| X01 | 完整前后端分离示例 | 14, 15, 16, 17, 18, 19, 20, 21, 25, 28, 34 | - |
| X02 | 简单门店系统与双消费者实证 | 14, 15, 19, 25, 26, 27, 28, 30, 34 | - |
| S01 | 普通OAuth状态机 | 11 | - |
| S02 | 登录与密码成本保护 | 12 | - |
| S03 | 多标签页会话 | 10 | - |
| M01 | 可信运行故障域 | 7 | - |
| M02 | 展示/授权/部署更新语义 | 8, 21 | - |
| M03 | 创建幂等 | 18, 21 | - |
| F01 | 菜单/路由/导航统一 | 9 | - |
| B01 | 授权成本与短事务 | 17, 18, 30 | - |
| B02 | 事务审计 | 13 | - |
| N01 | 通知有界调度与协议分支 | 22, 23, 32 | 37 |
| E01 | 工具链/CI | 1, 2, 4, 34 | - |
| F02 | 宿主加载/恢复/页面边界 | 29 | - |
| C01 | SDK与契约版本 | 6, 34 | - |
| B03 | Rust模块/能力边界 | 6, 11, 14, 19, 26, 27, 30 | - |
| O01 | 升级/保留/授权运维 | 14, 15, 16, 31, 33 | - |
| O02 | 观测/容量/发布恢复 | 23, 32, 33, 34 | - |

方案第12–15节的目标栈、推荐镜像/兼容ZIP/统一URL，以及工单/门店双项目由Phase2/3/5/14–28/30/34组合验收；ZIP跨机与平台容器控制仍为35/36条件扩展，协议失败分支为37。原第9节A/B/C/D只作里程碑。镜像登记Phase 28/CLI25不依赖ZIP安装5；最终Phase 34必须分别验收两条交付链。
| 2026-10-16 | Phase13沿用audit_logs，关键变更共用主事务并严格检查实际插入一行 | SQL成功也可能被trigger RETURN NULL跳过，不能commit后best-effort承诺原子 | 全调用链安全摘要/requestId/删除身份；统一进程饱和失败计数，普通诊断分类保留，无新迁移/消息总线 |
| 2026-10-16 | Phase13保持已有幂等及独立预检边界，requestId不是全局幂等键 | 无实质变化PUT不新增事件，失败回滚重试只留下成功事件；预检是独立持久观测 | ZIP未知commit继续对账，通知重复key不增投递/审计，指针与自身审计同tx；不扩token或grant协议 |
| 2026-10-16 | 新增必要父锁采用明确顺序，锁后另语句取计数或重新验证授权 | 防止client/app锁反转、等待前MVCC快照陈旧及组织授权写回已撤租户关系 | 保持用户优先、application→client及tenant→org，窄修onboarding兼容锁；真实交错证据范围见phase-13 |

| 2026-10-16 | Phase24 采用有界进程会话保存 verifier/device code/token，配置能力独立白名单解析，password 默认关闭；平台仅 test dev-dependency | 移除旧秘密/公开 verifier，避免配置流程名被当密码过滤；真实平台 fixture 与生产边界分离 | 开发 3010/3011，独立 HTTPS 非 root 镜像；Cargo 构建阶段补齐 path 解析，生产仍通过 HTTP；本次不改执行指针 |

### Phase30 已采纳决策

| Decision | Reason | Impact |
| --- | --- | --- |
| 一致聚合投影后新语句复核SHA-256 revision和token，最多重建一次 | 旧事务快照不能证明新撤销；全局版本行会引入串行锁点 | 无跨请求正向缓存，无新迁移；持续变化/预算耗尽503 |
| 工单combined默认，门店legacy默认；均显式可切换 | 调用数下降，但门店本轮耗时/冷尾收益不足，SQL累计执行时间也上升 | 不伪报全面提速；旧两HTTP协议与新快照检查并存 |
| 成员HTTP/DTO冻结，领域及SQL映射保持独立 | 两真实消费者尚不足以证明公共业务crate收益 | 单crate窄lib，无共享DB模型或根workspace |
| 平台page正数上限100万及checked offset，非法语法/default保持 | i64极大page不再溢出500 | 公共及真实HTTP回归进入fast/postgres |

### Phase31 已采纳决策

| Decision | Reason | Impact |
| --- | --- | --- |
| 仅静态发布只读试点，30天或最近5版并集保护，参数只加严 | Q08生产期限和旧页面生命周期未批准 | 所有目标仅人工审阅；apply始终不可用，其他表不purge |
| 未完成保护持久化，明确结束只移除本次UUID；取消/传输失败/结果未知保留 | 兼顾审计失败最终状态回滚和未知任务安全 | 无TTL/自动孤儿清理，既有409/CONFLICT契约不变 |
| 仅expand，冻结旧消费者实证兼容，不自行contract | 旧页面支持窗口未明确 | 旧checksum/UUID/编号/grants保持；旧ELF迁移器回退另验 |

### Phase32 实施边界决策

| Decision | Reason | Impact |
| --- | --- | --- |
| 三个独立Rust消费者共享纯std固定标签遥测原语，保留各自HTTP/SQL适配器 | 已有平台/工单/门店三个真实消费者，统一有界histogram和取消计数可避免标签漂移 | 单向依赖，无业务/DB模型，无根workspace；独立锁文件，仅路径包，无新增外部依赖；业务Docker源码构建上下文改仓库根以显式带入该小包 |
| Rust监控只发本地状态转换事件，外部接收渠道另配置 | Q09正式SLO/资源预算与实际告警渠道未知，不阻塞隔离候选测量 | 稳定DOWN/RECOVERED实测；100会话/10万记录及逐档增压，未达候选原样报告；不执行Phase33备份 |

### Phase32 验收采纳

| Decision | Reason | Impact |
| --- | --- | --- |
| 采用独立本地monitor，计数与真实收件区分；保留单主体/debug候选失败 | 全部原验收可在隔离环境证明，正式预算与渠道未知 | 100/150/200及10万记录，真实故障/私网/浏览器数据见phase-32；不承诺SLA，不实施Phase33 |

## 7. Open Questions（开放问题）

这里只保留未提供的环境值、未测结果和未定生产政策，Status仍用NOT_STARTED表示尚未收敛，不表示实现已启动。原Q01的推荐产物、Q12门店方向、Q13首期外部部署已收敛，详见第6节；其余问题缩小为下表实际余项。明确的模板/候选可先用于隔离验证，不需要重复询问已经采纳的选择。

| ID | Question | Impact | Owner | Status |
| --- | --- | --- | --- | --- |
| Q02 | 实际Linux目标是否同一主机、网络名称、DNS及trusted proxy地址？ | 入口独立容器已明确，3/28先用同机私网模板，真实环境不猜值 | 运维 | NOT_STARTED |
| Q03 | ZIP兼容启用哪些目标，实际卷类型/权限/备份怎样配置？ | 镜像无共享卷已明确；5同机兼容卷，只有ZIP跨机才考虑Phase 35 | 运维/实施负责人 | NOT_STARTED |
| Q04 | 实际域名、TLS/CA、镜像registry、CPU架构、目标机器和发行凭据？ | GitHub/Linux已明确，3/4/28发行前补真实值，可先隔离演练 | 运维 | NOT_STARTED |
| Q05 | 正式SDK发行支持/下线日及是否扩展Firefox/Safari？ | Phase10已实测Chrome155.0.8059.84/154.0.8037.92与Edge155.0.4283.60/154.0.4258.62的会话兼容；1.0.0仍为仓库候选，90天非发行历史或承诺下线日，未来版本不继承本轮通过 | 产品/实施负责人 | NOT_STARTED |
| Q06 | 未来是否需要审批/显式委派？ | Phase14已冻结system:data-grant:read/write与只读会话子树∩显式策略；首版已完成，未来审批/委派需另行明确需求，不阻塞主线 | 实施负责人/权限负责人 | NOT_STARTED |
| Q07 | Rust候选库的真实Socket.IO兼容结果是什么？ | Phase22已实证SOCKET_IO_COMPATIBLE：socketioxide0.18.7/engineioxide0.17.7与当前客户端；37未触发。后续版本需复验，真实平台/PG/调度归23。见phase-22证据。 | 实施负责人 | COMPLETED |
| Q08 | 生产静态/审计/通知/凭据/软删最终期限及旧页面支持窗口？ | Phase31隔离候选/协议已验收；生产期限/缓冲/contract窗口待定，apply不启用 | 产品/运维 | NOT_STARTED |
| Q09 | 实际业务规模/硬件/网络及正式性能、撤权和资源预算？ | Phase 29–32按候选口径测量，未达标如实记录，不以候选作SLA | 产品/运维/实施负责人 | NOT_STARTED |
| Q10 | 后续实际目标Linux预发布/灾备空间、隔离PG/证书及完整产品版本矩阵？ | Phase4已在授权WSL Linux完成隔离PG/三浏览器/HTTPS与静态恢复实测；28/33/34目标环境仍需明确，不继承为生产部署或灾备通过 | 运维 | NOT_STARTED |
| Q11 | 正式恢复profile/RPO/RTO、停写窗口、异机副本/密钥与告警渠道？ | Phase33完整隔离试点已通过；真实物理异机/外部密钥与生产15min/1h仍须WAL/PITR实测，不自动安装或发送 | 产品/运维 | OPEN（生产） |

## 8. Change Log（变更记录）

历史编号说明：v0.1–v0.19及“初始交付说明”保留当时编号；从v0.20起，当前阶段及后续执行使用[新编号](phase-numbering-map.md)。历史证据文件不重命名、不重写摘要。

| Version | Date | Change |
| --- | --- | --- |
| v0.21 / Phase22 | 2026-10-16 | 按明确指令独立完成Phase22：真实Rust Socket.IO协议矩阵44、单元7、聊天前端5、相关契约48与两crate严格检查通过；63项相关输入及Windows环境限制/前轮FAIL/最终LinuxPASS与清理归档。固定SOCKET_IO_COMPATIBLE及MIT库版本，37未触发；Rust聊天替换旧Node参考，生产通知未迁移。仅标记本阶段完成，整个执行指针不改。 |
| v0.1 | 2026-10-15 | 从架构优化方案及会话约束创建完整台账；32主线+2条件阶段；21项映射；所有实现NOT_STARTED；当前Phase1；未执行实施 |
| v0.2 | 2026-10-15 | 先补方案15节再优化台账；镜像推荐/ZIP兼容/统一URL、GitHub→LinuxDocker与独立Nginx；共享组织/根grant写权；新增门店35/36与WS条件37；同步26/28/32等依赖、候选保留/性能/恢复和余项；34主线+3条件、22项映射，全部NOT_STARTED，指针仍1，未执行实现 |
| v0.3 | 2026-10-15 | 按明确执行指令完成Phase1：固定工具、修复前端lint/类型/70项契约入口、增加单crate Rust检查、落地工程/协议/遗留退出清单与phase-01证据；既有fmt/Clippy/audit/缺deploy失败及ignored分别记录；同步README/architecture、Phase1详情/摘要COMPLETED，指针2 NOT_STARTED，其余阶段未执行 |
| v0.4 | 2026-10-15 | 按新指令完成Phase2：两个classic微应用/聊天前端独立Vite、根npm锁和受控开发/构建、相对资源/真实404、Bun清除及62项源码摘要；13开发/5交付/70契约与选定真实浏览器组通过，旧全量管理fixture失败单列；同步根/组件README、架构/基线、Phase2详情/摘要COMPLETED，指针3 NOT_STARTED，未跨阶段 |
| v0.5 | 2026-10-15 | 完成Phase3：独立Nginx入口/非root只读宿主镜像、默认无共享卷与ZIP/legacy模板、干净原生构建/CI缺失消费者修复；38策略/70契约/8 Rust及实际Docker七组零skip，HTTPS/换IP/ZIP重建/旧入口保留和清理实证。同步架构/README/49输入摘要及phase-03证据；Phase3 COMPLETED，指针4 NOT_STARTED，未迁移业务/未生产发布 |
| v0.6 | 2026-10-15 | 完成Phase4：项目受管四层CI、锁定浏览器/工具与干净Linux、PG21与自有库清理、三产品143组及最终部门11补证、fast19步和最终工具41、三OCI/ELF及实际HTTPS七组全部通过。修复POSIX进程排空及launcher取消清理，保存失败/不同快照与395项最终摘要；同步文档/详情/汇总/决策/指针5 NOT_STARTED。远端workflow/registry/生产部署未执行，无未来阶段实施 |
| v0.7 | 2026-10-15 | 完成Phase5：ZIP/tar私有暂存、规范摘要/禁止覆盖原子安装、0002不可变产物/跨环境幂等与未知提交对账、离线维护/metadata拒绝、升级恢复说明及CI回归。Windows17/维护3、Linux16/维护3/正式PG27/完整契约99/工具48与真实production HTTPS9通过，首启NULL缺陷修复；保留r1/r3失败和r5包装器97/2 FAIL及同快照99补验，403输入/294原生输入零漂移、清理独立核验。同步详情/汇总/决策/证据/指针6 NOT_STARTED；无未来阶段实施、外部发行或生产部署 |
| v0.8 | 2026-10-15 | 完成Phase6：首个SDK1.0.0候选及schema/TS/Rust/classic/ESM同源30项产物、宿主+三个前端接入、错误/限额/requestId/能力检查、真实旧bridge回退与旧401竞态修复。完整Linux fast23（契约169/dev19）、PG27、三产品各146、无缓存宿主97资源全部通过，早轮FAIL保留；472最终输入9c967…、343原生输入d928…与迁移零漂移，资源/lease独立核验。同步现状文档/详情/汇总/决策/证据，指针7 NOT_STARTED；无未来阶段、外部发行或生产部署 |
| v0.9 | 2026-10-15 | 完成Phase7：可信四阶段10秒截止、页面域队列/故障锁存、iframe独立销毁、稳定诊断/整页恢复及真实loader/取消微任务/模块重求值修复。runtime20+14、Windows最终29、完整Linux fast23（契约189/部署36/CI49/dev19/Rust3、16、3、9、8）及三产品各151通过，473输入4a2f9c…/11路径变化，后端/SDK/业务/锁不变；原r1 FAIL/partial/exit143和scratch策略拒绝保留，运行资源/lease与6项空查通过。同步架构/README/详情/汇总/证据/决策，指针8 NOT_STARTED；无未来阶段或外部发行/生产部署 |

| v0.10 | 2026-10-15 | 完成Phase8：宿主/bridge/client/现有工单完整授权投影与展示复用、续期/撤权旧响应拒绝、context完整入口快照与异步复核、fail-closed有界恢复。专项51/97及CHECK30、Windows最终47、Linux fast23（契约221/部署36/CI49/dev19/Rust3、16、3、9、8）和三产品各169通过；478项b6e5ed…/45路径变化，后端/锁/legacy/wire/登记/部署不变，旧失败/不同快照PASS与browser143及清理审计保留。同步README/架构/工程基线/详情/汇总/证据/决策/下一指针9 NOT_STARTED；SDK仍仓库候选，无未来阶段实施、外部发行或生产部署 |

| v0.11 | 2026-10-15 | 完成Phase9：统一auth query/key/epoch/signal与显式路由策略、按需菜单和共享导航代次、父层单一应用推断、非重叠ticket-create与菜单/工具闭环。专项22/24/70/16、Windows最终183、Linux fast23（契约311/部署36/CI51/dev19/Rust3、16、3、9、8）及三产品各183通过；493项ca3df485…/39路径含6份既有工具范围扩展，后端/锁/SDK/wire/session/执行故障域不变。Windows r1真实FAIL、r2输入拒绝、r3旧路径期望FAIL、prefreeze符号链接skip与历史前态来源保留；六项资源空查/lease退出0。同步README/架构/基线/详情/汇总/证据/决策/指针10 NOT_STARTED，无后续阶段、外部发行或生产操作 |

| v0.12 | 2026-10-16 | 开始Phase10：核对4/9依赖、实际493项ca3df485…起点与只读快照、HEAD32956c4及未提交内容。共享登录/组织、原子记录/Web Locks/版本通知与未知结果隔离；Chrome/Edge两稳定major真实多页及能力缺失列为必需验收，无Phase11/后续或生产操作。 |

| v0.12 | 2026-10-16 | 完成Phase10：单原子会话/Web Locks/版本通知、持久intent/unknown、首await前逻辑fence、条件登录/退出/组织切换、旧请求通知与草稿隔离；normal平台轮换保留工单iframe/实际app续期，通知HTTP fallback修复。专项39/21、本地60/181/326、Windows200、Linux fast23/契约362/政策52、完整三产品600与两旧稳定产品兼容34全部通过。499项d52dd7d9…/36路径，官方155/154版本、六空查与lease0、原始FAIL/skip/源漂移和34历史证据保留。同步文档/Q05余项/4验收/指针11 NOT_STARTED，不执行后续或生产 |

| v0.13 | 2026-10-16 | 按用户新指令采纳Phase11 COMPLETED：普通OAuth原子签发/实时失权/随机jti/0003兼容，PG41及fast24同502项162fb670…通过，旧fixture与严格检查失败分列；详情/摘要/决策同步。旧证据保留原执行时点语义。 |
| v0.14 | 2026-10-16 | 开始Phase12：用户明确要求下一阶段，核对Phase4依赖，采用有界登录计数/TTL/时钟、密码计算并发/队列/截止及真实代理来源，当前12 IN_PROGRESS，不执行后续或生产操作。 |

| v0.14 | 2026-10-16 | 完成Phase12：硬容量TTL登录存储、统一异步密码双许可/背压、共享alias累计截止、可信socket/代理来源、改密CAS和全HTTP接入。正式PG8组46与fast27同505项9b016ace…全部PASS零skip；100k计数、24混合HTTP、Windows86/73ignored、WIP夹具FAIL和严格fmt/Clippy既有FAIL分别保存。详情/摘要/证据/决策/指针同步13 NOT_STARTED；用户要求终态已采纳，无后续或生产操作。 |
| v0.15 | 2026-10-16 | 用户明确执行Phase13：核对Phase4/11完成，登记全部关键管理/授权调用链、严格事务审计及故障注入范围；状态13 IN_PROGRESS，未执行后续阶段或生产操作。 |

| v0.15 | 2026-10-16 | 完成Phase13：关键管理/授权/发布严格事务审计、对象安全前后摘要/删除快照、同值重试和有界失败指标；并发锁序与授权重验回归。正式PG10组64及fast27同506项582d2759…全部PASS必需零skip；Windows默认ignored、严格fmt/Clippy既有FAIL及早轮真实失败分别保存。同步详情/摘要/决策/证据/指针14 NOT_STARTED，无未来阶段或生产操作。 |

| v0.16 | 2026-10-16 | 用户明确执行Phase14：依赖6/13已完成，冻结独立权限、server-owned默认拒绝策略/应用资源动作闭集和目标管理子树、typed预览/差量/版本/事务审计；状态14 IN_PROGRESS，仅执行该阶段。 |

| v0.17 | 2026-10-16 | 完成Phase14：独立读写BUTTON权限、严格默认拒绝策略/目标范围/资源动作闭集、typed预览与差量/版本/UUID保留、事务审计及下一请求生效；正式PG11组71和fast28同511项b48d1219…全部PASS零skip。Windows默认95/88ignored、严格fmt/Clippy既有38错误与前验新增诊断修复如实保存。同步详情/摘要/证据/决策/指针15 NOT_STARTED，无下一阶段或生产操作。 |

| v0.18 | 2026-10-16 | 用户明确执行Phase15：依赖9/14已完成，冻结独立管理页、同Phase14范围窄发现、共用PATCH/requestId、精确预览/冲突/失权/对账和独立审计查看；真实浏览器→隔离PG验证。状态15 IN_PROGRESS，不执行Phase16或外部操作。 |

| v0.19 | 2026-10-16 | 完成Phase15：独立数据授权页、窄发现、精确预览/差量/冲突/失权/对账与独立审计；PG12组74、fast28/568及五产品80 live/5 harness通过。保留r2 Chromium主题测试FAIL；只改该测试等待，当前三产品各11项补验通过、其余537项不变，最终538项7a7df9f1…。Windows ignored与strict旧债务、失败及源码/资源/lease证据分列；同步详情/摘要/证据/决策/指针16 NOT_STARTED，无后续实施或生产操作。 |

| v0.20 | 2026-10-16 | 按用户明确要求重新编号：主线按原执行顺序映射为1–34，条件为35–37；旧35/36→新26/27、旧26–32→新28–34、旧33/34→新35/36、37保持条件WS。同步汇总/详情/依赖/锚点/未来证据路径/覆盖矩阵/开放问题/执行序列及现状文档；当前仍16 NOT_STARTED、1–15 COMPLETED。历史决策/变更及既有验证文件按旧编号保留，对照表可追溯；未执行新阶段。 |

| v0.21 | 2026-10-16 | 按用户明确要求开始Phase16，依赖1/4/6完成；独立Cargo crate、薄入口/窄库、独立配置与pool、存活/就绪和旧SHA256→SQLx基线接管。冻结同锁/严格结构验证/事务回滚与无Node运行验收；当前16 IN_PROGRESS，仅执行该阶段。 |

| v0.21 / Phase24 | 2026-10-16 | 按明确指令完成独立 OAuth Vite/Rust 演练：最终 Linux9步/27JS/17Rust/12live+1fixture、DockerHTTPS10、native13与JS/CI43通过；修复配置误脱敏及 path 构建兼容，逐文件/镜像/数据库/失败与清理归档。仅标记 Phase24 完成，Current Execution Pointer 整个区段不改，无其他阶段实施或外部发布。 |

| v0.22 | 2026-10-16 | Phase16全部必需验收通过并同步COMPLETED：独立Rust工单基线/旧SHA256→SQLx同事务接管，604项验证输入ab0a4b25c861…；fast40步/668项、平台PG74与工单PG15、无Node ELF11项、Windows与Linux检查及资源清理通过。保留失败尝试、文档补充等价证明及其他聊天结果；当前17 NOT_STARTED，不自动执行。 |

| v0.23 | 2026-10-17 | 明确执行Phase17，依赖4/6/11/16完成；有界两段平台验证、严格grant/组织scope、一致只读/CSV、页边界与EXPLAIN、Node后台参考退役；隔离PG与真实平台main联合验收，当前17 IN_PROGRESS，不执行18及后续。 |

| v0.23 / Phase25 | 2026-10-17 | 按明确指令完成独立Rust平台登记/授权CLI、schema/manifests、秘密保护和旧Node入口退役；最终Linux17步、21Rust/428JS/9TLS/13live+1fixture/9无Node通过，失败轮次/输入/数据库/产物及资源清理归档。仅同步Phase25完成，整个Current Execution Pointer不改，不自动执行下一阶段。 |

| v0.24 | 2026-10-17 | Phase17完成并采纳COMPLETED：两段有界授权、统一scope只读/CSV、分页边界与8计划、Node参考退役；674项59b0f8d1…候选PG9步（平台74/迁移15/read13/live1+13）、Linuxlib34/doc1、无Node11与本阶段范围门禁通过。完整fast48/50及700PASS/1FAIL、Phase25两失败和不同当前字节补验、PG外层宿主漂移FAIL、Windows环境及平台strict53诊断分别保留；边界/driver/hash/清理核验。仅推进18 NOT_STARTED，保留其他聊天结果，无写入/提醒/切流或生产操作。 |

| v0.25 / Phase23 | 2026-10-17 | 按明确指令完成Rust通知、有界FIFO/复验/传输预算、只读角色与正式镜像；最终Linux8步/34真实联合/双页6、Rust29、回归87/最新CI29及镜像9通过，682输入与所有scoped字节匹配并清理。修复原始半头HTTP停机并补首次认证真实await竞态；失败轮次及inactive RSA公告如实归档。仅标记Phase23完成，整个Current Execution Pointer逐字节不改，无后续实施/外部发布或生产操作。 |

| v0.26 | 2026-10-17 | 按明确请求执行Phase18：依赖6/13/17完成；创建header幂等/规范化摘要及原子receipt、版本状态/软删除、授权先于DB与有界事务、成员未接入默认拒绝；新增0002双历史兼容，当前18 IN_PROGRESS，仅实施本阶段。 |

| v0.27 | 2026-10-17 | Phase18完成并采纳COMPLETED：Rust创建header幂等/规范摘要/原子receipt、版本状态/软删除、授权先于DB与有界事务、Phase19前成员默认拒绝；0002双历史及旧0001保持。691项9bea11c1855b…候选，platform-postgres 74PASS、work-orders-postgres 21PASS、work-orders-read-postgres 13PASS、work-orders-write-postgres 14PASS、work-orders-live-postgres 1PASS/named13、work-orders-write-live-postgres 1PASS/named12；Linux lib43/doc1、同源runtime12项；完整fast子级状态PASS，52步/52PASS/0FAIL，测试745PASS/0FAIL/0skip；正式报告/边界/运行索引和四项PASS自检已核验，原始失败及不同来源补验保持。仅推进19 NOT_STARTED，保留并行阶段结果和全局实时缓存，不执行成员/提醒/切流或生产部署。 |

| v0.28 | 2026-10-17 | 按明确请求执行Phase19：依赖14/17/18完成；Rust成员目录/有效直接成员分配、平台窄用途声明/硬组织边界、预读与锁定组织版本复核/短有效期和SELF不扩张，当前19 IN_PROGRESS；仅本阶段，无提醒/前端/门店/切流或生产操作。 |

| v0.29 | 2026-10-17 | Phase19完成并采纳COMPLETED：Rust成员目录/有效直接成员分配、平台显式同应用窄用途声明、预读释放连接后成员HTTP/锁内组织版本与oldAssignee复核、从HTTP开始短有效期及SELF不扩张；历史receipt不重查成员，跨库竞态与Phase18回退限制明确。700项52ebb7a5904b…候选，platform-postgres 76PASS、work-orders-postgres 21PASS、work-orders-read-postgres 13PASS、work-orders-write-postgres 14PASS、work-orders-member-postgres 12PASS、work-orders-live-postgres 1PASS/named13、work-orders-write-live-postgres 1PASS/named12、work-orders-member-live-postgres 1PASS/named10；Linux lib50/doc1、同源runtime12项；完整fast子级状态PASS，53步/53PASS/0FAIL，测试765PASS/0FAIL/0skip；正式报告/边界/运行索引和三项PASS自检已核验，原始失败及不同来源补验保持。schema/依赖不变，仅推进20 NOT_STARTED，保留并行阶段结果和全局实时缓存，不执行提醒/前端/门店/切流或生产部署。 |

| v0.30 | 2026-10-17 | 按明确请求执行Phase20，前置17/18/19完成：Rust手动提醒使用当前应用Bearer、独立read/publish权限与grant；为严格保留未知事件键/正文及目标变化冲突，新增业务提醒意图与0003安全迁移，短事务后平台HTTP，工单version不变。当前20 IN_PROGRESS，只验持久发布与收件箱，不实施21前端、实时改造/切流或生产操作。 |

| v0.31 | 2026-10-17 | Phase20完成并采纳COMPLETED：Rust提醒双授权、精确平台域不可变意图及0003安全接管、短事务后HTTP、准备与发布未知分离、同键恢复/目标变化冲突、真实收件箱与实时离线不影响持久结果；工单version不变。711项4f9a2cb80ece…候选；platform-postgres 76PASS、work-orders-postgres 27PASS、work-orders-read-postgres 13PASS、work-orders-write-postgres 14PASS、work-orders-member-postgres 12PASS、work-orders-reminder-postgres 12PASS、work-orders-live-postgres 1PASS/named13、work-orders-write-live-postgres 1PASS/named12、work-orders-member-live-postgres 1PASS/named10、work-orders-reminder-live-postgres 1PASS/named12；Linux lib56/doc1、同源runtime12项；完整fast 53/53步PASS、778PASS/0FAIL/0skip。三项PASS自检与正式来源/清理已核验，开发观察及未采纳失败独立保持；保留旧SQL和所有其他阶段，只推进21 NOT_STARTED，不实施前端/实时改造/切流或生产部署。 |

| v0.32 | 2026-10-17 | 按明确请求执行Phase21，前置6/8/9/10/15/17/18/19/20完成：独立Vue/Vite标准入口、相对module/chunk与公开配置，Rust CRUD/成员/CSV/提醒及创建未知恢复、草稿隔离/续期/409/失权、真实opaque iframe与静态Nginx验收。当前21 IN_PROGRESS，不实施其他阶段、后台语义改动、无关视觉重设计、外部发行或生产操作。 |

| v0.33 | 2026-10-17 | Phase21完成并采纳COMPLETED：独立Vue/Vite、相对module/chunk与同步SDK握手，严格RustDTO/未知显式恢复、四元草稿隔离和主题/失权/409/MENU；726项675b639c3dd5…候选；完整fast 55步/832PASS/0FAIL/0skip；chromium真实15项、Phase8 4组/Phase10 17组；chrome真实15项、Phase8 4组/Phase10 17组；msedge真实15项、Phase8 4组/Phase10 17组；独立干净构建3步/15源码输入PASS。四同源运行、实际Nginx/双库/普通用户及资源清理、正式三项PASS核验，开发观察/原失败独立保持；不改后台/SQL/SDK/iframe权限，不部署发行。保留其他36阶段与全局缓存，22–25已完成，只推进26 NOT_STARTED。 |

| v0.34 | 2026-10-18 | 按明确请求执行Phase26，依赖4/6/14/17/19完成；新增stores独立Rust/业务库与专属账号、资料/员工/一店一日十进制日报和汇总，逐请求应用授权、受控成员与SQL边界/并发/日期回归。当前26 IN_PROGRESS，只实施本阶段，不做前端、公共crate、外部发行或生产操作。 |

| v0.35 | 2026-10-18 | Phase26完成并采纳COMPLETED：独立stores Rust/库/专属账号、门店/员工/一店一日报与精确汇总，逐请求授权/受控成员/SQL范围、版本/不可变归属/短事务和审计；757项2a00d4ecf055…；完整fast 61步/894PASS/0FAIL/0skip；stores SQL 1父测试/8具名检查、real live 1父测试/16具名检查PASS。五原验收PASS、真实三库账号隔离及清理证据绑定，原失败独立保持；保护109历史正式文件/其他36阶段/缓存，仅推进27 NOT_STARTED，不执行前端/公共框架/生产部署。 |

| v0.36 | 2026-10-18 | Phase26终态核验后采用已收到的Phase27明确授权，依赖全部完成；独立React/TypeScript/Vite门店、固定SDK与精确应用/菜单/CLI接入，普通用户真实资料/员工/日报/汇总与双消费者双页会话/组织实证。当前27 IN_PROGRESS，不做28镜像发行/公开v1v2路由、公共框架或生产部署；历史证据保持。 |

| v0.37 | 2026-10-18 | Phase27完成并采纳COMPLETED：独立React/TS/Vite门店及11菜单/九动作、真实三库普通用户CRUD/成员/日报/汇总和双应用会话实证；809项02304b07f8c9…最终r2，fast63步/1033PASS/0FAIL/0SKIP，三浏览器各21组PASS，CLI TLS各9具名检查另列；五原验收、625项managed证据、来源/资源/专属cache及历史114文件核验。保留r1原FAIL，其他36阶段及全局缓存字段不变，仅推进28 NOT_STARTED，无镜像发行/公共框架/生产部署。 |

| v0.38 | 2026-10-18 | 按明确请求执行Phase28，前置全部完成；工单/门店独立镜像、版本Nginx、Rust CLI/CI不可变持久清单及外部URL登记UI，隔离registry/Linux真实验收。当前28 IN_PROGRESS，保护119历史证据；平台不控制Docker，旧登记协议不伪加digest，无后续阶段/真实外部发行或生产操作。 |

| v0.39 | 2026-10-19 | Phase28完成并终态核验：839项f77df330f956…同源冻结输入，fast63步/1083PASS/0FAIL/0SKIP；六镜像真实构建/push/清除/pull/Compose、14项交付及三浏览器各9项PASS。五原验收、来源/原始日志/OCI/ELF/静态资源/清理及cache删除核验；119历史证据/其他36阶段/全局字段保持，开发原FAIL保留。默认API builder及远端/生产未执行，仅推进29 NOT_STARTED。 |

| v0.40 | 2026-10-19 | 按用户明确请求执行Phase29，前置3/4/7/9完成；先冻结登录生产构建/5次冷启动基线，再实施壳与面板按需、局部恢复和应用管理创建/编辑用例拆分。当前29 IN_PROGRESS；仅本阶段，不做Phase30、外部发行或生产部署，保护124份既有正式证据。 |

| v0.41 | 2026-10-19 | Phase29完成并采纳COMPLETED：壳/面板按需、局部Suspense/错误恢复、应用创建/编辑用例边界、真实Nginx404纯文本和旧hash保留。基线/最终各5冷启动，transfer下降19.93%，按实测冻结本地预算且不虚报20%候选/线上INP；完整Chromium209、三产品专项各9及既有200、隔离Linux391/策略55/TS/lint/Nginx7通过，旧FAIL和边界独立记录。851项300df64b7a9c…输入，保护124历史证据/其他36阶段/全局缓存字段，仅推进30 NOT_STARTED，无外部发行或生产部署。 |

| v0.42 | 2026-10-19 | 按用户明确请求执行Phase30，全部前置完成；851项300df64b…旧输入及130历史证据冻结。仅实施双应用受控授权合并/成员协议、独立新语句撤销与版本复核、窄lib/typed用例及平台checked分页；先实际测量旧链，禁止长期正向缓存或以contextVersion认证。当前30 IN_PROGRESS，不做31保留策略、无关平台搬迁、外部发行或生产部署。 |

| v0.43 | 2026-10-19 | Phase30完成并采纳COMPLETED：受控合并/单语句快照及新语句撤销复核、窄lib/私有SQL、checked分页及CI；862项178ffd6ed484…/45路径，正式31步/153Rust与补充205政策PASS。两应用新旧模式、旧/当前ELF及真实预算恢复、三库账号与工单四类联合通过；combined HTTP减50%/PG调用减21.05%，门店耗时收益不足保留legacy，SQL执行累计和冷尾回归明确披露。130历史证据/其他36阶段/原迁移与锁保持，专属cache及运行资源清理；原失败、既有fmt/严格Clippy债务、未执行矩阵独立归档，仅推进31 NOT_STARTED，无外部发行或生产部署。 |

| v0.44 | 2026-10-19 | 按用户明确请求执行Phase31，全部前置完成；冻结862项178ffd6e…及135历史证据。选择静态发布有限只读预览试点、持久在途操作保护与新增迁移，验证平台/工单旧库升级、门店初始化及旧前端契约；候选30天/最近5版仅隔离预览，生产apply不可用。当前31 IN_PROGRESS，不做32、真实清理或外部发行/生产部署。 |

后续每次状态/范围/决策变更追加记录，保留已有历史。调整依赖后检查无环，调整协议/默认模式后同步所有受影响Phase的输入、验收和指针；不得只改汇总表。

| v0.45 | 2026-10-19 | Phase31三项验收COMPLETED；885项621ea69d5326…/36路径，正式32步全部PASS，旧库/旧消费者双模式/只读引用与未知保护/旧URL与ZIP/审计回滚通过，失败及全部owner清理归档。135历史证据/其他36阶段保持；指针32 NOT_STARTED，生产apply不可用，不自动实施下一阶段。 |
| v0.46 | 2026-10-19 | 用户授权Phase32，核对依赖并冻结起点，实施固定指标协议和独立Rust监控；范围仅本阶段。 |
| v0.47 | 2026-10-19 | Phase32三项原验收COMPLETED；906项583f7f47d727…，正式服务40步/浏览器20步与宿主26项PASS；真实性能失败及资源/故障/私有隔离/补拉完整归档。139历史证据及其他36阶段详情保持；指针33 NOT_STARTED，需新请求。 |

| v0.48 | 2026-10-19 | 用户明确授权Phase33，核对依赖3/16/18/23/26/31/32完成；冻结906项源码及144份历史证据。独立Rust离线工具、三库协调停写、AES-GCM/HTTPS远端取回、新库保护/全凭据失效、定时模板及完整隔离恢复验收；33 IN_PROGRESS，不实施34/生产PITR/外部告警或安装定时。 |

| v0.49 | 2026-10-19 | Phase33完整隔离试点COMPLETED：932项ef0082e18930…；正式20步/十检查、Rust3/CI33和同源Windows通过。三库/静态/原生/路由/四镜像/四实际页面与旧凭据拒绝；RPO 44.808s/RTO 109.137s。144历史证据/20旧迁移锁/其他36阶段保持，owner资源/cache清理；旧失败保留。物理异机/生产PITR与外部渠道/定时未验收，不执行34或生产操作，仅推进34 NOT_STARTED。 |
| v0.50 | 2026-10-19 | 用户明确授权Phase34；全部1–33前置已完成，22选择SOCKET_IO_COMPATIBLE、37未触发。仅执行独立发行与当前源码最终分层验收，保护历史证据及迁移锁，不外部发布或生产操作。 |
| v0.51 | 2026-10-19 | Phase34六项验收全部PASS：935项最终输入、独立双包/六镜像、fast74/PG24/delivery7、三浏览器明确范围覆盖与Socket.IO44，恢复RPO 44.170s/RTO 114.557s；旧Node入口退役，历史证据及他阶段详情保持，主线验收完成，无自动下一阶段。35/36/37未触发，生产边界保持。 |
| v0.52 | 2026-10-19 | 用户明确请求Phase35；5/28/32/34依赖与源码核验通过，ZIP跨机/无共享存储、白名单/拓扑/CA/secretRef输入仍缺，条件未确认。记录BLOCKED及输入清单/方案比较，未实施分发或节点验收；主线完成及其他阶段保持，补输入后恢复35。 |
| v0.53 | 2026-10-19 | 用户明确请求Phase36；28/32/34依赖完成且不依赖35。缺受管目标、可信registry、固定模板/secretRef、Nginx权限及信任配置，记录BLOCKED与输入清单/现状边界。未实施agent/API/UI或运行验收；35详情及主线保持，已请求[35,36]与已触发[]分列。 |
| v0.54 | 2026-10-19 | 明确请求Phase37后核验：22选型SOCKET_IO_COMPATIBLE、34真实44/44 PASS与当前输入相同，未满足NATIVE_WS_REQUIRED触发条件。请求BLOCKED_NOT_APPLICABLE，37仍NOT_STARTED；未实施原生WS，不改其他阶段详情/当前指针或历史协议证据，主线完成不变。 |
| v0.55 | 2026-10-20 | 用户明确选择Phase36“仅本机隔离实现与演练”，触发36并恢复IN_PROGRESS；配置一次性Docker/私有registry/测试CA与固定模板，实施独立Rust agent、正式任务API/UI和受控路由/清理，保留既有阻塞及其他阶段证据，不连接真实目标或外部发布。 |
| v0.56 | 2026-10-20 | Phase36本机隔离验收完成：964项源码2ea7c8ff…，81步fast/1127、18部署/3浏览器、17发布/ZIP/升级均通过；相对/编码/分隔符别名与清理新引用已封堵，历史169证据和旧迁移保持，owner资源/缓存/临时CA清理完成。35另行恢复，37未触发。 |
| v0.57 | 2026-10-20 | Phase36终态后，按已收到的本机隔离多节点ZIP无共享存储授权单独恢复35：冻结964项源码及174份既有证据，固定两个独立目标/卷和测试信任边界，实施Rust分发/API/UI/逐节点激活门禁；35 IN_PROGRESS，不执行真实远程/生产或37。 |
| v0.58 | 2026-10-20 | Phase35本机隔离双节点ZIP无共享存储验收COMPLETED：15项实证/三浏览器、fast 82步/1133通过和发布回归17/17；174历史证据、旧迁移与其他阶段正文保持，owner容器/网络/缓存/私有目录清理。36保持完成，37未触发。 |

### 初始交付说明

v0.1历史交付仅创建台账；v0.2完善方案/台账，当时全部实现未开始。目标仍为GitHub镜像交付、LinuxDocker/独立Nginx、统一URL登记和ZIP兼容，门店为第二消费者。v0.3完成Phase1，v0.4完成Phase2，v0.5完成Phase3的Nginx/宿主镜像/原生构建与隔离实际验收，v0.6完成Phase4的项目受管分层CI与实际隔离验收，v0.7完成Phase5的ZIP原子安装/不可变登记/恢复与真实HTTPS验收，v0.8完成Phase6的版本化SDK/公开契约与多消费者/旧版兼容验收，v0.9完成Phase7的可信永久挂起故障隔离与稳定整页恢复；v0.10完成Phase8的展示/授权分类、完整入口与有界恢复；v0.11完成Phase9的菜单查询/路由策略/共享导航与非重叠创建规范；v0.12完成Phase10跨页共享会话、未知结果隔离和真实两稳定major兼容；该v0.12历史交付时指针为Phase11 NOT_STARTED，最新指针见第5节。业务Rust迁移、真实外部发行和生产部署尚未执行，数值候选与未提供生产环境仍见第6/7节。



2026-10-19最新采纳说明：Phase30已完成并终态核验，五项原验收及范围内必需门禁PASS；当前指针Phase31 NOT_STARTED，下一阶段须新请求，历史证据及其他阶段详情保持。

2026-10-16 本次采纳：Phase24 已 COMPLETED；按用户要求不改当前执行指针，详见 Phase24 详情和证据。

2026-10-17 本次采纳：Phase25 已 COMPLETED；按用户要求不改当前执行指针，详见 Phase25 详情和证据。

2026-10-17 本次采纳：Phase23 已 COMPLETED；按用户要求仅标记任务完成，整个当前执行指针保持本次更新前原样，实际验收见 Phase23 详情与证据。

2026-10-19最新采纳说明：Phase31已完成并终态核验，三项原验收及范围内必需门禁PASS；当前指针Phase32 NOT_STARTED，下一阶段须新请求，历史证据及其他阶段详情保持。

2026-10-19最新采纳说明：Phase32三项原验收通过并完成；候选性能/撤权观察按原始数据报告，不能作生产容量或SLA。当前指针Phase33 NOT_STARTED，执行须新请求。

2026-10-19最新采纳说明：Phase33完整隔离试点验收完成；当前指针Phase34 NOT_STARTED，下一阶段须新的明确请求。生产物理异机/PITR与外部投递/定时安装仍未验收。



2026-10-19最新采纳说明：Phase34全部六项验收通过，主线Phase1–34全部COMPLETED；主线验收完成，无自动下一阶段。35/36/37未触发，保留NOT_STARTED；原始失败、源码差异、发行与清理见Phase34报告。外部发行、生产部署、物理异机/PITR及真实投递/定时边界保持。

2026-10-19 Phase35请求核验：BLOCKED，待确认ZIP跨机/无共享存储和目标/信任配置，或明确仅隔离多节点演练。主线1–34完成不变；36/37未触发，未连接目标或执行分发。

2026-10-19 Phase36请求核验：BLOCKED，待明确本机隔离实现与演练，或提供受管目标及registry/template/secretRef/Nginx权限配置。Phase35阻塞记录与主线1–34完成不变；37未触发。

2026-10-19 Phase37条件核验：当前Socket.IO兼容分支已验收，原生WS fallback不适用；37未触发且保持NOT_STARTED。本次执行请求因选型约束冲突BLOCKED，不更改实时协议或当前可恢复指针。

