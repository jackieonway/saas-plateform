# Project Task Ledger

> 员工／客户身份与统一认证中心实施台账。版本 v1.7，2026-10-25。本身份功能使用新库、无历史数据；Phase 1～13 已完成，证据见[成员扩展](validation/phase-01.md)、[凭据绑定](validation/phase-02.md)、[员工隔离](validation/phase-03.md)、[通知隔离](validation/phase-04.md)、[管理界面隔离](validation/phase-05.md)、[客户最小能力／真实收缩](validation/phase-06.md)、[同进程认证边界](validation/phase-07.md)、[全局 SSO 会话](validation/phase-08.md)、[标准 OIDC](validation/phase-09.md)、[平台 BFF](validation/phase-10.md)、[独立客户门户](validation/phase-11.md)、[独立认证部署](validation/phase-12.md)与[灰度／容量／身份感知恢复](validation/phase-13.md)。当前指针为 DONE；Phase 13 按用户要求复用已通过子项、仅补测失败项，分轮结果和源码版本分别记录，不宣称同轮连续 35 分钟通过。创建、SSO 与 OIDC 开关默认仍关闭；仅在任务自有环境验证过显式启用，生产尚未开放。Phase 6 development 新库的原 manifest／receipt／二进制保持其历史验收范围。预览三库的保数据重编号由[UUID 专项台账](uuid-v7-task-ledger.md)独立管理，不作为本新库的历史搬迁前置。完整设计见[主方案](README.md)，建议详情见[开放问题建议](open-question-recommendations.md)。

## 1. Project Overview

### Project Name

统一账号、员工／客户组织身份与 SSO 分阶段演进。

### Project Goal

保留全平台统一的 `users`，在 `memberships` 区分员工和客户并实现身份隔离；先在现有平台进程内建立认证与组织授权的模块边界，再提供标准 OIDC，最终将认证中心独立部署。平台、客户门户和后续应用复用账号认证，各自验证组织身份和业务准入。

按最新范围，本身份新库不涉及历史账号分类、重复账号处理、旧→新 ID 映射、旧凭据迁移、历史通知回填或保数据导入。Phase 1 首先验收相关 v7 代码、已有迁移链与 seed 在空库上运行，记录真实基线证据，再实施成员与准入扩展。新生成的 `users.id` 此后作为稳定全局主体；不按手机号或邮箱合并账号。

原 v0.3 的本地预览三库 UUID 保数据专项继续独立保留其授权范围、状态与证据。本范围调整不取消该专项、不将 UUID-U03～U06 标为完成，也不把预览源库或转换目标当作本身份新库。

### Background

- 工作区包含六个独立 Git 仓库，不是统一 Cargo workspace；平台实际目录为 `saas-plateform`。本次台账位置是平台后端 `docs/identity-sso/`。
- 现有 `users.id` 是全局主体；组织成员、角色、数据授权、平台会话、应用 token、legacy OAuth 和通知共同形成当前权限链。增加一个成员类型字段，需要同步修改这些资格校验。
- 当前登录依赖有效组织成员，OAuth 已具备部分授权码、PKCE、设备码、内省和撤销能力，尚不能据此视为完整 OIDC 认证中心。
- `saas-customer-portal` 已在 Phase 11 创建为同级独立 Git 仓库，含 backend／frontend；`saas-identity` 已在 Phase 12 创建为同级独立包与部署单元，不跨仓链接业务 crate。

### Success Criteria

- 同一全局账号可在同一组织同时拥有员工和客户两条 `memberships`，每种类型最多一条，分别拥有独立的 ID、状态和版本；不同组织的身份也保持独立。
- 客户无法继承员工角色、超级管理员能力、数据授权、员工目录资格和员工通知；客户只能访问登记的客户入口及明确归属于自己的数据。
- 成员启停、移除和身份替换立即阻止相关旧凭据继续访问；同组织另一身份及其他组织身份保持有效。账号封禁、全局注销和密码重置使全部关联认证凭据失效。
- 无成员关系的合法账号可以完成 SSO 账号认证，但不能因此获得平台上下文或业务访问权。
- OIDC 授权码加 S256 PKCE、ID Token、发现文档、独立非对称签名与 JWKS、BFF 会话和注销行为通过契约与端到端验证。
- 平台与认证中心可过渡共用数据库，但账号认证、组织授权、混合 legacy 数据均有单一写入和迁移负责人。
- 上线切流、密钥轮换、权威状态校验不可用和恢复均有可复现证据；协议消费者清单明确，实际存在旧客户端时再验收退役条件。新库无历史客户端迁移窗口不等于可跳过活动 SDK／业务服务兼容；任何状态尚未验证时不宣称上线完成。

---

## 2. Global Constraints

### Technical Stack

- 延用各仓库已有 Rust、Axum、Tokio、SQLx、PostgreSQL、React/Vite 与 qiankun 技术基线，版本由各仓库锁文件决定。
- Rust 包、前端和数据库集成测试分别在所属仓库运行；数据库集成测试的 `ignore` 标记意味着普通 `cargo test` 不等于执行了数据库验收。
- SSO 使用标准 OIDC Authorization Code Flow 与 `S256` PKCE；浏览器通过服务端 BFF 持有业务登录会话，Cookie 使用 `HttpOnly`、`Secure` 和与部署拓扑匹配的 `SameSite`，写接口具备 CSRF 防护。

### Architecture Constraints

- 账号认证、平台上下文、应用 token 分三层：SSO 全局 session → 平台 context session → app token；ID Token 不作为业务 API 访问凭据。
- 平台 context 的 `origin_client_ref` 保存来源 issuer/client，应用 token 继承该准入；`application_service_client_ref` 另存应用后端服务凭据，不能用现有 app token 的 cid 覆盖来源登录 client。用户用途应用客户端可进一步收窄，机器 service client 不能扩大来源准入。
- 本身份新库的 `users.id` 从创建起使用 UUID v7，之后为稳定全局主体和 OIDC `sub`；无历史主体映射或外部 RP 旧主体协同任务，不按手机号或邮箱合并账号。预览库存量主体切换仅适用独立 UUID 专项。
- 系统自有主键、UUID 引用、jti、revision、request ID 和暂存标识统一 v7；PG17 基线下应用显式写 ID、追加移除 v4 默认的迁移。密钥、code、refresh、PKCE 等安全随机材料维持 CSPRNG，密码哈希及随机退避保持适当的现有要求，不以 v7 替代随机秘密。
- `memberships` 增加 `member_type`（`EMPLOYEE`／`CUSTOMER`）、`status`（`ENABLED`／`DISABLED`）、`identity_version >= 1` 和 `updated_at`；最终唯一键为 `UNIQUE(user_id, organization_id, member_type)`。阶段 1 先加三元唯一键并保留旧二元唯一约束，阶段 6 安全代码覆盖全部实例／写入器后，由独立收缩增量迁移删除旧二元约束。新库无旧 binary 时记录清退不适用，仍保留分阶段开关与迁移 fixture 验证。同账号同组织的两种身份最终使用两行独立记录，不建立员工／客户身份表。
- `member_type` 是单条成员关系创建后不可原地改写的类别。普通登录身份切换选择另一个有效 `membership_id`，不写成员类型、不创建身份；每个请求仅使用一个确定的 context。
- 所有原来按 `user_id + organization_id` 定位成员的查询、JOIN、`ON CONFLICT`、DELETE、触发器和广播均须审查，改为按 `membership_id` 或显式 `member_type` 定位；不能取首条或按账号合并两种身份。
- 人类登录 OAuth／OIDC 客户端与应用都具有 `allowed_member_types TEXT[]` 及递增的策略版本；约束为 `EMPLOYEE`／`CUSTOMER` 的非空子集。既有普通用户客户端、应用默认仅 `EMPLOYEE`，专用客户门户客户端为 `CUSTOMER`；同时包含两者仅允许选择已有的相应身份，不自动创建成员关系。不得复用表示 PUBLIC／CONFIDENTIAL 的 `clientType`。机器客户端明确区分主体用途，其空成员集合仅表示不适用成员身份，绝不表示无限制。
- 客户端用 `subject_mode=USER / MACHINE / LEGACY_MIXED` 明确约束：USER 的身份集合非空；MACHINE 集合为空且只允许机器 grant；LEGACY_MIXED 仅用于登记的旧混合客户端，不允许新建。应用身份集合始终非空。策略版本字段统一命名 `access_policy_version`，API 为 `accessPolicyVersion`。
- `users.is_super_admin` 当前属于平台授权历史字段；后续逻辑归平台授权边界，不放入通用 ID Token 或由 SSO 向客户端传播。
- 先通过同一 crate 内模块与消费者定义的 port 分离职责，有独立部署需求再拆服务；服务间仅通过明确协议，不跨仓库依赖对方业务实现 crate。
- 同库过渡不等于共享写权限：每类表和每份迁移只有一个 owner，使用独立数据库角色和必要的只读投影。逐类处置混合 `oauth_tokens`／`oauth_clients`，不得按 URL 前缀把 `/oauth/*` 整体搬走。
- OIDC issuer 从同进程服务阶段就采用最终稳定的公开身份地址；拆部署通过网关改变上游，不能偷偷改变 issuer。

### Business Constraints

- 客户必须关联真实存在且属于该租户的组织。首版仅管理员创建／绑定既有账号，不提供客户自助注册，不自动挂租户根组织、不自动创建组织。
- 管理员绑定账号必须使用可授权的精确账号标识，完成获授权的邀请或账号归属确认并记录绑定来源，不能通过手机号、邮箱猜测或自动合并主体。跨组织分别创建独立客户成员；本新库无历史生产客户归类任务，新建时显式指定类型和真实组织。
- 客户不分配角色和员工数据授权；客户规则显式核验租户、组织、当前成员身份及记录归属。首版客户能力仅为 profile/context 与本人基础资料，不编造订单、工单等尚未确定的客户业务。
- 既有业务应用默认仅允许 `EMPLOYEE`；只有明确登记客户用途、接口及准入条件的客户端或应用才允许客户。
- 实际业务准入取当前有效成员类型与客户端允许集合、应用允许集合的交集；客户端不能扩大应用范围，应用也不能扩大租户／组织范围。登录客户端必须精确绑定用途，不能挑选创建时间最早的客户端。
- 成员类型是当前组织关系的属性，不能通过“是否有角色”推断，也不能把用户全局 `user_type` 当成替代。
- “新增第二身份”和“替换旧身份”是两种显式管理操作。业务上的“员工转客户”是创建客户关系或显式启用已有客户关系，再停用员工关系的可审计组合；不能复用旧员工 ID，不重复创建已存在的同类型关系，不自动恢复目标身份的旧授权／会话。
- 员工离职只停用对应 `EMPLOYEE` 行，不影响同组织 `CUSTOMER` 行；反向同理。客户记录 owner 与员工操作 actor 都绑定具体 `membership_id`，不能因账号相同而混用。

### Security / Compliance Constraints

- 当前会话为客户类型时，门禁先于祖先组织角色继承、超级管理员判断、应用入口、目录和数据授权；不能因为同账号存在员工行就让客户 context 继承角色，也不能因为账号有客户行而阻止有效员工 context。前端隐藏菜单不构成授权。
- 状态变更、移除与身份替换在同一数据库事务内递增受影响成员的 `identity_version` 并撤销其平台 session、refresh、app token、legacy OAuth token、授权码和设备流程凭据；删除关系也必须保留等效撤销效果。同组织另一行不能按 `user_id + organization_id` 被误撤。
- 角色授权定位有效 `EMPLOYEE membership_id`；客户查询不得附带同账号员工角色。`user_roles` 可保留现有用户／祖先组织角色语义，但必须由有效员工关系证明资格，客户行不能替代资格。临时停用可保留角色配置，但读取资格立即失效；永久退出、删除或替换时仅清理不再由结尾有效员工集合支持的绑定，不能按用户全删或把所有客户账号设为无角色。
- 员工批量编辑不得覆盖或删除客户；旧请求缺少类型时只操作员工行。同组织仅有客户时，可在开关允许后新增独立员工行；存在已停用员工行时不得通过旧请求自动启用。`ON CONFLICT` 按三元键处理，重复创建同类型返回 `409`。旧 schema 的员工默认值仅用于迁移／兼容 fixture，不构成本新库的历史分类任务。
- 授权读取需要实时核验权威账号／会话／成员／版本状态。权威状态不可用时返回 `503` 且不执行业务；不能只验 JWT 签名或依赖异步事件替代立即撤销。鉴权失败与依赖不可用分开表达。
- 所有被接受的用户 access／refresh／code／device 及选择票据绑定账号 `account_version`（权威来源 users.session_version）和适用的具体成员 ID／版本；账号 owner 原子提升版本使兼容协议凭据也即时失效。无法证明绑定或无法在同组织两行中唯一定位的旧格式凭据必须拒绝并要求重认证，以隔离 fixture 验证，不执行真实历史凭据迁移、不猜测首条、不按当前版本补绑定复活旧凭据。跨 owner 的物理撤销清理由可靠事件完成，不代替权威版本拒绝。
- 首版客户通知建议保持关闭；员工通知与客户隔离，缺省广播仅限有效员工。列表、详情、已读、发送目标和实时连接都必须核验当前成员及身份版本；历史无身份标记场景用 fixture 验证安全默认，本新库无实际历史通知回填。
- 全局封禁、全局注销、密码重置覆盖 legacy 平台、应用、OAuth、refresh、code、device 和新 SSO 关联凭据；成员撤销只作用对应 `membership_id`，包括同组织员工／客户并存时。
- 应用或客户端的允许身份策略收紧时，同事务递增策略版本并使不再合规的 context／token／code／refresh／device 失效；已登录的全局 SSO session 不因某应用准入变化而全局注销。把客户加入允许集合仅通过入口准入，不能替代具体资源、动作和 owner 归属校验。
- SSO 不查成员来选择撤销 OIDC code：注册客户端用途策略改变后，同 client 旧 code／refresh／grant 按版本统一失效，经有效全局 SSO 重新授权；保留其他 client。策略 owner 原子提交版本和审计，使用方读权威状态即时拒绝，跨 owner 清理异步完成。
- 秘钥、Cookie、code、refresh、原始 token 和密码不写入日志或测试证据；审计记录操作者、主体、成员、上下文、变更原因和撤销范围。

