# 共享平台库的运行角色与权限

平台和身份服务共享物理数据库，使用不同 owner 和受限 runtime。唯一 canonical migration job 先在空库应用当前 `0001_init.sql`，此后执行受控角色/ACL 交接，再启动零 DDL 的 runtime。

`create-roles.sql` 使用明确 `role_prefix` 创建六个此前不存在的角色；同名直接失败。四个 owner 为 NOLOGIN/NOINHERIT，identity/platform runtime 为 LOGIN/NOINHERIT；所有角色都无超级用户、建库、建角色、复制和绕过 RLS 权限，不能彼此继承。

| 角色后缀 | 归属 |
| --- | --- |
| `_migration_owner` | 数据库、schema、SQLx 历史与 readiness |
| `_identity_owner` | 账号、SSO/OIDC、服务登记、身份审计和命令 receipt |
| `_platform_owner` | 租户组织、成员、授权、应用、会话/BFF、通知与业务 intent |
| `_bridge_owner` | 账号失效触发器和精确凭据撤销所需最小权限 |
| `_identity_runtime` | 认证列与私有身份数据的必要 DML，不能读取平台角色或修改管理员政策 |
| `_platform_runtime` | 平台业务数据，不能读取密码哈希或直接修改认证私有表 |

通过受保护通道设置独立 runtime 密码。`provision.sql` 接收 `expected_database`、UUIDv7 `deployment_id`、固定 `issuer` 和 `role_prefix`，要求数据库 COMMENT 为 `identity-deployment/1/<deployment_id>`，SQLx 历史为成功的当前版本 1。脚本核对全新角色闭包，撤销 PUBLIC 的广泛权限，明确分配 owner/ACL，最后写入 schema_version=1 的 ready marker。

认证与平台之间只暴露签名固定的窄 projection、来源锁和命令 intent 函数。SECURITY DEFINER 使用固定 search_path，runtime 不能读取上游 secret；在线 HTTP authority 仍是必要条件。账号撤销 bridge 只失效相应代际凭据并记录实际计数，不能扩大平台业务权限。

`verify.sql` 使用相同四个参数，只读核验数据库、marker、角色、owner、列权限、PUBLIC EXECUTE 与 definer search_path。两个 runtime 分别核对 current_user、固定 issuer 和当前 schema_version；任一失败均不进入服务状态。

同库权限交接不会为全新环境自动登记账号、RP 或服务客户端。这些由明确的初始化配置准备，不从旧部署导入。部署后还需验证真实双角色允许/拒绝、SSO/RP、回调、撤销与故障关闭。
