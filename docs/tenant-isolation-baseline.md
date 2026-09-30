# 租户隔离静态基线与验证记录（Phase 1）

> 日期：2026-09-30
> 方法：读取当前源码与迁移；未修改授权算法或业务数据。
> 运行环境：本机未发现 `psql`，`127.0.0.1:5432` 与 `127.0.0.1:3001` 均未监听，因此本文不声称完成实时数据库/API 验证。

## 1. 已核对的现有防线

- 登录后的组织选择通过 `memberships` 校验直接成员身份，并在签发会话时检查组织/租户到期时间；见 `backend/src/routes/auth.rs::context`。
- 普通用户的菜单和权限由角色菜单、当前组织菜单、所属租户菜单求交；见 `backend/src/routes/auth.rs::menus` 与 `backend/src/state.rs::permissions_of`。
- 租户、组织、部门、用户和角色的部分管理路由已有平台上下文或当前租户范围检查，例如 `state.require_platform`、`ensure_user_in_scope`、`ensure_role_in_scope`。这不足以证明所有路径均隔离。
- 创建组织/部门等操作有父节点归属应用层校验；数据库层主要是独立外键。Phase 2 应在审计存量后补结构约束。

## 2. 发现的风险与后续归属

| ID | 证据 | 风险 | 建议阶段 |
| --- | --- | --- | --- |
| R-01 | `backend/src/extractors.rs::CurrentUser` 解析 session 后只查 `users` 状态；不复核成员、组织/租户状态或到期。 | 被移除的成员或失效组织可在旧 JWT 有效期内继续使用部分接口。 | Phase 4 |
| R-02 | `backend/src/state.rs::permissions_of` 从角色祖先链计算权限，未检查当前用户是否仍为目标组织成员；进程内缓存 30 秒。 | 撤销成员关系后，旧 session 的权限结果仍可能存在。 | Phase 4、6 |
| R-03 | `backend/src/routes/users.rs::list` 先按租户筛出用户，再按用户 ID 无租户过滤地填充 `memberships`；`get_memberships` 也返回目标用户全部成员关系。 | 同一用户属于两个租户时，一个租户的管理员可能看到另一租户组织及部门信息。 | Phase 3 |
| R-04 | `backend/migrations/0001_init.sql` 中组织父节点、部门父节点、成员部门、角色租户和角色组织是独立外键。 | 数据库直写或遗漏应用层检查时可产生跨租户/组织关系。 | Phase 2 |
| R-05 | `backend/src/routes/oauth2.rs::session_token` 通过 `appCode` 查应用和 OAuth 客户端，但没有再次要求当前租户与组织的应用授权。 | 持有任意有效 session 的用户可尝试获取未授权应用的令牌。 | Phase 5 |
| R-06 | `backend/src/routes/external.rs::menus` 查客户端关联应用的全部启用菜单；`roles` 查该应用所有获授权组织的角色概览。 | 普通应用令牌可能看到超出当前用户/组织范围的配置。 | Phase 7 |

以上是源码可直接确认的行为或风险；实际可利用范围、已有数据异常数量和修复结果需在可控数据库/API 环境运行验证后补充。

## 3. 两租户验证矩阵

测试夹具 `tests/tenant-isolation-policy-fixture.sql` 在临时表中建立：租户 A/B、各自至少两层组织、同时加入 A/B 的共享用户、只加入 A 的用户、已无成员关系但残留角色的用户，以及不同应用和菜单授权。

| 用例 | 目标结果 | 当前算法基线预期 |
| --- | --- | --- |
| 共享用户处于 A 子组织，继承 A 根组织角色访问 A 菜单 | 允许 | 允许 |
| 共享用户处于 B 子组织，继承 B 根组织角色访问 B 菜单 | 允许 | 允许 |
| A 用户请求 B 组织的 B 菜单 | 拒绝 | 拒绝 |
| A 组织上下文请求 B 应用菜单 | 拒绝 | 拒绝 |
| 菜单仅有租户授权，没有当前组织授权 | 拒绝 | 拒绝 |
| 用户成员关系已删除但残留 A 角色，使用旧 A 上下文 | 拒绝 | 当前权限 SQL 可能允许 |
| 组织已禁用但角色及菜单授权仍在 | 拒绝 | 当前权限 SQL 可能允许 |

