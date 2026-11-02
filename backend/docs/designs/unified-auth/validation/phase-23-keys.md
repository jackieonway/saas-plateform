# Phase 23 签名密钥生命周期组件验证

状态：COMPONENT_PASS。2026-10-28 UTC。本报告只证明当前签名组件、canonical PostgreSQL／运行角色和实际 Router 的行为；阶段整体状态、真实 HTTPS 双实例／双 RP、真实 TOTP、K06 实际时间窗及隔离恢复由 Phase 23 统一报告验收。本报告不把这些其他层级标为通过，也不修改任务台账。

## 1. 范围与变更

执行项目是 `saas-identity`，使用当前平台 `0001_init.sql`、真实部署角色交接与 ACL 验证脚本。保留 `oidc_signing_keys`、`oidc_provider_settings`、`identity_oidc_admin_receipts` 的当前名称和单一权威。此次密钥组件没有修改生产生命周期逻辑；`keys.rs` 仅加入 `cfg(test)` 子模块声明。文件中已有的 `sign_typed` 和投递类型来自先前阶段。

新增五个测试模块：[配置加载](../../../../../../saas-identity/src/oidc/keys/config_tests.rs)、[临时材料](../../../../../../saas-identity/src/oidc/keys/test_keys.rs)、[canonical 员工来源](../../../../../../saas-identity/src/oidc/keys/actor_fixture.rs)、[隔离数据库／真实角色](../../../../../../saas-identity/src/oidc/keys/pg_fixture.rs)、[生命周期与事务](../../../../../../saas-identity/src/oidc/keys/lifecycle_tests.rs)。`Cargo.toml` 仅增加 test-only SQLx `migrate` feature，用实际迁移运行器执行当前基线，不增加新的生产依赖。

已新增[签名密钥轮换操作手册](../../../../../../saas-identity/docs/signing-key-rotation.md)。手册明确全实例预置与启动加载、原 operation 及期限、正常轮换、紧急吊销、第三方缓存、opaque token 和 canonical 恢复边界。手册本身不作为运行证据。

## 2. 证据环境与真实性

- 复用本任务原生 PostgreSQL `127.0.0.1:55494`，系统标识 `7701548153213487841`。每例显式读取 `IDENTITY_PHASE23_TEST_DATABASE_URL` 与 `IDENTITY_PHASE23_TEST_CLUSTER_ID`，拒绝环境 `DATABASE_URL`、其他端口／集群和非 loopback 管理库。
- 每例新建 `identity_phase23_keys_<32hex>` 数据库和 `i23k_<20hex>_*` 六个部署角色；数据库带对应 `identity-deployment/1/<deployment UUID>` comment。SQLx 运行当前唯一迁移，再执行当前 `create-roles.sql`、`provision.sql`、`verify.sql`。
- API 和签名使用真正 `identity_runtime` LOGIN pool，断言 `current_user=session_user` 为该角色，未使用 `SET ROLE`。角色没有 superuser／CREATEROLE／CREATEDB，不得创建表或原始读取 memberships。
- 当前该 loopback 路径使用任务 PG 已有 trust HBA；真实 LOGIN 和 ACL 证据成立，设置密码后连接成功不作为 SCRAM／密码正确性证据。正式部署认证由统一环境的窄 SCRAM 路径另验。
- owner 仅用于创建 fixture 和定向故障。员工不是超级管理员，实际具有根组织 EMPLOYEE、业务应用／菜单／角色授权、canonical PLATFORM_CONSOLE 客户端、同意、原 grant、SSO、RP session、console browser、BFF 和选中平台会话。没有替换任何授权函数，也没有给运行角色追加业务表权限。
- MFA 因子与最近 TOTP 事实是明确的 owner seed；占位 BFF 密文没有用于真实 token 解密。本报告不证明真实 TOTP 注册、验证码输入或完整浏览器流程。
- HTTP 通过当前 `crate::router` 的真实 ServicePrincipal、私有 guard、admission 和错误映射执行。签名直接调用同一实际签名函数并使用真实事务／行锁。
- 两个并发签名任务共用 AppState／KeySet／pool，证明并发事务与高水位；它们不是两个独立启动进程。实际双实例由统一 HTTPS 验证承担。

