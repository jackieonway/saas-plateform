# Phase 9：标准OIDC provider验收

日期：2026-10-25。状态：**COMPLETED**，依赖 [Phase 8](phase-08.md) 已完成。最终静态门禁、194项普通测试、9项新增PG、203项既有ignored、3项启动／receipt及37项配置检查均通过；openid-client 6.8.8与真实Edge两轮各117项、最终6图视觉验收均通过。静态、新PG和既有回归命令均exit 0，任务库及临时keyset清理成功；源码19项保持冻结，默认开关仍false。

已完成官方Core／Discovery、JWT BCP、RSA／PKCE与refresh安全要求的只读设计审查，随后核对七个OIDC模块、配置、bootstrap及schema；GET／POST协议方法、参数重复、固定issuer、claim投影、refresh语义、密钥边界及UI安全已按源码写入 [provider说明](../phase-09-oidc-provider.md)。这不代替运行验收。

范围依据 [Phase9台账](../task-ledger.md) 及 [provider边界](../phase-09-oidc-provider.md)：稳定issuer、discovery／JWKS、code＋S256、nonce／state、ID Token、独立非对称密钥与轮换、独立OIDC客户端及存储owner、刷新轮换／重用检测、账号与client撤销。平台BFF、客户门户和独立服务部署属于后续阶段。

## Self-Check：台账标准

| 验收项 | 结果 | 实际证据 |
| --- | --- | --- |
| 标准客户端可完成code＋S256并验证ID Token；无成员仅完成账号认证 | **PASS** | 独立标准RP库与真实Edge两轮通过；无默认成员／context、无业务Bearer权限 |
| code重放、错误issuer／audience／nonce／callback／PKCE、过期和算法降级拒绝 | **PASS** | PG正控后逐项负例及实际code重放；真实state／nonce与strict JWT变体分开记录 |
| 新旧密钥重叠及失效后行为正确，claim无员工权限，legacy回归通过 | **PASS** | 真实key重叠／60秒窗口、过期及fresh JWKS、最小claim；203项既有回归及3项启动全部通过 |
| 客户／员工client用途可识别且不扩大业务准入，收紧后旧code／refresh不恢复context | **PASS** | 用途登记与来源策略版本、client局部撤销及其他client／全局SSO保留；P9不创建业务context，后续BFF按最新策略重建 |

## 实际门禁与首次失败

以下仅记录根代理实际运行；独立审查未启动Cargo、数据库或浏览器。

