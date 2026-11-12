# 共享平台库的运行角色与权限

平台和身份服务共享物理数据库，使用不同 owner 和受限 runtime。唯一 canonical migration job 先在空库应用当前 `0001_init.sql`，此后执行受控角色/ACL 交接，再启动零 DDL 的 runtime。

`create-roles.sql` 使用明确 `role_prefix` 创建六个此前不存在的角色；同名直接失败。四个 owner 为 NOLOGIN/NOINHERIT，identity/platform runtime 为 LOGIN/NOINHERIT；所有角色都无超级用户、建库、建角色、复制和绕过 RLS 权限，不能彼此继承。

| 角色后缀 | 归属 |
| --- | --- |
| `_migration_owner` | 数据库、schema、SQLx 历史与 readiness |
| `_identity_owner` | 账号、SSO、统一认证 `auth_*` 模型、服务登记、身份审计和命令 receipt |
| `_platform_owner` | 租户组织、成员、授权、应用、会话/BFF、通知与业务 intent |
| `_bridge_owner` | 账号失效触发器和精确凭据撤销所需最小权限 |
| `_identity_runtime` | 认证列、统一认证 `auth_*` 模型的必要 DML，不能读取平台成员/角色或修改平台管理员政策 |
| `_platform_runtime` | 平台业务数据，不能读取密码哈希或直接读写身份 `auth_*` 私有表 |

通过受保护通道设置独立 runtime 密码。`provision.sql` 接收 `expected_database`、UUIDv7 `deployment_id`、固定 `issuer` 和 `role_prefix`，要求数据库 COMMENT 为 `identity-deployment/1/<deployment_id>`，SQLx 历史为成功的当前版本 1。脚本核对全新角色闭包，撤销 PUBLIC 的广泛权限，明确分配 owner/ACL，最后写入 schema_version=1 的 ready marker。

认证与平台之间只暴露签名固定的窄 projection、来源锁和命令 intent 函数。SECURITY DEFINER 使用固定 search_path，runtime 不能读取上游 secret；在线 HTTP authority 仍是必要条件。账号撤销 bridge 只失效相应代际凭据并记录实际计数，不能扩大平台业务权限。

员工与客户来源 projection 共用统一 `auth_*` grant、客户端、应用投影、同意组和原 SSO 代际的身份 owner 校验主体，分别以固定 EMPLOYEE/CUSTOMER 用途调用；平台 runtime 无权直接调用通用主体。客户包装还绑定原 access 摘要、认证时间、openid/profile 与准确截止，员工来源不能成为客户来源。来源锁按应用、客户端、同意组顺序持有共享锁，直到平台调用方事务完成。`bff_context_member_matches` 只返回本地员工业务会话是否仍绑定原 grant 选定的成员，不返回身份私有行，也不让身份 owner 读取业务成员表。业务请求仍要校验平台自身的成员、组织、租户和会话版本。

上下文与资源私有服务认证通过 `identity_credential_digest_in_use` 检查凭据是否复用了统一客户端或身份私有服务的 secret 摘要；该身份 owner 函数只返回布尔值。平台 runtime 仅有执行权限，认证与事务内复验均不直接查询上游凭据表。身份服务自身验证独立用途和准确 allowlist，不调用此跨 owner 防复用检查来拒绝自己的登记。

身份 runtime 对 `identity_service_clients` 保持只读，不能自行更新或锁定凭据行。来源证明事务通过身份 owner 的 `identity_context_source_service_active` 校验原服务 ID、摘要、用途和精确客户端范围并持有共享锁；等待期间发生的停用或轮换在取得锁后重新校验。该函数只允许身份 runtime 执行，不向平台开放服务登记表。

平台应用最小投影使用单独登记的 `APPLICATION_PROJECTION` 服务用途和 `IDENTITY_PROJECTION_SERVICE_CLIENT_ID` / `IDENTITY_PROJECTION_SERVICE_CLIENT_SECRET`；它与`KEY_ADMIN` 凭据分离。该私有端口只接受绑定 platform intent、operation ID、摘要、应用路径和投影版本的命令，不传递成员、租户或角色数据。第三方应用在 Phase 19 审核账号绑定前保持 `PENDING_REVIEW`，不能经此接口直接批准。

