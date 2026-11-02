# Phase 20 平台 BFF 组件验证

日期：2026-10-28。组件状态：COMPLETED；整体 Phase 20 的 canonical 身份服务、可靠投递及实际 HTTPS 门禁仍由阶段协调流程验收。

## 输入与范围

依据[登出与可靠投递合同](../phase-20-logout-delivery-contract.md)。平台仓库基础 HEAD 为 `34105757b040165daa5477bf6afff9637f617466`；原有 Phase 13–19 的未提交改动均保留，未提交或重置 Git。

本组件修改平台 `src/bff/*`、对应 `src/bff_tests.rs`/`src/bff_tests/backchannel.rs` 及[当前 SSO 操作说明](../../../sso-only.md)。canonical `0001`、最小角色 ACL、readiness 和其他 SQL fixture 由独立 schema owner 统一修改。客户门户由并行 owner 完成，见[门户组件报告](phase-20-portal.md)。

## 当前实现

- code 兑换的已验签 ID Token 必须携带 64 位小写 hex 的 opaque sid；公开 sub 与 sid 只保存在服务端 `oidc_public_sid`/`oidc_public_sub`，不作为内部账号或 SSO UUID。ID Token 与 logout token 的 typ 分别为 `JWT`、`logout+jwt`。refresh 无 ID Token 时保留原映射，返回新 ID Token 时必须匹配原 sid/sub。
- 浏览器原 `PLATFORM`/`GLOBAL` 范围分别映射到 canonical `CURRENT_APP`/`GLOBAL`。服务端先查询固定 `/oauth/logout/operations/{operationId}`，仅明确 404、`code=NOT_FOUND` 才向 `/oauth/logout` 提交同一原 operation、原 refresh、sid/sub 和 scope。完成回执严格校验原 operation、scope、状态及计数；未知结果不换来源或编号。
- 固定公开 `POST /auth/backchannel-logout` 只接受有界 Form `logout_token`，拒绝 query、重复/未知字段、其他认证方式和主机替换。它不需要浏览器 Cookie、Origin、CSRF 或已撤销的 access proof。
- 接收器从固定 issuer 的 JWKS 选择已公布 RSA 公钥，校验 RS256、明确 typ、准确单字符串 audience、原 sid/sub、UUIDv7 jti、固定事件及原 120 秒窗口，最多允许 5 秒未来时钟偏差。未知/重复 claims、nonce、算法降级、外部密钥 URL 和多余 header 均拒绝。重复 jti 为 `409 BACKCHANNEL_REPLAY`，错误签名、subject 绑定或过期为 400。
- 验签后的 jti receipt、sid/sub 摘要撤销事实、原 BFF 与派生平台会话和浏览器指针在同一事务处理。未知 sid 仍记录终态事实并返回 200；callback/refresh 在写事务中先取得相同原来源 advisory lock，再拒绝任何已知撤销事实。通知先于本地 INSERT 也不能让旧 sid 后来复活。
- 原来源锁先于账号、浏览器和会话行锁；网络验签/JWKS 获取在本地事务前完成。等待锁之后和提交前复验原 exp，不重新授予 TTL。
- 提前到达的通知可以先关闭正在发起注销的原会话。RPC 完成后仍确认同一原回执；新浏览器指针只产生 409，不清新 Cookie。即使旧回包到达时已有新登录，原完成事实也落盘一次，不反复远端恢复。已确认旧 Cookie 不能改注销范围或把新来源报告为已退出。

## 验证结果与层级

| 检查 | 结果 | 层级 |
| --- | --- | --- |
| `cargo check --locked --bin saas-platform-backend --tests` | PASS | 平台与全部测试 target 编译 |
| `cargo test --locked --bin saas-platform-backend bff_tests::backchannel -- --include-ignored --test-threads=1 --nocapture` | 6/6 PASS，0 ignored | 2 个真实 RSA 签名/重复字段单元矩阵；4 个独立 PostgreSQL + 实际 Axum handler 用例 |
| `... bff_tests::backchannel::refreshed_id_token_cannot_graft_a_new_public_sid -- --ignored --exact --nocapture` | 1/1 PASS，0 ignored | 真实轮换后不换 sid，带错 sid 的签名响应拒绝，未知轮换不能再次发送旧 family |
| `cargo clippy --locked --bin saas-platform-backend --tests -- -D warnings` | PASS | 最终组件版本，无代码 warning |
| 定向 `rustfmt --check`、`git diff --check` | PASS | 当前 BFF、测试及说明 |
| `cargo doc --locked --no-deps --bin saas-platform-backend` | 生成成功，存在已列明 warning | 两处组件之外的既有注释及同名 bin/lib 文档输出碰撞，见下文 |

