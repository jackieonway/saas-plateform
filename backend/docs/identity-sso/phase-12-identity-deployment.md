# Phase 12 独立身份部署、网络端口与同库权限

状态：**COMPLETED**。范围以 [台账 Phase 12](task-ledger.md)为准，[Phase 11](validation/phase-11.md)保持封账。identity42普通＋3真实PG、平台12stage及189 catalog全部通过；[第七轮最终live](../../../local-identity-sso-phase12-qa/live-evidence-round-07.json)collection=COMPLETE、missing=[]、driver=PASS／FINAL_EVIDENCE，协议221项通过。共享平台库加密备份／还原2.288372秒，五组catalog／稳定数据／DB ACL全部一致，受限identity还原健康通过；browser214条检查记录与8图root视觉QA通过，全部任务进程／库／角色／hosts已清理。三个Self-Check均PASS，前六轮失败原样保留。恢复验收限共享平台库与受限identity健康，不代表生产RPO／PITR或Phase13双库及完整业务恢复。

## 部署单元与稳定地址

[`saas-identity`](../../../../saas-identity/)是同级独立Rust仓与部署单元，main只进入库，lib装配配置、只读readiness、路由、启动和优雅退出；不链接平台或门户业务crate。独立Linux门禁、实际normal进程及最终双RP浏览器闭环均有独立证据。[身份服务README](../../../../saas-identity/README.md)及 [内部API](../../../../saas-identity/docs/internal-api.md)分别定义公开与私有接口。

公开issuer固定为 `https://auth.saas.test`。网关将认证host的 `/api/sso/*`、`/sso/*`、`/.well-known/openid-configuration`及公开 `/oidc/*`转至身份进程，当前QA网关还准确转发 `/health/ready`用于平台remote readiness；平台业务API、console BFF和客户门户callback仍在原host。当前内部listener默认 `0.0.0.0:3041`只是上游监听地址，不替换issuer、subject、Cookie host、RP client或已登记callback。私有 `/internal/identity/*`仅向已登记服务消费者提供经准确证书／hostname验证的HTTPS，不能因补ready路由而向业务host开放。

门户从Phase 11起已通过HTTPS访问provider与平台资源。本阶段需要替换的本地客户account proof adapter位于**平台customer_portal资源owner**，不能写成门户BFF此前直接查认证数据库。平台现有BFF仍按自己的PLATFORM_CONSOLE RP取得员工context，门户仍只选CUSTOMER，固定SDK／客户profile／通知合同不因进程拆分扩权。

本阶段允许共用一个PostgreSQL数据库，以明确owner／runtime ACL交接代替跨仓业务实现依赖；不强制拆数据库、不迁走全部legacy OAuth、不正式扩大灰度或提前停用活动协议。生产域名、角色、deployment UUID、容量和恢复目标须按真实环境落实，任务候选与默认关闭开关不构成生产承诺。

## 单一迁移与六角色交接

[`0020_identity_deployment.sql`](../../migrations/0020_identity_deployment.sql)追加ready state、单用途service登记、身份审计、账号tombstone／receipt／revocation fact及平台account／OIDC intent，源码新增24个UUID列；[静态不变量报告](../../../local-identity-sso-phase12-qa/static-invariants.json)已核0001～0019准确字节不变。[最终既有PG日志](../../../local-identity-sso-phase12-qa/platform-existing-pg.log)实际旧员工升级与fresh full install均确认189 UUID列、零UUID默认值及85个FK，fresh seed／restart幂等通过；不再仅以165＋24源码计数推定catalog。0020不自行创建完整角色闭包或完成owner交接；后者由 [identity-deployment运维作业](../../ops/identity-deployment/README.md)完成。

唯一canonical平台migration job执行既有0001～0020。身份runtime从不迁移、建库或seed；平台 `IDENTITY_REMOTE_ENABLED=true`分支先验证部署与成员readiness，跳过迁移、seed及地区初始化，不创建第二条SQLx历史。门户独立会话库保持自己的既有migration owner，不与这条共享历史竞争。

受控交接顺序是：停两个runtime，备份并记录恢复点；唯一job完成migration；明确标记目标数据库；创建此前不存在的六角色；provision交接owner／ACL并写ready；只读verify后才启动零DDL进程。`role_prefix`限定6～32位规范小写ASCII标识符，碰到重名失败；provision核数据库名、数据库comment `identity-deployment/1/<deployment_id>`、准确issuer和SQLx 0020成功记录，ready已存在时拒绝重复provision。