夹具输出 `expected`、`current_allowed`、`target_allowed` 和 `CURRENT_GAP/CURRENT_MATCH`。它验证目标矩阵的一致性并暴露现有权限 SQL 的已知缺口；**它不替代实际 Axum API 集成测试**。Phase 3/4/5/7 应扩展到真实服务请求。

## 4. 只读存量审计

`docs/sql/tenant-relationship-audit.sql` 检查：组织父节点跨租户、组织/部门树环路、部门父节点跨组织、成员部门跨组织、角色租户与组织归属不符、`user_roles` 上下文不符、组织应用/菜单缺租户授权、角色菜单缺组织授权。脚本以 `BEGIN READ ONLY` 运行并 `ROLLBACK`，不修改数据。

在有 PostgreSQL 客户端且有数据库只读权限的环境运行：

```bash
psql -X -v ON_ERROR_STOP=1 "$DATABASE_URL" -f docs/sql/tenant-relationship-audit.sql
psql -X -v ON_ERROR_STOP=1 "$TEST_DATABASE_URL" -f tests/tenant-isolation-policy-fixture.sql
```

第二条可使用隔离的测试数据库；夹具仅创建临时表并在事务末尾回滚。若审计有结果，先保存异常报告并决定迁移处理方式，不能直接删除或更改用户授权。

## 5. 已完成的可运行验证

- `pglast` 成功解析两份 SQL，未发现 PostgreSQL 语法错误。
- 使用隔离的 PGlite（PostgreSQL WASM）执行 `tests/tenant-isolation-policy-fixture.sql`：7/7 个目标矩阵断言通过；其中 `REMOVED_MEMBER_OLD_ROLE` 与 `DISABLED_ORGANIZATION` 输出 `CURRENT_GAP`，其余 5 项为 `CURRENT_MATCH`。
- 在同一隔离运行时执行 `backend/migrations/0001_init.sql` 后运行关系审计 SQL：执行成功、空库异常数为 0。此结果只证明脚本可运行，不代表真实库无异常。
- `tests/tenant-isolation-api.mjs` 通过 Node.js 语法检查；因本机无监听中的后端和测试 token，未执行真实 HTTP 断言。

## 6. 进入后续阶段仍需的证据

1. Q1/Q2：组织进入、祖先角色继承与业务数据范围的产品确认，见 [`authorization-policy.md`](./authorization-policy.md)。
2. 在可控 PostgreSQL 上执行两份 SQL，记录结果和版本；若夹具语法或结果不符，先修复测试材料。
3. 在运行中的专用测试环境，为两个**非平台租户管理员**准备具备用户和角色管理权限的 session token；在项目根目录 [`.env.test.example`](../.env.test.example) 的本地副本 `.env.test` 中填写 `BASELINE_API_URL`、`BASELINE_TOKEN_A`、`BASELINE_TOKEN_B`、`BASELINE_TENANT_A_ID`、`BASELINE_TENANT_B_ID`、`BASELINE_SHARED_USER_ID`，再运行 `node tests/tenant-isolation-api.mjs`。脚本会读取 `.env.test`，且不打印 token。该只读探针检查 A/B 租户列表、组织、用户、角色及共享用户成员关系是否泄漏；详情、写入和导出需在 Phase 3 扩充并记录结果。

Phase 1 的规则矩阵、风险清单、两租户夹具和只读审计脚本已可交付。Q1/Q2 的业务确认仍是 Phase 6/7/11 的前置条件；在真实数据库执行存量审计是 Phase 2 迁移上线门禁，在真实 API 环境运行 HTTP 探针是 Phase 3 隔离修复门禁。不得把隔离运行时结果当成生产数据库或 API 的验收结果。
