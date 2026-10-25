# Phase 11 独立客户门户 BFF 与本人资料

状态：**COMPLETED**。范围与证据见 [台账 Phase 11](task-ledger.md)及 [验收记录](validation/phase-11.md)，[Phase 10](validation/phase-10.md)保持封账。前端合法0账号版本修正后36／36及lint／typecheck／build通过；portal12普通／5PG、平台197普通／226既有／3rollout、新7组及165列catalog全部通过。最终真实browser3／driver3均PASS，196条检查记录含transport／control辅助记录，不能称196个独立场景；完成双入口SSO、精确客户profile、短期续期、双向成员局部撤销、跨tab、outage／uncertain恢复、PORTAL／GLOBAL注销及不同账号隔离。完整7项runtime报告、OIDC_REFRESH=4、三进程exit0／SHA不变／完整扫描无命中、两migrate exit0及两DB／role／hosts清理均有结果；8张最终PNG逐张核验通过，bridge退出0。三条Self-Check全部PASS，前两轮FAIL与TLS限制保留，TASK_OWNED完成不代表生产上线。

## 工程位置与最小范围

root已在创建前核对同级路径不存在，随后将独立Git仓库初始化为main：[`saas-customer-portal`](../../../../saas-customer-portal/)，工程分为 [`backend`](../../../../saas-customer-portal/backend/)和 [`frontend`](../../../../saas-customer-portal/frontend/)。这是本阶段已确定的实际落位，不再只是候选名称；初始化仓库不等于服务已启动或验收完成。

门户backend采用Rust薄main、厚lib，入口只负责配置与依赖装配、启动和退出，流程规则及可替换端口留在库内。门户维护独立PostgreSQL会话数据库；平台保存客户资源权威，门户不能借同库或跨仓业务crate调用绕过平台资源边界。认证provider的独立进程部署属于Phase 12。

frontend采用React 19、Vite 6和TypeScript，界面方向为暖纸色与墨蓝色的本人资料页面。只提供统一登录、有效客户身份选择、当前客户context、本人最少资料及注销；不复制管理宿主，不增加客户通知菜单、角色菜单、自助注册、订单、工单或支付能力。首个真实客户业务另立范围，不由本阶段推断。

## 身份、凭据与存储 owner

| 角色 | 已确定职责 | 必须保持的边界 |
| --- | --- | --- |
| 门户自有OIDC RP | 独立登记 `purpose=CUSTOMER_PORTAL`，`allowed_member_types`只含 `CUSTOMER`；以自己的client完成标准code＋S256登录、证明校验及刷新。 | 不使用平台 `PLATFORM_CONSOLE` RP或其他允许双类型的client借准入；同账号有效员工关系不自动成为客户关系。 |
| 独立profile服务client | 使用已登记的USER confidential client通过Basic认证调用平台内部客户资源接口，绑定 `CUSTOMER`、`profile`及固定应用。 | 该服务client与门户OIDC RP分别登记。已有 `CUSTOMER_PROFILE_CLIENT_REF`是客户端数据库主键，`CUSTOMER_PROFILE_APPLICATION_ID`是精确应用主键；两者不能与公开OIDC client ID混用。 |
| 平台客户资源owner | [`0019_customer_portal.sql`](../../migrations/0019_customer_portal.sql)已加入 `customer_portal_contexts`，维护独立opaque context的签发、持久证明、校验与撤销，并复核具体客户成员和应用交集。 | 不把客户写为0018员工platform context，不将客户opaque当作平台session、app token或普通legacy OAuth token。新增10个UUID／10个独立v7 CHECK，无UUID默认值；完整165列catalog的exact CHECK／无UUID默认值与非法版本／variant负例已由最终既有PG测试实际通过。 |
| 门户服务端 | 唯一保管门户获得的OIDC access／refresh及平台resource opaque，使用AES-GCM保存可恢复密文；维护独立会话、登录flow、代际及pending命令。 | 不把凭据返回浏览器、放进URL、日志或广播。独立外部keyring与AAD绑定合同见门户README／crypto实现，普通／PG及最终live分别验证；不复用平台keyring或据任务配置宣称生产已部署。 |
| 浏览器 | 仅接收防护Cookie、portal session DTO、CSRF和本人profile；内存状态按版本与epoch协调。 | 无Bearer、OIDC refresh或平台resource opaque；不引入客户通知socket或SDK token消费合同。 |