### Code Style / Engineering Rules

- 遵守上级 `AGENTS.md` 及各仓库局部规则，保留薄入口、厚库和依赖单向；不为目录整齐重构无关模块，不提前创建通用共享 crate。
- 历史 SDK 2.0.0 等发行字节和摘要保持不变；本新库活动 SDK 与消费者须满足 v7 生成／输入和合同验收，复用专项中适用的实际产物，不能以预览专项未来发布计划代替验收。需要升级时同步版本允许列表、生成 DTO、来源摘要和校验。MicroProps 2、Context 1、Notification 1 继续保持对应线协议含义。新客户先用独立 portal DTO，内部身份绑定不能静默修改旧 wire contract。
- 数据库使用增量迁移，不改历史迁移；迁移、序列化、公开协议和撤销都需相称的兼容性／回归验证。
- 阶段 2～5 的双身份 DB 用例使用专用目标 schema 隔离 fixture，在测试准备中显式移除旧二元约束以创建两行；这不是发布收缩迁移，不得作用真实环境或默认扩展 schema。阶段 6 另外验收真实收缩迁移与全实例升级顺序。
- 每阶段交付最小充分检查、失败与边界验证及 `validation/phase-NN.md`。已有[空库基线](validation/fresh-v7-baseline.md)及 Phase 1～9 验收证据；Phase 10～13 的证据按实际执行补齐，记录真实命令、结果、限制和实际改动仓库。
- 现有仓库命令优先；平台后端至少按变更运行 `cargo fmt --all -- --check`、`cargo check --locked --all-targets`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --locked --all-targets` 和 `cargo doc --locked --no-deps`，涉及数据库的阶段另显式执行隔离库测试。其他仓库使用各自门禁。
- 本台账保留原 13 个身份阶段编号；六阶段 UUID 保数据专项独立管理预览库存量转换，不是本身份新库的搬迁前置。分别验收通知隔离、SSO 会话、OIDC provider、两个客户端及独立部署，两份台账各自维护执行指针与证据，不互相冒领完成状态。

### Fresh Database UUID v7 Baseline

Phase 1 已完成本身份新库基线验收，证据见 [validation/fresh-v7-baseline.md](validation/fresh-v7-baseline.md)。已真实验证下列要求；该结论不代替后续身份隔离和独立 UUID 专项的验收：

- 明确登记隔离的新目标库，从空库执行当前历史迁移链与 seed；不读取、转换或导入预览库旧主体，不修改历史 migration 字节。
- 最终 schema 不依赖 v4 默认，系统自有主键、引用、生产发号入口、seed 和活动 SDK／消费者使用 v7；typed UUID 入口拒绝不合法版本和 variant。历史发行物与 v4 负例保留其原义。
- 外键、自然键、生成／解析入口、活动 SDK 与门店／工单服务协议有实际验证；secret、code、refresh、nonce 等安全随机材料维持 CSPRNG，密码哈希维持现有安全要求。
- 新环境从空会话开始，测试账号／成员均为明确标注的 fixtures；成员扩展／收缩迁移、双身份与旧接口语义仍须验证。新库无历史实例时，旧 binary 清退和实际历史回填登记为本范围不适用。

### Recorded Recommendations and Deferred Production Inputs

下表记录实施建议，不代表已签发客户端、配置域名、指派维护人或达到运维指标。详细合同见[开放问题建议](open-question-recommendations.md)。真实生产值仅在受影响的接入／部署阶段确定，不阻止前期隔离实现。

| 项目 | 已记录建议 | 实际使用前需落实 |
|---|---|---|
| 客户范围 | 管理员显式选择租户内有效组织；跨组织分别建 CUSTOMER；首版只做本人资料、context 和已登记客户应用目录，无客户 RBAC | 创建时检查账号绑定依据、组织和管理范围；真实客户业务另定资源／动作合同 |
| 开发 OIDC | issuer `https://auth.saas.test`；控制台／门户分别为 `https://console.saas.test`、`https://customer.saas.test`；配置别名 `platform-console-dev`／`customer-portal-dev` | hosts、受信任开发证书、HTTPS 网关、实际签发 client ID；生产采用自有主域并分别注册 |
| 回调与 Cookie | 各门户精确 `/auth/callback`，post-logout 为各自 `/signed-out`；独立 confidential USER client，仅 EMPLOYEE／CUSTOMER；host-only `__Host-` Cookie、Secure／HttpOnly／Lax 与 CSRF | 分环境精确 URI 白名单；不将别名写成已创建 client；同环境拆部署保持 issuer |
| grant 与兼容 | 新 USER client 默认 code＋S256，refresh 按 BFF 需要显式开启；password／device 默认关闭，机器 grant 独立 | 活动 SDK／应用／机器服务消费者清单和合同测试；有真实旧消费者时再确定退役窗口 |
| 隔离容量 | 100 并发会话；认证流程合计 10 RPS、权威鉴权 100 RPS，30 分钟，2 倍突发 5 分钟；权威鉴权 p95≤100ms／p99≤300ms，换码／刷新 p95≤500ms／p99≤1s | 记录硬件、Argon2 参数和实测；密码认证单独统计，p95 候选≤1s；不将假设写成已通过 |
| 超时与 SLO | 业务鉴权总预算 3s、连接最多 1s；新 SSO 换码／刷新候选 3s、不自动重试；生产可用性候选滚动 30 天 99.9% | 权威状态不可确认返回 503；明确 SLI 口径并计入依赖故障，实测后确认生产承诺 |
| 恢复 | 内部试运行候选 RPO≤24h／RTO≤4h、每 12h 协调加密备份、14 天＋8 周保留及月度演练；生产候选 RPO≤15min／RTO≤60min | 现有 dump 工具无 WAL/PITR 证明；生产目标需新增持续 WAL／PITR 和密钥／一致恢复验证，登记具体负责人 |
| 工程 | Phase 7～10 平台内认证模块与 port；Phase 11 同级独立 `saas-customer-portal`，Phase 12 同级独立 `saas-identity` | 创建前核对实际路径与维护归属；门户 backend/frontend；不跨仓依赖业务 crate |
| 通知 | 首版客户通知关闭，缺省广播仅员工；绑定 recipient_membership_id／类型／版本，旧代仅保留审计 | Phase 4 实施所有收件／实时通道隔离；后续客户事件、路由和归档另定 |

### Forbidden Actions

- 每次按执行指针完成一个身份阶段及其验证，不提前实施后续 UI、OIDC、门户或独立服务，不把方案或文档更新计为实现完成。用户后续“继续执行”已用于本次 Phase 1 的代码与隔离数据库验收，原先“先更新文档”的交付限制不再适用。
- 阶段 1～5 的新生命周期写入口只在隔离环境验证，正式环境新增的成员启停、新增第二身份和替换身份功能保持关闭；完整身份、状态、版本及双身份查询安全基线通过后才启用。开关覆盖创建、替换、导入、CLI、种子等全部产生客户或同组织第二行的路径。
- 不创建客户角色、不复用员工管理员权限给客户、不把客户加入员工候选、不默认向客户开放旧业务应用。
- 不在客户或双身份数据已存在时回滚至不识别成员类型、仍假设 `user_id + organization_id` 唯一的旧版本；不以删除类型字段、还原二元唯一键或恢复旧广播语义作为恢复方案。
- 不强迫用户一次回答全部问题；每项疑问在影响其阶段的操作前解决，前期可使用明确标注的隔离测试数据继续。
- 不静默将 legacy refresh 换成 OIDC refresh，不把平台上下文 JWT 当作 SSO 全局 session，不将 ID Token 当作 app token。

---

## 3. Phase Status Summary

| Phase | Name | Status | Depends On | Output Summary |
|---|---|---|---|---|
| 1 | 成员 schema 与显式生命周期 API | COMPLETED | 空库 v7 基线（已通过） | [空库基线](validation/fresh-v7-baseline.md)、0013 扩展／双约束、生命周期 API／真实原因审计、员工兼容守卫及默认关闭开关；[验收](validation/phase-01.md) |
| 2 | 平台 context 与既有凭据绑定身份版本 | COMPLETED | 1 | 0014、完整成员／账号／来源／策略证明、精确撤销、准入管理 API／CLI 与 [验收](validation/phase-02.md) |
| 3 | 员工授权、目录及双向授权入口隔离 | COMPLETED | 2 | 精确员工资格、角色／目录／数据授权、bulk 与最终支持清理、跨仓候选及[验收](validation/phase-03.md) |
| 4 | 通知受众、历史隔离与实时撤权 | COMPLETED | 3 | 0015 具体收件成员／代际、HTTP／偏好隔离、私有实时证明、真实双服务撤权及[验收](validation/phase-04.md) |
| 5 | 平台管理 UI 与缓存会话隔离 | COMPLETED | 4 | 具体成员分视图／生命周期、完整 proof／跨页缓存和准入编辑；[验收](validation/phase-05.md) |
| 6 | 客户最小 API 与应用准入策略 | COMPLETED | 5 | 独立客户 DTO／登记准入、真实独立收缩、环境 receipt 和 development HTTP 验收；[证据](validation/phase-06.md) |
| 7 | 同进程认证模块、账号管理 port 与存储 owner | COMPLETED | 6 | 消费者 port／同库原事务适配、明确存储 owner；194 普通、178 既有＋4 新 PG、启动验收及[证据](validation/phase-07.md) |
| 8 | SSO 全局 session 与登录 Cookie | COMPLETED | 7 | 0016、成员独立账号会话、Cookie／CSRF、全局撤销与 legacy 改密再验；[证据](validation/phase-08.md) |
| 9 | 标准 OIDC provider | COMPLETED | 8 | 0017、code＋S256／ID Token／发现／JWKS／独立 RS256 轮换、用途／策略及真实标准 RP；[证据](validation/phase-09.md) |
| 10 | 平台 BFF OIDC 客户端与 legacy 桥 | COMPLETED | 9 | 0018、服务端凭据／明确员工选择、内存短 Bearer、真实 qiankun／通知续期撤权、兼容及恢复；[验收](validation/phase-10.md) |
| 11 | 独立客户门户 BFF 与本人资料 | COMPLETED | 10 | 独立门户／0019 客户资源 port、服务端凭据、明确客户选择与本人资料；真实双 RP／双向撤销及[验收](validation/phase-11.md) |
| 12 | SSO 独立部署与同库权限治理 | COMPLETED | 11 | 独立 saas-identity／0020、网络 port、单一迁移与最小角色；221 协议、214 浏览器检查、完整共享库恢复及[验收](validation/phase-12.md) |
| 13 | 多客户端灰度、恢复与退役验收 | COMPLETED | 12 | 分轮 steady 1800s／burst 300s、221 协议／15 灰度、140 恢复、343 初始／247 恢复浏览器及消费者合同通过；[验收](validation/phase-13.md) |

允许的阶段状态仅为 `NOT_STARTED`、`IN_PROGRESS`、`BLOCKED`、`COMPLETED`。执行时只推进当前指针阶段；阶段通过验收并记录证据后再更新状态与指针。

预览三库 UUID-U01～UUID-U06 的真实状态、依赖和交付仅以[UUID 专项台账](uuid-v7-task-ledger.md)及其证据为准，本台账不复制其动态指针。本新库已完成 Phase 1，不依赖历史搬迁；本次只推进身份阶段状态，不改写 UUID 专项状态。

---

## 4. Phase Details

### Phase 1: 成员 schema 与显式生命周期 API

#### Status
COMPLETED

#### Goal
在现有成员关系表达同组织双身份、独立启停和版本，建立受控的成员生命周期入口，同时保持客户及第二身份创建默认关闭。

#### Scope
`memberships` 增量迁移及三元唯一键、数据库约束、内部枚举与兼容 DTO、显式创建／绑定／启停／新增第二身份／替换旧身份／移除用例、审计和创建开关；应用与人类登录客户端的允许身份集合及策略版本 schema。

