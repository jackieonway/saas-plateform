# Phase 8 全局认证会话与撤销边界

状态：**COMPLETED**，依赖 [Phase 7](validation/phase-07.md) 已完成。194项普通测试、21项新增数据库测试、182项既有ignored、3项rollout启动／receipt、16项normal binary配置及15项真实Edge检查均通过，集中门禁与既有基线整链均exit 0。OIDC、BFF与独立SSO部署不在本阶段交付范围。完整结果和首次失败见 [Phase 8 验收](validation/phase-08.md)。

## 配置与账号API

| 配置 | 当前约束 |
| --- | --- |
| `SSO_ENABLED` | 默认 `false`；开启才接受全局认证入口 |
| `SSO_ORIGIN` | 开启时必需，一个精确HTTPS origin；host必须与 `PUBLIC_APP_URL` 不同，不能只用不同端口隔离Cookie |
| `SSO_SESSION_TTL_SECONDS` | 默认43200，允许1～86400秒；绝对期限，读取认证态不延长 |
| `SSO_PREAUTH_TTL_SECONDS` | 默认300，允许30～600秒 |

`SSO_ENABLED=true` 时还必须显式提供单一精确HTTP(S) origin的 `PUBLIC_APP_URL`，供配置校验比较平台host与认证host；只指定 `SSO_ORIGIN` 不足以启用。非HTTP(S)、含凭据、path、query、多origin或空值均不满足该合同。即使二者端口不同，只要host相同也必须拒绝，不能以端口模拟Cookie隔离。关闭SSO时保留原legacy配置语义。

接口由 [`sso/routes.rs`](../../src/sso/routes.rs) 提供，HTTP cookie和来源规则位于 [`sso/mod.rs`](../../src/sso/mod.rs)。它们只接受全局Cookie用途；请求携带Authorization头会被拒绝，查询参数也被拒绝，不提供Bearer兼容回退。

| 方法与路径 | 输入与证明 | 当前响应与作用 |
| --- | --- | --- |
| `GET /api/sso/preauth` | 精确Origin，或下文限定的同源安全GET证明；来源限流 | 创建有限寿命的一次性登录挑战，设置preauth Cookie，返回 `csrfToken,expiresIn`；不建立账号session |
| `POST /api/sso/login` | 严格JSON字段 `username,password`；精确Origin、preauth Cookie及 `X-CSRF-Token` | 先原子消费挑战，再验证精确username的账号密码；新建全局session并更换全新Cookie，清除preauth Cookie。成功返回下述账号投影；无成员也可成功 |
| `GET /api/sso/session` | 全局Cookie及安全GET来源证明 | 只读返回当前账号认证态和可恢复的session CSRF；不延长session，不创建context |
| `POST /api/sso/logout` | 全局Cookie、精确Origin及session `X-CSRF-Token` | 在账号锁后重验原sid及版本，执行账号级全局撤销，清除两种Cookie，返回 `{"ok":true}` |
| `PUT /api/sso/password` | 严格JSON字段 `oldPassword,newPassword`；全局Cookie、精确Origin及session CSRF | 验证密码及新密码策略，锁账号后重验原sid／状态／版本，再经账号port换密码并全局撤销；清除两种Cookie，返回 `{"ok":true}` |

login／session的成功投影为 `contractVersion: "sso-session/1"`，`account` 仅含 `id,username,realName,avatar`，另含 `authTime,expiresAt,csrfToken`。没有租户、组织、成员、角色、超管标记、业务token或refresh。`username`按既有账号名精确查找，不把共享电话／email自动合并为一个主体。全局sid是内部UUIDv7记录，不是暴露给业务API的新Bearer。

已定义接口的成功与错误响应统一 `Cache-Control: no-store`、`Pragma: no-cache` 及 `Vary: Origin, Cookie`。来源／CSRF错误拒绝、缺失或失效认证返回未认证、限流返回429与Retry-After，无法确认权威账号／session状态返回503；没有将依赖不可用解释成有效状态。

## 存储与秘密材料

[`0016_sso_global_sessions.sql`](../../migrations/0016_sso_global_sessions.sql) 属于当前唯一canonical迁移链，未创建第二个身份迁移器。

| 表 | 字段与用途 | owner与约束 |
| --- | --- | --- |
| `sso_sessions` | `id,user_id,token_hash,csrf_hash,account_version,authenticated_at,expires_at,revoked_at` | 账号认证owner；id／user_id为RFC UUIDv7，secret仅保存64位小写SHA256，版本非负，过期晚于认证时间，账号删除级联清理；没有member／tenant／org列 |
| `sso_login_challenges` | `id,token_hash,csrf_hash,created_at,expires_at,consumed_at` | 账号认证owner；UUIDv7、hash-only、期限及一次消费；不关联成员，不表示已认证账号 |

