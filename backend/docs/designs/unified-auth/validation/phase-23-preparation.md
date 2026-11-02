# Phase 23 只读准备：密钥生命周期与隔离恢复

状态：PREPARATION_ONLY。2026-10-28，基于台账 v1.9 和当时工作区源码；不构成 Phase 23 实施、验收或发布证据。Phase 22 仍在执行，须等待其报告、冻结输入及主执行者明确启动 Phase 23。本文不改台账、生产代码、数据库或运行配置，未复用已经清理的 Phase 20 环境。

## 1. 依赖与输入版本

- 权威范围：[台账 Phase 23](../../unified-oauth-oidc-sso-task-ledger.md)，依赖 Phase 22；Q9 为内部候选 RPO 15 分钟、RTO 4 小时，实际值班人员及正式承诺仍待落实。
- `saas-identity` HEAD：`7025a0a39cff5577bc3aabf239aaae8df529714e`。
- `saas-plateform` HEAD：`34105757b040165daa5477bf6afff9637f617466`。
- `saas-customer-portal` HEAD：`648e71e082929605c67e4e9f7e19fc9ecdbd89d4`。
- 三仓均有其他阶段的未提交修改；HEAD 不代表当前源码。正式执行必须记录冻结源码逐文件摘要、迁移及 ACL、工具、二进制和前端输入，不使用本文 HEAD 充当版本验收。
- 已与 Phase 22 owner `audit_foundation` 对齐：22 不修改密钥／撤销协议；预期交付 metrics、脱敏日志、有界预算、429／503、容量及双实例正确性证据。冻结标签和报告尚未交付，不预设其 PASS。
- 已与 Phase 24 owner `phase24_inventory` 对齐，并纳入 root 后续裁决：当前 canonical 协议仍复用旧命名签名模块、settings 和管理 receipt；没有语义收益就保留实际名称，不机械重命名。23 按真实接口验证，24 以调用链和 allowlist 证明保留对象是单权威，只删除旧 registry/token/routes/fallback 分支；不删除仍被 canonical 调用的模块。

## 2. 当前可用接口及边界

| 能力 | 实际入口／存储 | 执行时需要证明 |
|---|---|---|
| 公钥发布与签名 | identity [keys.rs](../../../../../../saas-identity/src/oidc/keys.rs)，`OIDC_KEYSET_PATH`、`oidc_signing_keys` | 唯一权威；外部私钥和库内 `kid/n/e` 必须匹配；RS256 固定，RSA 2048–4096 位、e=65537 |
| 固定 issuer | `oidc_provider_settings`，canonical `/oauth` authority | 恢复后仍与显式配置完全一致，不能产生第二 issuer |
| 公共 JWKS | `GET /oauth/jwks` | 只返回未退役、未吊销的公钥，不返回私钥；旧 `/oidc/jwks` 属 24 退役对象 |
| 管理 API | `POST /internal/identity/oidc/keys/publish`、`.../activate`；`DELETE .../keys/{kid}`；`POST .../keys/{kid}/revoke` | 当前 purpose 是 **OIDC_ADMIN**，不能写成 AUTH_ADMIN；公开网关拒绝私有路径 |
| 平台代理 | `POST /api/oidc/keys/publish`、`.../activate`，`DELETE .../keys/{kid}`，`POST .../{kid}/revoke` | 当前 `system:oauth:keys`、根员工身份、近期真实 TOTP、来源与 CSRF；旧命名不等于旧 token authority |
| 原命令恢复 | [admin_operations.rs](../../../../../../saas-identity/src/admin_operations.rs)，`GET /internal/identity/oidc/operations/{id}` | 原 UUIDv7 operation、摘要、service/key、method/path 一致；60 秒 intent 窗口提交后恢复 receipt，不能换 operation 重放 |
| 正常退役 | 非活动 key 且 `last_signed_exp + 60s < clock_timestamp()` | 活动 key 或尚在验证窗口必须 409；签名事务持 key 行锁至提交，不遗漏最后一枚 token |
| 紧急吊销 | 设置 `active=false, revoked_at`，从新 JWKS 移除 | 可立即结束活动 key，但必须另有可用 key 才能继续签发；无活动 key 的签发应失败关闭 |
| RP 验证 | 平台 [protocol.rs](../../../../src/bff/protocol.rs)、门户 [protocol.rs](../../../../../../saas-customer-portal/backend/src/protocol.rs) | 当前每次验 ID Token 获取权威 JWKS；签名登出也要测试相同 key 窗口；不以静态 fixture 代替真实 identity JWKS |
| 原生运行 | [snapshot-build.py](../../../../tests/unified-auth-https/snapshot-build.py)、[prepare.py](../../../../tests/unified-auth-https/prepare.py)、[environment.sh](../../../../tests/unified-auth-https/environment.sh)、[run-service.py](../../../../tests/unified-auth-https/run-service.py) | root 冻结、准备、启动及清理；与 22 分开新建任务库，禁止引用 20 已清理实例 |