| 组 | 实际结果与范围 |
| --- | --- |
| 初版静态 | [initial-check.log](../../../../local-identity-sso-phase09-qa/initial-check.log)中check1m09s；[static-ordinary.log](../../../../local-identity-sso-phase09-qa/static-ordinary.log)中Clippy1m24s、普通194通过，主组166／203 ignored／8.04s。当时尚未注册新增9项。root在后续normal build时主动停止旧命令，exit137，用于收窄修补后重跑；不把它写成正常构建通过或生产崩溃。 |
| 新测试首次注册编译 | [registered-static.log](../../../../local-identity-sso-phase09-qa/registered-static.log)实际报E0277：测试helper比较 `&Cow<str>` 与String无对应PartialEq；只修比较类型后进入最终门禁。此为测试编译失败，未执行PG断言。 |
| 最终静态整链 | **exit 0**，见 [static-final.log](../../../../local-identity-sso-phase09-qa/static-final.log)：fmt、check43.77s、Clippy58.17s、普通test构建1m11s、normal backend／migration binary build58.86s、doc28.01s均通过。 |
| 最终普通测试 | **194 PASS**：14 lib＋166 main＋14 external（4＋1＋7＋2）；主组212 ignored，执行7.12s。ignored数量包含新增9项，普通通过不代表PG测试已执行。 |
| 新增9项PG | **9／9 PASS，112.95s，exit 0**，见 [new-pg.log](../../../../local-identity-sso-phase09-qa/new-pg.log)。最后密钥组含真实窗口等待；9次任务库删除／pool关闭及临时keyset移除均true。 |
| normal binary配置 | **37／37 PASS**，见 [config-check.log](../../../../local-identity-sso-phase09-qa/config-check.log)。全部在配置层终止，没有数据库连接；默认关闭、合法issuer／keypair及TTL边界正控用预置非法bind终止，不能算服务启动。 |
| 独立标准RP／真实Edge | **两轮各117项PASS，均exit 0**；首轮 [主结果](../../../../local-identity-sso-phase09-qa/browser-result-initial.json)／[driver](../../../../local-identity-sso-phase09-qa/browser-driver-result-initial.json)与最终 [主结果](../../../../local-identity-sso-phase09-qa/browser-result.json)／[driver](../../../../local-identity-sso-phase09-qa/browser-driver-result.json)均保留。不把重复运行累加成234种独立用例。 |
| 最终视觉取样 | **6图PASS**，见 [visual-review.json](../../../../local-identity-sso-phase09-qa/visual-review.json)。首轮2张取到有限动画中间帧，保留原图后仅调整截图工具；产品UI未为重跑更改，root逐张看完最终图确认可读、无裁切／横向溢出。 |
| legacy／既有ignored基线 | **203／203 PASS，868.15s**，见 [existing-baseline.log](../../../../local-identity-sso-phase09-qa/existing-baseline.log)；175 filtered为166普通＋9项新OIDC。这是0017及新binary后的实际重跑，包含Phase8的21项，没有借用旧阶段结果。 |
| 启动／receipt基线 | **3／3 PASS，246.88s**，三个用例各有owned database cleanup PASS；与203项同一串行命令链，整链exit 0。实际使用当前0017迁移链、normal binary和fresh任务库，未复用Phase6保留库的旧receipt。 |

根代理在Linux环境执行的最终静态命令如下，整链exit 0：

```sh
cargo fmt --all -- --check &&
cargo check --locked --all-targets --all-features &&
cargo clippy --locked --all-targets --all-features -- -D warnings &&
cargo test --locked --all-targets --all-features &&
cargo build --locked --bin saas-platform-backend --bin platform-migrate &&
cargo doc --locked --no-deps
```

新增PG命令已exit 0：

```sh
cargo test --locked --bin saas-platform-backend oidc_tests:: -- --ignored --test-threads=1 --nocapture
```

既有回归按下列命令串行执行，203项及3项全部结束，整链exit 0：

```sh
cargo test --locked --bin saas-platform-backend -- --ignored --test-threads=1 --skip oidc_tests:: &&
cargo test --locked --test membership_rollout -- --ignored --test-threads=1 --nocapture
```

203项既有回归中，UUID目标检查实际覆盖137个独立UUID列并通过，历史exact baseline、并发及checksum用例也通过；新表不改历史迁移字节。根代理另外确认19项源码快照无漂移、旧0016 SHA未变，保持当前binary与源码审查合同的同一范围。

37项配置检查涵盖OIDC必须启用SSO、缺失／不匹配／HTTP／path／尾斜杠issuer，外部keyset缺失／相对路径／不可读／空／畸形／重复kid／错误配对／弱或非canonical modulus／错误指数／无效私钥及kid，以及code／token／refresh／interaction TTL上下限拒绝与边界正控。输出只含固定类别、退出码及字节数，没有原始PEM或秘密。

## 证据登记边界

- 实现owner冻结endpoint、配置、schema与crypto合同后，再将源码事实写入说明；源码审查发现与执行失败分开记录。
- 独立标准RP库的实际版本、验证选项及成功／拒绝用例必须记录，不能用provider自己的JWT解码器自证互通。
- 浏览器和协议测试使用明确TASK_OWNED域名、临时TLS与测试密钥；记录实际信任范围，不变更系统／生产信任或虚构生产issuer。
- OIDC code／refresh／Cookie、原始ID／access token、client secret、签名私钥及密码不落日志。证据保留状态、摘要、claim字段名及经过检查的最小投影。
- 离线ID Token的有效期与在线grant／账号撤销分开说明；不声称撤销数据库行自动使任意RP已缓存的公钥失效。
- 命令、首次失败、修正、重跑、任务库／进程清理及实际改动仓库按真实运行补齐，尚未执行的不记PASS。

