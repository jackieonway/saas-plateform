# Phase 13 验收记录

状态：**IN_PROGRESS**。依据 [台账 Phase 13](../task-ledger.md)，当前实现与计划见 [灰度与身份感知恢复](../phase-13-rollout-recovery.md)。Phase 1～12 已完成；本阶段先记录当前源码与准备，不将旧阶段、helper纯测试或skip-capacity preflight当作新运行验收。以下五项均PENDING，尚无本阶段35分钟容量、完整双库恢复／两轮browser／通知的最终真实receipt。

### 本轮执行进展（未封账）

2026-10-25 已实际运行并保留 preflight 01～08 的安全回执，任务库和角色均清理。通知角色权限检查已修复 PostgreSQL 条件重排导致的非序列表权限函数报错；严格 HTTP 客户端已修复 HTTP/1.0／Connection: close 的文件引用生命周期。完整协议轮已通过 221 项检查；08 调试轮实际通过 10／50／100% RP 请求灰度、候选故障与 2.396338 秒显式路由回退。08 浏览器通过 98 个检查后在有效员工会话重新登录的工具回调失败，已改为支持的 PLATFORM 注销再登录；其恢复调试因明确跳过协议而缺少真实 OIDC 创建 receipt，未计恢复通过。后续13调试轮 initial341／restored247及140条恢复检查已通过，但最终扫描仍命中，整轮保持FAIL。源码随后证实普通code标签会被错误登记；历史具体匹配值未保存，不反写旧回执。14轮在容量前主动停止以修正服务／客户端锁等待计量边界；54项Linux工具回归通过后，15轮冻结608文件启动。第15轮在约180秒时已出现容量漏发与延迟不达标，主动停止并保留FAIL；第16轮采用身份release与并发TLS握手，协议221及灰度15项通过，但约120秒仍严重漏发，整轮FAIL并完成任务清理。两轮部分容量及实际清理详见各自失败分析，不能当完整35分钟。独立严格TLS回显对照复现了同进程客户端／网关线程竞争，已改为新解释器常驻网关，21项worker与55项load回归通过。完整流scanner改为精确末12bytes索引，13项等价／边界回归通过，100k模拟凭据＋1MiB完整日志在3.394秒完成。原100会话、1800＋300秒、主计数与延迟阈值全部不变。第17轮以609文件快照cd5d1c842acad7b3f4c1df5b700f3a0d5e9fe1ce375fc15f143d0c4aa454cce2实测约214秒，服务部分分位数已低于候选门槛，但18条introspect漏发使整轮仍FAIL，独立gateway退出与扫描通过。第18轮将负载发生器资源从32／32调整为100 workers／100 in-flight，与100 actor规模对齐，新增五类有界漏发诊断；不改产品配置、请求目标、速率、时间、0.5s调度上限、3s绝对预算或延迟门槛。Linux59项工具回归通过后，第18轮以609文件快照139abb88a721fccc9b6983697c5202e37ccf09eeafe7cca02b1a51f44c83e721实测至约754.5秒自动异常终止；已完成主操作全success且scheduler五类全0，但内层异常类未保存，整轮FAIL。后来独立复现两处harvest跨时点后负数sleep竞态，修为重新计算remaining且仅正值等待，并保留安全异常类别与筛选源码帧；不能倒推该次现场异常已被直接证明。Linux69项回归与五种完整264000槽／2100假秒纯调度模式通过后，第19轮以609文件快照66dba13490fd979db12fa2308f4b2766ece2e3e484cdbfadeeb8881defc655e6启动，原真实2100秒／全部计数及阈值不变。正式容量、最终两轮浏览器／扫描／清理仍待同轮完整回执。

活动消费者 [本轮审查](../phase13-consumer-audit.md) 已实际通过 233 项 Node 和 184 项 Rust 普通测试；806 个源码／合同／发行物文件与 13 份日志已登记摘要和凭据模式扫描。当前成员字面量检索口径为29个生产文件、5个writer／11个mutation位置、两处三元UPSERT。上述结果不替代数据库／容量／浏览器验收。

## Self-Check：台账五项标准

