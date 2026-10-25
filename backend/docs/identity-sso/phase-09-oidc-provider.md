# Phase 9 OIDC provider、密钥与协议边界

状态：**COMPLETED**，依赖 [Phase 8](validation/phase-08.md) 已完成。最终静态门禁、194项普通测试、9项新增PG、203项既有ignored、3项启动／receipt及37项配置检查通过；openid-client 6.8.8与真实Edge两轮各117项、最终6图视觉验收也通过。静态、新PG及既有回归命令均exit 0，完整证据及首次失败见 [Phase 9 验收](validation/phase-09.md)。

## 稳定issuer与认证用途

[台账Phase 9](task-ledger.md)要求当前环境从同进程阶段固定公开HTTPS issuer，discovery的 `issuer`、ID Token的 `iss` 及RP期望值精确一致，不由请求Host或转发头推断。未来拆进程通过网关改上游，保留同一环境issuer。本阶段两轮TASK_OWNED测试均固定issuer为 `https://auth.saas.test`，经443临时gateway，RP为 `https://console.saas.test`，精确callback为 `https://console.saas.test/auth/callback`；实际信任范围见验收，不把隔离测试资产写成生产域名或证书已落实。

[`oidc::initialize`](../../src/oidc/mod.rs)在bootstrap及每次协议authority检查时读取 `oidc_provider_settings` 的单例issuer；首次显式初始化后，同一数据库中的配置issuer必须继续相同，漂移返回503，不把旧client／grant静默换到新issuer。当前issuer限定为一个根HTTPS origin，精确等于SSO origin。

| 配置 | 默认与约束 |
| --- | --- |
| `OIDC_ENABLED` | 默认false；开启必须先启用SSO |
| `OIDC_ISSUER` | 开启时必需，单一精确HTTPS origin，等于 `SSO_ORIGIN`；延续SSO独立于平台host的限制 |
| `OIDC_KEYSET_PATH` | 开启时必需，外部keyset的绝对路径；私钥不写数据库或仓库 |
| `OIDC_CODE_TTL_SECONDS` | 默认120，允许1～300秒 |
| `OIDC_TOKEN_TTL_SECONDS` | 默认300，允许1～600秒；实际access／ID期限还取原SSO及grant的最早绝对到期 |
| `OIDC_REFRESH_TTL_SECONDS` | 默认43200，允许1～86400秒；grant／refresh不超过原SSO期限，刷新不延长原grant绝对期限 |
| `OIDC_INTERACTION_TTL_SECONDS` | 默认300，允许30～600秒 |

ID Token只表示给特定RP的账号认证结果。`sub`为本新库从创建起稳定的RFC UUIDv7 `users.id`，没有phone／email主体合并或旧主体映射。通用claim不包含成员、组织权限、角色或 `is_super_admin`；有效无成员账号可以完成OIDC认证，但不会获得平台context或业务权限。

离线ID Token在签名、issuer、audience及时间等验证通过时，可能仍在绝对有效窗口内；provider撤销账号或grant不能使已缓存公钥的普通RP自动得知该事实。code、refresh、access／userinfo等在线消费必须检查原全局sid、账号及客户端版本。后续平台BFF建立业务context时还需验证当前上游证明，不把ID Token的离线验签结果当作实时业务授权。

## 协议接受点

接口位于 [`oidc/protocol.rs`](../../src/oidc/protocol.rs)，不复用legacy `/oauth/*` 存储或issuer。客户端为confidential USER，认证方法为 `client_secret_basic`；只开放code＋S256及显式允许的refresh。支持scope为 `openid,profile,offline_access`，offline_access每次均要求实际同意交互，不能由prompt=none绕过。

Basic可带同值canonical body client_id作为冗余主体标识；不匹配ID及body secret／assertion等第二套认证材料被拒绝。OidcError映射中的invalid_client为401及Basic challenge，invalid_token为401及Bearer challenge，参数／proof错误采用OAuth JSON；权威故障503 temporarily_unavailable，限流429 temporarily_unavailable及Retry-After，内部失败500 server_error且沿用脱敏trace。普通管理API继续使用业务错误合同，框架提取层的畸形请求响应不能以该映射的源码存在冒称已完整实测。