## 3. 执行结果

| 检查 | 实际结果 |
|---|---|
| Windows `cargo fmt --all -- --check` | PASS |
| Windows `cargo check --locked --all-targets --all-features` | PASS |
| Windows `cargo clippy --locked --all-targets --all-features -- -D warnings` | PASS |
| Windows `cargo doc --locked --no-deps --all-features` | PASS |
| Windows 完整 `cargo test --locked --all-targets --all-features` | lib 80 PASS、6 FAIL、59 ignored；失败历史保留，见下文 |
| Windows 只排除下列六项后的同一命令 | lib 80 PASS、0 FAIL、59 ignored、6 filtered；main 1 PASS。只是有排除的结果 |
| WSL/Linux `cargo test --locked --all-targets --all-features -j2` | **lib 88 PASS、0 FAIL、57 ignored、0 filtered；main 1 PASS**，测试运行 3.56 秒 |
| WSL/Linux `cargo clippy --locked --all-targets --all-features -j2 -- -D warnings` | PASS |
| Windows 显式 PG `cargo test --locked --lib phase23_keys_ -- --ignored --nocapture --test-threads=1` | **3/3 PASS**，最终一轮 13.29 秒；每例 DB／角色均删除 |
| WSL/Linux 显式 PG 同一过滤器，加 `-j2` | **3/3 PASS**，8.21 秒；每例 DB／角色均删除，Unix 私钥目录显式删除并核对不存在 |

Linux 使用现有 `jackie`、Rust 1.97、共享 `CARGO_HOME` 和 `/tmp/rust-saas-unified-auth-20261028/target-linux`，`CARGO_INCREMENTAL=0`，没有新建容器或另一套构建缓存。完整 Linux 测试包括 K01；三个需要显式数据库的新增测试另外执行，不把 ignored 当作通过。

Windows 六个失败均为现有实际 socket 场景，原本机回环探针也连接超时；同一当前源码在上表 Linux 完整运行中全部通过：

1. `delivery::transport::tests::declared_streamed_and_exact_response_bounds_are_enforced`
2. `delivery::transport::tests::environment_proxies_are_ignored_in_an_isolated_process`
3. `delivery::transport::tests::owned_loopback_tcp_fixture_is_reachable`
4. `delivery::transport::tests::redirects_are_not_followed_and_network_requests_are_not_retried`
5. `delivery::transport::tests::slow_response_cannot_extend_the_total_three_second_deadline`
6. `delivery::transport::tests::verified_tls_pins_one_dns_snapshot_and_preserves_original_host_and_proof`

前三轮开发发现并纠正的是 fixture 问题：`user_roles.organization_id` 必填、审计注入必须指向 `identity_audit_logs`，以及负例必须精确断言状态码。未据此修改生产行为。后续独立审查补齐私钥删除检查、JWKS 公有字段 allowlist、签名持锁与退役直接并发、跨服务／新 secret／换 body 或 path 的原 operation 冲突。

## 4. K01–K09 对照