| 验收项 | 结果 | 尚缺实际证据 |
| --- | --- | --- |
| 平台／门户真实配置下完成双身份登录／选择／局部与全局撤销，客户不泄露员工权限、目录或历史通知 | **PENDING** | initial及restored真实browser／notification、双向局部撤权、跨账号／跨client与实际旧载体拒绝；截图、完整扫描和清理尚待。 |
| 按容量／SLO／RPO／RTO完成演练，未达项修复后才通过 | **PENDING** | fresh完整1800＋300秒、100独立actor／主24,000认证＋240,000权威检查、分位数／资源／失败分类；双库＋材料＋witness完整恢复及实测RPO／RTO。 |
| 身份感知回退与真实退役条件，历史对象N/A有清单依据，活动兼容仍受撤销保护 | **PENDING** | 实际10／50／100% eligible RP请求分流、candidate故障503、显式set0路由回退和两RP新登录；最终消费者／retirement清单与回退耗时。 |
| 查询／JOIN／ON CONFLICT／DELETE／trigger／广播的双身份歧义审查闭环，旧无唯一成员证明凭据拒绝 | **PENDING** | 当前逐命中源码分类已完成且无新阻断；最终source／catalog／真实凭据负例仍须与本阶段receipt关联，不能只引用旧测试名。 |
| 全阶段交付可追溯，实施状态、legacy支持和首个客户业务范围在文档一致 | **PENDING** | 最终source／静态发行字节、normal pins、各轮失败／修复／完整运行／视觉／清理与文档交叉核对；五项完成后再封账。 |

## 准备与证据分层

| 范围 | 当前事实 | 证据与限制 |
| --- | --- | --- |
| Phase12依赖 | COMPLETED，独立拓扑基线 | [Phase12验收](phase-12.md)含42普通＋3identity PG、平台12stage、protocol221、browser214记录／8图及共享平台库restricted identity health；不替代本阶段双库／完整业务恢复。 |
| 四份历史准备 | PREPARATION_ONLY／NOT_ACCEPTANCE | [总体](../../../../local-identity-sso-phase12-qa/phase13-readonly-preparation.md)、[负载](../../../../local-identity-sso-phase12-qa/phase13-load-implementation-preparation.md)、[恢复](../../../../local-identity-sso-phase12-qa/phase13-recovery-implementation-preparation.md)、[通知](../../../../local-identity-sso-phase12-qa/phase13-notification-implementation-preparation.md)保留归档时点状态／源码行号，当前以QA13新源码与真实receipt为准。 |
| 当前integration | 源码接线合同，非运行证据 | [integration-context.md](../../../../local-identity-sso-phase13-qa/integration-context.md)界定私有context、callbacks、35分钟、initial／restored周期和清理；私有state不得直接序列化。 |
| 工具镜像 | 准备PASS | [tools-image-result.json](../../../../local-identity-sso-phase13-qa/tools-image-result.json)记录phase13-tools及准确image SHA，仅加Debian Argon2，productDependenciesChanged=false；不是reset／restore实测。 |
| 独立PKI／keyrings | 准备PASS | [pki-result.json](../../../../local-identity-sso-phase13-qa/pki-result.json)CA:false leaf、auth／console／customer SAN、两BFF keyrings，早期PKI未动；不是live／OS信任结果。 |
| integration preflight | 计划跳过容量，NOT_ACCEPTANCE | root先用调试轮验证完整接线；skip-load／缩短／fake运行不得满足容量项，即使其他stage局部通过也不封账。 |
| helper纯／AST验证 | 作者准备结果，最终receipt待归档 | load、notification、recovery的纯测试与AST按真实作者输出和冻结SHA分开保存，不能当真实网络／数据库／35分钟运行。 |

## 实际运行登记