统一客户端管理使用独立 `AUTH_ADMIN` 服务用途和平台 `IDENTITY_AUTH_ADMIN_SERVICE_CLIENT_ID` / `IDENTITY_AUTH_ADMIN_SERVICE_CLIENT_SECRET` 配置；它既不复用 `KEY_ADMIN`，也不复用 `APPLICATION_PROJECTION`。该服务只操作身份 owner 的 `auth_*` 客户端/资源/scope/凭据记录。客户端 secret 只在原 operation 的一次性返回/恢复 DTO 出现，receipt 保存前会移除；凭据轮换让旧 secret 最多重叠 24 小时，紧急撤销会推进客户端策略版本。角色交接脚本不会自动生成这些 service client，须由受控初始化配置登记用途和秘密。

平台现仅开放客户端管理，不再提供签名密钥管理代理，也无需为该页面配置 `KEY_ADMIN`。认证中心仍使用独立部署的签名材料、JWKS 和既有密钥生命周期事实完成协议签发与校验；这些内部能力不构成平台管理入口。

管理员 TOTP 恢复使用另一个独立的 `AUTH_SECURITY` 服务用途；该凭据不能复用上述服务密钥。平台在 Phase 19 接入时必须先校验两名不同管理员的当前角色、近期密码认证和 TOTP step-up，再为每名审批人签署与目标账号、操作 ID、审批 ID 及理由摘要绑定的 HMAC 证明。没有配置该服务身份时，两人恢复接口保持不可调用。

合作方人工审核、生产接入发布及账号邀请已从平台移除，相关公开管理路由和菜单权限不再提供。平台无需配置 `AUTH_REVIEW` 或 `AUTH_INVITATION` 的服务凭据；schema 中保留的审核、邀请、来源与审计结构属于共享认证边界，不能据此推断已移除功能仍有可用入口。

`verify.sql` 使用相同四个参数，只读核验数据库、marker、角色、owner、列权限、PUBLIC EXECUTE 与 definer search_path。它还核对每张 `auth_*` 表归 `_identity_owner` 所有、identity runtime 仅有各表所需的 DML（append-only receipt／来源记录没有 UPDATE/DELETE）、platform runtime 没有表/列权限。两个 runtime 分别核对 current_user、固定 issuer 和当前 schema_version；任一失败均不进入服务状态。身份 readiness 复核同一新结构与权限，并确认平台仍不能读取认证秘密。

