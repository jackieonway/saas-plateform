# Phase 11 验收记录

状态：**COMPLETED**。范围依据 [主台账 Phase 11](../task-ledger.md)，架构见 [独立客户门户](../phase-11-customer-portal.md)，[Phase 10](phase-10.md)保持封账。前端0版本修正后36／36及lint／typecheck／build通过；portal12普通／5PG、平台197普通／226既有／3rollout、新7组及165列catalog全过。最终browser3／driver3均PASS，196条检查记录含transport／control辅助项，非196个独立场景；完成双入口SSO、客户本人profile、真实续期、双向局部撤销、跨tab、outage／uncertain恢复、PORTAL／GLOBAL注销、账号隔离及Storage。7项runtime报告完整、OIDC_REFRESH=4、三process及两migrate exit0／SHA不变／完整流扫描无命中，两DB／role／hosts清理，8张最终PNG逐张核验通过、bridge退出0。527项最终snapshot与invariant已重封，**三条Self-Check全部PASS**；前两轮FAIL和TLS限制保留，TASK_OWNED完成不代表生产上线。

## Self-Check：台账标准

| 验收项 | 结果 | 最终运行证据与范围 |
| --- | --- | --- |
| 同浏览器先登录平台再进入门户可复用有效SSO；两个入口持有正确组织身份及独立会话 | **PASS** | 最终browser3实际平台员工→门户客户复用同一SSO而无第二密码，精确选择本人客户成员、四字段profile及独立Cookie；access到期后的实际续期、PORTAL／GLOBAL注销均通过，driver3 OIDC_REFRESH=4。 |
| 客户只看本人资料；员工专用、客户停用、错误组织及伪造成员ID不能绕过准入 | **PASS** | 平台新7／portal5PG证明内部来源／opaque／固定client-app／错组织等边界；browser3另实际拒员工、他人／非规范成员、account注入、profile代读query和管理API，员工专用账号无自动客户关系，停用客户立即撤profile、outage503不返回旧资料。 |
| 同组织双身份独立，全局注销两端生效；成员局部停用双向不误伤，缓存／通知／actor／owner不串身份 | **PASS** | browser3实际客户停用保留同组织员工，员工停用保留客户，跨tab客户切换保留员工；GLOBAL立即撤原账号两入口而保留不同账号／员工专用账号SSO。定向与既有226／rollout回归保持通知及actor／owner边界，门户没有客户通知或员工导航，最终Storage／视觉均无旧资料或持久凭据。 |

## 已确定事实与合同

以下列出已冻结的合同与实现事实；对应运行层级和实际结果在后表分别登记，不以源码存在替代执行证据：