| 项目 | 当前结果 | 必需回执 |
| --- | --- | --- |
| 初始独立四进程／canonical迁移／roles／protocol | PENDING | task／cluster／两DB marker、normal pins、唯一schema history、受限角色与HTTP／故障结果。 |
| task RP请求canary10／50／100%及显式回退 | PENDING | 准确两client／fixture tenant、eligible=baseline＋candidate、实际比例、candidate故障503、不自动fallback、set0及两RP重新登录。 |
| 正式35分钟容量 | PENDING | 1800＋300真实秒，100不同sid／grant，主login4,800／code9,600／refresh9,600／introspect240,000，scheduled／sent／completed／success逐项一致。 |
| 容量分位数／资源／辅助计数 | PENDING | introspect p95／p99≤100／300ms；code／refresh≤500／1000ms；password p95≤1000ms；CPU／RSS／DB／Argon2／queue／timeout／saturation与辅助HTTP分计。 |
| initial browser／notification | PENDING | 双RP、实时通知与实际撤销／故障、安全私有见证、initial截图和输出扫描。 |
| 双库／材料checkpoint及外部witness | PENDING | 两库owner／comment／ACL／data／history、加密材料认证、原本仍有效旧载体、post-backup高水位与deny、旧signed sequence／tamper拒绝。 |
| 原名restore／fence／deny-only／轮换reset | PENDING | 两个真实pg_restore、catalog／safe data、库外材料、超外部highwater generation、全载体／旧密码拒绝、精确批准账号新密码及受限启动receipt。 |
| restored browser／notification／RPO／RTO | PENDING | 更新私有材料后新双RP业务、第二轮通知、原始checkpoint／fault／complete／业务marker时点、实际丢失及认证状态主动废弃分类。 |
| 最终source／consumer／SQL／visual／清理 | PENDING | 准确文件集与字节不变量、逐命中闭环、图像逐张复核、完整日志扫描、进程／候选／通知／库／角色／hosts／bridge清理。 |

容量仅TASK_OWNED隔离候选；35分钟不证明滚动30天99.9%。恢复候选RPO≤24h、RTO≤4h以当次实测计算；生产15min／60min、持续WAL／PITR、异地回读和备份调度不由年轻checkpoint或source函数推定。路由回退仅同binary配置，不声明旧二进制降级。初始及还原后两轮均须真实取证，不沿用Phase12的214记录。

## SQL逐命中审查登记

当前逐命中只读源码审查已完成，检索backend/src的memberships、mutation／ON CONFLICT／JOIN／fetch／first／[0]／LIMIT与广播，并追踪0001～0020相关函数最终替换关系。29个生产区域文件包含memberships字面量（包含配置／路由标签，非全部SQL）；排除remote.rs:407起cfg(test)后，生产成员mutation为5文件／11个SQL位置，两个成员ON CONFLICT均三元键。当前检索与逐调用范围未发现新生产SQL／接口安全阻断；该只读SQL分类本身不计Cargo／DB运行；最终source seal及实际负例仍PENDING，各阶段执行结果独立引用。

