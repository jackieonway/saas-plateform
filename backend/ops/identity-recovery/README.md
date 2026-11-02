# Canonical 身份感知隔离恢复

本工具为当前 `0001`／`schema_version=1` 提供受控离线恢复。当前基线只建立统一 `auth_*` 权威，签名状态使用 `auth_signing_keys`／`auth_provider_settings`，管理幂等记录使用 `identity_auth_command_receipts`／`platform_auth_command_intents`。不接管旧迁移 checksum，也不接受旧 registry、旧签名表或旧恢复 inventory 作为当前版本。结构退役及当前版本验证见[退役清单](../../docs/designs/unified-auth/phase-24-retirement-list.md)；[Phase 23 演练](../../docs/designs/unified-auth/validation/phase-23.md)只证明该报告记录的冻结输入，不替代当前版本恢复验证。本文和纯组件测试不是部署或实际恢复通过证据。

工具由任务环境 owner 运行；它不修改 HBA、不停止 PostgreSQL、不删除源库或目标库、不启动应用、不自动开放流量。任何失败均返回固定 `QUARANTINED` 类别，调用方须在 `finally` 完成本轮定向清理。正式库恢复须另有明确资产、授权、可信人员和完整库外控制日志；本工具的 task binding 不是生产授权。

## 模块与安全边界

- `recovery.py` 是薄 CLI；`core.py` 处理私有文件、连接、加密 checkpoint、隔离新库、原 owner/ACL 恢复、启动 receipt 和明确 rearm。
- `inventory.py` 是当前结构的显式 typed 合同：平台共享库 86 张表、门户库 5 张表、950 个字段；每库一次 SELECT 捕获并核对真实列类型、nullable 和 PK。这个清单覆盖恢复所需实体，不等于共享库全部业务表数量。上下文登记只允许 `AUTH`，应用及应用策略版本不可为空。遗漏／额外 canonical 族、类型漂移、重复 PK、非法代际或不变事实均拒绝。PHC、密文、任意 JSON、原 state/nonce、秘密和个人资料（含职位与坐标）只在 inventory 中保留 SHA256，原始数据仅进入加密 artifact。
- `witness.py` 使用有版本 HMAC 链、认证 CURRENT、POSIX anchored open、0600 文件、0700 目录、fsync 和单 writer 锁。每次追加保留被删除对象的历史 highwater；读取重验整条链。没有 v1／旧 16 族 fallback。
- rearm 读取整条已认证历史，拒绝中间轮换后被覆盖／删除的 secret 跨用途复用；按原 `client_id` 汇总已删除 credential 的最高历史版本，新凭据必须严格高于该版本。私有文件以非阻塞方式打开后先核对常规文件类型，损坏 FIFO 不会卡住错误处理和清理。
- 邀请 keyset 追加独立的新 `activeVersion`，保留已认证备份中的历史版本，用于原领取操作摘要与不可变 receipt 的校验；旧邀请及核验的关闭状态、消费账号停用和旧投递终态不变。历史 key 不作为新签发版本，且不得跨用途复用。当前 loader 最多接受 16 项，无法同时保留历史与新活动版本时在输出前失败，不静默丢弃原操作仍需的验证材料。原合同见[邀请密钥和原结果恢复](../../../../saas-identity/docs/internal-api.md)。
- 业务身份选择登记先全部隔离。只有双快照中 registry／成员类型／应用及其策略代际／consumer 绑定未变、客户端与 consumer 均 eligible 且本次明确选中时，rearm 才将登记推进到本轮生成的精确客户端代际。未选、已禁用或有绑定／应用历史代际漂移的登记继续禁用；不会借恢复更新其业务用途。gate、SQL 验证与只读不变量对登记全表及实际客户端、应用代际逐字段核对，旧代际和任意额外推进均拒绝。
- `reconcile.py` 按当前真实触发器和 FK 生成 deny-only SQL，保留 audit／receipt／subject／source tombstone，显式关闭包括 offline USER、MACHINE 和两 BFF 在内的所有旧载体。未知 refresh 不能仅清 lease 后重用，已消费结果不能重签，投递预算和终态不回退。
- 每个 dump、库外材料和原始事实 delta 使用独立随机 nonce 的 AES-256-GCM，以任务／cluster／两源库／issuer／artifact kind 为 AAD。checkpoint、外部 floor 和 start receipt 另用独立 HMAC key 认证；witness 使用第三把独立 key。
- catalog 指纹覆盖 schema/relation/column/function/default ACL、SECDEF 和完整 function definition、约束、触发器状态、索引定义／partial predicate／unique／valid／ready，以及 SQLx 版本与 checksum。relation 的 NULL ACL 按 PostgreSQL 表／序列的 owner 默认 ACL 展开，允许 `pg_restore` 省略等价的显式 owner 默认授权；额外授权、grantor／owner 或 column ACL 变化仍拒绝。恢复后、reconcile 前、最终 gate 再核对，不仅比较数据。