`KeySet` 在启动时加载，没有文件热重载。正常轮换需先向所有候选身份实例配置新旧私钥集合并重启／核验，再发布和激活新 key；只改文件不证明进程已经加载。公钥可继续留在库内验证而不要求继续保管旧签名私钥，但删除旧私钥前必须确认全部实例停止使用旧活动 key，且存在匹配的新活动 key。

紧急吊销不能使第三方已经缓存的 RSA 公钥在数学上立即失效。必须分别报告新 JWKS／本系统在线验证的拒绝和独立第三方缓存边界。ID Token 不是业务 Bearer；业务 Access Token 由在线内省／来源检查拒绝。若事故要求同时关闭账号或 grant，必须明确执行对应撤销命令，不能声称“吊销签名 key 自动撤销全部 opaque token”。

## 3. 既有恢复工具不能直接用作新证据

| 当前文件 | 只读核实的缺口 | Phase 23 最低处理 |
|---|---|---|
| [identity-recovery/recovery.py](../../../../ops/identity-recovery/recovery.py) | `INVENTORY`、`INVALIDATION`、`rotate_credentials` 依赖旧 `oidc_clients`／`oauth_clients`／`oidc_grants`；没有完整 canonical `auth_*` | 按当前 0001 重建清单、失效及凭据轮换；旧模型不得作为 fallback |
| 同上 `capture_checkpoint` | 硬编码 deployment `schema_version='20'`；当前 canonical 为 1 | 核对当前版本 1、真实 SQLx checksum、owner/runtime 与角色 SQL，不修改历史 checksum |
| 同上 context | 旧 task UUID 目录、桥接地址、角色前缀和 three-service credential map；不是当前 HTTPS fixture 的命令行 driver | 对接 root 实际 manifest、端口、cluster、comment、私有目录及当前所有独立服务身份，先检查绑定再动作 |
| [identity-recovery/witness.py](../../../../ops/identity-recovery/witness.py) | 16 类旧 inventory，缺 canonical policy／consent／credential／key lifecycle／邀请等；HMAC 链不能发现整个目录及 CURRENT 一起回滚 | 新的有版本 typed inventory；恢复集外独立 minimum-sequence，高水位及完整性门禁；缺日志即保持隔离 |
| [recovery/invalidate.sql](../../../../ops/recovery/invalidate.sql) | 主要处理旧 OAuth 和平台 session，没有 canonical consent、grant、refresh、邀请、RP tombstone | 不能把 `credentials_invalidated=true` 输出当真实失效；逐类核对零 live 行与真实正常端点旧凭据拒绝 |
| [recovery 工具](../../../../ops/recovery/README.md) | 当前三库是 platform/stores/work_orders；门户库另列；仅加密／结构对比不处理所有新认证事实 | 写明本轮实际库集；核心身份恢复必须包含共享库和门户库，资源业务库未参与就显式排除，25 再做完整安装矩阵 |
| [snapshot-build.py](../../../../tests/unified-auth-https/snapshot-build.py) | 现白名单包括 `ops/recovery/src`，没有完整 `ops/identity-recovery`、恢复 SQL／配置／锁文件 | 实施时把实际使用恢复工具一并冻结并记录摘要，避免运行报告之外的 live 脚本 |