| 角色后缀 | owner或runtime能力 | 边界 |
| --- | --- | --- |
| `_migration_owner` | NOLOGIN／NOINHERIT；DB、public schema、SQLx history、ready state及共同UUID约束owner。 | 仅受控迁移job使用；runtime不继承、不SET ROLE。 |
| `_identity_owner` | NOLOGIN／NOINHERIT；真实users、SSO／OIDC、service登记、身份审计、receipt、tombstone及认证projection。 | ready中的projection_owner合并使用该身份，不新增第七角色。 |
| `_platform_owner` | NOLOGIN／NOINHERIT；成员、组织、角色菜单、平台策略、legacy OAuth、平台session、BFF、客户context和durable intents。 | 不拥有或直接写认证私密凭据。 |
| `_bridge_owner` | NOLOGIN／NOINHERIT；仅拥有两个受约束账号失效trigger。 | 平台凭据仅必要SELECT及失效列UPDATE；认证fact只允许受控INSERT／两计数列累加，不是通用跨owner写入身份。 |
| `_identity_runtime` | LOGIN／NOINHERIT；明确认证列和身份私有状态必要DML。 | 不读写is_super_admin，不接触成员／角色／菜单，不DELETE用户、不UPDATE主体ID、不TRUNCATE或DDL。 |
| `_platform_runtime` | LOGIN／NOINHERIT；平台表必要DML、users非PHC列SELECT与仅UPDATE(is_super_admin)。 | 最后一项保留原账号行锁资格，不授予认证状态／版本／密码列UPDATE或认证私有表访问。 |

六角色无superuser／CREATEDB／CREATEROLE／REPLICATION／BYPASSRLS能力，彼此无membership，runtime还必须没有任何其他role membership。身份服务最新42普通链覆盖这一readiness源码收窄；平台最终整链角色4PG全部通过，本体5.22秒、命令7.96秒，四组断言与库／角色／pool清理均true。第二链的2项权限失败及第五链通过分别保留；角色PG不能替代真实进程验收。owner的物理表所有权不等于runtime获得整表权限。

真实users表、原ID、FK与稳定sub保留。identity的INSERT排除is_super_admin、默认false；账号RETIRE是不可逆停用＋代际提升＋tombstone，保留subject／loginname及物理行，不能借DELETE级联平台成员。平台在receipt后只清理接受时捕获的准确member ID／version和role tuple／createdAt，代际清理只到receipt.beforeVersion，不能删除新登录或新关系。

provision撤目标DB的PUBLIC CONNECT／CREATE／TEMP、public schema的PUBLIC及目标表／序列／函数PUBLIC权限，并关闭未来对象的PUBLIC默认权限。runtime只得必要CONNECT／USAGE／列DML／指定函数EXECUTE，无owner／TRIGGER／REFERENCES／DDL。额外业务schema未明确处理时拒绝；扩展内部对象保留其owner，不无差别改写。该目标库ACL不等于整个实例其他数据库ACL，部署角色不得随意复用，仍须专用实例或pg_hba边界。首轮真实角色失败暴露provision对实际owner也撤销普通DML权限；修正只恢复每表实际NOLOGIN owner的普通权限，并在provision／verify检查，不新增runtime原始表grant或扩大bridge。最终角色4PG、create／provision／verify及独立normal runtime部署均通过，前轮权限失败保留。

## 原始来源投影、锁与撤销bridge

跨owner函数使用固定 `pg_catalog,public` search_path、全限定表名、准确owner与指定签名EXECUTE，不授予raw auth表读权限：

| 函数组 | 调用方向与作用 | 不可推定的能力 |
| --- | --- | --- |
| bff_oidc_proof_active、customer_portal_account_source_active、oidc_console_source_active | identity owner持有，platform runtime仅获窄EXECUTE；对原始sid／grant／issuer／client／版本等返回deny-only bool。 | true不代替逐请求在线HTTP proof，不能返回PHC、token或平台授权。 |
| identity_lock_oidc_source | identity owner持有，platform runtime按canonical client及pinned issuer锁准确RP登记。 | 只返回bool并持锁至本地事务结束，不返回client secret。 |
| platform_account_command_intent_active、platform_password_context_active | platform owner持有，identity runtime仅获窄EXECUTE；分别锁原intent或原成员／session并核原前提。 | identity不读取平台角色或获取一般成员／session表权限；不领养新账号／新成员证明。 |
| identity_oidc_command_intent_active | 名称虽含identity，owner仍为platform；identity仅按原op／service／digest／method／完整path和60秒窗口检查接受事实。 | 不是由identity自行决定平台root管理员权限，也不是过期新命令仍可执行。 |

