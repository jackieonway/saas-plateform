# Phase 7 账号认证边界与存储 owner

本文保留 Phase 7 验收时的实现快照。后续 [Phase 8](phase-08-global-session.md) 已建立独立全局 SSO 表／Cookie，并完成旧个人改密的账号锁后原 session／成员再验；实际验证见[Phase 8 证据](validation/phase-08.md)。下文“当前不存在全局会话”和在途请求限制描述 Phase 7 时点，不表示后续修复尚未交付。OIDC、BFF 及独立部署仍按台账各自阶段验收。

本阶段在现有后端进程内分离账号认证与平台组织授权。账号认证回答“哪个账号通过了何种凭证、当前账号状态与代际是否有效”；平台决定具体成员、租户／组织、来源客户端、应用准入、角色、菜单和数据范围。认证结果不授予平台权限，也不隐式创建成员。

本文区分三件事：数据的逻辑 owner、同进程过渡实现实际执行 SQL 的位置、未来独立部署的迁移／数据库权限 owner。Phase 7 没有创建独立 SSO 服务或 OIDC provider；Phase 8 全局 session、Phase 9 OIDC 和 Phase 12 服务拆分都不能由模块命名推定已经交付。测试结果以本阶段 validation 文档为准。

## 账号字段逐项归属

当前 `public.users` 是混合物理表。其 14 个字段来自 `0001_init.sql`；`0012_uuid_v7.sql` 移除数据库随机 UUID 默认值并加入 RFC UUIDv7 CHECK，`0014_credential_identity_binding.sql` 加强账号代际与撤销保护。稳定主体始终是新库创建后不变的 `users.id`。

| 字段 | 逻辑 owner 与用途 | 平台可消费的投影／命令 | 独立部署约束 |
| --- | --- | --- | --- |
| `id` | 认证账号主体，canonical RFC UUIDv7 | 按主体引用；成员等关系仍由平台拥有 | 保持 ID，未来 OIDC `sub` 使用同一值；不重建或映射第二账号 |
| `username` | 认证账号登录名，唯一 | 管理目录／本人显示；通过账号管理命令创建 | 不凭平台应用策略改写；唯一性由认证 owner 维护 |
| `password_hash` | 认证凭证，Argon2 编码，仅认证内部可读写 | 仅提交明文到验证／更改／重置 port；不返回 hash | 从平台可读投影排除；认证角色独占写权限，平台不能直接 UPDATE |
| `real_name` | 账号基础显示资料 | 本人／受授权目录的显示，按 scope 经管理命令修改 | 不等于员工姓名授权证明；共享资料不带组织权限 |
| `email` | 账号联系资料，同时是现有 legacy 登录别名 | 经管理命令修改，目录按既有范围读取 | 当前允许共享别名，不作为人或成员的唯一键，不假定已验证 Email |
| `phone` | 账号联系资料，同时是现有 legacy 登录别名 | 同上，复用既有规范化口径 | 当前允许共享电话，不凭电话合并账号、成员或角色，不假定已验证电话 |
| `avatar` | 账号显示资料 | 最小 profile 只读投影；修改归账号管理 owner | 不携带组织归属或权限 |
| `gender` | 账号显示资料 | 按既有枚举与范围通过账号管理命令修改／读取 | 不进入认证授权 claims，不用于准入 |
| `status` | 全局账号 ENABLED／DISABLED，认证 owner | 平台先核验管理员权限、目标范围、自我保护，再发起全局启停命令 | 全局禁用影响该账号所有身份；不能替代停用某一成员 |
| `is_super_admin` | **平台授权策略**，不是通用账号身份声明 | 平台新鲜员工证明、CurrentUser、菜单／应用授权读取；现有 bootstrap 种子显式写入 | 保留平台 owner；不出现在认证证明、全局 session 或 OIDC claims 中 |
| `last_login_at` | 认证行为元数据 | 按管理／本人范围只读显示 | 由认证成功流程更新；不能证明成员仍有效 |
| `created_at` | 账号创建元数据 | 只读目录／审计 | 创建 owner 写入，平台不得借更新时间替换主体 |
| `updated_at` | 当前混合行更新时间 | 只读目录／审计 | 当前过渡行共用；拆表后凭证／资料和平台策略各自保留更新时间，不能让双方任意整行写入 |
| `session_version` | **当前账号权威代际**，认证 owner；平台凭据的 `account_version` 是其快照 | 按主体读取状态／版本；重置、全局撤销由账号管理命令提升 | 单调提升，不因局部成员变更提升；未来可换存储名，不能出现两个可各自写入的版本来源 |