| 当前源码位置／行号 | 语义分类与核对结论 | 既有回归源码 |
| --- | --- | --- |
| [membership_lifecycle/service.rs](../../../src/membership_lifecycle/service.rs):333/436/483/551/634/656/696 | 精确成员／唯一类型；user＋org fetch_all锁全部关系后按源ID／目标类型定位。同类型409，状态递增版本、删除按ID，第二身份与替换分开，不原地改type。 | R1 |
| [routes/users.rs](../../../src/routes/users.rs):1165/1217/1241/1242/1325 | 员工bulk集合；重复org先拒绝，DELETE限EMPLOYEE及管理范围，三元UPSERT仅部门／岗位，不改原ID／停用状态；离开按准确员工ID。 | R2 |
| [routes/onboarding.rs](../../../src/routes/onboarding.rs):210/213/225/871/877 | 唯一员工类型／业务初始化；显式EMPLOYEE＋三元DO NOTHING后复查已存在员工及启用状态。role绑定幂等不是成员二元冲突，LIMIT1取已有管理员另核员工资格，不作登录成员选择。 | R1/R3 |
| [seed.rs](../../../src/seed.rs):1336；[users.rs](../../../src/routes/users.rs):561；[remote.rs](../../../src/routes/users/remote.rs):244 | 明确EMPLOYEE写入；remote welcome要求恰好一行，静默抑制不是成功。客户／第二身份仍走生命周期门禁。 | R1/R3 |
| [users.rs](../../../src/routes/users.rs):150/178/186/220/402；[remote.rs](../../../src/routes/users/remote.rs):119/206 | 账号级EXISTS／全局范围集合命令；RETIRE接受时捕获全部成员(id,identityVersion)，只删原快照，不领养新行／新版本，无type条件符合全账号语义。 | R3/R7 |
| [users.rs](../../../src/routes/users.rs):722/729 | 全局账号删除兼容分支；remote转RETIRE，本地DELETE users经全局范围校验，不误当员工局部离职。 | R3/R7 |
| [authorization.rs](../../../src/authorization.rs):68/230；[users.rs](../../../src/routes/users.rs):75；[roles.rs](../../../src/routes/roles.rs):655 | 原session完整proof先于角色／超管；非EMPLOYEE先拒，祖先支持集合只认ENABLED EMPLOYEE，客户不支持角色／缓存超管。 | R2 |
| [service.rs](../../../src/membership_lifecycle/service.rs):801/814 | 最终员工支持集合清理；删除／替换后仅清理失去剩余有效员工及祖先支持的角色，不按user全删、不让CUSTOMER支持员工角色。 | R2 |
| [data_grants/store.rs](../../../src/data_grants/store.rs):84/272；[discovery.rs](../../../src/data_grants/discovery.rs):156；[app_members.rs](../../../src/routes/app_members.rs):227/258 | grant写精确membership并核user／org／tenant／type／状态／版本；候选仅有效直接员工，返回user ID是业务DTO，不代替身份资格。 | R2/R8 |
| [auth.rs](../../../src/routes/auth.rs):61/333/365/399 | 展示／审计／选择票据锚点；可显示客户／停用且标不可选，find用于LOGIN审计，accounts.first只锚签名选择票据，保留全部验证集合，不直接签业务context。 | R4 |
| [auth.rs](../../../src/routes/auth.rs):510/642；[oauth2.rs](../../../src/routes/oauth2.rs):1472 | 明确选择后唯一性；候选多于一条先拒，员工context或password grant按client允许集合／准确成员过滤，零／多行拒后才[0]或remove(0)。 | R4/R5 |
| [oauth_issuance.rs](../../../src/oauth_issuance.rs):109；[app_token.rs](../../../src/app_token.rs):139；[app_authorization/store.rs](../../../src/app_authorization/store.rs):77 | 完整凭据JOIN以member ID定位，再核user／org／type／身份及账号代际／来源／app／父session；partial／旧proof不按当前关系补绑。 | R4 |
| [bff/context.rs](../../../src/bff/context.rs):69；[customer_portal/store.rs](../../../src/customer_portal/store.rs):87/276/338 | 两入口精确交集；固定EMPLOYEE／CUSTOMER，与RP／resource client／app／tenant／org相交，客户最终原source／组织／版本复验。 | R5 |
| [notifications.rs](../../../src/notifications.rs):238/425/543/555 | 员工广播／原收件身份；publisher绑定session／member／version；APP有效员工，SYSTEM按user／tenant／org去重后INSERT SELECT解析员工ID／version，客户不产生第二条员工收件。 | R6 |
| [notifications.rs](../../../src/notifications.rs):396/469；[routes/notifications.rs](../../../src/routes/notifications.rs):41/650 | 业务幂等／可见性；publication含publisher member／version，收件原proof，列表／详情／改删共用完整VISIBLE，prefs按member＋version；业务唯一键不误报成员二元键。 | R6 |
| [search.rs](../../../src/routes/search.rs):76/79；[organizations.rs](../../../src/routes/organizations.rs):139/256；[tenants.rs](../../../src/routes/tenants.rs):82/149/748；[monitor.rs](../../../src/routes/monitor.rs):79 | 展示／统计；scope内LIMIT1仅代表组织名称，不签凭据。人数DISTINCT user，删除依赖／审计count统计关系行，允许双身份两行。 | R7 |
| [0013](../../../migrations/0013_membership_identity_policy.sql):16/19；[readiness/schema.rs](../../../src/database/identity_readiness/schema.rs):105/146/180；[identity_readiness.rs](../../../src/database/identity_readiness.rs):346 | 模型／catalog／独立收缩；三元唯一、身份immutable、版本单调，二元约束实际存在／缺失受catalog验证，独立job收缩，runtime不自动迁移。 | R1/R9 |
| [0014](../../../migrations/0014_credential_identity_binding.sql):135/151/158；[0018](../../../migrations/0018_platform_bff.sql):123；[0020](../../../migrations/0020_identity_deployment.sql):233 | 当前有效trigger；成员按OLD.id局部撤派生，当前session活性为0018，0020账号代际bridge按user全局撤并记实际数量。0001／0013旧定义被替换，不改写历史字节。 | R4/R6/R10 |