两个bridge是 `revoke_sessions_on_user_disable`与 `revoke_credentials_on_account_generation`；runtime没有直接EXECUTE。触发目标来自users OLD／NEW，bridge仅同步撤销平台session／refresh／oauth token／code／device，不增授权、成员或角色。实际ROW_COUNT累加到auth owner的 `identity_account_revocation_facts`精确user＋新代际，后执行的零行不覆盖已有计数。identity runtime只有SELECT，可在本次mutation receipt中返回真实revokedSessions／revokedTokens；后者是被修改的oauth_tokens行数，不把全部code／device／refresh合并冒称token数。无代际变化的CREATE／资料更新返回零，不能消费历史fact。

## 三个独立内部服务用途

service登记在identity_service_clients，只存secret SHA256且UNIQUE，单独purpose为ACCOUNT_COMMAND、CUSTOMER_ACCESS_PROOF或OIDC_ADMIN。每个用途使用独立canonical UUIDv7 ID及64位小写hex secret，不复用RP secret、resource client或其他purpose。内部只接准确Basic，不收平台Bearer／Cookie或query认证，JSON拒绝未知字段，响应no-store／no-referrer。错误认证／purpose属于基础设施不可用，消费者不能回退本地认证。

| 消费者 | 当前平台配置 | 固定能力 |
| --- | --- | --- |
| 账号port | IDENTITY_SERVICE_CLIENT_ID／IDENTITY_SERVICE_CLIENT_SECRET | candidates、账号安全snapshot、密码verify／login记录和durable account commands／receipt。 |
| 客户原始access proof | CUSTOMER_AUTHORITY_SERVICE_CLIENT_ID／SECRET | POST `/internal/identity/customer-access-proof`，固定CUSTOMER_PORTAL_OIDC_CLIENT_ID。 |
| OIDC管理port | IDENTITY_ADMIN_SERVICE_CLIENT_ID／SECRET | 独立OIDC_ADMIN私有管理及原操作receipt查询，不向identity传平台Bearer。 |

平台remote模式由 `IDENTITY_REMOTE_ENABLED`显式开启，默认false；要求本进程SSO／OIDC关闭、自动建库关闭且不处于imported模式，IDENTITY_SERVICE_URL是准确HTTPS origin并与OIDC_ISSUER一致，CA_FILE为明确可读的绝对路径。三个service登记分别配置；本地兼容不是网络故障fallback。

平台 [`customer_portal/remote_authority.rs`](../../src/customer_portal/remote_authority.rs)没有DB／cache，固定请求合同customer-access-proof-request/1和返回customer-access-proof/1；只收原OIDC access，严格tokenType=access_token、purpose=CUSTOMER_PORTAL、仅CUSTOMER及原grant含openid＋profile、完整原source和四字段account，不选择成员。错误／退役来源401，服务403、坏协议、依赖／容量／deadline失败为503。当前3秒共享预算／1秒连接／16无等待slot／64KiB响应、无proxy／redirect／retry；前后两次online proof和0019最终deny检查仍保留。

## 账号命令的接受、未知结果与安全收尾

[`authentication/remote.rs`](../../src/authentication/remote.rs)传安全账号snapshot，不传PHC、PreparedPassword或SQLx事务。candidates保留现有别名规则与65上限；verify完成Argon2后重验原hash／enabled／account generation，login只更新准确原代际，不签发session。

[`authentication/commands.rs`](../../src/authentication/commands.rs)由平台先完成fresh actor、目标范围及原角色约束，再提交durable intent作为管理员接受点，保存精确actor／target／expectedVersion和非秘密followup。canonicalDigest为HMAC-SHA256，以service secret的UTF8字节为key，对完整原JSON去掉顶层canonicalDigest、递归排序键后compact编码；包含原expiresAt、显式null及数组顺序，不独立存无盐密码指纹，不持久化明文、加密密码或secret payload。