`identity-recovery` 是被调用的 Python primitives，当前仓库搜索未找到调用其 `capture_checkpoint`／`restore_checkpoint`／`assert_start_allowed` 的现行 HTTPS driver。不要把导入、语法检查或旧 README 的历史参数称为可执行演练。需要新增并验证当前阶段的受控 driver，再将实际命令写入正式 runbook。

## 4. canonical 恢复覆盖合同

以下是实施清单而非已运行结果。恢复工具必须使用当前 canonical 字段和一份实际表名合同；`oidc_signing_keys`、`oidc_provider_settings`、`identity_oidc_admin_receipts` 等承载当前唯一权威的对象允许保留名称，不维护两份旧／新恢复流程。

| 安全族 | 需要持久的身份／高水位／拒绝事实 | 隔离恢复后的最低断言 |
|---|---|---|
| `users`、`identity_account_revocation_facts`、`identity_account_tombstones` | account `session_version` 与退休／处置事实；完整 ID inventory | 版本严格高于 dump 与独立 witness；已注销 ID 不启用、不替换，不复用原 subject |
| 平台业务授权 | membership/identity_version、租户／组织／应用、角色菜单、应用／动作／数据范围及独立服务注册 | 只做 deny-only 删除、禁用、交集和高水位；不重建备份后新增对象，不扩大 scope／成员资格 |
| `auth_applications`、`auth_application_admissions`、`auth_resources`、`auth_scopes` | 投影／审批／资源版本、状态、用途与受众、环境、生产责任、审核及域名事实 | 备份后的停用或缩权继续生效；事实不完整的能力保持停用，生产默认关闭 |
| `auth_clients`、`auth_client_credentials`、`auth_client_permissions` | client `version/policy_version`、credential_version、revoked/expiry、scope 集合 | MACHINE 与 USER 原凭据和备份后的吊销不复活；已撤销或退役 client 不重新启用；仅给明确 fixture 重发新凭据 |
| `auth_consent_groups`、`auth_consent_group_clients`、`auth_consents`、`auth_consent_scopes` | group/version、consent generation/version/status、display_deleted，资源/scope 交集 | 备份后撤销／隐藏的同意不恢复；新授权需要当前确认；旧标签页版本仍冲突，不重置 generation |
| `auth_subject_sectors`、`auth_subject_links` | sector 归属、稳定 opaque sub、retired_at | 正常 key 轮换不改 sub；恢复不重新随机生成既有映射；退休账号不能继承旧 sub |
| `auth_authorization_requests/codes`、`auth_context_interactions` | 一次性消费、来源版本与原截止、加密结果 | 全部旧 code／pending interaction 失效；旧 original operation 不产生第二次交付或延长期限 |
| `auth_grants`、`auth_grant_scopes`、`auth_access_tokens`、`auth_access_token_scopes`、`auth_refresh_families/tokens` | consent/account/client/application/credential 代际、grant/family version、消费与撤销 | 显式失效全部恢复前在线及 offline USER 和 MACHINE grant、Access Token、refresh；原 code/refresh 重放不能恢复活性 |
| SSO、两 BFF、customer contexts、内部派生能力 | 原 source/session、account/context/browser generation、pending lease、AEAD 和 capability 版本 | 先关闭旧来源及指针，再轮换秘密；旧 Cookie、派生令牌、来源证明在原正常端点拒绝；新双 RP 可建立全新会话 |
| `auth_rp_sessions`、`auth_logout_command_receipts` | 不变 grant/client/public sid/sub 绑定与原操作结果 | 保留原 receipt，不能换 sid/sub 或新 token 恢复旧命令；不删除它们以绕过约束 |
| `platform_backchannel_logout_sources/receipts`、门户同名族 | source tombstone、已收 jti、原 issuer/client/sid/sub 摘要 | 备份后收到的 tombstone 不丢失；通知先于 callback 的原 source 仍不能插入；已处理 jti 不能再生效 |
| `auth_delivery_targets/jobs`、consent/security outbox | 精确目标/version、event/source、终态、租约及重试计数 | 不恢复已吊销目标或把 DELIVERED/TERMINAL 改 PENDING；旧 LEASED 不在恢复后无审查继续发；worker 仅对受控 collector 发当前合法任务 |
| `auth_invitations`、`auth_invitation_verifications`、`auth_invitation_activation_receipts` | 原 recipient/purpose、版本、24h 期限、消费／吊销、核验目标代际、原 operation | 备份后已消费或撤销邀请不能再次创建账号；恢复前未完成邀请保守废弃，receipt 与审计保留 |
| TOTP enrollment/factor/recovery-code、MFA receipt/approval | 一次性恢复码消费和原因子版本 | 恢复不能重启已用恢复码／旧双人证明；无法完整恢复消费事实时保持账号／因子不可用，明确安全重绑流程 |
| 签名 key 和 provider settings | immutable kid/n/e、active/published/activated、last_signed_exp highwater、retired/revoked、固定 issuer | 备份后吊销 key 不复活；旧 kid 不换材料；窗口高水位不倒退；无可证明的活动 key 保持不签发 |
| 管理命令 intent／receipt／审计 | exact operation/service/digest/method/path/key 绑定、原结果、原截止 | 旧未提交命令废弃；已提交原结果可按合同查询；换 service secret 不能重建旧 secret 结果 |

