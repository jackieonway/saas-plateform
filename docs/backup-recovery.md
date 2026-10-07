# PostgreSQL 加密备份与新库恢复

本手册对应 Phase 12 已确认的本机 Linux 原生准生产交付目标。平台库与工单库每日备份，RPO 24 小时、RTO 1 小时为演练目标；本项目不据此承诺商业 SLA。业务、通知、审计及认证历史没有自动删除任务。

## 1. 工具与凭据

`deploy/recovery/pg-backup.mjs` 使用 Linux Node 24、PostgreSQL 17 的 `pg_dump`／`pg_restore`／`psql`，不依赖 npm 包。数据库 URL、密码和客户端证书配置只在当前用户拥有的普通 `0600` 配置文件中提供；密码通过子进程 `PGPASSWORD` 环境传递，不进入命令参数或工具日志。原始 PostgreSQL 错误输出不会透传。

推荐由管理员部署代码到只读 `/opt/saas/current`，将配置与密钥放在 `/etc/saas/recovery`，备份放在独立的 `/var/lib/saas-backups`。配置和密钥不放入前端静态目录、Git、备份集合或聊天。备份目录必须由运行工具的用户拥有且没有组／其他用户权限；集合为 `0700`，密文为 `0600`。Windows 挂载目录的默认权限可能不符合门禁，验收时使用真正的 Linux 私有目录。

从模板 `deploy/recovery/config.env.example` 创建实际配置。用操作系统随机源生成 **32 字节二进制密钥**，并单独离线托管：

```sh
umask 077
openssl rand -out /etc/saas/recovery/backup.key 32
chmod 0600 /etc/saas/recovery/backup.key
chmod 0600 /etc/saas/recovery/backup.env
```

密钥丢失将无法恢复备份。将密钥或配置备份到另外的受控密钥保管位置，记录接管人和恢复方式；不要和密文放在同一个存储故障域。该工具不打印或自动创建密钥。

### 固定客户端

优先使用运维安装的 PostgreSQL 17 客户端。隔离验收可在局部目录下载与校验：

```sh
node deploy/recovery/install-clients.mjs target/phase12-runtime/postgres-client
```

固定文件与 SHA-256 见 `deploy/recovery/postgres-client-pin.json`：客户端 `17.11-1.pgdg24.04+2`，配套 libpq `18.6-1.pgdg24.04+2`，上游为 PostgreSQL 官方 apt 仓库。只解包，不全局安装。Ubuntu 26.04 的现有系统库可满足本次演练；其他 Linux 发行版须使用兼容包并重跑门禁，不据此宣称支持全部发行版。

配置中的 `PG_DUMP_BIN`、`PG_RESTORE_BIN`、`PSQL_BIN` 为绝对路径，例如 `<client-root>/usr/lib/postgresql/17/bin/pg_dump`；局部解包时 `PG_CLIENT_LIBRARY_PATH=<client-root>/usr/lib/x86_64-linux-gnu`。源码和客户端升级应重新评审并固定校验和。当前工具明确限制数据库与客户端 major 17，避免以未验证的跨版本恢复作为交付结果。

## 2. 备份与一致性窗口

平台与业务数据库必须分别配置。备份角色应具有所有应备份表的完整读取权限；不能使用受 RLS 限制而遗漏记录的角色。该工具完整备份数据库中的应用表、函数、索引、计数器、迁移记录与历史记录，不过滤已删除业务记录，不执行 `DELETE`。

两个数据库各自采用 PostgreSQL 一致性快照。**跨数据库的一致性来自共同停写窗口**：停止平台、工单、通知服务及所有其他写入定时任务／作业，等待连接与未完成写事务退出，再将 `BACKUP_WRITES_QUIESCED=true` 交给工具。这个字段是运维确认，并不能侦测未声明的外部写入者。若不能停写，应另行设计跨库一致性方案，不能跳过本条件。

```sh
node deploy/recovery/pg-backup.mjs backup --env-file /etc/saas/recovery/backup.env
```

成功时 stdout 只有一行 JSON：

```json
{"event":"backup-complete","backupSet":"/var/lib/saas-backups/<time>-<uuid>","setId":"<uuid>","startedAt":"<UTC>","completedAt":"<UTC>","durationMs":1234,"clients":{"PG_DUMP_BIN":"17.11","PG_RESTORE_BIN":"17.11","PSQL_BIN":"17.11"}}
```

每个集合包含 `platform.enc`、`work-orders.enc`、`manifest.enc`。数据库 custom dump 直接流式加密，不落盘明文。AES-256-GCM 的随机 nonce、认证附加数据及标签保护集合／角色／格式；加密清单记录完整 dump SHA-256、字节数、数据库版本及应用迁移版本／校验和。工具验证两份生成的密文后，才将 `.incomplete-<uuid>` 发布为新的不可变集合；再次运行创建新的集合，不覆盖旧集合。