Cookie原始值和preauth CSRF均为独立32字节CSPRNG生成值，编码为64位小写hex。session Cookie名是 `__Host-saas_sso`，preauth名是 `__Host-saas_sso_preauth`，两者均设置 `Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=...` 且没有Domain。每次成功登录生成新secret，不接收浏览器预置值作为新session。

session CSRF采用 `SHA256("sso-session-csrf-v1:" + sessionCookieSecret)` 的独立用途派生值，数据库只保存该值的hash。`GET /session`验证当前Cookie及存储hash后重新计算并返回它，使页面reload可以恢复CSRF；并非从数据库取出明文Cookie。preauth CSRF独立随机且只可消费一次，登录失败后不能重放同一挑战。

[`sso/store.rs`](../../src/sso/store.rs)在事务advisory lock内清理最多1024条已消费／过期挑战，检查live挑战容量，再插入；容量使用 `LOGIN_MAX_ENTRIES`（默认4096）。因此容量检查与预留跨进程串行，而非每个实例独立计数后竞态超额。preauth和login也消耗来源限流，login中的密码证明复用账号失败预算及同一个PasswordService worker，处理预算不因为新增API另开一套。

`GET /preauth`是受限的认证引导状态创建；其余认证态GET只读。不能把这个短期挑战的创建描述为建立账号session，也不能允许GET执行logout或密码变化。

## 三层状态与权限

| 层次 | 归属与含义 | 必须保持的限制 |
| --- | --- | --- |
| 账号／全局session | 认证owner；主体、账号状态与代际、认证时间、期限和撤销 | 与成员独立；有效无成员账号可以认证，不能自动创建成员或默认组织 |
| 平台成员context | 平台owner；具体 `membership_id`、类型／代际、租户组织、来源和适用策略 | 员工与客户同组织也使用不同成员；全局登录不直接产生员工角色或超级管理员能力 |
| app／legacy派生凭据 | 平台owner；目标client／application与人员或机器用途、父上下文和策略版本 | SSO Cookie不能直接充当业务Bearer；普通JWT验签不能替代实时权威校验 |

已有账号权威代际存储于 `users.session_version`，平台凭据的 `account_version` 是快照；本阶段不得引入另一套可独立写入且互不同步的账号版本。`users.is_super_admin` 继续是平台授权策略，不是全局认证claim。

## 已确定的撤销接受点

| 触发操作 | 认证侧必须原子完成 | 平台／legacy侧必须立即满足 |
| --- | --- | --- |
| 全局注销、全局kick、账号ban、密码reset | 在账号锁下提升唯一权威账号代际，撤销该账号全局session；状态／密码变化与结果保持原子 | 所有旧平台／app／ordinary OAuth access、refresh、code、approved device失效；没有全局sid的旧凭据也要检查账号版本，不能等异步物理回收 |
| 停用／删除／替换一个成员 | 不撤独立全局账号认证，不提升账号代际 | 只按具体成员撤相关context和派生凭据，保留同账号同组织另一身份及其他组织 |
| 撤销一个平台context | 保留全局认证及其他合法context | 父session及其派生凭据失效，不扩大为无条件全账号注销 |
| client／application准入收紧 | 保留全局账号认证 | 按来源／目标策略版本使不合规context、token、code、refresh、device失效，不从其他client借用准入 |
| 权威账号／session依赖不可用 | 不能把不可证明状态当作已登录 | 受保护操作返回503，不退化为只验JWT、缓存放行或平台本地密码旁路 |

当前 `platform_session_active` 仍含legacy issuer约束，`StoredBinding`／`UserBinding`没有全局sid，ordinary password grant也不虚构平台父session。本阶段没有从全局Cookie创建平台context或app token；旧凭据通过 `user_id` 与实时账号版本关联全局撤销，**没有冒称所有legacy凭据已绑定global sid**。全局session自身过期使该Cookie失效，不等同于执行提升账号代际的全局logout。未来真正从SSO派生context时，必须记录并逐次验证原父关联；不得通过清空sid或 `ON DELETE SET NULL` 降级为可接受的无sid legacy凭据，也不能把现有JWT的平台sid改解释成全局sid。

## Cookie、CSRF与登录保护接受点