#### Out of Scope
开放生产客户创建、客户自助注册、SSO 服务、角色 UI 和客户业务页面。

#### Inputs
本台账全局约束、完整方案、平台 migrations 与成员管理路由、管理审计服务、账号和组织模型；阶段执行时重新核对真实 schema。

首项验收并形成 `validation/fresh-v7-baseline.md`：空库真实迁移与 seed、v7 生成／输入、FK／自然键及活动消费者合同。没有历史映射、外部旧主体协同或保数据导入前置；不依赖 UUID-U06。原 v4 用户／成员／应用 ID 不可作为本新库的活动参数。

#### Execution Requirements
- 新增 `member_type`、`status`、`identity_version`、`updated_at` 和三元唯一键，暂时保留旧二元唯一约束。新库无实际存量回填；迁移 fixture 中的既有员工语义、ID 和授权引用仍须保留。阶段 1 只提交扩展迁移，不能把删除旧约束的收缩迁移提前交给启动时 auto migrate。
- 迁移前预检引用二元唯一键的外键、所有 `ON CONFLICT`／JOIN／DELETE／触发器／广播及导入路径。新代码改用三元 `ON CONFLICT`、具体成员 JOIN 与局部撤销，在双约束期间兼容运行；全实例／写入器部署安全代码，存在旧 binary 才执行清退，新库没有则记录不适用。阶段 6 再执行独立收缩增量迁移并允许第二行。
- 禁止原地更新 `member_type`；提供独立的第二身份创建与身份替换用例，替换拟使用 `POST /api/memberships/{id}/replace-identity`，不提供 change-type 原地更新。目标同类型行已存在时只允许显式启用／复用该行，不再创建；停用目标须明确启用权限，递增版本且不自动恢复旧授权／会话。替换记录目标行和源行并停用源行，普通新增第二身份不停止源行。
- 为应用和人类登录客户端增加 `allowed_member_types TEXT[]` 与策略版本，非空且只能包含已知成员类型；历史普通用户客户端与应用安全默认 `EMPLOYEE`。机器客户端按明确用途区分，空集合为不适用，不能理解为放开。
- 同步实现主体模式的条件约束与迁移清单；新客户端禁止 LEGACY_MIXED，策略字段使用 `access_policy_version`。新生命周期写接口在正式环境保持关闭，避免阶段 2 前旧鉴权忽略新 status 后重新发放凭据。
- 使用显式生命周期用例集中变更；所有新增写请求必填实际变更原因 `reason`，trim 后非空、最多 500 字符且无控制字符。状态变更／替换在同一事务中对受影响行递增版本、审计和撤销现有凭据，保留同组织另一行。身份版本绑定在阶段 2 完善，在此之前客户与第二身份创建开关持续关闭。
- 校验真实组织、所属租户、操作者管理范围、已存在账号及并发版本。成员查询与写入使用 `membership_id` 或显式类型，不得由默认 DTO、首条成员或二元键定位意外改写另一身份。

#### Expected Outputs
[0013 扩展迁移](../../migrations/0013_membership_identity_policy.sql)、[成员生命周期模块](../../src/membership_lifecycle/mod.rs)和管理 API、默认关闭的配置、[API／迁移说明](phase-01-api.md)、[成员写路径及二元引用清单](phase-01-membership-path-audit.md)、[空库基线](validation/fresh-v7-baseline.md)与[阶段验收](validation/phase-01.md)。旧人类凭据入口仅接受有效员工，完整身份与版本绑定仍由 Phase 2 完成。

#### Acceptance Criteria
- [x] 空库 v7 基线有真实证据，隔离库全新安装和既有 schema／员工 fixture 扩展升级成功；fixture 员工 ID／授权保留，三元唯一键与旧二元唯一约束同时存在；非法类型／状态／版本被拒绝，重复创建及并发竞态返回确定的 `409`，第二行暂被开关和旧约束阻止。
- [x] 应用和客户端允许集合非法值、空的人类准入集合及非法策略版本被拒绝；迁移不会把普通旧客户端开放给客户。
- [x] 对成员增删改、第二身份创建、替换、目标已存在／停用、错误租户和并发变更有可复现验证；客户及第二身份创建默认被拒绝，类型原地修改被拒绝。
- [x] 创建、替换、绑定、批量、CLI、导入等全部产生 CUSTOMER 或同组织第二行的路径都受开关限制；完整基线完成前正式环境的新启停／替换入口关闭。
- [x] 旧无类型请求只作用员工且已有停用员工不自动启用；扩展期新增第二行明确拒绝。独立目标 schema fixture 验证仅有客户时可新增员工独立行且不误写客户，实际开放仍留到阶段 6。
- [x] 已提供二元键查询与引用审查清单，扩展迁移后新 `ON CONFLICT` 按三元键运行，旧接口语义通过 fixture 验证；无旧部署时清退不适用。新生命周期与员工写入不以首条／多行 JOIN 定位目标；其他凭据、授权与通知读取按清单继续由阶段 2～4 迁移，收缩明确留到阶段 6。

#### Risks
遗留写路径绕过生命周期服务；二元键 JOIN／冲突处理产生歧义；旧外键依赖被删除的唯一约束；第二行先于安全代码出现；默认值掩盖新建成员的显式类型要求。

#### Dependencies
无历史数据搬迁前置。空库 v7 基线是本阶段首项，必须真实验收后再推进成员扩展并与本阶段一并交付证据；不依赖 UUID-U06。Q1／Q11 已按新库范围解决；Q2 的组织规则已有建议，测试使用明确的租户／组织与 v7 成员 fixture，真实创建时必须校验组织归属和绑定依据。

#### Notes
本阶段已完成，验收六项均 PASS，完整检查、首轮失败与修复后的重跑见[阶段证据](validation/phase-01.md)。Phase 1 验收时所有新生命周期写入口只允许隔离 `APP_ENV=test` 开启，正式环境默认关闭；现行非 test 门禁与任务开发库验收见[Phase 6](validation/phase-06.md)。旧人类凭据统一要求 ENABLED EMPLOYEE；暂以审计标记的 EMPLOYEE_COMPATIBILITY 撤销员工适用旧凭据，客户变更不误撤员工。精确 membership／account／client／app 版本绑定留在 Phase 2，不冒领完整双身份凭据链。按稳定顺序锁操作者与目标账号，再锁成员；锁等待后及提交前复核具体 session、实时权限、组织／租户及到期指纹。当前上下文员工的自行启停／删除／替换返回 403，由其他管理员执行。Phase 6 删除旧约束前仍须确认所有实际写入器具备完整安全基线；本新库无旧实例清退不适用。

---

### Phase 2: 平台 context 与既有凭据绑定身份版本

#### Status
COMPLETED

#### Goal
让所有既有授权载体绑定实际成员与身份版本，并在成员变更后立即失效。

#### Scope
平台登录上下文、组织／成员身份选择与切换、session／refresh／app token、legacy OAuth token、授权码和设备流程的具体成员绑定、内省与撤销；客户端／应用准入策略管理 API、CLI／seed 兼容与策略变更事务。

#### Out of Scope
新 OIDC provider、全局 SSO Cookie、改变已发布 SDK wire contract、客户业务接口。

#### Inputs
阶段 1 模型与生命周期用例、平台 `session`、`auth`、`oauth`、应用授权实现、完整方案中的凭据与撤销矩阵。

双身份测试输入另含专用目标 schema fixture：仅在隔离测试准备时去掉旧二元约束，不能提前发布阶段 6 收缩迁移。

#### Execution Requirements
- 把 `membership_id`、类型和 `identity_version` 绑定到服务器权威凭据记录；同账号同组织的员工和客户分别建立 context，不能仅凭 user／org 恢复上下文。新签发凭据都须完整绑定；旧格式／歧义凭据用隔离 fixture 验证拒绝与重认证，不导入或补绑真实历史 token，不能仅以 JWT 声明或首条成员为准。
- 同时绑定全局账号版本及原认证证明版本；对不能证明历史绑定的用户凭据安排失效重授权，不能读取最新版本后把旧凭据静默升级。账号版本比较覆盖所有使用／兑换路径。
- 逐条覆盖发放、刷新、切组织、兑换授权码、设备批准／轮询、内省、撤销和业务校验，不遗漏祖先组织或 app 派生路径。
- 成员启停／移除／替换事务同时递增受影响行版本并撤销关联凭据；删除关系按成员标识定位凭据。保留同组织另一身份和其他组织的会话，不能按 `user_id` 或 user＋org 粗暴注销。普通切换只选另一个有效 `membership_id`，不改变成员类型或状态。
- 权威状态不可用返回 `503`；对失效成员返回对应未授权／禁止状态。异步缓存失效和通知只能辅助权威判断。
- 凭据记录绑定适用的客户端／应用策略版本；策略收紧与不合规凭据撤销采用与成员变更一致的原子性和并发保障。
- 分别验证默认扩展 schema 的兼容性与专用目标 schema fixture 的同组织双身份撤销；fixture 去约束仅属于测试准备，正式第二行开关仍关闭。
- 完成应用及 OAuth 客户端 allowedMemberTypes／accessPolicyVersion 管理 API、DTO、CLI 与 seed 兼容；精确来源 client 与应用服务凭据分开绑定。阶段 5 使用这些已验收接口，阶段 6 仅完成准入消费和整体开放门禁。

#### Expected Outputs
身份感知 context／凭据模型、事务撤销实现、准入策略读写 API 与 CLI／seed 适配、legacy 绑定兼容方案、撤销矩阵用例、`validation/phase-02.md`。

#### Acceptance Criteria
- [x] 替换员工身份、停用和移除后，受影响成员变更前 session、refresh、app token、OAuth token、code 与 device 全部无法继续取得或使用对应权限。
- [x] 同组织双身份同时在线时，停用员工不误踢客户，停用客户不误踢员工；同账号其他组织也保持可用，伪造成员 ID／类型／组织／版本不能切换身份。
- [x] 旧凭据无法在同组织两行中唯一定位时失效重认证，不猜测或补绑；普通切换不修改数据库成员类别，也不会合并两行权限。
- [x] 权威存储故障返回 `503` 且业务处理未发生；旧格式员工 token 兼容／拒绝 fixture 及固定 SDK 契约验证通过，不以迁移真实历史 token 为输入。
- [x] 应用／客户端移除某身份类型后，对应旧 context、token、code、refresh 和 device 不能继续访问或兑换；其他未变更客户端／应用的身份不受影响，收紧 client 内仍合规账号可经 SSO 重新授权。
- [x] 阶段 5 所需的策略列表、读取、保存、并发冲突与收紧 API 已通过验证；来源客户端不会被 service client 覆盖。

#### Risks
混合 OAuth 表中的用途判断不完整；旧凭据只有 user／org 导致成员歧义；刷新竞态产生未撤销的新 token；局部撤销错误影响同组织另一身份。

#### Dependencies
阶段 1 完成；本阶段所有安全检查完成前保持客户创建关闭。

#### Notes
本阶段六项标准均 PASS，见[Phase 2 验收](validation/phase-02.md)。交付 0014、明确 legacy 来源注册、账号／成员／策略完整证明、精确撤销和管理 API／CLI；185 普通测试、151 数据库测试（150 全量＋1 定点）、58 CLI 与固定 SDK 合同均有通过证据。旧凭据不补绑；双唯一约束和真实创建关闭门禁保留。SDK 3.0.0 固定产物已核验，Context 1／MicroProps 2／Notification 1 及历史 2.0.0 字节不改。

---

### Phase 3: 员工授权、目录及双向授权入口隔离

#### Status
COMPLETED

#### Goal
客户无法进入员工 RBAC、数据授权、组织员工目录和处理人候选，员工管理不会误删或提升客户身份。

#### Scope
角色和祖先组织继承、超级管理员门禁、成员／角色双向管理、数据授权读写、组织批量成员编辑、受控应用成员目录和候选资格校验。

#### Out of Scope
客户 RBAC、新客户角色、业务处理人 UI 的无关改版、SSO 协议。

#### Inputs
阶段 2 context、`users.is_super_admin`、成员／角色／数据授权路由、应用成员目录及门店与工单的候选接口调用链。

同组织双身份验证使用专用目标 schema 隔离 fixture；仅测试准备显式去掉旧二元约束，不发布实际收缩迁移。

#### Execution Requirements
- 先检查当前 context 绑定的成员为有效 `EMPLOYEE`，再执行角色、祖先权限或超级管理员逻辑；同账号在同组织还有客户行既不能提升客户 context，也不能阻止合法员工 context。列举平台与应用全部入口。
- 角色分配必须定位有效员工 `membership_id`，客户查询不附带同账号员工角色；`user_roles` 可保留现有用户／祖先角色含义，不要求新增角色表，但每次使用须由有效员工关系证明资格。临时停用可保留配置、读取资格立即失效；永久退出／删除／身份替换只清理结尾有效员工集合不再支持的绑定，不能按 user 全删。
- 员工批量保存只作用 `EMPLOYEE` 集合，以三元键冲突处理并保留客户行。旧无类型请求可在开关及目标 schema 允许后为仅有客户的账号新增员工独立行；不得将已有停用员工自动启用，启用只能走显式生命周期 API。
- 员工目录先验证调用者 actor 的当前 context，`CUSTOMER` context 不能调用。列表和单人选择的 target 则按有效 `EMPLOYEE` 行过滤和定位：仅有客户关系的目标不能作为员工，但同 user／org 同时有两种身份的目标，其有效员工行应允许合规员工调用者选择，不能因还存在客户行就拒绝该账号。同 user／org 的客户行不能替代停用员工证明，旧 user ID DTO 内部也须显式限定员工类型。门店联系人、工单处理人同时验证调用者与目标，不能传 ID 绕过。