回归键只标当前源码中的已有测试，不计为本轮执行：R1 [membership_lifecycle_tests.rs](../../../src/membership_lifecycle_tests.rs):873/1471/1592；R2 [employee_isolation_tests.rs](../../../src/employee_isolation_tests.rs):525/723/793/840/877/1249；R3 [remote.rs测试](../../../src/routes/users/remote.rs):547/681/708；R4 [identity_binding_tests.rs](../../../src/identity_binding_tests.rs):622/687/906与 [oauth_issuance.rs](../../../src/oauth_issuance.rs):386；R5 [customer_identity_tests.rs](../../../src/customer_identity_tests.rs):720、[customer_portal_tests.rs](../../../src/customer_portal_tests.rs):411/439/567及 [bff_tests.rs](../../../src/bff_tests.rs):2060；R6 [notification_identity_tests.rs](../../../src/notification_identity_tests.rs):494/564/703/758/798/963/1496；R7 [scope_tests.rs](../../../src/scope_tests.rs):292及 [organization_subtree_reporting_tests.rs](../../../src/organization_subtree_reporting_tests.rs):435/441；R8 [data_grant_discovery_tests.rs](../../../src/data_grant_discovery_tests.rs):660/843/1055及 [management_audit_tests.rs](../../../src/management_audit_tests.rs):729；R9 [readiness/tests.rs](../../../src/database/identity_readiness/tests.rs):86；R10 [identity_deployment_tests.rs](../../../src/identity_deployment_tests.rs):515/645。

测试文件和remote cfg(test)内省略type的INSERT／负例查询／故障trigger不计生产writer。目录业务owner／assignee账号、ChatRoom的socket.id、历史migration与scope展示LIMIT1分别归类，不因名字含user/org就改成成员凭据。最终本阶段运行仍须证明旧凭据拒绝、通知／恢复和catalog闭环。

## 活动消费者与legacy合同登记

当前八仓只读消费者审查完成，未发现新安全阻断。范围为六旧仓platform／SDK／stores／work-orders／notification／examples与新identity／portal。源码、登记fixture、真实任务流量与生产流量分开；以下路径／行号是当前读取时点，既有回归只证明测试源码存在，不冒称本轮运行。