## 源码审查发现与修正

以下均为开发中只读审查发现，不记录成已经执行过的失败测试：

- **派生凭据在审计等待期间过期。** code创建／access-ID签发已返回原绝对expiry，自动授权、手工同意、code兑换及refresh在审计后纳入最终最早deadline；后续PG已实际覆盖自动／手工授权的code在审计锁等待期间过期并整体回滚，不把该用例扩大为所有命令的全部过期交错。
- **max_age在锁／审计等待后越界。** 自动批准锁后重新eligible，自动与手工批准的最终期限包含原auth_time＋正max_age；PG真实审计等待已验证两条路径越界拒绝，不以新iat代替重认证。
- **管理员原事务内跨pool检查。** admin_tx已改同事务读取平台根组织及context_menu_rows_in_tx，保留员工权限门禁；PG中的单连接pool管理正控实际通过。发现方式是静态审查，不写成曾运行发生死锁。
- **同值body client_id兼容。** 初版Basic认证把任何body client_id一律当第二认证方法拒绝；源码现允许同值canonical ID，继续拒不匹配及真正的secret／assertion混用，PG正负控已通过。
- **token错误envelope。** 初版RateLimited经通用业务映射缺少顶层OAuth error；已改429 temporarily_unavailable及Retry-After，PG实际核对该错误、正Retry-After及没有业务envelope。内部Server失败另为500 server_error并保留脱敏trace；PG实际审计失败已验证服务器错误及完整回滚，不扩大成所有500错误路径均逐一实测。
- **cross-site POST的多flow浏览器绑定。** Lax binder在跨来源顶层POST中缺失，可能生成新binder覆盖已有flow。仅非账号OIDC browser binder改为None，保留Secure／HttpOnly及每flow CSRF；PG HTTP用例及两轮真实Edge已有binder的cross-site POST均通过，SSO／preauth仍Lax。

当前issuer持久pin、RS256公私钥配对、唯一kid／公钥不可变、元数据与实际endpoint对应、get／post方法、code／refresh reuse先核原client及userinfo scope最小投影已源级确认，9项PG已通过；不能把其中自有JWT解码测试说成独立标准RP互通。refresh继续带原nonce是有记录的Core SHOULD差异，未当作MUST违规。

## 新增PG证据的范围

最终9项实际通过的证明如下；每项使用独立任务库与临时keyset，清理结果见new-pg日志：

| 组 | 实际断言 |
| --- | --- |
| 无成员code＋S256与claim | 固定issuer／主要Discovery元数据、真实无成员登录／userinfo、不创建成员；ID／access不能替代业务凭据；JWT签名及错误issuer／audience／nonce／sub／expiry／HS256／none拒绝；audit敏感值负控 |
| 协议参数与code绑定 | 精确callback、无效client、PKCE降级／畸形、scope／nonce／prompt／max_age／重复字段拒绝；失败不消费code；同值body client_id成功、错误／混用认证拒绝、POST userinfo可用；429 OAuth error／Retry-After／no-store；顺序重放撤原grant |
| refresh族与并发 | 错误client不破坏原族；实际轮换保留auth_time；旧refresh reuse撤原及后代凭据，其他client／全局SSO保留；并发一成功一invalid_grant，随后连成功者的新族也失效 |
| client用途与策略版本 | 单连接pool管理正控；真实策略更新撤同client跨账号旧grant／access／refresh／pending code；恢复旧物理行仍被版本拒绝；其他client及SSO保留，expected version过旧409 |
| interaction及账号切换 | flow Cookie／独立CSRF及已建立binder的多flow绑定、跨来源POST binder保留的HTTP用例；换账号不能批准旧flow，restart克隆并要求新认证；deny callback、prompt=login／max_age=0及none无认证错误 |
| 在线权威与issuer故障 | global logout撤两个client旧凭据；原sid过期只影响关联链，另一sid保留；旧物理行受账号代际拒绝；issuer漂移及pool关闭返回503且无部分认证 |
| 换码锁等待与审计回滚 | 观察真实blocker PID／账号锁等待，再提交sid／账号／client策略撤销，换码invalid_grant、不消费code／不写token或成功audit；真实audit trigger失败整体回滚，包括key窗口状态，恢复后同code仍可兑换 |
| 密钥真实窗口 | A／B预发布／激活重叠签验、active及窗口内旧key不能正常退役，真实等待last_signed_exp＋60后退役；旧kid不可复活；紧急撤销移除fresh JWKS、无active signer503且不消费code；无私钥字段 |
| 审计等待跨期限 | 自动与手工授权分别覆盖code寿命及原认证max_age在已观察审计锁等待中到期；无callback、整体snapshot回滚，手工401／自动400 invalid_grant |