三个Cookie分别为 `__Host-saas_customer`、`__Host-saas_customer_browser`和 `__Host-saas_customer_login`，均Secure、HttpOnly、Path=/、SameSite=Lax且无Domain，不借用平台或SSO Cookie。SSO复用来自provider的现有认证会话，门户和平台各自仍须建立本RP与具体成员的授权证明。最终browser3实际验证两入口Cookie独立、host-only及防护属性，不以共享Cookie实现登录复用。

## OIDC与HTTP合同

门户使用自己的准确issuer、client和已登记 `/auth/callback`，执行标准authorization code＋S256。登录flow绑定state、nonce与发起browser；callback验证严格ID Token、可信JWKS、UserInfo及自身client认证的introspection，不能只验签后直接枚举或签发任意组织身份。普通／PG和真实TASK_OWNED登记／登录分别记证，实际生产域名及client不由任务验收推定。

HTTP合同已确定为共享总预算3秒、连接预算1秒、并发上限16，关闭proxy、redirect和retry。固定可信端点、严格TLS和原始来源证明共同构成准入条件；上游不可用不能改走本地password登录、仅缓存签名验证或另一issuer。Discovery／JWKS读取和刷新并发细节以实际实现及边界测试冻结，本页不额外引入缓存或重试设计。

OIDC access与refresh均留门户服务端。刷新采用独立会话的版本约束和uncertain状态防止重放；结果未知不能再次发送旧refresh或把失败当作有效续期。最终lease、deadline及原始快照提交检查由普通／PG验证；browser3实际覆盖access到期后的profile续期、outage503、人工retry仍503且不显示旧profile、上游恢复后仍需用户显式reconfirm。

## 浏览器API合同

下表合同已落于门户 [`routes.rs`](../../../../saas-customer-portal/backend/src/routes.rs)等源码。session固定字段为contractVersion、version、csrfToken、account、memberships、context，版本以规范十进制字符串表示；接口存在仍不代表真实provider／DB／浏览器闭环已执行。

| 方法与路径 | 请求与返回合同 | 身份与并发约束 |
| --- | --- | --- |
| `GET /auth/config` | 启用时mode=BFF，否则为DISABLED。 | 上游失败不改变为密码或legacy模式。 |
| `GET /auth/session` | 合同标识 `portal-session/1`；未带session Cookie的匿名返回200，version='0'、account=null、memberships=[]、context=null及csrfToken／browser Cookie。 | 无效旧session Cookie继续401，权威不可用503，不把它们降为匿名成功；匿名200不授予客户context。 |
| `GET /auth/login-bootstrap` | 只供显式login／reconfirm取得非授权匿名session，version='0'及独立browser CSRF，即使旧HttpOnly session Cookie仍在也可使用。 | 普通载入、401、503、signed-out落地不自动调用；该GET不授予账号／成员／resource opaque，用户随后仍须以CSRF POST登录。 |
| `POST /auth/login` | body为 `returnTo:'/'`；成功返回redirectTo。 | Cookie＋CSRF＋准确Origin共同校验，由用户操作发起统一登录；不扩展任意return URL。 |
| `GET /auth/callback` | 严格解析issuer／state／code，绑定独立login Cookie及browser generation，一次消费flow并验证code兑换后跳回已检查本地地址。 | callback不借另一browser／旧代际覆盖当前账号；边界由普通／PG验证，真实code＋S256登录由最终browser3记录。 |
| `POST /auth/context` | body包含expectedVersion和membershipId；返回同一portal session合同。 | 精确选择本账号当前有效CUSTOMER关系；比较原版本，不能按首条成员、员工关系或新请求状态补旧证明。 |
| `GET /api/profile` | 返回Phase 6客户context与四字段account：id、username、realName、avatar。 | 只读本人；服务端使用当前已验证客户证明，不以客户端传入的user／tenant／org／成员ID代读。 |
| `POST /auth/logout` | body包含expectedVersion及scope=`PORTAL`或`GLOBAL`；完成后返回 `redirectTo:'/signed-out'`。 | durable pending保存原始授权命令；迟到响应不发送清除新Cookie的指令。不可把GET恢复变成新的注销授权或领养新会话。 |