| 方法与路径 | 当前作用 |
| --- | --- |
| `GET /.well-known/openid-configuration` | 匿名标准发现；固定issuer及实际endpoint／RS256／S256／Basic／scope能力，回调issuer参数支持为true |
| `GET /oidc/jwks` | 匿名公钥集，已发布且未退役／撤销的kid；no-store |
| `GET, POST /oidc/authorize` | query或form授权请求，完成验证后创建flow或在已同意范围直接发code；callback含原state及iss |
| `GET /oidc/interaction/{id}` | 同源浏览器读取安全DTO及该flow独立CSRF；首次绑定原账号sid／版本 |
| `POST /oidc/interaction/{id}` | JSON `action=approve/deny/restart`；浏览器绑定、精确Origin及flow CSRF；回调仅服务端构造 |
| `POST /oidc/token` | form＋Basic，code兑换或refresh轮换；不以Cookie认证或要求浏览器Origin |
| `GET, POST /oidc/userinfo` | 仅opaque OIDC access Bearer，实时权威检查；openid-only只返回sub，profile另含preferred_username、name及非空picture |
| `POST /oidc/introspect` | form＋Basic；仅同client可得active信息，未知／错误client的token为active:false |
| `POST /oidc/revoke` | form＋Basic；自client任一access／refresh撤整个grant，未知或他client token为200且无副作用 |
| `GET /sso/login`、`GET /sso/assets/identity.css, identity.js` | 同源账号登录／同意页面和本地静态资源；只接受flow，不接受任意return URL |

OIDC／UI路由统一no-store、no-cache、no-referrer、nosniff、X-Frame-Options DENY及CSP：脚本／样式／连接仅self，base-uri none、form-action self、frame-ancestors none；Vary为Origin、Cookie。页面无inline JSON、Storage或远端头像，账号和client文本经安全DOM API显示。

| 边界 | 本阶段必须满足的合同 | 当前状态 |
| --- | --- | --- |
| Discovery与JWKS | 固定issuer下的标准发现位置；只广告实际支持的endpoint、response type、scope、认证方法和算法；JWKS仅公钥，`kid`唯一 | PG及真实标准RP已验证 |
| Authorization Code | 精确登记client及redirect URI，`response_type=code`，包含openid，强制S256和合法verifier；code短期、hash-only、单次消费及client／callback绑定 | PG负例与真实RP正控已通过 |
| state与nonce | RP保存并一次验证state／nonce／verifier事务；provider绑定原nonce并原样返回合法state，不以成员选择替代全局认证 | PG及真实RP callback负例通过 |
| ID Token | 固定RS256及可信JWKS中的kid；RP验证iss、sub、aud、exp、iat、nonce与适用auth_time | PG、标准RP正控及strict变体验证通过，具体范围见验收 |
| Refresh | 显式登记及scope、摘要存储、绝对期限、client／grant绑定、轮换与reuse撤族、失败无部分签发 | PG轮换／reuse／并发已通过；真实RP两轮仅code流程 |
| 认证状态故障 | 权威账号／SSO／client状态无法确认返回可识别503，不回退legacy密码、旧issuer或缓存放行 | PG真实pool故障已通过；页面503另为模拟 |

不得直接复用现有 `/oauth/.well-known/openid-configuration` 为新标准发现文档：现有legacy实现的issuer及endpoint、grant和PKCE广告属于原兼容协议范围，没有因此补齐ID Token、nonce、JWKS或标准RP验证。

authorize须接受GET query与POST form，UserInfo支持GET与POST Bearer；token是标准POST form与登记的client认证。RP发起的协议请求不能套用P8同源页面的Origin／Fetch Metadata门禁；真正的登录和同意页面仍使用Cookie、精确Origin及独立CSRF。参数解析须在转为键值映射之前拒绝重复字段；按规范忽略未知参数，无效client／redirect时不向提交地址跳转。PKCE verifier采用43～128个ASCII unreserved字符，S256为SHA-256后无padding base64url，拒绝plain及降级。