失败时只输出稳定错误码，未完成集合保留为 `.incomplete-*`，不会假装成功，也不自动清除。运维可在确认集合无用且没有活动写入后，单独处理自己拥有的失败目录。成功集合复制到受控异机或对象存储后，再执行完整 `inspect`，避免单机磁盘故障同时损失库与备份。远端传输与保管由部署方配置。

## 3. 每日调度与故障处理

交付模板：

- `deploy/recovery/saas-backup.service` 与 `saas-backup.timer`：每天服务器本地时间 02:15，错过执行后补跑。
- `deploy/recovery/schedule.env.example`：明确列出全部写服务，顺序为重新启动依赖顺序。
- `deploy/recovery/scheduled-backup.mjs`：用参数数组调用 `systemctl`，记录原先 active 的服务，停写成功后备份，无论备份成功或失败都重新启动原 active 服务；原先 inactive 的服务保持原状态。服务状态不明确或停写失败即拒绝备份。

此调度包含每日停写窗口，应在部署计划中记录其影响。root service 模板用于执行已明确授权的本机服务管理；配置、代码和工具目录须由管理员控制。按实际部署路径修改 Node 路径、服务名单、`ReadWritePaths` 与告警处理器，安装后使用 `systemd-analyze verify` 检查。模板的 `OnFailure=saas-operations-alert@%n.service` 要连接实际部署的告警处理器。

```sh
systemctl daemon-reload
systemctl enable --now saas-backup.timer
systemctl list-timers saas-backup.timer
systemctl start saas-backup.service
journalctl -u saas-backup.service
```

SIGTERM／SIGINT 触发正常取消和服务重启流程；进程被 SIGKILL、机器掉电或 systemd 自身失效后，运维需要检查并恢复原写服务。不可把一次成功备份视为每天备份已运行。监控备份退出码、最近完整认证成功集合、年龄、存储空间及复制结果；建议 23 小时预警、24 小时告警。只有实际成功的集合能成为恢复点。没有自动保留期删除；增长、正式保留期与人工清理由后续运维决策确定。

调度状态恢复在适配器测试中验证；本机准生产完整演练由自己的原生进程提供停写窗口并调用真实备份 CLI。没有据此宣称本机已启用系统范围的每日备份服务。

## 4. 验证密文

```sh
node deploy/recovery/pg-backup.mjs inspect --env-file /etc/saas/recovery/backup.env --backup-set /var/lib/saas-backups/<time>-<uuid>
```

`inspect` 解密认证清单，并流式校验 **两份** GCM 标签、完整 SHA-256 与字节数，不持久保存明文。stdout 的 `backup-verified` JSON 包含集合 ID、时间、迁移版本／校验和与 dump 大小／哈希，不包含连接 URL、用户业务数据或密码。错误密钥、被修改元数据、损坏密文、缺失文件或替换为错误集合会拒绝。

## 5. 新库恢复与认证安全

始终恢复到未被任何网关／服务引用的 **新数据库**。在 `restore.env` 配置独立维护连接与明确的新平台／业务数据库名；配置文件同样普通文件、当前用户拥有、`0600`。维护角色需要建库及恢复权限。保护名称 `postgres`、`template0`、`template1` 和本次维护库均拒绝；现有目标即使为空也拒绝。两目标不能指向同一个数据库。工具不调用 `--clean`，不覆盖现库，不停止或删除其他连接／数据库。

```sh
node deploy/recovery/pg-backup.mjs restore --env-file /etc/saas/recovery/restore.env --backup-set /var/lib/saas-backups/<time>-<uuid>
```

执行顺序：

1. 认证清单，完整认证两份密文并比对 dump 哈希。**两份都通过之前没有数据库操作**。
2. 明文仅临时写入当前用户的 `0700`／文件 `0600` 的 Linux **tmpfs**；优先确认 `/tmp` 为 tmpfs，否则使用 `/dev/shm`。缺少足够的易失恢复空间会失败，不降级为磁盘明文。操作结束清除自己的临时目录；tmpfs 可能被系统 swap，应由运维禁用或加密 swap 并限制同用户访问。异常断电后也需检查 tmpfs 剩余目录。
3. 检查两个目标都不存在，以 `template0` 创建，再 `pg_restore --single-transaction --exit-on-error --no-owner --no-privileges`。恢复对象归维护角色所有； PostgreSQL 角色／ACL／RLS 的部署权责由授权 DBA 单独处理，应用 `membership_data_grants` 等授权数据完整保留。
4. 平台 dump 恢复后立即执行不可跳过的认证重置事务：全部用户 `session_version + 1`、撤销全部平台会话及 OAuth access／refresh／应用 token、消费和过期未使用授权请求／code、过期 device code。即使之后业务库恢复失败，也不会因成功恢复的平台 snapshot 复活旧认证。
5. 追加 `RECOVERY_AUTH_RESET` 审计，只包含集合 ID、重置计数和时间，不记录 token、用户名、个人 UUID 或数据库连接。没有删除历史认证数据。检查两库应用迁移版本及校验和与清单完全相同。