| 消费者／载体 | 当前合同与准确源码位置 | 运行与退役限制 |
| --- | --- | --- |
| 平台员工BFF | [protocol.rs](../../../src/bff/protocol.rs):291固定PLATFORM_CONSOLE＋EMPLOYEE；[context.rs](../../../src/bff/context.rs):69精确员工；[mod.rs](../../../src/bff/mod.rs):118逐请求原BFF／sid／代际／policy复验；[authentication-mode.ts](../../../../frontend/src/lib/authentication-mode.ts):3先定模式。 | 上游失败不切登录方式；BFF服务端OIDC refresh不同于公开legacy refresh。 |
| 平台psr_兼容refresh | [frontend api.ts](../../../../frontend/src/lib/api.ts):40与 [oauth2.rs](../../../src/routes/oauth2.rs):1059实际POST /oauth/token调用session::refresh，拒client credentials／scope扩大。 | 不能虚构/api/auth/refresh，也不能与legacy OAuth或/oidc/token混成一个namespace。 |
| 活动SDK3 | [release.json](../../../../../saas-micro-app-sdk/release.json):3、[generated](../../../../../saas-micro-app-sdk/generated/protocol.mjs):2固定3.0.0／MicroProps2.0／Context1.0／Notification1.0；[http.mjs](../../../../../saas-micro-app-sdk/src/http.mjs):82/107捕获Bearer、credentials omit；[notification-publisher](../../../../../saas-micro-app-sdk/src/notification-publisher.mjs):14仅HTTP发布。 | REPOSITORY_CANDIDATE／private package不是外部已发布；SDK不登录、不refresh、不建Socket.IO，DTO真实名MicroProps／AppAuthorizationContext。 |
| SDK1／2归档 | [release.json](../../../../../saas-micro-app-sdk/release.json):25与 [generation.test.mjs](../../../../../saas-micro-app-sdk/tests/generation.test.mjs):94规定不可变归档及逐文件digest。 | 历史字节与当前vendor按最终不变量另验，不把旧README的SDK2标签当活动SDK2消费者。 |
| stores | [config.rs](../../../../../saas-stores/backend/src/config.rs):217默认legacy；[authorization/client.rs](../../../../../saas-stores/backend/src/authorization/client.rs):77固定legacy introspect＋context或显式combined /authorize；[authorization.rs](../../../../../saas-stores/backend/src/authorization.rs):77逐请求有界校验；[policy.rs](../../../../../saas-stores/backend/src/authorization/policy.rs):229/253分别校验introspection与精确context。 | 失败直接传播，无fallback；生产src不登录或兑换/oauth/token；3s／1s及actor／目录witness不放宽。 |
| work-orders | [config.rs](../../../../../saas-work-orders/backend/src/config.rs):218默认combined；[client.rs](../../../../../saas-work-orders/backend/src/authorization/client.rs):77仍支持显式legacy；[authorization.rs](../../../../../saas-work-orders/backend/src/authorization.rs):69新取授权；[member_contracts.rs](../../../../../saas-work-orders/backend/tests/member_contracts.rs):1013显式同assignee重验。 | 无fallback／成功授权缓存；后端HTTP contract1.0与MicroProps2／SDK3分开，业务owner userID不代替身份。 |
| notification | [protocol.rs](../../../../../saas-notification/src/protocol.rs):113/163/186严格{token}、公开5／12字段；[platform.rs](../../../../../saas-notification/src/platform.rs):66固定proof；[server.rs](../../../../../saas-notification/src/server.rs):180/291逐条及emit前后复验；[store.rs](../../../../../saas-notification/src/store.rs):25原recipient。 | 不登录／签发／refresh；周期proof不改绑／续期限，客户首版无通知；持久行、hint与连接member／type／version相交。 |
| machine／CLI | [oauth2.rs](../../../src/routes/oauth2.rs):1366支持现有confidential MACHINE／LEGACY_MIXED；[bootstrap.rs](../../../tools/platform-admin/src/bootstrap.rs):93新登记严格MACHINE＋[]＋client_credentials及EMPLOYEE app；[http.rs](../../../tools/platform-admin/src/http.rs):57/141仅输入Bearer／同源/api。 | CLI无登录／密码／兑换／refresh；登记不是实际机器流量，新规则不等于全部LEGACY_MIXED已退役。 |
| Vue／jQuery／报表／聊天 | [Vue app.js](../../../../../saas-examples/micro-vue/public/app.js):150、[jQuery](../../../../../saas-examples/micro-jquery/public/app.js):83用props.platform；[报表](../../../../../saas-examples/report-center/src/app.tsx):148用app Bearer／omit／signal；[聊天lifecycle](../../../../../saas-examples/websocket/src/qiankun-lifecycle.ts):14、[server](../../../../../saas-examples/websocket/backend/src/server.rs):86为匿名socket.id。 | 聊天是挂载期授权，不是member writer／持久通知消费者；源码示例不当生产流量。 |
| OAuth playground | [http.rs](../../../../../saas-examples/oauth-playground/backend/src/http.rs):118/255、[upstream.rs](../../../../../saas-examples/oauth-playground/backend/src/upstream.rs):127实际legacy machine/password/code＋PKCE/device/refresh统一/oauth/token；[config.rs](../../../../../saas-examples/oauth-playground/backend/src/config.rs):51默认password关闭。 | 缺配置不广告能力，各流程独立client；不是标准OIDC RP，不证明生产旧流量。 |
| 独立portal | [protocol.rs](../../../../../saas-customer-portal/backend/src/protocol.rs):288固定CUSTOMER_PORTAL＋CUSTOMER；[resource.rs](../../../../../saas-customer-portal/backend/src/resource.rs):188客户验证；[config.rs](../../../../../saas-customer-portal/backend/src/config.rs):163独立resource client；平台 [store.rs](../../../src/customer_portal/store.rs):251最终source proof。 | OIDC／refresh／opaque留服务器；门户DTO不是SDK Context1，不向员工API／通知开放。 |
| 独立identity | [Cargo.toml](../../../../../saas-identity/Cargo.toml):7无platform／portal业务crate；[lib.rs](../../../../../saas-identity/src/lib.rs):52装配账号／SSO／OIDC；[internal.rs](../../../../../saas-identity/src/internal.rs):127/150私有单用途服务与customer proof。 | service Basic不是人员proof，稳定issuer／独立keyset／两RP合同不因切流改变。 |