client与精确redirect已经可信后，授权协议错误及拒绝通过安全构造的callback返回error、原state与issuer；不能把所有有效RP的授权错误都改成无法关联流程的本地JSON。client／redirect本身无效或重复到无法唯一确定时，则在本地拒绝，不使用提交地址。

本项目要求RP把state、nonce、verifier同时绑定一次发起流程及精确callback，限时且一次消费。这是加强profile：state关联浏览器回调并由OP原样回传；nonce绑定初始ID Token；verifier证明兑换者持有原challenge对应秘密。它们不能互相替代，也不能把三者均描述为Core对code flow无条件要求的MUST。

code重复先核对原client、精确callback和verifier，再撤销基于该code签发的整个grant。refresh的已用token关系保留以检测reuse，先核对原client才能撤其family；错误client不能借他人的token触发撤销。若两个并发刷新中一个成功、另一个随后命中reuse撤family，成功者刚得到的新凭据也失效，不把“HTTP单赢家”误解释成其新族必然继续有效。

`prompt=none`不显示页面：无可用全局认证返回 `login_required`，没有有效既有grant、显式consent或请求offline_access时返回 `consent_required`。支持prompt为none／login／consent；none不能与其他值共用。`prompt=login`或超过 `max_age` 时须真实重认证，max_age=0同样要求新登录；不能刷新iat伪造auth_time。flow绑定原client、nonce、scopes、redirect，首次sid一旦绑定不随账号Cookie变化；restart原子消费旧flow并克隆单用新flow，强制fresh login。

当前浏览器绑定为独立 `__Host-saas_oidc_browser`，每个flow保存其hash及独立派生CSRF，不承载账号权限。该binder使用SameSite=None、Secure、HttpOnly、Path=/、无Domain及Max-Age=86400。两轮真实Edge跨来源顶层POST均观察到binder发送、Lax账号Cookie不发送，跳转后同源GET恢复原账号且保留先前flow。真正SSO与preauth Cookie继续SameSite=Lax，不把非账号binder政策扩散为账号Cookie政策。

完全没有binder的匿名并发首发authorize可能分别Set-Cookie，使其中一个flow在后续绑定校验中fail closed；已建立binder后的多flow有实测，不承诺所有匿名首发并发均能同时成功。

## 独立RP验证合同

标准RP库必须实际验证Discovery与token，而不是用provider自己的JWT解析器自证。当前ID Token提供RS256签名及 `iss,sub,aud,exp,iat,nonce,auth_time,sid`；single aud为实际client ID。RP核对固定issuer、当前audience、原nonce及RS256，按可信issuer的JWKS与唯一kid选key，不接受token内任意 `jku`／`jwk` 或回退首key。本issuer额外要求canonical UUIDv7 sub；这是本项目主体约束，不是任意OIDC issuer的通用UUID要求。

`exp`、`iat`由数据库时钟生成秒单位NumericDate，ID／access实际expiry相同且必须晚于iat。RP还需明确有限时钟偏差、未来iat和最大接受年龄，不能依赖库未检查的默认行为；auth_time来自原SSO主动认证时间，使用max_age时RP独立核对。当前Core errata set 2把azp作为可选扩展；当前single aud不额外制造azp，也不以azp替代aud验证。

当前refresh响应继续包含ID Token：新iat表示新签发，auth_time和nonce保持原grant值。这满足Core“若返回nonce必须为原值”的要求，与其默认建议省略nonce的SHOULD存在明确选择差异。初始code的nonce规则不无区别套用到refresh。refresh可带scope，只接受包含openid的原scope子集并收窄同grant；移除offline_access后不再签新refresh，不能扩大scope。

## 存储owner与撤销范围