#### Expected Outputs
统一员工资格判断、目录与授权入口改造、双向回归用例、旧批量请求兼容说明、`validation/phase-03.md`。

#### Acceptance Criteria
- [x] 具有祖先组织角色或全局超管标识的账号切入客户身份后仍无法访问任何员工管理 API。
- [x] 同组织两行并存时员工 context 正常有权、客户 context 无员工角色且不能调用员工目录；从成员和从角色两个方向均无法给客户行授权。
- [x] 仅有客户关系的 target 不出现在员工候选、直接提交其 ID 也被拒绝；双身份 target 的有效员工行可被有范围权限的员工 caller 选择，不能因同账号客户行而拒绝；提交客户成员 ID 或只有停用员工行均不能通过。
- [x] 员工批量新增、删除、并发保存不覆盖／删除客户；旧请求不自动启用停用员工；目标 schema fixture 下仅有客户时可新增员工独立行，同类型重复返回 `409`。
- [x] 停用员工后同组织客户不能继续证明员工角色资格；保留其他有效员工关系支持的角色，身份替换不误删其他组织或同组织另一身份的会话。
- [x] 临时停用时保留的角色配置不能继续授权；永久退出／删除／替换只清理无有效员工支持的配置，不误删仍有员工资格的账号授权。
- [x] 成员目录、角色和数据授权全部 JOIN 已消除 user＋org 两行歧义，不依赖首条记录或合并的账号角色。

#### Risks
按用户全局解析角色而忽略当前上下文；只过滤列表未验证已选 ID；角色清理范围过宽。

#### Dependencies
阶段 2 完成；涉及跨仓业务候选校验时，逐仓记录改动与各自测试，不建立业务 crate 路径依赖。

#### Notes
本阶段的目标是资格闭环，非将所有客户接口塞入现有员工权限菜单。

---

### Phase 4: 通知受众、历史隔离与实时撤权

#### Status
COMPLETED

#### Goal
客户不会因复用账号、组织或连接而读到员工消息，身份撤销立即作用于通知 HTTP 和实时通道。

#### Scope
平台通知持久化、受众展开、旧结构隔离 fixture、列表／详情／已读、Socket.IO 连接与重连／补拉，以及通知服务资格验证；本新库无实际历史消息归类或回填。

#### Out of Scope
修改已发布 Notification 1 契约、实现新的营销通知业务、泛化消息中心或消息队列改造。

#### Inputs
阶段 3 员工资格、平台通知路由、`saas-notification` 实时协议、旧结构通知 fixture 与权限校验调用链；Q9 首版不开放客户通知的建议。

同组织双身份与通知去重测试使用专用目标 schema 隔离 fixture，仅测试准备去掉旧二元约束；实际收缩迁移保留到阶段 6。

#### Execution Requirements
- 在服务器存储与查询中明确员工／客户受众，收件成员绑定具体 `recipient_membership_id`／身份版本，通知去重也从 user／org 维度改为收件成员维度；无身份标记的旧广播默认员工，不能按相同 `user_id` 或组织直接跨身份展示，同组织双身份也不能合并收件箱。
- 首版建议不开放客户通知，未登记客户用途的发送请求拒绝；隔离测试仍验证客户受众边界。后续客户定向消息必须具有明确用途与客户成员归属，员工发送者／经办者 actor 与客户 owner 分别绑定各自成员，不能让员工广播对客户可见。
- HTTP、初次连接、重连、补拉和在线投递均绑定当前有效成员／版本；撤销事件可加速断开，下一次读写仍核验权威状态。
- 保持 Notification 1 历史 payload 稳定；所需新增数据优先留在服务端，确需新协议时单独版本化且记录兼容策略。

#### Expected Outputs
通知受众模型、无历史回填说明及旧结构 fixture 验证、实时资格校验、跨身份／撤销用例、`validation/phase-04.md`。

#### Acceptance Criteria
- [x] 客户通过列表、详情、已读、补拉或已有连接均不能获得员工广播／定向消息；旧结构通知隔离用 fixture 验证，本新库不要求真实历史消息回填。
- [x] 替换／停用员工后旧员工连接无法继续收取员工消息；重连与重放也不能恢复资格，同组织仍有效的客户连接可继续读取其合规消息，反向同理。
- [x] 同组织双身份切换不混用通知、已读、广播和跳转路由；通知查询／广播展开不因两行 JOIN 重复投递或跨身份可见。
- [x] 权威状态故障时通知接口／连接拒绝继续授权；原 Notification 1 消费者契约不变。

#### Risks
历史消息仅以账号寻址；连接只在握手验权；已读更新通过消息 ID 绕过受众规则。

#### Dependencies
阶段 3 完成；通知服务单独仓库门禁及隔离服务测试必须执行。

#### Notes
本阶段完成之前，客户创建开关保持关闭；不能用“客户页面尚未上线”替代后端隔离。

---

### Phase 5: 平台管理 UI 与缓存会话隔离

#### Status
COMPLETED

#### Goal
平台管理员能够明确区分员工与客户，身份切换不会复用另一身份的权限、菜单、应用或通知缓存。

#### Scope
员工／客户分视图、同组织双身份展示与选择、精确账号绑定、独立启停／新增第二身份／替换旧身份交互、员工角色入口限制、身份切换后的 query/cache／tab／会话命名空间；客户端与应用允许身份集合管理。

#### Out of Scope
客户独立门户、用户自助注册、后端未完成时开放客户创建、改造固定 SDK 发布副本。

#### Inputs
阶段 1～4 管理 API、context 与成员 DTO、平台 React 管理界面、缓存键和通知状态实现。

双身份浏览器／API 测试使用专用目标 schema 隔离 fixture，仅在测试准备移除旧二元约束；不能为界面演示提前向正式环境发布收缩迁移或开启第二行。

#### Execution Requirements
- 由服务端返回当前 `membership_id` 与真实身份；同 user／org 的员工和客户分别显示为独立关系及状态，客户行不出现角色、部门岗位等不适用的员工授权入口。身份选择不能只传组织 ID。
- 清晰区分“新增第二身份”（保留旧身份）与“替换旧身份”（显式创建／启用目标后停用源）。后者显示源／目标成员及旧身份失效影响和原因，目标已存在时显式选择启用／复用，不能盲目复活旧授权／会话；调用组合生命周期 API，禁止编辑单行 `member_type`。
- 客户与第二身份创建受后端开关控制；阶段 6 安全基线及收缩迁移未完成前按钮不可用且后端同样拒绝。普通切换已有身份只选择有效成员，不写类型、不创建关系。
- 前端 login key、缓存键和服务端会话明确包含账号、租户、组织、具体 `membershipId` 与身份版本；旧 context 必须由服务器证明其具体成员绑定，不能按 user／org 猜测。切换身份清理旧菜单、权限、应用、通知和活动请求，避免多标签页混用。
- 应用／人类客户端表单分别提供员工、客户多选和策略收紧影响提示；不把 PUBLIC／CONFIDENTIAL 选项改成员类型，不允许以客户端配置扩张应用授权，机器客户端采用独立主体用途展示。

#### Expected Outputs
分视图管理界面、身份与缓存切换处理、关键浏览器用例、`validation/phase-05.md`。

#### Acceptance Criteria
- [x] 员工批量编辑只操作员工，客户在刷新后仍存在；客户不能通过 UI 或直接 API 获得员工角色。
- [x] 同组织双身份、多组织、多标签页、身份替换与旧请求晚返回场景不复用另一 `membership_id` 的菜单、数据或通知；普通切换不改成员记录。
- [x] 新增第二身份不停用源；替换身份必须显式操作源与目标，目标已停用时不自动恢复旧权限；两条身份可以分别启停。
- [x] 创建开关关闭时 UI 和 API 均阻止创建；固定 SDK 及历史协议副本未修改。
- [x] 应用／客户端集合编辑保存正确且安全默认员工；策略收紧后旧页面及旧凭据不能保留已撤准入。

#### Risks
只按 `user_id` 缓存权限；旧异步响应污染新身份；前端仅隐藏控件而后端放行。

#### Dependencies
阶段 4 完成；前端按所属仓库执行构建、现有测试和必要浏览器验证。

#### Notes
本阶段可审查完整管理体验，但客户生产创建仍等待下一阶段的后端整体安全验收。

---

### Phase 6: 客户最小 API 与应用准入策略

#### Status
COMPLETED

#### Goal
形成可验证的最小客户访问闭环，并以完整后端安全基线控制客户创建开关。

#### Scope
独立客户 profile/context DTO、本人基础资料、消费阶段 2 的客户端／应用准入与策略版本、未登记客户入口拒绝规则、阶段 1～6 安全回归、独立收缩增量迁移删除旧二元唯一约束、客户与第二身份创建启用条件。

#### Out of Scope
编造订单、支付、工单等客户业务；客户角色配置；直接开放既有员工应用；独立门户工程。

#### Inputs
阶段 1～5 产物、应用登记与 token 发放链、完整方案中的客户固定规则；Q2 的显式组织选择、跨组织独立成员及账号绑定依据。本新库 Q1 历史分类不适用。

#### Execution Requirements
- profile/context 仅返回当前账号所选有效客户 `membership_id`、租户组织及最小本人资料；同组织员工成员不附带为客户角色或 owner。每次读取核验主体和成员，不允许任意传 `user_id` 或 `membership_id` 代读；真实客户资源 owner 与员工经办 actor 将来分别绑定具体成员，不能仅因账号相同合并。
- 旧应用默认 `EMPLOYEE`；允许客户必须显式登记用途、入口、身份类型和后端校验，不将平台菜单授权当作客户准入。
- 消费阶段 2 已完成的应用与人类 OAuth 客户端 `allowedMemberTypes` 契约，授权取成员实际类型 ∩ 来源客户端集合 ∩ 应用集合，并继续核验租户组织；客户专用 client 精确绑定客户用途，不采用最早 client 等兜底选择。机器客户端按独立机器授权处理，不能伪装客户／员工。
- 集合收紧同事务递增 policy version 并撤销不合规凭据；客户出现在集合只表示入口允许，具体接口仍须核验资源、动作和 owner。
- 使用独立 portal DTO，不静默扩展已固定 Context 1／MicroProps 2；客户无角色权限，但仍具备明确的业务身份校验。
- 双类型客户端／应用只允许在已存在的有效身份中选择；每次请求使用一个确定 context，员工入口选择员工行、客户入口选择客户行，不按首条成员默认，不自动新增身份。
- 核查全部实例、CLI、导入器和批任务已采用双身份安全代码；实际存在仍使用二元 `ON CONFLICT`／user＋org 唯一查询的旧 binary 时清退，不存在则记录不适用。确认无依赖旧二元唯一键的外键后，单独提交并执行收缩增量迁移删除旧约束，不能提前放入阶段 1 auto migrate。
- 仅在阶段 1～6 的所有后端门禁、双身份 JOIN／局部撤销、通知隔离、固定契约与收缩迁移验证通过后，才允许按环境启用管理员客户和第二身份创建；新建时落实 Q2 的真实组织、租户／管理范围与账号绑定依据，不等待不适用的 Q1 旧分类清单。开启后禁止回退到假设二元唯一的旧版。

#### Expected Outputs
最小客户 API、应用身份准入策略、删除旧二元唯一约束的独立收缩增量迁移与全实例 ready 记录、客户／第二身份创建启用清单、`validation/phase-06.md`。

#### Acceptance Criteria
- [x] 客户能读取自己的有效 profile/context，不能读取其他客户、跨租户成员或员工授权。
- [x] 客户无法对旧应用取得 app token；显式登记的最小客户入口可按规则使用，未登记用途一律拒绝。
- [x] 客户 client＋员工 app、员工 client＋客户 app、错误组织、策略版本过期均拒绝；双类型集合只允许选择已有身份，不自动创建，单次请求不能合并两行权限，机器客户端不能套用成员准入。
- [x] 收缩迁移前所有实例／写入器已 ready；旧 binary 清退按实际部署验证或登记不适用。收缩后同账号同组织两行可并存、同类型重复／并发创建返回 `409`，既有员工 fixture 的 ID／授权与三元 `ON CONFLICT` 正常。
- [x] 开启第二行后验证同组织员工 bulk 新增／删除不误写客户、停用员工不误踢客户及反向场景；不存在 user＋org 查询取首条歧义。
- [x] 有阶段 1～6 的安全回归及独立收缩迁移证据后才能开启创建；真实创建必须校验组织归属、管理范围和绑定依据，没有有效目标时拒绝而不自动挂根或建组织。