`is_super_admin` 的读取不能省略员工资格。`authorization::employee_context` 先以精确平台 session 取得有效 EMPLOYEE 成员及版本，再读取此平台策略；应用授权也使用平台当前状态。持有同一账号的 CUSTOMER context 不因账号上有这个位就获得后台权限。基础资料、认证成功结果和未来 ID Token 均不能携带一个可被下游直接解释为超级管理员授权的声明。

密码不是用户资料更新字段。账号资料投影不含 `password_hash`，审计只记录 `passwordChanged`、版本变化和撤销数量，不记录明文、hash、refresh、JWT 或 client secret。共享 email／phone 登录可以返回多个已验证账号候选，但平台必须再由明确 `user_id` 与 `membership_id` 选取当前身份，不能按别名合并权限。

## 现有会话与凭据存储

| 存储／凭据 | 当前真正用途 | 唯一写入 owner | 读调用与跨边界合同 | 迁移 owner |
| --- | --- | --- | --- | --- |
| `users.password_hash` 及账号状态／代际 | 全局账号认证与凭证失效 | 账号认证／账号管理模块；本阶段同进程适配执行数据库写入 | 验证、账号命令和状态证明 port；平台不依赖 Argon2 编码 | 当前仍由唯一 canonical 平台迁移 job 管理混合表；Phase 12 按字段拆分并转交认证 schema |
| legacy `credential` 票据 | 密码认证后、选择平台成员前的短时签名证明；当前还包含候选成员快照 | 平台 legacy 登录适配器 | `/api/auth/context` 必须重新验证账号代际、来源策略、所选具体成员及组织链 | 当前无独立表；签名和用途保留平台兼容 owner，不能更名成全局 session |
| **全局 SSO session** | Phase 8 才建立、与成员独立的账号级认证状态 | 未来认证 owner | 未来全局 sid／版本／认证时间／撤销证明；平台需另建 context | **当前不存在该表／Cookie／独立部署**；未来单一认证迁移任务创建 |
| `platform_sessions` | 选定 EMPLOYEE 成员后的平台组织 context，含账号／成员／来源策略快照 | 平台 session owner | `session::active_user`、`platform_session_active`、平台 refresh、派生 app／code／device校验；账号全局代际变化使其失效 | 平台迁移 job，未来继续归平台，不随 SSO 全局 session 迁走 |
| `platform_session_refresh_tokens` | `psr_` 命名空间、只存 hash、一次消费轮换且绝对过期不延长 | 平台 session owner | 同一 session、账号代际、来源策略和成员实时校验；与 OAuth refresh 区分 | 平台迁移 job；未来 BFF 在服务端管理，不能当 OIDC refresh |
| 平台 app token | 平台签发的精确应用／组织／成员业务凭据，物理存于 `oauth_tokens` 的 `token_use='app'` 行 | 平台应用凭据 owner，由 `routes/oauth2.rs::session_token` 在原事务写入 | app token 权威校验、app authorize/context/目录；绑定来源人类 client、目标服务 client、应用和代际 | 继续平台 owner；不能因表名含 OAuth 迁入认证核心 |
| `oauth_clients` | 当前 legacy 协议客户端、console 来源和应用服务客户端的混合登记 | 平台 legacy OAuth／准入策略 owner | client认证、grant与scope/PKCE、应用绑定、subject mode与类型准入；认证算法可通过窄port调用 | 保留平台；未来OIDC RP注册独立建模，不能让两个服务共同迁移此表 |
| legacy OAuth access／refresh | `oauth_tokens` 的 `token_use='oauth'` 行，包含 USER、MACHINE 与 legacy 混合来源 | 平台 legacy OAuth owner | token／introspection／revoke、应用准入与相应主体的实时证明 | Phase 7～12 保留平台 legacy 表；未来新 OIDC 使用独立注册及独立存储 |
| `oauth_authorization_codes` | 当前 legacy 授权码交换，含所选平台成员／来源／应用策略快照 | 平台 legacy OAuth owner | 签发、一次消费及换 token 都检查当前状态，不能凭认证 session 跳过平台准入 | 保留平台；未来 OIDC code 属认证 provider 的新模型，不共用旧表含糊切换 owner |
| `oauth_authorization_requests` | 当前 legacy 交互请求／确认状态，绑定 client 与应用策略版本 | 平台 legacy OAuth owner | consent 前后策略复核及消费 | 保留平台迁移 owner |
| `oauth_device_codes` | legacy device 授权、待审批／已审批／过期状态及选定平台成员 | 平台 legacy OAuth owner | device 创建／审批／轮询兑换、过期维护；账号和成员撤销都覆盖尚可兑换的状态 | 保留平台；不能按 `/oauth/device*` 路径推为独立认证数据 |