## 执行前提和配置

需要 POSIX Python 3、现有 `cryptography`（AESGCM、Argon2id）和 PostgreSQL 18 的 `psql/pg_dump/pg_restore`。纯测试不连接服务、不创建 Docker 或数据库：

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s saas-plateform/backend/ops/identity-recovery -p 'test_*.py' -v
```

所有 CLI 参数只包含操作、文件路径及非秘密序号。配置、连接、新密码和秘密映射必须使用绝对 0600 常规文件；文件及私有目录 owner 必须与执行 UID 一致，不接受符号链接或多硬链接私有文件。整个工作区、备份目录或命令参数不得包含真实密码／token／PHC 输出。

0600 配置 JSON 的字段固定如下。尖括号均为需要环境 owner 绑定的参数，不是可直接执行的示例凭据：

```json
{
  "format": "identity-recovery/2",
  "taskId": "<current deployment UUIDv7>",
  "clusterId": "<本轮实际 PostgreSQL system_identifier 的十进制字符串>",
  "issuer": "https://auth.saas.test",
  "taskRoot": "/tmp/rust-saas-unified-auth-20261028",
  "workDirectory": "/tmp/rust-saas-unified-auth-20261028/phase23-recovery-private",
  "witnessDirectory": "/tmp/rust-saas-unified-auth-20261028/phase23-recovery-witness",
  "floorFile": "/tmp/rust-saas-unified-auth-20261028/phase23-recovery-floor.json",
  "keyFile": "/tmp/rust-saas-unified-auth-20261028/phase23-recovery-keys.json",
  "adminConnectionFile": "/tmp/rust-saas-unified-auth-20261028/phase23-admin-connection.json",
  "pgBin": "/usr/lib/postgresql/18/bin",
  "databases": {
    "platform": {
      "source": "unified_auth_https_<32 hex>",
      "target": "unified_auth_restore_<same 32 hex>",
      "owner": "<this task migration owner>",
      "sourceMarker": "<exact original database comment containing deployment UUID>",
      "targetMarker": "phase23-recovery/2/<deployment UUID>",
      "connectionFile": "/tmp/rust-saas-unified-auth-20261028/phase23-platform-connection.json"
    },
    "portal": {
      "source": "unified_auth_portal_<same 32 hex>",
      "target": "unified_auth_restore_portal_<same 32 hex>",
      "owner": "<this task portal owner>",
      "sourceMarker": "<exact original portal database comment containing deployment UUID>",
      "targetMarker": "phase23-recovery/2/<deployment UUID>",
      "connectionFile": "/tmp/rust-saas-unified-auth-20261028/phase23-portal-connection.json"
    }
  },
  "privateMaterials": {
    "environment": "/tmp/rust-saas-unified-auth-20261028/phase23-environment.json",
    "signing": "/tmp/rust-saas-unified-auth-20261028/phase23-signing.json"
  }
}
```

`privateMaterials` 必须由环境 owner 穷举当前实际使用的签名／BFF／context／TOTP／邀请／服务／RP／机器／资源／metrics／runtime 连接与 CA 等材料。上面两个标签只展示格式，不代表完整资产集。keyFile、floorFile 和 witness 必须在备份 workDirectory 外；不能把它们加进 privateMaterials。`init` 生成的三把独立 key 不进入备份。

`clusterId` 由环境 owner 从本轮已确认归属的集群执行 `SELECT system_identifier::text FROM pg_control_system()` 读取后写入上述受保护配置；必须为无前导零的非零 64 位无符号十进制字符串（最大 `18446744073709551615`），不能使用 JSON 数值或沿用旧演练值。初始化新集群后重新生成本轮配置与备份绑定；每次数据库操作前的 `guard` 仍逐字核对实际集群标识，配置格式合法不代表可访问其他集群。

adminConnectionFile 仅允许字符串 `PGHOST=127.0.0.1`、`PGPORT=55494`、`PGUSER=postgres`、`PGDATABASE=postgres` 和可选 `PGPASSWORD` 字段，以 JSON 对象表示。连接秘密传子进程环境，SQL 和 dump 走 stdin，命令行不含连接串或密码。每库 connectionFile 为环境 owner 的源连接资产引用；恢复管理作业统一使用前述独立管理员，不把 runtime 升权为恢复管理员。

源与目标复用本轮独占 owner/runtime 角色，不做 role remap。环境 owner 停止全部源写入者、worker 和候选实例并核对 0 其他连接；恢复前撤销源的 runtime CONNECT，保持源隔离。新目标 name/comment 是明确物理映射；库内原 deployment UUID、owner、grantor、ACL、函数和 SQLx 事实保持不变。不得用 `--no-owner`／`--no-acl` 或重新 provision 掩盖漂移。

## 分步运行

以下命令必须由环境 owner 按前置状态串行调用。`RECOVERY_CONFIG` 指向已核对的私有配置；`MINIMUM_SEQUENCE` 是从上一次成功输出独立保留的序号，不从旧备份推导。key 与运行材料不进入这些变量：

```sh
python3 saas-plateform/backend/ops/identity-recovery/recovery.py init --config "$RECOVERY_CONFIG"
python3 saas-plateform/backend/ops/identity-recovery/recovery.py capture --config "$RECOVERY_CONFIG"
python3 -B saas-plateform/backend/ops/identity-recovery/negative_probe.py \
  --stage save-baseline --config "$RECOVERY_CONFIG" --minimum-sequence 1