完整数据库 dump 可以包含这些表，但只恢复 dump 不保留 checkpoint 之后发生的拒绝事实。必须有备份集之外、不会同时被恢复回滚的 deny/highwater 见证；本次有限 fixture 覆盖与生产持续完整控制日志的差距单列。

### 4.1 两 BFF 与不可变事实的具体恢复顺序

以下为 Phase18 完成后补充的只读约束核对，尚未执行恢复。正式23冻结时须对当前源码重新核实；后续24变更基线后也须同步。

- 平台 `bff_sessions` 的 `refresh_lease_id/refresh_started_at/refresh_uncertain` 表达持久未知结果；`logout_request_id/logout_scope/logout_confirmed_at` 及原 opaque sid/sub 受不变触发器保护。恢复不能仅清 lease 或 uncertain 后继续使用旧来源。先撤销旧 BFF/platform session、消费旧 refresh，清 `console_browsers.current_session_id` 并抬 generation，再清旧 selected pointer／ciphertext，废弃旧 login flow。
- 平台内部 capability 对 platform session 的 FK 不允许依赖硬删级联；浏览器 current/selected pointer 没有 FK。门户 `portal_sessions` 的 pending/context lease、customer ciphertext／期限和成员／组织／版本受成对 CHECK 约束，须按整组安全更新。两端旧 cookie、flow、pointer 都不能在恢复后重新指向 live 来源。
- 两端 `*_backchannel_logout_sources` 按 `issuer/client_id/sid_digest` 唯一，保留精确 `sub_digest/revoked_at`；`*_backchannel_logout_receipts` 按 `issuer/client_id/jti_digest` 唯一。两族都没有 FK，runtime 只有 SELECT/INSERT；source 墓碑没有 expiry。120秒 logout JWT 到期不表示可以删除 source 墓碑。平台 `bff_session_active` 不直接读墓碑，因此仅恢复墓碑不足以关闭已有 live pointer。
- callback／refresh 的 source advisory lock 必须与墓碑合并顺序一致。已消费 flow、旧 generation、原 lease/version、uncertain 与原 source 始终参与拒绝；不能为了恢复可用性清掉这些事实。浏览器 `__platform_context_switch` 仅为提示，内存 pending/settled 和旧 URL 都不是恢复权威。
- `platform_app_capabilities` 的原 hash/jti/parent/member/account/application/source/deadline 不可改，旧能力明确单向撤销。已消费 `context_selections` 的 COMMITTED/result/binding 和 switch 唯一围栏保留；`resource_revoked_at` 不可清，不能按旧结果重新制造 bearer。
- `auth_rp_sessions` INSERT 要求原 active USER grant；`auth_logout_command_receipts` INSERT 校验原 RP、原 token digest，且 UPDATE 全部拒绝。不能先撤销 grant 再把这些事实当普通新行补插，也不能重建缺失 grant 以满足 FK。checkpoint 后无法安全物化的事实保留库外 witness 并继续隔离，不能通过放宽约束完成启动。
- `auth_delivery_jobs` 的 source/target/version、event、次数与租约预算不能回滚；DELIVERED/TERMINAL/DEGRADED 状态元组不可改。旧 LEASED 可能被 worker 自动重租，必须先停 worker、合并终态与 target 吊销／版本高水位，再审查恢复后的剩余任务。target 的 deny-only 规则由 driver 额外执行，不能因可 UPDATE 就把 REVOKED 改回 ACTIVE。
- SSO source revoke 不自动覆盖 offline USER/MACHINE。恢复须显式废弃全部旧 grant/access/refresh family/token，再以原正常端点验证逐类拒绝；原 operation／receipt、consumed 和 generation 高水位保留。