`platform_sessions` 基础字段是 `id,user_id,tenant_id,organization_id,issued_at,expires_at,revoked_at,ip,user_agent`，扩展字段是 `account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version`。这些字段共同表达平台选定上下文，不能仅改表名或去掉名称中的 platform 就充当全局认证会话。

平台 refresh 字段是 `id,session_id,token_hash,session_version,issued_at,expires_at,consumed_at`；其中版本必须与所属 session 的 `account_version` 及账号权威代际一致。局部 session logout 和设备撤销属于平台；全账号注销、封禁和密码重置属于账号 owner 发起的全局变化。

## 混合 legacy OAuth 的字段与用途分类

以下字段按当前真实迁移链 1～15 列出，不能因为相邻列属于协议或 hash 就把整行转交认证服务。所有这些现有物理表的单一逻辑 owner 和迁移 owner 都是**平台 legacy 凭据模块／平台 canonical 迁移 job**。内部 HTTP handler、发放器、管理接口、维护任务和保护 trigger 是该 owner 的实现入口，不是互相独立的第二个表 owner。

| 表 | 协议／凭证及生命周期字段 | 平台上下文／策略字段 | 归属结论 |
| --- | --- | --- | --- |
| `oauth_clients`（19列） | `id,client_id,client_secret_hash,client_name,description,client_type,redirect_uris,grant_types,scopes,access_token_validity,refresh_token_validity,status,created_at,updated_at,require_pkce` | `application_id,subject_mode,allowed_member_types,access_policy_version` | client secret校验可以复用认证算法能力，登记和用途策略继续平台owner；新的OIDC注册不得继承含糊legacy client |
| `oauth_tokens`（28列） | `id,token_hash,token_type,jwt,scope,expires_at,revoked,created_at,storage_version` | `user_id,client_id,tenant_id,organization_id,token_use,audience,jti,application_id,platform_session_id,account_version,membership_id,member_type,identity_version,client_policy_version,application_policy_version,origin_issuer,origin_client_ref,origin_client_policy_version,application_service_client_ref` | `token_use`和实际主体／proof决定用途；app token和legacy OAuth不能整表迁移 |
| `oauth_authorization_codes`（24列） | `id,code_hash,client_id,redirect_uri,scope,code_challenge,code_challenge_method,expires_at,consumed,created_at` | `user_id,tenant_id,organization_id,account_version,membership_id,member_type,identity_version,client_policy_version,application_id,application_policy_version,origin_issuer,origin_client_ref,origin_client_policy_version,platform_session_id` | 旧码承担平台成员准入与派生凭据交换；继续平台owner |
| `oauth_device_codes`（24列） | `id,device_code_hash,user_code,client_id,scope,status,interval_seconds,last_polled_at,expires_at,created_at` | `user_id,tenant_id,organization_id,account_version,membership_id,member_type,identity_version,client_policy_version,application_id,application_policy_version,origin_issuer,origin_client_ref,origin_client_policy_version,platform_session_id` | 平台成员审批与一次兑换状态不能移给只管理账号的内核 |
| `oauth_authorization_requests`（13列） | `id,client_id,redirect_uri,scope,state,code_challenge,code_challenge_method,expires_at,consumed,created_at` | `client_policy_version,application_id,application_policy_version` | 未选择成员前也已绑定平台client/app策略；继续平台owner |