成功 stdout：

```json
{"event":"restore-complete","setId":"<uuid>","createdTargets":[{"role":"platform","database":"<new-platform>"},{"role":"work-orders","database":"<new-business>"}],"authenticationReset":{"users":1,"sessions":1,"tokens":2,"requests":1,"codes":1,"devices":1},"jwtKeyRotationRequired":true,"reloginRequired":true,"durationMs":1234,"migrations":{"platform":[],"work-orders":[]}}
```

错误 stderr 为 `{"event":"recovery-error","code":"<stable-code>"}`。定位故障时可额外提供固定 `operationStage`、`pgFailureClass`、`pgTool`、经过验证的五字符 `sqlState`、数字 `processExitCode` 和固定 `processSignal`；例如 `platform-auth-reset`／`CONSTRAINT`／`23514`。这些字段只描述阶段、类别和协议代码，不复制原始 stderr、SQL、连接值或异常文字。psql 使用 SQLSTATE verbosity；其他客户端的错误文本仅在有界内存中分类后丢弃。工具没有自动重试数据库变更。

创建过自己明确指定的目标后失败，会额外记录 `createdTargets`，方便运维调查；目标保留且仍须隔离，不自动删除，也不能将失败目标接入网关。清理或重试请使用另外明确的新名称；这避免误删重试期间已有的数据。

**恢复完成后先轮换 JWT signing key，再重新登录，最后开放网关。** 仅轮换签名密钥不足以撤销可能被 snapshot 复活的 legacy opaque 内省记录，因此数据库认证重置是强制步骤。按 `restore-complete` 仍需执行实际 API 拒绝旧 credential、重新登录获得上下文／授权及恢复数据检查，不直接把新库当作对外运行就绪。

备份不包含平台签名密钥、业务机密客户端运行配置、TLS 私钥、数据库角色定义和静态应用版本文件。运行凭据须单独安全托管；不可变微应用产物／宿主产物和部署配置须独立保存并校验，恢复数据库注册记录后仍需对应版本文件。恢复不会修改密码或扩大角色／数据授权。

## 6. 演练检查与证据

记录集合 ID、备份时间、开始恢复／完整服务就绪时间、认证校验结果、源／恢复库记录比较与清理记录。RPO 使用实际恢复点年龄；RTO 包含认证重置、轮换配置、迁移检查、服务重启和重新登录后的关键 API 验证。仅 CLI 耗时不代表完整服务恢复时间。

```sh
node --test deploy/recovery/tests/envelope.test.mjs deploy/recovery/tests/schedule.test.mjs deploy/recovery/tests/systemd.test.mjs deploy/recovery/tests/diagnostics.test.mjs
node --env-file=.env.test --test deploy/recovery/tests/postgres-roundtrip.test.mjs
```

实际 PostgreSQL 测试只为已授权隔离实例创建随机命名的自有库，并在结束后清理自己的库／连接／临时目录；不会打印 `.env.test`。WSL 验收主机已有 TCP 分段问题时，允许同一测试进程使用既有局部 helper，不能将其当成生产网络方案。

本次恢复组件实测：Linux Node `24.3.0`、PostgreSQL 服务 `17.6`、客户端 `17.11`；8 项 crypto/config／调度／systemd 模板／安全诊断门禁通过，真实 PostgreSQL 7 项门禁通过，0 skip。systemd 门禁仅在私有副本中替换实际 Node 路径后验证，没有安装系统 unit 或启用 timer。真实门禁应用全部 22 条平台迁移及工单迁移，恢复授权、通知／发布、原有审计、软删除工单、计数器和校验和，证明错误 key、损坏第二份 dump、已存在第二目标及维护库名称在任何新库创建前拒绝，输出真实连接故障时不泄漏地址／密码，并验证真实 SQLSTATE 异常中的私有 SQL 文本不会进入错误对象。恢复认证状态按强制安全动作改变，其余保留数据逐表比较。最终准生产全链路恢复及 RPO／RTO 证据见 Phase 12 验收报告，组件耗时不能替代该报告。