平台发送前先持久UNKNOWN，旧命令只发送一次；timeout／404后只能GET原operation＋digest receipt，不重放secret。尚未发送的PENDING_SECRET到期可ABANDONED，新操作须显式发起；UNKNOWN不能假装完成。identity按operation advisory锁串行化，先取同service／digest receipt再核新命令期限；新命令最多60秒、平台当前50秒intent。目标账号锁、准确intent窄fence、密码／状态／代际变化、身份审计及receipt原子提交，等待后与receipt写入后再验expiry。

个人改密另外绑定原user／sid／tenant／org／accountVersion，identity先锁users，再通过平台owner函数按member→原session顺序锁定并复验原员工来源，持锁至账号mutation提交，不能用新代际补旧context。该窄数据库fence是同库受审计交接，不意味着跨HTTP仍共享一个SQLx事务。平台收到安全receipt后在新事务完成原精确业务followup／audit／complete；崩溃恢复只查原receipt，不返或重发初始密码。

## OIDC管理的durable receipt与终态查询

[`oidc/remote_admin.rs`](../../src/oidc/remote_admin.rs)保留7个既有管理path成功DTO。平台先fresh员工、platform-root与system:oauth，再同事务写intent＋OIDC_REMOTE_ACCEPT并提交为接受点，不持平台锁跨HTTP；全请求3秒覆盖CurrentUser／DB／HTTP／response／ACK，16个无等待slot。

mutation发准确operation ID与digest双header。此digest是**SHA256**，对象为method＋完整固定internal path＋未经trim／策略规范化的原始body的递归排序compact JSON，bodyless用null；不同于账号命令HMAC。identity使用同operation advisory锁、准确service／secret hash／digest／method／path receipt及原60秒intent fence，mutation／audit／安全receipt同事务提交。新client ID等于operation ID，secret由原service secret＋operation／digest HMAC确定生成；receipt不存secret，但同service且secret未轮换的认证recovery可重建原创建secret，轮换后409须明确reprovision。

未知结果平台返回503、code=OIDC_REMOTE_RESULT_UNKNOWN与operationId，不自动retry mutation。新增GET `/api/oidc/operations/{id}`重新核fresh admin、原actor／tenant／org／service匹配，仅GET identity原operation。成功receipt返回COMPLETED／原DTO，先提交平台ACK再交付。identity lookup与执行使用相同operation advisory transaction锁；平台在原intent的同一DB查询中按clock判是否已过60秒，**只有已过期＋受该锁保护的lookup404**才返回NOT_APPLIED／result=null。未过期404、403、503及未知结果仍UNKNOWN，不重POST旧操作；不能仅因404就认定未执行。

起稿时发现的两处引用文档旧措辞已由root同步：部署README明确receipt不存secret但原未轮换service可受认证HMAC恢复；内部API补齐平台已过期＋同锁lookup404才NOT_APPLIED的终态条件。本页与当前源码／引用合同一致，旧发现作为文档修正历史保留。

## Readiness、部署与运维验证范围

身份服务只读ready marker及catalog，核schema20、固定issuer、准确runtime current_user=session_user、非owner／非高权角色、零membership、无DDL与列／函数边界；平台remote启动也验证对应marker与权限。marker不是生产readiness证书，不替代DB／authority失败503。

系统schema过滤按字面pg_前缀匹配，不能用会误排除其他用户schema的宽模式跳过owner／CREATE权限检查。role辅助membership反例与intent锁后新的SQL statement时钟重验已补源码；后者不假设锁等待前成立的deadline在锁后仍有效。最终平台整链角色4PG与账号3PG已通过并保留先前权限／fixture失败；OIDC平台admin2PG与真实TLS1也已通过，不能据这些测试提前认定双RP live闭环完成。

当前路由 `/health`、`/healthz`、`/ready`、`/readyz`及 `/health/ready`共用数据库／owner核验，成功精确JSON为status=UP、service=saas-identity、issuer固定值，失败503。第二轮QA ready路由缺项、第三至第六轮backup失败均保留；第七轮最终协议221通过，实际process／DB故障503、恢复／monitor与key／JWKS生命周期均取证。三normal进程、bootstrap及还原identity均exit0／pins不变，最终恢复与双RP浏览器独立通过。身份服务只读显式process env，无.env发现，SSO／OIDC默认false，外部RS256 keyset只读挂载；日志禁止密码、Authorization、Cookie、token、请求body、私钥及连接串。