frontend维持自己的版本、epoch和abort控制。跨标签页只广播失效或版本等metadata，不广播Cookie、CSRF、OIDC凭据、resource opaque或profile正文。切换／注销／恢复中旧请求不能覆盖新身份，状态不明时不保留可用旧资料。协调器／transport回归及最终browser3跨tab与Storage检查分别验证这些边界；具体metadata实现以已冻结源码为准。

## 平台内部资源与证明端口

平台 [`customer_portal`](../../src/customer_portal/mod.rs)源码提供四个POST：`/internal/customer-portal/memberships`、`/internal/customer-portal/context`、`/internal/customer-portal/profile`、`/internal/customer-portal/revoke`，完整前缀不含 `/api`。这条服务端链与浏览器 `/auth/*`、`/api/profile`路径分开。

| 内部操作 | 已确定用途 | 校验边界 |
| --- | --- | --- |
| memberships | 根据原始OIDC account proof列出本账号的有效CUSTOMER候选。 | 仅客户行；同组织员工行不补充角色或扩充允许类型。 |
| context | 根据明确membershipId建立独立平台opaque客户context。 | 精确客户成员、tenant／organization、来源RP、profile服务client和固定app交集；平台owner签发并持久保存证明。 |
| profile | 根据当前opaque和原始来源读取本人资料。 | 原source完整字段和客户／应用交集在读取前后复核；不只凭opaque存在或缓存DTO放行。 |
| revoke | 撤销门户对应的客户opaque资源证明，平台成功响应200 JSON `{revoked:true}`。 | 精确原始来源与受权范围，门户严格解析DTO且只接受true；12普通／5PG和最终真实PORTAL／GLOBAL注销通过，修复前11普通／5PG独立封存，不改写历史覆盖范围。 |

每次内部调用以独立登记的profile USER confidential client做Basic认证。JSON `oidcAccessToken`仅作账户来源proof，不替代Basic服务认证，也不当作浏览器Bearer或平台资源访问token；选择请求另含membershipId、organizationId和expectedIdentityVersion，profile／revoke另含customerContext。源证明固定原access SHA、issuer、RP client、grant、global sid、user、account version、client policy version、authTime及sourceExpiry，网络或数据库等待后不能接受一个新来源来修复原请求。

`CustomerAccountProofPort`由客户资源使用方定义。Phase 11当前确定采用本地OIDC access-only adapter，准确绑定新增 `CUSTOMER_PORTAL_OIDC_CLIENT_ID`、`purpose=CUSTOMER_PORTAL`及仅CUSTOMER允许集合。它不接受ID Token、平台context、普通legacy OAuth或任意来源client作为同等account proof；不放宽0018的EMPLOYEE检查。Phase 12再将此port装配为HTTPS适配器，当前阶段不提前宣称认证服务已拆进程。

0019还加入认证owner的窄boolean投影 `customer_portal_account_source_active`，仅以完整原access SHA与source tuple作deny检查，不返回任何凭据。当前为SECURITY INVOKER、固定search_path，并撤PUBLIC的EXECUTE；false拒绝，true仍须完整的前后 `CustomerAccountProofPort`在线验证。资源最终检查使用新的SQL statement，重新核对原source、member、租户／组织期限、profile client／app／scope交集及opaque撤销，不能把单次transaction snapshot或同库bool true当作成功授权。平台新7组PG已独立7／7、7.67秒通过；165列catalog也已由最终既有测试实际通过，两类运行证据分别登记。