[`oidc_tests.rs`](../../../src/oidc_tests.rs)的验证helper从公开JWKS选key，用jsonwebtoken核对JWS与claim；provider也使用该JWT crate。这是独立于签发逻辑的测试校验器，**不是外部成熟OIDC RP／Discovery库**。它设零leeway、要求iat不晚于主机now＋1秒、exp大于iat、auth_time为正，首项另与SSO数据库原auth_time／sid精确比较，refresh另要求auth_time保持。

code重放用例为顺序重放，refresh有真实并发单赢家及随后reuse撤整族（包括赢家新凭据）；不能混称并发换码已实测。密钥用例使用fresh JWKS及真实数据库last_signed_exp＋60秒窗口；不证明缓存RP未知kid刷新预算或紧急清除旧cache。9项没有直接HTTP请求introspect／revoke，保密断言查audit_logs，不代替进程运行日志扫描。RP侧max_age、iat最大年龄及60秒偏差接受边界也不能据helper零leeway推断已验证。

## 独立标准RP与真实Edge

两轮实际使用Microsoft Edge **142.0.3595.94**、Playwright **1.64.1**、**openid-client 6.8.8**，明确开启enableNonRepudiationChecks，以同一actual normal backend binary分别完成117项检查。原始结构化结果保存为initial及最终两份；以下为共同覆盖，不把Node语法检查或服务READY当作浏览器通过。

两轮实际TASK_OWNED issuer均为 `https://auth.saas.test`（无显式端口，443临时gateway），RP origin为 `https://console.saas.test`，精确callback为 `https://console.saas.test/auth/callback`。临时证书SAN另含customer.saas.test，但P9未用该域作为RP，不据此声称客户门户已接入；这些地址均不是已落实的生产域名。

| 组 | 实际证明 |
| --- | --- |
| 独立RP正控 | 成熟库真实Discovery、code＋S256、单次token交换、ID Token签名与claim、userinfo及精确subject；桌面approve、restart后approve及撤销后fresh同client流程完成 |
| 真实state／nonce负例 | 错误expected state在真实callback且兑换前被openid-client拒绝；错误expected nonce在真实token响应后被拒绝 |
| JWT变体控制 | 错误issuer／audience／额外audience／expiry／nonce／v4主体／算法降级／改签名通过内存TASK_OWNED RSA签名控制及独立strict verifier拒绝；它们不是伪造真实provider响应后全部交由openid-client拒绝的证据 |
| 真实登录／同意 | Enter登录不自动批准；server给出的client／用途／账号／openid-profile显示，取消返回access_denied及原state；移动页无横向溢出，restart需fresh login，任意return URL不发请求或导航 |
| Cookie与请求证明 | 账号Cookie host-only Secure／HttpOnly，非账号binder另为None；Storage无Bearer／CSRF，真实login和interaction写请求有Origin＋CSRF，同源GET使用Fetch Metadata；CSP／cache／frame限制有检查 |
| 跨来源POST | 真正Edge opaque data URL HTML form顶层POST，Origin null、Sec-Fetch-Site cross-site；binder发送、Lax SSO不发送，原未消费flow保留，重定向后的GET恢复原账号 |
| introspect／revoke | 当前confidential client实际Basic introspect200且active／client／sub匹配，无业务claim；revoke200后introspect只active:false，userinfo为Bearer invalid_token 401；全局SSO仍200，新同client同意可重新发凭据，旧access仍inactive |
| UI故障呈现 | 单次interaction GET 503由Playwright route模拟，只证明页面显式retry及恢复交互；真实后端权威故障证据来自PG关闭pool用例，不混为一个事实 |