当前回归来源：stores [client_tests.rs](../../../../../saas-stores/backend/src/authorization/client_tests.rs):26/310、[witness_tests.rs](../../../../../saas-stores/backend/src/authorization/witness_tests.rs):29与 [postgres_contracts.rs](../../../../../saas-stores/backend/tests/postgres_contracts.rs):980；work-orders [authorization/tests.rs](../../../../../saas-work-orders/backend/src/authorization/tests.rs):494/552、[member_contracts.rs](../../../../../saas-work-orders/backend/tests/member_contracts.rs):1046/1534；notification [identity_isolation.rs](../../../../../saas-notification/tests/identity_isolation.rs):774/839/890；平台 [bff_tests.rs](../../../src/bff_tests.rs):2004/2245/2342与 [customer_portal_tests.rs](../../../src/customer_portal_tests.rs):411/567；SDK [client.test.mjs](../../../../../saas-micro-app-sdk/tests/client.test.mjs):54、playground [upstream_contract.rs](../../../../../saas-examples/oauth-playground/backend/tests/upstream_contract.rs):302/520、CLI [bootstrap.rs](../../../tools/platform-admin/src/bootstrap.rs):630。业务PG的上游authority为HTTP fixture，不当平台真实成员DB；这些是既有测试源码，不计本轮执行。

## 退役N/A与文档同步

本TASK_OWNED身份新库无历史账号／凭据导入，也无已登记计划退役的历史客户端；历史转换、停用日期及观察窗口只在这一范围N/A。活动SDK／app派生／机器grant／CLI／playground／legacy及combined调用仍支持并受新撤销边界保护；其他部署是否有旧客户端未知，不编owner、流量或停用日期。

当前文档修正清单：工作区README的活动SDK2.0.0→3.0.0及六→八仓／补identity和portal导航、notification README的SDK2.0→3.0.0、platform-admin README的当前内置SDK2.0.0→3.0.0；主identity README纠正“只交付1～10”“认证工程尚待创建”为已交付1～12及两新工程已存在。修正仅当前文字，不改archive／vendor、历史准备、台账或Phase1～12证据。其它当前漂移逐项记证，不能扩成重写历史。

## 封账要求与证据安全

保持五项PENDING直到正式fresh运行包含真实35分钟、灰度故障回退、双库／库外材料恢复、旧密码／凭据拒绝、新两RP与通知、最终逐命中审查及全部清理。preflight即使完成也保持NOT_ACCEPTANCE，未测量或失败项保留原样，修复后另轮重跑，不把局部计数相加成独立场景。

只保存固定类别、状态、字段名、公共pins、计数、原始时点与字节／SHA；不输出密码、PHC、token、Cookie、Basic、secret、env、连接串、完整私有fixture或raw工具诊断。外部本地witness不是灾备冗余，ID Token离线签名仍可能有效；必须实际证明旧在线授权与业务Bearer被拒。首个客户业务维持本人资料，生产默认关闭与UUID专项独立。