相应 schema 来源为 `0001_init.sql`、`0003_oauth_issuance.sql`、`0007_oauth_pkce_policy.sql`、`0013_membership_identity_policy.sql` 与 `0014_credential_identity_binding.sql`。禁止重新编号或改写历史 checksum。

`jwt` 是兼容存储字段，不代表新流程保存原始 JWT：新 ordinary／app 发放保存 hash，`jwt=NULL`；`storage_version=1` 的旧字段兼容不能变成新存储策略。`0014` 已使缺少可靠成员／账号／来源绑定的旧行失效，不能读取当前成员后为旧凭据补签 proof。`origin_*` 列的存在也不证明已有全局 SSO 关联：当前 legacy code／device 的实际签发和审批主要保存账号／成员／client／app快照及 `platform_session_id`，后者是平台成员 context，**不是 SSO sid**。

三种 client subject mode 与 token 用途必须交叉判断：

| 情形 | 主体和证明 | 平台／认证边界 |
| --- | --- | --- |
| `USER` + ordinary password/code/device/refresh | `users.id`、精确member及账号／成员／client/app策略快照 | 密码验证属认证；成员选择、准入与legacy token发放属平台 |
| `MACHINE` 或现存 `LEGACY_MIXED` + `client_credentials` | 机器client；`user_id/tenant_id/organization_id`及人员member proof必须为空 | client凭证可验证机器主体；不产生员工或客户context |
| CONFIDENTIAL服务client + `session_token` | 从真实有效平台session派生人员app token，`token_use='app'`；目标服务client可以是MACHINE | **MACHINE登记不表示该client全部token都是机器人员替代物**；来源人类client、实际member与目标服务client分别校验 |
| `LEGACY_MIXED` 既有登记 | 兼容存量grant组合，按每次实际grant分类 | 禁止新增或把其他模式切回混合模式；不自动解释成OIDC client |

legacy机器JWT的 `sub` 使用 `client_id`，人员JWT才使用账号ID；不能把所有legacy `sub` 当作未来OIDC人员主体。现有 `/oauth/userinfo` 还可能返回平台租户／组织与员工有效角色，它是平台兼容接口，不是可整体迁走的纯账号资料API；未来OIDC userinfo不能把这些组织权限包装为通用认证claims。

## 当前读写调用链与过渡适配