同库权限交接不会自动登记账号、RP 或服务客户端。全新环境应在 canonical migration 完成后、执行权限交接前，以管理连接运行 `platform-bootstrap`，初始化一个平台租户、根组织和超管及基础菜单授权；密码由 `BOOTSTRAP_ADMIN_PASSWORD` 明确传入，用户名默认 `admin`。命令和数据 SQL 见[全新安装](../../docs/fresh-installation.md#平台租户与超管初始化)。RP 和服务客户端仍由当前部署明确登记。部署后还需验证真实双角色允许/拒绝、SSO/RP、回调、撤销与故障关闭。


资源后端使用平台 owner 的 `platform_resource_services` 认证独立私有调用方。`APP_RESOURCE` 服务登记准确目标业务应用、`audience=applications.code`、成员类型及 `resource:action` allowlist；客户资料独立使用 `CUSTOMER_PROFILE`，类型准确为 CUSTOMER，操作准确为 `customer:profile`。`CUSTOMER_PROFILE_CLIENT_REF` 配置继续保留名称，但指向新私有服务的 UUID，而不是 OAuth client。服务任何用途、密钥、应用、受众或范围变化都推进 `credential_version`，不能通过重新启用修复原客户 context。

平台到身份的资源证明另用 `RESOURCE_PROOF_SERVICE_CLIENT_ID` / `RESOURCE_PROOF_SERVICE_CLIENT_SECRET`，身份登记用途为 `RESOURCE_ACCESS_PROOF` 且 `allowed_auth_resources` 精确列出可证明的 canonical `auth_resources.id`。该服务不能复用账号命令、客户证明、上下文来源、管理审核／邀请或 RP secret。`platform_resource_bindings` 明确每个目标业务应用资源动作的 canonical key、audience、scope；它不是 OAuth scope 签发器。MACHINE 必须另有 `platform_machine_delegations` 的明确源 client／OAuth 应用、目标业务应用、原凭据版本及数据范围登记；空表就是拒绝。

四个资源／能力表归 platform owner，身份 runtime 无读写权限；平台 runtime 使用自己的最小 DML 和中央 `platform_app_capability_active` / `platform_machine_scope_valid`，仍无 auth_* 表权限。当前验证器与私有 DTO 见 [资源能力合同](../../docs/designs/unified-auth/phase-15-resource-capability-contract.md)。

`context_selections` 的 120 秒截止只约束一次性选择凭证。已消费记录保留不可变原来源、成员与 SWITCH fence，供后续外部资源和客户续期复验；普通 runtime DELETE 被数据库约束拒绝。定时清理仅移除未消费的过期行。RP／源应用授权撤销会永久标记原记录，重新启用或重新授权不能清除该标记；安全归档须由独立受控流程核对保留义务，不放宽普通 runtime 的删除权限。

Phase 20 的注销与投递继续在同一 canonical `0001` 中建立。`auth_rp_sessions` 将原 USER grant/client/account/SSO 与随机公开 RP sid、pairwise sub 精确绑定，runtime 只能 SELECT/INSERT，不能替换来源。当前签发器生成 64 位小写 hex 的公开 sid；消费者按 opaque 标识处理，不能作为内部 SSO UUID；两个 BFF 保存不可变 `oidc_public_sid/sub`，重放账本及未知 sid 的撤销事实独立持久化，防止延迟回调重新建立已退出的来源。

| 数据 | owner | 对应 runtime 最小权限 |
| --- | --- | --- |
| `auth_delivery_targets`、`auth_invitation_verifications` | identity owner 核验事实 | identity SELECT；平台无权限 |
| `auth_rp_sessions`、`auth_logout_command_receipts`、`auth_invitation_activation_receipts` | identity owner 原来源／幂等 receipt | identity SELECT/INSERT；平台无权限 |
| `auth_delivery_jobs`、consent/security source outbox | identity owner 投递事实 | identity SELECT/INSERT/UPDATE，无 DELETE；平台无权限 |
| `platform_backchannel_logout_receipts/sources` | platform owner 接收／来源撤销事实 | platform SELECT/INSERT，无 UPDATE/DELETE；identity 无权限 |

owner-only 核验事实不能授给 runtime UPDATE。需要事务共享锁时，identity runtime 只调用准确 tuple/version 的 `identity_delivery_target_active` 和 `identity_invitation_verification_active` 布尔 fence；函数归 identity owner、固定 search_path，先锁 target 再锁 verification，并在锁等待后用新 statement 重新核对截止。它们不写核验事实，也不让平台读身份数据。内部派生 job ID、enqueue 和来源失效 trigger 支持函数不授给任何 runtime 作为任意目标 RPC。

grant 撤销事务原子创建一个稳定 event_key 的 LOGOUT job，账号／SSO 失效统一推进 canonical USER grants；consent 和 security source outbox 原子展开为中央 jobs，网络不在撤销事务内执行。没有 back-channel 或可信收件渠道时明确 DEGRADED；lease、重试和终态由统一 worker 按[Phase 20 合同](../../docs/designs/unified-auth/phase-20-logout-delivery-contract.md)处理。邀请领取 receipt 还保存原邀请、verification、target 版本和新账号，CONSUMED 必须绑定这组准确事实，不能用服务 ID 或任意 verification UUID 冒充公开领取。

`identity_phase20_schema_ready()` 仅查询 catalog 和部署 marker，两个 runtime 都可执行；它核对完整最小 ACL、owner、来源函数用途和 BFF 不可变字段，不能读取账号秘密或业务成员。provision、verify、平台启动和身份 readiness 共同执行此闭包检查。门户独立库使用自己的五张业务表和只读 schema readiness，见[门户说明](../../../../saas-customer-portal/backend/README.md)。

## 私有资源服务预算

平台的私有资源认证使用每进程一分钟窗口：`RESOURCE_SERVICE_REQUESTS_PER_MINUTE` 默认 600（每个已认证服务），`RESOURCE_SERVICE_AUTH_FAILURES_PER_MINUTE` 默认 30（每来源 IP 的认证失败），`RESOURCE_SERVICE_ATTEMPTS_PER_MINUTE` 默认 3000（每来源 IP 的请求尝试）。取值范围分别为 1..100000、1..10000、1..1000000；身份键数量上限或锁异常同样拒绝。多副本下这些进程预算分别生效，不能当作全局限额；由部署入口补充整体预算。它们限制平台私有资源调用，不是另一个公开 OAuth 内省端点配置。