该HTTP token管理证据限于当前confidential client；其他client和全局保留的更广撤销证据结合PG组，不能说成该浏览器脚本已遍历所有client。两轮浏览器及gateway均正常关闭，bridge退出0；TLS临时证书仅由issuer customFetch信任，隔离Edge context接受fixture TLS，没有修改系统DNS或信任。真实RP请求openid／profile，不签refresh；refresh轮换与reuse的实际协议证明来自PG组，未冒称成熟RP库的refresh流程也在这两轮执行。

首轮driver实际TASK_OWNED库为 `identity_phase09_browser_01a16f7189197985a28839f503f71763`，账号仍memberless；normal backend SHA-256为 `c039808dfa2f070b734c505210dba91eaa234f4ce588ca3a346dc62478a9b231`，migration binary为 `0ee0e0235a2e505a8eb9fe379688bc7bb31b02e0de1d1ca9931c16c725dd20f5`。backend退出0、删除前连接0、任务库已删除。全局session4／active4，OIDC grant4／active3，成功审计SSO_LOGIN、OIDC_AUTHORIZE、OIDC_EXCHANGE_CODE各4；局部撤销没有把全局认证误改为注销。

首轮完整backend stdout **44265字节**／stderr **0字节**、migration stdout **30字节**／stderr **0字节**均由完整reader在内存扫描九类模式及已知fixture秘密，无命中，只保存SHA-256、字节数和类别。这里扫描的是backend／migration输出；[launcher摘要](../../../../local-identity-sso-phase09-qa/browser-launcher-result-initial.json)明确父pipeline没有捕获native Node child stdout，browser-check日志是从sanitized主JSON明确复制的内容，不是原始Node输出，不能把它称为Node原日志扫描结果。

首轮6张截图保留于screenshots-initial；desktop-consent与desktop-account-restarted取到arrive动画中间帧，没有将它们当产品失败。仅将截图工具设置为animations disabled，使用新任务库 `identity_phase09_browser_01a16f7b1a527031a37d19651d743370` 重跑全部117项，产品源码及两个binary hash未改变。最终driver／bridge均exit 0，memberless为true、成功审计各4、SSO session4／active4、grant4／active3（精确撤一个），backend退出0、删除前连接0、库已删除。最终完整backend stdout **44213字节**／stderr0及migration30／0同样reader完整、模式扫描无命中；Node launcher的结构化主结果复制说明继续保留。

root通过view_image逐张检查最终6图，确认可读、无裁切及横向溢出，见visual-review.json。最终截图如下，首次捕获及runtime／launcher证据均未覆盖删除：

| 桌面 | 移动与错误状态 |
| --- | --- |
| [桌面登录](../../../../local-identity-sso-phase09-qa/screenshots/desktop-login.png) | [移动登录](../../../../local-identity-sso-phase09-qa/screenshots/mobile-login.png) |
| [桌面同意](../../../../local-identity-sso-phase09-qa/screenshots/desktop-consent.png) | [移动同意](../../../../local-identity-sso-phase09-qa/screenshots/mobile-consent.png) |
| [切换账号后](../../../../local-identity-sso-phase09-qa/screenshots/desktop-account-restarted.png) | [无效登录链接](../../../../local-identity-sso-phase09-qa/screenshots/invalid-login-link.png) |