| 入口／模块 | 账号认证或管理职责 | 继续由平台拥有的职责 |
| --- | --- | --- |
| `routes/auth.rs::login` | 登录标识候选、密码证明、状态／代际、登录时间 | legacy来源登记、员工候选、短时credential票据、审计与最终context选择 |
| `routes/auth.rs::context` | 读取当前账号有效状态／代际证明 | 按明确user/member重验、组织链／来源准入并建platform_session及refresh |
| `routes/auth.rs::change_password` | 原密码证明、新密码策略／hash、锁后并发检查、提升账号版本及全局撤销 | CurrentUser权限、HTTP契约和平台审计事务 |
| `routes/users.rs` 创建／资料／启停／重置／删除 | 创建、启停与重置经账号port；基础资料UPDATE及账号DELETE暂保留原route内的受控同库存储适配，不操作PHC | system:user、目标租户及共享账号范围、自我保护、成员／角色操作、审计和通知；Phase12前收敛剩余全局账号写入口 |
| `routes/onboarding.rs` 联系人账号初始化 | 安全随机初始密码与账号创建命令 | 匹配／复用的组织范围、禁止按共享电话合并、EMPLOYEE成员及角色分配 |
| `routes/oauth_admin.rs` 全局kick | 通过账号管理命令提升权威版本并撤销该账号全部凭据 | 管理授权、目标scope、审计；单token／device撤销仍为平台局部操作 |
| `seed.rs` | 受控bootstrap内部适配创建账号、初始化凭证及已知默认凭证检查 | 仅空新库初始化、显式平台超管策略及平台结构／权限种子；不形成服务故障时的本地密码旁路 |
| `routes/oauth2.rs` | 密码grant调用账号认证；client凭证算法可复用 | authorization request/code、refresh/device消费、session_token及client/app/member proof |
| `oauth_issuance.rs` | 消费安全账号证明，不决定平台权限 | 统一构造ordinary OAuth的 `IssuanceIdentity`／`StoredBinding` 及事务内token pair；机器的账号／成员／session证明为空，仍绑定client及适用app策略版本；人员proof完整；app token由 `session_token` 的独立平台适配写入 |
| `session.rs`／`credential_binding.rs`／`app_token.rs` | 消费账号状态与代际 | 平台session／refresh／app权威、成员和来源／应用策略，局部撤销 |

现有 legacy 写入顺序可从 [`routes/oauth2.rs`](../../src/routes/oauth2.rs)、[`oauth_issuance.rs`](../../src/oauth_issuance.rs)、[`routes/device.rs`](../../src/routes/device.rs) 和 [`routes/oauth_admin.rs`](../../src/routes/oauth_admin.rs) 追踪：创建授权请求，确认后单次消费请求并生成code；code、refresh、approved device在兑换时单次消费；`oauth_issuance`在同一事务写access／refresh；device审批绑定当前员工session。client CRUD／secret轮换、逐token撤销、全局kick、device撤销、审计purge及过期维护都有明确平台owner，不能因它们分散于多个文件而给表分配两个服务owner。

消费者平台在 `platform_accounts.rs` 定义窄 `AuthenticationPort` 与 `AccountAdministrationPort`，同进程 provider处理密码与账号用例，外层明确的PostgreSQL事务适配器借用调用者的原事务。`AccountMutationTx` 以具名账号快照、创建、状态、版本与全局撤销操作表达同库事务能力，不使用 `Any`／downcast隐藏数据库依赖，也不在适配器中commit。平台仍保存自己的成员／策略proof。

认证port在同进程内返回的 `AccountCredential`、`CredentialHash`、`PreparedPassword` 是不实现Debug／Serialize的存储句柄，HTTP用例只把句柄交回port而不读取编码。其编码访问器目前是 `pub(crate)`，供provider和PostgreSQL适配器使用，**没有由类型系统对整个crate完全隔绝PHC访问**；这是当前内部约定及可审查调用边界，不能描述成密码材料已经从进程／crate迁走。外部DTO不返回编码、`is_super_admin`或成员权限，不能把内部句柄直接加入HTTP响应。

当前 `AccountFuture` 复用既有 `AppError`，该应用错误实现Axum `IntoResponse`。端口签名不暴露SQLx连接、Axum extractor或HTTP Request／Response对象，但本阶段同进程适配层仍共享应用错误映射，**不是完全无框架依赖的领域crate**。Phase12网络port需定义稳定失败code并在外层映射HTTP，不直接序列化 `AppError` 或数据库错误链。fake／契约验证应证明替换后用例仍执行账号前提、失败映射与撤销语义；只替换函数名或mock返回true不构成边界验证。