| 项目 | 组件结果与证据 | 本报告边界 |
|---|---|---|
| K01 配置 | PASS。三把独立 RSA；相对／缺失路径、非法／过大 JSON、未知字段、空／超 8、重复／非法／超长 kid、错误 exponent／modulus 编码／尺寸、非法／过大 PEM、公私钥不匹配、真实 Ed25519 PKCS8 均拒绝；错误无材料内容 | 直接加载真实配置函数；不冒称生产进程启动测试 |
| K02 预发布 | PASS。未发布 B 激活409；发布 A／激活 A／发布 B 后 JWKS 含 A/B，签名仍为 A。逐 key 只准 `kty/use/alg/kid/n/e` 六字段 | 实际管理 Router＋PG；HTTPS／真实 TOTP 另验 |
| K03 激活与原结果 | PASS。B 唯一 active，ID JWT 与 `logout+jwt` 都用 B；A 仍未过期时可由 A 验证。相同 operation 过期后重放／原摘要 lookup，只有一份 receipt；变 body／path、另一真实 OIDC_ADMIN service、新 service secret 均409 | 模拟回包未知是丢弃已返回结果再恢复；不声称实际网络丢包 |
| K04 不变性 | PASS。新 operation 重发相同 kid409；DB trigger 拒绝 n 改写；退役／吊销后激活409，已结束 key 不复活 | API 与当前约束均真实执行 |
| K05 保留与并发 | PASS。活动／窗口内退役409；签名事务持行锁，另一签名、退役和激活各自进入真实 PG Lock barrier。最后签名 commit 后退役409／激活成功，高水位包含最后提交值；较短 exp 不覆盖 MAX，回滚不提交新值 | 使用 PG wait_event 观测，不以任意 sleep 代替并发 barrier |
| K06 正常退役 | 无签发记录的非活动 C 正常退役 PASS，退出 JWKS、不可激活 | **有真实签发记录的 A 的 exp+60 实时等待由统一环境另验**；没有调小高水位／时钟冒充等待 |
| K07 紧急吊销 | PASS。A 尚有合法 token 时吊销，鲜活 JWKS 删除 A，B 保留；A 原缓存公钥仍可数学验签。吊销活动 B 后无活动 key，签发拒绝。独立 ACCOUNT_ONLY grant／access／refresh family 均未被 key-only 吊销 | 当前 RP 对未知／撤销 key 及错误 issuer/aud/typ/alg 的真实拒绝由统一 HTTPS/RP 矩阵另验 |
| K08 缺材料 | PASS。活动 B 而候选状态仅有 A、或 B kid 指向不同独立私钥，签名均失败；全部 active key 吊销后不回退 | DB／JWKS 故障和 HTTP 有界503由统一环境／Phase22当前门禁覆盖，不把直接函数 Err 冒称 HTTP503 |
| K09 权限和原子性 | PASS。无菜单／陈旧 MFA／原 grant 撤销409；错服务用途／禁用／secret403；Cookie400、错摘要409、过期 intent409。审计注入503、receipt注入500，lifecycle／audit／receipt 均0；真实生命周期锁等待跨60秒后重新采样数据库时间并精确409 | Cookie拒绝不是平台CSRF验证；平台来源／CSRF／真实MFA另验。runtime真实ACL无提升 |

## 5. 原子性与时间证据细节

实际管理命令持有当前 `AdminTransaction`，未插入伪造 principal 或绕过 `identity_oidc_command_intent_active`。权限事实逐项由 owner 在本例隔离库修改，恢复 fixture 后再执行下一项。故障 trigger 只在本例库中创建，分别令审计和 receipt INSERT 真实失败；确认不存在新 key、对应 publish 审计或 receipt。

跨期限例把该例 intent 的接受时间设为真实数据库当前时间前 59 秒，先持有实际 `oidc-key-lifecycle-v1` advisory lock。实际 API 等待被 `pg_stat_activity.wait_event_type='Lock'` 观测后，等真实时间越过截止，重新读取 `clock_timestamp()>accepted_at+60秒`，再释放锁。API 必须409；503、500、timeout 都不能使该例通过。该设置用于期限边界，未改变服务器时钟或 key 高水位。

签名并发例使用两个实际连接：长 exp 签名持有 key 行，第二个较短 exp 被真实行锁阻塞。长事务提交后短事务成功，DB 保存长 exp；预备签名回滚不能推进高水位。随后分别证明退役及激活在真实签名事务上等待，并包含其最后提交 exp。

## 6. 清理与资源归属

最终 Linux PG 三例为：

| 数据库 | 六角色前缀 | 结果 |
|---|---|---|
| `identity_phase23_keys_01a182ca04a370118ccb7cc3724f9a31` | `i23k_70118ccb7cc3724f9a31` | DB／角色0残留 |
| `identity_phase23_keys_01a182ca120973faae512b37f7705f91` | `i23k_73faae512b37f7705f91` | DB／角色0残留 |
| `identity_phase23_keys_01a182ca1c3d737385f46c60ef3bf947` | `i23k_737385f46c60ef3bf947` | DB／角色0残留 |

每例正常返回、失败和 assertion panic 都回到统一 cleanup：关运行／owner pool、重新核对 cluster 与精确数据库 marker、DROP 该库 WITH FORCE、DROP 六个精确角色，并查询无残留。私钥 helper 为本例新建目录；Unix目录0700／文件0600，独立生成材料的临时目录也立即确认已删除，外层显式 `cleanup()` 返回错误则测试失败；Drop 仅作兜底。

