# Phase 20 schema、ACL 与 readiness 组件验证

日期：2026-10-28。状态：组件完成；本报告不代替 Phase 20 的服务、网络投递与实际 HTTPS 总验收。

## 边界与当前基线

仅编辑平台 canonical [0001](../../../../migrations/0001_init.sql)、[provision](../../../../ops/identity-deployment/provision.sql)、[verify](../../../../ops/identity-deployment/verify.sql)、schema readiness、相关真实角色 fixture，以及门户独立 [0001](../../../../../../saas-customer-portal/backend/migrations/0001_customer_portal.sql)。没有接管旧 checksum、搬迁数据、更新运行库结构或创建容器。角色仍使用现有六角色模型，没有新增万能 worker 角色。

结构与 DTO 依照[统一合同](../phase-20-logout-delivery-contract.md)，由注销／worker、两个 BFF 和邀请 owner 协调后冻结。

| 当前文件 | SHA-256 |
| --- | --- |
| 平台 `migrations/0001_init.sql` | `33a3cb1f47b80ee38d97ed3d077b5c8296e5383edf75e909b310e183a0b03b59` |
| `ops/identity-deployment/provision.sql` | `7cd110d890c99fc8e532e5c4ca3bb185f787bc61a72acdac955b8fdeac80871b` |
| `ops/identity-deployment/verify.sql` | `621e5d9efb1c3191d15b14263a44fba0e5da50519f680957c7d58e4417e39eb0` |
| 门户 `0001_customer_portal.sql` | `c047c0c37f2a5da1dddc2233e9fdb63935ce5cc29ecb510777c3281e7a0ea4d4` |

这些摘要用于核对本组件当前输入；总验收应冻结包括所有运行时代码和脚本的同一来源快照。

## 结构与权限

`auth_rp_sessions` 绑定原 USER grant、client、account 与 SSO，公开 sid 为独立随机 64 位小写 hex，pairwise sub 为不透明 ASCII 字符串。runtime 只能 SELECT/INSERT，不能替换原来源。两个 BFF 保存不可变 `oidc_public_sid/sub`，内部 `global_session_id UUID` 继续独立使用；logout operation／scope 与 confirmed receipt 一旦建立不可换绑或回退。

两个 RP 各有独立 backchannel receipt 和 source tombstone。receipt 按 issuer/client/jti 摘要唯一，source 按 issuer/client/sid 摘要唯一；公开标识不当作 UUID。它们只允许对应 BFF runtime SELECT/INSERT，不能自行清掉重放或未知 sid 的撤销事实。门户最终有五张业务表、20 个 UUID 列，每个 UUID 都有独立已验证 v7 CHECK、没有 UUID 默认值；schema fixture 和文档已同步。

| identity owner 数据 | identity runtime | platform runtime |
| --- | --- | --- |
| `auth_delivery_targets`、`auth_invitation_verifications` | SELECT | 无权限 |
| `auth_rp_sessions`、`auth_logout_command_receipts`、`auth_invitation_activation_receipts` | SELECT/INSERT | 无权限 |
| `auth_delivery_jobs`、consent/security source outbox | SELECT/INSERT/UPDATE，无 DELETE | 无权限 |

核验事实只由 owner 登记。identity runtime 通过两个 exact-tuple/version 的 SECURITY DEFINER 布尔 fence 取得共享锁，仍没有事实 UPDATE：

- `identity_delivery_target_active(uuid,bigint,text,uuid,text,text)` 精确绑定目标、版本、用途、client 或 recipient/channel。
- `identity_invitation_verification_active(uuid,bigint,uuid,uuid,bigint,text,text)` 先锁 target 再锁 verification，并绑定原邀请、INVITATION job、收件人与用途。

两者均固定 `search_path=pg_catalog,public`，锁等待后以新 statement 再校验时间和当前 tuple。平台无 EXECUTE。内部 event ID 派生、enqueue、source trigger 支持函数归 identity owner，但不向任何 runtime 暴露为任意目标 RPC。

`identity_phase20_schema_ready()` 只读 catalog／部署 marker，核对完整 owner／最小 ACL／函数用途／BFF 不可变字段。provision、verify、平台启动和 identity readiness 使用同一闭包；门户使用独立 [schema_readiness](../../../../../../saas-customer-portal/backend/src/schema_readiness.rs)，启动前检查五表、非 owner 的真实 LOGIN、无 owner 继承／DDL／TEMP 和两账本的最小权限。

## 事务与失效

grant 撤销原子产生单个稳定 event_key 的 LOGOUT job；source UUID 的时间位和目的分离摘要派生合法 v7 job ID，不引入扩展、v4 或 UUID 默认值。账号／SSO 失效及客户端停用／策略代际收缩统一失效准确 canonical grants，USER 才产生 RP 通知，不扩大到其他 client 或 MACHINE 的通知主体。consent、security outbox 保留来源并展开为中央 jobs；原 job 已经 DELIVERED 时，consent 的关联不会被重新标为未完成。job 原来源／目标代际不可换绑，终态不可恢复为待投递。

