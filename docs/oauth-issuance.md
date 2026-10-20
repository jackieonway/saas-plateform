# 普通 OAuth 签发、实时失权与升级契约

本文记录 Phase 11 的普通 OAuth 实现，源码入口为 [oauth2.rs](../backend/src/routes/oauth2.rs)、窄签发模块 [oauth_issuance.rs](../backend/src/oauth_issuance.rs) 和 [0003 增量迁移](../backend/migrations/0003_oauth_issuance.sql)。实际测试、源码快照和环境结果见 [Phase 11 验证记录](validation/phase-11.md)。平台 `psr_` 刷新和应用 token 继续使用各自协议。

## 端点与一次性状态

现有 `/oauth/authorize`、`/oauth/token`、`/oauth/userinfo`、`/oauth/introspect`、`/oauth/revoke`、`/oauth/device/authorization` 路径与 OAuth 表单格式保持。授权码仍校验客户端、完整 redirect URI 和已登记的 PKCE；设备码仍保留轮询间隔与 `authorization_pending`、`slow_down`、`access_denied`、`expired_token` 错误。

| 对象 | 可兑换条件 | 成功提交后的状态 | 失败处理 |
| --- | --- | --- | --- |
| 授权确认请求 | 未消费且未到期；当前用户与客户端有效 | 批准时在同事务内消费请求并保存短期授权码；拒绝时消费请求 | 授权码插入失败会回滚请求消费，允许重新确认 |
| 授权码 | 未消费、未到期，客户端/redirect URI/PKCE 匹配 | `consumed=true` 与完整 token 对及签发审计同事务提交 | 条件消费失败为 `invalid_grant`；后续写入失败回滚消费 |
| 普通 refresh | `token_use=oauth`、类型 refresh、未撤销、未到期，属于调用客户端 | 旧行 `revoked=true`，新 access/refresh 及签发审计同事务提交 | 并发只有一次成功；重放为 `invalid_grant`；新 token 或审计写入失败不烧毁旧值 |
| 已批准设备码 | `APPROVED`、未到期，客户端与已批准用户上下文匹配 | `EXPIRED` 与完整 token 对及签发审计同事务提交 | 未批准时保留轮询状态；批准后的消费或签发失败整体回滚 |

每次条件消费核对受影响行数。access 与 refresh 分别插入时也要求恰好写入一行；数据库触发器返回 `NULL` 而跳过插入不能产生成功响应或半套 token。普通 `/oauth/token` 成功响应带 `Cache-Control: no-store` 和 `Pragma: no-cache`。

普通 refresh 轮换固定原用户、租户与组织，不接受借刷新切换组织。省略或传入空白 scope 时沿用原 grant；显式 scope 只能取原 grant 与客户端当前注册 scope 的子集。授权码和设备码在实际签发时也重新核对当前客户端 scope。普通刷新按客户端当前 TTL 生成新凭据，并受当前身份到期上限限制；它不继承平台 `psr_` 的固定绝对刷新截止规则。轮换消费的是旧 refresh，不会自动撤销同 grant 之前发出的 access。

历史 password grant 保留兼容，默认使用现有第一条直接成员关系作为上下文，实际签发前按下文规则复核。密码验证后获取用户锁，再核对数据库密码 hash 与验证时一致，防止管理员重置后继续凭旧密码签发。该流程不是新业务推荐的登录方式；密码计算资源保护属于 Phase 12。

## 用户与机器身份的统一有效性

普通用户 token 的 refresh、userinfo、introspect 共用 `active_identity`。用户型身份必须同时满足：

1. 客户端仍存在且 `status=ENABLED`。
2. 用户存在且 `status=ENABLED`，用户/租户/组织上下文均完整。
3. 该用户仍是 token 绑定组织的直接成员；其它组织成员或祖先角色不代替这条成员关系。
4. 组织归属 token 绑定租户，租户启用且未到期。
5. 当前组织及所有祖先启用、未到期且归属同一租户，祖先链能到达根组织。

身份到期使用 `clock_timestamp()`，避免事务或锁等待越过真实到期时刻后仍沿用事务开始时的时间。新 access/refresh 的期限还截断到租户与组织祖先最早的已知到期时间；后续停用、缩短期限或成员移除仍通过实时检查生效。userinfo/introspect 读取的 token 行还必须为 access、未撤销且未到期。userinfo 要求 JWT 签名、严格到期和类型验证；普通 introspect 保留按数据库摘要识别 opaque access 的现有契约，不把仅凭 JWT 可解析作为 active 的前提。应用 token 的 introspect 继续使用独立的 JWT/受众和完整绑定检查。

| 调用 | 用户或绑定上下文当前失效时的结果 |
| --- | --- |
| `/oauth/userinfo` | HTTP 401，`invalid_token`，不返回有效用户身份 |
| `/oauth/introspect` | `{"active":false}`，不返回有效身份元数据 |
| 普通 refresh 兑换 | HTTP 400，`invalid_grant`；被禁用/删除客户端在客户端认证阶段为 HTTP 401 `invalid_client` |

`client_credentials` 没有用户；其 `user_id`、`tenant_id`、`organization_id` 全部为空时按客户端有效性独立处理，用户或组织停用不使无关机器 token 失效。机器 token 可以内省为 active，但不能调用 userinfo 获得用户身份。为兼容旧实现，已登记 refresh grant 的机器客户端仍可获得并轮换无用户 refresh。只缺一部分用户上下文的记录会被拒绝。