依据为平台／门户当前 canonical `0001`、两端 backchannel 与 callback/refresh store、内部能力和浏览器协调器。本节只固定实施约束，不构成新增动态通过证据。

## 5. 启动后可并行的实施分工

1. **密钥组**：当前 key API／真实 TOTP 管理／JWKS／签发并发和 RP 验证矩阵，完善轮换 runbook。
2. **恢复组**：canonical typed inventory、加密 checkpoint、deny-only reconciliation、start receipt 和 fail-closed guard；先做纯文件／纯数据测试，不接业务库。
3. **独立评审组**：对照当前 0001/ACL 审查遗漏族、secret 泄漏、回滚和跨任务删除门禁；补齐受控负向 fixture。
4. **root 环境组**：Phase 22 freeze 后确定 Phase 23 快照、新专属源／恢复库及角色、实际 HTTPS；只由一个环境 owner 操作进程、网关、数据库和清理。

密钥与恢复的组件工作可并行，真实同 issuer 生命周期与恢复切换必须串行，避免一组 key 激活或库恢复影响另一组正在验签。schema／ACL 由既有 owner 统一处理。24 按 root 裁决不为旧名称机械改名；若其必要退役实际影响 key/readiness/restore 合同，必须重跑相应最小验证，不用23冻结输入的结果冒充修改后通过。

## 6. 实际密钥矩阵与证据

| Case | 触发 | 必须观察 |
|---|---|---|
| K01 配置验证 | 相对路径、非法 JSON／额外字段、空／超8 key、重复 kid、不同 n/e 与私钥、错误算法／尺寸 | 启动或配置加载拒绝且无材料日志；fixture 采用独立生成 RSA，禁止把测试私钥提交到报告 |
| K02 预发布 | 先配置 A/B，当前 A 活动；真实管理发布 B | JWKS A/B；新 ID Token 仍用 A；B 未发布前激活拒绝 |
| K03 激活 | 相同原 operation 激活 B，并模拟回包丢失 | 唯一活动 B；原 operation 恢复一次；ID Token 和 logout+jwt 都用 B；A 签发的未过期合法 token 仍可验证 |
| K04 不变性 | 相同 kid 换材料、重发新 operation、退役或吊销后再激活 | 409／404 对应真实合同；不改变 kid/n/e，不出现第二个活动 key |
| K05 保留窗口 | A 仍 active、A 刚签发／窗口内退役 | 409；真实签名事务持行锁与退役并发，last_signed_exp 包含最后一枚已提交 token |
| K06 正常退役 | 切 B 后等待 A 最后 exp+60 秒之后，再退役 A | 真实时间记录，A 移出 JWKS；B 合法 token 可验；A 过期 token 拒绝。缩短 token TTL 的 fixture 与运行默认 TTL 分别写明 |
| K07 紧急吊销 | A 仍有合法未过期 token 时吊销 A，B 已可用 | 新 JWKS 和本系统 RP 拒绝 A，B 正常；错误签名/未知 kid/错误 issuer/aud/typ/算法拒绝；第三方缓存行为单列 |
| K08 无可用材料 | 激活 B 后一个候选实例缺 B 私钥、全部 key 被吊销、DB/JWKS 不可用 | 签发与验签失败关闭，保持 Phase 22 有界503／超时语义；不得回退旧 key、任意 issuer 或关闭 TLS |
| K09 权限和事务 | 错用途 secret、无 `system:oauth:keys`、陈旧 TOTP、CSRF／来源、过期 intent、审计失败、锁等待跨期限 | 无 lifecycle／receipt 部分提交；当前运行角色无额外 owner／DDL 权限；真实运行日志无 token/private key |

