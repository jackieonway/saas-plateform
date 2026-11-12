# 管理员设备与会话

平台根组织通过“开放平台 → 设备与会话”（`/system/sessions`）查看平台登录设备和按用户汇总的会话。查询需要 `system:session`，吊销额外需要 `system:session:revoke`。租户和非平台根组织不能使用全局会话管理；个人中心的本人设备管理独立保留。

| 接口 | 行为 |
| --- | --- |
| `GET /api/admin/sessions` | 分页查询平台设备，支持 `keyword`、`activeOnly`、`page`、`size` |
| `GET /api/admin/session-users` | 按用户汇总平台会话，支持相同过滤和分页参数 |
| `POST /api/admin/sessions/{id}/revoke` | 撤销指定平台设备及绑定的 BFF 会话和本地刷新能力，其他设备不受影响 |
| `POST /api/admin/session-users/{id}/revoke-all` | 通过认证中心的 `REVOKE_ALL` 命令撤销账号全部认证会话与授权令牌，再按原账号代际收敛平台会话 |

设备吊销与 `ADMIN_REVOKE_SESSION` 审计在同一事务内，重复操作不重复记录；审计失败会回滚设备、BFF 和刷新能力的撤销。全账号吊销沿用持久化账号命令和 `KICK_SESSIONS` 审计，未知结果保留原操作编号，通过原账号操作恢复接口查询／完成，不重新提交。页面按登录上下文保存待恢复操作，并在操作前再次检查上下文与权限。

初始化语句 [`0001_platform.sql`](../initialization/0001_platform.sql) 包含地区管理、设备与会话及吊销按钮，并在同一事务内授予平台租户、总部组织和管理员角色。已有库再次运行 bootstrap 只核对已有初始化，不补写菜单；需要按明确的运维变更授权更新菜单，或在获准重置的专用库执行全新初始化。

回归检查：`cargo test --locked --test platform_bootstrap -- --ignored`；`cargo test --locked --bin saas-platform-backend routes::sessions::tests:: -- --ignored`。两者使用 `TEST_DATABASE_URL` 为自己创建并删除测试库。前端回归为 `node --test tests/session-management.mjs`，也纳入 `npm test`。