普通 OAuth 采用实时动态状态语义：若用户/组织/租户恢复启用或重新加入原成员关系，仍未撤销、未消费且未到期的旧凭据可以恢复可用。显式 revoke、已经消费的授权码/refresh/设备码不会因此复活。本阶段没有新增对所有身份事件永久撤销普通 OAuth 的触发器，也不重写平台会话原有的持久撤销规则。

## 事务、锁序与管理撤销

路由负责客户端认证、协议错误、一次性凭据消费和事务提交；`oauth_issuance` 负责共享身份规则与事务内写入 token。签发顺序如下：

```text
开始事务 → 锁 users（用户型） → 锁 oauth_clients 并复核认证配置
        → 锁授权码 / refresh / 设备码 → 条件消费 → 复核身份与 scope
        → 写 access → 写可选 refresh → 写 TOKEN_ISSUED 审计 → 提交
```

首次只读查询只用于定位锁定用户；持锁后重新读取一次性凭据，旧值不能授权签发。客户端在锁内与已认证配置比对，等待期间密钥、状态、类型、grant、scope 或 TTL 变化会拒绝本次交换，调用者可按当前配置重新发起。成员维护和平台会话撤销同样先锁用户，避免凭据锁与用户锁形成反向依赖。

管理员踢出把用户会话版本提升、平台 sessions 撤销和该用户 OAuth token 撤销放入同一事务，消除两个提交之间刷新产生新 token 的窗口。这是 Phase 11 的失权闭合。`TOKEN_ISSUED` 审计随签发事务提交；管理动作、consent 审计等全部迁入事务的工作仍归 Phase 13，不能把本阶段视作全面完成管理审计迁移。

## 存储与旧 token 兼容

新普通 access 使用随机 `jti`，相同用户、scope 与同一秒签发的两个 token 仍具有不同字符串和独立数据库行。撤销其中一个不影响另一个。插入不再用 token hash 冲突更新旧行，已撤销状态不会被签发恢复。

数据库只保存 token hash、类型、身份上下文、scope、期限、jti 与撤销等必要元数据。新 access/refresh 的 `jwt` 均为 `NULL`，token 原文只出现在成功响应中；错误、日志与审计不记录 token 或客户端 secret。

0003 为既有行标记 `storage_version=1`，随后把默认值改为 2；新签发显式写 2。约束限定版本为 1/2，并以 `NOT VALID` 检查约束要求版本 2 的普通 OAuth 行 `jwt IS NULL`。PostgreSQL 的 `NOT VALID` 仍会检查旧行的 UPDATE，因此保留版本 1 的例外，允许旧原文行正常撤销或消费。

兼容窗口覆盖既有无 `jti` 的普通 access JWT、原文仍在 `jwt` 列的版本 1 行，以及只保存摘要的旧 opaque refresh。普通 introspect 也保留已入库 opaque access 的摘要契约；该 opaque 值不能因此用于要求 JWT 的 userinfo。读取依据 token hash 和元数据，不依赖旧原文；旧无 jti 本身不会被视作无效。迁移不删除 token、清空原文、改写 0001/0002 或重置 SQLx 迁移记录。旧原文与历史凭据清理期限仍需正式保留政策，由后续受控清理处理；未批准前继续保留。

## 未知提交、升级与回退

事务内错误会回滚凭据消费、token 对和签发审计。网络断开或提交回执丢失时，客户端无法据此证明事务未提交；本协议没有返回已签发 token 原文的对账/幂等恢复接口。不要自动重耗结果未知的 refresh，也不要把 HTTP 超时当作数据库已回滚。停止使用未知轮换结果并重新授权；请求编号可用于查询脱敏诊断，不能用于恢复 token 原文。对授权码/设备码成功响应丢失的情况同样需重新发起授权流程。

采用步骤为：先在隔离副本核对旧 token 形状与 0003 升级，再停用并排空所有旧普通 OAuth issuer，保留备份与迁移校验和，用嵌入 0001–0003 的新版二进制执行正常启动迁移，最后运行本次身份、重放、并发与故障门禁。空库顺序执行三份迁移；已采用兼容基线的库只追加未执行项。

应用版本回退不能删除 0003、降 schema 或删 `_sqlx_migrations` 记录。旧 issuer 仍尝试写 JWT 原文、使用旧 upsert 和非事务消费，与新 schema/状态机不兼容，不能混跑或作为无条件回退。需要回退时使用保留 0003 读取/存储约束与本状态机的兼容构建；若必须恢复旧系统，应在停写窗口评估整体数据库、程序与凭据恢复，旧 token 不自动导出回灌。此说明不授权生产迁移或恢复操作。

## 验证入口

`scripts/ci/manifest.mjs` 为 PostgreSQL 必测清单，当前七组共 41 项：既有五组 27 项、`oauth_issuance_tests` 10 项及 `platform_session_refresh_tests` 4 项，包含应用授权/隔离回归。通过隔离 launcher 执行：

```sh
node scripts/ci/linux-runner.mjs --gate postgres --image saas-ci:phase04-tools --out-dir /absolute/new-report/phase11-postgres
```

不得把生产连接串传给测试。预期数量以当前 manifest 为准，实际通过/失败/跳过、至少 20 并发兑换、写入与消费故障、用户/成员/组织祖先/租户/客户端状态和 psr_/app 回归必须由 [Phase 11 报告](validation/phase-11.md) 的同一最终源码证据核实。测试声明、历史 PASS 和旧阶段计数不能替代本阶段实际执行。