K01/K04/K05/K09 的组件覆盖与 K02/K03/K06/K07 的真实 identity→JWKS→平台／门户 HTTPS 证据分开。独立第三方 RP 如复用 21 产物，先核对冻结版本；没有实际运行就不计通过。

## 7. 隔离恢复演练顺序

1. **资产与权限前置**：等待22实际报告；root 记录本轮项目 identity/platform/portal、0容器、新专属源库与恢复库、精确 owner/runtime 和 comment、cluster ID、端口、可用磁盘、临时路径及清理边界。可复用 root 仍持有的适用 PG 引擎，不能推断旧库或 PID 仍存在。Phase23角色必须与22或其他业务任务隔离。
2. **配置与来源固定**：快照完整工具及当前0001/ACL，生成独立 A/B RSA、BFF/context AEAD、TOTP、邀请、各用途服务／RP／机器／资源凭据与备份加密、MAC、witness key。秘密只在0700/0600任务私有目录、内存和保护通道；各 key 不复用。
3. **建立 checkpoint 前样本**：用实际 HTTPS 建立员工、客户、ACCOUNT_ONLY、MACHINE、在线／批准的 offline refresh、授权同意、稳定 sub、一次性邀请和管理 receipt，先证明每枚待测旧凭据在其正常端点确实成功。
4. **停写一致点**：停止本轮所有写入者／worker／候选实例，核对非管理连接为0，在明确写屏障下 sequential dump 共享库与门户库、完整 owner/ACL/checksum/catalog、库外材料与业务 marker。记录开始／结束，不称跨库原子 snapshot。使用当前 PG 工具版本，禁止照搬旧 README 的 PG17 参数当作本机事实。
5. **备份后拒绝事实**：源 fixture 恢复受控运行，依次执行同意撤销及展示删除、refresh重用family撤销、客户端secret吊销／缩权、账号退休、成员删除、签名key紧急吊销、邀请消费／吊销、backchannel未知sid tombstone及投递终态。每一步写入独立完整witness并确认持久；新增一笔checkpoint后业务marker用于RPO测量。
6. **故障起点与关闭入口**：记录 monotonic 与UTC时间，从故障／停止接流即开始RTO计时；停止所有本轮writer/worker并验证排空。源库保留隔离，不拿它作恢复目标，不碰本机5432业务库或既有443服务。
7. **恢复前负向门禁**：篡改密文／manifest、错误key、混任务／cluster／issuer／库名、错owner／ACL、旧witness／CURRENT／minimum-sequence、缺链／漏族、备份材料不匹配、空间不足、非零连接、已存在目标库。全部在可行的最早阶段拒绝，入口保持关闭，不能无界重试或清掉原源库。
8. **恢复至新隔离库**：只向明确尚不存在的任务恢复目标执行 pg_restore，保留源 owner/grantor/ACL并比较；不使用 `--no-owner/--no-acl` 掩盖差异。后续所有动作再次核对目标绑定。若复用同cluster源角色，源实例必须持续停止且只允许本轮角色，禁止轮换其他阶段共享角色。
9. **当前拒绝事实对账**：在任何接流前应用完整canonical witness、恢复代际高水位、key吊销和正常验证窗口；显式废弃全部恢复前grant／code／refresh／SSO／BFF来源及未完成邀请，不删除审计／receipt／source tombstone。保留终态投递事实，旧租约不得继续发送。逐类精确行数与读回核对，trigger保持启用。
10. **秘密和账号恢复**：轮换本轮所有可恢复秘密和数据库runtime密码；保留不变sub与kid材料，旧客户端secret全部拒绝，只给明确fixture发新版本。账号/因子不能仅因dump曾启用就恢复；根据最新证据明确重设密码及MFA处理。SCRAM密码验证使用实际要求密码的原端点；trust连接成功不计轮换通过，不擅改共享HBA。
11. **启动栅栏**：start receipt绑定checkpoint、最新sequence、issuer、完整fenced inventory及floor，真实启动前复验；在receipt后再回滚库／目录必须拒绝。不满足任何族的完整性就继续QUARANTINED，不以ready marker替代恢复门禁。
12. **旧拒绝与新业务**：实际原正常端点验证旧密码、Cookie、Access/refresh/MACHINE/service/RP/资源secret、code、邀请、已用恢复码和旧pending operation均拒绝；旧token用新合法secret内省应inactive。再经受信任CA、相同唯一issuer完成全新员工/客户/第三方/机器流程，敏感写入与最小scope仍受控；worker只面向任务collector。
13. **时间与清理**：只有对账、旧拒绝、完整配置重启和新双RP业务均成功才结束RTO。记录业务marker真实丢失和checkpoint龄得到RPO；主动废弃认证状态单列。成功／失败／中断均由finally清理任务源／恢复库、角色、密钥、备份、witness、日志和临时故障文件；保留脱敏正式报告。root最终检查0残留。