#### Risks
把“无角色”误解为匿名放行；以菜单授权间接开放客户；收缩迁移早于旧 binary 清退导致旧 `ON CONFLICT` 失效；双行 JOIN 混淆权限；为未确认业务预建大量 API。

#### Dependencies
阶段 5 完成，阶段 1～4 安全基线均通过；Q1／Q11 已按新库范围解决，Q2 的组织／绑定规则在创建时执行，不要求历史分类清单。Q3 已有本人资料最小范围建议，订单／工单等另在真实业务立项时细化。

#### Notes
已完成身份模型与客户最小能力，仍由现有平台进程提供认证。实际证据见[Phase 6](validation/phase-06.md)：193 普通、main 178 项 ignored 逐项分组通过（保留 176／2 失败及定点修复）、rollout 3 项、通知服务 36＋3、活动合同与继承前端；任务新库真实 7 项资产／10 项 gate、独立收缩、三个启动拒绝及同 receipt 恢复后 30 项 HTTP 均通过。首次地区 COPY 超时及 60 秒初始化预算修正保留。旧 binary 不适用与创建启用仅限该 TASK_OWNED 新库，生产默认关闭；后端文件／迁移／结构变化须重新取得对应 receipt。接下来开始认证边界演进。

---

### Phase 7: 同进程认证模块、账号管理 port 与存储 owner

#### Status
COMPLETED

#### Goal
在当前平台进程内分离账号认证和组织授权，为独立 SSO 建立可测试的依赖边界。

#### Scope
账号凭证与认证用例模块、平台使用的认证 port、账号管理 port、存储与迁移 owner 清单、混合 legacy OAuth 分类。

#### Out of Scope
立即部署独立 SSO、新 OIDC 协议、无收益拆 crate、一次迁走所有 `/oauth/*`。

#### Inputs
阶段 6 现状、账号登录／密码重置／账号启停路由、`users`、session、OAuth 与审计调用链、上级工程规则。

#### Execution Requirements
- 消费者定义窄 port；密码校验、全局账号启停和重置归认证职责，租户／组织／成员／角色／菜单／数据授权归平台。
- 区分账号基础资料和平台授权字段；当前 `users.is_super_admin` 由平台策略读取，SSO 不将其包装为通用身份授权声明。
- 给 `users`、凭证、全局 session、平台 session、app token、混合 OAuth 表逐项标注读写和迁移 owner，提出只读投影或过渡适配。
- 通过 fake／契约测试验证 port 可替换；保留当前外部入口及撤销语义，不用跨仓业务 crate 共享实现。

#### Expected Outputs
同进程认证模块及 port、存储 owner／调用清单、混合 OAuth 分类和迁移策略、`validation/phase-07.md`。

#### Acceptance Criteria
- [x] 平台组织授权不直接操作密码验证实现；重置密码、全局封禁等从账号管理 port 发起。
- [x] 每类账号／会话／OAuth 数据只有明确 owner；不存在把全部 OAuth 按前缀迁出的计划。
- [x] port 的假实现或契约验证通过；既有员工、客户和 legacy OAuth 行为回归通过。

#### Risks
伪分层仍互调内部模块；混合表被两个服务同时迁移；为了抽离引入循环依赖。

#### Dependencies
阶段 6 完成；工程目录与包形态先保持原状，独立服务位置在阶段 12 决定。

#### Notes
本阶段已完成，见[边界与 owner](phase-07-authentication-boundary.md)和[实际验收](validation/phase-07.md)：194 普通、178 既有 ignored、4 新 PG／HTTP、真实 development 启动单项及静态／文档门禁通过；首轮 lint 与夹具失败保留。端口为复用应用错误的同进程能力接口，hash 访问器仍为 crate 内可见，基础资料／删除／seed 保留明确过渡 writer，不宣称已成为独立无框架服务。旧个人改密 in-flight 限制已记录，Phase 8 必须补锁后原认证上下文再验。

---

### Phase 8: SSO 全局 session 与登录 Cookie

#### Status
COMPLETED

#### Goal
把账号认证建立为不依赖成员关系的全局会话，并与平台 context 和应用凭据关联撤销。

#### Scope
账号级登录、SSO 全局 session、认证 Cookie、认证态读取、全局注销／封禁／密码重置、SSO 与 legacy 凭据关联及审计。

#### Out of Scope
完整 OIDC provider、业务应用依靠 SSO Cookie 直接授权、给无成员账号创建默认组织。

#### Inputs
阶段 7 认证模块与 port、阶段 2 凭据矩阵、账号保护规则、阶段 4 实时撤权机制。

#### Execution Requirements
- 允许有效且无 membership 的账号完成账号认证；平台另行选取有效身份建立 context，无成员时只显示无可访问身份。
- 全局 session 与 context 分开存储和管理，明确 session ID／版本、过期、撤销、认证时间、Cookie 范围与 CSRF 防护。
- 全局注销／封禁／密码重置由账号 owner 原子提升 account_version 并撤销全局会话；平台 legacy 凭据实时比较权威版本，无 SSO sid 也立即失效。按 user_id 清理与 outbox 幂等回收作为后续物理清理，覆盖 code／device，不能留事件送达前继续兑换的窗口。
- 对账号自助命令增加提交前认证上下文再验：Phase 7 审查确认旧个人改密只在账号锁后比较密码 hash，ban／kick／成员撤销先提交时，已开始的命令仍可能完成（不会恢复 session 或成员，重置已改 hash 则拒绝）。Phase 8 必须为平台 legacy 改密在账号锁后复核原 session／具体成员；新的 SSO 自助命令复核 global sid、账号状态与版本。旧认证上下文失效时拒绝 pending 命令；以密码重新认证须显式建立新认证，不能补读当前版本复活旧上下文。使用真实锁等待回归覆盖 ban、kick、成员撤销及 password reset，保持账号先锁的顺序。
- 成员变更只按具体 `membership_id` 撤对应 context 和派生凭据，不能破坏同账号同组织另一身份及其他合法成员；SSO 状态校验不可用时拒绝继续业务授权并返回 `503`。

#### Expected Outputs
[0016 全局会话迁移](../../migrations/0016_sso_global_sessions.sql)、[SSO 模块](../../src/sso/mod.rs)、[Cookie／撤销合同](phase-08-global-session.md)、legacy 个人改密锁后原会话再验及[实际验证](validation/phase-08.md)。

#### Acceptance Criteria
- [x] 无成员账号能认证，但无法取得任何组织 context 或业务权限。
- [x] 全局注销、封禁及密码重置后，全部关联新旧凭据无法使用或继续兑换；仅停用一成员保留同组织另一身份、其他组织身份与全局认证。
- [x] Cookie 固定、伪造、CSRF、过期和权威状态故障有验证，敏感凭据不落日志。

#### Risks
把平台登录 session 更名为 SSO 却仍绑定组织；旧凭据未关联导致全局注销失效；Cookie Domain 过宽。

#### Dependencies
阶段 7 完成；测试可以使用隔离域名，实际公开域名与 Cookie 拓扑在阶段 9／12 前按 Q4 确定。

#### Notes
本阶段已完成：[实际证据](validation/phase-08.md)记录 194 普通、21 新增 PG／HTTP、182 既有 ignored、3 rollout、16 配置及真实 Edge 15 项检查均 PASS，保留夹具／协议错误类别与 QA 驱动的首轮失败。无成员账号可登录、改密和全局注销；Cookie 不能直接授权平台 API。legacy pending 改密在账号锁后重验原 session／成员；SSO 自助命令重验原 sid、状态／版本并在提交前检查原期限，审计等待到期则整体回滚。当前 legacy 以 user／account_version 关联全局撤销，没有伪造 sid 或新建组织上下文。SSO_ENABLED 与成员创建开关默认 false；本阶段不宣称已交付 OIDC、BFF 或生产部署。

---

### Phase 9: 标准 OIDC provider

#### Status
COMPLETED

#### Goal
在认证边界实现可被独立客户端按标准验证的 OIDC 授权码登录。

#### Scope
发现文档、授权／token／userinfo／JWKS、客户端登记及用途／允许身份集合、精确 redirect URI、code 单次使用、S256 PKCE、ID Token、nonce／state、非对称签名与密钥轮换。

#### Out of Scope
把 ID Token 用于员工或客户 API 授权、整体迁移 legacy OAuth、改变平台旧 refresh endpoint、独立部署。

#### Inputs
阶段 8 全局 session、阶段 7 legacy OAuth 分类、Q4 的公开 issuer／客户端和 callback、完整方案中的协议与密钥设计。

#### Execution Requirements
- 公开 issuer 在当前环境从此固定；ID Token 最少严格校验 `iss`、`sub`、`aud`、`exp`、`iat`、`nonce` 和签名算法，`sub` 对应本新库从创建后保持稳定的 v7 `users.id`。不执行旧主体映射，v4／错误主体作为拒绝用例。
- 授权码短期、一次性、绑定客户端和回调地址；强制 S256 并验证 code_verifier，拒绝 redirect 通配、code 重用、错误客户端和降级算法。
- 新 provider 使用独立非对称密钥和 `kid`／JWKS；不复用现有 app JWT 共享密钥。轮换保留旧公钥至对应 token 最大有效窗口结束。
- OIDC claims 不携带员工角色、超级管理员能力或可直接授权的组织权限；refresh 签发策略、轮换／重用检测与注销策略显式记录。
- OIDC 注册客户端明确 RP 用途和 `allowed_member_types`，客户门户 client 为客户用途，后台 client 为员工用途；它们不替代平台的实时成员和应用交集校验。SSO 核心仍允许无成员账号认证，不能为使 client 准入通过而虚构组织 context。
- 新 OIDC 客户端不继承含糊的 legacy 首个客户端；client 用途策略改变后同 client 全部旧 code／refresh／grant 统一按版本失效，平台重建所需成员 context，保留全局账号认证及其他 client；SSO 不查询成员来选择撤销某个 code。

#### Expected Outputs
OIDC provider、客户端配置与发现文档、密钥轮换操作说明、标准协议／负面用例、`validation/phase-09.md`。

#### Acceptance Criteria
- [x] 标准客户端可完成 code＋S256 并验证 ID Token；无成员账号也能完成认证但无业务授权。
- [x] code 重放、错误 issuer／audience／nonce／回调／PKCE、过期和算法降级全部被拒绝。
- [x] 新旧密钥重叠窗口及失效后行为验证通过；provider 不泄露员工权限，legacy OAuth 回归仍通过。
- [x] 客户／员工 client 的登记用途可识别且不能扩大应用准入；客户端收紧后旧 code／refresh 无法恢复已撤的业务上下文。

#### Risks
只实现 OAuth code 却遗漏 OIDC ID Token／发现；issuer 随部署变化；全局 session 撤销与 token 生命周期脱节。

#### Dependencies
阶段 8 完成；Q4 的稳定 issuer 和计划接入客户端在实际配置前确认，隔离协议测试可先行。

#### Notes
本阶段已完成，见[协议与密钥](phase-09-oidc-provider.md)及[实际验收](validation/phase-09.md)：194 普通、9 新 PG／HTTP、203 既有 ignored、3 rollout、37 配置与两轮各 117 项真实 Edge／openid-client 检查均 PASS，最终 6 张截图已逐张复核；137 个 UUID 列独立 CHECK 覆盖通过。保留测试编译失败、主动停止旧构建链及截图动画中间帧记录。固定隔离 issuer、客户端与外部密钥仅属于 TASK_OWNED 验证，不代表生产配置；离线 ID Token 与已缓存公钥的撤销限制已记录。SSO／OIDC 及成员创建默认关闭，下一阶段才实施平台 BFF。

---

### Phase 10: 平台 BFF OIDC 客户端与 legacy 桥

#### Status
COMPLETED

#### Goal
平台以 OIDC 客户端完成登录，在服务端建立组织上下文；按本环境实际消费者保留必要的协议兼容路径，不制造历史凭据迁移或既有用户转换窗口。

#### Scope
平台 BFF login／callback／logout、服务端 token 存储、state／nonce／PKCE、成员选择与 context 创建、实际保留的登录／refresh 协议适配与准入开关。

#### Out of Scope
客户门户工程、无审批批量下线旧客户端、浏览器长期保存 OIDC refresh token、用 ID Token 访问 app API。

#### Inputs
阶段 9 OIDC provider、阶段 2 context、阶段 8 全局撤销、当前平台 `/api/auth/*` 与前端会话逻辑、Q5 活动 SDK／应用／机器服务与兼容入口消费者清单；本新库无历史凭据输入。