```

`capture` 要求两源无其他连接，记录逐库 custom dump、完整 catalog／ACL、typed inventory 与加密库外材料；确认捕获前后数据未变后认证 checkpoint，创建 witness 序号 1 和独立 floor。顺序双库 dump 依赖 owner 的停写屏障，不声称 PostgreSQL 跨库原子 snapshot、PITR 或持续 RPO。

随后只在本轮源 fixture 受控运行中产生实际拒绝事实。完成后再次停所有 writer／worker／pool，再捕获：

```sh
python3 saas-plateform/backend/ops/identity-recovery/recovery.py capture-deny \
  --config "$RECOVERY_CONFIG" --minimum-sequence "$MINIMUM_SEQUENCE"
```

它保存全族拒绝事实、高水位和已认证加密原始 delta，并把新序号和摘要输出给环境 owner 独立记录。中断导致孤儿 witness／floor 不一致时保持隔离，不自动挑选更旧状态。整个外部目录和 floor 一起回滚仍需调用方独立保存的 `MINIMUM_SEQUENCE` 才能检测；本机 HMAC 链不冒充异地不可变控制日志。

源全部停写、0连接且两个目标尚未创建时，执行真实文件负向探针：

```sh
python3 -B saas-plateform/backend/ops/identity-recovery/negative_probe.py \
  --stage probe --config "$RECOVERY_CONFIG" --minimum-sequence "$MINIMUM_SEQUENCE"
```

save-baseline 保留序号1的真实签名floor/CURRENT/record到witness目录之外的0700目录；probe要求独立最新序号至少2，验证密文／manifest篡改、错误加密key、混绑定、真实旧floor／CURRENT回放、缺链和额外链。它调用真实认证文件加载器及只读PG预检，逐案检查源连接为0、目标仍不存在，**不执行restore SQL或任何数据库写入**。每次变更先写0600 before-image journal，finally只在当前字节仍等于本次变更时精确还原；并发冲突保留journal和他人内容并停止，不blind reset。此层级不能冒充真实ACL漂移、磁盘耗尽或完整restore SQL负向。

```sh
python3 saas-plateform/backend/ops/identity-recovery/recovery.py restore-new \
  --config "$RECOVERY_CONFIG" --minimum-sequence "$MINIMUM_SEQUENCE"