## 管理命令、授权和事务边界

账号管理 port 的消费者是平台管理用例，因此 **平台定义它所需的窄输入输出并执行管理员授权**；认证实现不能自行读取角色、组织、成员来决定管理员权限。平台在调用前保留 `system:user`／平台全局权限、租户范围、共享账号、不能禁用／删除自身等既有规则，不能把“认证模块能更新账号”当作授权。

同进程过渡保留账号先于session／凭据的锁顺序；平台管理类写入仍在最终锁之后复核其成员／管理范围，账号代际提升、密码／状态变化、全部旧凭据失效和平台审计／通知在受控事务内一致。不能据此笼统推定每个个人账号请求都已有提交前actor再验；现有个人改密的精确限制见下。`0014` 的账号保护trigger是当前平台凭据的数据库防线，不是独立认证服务已经持有平台表写权限的证据。

实际适配实现是 `authentication/postgres.rs::PostgresAccountTransaction`，它借用调用者连接；其中 `revoke_account` 仍会在同一事务写平台session和legacy token／code／device。它是当前账号全局命令与平台撤销的**同库过渡适配**，没有把这些物理表的逻辑owner转给独立认证服务。`session.rs` 仅保留 `cfg(test)` 兼容桥；生产入口通过账号管理port发起全局撤销。基础资料UPDATE、账号DELETE和seed内部适配仍按上表登记，不宣称所有 `users` SQL 已集中到provider。

### 个人改密的既有在途请求限制与 Phase 8 必闭环项

当前 `PUT /api/auth/password` 先由 `CurrentUser` 验证平台session，再读取账号并验证oldPassword，准备新hash，最后锁账号。`replace_password` 在锁后只比较主体ID与此前验证的hash，不比较原账号版本／状态，也没有调用 `session::require_active_in_tx`。Git HEAD中的原路由采用同样顺序与比较规则；Phase7保持此语义，不把它记作本次port抽离新增回归。

源码允许如下确定交错；这是只读分析，没有把未执行的并发实验写成已通过：

1. 请求A通过 `CurrentUser`，读取hash H并验证oldPassword，在新hash准备或等账号锁期间尚未进入最终写入。
2. 请求B先取得账号锁并提交全局ban、kick，或持账号／成员锁提交该成员撤销。这些操作使原凭据失效，但不改变密码hash H。
3. 请求A随后锁账号，仍匹配H，所以会完成改密并再次提升账号代际、撤销全部人员凭据。数据库trigger不会让这次在途UPDATE自动失败。

此操作**不签发或复活任何context／token，不把DISABLED账号改回ENABLED，也不恢复已撤销成员**；撤销完成后才发起的新请求会在 `CurrentUser` 处拒绝。若B是管理员密码重置并已改变hash，A锁后的hash比较会返回409，不能覆盖管理员重置。只有旧Bearer而不知道正确oldPassword也不能完成此流程。

oldPassword相等只证明密码材料没有变化，不能证明账号仍ENABLED或原平台context仍有效。特别是ban已提交时，不能把A继续完成称为“重新建立了有效账号认证”。账号仍有效的kick之后，用户可以通过明确的新密码认证建立新证明；当前在途请求却没有显式建立该新认证，也不能靠默读最新版本把旧context升级为新证明。

**Phase8必须闭环，Phase7不混入该行为变更：** 平台legacy个人改密应在取得账号锁后调用 `session::require_active_in_tx` 再验原平台session及成员；未来SSO自助命令则核验全局sid、账号状态与对应版本。ban／kick／成员撤销先提交时，依赖旧context的pending命令须拒绝；如需以密码重新认证，必须显式建立新认证，不能默补当前版本。应以受控并发屏障覆盖三类交错、管理员reset的409，以及有效独立全局session下无成员账号的自助能力。通用管理员reset仍须允许受授权地重置DISABLED账号，不能把平台成员条件直接塞进通用账号替换方法。