- 创建前已核对同级 `saas-customer-portal`路径不存在，现已初始化独立Git仓库main；包含backend／frontend。Rust采用薄main厚lib和独立PG会话数据库；frontend采用React 19、Vite 6、TypeScript及暖纸墨蓝的本人资料方向。
- 门户自有OIDC RP仅 `CUSTOMER_PORTAL`／CUSTOMER，与平台员工RP、独立profile USER confidential服务client和平台资源owner分别处理。
- 门户服务端AES-GCM保管OIDC access／refresh及resource opaque；浏览器仅防护Cookie、session DTO、CSRF和本人profile，无Bearer／refresh与客户通知菜单。Cookie为 `__Host-saas_customer`、`__Host-saas_customer_browser`、`__Host-saas_customer_login`，均Secure／HttpOnly／SameSite=Lax／Path=/且无Domain；最终browser3实际验证两入口属性／独立性及各阶段Storage无持久凭据。
- 标准code＋S256、state／nonce、严格ID Token／JWKS、UserInfo与自身RP introspection；HTTP共享3秒、连接1秒、并发16，关闭proxy／redirect／retry。
- 平台 [`0019_customer_portal.sql`](../../../migrations/0019_customer_portal.sql)新增platform owner的customer_portal_contexts，10UUID列各有独立v7 CHECK且无UUID默认值；最终既有PG已实际核165列catalog／exact CHECK／无UUID默认值。完整内部prefix为 `/internal/customer-portal`，四个操作均POST，Basic认证准确profile USER confidential client，JSON oidcAccessToken仅作来源proof；revoke成功200 JSON `{revoked:true}`必须严格解析并为true，不能仅凭HTTP成功确认撤销。
- `CustomerAccountProofPort`当前为本地OIDC access-only adapter，固定新增 `CUSTOMER_PORTAL_OIDC_CLIENT_ID`、CUSTOMER_PORTAL用途和仅CUSTOMER集合；Phase 12再装配HTTPS，不修改0018员工约束、不借password或平台员工session取得客户身份。
- 每次profile按原source完整字段与准确客户／app交集读前读后复核；权威不可确认503拒绝继续。独立版本／epoch／abort、仅metadata广播、durable pending注销和uncertain refresh共同防止重放或迟到响应覆盖新身份。
- 0019认证owner窄bool `customer_portal_account_source_active`仅接受原access SHA＋issuer／RP／grant／sid／user／account version／client policy／authTime／sourceExpiry作deny投影；SECURITY INVOKER、固定search_path、撤PUBLIC EXECUTE，无credential输出。false拒绝，true仍前后在线AccountProofPort，最终新SQL statement再核原来源、客户成员与期限、profile client／app／scope及opaque撤销。
- Phase 12必须落实auth／projection单一owner及移除平台对raw auth／credential表的权限；窄bool projection可以经严格审计保留准确EXECUTE与固定search_path／owner／无凭据输出，或替换为等效HTTP最终检查。没有等效收尾时不得预定撤销全部function grants，也不能让bool true取代在线proof。
- 新增 `GET /auth/login-bootstrap`只用于用户显式login／reconfirm，返回非授权匿名version='0'和browser CSRF，使旧invalid HttpOnly session Cookie仍在时可明确重登。普通load、401、503、signed-out不自动bootstrap；GETsession仍保持invalid401／权威503，logout不发送清除较新Cookie的指令。

## 当前门禁登记