账号／全局session继续沿用Phase 7、8的认证owner与唯一权威 `users.session_version`。[0017迁移](../../migrations/0017_oidc_provider.sql)新增独立的 `oidc_provider_settings,oidc_clients,oidc_authorization_requests,oidc_grants,oidc_authorization_codes,oidc_access_tokens,oidc_refresh_tokens,oidc_signing_keys`，均归OIDC owner及当前唯一canonical迁移作业。既有 `oauth_clients`、`oauth_tokens`、平台context和app凭据仍归平台legacy owner，未按URL前缀整表迁移。

所有系统UUID列独立约束v7，code／access／refresh／browser／CSRF只保存hash；state、nonce和challenge属于为协议回传保存的事务元数据，不是密码或原始Bearer。grant强绑定非空原SSO sid、账号版本及client策略版本，SSO父记录删除级联，不会清空sid降级为legacy。在线消费实时JOIN账号、SSO、client、grant及期限；物理回收不能代替权威检查。

| 触发 | OIDC侧需要的实时拒绝 | 必须保留的状态 |
| --- | --- | --- |
| 账号全局logout／kick／ban／密码reset | 所有关联旧sid／账号版本的code、refresh、access及userinfo拒绝；不可借新账号版本补绑定复活 | 无关账号及机器凭据 |
| 撤销单个OIDC client grant／refresh族 | 自client access／refresh撤整个原grant；code／refresh复用也撤同grant，其他client请求不触发撤销 | 全局SSO及其他client |
| OIDC client用途／允许身份策略变更 | 同client全部旧code／refresh／grant按递增策略版本失效，不查成员挑选所谓不合规code | 全局SSO及其他client，平台依据最新策略重建context |
| 停用一个平台成员或收紧某应用 | 平台以具体成员／应用及来源client策略拒绝业务context；OIDC不虚构成员授权 | 独立全局账号认证及不受影响身份／client |

员工控制台和客户门户的RP用途及 `allowed_member_types`须明确登记；它们表达来源准入上限，不能扩大平台实际成员与应用允许集合的交集。新client不继承legacy首个／最早客户端，也不默认启用password、device或machine grant。

管理API为 `GET/POST /api/oidc/clients`、`PATCH /api/oidc/clients/{id}`，要求当前平台根组织员工context及实时 `system:oauth` 权限。用途PLATFORM_CONSOLE只允许EMPLOYEE，CUSTOMER_PORTAL只允许CUSTOMER，ACCOUNT允许非空合法子集；系统生成v7 client ID与256-bit随机secret，注册仅首次响应secret。精确规范HTTPS callback不允许通配、凭据、fragment或已有code／state／error／iss参数。PATCH使用expectedAccessPolicyVersion，每次成功递增策略版本并同事务审计，旧grant即时失效。

## 密钥轮换与运维合同

[`oidc/keys.rs`](../../src/oidc/keys.rs)固定RS256，不复用平台共享secret，也未新增Cargo依赖。外部JSON格式为keys数组，每项字段 `kid,privateKeyPem,n,e`；文件最多512KiB、1～8把key，PEM最多16384字节。kid为1～80个ASCII字母数字／下划线／连字符且不重复，n为canonical无padding base64url、RSA为2048～4096-bit且无多余前导零，e固定AQAB（65537）。加载通过RS256签验探针验证公私钥配对，错误仅固定类别；KeySet不实现Debug／Serialize。

数据库只存公钥、生命周期和last_signed_exp，kid／n／e由不可变trigger保护。JWKS只发布 `kty:RSA,use:sig,alg:RS256,kid,n,e`；当前签名Header固定RS256及active kid，不含任意远程key URL或私钥。

正常轮换顺序为先发布新公钥，再切换新kid签发，旧公钥至少保留至最后旧token绝对到期加允许时钟偏差。必须以实际新旧token验证重叠窗口、未知kid和旧token过期行为；不能用“配置支持多个key”代替轮换验收。私钥泄漏的紧急撤销与普通到期轮换分开处理，未知kid不能触发无预算请求或跳过算法／签名检查。

当前操作顺序如下，均要求上述平台员工管理员权限，变更与审计使用原事务：