#### Execution Requirements
- BFF 验证 ID Token 后依据 `sub` 枚举当前有效成员，再以明确 `membership_id` 选择上下文；员工入口只选择员工行，即使同组织也有客户行也不取首条或合并。无成员只完成登录，不自动建关系。
- 平台 RP 精确使用自己的 client ID，并按其员工允许集合与目标应用集合筛选上下文；被选 client、member、tenant／organization、application 均须同时匹配，不能从其他 client 借用客户或员工准入。
- OIDC token 和平台 refresh 留服务端，浏览器使用防护 Cookie；callback 绑定发起会话并拒绝登录 CSRF，组织切换仍由平台授权。平台 BFF 的 POST /auth/context-token 在 Cookie＋CSRF＋权威状态校验后返回最长 300 秒的平台 context Bearer，仅存浏览器内存，用于现有 HTTP 和 socket token 合同；不能把 SSO token 或 HttpOnly Cookie 值传给原 token 字段。
- 按实际消费者清单保留必要的平台 refresh endpoint 与既有 wire contract，明确其刷新的是本环境新签发的平台／兼容协议凭据。确需适配时仅在服务端证明主体和认证会话关联后执行，不导入历史 refresh、不无条件换成 SSO token。无消费者的旧 grant 默认关闭并记录，不强制制造历史兼容桥。
- 对相同账号同组织双身份、不同组织、全局注销和成员局部撤销逐一验证；普通身份选择不改写 `member_type`。灰度开关仅能退回相同认证 owner、权威账号版本和双身份门禁下的兼容协议／界面，SSO 不可用时不能启动平台本地密码旁路、恢复旧 issuer 或二元唯一假设。

#### Expected Outputs
平台 BFF OIDC 客户端、前端登录接入、实际协议适配／开关与消费者清单（含不适用项）、`validation/phase-10.md`。

#### Acceptance Criteria
- [x] 平台 OIDC 登录、组织／成员身份选择、切组织、过期重登录和注销闭环通过；同组织员工 context 不携带客户 owner，客户 context 不携带员工角色，普通切换不写成员类型。
- [x] 平台 client 的允许身份收紧或 app 策略改变后旧上下文失效；不会因为另一个 client 同时允许两种身份而扩大当前 RP 的准入。
- [x] 实际保留的平台 refresh endpoint 保持契约且执行新身份撤销，未保留的旧 grant 默认关闭并有消费者审查记录；无历史凭据迁移。直接使用 ID Token 调业务 API 被拒绝。
- [x] callback 伪造、错误 state／nonce、重放、跨标签页及依赖不可用有验证；浏览器无长期 OIDC token。
- [x] 使用短期 context Bearer 完成真实 qiankun 应用 token 获取、挂载、通知 context、authenticate({token})、续期及撤权；旧 SDK／通知载荷不改，浏览器无平台 refresh。

#### Risks
把全局 SSO 登录自动映射成任意组织管理员；桥接绕过身份版本；前后端同时存两套冲突 token。

#### Dependencies
阶段 9 完成；Q4 当前接入环境的平台 client ID／callback 真实配置落实，开发候选见已记录建议。Q5 先完成活动消费者合同与必要兼容路径清单；实际存在待退役客户端时再确定其窗口，不因本新库无历史客户端阻塞接入。

#### Notes
本阶段已完成，见[平台 BFF／消费者清单](phase-10-platform-bff.md)与[实际验收](validation/phase-10.md)。最终 195 普通、14 BFF PG、212 既有 ignored、3 rollout、66 配置、505 Node、受影响 qiankun 4 组及真实 Edge 98 条检查记录均 PASS；9 张最终截图逐张复核通过。新增跨 client 实际管理 API 负例扩展既有 policy 测试，定向 1／1、7.63 秒通过，不增加 14 组计数。真实 driver 的 OIDC_REFRESH=3、三个进程完整输出扫描／生命周期及 DB／role／hosts 清理均通过；最终二进制 a62e…与配置证据一致。首次 baseline 198／14 失败、工具错误、二进制重建冲突和已修复微应用本地到期永久卸载均保留。平台旧 refresh 与 OIDC refresh 的端点／owner／用途明确区分；固定 SDK 不变，生产默认关闭。源码快照为 498 文件、a0476bf6…，不替代生产部署授权。

---

### Phase 11: 独立客户门户 BFF 与本人资料

#### Status
COMPLETED

#### Goal
建立与管理后台独立的客户入口，通过同一 SSO 认证，只取得有效客户上下文和本人基础资料。

#### Scope
独立客户门户前后端工程、BFF OIDC 客户端、客户组织选择、最小 profile/context 页面、独立 Cookie／缓存及跨客户端登录验证。

#### Out of Scope
未确定的订单／工单业务、客户自助注册、客户角色菜单、把平台管理宿主完整复制为客户入口。

#### Inputs
阶段 6 客户 DTO 和准入、阶段 9 provider、阶段 10 BFF 经验、Q4 客户门户回调和 Q8 工程位置／命名。

#### Execution Requirements
- 创建工程前按 Q8 确定实际仓库／路径；可默认独立仓库的 `saas-customer-portal` 规划，不把它描述为当前已存在项目。
- 门户作为独立 OIDC client，使用自己的 callback、Cookie 和会话命名空间；复用已有 SSO 认证后仍逐次核验具体有效 `CUSTOMER membership_id`。同组织还有员工行时只选择客户行，不按首条或账号合并角色。
- 门户 client 的 `allowed_member_types` 显式为 `CUSTOMER`，只能进入同样登记客户用途的 API／应用；策略版本与权威状态复核和员工客户端一致。
- 仅有员工关系的账号进入门户时明确无可用客户身份，不自动新增客户；同时有两种身份的账号分别选择入口对应成员。客户 context 访问后台不能因此获得管理员能力，其合法员工 context 仍可按员工授权访问。
- 只实现本人资料与当前身份的最小闭环。Q3 首个真实客户业务另行细化阶段，不凭空设计订单状态或资源模型。

#### Expected Outputs
客户门户前后端与 BFF、最小本人资料页面、客户端配置说明及端到端用例、`validation/phase-11.md`。

#### Acceptance Criteria
- [x] 同浏览器先登录平台再进入门户可复用有效 SSO 认证；平台和门户分别持有正确组织身份及独立会话。
- [x] 客户只能看到本人资料；员工专用、客户已停用、错误组织及直接伪造成员 ID 均不能绕过准入。
- [x] 同账号同组织员工／客户分别登录两个入口，全局注销同时生效；只停用客户成员不破坏同组织有效员工上下文，反向同理，缓存／通知／actor／owner 不串身份。

#### Risks
客户门户复制管理员 UI 带入敏感入口；共享 Cookie 导致会话互相覆盖；工程新建位置未经确定。

#### Dependencies
阶段 10 完成；Q8 只在创建新工程前需要确定，Q4 门户 client／callback 在接入前确定；Q3 不阻碍本人资料最小闭环。

#### Notes
首次真实客户业务可在此后提出独立后续台账，不以臆造业务扩张本阶段。

---

### Phase 12: SSO 独立部署与同库权限治理

#### Status
COMPLETED

#### Goal
把已验证的认证边界部署成独立单元，通过稳定 issuer 与同库最小权限维持现有客户端兼容。

#### Scope
规划 `saas-identity` 独立服务、HTTP port 适配、容器／网关／环境配置、数据库独立角色、只读投影与单一迁移任务、密钥／备份／监控／恢复操作说明。

#### Out of Scope
强制立即拆数据库、共享平台业务 crate、顺手迁移全部 OAuth、正式全量切流或提前删除旧代码。

#### Inputs
阶段 7 owner 与协议清单、阶段 9 稳定 issuer、阶段 10～11 客户端、Q4 域名与网关、Q6 容量／可用性、Q7 RPO／RTO、Q8 服务工程位置。

#### Execution Requirements
- 新工程创建前确定实际仓库和服务位置，遵守薄入口、厚库；平台通过网络 port 调用认证能力，不链接认证服务业务实现。
- 迁移 owner 按实际数据分类执行，平台与认证服务启动不得并发竞争同一组迁移；仅 owner 可写，展示字段走明确投影或 API。
- 网关维持 issuer／发现／JWKS 公开地址不变；分别隔离业务 API、SSO Cookie、客户端回调、运维入口和数据库凭据。
- 密钥、TLS、健康检查、权威状态校验超时／失败、日志脱敏、告警和备份恢复具备可运行说明；SSO 不可用不允许仅验 JWT 后继续业务。

#### Expected Outputs
独立身份服务与部署配置、DB grant／迁移 owner 清单、运维与恢复手册、隔离部署演练、`validation/phase-12.md`。

#### Acceptance Criteria
- [x] 独立进程部署前后 issuer 与客户端配置兼容，两客户端登录和注销闭环通过，无跨仓业务 crate 依赖。
- [x] SSO DB 角色不能修改平台授权数据；平台不能直接改认证凭证；迁移只执行一次且混合表 owner 明确。
- [x] 认证／DB 故障时权威授权失败返回 `503`；密钥轮换、备份恢复、监控告警的演练结果与实际限制已记录。

#### Risks
发行 issuer 随内部服务地址变化；两套迁移器同写；通过过宽数据库角色绕过接口边界；故障降级为仅签名校验。

#### Dependencies
阶段 11 完成；Q4／Q6／Q7／Q8 在真实部署前解决，隔离部署测试可以使用明确的测试指标和域名。

#### Notes
共用一个 PostgreSQL 实例或数据库是可接受的过渡形态，不能据此省略写权限和 migration owner。

---

### Phase 13: 多客户端灰度、恢复与退役验收

#### Status
COMPLETED

#### Goal
以可观测且可恢复的切流完成两客户端统一认证，按实际使用与期限有条件退役 legacy 登录路径。

#### Scope
新部署账号／组织与客户端配置复核、按租户／客户端灰度、端到端及容量验证、撤销故障注入、身份感知恢复演练、活动协议消费者合同及有实际对象时的退役条件。

#### Out of Scope
未经验证全量开启、删除身份字段回滚、在尚有活动调用时直接停旧 refresh／OAuth、以未确认 SLO 宣称达标。

#### Inputs
阶段 1～12 证据、新部署账号／组织与客户端配置、Q2 新建归属／绑定规则、Q5 活动消费者清单、Q6 SLO／容量、Q7 RPO／RTO、实际部署资产。Q1／Q11 历史事项在本范围不适用，只有实际存在待退役客户端才要求其退役窗口与期限。

#### Execution Requirements
- 灰度先隔离 fixture 再指定测试租户／客户端，最后逐步扩大；设置登录成功率、回调错误、授权失败、撤销延迟与依赖不可用的观察指标和回退阈值。
- 覆盖同账号同组织两行并存、同类型重复 `409`、并发创建、旧员工 bulk、新增第二身份与替换旧身份区别、同组织双向局部停用、跨租户身份、祖先角色／超管客户门禁、全局封禁／密码重置／注销、code／device／refresh 重放和实时通知撤权。
- 回退只能至已支持同组织双身份、具体成员局部撤销、三元冲突处理与客户隔离的兼容版本；保持 schema 与稳定 issuer，保留能重新登录的身份感知路径，不恢复 user＋org 唯一假设。
- 对实际存在且计划退役的客户端分别登记 owner、活动流量、迁移验证、停用日期与撤销方式；满足条件后才停用。新库没有历史客户端时记录不适用及消费者清单依据，不编造转换窗口或退役日期；活动 SDK／应用／机器服务合同仍须验收。

#### Expected Outputs
灰度与观察记录、性能／恢复／撤销验收、活动消费者兼容清单及实际退役决议或不适用说明、最终操作手册、`validation/phase-13.md`。

#### Acceptance Criteria
- [x] 平台与门户在真实配置下完成同账号同组织双身份登录、切身份、局部与全局撤销；员工离职不影响同组织客户，反向同理，客户无员工权限、目录或历史通知泄露。
- [x] 已按确认的容量／SLO／RPO／RTO 完成演练并记录实测结果；未达项明确留待修复，不标完成。
- [x] 回退演练不退到无类型或假设 user＋org 唯一的旧版；实际旧客户端仅在迁移、观察窗口和确定期限满足后退役，无历史客户端则凭清单记录不适用，活跃兼容路径仍受新撤销规则保护。
- [x] 对查询／JOIN／ON CONFLICT／DELETE／触发器／广播的双身份歧义审查闭环；旧凭据无法唯一确定成员时确实失效重认证，不能靠回退恢复。
- [x] 所有阶段交付与证据可追溯，实施状态、仍支持的 legacy 路径和后续首个客户业务范围在文档一致。

#### Risks
灰度范围与真实数据未对齐；以关闭新入口掩盖旧凭据仍有效；把恢复误当 schema 降级；提前退役真实客户端。

#### Dependencies
阶段 12 完成；实际全量切流前落实 Q2 新建组织规则、Q5 活动消费者合同、Q6 容量／SLO 与 Q7 恢复目标。Q1／Q11 已按本新库范围解决，Q5 历史退役输入只在有实际对象时适用，其余不影响范围的问题可列为后续事项。

#### Notes
实际旧客户端尚未满足退役条件时，保留明确兼容窗口及 owner；没有历史客户端不构成完成障碍，但新部署的兼容、安全、容量与恢复交付必须真实验收，不能因不适用说明而假标 `COMPLETED`。