| 范围 | 当前状态 | 记录要求 |
| --- | --- | --- |
| 工程与owner落位 | **独立实现、静态／PG及live已验证** | 同级新仓backend／frontend与独立门户DB已实际运行，平台opaque资源和门户会话分别由各自owner管理；架构审查、最终两仓门禁与真实driver／browser分别记证，不只依赖仓库初始化。 |
| 门户Rust先行check／Clippy | **已通过，保留为修复前历史** | root记录fmt修正后check17.30s、Clippy3.31s；后续发现strict revoke DTO缺口。这些先行检查不替代下一行独立最终链，不能声称旧检查已含新断言。 |
| 门户Rust pre-final链 | **修复前结果独立封存** | [portal-gates-summary-before-strict-revoke.json](../../../../local-identity-sso-phase11-qa/portal-gates-summary-before-strict-revoke.json)及before-strict-revoke日志保留11 ordinary／5PG与Clippy／build／doc结果，不含最后严格撤销确认负例，不作为最终代码receipt。 |
| 门户Rust最终strict-revoke链 | **全PASS：12普通／5PG与全部静态项** | [portal-gates-summary.json](../../../../local-identity-sso-phase11-qa/portal-gates-summary.json)登记Clippy／ordinary／PG／normal-build／doc各exit0，fmt及check由 [portal-final-check.log](../../../../local-identity-sso-phase11-qa/portal-final-check.log)记录，check1.96s。5PG测试本体0.81s，命令总耗时另记2.72s，不混为同一计时；liveAcceptance=false。 |
| 门户独立PG会话与状态机 | **5／5 PASS**，仅PG层结果 | [portal-postgres.log](../../../../local-identity-sso-phase11-qa/portal-postgres.log)实际覆盖独立库／UUID、原始快照跨tab竞态、unknown refresh不重放及invalid旧Cookie显式bootstrap；与平台7组及最终真实多进程browser3分别记证。 |
| 平台0019与资源端口新7组 | **7／7 PASS，7.67s**、exit0，七库／pool全部清理 | [platform-new-pg-result.json](../../../../local-identity-sso-phase11-qa/platform-new-pg-result.json)及 [round-02.log](../../../../local-identity-sso-phase11-qa/platform-new-pg-round-02.log)。首次0／7为fixture未收缩任务空库的二元unique，修正后完整独立7／7，不拼接局部结果；10新UUID／10CHECK来源于source计数，实际完整165 catalog由下列既有测试单独证明。 |
| 平台第二链静态与普通 | **fmt／check／Clippy／普通197通过**；该轮整体FAIL保留 | [round-02摘要](../../../../local-identity-sso-phase11-qa/platform-gates-summary-round-02.json)普通main169＋其他28=197；随后新7因fixture失败，所以不把该轮记全链通过。新7修正后的独立结果见上行。 |
| 平台最终回归串链 | **完整PASS、exit0** | [platform-gates-summary.json](../../../../local-identity-sso-phase11-qa/platform-gates-summary.json)实际fmt／check／Clippy／普通197／既有226（含旧BFF14）／rollout3／normal build／doc全部exit0；sourceFrozen=true、liveAcceptance=false。新7已有独立7／7，本链通过skip新customer_portal组避免重复，不把静态／PG通过记为live验收。 |
| 平台最终普通 | **197 PASS**：main169＋其他28 | [platform-ordinary.log](../../../../local-identity-sso-phase11-qa/platform-ordinary.log)main169实际14.32s、ignored233；其他28按lib／独立target分别执行。整个ordinary命令17.40s，各target本体与命令计时分别记。 |
| 平台既有226／rollout3 | **226／226 PASS；3／3 PASS**，各exit0 | [platform-existing-pg.log](../../../../local-identity-sso-phase11-qa/platform-existing-pg.log)226本体1109.94s／命令1112.71s；[platform-rollout.log](../../../../local-identity-sso-phase11-qa/platform-rollout.log)3本体251.72s／命令254.33s。旧BFF14包含在226中，本阶段新7另有独立结果，不合并为未执行的组数。 |
| UUID165 catalog／历史升级／fresh seed | **实际PASS**，不再只是source预期 | [existing log](../../../../local-identity-sso-phase11-qa/platform-existing-pg.log)中 `release_tests::database_upgrade::uuid_v7_target_enforces_versions_variants_and_application_generated_ids ... ok`，完整165列／exact validated单列CHECK／UUID默认值0及版本／variant／省略ID负例保持；旧升级／fresh seed记post_uuid_columns=165、defaults=0；Phase31实际old=3 current=19、原行／checksum保留并拒腐坏。 |
| 最终normal binary manifest | **四项SHA已固定，全部Cargo结束** | [binary-manifest.json](../../../../local-identity-sso-phase11-qa/binary-manifest.json)平台 `d7857845837b45281001601a69881a007bceabd1d9c0bae52e92c9da5bde2dd8`、portal `5f421019b202e5e478b93ee55104002f9fe5af1bb09246b2967e642bb33f3408`；platform-migrate `393ade226f754d2fcff0115b9df86546194578055a1d2e037b81c0286e8b8da4`、portal-migrate `8a736f6fe2be2cdecae3e2f356235d0b060acd4afeec07adb499e187ad6de527`。该manifest绑定后续live，不等于进程生命周期已通过。 |
| 门户前端修正前静态／组件 | **旧33／33 PASS**，lint0／0、typecheck／build exit0；独立历史 | [frontend-static-summary.json](../../../../local-identity-sso-phase11-qa/frontend-static-summary.json)原33组未覆盖合法accountVersion='0'，不改写旧组范围；原stdout未保存为log、耗时未记录，不补造。最终0版本修正见下一行。 |
| 门户前端0版本修正／最终build | **36／36 PASS**，lint／typecheck／build exit0；修正前36中33成功／3失败保留 | [frontend-account-version-repair-result.json](../../../../local-identity-sso-phase11-qa/frontend-account-version-repair-result.json)先实测新增三项失败，再修为36／36。accountVersion仅放行规范非负i64字符串，session／identityVersion仍正数，负数／padding／numeric／overflow与profile代际漂移仍拒绝。build index-C792xhk5.js，SHA `5fc5e9381c93365b96f704db5bb7a3f2c918950e117ea0095e8d8afebebe0e94`；没有raw stdout／耗时，不造log或时间。 |
| 显式login-bootstrap五项回归 | **已通过，包含在33项内** | invalid旧Cookie显式reconfirm、uncertain outage不自动bootstrap、迟到bootstrap不能覆盖新认证、跨tab取消bootstrap、不接收认证材料的transport；普通401／503／signed-out均不自动认证。 |
| 任务配置与默认边界 | **冻结合同及TASK_OWNED显式启用已验证** | 平台CUSTOMER_PORTAL_PORT_ENABLED默认false、固定CUSTOMER_PORTAL_OIDC_CLIENT_ID与profile pair，门户CUSTOMER_PORTAL_ENABLED默认false及独立DB／issuer／RP／resource／keyring合同见README；实际使用由最终driver／browser验证。没有另造全配置矩阵或生产启用结论。 |
| 实际PG／HTTP准入负例 | **平台7组完整通过**，最终真实多进程结果另独立PASS | 员工、CUSTOMER停用、同组织双身份、伪造成员／错组织／错RP、其他宽client、错profile client／app、错误凭据用途和权威故障见新7组receipt；最终browser3／driver3为独立真实网络证据，不以PG代替。 |
| 任务PKI与最终TLS限制 | **任务联调完成，叶证书CA:false**；Edge ignoreHTTPSErrors限制保留 | [pki-result.json](../../../../local-identity-sso-phase11-qa/pki-result.json)独立CA、三个SAN及两AEAD keyring，旧阶段PKI未动、私密文件不落repo。最终browser3仍使用Edge ignoreHTTPSErrors；Node及两BFF严格验证任务CA／准确hostname。任务验收不代表浏览器系统信任或生产DNS／TLS已配置。 |
| QA helper静态／纯内存 | **32项内存检查通过**；无运行时操作 | [helper-static-summary.json](../../../../local-identity-sso-phase11-qa/helper-static-summary.json)scope=SOURCE_ONLY_HELPER_VALIDATION，database／browser／Cargo／service操作数0，liveAcceptance=NOT_EXECUTED_BY_HELPER_AUTHOR。不能当作driver、live、截图或DB清理证据。 |
| 已完成阶段相关回归 | **既有226／rollout3实际PASS** | 本轮完整既有组包含Phase 6客户资源、旧BFF14及账号／成员／通知／actor／owner相关安全边界，rollout另3组通过；不只是沿用历史receipt，也不与本阶段新7重复计数。 |
| 真实双入口browser1 | **FAIL／CONSOLE_EMPLOYEE_LOGIN**，17条检查记录 | [browser-round-01-result.json](../../../../local-identity-sso-phase11-qa/browser-round-01-result.json)失败OWNED_PLAYWRIGHT_WAIT_TIMEOUT；console真实 `/api/sso/login`200、SSO_LOGIN=1后，helper重复fill竞态导致等待超时。pageerror=0／passwordSubmissions=1，尚未完成门户profile／refresh。17条含辅助记录，不称独立场景数。 |
| 首轮runtime／清理 | driver整体 **FAIL**，三个process均PASS且清理完成 | [live-round-01-evidence.json](../../../../local-identity-sso-phase11-qa/live-round-01-evidence.json)为FINAL_EVIDENCE／PHASE11_REAL_OIDC_REFRESH_NOT_EXERCISED。bootstrap／platform／portal均exit0、SHA不变、stdout／stderr完整、敏感模式扫描空；两DB／portal role／容器hosts清理true，成员未新增或改类型、memberless仍无成员。不是业务闭环PASS。 |
| 首轮browser launcher | **exit1**，全流credential scan完成且无命中 | [browser-round-01-launch-result.json](../../../../local-identity-sso-phase11-qa/browser-round-01-launch-result.json)rawOutputStored=false，保存UTF8解码流字节数和SHA；fixtureSecretsChecked=true、knownSecretMatches=false。launcher扫描通过不覆盖browser FAIL。 |
| 首轮helper修正 | **Node语法＋17项纯内存PASS**，仅helper source | [helper-repair-round-02-result.json](../../../../local-identity-sso-phase11-qa/helper-repair-round-02-result.json)记录browser-check.mjs SHA `9aabbcf608b0f432cf306f6230d450b9be1c3b4c24ed56d9705d9ff8ecbcf47d`。不重复fill／submit、精确consent及callback门禁和原35秒预算保留；browser／DB／Cargo／service操作0，无耗时／raw stdout。不是第二轮业务PASS，不与原32内存项累加为业务场景。 |
| 真实双入口browser2 | **FAIL／PORTAL_REUSES_GLOBAL_SSO**，26条检查记录；保留产品失败 | [browser-round-02-result.json](../../../../local-identity-sso-phase11-qa/browser-round-02-result.json)失败OWNED_PLAYWRIGHT_WAIT_TIMEOUT；已实际复用SSO、列客户候选及Cookie独立。context POST200后前端错误拒绝合法accountVersion='0'，ProtocolError、error1／profile0；backend确认初始0合法。随后新增36项先33／3失败，再修为36／36及第三轮实际通过，不改写本轮失败。 |
| 第二轮runtime／清理 | driver整体 **FAIL**，已OIDC_REFRESH=1；三个process均PASS，清理完成 | [live-round-02-evidence.json](../../../../local-identity-sso-phase11-qa/live-round-02-evidence.json)失败类别PHASE11_REQUIRED_LIVE_CONTROLS_MISSING，另有BFF_CONTEXT_RENEW=1／SELECT=1／LOGIN=1。三进程exit0／SHA不变／完整流扫描无命中，两DB／portal role／hosts清理true，成员未新增／改类型且memberless无成员。不能沿用首轮未refresh的类别，也不能因cleanup通过关闭阶段。 |
| 最终真实双入口browser3 | **PASS，196条检查记录**，pageerror=0 | [browser-round-03-result.json](../../../../local-identity-sso-phase11-qa/browser-round-03-result.json)含transport／control辅助记录，非196个独立场景。实际本人profile、真实到期续期、双向局部停用、跨tab、outage／uncertain retry仍503及显式reconfirm、员工专用／不同账号隔离、两种注销、no-store和Storage均通过。 |
| 最终runtime／迁移／清理 | driver **PASS**，7项报告COMPLETE、missing=[]、OIDC_REFRESH=4 | [live-round-03-evidence.json](../../../../local-identity-sso-phase11-qa/live-round-03-evidence.json)三进程exit0／SHA不变／完整stream扫描无命中，两migrate exit0／SHA不变；两DB／portal role／容器hosts清理true，成员未新增或改类型、memberless仍无成员。root已停止临时bridge并确认exit0；前两轮清理通过不覆盖其流程FAIL。 |
| 最终browser3 launcher | **exit0**，全流凭据扫描完成且无命中 | [browser-round-03-launch-result.json](../../../../local-identity-sso-phase11-qa/browser-round-03-launch-result.json)rawOutputStored=false、fixtureSecretsChecked=true／knownSecretMatches=false；与browser3实际PASS及runtime结果分别记证。 |
| 最终视觉 | **8／8张PNG逐张view_image PASS** | [visual-review.json](../../../../local-identity-sso-phase11-qa/visual-review.json)root核验desktop／mobile、employee-only、outage／uncertain及两种signed-out等最终画面，无员工控制项／凭据／旧profile；第二轮failure PNG明确作为历史排除，不混入最终8图。 |
| 最终source snapshot／invariant | **527项重封，最终记录完成** | [source-snapshot.json](../../../../local-identity-sso-phase11-qa/source-snapshot.json)platform493／portal34，文件SHA `074912b72fce21765f0439e7ff189c31614031a865cf771b8b140b293d931538`；[invariant-result.json](../../../../local-identity-sso-phase11-qa/invariant-result.json)SHA `a611ebb5553c5d3878a6b5c868e6b384319ffa3cb9c70497dd9ac09cb3175ea5`。源码／配置／QA helper选择集content SHA `62d7633a2658ca22badc03390fd1c4c39a3a8e9a18225e787fd3afeba73ea71d`，动态ledger／导航／本页不纳入source seal；旧LIVE_PENDING记录封存。 |