本阶段已完成固定issuer下独立拓扑、两个RP登录／真实续期／局部及全局注销、仍支持legacy合同回归、真实故障／恢复、key生命周期及监控观察。共享平台库备份经加密认证／tamper拒绝、实际dump／restore、五family及稳定数据／DB ACL一致、受限identity健康和清理验证；还原后的业务双入口与门户库恢复不在本轮restore范围。前六轮失败与mini的7项机制证据保留，第七轮完整237项relation比较现已通过。生产SLO／RPO／RTO、PITR及Phase13完整身份感知恢复另按台账实施。

工具镜像原基底缺psql／pg_dump，root从同一精确基底构建 [tools-image-result.json](../../../local-identity-sso-phase12-qa/tools-image-result.json)登记的saas-ci:phase12-tools，只加入PostgreSQL17.6 clients与libpq17，compiler和原runtime依赖未变。最终live部署、四个backup作业及受限identity还原健康全部通过。Phase12独立CA／CA:false leaf公共pins见 [pki-result.json](../../../local-identity-sso-phase12-qa/pki-result.json)，SAN固定三个任务host，旧阶段PKI未动。定向TLS1、Node与两个BFF均验证task CA及准确hostname；Edge浏览器为task-CA leaf使用ignoreHTTPSErrors，因此不宣称生产DNS／OS信任或生产TLS发布。

已归档的 [identity审计行数修正前链](../../../local-identity-sso-phase12-qa/identity-gates-summary-before-audit-rowcount.json)保留42普通及静态／build／doc通过范围。后续源码审查发现BEFORE trigger可静默抑制audit INSERT并返回零行，原helper未要求恰好一行；现已补audit／receipt与必要目标写入精确行数守卫。[最新identity整链](../../../local-identity-sso-phase12-qa/identity-gates-summary.json)fmt／check／Clippy／42普通／3项真实PG／build／doc全exit0，普通本体3.57秒、命令16.35秒；[durability PG](../../../local-identity-sso-phase12-qa/identity-durability-pg.log)本体2.22秒、命令4.18秒，验证账号必要写入／receipt抑制、audit静默抑制和真实OIDC admin路径的receipt／audit／单目标写入，三项均通过且三taskDB清理true。新增回归首轮Clippy cmp_owned失败另归档，修正后通过，不删除旧失败或将PG通过扩大成live验收。

平台 [第一整链](../../../local-identity-sso-phase12-qa/platform-gates-summary-round-01.json)因Clippy type_complexity失败；[第二整链](../../../local-identity-sso-phase12-qa/platform-gates-summary-round-02.json)静态／221普通通过后角色2／4权限失败，cleanup全部true，失败assertions_ok=false保留。第三链因测试helper冗余Ok／?触发Clippy失败、第四链因该helper格式失败，各轮均独立归档。[第五链](../../../local-identity-sso-phase12-qa/platform-gates-summary-round-05.json)静态／221普通与角色4PG通过，账号3PG为1通过／2项uq_org_root_per_tenant fixture失败，三taskDB与pool清理全部true；修正只为三处子组织补原root父关系，生产唯一约束保留。[最终平台整链](../../../local-identity-sso-phase12-qa/platform-gates-summary.json)现已exit0，12stage全PASS：221普通、角色4／账号3／admin2／TLS1、既有233及rollout3，normal build68.93秒、docs29.76秒。既有233本体1131.89秒／命令1134.74秒，rollout3本体256.62秒／命令259.26秒，普通、PG与独立部署验收分别记证。

[静态不变量](../../../local-identity-sso-phase12-qa/static-invariants.json)核19历史migration、SDK1／2／3归档37文件及消费者44文件准确字节全部通过，合同仍为microProps2.0／notification1.0／context1.0；此检查未访问DB或调用Cargo。[helper静态结果](../../../local-identity-sso-phase12-qa/helper-static-summary.json)与 [当前纯内存合同](../../../local-identity-sso-phase12-qa/helper-contract-result.json)均登记40项，control phase后的旧36项与ready路由追加4项范围分别保留。OIDC原60秒观察已改为按未改写的原accepted_at＋DB clock只读轮询，最多70秒预算；[14项fake-clock／DB合同](../../../local-identity-sso-phase12-qa/oidc-original-deadline-result.json)通过，不冒称这组fake检查已真实等待或实库执行。

