# Phase 23 数据库密码认证探针

独立 Rust/SQLx CLI，只连接固定 `127.0.0.1:25495`。该端口由环境执行者建立透明代理，转到任务 PostgreSQL 55494，服务端须看到 `127.0.0.2`。探针不修改 HBA、角色、密码、库或服务，不创建容器，也不执行恢复。

只有 `--input /TASK/0600.json --output /TASK/phase23-new0600.json` 两个参数。使用任务 owner **jackie** 执行；不接受密码、URL、目标主机或端口参数。源码、Cargo.toml 和 Cargo.lock 都是公开构建输入，须随后续快照固定。

## 精确输入形状

下例中的大写变量为环境执行者填入的私有值；禁止把实际输入文件放入源码或正式报告。

```json
{
  "contractVersion": "identity-db-password-probe-input/1",
  "binding": {
    "mode": "SOURCE",
    "sharedDatabase": "unified_auth_https_SOURCE32HEX",
    "portalDatabase": "unified_auth_portal_SOURCE32HEX",
    "rolePrefix": "uah_SOURCE_LAST12HEX",
    "deploymentId": "CANONICAL_UUIDV7"
  },
  "cases": [
    {
      "label": "IDENTITY_CORRECT",
      "database": "unified_auth_https_SOURCE32HEX",
      "role": "uah_SOURCE_LAST12HEX_identity_runtime",
      "passwordFile": "/tmp/rust-saas-unified-auth-20261028/phase23-identity-password",
      "expected": "ACCEPT",
      "expectedMarker": "identity-deployment/1/CANONICAL_UUIDV7"
    },
    {
      "label": "IDENTITY_WRONG",
      "database": "unified_auth_https_SOURCE32HEX",
      "role": "uah_SOURCE_LAST12HEX_identity_runtime",
      "passwordFile": "/tmp/rust-saas-unified-auth-20261028/phase23-wrong-password",
      "expected": "REJECT_28P01",
      "expectedMarker": "identity-deployment/1/CANONICAL_UUIDV7"
    }
  ]
}
```

`binding.sharedDatabase` **始终是 SOURCE 原库名**，尾部为恰好 32 位小写 hex；`rolePrefix` 必须为该尾部的末 12 位。`portalDatabase` 可为 null 或省略；存在时必须为相同尾部的源门户库。mode 只能为 SOURCE 或 RESTORE，case 的物理库与 marker 只能按下表推导。

| mode / 用途 | 允许的物理库 | 精确 marker |
| --- | --- | --- |
| SOURCE / identity、platform | `unified_auth_https_<source32hex>` | `identity-deployment/1/<deploymentId>` |
| SOURCE / portal | `unified_auth_portal_<source32hex>` | `unified-auth-https-portal/1/<deploymentId>` |
| RESTORE / identity、platform | `unified_auth_restore_<source32hex>` | `phase23-recovery/2/<deploymentId>` |
| RESTORE / portal | `unified_auth_restore_portal_<source32hex>` | `phase23-recovery/2/<deploymentId>` |

角色仅为同一个源前缀的 `identity_runtime`、`platform_runtime`、`portal_runtime`；portal 角色只连接所选门户库。label 为 `IDENTITY`、`PLATFORM`、`PORTAL` 与 `CORRECT`、`WRONG`、`OLD`、`NEW` 的 12 种固定组合，必须唯一且角色类别一致。cases 为 1–16 项。所有对象拒绝未知字段，expected 只能为 ACCEPT 或 REJECT_28P01。

## 文件与结果边界

TASK 固定为 `/tmp/rust-saas-unified-auth-20261028`，须为当前有效 uid 拥有的真实 **0700** 目录；OWNER 为同 uid 的 **0600** 单链接普通文件，内容准确为任务 supervisor marker，含末尾换行。input 和 passwordFile 必须为 TASK 直接子文件、当前有效 uid、0600、单链接普通文件。输入最多 64 KiB；每个密码文件最多 4096 字节，内容为非空 ASCII 可打印密码，可带一个末尾 LF。

Linux 目录 fd 一次固定；单层 `openat(O_NOFOLLOW|O_NONBLOCK)` 拒绝路径逃逸、symlink、hardlink、FIFO/设备、不安全权限及读取期间元数据变化。输出在任何数据库连接前以 `O_EXCL|O_NOFOLLOW` 独占创建为 0600，已存在时拒绝，不覆盖旧报告。报告仅含固定 contract、PASS/FAIL、label、固定 observation 和 SQLSTATE 分类，不含密码、URL、账号/角色名、库名、marker、文件路径、server text 或 traceback。

每 case 使用新连接池，最大连接数 1；**连接与证明查询合计 10 秒**，随后总是发起关闭，关闭另有 1 秒预算。PostgreSQL SSL 模式明确保持 baseline **Prefer**，包含 Rustls 支持；没有直连 55494、其他主机或代理失败后的 fallback。

- ACCEPT：新连接成功，且 SQL 同时证明 `inet_client_addr()=127.0.0.2`、current_user、current_database 和数据库 comment 与原 case 完全一致。
- REJECT_28P01：只有连接阶段的真实 `sqlx::Error::Database`、准确 SQLSTATE **28P01** 才 PASS。SQLSTATE 3D000/28000、连接/资源错误、超时、代理停止、错误 marker、unexpected accept、query 阶段错误和 close 失败全部 FAIL。不能把依赖不可用当作密码被拒绝。

exit 0 表示所有 case 符合预期；exit 1 表示已写出的 report 有 FAIL；exit 2 表示输入、文件或输出边界拒绝。stdout/stderr 只输出固定状态码。环境执行者应成对运行真实正确/错误密码，并分别记录 SOURCE/RESTORE 及轮换前后结果。

## 离线构建与调用

在 WSL 复用本任务已有 Cargo cache 和 target，不创建另一套缓存：

```sh
export PATH=/home/jackie/.local/rust-1.97/bin:/usr/local/bin:/usr/bin:/bin
export CARGO_HOME=/mnt/d/Software/Scoop/persist/rustup/.cargo
export CARGO_TARGET_DIR=/tmp/rust-saas-unified-auth-20261028/target-linux
export CARGO_INCREMENTAL=0
PROBE_SOURCE=/mnt/d/WorkSpace/rust/rust-saas/saas-plateform/backend/tests/unified-auth-https/phase23-db-auth-probe
cargo test --offline --locked --manifest-path "$PROBE_SOURCE/Cargo.toml" -j2 -- --test-threads=1
cargo clippy --offline --locked --manifest-path "$PROBE_SOURCE/Cargo.toml" --all-targets -- -D warnings
cargo build --offline --locked --manifest-path "$PROBE_SOURCE/Cargo.toml" -j2
```

环境执行者完成透明代理、真实目标库/marker、HBA 和私密文件的 jackie 归属后，在该 uid 下运行：

```sh
/tmp/rust-saas-unified-auth-20261028/target-linux/debug/phase23-db-auth-probe \
  --input /tmp/rust-saas-unified-auth-20261028/phase23-probe-input.json \
  --output /tmp/rust-saas-unified-auth-20261028/phase23-probe-result-new.json
```

文件/派生规则/错误分类自测与编译通过不能计作真实密码正反例。HBA、proxy、数据库、角色、私密输入、报告归档和退出后的定向清理由环境执行者负责；本 crate 不接管这些生命周期。