本阶段按用户批准的分项复用策略完成，见[操作手册](phase-13-rollout-recovery.md)及[验收](validation/phase-13.md)：R20 完整 1800 秒 steady，R21 的 221 协议／15 灰度与 3.336875 秒显式回退，R22 完整 300 秒 burst／140 项双库恢复及恢复浏览器 247 项，R24 当前门户修复后的初始浏览器 343 项分别通过。RPO 5.805 秒、RTO 21.443394 秒，范围为本地任务双库与库外材料恢复至旧凭据拒绝及新双 RP 业务可用。R22 恢复浏览器使用第一次 UI 修复，R24 使用第二次有界只读修复；未在第二次 UI 版本上重跑已成功恢复。原失败／中止、准确源码／运行 pin、55 项新 UI 门禁及 417 项既有合同的字节适用范围均保留。24 个任务运行目录和本阶段 PKI、Windows 私有材料完成清理，公开回执与 16 张最终截图保留。上述结果不构成生产 SLO／PITR 或全量切流；活跃 legacy 继续支持，首个客户业务仍限本人资料。

---

## 5. Current Execution Pointer

Current Phase: DONE — Phase 1～13 COMPLETED（本身份新库任务范围）

Current Objective:
13 个身份阶段均已按各自验收范围完成。本阶段分轮容量、指定客户端灰度／安全回退、双库／库外材料恢复、外部高水位／deny-only 对账、旧凭据拒绝、通知撤权和活动消费者合同已聚合封存，见[Phase 13 验收](validation/phase-13.md)。没有后续未实施的身份阶段；真实客户新业务与生产部署另按已记录输入推进。公开 issuer 固定为 https://auth.saas.test。TASK_OWNED 实测不提升为生产 SLO／PITR／资产就绪结论，预览 UUID 专项保持独立。

Current Restrictions:
- 2026-10-25 按用户最新要求跳过已成功项、只重测失败项：复用 R20 steady、R21 协议／灰度、R22 burst／完整恢复／恢复浏览器，R24 仅补测初始浏览器。R21 重复 steady 按范围优化中止，R22／R23 初始浏览器失败原件保留。各轮及传输／UI 版本分别记证，不能宣称同轮连续 35 分钟或当前 UI 重跑全部阶段通过。原各操作计数、延迟、并发、预算与安全门槛不降低；产品默认关闭及生产边界不变。
- Phase 13 五项验收已完成并封账；没有执行生产全量切流或提前退役活动客户端。
- 所有阶段更新依据逐项验收与真实记录；预览库 UUID 专项的授权、数据库和动态指针保持独立。
- 门户 BFF 必须精确使用本 CUSTOMER RP／已有客户成员／tenant-org／登记入口交集；平台仍只使用自己的员工 RP。长期凭据留服务端，ID Token 不作为业务 Bearer；SSO 不可用时不能启用本地密码旁路、只验 JWT 或更换 issuer。本人资料外不新增未经确定的客户业务或通知。
- 正式创建默认仍关闭，只有完整安全回归、收缩迁移和实际环境就绪条件满足后才允许启用；测试开关与 fixture 不能冒充生产开放。
- 已有 validation/fresh-v7-baseline.md 及 phase-01.md～phase-13.md；各记录只证明实际测试范围。域名、客户端、工程和运维候选不等于生产配置或已达标结果；任务自有 receipt 也不替代生产资产审查。

---
## 6. Decision Log

历史行保留决策产生时的范围与指针描述；本身份新库以 v0.4 范围决策及 v0.5 实施记录为准。它们取代 v0.3 对本新库的历史搬迁前置，不改写预览库 UUID 专项的授权或真实完成状态。

| Date | Decision | Reason | Impact |
|---|---|---|---|
| 2026-10-25 | v1.7：Phase 13 COMPLETED，身份任务指针 DONE | 用户要求仅补失败项；R20 steady、R21 协议／灰度、R22 burst／完整恢复／恢复浏览器、R24 初始浏览器分别 PASS，源码／消费者／扫描／清理闭环 | 分轮及两次 UI 修复的范围明确，保留全部失败／中止；生产默认关闭，活跃 legacy 保留，本人资料外客户业务及预览 UUID 专项独立 |
| 2026-10-25 | Phase 13 改为用户授权的分项补测与分轮证据聚合 | 用户要求“跳过已成功的阶段只测试失败的阶段”；R20 steady 已完整通过，R21 协议／灰度已通过，重复执行成本高 | 复用准确 task／SHA 的 PASS 子项，失败及中止原回执保留；定向重测原 300 秒 burst 并完成未闭环恢复／浏览器／扫描。不同传输版本与轮次明确披露，不冒称同轮 35 分钟通过 |
| 2026-10-25 | v1.6：Phase 12 COMPLETED，指针推进 Phase 13 | 独立身份 42 普通／3 PG、平台 221 普通／4 roles／3 accounts／2 admin／TLS1／233 既有／3 rollout、221 协议、214 浏览器及 8 图、完整共享库恢复通过；保留六轮失败与修复 | 独立进程与最小权限已验收，进入实际容量、灰度回退、双库身份感知恢复和当前消费者验收；生产默认及 UUID 专项边界不变 |
| 2026-10-25 | v1.5：Phase 11 COMPLETED，指针推进 Phase 12 | 独立门户 12 普通／5 PG、平台 197 普通／7 新 PG／226 既有／3 rollout、前端 36 Node、真实双 RP 196 条检查及 8 图通过，保留登录 helper 与 accountVersion=0 两轮失败及修复 | 交付 0019 与独立门户；进入认证进程、网络 port 与最小权限治理，默认开关、固定 SDK 和 UUID 专项边界不变 |
| 2026-10-25 | v1.4：Phase 10 COMPLETED，指针推进 Phase 11 | Rust 全链、66 配置、505 Node、真实 qiankun／通知／恢复与注销 98 条检查和 9 图、跨 client 定向负例全部通过，498 文件快照封存 | 独立客户门户按 CUSTOMER RP／会话／本人资料最小闭环实施；默认开关、SDK 和 UUID 专项边界不变 |
| 2026-10-25 | v1.3：Phase 9 COMPLETED，指针推进 Phase 10 | 194 普通、9 新 PG／HTTP、203 既有、3 rollout、37 配置、两轮各 117 项真实 Edge／标准 RP 及最终 6 图均 PASS，137 UUID 列检查通过；失败与修正保留 | 进入平台 BFF／服务端凭据与具体成员 context，不提前实施门户或独立部署；默认关闭开关及生产配置边界不变 |
| 2026-10-25 | v1.2：Phase 8 COMPLETED，指针推进 Phase 9 | 194 普通、21 新增、182 既有、3 rollout、16 配置、真实 Edge15 均 PASS；旧个人改密竞态与新 SSO 自助预算／提交期限已闭环 | 进入标准 OIDC 与独立密钥，不预实施 BFF／门户／独立部署；SSO 与成员开关默认关闭，生产配置仍须按环境落实 |
| 2026-10-25 | v1.1：Phase 7 COMPLETED，指针推进 Phase 8 | 194 普通、178 既有 ignored、4 新 PG／HTTP 与真实 development 启动单项全部 PASS；端口可替换、原事务回滚及账号全局撤销有实际证据 | 进入成员独立的 SSO session／Cookie／CSRF，保留端口同进程和个人改密旧竞态的准确限制；生产创建仍默认关闭 |
| 2026-10-25 | Phase 7 保持旧个人改密 in-flight 行为，Phase 8 必补提交前认证上下文再验 | 只读核对旧代码与当前实现，确认账号锁后仅比较 hash；已撤凭据不会复活，但已开始且持有正确密码的命令可能完成，尤其 ban 不属于新的有效账号认证 | 不以纯端口抽离冒称所有 pending 命令已强制取消；Phase 8 增加平台 session／global sid 的锁后权威再验及真实等待测试 |
| 2026-10-25 | v1.0：Phase 6 COMPLETED，指针推进 Phase 7 | 客户 6、main 178 逐项分组、rollout 3、普通 193、通知及合同门禁通过；真实任务新库完成独立收缩、receipt 启动与 30 项 HTTP，保留失败和恢复证据 | 身份模型与客户最小闭环完成；进入同进程认证／账号管理 port 与存储 owner，生产创建仍默认关闭 |
| 2026-10-25 | v0.9：Phase 5 COMPLETED，指针推进 Phase 6 | 456 Node、185 普通、3 新＋31 相关 PG、九套浏览器逐组及新成员 14／准入 5／应用接入 10 项已有真实证据，保留失败和 StrictMode 修复 | 完成管理 UI／缓存／会话与准入编辑；进入客户最小能力及独立收缩迁移，真实创建仍关闭 |
| 2026-10-24 | v0.8：Phase 4 COMPLETED，指针推进 Phase 5 | 9 新平台专项含真实双服务、65 相关逐项通过、185 普通、通知仓 36 普通＋3 集成及权限验收已有证据，保留失败／重跑过程 | 交付 0015、具体通知身份、私有 authority 和实时撤权；进入管理 UI／缓存，创建开关继续关闭 |
| 2026-10-24 | v0.7：Phase 3 COMPLETED，指针推进 Phase 4 | 185 普通、160 数据库逐项通过（保留全量与分组失败／重跑）、9 新专项、两业务仓合同和 SDK 7 项已有证据 | 员工授权／目录／双向角色／bulk／最终支持清理闭环；通知 recipient 隔离开始，正式创建继续关闭 |
| 2026-10-24 | v0.6：Phase 2 COMPLETED，当前指针推进 Phase 3 | 185 普通、151 数据库、58 CLI 及固定 SDK 合同已有真实通过证据，失败与分组重跑分别记录 | 交付 0014 完整身份／来源／策略证明与准入管理；创建关闭、双约束保留；用户要求按依赖持续完成全部阶段 |
| 2026-10-24 | v0.5：Phase 1 COMPLETED，当前指针推进 Phase 2（NOT_STARTED） | 空库／升级／生命周期／并发及广泛回归已有真实通过与重跑证据 | 交付 0013、API、路径清单与阶段证据；其余阶段未实施，预览 UUID 专项独立 |
| 2026-10-24 | v0.5：扩展期旧人类凭据仅接受 ENABLED EMPLOYEE，员工兼容撤销显式审计 | 现有凭据尚无完整成员绑定；客户不得借旧入口签发或影响员工凭据 | 客户变更不误撤员工；Phase 2 完成精确版本与撤销，全部真实环境新生命周期／客户／第二身份开关继续关闭 |
| 2026-10-24 | v0.5：生命周期写请求必填真实 reason，同事务审计并在等待后重验实时权威状态 | 操作名称不等于变更原因，锁等待期间可能撤权或到期 | trim 后 1～500 字符、无控制字符；失败无部分变更，当前员工自启停／删除／替换返回 403 |
| 2026-10-24 | v0.4：本身份功能使用新库，无历史数据；Phase 1 首项为空库 v7 基线验收 | 用户明确新库新开功能，并要求先更新台账 | Q1／Q11 的历史分类与映射在本范围解决为不适用；不依赖 U06、不导入旧凭据，13 个身份阶段仍 NOT_STARTED |
| 2026-10-24 | v0.4：开放问题建议纳入台账，生产值与实测结果继续分开 | 用户要求先给建议、再更新文档，未提供真实生产配置或运维承诺 | 记录组织／本人资料／通知、开发 OIDC、兼容、容量、恢复与工程候选；按受影响阶段落实，不伪装已配置或已验收 |
| 2026-10-24 | v0.4：预览库保数据专项独立保留范围与状态 | 本轮身份新库与正在处理的预览三库不是同一数据范围 | 两份台账各自维护指针，专项状态不因本文更新而完成、取消或重置 |
| 2026-10-24 | 历史 v0.1/v0.2 原 UUID 保持不变的前提已由 v0.3 用户要求取代 | 已有非 v7 数据也需要转换后重新初始化 | 先执行一次 UUID 身份基线切换，以权威旧→新映射保留业务归属；此后新 v7 users.id/sub 稳定，手机号邮箱不用于合并 |
| 2026-10-24 | v0.3：增加六阶段 UUID 前置专项，原身份 13 阶段编号保留 | 全系统生成器与三库数据引用需先统一到 v7 | 当前指针为 UUID-U01，原身份阶段 1 依赖 UUID-U06；本轮不执行转换或初始化 |
| 2026-10-24 | 历史 v0.1：员工／客户共用 `memberships`，原先约定同组织单身份；该限制已被同日 v0.2 决策取代 | 满足不另建身份表的初始讨论，保留历史决策来源 | 不再作为实施约束；以以下同组织双身份、三元唯一键和独立成员决策为准 |
| 2026-10-24 | v0.2：同 user／org 支持员工与客户两条独立 `memberships`，每类最多一行，类型创建后不可改写 | 用户明确要求同账号同组织同时拥有两种身份 | 最终使用三元唯一键，独立 ID／状态／版本；切换选择成员，新增第二身份与替换旧身份分开 |
| 2026-10-24 | v0.2：扩展迁移与删除旧唯一约束的收缩迁移分阶段执行 | 旧二元 `ON CONFLICT` 在直接删约束后会失败，旧查询也会出现多行歧义 | 阶段 1 保留双约束，阶段 6 全实例安全代码就绪及旧 binary 清退后删除旧约束并开放第二行 |
| 2026-10-24 | v0.2：局部撤销、角色资格、通知／缓存、owner／actor 均定位具体成员 | 同账号或同组织不能代表同一身份上下文 | 员工离职不误停客户，客户不能证明员工角色资格，旧凭据有成员歧义时失效重认证 |
| 2026-10-24 | 客户关联真实组织，仅管理员创建／绑定既有账号 | 避免臆造组织与账号绑定规则 | 无自助注册、自动挂根或自动建组织 |
| 2026-10-24 | 客户不进入 RBAC，按登记入口和本人归属固定授权 | 符合客户没有角色权限的要求 | 客户 API、目录、通知和应用准入需显式隔离 |
| 2026-10-24 | 应用和人类 OAuth／OIDC 客户端都增加允许成员类型集合与策略版本 | 用户明确要求两处准入可独立配置 | 按成员、client、app 交集授权，旧项默认员工，客户门户明确客户用途，收紧立即撤销 |
| 2026-10-24 | 先完成所有后端隔离再允许客户创建 | 仅新增字段会使旧授权路径继续承认客户 | 阶段 1～5 开关默认关闭，阶段 6 安全验收后决定启用 |
| 2026-10-24 | 成员状态变更／移除／身份替换事务递增受影响成员版本并撤销其派生凭据 | 保证在旧 token、code、device 和实时通道下立即生效，单行类型不原地修改 | 局部成员失效与全局账号失效分别实现，同组织另一身份保持独立 |
| 2026-10-24 | 固定 SDK／MicroProps／Context／Notification 历史契约不改 | 维持既有跨仓应用兼容 | 客户先使用独立 portal DTO，内部凭据绑定走服务器存储 |
| 2026-10-24 | 先同进程认证边界，再 SSO session、OIDC、客户端和独立部署 | 控制迁移规模并逐步验收 | 13 阶段均可独立交付证据，不提前创建大型 workspace |
| 2026-10-24 | 采用 code＋S256、BFF、独立非对称密钥和稳定 issuer | 满足标准 SSO 接入和密钥治理 | 浏览器不长期持有 OIDC token，平台权限不进入 ID Token |
| 2026-10-24 | 同库过渡允许，但表／迁移单一 owner 与独立 DB 角色 | 降低初期拆分成本且保留职责边界 | 混合 OAuth 按数据用途迁移，不按路径一刀切 |
| 2026-10-24 | 权威状态校验不可用返回 `503`，事件仅辅助撤销 | 保证全局与局部撤销不会因缓存失效而失效 | 不采用仅验 JWT 或消息最终一致性替代授权检查 |
| 2026-10-24 | legacy refresh 保留受控兼容窗口；不允许回退到无类型旧版 | 保持现有客户端并避免客户变员工的回滚风险 | 最后阶段按真实迁移与流量条件退役 |