## 浏览器冻结合同与验证边界

| API | 冻结合同 | 验证边界 |
| --- | --- | --- |
| `GET /auth/config` | 启用mode=BFF，否则DISABLED。 | 实际关闭／故障时不切password或legacy模式。 |
| `GET /auth/session` | `portal-session/1`固定字段contractVersion／version／csrfToken／account／memberships／context；无session Cookie匿名200、version='0'。 | invalid旧Cookie401和权威503不降为匿名成功；真实Cookie与pending恢复语义。 |
| `GET /auth/login-bootstrap` | 显式login／reconfirm专用，匿名version='0'、null account／context、空memberships和browser CSRF。 | 真实旧invalid HttpOnly Cookie仍可显式重登；普通401／503／signed-out不得自动bootstrap。 |
| `POST /auth/login` | `returnTo:'/'`，Cookie＋CSRF＋准确Origin，返回redirectTo。 | 完整错误DTO、快速重复／登录CSRF／callback错误与重放。 |
| `GET /auth/callback` | state／issuer／code严格解析，一次消费flow并绑定login Cookie／browser generation。 | 真实provider、nonce／ID Token／UserInfo／introspection及重放／旧代际负例。 |
| `POST /auth/context` | expectedVersion＋membershipId，返回同session合同。 | 明确CUSTOMER、当前版本、错误组织／伪造成员／并发切换和迟到响应。 |
| `GET /api/profile` | Phase 6客户context＋account四字段id／username／realName／avatar。 | 原source全字段、固定profile服务client／app交集、无代读入口及前后复核。 |
| `POST /auth/logout` | expectedVersion＋PORTAL或GLOBAL，完成后redirectTo='/signed-out'。 | durable pending、结果未知、原scope／命令幂等、不发送迟到清Cookie、不同账号与两入口撤销范围。 |

