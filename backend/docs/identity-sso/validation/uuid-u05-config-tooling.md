# UUID-U05 配置工具验证

状态：**CODE_VALIDATED / PRIVATE_RUNTIME_EXECUTION_PENDING**。本文只证明配置工具和有限 owner handler 的实现验证；未将 U05 标为完成，未证明真实配置已改号、secret 已轮换或服务已可用。

工具入口为 `saas-recovery uuid-config-rekey`。操作合同和配置示例位于 [uuid-config-rekey-contract.md](../../../ops/recovery/uuid-config-rekey-contract.md) 与 [uuid-config-rekey.example.json](../../../ops/recovery/uuid-config-rekey.example.json)。实现位于 `backend/ops/recovery/src/uuid_rekey/config_rekey.rs` 及其三个有限 owner 子模块。

实现复用 `verify::load_verified` 认证 source/converted/map/registry 全包及映射源闭包。只处理既有登记的 `data-grant-policy`、`platform-admin-grants` 和 regeneration-only 的 `preview-private-state`；未改 v2 注册表、U03/U04 行为、真实秘密或数据库。policy/grants 按封闭 owner DTO 改号并在改号前后验证语义，wildcard 只访问 map value、保留逻辑键；typed map resolve 必须命中相应 authority 的 source entity。旧 state 最终只产出常量重建要求和 typed counts，没有任何旧/新业务身份清单。非空 organization children 明确阻断，真实旧 state 如有这种树，须整体隔离并通过新目标 API 重建。

文件输入限制 owned mode 0600 单链接普通文件；Linux 使用 `O_NOFOLLOW | O_NONBLOCK`，打开后重新检查文件类型、mode、owner、nlink、长度及与检查前一致的 dev/ino。新输出 parent 要求 owned mode 0700，新目录 0700 / 文件 0600 create-new，不覆盖旧文件；可复用输出逐字节回读，完成 summary 另以 package key 和独立 purpose 认证封存。

2026-10-24 最终执行结果：

| 检查 | 结果 |
| --- | --- |
| Windows `cargo fmt --all -- --check` | PASS |
| Windows `cargo test --locked --offline --all-targets -- --quiet` | 150 passed，0 failed |
| Linux 同一命令，独立 `/runtime/uuid-config-target` | 161 passed，0 failed，1 ignored |
| Windows / Linux `cargo clippy --locked --offline --all-targets -- -D warnings` | PASS |
| Linux `cargo build --locked --offline --release` | PASS；独立 target，不覆盖 U04 执行二进制 |

ignored 项为既有 `import::tests::write_private_preflight_sql_for_operator`，须显式指定私有 tmpfs 目录才能产出 SQL；它不是配置 owner 测试，也不代表真实导入验收通过。

配置子模块共有 20 项跨平台 fixtures，另有 1 项 Linux 文件权限/符号链接/硬链接/大小限制回归。覆盖严格重复键、全部封闭 DTO 的未知字段和错误脱敏、canonical source v4/v7 与 target v7、版本和字段类型、owner 集合上限/唯一性/lookup 闭包、有限应用菜单/permission、target owner 一致性、跨 authority 同原 UUID、map values 的 wildcard、完整 policy/grants 改号后验证、业务字段和顺序保持、缺映射拒绝、未知 UUID 路径阻断、session/secret/initialPassword 丢弃、regeneration 输出不含身份/秘密和未知递归树阻断。独立只读复核发现的 private-file TOCTOU 和嵌套 schemaVersion 绕过均已修复并加入覆盖。

本次工具源文件 SHA-256：

| 文件 | SHA-256 |
| --- | --- |
| `config_rekey.rs` | `e8a88ed8f54a64204a14c054eba448e245abd1c1ec98e01a1fd8b42f298906cc` |
| `config_policy.rs` | `1e903753f1d18a3d69b9251944a5cec9ece954de043dd39b50d09de9def22dca` |
| `config_grants.rs` | `fb9f2737b6be25db97426d0a43f99f85173ab93324a674d14ea8cc81538d8124` |
| `config_state.rs` | `46541400766c1ae20f8ff87014e40b4bb6ce9221df590628fe5d55ad306e31f5` |

v2 registry 摘要仍为 `da5eeacbf56af6a577d9b7cb4d7e2d2f10482bee79a3efb5254a10a88dd5c313`。

独立 Linux release 位于运行 volume 的 `/runtime/uuid-config-target/release/saas-recovery`，SHA-256 为 `ac778c31bc2ee00377bb5a59517e8c807a374b98a978385a2ba0c9c1058865a5`。它是本次工具交付版本，不替代各 U03/U04 历史执行版本记录。

后续实际执行须记录新 binary SHA、真实 input/output 摘要和 typed counts、认证 manifest 结果及新启动配置的消费校验。不要把 fixtures、工具 release 成功或认证源包当作真实配置/运行链路的完成证据。fresh login、confidential client secret 轮换、JWT key 轮换、目标 API 重查身份和运行验收由主任务单独执行并写入正式 U05 validation。