具体而言，`revoke_credentials_on_account_generation` 是默认invoker的PL/pgSQL函数，会跨写 `platform_sessions`、`platform_session_refresh_tokens`、`oauth_tokens`、code与device。未来认证角色如果仅获准写账号表，原样沿用它会因缺少平台写权限而失败；不能为省略拆分而给认证runtime授予全部平台表写权限，也不能静默停用保护。必须按下一段的权威代际／受控回收模型重新验证权限和撤销时序。

Phase 12 网络适配不能假装跨 HTTP 仍有 SQLx 本地事务。账号命令须有请求幂等键、预期账号版本和明确结果；账号 owner 原子完成状态／代际及全局 session 变化，并发布幂等撤销事实。平台凭据的每次使用、refresh、code／device兑换先比较实时账号权威代际，物理回收通过受控 outbox／消费者补齐，不能等消息投递才停止授权。平台范围检查与账号命令之间的并发变化必须以可验证命令前提或受控编排处理，不能扩大目标范围。

当前账号删除会因 `users` 外键 `ON DELETE CASCADE` 连带删除平台成员等数据。同进程的受授权事务可以保留该兼容行为；独立部署时不能直接授予认证 runtime 任意 DELETE 账号的能力并依靠级联修改平台授权数据。必须先完成平台解除／归档协议，或只提供账号 tombstone／全局停用并由平台 owner 回收关系，待受审查后再开放物理删除。

## 单一迁移与 Phase 12 过渡策略

当前唯一 canonical schema owner 是 `database::run_migrations` 所用的 `migrations/` 链；普通服务与 `platform-migrate` 共享同一真实迁移历史和锁。Phase 6 的 `ops/identity/` 收缩／receipt由显式 rollout 操作单独拥有，不是第二套可同时写 SQLx 历史的迁移器。Phase 7 不新增第二个身份服务迁移入口。

Phase 12 可保持同一个 PostgreSQL 数据库，但必须按以下顺序交接，而不是让两个服务启动时争抢混合表：

1. 固定主体 ID、账号状态／代际和账号命令协议，盘点全部读取、写入、数据库函数、trigger、FK、seed、CLI 和离线任务。
2. 从 `users` 的混合语义分离平台策略 `is_super_admin` 与认证私有密码凭证。表名可继续兼容，但认证凭证绝不出现在平台只读投影中；迁移需要逐项处理现有 FK 和函数，不能未经核验把原表替换成 view。
3. 为平台提供最小只读账号投影：主体、显示资料、全局状态／代际及必要时间字段；平台自身再 JOIN 平台策略和具体成员。认证服务不读取平台角色、菜单或数据授权。展示资料按现有管理范围过滤，不因为投影可读而允许全量对外枚举。
4. 认证 runtime 仅写认证账号／凭证／未来全局 session／OIDC私有表；平台 runtime 仅写成员、组织授权、平台策略、平台 session、app token 和 legacy OAuth。迁移角色与 runtime 分开，混合表过渡期使用明确列权限／受控适配而非整行双写。
5. 一个受控迁移 job 按 owner 顺序执行跨表演进，停用另一个服务中的同组迁移入口；保留已验证 checksum、UUIDv7、receipt 和 imported 交接约束。切换读投影／HTTP port之前用真实双身份及撤销回归证明行为不变。
6. 最后才按 Phase 12 确认的工程位置建立独立服务与部署角色。issuer、Cookie、网络／数据库故障、最小权限和恢复均需独立演练；不能把本页的规划标为已部署结果。

任何共享数据库角色、migration owner 或 projected view 都需要在 Phase 12 的实际部署 grant 清单中核实。本阶段只确立可执行代码边界和可审查的归属，不用新建 crate、复制业务实现或跨仓库链接平台 business crate 来替代 port。