- 全局sid使用UUIDv7作为记录标识；Cookie认证secret独立使用CSPRNG，不能用sid代替秘密，持久化只保存hash。每次成功登录使用新的认证secret，旧值或攻击者预置值不能固定新会话。
- Cookie使用host限定命名与作用域，采用 `__Host-` 所要求的Secure、Path=/、无Domain，并设置HttpOnly与明确SameSite。Cookie不含成员、权限、密码或业务token；实际同进程origin配置不能冒充已经固定的OIDC issuer。
- 精确Origin和CSRF验证覆盖登录前以及已登录后的写操作；登录CSRF不能因“还没有会话”被遗漏。CSRF证明绑定预认证挑战或具体全局session，不能跨sid复用。SameSite是辅助条件，不替代CSRF验证。
- 安全GET允许浏览器省略Origin，但必须同时提供唯一 `Sec-Fetch-Site: same-origin` 和唯一Host，Host与配置HTTPS authority精确匹配（host大小写不敏感）。若Origin存在则必须精确匹配，坏／重复Origin不能由metadata补救；不信任 `X-Forwarded-Host`。写操作必须有唯一精确Origin，metadata不能替代它。
- 认证态GET只读；logout、改密不通过GET执行。拒绝缺失／错误来源证明、缺失／错误／跨session CSRF、伪造／重复或非canonical Cookie和错误用途的凭据。相关Cookie可分布在多个合法Cookie头，但相同认证Cookie名只能出现一次。
- 登录继续遵循输入上限、来源及账号预算、共享Argon2 worker容量和超时；不因新增入口扩大总密码并发或绕过账号保护。共享电话／email不能合并主体或权限，歧义选择必须明确。
- 新SSO个人改密的旧密码证明同样先检查输入上限、来源预算和 `user:{id}` 账号失败预算；错误旧密码记录失败，预算耗尽返回429与Retry-After，不能借有效Cookie绕过登录失败锁定。只有成功提交后才清除该账号失败预算，失败／限流不会撤销已有session或修改密码／代际。
- 账号ban、Cookie过期、撤销、账号版本不匹配与依赖故障分别验证；不可用返回503，不能伪装为成功认证或把已失效证明升级到当前版本。
- 日志和证据不保存密码、PHC、Cookie认证值、CSRF secret、raw access／refresh、code或client secret；审计只记录主体、事件、代际、范围和结果。

## 自助命令的提交前再验

[Phase 7 已确认的在途请求边界](phase-07-authentication-boundary.md#个人改密的既有在途请求限制与-phase-8-必闭环项)在本阶段通过两类适配处理。平台legacy个人改密先由PostgreSQL适配器锁账号并比较原密码快照，管理员reset已改变hash时保留409；随后route调用 `session::require_active_in_tx` 重验原平台session及具体成员，最后才经通用账号port写入。新的SSO自助命令则按原token hash查回同一sid，锁账号后重验账号状态、原版本、撤销及过期；不依赖任何成员。

ban／kick／成员撤销先提交时，依赖旧context的pending命令须拒绝；SSO自助命令遇到原global sid已撤销／过期或账号代际变化也拒绝。若用户要用密码重新认证，必须显式建立新认证，不能默补当前账号版本复活旧证明。新SSOlogout／改密在审计等等待之后、提交之前再检查原认证绝对截止时间，不因命令已撤销自己的session而重新启用它。新SSO命令可能先因原session失效返回未认证，不能把legacy管理员reset的409优先规则笼统移植到新API。

独立全局账号自助不应依赖成员存在；管理员reset仍允许受授权地重置DISABLED账号。平台成员条件留在平台入口，不能塞进通用账号密码替换方法。并发验证应使用受控屏障与真实账号锁等待，覆盖ban、kick、成员撤销及管理员reset已改hash后的409，不用固定sleep推断安全。

## 当前实施与验收边界

本阶段在现有平台包和进程内完成全局认证能力；194项普通、21项新增数据库、182项既有ignored、3项启动／receipt、16项配置及15项真实Edge检查通过。夹具、错误类别断言、配置QA环境与browser driver的首次失败均保留在验收记录。现有成员准入、固定SDK与通知合同保持各自用途；新全局登录不替代这些业务证明。

数据库与进程验证只操作明确的任务自有目标；Phase6保留开发库的receipt绑定其原文件hash和迁移／schema摘要，新构建及新增0016迁移不能借用旧receipt冒充已就绪。实际命令、失败与修复及各项结果按真实运行证据填入验收记录，不以本页的接受点代替PASS。