实际数据库用例验证：

1. 错 sub 不改会话；正确通知不调用账号 access proof；原 BFF/派生会话关闭，重复 jti 返回准确 409，后来新登录仍可正常读取真实 session。
2. callback 已取得签名和私有 proof、但本地写入尚未开始时，使用实际 barrier 先提交未知 sid 通知。释放 callback 后返回 401、该 sid 本地 session 数为 0，另一个 sid 可以正常登录。
3. 固定 canonical RPC transport responder 在回包前实际调用签名接收器，并丢失首次成功回包。旧 Cookie 只查原 operation 完成恢复；两个范围各提交一次、原审计一次，不换 token 或操作。另一个账户在原回包前登录时，旧回包返回 409，但原 receipt 已确认，新账户 Cookie 保持有效；已确认的旧请求也不能扩大 scope。
4. `pg_blocking_pids` 证明接收器真实等待原来源 advisory lock，开始等待时通知尚未过期。按数据库时钟跨越原 exp 后释放，实际 HTTP 返回 400，jti receipt 与来源 tombstone 均为 0，没有把等待后时间作为新 TTL。
5. 实际 refresh family 轮换省略 ID Token 时，原 sid/sub 保持、加密 access 已变化；下一次返回有正确签名但新 sid 的响应时拒绝，原凭据和 sid/sub 不被替换，后续读取保持未知结果失败关闭。

登录准备仍使用明确标记的历史 provider transport adapter；新的 receiver 使用真正 RS256 签名、实际平台路由及真实 SQL，RPC 恢复使用固定 canonical 形状的独立 responder。这些是本地 RP 组件证据，不冒充 canonical 身份服务签发/撤销或跨服务 TLS 互操作。整体 Phase 20 必须另外记录当前身份服务及 worker 的实际 HTTPS 结果，Phase 25 还需重跑最终版本。

## 资源、清理与限制

测试只连接任务所有的原生 PostgreSQL `55494`。独立目录核查返回集群标识 `7701548153213487841`，本组件七个已创建数据库及活动连接残留为 `0|0`。所有 fixture 输出数据库删除、连接池关闭和临时 cipher/keyset 文件删除成功；未创建角色、容器、网络、卷、镜像、独立 target 目录或导出运行日志。共享 PostgreSQL、构建缓存和源码快照保留给根任务继续使用，不在本组件停止或删除，不声称 Windows 虚拟磁盘空间已恢复。

文档构建提示平台已有同名 binary/library 输出到同一个文档路径，并报告组件之外的两条原有 rustdoc warning：`routes/notifications.rs` 的 `[01]` 正则被识别为链接、`extractors.rs` 的 `<session JWT>` 被识别为 HTML。已交根协调流程处理；本组件没有跨文件重排或改造 crate。

本地来源 tombstone 是不可复活事实，普通 runtime 不能更新或删除；受控留存/清理策略继续由后续密钥和恢复生命周期阶段处理。已知本地事实的事务串行化不承诺远端 HTTP 与独立数据库构成分布式原子事务。

## 组件自查

| 条件 | 结果 |
| --- | --- |
| opaque sid/sub 来源、ID Token 与 refresh 绑定 | PASS |
| canonical 原 operation/凭据查询与恢复，不扩大注销范围 | PASS |
| 独立签名接收、重放/错受众/错事件/错期限拒绝 | PASS |
| 原来源关闭、未知 sid 先到不复活、迟到回执不覆盖新来源 | PASS |
| 原始期限经真实锁等待后仍约束提交 | PASS |
| 专属测试资源定向清理、未冒称整阶段 HTTPS 完成 | PASS |

机器证据与本组件所拥有文件的原始字节摘要见[组件记录](phase-20-platform-bff-components.json)。它是所列输入文件的组件清单，不是完整仓库或正式运行环境的构建证明。