Phase 12必须确立认证表／projection的单一migration owner并收紧平台对原始auth／credential表的权限。最终deny checkpoint可以在独立审计下保留窄EXECUTE的boolean projection，例如需要时采用固定search_path、准确owner且不输出凭据的SECURITY DEFINER，或采用等效HTTP收尾；不能在没有等效最终检查时预定撤销全部function EXECUTE。保留投影也不替代在线account proof。

## 已确认配置与任务PKI

平台配置已加入 `CUSTOMER_PORTAL_PORT_ENABLED`，默认false，另以 `CUSTOMER_PORTAL_OIDC_CLIENT_ID`固定门户RP，并复用准确成对的 `CUSTOMER_PROFILE_CLIENT_REF`／`CUSTOMER_PROFILE_APPLICATION_ID`。开启入口不创建client、成员或应用授权。

门户完整变量合同见其 [`backend/README.md`](../../../../saas-customer-portal/backend/README.md)：`CUSTOMER_PORTAL_ENABLED`默认false；独立DATABASE_URL、PUBLIC_CUSTOMER_URL、OIDC_ISSUER／CLIENT_ID／CLIENT_SECRET／REDIRECT_URI、CUSTOMER_RESOURCE_URL／CLIENT_ID／CLIENT_SECRET、PORTAL_TOKEN_KEY_FILE及两个可选CA_FILE分别配置。RP与resource服务client及其secret不能混用，portal-migrate与runtime的DB职责分开。当前只登记源码合同，未据此声称生产配置或所有非法配置负例已验证。

任务PKI已prepare独立CA与CA:false leaf，SAN为auth.saas.test、console.saas.test、customer.saas.test，证据只保留 [pki-result.json](../../../local-identity-sso-phase11-qa/pki-result.json)的公共pins和状态；私密文件不落repo，console与portal使用独立AEAD keyring。该准备结果不等于严格TLS实际网络、双入口登录或生产域名登记已验收。

本人资源保持 [Phase 6 `customer-profile/1`](phase-06-customer-profile.md)的context和四字段account边界。Phase 6普通OAuth入口原有合同与本阶段opaque新入口分属不同来源，新增门户不复用password grant、不导入历史refresh，也不把旧JWT凭据改成新opaque。每次profile均须检查原始source全字段与精确CUSTOMER／app交集，完成后再核一次；权威不可确认返回503，已确认的失效按最终错误合同拒绝，禁止沿用旧profile、员工权限或另一客户关系。

## 会话、注销与双身份隔离

门户只有账号认证、选中具体客户成员后的资源context，以及当前本人资料三个层次。无成员或仅有员工关系时可以完成账号认证，但客户候选为空；不自动新建关系、选择员工、授予管理员能力或隐藏错误成员输入。

同账号同组织双身份必须在平台入口选员工、在门户入口选客户，各自Cookie、数据库会话、缓存key、版本与资源凭据独立。客户局部撤销不得破坏仍合法的员工context，员工局部撤销也不得移除仍合法客户身份；全局账号撤销则使两端原账号会话失效。客户context访问后台不能获得员工角色或超级管理员能力。页面没有客户通知菜单，不据此宣称旧通知／actor／owner隔离已被重新验证，相关继承与实际回归范围必须分开记证。

PORTAL注销与GLOBAL注销均绑定expectedVersion和原始授权命令。结果不明时持久pending，恢复只确认同一命令；迟到响应不能清除较新的登录Cookie或更换scope。竞态／幂等由普通／PG验证，最终browser3实际证明PORTAL撤旧门户profile但保留平台及全局SSO，GLOBAL即时撤原账号两入口而保留不同账号与员工专用账号SSO，注销后页面及Storage无旧profile／持久凭据。

## 最终证据、历史失败与阶段边界

前端旧 [frontend-static-summary.json](../../../local-identity-sso-phase11-qa/frontend-static-summary.json)保留27／28固定microtask fixture失败→订阅28／28→bootstrap新增5项后33／33的历史。第二轮live暴露0版本bug后，[frontend-account-version-repair-result.json](../../../local-identity-sso-phase11-qa/frontend-account-version-repair-result.json)先实际复现36项中33成功／3失败，再修正为36／36、lint／typecheck／build全通过。accountVersion允许规范非负i64十进制字符串，已认证session／identityVersion仍为正数，负数／padding／numeric／overflow和profile换账号代际仍拒绝。最终build为index-C792xhk5.js，SHA256 `5fc5e9381c93365b96f704db5bb7a3f2c918950e117ea0095e8d8afebebe0e94`；两份receipt都未保存原stdout／耗时，不补造。portal严格revoke后的12普通／5PG及静态全链另独立通过。