1. 在环境外安全准备新key，保留需继续签发的active私钥，将新项加入所有签发实例的 `OIDC_KEYSET_PATH` 并重启／部署验证。KeySet在配置加载时验证并缓存于Config Arc，不热读文件；仅修改文件不会自动轮换。不把私钥放请求body、数据库或证据。
2. `POST /api/oidc/keys/publish`，JSON只传kid；确认新公钥已出现在匿名JWKS。重复kid或不同材料不接受覆盖。
3. `POST /api/oidc/keys/activate`，JSON只传kid；必须已发布且与当前验证过的keyset相同，不可激活已退役／撤销kid。新签名改用新kid，旧公钥仍发布。
4. 等旧kid的最后签发exp加60秒窗口结束后，`DELETE /api/oidc/keys/{kid}` 正常退役。active或仍在窗口内返回冲突；结束生命周期的kid不能重用。
5. 私钥泄漏时用独立的 `POST /api/oidc/keys/{kid}/revoke` 紧急撤销，立即停止该kid新签发并从JWKS移除；必要时先激活干净key，避免没有active key导致换码／刷新503。

停止旧kid签发后，可以从后续实例配置移除其私钥；旧公钥独立保留在数据库，继续满足验证窗口。某实例若缺少新active kid的已验证私钥材料，换码／刷新返回503，不悄悄用旧key签发。多实例的配置／部署协调属于操作合同，不把当前同进程结果说成已完成独立部署。

签发持有active key行锁直到原token事务提交，并同事务推进last_signed_exp；生命周期命令用advisory及对应key行锁，避免退役判断后仍签出更晚token。60秒须覆盖登记RP的总接受偏差，不能无界叠加leeway。紧急删公钥不会立即清除RP已缓存的key，不能把该动作描述为离线ID Token即时全局失效。此阶段未实现或广告RP end-session endpoint；账号全局logout仍使用Phase8 API，后续BFF另定RP会话注销。

## 已完成的协议审查来源

本轮已只读核对官方规范，以下链接用于解释协议合同，不代表实现或认证测试通过：

- [OIDC Core：认证请求](https://openid.net/specs/openid-connect-core-1_0.html#AuthRequest)、[ID Token验证](https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation)、[refresh响应](https://openid.net/specs/openid-connect-core-1_0.html#RefreshTokenResponse)、[签名密钥轮换](https://openid.net/specs/openid-connect-core-1_0.html#RotateSigKeys)。
- [Discovery元数据](https://openid.net/specs/openid-connect-discovery-1_0.html#ProviderMetadata)及[配置验证](https://openid.net/specs/openid-connect-discovery-1_0.html#ProviderConfigurationValidation)。
- [RFC7636：verifier与S256](https://www.rfc-editor.org/rfc/rfc7636.html#section-4.1)、[RFC7518：RS256与RSA公钥](https://www.rfc-editor.org/rfc/rfc7518.html#section-3.3)、[JWT BCP算法验证](https://www.rfc-editor.org/rfc/rfc8725.html#section-3.1)。
- [RFC9700：refresh重用检测](https://www.rfc-editor.org/rfc/rfc9700.html#section-4.14.2)及[认证／授权UI防clickjacking](https://www.rfc-editor.org/rfc/rfc9700.html#section-4.16)。登录／同意／错误页面须有frame限制、no-store及收窄的脚本／表单策略；文本用安全DOM API呈现，不嵌未转义账号名或client显示名。

## 当前交付范围

本阶段在现有平台进程内完成独立OIDC provider，门禁、独立标准RP库与真实Edge浏览器结果已记录；平台BFF、客户门户及独立身份服务部署按后续阶段验收。TASK_OWNED TLS、域名和密钥仅证明本次隔离实现与测试，不表示生产资产已落实。

本阶段四项台账标准均已通过。当前19项源码保持验证时快照，历史0016未改，SSO与OIDC默认开关继续false；TASK_OWNED环境的测试启用不表示生产已开放。后续BFF使用当前固定issuer、client用途和在线原sid／版本证明建立业务context，不能把本阶段ID Token直接当作该context。