python3 saas-plateform/backend/ops/identity-recovery/recovery.py reconcile \
  --config "$RECOVERY_CONFIG" --minimum-sequence "$MINIMUM_SEQUENCE"
python3 saas-plateform/backend/ops/identity-recovery/recovery.py gate \
  --config "$RECOVERY_CONFIG" --minimum-sequence "$MINIMUM_SEQUENCE"
```

restore-new 在首次 CREATE 前认证全部 artifact、floor 和归属，并确认两个目标都不存在。它仅新建精确目标，设置 `CONNECTION LIMIT 0`，保留 owner／grantor／ACL 恢复并逐项比较，恢复私有材料到新目录而不覆盖原文件。失败目标保持隔离、没有 start receipt；环境 owner 的 finally 按已记录归属清理，本工具绝不自行 DROP。

reconcile 先合并有原 FK 来源的不可变 receipt、库外 tombstone 和 JTI、原操作结果、key 吊销及验证窗口、投递终态和次数，再显式失效所有旧载体，账号及独立 service 凭据先禁用。缺失的原 grant／user／RP 不被制造出来满足 receipt FK；例如备份后新账号的邀请 activation receipt 无法安全物化时失败关闭，保留外部 witness 并要求对账／更近完整 checkpoint。旧 grant 的所有在线／offline／MACHINE 分支均独立失效。跨库不是原子提交；任一库失败都不得启动，不能把部分提交称为恢复成功。

## 明确 rearm 和接流门禁

本轮三服务／标准RP fixture 可使用[纯文件适配器](../../tests/unified-auth-https/phase23-recovery-files.py)生成配置与新运行资产。它不连接数据库或启动进程；prepare读取既有私有source environment、原cleanup manifest及 `{ca,certificate,key}` 的0600 TLS路径映射，生成固定13类材料合同。rearm要求 `--minimum-sequence` 和明确eligible selection，从实际解密还原的材料取原签名／TLS字节，独立生成新密码、秘密、AEAD／TOTP／邀请材料和三runtime密码；原source及cleanup manifest不覆盖。

```sh
python3 -B saas-plateform/backend/tests/unified-auth-https/phase23-recovery-files.py prepare \
  --task-dir "$TASK_DIR" --environment "$SOURCE_ENV" --manifest "$SOURCE_MANIFEST" \
  --output-config "$RECOVERY_CONFIG" --tls-materials "$TLS_MATERIALS"
python3 -B saas-plateform/backend/tests/unified-auth-https/phase23-recovery-files.py rearm \
  --config "$RECOVERY_CONFIG" --environment "$SOURCE_ENV" --selection "$SELECTION_FILE" \
  --output-env "$TARGET_ENV" --output-rearm "$REARM_FILE" --minimum-sequence "$MINIMUM_SEQUENCE"
```

adapter的rearm在core reconcile成功之后运行，随后再执行下面core rearm／gate。它额外生成 `phase23-rearm-runtime-passwords.json`；环境owner必须先按新env中的摘要核对该保护文件，才由root／owner执行对应本轮角色的数据库密码轮换及实际SCRAM正反例。恢复环境将旧OIDC配置请求恢复为source metadata中的300秒；演练中请求的600秒不改变canonical Access／ID最多300秒、code固定60秒的实现。旧载体必须按实际数据库截止逐次验证，不得按配置请求推断期限。

环境owner也可使用[窄串行调用器](../../tests/unified-auth-https/phase23-recovery-cycle.py)一次执行尚未开始的 restore-new → reconcile → adapter rearm → core rearm → gate：

```sh
python3 -B saas-plateform/backend/tests/unified-auth-https/phase23-recovery-cycle.py \
  --config "$RECOVERY_CONFIG" --environment "$SOURCE_ENV" --selection "$SELECTION_FILE" \
  --output-env "$TARGET_ENV" --output-rearm "$REARM_FILE" --output "$CYCLE_RECEIPT" \
  --minimum-sequence "$MINIMUM_SEQUENCE"