[binary-manifest.json](../../../local-identity-sso-phase12-qa/binary-manifest.json)已pin平台／migrate／rollout、identity及门户／migrate共六个normal executables，门户与Phase11准确字节一致。live期间三进程产品源码与normal binary冻结，不以重编译替换已记录产物。[live首轮](../../../local-identity-sso-phase12-qa/live-evidence-round-01.json)两migration均exit0，却因SELECT version::text后按文本ORDER BY使末尾9误判history失败；driver三处已改按原数值version排序，库清理true。[live第二轮](../../../local-identity-sso-phase12-qa/live-evidence-round-02.json)确认canonical20条、create／provision／verify及受限identity ready，平台因QA网关缺ready路由fail closed；两DB、六deployment roles＋一portal role及container hosts均清理／恢复。

[live第三轮](../../../local-identity-sso-phase12-qa/live-evidence-round-03.json)与 [第四轮](../../../local-identity-sso-phase12-qa/live-evidence-round-04.json)各自221项协议检查全部通过；两次均观察process authority4／DB authority3项故障响应、原400／409丢失及UNKNOWN恢复，属于同一套合同重跑，不能相加为442个独立场景。两次driver均在ENCRYPTED_BACKUP_AND_REAL_RESTORE失败，三normal进程／bootstrap exit0且pins不变；源两DB、七roles与container hosts均清理／恢复，restore DB与raw dump另确认已移除。第三轮pg_restore exit1的 [安全分类](../../../local-identity-sso-phase12-qa/backup-failure-classification-round-03.json)确认为helper提前删除默认public，而PG17 archive先ALTER其owner；修正保留并验证template0的空public，仍准确核目标DB／cluster／marker与最终owner／ACL。

第四轮dump／加密／解密／pg_restore均exit0，现场安全对账确认稳定非秘密数据与DB owner／ACL一致，最终对象catalog比较仍报OBJECT_OWNERS_OR_ACLS_MISMATCH。列attacl的NULL与空数组应同为“无额外列grant”，第一次修正的19项纯回归通过仍未检出空数组进入aclexplode的问题；[第五轮安全证据](../../../local-identity-sso-phase12-qa/live-evidence-round-05.json)记录protocol221通过，backup在PREFLIGHT约0.05秒失败、尚无dump／restore作业。三normal及bootstrap exit0、pins不变，2源DB／7roles／container hosts均清理／恢复，原失败与 [独立ID](../../../local-identity-sso-phase12-qa/live-task-id-round-05.txt)保留。

[backup helper](../../../local-identity-sso-phase12-qa/backup-recovery.py)的CASE守卫先判断NULL与cardinality=0，避免空数组进入aclexplode；columns NULL／{}都为EMPTY，其他ACL的默认与空撤权仍有明确区别，relation的owner默认展开另见下段。[20纯历史receipt](../../../local-identity-sso-phase12-qa/backup-pure-regression-receipt-01.json)及 [18只读历史receipt](../../../local-identity-sso-phase12-qa/backup-acl-readonly-receipt-01.json)仅归档此前实际输出，没有为写receipt追加运行，未保留的catalog计数／SHA不补造；后续 [23纯及3AST receipt](../../../local-identity-sso-phase12-qa/backup-pure-regression-receipt-02.json)与 [18只读重跑receipt](../../../local-identity-sso-phase12-qa/backup-acl-readonly-receipt-02.json)分别保存当前真实范围。只读PG为常量ACL／postgres catalog查询与回滚，不接触产品库，不等于完整恢复。

[第六轮安全证据](../../../local-identity-sso-phase12-qa/live-evidence-round-06.json)task01a170e3-4c0c-7ab0-afe8-a44d6cd1c724为FAIL／FAILURE_PARTIAL，protocol221通过，dump／加密／解密／pg_restore均exit0。comparison明确stable_data_equal及database_acl_equal=true；schemas1／columns952／functions83／default_privileges0计数与SHA均一致，只有relations源／还原各237条而SHA不同，object_owners_and_acls_equal=false。stage虽为RESTORED_CATALOG_DEFAULT_PRIVILEGES，实际差异family仅relations。三normal与bootstrap exit0／pins不变，2源DB／7roles／container hosts、restore DB及raw dumps全部清理；该失败证据保留原状，随后实测诊断不把它改成PASS。

