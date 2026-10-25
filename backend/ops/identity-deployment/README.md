# 独立身份服务的同库 owner 交接

本目录是 Phase 12 的单次受控部署作业与权限核验材料。它不运行第二套 schema 迁移器，不代表生产环境已经启用或达到 SLO。公开 issuer 必须保持原值；所有角色、数据库和 deployment UUID 都由实际部署明确提供。

部署顺序为：停止两个 runtime，备份并记录恢复点；由唯一 canonical migration job 应用平台 `migrations/0001`～`0020`；在同一个明确数据库完成角色／owner／权限交接并写入 ready marker；验证成功后再启动零 DDL 的身份和平台进程。runtime 不执行迁移、seed、地区初始化或角色授予。

`create-roles.sql` 需要独立管理连接与 `role_prefix`，只创建此前不存在的六个角色。前缀限定为 6～32 位小写 ASCII 字母、数字、下划线，首位字母；碰到任何重名直接失败。四个 owner 都是 `NOLOGIN NOINHERIT`，两个 runtime 是 `LOGIN NOINHERIT`，六者都无超级用户、建库、建角色、复制或绕过 RLS 能力，彼此没有 membership。LOGIN 初始没有密码；运维通过受保护的密码通道配置独立强密码，勿将凭据写入本目录、命令行或证据。

| 角色后缀 | 归属与权限 |
| --- | --- |
| `_migration_owner` | DB、public schema、SQLx history、ready state、共同 UUID 检查；NOLOGIN，仅受控迁移 job 操作 |
| `_identity_owner` | 真实 `users`、SSO／OIDC、服务登记、身份审计、命令 receipt、tombstone；也是明确合并的 projection owner |
| `_platform_owner` | 成员、租户／组织、角色／菜单、legacy OAuth、平台 session、BFF、客户 context 与命令 intent |
| `_bridge_owner` | 仅拥有两个账号失效 trigger 函数；只有五类平台凭据的必要 SELECT 与失效列 UPDATE，以及认证侧精确代际撤销计数 fact 的受控 INSERT／累加 |
| `_identity_runtime` | 账号认证列与身份私有状态的必要 DML，无法读取／修改 `is_super_admin`；无法删除账号、修改主体 ID、TRUNCATE 或访问平台授权表 |
| `_platform_runtime` | 平台表 DML、账号非 PHC 列 SELECT、仅 `UPDATE(is_super_admin)`，后者保留既有账号行锁资格；无法读取 PHC 或直接修改任何认证列／私有认证表 |

`users` 始终保留真实表、原 UUID 和外键，避免通过 view 替换破坏 FK／稳定 `sub`。认证 runtime 的 `INSERT` 明确排除 `is_super_admin`，其默认 false；没有账号 DELETE 权限，因此不能通过既有级联外键删除成员或平台数据。所有 owner 都不能由 runtime `SET ROLE` 或继承。平台只读账号展示并不授予对外全量目录权限；既有员工／客户资源范围校验仍须执行。

`provision.sql` 接收 `expected_database`、canonical UUIDv7 `deployment_id`、固定 `issuer`、`role_prefix`。当前数据库的 comment 必须预先由部署 owner 明确设置为 `identity-deployment/1/<deployment_id>`；脚本验证数据库名、标记、issuer、真实 SQLx 0020 成功历史和全新角色闭包，ready state 已存在时拒绝再次 provision。它只接受 canonical public schema 布局，对额外业务 schema 直接失败，要求运维先明确处理。

脚本撤销目标数据库 PUBLIC 的 CONNECT／CREATE／TEMP、public schema 的 PUBLIC 权限、所有 public 表／序列／函数的 PUBLIC 权限，并关闭未来迁移对象的 PUBLIC 默认权限。两个 runtime 只获 CONNECT／schema USAGE；没有 schema／DB CREATE、TEMP、表 owner、TRUNCATE、TRIGGER 或 REFERENCES 权限。扩展成员保留迁移／扩展 owner，不改写扩展内部函数；平台仅得到现有搜索／索引函数的 EXECUTE。角色不应在别的部署数据库复用；专用实例或 `pg_hba.conf` 应按数据库／角色限定连接，不能把此目标库的 ACL 误称为整个 PostgreSQL 实例其他数据库的 ACL。

