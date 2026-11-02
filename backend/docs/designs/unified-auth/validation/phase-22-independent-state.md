# Phase 22 独立身份实例与故障组件验证

日期：2026-10-28。状态：COMPLETED（组件）。整体 Phase 22 容量、遥测及实际部署验收仍由总报告记录。

## 范围与方法

测试位于 [phase22_tests.rs](../../../../../../saas-identity/src/oidc/oauth/phase22_tests.rs)。每组建立当前 `0001` canonical 空库，使用临时签名材料和明确账号、客户端、SSO、consent/grant 测试事实，再经过完整生产 `crate::router` 的 middleware 和 handler。授权码正向走实际 `/oauth/authorize`、interaction/明确 consent，未直接插入 code。

竞争用例分别调用两个 `IdentityState::new`。`auth_limits`、`login_limits`、auth/internal/context semaphores、authentication/account ports 与 context port 的 `Arc::ptr_eq` 均为 false；共享内容为数据库及不可变配置。测试在同一 OS 测试进程中运行两个独立状态，不冒充两个已部署进程或跨服务 HTTPS 证据。

三个竞争用例先持有真实数据库行锁或 advisory lock，再启动两个请求，通过 `pg_blocking_pids` 确认两者同时等候原 PostgreSQL blocker 后释放；不是用单连接池或同一进程锁把请求预先串行化。其余用例使用独立有限连接池、实际表锁和真实 `ConnectInfo<Peer>` 请求元数据。

共享 [test_support.rs](../../../../../../saas-identity/src/test_support.rs) 只补合法测试事实：refresh family 的 absolute deadline 取原 `grant.expires_at`，idle deadline 不超过它，避免后建 family 的时间比原授权上限晚几毫秒。未放宽生产校验；本组件未修改 OAuth、config、state、readiness、keys、schema 或部署服务。

## 最终结果

| canonical PostgreSQL 场景 | 结果及关键断言 |
| --- | --- |
| `independent_nodes_redeem_one_actual_authorization_code_once` | PASS；同 code 两个独立状态仅一次 200，另一请求 400 `invalid_grant`；grant、已消费 code、已消费 request 和 `AUTH_CODE_EXCHANGE` 审计各 1。 |
| `independent_nodes_refresh_race_detects_reuse_without_crossing_sources` | PASS；同 refresh 两请求仅一次轮换成功，另一请求准确重放拒绝；原 family 撤销、仅两代 token、原 access 全撤销、reuse 审计 1。已返回 successor 不能再用；同账号、同客户端和同 SSO 的另一 grant/refresh 保持可用。 |
| `independent_nodes_logout_operation_is_one_durable_commit` | PASS；同 operation 两请求返回相同完成结果，receipt、审计及本地 logout outbox 各 1；另一 grant 与 SSO/账号代际保持。原 operation 改为 GLOBAL 返回 409，第三个状态可恢复原 receipt。outbox 为短会话降级事实，不声称已投递。 |
| `saturated_and_closed_pools_return_503_without_clearing_legal_sources` | PASS；max=1、acquire=70ms 池被实际占满，以及同池显式关闭后，token、userinfo、SSO session 均 503；无 `WWW-Authenticate` 和清 Cookie。账号代际、SSO、grant、access、refresh/family 不变；释放容量后原 Cookie 成功，另一状态仍能使用原 refresh。 |
| `actual_client_table_lock_failure_is_503_and_preserves_original_tokens` | PASS；实际 `ACCESS EXCLUSIVE auth_clients` 锁，独立池 lock=100ms、statement=500ms，使正确凭据 token/userinfo 请求返回 503，不转成 401；解除锁后原 access 和 refresh 成功，故障期间未消费或撤销原来源。 |
| `client_budget_cannot_be_reset_by_rotating_socket_sources` | PASS；client budget=2，额度内错误 secret 为 401，正确 secret 到达 grant 检查；切换两个 socket IP 并伪造不可信 `X-Real-IP` 仍为 429，携带 1–60 秒 `Retry-After`。另一独立状态拥有自己的本地额度；不把进程本地限额描述为全局集群配额。无 grant 或授权审计生成。 |

最终整组 **6/6 PASS、0 ignored，10.35s**。命令：

```powershell
$env:IDENTITY_PHASE22_TEST_DATABASE_URL='postgres://postgres@127.0.0.1:55494/postgres'
$env:IDENTITY_PHASE22_TEST_CLUSTER_ID='7701548153213487841'
cargo test --locked --lib oidc::oauth::phase22_tests -- --include-ignored --test-threads=1 --nocapture
cargo clippy --locked --lib --tests -- -D warnings
```

strict Clippy PASS（15.82s）；所属测试文件及 helper 的 Rust 2021 格式与 diff 空白检查 PASS。最初整组 5/6 的失败来自测试 SSO GET 漏带同源 Origin，生产正确返回 403；补齐 fixture 后定向 1/1 PASS（2.06s），再执行上述最终整组 6/6，失败结果未充作通过证据。

## 资源与证据限制

仅连接任务明确提供的原生 PostgreSQL `127.0.0.1:55494`，每次建库及删除前核对集群标识 `7701548153213487841`。首轮 6 个、定向修复 1 个、最终 6 个，共 13 个 UUID 数据库均由 isolated helper 在成功或失败退出后删除，并逐个查询确认不存在。独立 `psql` catalog 复核：`identity_phase22_core_*` 数据库 **0**、活动连接 **0**；临时 canonical identity 签名目录 **0**。

未启停或修改 443/24445 服务；无网络 listener、浏览器或 fixture server，无容器创建或复用，无卷、网络、镜像、专用缓存和导出运行日志。保留根任务共享 PostgreSQL/构建缓存。无本组件 Docker 空间变化，不声称 Windows/WSL 虚拟磁盘已回收。

机器记录、最终计数及测试/helper/schema 原始字节 SHA-256 见 [phase-22-independent-state.json](phase-22-independent-state.json)。这些是实际 handler 与 canonical SQL 的组件证据，不替代真实多副本部署、长期容量基准或后续最终 HTTPS 门禁。