[mini roundtrip 01](../../../local-identity-sso-phase12-qa/backup-relation-roundtrip-receipt-01.json)及 [02](../../../local-identity-sso-phase12-qa/backup-relation-roundtrip-receipt-02.json)使用各自带标记源／还原库，实际pg_dump／pg_restore均exit0，每轮21检查通过并清理库／dump。两次均确认mini表和sequence冗余显式owner默认ACL还原为NULL，7项mini物理指纹不同而原owner与有效grant一致；真实空ACL仍撤权且不同于默认，sequence准确有USAGE／SELECT／UPDATE。这证明表示机制与mini权限边界，不证明原完整237项均属该变化。修正仅在具ACL能力relkind r／p／v／m／f／S时展开原c.relowner内置acldefault，表类用'r'、sequence用小写's'；不取当前pg_default_acl，不把大写'S'误作sequence ACL类型。其余relkind和其他family保持既定规则，EMPTY仍撤权，grant四维与owner继续比较。

当前helper SHA256为7f7005a8976e8e24ee222102565cc9a94f5c5781810f6841a94d5796a3e973f0；[最终source snapshot](../../../local-identity-sso-phase12-qa/source-snapshot.json)594项，contentSetSha256=6b7fdcdb1429bccbca8bc76a7036af8fafae51ff92f89a31e52c4d6d1b47cc70，文件SHA256=21384460dc185529b151380acdd98be10572a2207f3cf2f8aec3cb6982155290。[normal manifest](../../../local-identity-sso-phase12-qa/binary-manifest.json)六pins与此前相同、compiled Rust未变、门户匹配Phase11。[最终静态不变量](../../../local-identity-sso-phase12-qa/static-invariants.json)文件SHA256=c5b88d7b93667ca2ed6cd35cbada0d065710e8b670c5bfc09597b59bd5b1dea6，19历史migration／37SDK归档／44消费者准确字节均通过；source与静态证据仍与实际运行分开记。

第七轮task01a170ec-293e-7a97-8c14-ed22abee92be最终 [live证据](../../../local-identity-sso-phase12-qa/live-evidence-round-07.json)完整收集、无缺report，driver=PASS／FINAL_EVIDENCE。backup2.288372秒、四job exit0，schemas1／relations237／columns952／functions83／default_privileges0的源／还原计数与SHA全部一致，stable data／DB ACL／object owners与ACL均equal，relation difference_count=0；restored restricted identity health PASS、exit0、binary unchanged，restore DB及raw dump移除。canonical20条history在normal启动后不变，未新建／重定型membership，memberless仍无成员，实际OIDC refresh审计已核。

[browser-result.json](../../../local-identity-sso-phase12-qa/browser-result.json)PASS、214条检查记录、pageErrorCount=0，覆盖固定issuer、员工／客户入口、真实短token续期、双向局部撤销、故障后显式重认与全局注销；214包含重复合同与辅助记录，不能当214独立场景。[visual-review.json](../../../local-identity-sso-phase12-qa/visual-review.json)登记8PNG由root逐图QA通过。[browser-launch结果](../../../local-identity-sso-phase12-qa/browser-launch-result.json)exit0、完整stdout／stderr扫描已检查fixture known secrets、categories为空、rawOutputStored=false；三normal与bootstrap扫描也均无敏感类别。两源DB、七roles及container hosts全清理／恢复，[bridge最终结果](../../../local-identity-sso-phase12-qa/bridge-final-result.json)确认browser与driver退出后才停止loopback bridge、exit0。三条阶段验收至此PASS，历史FAIL不改写。

[Phase13只读准备归档](../../../local-identity-sso-phase12-qa/phase13-readonly-preparation.md)及其他准备文档仅保存后续容量／灰度／恢复与SQL／消费者审查，封账时尚未实施Phase13。Phase12恢复仅共享平台库和受限identity health；Phase13双库／库外材料、完整业务恢复、生产RPO／PITR、灰度与观察窗口、实际退役和身份感知回退不得由本阶段结果提前认定完成。