跨 owner 只保留可审计的窄函数：

- 两个旧账号 trigger `revoke_sessions_on_user_disable()` 和 `revoke_credentials_on_account_generation()` 由 `_bridge_owner` 持有，以固定 `pg_catalog, public` search_path、全限定表名执行，只写 revoked／consumed／EXPIRED 等失效列。runtime 没有直接 EXECUTE，更没有任意 target 的通用回收 helper。
- `bff_oidc_proof_active(uuid,uuid,uuid,bigint,uuid,bigint,text)`、`customer_portal_account_source_active(text,text,uuid,uuid,uuid,uuid,bigint,bigint,bigint,timestamptz)`、`oidc_console_source_active(text,text,bigint,text)` 由认证 owner 以固定 search_path 的 SECURITY DEFINER 返回 bool，平台仅获指定签名 EXECUTE。它们只能最终拒绝旧原始来源，不能替代逐请求 HTTP online proof；认证／DB 不可用仍为 503。
- `identity_lock_oidc_source(text,text)` 只按 canonical client ID 与 pinned issuer 锁定登记行，返回 bool；不读取／返回 client secret，锁保持至调用事务结束。
- `platform_account_command_intent_active(uuid,uuid,bigint,text,text)` 和 `platform_password_context_active(uuid,uuid,uuid,uuid,bigint)` 由平台 owner 持有，仅身份 runtime 可 EXECUTE。前者锁定精确 durable intent，后者在身份先锁账号后，依次锁原 member、原 session 并复验原完整员工来源，锁持有到账号 mutation commit；不返回组织权限、不修补当前代际。
- `identity_oidc_command_intent_active(uuid,uuid,text,text,text)` 同样由平台 owner 持有且仅身份 runtime 可 EXECUTE。它核验原 operation／service／digest／HTTP method／完整内部 path 与 60 秒的新命令接受期限；平台提交 intent 是已完成新鲜 root 员工与 `system:oauth` 授权的接受点，身份服务不重新读取平台角色。已完成的认证 owner receipt 查询不受后续 intent 过期影响。

账号管理 intent 只保存 HMAC digest 和明确 nonsecret followup，不能保存密码、PHC、加密密码、token 或服务 secret。账号 owner 原子提交 mutation 与精确 operation／service／digest receipt；未知结果只能查 receipt 或显式重试查询，不能把不确定性当作新 mutation 许可。认证审计使用独立 `identity_audit_logs`，不调用会依赖平台组织／应用表的 audit helper。

服务登记的 `secret_hash` 有 UNIQUE 约束，禁止不同 service／purpose 复用同一内部密钥。OIDC 管理 receipt 另绑定原 service secret hash、method、完整 path 和 digest，仅持久化去除新发 client secret 后的安全 result；原 service 使用尚未轮换的服务密钥，可在受认证的 receipt 查询中按固定 HMAC 域重建同一新发 client secret；receipt 自身从不保存该 secret。服务密钥已轮换时旧创建结果返回 409，必须按明确重新配置流程处理，不用新密钥导出冒充原结果。

`identity_account_revocation_facts` 由认证 owner 持有，两个失效 bridge 以账号／新代际复合键记录本事务实际影响的 session／OAuth token 行数。后执行 trigger 的零行不会覆盖之前的计数；身份 runtime 只有 SELECT，可在同一 mutation 事务将真实计数写入 receipt，无需访问平台原始凭据表。只有代际实际变化的命令消费该代际的新 fact；创建和只改基础资料等无代际变化的命令不得把既有历史 fact 当作本次撤销计数。runtime 无权改写 fact，bridge 对该表仅有必要 INSERT 与两个计数列 UPDATE。

`verify.sql` 用相同四个变量只读检查 marker、角色闭包、owner、列权限、PUBLIC EXECUTE 与 definer search_path，成功仅返回安全 receipt。实际服务还必须核对 ready 行里的 `current_user` 对应 runtime 名称、固定 issuer 和 `schema_version=20`；ready marker 不替代数据库不可用处理。把只读 catalog 验证、真实双角色允许／拒绝测试、双 RP 登录注销、503、密钥轮换与恢复演练的结果写入本阶段验收证据后，才可以宣称部署通过。