---

## 7. Open Questions

| ID | Question | Impact／Recorded Recommendation | Owner | Resolution Status |
|---|---|---|---|---|
| Q1 | 现有生产账号／成员中是否已有实际客户？如何归类和处理重复账号？ | 用户已明确本身份新库无历史账号／成员，此类分类、合并和回填不适用；新建时显式类型，不按联系方式合并 | User／业务负责人 | COMPLETED（本范围无历史数据） |
| Q2 | 客户归属组织及跨组织关系如何确定？ | 建议管理员显式选择本租户内有效组织；跨组织分别建 CUSTOMER，同组织双身份独立；创建时验证管理范围、邀请／账号归属确认及绑定来源 | User／组织管理员 | 建议已记录；创建时校验 |
| Q3 | 首个客户功能及后续业务资源／动作是什么？ | 建议首版本人资料、客户 context 与已登记客户应用目录；订单／工单另定资源、动作、owner／actor，不阻塞最小闭环 | User／产品负责人 | 首版建议已记录；真实业务后续细化 |
| Q4 | 最终 issuer、域名、client ID 与精确回调是什么？ | 开发建议 auth／console／customer.saas.test、各门户 /auth/callback 与 /signed-out，独立 USER client；实际签发 ID 及生产自有域名在对应接入前落实 | User／运维负责人 | 开发候选已记录；生产待定 |
| Q5 | 实际消费者、兼容入口及退役范围是什么？ | 新库无历史迁移／待退役对象，凭活动清单记录不适用；417 项既有合同与当前字节范围已核对，活跃 SDK／应用／机器服务及 legacy 继续支持 | User／客户端负责人 | 本任务清单及兼容验收完成；真实退役另定 |
| Q6 | SLO、并发、调用量与超时预算是什么？ | 100 actors、1s 连接／3s 操作预算，1800s steady 与 300s burst 按用户授权分轮通过；生产 99.9% 和长周期可用性仍为候选 | User／运维负责人 | 任务容量实测完成；生产 SLO 未声明 |
| Q7 | RPO／RTO、备份保留与恢复责任是什么？ | 任务目标 24h／4h，实测 RPO 5.805s、RTO 21.443394s；双库／库外材料、失效边界与旧凭据拒绝完成。生产候选 15min／60min、保留／真实人员及 PITR 需实际部署落实 | User／运维负责人 | 任务恢复演练完成；生产／PITR 另行验收 |
| Q8 | 新工程位置、名称与维护归属是什么？ | 同级独立 saas-customer-portal/backend＋frontend 与 saas-identity 已按 Phase 11～12 交付；工程／端口／owner 边界已验收，真实生产维护人仍需登记 | User／工程负责人 | 工程落位完成；生产维护人待实际部署 |
| Q9 | 客户通知用途、路由与跨代归档规则是什么？ | 建议首版不开放客户通知；Phase 4 仍实现成员／版本隔离，缺省广播仅员工、旧代保留审计不自动可见；本新库无历史回填，真实客户事件及路由后续另定 | User／产品负责人 | 首版建议已记录；扩展时细化 |
| Q10 | 同账号同组织是否需要同时拥有员工和客户身份？ | 已由用户确认需要；v0.2 采用两行独立成员、最终三元唯一键及独立启停，问题已确认不等于阶段已实施 | User／产品负责人 | COMPLETED |
| Q11 | 本身份新库是否需旧 UUID 映射、外部 RP 旧主体或保数据导入？ | 用户已明确无历史数据，此类搬迁在本范围不适用，以空库 v7 基线替代；预览三库的实际保留清单与转换仍由独立 UUID 专项管理 | User／数据与运维负责人 | COMPLETED（本范围无历史搬迁） |

问题的处理状态不等于实施阶段状态：COMPLETED 仅表示相应范围／模型已确认，“建议已记录”也不表示生产方案已批准、配置已完成或测试已通过。上述问题按受影响阶段落实，不构成“全部回答后才可以开始”的审批门禁；真实域名、生产承诺、切流和客户业务归属不得编造。建议全文与依据见[开放问题建议](open-question-recommendations.md)。

---

## 8. Change Log

| Version | Date | Change |
|---|---|---|
| v1.7 | 2026-10-25 | 完成 Phase 13 并将身份任务指针置 DONE。按用户要求分轮复用成功项，仅补测失败项；R20 steady／R21 221 协议与15灰度／R22 burst与140恢复及247恢复浏览器／R24 343初始浏览器全部有独立PASS证据。保留失败与中止、两次门户只读核对修复及55项门禁、当前源码与7运行pin、417旧合同字节适用范围及24轮私有材料清理；生产默认关闭、legacy继续支持、UUID专项独立 |
| v1.6 | 2026-10-25 | 完成 Phase 12 独立 saas-identity、0020、网络 port、最小 owner／runtime 和跨 owner 窄桥；Rust 完整门禁、221 协议、214 条浏览器检查及八图、加密恢复五族 catalog 与受限健康通过。保留六轮失败和全部清理，指针推进 Phase 13，尚未声明其容量／完整双库恢复验收 |
| v1.5 | 2026-10-25 | 完成 Phase 11 独立客户门户、0019 资源 port、精确客户选择与本人资料；Rust／前端门禁及真实双 RP 196 条检查、8 图、刷新／撤销／故障恢复全 PASS。保留两轮失败与修复，指针推进 Phase 12，生产默认不变 |
| v1.4 | 2026-10-25 | 完成 Phase 10 平台 BFF、0018、服务端凭据／明确成员选择／短 Bearer、实际消费者兼容与有界恢复；195 普通、14 BFF PG、212 既有、3 rollout、66 配置、505 Node、真实 Edge 98 条检查及 9 图通过。保留失败与修复，指针推进 Phase 11，生产默认不变 |
| v1.3 | 2026-10-25 | 完成 Phase 9 0017、标准 OIDC／code＋S256／ID Token／发现与 JWKS、独立 RS256 生命周期、固定 issuer、客户端用途／策略与 refresh 重用撤销；194 普通、9 新 PG、203 既有、3 rollout、37 配置及两轮各 117 真实 RP／Edge 全 PASS，最终 6 图与 137 UUID 检查通过。保留失败／主动停止／截图修正证据，指针推进 Phase 10，生产默认不变 |
| v1.2 | 2026-10-25 | 完成 Phase 8 0016、成员独立 SSO session、严格 Cookie／CSRF、账号全局撤销与 legacy 改密锁后原上下文再验；记录 194 普通、21 新 PG／HTTP、182 既有、3 rollout、16 配置及真实 Edge15 PASS，保留失败与修复。指针推进 Phase 9，默认开关不变，未提前实现 OIDC／BFF／门户／独立部署 |
| v1.1 | 2026-10-25 | 完成 Phase 7 同进程认证／账号管理 port、原事务适配与字段／混合 OAuth owner 清单；记录 194 普通、178 既有 ignored、4 新 PG／HTTP、启动单项与静态／文档 PASS，保留两轮失败及修复。指针推进 Phase 8，并明确个人改密锁后再验要求；未提前实现 OIDC／门户／独立部署 |
| v1.0 | 2026-10-25 | 完成 Phase 6 独立客户最小资源、精确用途登记、写入器审查、独立收缩和二进制／环境 receipt；记录 193 普通、178 main ignored 分组、3 rollout 与真实新库 30 HTTP、首次 COPY 超时及同 receipt 恢复。指针推进 Phase 7；生产默认关闭，UUID 专项独立 |
| v0.9 | 2026-10-25 | 完成 Phase 5 成员管理、服务器 proof／跨页 namespace 与准入 UI，记录真实 PostgreSQL、Node、浏览器、构建与 StrictMode 修复证据；指针推进 Phase 6，生产创建保持关闭，未执行未来 SSO／门户阶段 |
| v0.8 | 2026-10-24 | 完成 Phase 4 通知成员／代际绑定、去重、旧行归档、HTTP／偏好隔离、私有实时证明及双向在线撤权；记录真实两服务、通知仓与平台门禁、独立 UUID CHECK／119 列和最小角色权限验收，指针推进 Phase 5，真实客户创建仍关闭 |
| v0.7 | 2026-10-24 | 完成 Phase 3 员工身份门禁、双向角色、目录候选、bulk 与最终角色支持集合，修复真实锁等待竞态／角色父子锁序及旧自行退出兼容；记录 185 普通、160 数据库逐项通过、两业务仓与固定 SDK 门禁，指针推进 Phase 4；客户和第二身份真实创建仍关闭 |
| v0.6 | 2026-10-24 | 完成 Phase 2 具体成员／账号／来源客户端／准入版本绑定、旧凭据拒绝、精确撤销、管理 API／CLI 与真实 HTTP 证明链；补迁移目录构建追踪和完整 UUID 列覆盖；记录全门禁及分组重跑结果，指针推进 Phase 3，全部后续阶段依次执行，真实客户创建保持关闭 |
| v0.1 | 2026-10-24 | 创建员工／客户与 SSO 的 13 阶段实施台账；仅文档，所有阶段未开始，执行指针为 Phase 1 |
| v0.2 | 2026-10-24 | 按用户确认加入同账号同组织双身份；改为独立成员行与最终三元唯一键，扩展／收缩迁移分阶段，补齐角色、凭据、批量编辑、通知、缓存及两门户验收；Q10 已确认，13 阶段仍全部未开始、指针仍为 Phase 1 |
| v0.3 | 2026-10-24 | 增加 UUID v7 审查和六阶段保数据重建前置专项；修订稳定主体为一次性映射后 v7 基线，原阶段 1 依赖 U06，指针改为 UUID-U01；所有实施阶段仍未开始 |
| v0.4 | 2026-10-24 | 按用户新库新功能范围解除本身份库的历史搬迁／U06 前置，Phase 1 首项改为空库 v7 基线；统一指针与限制，修订 Phase 1／2／4／6／9／10／13 历史数据与条件退役要求，记录 Q2～Q9 建议、解决 Q1／Q11 范围问题；预览 UUID 专项独立，13 个身份阶段仍 NOT_STARTED；本轮仅更新文档 |
| v0.5 | 2026-10-24 | 完成 Phase 1 空库 v7 基线、0013 成员／准入扩展、显式生命周期与真实原因审计、默认关闭开关、旧员工写入与凭据资格兼容；记录 PostgreSQL、SDK／消费者及跨平台回归的实际通过和修复证据。Phase 1 标记 COMPLETED，指针推进 Phase 2（NOT_STARTED）；生产创建仍关闭，未实施后续阶段，UUID 专项状态保持独立 |