上述步骤需先实现第3节列出的当前driver和合同，现阶段没有声称存在一条可直接运行的完整恢复命令。正式runbook必须收录实际driver的精确 `--help`、参数、前置检查、失败退出与cleanup命令，不提供把旧脚本直接指向新库的替代操作。

### 7.1 同集群真实 SCRAM 验证方案（尚未实施）

root补充的环境事实：当前 `127.0.0.1:55494` 原生PostgreSQL完全由本goal创建并持有；现有loopback使用trust。源码 [postgres.sh](../../../../tests/unified-auth-https/postgres.sh) 也声明 `initdb --auth=trust`、`listen_addresses='127.0.0.1'`。本段只评估方案，没有读取或修改活动HBA、连接数据库或启动代理。

可由root作为唯一环境owner，在Phase23正式启动后使用同一PG，避免第二PG和Docker：一个task-owned透明TCP代理监听尚未占用的 `127.0.0.1:<专用端口>`，其连接PG的出站socket显式bind到 `127.0.0.2`，目标保持 `127.0.0.1:55494`。PG看到的是 `127.0.0.2` 来源；`listen_addresses` 约束目标地址，不要求把源地址加入监听。代理及PG必须处在同一Linux网络namespace；不能经Windows localhost转发后假设源地址仍保留，正式验证时用成功连接的 `inet_client_addr()` 证明。