Windows 开发失败与重试各轮也已执行相同数据库／角色清理。未复用旧被清理的数据库、密钥或角色；没有停止共享 PostgreSQL／其他监听，没有写 HBA，没有在源码／报告存储私钥、token、服务 secret 或管理 URL。共享编译目录由统一环境继续使用，待任务整体结束由其 owner 做最终定向清理。

最终独立残留查询结果：本轮命名空间数据库 `0`、角色 `0`，Linux `/tmp/identity-phase23-keyset-*` 目录 `0`，Windows 用户 TEMP 下同前缀目录 `0`。

本组件创建容器0、复用容器0，没有新增卷／网络／专用镜像或 Docker 日志，也没有运行 Docker prune 或 WSL 磁盘压缩。Docker 磁盘占用前后核查与所有阶段总资源清理由统一环境负责人归档；本报告不声称 Windows WSL 虚拟磁盘已缩小。

## 7. 输入 SHA-256

下列摘要绑定本报告组件输入；实际多进程部署仍需自己的源码／二进制／配置 attestation。

| 输入 | SHA-256 |
|---|---|
| `saas-identity/src/oidc/keys.rs` | `037423faf741b6995cf9c792cc346db2c91ca10e100c98102fc2be36984efee7` |
| `keys/actor_fixture.rs` | `8c7b4ca05f887f5ac41a22564ad4a9af3aa2465101545d9cb24e33ec1a9c30a9` |
| `keys/config_tests.rs` | `f2160b38a45d0dfc50e9af6558d0ac0c2a95e66a01632c17e126806ac7ac781e` |
| `keys/lifecycle_tests.rs` | `f567398c41dddbea5d5e9eb1f99f913cf8be890eca7caa1e57acf93c4d5ed587` |
| `keys/pg_fixture.rs` | `d8c0fc0897dc1979dc2333ed9c041b53ad569d9626c6e885a5989ed69fd1ba88` |
| `keys/test_keys.rs` | `8f9b4dc8be5f9cdf03c69329f779bda79c671eb373cd0a179bc0fcc3d0b75cef` |
| `saas-identity/docs/signing-key-rotation.md` | `f4250f60a616ded7283eeb7bcefb3ecc0c29e5f7ca0a57baa4dc5a8c4b1a6c1a` |
| `saas-identity/Cargo.toml` | `886f28e9354f7a6c5a8c5e90c27a4f3161070610eec9ce4af3f70d603d6fc411` |
| `migrations/0001_init.sql` | `33a3cb1f47b80ee38d97ed3d077b5c8296e5383edf75e909b310e183a0b03b59` |
| `identity-deployment/create-roles.sql` | `5389a1c2187aba441f8ce7b936950112369293818b0e7f2c5f84be7ec6681259` |
| `identity-deployment/provision.sql` | `7cd110d890c99fc8e532e5c4ca3bb185f787bc61a72acdac955b8fdeac80871b` |
| `identity-deployment/verify.sql` | `621e5d9efb1c3191d15b14263a44fba0e5da50519f680957c7d58e4417e39eb0` |

## 8. 自检与整体阶段接续

| 组件验收 | 结果 |
|---|---|
| canonical结构、真实LOGIN、现有权限与单一key权威 | PASS |
| 配置拒绝、公私材料匹配与无材料日志 | PASS |
| 发布／激活／终态、原 operation 恢复及围栏 | PASS |
| MAX高水位、签名行锁、激活和退役并发 | PASS |
| 审计／receipt／期限／当前授权的事务回滚 | PASS |
| 密钥轮换及恢复边界手册 | PASS |
| 本组件DB／角色／私钥定向清理 | PASS；整体共享资源清理由owner继续承担 |

交付主执行者汇总。Phase 23 整体的 K06 实时等待、真实员工／客户／独立 RP 验签、公开网关、真实 TOTP、两独立实例及 canonical 隔离恢复仍按统一环境运行记录判断；本组件不建议单独将整个 Phase 23 标为 COMPLETED。