平台曾因unused header import发生Clippy失败，删除未使用import后继续；第一全链普通168成功／1失败是新runner遗漏既有test-artifacts可写volume，static_files遇ROFS，补回原有挂载。第二链fmt／check／Clippy／普通197（main169＋其他28）通过，但新7组都因fixture保留旧二元unique而预备失败、七库全清理。只修 `customer_portal_tests`的marked fresh empty任务库fixture：删除旧二元key、保留三元key，状态mutation显式提升identity_version；未修改生产收缩流程或放宽断言。随后fmt与新7组独立7／7、7.67秒通过，见 [result](../../../local-identity-sso-phase11-qa/platform-new-pg-result.json)。

最终 [platform-gates-summary.json](../../../local-identity-sso-phase11-qa/platform-gates-summary.json)串链已exit0、全项通过：fmt／check／Clippy、普通197（main169＋其他28）、既有226（含Phase 10旧BFF14，本体1109.94秒／命令1112.71秒）、rollout3（本体251.72秒／命令254.33秒）、normal build和doc。新7已有独立通过回执，本串链明确skip新customer_portal组，不重复。`uuid_v7_target_enforces_versions_variants_and_application_generated_ids`实际通过165列catalog；旧升级与fresh seed日志也记录post_uuid_columns=165、defaults=0，3→19升级保留原行／checksum并拒绝腐坏。

所有Cargo已结束，[binary-manifest.json](../../../local-identity-sso-phase11-qa/binary-manifest.json)固定平台SHA256 `d7857845837b45281001601a69881a007bceabd1d9c0bae52e92c9da5bde2dd8`及门户 `5f421019b202e5e478b93ee55104002f9fe5af1bb09246b2967e642bb33f3408`，两migrate摘要同档记录。首轮真实 [browser-round-01-result.json](../../../local-identity-sso-phase11-qa/browser-round-01-result.json)为FAIL／CONSOLE_EMPLOYEE_LOGIN／OWNED_PLAYWRIGHT_WAIT_TIMEOUT，17条成功检查记录含辅助记录，不称17个独立场景。console真实 `/api/sso/login`已200且SSO_LOGIN审计=1，后续helper重复fill竞态导致等待超时；不能把服务端登录200写成平台／门户完整回调或业务通过。pageerror=0、passwordSubmissions=1也不改变该轮FAIL。

对应 [live-round-01-evidence.json](../../../local-identity-sso-phase11-qa/live-round-01-evidence.json)driver整体FAIL／PHASE11_REAL_OIDC_REFRESH_NOT_EXERCISED。bootstrap／platform／portal三个进程均PASS／exit0、binary不变、stdout／stderr读取完整且敏感模式扫描无命中；两任务DB、portal role及容器hosts全部清理，成员未新增或改类型、memberless仍无成员。launcher另有 [独立结果](../../../local-identity-sso-phase11-qa/browser-round-01-launch-result.json)，exit1、全流扫描无命中、rawOutputStored=false。首轮工具失败及清理记录保留，后续第二轮仍有独立真实失败。

首轮helper已修，见 [helper-repair-round-02-result.json](../../../local-identity-sso-phase11-qa/helper-repair-round-02-result.json)：browser-check.mjs SHA256为 `9aabbcf608b0f432cf306f6230d450b9be1c3b4c24ed56d9705d9ff8ecbcf47d`，Node语法与17项纯内存回归通过，保留原35秒预算、精确RP／consent和callback要求，未放宽断言。该receipt无browser／DB／Cargo／service操作，不当作live成功或与原32项累加为独立业务场景。