通知地址过期不能阻止纯停用或紧急 credential revoke。SQL 只允许两种窄收缩例外：准确保持全部元数据／target／URI 不变的 DISABLED/RETIRED；或 status 不变且仅严格推进 policy_version、version、updated_at。启用、用途或 binding 改动继续要求当前 owner 核验事实；失效目标对应 DEGRADED，而不阻止本地撤销。

邀请原收件人、用途、token 摘要、actor、operation 和原期限不可重写，终态不可重新 ACTIVE。领取 receipt 保存原 invitation／verification／target 版本、新账号及目的分离请求摘要；CONSUMED 必须通过复合 FK 指向相同邀请、verification 与新账号的 receipt，不能伪造消费服务。实际密码安全策略、HMAC 和 HTTP 生命周期由邀请 owner 的处理器验收，本组件的账号 fixture 仅证明数据库／权限绑定与无业务成员创建。

## 本轮执行证据

| 检查 | 结果 | 范围 |
| --- | --- | --- |
| 平台 [Phase 20 real LOGIN tests](../../../../src/identity_deployment_tests/phase20.rs) | 3/3 PASS，0 ignored，2.43 秒 | 完整 canonical migration、实际 identity/platform LOGIN、owner-only 事实、原 RP／grant enqueue、同意保留、纯新账号 receipt、准确 source fences |
| 最新紧急撤销 focused real LOGIN | 1/1 PASS，0 ignored，1.20 秒 | 过期 target 下 version-only emergency revoke 与 pure stop 均不阻塞本地失效、DEGRADED 正确、其他 client 不受影响 |
| 必要 Phase 15 real LOGIN 回归 | 3/3 PASS，0 ignored，7.95 秒 | 原能力／账号／成员／应用／SSO 来源收缩、不可复活、真实锁后过期和 durable context |
| 门户角色／schema 与 receiver PG 套件 | 门户 owner 回报 13/13 PASS | 包含 [schema_roles](../../../../../../saas-customer-portal/backend/src/tests/schema_roles.rs) 的真实 LOGIN、五表 20 UUID、receipt/source SI-only；本组件未将它冒称自己的单独执行 |
| 平台 `cargo clippy --locked --all-targets -- -D warnings` | PASS | 最新 schema／测试可编译、无 warning |
| 平台全量 `cargo fmt --all -- --check`、两个仓库 diff 与文档链接检查 | PASS | 全量平台格式通过；本报告、部署 README 与门户 backend README 的 14 个相对文件链接全部存在 |

实际锁证据：identity runtime 通过 owner fence 请求一个短期有效 target；测试持有它的 FOR UPDATE，并通过 `pg_stat_activity` 与 `pg_blocking_pids` 确认等待。等待跨过实际 expires_at 后释放锁，函数返回 false；同一 runtime 对事实的直接 UPDATE 仍为 42501。这验证共享锁不会要求放宽 owner-only UPDATE，也不会沿用等待前的旧时间。

真实角色还证明：平台不能读认证／投递表；identity 不能读 BFF receipt/source；runtime 不能直接调用内部 enqueue；任意 verification UUID 不能替代 owner 核验事实；两个 BFF 的公开来源不可更新；新账号初始 generation=1 且 memberships=0；重复事件、终态恢复、收件人／来源换绑均受唯一键、权限或触发器限制。

## 清理与证据边界

所有测试只选择任务自有原生 PostgreSQL `127.0.0.1:55494/postgres`，核对 cluster system identifier `7701548153213487841`。每个 canonical 角色用例建立随机新库和角色，结束均输出 database_removed／roles_removed／pool_closed=true。最后核对 `identity_phase12_%` 数据库=0、`id12_%` 角色=0、`phase20_schema_smoke_%` 数据库=0、门户 `portal20_roles_%` 角色=0。

本组件创建容器 0、复用容器 0，没有新卷、网络、镜像、导出日志或持久测试数据。共享 PostgreSQL 保留给其他阶段，外部 Cargo cache 的迁移及最终清理由主代理按资源记录负责；这里不声称 WSL／Windows 物理空间已压缩回收。

本组件不宣称整体 Phase 20 完成。实际 CURRENT_APP／GLOBAL RPC、签名接收、网络 SSRF／DNS 绑定、lease／重试／告警、可信邀请投递／单次领取和全版本 HTTPS 仍由对应 owner 与主代理合并验收。只读核验函数的允许／拒绝与锁后截止已由真正 runtime 验证，没有用管理员连接冒充运行角色。

## 组件 Self-Check

| 组件要求 | 结果 |
| --- | --- |
| 唯一新库 `0001`，不接管旧历史或运行 DDL | PASS |
| 公开 RP sid/sub 与内部身份严格分离并不可换绑 | PASS |
| owner-only 核验事实，窄 owner 锁，不扩大两个 runtime 数据边界 | PASS |
| 原来源／operation／receipt／outbox 的事务与幂等结构 | PASS |
| 最小 ACL／readiness 与真实 LOGIN 正负、锁等待回归 | PASS |
| 隔离 fixture 与完整资源清理 | PASS |

建议总报告引用本组件证据并按实际全阶段门禁更新台账；本代理不修改台账或给未执行的服务验收打勾。