## 失败、限制与后续证据

| 类别 | 原始结果、原因与修正范围 |
| --- | --- |
| 前端首次固定microtask fixture | 首轮28项中27成功／1失败，名称cross-tab notification synchronously hides data and verifies server state。fixture固定等待microtask次数，观察到controller仍loading；改为订阅目标signed-out状态，未因此修改产品源码，随后独立28／28。原stdout未保存，summary保留实际观察历史，不虚构旧log。 |
| 显式bootstrap产品补充 | 后端联调发现旧invalid HttpOnly Cookie阻止明确重登，于是仅显式login／reconfirm增加GETlogin-bootstrap；新增5项实际回归，最终33／33。不是放宽GETsession、自动登录或logout清Cookie；lint0／0、typecheck／build另实际重跑。 |
| 门户Rust首次fmt | 当前记录为fmt修正后check17.30s、Clippy3.31s；首轮格式修正及后续链分别保留。未凭格式检查推定PG或真实SSO。 |
| revoke严格成功回执缺口 | 平台成功合同是200 JSON `{revoked:true}`。审计发现门户必须拒绝缺字段／false／额外error等非严格确认，不能把HTTP200直接当完成。已补严格DTO和负例，最终12普通／5PG及静态全链另实际通过；修复前11／5链独立封存，不改写为已含新断言。 |
| 平台unused header Clippy失败 | 最初Clippy因未使用header import失败，root仅删除未使用import后重跑；随后两轮静态已有通过记录。没有通过关闭warning或放宽Clippy门禁掩盖首失败。 |
| 平台首轮ordinary环境失败 | [round-01摘要](../../../../local-identity-sso-phase11-qa/platform-gates-summary-round-01.json)与 [ordinary原log](../../../../local-identity-sso-phase11-qa/platform-ordinary-round-01.log)记录168成功／1失败，static_files遇ROFS。新runner漏挂既有test-artifacts可写volume，补回原有挂载，第二轮main169全过；不是改产品静态文件逻辑或跳过该测试。 |
| 平台首轮新7 fixture失败 | [新7 round-01.log](../../../../local-identity-sso-phase11-qa/platform-new-pg-round-01.log)记录0／7，fixture仍保留旧二元unique，七库均已清理。只在标记且确认fresh empty的任务库移除二元key、保留三元key；状态mutation显式identity_version+1。不是生产Phase 6收缩启用，也未放宽原断言。fmt及round02全7组独立7／7、7.67s，全部DB／pool清理true。 |
| UUID catalog fixture更新与实际收口 | 0019新增10列后，将预期155改165及更新注释；exact validated单列CHECK完整集合、无UUID默认值及非法版本／variant等负例断言不变。最终既有226已实际执行该catalog测试并通过；source调整与最终运行结果分别记证，没有放宽约束集合。 |
| live1 console helper重复fill竞态 | console真实 `/api/sso/login`已200，但helper重复fill竞态导致CONSOLE_EMPLOYEE_LOGIN／OWNED_PLAYWRIGHT_WAIT_TIMEOUT，17条记录，driver又因无OIDC_REFRESH整体FAIL。后已修helper并通过Node＋17内存检查；第二轮另有独立结果，不将旧失败改写为通过或归咎于SSO拒绝。 |
| live2合法accountVersion零值解析bug | 初始账号版本0是合法schema语义；前端parseContext按正整数拒绝它，context POST200仍抛ProtocolError、无profile。前端／后端独立定位后补三项回归，36中33成功／3失败实际复现，再只允许accountVersion规范非负、保留session／identityVersion正数，最终36／36及第三轮live全过；不是helper失败或后端context拒绝，也未泛化放宽版本。 |

所有失败、确认原因、实现／fixture／helper修正及独立重跑按原轮次保留，不累加局部结果为未执行的整链。前端旧33未覆盖0版本bug，新36先33／3失败再全过；stdout未保存且耗时未记录，不补造。前两轮browser／driver各自FAIL和清理回执完整保留，最终第三轮另独立196记录PASS、driver／视觉／清理齐备，527项source与invariant重封，三条Self-Check全部完成。TASK_OWNED完成不代表生产上线、Phase12独立认证部署或Phase13灰度退役。

配置、路由与DTO以 [架构记录](../phase-11-customer-portal.md)及 [门户backend README](../../../../../saas-customer-portal/backend/README.md)的冻结合同为准，TASK_OWNED client／callback实际登记、独立schema／状态机、错误边界与运行限制分别由上述普通／PG／最终live receipt记录。只登记公开元数据，不读取或展示私密环境、keyset、token fixture或原始请求凭据。Phase 11完成不包含独立身份服务部署、生产域名／资产审批、业务扩张或创建默认开关开放；任务验收范围和TLS限制继续保留。