第二轮 [browser-round-02-result.json](../../../local-identity-sso-phase11-qa/browser-round-02-result.json)为FAIL／PORTAL_REUSES_GLOBAL_SSO／OWNED_PLAYWRIGHT_WAIT_TIMEOUT，26条记录。实际平台员工选择、门户无第二密码复用SSO、准确客户候选与两Cookie独立已走到；context POST200后，前端parseContext错误拒绝合法accountVersion='0'，抛ProtocolError而无profile。backend独立确认schema允许初始0，这是前端产品bug；随后36项回归先33成功／3失败，再36／36及最终第三轮live完整通过。原失败及第二轮driver controls缺失分类均保留。

对应 [live-round-02-evidence.json](../../../local-identity-sso-phase11-qa/live-round-02-evidence.json)driver为FAIL／PHASE11_REQUIRED_LIVE_CONTROLS_MISSING，已真实记录OIDC_REFRESH=1、BFF_CONTEXT_RENEW=1／SELECT=1／LOGIN=1，不能沿用首轮“未覆盖refresh”的失败类别。三进程仍PASS／exit0／SHA不变、完整流扫描无命中，两DB／portal role／hosts全部清理，成员未新增／改类型且memberless仍无成员；这些结果不覆盖browser2失败或未执行的后续controls。

最终 [browser-round-03-result.json](../../../local-identity-sso-phase11-qa/browser-round-03-result.json)为PASS，196条记录含transport／control等辅助项，不称196个独立场景，pageerror=0。实际验证同浏览器平台员工→门户客户复用SSO、精确本人四字段profile、无员工导航／代读／伪造成员、真实短期续期、客户停用不伤员工及反向局部撤销、跨tab客户切换保留员工、outage503／uncertain不重放及显式reconfirm、员工专用／不同账号隔离、两种注销、响应no-store及各阶段Storage无持久凭据。

最终 [live-round-03-evidence.json](../../../local-identity-sso-phase11-qa/live-round-03-evidence.json)driver PASS，collection=COMPLETE／missing_reports=[]（7项），OIDC_REFRESH=4，三进程均exit0／SHA不变／完整stream读取且扫描无命中，两migrate均exit0且SHA不变，两任务DB／portal role／hosts清理true，未新增或改成员类型、memberless仍无成员。第三轮launcher exit0且全流无凭据命中；root逐张view_image核验8张最终PNG，见 [visual-review.json](../../../local-identity-sso-phase11-qa/visual-review.json)，第二轮失败截图另保留；临时bridge也已停止并exit0。

最终Edge仍使用ignoreHTTPSErrors接受任务CA leaf；Node和两个BFF严格验证任务CA与准确hostname，未更改系统信任或生产DNS。该限制在最终receipt中保留，不能写成浏览器系统信任或生产TLS已部署。helper32项及修复17项仅各自source／内存证据，不替代最终live。

最终 [source-snapshot.json](../../../local-identity-sso-phase11-qa/source-snapshot.json)已重封527项source／配置／QA helper选择集（platform493／portal34），文件SHA256为 `074912b72fce21765f0439e7ff189c31614031a865cf771b8b140b293d931538`，content set为 `62d7633a2658ca22badc03390fd1c4c39a3a8e9a18225e787fd3afeba73ea71d`；动态ledger／导航／本阶段文档不纳入source seal。最终 [invariant-result.json](../../../local-identity-sso-phase11-qa/invariant-result.json)SHA256为 `a611ebb5553c5d3878a6b5c868e6b384319ffa3cb9c70497dd9ac09cb3175ea5`，重核源码字节、历史migration、SDK／依赖／凭据边界、实际catalog及live／视觉／清理证据。旧LIVE_PENDING文件封存为历史，不充当最终seal；三条Self-Check全部完成。

所有验收结果见 [Phase 11记录](validation/phase-11.md)，设计、源码、fixture和真实运行分别记证。三条台账标准已有实际完成证据，不以仓库初始化或历史阶段替代门户端到端结果。生产域名、client secret、数据库账号和运维归属仍由实际环境落实；TASK_OWNED完成不代表生产上线、独立认证部署或灰度退役，默认创建与生产开关边界保持。