```

调用器先只读确认源0连接、三个runtime的源CONNECT确实撤销；所有输入前后摘要固定，每步只接受原定status并记录安全metadata。它不改HBA／角色密码／连接限额、不DROP或启动服务；子进程超时或owner中断只结束本次独立会话中的CLI／PG客户端，不触及独立PGserver。SIGTERM/HUP受控处理，中断先写隔离receipt再返回130。cycle耗时不等于完整RTO。cycle通过后，环境owner在应用仍停止时按gate输出将目标connlimit恢复原值，执行root负责的密码轮换与真实SCRAM正反例；探针关闭后再次离线gate，再启动新应用。正确runtime密码的正向连接需要目标已允许连接，不能在connlimit0时把53300误当密码认证结果。

本轮隔离 fixture 可以用另一个0600 JSON 明确指定新凭据：

```json
{
  "users": [{"id": "<eligible user UUID>", "password": "<fresh protected password>"}],
  "clients": [{"id": "<eligible confidential client UUID>", "secret": "<fresh protected secret>"}],
  "memberships": ["<eligible membership UUID>"],
  "services": [{"table": "identity_service_clients", "id": "<eligible service UUID>", "secret": "<fresh protected secret>"}],
  "materials": {"environment": "<absolute new 0600 environment file>", "signing": "<absolute 0600 signing file>"}
}
```

精确 eligible 列表来自 reconcile 的已认证 requirements：必须在 checkpoint 和最新 witness 都合法且无退休／缩权矛盾。三种 service 表仅限 `identity_service_clients`、`context_selection_services`、`platform_resource_services`；绑定和权限不由 rearm 修改。未指定或不 eligible 的账号、成员、客户端和服务不自动恢复。全新密码使用 Argon2id v19、32 MiB／t2／p1；拒绝与 checkpoint 或最新旧 PHC 相同的密码。机密客户端新凭据版本高于所有历史版本，最长90天；旧凭据保持撤销，service secret 同样拒绝复用旧 hash。

```sh
python3 saas-plateform/backend/ops/identity-recovery/recovery.py rearm \
  --config "$RECOVERY_CONFIG" --minimum-sequence "$MINIMUM_SEQUENCE" --rearm-file "$REARM_FILE"
python3 saas-plateform/backend/ops/identity-recovery/recovery.py gate \
  --config "$RECOVERY_CONFIG" --minimum-sequence "$MINIMUM_SEQUENCE"
```

materials 标签必须与配置穷举集完全一致。环境 owner 生成新的 BFF／context／邀请／TOTP 等材料，更新新环境连接目标和 secret，并保留正常窗口所需的合法签名 key。gate 认证最新 witness/floor、精确 fenced inventory、catalog／ACL／trigger／索引和材料 SHA；任一备份回退、变更或丢失都拒绝。`opensTraffic:false` 表示它不负责开流量；只有环境 owner 在最终 gate 后按批准资产开放目标 CONNECT／connection limit、启动同一验证二进制与唯一 issuer。

数据库密码轮换由 root 作为 HBA 唯一 owner，通过当前[驱动](../../tests/unified-auth-https/phase23-execute.ps1)与[有journal的HBA包装器](../../tests/unified-auth-https/phase23-hba.py)限定精确角色／库／127.0.0.2 SCRAM 路径，并在 finally 还原原字节、stat、reload后清理receipt。trust 连接成功不计密码轮换证据。所有旧密码、RP／service／机器／资源 secret、Cookie、code、Access／refresh 和一次性邀请须在其恢复前成功的原正常端点验证拒绝；再完成新的员工、客户、第三方和机器业务。完整调用和证据分组要求见[当前验证说明](../../tests/unified-auth-https/README.md)。

## 计时、失败与清理

RTO 从故障／停止接流开始，直至完整对账、旧拒绝、新配置重启和新双 RP 业务成功才结束。RPO 按真实 checkpoint 年龄及备份后业务 marker 丢失计算；主动废弃的认证状态单列。比较内部候选 RPO 15 分钟／RTO 4 小时，不能把小样本耗时扩写为生产持续 WAL、异地恢复、备份频率、真实值班响应或99.9%承诺。

成功、失败或中断后，环境 owner 必须停本轮服务、关闭连接，按源／目标精确名称与 comment 清除本轮四库和八角色，清除私有环境、新旧秘密、加密备份、witness/floor/keys、临时日志和故障文件，核对无残留。只保留脱敏正式报告与获准保留的公共工具／构建缓存，不触其他阶段或业务5432／既有443。不得调用全局 Docker prune，也不得声称删除 Linux 文件已经缩小 Windows WSL 磁盘。