| 门禁 | 正式实施时的条件与证据 |
|---|---|
| 目标归属 | 再核对实际cluster ID、data_directory、hba_file、任务OWNER marker、Phase23库/角色/comment及代理端口归属；只操作本轮精确LOGIN角色，不对所有用户或其他阶段库加规则 |
| 第一匹配规则 | 在任何可能匹配该来源的trust规则之前插入精确数据库列表、精确Phase23角色列表、`127.0.0.2/32`、`scram-sha-256`的窄host规则。原`127.0.0.1`trust和其余规则字节保持不变，不能写`all all`或扩大地址范围 |
| 密码验证器 | 为Phase23角色创建/轮换密码时显式 `password_encryption='scram-sha-256'`；管理员只记录verifier具有SCRAM前缀的布尔证明，不输出`pg_authid.rolpassword`、PHC或密码。HBA写SCRAM本身不证明数据库已有合适verifier |
| 生效证明 | baseline记录HBA摘要、owner/mode和受控副本；插入后保存expected-current摘要，核验 `pg_hba_file_rules` 无解析error，再reload。reload成功不单独计PASS，必须新建真实连接完成正反例；现有pool连接不会重新认证 |
| 轮换前正反例 | 在同一代理端点、同一db/user/连接选项下：原密码新连接成功，固定错误密码新连接以实际认证失败拒绝；成功查询核对 `inet_client_addr()='127.0.0.2'`、准确current_user/current_database。不得回退Unix socket、IPv6、原55494或其他service配置 |
| 轮换后正反例 | 清掉旧连接后，在同一路径验证旧密码失败、新密码成功、固定错误密码失败；负例要识别真正password authentication failure/SQLSTATE `28P01`，不能把超时、代理停止、HBA不匹配或数据库不存在记作密码拒绝。客户端若无法给出结构化SQLSTATE，使用能提供该状态的当前DB客户端 |
| 秘密与协议 | proxy只做有界字节转发，不解析或记录认证流；不使用命令行密码、trace、body dump或端口抓包。临时凭据文件0600且不进入报告；原连接TLS策略保持，不能为代理关闭原有证书校验 |
| 并发与还原 | root独占HBA修改窗口。finally先停止本轮使用代理的客户端/连接，核对当前HBA仍是expected-current摘要及原owner/mode，原子还原baseline、reload并核对摘要及解析结果，然后关闭proxy进程、连接、监听及其临时配置/秘密文件。若hash已被其他写入改变，禁止盲覆盖，root须对账后定向移除本轮规则 |

该方案仅证明本轮精确角色经指定SCRAM路径的真实密码轮换。原trust路径仍在，不声称整个集群已经改成密码认证，也不把它描述为生产数据库网络安全配置。如果因namespace、HBA权限、有效SCRAM verifier、真实认证错误识别或清理核对而无法完成，则数据库密码拒绝项明确为 `NOT_TESTED`，trust成功不计PASS；其他不依赖它的key／恢复用例可以继续形成独立证据。

## 8. 输出、计时和验收记录

建议正式交付：`phase-23-key-rotation-runbook.md`、`phase-23-recovery-runbook.md`、`validation/phase-23.md`，以及实际命令／源码和二进制SHA／逐case状态／UTC与monotonic耗时／资源清单／cleanup receipt的脱敏JSON。正式执行时由owner决定最终文件名，本文不创建空PASS报告。

| 台账验收 | 计划证据 | 当前结果 |
|---|---|---|
| 发布、激活、窗口与紧急撤销 | K01–K09，真实管理、JWKS与签发事务 | NOT_RUN |
| 并存合法key可验，错／撤销key拒绝 | K02/K03/K06/K07＋两RP及可选21独立RP | NOT_RUN |
| 撤销、注销和失效来源不复活 | 完整canonical inventory、备份后变更、外部witness和实际旧端点拒绝 | NOT_RUN；旧恢复工具存在明确缺口 |
| 实测RTO/RPO、目标差距及清理 | 完整故障到新业务时长、真实marker、finally cleanup及独立残留查询 | NOT_RUN |

候选目标只用于比较：**RPO≤15分钟、RTO≤4小时**。一次小fixture的实测不能证明生产持续RPO、WAL/PITR、异地保管、备份频率、真实值班响应或99.9%可用性；未配置这些能力时按事实列明差距。备份大小、key数量、账号／grant／outbox样本量和机器规格必须随耗时披露。

## 9. 本次准备的资源与台账建议

本次只读取源码、文档、Git状态并新增本文；没有启动应用／数据库，没有运行测试，没有创建或复用容器，没有使用Phase20环境，没有生成私钥／备份或删除资源。Docker磁盘占用变化未测量，不能声称回收空间或缩小WSL磁盘。

建议台账保持 `Phase 23: NOT_STARTED`，可将本文作为准备链接；在Phase22当前证据与启动授权到位前不推进状态。实施开始时优先完成canonical恢复合同与driver，密钥矩阵可并行开发；正式验收须满足四项标准后才建议 `COMPLETED`。本准备工作没有阻断或暂停root已授权的其他阶段。
